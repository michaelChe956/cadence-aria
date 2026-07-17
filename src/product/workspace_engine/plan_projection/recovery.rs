use super::*;

impl WorkspaceEngine {
    pub(crate) fn load_initial_plan_compile_outcome(
        &self,
        tx: &WorkItemPlanCompileTransaction,
    ) -> Result<Option<InitialPlanCompileOutcome>, WorkspaceEngineError> {
        if tx.project_id != self.session.project_id
            || tx.issue_id != self.session.issue_id
            || tx.plan_id != self.session.entity_id
            || tx.previous_plan_snapshot.project_id != tx.project_id
            || tx.previous_plan_snapshot.issue_id != tx.issue_id
            || tx.previous_plan_snapshot.id != tx.plan_id
        {
            return Err(WorkspaceEngineError::InvalidInitialPlan(
                "initial plan compile transaction scope is inconsistent".to_string(),
            ));
        }
        let store = self.revision_store();
        let journal = match store.get_initial_plan_publication_journal(
            &tx.project_id,
            &tx.issue_id,
            &tx.plan_id,
            &tx.compile_id,
        ) {
            Ok(journal) => journal,
            Err(ProductStoreError::NotFound { .. }) => {
                return match store.get_plan_lineage(&tx.project_id, &tx.issue_id, &tx.plan_id) {
                    Err(ProductStoreError::NotFound { .. }) => Ok(None),
                    Ok(_) => Err(WorkspaceEngineError::InvalidInitialPlan(
                        "active initial plan publication journal is missing".to_string(),
                    )),
                    Err(error) => Err(error.into()),
                };
            }
            Err(error) => return Err(error.into()),
        };
        let tx_draft_ids = tx.active_draft_ids.iter().cloned().collect::<BTreeSet<_>>();
        let journal_draft_ids = journal
            .active_draft_revision_ids
            .values()
            .cloned()
            .collect::<BTreeSet<_>>();
        if journal.outline_version_ref != tx.outline_version_ref
            || journal.created_at != tx.created_at
            || tx_draft_ids.len() != tx.active_draft_ids.len()
            || journal_draft_ids.len() != journal.active_draft_revision_ids.len()
            || journal_draft_ids != tx_draft_ids
        {
            return Err(WorkspaceEngineError::InvalidInitialPlan(
                "initial plan publication does not match the compile transaction".to_string(),
            ));
        }
        let logical_ids = journal
            .artifacts
            .work_items
            .iter()
            .map(|item| item.logical_work_item.id.clone())
            .collect::<Vec<_>>();
        let allocated_ids = store.allocate_initial_plan_publication_ids(
            &tx.project_id,
            &tx.issue_id,
            &tx.plan_id,
            &tx.compile_id,
            &logical_ids,
        )?;
        if journal.allocated_ids != allocated_ids {
            return Err(WorkspaceEngineError::InvalidInitialPlan(
                "initial plan publication allocator identity is inconsistent".to_string(),
            ));
        }
        let lineage = match store.get_plan_lineage(&tx.project_id, &tx.issue_id, &tx.plan_id) {
            Err(ProductStoreError::NotFound { .. }) => return Ok(None),
            Ok(lineage) => lineage,
            Err(error) => return Err(error.into()),
        };
        match lineage.active_revision_id.as_deref() {
            None => return Ok(None),
            Some(active) if active == journal.artifacts.plan_revision.id => {}
            Some(_) => {
                return Err(WorkspaceEngineError::InvalidInitialPlan(
                    "active plan revision does not match the compile publication".to_string(),
                ));
            }
        }

        let published = store.publish_or_resume_initial_plan_revision(&journal)?;
        if published.phase != InitialPlanPublicationPhase::PlanActivated {
            return Err(WorkspaceEngineError::InvalidInitialPlan(
                "active initial plan publication marker is missing".to_string(),
            ));
        }
        let expected = published.artifacts;
        if expected.plan_revision.reason != PlanRevisionReason::InitialCompile {
            return Err(WorkspaceEngineError::InvalidInitialPlan(
                "active initial plan revision reason is not initial_compile".to_string(),
            ));
        }
        let live_lineage = store.get_plan_lineage(&tx.project_id, &tx.issue_id, &tx.plan_id)?;
        if live_lineage.active_revision_id.as_deref() != Some(expected.plan_revision.id.as_str()) {
            return Err(WorkspaceEngineError::InvalidInitialPlan(
                "active plan revision does not match the compile publication".to_string(),
            ));
        }
        let mut expected_live_lineage = expected.lineage.clone();
        expected_live_lineage.active_revision_id = Some(expected.plan_revision.id.clone());
        if live_lineage != expected_live_lineage {
            return Err(WorkspaceEngineError::InvalidInitialPlan(
                "active initial plan lineage differs from the publication journal".to_string(),
            ));
        }
        let plan_revision = store.get_plan_revision(
            &tx.project_id,
            &tx.issue_id,
            &tx.plan_id,
            &expected.plan_revision.id,
        )?;
        let dependency_graph_revision = store
            .get_dependency_graph_revision(&live_lineage, &expected.dependency_graph_revision.id)?;
        let validation_report =
            store.get_plan_validation_report(&live_lineage, &expected.validation_report.id)?;
        let plan_projection_bundle =
            store.get_plan_projection_bundle(&live_lineage, &expected.plan_projection_bundle.id)?;
        if plan_revision != expected.plan_revision
            || dependency_graph_revision != expected.dependency_graph_revision
            || validation_report != expected.validation_report
            || plan_projection_bundle != expected.plan_projection_bundle
        {
            return Err(WorkspaceEngineError::InvalidInitialPlan(
                "active initial plan artifacts differ from the publication journal".to_string(),
            ));
        }

        let mut work_items = Vec::with_capacity(expected.work_items.len());
        for item in expected.work_items {
            let mut expected_logical = item.logical_work_item;
            expected_logical.active_revision_id = Some(item.work_item_revision.id.clone());
            let logical = store.get_logical_work_item(&live_lineage, &expected_logical.id)?;
            let draft_revision =
                store.get_draft_revision(&live_lineage, &item.draft_revision.id)?;
            let work_item_revision = store.get_work_item_revision(
                &live_lineage,
                &expected_logical.id,
                &item.work_item_revision.id,
            )?;
            let verification_plan_revision = store.get_verification_plan_revision(
                &live_lineage,
                &item.verification_plan_revision.id,
            )?;
            let projection_bundle =
                store.get_work_item_projection_bundle(&live_lineage, &item.projection_bundle.id)?;
            if logical != expected_logical
                || draft_revision != item.draft_revision
                || work_item_revision != item.work_item_revision
                || verification_plan_revision != item.verification_plan_revision
                || projection_bundle != item.projection_bundle
            {
                return Err(WorkspaceEngineError::InvalidInitialPlan(format!(
                    "active initial work item `{}` differs from the publication journal",
                    expected_logical.id
                )));
            }
            work_items.push(CompiledWorkItemRevision {
                draft_revision,
                work_item_revision,
                verification_plan_revision,
                projection_bundle,
            });
        }
        let contract_validation = validation_report.contract_validation.clone();
        let projection_validation = validation_report.projection_validation.clone();
        Ok(Some(InitialPlanCompileOutcome {
            plan_revision,
            dependency_graph_revision,
            validation_report,
            plan_projection_bundle,
            work_items,
            contract_validation,
            projection_validation,
        }))
    }
}
