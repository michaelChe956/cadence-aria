use super::*;
use crate::product::workspace_engine::WorkItemDraftAuthorOutcome;

#[macro_use]
#[path = "run/followups.rs"]
mod followups;

use followups::{
    combine_outline_auto_retry_feedback, drive_current_work_item_plan_outline_run,
    work_item_plan_retry_error,
};
#[path = "run/provider_run.rs"]
mod provider_run;
pub(crate) use provider_run::spawn_provider_run_from_handler;

pub(crate) static NEXT_ACTIVE_RUN_TOKEN: AtomicU64 = AtomicU64::new(1);

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

pub(crate) enum ProviderRunKind {
    Author { content: String },
    AuthorChoiceFollowup { content: String },
    Revision,
    ReviewOnly,
    WorkItemPlanAuthor,
    WorkItemPlanOutlineRevision { feedback: Option<String> },
    WorkItemPlanDraft { feedback: Option<String> },
    WorkItemPlanBatch,
    WorkItemPlanRevision { feedback: Option<String> },
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
