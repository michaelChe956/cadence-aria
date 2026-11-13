use super::*;
use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::cross_cutting::streaming_provider::ProviderSession;
use crate::product::logical_codebase::{PlanningContextSnapshotStore, ResolvedPlanningContext};
use crate::product::workspace_engine::WorkItemDraftAuthorOutcome;

#[macro_use]
#[path = "run/followups.rs"]
mod followups;

use followups::{
    combine_outline_auto_retry_feedback, drive_current_work_item_plan_outline_run,
    work_item_plan_retry_error,
};
#[path = "run/gateway_start.rs"]
mod gateway_start;
#[cfg(test)]
pub(crate) use gateway_start::PlanAuthorLaunch;
#[cfg(test)]
pub(crate) use gateway_start::begin_plan_split_run;
pub(crate) use gateway_start::dispatch_lc_fresh_recovery_run;
#[cfg(test)]
pub(crate) use gateway_start::{LcFreshRunKind, lc_fresh_run_kind_for_recovery_context};
pub(crate) use gateway_start::{
    PlanSplitRunContext, begin_plan_split_run_if_logical, close_plan_split_run,
    close_plan_split_run_with_markdown, fail_plan_split_run, resolve_plan_author_launch,
    start_work_item_plan_author,
};
#[path = "run/provider_run.rs"]
mod provider_run;
#[path = "run/single_candidate.rs"]
mod single_candidate;
// P01：SC 教学重驱 transport 的 run 级路径登记。实现文件为
// `run/sc_failure_transport.rs`；模块本体声明在 single_candidate.rs 内——
// 归一化 helper（`prepare_author_delivery_for_compile` 与两个归一化事件发射器）
// 是 `pub(in single_candidate)` 可见性，只有 single_candidate 子树可达。
pub(in crate::web::workspace_ws_handler::run) use single_candidate::sc_failure_transport;
// P08：SC 轮入场编排（Ready 才真实 start / Completed 复用 durable raw / Waiting
// 走既有 CAS）的 run 级路径登记。实现文件为 `run/sc_failure_round_entry.rs`；
// 模块本体声明在 single_candidate.rs 内（与 P01 transport 同形）。
pub(in crate::web::workspace_ws_handler::run) use single_candidate::sc_failure_round_entry;
// P09：SC 失败证据 blocking reasons 的 run 级路径登记。实现文件为
// `run/sc_failure_evidence.rs`；模块本体声明在 single_candidate.rs 内。
pub(in crate::web::workspace_ws_handler::run) use single_candidate::sc_failure_evidence;
// P20：SC 失败 source resolver 的 run 级路径登记。实现文件为
// `run/sc_teaching_source.rs`；模块本体声明在 single_candidate.rs 内。
pub(in crate::web::workspace_ws_handler::run) use single_candidate::sc_teaching_source;
// P22：SC 教学重驱 prompt facade 的 run 级路径登记。实现文件为
// `run/sc_teaching_prompt.rs`；模块本体声明在 single_candidate.rs 内——
// 窄再导出模块本身（调用点在 single_candidate 子树内，facade 项用
// `pub(super)` 可见性即可）。
pub(in crate::web::workspace_ws_handler::run) use single_candidate::sc_teaching_prompt;
#[path = "run/single_candidate_structure_reredrive.rs"]
mod single_candidate_structure_reredrive;
pub(crate) use provider_run::{
    spawn_provider_run_claiming_idle, spawn_provider_run_from_event,
    spawn_provider_run_from_handler,
};
// 转述门/取消修订单测的调用入口;库构建不经此处再导出。
#[cfg(test)]
pub(crate) use provider_run::{
    deliver_attempt2_frozen_prompt, fail_cancelled_human_gate_revision_turn,
};
// S0 体积基线 raw 构造点窄再导出（stack_budget 专用；库构建不经此处）。
// P2（任务3）追加：ArmExit（basic_sc_arm_cases 直驱断言）与 SC child
// `run_single_candidate_author`（先测该 child，超 131072 才在调用点 box——
// brief task-3 Step 3）；P3（任务4）追加 legacy outline 臂
// `drive_legacy_outline_arm`（legacy_outline_cases 量测/直驱）；仍限
// cfg(test) 窄导出，不做公共 API。
#[cfg(test)]
pub(crate) use provider_run::{
    ArmDrive, ArmExit, PROVIDER_RUN_TASK_BODY_CONSTRUCTIONS, drive_author_arm,
    drive_author_choice_followup_arm, drive_human_gate_sc_revision_arm, drive_legacy_outline_arm,
    drive_plan_batch_arm, drive_plan_draft_arm, drive_review_only_arm, drive_revision_arm,
    drive_single_candidate_author_arm, drive_until_cancelled_settles, provider_run_drive_body,
    provider_run_task, provider_run_task_body,
};
#[cfg(test)]
pub(crate) use single_candidate::{
    SingleCandidateAuthorRecoveryEntry, run_single_candidate_author,
};

pub(crate) static NEXT_ACTIVE_RUN_TOKEN: AtomicU64 = AtomicU64::new(1);

#[cfg(test)]
static WORK_ITEM_PLAN_PARSER_PATHS: std::sync::OnceLock<
    std::sync::Mutex<Vec<(String, &'static str)>>,
> = std::sync::OnceLock::new();

#[cfg(test)]
static SINGLE_CANDIDATE_GENERATION_STEPS: std::sync::OnceLock<
    std::sync::Mutex<Vec<(String, &'static str)>>,
> = std::sync::OnceLock::new();

#[cfg(test)]
fn record_work_item_plan_parser_path(session_id: &str, path: &'static str) {
    WORK_ITEM_PLAN_PARSER_PATHS
        .get_or_init(|| std::sync::Mutex::new(Vec::new()))
        .lock()
        .expect("work item plan parser spy")
        .push((session_id.to_string(), path));
}

#[cfg(test)]
pub(crate) fn work_item_plan_parser_paths_for_session(session_id: &str) -> Vec<&'static str> {
    WORK_ITEM_PLAN_PARSER_PATHS
        .get_or_init(|| std::sync::Mutex::new(Vec::new()))
        .lock()
        .expect("work item plan parser spy")
        .iter()
        .filter_map(|(recorded_session_id, path)| {
            (recorded_session_id == session_id).then_some(*path)
        })
        .collect()
}

#[cfg(test)]
fn record_single_candidate_generation_step(session_id: &str, step: &'static str) {
    SINGLE_CANDIDATE_GENERATION_STEPS
        .get_or_init(|| std::sync::Mutex::new(Vec::new()))
        .lock()
        .expect("single candidate generation order spy")
        .push((session_id.to_string(), step));
}

#[cfg(test)]
pub(crate) fn single_candidate_generation_steps_for_session(session_id: &str) -> Vec<&'static str> {
    SINGLE_CANDIDATE_GENERATION_STEPS
        .get_or_init(|| std::sync::Mutex::new(Vec::new()))
        .lock()
        .expect("single candidate generation order spy")
        .iter()
        .filter_map(|(recorded_session_id, step)| {
            (recorded_session_id == session_id).then_some(*step)
        })
        .collect()
}

#[derive(Clone)]
pub(crate) struct ProviderRunContext {
    pub(crate) provider_registry: Arc<ProviderRegistry>,
    pub(crate) manager: Arc<crate::web::workspace_session::WorkspaceSessionManager>,
    pub(crate) engine: Arc<Mutex<WorkspaceEngine>>,
    pub(crate) workspace_runs: WorkspaceRunRegistry,
    pub(crate) session_id: String,
    pub(crate) connection_id: Option<String>,
    pub(crate) app_paths: ProductAppPaths,
    pub(crate) session_record: WorkspaceSessionRecord,
}

#[cfg(test)]
impl ProviderRunContext {
    pub(crate) fn test_fixture(
        provider_registry: Arc<ProviderRegistry>,
        engine: Arc<Mutex<WorkspaceEngine>>,
        workspace_runs: WorkspaceRunRegistry,
        session_id: String,
        app_paths: ProductAppPaths,
        session_record: WorkspaceSessionRecord,
    ) -> Self {
        let manager =
            crate::web::workspace_session::WorkspaceSessionManager::test_fixture_with_parts(
                &session_id,
                engine.clone(),
                provider_registry.clone(),
                app_paths.clone(),
                session_record.clone(),
            );
        Self {
            provider_registry,
            manager,
            engine,
            workspace_runs,
            session_id,
            connection_id: None,
            app_paths,
            session_record,
        }
    }
}

/// 新 BLOCKER 修复：rebuilt snapshot 仅在 **provider 成功启动后** 才 commit（延迟落盘）。
///
/// provider `start` 失败（`Err`）时**不落盘** —— 确保 provider 不可用/启动前置失败时
/// snapshot 不被提前更新，同一会话重连仍判 `StaleContext`（避免再次 TOCTOU：若在
/// provider 启动前无条件落盘，失败后重连会因 snapshot 已被更新而误判 `SameContext`，
/// 重新沿用原中断会话）。返回是否已落盘。
pub(crate) fn commit_rebuilt_snapshot_after_provider_start(
    app_paths: &ProductAppPaths,
    rebuilt: &ResolvedPlanningContext,
    provider_started: &Result<ProviderSession, ProviderAdapterError>,
) -> bool {
    if provider_started.is_err() {
        return false;
    }
    match PlanningContextSnapshotStore::new(app_paths.clone()).save(&rebuilt.snapshot) {
        Ok(()) => true,
        Err(error) => {
            tracing::warn!(%error, "commit rebuilt planning snapshot after provider start failed");
            false
        }
    }
}

pub(crate) fn parse_work_item_split_structured_output(
    full_output: &str,
) -> Result<serde_json::Value, String> {
    parse_last_structured_output(full_output)
        .map_err(|error| error.details)
        .and_then(|structured| {
            structured.ok_or_else(|| "missing structured output sentinel".to_string())
        })
}

pub(crate) async fn complete_work_item_plan_outline_author_from_output(
    engine: &mut WorkspaceEngine,
    full_output: &str,
) -> Result<WorkItemPlanAuthorOutcome, String> {
    #[cfg(test)]
    record_work_item_plan_parser_path(&engine.session().session_id, "legacy_outline");
    let structured_output = match parse_work_item_split_structured_output(full_output) {
        Ok(output) => output,
        Err(message) => {
            return engine
                .complete_work_item_plan_outline_author_output_error(
                    "outline_structured_output_parse_error",
                    message,
                )
                .await;
        }
    };
    let output = match parse_work_item_plan_outline_output(structured_output) {
        Ok(output) => output,
        Err(error) => {
            return engine
                .complete_work_item_plan_outline_author_output_error(error.code, error.message)
                .await;
        }
    };
    engine.complete_work_item_plan_outline_author(output).await
}
