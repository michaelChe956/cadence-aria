use super::*;

impl WorkspaceEngine {
    pub(super) fn resume_initial_plan_compile_transaction(
        &mut self,
        store: &WorkItemPlanStore,
        tx: &mut WorkItemPlanCompileTransaction,
    ) -> Result<InitialPlanCompileOutcome, String> {
        let index = store
            .load_active_index(&tx.project_id, &tx.issue_id, &tx.plan_id)
            .map_err(|error| format!("load work item plan active index failed: {error}"))?
            .ok_or_else(|| "work item plan active index missing".to_string())?;
        if index.current_generation_round_id != tx.generation_round_id {
            return Err("compile recovery generation round changed".to_string());
        }
        let outline_candidate = self.latest_work_item_plan_outline_candidate()?;
        if outline_candidate.outline.id != tx.outline_version_ref {
            return Err("compile recovery outline version changed".to_string());
        }
        let outline_order = work_item_plan_outline_topological_order(&outline_candidate.outline)?;
        let draft_records =
            self.accepted_active_draft_records_for_compile(store, &index, &outline_order)?;
        let active_draft_ids = draft_records
            .iter()
            .map(|record| record.draft_id.clone())
            .collect::<Vec<_>>();
        if active_draft_ids != tx.active_draft_ids {
            return Err("compile recovery active draft bindings changed".to_string());
        }
        let accepted_drafts = draft_records
            .iter()
            .map(work_item_draft_revision_from_record)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        let outcome = self
            .compile_initial_plan_revision(&accepted_drafts)
            .map_err(|error| error.to_string())?;
        let compiled_by_logical_id = outcome
            .work_items
            .iter()
            .map(|item| (item.work_item_revision.logical_work_item_id.as_str(), item))
            .collect::<BTreeMap<_, _>>();
        tx.outline_to_work_item_id = draft_records
            .iter()
            .map(|draft| {
                (
                    draft.outline_id.clone(),
                    draft.candidate.logical_work_item_id.clone(),
                )
            })
            .collect();
        tx.outline_to_verification_plan_id = draft_records
            .iter()
            .map(|draft| {
                let compiled = compiled_by_logical_id
                    .get(draft.candidate.logical_work_item_id.as_str())
                    .ok_or_else(|| {
                        format!(
                            "compiled outcome missing `{}` during recovery",
                            draft.candidate.logical_work_item_id
                        )
                    })?;
                Ok((
                    draft.outline_id.clone(),
                    compiled.verification_plan_revision.id.clone(),
                ))
            })
            .collect::<Result<BTreeMap<_, _>, String>>()?;
        tx.step_cursor = "publication_resumed".to_string();
        tx.updated_at = tx.created_at.clone();
        store
            .put_compile_transaction(tx)
            .map_err(|error| format!("save resumed publication cursor failed: {error}"))?;
        Ok(outcome)
    }

    pub(super) async fn finalize_initial_plan_compile(
        &mut self,
        lifecycle: &LifecycleStore,
        store: &WorkItemPlanStore,
        tx: &mut WorkItemPlanCompileTransaction,
        outcome: &InitialPlanCompileOutcome,
    ) -> Result<(), String> {
        let work_item_ids = outcome
            .plan_projection_bundle
            .coder_group_context
            .ordered_logical_work_item_ids
            .clone();
        let compiled_by_logical_id = outcome
            .work_items
            .iter()
            .map(|item| (item.work_item_revision.logical_work_item_id.as_str(), item))
            .collect::<BTreeMap<_, _>>();
        let verification_plan_ids = work_item_ids
            .iter()
            .map(|logical_id| {
                compiled_by_logical_id
                    .get(logical_id.as_str())
                    .map(|item| item.verification_plan_revision.id.clone())
                    .ok_or_else(|| {
                        format!(
                            "compiled outcome missing logical work item `{logical_id}` during finalization"
                        )
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;

        lifecycle
            .commit_issue_work_item_plan(
                &tx.project_id,
                &tx.issue_id,
                &tx.plan_id,
                IssueWorkItemPlanUpdate {
                    work_item_ids: work_item_ids.clone(),
                    verification_plan_ids: verification_plan_ids.clone(),
                    repository_profile_ref: tx
                        .previous_plan_snapshot
                        .repository_profile_ref
                        .clone(),
                    dependency_graph: outcome
                        .dependency_graph_revision
                        .edges
                        .iter()
                        .map(|edge| IssueWorkItemDependencyEdge {
                            from_work_item_id: edge.from.clone(),
                            to_work_item_id: edge.to.clone(),
                        })
                        .collect(),
                    created_from_provider_run: tx
                        .previous_plan_snapshot
                        .created_from_provider_run
                        .clone(),
                    validator_findings: tx.validator_findings.clone(),
                },
            )
            .map_err(|error| format!("commit issue work item plan failed: {error}"))?;
        tx.step_cursor = "plan_summary_committed".to_string();
        tx.updated_at = tx.created_at.clone();
        store
            .put_compile_transaction(tx)
            .map_err(|error| format!("save plan summary cursor failed: {error}"))?;
        #[cfg(test)]
        self.maybe_fail_work_item_plan_compile_finalizer(
            tx,
            WorkItemPlanCompileFinalizerCheckpoint::PlanSummaryCommitted,
        )?;

        let mut sessions = lifecycle
            .list_workspace_sessions(&tx.project_id, &tx.issue_id)
            .map_err(|error| format!("list child work item workspaces failed: {error}"))?;
        tx.child_session_ids.clear();
        for (index, logical_id) in work_item_ids.iter().enumerate() {
            let matched = sessions
                .iter()
                .filter(|session| {
                    session.workspace_type == WorkspaceType::WorkItem
                        && session.entity_id == *logical_id
                })
                .collect::<Vec<_>>();
            let session_id = match matched.as_slice() {
                [existing] => existing.id.clone(),
                [] => {
                    let created = lifecycle
                        .create_workspace_session(CreateWorkspaceSessionInput {
                            project_id: tx.project_id.clone(),
                            issue_id: tx.issue_id.clone(),
                            entity_id: logical_id.clone(),
                            workspace_type: WorkspaceType::WorkItem,
                            author_provider: self.session.author_provider.clone(),
                            reviewer_provider: self
                                .session
                                .reviewer_provider
                                .clone()
                                .unwrap_or(ProviderName::Codex),
                            review_rounds: self.session.review_rounds,
                            superpowers_enabled: self.session.superpowers_enabled,
                            openspec_enabled: self.session.openspec_enabled,
                        })
                        .map_err(|error| {
                            format!("create child work item workspace failed: {error}")
                        })?;
                    let id = created.id.clone();
                    sessions.push(created);
                    id
                }
                _ => {
                    return Err(format!(
                        "expected exactly one child work item workspace for `{logical_id}`, found {}",
                        matched.len()
                    ));
                }
            };
            tx.child_session_ids.push(session_id);
            tx.step_cursor = format!("child_session_{:03}_ensured", index + 1);
            tx.updated_at = tx.created_at.clone();
            store
                .put_compile_transaction(tx)
                .map_err(|error| format!("save child session cursor failed: {error}"))?;
            #[cfg(test)]
            if index == 0 {
                self.maybe_fail_work_item_plan_compile_finalizer(
                    tx,
                    WorkItemPlanCompileFinalizerCheckpoint::FirstChildSessionEnsured,
                )?;
            }
        }
        tx.step_cursor = "child_sessions_ensured".to_string();
        tx.updated_at = tx.created_at.clone();
        store
            .put_compile_transaction(tx)
            .map_err(|error| format!("save child sessions cursor failed: {error}"))?;

        let compile_report = WorkItemPlanCompileReportPayload {
            compile_id: tx.compile_id.clone(),
            generation_round_id: tx.generation_round_id.clone(),
            status: WorkItemPlanCompileStatus::Committed,
            plan_commit_state: WorkItemPlanCommitState::Committed,
            work_item_ids,
            verification_plan_ids,
            child_session_ids: tx.child_session_ids.clone(),
            validator_findings: work_item_split_findings_to_dto(&tx.validator_findings),
        };
        self.persist_compile_report(lifecycle, compile_report, &tx.created_at)
            .await?;
        tx.step_cursor = "compile_report_persisted".to_string();
        tx.updated_at = tx.created_at.clone();
        store
            .put_compile_transaction(tx)
            .map_err(|error| format!("save compile report cursor failed: {error}"))?;
        #[cfg(test)]
        self.maybe_fail_work_item_plan_compile_finalizer(
            tx,
            WorkItemPlanCompileFinalizerCheckpoint::CompileReportPersisted,
        )?;

        tx.status = WorkItemPlanCompileStatus::Committed;
        tx.plan_commit_state = WorkItemPlanCommitState::Committed;
        tx.committed_at = Some(tx.created_at.clone());
        tx.failure_reason = None;
        tx.step_cursor = "committed".to_string();
        tx.updated_at = tx.created_at.clone();
        store
            .put_compile_transaction(tx)
            .map_err(|error| format!("save compile committed marker failed: {error}"))
    }

    async fn persist_compile_report(
        &mut self,
        lifecycle: &LifecycleStore,
        compile_report: WorkItemPlanCompileReportPayload,
        created_at: &str,
    ) -> Result<(), String> {
        let payload = ArtifactPayload::WorkItemPlanCompileReport {
            compile_report: Box::new(compile_report.clone()),
        };
        let matching = self
            .artifact_versions
            .iter()
            .enumerate()
            .filter(|(_, version)| {
                matches!(
                    &version.payload,
                    ArtifactPayload::WorkItemPlanCompileReport { compile_report: existing }
                        if existing.compile_id == compile_report.compile_id
                )
            })
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        if matching.len() > 1 {
            return Err(format!(
                "duplicate compile reports for `{}`",
                compile_report.compile_id
            ));
        }
        if let Some(index) = matching.first().copied() {
            if self.artifact_versions[index].payload != payload {
                return Err(format!(
                    "compile report identity mismatch for `{}`",
                    compile_report.compile_id
                ));
            }
            self.artifact_versions[index].created_at = created_at.to_string();
            self.session.artifact = Some(payload);
        } else {
            self.update_artifact(payload).await;
            let version = self.artifact_versions.last_mut().ok_or_else(|| {
                "compile report artifact version missing after update".to_string()
            })?;
            version.created_at = created_at.to_string();
        }
        lifecycle
            .save_artifact_versions(&self.session.session_id, &self.artifact_versions)
            .map_err(|error| format!("persist compile report artifact failed: {error}"))
    }
}
