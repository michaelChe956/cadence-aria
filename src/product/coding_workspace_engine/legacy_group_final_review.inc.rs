impl CodingWorkspaceEngine {
    // Legacy group review executor retained for regression coverage
    // (`group_review_runner`, `group_review_compatibility`, and `group_review_e2e`)
    // and legacy artifact reader support. No production callers remain after the
    // Task 6 recovery reroute. Remove only when legacy shard/reduction artifacts
    // no longer need regression coverage.
    #[allow(dead_code)]
    pub(crate) async fn execute_group_final_review_with_commands(
        &self,
        attempt: &CodingExecutionAttempt,
        provider: &dyn StreamingProviderAdapter,
        _command_rx: &mut mpsc::Receiver<CodingRunnerCommand>,
    ) -> Result<InternalPrReview, CodingWorkspaceEngineError> {
        use super::group_review_budget::GROUP_REVIEW_QUALITY_TARGET_BYTES;
        use super::group_review_errors::GroupReviewOrchestrationError;
        use super::group_review_material::compile_group_review_material;
        use super::group_review_orchestrator::{GroupReviewOrchestrator, RealGroupReviewExecutor};
        use super::group_review_prompts::GroupReviewPromptBuilder;

        let attempt = self.admit_provider_run(
            attempt,
            &CodingExecutionStage::InternalPrReview,
            "group_final_review",
        )?;
        if attempt.scope != CodingAttemptScope::WorkItemGroup {
            return Err(CodingWorkspaceEngineError::FinalConfirmNotReady(
                attempt.id.clone(),
            ));
        }
        let worktree_path = attempt
            .worktree_path
            .clone()
            .ok_or_else(|| CodingWorkspaceEngineError::MissingWorktree(attempt.id.clone()))?;
        let review_request_id = attempt
            .review_request_id
            .as_deref()
            .ok_or_else(|| CodingWorkspaceEngineError::MissingReviewRequest(attempt.id.clone()))?;
        let review_request = self.store.get_review_request(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            review_request_id,
        )?;
        let attempt = self.store.update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::InternalPrReview,
        )?;
        let node = self.create_internal_pr_review_timeline_node(&attempt)?;
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingTimelineNodeCreated { node: node.clone() })
            .await;
        let role_run = self.store.create_role_run(
            &attempt,
            CodingExecutionStage::InternalPrReview,
            CodingProviderRole::InternalReviewer,
            CodingRoleRunTrigger::Initial,
            Some(node.id.clone()),
        )?;
        let reviewer = match self.store.get_role_provider_config_snapshot(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
        ) {
            Ok(snapshot) => snapshot.internal_reviewer,
            Err(error) => {
                return self
                    .finalize_group_review_failure(
                        &attempt,
                        &node.id,
                        &role_run.id,
                        CodingWorkspaceEngineError::Store(error),
                    )
                    .await;
            }
        };
        // C2 Task 5（REQ-CRO-05）：internal reviewer 缺失（空 effective）fail-closed——
        // group final review 不以任何默认 reviewer 继续编排。
        let Some(reviewer) = reviewer else {
            return self
                .finalize_group_review_failure(
                    &attempt,
                    &node.id,
                    &role_run.id,
                    CodingWorkspaceEngineError::ReviewerConfigurationMissing {
                        attempt_id: attempt.id.clone(),
                        role: CodingProviderRole::InternalReviewer,
                    },
                )
                .await;
        };
        let executor = RealGroupReviewExecutor::new(
            self,
            attempt.clone(),
            provider,
            node.id.clone(),
            role_run.clone(),
            reviewer,
        );
        let orchestrator =
            GroupReviewOrchestrator::new(&executor, &self.store).with_role_run_id(&role_run.id);
        let bindings = match self.authoritative_group_reviewer_bindings(&attempt) {
            Ok(bindings) => bindings,
            Err(error) => {
                return self
                    .finalize_group_review_failure(&attempt, &node.id, &role_run.id, error)
                    .await;
            }
        };
        let mut snapshots = Vec::with_capacity(bindings.len());
        for binding in &bindings {
            let snapshot = match self
                .store
                .get_unit_review_conclusion_snapshot(&attempt.id, &binding.run.id)
            {
                Ok(Some(snapshot)) => snapshot,
                Ok(None) => {
                    return self
                        .handle_group_review_orchestration_failure(
                            &orchestrator,
                            &attempt,
                            &node.id,
                            &role_run.id,
                            GroupReviewOrchestrationError::IdentityMissing,
                        )
                        .await;
                }
                Err(error) => {
                    return self
                        .finalize_group_review_failure(
                            &attempt,
                            &node.id,
                            &role_run.id,
                            CodingWorkspaceEngineError::Store(error),
                        )
                        .await;
                }
            };
            snapshots.push(snapshot);
        }
        let git_facts = match self
            .collect_group_git_facts(&attempt, &bindings, &review_request, &worktree_path)
            .await
        {
            Ok(facts) => facts,
            Err(error @ CodingWorkspaceEngineError::CompletionCommitMissing(_)) => {
                return self
                    .handle_group_review_orchestration_failure(
                        &orchestrator,
                        &attempt,
                        &node.id,
                        &role_run.id,
                        GroupReviewOrchestrationError::IdentityMissing,
                    )
                    .await
                    .map_err(|mapped| match mapped {
                        CodingWorkspaceEngineError::GroupReviewBlocked { gate_id, .. } => {
                            CodingWorkspaceEngineError::GroupReviewBlocked {
                                reason_code: error.to_string(),
                                gate_id,
                            }
                        }
                        other => other,
                    });
            }
            Err(error) => {
                return self
                    .finalize_group_review_failure(&attempt, &node.id, &role_run.id, error)
                    .await;
            }
        };
        let snapshot = match compile_group_review_material(
            &bindings,
            &snapshots,
            &review_request,
            &git_facts,
            &GroupReviewPromptBuilder,
            GROUP_REVIEW_QUALITY_TARGET_BYTES,
        ) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                let orchestration_error = match error {
                    super::group_review_types::GroupMaterialError::AuthorityValidation(_) => {
                        return self
                            .handle_group_review_orchestration_failure(
                                &orchestrator,
                                &attempt,
                                &node.id,
                                &role_run.id,
                                GroupReviewOrchestrationError::IdentityMissing,
                            )
                            .await;
                    }
                    super::group_review_types::GroupMaterialError::GitFact(message) => {
                        CodingWorkspaceEngineError::GroupReviewGitFact(message)
                    }
                    super::group_review_types::GroupMaterialError::Internal(message) => {
                        CodingWorkspaceEngineError::GroupReviewMaterial(message)
                    }
                };
                return self
                    .finalize_group_review_failure(
                        &attempt,
                        &node.id,
                        &role_run.id,
                        orchestration_error,
                    )
                    .await;
            }
        };
        if let Err(error) = self
            .store
            .activate_group_review_snapshot(&attempt.id, &snapshot.content_hash)
        {
            return self
                .finalize_group_review_failure(
                    &attempt,
                    &node.id,
                    &role_run.id,
                    CodingWorkspaceEngineError::Store(error),
                )
                .await;
        }

        let shard_reports = match orchestrator.execute_shards(&snapshot).await {
            Ok(reports) => reports,
            Err(error) => {
                return self
                    .handle_group_review_orchestration_failure(
                        &orchestrator,
                        &attempt,
                        &node.id,
                        &role_run.id,
                        error,
                    )
                    .await;
            }
        };
        let reduction = match orchestrator
            .execute_reduction(&snapshot, &shard_reports, &[])
            .await
        {
            Ok(reduction) => reduction,
            Err(error) => {
                return self
                    .handle_group_review_orchestration_failure(
                        &orchestrator,
                        &attempt,
                        &node.id,
                        &role_run.id,
                        error,
                    )
                    .await;
            }
        };
        let completion_result = async {
            let raw_ref = reduction.raw_provider_output_refs.last().ok_or_else(|| {
                CodingWorkspaceEngineError::ProviderStream(
                    "group_review_reduction_raw_output_missing".to_string(),
                )
            })?;
            let review = self
                .store
                .list_internal_pr_reviews(&attempt.project_id, &attempt.issue_id, &attempt.id)?
                .into_iter()
                .find(|review| review.raw_provider_output_ref.as_deref() == Some(raw_ref.as_str()))
                .ok_or_else(|| {
                    CodingWorkspaceEngineError::ProviderStream(
                        "group_review_internal_review_missing".to_string(),
                    )
                })?;
            let raw_refs = shard_reports
                .iter()
                .flat_map(|report| report.raw_provider_output_refs.iter().cloned())
                .chain(reduction.raw_provider_output_refs.iter().cloned())
                .collect::<Vec<_>>();
            self.store.update_role_run_refs(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &role_run.id,
                raw_refs,
                vec![review.id.clone()],
            )?;
            self.store.update_role_run_status(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &role_run.id,
                CodingRoleRunStatus::Completed,
                None,
            )?;
            let node_status = match review.verdict {
                ReviewVerdict::Approve => CodingTimelineNodeStatus::Completed,
                ReviewVerdict::RequestChanges => CodingTimelineNodeStatus::Failed,
                ReviewVerdict::Blocked => CodingTimelineNodeStatus::Blocked,
            };
            self.complete_timeline_node(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &node.id,
                node_status,
                Some(review.summary.clone()),
            )
            .await?;
            Ok::<_, CodingWorkspaceEngineError>(review)
        }
        .await;
        let review = match completion_result {
            Ok(review) => review,
            Err(error) => {
                return self
                    .finalize_group_review_failure(&attempt, &node.id, &role_run.id, error)
                    .await;
            }
        };
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::InternalPrReviewComplete {
                review: Box::new(review.clone()),
            })
            .await;
        Ok(review)
    }
}
