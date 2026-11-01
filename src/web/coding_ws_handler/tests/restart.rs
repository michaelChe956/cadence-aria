use super::*;

#[test]
fn terminal_attempt_allows_only_explicit_restart_message() {
    // F-44：Aborted/Failed 只放行显式重开动作 RestartCoding（任一 stage）。
    for stage in [
        CodingExecutionStage::PrepareContext,
        CodingExecutionStage::WorktreePrepare,
        CodingExecutionStage::Coding,
        CodingExecutionStage::CodeReview,
        CodingExecutionStage::ReviewRequest,
        CodingExecutionStage::InternalPrReview,
        CodingExecutionStage::FinalConfirm,
    ] {
        for status in [CodingAttemptStatus::Aborted, CodingAttemptStatus::Failed] {
            assert!(
                is_coding_ws_message_allowed(&status, &stage, &CodingWsInMessage::RestartCoding),
                "终态 {status:?} 必须放行显式重新开始（stage={stage:?}）"
            );
            // 终态不得被隐式唤醒：StartCoding 等既有消息维持 F-14 fail-closed。
            assert!(
                !is_coding_ws_message_allowed(&status, &stage, &CodingWsInMessage::StartCoding),
                "终态 {status:?} 不得放行 StartCoding（stage={stage:?}）"
            );
            assert!(!is_coding_ws_message_allowed(
                &status,
                &stage,
                &CodingWsInMessage::ContextNote {
                    content: "retry".to_string(),
                },
            ));
        }
    }
    // 已完成的 attempt 不提供重开。
    assert!(!is_coding_ws_message_allowed(
        &CodingAttemptStatus::Completed,
        &CodingExecutionStage::FinalConfirm,
        &CodingWsInMessage::RestartCoding,
    ));
    // 非终态一律拒绝（重开通道是终态专属，不得成为跳过阶段门的旁路）。
    for status in [
        CodingAttemptStatus::Created,
        CodingAttemptStatus::Running,
        CodingAttemptStatus::WaitingForHuman,
        CodingAttemptStatus::Blocked,
        CodingAttemptStatus::AwaitingManualRecovery,
        CodingAttemptStatus::AwaitingPlanAmendment,
        CodingAttemptStatus::ApplyingPlanAmendment,
        CodingAttemptStatus::AmendmentApplyFailed,
    ] {
        assert!(
            !is_coding_ws_message_allowed(
                &status,
                &CodingExecutionStage::Coding,
                &CodingWsInMessage::RestartCoding
            ),
            "非终态 {status:?} 不得放行 RestartCoding"
        );
    }
}
