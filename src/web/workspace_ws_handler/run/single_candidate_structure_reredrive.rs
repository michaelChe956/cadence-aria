//! SG3a.1 作者阶段结构门拒收的「恰一次教学重驱」驱动臂（REQ-SG-02/03、
//! REQ-WSC-18/25）。
//!
//! 从 `run/single_candidate.rs` 拆出以守住 large_file_guard 登记上限：本文件
//! 只做「修复指令 prompt 的可观测呈现 + 复用既有重驱通道」——事件发射与
//! provider 驱动和 compile/IR 教学重驱同一形状（同 node 再驱一次 provider），
//! **不新造第二个 provider attempt 通道**；槽位（结构门类**独立槽**
//! `structure_rerun_used`）由调用方占用，与 compile/IR 槽互不挤占（每类
//! 至多一次）。

use super::gateway_start::PlanAuthorLaunch;
use super::single_candidate::SingleCandidateProviderRunError;
use super::*;

/// 结构门拒收的首轮失败描述（终态错误文案用；只述事实，不虚构原因）。
pub(super) fn structure_gate_first_round_failure() -> String {
    format!(
        "candidate structure gate rejected ({})",
        crate::product::workspace_engine::single_candidate_structure_failure::STRUCTURE_STABLE_CODE
    )
}

/// 结构门拒收教学重驱：发一条可观测的 prompt 事件（与 compile/IR 重驱的
/// 事件同形），随后复用 `drive_single_candidate_reredrive` 同一 provider 通道
/// 再驱一次作者，返回归一化后的 markdown source——调用方回编译循环重走
/// compile → IR 预校验 → 候选结构门。
#[allow(clippy::too_many_arguments)]
#[allow(clippy::result_large_err)] // 同上：该枚举是运行收尾的唯一 typed 出口。
pub(super) async fn drive_structure_gate_reredrive(
    engine: &mut WorkspaceEngine,
    lifecycle: &LifecycleStore,
    launch: &PlanAuthorLaunch,
    provider_for_run: Arc<dyn StreamingProviderAdapter>,
    run_cancel: &CancellationToken,
    command_rx: &mut mpsc::Receiver<ProviderCommand>,
    node_id: &str,
    author_provider: &ProviderName,
    repair_prompt: &str,
    repository_path: &std::path::Path,
) -> Result<String, SingleCandidateProviderRunError> {
    engine
        .emit_execution_event(
            ProviderExecutionEvent {
                event_id: format!("{node_id}_prompt_structure_reredrive"),
                kind: ProviderExecutionEventKind::Output,
                status: ProviderExecutionEventStatus::Started,
                title: "SC 结构门拒收教学重驱提示词".to_string(),
                detail: Some(
                    "候选结构门拒收（final_compile_structure_rejected）的教学重驱\
                     （环在哪 + 两个合法修法各带后果 + 保护项 + 完整修订版输出指令），\
                     结构门类自己的恰一次机会（与 compile/IR 槽互不挤占）"
                        .to_string(),
                ),
                command: None,
                cwd: None,
                output: Some(repair_prompt.to_string()),
                exit_code: None,
            },
            Some(node_id.to_string()),
            Some(author_provider.clone()),
        )
        .await;
    let reredrive_run = match begin_plan_split_run_if_logical(
        launch,
        lifecycle,
        &engine.session().project_id,
        &engine.session().issue_id,
        author_provider,
        &engine.session().session_id,
    ) {
        Ok(run) => run,
        Err(message) => {
            engine.persist_single_candidate_terminal_phase(
                crate::product::models::SingleCandidatePhase::Failed,
            );
            return Err(SingleCandidateProviderRunError::Message(message));
        }
    };
    super::single_candidate::drive_single_candidate_reredrive(
        engine,
        launch,
        provider_for_run,
        run_cancel,
        command_rx,
        node_id,
        author_provider,
        repair_prompt,
        repository_path,
        reredrive_run.as_ref(),
        None,
    )
    .await
}
