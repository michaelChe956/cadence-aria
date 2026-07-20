use super::{PlanRepairEngine, PreparedPlanAmendment};
use crate::product::plan_repair::{PlanRepairError, load_plan_repair_candidate_package};

impl PlanRepairEngine {
    pub fn load_prepared_amendment(
        &self,
        candidate_package_artifact_id: &str,
    ) -> Result<PreparedPlanAmendment, PlanRepairError> {
        let candidate = load_plan_repair_candidate_package(
            &self.store,
            &self.plan,
            candidate_package_artifact_id,
        )?;
        let next_plan_revision = self
            .store
            .get_plan_revision(
                &self.plan.project_id,
                &self.plan.issue_id,
                &self.plan.id,
                &candidate.new_plan_revision_id,
            )
            .map_err(PlanRepairError::Store)?;
        let publication_ids = self
            .store
            .allocate_plan_amendment_publication_ids(
                &self.plan,
                &candidate.amendment_id,
                next_plan_revision.revision_no,
                &candidate
                    .minimum_manifest
                    .revised_work_items
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>(),
            )
            .map_err(PlanRepairError::Store)?;
        let mut draft_revisions = Vec::new();
        let mut revised_work_items = Vec::new();
        let mut verification_plan_revisions = Vec::new();
        let mut logical_work_items = Vec::new();
        for (logical_id, replacement) in &candidate.minimum_manifest.revised_work_items {
            let revision = self
                .store
                .get_work_item_revision(&self.plan, logical_id, &replacement.next_revision_id)
                .map_err(PlanRepairError::Store)?;
            draft_revisions.push(
                self.store
                    .get_draft_revision(&self.plan, &revision.source_draft_revision_id)
                    .map_err(PlanRepairError::Store)?,
            );
            verification_plan_revisions.push(
                self.store
                    .get_verification_plan_revision(
                        &self.plan,
                        &revision.verification_plan_revision_id,
                    )
                    .map_err(PlanRepairError::Store)?,
            );
            logical_work_items.push(
                self.store
                    .get_logical_work_item(&self.plan, logical_id)
                    .map_err(PlanRepairError::Store)?,
            );
            revised_work_items.push(revision);
        }
        let dependency_graph_revision = self
            .store
            .get_dependency_graph_revision(
                &self.plan,
                &next_plan_revision.dependency_graph_revision_id,
            )
            .map_err(PlanRepairError::Store)?;
        Ok(PreparedPlanAmendment {
            base_plan_revision_id: candidate.base_plan_revision_id.clone(),
            publication_ids,
            next_plan_revision,
            draft_revisions,
            revised_work_items,
            verification_plan_revisions,
            work_item_projection_bundles: candidate.work_item_projection_bundles.clone(),
            logical_work_items,
            dependency_graph_revision,
            plan_projection_bundle: candidate.plan_projection_bundle.clone(),
            validation_report: candidate.validation_report.clone(),
            contract_deltas: candidate.minimum_manifest.contract_deltas.clone(),
            impact_report: candidate.impact_report.clone(),
            manifest: candidate.minimum_manifest.clone(),
            candidate_package: candidate,
            subgraph_replan: None,
        })
    }
}
