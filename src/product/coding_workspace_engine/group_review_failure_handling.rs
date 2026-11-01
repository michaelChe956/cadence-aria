use super::group_review_errors::{GroupReviewExecutionError, GroupReviewOrchestrationError};
use super::group_review_orchestrator::GroupReviewOrchestrator;
use super::*;

// Legacy group review executor retained for regression coverage
// (`group_review_e2e` 及 orchestrator 直测；原 `group_review_runner` 已随
//  Task 12 删除)
// and legacy artifact reader support. No production callers remain after the
// Task 6 recovery reroute. Remove only when legacy shard/reduction artifacts
// no longer need regression coverage.
#[allow(dead_code)]
pub(crate) fn map_group_review_orchestration_error(
    error: GroupReviewOrchestrationError,
    gate_id: Option<String>,
) -> CodingWorkspaceEngineError {
    match error {
        GroupReviewOrchestrationError::CapacityExceeded => {
            CodingWorkspaceEngineError::GroupReviewBlocked {
                reason_code: "capacity_exceeded".to_string(),
                gate_id,
            }
        }
        GroupReviewOrchestrationError::MaterialOverflow { .. } => {
            CodingWorkspaceEngineError::GroupReviewBlocked {
                reason_code: "material_overflow".to_string(),
                gate_id,
            }
        }
        GroupReviewOrchestrationError::IdentityMissing => {
            CodingWorkspaceEngineError::GroupReviewBlocked {
                reason_code: "identity_missing".to_string(),
                gate_id,
            }
        }
        GroupReviewOrchestrationError::ShardOutputInvalid { .. } => {
            CodingWorkspaceEngineError::GroupReviewBlocked {
                reason_code: "shard_output_invalid".to_string(),
                gate_id,
            }
        }
        GroupReviewOrchestrationError::ReductionOutputInvalid { .. } => {
            CodingWorkspaceEngineError::GroupReviewBlocked {
                reason_code: "reduction_output_invalid".to_string(),
                gate_id,
            }
        }
        GroupReviewOrchestrationError::ShardTransportExhausted { .. } => {
            CodingWorkspaceEngineError::GroupReviewBlocked {
                reason_code: "shard_transport_exhausted".to_string(),
                gate_id,
            }
        }
        GroupReviewOrchestrationError::ReductionTransportExhausted => {
            CodingWorkspaceEngineError::GroupReviewBlocked {
                reason_code: "reduction_transport_exhausted".to_string(),
                gate_id,
            }
        }
        GroupReviewOrchestrationError::ShardInProgress { shard_id } => {
            CodingWorkspaceEngineError::GroupReviewBlocked {
                reason_code: format!("shard_in_progress:{shard_id}"),
                gate_id: None,
            }
        }
        GroupReviewOrchestrationError::ReductionInProgress => {
            CodingWorkspaceEngineError::GroupReviewBlocked {
                reason_code: "reduction_in_progress".to_string(),
                gate_id: None,
            }
        }
        GroupReviewOrchestrationError::ReductionNotReady => {
            CodingWorkspaceEngineError::GroupReviewBlocked {
                reason_code: "reduction_not_ready".to_string(),
                gate_id: None,
            }
        }
        GroupReviewOrchestrationError::ShardStaleAudit => {
            CodingWorkspaceEngineError::GroupReviewShardStaleAudit
        }
        GroupReviewOrchestrationError::ReductionStale => {
            CodingWorkspaceEngineError::GroupReviewReductionStale
        }
        GroupReviewOrchestrationError::Executor(GroupReviewExecutionError::UserCancelled) => {
            CodingWorkspaceEngineError::Aborted
        }
        GroupReviewOrchestrationError::Executor(GroupReviewExecutionError::Transport(message)) => {
            CodingWorkspaceEngineError::GroupReviewExecutorTransport(message)
        }
        GroupReviewOrchestrationError::Executor(GroupReviewExecutionError::ProviderProtocol(
            message,
        )) => CodingWorkspaceEngineError::GroupReviewExecutorInternal(message),
        GroupReviewOrchestrationError::Executor(GroupReviewExecutionError::Internal(message)) => {
            CodingWorkspaceEngineError::GroupReviewExecutorInternal(message)
        }
        // P4-b（REQ-PTR-IDLE）：internal reviewer 零活动挂起耗尽映射为可人工
        // 恢复的 Blocked 语义（stalled 门已由 create_failure_gate 落
        // code_review_provider_stalled），不进 Failed 收口。
        GroupReviewOrchestrationError::Executor(GroupReviewExecutionError::IdleStall {
            ..
        }) => CodingWorkspaceEngineError::GroupReviewBlocked {
            reason_code: CODE_REVIEW_PROVIDER_STALLED_REASON_CODE.to_string(),
            gate_id,
        },
        GroupReviewOrchestrationError::Store(error) => CodingWorkspaceEngineError::Store(error),
    }
}

// Legacy group review executor retained for regression coverage
// (`group_review_e2e` 及 orchestrator 直测；原 `group_review_runner` 已随
//  Task 12 删除)
// and legacy artifact reader support. No production callers remain after the
// Task 6 recovery reroute. Remove only when legacy shard/reduction artifacts
// no longer need regression coverage.
#[allow(dead_code)]
#[derive(Clone, Copy)]
enum GroupReviewFailureDisposition {
    Blocked,
    Failed,
    Aborted,
    StaleAudit,
}

// Legacy group review executor retained for regression coverage
// (`group_review_e2e` 及 orchestrator 直测；原 `group_review_runner` 已随
//  Task 12 删除)
// and legacy artifact reader support. No production callers remain after the
// Task 6 recovery reroute. Remove only when legacy shard/reduction artifacts
// no longer need regression coverage.
#[allow(dead_code)]
fn group_review_failure_disposition(
    error: &CodingWorkspaceEngineError,
) -> GroupReviewFailureDisposition {
    match error {
        CodingWorkspaceEngineError::GroupReviewBlocked { .. }
        | CodingWorkspaceEngineError::CompletionCommitMissing(_) => {
            GroupReviewFailureDisposition::Blocked
        }
        CodingWorkspaceEngineError::Aborted => GroupReviewFailureDisposition::Aborted,
        CodingWorkspaceEngineError::GroupReviewShardStaleAudit
        | CodingWorkspaceEngineError::GroupReviewReductionStale => {
            GroupReviewFailureDisposition::StaleAudit
        }
        _ => GroupReviewFailureDisposition::Failed,
    }
}

impl CodingWorkspaceEngine {
    // Legacy group review executor retained for regression coverage
    // (`group_review_e2e` 及 orchestrator 直测；原 `group_review_runner` 已随
    //  Task 12 删除)
    // and legacy artifact reader support. No production callers remain after the
    // Task 6 recovery reroute. Remove only when legacy shard/reduction artifacts
    // no longer need regression coverage.
    #[allow(dead_code)]
    pub(crate) async fn finalize_group_review_failure(
        &self,
        attempt: &CodingExecutionAttempt,
        node_id: &str,
        role_run_id: &str,
        error: CodingWorkspaceEngineError,
        review_window: Option<&crate::product::reviewer_window::ReviewerWindowHandle>,
    ) -> Result<InternalPrReview, CodingWorkspaceEngineError> {
        // 批2 Task 20b(决策项⑧):group 审查窗在失败收尾前技术收口
        // (锁保持到恢复完成;收口错误只 warn 不掩盖原始错误)。
        if let Some(window) = review_window {
            self.close_reviewer_window_after_failure(window, "group_review_failed")
                .await;
        }
        let message = error.to_string();
        match group_review_failure_disposition(&error) {
            GroupReviewFailureDisposition::Blocked => {
                self.store.update_attempt_status(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &attempt.id,
                    CodingAttemptStatus::Blocked,
                )?;
                self.store.update_role_run_status(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &attempt.id,
                    role_run_id,
                    CodingRoleRunStatus::Blocked,
                    Some(message.clone()),
                )?;
                self.complete_timeline_node(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &attempt.id,
                    node_id,
                    CodingTimelineNodeStatus::Blocked,
                    Some(message),
                )
                .await?;
            }
            GroupReviewFailureDisposition::Failed => {
                self.store.update_role_run_status(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &attempt.id,
                    role_run_id,
                    CodingRoleRunStatus::Failed,
                    Some(message.clone()),
                )?;
                self.complete_timeline_node(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &attempt.id,
                    node_id,
                    CodingTimelineNodeStatus::Failed,
                    Some(message),
                )
                .await?;
                self.handle_attempt_failed(&attempt.project_id, &attempt.issue_id, &attempt.id)
                    .await?;
            }
            GroupReviewFailureDisposition::StaleAudit => {}
            GroupReviewFailureDisposition::Aborted => {
                self.store.update_role_run_status(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &attempt.id,
                    role_run_id,
                    CodingRoleRunStatus::Aborted,
                    Some("abort_attempt".to_string()),
                )?;
                self.complete_timeline_node(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &attempt.id,
                    node_id,
                    CodingTimelineNodeStatus::Failed,
                    Some(message),
                )
                .await?;
                self.store.update_attempt_status(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &attempt.id,
                    CodingAttemptStatus::Aborted,
                )?;
            }
        }
        Err(error)
    }

    // Legacy group review executor retained for regression coverage
    // (`group_review_e2e` 及 orchestrator 直测；原 `group_review_runner` 已随
    //  Task 12 删除)
    // and legacy artifact reader support. No production callers remain after the
    // Task 6 recovery reroute. Remove only when legacy shard/reduction artifacts
    // no longer need regression coverage.
    #[allow(dead_code)]
    pub(crate) async fn handle_group_review_orchestration_failure(
        &self,
        orchestrator: &GroupReviewOrchestrator<'_>,
        attempt: &CodingExecutionAttempt,
        node_id: &str,
        role_run_id: &str,
        error: GroupReviewOrchestrationError,
        review_window: Option<&crate::product::reviewer_window::ReviewerWindowHandle>,
    ) -> Result<InternalPrReview, CodingWorkspaceEngineError> {
        let gate_id = match &error {
            GroupReviewOrchestrationError::CapacityExceeded
            | GroupReviewOrchestrationError::MaterialOverflow { .. }
            | GroupReviewOrchestrationError::IdentityMissing
            | GroupReviewOrchestrationError::ShardOutputInvalid { .. }
            | GroupReviewOrchestrationError::ReductionOutputInvalid { .. }
            | GroupReviewOrchestrationError::ShardTransportExhausted { .. }
            | GroupReviewOrchestrationError::ReductionTransportExhausted
            | GroupReviewOrchestrationError::Executor(GroupReviewExecutionError::IdleStall {
                ..
            }) => match orchestrator.create_failure_gate(attempt, node_id, &error) {
                Ok(gate) => Some(gate.gate_id),
                Err(store_error) => {
                    return self
                        .finalize_group_review_failure(
                            attempt,
                            node_id,
                            role_run_id,
                            CodingWorkspaceEngineError::Store(store_error),
                            review_window,
                        )
                        .await;
                }
            },
            GroupReviewOrchestrationError::ShardInProgress { .. }
            | GroupReviewOrchestrationError::ReductionInProgress
            | GroupReviewOrchestrationError::ReductionNotReady
            | GroupReviewOrchestrationError::ReductionStale
            | GroupReviewOrchestrationError::ShardStaleAudit
            | GroupReviewOrchestrationError::Store(_)
            | GroupReviewOrchestrationError::Executor(_) => None,
        };
        let mapped = map_group_review_orchestration_error(error, gate_id);
        self.finalize_group_review_failure(attempt, node_id, role_run_id, mapped, review_window)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::json_store::ProductStoreError;

    #[test]
    fn group_review_orchestration_error_mapping_preserves_semantics_and_gate_identity() {
        let blocked = map_group_review_orchestration_error(
            GroupReviewOrchestrationError::CapacityExceeded,
            Some("coding_blocked_gate_0001".to_string()),
        );
        assert!(matches!(
            blocked,
            CodingWorkspaceEngineError::GroupReviewBlocked {
                ref reason_code,
                gate_id: Some(ref gate_id),
            } if reason_code == "capacity_exceeded" && gate_id == "coding_blocked_gate_0001"
        ));

        let exhausted = map_group_review_orchestration_error(
            GroupReviewOrchestrationError::ShardTransportExhausted {
                shard_id: "shard_0001".to_string(),
            },
            Some("coding_blocked_gate_0002".to_string()),
        );
        assert!(matches!(
            exhausted,
            CodingWorkspaceEngineError::GroupReviewBlocked {
                ref reason_code,
                gate_id: Some(ref gate_id),
            } if reason_code == "shard_transport_exhausted" && gate_id == "coding_blocked_gate_0002"
        ));

        let in_progress = map_group_review_orchestration_error(
            GroupReviewOrchestrationError::ShardInProgress {
                shard_id: "shard_0001".to_string(),
            },
            None,
        );
        assert!(matches!(
            in_progress,
            CodingWorkspaceEngineError::GroupReviewBlocked {
                ref reason_code,
                gate_id: None,
            } if reason_code == "shard_in_progress:shard_0001"
        ));

        assert!(matches!(
            map_group_review_orchestration_error(
                GroupReviewOrchestrationError::ReductionStale,
                None,
            ),
            CodingWorkspaceEngineError::GroupReviewReductionStale
        ));
        assert!(matches!(
            map_group_review_orchestration_error(
                GroupReviewOrchestrationError::Executor(GroupReviewExecutionError::Transport(
                    "network unavailable".to_string(),
                )),
                None,
            ),
            CodingWorkspaceEngineError::GroupReviewExecutorTransport(ref message)
                if message == "network unavailable"
        ));
        assert!(matches!(
            map_group_review_orchestration_error(
                GroupReviewOrchestrationError::Store(ProductStoreError::Io(
                    "disk unavailable".to_string(),
                )),
                None,
            ),
            CodingWorkspaceEngineError::Store(ProductStoreError::Io(ref message))
                if message == "disk unavailable"
        ));
    }
}
