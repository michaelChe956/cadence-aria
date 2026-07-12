use tokio::sync::mpsc;

use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::coding_models::{
    CodeReviewReport, CodingAttemptStatus, CodingExecutionAttempt, CodingExecutionStage,
    ReviewVerdict,
};
use crate::product::coding_workspace_engine::{
    CodingWorkspaceEngine, CodingWorkspaceEngineError, code_review_report_has_actionable_findings,
};
use crate::product::coding_workspace_runner::CodingRunnerCommand;
use crate::product::git_workspace_service::GitWorkspaceService;
use crate::web::state::{CodingRunReservation, WebAppState};

use super::runner_support::{handle_pending_runner_commands, provider_for};
use super::{
    CodingWsOutMessage, await_stage_gate, coding_execution_context, emit_current_session_state,
    ensure_work_item_execution_plan_confirmed, repository_path_for_attempt,
};

pub(crate) fn spawn_coding_runner(
    state: WebAppState,
    coding_store: CodingAttemptStore,
    event_tx: mpsc::Sender<CodingWsOutMessage>,
    attempt: CodingExecutionAttempt,
) -> mpsc::Sender<CodingRunnerCommand> {
    let (command_tx, command_rx) = mpsc::channel(32);
    let registry_attempt_id = attempt.id.clone();
    let registry_run_id = state
        .coding_runs
        .insert(registry_attempt_id.clone(), command_tx.clone());
    spawn_coding_runner_task(
        state,
        coding_store,
        event_tx,
        attempt,
        command_rx,
        registry_run_id,
    );
    command_tx
}

pub(crate) fn spawn_coding_runner_reserved(
    state: WebAppState,
    coding_store: CodingAttemptStore,
    event_tx: mpsc::Sender<CodingWsOutMessage>,
    attempt: CodingExecutionAttempt,
    recovery_gate_id: &str,
    reservation: CodingRunReservation,
) -> Result<mpsc::Sender<CodingRunnerCommand>, CodingWorkspaceEngineError> {
    let (command_tx, command_rx) = mpsc::channel(32);
    let registry_run_id = reservation.activate(command_tx.clone()).ok_or_else(|| {
        CodingWorkspaceEngineError::ProviderStream("coding_recovery_reservation_lost".to_string())
    })?;
    if let Err(error) =
        coding_store.complete_failed_code_review_recovery_journal(&attempt.id, recovery_gate_id)
    {
        state.coding_runs.remove(&attempt.id, registry_run_id);
        return Err(error.into());
    }
    spawn_coding_runner_task(
        state,
        coding_store,
        event_tx,
        attempt,
        command_rx,
        registry_run_id,
    );
    Ok(command_tx)
}

fn spawn_coding_runner_task(
    state: WebAppState,
    coding_store: CodingAttemptStore,
    event_tx: mpsc::Sender<CodingWsOutMessage>,
    attempt: CodingExecutionAttempt,
    command_rx: mpsc::Receiver<CodingRunnerCommand>,
    registry_run_id: u64,
) {
    let registry_attempt_id = attempt.id.clone();
    let coding_runs = state.coding_runs.clone();
    tokio::spawn(async move {
        let engine = CodingWorkspaceEngine::with_provider(
            coding_store.clone(),
            GitWorkspaceService::new(),
            state.provider_adapter.clone(),
            event_tx.clone(),
        );
        let result = execute_start_coding_flow(
            &state,
            &coding_store,
            &engine,
            &event_tx,
            command_rx,
            &attempt,
        )
        .await;
        if let Err(error) = result
            && !matches!(error, CodingWorkspaceEngineError::Aborted)
        {
            let code = match &error {
                CodingWorkspaceEngineError::ExecutionPlanNotConfirmed(_) => {
                    "work_item_execution_plan_not_confirmed".to_string()
                }
                _ => "coding_start_failed".to_string(),
            };
            let _ = event_tx
                .send(CodingWsOutMessage::CodingProtocolError {
                    code,
                    message: error.to_string(),
                })
                .await;
        }
        coding_runs.remove(&registry_attempt_id, registry_run_id);
    });
}

pub(crate) fn should_resume_runner_after_gate_response(
    action_id: &str,
    previous_attempt: &CodingExecutionAttempt,
) -> bool {
    matches!(
        action_id,
        "retry_test_plan"
            | "send_to_coder"
            | "rerun_missing_steps"
            | "retry_review"
            | "retry_internal_review"
            | "accept_testing_result"
            | "rerun_testing"
    ) && matches!(
        previous_attempt.status,
        CodingAttemptStatus::Blocked | CodingAttemptStatus::WaitingForHuman
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CodeReviewFlowDecision {
    RunCoderFix,
    StopForHumanTriage,
    ContinueAfterApprove,
}

pub(crate) fn code_review_flow_decision(report: &CodeReviewReport) -> CodeReviewFlowDecision {
    match report.verdict {
        ReviewVerdict::RequestChanges => CodeReviewFlowDecision::RunCoderFix,
        ReviewVerdict::Blocked if code_review_report_has_actionable_findings(report) => {
            CodeReviewFlowDecision::RunCoderFix
        }
        ReviewVerdict::Blocked => CodeReviewFlowDecision::StopForHumanTriage,
        ReviewVerdict::Approve => CodeReviewFlowDecision::ContinueAfterApprove,
    }
}

pub(crate) async fn execute_start_coding_flow(
    state: &WebAppState,
    coding_store: &CodingAttemptStore,
    engine: &CodingWorkspaceEngine,
    event_tx: &mpsc::Sender<CodingWsOutMessage>,
    mut command_rx: mpsc::Receiver<CodingRunnerCommand>,
    attempt: &CodingExecutionAttempt,
) -> Result<(), CodingWorkspaceEngineError> {
    let app_paths = ProductAppPaths::new(state.workspace_root.join(".aria"));

    let mut current =
        coding_store.get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
    'pipeline: loop {
        ensure_work_item_execution_plan_confirmed(&app_paths, &current)?;

        if matches!(current.stage, CodingExecutionStage::PrepareContext) {
            current = engine
                .start_attempt(&current.project_id, &current.issue_id, &current.id)
                .await?;
            if handle_pending_runner_commands(
                &mut command_rx,
                coding_store,
                engine,
                event_tx,
                &current,
            )
            .await?
            {
                return Ok(());
            }
        }

        if matches!(current.stage, CodingExecutionStage::WorktreePrepare) {
            let repo_path = repository_path_for_attempt(&app_paths, &current)?;
            current = engine
                .execute_worktree_prepare(&current, &repo_path)
                .await?;
            if handle_pending_runner_commands(
                &mut command_rx,
                coding_store,
                engine,
                event_tx,
                &current,
            )
            .await?
            {
                return Ok(());
            }
        }

        let execution_context = coding_execution_context(&app_paths, &current)?;

        if current.stage.order() <= CodingExecutionStage::Coding.order() {
            let Some(next) = await_stage_gate(
                &mut command_rx,
                coding_store,
                engine,
                event_tx,
                &current,
                CodingExecutionStage::Coding,
            )
            .await?
            else {
                return Ok(());
            };
            current = next;
            let author_provider_name = coding_store
                .get_role_provider_config_snapshot(
                    &current.project_id,
                    &current.issue_id,
                    &current.id,
                )?
                .coder;
            let author_provider =
                provider_for(state, &author_provider_name, "coding author provider")?;
            current = engine
                .execute_coding_with_commands(
                    &current,
                    author_provider.as_ref(),
                    &execution_context,
                    &mut command_rx,
                )
                .await?;
            if handle_pending_runner_commands(
                &mut command_rx,
                coding_store,
                engine,
                event_tx,
                &current,
            )
            .await?
            {
                return Ok(());
            }
        }

        if current.stage == CodingExecutionStage::InternalPrReview {
            let Some(next) = await_stage_gate(
                &mut command_rx,
                coding_store,
                engine,
                event_tx,
                &current,
                CodingExecutionStage::InternalPrReview,
            )
            .await?
            else {
                return Ok(());
            };
            current = next;
            let internal_reviewer_provider_name = coding_store
                .get_role_provider_config_snapshot(
                    &current.project_id,
                    &current.issue_id,
                    &current.id,
                )?
                .internal_reviewer;
            let internal_reviewer_provider = provider_for(
                state,
                &internal_reviewer_provider_name,
                "coding internal reviewer provider",
            )?;
            let internal_review = engine
                .execute_internal_pr_review_with_commands(
                    &current,
                    internal_reviewer_provider.as_ref(),
                    &mut command_rx,
                )
                .await?;
            current =
                coding_store.get_attempt(&current.project_id, &current.issue_id, &current.id)?;
            if handle_pending_runner_commands(
                &mut command_rx,
                coding_store,
                engine,
                event_tx,
                &current,
            )
            .await?
            {
                return Ok(());
            }
            match internal_review.verdict {
                ReviewVerdict::Approve => {
                    current = if current.scope
                        == crate::product::coding_models::CodingAttemptScope::WorkItemGroup
                    {
                        engine
                            .complete_group_attempt_after_final_review(&current)
                            .await?
                    } else {
                        engine.complete_attempt_after_final_rework(&current).await?
                    };
                    return emit_current_session_state(event_tx, coding_store, &current).await;
                }
                ReviewVerdict::RequestChanges | ReviewVerdict::Blocked => {
                    return emit_current_session_state(event_tx, coding_store, &current).await;
                }
            }
        }

        if current.stage.order() <= CodingExecutionStage::CodeReview.order() {
            let Some(next) = await_stage_gate(
                &mut command_rx,
                coding_store,
                engine,
                event_tx,
                &current,
                CodingExecutionStage::CodeReview,
            )
            .await?
            else {
                return Ok(());
            };
            current = next;
            let reviewer_provider_name = coding_store
                .get_role_provider_config_snapshot(
                    &current.project_id,
                    &current.issue_id,
                    &current.id,
                )?
                .code_reviewer;
            let reviewer_provider =
                provider_for(state, &reviewer_provider_name, "coding reviewer provider")?;
            let review_report = engine
                .execute_code_review_with_commands(
                    &current,
                    reviewer_provider.as_ref(),
                    &mut command_rx,
                )
                .await?;
            current =
                coding_store.get_attempt(&current.project_id, &current.issue_id, &current.id)?;
            if handle_pending_runner_commands(
                &mut command_rx,
                coding_store,
                engine,
                event_tx,
                &current,
            )
            .await?
            {
                return Ok(());
            }
            match code_review_flow_decision(&review_report) {
                CodeReviewFlowDecision::RunCoderFix => {
                    let coder_provider_name = coding_store
                        .get_role_provider_config_snapshot(
                            &current.project_id,
                            &current.issue_id,
                            &current.id,
                        )?
                        .coder;
                    let coder_provider = provider_for(
                        state,
                        &coder_provider_name,
                        "coding coder provider (reviewer feedback fix)",
                    )?;
                    current = engine
                        .execute_coder_fix_from_review(
                            &current,
                            &review_report,
                            &execution_context,
                            coder_provider.as_ref(),
                            &mut command_rx,
                        )
                        .await?;
                    current = coding_store.get_attempt(
                        &current.project_id,
                        &current.issue_id,
                        &current.id,
                    )?;
                    if handle_pending_runner_commands(
                        &mut command_rx,
                        coding_store,
                        engine,
                        event_tx,
                        &current,
                    )
                    .await?
                    {
                        return Ok(());
                    }
                    match current.status {
                        CodingAttemptStatus::Blocked | CodingAttemptStatus::WaitingForHuman => {
                            return emit_current_session_state(event_tx, coding_store, &current)
                                .await;
                        }
                        _ => continue 'pipeline,
                    }
                }
                CodeReviewFlowDecision::StopForHumanTriage => {
                    return emit_current_session_state(event_tx, coding_store, &current).await;
                }
                CodeReviewFlowDecision::ContinueAfterApprove => {
                    current = coding_store.update_attempt_stage(
                        &current.project_id,
                        &current.issue_id,
                        &current.id,
                        CodingExecutionStage::ReviewRequest,
                    )?;
                }
            }
        }
        match current.stage {
            CodingExecutionStage::Coding
            | CodingExecutionStage::Testing
            | CodingExecutionStage::CodeReview => continue 'pipeline,
            CodingExecutionStage::ReviewRequest => {}
            _ => return emit_current_session_state(event_tx, coding_store, &current).await,
        }

        if current.scope == crate::product::coding_models::CodingAttemptScope::WorkItemGroup
            && !engine.group_attempt_ready_for_final_review(&current)?
        {
            current = engine
                .complete_group_unit_after_code_review(&current)
                .await?;
            emit_current_session_state(event_tx, coding_store, &current).await?;
            if current.stage == CodingExecutionStage::PrepareContext {
                continue 'pipeline;
            }
        }
        if current.stage != CodingExecutionStage::ReviewRequest {
            return emit_current_session_state(event_tx, coding_store, &current).await;
        }

        {
            let review_request = engine
                .execute_review_request(&current, "origin", "feat: implement work item")
                .await?;
            current =
                coding_store.get_attempt(&current.project_id, &current.issue_id, &current.id)?;
            if handle_pending_runner_commands(
                &mut command_rx,
                coding_store,
                engine,
                event_tx,
                &current,
            )
            .await?
            {
                return Ok(());
            }
            if review_request.push_status != crate::product::coding_models::PushStatus::Pushed {
                return emit_current_session_state(event_tx, coding_store, &current).await;
            }
            if current.scope == crate::product::coding_models::CodingAttemptScope::WorkItem {
                current = engine
                    .complete_attempt_after_review_request(&current)
                    .await?;
                return emit_current_session_state(event_tx, coding_store, &current).await;
            }
        }

        {
            if handle_pending_runner_commands(
                &mut command_rx,
                coding_store,
                engine,
                event_tx,
                &current,
            )
            .await?
            {
                return Ok(());
            }

            let Some(next) = await_stage_gate(
                &mut command_rx,
                coding_store,
                engine,
                event_tx,
                &current,
                CodingExecutionStage::InternalPrReview,
            )
            .await?
            else {
                return Ok(());
            };
            current = next;
            let internal_reviewer_provider_name = coding_store
                .get_role_provider_config_snapshot(
                    &current.project_id,
                    &current.issue_id,
                    &current.id,
                )?
                .internal_reviewer;
            let internal_reviewer_provider = provider_for(
                state,
                &internal_reviewer_provider_name,
                "coding internal reviewer provider",
            )?;
            let internal_review = engine
                .execute_group_final_review_with_commands(
                    &current,
                    internal_reviewer_provider.as_ref(),
                    &mut command_rx,
                )
                .await?;
            current =
                coding_store.get_attempt(&current.project_id, &current.issue_id, &current.id)?;
            if handle_pending_runner_commands(
                &mut command_rx,
                coding_store,
                engine,
                event_tx,
                &current,
            )
            .await?
            {
                return Ok(());
            }
            match internal_review.verdict {
                ReviewVerdict::Approve => {
                    current = engine
                        .complete_group_attempt_after_final_review(&current)
                        .await?;
                    return emit_current_session_state(event_tx, coding_store, &current).await;
                }
                ReviewVerdict::RequestChanges | ReviewVerdict::Blocked => {
                    return emit_current_session_state(event_tx, coding_store, &current).await;
                }
            }
        }
    }
}
