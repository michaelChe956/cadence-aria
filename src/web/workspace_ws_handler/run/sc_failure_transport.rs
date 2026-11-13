//! SC 教学重驱 transport（P01 纯搬迁切片）。
//!
//! 实现自 `run/single_candidate.rs` 整体迁出：函数签名、函数体、归一化与
//! 取消/AlreadyFinished/close/finalize 时序全部保持原样。归一化 helper 与
//! 两个归一化事件发射器仍由 `single_candidate_compile_preflight` 原样提供，
//! 本模块只调用不复制其实现。

use super::single_candidate_compile_preflight::{
    emit_author_ears_normalized_event, emit_author_heading_normalized_event,
    prepare_author_delivery_for_compile,
};
use super::*;

/// F2-B/3.6：教学重驱共用驱动——同 node 再驱一次 provider，返回归一化后的
/// markdown source。驱动中断（会话失败/取消）时已按既有终态形态持久化 Failed
/// 并返回 AlreadyFinished，与首轮驱动失败处理一致。
#[allow(clippy::too_many_arguments)]
#[allow(clippy::result_large_err)] // 上述枚举是运行收尾的唯一 typed 出口。
pub(in crate::web::workspace_ws_handler::run) async fn drive_single_candidate_reredrive(
    engine: &mut WorkspaceEngine,
    launch: &crate::web::workspace_ws_handler::run::gateway_start::PlanAuthorLaunch,
    provider_for_run: Arc<dyn StreamingProviderAdapter>,
    run_cancel: &CancellationToken,
    command_rx: &mut mpsc::Receiver<ProviderCommand>,
    node_id: &str,
    author_provider: &crate::product::models::ProviderName,
    reredrive_prompt: &str,
    repository_path: &std::path::Path,
    // Task 1b 段②生产臂:重驱是独立 provider run,调用方每轮 begin 新
    // handle 后传入。
    plan_split_run: Option<&crate::web::workspace_ws_handler::run::PlanSplitRunContext>,
    // Task 4（REQ-WSC-28C）：教学 attempt 引用（claim 侧 Task 6 接线）。
    sc_teaching_attempt: Option<&crate::product::work_item_plan_policy::ScTeachingAttemptRef>,
) -> Result<String, SingleCandidateProviderRunError> {
    let reredrive_input = engine
        .build_work_item_plan_streaming_input(
            crate::product::work_item_split_engine::types::provider_name_to_type(author_provider),
            reredrive_prompt.to_string(),
            repository_path.to_string_lossy().to_string(),
            author_provider.clone(),
            PlanAuthorOutputContract::Structured,
        )
        .map_err(SingleCandidateProviderRunError::Message)?;
    let reredrive_input = engine.attach_tool_policy_audit(reredrive_input);
    // T6 fix-loop r2：trust 分类已下沉 start_work_item_plan_author 的
    // Logical 分支共享口（教学重驱同臂）。
    let reredrive_session = start_work_item_plan_author(
        launch.clone(),
        Arc::clone(&provider_for_run),
        reredrive_input,
        run_cancel.clone(),
        plan_split_run,
    )
    .await;
    // Task 4（REQ-WSC-28C）：spawn 确认后、事件循环前同步回填（fail-closed）。
    let reredrive_session =
        engine.backfill_sc_teaching_launch(sc_teaching_attempt, reredrive_session);
    let reredrive_output = match engine
        .drive_work_item_plan_provider_session_to_output(
            reredrive_session,
            command_rx,
            node_id.to_string(),
            author_provider.clone(),
        )
        .await
    {
        Ok(output) => output,
        Err(message) => {
            crate::web::workspace_ws_handler::run::fail_plan_split_run(
                plan_split_run,
                &format!("provider session drive failed: {message}"),
            );
            engine.persist_single_candidate_terminal_phase(
                crate::product::models::SingleCandidatePhase::Failed,
            );
            return Err(SingleCandidateProviderRunError::AlreadyFinished);
        }
    };
    crate::web::workspace_ws_handler::run::close_plan_split_run_with_markdown(
        plan_split_run,
        reredrive_prompt,
        &reredrive_output,
    );
    let reredrive_delivery = prepare_author_delivery_for_compile(&reredrive_output);
    emit_author_heading_normalized_event(
        engine,
        node_id,
        &reredrive_output,
        &reredrive_delivery,
        author_provider,
    )
    .await;
    emit_author_ears_normalized_event(
        engine,
        node_id,
        &reredrive_output,
        &reredrive_delivery,
        author_provider,
    )
    .await;
    Ok(reredrive_delivery.source)
}
