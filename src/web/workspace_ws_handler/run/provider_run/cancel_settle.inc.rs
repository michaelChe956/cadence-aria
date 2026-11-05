// 从 provider_run.rs 主体拆出以满足 large_file_guard 的 1200 行上限（纯移动 + 取消
// 收口窗口收敛，无其它行为变化）。
//
// 取消观察与有界收口窗口：`run_cancel` 与 engine token 是同一个 token（见
// `use_run_token`），`provider_drive` 内建取消臂因此能自行收口
// （`finish_aborted_run`：节点 Failed + 回 PrepareContext + StageChange 帧）。
// 裸 `select!` 直接 drop drive 会把「谁先被 poll 到」变成终止契约的判据：取消臂
// 赢即零收口——会话停在 Running、时间线节点停在 Active、客户端收不到
// prepare_context（现场：ws abort 用例在 1b8e6670 基线上同样按此竞态复现）。
// 故取消后先给 drive 一个有界收口窗口，窗口内未收口（裸 await 悬停/真卡死）才
// 退回兜底路径。
//
// 窗口与调用面预算：接替 run 的 engine 锁回收口径是 1s 内有界（见
// provider_run_events 的 supersede_reclaims_engine_lock 用例），窗口必须显著小于
// 它；drive 侧收口只有本地 flush + durable 写 + 事件帧（毫秒级），250ms 留足余量。
#[allow(clippy::too_many_arguments)] // 收口上下文与 run 主体同源，逐个传入不再包一层。
pub(crate) async fn drive_until_cancelled_settles<F>(
    drive: F,
    run_cancel: &CancellationToken,
    cancelled_revision_turn_id: Option<String>,
    engine_for_run: &Arc<Mutex<crate::product::workspace_engine::WorkspaceEngine>>,
    outbound_tx: &mpsc::Sender<OutboundControl>,
    session_id: &str,
    run_label: &str,
    manager: &Arc<crate::web::workspace_session::WorkspaceSessionManager>,
    run_token: u64,
) where
    F: Future<Output = ()>,
{
    // S2 boxed 边界注：调用方（provider_run_task_body）传入的 drive 已是
    // `Pin<Box<provider_run_drive_body>>`——本 helper 的 pin/select/timeout
    // 只搬运 boxed 指针，不再把完整 drive 状态机放回栈上（设计 §3.1.3）。
    let drive_settled = {
        // pin 在本块内：未收口的 drive 随块结束被 drop，其持有的 engine 锁随之
        // 释放——兜底路径与接替 run 才能在预算内取锁。
        let mut drive = std::pin::pin!(drive);
        let cancelled_first = tokio::select! {
            _ = run_cancel.cancelled() => true,
            _ = drive.as_mut() => false,
        };
        !cancelled_first
            || tokio::time::timeout(PROVIDER_RUN_CANCELLED_SETTLE, drive.as_mut())
                .await
                .is_ok()
    };
    if drive_settled {
        // drive 正常返回：其尾部（followups + mark_active_run_finished +
        // finish_run）已收口；取消后自建取消臂收口同样走完整尾部。
        return;
    }
    // 蒸发链顺手项（取消臂回显化）：drive 在收口窗口内未完成——SC 修订 turn 会
    // 永停 Reserved/Running（后续 feedback 全被 gate_busy 卡死）且零回显。此处
    // 兜底：打点 + durable Failed(cancelled) + HumanGateTurnFailed 回显帧；刚自然
    // 收口的 turn 幂等跳过（见助手注释），等待/typed 恢复面不会被误 fail。
    eprintln!(
        "[aria-cancellation] workspace provider_run_cancelled_dropped_drive session_id={} run_label={} revision_turn={:?}",
        session_id, run_label, cancelled_revision_turn_id
    );
    if let Some(turn_id) = cancelled_revision_turn_id {
        fail_cancelled_human_gate_revision_turn(engine_for_run, outbound_tx, session_id, &turn_id)
            .await;
    }
    let _ = manager.finish_run(run_token).await;
}
