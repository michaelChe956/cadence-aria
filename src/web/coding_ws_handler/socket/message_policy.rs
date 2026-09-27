use super::*;

/// F-43：把该 attempt 当前未决 choice 逐帧补发给本次连接。
///
/// 读取面与快照同源（`build_coding_session_state` 读同一 choice-gate 目录），
/// 故此处读取失败只可能是「快照已失败/已给出空投影」的同一次故障，不再额外打断
/// 连接；仅写失败（连接已断）返回 false 交调用方收尾。
pub(super) async fn send_pending_choice_frames<S>(
    socket: &mut S,
    coding_store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
) -> bool
where
    S: Sink<Message> + Unpin,
{
    let Ok(frames) = pending_choice_frames(coding_store, attempt) else {
        return true;
    };
    for frame in &frames {
        if !send_coding_json(socket, frame).await {
            return false;
        }
    }
    true
}

/// F-43：交互应答（choice/permission）的投递通道。
///
/// 本 socket 持有 runner 句柄（StartCoding/RecoverCoding 由本连接启动）时逐字
/// 沿用既有通道，行为零变化；无句柄的连接（页面刷新/新开 tab）回落到注册表里该
/// attempt 的 runner 命令通道——runner 仍在等这份应答，否则作答被拒
/// （`coding_choice_runner_not_active`）或被静默丢弃，「卡在了、点了没反应」仍是
/// 死锁（与 `abort_attempt` 同源的 attempt 级路由）。
pub(super) fn interactive_runner_sender(
    state: &WebAppState,
    attempt_key: &CodingAttemptRunKey,
    local: Option<&mpsc::Sender<CodingRunnerCommand>>,
) -> Option<mpsc::Sender<CodingRunnerCommand>> {
    local
        .cloned()
        .or_else(|| state.coding_runs.command_sender(attempt_key))
}

pub fn is_coding_ws_message_allowed(
    status: &CodingAttemptStatus,
    stage: &CodingExecutionStage,
    message: &CodingWsInMessage,
) -> bool {
    if matches!(
        message,
        CodingWsInMessage::CodingHello { .. } | CodingWsInMessage::CodingPing
    ) {
        return true;
    }
    if matches!(
        status,
        CodingAttemptStatus::Completed | CodingAttemptStatus::Failed | CodingAttemptStatus::Aborted
    ) {
        // F-44：`Aborted`/`Failed` 是「可重新开始」的终态——只放行显式重开动作
        // RestartCoding（重走 admission CAS 回 Running 并重启 runner）；已完成
        // 的 attempt 不提供重开。StartCoding 等其余消息维持 F-14 fail-closed
        // 拒绝（终态不得被隐式唤醒，F-43 终态 group 拒绝面零回归）。
        return matches!(
            status,
            CodingAttemptStatus::Aborted | CodingAttemptStatus::Failed
        ) && matches!(message, CodingWsInMessage::RestartCoding);
    }
    if matches!(message, CodingWsInMessage::ContextNote { .. }) && status.is_active() {
        return true;
    }
    if matches!(message, CodingWsInMessage::StageGateConfirm { .. }) && status.is_active() {
        return true;
    }
    if matches!(message, CodingWsInMessage::ProviderSelect { .. }) && status.is_active() {
        return true;
    }
    if matches!(message, CodingWsInMessage::PermissionModeSelect { .. }) && status.is_active() {
        return true;
    }
    if matches!(message, CodingWsInMessage::GateResponse { .. })
        && *status == CodingAttemptStatus::WaitingForHuman
    {
        return true;
    }
    if *status == CodingAttemptStatus::Blocked {
        return matches!(
            message,
            CodingWsInMessage::GateResponse { .. } | CodingWsInMessage::AbortAttempt
        );
    }
    if *status == CodingAttemptStatus::AwaitingManualRecovery {
        // F-16：AbortAttempt（终态出口）之外仅放行显式恢复动作 RecoverCoding
        // （重走 admission CAS 回 Running + 重启 runner）；F-14 fail-closed
        // 白名单的其余收紧面不动。
        return matches!(
            message,
            CodingWsInMessage::AbortAttempt | CodingWsInMessage::RecoverCoding
        );
    }
    match stage {
        CodingExecutionStage::PrepareContext => matches!(
            message,
            CodingWsInMessage::ContextNote { .. }
                | CodingWsInMessage::StartCoding
                | CodingWsInMessage::ProviderSelect { .. }
                | CodingWsInMessage::PermissionModeSelect { .. }
                | CodingWsInMessage::MaxAutoReworkSelect { .. }
                | CodingWsInMessage::AbortAttempt
        ),
        CodingExecutionStage::WorktreePrepare => matches!(message, CodingWsInMessage::AbortAttempt),
        CodingExecutionStage::ReviewRequest => {
            matches!(
                message,
                CodingWsInMessage::StartCoding
                    | CodingWsInMessage::RetryPush
                    | CodingWsInMessage::AbortAttempt
            )
        }
        CodingExecutionStage::Coding
        | CodingExecutionStage::CodeReview
        | CodingExecutionStage::InternalPrReview => matches!(
            message,
            CodingWsInMessage::ContextNote { .. }
                | CodingWsInMessage::PermissionResponse { .. }
                | CodingWsInMessage::ChoiceResponse { .. }
                | CodingWsInMessage::AbortAttempt
        ),
        CodingExecutionStage::FinalConfirm => matches!(
            message,
            CodingWsInMessage::FinalConfirm
                | CodingWsInMessage::GateResponse { .. }
                | CodingWsInMessage::RetryPush
                | CodingWsInMessage::AbortAttempt
        ),
    }
}

/// P2 Task 5：人工 WS StartCoding 帧的一次性 command_id——随机合法相对
/// ID（uuid v4，过 `validate_relative_id`），每帧仅生成一次；durable claim
/// 以 command_id 为幂等键，同帧重试复用同一身份。
pub(super) fn manual_command_id_for_this_frame() -> String {
    uuid::Uuid::new_v4().to_string()
}
