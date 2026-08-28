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
pub(crate) use gateway_start::{resolve_plan_author_launch, start_work_item_plan_author};
#[path = "run/provider_run.rs"]
mod provider_run;
#[path = "run/single_candidate.rs"]
mod single_candidate;
pub(crate) use provider_run::{spawn_provider_run_from_event, spawn_provider_run_from_handler};

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
    pub(crate) engine: Arc<Mutex<WorkspaceEngine>>,
    pub(crate) current_run: Arc<Mutex<Option<WorkspaceActiveRun>>>,
    pub(crate) workspace_runs: WorkspaceRunRegistry,
    pub(crate) session_id: String,
    pub(crate) next_run_id: Arc<Mutex<u64>>,
    pub(crate) app_paths: ProductAppPaths,
    pub(crate) session_record: WorkspaceSessionRecord,
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

pub(crate) async fn active_run_command_tx(
    current_run: &Arc<Mutex<Option<WorkspaceActiveRun>>>,
    workspace_runs: &WorkspaceRunRegistry,
    session_id: &str,
) -> Option<mpsc::Sender<ProviderCommand>> {
    active_run(current_run, workspace_runs, session_id)
        .await
        .map(|run| run.command_tx.clone())
}

pub(crate) async fn active_run(
    current_run: &Arc<Mutex<Option<WorkspaceActiveRun>>>,
    workspace_runs: &WorkspaceRunRegistry,
    session_id: &str,
) -> Option<WorkspaceActiveRun> {
    let local = { current_run.lock().await.clone() };
    if local.is_some() {
        return local;
    }
    workspace_runs.run(session_id).await
}

pub(crate) async fn abort_workspace_run(run: &WorkspaceActiveRun) {
    let _ = run.command_tx.send(ProviderCommand::Abort).await;
    run.cancel.cancel();
}

pub(crate) async fn abort_active_run(
    current_run: &Arc<Mutex<Option<WorkspaceActiveRun>>>,
    workspace_runs: &WorkspaceRunRegistry,
    session_id: &str,
) -> bool {
    let active = { current_run.lock().await.take() };
    if let Some(run) = active {
        let _ = workspace_runs.remove_if_token(session_id, run.token).await;
        abort_workspace_run(&run).await;
        return true;
    }

    if let Some(run) = workspace_runs.take(session_id).await {
        abort_workspace_run(&run).await;
        return true;
    }

    false
}

pub(crate) async fn clear_active_run_if_token(
    current_run: &Arc<Mutex<Option<WorkspaceActiveRun>>>,
    workspace_runs: &WorkspaceRunRegistry,
    session_id: &str,
    run_token: u64,
) {
    let _ = workspace_runs.remove_if_token(session_id, run_token).await;
    let mut current = current_run.lock().await;
    if current
        .as_ref()
        .is_some_and(|active| active.token == run_token)
    {
        *current = None;
    }
}
