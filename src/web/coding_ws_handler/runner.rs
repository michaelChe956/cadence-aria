use std::sync::{Arc, Mutex};

use tokio::sync::{mpsc, oneshot};

use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::coding_models::{
    CodingAttemptStatus, CodingExecutionAttempt, CodingExecutionStage,
};
pub(crate) use crate::product::coding_workspace_engine::{
    CodeReviewFlowDecision, code_review_flow_decision,
};
use crate::product::coding_workspace_engine::{CodingWorkspaceEngine, CodingWorkspaceEngineError};
use crate::product::coding_workspace_runner::CodingRunnerCommand;
use crate::product::git_workspace_service::GitWorkspaceService;
use crate::web::state::{CodingAttemptRunKey, CodingRunReservation, WebAppState};

use super::runner_support::{handle_pending_runner_commands, provider_for};
use super::{
    CodingWsOutMessage, await_stage_gate, coding_execution_context, emit_current_session_state,
    ensure_work_item_execution_plan_confirmed, repository_path_for_attempt,
};

pub(crate) struct CodingRunnerStartProbe {
    pub(crate) events: Arc<Mutex<Vec<&'static str>>>,
    pub(crate) provider_entry_tx: oneshot::Sender<()>,
    pub(crate) continue_rx: oneshot::Receiver<()>,
}

struct CodingRunnerTask {
    state: WebAppState,
    coding_store: CodingAttemptStore,
    event_tx: mpsc::Sender<CodingWsOutMessage>,
    attempt: CodingExecutionAttempt,
    command_rx: mpsc::Receiver<CodingRunnerCommand>,
    registry_run_id: u64,
    start_rx: Option<oneshot::Receiver<()>>,
    probe: Option<CodingRunnerStartProbe>,
}

pub(crate) fn spawn_coding_runner(
    state: WebAppState,
    coding_store: CodingAttemptStore,
    event_tx: mpsc::Sender<CodingWsOutMessage>,
    attempt: CodingExecutionAttempt,
) -> Option<mpsc::Sender<CodingRunnerCommand>> {
    let (command_tx, command_rx) = mpsc::channel(32);
    let registry_attempt_key = CodingAttemptRunKey::from_attempt(&attempt);
    let registry_run_id = state
        .coding_runs
        .insert(&registry_attempt_key, command_tx.clone())?;
    spawn_coding_runner_task(CodingRunnerTask {
        state,
        coding_store,
        event_tx,
        attempt,
        command_rx,
        registry_run_id,
        start_rx: None,
        probe: None,
    });
    Some(command_tx)
}

pub(crate) fn spawn_coding_runner_reserved(
    state: WebAppState,
    coding_store: CodingAttemptStore,
    event_tx: mpsc::Sender<CodingWsOutMessage>,
    attempt: CodingExecutionAttempt,
    recovery_gate_id: &str,
    reservation: CodingRunReservation,
) -> Result<mpsc::Sender<CodingRunnerCommand>, CodingWorkspaceEngineError> {
    spawn_coding_runner_reserved_inner(
        state,
        coding_store,
        event_tx,
        attempt,
        recovery_gate_id,
        reservation,
        None,
    )
}

#[cfg(test)]
pub(crate) fn spawn_coding_runner_reserved_with_probe(
    state: WebAppState,
    coding_store: CodingAttemptStore,
    event_tx: mpsc::Sender<CodingWsOutMessage>,
    attempt: CodingExecutionAttempt,
    recovery_gate_id: &str,
    reservation: CodingRunReservation,
    probe: CodingRunnerStartProbe,
) -> Result<mpsc::Sender<CodingRunnerCommand>, CodingWorkspaceEngineError> {
    spawn_coding_runner_reserved_inner(
        state,
        coding_store,
        event_tx,
        attempt,
        recovery_gate_id,
        reservation,
        Some(probe),
    )
}

fn spawn_coding_runner_reserved_inner(
    state: WebAppState,
    coding_store: CodingAttemptStore,
    event_tx: mpsc::Sender<CodingWsOutMessage>,
    attempt: CodingExecutionAttempt,
    recovery_gate_id: &str,
    reservation: CodingRunReservation,
    probe: Option<CodingRunnerStartProbe>,
) -> Result<mpsc::Sender<CodingRunnerCommand>, CodingWorkspaceEngineError> {
    let (command_tx, command_rx) = mpsc::channel(32);
    let registry_run_id = reservation.activate(command_tx.clone()).ok_or_else(|| {
        CodingWorkspaceEngineError::ProviderStream("coding_recovery_reservation_lost".to_string())
    })?;
    let (start_tx, start_rx) = oneshot::channel();
    let probe_events = probe.as_ref().map(|probe| Arc::clone(&probe.events));
    spawn_coding_runner_task(CodingRunnerTask {
        state: state.clone(),
        coding_store: coding_store.clone(),
        event_tx,
        attempt: attempt.clone(),
        command_rx,
        registry_run_id,
        start_rx: Some(start_rx),
        probe,
    });
    record_runner_start_event(probe_events.as_ref(), "task_created");
    if let Err(error) =
        coding_store.complete_failed_code_review_recovery_journal(&attempt, recovery_gate_id)
    {
        state.coding_runs.remove(
            &CodingAttemptRunKey::from_attempt(&attempt),
            registry_run_id,
        );
        drop(start_tx);
        return Err(error.into());
    }
    record_runner_start_event(probe_events.as_ref(), "journal_completed");
    if start_tx.send(()).is_err() {
        state.coding_runs.remove(
            &CodingAttemptRunKey::from_attempt(&attempt),
            registry_run_id,
        );
        return Err(CodingWorkspaceEngineError::ProviderStream(
            "coding_recovery_runner_start_failed".to_string(),
        ));
    }
    Ok(command_tx)
}

fn spawn_coding_runner_task(task: CodingRunnerTask) {
    let CodingRunnerTask {
        state,
        coding_store,
        event_tx,
        attempt,
        command_rx,
        registry_run_id,
        start_rx,
        probe,
    } = task;
    let registry_attempt_key = CodingAttemptRunKey::from_attempt(&attempt);
    let coding_runs = state.coding_runs.clone();
    tokio::spawn(async move {
        if let Some(start_rx) = start_rx
            && start_rx.await.is_err()
        {
            coding_runs.remove(&registry_attempt_key, registry_run_id);
            return;
        }
        if let Some(probe) = probe {
            record_runner_start_event(Some(&probe.events), "provider_entry");
            let _ = probe.provider_entry_tx.send(());
            if probe.continue_rx.await.is_err() {
                coding_runs.remove(&registry_attempt_key, registry_run_id);
                return;
            }
        }
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
            let latest_attempt =
                coding_store.get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id);
            if let Ok(latest_attempt) = latest_attempt
                && !should_emit_coding_runner_protocol_error(&latest_attempt.status)
            {
                if let Err(snapshot_error) =
                    emit_current_session_state(&event_tx, &coding_store, &latest_attempt).await
                {
                    tracing::warn!(
                        attempt_id = attempt.id.as_str(),
                        error = %snapshot_error,
                        "failed to rebuild recoverable coding session state"
                    );
                }
            } else {
                let code = match &error {
                    CodingWorkspaceEngineError::ExecutionPlanNotConfirmed(_) => {
                        "work_item_execution_plan_not_confirmed".to_string()
                    }
                    _ if error
                        .to_string()
                        .contains("plan_amendment_blocks_provider_run") =>
                    {
                        "plan_amendment_blocks_provider_run".to_string()
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
        }
        coding_runs.remove(&registry_attempt_key, registry_run_id);
    });
}

fn record_runner_start_event(events: Option<&Arc<Mutex<Vec<&'static str>>>>, event: &'static str) {
    if let Some(events) = events {
        events
            .lock()
            .expect("coding runner start events")
            .push(event);
    }
}

pub(crate) fn should_resume_runner_after_gate_response(
    action_id: &str,
    previous_attempt: &CodingExecutionAttempt,
) -> bool {
    matches!(
        action_id,
        "retry_test_plan"
            | "retry_coding"
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

pub(crate) fn should_emit_coding_runner_protocol_error(status: &CodingAttemptStatus) -> bool {
    !matches!(
        status,
        CodingAttemptStatus::Blocked | CodingAttemptStatus::WaitingForHuman
    )
}

pub(crate) async fn start_plan_repair_for_execution_outcome_if_needed(
    engine: &CodingWorkspaceEngine,
    current: &CodingExecutionAttempt,
    decision: Option<CodeReviewFlowDecision>,
    report: Option<&crate::product::coding_workspace_engine::ExecutionPlanDefectReport>,
) -> Result<Option<CodingExecutionAttempt>, CodingWorkspaceEngineError> {
    if decision != Some(CodeReviewFlowDecision::StartPlanRepair) {
        return Ok(None);
    }
    let report = report.ok_or_else(|| {
        CodingWorkspaceEngineError::ProviderStream(
            "plan_repair_execution_report_missing".to_string(),
        )
    })?;
    engine
        .start_plan_repair_from_execution_report(current, report)
        .await
        .map(Some)
}

async fn handle_internal_review_flow_decision(
    coding_store: &CodingAttemptStore,
    engine: &CodingWorkspaceEngine,
    event_tx: &mpsc::Sender<CodingWsOutMessage>,
    current: &CodingExecutionAttempt,
    internal_review: &crate::product::coding_models::InternalPrReview,
) -> Result<(), CodingWorkspaceEngineError> {
    let current = match engine
        .internal_review_flow_decision_for_attempt(current, internal_review)?
    {
        CodeReviewFlowDecision::ContinueAfterApprove => {
            if current.scope == crate::product::coding_models::CodingAttemptScope::WorkItemGroup {
                engine
                    .complete_group_attempt_after_final_review(current)
                    .await?
            } else {
                engine.complete_attempt_after_final_rework(current).await?
            }
        }
        CodeReviewFlowDecision::StartPlanRepair => {
            engine
                .start_plan_repair_from_internal_review(current, internal_review)
                .await?
        }
        CodeReviewFlowDecision::RunCoderFix
        | CodeReviewFlowDecision::RetryVerification
        | CodeReviewFlowDecision::StartStoryAmendment
        | CodeReviewFlowDecision::StartDesignAmendment
        | CodeReviewFlowDecision::OpenOperationalGate
        | CodeReviewFlowDecision::StopForHumanTriage => current.clone(),
    };
    emit_current_session_state(event_tx, coding_store, &current).await
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
    if matches!(
        current.status,
        CodingAttemptStatus::AwaitingPlanAmendment
            | CodingAttemptStatus::ApplyingPlanAmendment
            | CodingAttemptStatus::AmendmentApplyFailed
    ) {
        current = engine.recover_plan_amendment(&current).await?;
    }
    coding_store.ensure_provider_run_allowed(&current)?;
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
            let coding_outcome = engine
                .execute_coding_with_commands_outcome(
                    &current,
                    author_provider.as_ref(),
                    &execution_context,
                    &mut command_rx,
                )
                .await?;
            let plan_defect_decision = coding_outcome.plan_defect_decision;
            let plan_defect_report = coding_outcome.plan_defect_report;
            current = coding_outcome.attempt;
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
            if let Some(paused) = start_plan_repair_for_execution_outcome_if_needed(
                engine,
                &current,
                plan_defect_decision,
                plan_defect_report.as_ref(),
            )
            .await?
            {
                current = paused;
                return emit_current_session_state(event_tx, coding_store, &current).await;
            }
            if plan_defect_decision
                .is_some_and(|decision| decision != CodeReviewFlowDecision::RunCoderFix)
            {
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
            return handle_internal_review_flow_decision(
                coding_store,
                engine,
                event_tx,
                &current,
                &internal_review,
            )
            .await;
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
            let reviewer_projection = engine.reviewer_projection_for_attempt(&current)?;
            match code_review_flow_decision(&review_report, &reviewer_projection) {
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
                    let rework_outcome = engine
                        .execute_coder_fix_from_review_outcome(
                            &current,
                            &review_report,
                            &execution_context,
                            coder_provider.as_ref(),
                            &mut command_rx,
                        )
                        .await?;
                    let plan_defect_decision = rework_outcome.plan_defect_decision;
                    let plan_defect_report = rework_outcome.plan_defect_report;
                    current = rework_outcome.attempt;
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
                    if let Some(paused) = start_plan_repair_for_execution_outcome_if_needed(
                        engine,
                        &current,
                        plan_defect_decision,
                        plan_defect_report.as_ref(),
                    )
                    .await?
                    {
                        current = paused;
                        return emit_current_session_state(event_tx, coding_store, &current).await;
                    }
                    if plan_defect_decision
                        .is_some_and(|decision| decision != CodeReviewFlowDecision::RunCoderFix)
                    {
                        return emit_current_session_state(event_tx, coding_store, &current).await;
                    }
                    match current.status {
                        CodingAttemptStatus::Blocked | CodingAttemptStatus::WaitingForHuman => {
                            return emit_current_session_state(event_tx, coding_store, &current)
                                .await;
                        }
                        _ => continue 'pipeline,
                    }
                }
                CodeReviewFlowDecision::StartPlanRepair => {
                    let (finding_index, finding) = review_report
                        .findings
                        .iter()
                        .enumerate()
                        .find(|(_, finding)| {
                            matches!(
                                finding.defect_class,
                                crate::product::models::PlanDefectClass::CurrentWorkItemInvalid
                                    | crate::product::models::PlanDefectClass::UpstreamContractInvalid
                                    | crate::product::models::PlanDefectClass::DependencyGraphInvalid
                            )
                        })
                        .ok_or_else(|| {
                            CodingWorkspaceEngineError::ProviderStream(
                                "plan_repair_finding_missing".to_string(),
                            )
                        })?;
                    current = engine
                        .start_plan_repair_from_review(
                            &current,
                            &review_report.id,
                            &format!("{}_finding_{:04}", review_report.id, finding_index + 1),
                            finding,
                            &reviewer_projection,
                        )
                        .await?;
                    return emit_current_session_state(event_tx, coding_store, &current).await;
                }
                CodeReviewFlowDecision::RetryVerification
                | CodeReviewFlowDecision::StartStoryAmendment
                | CodeReviewFlowDecision::StartDesignAmendment
                | CodeReviewFlowDecision::OpenOperationalGate
                | CodeReviewFlowDecision::StopForHumanTriage => {
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
            return handle_internal_review_flow_decision(
                coding_store,
                engine,
                event_tx,
                &current,
                &internal_review,
            )
            .await;
        }
    }
}
