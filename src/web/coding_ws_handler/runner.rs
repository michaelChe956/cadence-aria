use std::sync::Arc;

use tokio::sync::{mpsc, oneshot};

use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::coding_models::{
    CodingAttemptStatus, CodingExecutionAttempt, CodingExecutionStage, CodingProviderRole,
};
pub(crate) use crate::product::coding_workspace_engine::{
    CodeReviewFlowDecision, code_review_flow_decision,
};
use crate::product::coding_workspace_engine::{CodingWorkspaceEngine, CodingWorkspaceEngineError};
use crate::product::coding_workspace_runner::CodingRunnerCommand;
use crate::web::state::{CodingAttemptRunKey, CodingRunReservation, WebAppState};

use super::runner_support::{
    handle_pending_runner_commands, provider_for, recover_plan_amendment_if_needed,
    refresh_runtime_revision_history,
};
use super::{
    CodingWsOutMessage, await_stage_gate, coding_execution_context, emit_current_session_state,
    ensure_work_item_execution_plan_confirmed, repository_path_for_attempt,
};

mod amendment;
mod registration;
mod start;
mod task;
pub(crate) use amendment::spawn_plan_amendment_runner_reserved;
pub(crate) use start::CodingRunnerStartProbe;
use start::record_runner_start_event;
#[cfg(test)]
pub(crate) use start::{
    spawn_coding_runner_panicking_after_registration, spawn_coding_runner_with_start_probe,
};
use task::{CodingRunnerTask, spawn_coding_runner_task};

pub(crate) fn spawn_coding_runner(
    state: WebAppState,
    coding_store: CodingAttemptStore,
    event_tx: mpsc::Sender<CodingWsOutMessage>,
    attempt: CodingExecutionAttempt,
) -> Option<mpsc::Sender<CodingRunnerCommand>> {
    let (command_tx, command_rx) = mpsc::channel(32);
    let registry_attempt_key = CodingAttemptRunKey::from_attempt(&attempt);
    let registration = state
        .coding_runs
        .insert_cancellable(&registry_attempt_key, command_tx.clone())?;
    spawn_coding_runner_task(CodingRunnerTask {
        state,
        coding_store,
        event_tx,
        attempt,
        command_rx,
        registry_run_id: registration.run_id,
        cancellation: registration.cancellation,
        start_rx: None,
        probe: None,
        #[cfg(test)]
        panic_after_registration: None,
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

/// P2 Task 4：首启专用 reserved spawn——durable claim 已在 attempt 文件，
/// 此处完成「registry 激活 → RunnerRegistered → ProviderMayHaveStarted →
/// 放行」的持久 barrier：任一 checkpoint 写失败即 drop oneshot、撤
/// registry，不放 provider；`ProviderMayHaveStarted` 恒先于 `start_tx`
/// 持久（跨进程重启此窗口不保证未触达 provider）。claim 的 command
/// 身份在此防御性复核，不同 command 不得二次放行。
pub(crate) fn spawn_coding_runner_first_start_reserved(
    state: WebAppState,
    coding_store: CodingAttemptStore,
    event_tx: mpsc::Sender<CodingWsOutMessage>,
    attempt: CodingExecutionAttempt,
    reservation: CodingRunReservation,
    command_id: &str,
) -> Result<mpsc::Sender<CodingRunnerCommand>, CodingWorkspaceEngineError> {
    spawn_coding_runner_first_start_reserved_inner(
        state,
        coding_store,
        event_tx,
        attempt,
        reservation,
        command_id,
        None,
    )
}

#[cfg(test)]
pub(crate) fn spawn_coding_runner_first_start_reserved_with_probe(
    state: WebAppState,
    coding_store: CodingAttemptStore,
    event_tx: mpsc::Sender<CodingWsOutMessage>,
    attempt: CodingExecutionAttempt,
    reservation: CodingRunReservation,
    command_id: &str,
    probe: CodingRunnerStartProbe,
) -> Result<mpsc::Sender<CodingRunnerCommand>, CodingWorkspaceEngineError> {
    spawn_coding_runner_first_start_reserved_inner(
        state,
        coding_store,
        event_tx,
        attempt,
        reservation,
        command_id,
        Some(probe),
    )
}

fn spawn_coding_runner_first_start_reserved_inner(
    state: WebAppState,
    coding_store: CodingAttemptStore,
    event_tx: mpsc::Sender<CodingWsOutMessage>,
    attempt: CodingExecutionAttempt,
    reservation: CodingRunReservation,
    command_id: &str,
    probe: Option<CodingRunnerStartProbe>,
) -> Result<mpsc::Sender<CodingRunnerCommand>, CodingWorkspaceEngineError> {
    use crate::product::coding_models::CodingStartPhase;

    let Some(claim) = attempt.start_claim.as_ref() else {
        return Err(CodingWorkspaceEngineError::ProviderStream(
            "coding_first_start_claim_missing".to_string(),
        ));
    };
    if claim.command_id != command_id {
        return Err(CodingWorkspaceEngineError::ProviderStream(format!(
            "coding_first_start_claim_command_mismatch: claim={}, spawn={command_id}",
            claim.command_id
        )));
    }
    let attempt_key = CodingAttemptRunKey::from_attempt(&attempt);
    let (command_tx, command_rx) = mpsc::channel(32);
    let registration = reservation
        .activate_cancellable(command_tx.clone())
        .ok_or_else(|| {
            CodingWorkspaceEngineError::ProviderStream(
                "coding_first_start_reservation_lost".to_string(),
            )
        })?;
    let (start_tx, start_rx) = oneshot::channel();
    let probe_events = probe.as_ref().map(|probe| Arc::clone(&probe.events));
    spawn_coding_runner_task(CodingRunnerTask {
        state: state.clone(),
        coding_store: coding_store.clone(),
        event_tx,
        attempt: attempt.clone(),
        command_rx,
        registry_run_id: registration.run_id,
        cancellation: registration.cancellation,
        start_rx: Some(start_rx),
        probe,
        #[cfg(test)]
        panic_after_registration: None,
    });
    record_runner_start_event(probe_events.as_ref(), "task_created");
    for phase in [
        CodingStartPhase::RunnerRegistered,
        CodingStartPhase::ProviderMayHaveStarted,
    ] {
        if let Err(error) =
            coding_store.advance_coding_start_phase(&attempt, command_id, phase.clone())
        {
            state.coding_runs.remove(&attempt_key, registration.run_id);
            drop(start_tx);
            return Err(error.into());
        }
        record_runner_start_event(
            probe_events.as_ref(),
            match phase {
                CodingStartPhase::RunnerRegistered => "runner_registered",
                _ => "provider_may_have_started",
            },
        );
    }
    if start_tx.send(()).is_err() {
        state.coding_runs.remove(&attempt_key, registration.run_id);
        return Err(CodingWorkspaceEngineError::ProviderStream(
            "coding_first_start_runner_start_failed".to_string(),
        ));
    }
    Ok(command_tx)
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
    let registration = reservation
        .activate_cancellable(command_tx.clone())
        .ok_or_else(|| {
            CodingWorkspaceEngineError::ProviderStream(
                "coding_recovery_reservation_lost".to_string(),
            )
        })?;
    let (start_tx, start_rx) = oneshot::channel();
    let probe_events = probe.as_ref().map(|probe| Arc::clone(&probe.events));
    spawn_coding_runner_task(CodingRunnerTask {
        state: state.clone(),
        coding_store: coding_store.clone(),
        event_tx,
        attempt: attempt.clone(),
        command_rx,
        registry_run_id: registration.run_id,
        cancellation: registration.cancellation,
        start_rx: Some(start_rx),
        probe,
        #[cfg(test)]
        panic_after_registration: None,
    });
    record_runner_start_event(probe_events.as_ref(), "task_created");
    if let Err(error) =
        coding_store.complete_failed_code_review_recovery_journal(&attempt, recovery_gate_id)
    {
        state.coding_runs.remove(
            &CodingAttemptRunKey::from_attempt(&attempt),
            registration.run_id,
        );
        drop(start_tx);
        return Err(error.into());
    }
    record_runner_start_event(probe_events.as_ref(), "journal_completed");
    if start_tx.send(()).is_err() {
        state.coding_runs.remove(
            &CodingAttemptRunKey::from_attempt(&attempt),
            registration.run_id,
        );
        return Err(CodingWorkspaceEngineError::ProviderStream(
            "coding_recovery_runner_start_failed".to_string(),
        ));
    }
    Ok(command_tx)
}

/// C2 Task 4（REQ-CRO-04）：manual_continue／accept_risk 已确认结论时，
/// 续跑 runner 跳过已完成的 Code Reviewer（复用最新持久化报告，不重跑）。
/// 判据：最新报告存在，且同 stage 的已解决门以 manual_continue／accept_risk
/// 解决并引用了该报告（retry_* 解决的门不构成跳过——重跑是预期）。
pub(crate) fn reviewer_conclusion_already_confirmed(
    coding_store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
) -> Result<bool, crate::product::json_store::ProductStoreError> {
    use crate::product::coding_models::CodingExecutionStage;

    if attempt.stage != CodingExecutionStage::CodeReview {
        return Ok(false);
    }
    let Some(latest_report_id) = coding_store
        .list_code_review_reports(&attempt.project_id, &attempt.issue_id, &attempt.id)?
        .into_iter()
        .map(|report| report.id)
        .max()
    else {
        return Ok(false);
    };
    coding_store.has_resolved_gate_confirming_report(
        &attempt.project_id,
        &attempt.issue_id,
        &attempt.id,
        CodingExecutionStage::CodeReview,
        &latest_report_id,
    )
}

pub(crate) fn should_resume_runner_after_gate_response(
    action_id: &str,
    previous_attempt: &CodingExecutionAttempt,
) -> bool {
    // C2 Task 4（REQ-CRO-04，A08）：manual_continue／accept_risk（允许的
    // 质量绕过）加入续跑白名单——门响应先落 durable gate 记录再唤回 runner
    // continuation，续跑不依赖提交动作的连接保持打开；状态仍限
    // Blocked｜WaitingForHuman。
    matches!(
        action_id,
        "retry_coding"
            | "send_to_coder"
            | "retry_review"
            | "retry_internal_review"
            | "retry_group_review_shard"
            | "retry_group_reduction"
            | "manual_continue"
            | "accept_risk"
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
            // Only pre-existing attempts that had already entered InternalPrReview may
            // reach this compatibility path. Fresh groups bypass it for human
            // FinalConfirm via `prepare_group_final_confirm_from_readiness`.
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
    emit_current_session_state(
        event_tx,
        coding_store,
        &current,
        engine.cancellation_token(),
    )
    .await
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

    let current = coding_store.get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
    let (mut current, current_child_session_id) =
        recover_plan_amendment_if_needed(engine, &current).await?;
    let refresh_runtime_history = |attempt: &CodingExecutionAttempt| {
        refresh_runtime_revision_history(&app_paths, attempt, current_child_session_id.as_deref())
    };
    if current_child_session_id.is_some() {
        refresh_runtime_history(&current)?;
    }
    coding_store.ensure_provider_run_allowed(&current)?;
    'pipeline: loop {
        engine.ensure_not_cancelled()?;
        refresh_runtime_history(&current)?;
        ensure_work_item_execution_plan_confirmed(&app_paths, &current)?;

        if matches!(current.stage, CodingExecutionStage::PrepareContext) {
            current = engine
                .start_attempt(&current.project_id, &current.issue_id, &current.id)
                .await?;
            engine.ensure_not_cancelled()?;
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
            engine.ensure_not_cancelled()?;
            let repo_path = repository_path_for_attempt(&app_paths, &current)?;
            current = engine
                .execute_worktree_prepare(&current, &repo_path)
                .await?;
            engine.ensure_not_cancelled()?;
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
            refresh_runtime_history(&current)?;
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
                return emit_current_session_state(
                    event_tx,
                    coding_store,
                    &current,
                    engine.cancellation_token(),
                )
                .await;
            }
            if plan_defect_decision
                .is_some_and(|decision| decision != CodeReviewFlowDecision::RunCoderFix)
            {
                return emit_current_session_state(
                    event_tx,
                    coding_store,
                    &current,
                    engine.cancellation_token(),
                )
                .await;
            }
        }

        // Recovery compatibility: historical group attempts may already have entered
        // InternalPrReview under the former shard/reduction pipeline. They must never
        // resume a provider-backed review: rebuild only the authoritative readiness
        // evidence and hand it to the human FinalConfirm surface. Incomplete evidence,
        // including identity mismatches, stays visible there and `handle_final_confirm`
        // remains fail-closed.
        if current.stage == CodingExecutionStage::InternalPrReview
            && current.scope == crate::product::coding_models::CodingAttemptScope::WorkItemGroup
        {
            current = engine
                .prepare_group_final_confirm_from_readiness(&current)
                .await?;
            return emit_current_session_state(
                event_tx,
                coding_store,
                &current,
                engine.cancellation_token(),
            )
            .await;
        }

        // Single-work-item attempts retain the InternalPrReview provider flow.
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
            let internal_reviewer_provider = match internal_reviewer_provider_name {
                Some(provider_name) => {
                    provider_for(state, &provider_name, "coding internal reviewer provider")?
                }
                // C2 Task 5（REQ-CRO-05）：缺 internal reviewer → 落缺配置门后停止
                // 推进，绝不以 author 顶替启动 reviewer。
                None => {
                    return Err(engine
                        .block_missing_reviewer_configuration(
                            &current,
                            CodingExecutionStage::InternalPrReview,
                            CodingProviderRole::InternalReviewer,
                        )
                        .await);
                }
            };
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
            // C2 Task 4：manual_continue／accept_risk 已确认结论 → 跳过已
            // 完成的 Code Reviewer（不重跑、role run 数不增加），从原阶段
            // 之后继续推进（等价 ContinueAfterApprove 的阶段流转）。
            if reviewer_conclusion_already_confirmed(coding_store, &current)? {
                current = coding_store.update_attempt_stage(
                    &current.project_id,
                    &current.issue_id,
                    &current.id,
                    CodingExecutionStage::ReviewRequest,
                )?;
                continue 'pipeline;
            }
            let reviewer_provider = match reviewer_provider_name {
                Some(provider_name) => {
                    provider_for(state, &provider_name, "coding reviewer provider")?
                }
                // C2 Task 5（REQ-CRO-05）：缺 code reviewer → 落缺配置门后停止推进。
                None => {
                    return Err(engine
                        .block_missing_reviewer_configuration(
                            &current,
                            CodingExecutionStage::CodeReview,
                            CodingProviderRole::CodeReviewer,
                        )
                        .await);
                }
            };
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
                    refresh_runtime_history(&current)?;
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
                        return emit_current_session_state(
                            event_tx,
                            coding_store,
                            &current,
                            engine.cancellation_token(),
                        )
                        .await;
                    }
                    if plan_defect_decision
                        .is_some_and(|decision| decision != CodeReviewFlowDecision::RunCoderFix)
                    {
                        return emit_current_session_state(
                            event_tx,
                            coding_store,
                            &current,
                            engine.cancellation_token(),
                        )
                        .await;
                    }
                    match current.status {
                        CodingAttemptStatus::Blocked | CodingAttemptStatus::WaitingForHuman => {
                            return emit_current_session_state(
                                event_tx,
                                coding_store,
                                &current,
                                engine.cancellation_token(),
                            )
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
                    return emit_current_session_state(
                        event_tx,
                        coding_store,
                        &current,
                        engine.cancellation_token(),
                    )
                    .await;
                }
                CodeReviewFlowDecision::RetryVerification
                | CodeReviewFlowDecision::StartStoryAmendment
                | CodeReviewFlowDecision::StartDesignAmendment
                | CodeReviewFlowDecision::OpenOperationalGate
                | CodeReviewFlowDecision::StopForHumanTriage => {
                    return emit_current_session_state(
                        event_tx,
                        coding_store,
                        &current,
                        engine.cancellation_token(),
                    )
                    .await;
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
            CodingExecutionStage::Coding | CodingExecutionStage::CodeReview => continue 'pipeline,
            CodingExecutionStage::ReviewRequest => {}
            _ => {
                return emit_current_session_state(
                    event_tx,
                    coding_store,
                    &current,
                    engine.cancellation_token(),
                )
                .await;
            }
        }

        if current.scope == crate::product::coding_models::CodingAttemptScope::WorkItemGroup
            && !engine.group_attempt_ready_for_final_review(&current)?
        {
            current = engine
                .complete_group_unit_after_code_review(&current)
                .await?;
            refresh_runtime_history(&current)?;
            // N5（REQ-CS-05）：group completion 返回后、任何 ReviewRequest/push
            // 分支之前识别真实 plan-run 安全暂停（matching first_ready run
            // phase=Paused，而非任意 Blocked 文本），emit session_state 并立即
            // return——不依赖 stage 自然停止，内部循环不再获得推进机会。
            if engine.plan_first_unit_safely_paused(&current)? {
                return emit_current_session_state(
                    event_tx,
                    coding_store,
                    &current,
                    engine.cancellation_token(),
                )
                .await;
            }
            emit_current_session_state(
                event_tx,
                coding_store,
                &current,
                engine.cancellation_token(),
            )
            .await?;
            if current.stage == CodingExecutionStage::PrepareContext {
                continue 'pipeline;
            }
        }
        if current.stage != CodingExecutionStage::ReviewRequest {
            return emit_current_session_state(
                event_tx,
                coding_store,
                &current,
                engine.cancellation_token(),
            )
            .await;
        }

        {
            let _review_request = engine
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
            // push 失败不再阻断主流程：review_request.push_status=Failed + push_error 已记录，
            // 后续流转（WorkItem 直接完成 / Group 经 stage gate 进 GroupFinalReview）照常推进。
            if current.scope == crate::product::coding_models::CodingAttemptScope::WorkItem {
                current = engine
                    .complete_attempt_after_review_request(&current)
                    .await?;
                return emit_current_session_state(
                    event_tx,
                    coding_store,
                    &current,
                    engine.cancellation_token(),
                )
                .await;
            }
        }

        if current.scope == crate::product::coding_models::CodingAttemptScope::WorkItemGroup {
            current = engine
                .prepare_group_final_confirm_from_readiness(&current)
                .await?;
            return emit_current_session_state(
                event_tx,
                coding_store,
                &current,
                engine.cancellation_token(),
            )
            .await;
        }

        unreachable!("only work-item and work-item-group coding attempt scopes exist");
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::Path;
    use std::process::Command as StdCommand;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    use crate::cross_cutting::provider_adapter::ProviderAdapterError;
    use crate::cross_cutting::provider_registry::ProviderRegistry;
    use crate::cross_cutting::streaming_provider::{
        ProviderSession, StreamingProviderAdapter, StreamingProviderInput,
    };
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::coding_attempt_store::{
        CodingAttemptStore, CreateCodingExecutionUnitInput, CreateGroupCodingAttemptInput,
    };
    use crate::product::coding_models::{
        CodingAttemptPlanBinding, CodingAttemptStatus, CodingExecutionAttempt,
        CodingExecutionUnitStatus, CodingUnitRun, CodingUnitRunStatus,
    };
    use crate::product::coding_plan_run::{
        CodingPlanRun, CodingPlanRunPhase, CodingPlanRunStore, CodingPlanSelection,
        PLAN_CODING_MODE_FIRST_READY_UNIT,
    };
    use crate::product::coding_workspace_engine::CodingWorkspaceEngine;
    use crate::product::git_workspace_service::GitWorkspaceService;
    use crate::product::models::ProviderName;
    use crate::product::models::{
        DependencyGraphRevision, LogicalWorkItem, PlanProjectionBundle, PlanRevisionReason,
        VerificationPlanRevision, WorkItemPlanLineage, WorkItemPlanRevision,
        WorkItemProjectionBundle, WorkItemRevision,
    };
    use crate::product::work_item_contract::{
        CanonicalWorkItemContract, HandoffContract, WorkItemContractIdentity, WorkItemGoal,
        WorkItemWritePolicy, canonical_contract_hash,
    };
    use crate::product::work_item_projection::{
        CoderGroupContext, CompiledPlanProjections, HumanGroupProjection,
        HumanGroupWorkItemSummary, ReviewerGroupMatrix, ReviewerGroupMatrixEntry,
        WorkItemProjectionCompiler, plan_projection_hashes, projection_hashes, renderer_for,
    };
    use crate::product::work_item_revision_store::WorkItemRevisionStore;
    use crate::web::coding_ws_handler::execute_start_coding_flow;
    use crate::web::runtime::WebRuntime;
    use crate::web::state::WebAppState;
    use crate::web::workspace_ws_types::ProviderConfigSnapshot;

    use tempfile::TempDir;

    /// 受控 provider probe：任何 start 都计数并失败——暂停路径 provider
    /// 计数必须为 0。
    #[derive(Clone, Default)]
    struct CountingProvider {
        starts: Arc<AtomicUsize>,
    }
    impl CountingProvider {
        fn starts(&self) -> usize {
            self.starts.load(Ordering::SeqCst)
        }
    }

    #[async_trait::async_trait]
    impl StreamingProviderAdapter for CountingProvider {
        async fn start(
            &self,
            _input: StreamingProviderInput,
            _cancel: CancellationToken,
        ) -> Result<ProviderSession, ProviderAdapterError> {
            self.starts.fetch_add(1, Ordering::SeqCst);
            Err(ProviderAdapterError::command_missing(
                "provider must not start during the plan-coding pause",
            ))
        }
    }

    struct N5RunnerFixture {
        _root: TempDir,
        paths: ProductAppPaths,
        store: CodingAttemptStore,
        attempt: CodingExecutionAttempt,
        unit: crate::product::coding_models::CodingExecutionUnit,
        original_head: String,
    }

    fn run_git(worktree: &Path, args: &[&str]) {
        let status = StdCommand::new("git")
            .args(args)
            .current_dir(worktree)
            .status()
            .expect("git command");
        assert!(status.success(), "git {args:?} failed");
    }

    fn git_head(worktree: &Path) -> String {
        let output = StdCommand::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(worktree)
            .output()
            .expect("git rev-parse");
        assert!(output.status.success(), "git rev-parse failed");
        String::from_utf8(output.stdout)
            .expect("utf8 head")
            .trim()
            .to_string()
    }

    fn minimal_contract(logical_id: &str) -> CanonicalWorkItemContract {
        CanonicalWorkItemContract {
            intent_contract: None,
            schema_version: 1,
            identity: WorkItemContractIdentity {
                logical_work_item_id: logical_id.to_string(),
                title: logical_id.to_string(),
                kind: "implementation".to_string(),
            },
            goal: WorkItemGoal {
                summary: logical_id.to_string(),
            },
            non_goals: Vec::new(),
            depends_on: Vec::new(),
            input_contracts: Vec::new(),
            output_contracts: Vec::new(),
            tasks: Vec::new(),
            write_policy: WorkItemWritePolicy {
                exclusive_scopes: Vec::new(),
                forbidden_scopes: Vec::new(),
            },
            acceptance_criteria: Vec::new(),
            verification_checks: Vec::new(),
            handoff_contract: HandoffContract {
                required_fields: Vec::new(),
                provided_contract_refs: Vec::new(),
                reviewer_check_refs: Vec::new(),
            },
            blocker_rules: Vec::new(),
            design_traceability: Vec::new(),
        }
    }

    /// 单 unit group 夹具：attempt Running@ReviewRequest + 活跃 Running
    /// unit run + 完整 plan lineage/binding（web 版 plan_repair 支撑夹具的
    /// 最小化复刻；进入 ReviewRequest 完成分支不需要 provider 会话）。
    fn n5_runner_fixture() -> N5RunnerFixture {
        let root = TempDir::new().expect("temp dir");
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        let worktree = root.path().join("worktree");
        std::fs::create_dir_all(&worktree).expect("worktree directory");
        run_git(&worktree, &["init", "--quiet"]);
        run_git(&worktree, &["config", "user.email", "n5@example.test"]);
        run_git(&worktree, &["config", "user.name", "N5 Runner"]);
        std::fs::write(worktree.join("README.md"), "# n5\n").expect("readme");
        run_git(&worktree, &["add", "README.md"]);
        run_git(&worktree, &["commit", "--quiet", "-m", "initial commit"]);
        let original_head = git_head(&worktree);

        let store = CodingAttemptStore::new(paths.clone());
        let attempt = store
            .create_group_attempt(CreateGroupCodingAttemptInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                plan_id: "work_item_plan_0001".to_string(),
                current_work_item_id: "wi_current".to_string(),
                base_branch: original_head.clone(),
                branch_name: "aria/issues/issue_0001".to_string(),
                worktree_path: Some(worktree.clone()),
                provider_config_snapshot: ProviderConfigSnapshot {
                    author: ProviderName::Codex,
                    reviewer: Some(ProviderName::ClaudeCode),
                    review_rounds: 1,
                    permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(
                    ),
                },
                target_snapshot: None,
                max_auto_rework: 2,
                start_run_policy: crate::product::coding_models::CodingStartRunPolicy::Manual,
            })
            .expect("group attempt");

        let revision_store = WorkItemRevisionStore::new(paths.clone());
        let plan = WorkItemPlanLineage {
            id: "work_item_plan_0001".to_string(),
            project_id: attempt.project_id.clone(),
            issue_id: attempt.issue_id.clone(),
            story_spec_refs: Vec::new(),
            design_spec_refs: Vec::new(),
            active_revision_id: None,
            active_amendment_id: None,
            created_at: "2026-10-13T00:00:00Z".to_string(),
            updated_at: "2026-10-13T00:00:00Z".to_string(),
        };
        revision_store
            .put_plan_lineage(&plan)
            .expect("plan lineage");
        let current_item = LogicalWorkItem {
            id: "wi_current".to_string(),
            plan_id: plan.id.clone(),
            title: "wi_current".to_string(),
            active_revision_id: None,
            created_at: "2026-10-13T00:00:00Z".to_string(),
            updated_at: "2026-10-13T00:00:00Z".to_string(),
        };
        revision_store
            .put_logical_work_item(&plan, &current_item)
            .expect("logical work item");
        let contract = minimal_contract(&current_item.id);
        let revision = WorkItemRevision {
            id: "work_item_revision_current".to_string(),
            logical_work_item_id: current_item.id.clone(),
            source_draft_revision_id: "draft_work_item_revision_current".to_string(),
            canonical_contract_hash: canonical_contract_hash(&contract).unwrap(),
            canonical_contract: contract,
            work_item_projection_bundle_id: "projection_bundle_current".to_string(),
            verification_plan_revision_id: "verification_work_item_revision_current".to_string(),
            created_at: "2026-10-13T00:00:00Z".to_string(),
        };
        revision_store
            .put_verification_plan_revision(
                &plan,
                &VerificationPlanRevision {
                    id: revision.verification_plan_revision_id.clone(),
                    logical_work_item_id: current_item.id.clone(),
                    source_draft_revision_id: revision.source_draft_revision_id.clone(),
                    verification_checks: Vec::new(),
                    created_at: "2026-10-13T00:00:00Z".to_string(),
                },
            )
            .expect("verification plan revision");
        revision_store
            .put_work_item_revision(&plan, &revision)
            .expect("work item revision");
        let compiled = WorkItemProjectionCompiler
            .compile(&revision.canonical_contract, &revision.id)
            .unwrap();
        let hashes = projection_hashes(&compiled).unwrap();
        let bundle = WorkItemProjectionBundle {
            id: revision.work_item_projection_bundle_id.clone(),
            work_item_revision_id: revision.id.clone(),
            canonical_contract_hash: revision.canonical_contract_hash.clone(),
            projection_schema_version: 1,
            compiler_version: "work-item-projection-compiler-v1".to_string(),
            human_projection: compiled.human.clone(),
            coder_projection: compiled.coder.clone(),
            reviewer_projection: compiled.reviewer.clone(),
            human_projection_hash: hashes.human,
            coder_projection_hash: hashes.coder.clone(),
            reviewer_projection_hash: hashes.reviewer.clone(),
            created_at: "2026-10-13T00:00:00Z".to_string(),
        };
        revision_store
            .put_work_item_projection_bundle(&plan, &bundle)
            .expect("work item projection bundle");
        revision_store
            .set_active_work_item_revision(&plan, &current_item, None, &revision.id)
            .expect("active work item revision");
        let dependency_graph = DependencyGraphRevision {
            id: "dependency_graph_revision_n5".to_string(),
            plan_id: plan.id.clone(),
            edges: Vec::new(),
            created_at: "2026-10-13T00:00:00Z".to_string(),
        };
        revision_store
            .put_dependency_graph_revision(&plan, &dependency_graph)
            .expect("dependency graph revision");
        let plan_revision = WorkItemPlanRevision {
            id: "plan_revision_n5".to_string(),
            plan_id: plan.id.clone(),
            revision_no: 1,
            supersedes: None,
            reason: PlanRevisionReason::InitialCompile,
            work_item_bindings: BTreeMap::from([(current_item.id.clone(), revision.id.clone())]),
            dependency_graph_revision_id: dependency_graph.id.clone(),
            validation_report_ref: "validation_report_n5".to_string(),
            plan_projection_bundle_id: "plan_projection_bundle_n5".to_string(),
            publication_provenance_ref: None,
            created_at: "2026-10-13T00:00:00Z".to_string(),
        };
        let compiled_plan = CompiledPlanProjections {
            human: HumanGroupProjection {
                plan_id: plan.id.clone(),
                goal: "n5 runner fixture".to_string(),
                split_reason: "n5 runner fixture".to_string(),
                work_items: vec![HumanGroupWorkItemSummary {
                    logical_work_item_id: current_item.id.clone(),
                    title: compiled.human.title.clone(),
                    goal: compiled.human.goal.clone(),
                    depends_on: Vec::new(),
                    provides: Vec::new(),
                    scope_summary: compiled.human.scope_summary.clone(),
                }],
                contract_flow: Vec::new(),
                risks: Vec::new(),
                source_refs: Vec::new(),
                normative: false,
                used_by_provider: false,
            },
            coder: CoderGroupContext {
                plan_id: plan.id.clone(),
                ordered_logical_work_item_ids: vec![current_item.id.clone()],
                dependency_edges: Vec::new(),
                group_write_scopes: BTreeMap::from([(
                    current_item.id.clone(),
                    compiled.coder.write_policy.clone(),
                )]),
            },
            reviewer: ReviewerGroupMatrix {
                plan_id: plan.id.clone(),
                work_items: vec![ReviewerGroupMatrixEntry {
                    logical_work_item_id: current_item.id.clone(),
                    criterion_refs: compiled.reviewer.criterion_refs.clone(),
                    input_contract_refs: Vec::new(),
                    output_contract_refs: Vec::new(),
                }],
                dependency_edges: Vec::new(),
                design_traceability_refs: Vec::new(),
            },
        };
        let plan_hashes = plan_projection_hashes(&compiled_plan).unwrap();
        revision_store
            .put_plan_projection_bundle(
                &plan,
                &PlanProjectionBundle {
                    id: plan_revision.plan_projection_bundle_id.clone(),
                    plan_revision_id: plan_revision.id.clone(),
                    dependency_graph_revision_id: plan_revision
                        .dependency_graph_revision_id
                        .clone(),
                    work_item_projection_bundle_refs: vec![bundle.id.clone()],
                    human_group_projection: compiled_plan.human,
                    coder_group_context: compiled_plan.coder,
                    reviewer_group_matrix: compiled_plan.reviewer,
                    human_group_projection_hash: plan_hashes.human,
                    coder_group_context_hash: plan_hashes.coder,
                    reviewer_group_matrix_hash: plan_hashes.reviewer,
                    compiler_version: "plan-projection-compiler-v1".to_string(),
                    created_at: "2026-10-13T00:00:00Z".to_string(),
                },
            )
            .expect("plan projection bundle");
        revision_store
            .put_plan_revision(&plan, &plan_revision)
            .expect("plan revision");
        revision_store
            .set_active_plan_revision(&plan, &plan_revision.id)
            .expect("active plan revision");
        store
            .save_plan_binding(
                &attempt,
                &CodingAttemptPlanBinding {
                    attempt_id: attempt.id.clone(),
                    plan_id: plan.id.clone(),
                    bound_plan_revision_id: plan_revision.id.clone(),
                    applied_amendment_ids: Vec::new(),
                    updated_at: "2026-10-13T00:00:00Z".to_string(),
                },
            )
            .expect("plan binding");
        let lifecycle = crate::product::lifecycle_store::LifecycleStore::new(paths.clone());
        let plan_session = lifecycle
            .create_workspace_session(
                crate::product::lifecycle_store::CreateWorkspaceSessionInput {
                    project_id: attempt.project_id.clone(),
                    issue_id: attempt.issue_id.clone(),
                    entity_id: plan.id.clone(),
                    workspace_type: crate::product::models::WorkspaceType::WorkItemPlan,
                    author_provider: ProviderName::Codex,
                    reviewer_provider: Some(ProviderName::ClaudeCode),
                    review_rounds: 1,
                    superpowers_enabled: true,
                    openspec_enabled: true,
                    work_item_plan_options: None,
                },
            )
            .expect("plan workspace session");
        lifecycle
            .save_artifact_versions(
                &plan_session.id,
                &[crate::web::workspace_ws_types::ArtifactVersion {
                    version: 1,
                    payload:
                        crate::web::workspace_ws_types::ArtifactPayload::WorkItemRevisionHistory {
                            history: Box::new(
                                crate::web::workspace_ws_types::WorkItemRevisionHistoryDto {
                                    entries: vec![
                                        crate::web::workspace_ws_types::WorkItemHistoryEntryDto {
                                            kind:
                                            crate::web::workspace_ws_types::WorkItemHistoryEntryKind::WorkItemRevision,
                                            id: revision.id.clone(),
                                            logical_work_item_id: current_item.id.clone(),
                                            related_revision_id: None,
                                            summary: "Compiled current WorkItem revision"
                                                .to_string(),
                                            created_at: "2026-10-13T00:00:00Z".to_string(),
                                        },
                                    ],
                                },
                            ),
                        },
                    generated_by: ProviderName::Codex,
                    reviewed_by: None,
                    review_verdict: None,
                    confirmed_by: None,
                    is_current: false,
                    created_at: "2026-10-13T00:00:00Z".to_string(),
                    source_node_id: "timeline_node_compile".to_string(),
                }],
            )
            .expect("plan history artifact");

        let unit = store
            .create_coding_unit(CreateCodingExecutionUnitInput {
                attempt_id: attempt.id.clone(),
                project_id: attempt.project_id.clone(),
                issue_id: attempt.issue_id.clone(),
                plan_id: plan.id.clone(),
                logical_work_item_id: current_item.id.clone(),
                work_item_revision_id: revision.id.clone(),
                dependency_logical_work_item_ids: Vec::new(),
                order_index: 1,
                status: CodingExecutionUnitStatus::Running,
            })
            .expect("coding unit");
        let mut seeded = store
            .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .unwrap();
        seeded.status = CodingAttemptStatus::Running;
        // Running 必须带 admission 会话标记，否则过不了 ensure_provider_run_allowed。
        seeded.admission_ticket_consumed_at = Some(chrono::Utc::now().to_rfc3339());
        seeded.stage = crate::product::coding_models::CodingExecutionStage::ReviewRequest;
        seeded.active_unit_id = Some(unit.id.clone());
        seeded.current_work_item_id = Some(current_item.id.clone());
        store
            .write_coding_attempt_for_test(&seeded)
            .expect("seeded running attempt");
        store
            .create_coding_unit_run(
                &seeded,
                &CodingUnitRun {
                    id: "coding_unit_run_n5_0001".to_string(),
                    unit_id: unit.id.clone(),
                    execution_no: 1,
                    work_item_revision_id: revision.id.clone(),
                    resolved_handoff_revision_ids: Vec::new(),
                    canonical_contract_hash: bundle.canonical_contract_hash.clone(),
                    projection_bundle_id: bundle.id.clone(),
                    projection_compiler_version: bundle.compiler_version.clone(),
                    coder_provider_renderer_version: renderer_for(&ProviderName::Codex)
                        .renderer_version()
                        .to_string(),
                    reviewer_provider_renderer_version: renderer_for(&ProviderName::ClaudeCode)
                        .renderer_version()
                        .to_string(),
                    internal_reviewer_provider_renderer_version: None,
                    coder_projection_hash: hashes.coder,
                    reviewer_projection_hash: hashes.reviewer,
                    coder_execution_context_hash: None,
                    reviewer_execution_context_hash: None,
                    internal_reviewer_execution_context_hash: None,
                    status: CodingUnitRunStatus::Running,
                    unit_rework_count: 0,
                    verification_retry_count: 0,
                    operational_retry_count: 0,
                    plan_repair_count: 0,
                    start_commit: Some(original_head.clone()),
                    completion_commit: None,
                    created_at: "2026-10-13T00:00:00Z".to_string(),
                    updated_at: "2026-10-13T00:00:00Z".to_string(),
                },
            )
            .expect("running unit run");

        N5RunnerFixture {
            _root: root,
            paths,
            store,
            attempt: seeded,
            unit,
            original_head,
        }
    }

    fn seed_n5_plan_run(fixture: &N5RunnerFixture) {
        let selection = CodingPlanSelection {
            unit_id: fixture.unit.id.clone(),
            logical_work_item_id: fixture.unit.logical_work_item_id.clone(),
            attempt_id: fixture.attempt.id.clone(),
            target_repository_id: "repository_0001".to_string(),
            start_command_id: CodingPlanSelection::deterministic_start_command_id(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                "work_item_plan_0001",
                "cmd_n5_plan_run",
                "repository_0001",
            ),
        };
        let run = CodingPlanRun {
            project_id: fixture.attempt.project_id.clone(),
            issue_id: fixture.attempt.issue_id.clone(),
            plan_id: "work_item_plan_0001".to_string(),
            command_id: "cmd_n5_plan_run".to_string(),
            bound_plan_revision_id: "plan_revision_n5".to_string(),
            dependency_graph_revision_id: "dependency_graph_revision_n5".to_string(),
            target_attempts: Vec::new(),
            mode: PLAN_CODING_MODE_FIRST_READY_UNIT.to_string(),
            selection: Some(selection),
            phase: CodingPlanRunPhase::Started,
            start_outcome: None,
            pause_reason: None,
            created_at: "2026-10-13T00:00:00Z".to_string(),
            updated_at: "2026-10-13T00:00:00Z".to_string(),
        };
        let runs_dir = fixture
            .paths
            .issue_root(&fixture.attempt.project_id, &fixture.attempt.issue_id)
            .join("coding-plan-runs");
        std::fs::create_dir_all(&runs_dir).expect("coding-plan-runs directory");
        std::fs::write(
            runs_dir.join("work_item_plan_0001.json"),
            serde_json::to_string(&run).expect("serialize plan run"),
        )
        .expect("persist plan run");
    }

    /// runner 在 group completion 返回后、任何 ReviewRequest/push 之前识别
    /// 真实暂停并 return：无下一 unit 启动、无 review request/push、provider
    /// probe 计数 0；暂停后重入被 N2 provider gate 拒绝。
    #[tokio::test]
    async fn plan_coding_n5_retry_and_runner_do_not_push_or_start_next() {
        let fixture = n5_runner_fixture();
        seed_n5_plan_run(&fixture);
        let provider = CountingProvider::default();
        let mut registry = ProviderRegistry::new();
        registry.register(ProviderName::Codex, Arc::new(provider.clone()));
        let mut web_state = WebAppState::new(
            fixture._root.path().to_path_buf(),
            WebRuntime::new_fake(fixture._root.path().to_path_buf()),
        );
        web_state.provider_registry = Arc::new(registry);
        web_state.test_provider_enabled = true;

        let (event_tx, mut event_rx) = mpsc::channel(64);
        let (_command_tx, command_rx) = mpsc::channel(8);
        let engine = CodingWorkspaceEngine::new(
            fixture.store.clone(),
            GitWorkspaceService::new(),
            event_tx.clone(),
        );
        let attempt = fixture.attempt.clone();
        execute_start_coding_flow(
            &web_state,
            &fixture.store,
            &engine,
            &event_tx,
            command_rx,
            &attempt,
        )
        .await
        .expect("paused runner flow returns without push");
        drop(event_tx);

        let paused = fixture
            .store
            .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("paused attempt");
        assert_eq!(paused.status, CodingAttemptStatus::Blocked);
        assert_eq!(
            paused.stage,
            crate::product::coding_models::CodingExecutionStage::CodeReview
        );
        assert!(paused.active_unit_id.is_none());
        assert!(paused.current_work_item_id.is_none());
        assert!(
            fixture
                .store
                .list_review_requests(&paused.project_id, &paused.issue_id, &paused.id)
                .expect("review requests")
                .is_empty(),
            "runner must not issue review request/push after the pause"
        );
        let units = fixture
            .store
            .list_coding_units(&paused.project_id, &paused.issue_id, &paused.id)
            .expect("units");
        assert_eq!(units.len(), 1, "no next unit may be started");
        assert_eq!(units[0].status, CodingExecutionUnitStatus::Completed);
        let unit_run = fixture
            .store
            .list_coding_unit_runs(&paused, &units[0].id)
            .expect("unit runs")
            .into_iter()
            .find(|run| run.id == "coding_unit_run_n5_0001")
            .expect("authoritative run preserved");
        assert_eq!(unit_run.status, CodingUnitRunStatus::Completed);
        assert_eq!(
            unit_run.completion_commit.as_deref(),
            Some(fixture.original_head.as_str())
        );
        assert_eq!(
            fixture
                .store
                .list_open_blocked_gates(&paused.project_id, &paused.issue_id, &paused.id)
                .expect("pause gates")
                .len(),
            1
        );
        assert_eq!(
            CodingPlanRunStore::new(fixture.paths.clone())
                .get(&paused.project_id, &paused.issue_id, "work_item_plan_0001")
                .expect("plan run lookup")
                .expect("plan run present")
                .phase,
            CodingPlanRunPhase::Paused
        );
        assert_eq!(
            provider.starts(),
            0,
            "provider probe must not start during the paused flow"
        );
        let mut session_state_events = 0;
        while let Ok(event) = event_rx.try_recv() {
            if matches!(
                event,
                crate::web::coding_ws_handler::CodingWsOutMessage::CodingSessionState { .. }
            ) {
                session_state_events += 1;
            }
        }
        assert!(
            session_state_events > 0,
            "runner must emit session_state when it stops at the pause"
        );

        // 暂停后重入（普通 retry/内部重跑）不得绕行：provider gate 拒绝。
        let (event_tx2, _event_rx2) = mpsc::channel(16);
        let (_command_tx2, command_rx2) = mpsc::channel(8);
        let engine2 = CodingWorkspaceEngine::new(
            fixture.store.clone(),
            GitWorkspaceService::new(),
            event_tx2.clone(),
        );
        let error = execute_start_coding_flow(
            &web_state,
            &fixture.store,
            &engine2,
            &event_tx2,
            command_rx2,
            &paused,
        )
        .await
        .expect_err("paused plan run must reject runner re-entry");
        assert!(
            error.to_string().contains("plan_coding_paused"),
            "unexpected re-entry result: {error}"
        );
        assert_eq!(provider.starts(), 0);
        assert!(
            fixture
                .store
                .list_review_requests(&paused.project_id, &paused.issue_id, &paused.id)
                .expect("review requests after re-entry")
                .is_empty()
        );
    }
}
