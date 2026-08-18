use super::*;
use crate::product::models::WorkItemRuntimeBinding;
use crate::web::workspace_context::ensure_workspace_context_message;
use crate::web::workspace_ws_types::{
    WorkItemHistoryEntryDto, WorkItemHistoryEntryKind, WorkItemRevisionHistoryDto,
};

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

        let plan_update = IssueWorkItemPlanUpdate {
            work_item_ids: work_item_ids.clone(),
            verification_plan_ids: verification_plan_ids.clone(),
            repository_profile_ref: tx.previous_plan_snapshot.repository_profile_ref.clone(),
            dependency_graph: outcome
                .dependency_graph_revision
                .edges
                .iter()
                .map(|edge| IssueWorkItemDependencyEdge {
                    from_work_item_id: edge.from.clone(),
                    to_work_item_id: edge.to.clone(),
                })
                .collect(),
            created_from_provider_run: tx.previous_plan_snapshot.created_from_provider_run.clone(),
            validator_findings: tx.validator_findings.clone(),
        };
        tx.step_cursor = "plan_summary_prepared".to_string();
        tx.updated_at = tx.created_at.clone();
        store
            .put_compile_transaction(tx)
            .map_err(|error| format!("save plan summary cursor failed: {error}"))?;
        #[cfg(test)]
        self.maybe_fail_work_item_plan_compile_finalizer(
            tx,
            WorkItemPlanCompileFinalizerCheckpoint::PlanSummaryPrepared,
        )?;

        let mut sessions = lifecycle
            .list_workspace_sessions(&tx.project_id, &tx.issue_id)
            .map_err(|error| format!("list child work item workspaces failed: {error}"))?;
        tx.child_session_ids.clear();
        for (index, logical_id) in work_item_ids.iter().enumerate() {
            let compiled = compiled_by_logical_id
                .get(logical_id.as_str())
                .ok_or_else(|| {
                    format!(
                        "compiled outcome missing logical work item `{logical_id}` during child workspace preparation"
                    )
                })?;
            let binding = WorkItemRuntimeBinding {
                plan_id: tx.plan_id.clone(),
                plan_revision_id: outcome.plan_revision.id.clone(),
                logical_work_item_id: logical_id.clone(),
                work_item_revision_id: compiled.work_item_revision.id.clone(),
                projection_bundle_id: compiled.projection_bundle.id.clone(),
                verification_plan_revision_id: compiled.verification_plan_revision.id.clone(),
                canonical_contract_hash: compiled
                    .work_item_revision
                    .canonical_contract_hash
                    .clone(),
                projection_compiler_version: compiled.projection_bundle.compiler_version.clone(),
                human_projection_hash: compiled.projection_bundle.human_projection_hash.clone(),
                coder_projection_hash: compiled.projection_bundle.coder_projection_hash.clone(),
                reviewer_projection_hash: compiled
                    .projection_bundle
                    .reviewer_projection_hash
                    .clone(),
            };
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
            tx.child_session_ids.push(session_id.clone());
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
            let bound_session = lifecycle
                .ensure_work_item_runtime_binding(&session_id, &binding)
                .map_err(|error| format!("ensure child runtime binding failed: {error}"))?;
            tx.step_cursor = format!("child_session_{:03}_binding_ensured", index + 1);
            tx.updated_at = tx.created_at.clone();
            store
                .put_compile_transaction(tx)
                .map_err(|error| format!("save child binding cursor failed: {error}"))?;
            #[cfg(test)]
            if index == 0 {
                self.maybe_fail_work_item_plan_compile_finalizer(
                    tx,
                    WorkItemPlanCompileFinalizerCheckpoint::FirstChildBindingEnsured,
                )?;
            }
            ensure_workspace_context_message(&lifecycle.app_paths(), lifecycle, bound_session)
                .await
                .map_err(|error| format!("ensure child workspace context failed: {error}"))?;
            tx.step_cursor = format!("child_session_{:03}_context_prepared", index + 1);
            tx.updated_at = tx.created_at.clone();
            store
                .put_compile_transaction(tx)
                .map_err(|error| format!("save child context cursor failed: {error}"))?;
            #[cfg(test)]
            if index == 0 {
                self.maybe_fail_work_item_plan_compile_finalizer(
                    tx,
                    WorkItemPlanCompileFinalizerCheckpoint::FirstChildContextPrepared,
                )?;
            }
        }
        tx.step_cursor = "child_workspaces_prepared".to_string();
        tx.updated_at = tx.created_at.clone();
        store
            .put_compile_transaction(tx)
            .map_err(|error| format!("save child sessions cursor failed: {error}"))?;

        lifecycle
            .commit_issue_work_item_plan(&tx.project_id, &tx.issue_id, &tx.plan_id, plan_update)
            .map_err(|error| format!("commit issue work item plan failed: {error}"))?;
        tx.step_cursor = "plan_confirmed".to_string();
        tx.updated_at = tx.created_at.clone();
        store
            .put_compile_transaction(tx)
            .map_err(|error| format!("save plan confirmed cursor failed: {error}"))?;

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
        let compile_report_update = self
            .persist_compile_report(compile_report, &tx.created_at)
            .await?;
        self.persist_initial_projection_artifacts(lifecycle, outcome, compile_report_update)
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
        compile_report: WorkItemPlanCompileReportPayload,
        created_at: &str,
    ) -> Result<ArtifactUpdateEvent, String> {
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
            self.session.artifact = Some(payload.clone());
            Ok(ArtifactUpdateEvent {
                version: self.artifact_versions[index].version,
                payload,
            })
        } else {
            let update = self.append_artifact_version_without_event(payload).await;
            let version = self.artifact_versions.last_mut().ok_or_else(|| {
                "compile report artifact version missing after update".to_string()
            })?;
            version.created_at = created_at.to_string();
            Ok(update)
        }
    }

    async fn persist_initial_projection_artifacts(
        &mut self,
        lifecycle: &LifecycleStore,
        outcome: &InitialPlanCompileOutcome,
        compile_report_update: ArtifactUpdateEvent,
    ) -> Result<(), String> {
        let mut updates = vec![compile_report_update];
        for item in &outcome.work_items {
            updates.push(
                self.ensure_initial_projection_artifact(ArtifactPayload::WorkItemProjection {
                    projection: Box::new(item.projection_bundle.clone()),
                })
                .await?,
            );
        }
        updates.push(
            self.ensure_initial_projection_artifact(ArtifactPayload::ProjectionValidation {
                report: Box::new(outcome.projection_validation.clone()),
            })
            .await?,
        );
        updates.push(
            self.ensure_initial_projection_artifact(ArtifactPayload::WorkItemRevisionHistory {
                history: Box::new(self.initial_revision_history(outcome)),
            })
            .await?,
        );
        let plan_payload = ArtifactPayload::WorkItemPlanProjection {
            projection: Box::new(outcome.plan_projection_bundle.clone()),
        };
        let plan_update = if let Some(index) = self
            .artifact_versions
            .iter()
            .position(|version| version.payload == plan_payload)
        {
            for version in &mut self.artifact_versions {
                version.is_current = false;
            }
            self.artifact_versions[index].is_current = true;
            self.session.artifact = Some(plan_payload.clone());
            ArtifactUpdateEvent {
                version: self.artifact_versions[index].version,
                payload: plan_payload,
            }
        } else {
            self.ensure_projection_identity_is_immutable(&plan_payload)?;
            self.append_artifact_version_without_event(plan_payload)
                .await
        };
        updates.push(plan_update);
        lifecycle
            .save_artifact_versions(&self.session.session_id, &self.artifact_versions)
            .map_err(|error| format!("persist initial projection artifacts failed: {error}"))?;
        let _ = self
            .event_tx
            .send(EngineEvent::ArtifactBatchUpdate { updates })
            .await;
        Ok(())
    }

    async fn ensure_initial_projection_artifact(
        &mut self,
        payload: ArtifactPayload,
    ) -> Result<ArtifactUpdateEvent, String> {
        if let Some(version) = self
            .artifact_versions
            .iter()
            .find(|version| version.payload == payload)
        {
            return Ok(ArtifactUpdateEvent {
                version: version.version,
                payload,
            });
        }
        self.ensure_projection_identity_is_immutable(&payload)?;
        Ok(self.append_artifact_version_without_event(payload).await)
    }

    async fn append_artifact_version_without_event(
        &mut self,
        payload: ArtifactPayload,
    ) -> ArtifactUpdateEvent {
        self.session.artifact = Some(payload.clone());
        for version in &mut self.artifact_versions {
            version.is_current = false;
        }
        let version = self.artifact_versions.len() as u32 + 1;
        let source_node_id = self
            .active_node_id
            .clone()
            .unwrap_or_else(|| "timeline_node_unknown".to_string());
        self.artifact_versions.push(ArtifactVersion {
            version,
            payload: payload.clone(),
            generated_by: self.session.author_provider.clone(),
            reviewed_by: None,
            review_verdict: None,
            confirmed_by: None,
            is_current: true,
            created_at: chrono::Utc::now().to_rfc3339(),
            source_node_id: source_node_id.clone(),
        });
        let _ = self
            .persist_artifact_ref(
                &source_node_id,
                ArtifactRef {
                    artifact_id: format!("artifact_version_{version:03}"),
                    version,
                },
            )
            .await;
        ArtifactUpdateEvent { version, payload }
    }

    fn ensure_projection_identity_is_immutable(
        &self,
        payload: &ArtifactPayload,
    ) -> Result<(), String> {
        for version in &self.artifact_versions {
            let identity_conflict = match (&version.payload, payload) {
                (
                    ArtifactPayload::WorkItemPlanProjection {
                        projection: existing,
                    },
                    ArtifactPayload::WorkItemPlanProjection { projection: next },
                ) => existing.id == next.id && existing != next,
                (
                    ArtifactPayload::WorkItemProjection {
                        projection: existing,
                    },
                    ArtifactPayload::WorkItemProjection { projection: next },
                ) => existing.id == next.id && existing != next,
                _ => false,
            };
            if identity_conflict {
                return Err("projection artifact identity mismatch".to_string());
            }
        }
        Ok(())
    }

    fn initial_revision_history(
        &self,
        outcome: &InitialPlanCompileOutcome,
    ) -> WorkItemRevisionHistoryDto {
        let mut entries = Vec::new();
        for item in &outcome.work_items {
            entries.push(WorkItemHistoryEntryDto {
                kind: WorkItemHistoryEntryKind::DraftRevision,
                id: item.draft_revision.id.clone(),
                logical_work_item_id: item.draft_revision.logical_work_item_id.clone(),
                related_revision_id: Some(item.work_item_revision.id.clone()),
                summary: format!("Draft revision {}", item.draft_revision.revision_no),
                created_at: item.draft_revision.created_at.clone(),
            });
            entries.push(WorkItemHistoryEntryDto {
                kind: WorkItemHistoryEntryKind::WorkItemRevision,
                id: item.work_item_revision.id.clone(),
                logical_work_item_id: item.work_item_revision.logical_work_item_id.clone(),
                related_revision_id: Some(item.work_item_revision.source_draft_revision_id.clone()),
                summary: format!(
                    "Compiled WorkItem revision from {}",
                    item.work_item_revision.source_draft_revision_id
                ),
                created_at: item.work_item_revision.created_at.clone(),
            });
        }
        for node in self.timeline_nodes.iter().filter(|node| {
            matches!(
                node.node_type,
                TimelineNodeType::WorkItemPlanOutlineReview
                    | TimelineNodeType::WorkItemDraftReview
                    | TimelineNodeType::WorkItemBatchReview
            )
        }) {
            let logical_work_item_id = self.artifact_versions.iter().find_map(|version| {
                if version.source_node_id != node.node_id {
                    return None;
                }
                match &version.payload {
                    ArtifactPayload::WorkItemDraftCandidate { draft_candidate } => Some(
                        draft_candidate
                            .draft_record
                            .candidate
                            .logical_work_item_id
                            .clone(),
                    ),
                    _ => None,
                }
            });
            let Some(logical_work_item_id) = logical_work_item_id else {
                continue;
            };
            let related_revision_id = outcome
                .work_items
                .iter()
                .find(|item| item.work_item_revision.logical_work_item_id == logical_work_item_id)
                .map(|item| item.work_item_revision.id.clone());
            entries.push(WorkItemHistoryEntryDto {
                kind: WorkItemHistoryEntryKind::PlanReview,
                id: node.node_id.clone(),
                logical_work_item_id,
                related_revision_id,
                summary: node.summary.clone().unwrap_or_else(|| node.title.clone()),
                created_at: node.started_at.clone(),
            });
        }
        entries.sort_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| left.id.cmp(&right.id))
        });
        WorkItemRevisionHistoryDto { entries }
    }
}
