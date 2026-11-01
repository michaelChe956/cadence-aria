use super::*;

pub(crate) async fn spawn_provider_run_from_event(
    run_context: ProviderRunContext,
    run_kind: ProviderRunKind,
    requested_node_id: Option<String>,
    outbound_tx: mpsc::Sender<OutboundControl>,
) -> Result<(), String> {
    if let Some(active_run) = run_context.manager.active_run().await {
        // F2（workspace_session_0018 现场）：去重判据必须含 kind——同节点不等于
        // 同请求。无附件 REST 反馈链的门修订 run 注册在仍开启的 human_confirm
        // 门节点上，而委托返修接力（WorkItemPlanSingleCandidateAuthor，emit 时
        // 活动节点仍是同一门节点）会命中纯 node_id 判据被静默 drain；followups
        // 同时按 phase=Generate 让位，无人驱动重跑，会话永久卡 running。
        let same_request = active_run.node_id == requested_node_id
            && std::mem::discriminant(&active_run.kind) == std::mem::discriminant(&run_kind);
        if same_request {
            tracing::debug!(
                session_id = %run_context.session_id,
                node_id = ?requested_node_id,
                "drained duplicate provider run request"
            );
            return Ok(());
        }
        // ReviewOnly 接力让位（claude×轻 rep1v11 session_0255 双 spawn 误杀终局）：
        // 两处 ReviewOnly 发射点（product/workspace_engine/single_candidate.rs 的
        // SC compile 完成路径与 conversational_gate.rs 的 SC 人工修订 turn 路径）
        // 均由仍在途的活跃 run 发出，且该 run 的任务体会经 followups 宏内联驱动
        // CrossReview 评审（`while stage == CrossReview`）——接力事件与在途 run 的
        // 自续驱动是同一逻辑 review 启动的两条执行路径。接力请求的 node_id
        // （ReviewerRun 节点）与在途 run 注册时的 node_id（author 节点）必然不同，
        // 同节点去重不命中；若放行到 from_handler，其无条件 supersede 会取消在途
        // run 的 token，杀死正在握手的 reviewer 会话（handshake failed: cancelled）
        // 并双驱动 review。故会话有活跃 run 在途时 drain 接力，让位自续。
        // 返修接力（WorkItemPlan* kind，由已退场/让位的 run 经策略路由委托）与
        // 用户显式路径（直调 from_handler）不在此收窄范围内，supersede 语义不变。
        if matches!(run_kind, ProviderRunKind::ReviewOnly) {
            tracing::debug!(
                session_id = %run_context.session_id,
                node_id = ?requested_node_id,
                "drained review relay; in-flight run owns review continuation"
            );
            return Ok(());
        }
    }

    let outcome = spawn_provider_run_from_handler(run_context.clone(), run_kind, outbound_tx).await;
    if let Err(message) = &outcome {
        // F2（REQ-WIGA-05，0018 现场）：无附件 relay spawn 失败/被拒此前只回丢弃
        // 通道（outbound 无订阅者）——错误既不回 REST 也不落 durable，委托态会话
        // 永久滞留 running。此处回执到 durable：委托态回落人工门（引擎守卫，
        // 非委托态零副作用）。
        let engine = run_context.engine.clone();
        let mut engine = engine.lock().await;
        engine.recover_delegated_author_rerun_failure(message).await;
    }
    outcome
}

/// P1 WIGA Task 5：auto 非 superseding 启动入口——manager 临界区内只认领
/// 空闲 session（活 run 存在返回 `Ok(false)` 只观察，不取消/不 supersede）。
pub(crate) async fn spawn_provider_run_claiming_idle(
    run_context: ProviderRunContext,
    run_kind: ProviderRunKind,
    outbound_tx: mpsc::Sender<OutboundControl>,
) -> Result<bool, String> {
    spawn_provider_run_with_start_mode(
        run_context,
        run_kind,
        outbound_tx,
        ProviderRunStartMode::ClaimIfIdle,
    )
    .await
}

/// manual WS 显式重跑入口：保留既有 supersede 语义（abort 活 run 后启动）。
pub(crate) async fn spawn_provider_run_from_handler(
    run_context: ProviderRunContext,
    run_kind: ProviderRunKind,
    outbound_tx: mpsc::Sender<OutboundControl>,
) -> Result<(), String> {
    spawn_provider_run_with_start_mode(
        run_context,
        run_kind,
        outbound_tx,
        ProviderRunStartMode::SupersedeFromAttachment,
    )
    .await
    .map(|_| ())
}

#[derive(Clone, Copy)]
pub(crate) enum ProviderRunStartMode {
    SupersedeFromAttachment,
    ClaimIfIdle,
}
