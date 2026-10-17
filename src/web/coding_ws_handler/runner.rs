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
            | "retry_coding_with_context"
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

/// Bug2-B(Task 4)：下一 stage 残留 open 门只读告警——open 门中 stage 严格
/// 早于当前 stage 的即为残留（同 stage 门是正常人工等待面）。返回告警行
/// 由调用侧 eprintln；**绝不自动解门**（门只经动作/统一关闭原语收口），
/// 读取失败时静默尽力而为（告警面不阻塞推进主链）。
pub(crate) fn residual_open_blocked_gate_warnings(
    coding_store: &CodingAttemptStore,
    current: &CodingExecutionAttempt,
) -> Vec<String> {
    let Ok(open_gates) =
        coding_store.list_open_blocked_gates(&current.project_id, &current.issue_id, &current.id)
    else {
        return Vec::new();
    };
    open_gates
        .into_iter()
        .filter(|gate| {
            gate.stage
                .as_ref()
                .is_some_and(|stage| stage.order() < current.stage.order())
        })
        .map(|gate| {
            format!(
                "residual open blocked gate {} (stage {:?}) while attempt {} advanced to {:?}: manual attention required; auto-resolution is forbidden",
                gate.gate_id, gate.stage, current.id, current.stage
            )
        })
        .collect()
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

/// T19 采信门:报告采信投影是否允许 approve 交付推进。缺字段(None)=
/// 历史未核验报告(不改历史行为);仅显式 Accepted 放行,
/// PendingAcceptance/Invalidated 一律停人工面。
fn review_acceptance_permits_delivery(
    acceptance: Option<crate::product::coding_models::ReviewReportAcceptance>,
) -> bool {
    !matches!(
        acceptance,
        Some(
            crate::product::coding_models::ReviewReportAcceptance::PendingAcceptance
                | crate::product::coding_models::ReviewReportAcceptance::Invalidated
        )
    )
}

async fn handle_internal_review_flow_decision(
    coding_store: &CodingAttemptStore,
    engine: &CodingWorkspaceEngine,
    event_tx: &mpsc::Sender<CodingWsOutMessage>,
    current: &CodingExecutionAttempt,
    internal_review: &crate::product::coding_models::InternalPrReview,
) -> Result<(), CodingWorkspaceEngineError> {
    let raw_decision =
        engine.internal_review_flow_decision_for_attempt(current, internal_review)?;
    // T19 采信门:Pending/Invalidated 报告即使内容 approve 也不驱动
    // 交付推进(完成/整组完成一律停在人工面)。
    let decision = if raw_decision == CodeReviewFlowDecision::ContinueAfterApprove
        && !review_acceptance_permits_delivery(internal_review.acceptance)
    {
        CodeReviewFlowDecision::StopForHumanTriage
    } else {
        raw_decision
    };
    let current = match decision {
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
            // T14/T19 合同预留:Coder 写入不得与 Reviewer 窗口重叠——
            // 窗口冻结期间(Coder 窗外)fail-closed;不依赖 T14 实现,
            // 由窗口租约面直接判定。
            if let Some(worktree) = current.worktree_path.as_ref() {
                engine.ensure_no_open_reviewer_window(worktree)?;
            }
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
            // T19 采信门:Pending/Invalidated 报告即使内容 approve 也不
            // 驱动交付推进(ReviewRequest/完成链一律停在人工面)。
            let review_flow_decision = {
                let raw = code_review_flow_decision(&review_report, &reviewer_projection);
                if raw == CodeReviewFlowDecision::ContinueAfterApprove
                    && !review_acceptance_permits_delivery(review_report.acceptance)
                {
                    CodeReviewFlowDecision::StopForHumanTriage
                } else {
                    raw
                }
            };
            match review_flow_decision {
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
                    // T14/T19 合同预留:返修 Coder 写入同样不得与 Reviewer
                    // 窗口重叠(review 窗口已收口,此处拦的是他 attempt
                    // 残留窗口/兄弟树窗口)。
                    if let Some(worktree) = current.worktree_path.as_ref() {
                        engine.ensure_no_open_reviewer_window(worktree)?;
                    }
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
                    // Bug2-B(Task 4)：推进检测——进入下一 stage 时扫描残留
                    // open 门,只读告警绝不自动解门(门只经动作/统一关闭
                    // 原语收口)。
                    for warning in residual_open_blocked_gate_warnings(coding_store, &current) {
                        eprintln!("[aria-runner] {warning}");
                    }
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
            // S2 parked 修复(切片A):完成链可能 park——下一 unit 依赖桶外
            // 上游时,advance Waiting 清 active_unit_id、stage 保持
            // ReviewRequest。此形状禁止落入下方 review_request/final_confirm
            // 准备段:那会发出 review request 并撞终审就绪守卫,runner 死亡
            // 转 AMR。
            let parked_after_completion = current.scope
                == crate::product::coding_models::CodingAttemptScope::WorkItemGroup
                && current.active_unit_id.is_none()
                && !engine.group_attempt_ready_for_final_review(&current)?;
            // S2b（N5 暂停移除）：完成链自动推进后无默认步暂停停点——直接
            // emit session_state；PrepareContext 即继续内循环驱动下一 unit。
            // 跨桶就绪集推进在完成事件上经入口面 drive 桥重入（编排器复用
            // 同一 engine 推进入口，本 runner 不引入跨 plan 串行假设）。
            emit_current_session_state(
                event_tx,
                coding_store,
                &current,
                engine.cancellation_token(),
            )
            .await?;
            // S2b 完成事件挂点：跨桶就绪集推进经入口面 drive 桥重入（per-plan
            // 粒度，无跨 plan 串行假设；runner 任务不在入口 POST 锁内，按锁
            // 契约自取 work_item_group 串行锁；桥幂等、跳过忙 attempt，失败
            // 仅告警不阻断本桶执行链）。
            if let Some(plan_id) = current.work_item_group_id.clone() {
                let _drive_guard = state
                    .coding_runs
                    .lock_named(&format!(
                        "work_item_group:{}:{}:{}",
                        current.project_id, current.issue_id, plan_id
                    ))
                    .await;
                if let Err(error) = crate::web::handlers::drive_ready_units_for_plan(
                    state,
                    &current.project_id,
                    &current.issue_id,
                    &plan_id,
                    Some(&current.id),
                )
                .await
                {
                    tracing::warn!(?error, "plan_coding_ready_set_drive_failed");
                }
            }
            if parked_after_completion {
                // 重读 attempt:循环变量 current 是完成链返回时的陈旧值,
                // 上方 drive 桥(切片B in_place 物化)可能已把就绪的下一
                // unit 物化为 PrepareContext+active 指针——继续内循环排程;
                // 否则干净退出(attempt 保持 Running,由入口面 drive 续跑)。
                current = coding_store.get_attempt(
                    &current.project_id,
                    &current.issue_id,
                    &current.id,
                )?;
                if current.stage == CodingExecutionStage::PrepareContext
                    && current.active_unit_id.is_some()
                {
                    continue 'pipeline;
                }
                return Ok(());
            }
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

    /// S2b（N5 暂停移除）：runner 在 group completion 自动推进后不再停——
    /// 单 unit plan 完成即 plan_complete：ReviewRequest→review request/push→
    /// 人工终审准备（FinalConfirm+WaitingForHuman）；run 相位保持 Started、
    /// 零暂停 gate、全程零 provider start。
    #[tokio::test]
    async fn plan_coding_s2_runner_flows_to_final_confirm_with_plan_run() {
        let fixture = n5_runner_fixture();
        seed_n5_plan_run(&fixture);
        // 推送远端：review request push 走真实 bare origin（失败也不阻断，
        // 此处尽量让 push 成功以贴近真实链）。
        let worktree = fixture
            .attempt
            .worktree_path
            .clone()
            .expect("fixture worktree");
        let remote = fixture._root.path().join("origin.git");
        let status = std::process::Command::new("git")
            .args(["init", "--bare", remote.to_str().expect("remote path")])
            .status()
            .expect("create bare origin");
        assert!(status.success(), "create bare origin");
        run_git(&worktree, &["branch", "aria/issues/issue_0001"]);
        run_git(
            &worktree,
            &[
                "remote",
                "add",
                "origin",
                remote.to_str().expect("remote path"),
            ],
        );
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
        .expect("runner completes through final confirmation without pause");
        drop(event_tx);

        let final_attempt = fixture
            .store
            .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("final attempt");
        assert_eq!(final_attempt.status, CodingAttemptStatus::WaitingForHuman);
        assert_eq!(
            final_attempt.stage,
            crate::product::coding_models::CodingExecutionStage::FinalConfirm
        );
        assert!(
            !fixture
                .store
                .list_review_requests(
                    &final_attempt.project_id,
                    &final_attempt.issue_id,
                    &final_attempt.id
                )
                .expect("review requests")
                .is_empty(),
            "runner must issue the review request after auto-advance"
        );
        let units = fixture
            .store
            .list_coding_units(
                &final_attempt.project_id,
                &final_attempt.issue_id,
                &final_attempt.id,
            )
            .expect("units");
        assert_eq!(units.len(), 1);
        assert_eq!(units[0].status, CodingExecutionUnitStatus::Completed);
        assert!(
            fixture
                .store
                .list_open_blocked_gates(
                    &final_attempt.project_id,
                    &final_attempt.issue_id,
                    &final_attempt.id
                )
                .expect("blocked gates")
                .is_empty(),
            "no pause gate may be created"
        );
        assert_eq!(
            CodingPlanRunStore::new(fixture.paths.clone())
                .get(
                    &final_attempt.project_id,
                    &final_attempt.issue_id,
                    "work_item_plan_0001"
                )
                .expect("plan run lookup")
                .expect("plan run present")
                .phase,
            CodingPlanRunPhase::Started
        );
        assert_eq!(
            provider.starts(),
            0,
            "the review-request tail path must not start a provider"
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
            "runner must emit session_state while it flows to final confirmation"
        );
    }
    struct N5ParkedRunnerFixture {
        _root: TempDir,
        paths: ProductAppPaths,
        store: CodingAttemptStore,
        attempt: CodingExecutionAttempt,
        unit_a: crate::product::coding_models::CodingExecutionUnit,
        unit_b: crate::product::coding_models::CodingExecutionUnit,
    }

    /// 2-unit sc_advance 桶 parked 夹具(S2 切片C):桶内 wi_a/wi_b,wi_b
    /// 依赖桶外 wi_x——wi_x 只进 plan 权威图/绑定,不落桶内 unit,SC 桶内
    /// 依赖门对桶外上游恒 Waiting(n5 单 unit 夹具按三工作项推广)。
    fn n5_sc_advance_parked_fixture() -> N5ParkedRunnerFixture {
        let root = TempDir::new().expect("temp dir");
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        let worktree = root.path().join("worktree");
        std::fs::create_dir_all(&worktree).expect("worktree directory");
        run_git(&worktree, &["init", "--quiet"]);
        run_git(&worktree, &["config", "user.email", "n5@example.test"]);
        run_git(&worktree, &["config", "user.name", "N5 Runner"]);
        std::fs::write(worktree.join("README.md"), "# n5 parked\n").expect("readme");
        run_git(&worktree, &["add", "README.md"]);
        run_git(&worktree, &["commit", "--quiet", "-m", "initial commit"]);
        let original_head = git_head(&worktree);

        let store = CodingAttemptStore::new(paths.clone());
        let attempt = store
            .create_group_attempt(CreateGroupCodingAttemptInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                plan_id: "work_item_plan_0001".to_string(),
                current_work_item_id: "wi_a".to_string(),
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
        let logical_ids = ["wi_a", "wi_b", "wi_x"];
        let mut revisions = BTreeMap::new();
        let mut bundles = BTreeMap::new();
        let mut plan_bindings = BTreeMap::new();
        let mut human_summaries = Vec::new();
        let mut coder_scopes = BTreeMap::new();
        let mut reviewer_rows = Vec::new();
        let mut history_entries = Vec::new();
        for logical_id in logical_ids {
            let current_item = LogicalWorkItem {
                id: logical_id.to_string(),
                plan_id: plan.id.clone(),
                title: logical_id.to_string(),
                active_revision_id: None,
                created_at: "2026-10-13T00:00:00Z".to_string(),
                updated_at: "2026-10-13T00:00:00Z".to_string(),
            };
            revision_store
                .put_logical_work_item(&plan, &current_item)
                .expect("logical work item");
            let contract = minimal_contract(logical_id);
            let revision = WorkItemRevision {
                id: format!("work_item_revision_{logical_id}"),
                logical_work_item_id: logical_id.to_string(),
                source_draft_revision_id: format!("draft_work_item_revision_{logical_id}"),
                canonical_contract_hash: canonical_contract_hash(&contract).unwrap(),
                canonical_contract: contract,
                work_item_projection_bundle_id: format!("projection_bundle_{logical_id}"),
                verification_plan_revision_id: format!(
                    "verification_work_item_revision_{logical_id}"
                ),
                created_at: "2026-10-13T00:00:00Z".to_string(),
            };
            revision_store
                .put_verification_plan_revision(
                    &plan,
                    &VerificationPlanRevision {
                        id: revision.verification_plan_revision_id.clone(),
                        logical_work_item_id: revision.logical_work_item_id.clone(),
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
            plan_bindings.insert(logical_id.to_string(), revision.id.clone());
            human_summaries.push(HumanGroupWorkItemSummary {
                logical_work_item_id: logical_id.to_string(),
                title: compiled.human.title.clone(),
                goal: compiled.human.goal.clone(),
                depends_on: if logical_id == "wi_b" {
                    vec!["wi_x".to_string()]
                } else {
                    Vec::new()
                },
                provides: Vec::new(),
                scope_summary: compiled.human.scope_summary.clone(),
            });
            coder_scopes.insert(logical_id.to_string(), compiled.coder.write_policy.clone());
            reviewer_rows.push(ReviewerGroupMatrixEntry {
                logical_work_item_id: logical_id.to_string(),
                criterion_refs: compiled.reviewer.criterion_refs.clone(),
                input_contract_refs: Vec::new(),
                output_contract_refs: Vec::new(),
            });
            history_entries.push((revision.id.clone(), logical_id.to_string()));
            revisions.insert(logical_id.to_string(), revision);
            bundles.insert(logical_id.to_string(), bundle);
        }
        // 权威依赖图:wi_x -> wi_b(桶外上游;SC 门 N1 同源校验按全图)。
        let dependency_graph = DependencyGraphRevision {
            id: "dependency_graph_revision_n5_parked".to_string(),
            plan_id: plan.id.clone(),
            edges: vec![crate::product::work_item_contract::DependencyContractEdge {
                from: "wi_x".to_string(),
                to: "wi_b".to_string(),
                required_contracts: Vec::new(),
            }],
            created_at: "2026-10-13T00:00:00Z".to_string(),
        };
        revision_store
            .put_dependency_graph_revision(&plan, &dependency_graph)
            .expect("dependency graph revision");
        let plan_revision = WorkItemPlanRevision {
            id: "plan_revision_n5_parked".to_string(),
            plan_id: plan.id.clone(),
            revision_no: 1,
            supersedes: None,
            reason: PlanRevisionReason::InitialCompile,
            work_item_bindings: plan_bindings,
            dependency_graph_revision_id: dependency_graph.id.clone(),
            validation_report_ref: "validation_report_n5_parked".to_string(),
            plan_projection_bundle_id: "plan_projection_bundle_n5_parked".to_string(),
            publication_provenance_ref: None,
            created_at: "2026-10-13T00:00:00Z".to_string(),
        };
        let compiled_plan = CompiledPlanProjections {
            human: HumanGroupProjection {
                plan_id: plan.id.clone(),
                goal: "n5 parked fixture".to_string(),
                split_reason: "n5 parked fixture".to_string(),
                work_items: human_summaries,
                contract_flow: Vec::new(),
                risks: Vec::new(),
                source_refs: Vec::new(),
                normative: false,
                used_by_provider: false,
            },
            coder: CoderGroupContext {
                plan_id: plan.id.clone(),
                ordered_logical_work_item_ids: logical_ids
                    .iter()
                    .map(|logical_id| (*logical_id).to_string())
                    .collect(),
                dependency_edges: dependency_graph.edges.clone(),
                group_write_scopes: coder_scopes,
            },
            reviewer: ReviewerGroupMatrix {
                plan_id: plan.id.clone(),
                work_items: reviewer_rows,
                dependency_edges: dependency_graph.edges.clone(),
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
                    work_item_projection_bundle_refs: bundles
                        .values()
                        .map(|bundle| bundle.id.clone())
                        .collect(),
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
                                    entries: history_entries
                                        .into_iter()
                                        .map(|(revision_id, logical_work_item_id)| {
                                            crate::web::workspace_ws_types::WorkItemHistoryEntryDto {
                                                kind: crate::web::workspace_ws_types::WorkItemHistoryEntryKind::WorkItemRevision,
                                                id: revision_id,
                                                logical_work_item_id,
                                                related_revision_id: None,
                                                summary: "Compiled WorkItem revision".to_string(),
                                                created_at: "2026-10-13T00:00:00Z".to_string(),
                                            }
                                        })
                                        .collect(),
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

        let unit_a = store
            .create_coding_unit(CreateCodingExecutionUnitInput {
                attempt_id: attempt.id.clone(),
                project_id: attempt.project_id.clone(),
                issue_id: attempt.issue_id.clone(),
                plan_id: plan.id.clone(),
                logical_work_item_id: "wi_a".to_string(),
                work_item_revision_id: revisions["wi_a"].id.clone(),
                dependency_logical_work_item_ids: Vec::new(),
                order_index: 1,
                status: CodingExecutionUnitStatus::Running,
            })
            .expect("coding unit a");
        let unit_b = store
            .create_coding_unit(CreateCodingExecutionUnitInput {
                attempt_id: attempt.id.clone(),
                project_id: attempt.project_id.clone(),
                issue_id: attempt.issue_id.clone(),
                plan_id: plan.id.clone(),
                logical_work_item_id: "wi_b".to_string(),
                work_item_revision_id: revisions["wi_b"].id.clone(),
                dependency_logical_work_item_ids: vec!["wi_x".to_string()],
                order_index: 2,
                status: CodingExecutionUnitStatus::Pending,
            })
            .expect("coding unit b");
        let mut seeded = store
            .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .unwrap();
        seeded.status = CodingAttemptStatus::Running;
        seeded.admission_ticket_consumed_at = Some(chrono::Utc::now().to_rfc3339());
        seeded.admission_kind = crate::product::coding_models::CodingAdmissionKind::ScAdvance;
        seeded.stage = crate::product::coding_models::CodingExecutionStage::ReviewRequest;
        seeded.active_unit_id = Some(unit_a.id.clone());
        seeded.current_work_item_id = Some("wi_a".to_string());
        store
            .write_coding_attempt_for_test(&seeded)
            .expect("seeded running sc advance attempt");
        store
            .create_coding_unit_run(
                &seeded,
                &CodingUnitRun {
                    id: "coding_unit_run_n5_parked_0001".to_string(),
                    unit_id: unit_a.id.clone(),
                    execution_no: 1,
                    work_item_revision_id: revisions["wi_a"].id.clone(),
                    resolved_handoff_revision_ids: Vec::new(),
                    canonical_contract_hash: bundles["wi_a"].canonical_contract_hash.clone(),
                    projection_bundle_id: bundles["wi_a"].id.clone(),
                    projection_compiler_version: bundles["wi_a"].compiler_version.clone(),
                    coder_provider_renderer_version: renderer_for(&ProviderName::Codex)
                        .renderer_version()
                        .to_string(),
                    reviewer_provider_renderer_version: renderer_for(&ProviderName::ClaudeCode)
                        .renderer_version()
                        .to_string(),
                    internal_reviewer_provider_renderer_version: None,
                    coder_projection_hash: bundles["wi_a"].coder_projection_hash.clone(),
                    reviewer_projection_hash: bundles["wi_a"].reviewer_projection_hash.clone(),
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

        N5ParkedRunnerFixture {
            _root: root,
            paths,
            store,
            attempt: seeded,
            unit_a,
            unit_b,
        }
    }

    fn seed_n5_parked_plan_run(fixture: &N5ParkedRunnerFixture) {
        // 空 target_attempts:完成挂点 drive 桥可运行但零推进(wi_x 完成事实
        // 不在任何绑定桶)——runner 对本 attempt 必须 park 而非穿到终审。
        let selection = CodingPlanSelection {
            unit_id: fixture.unit_a.id.clone(),
            logical_work_item_id: fixture.unit_a.logical_work_item_id.clone(),
            attempt_id: fixture.attempt.id.clone(),
            target_repository_id: "repository_0001".to_string(),
            start_command_id: CodingPlanSelection::deterministic_start_command_id(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                "work_item_plan_0001",
                "cmd_n5_parked_plan_run",
                "repository_0001",
            ),
        };
        let run = CodingPlanRun {
            project_id: fixture.attempt.project_id.clone(),
            issue_id: fixture.attempt.issue_id.clone(),
            plan_id: "work_item_plan_0001".to_string(),
            command_id: "cmd_n5_parked_plan_run".to_string(),
            bound_plan_revision_id: "plan_revision_n5_parked".to_string(),
            dependency_graph_revision_id: "dependency_graph_revision_n5_parked".to_string(),
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

    /// S2 parked 红测(切片C):2-unit sc_advance 桶,wi_b 依赖桶外 wi_x。
    /// 完成 wi_a 后:无 review_request 发出、无 git journal 写入、attempt
    /// 非 AMR(保持 Running park 形状:active 清空/stage 保持)、wi_b 物化
    /// 在场,上游未完成(wi_x 完成事实不在任何绑定桶)→ runner 干净退出。
    #[tokio::test]
    async fn sc_advance_completion_parks_cleanly_when_next_unit_waits_on_out_of_bucket_work_item() {
        let fixture = n5_sc_advance_parked_fixture();
        seed_n5_parked_plan_run(&fixture);
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
        .expect("parked completion must exit cleanly instead of dying at final confirm");
        drop(event_tx);

        let final_attempt = fixture
            .store
            .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("final attempt");
        assert_ne!(
            final_attempt.status,
            CodingAttemptStatus::AwaitingManualRecovery,
            "parked attempt must not land in manual recovery"
        );
        assert_eq!(
            final_attempt.status,
            CodingAttemptStatus::Running,
            "park keeps the attempt running for the entry-face continuation drive"
        );
        assert_eq!(
            final_attempt.stage,
            crate::product::coding_models::CodingExecutionStage::ReviewRequest,
            "park preserves the completion-chain entry stage"
        );
        assert!(
            final_attempt.active_unit_id.is_none(),
            "park clears the execution-authority pointer"
        );
        assert!(
            fixture
                .store
                .list_review_requests(
                    &final_attempt.project_id,
                    &final_attempt.issue_id,
                    &final_attempt.id
                )
                .expect("review requests")
                .is_empty(),
            "no review request may be issued for a parked attempt"
        );
        assert!(
            fixture
                .store
                .get_coding_git_operation(&final_attempt)
                .expect("git operation journal")
                .is_none(),
            "parked completion must not write a review-request git journal"
        );
        let units = fixture
            .store
            .list_coding_units(
                &final_attempt.project_id,
                &final_attempt.issue_id,
                &final_attempt.id,
            )
            .expect("units");
        assert_eq!(units.len(), 2, "both bucket units stay materialized");
        let unit_a = units
            .iter()
            .find(|unit| unit.id == fixture.unit_a.id)
            .expect("unit a");
        let unit_b = units
            .iter()
            .find(|unit| unit.id == fixture.unit_b.id)
            .expect("unit b");
        assert_eq!(unit_a.status, CodingExecutionUnitStatus::Completed);
        assert_eq!(
            unit_b.status,
            CodingExecutionUnitStatus::Pending,
            "waiting unit stays materialized and pending"
        );
        assert_eq!(unit_b.dependency_logical_work_item_ids, vec!["wi_x"]);
        assert_eq!(
            provider.starts(),
            0,
            "the parked exit path must not start a provider"
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
            "runner must emit session_state for the parked shape before exiting"
        );
    }

    /// Bug2-B(Task 4)：残留 open 门告警只标记严格早于当前 stage 的门；
    /// 同 stage 门不告警；纯读不写盘（告警绝不自动解门）。
    #[test]
    fn residual_open_gate_warnings_flag_only_earlier_stage_gates() {
        let root = tempfile::tempdir().expect("root");
        let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
        let attempt = store
            .create_attempt(
                crate::product::coding_attempt_store::CreateCodingAttemptInput {
                    project_id: "project_0001".to_string(),
                    issue_id: "issue_0001".to_string(),
                    work_item_id: "work_item_0001".to_string(),
                    base_branch: "HEAD".to_string(),
                    branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
                    worktree_path: None,
                    provider_config_snapshot: ProviderConfigSnapshot {
                        author: ProviderName::Fake,
                        reviewer: Some(ProviderName::Fake),
                        review_rounds: 1,
                        permission_modes: Default::default(),
                    },
                    target_snapshot: None,
                    max_auto_rework: 1,
                },
            )
            .expect("attempt");
        let gate_input = |stage: crate::product::coding_models::CodingExecutionStage,
                          reason: &str| {
            crate::product::coding_attempt_store::CreateBlockedGateInput {
                attempt_id: attempt.id.clone(),
                stage,
                node_id: None,
                role: Some(crate::product::coding_models::CodingProviderRole::CodeReviewer),
                title: "残留门".to_string(),
                description: "residual gate fixture".to_string(),
                reason_code: Some(reason.to_string()),
                evidence_refs: Vec::new(),
                raw_provider_output_ref: None,
                available_actions: Vec::new(),
            }
        };
        let earlier = store
            .create_blocked_gate(
                &attempt,
                gate_input(
                    crate::product::coding_models::CodingExecutionStage::Coding,
                    "residual_coding_gate",
                ),
            )
            .expect("earlier-stage gate");
        let same_stage = store
            .create_blocked_gate(
                &attempt,
                gate_input(
                    crate::product::coding_models::CodingExecutionStage::ReviewRequest,
                    "review_request_gate",
                ),
            )
            .expect("same-stage gate");
        let mut current = attempt.clone();
        current.stage = crate::product::coding_models::CodingExecutionStage::ReviewRequest;

        let warnings = super::residual_open_blocked_gate_warnings(&store, &current);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains(&earlier.gate_id));
        assert!(!warnings[0].contains(&same_stage.gate_id));

        // 早阶段门关闭后零告警；同 stage 门保持 open（告警纯读不写盘）。
        store
            .resolve_blocked_gate(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &earlier.gate_id,
            )
            .expect("close earlier gate");
        assert!(super::residual_open_blocked_gate_warnings(&store, &current).is_empty());
        assert_eq!(
            store
                .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
                .expect("open gates")
                .len(),
            1,
            "same-stage gate must remain open (warnings are read-only)"
        );
    }
}
