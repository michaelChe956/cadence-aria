use tokio::sync::mpsc;

use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::coding_models::{
    CodingAttemptStatus, CodingExecutionAttempt, CodingExecutionStage, TestingOverallStatus,
};
use crate::product::coding_workspace_engine::{
    CodingWorkspaceEngine, CodingWorkspaceEngineError, testing_report_should_enter_analyst,
};
use crate::product::coding_workspace_runner::CodingRunnerCommand;
use crate::product::git_workspace_service::GitWorkspaceService;
use crate::product::tester_agent_loop::TesterAgentOptions;
use crate::web::state::WebAppState;

use super::runner_support::{
    handle_pending_runner_commands, latest_analyst_role_run_evidence, provider_for,
    testing_result_acceptance_pending_analyst,
};
use super::{
    CodingWsOutMessage, await_stage_gate, code_review_rework_evidence, coding_execution_context,
    emit_current_session_state, ensure_work_item_execution_plan_confirmed,
    internal_pr_review_rework_evidence, repository_path_for_attempt, test_specs_for_attempt,
    testing_rework_evidence,
};

pub(crate) fn spawn_coding_runner(
    state: WebAppState,
    coding_store: CodingAttemptStore,
    event_tx: mpsc::Sender<CodingWsOutMessage>,
    attempt: CodingExecutionAttempt,
) -> mpsc::Sender<CodingRunnerCommand> {
    let (command_tx, command_rx) = mpsc::channel(32);
    tokio::spawn(async move {
        let engine = CodingWorkspaceEngine::with_provider(
            coding_store.clone(),
            GitWorkspaceService::new(),
            state.provider_adapter.clone(),
            event_tx.clone(),
        );
        if let Err(error) = execute_start_coding_flow(
            &state,
            &coding_store,
            &engine,
            &event_tx,
            command_rx,
            &attempt,
        )
        .await
        {
            if matches!(error, CodingWorkspaceEngineError::Aborted) {
                return;
            }
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
    });
    command_tx
}

pub(crate) fn should_resume_runner_after_gate_response(
    action_id: &str,
    previous_attempt: &CodingExecutionAttempt,
) -> bool {
    matches!(
        action_id,
        "retry_test_plan"
            | "continue_rework"
            | "rerun_missing_steps"
            | "retry_review"
            | "retry_internal_review"
            | "retry_analyst"
            | "send_raw_output_to_analyst"
            | "accept_testing_result"
            | "rerun_testing"
    ) && matches!(
        previous_attempt.status,
        CodingAttemptStatus::Blocked | CodingAttemptStatus::WaitingForHuman
    )
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

        if current.stage == CodingExecutionStage::Rework
            || testing_result_acceptance_pending_analyst(coding_store, &current)?
        {
            let analyst_provider_name = coding_store
                .get_role_provider_config_snapshot(
                    &current.project_id,
                    &current.issue_id,
                    &current.id,
                )?
                .analyst;
            let analyst_provider =
                provider_for(state, &analyst_provider_name, "coding analyst provider")?;
            let evidence = latest_analyst_role_run_evidence(coding_store, &current)?;
            current = engine
                .execute_rework_with_commands(
                    &current,
                    &evidence,
                    analyst_provider.as_ref(),
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
            continue 'pipeline;
        }

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

        if current.stage.order() <= CodingExecutionStage::Testing.order() {
            let Some(next) = await_stage_gate(
                &mut command_rx,
                coding_store,
                engine,
                event_tx,
                &current,
                CodingExecutionStage::Testing,
            )
            .await?
            else {
                return Ok(());
            };
            current = next;
            let test_specs = test_specs_for_attempt(&current, &execution_context);
            let tester_provider_name = coding_store
                .get_role_provider_config_snapshot(
                    &current.project_id,
                    &current.issue_id,
                    &current.id,
                )?
                .tester;
            let tester_provider =
                provider_for(state, &tester_provider_name, "coding tester provider")?;
            let testing_report = engine
                .execute_testing_with_provider_commands(
                    &current,
                    tester_provider.as_ref(),
                    &execution_context,
                    &test_specs,
                    TesterAgentOptions::default(),
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

            if engine
                .create_testing_result_review_gate(&current, &testing_report)
                .await?
                .is_some()
            {
                current = coding_store.get_attempt(
                    &current.project_id,
                    &current.issue_id,
                    &current.id,
                )?;
                return emit_current_session_state(event_tx, coding_store, &current).await;
            }
            current =
                coding_store.get_attempt(&current.project_id, &current.issue_id, &current.id)?;
            if current.status == CodingAttemptStatus::Blocked {
                return emit_current_session_state(event_tx, coding_store, &current).await;
            }

            if testing_report_should_enter_analyst(&testing_report) {
                let Some(next) = await_stage_gate(
                    &mut command_rx,
                    coding_store,
                    engine,
                    event_tx,
                    &current,
                    CodingExecutionStage::Rework,
                )
                .await?
                else {
                    return Ok(());
                };
                current = next;
                let analyst_provider_name = coding_store
                    .get_role_provider_config_snapshot(
                        &current.project_id,
                        &current.issue_id,
                        &current.id,
                    )?
                    .analyst;
                let analyst_provider =
                    provider_for(state, &analyst_provider_name, "coding analyst provider")?;
                let evidence = testing_rework_evidence(&testing_report);
                current = engine
                    .execute_rework_with_commands(
                        &current,
                        &evidence,
                        analyst_provider.as_ref(),
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

                match current.stage {
                    CodingExecutionStage::Coding => continue 'pipeline,
                    CodingExecutionStage::Testing => continue 'pipeline,
                    CodingExecutionStage::CodeReview => {}
                    _ => return emit_current_session_state(event_tx, coding_store, &current).await,
                }
            } else if matches!(
                testing_report.overall_status,
                TestingOverallStatus::Failed
                    | TestingOverallStatus::Blocked
                    | TestingOverallStatus::SkippedByUserDecision
            ) {
                return emit_current_session_state(event_tx, coding_store, &current).await;
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
            let Some(next) = await_stage_gate(
                &mut command_rx,
                coding_store,
                engine,
                event_tx,
                &current,
                CodingExecutionStage::Rework,
            )
            .await?
            else {
                return Ok(());
            };
            current = next;
            let analyst_provider_name = coding_store
                .get_role_provider_config_snapshot(
                    &current.project_id,
                    &current.issue_id,
                    &current.id,
                )?
                .analyst;
            let analyst_provider =
                provider_for(state, &analyst_provider_name, "coding analyst provider")?;
            let evidence = internal_pr_review_rework_evidence(&internal_review);
            current = engine
                .execute_rework_with_commands(
                    &current,
                    &evidence,
                    analyst_provider.as_ref(),
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
            match current.stage {
                CodingExecutionStage::Coding => continue 'pipeline,
                CodingExecutionStage::FinalConfirm => {
                    return emit_current_session_state(event_tx, coding_store, &current).await;
                }
                _ => return emit_current_session_state(event_tx, coding_store, &current).await,
            }
        }

        {
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
            let Some(next) = await_stage_gate(
                &mut command_rx,
                coding_store,
                engine,
                event_tx,
                &current,
                CodingExecutionStage::Rework,
            )
            .await?
            else {
                return Ok(());
            };
            current = next;
            let analyst_provider_name = coding_store
                .get_role_provider_config_snapshot(
                    &current.project_id,
                    &current.issue_id,
                    &current.id,
                )?
                .analyst;
            let analyst_provider =
                provider_for(state, &analyst_provider_name, "coding analyst provider")?;
            let evidence = code_review_rework_evidence(&review_report);
            current = engine
                .execute_rework_with_commands(
                    &current,
                    &evidence,
                    analyst_provider.as_ref(),
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
            match current.stage {
                CodingExecutionStage::Coding
                | CodingExecutionStage::Testing
                | CodingExecutionStage::CodeReview => continue 'pipeline,
                CodingExecutionStage::ReviewRequest => {}
                _ => return emit_current_session_state(event_tx, coding_store, &current).await,
            }

            if current.scope == crate::product::coding_models::CodingAttemptScope::WorkItemGroup {
                current = engine
                    .complete_group_unit_after_code_review(&current)
                    .await?;
                emit_current_session_state(event_tx, coding_store, &current).await?;
                if current.stage == CodingExecutionStage::PrepareContext {
                    continue 'pipeline;
                }
                if current.stage == CodingExecutionStage::ReviewRequest {
                    let review_request = engine
                        .execute_review_request(&current, "origin", "feat: implement work item")
                        .await?;
                    current = coding_store.get_attempt(
                        &current.project_id,
                        &current.issue_id,
                        &current.id,
                    )?;
                    if review_request.push_status
                        != crate::product::coding_models::PushStatus::Pushed
                    {
                        return emit_current_session_state(event_tx, coding_store, &current).await;
                    }
                }
            } else {
                let review_request = engine
                    .execute_review_request(&current, "origin", "feat: implement work item")
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
                if review_request.push_status != crate::product::coding_models::PushStatus::Pushed {
                    return emit_current_session_state(event_tx, coding_store, &current).await;
                }
            }

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
            let Some(next) = await_stage_gate(
                &mut command_rx,
                coding_store,
                engine,
                event_tx,
                &current,
                CodingExecutionStage::Rework,
            )
            .await?
            else {
                return Ok(());
            };
            current = next;
            let analyst_provider_name = coding_store
                .get_role_provider_config_snapshot(
                    &current.project_id,
                    &current.issue_id,
                    &current.id,
                )?
                .analyst;
            let analyst_provider =
                provider_for(state, &analyst_provider_name, "coding analyst provider")?;
            let evidence = internal_pr_review_rework_evidence(&internal_review);
            current = engine
                .execute_rework_with_commands(
                    &current,
                    &evidence,
                    analyst_provider.as_ref(),
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
            match current.stage {
                CodingExecutionStage::Coding => continue 'pipeline,
                CodingExecutionStage::FinalConfirm => {
                    return emit_current_session_state(event_tx, coding_store, &current).await;
                }
                _ => return emit_current_session_state(event_tx, coding_store, &current).await,
            }
        }
    }
}
