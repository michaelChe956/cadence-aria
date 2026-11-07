//! P2 GAP-E/G（Task 0.1）：失败 SingleCandidate 会话的人工显式重驱 REST。
//!
//! 独立于 human gate（不借 gate_id）；仅受理 durable Failed 的
//! WorkItemPlan/SingleCandidate/Interactive、最新失败节点且无活 manager run
//! 的现场；session 文件锁内冻结 `(failed_node_id, command_id)` 单发认领
//!（异键 409、同键读原结果），经现有 `start_generation` +
//! `spawn_provider_run_claiming_idle`（非 superseding）重驱同 session；
//! 派发副作用不明时保留认领并给人工分诊，绝不自动重试（GAP-H 裁决）。

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde_json::json;

use crate::product::issue_automation_store::IssueAutomationStore;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::{SingleCandidatePhase, WorkspaceSessionStatus, WorkspaceType};
use crate::product::work_item_plan_policy::{RunPolicy, WorkItemPlanFlowKind};
use crate::web::error::{ApiError, ApiResult};
use crate::web::handlers::support::{product_app_paths, product_store_api_error};
use crate::web::handlers::workspace_choice::resolve_session_manager;
use crate::web::state::WebAppState;
use crate::web::types::{RetryFailedScRunRequest, RetryFailedScRunStatus};
use crate::web::workspace_session::WorkspaceSessionManager;
use crate::web::workspace_ws_handler::{ProviderRunKind, spawn_provider_run_claiming_idle};
use crate::web::workspace_ws_types::{ProviderConfigSnapshot, TimelineNodeStatus};

pub async fn post_workspace_failed_sc_run_retry(
    State(state): State<WebAppState>,
    Path((session_id, failed_node_id)): Path<(String, String)>,
    Json(request): Json<RetryFailedScRunRequest>,
) -> ApiResult<(StatusCode, Json<RetryFailedScRunStatus>)> {
    if request.command_id.trim().is_empty() {
        return Err(ApiError::validation(
            "invalid_command_id",
            "command_id must not be blank",
        ));
    }
    if request.expected_phase != SingleCandidatePhase::Failed {
        return Err(ApiError::validation(
            "sc_recovery_rejected",
            "expected_phase must be failed for explicit recovery",
        ));
    }
    let lifecycle = LifecycleStore::new(product_app_paths(&state));
    let manager = resolve_session_manager(&state, &session_id).await?;

    // 同键重放优先：认领已冻结时读原结果，不再触发任何准入/派发。
    let record = lifecycle
        .get_workspace_session(&session_id)
        .map_err(product_store_api_error)?;
    if let Some(claim) = record.sc_recovery_claim.clone() {
        if claim.failed_node_id == failed_node_id && claim.command_id == request.command_id {
            let state_name = match claim.state {
                crate::product::models::ScRecoveryClaimState::NeedsHuman => "needs_human",
                crate::product::models::ScRecoveryClaimState::Accepted => "replayed",
            };
            return Ok((
                StatusCode::OK,
                Json(retry_status(&request, &failed_node_id, state_name)),
            ));
        }
        if record.single_candidate_phase == Some(SingleCandidatePhase::Failed) {
            return Err(ApiError::runtime(
                "sc_recovery_conflict",
                "failed single-candidate recovery was already claimed by another command",
                json!({ "session_id": session_id }),
            ));
        }
    }

    // durable 准入：Failed 的 SingleCandidate WorkItemPlan + Interactive。
    if record.workspace_type != WorkspaceType::WorkItemPlan
        || record.flow_kind != WorkItemPlanFlowKind::SingleCandidate
        || record.run_policy != RunPolicy::Interactive
        || record.single_candidate_phase != Some(SingleCandidatePhase::Failed)
        || record.status != WorkspaceSessionStatus::Failed
    {
        return Err(ApiError::runtime(
            "sc_recovery_rejected",
            "session is not a durably failed interactive SingleCandidate WorkItemPlan",
            json!({ "session_id": session_id }),
        ));
    }

    // 最新失败节点：failed_node_id 必须等于 durable timeline 中最新的 Failed 节点。
    let latest_failed_node = latest_failed_timeline_node(&lifecycle, &record);
    if latest_failed_node.as_deref() != Some(failed_node_id.as_str()) {
        return Err(ApiError::runtime(
            "sc_recovery_node_mismatch",
            "failed_node_id must reference the latest failed timeline node",
            json!({
                "session_id": session_id,
                "expected": latest_failed_node,
                "actual": failed_node_id,
            }),
        ));
    }

    // 无活 manager run（活 run 由其持有者驱动；重驱绝不 supersede）。
    if manager.is_active_run() {
        return Err(ApiError::runtime(
            "sc_recovery_busy",
            "session has an active provider run; explicit recovery must wait",
            json!({ "session_id": session_id }),
        ));
    }

    // session 文件锁内单发认领（异键 409、同键 Existing）。
    let claimed = lifecycle
        .claim_failed_sc_run_retry(&session_id, &failed_node_id, &request.command_id)
        .map_err(product_store_api_error)?;
    if let crate::product::lifecycle_store::ClaimScRecoveryOutcome::Existing(existing) = claimed {
        let state_name = match existing.sc_recovery_claim.as_ref().map(|claim| claim.state) {
            Some(crate::product::models::ScRecoveryClaimState::NeedsHuman) => "needs_human",
            _ => "replayed",
        };
        return Ok((
            StatusCode::OK,
            Json(retry_status(&request, &failed_node_id, state_name)),
        ));
    }

    // 现有 start_generation（SC Failed 重臂）+ 非 superseding 派发。
    let provider_config = ProviderConfigSnapshot {
        author: record.author_provider.clone(),
        // C2 Task 5（REQ-CRO-05）：三值透传——缺失保持空 effective。
        reviewer: record.reviewer_provider.clone(),
        review_rounds: record.review_rounds,
        permission_modes: record.permission_modes.clone(),
    };
    let reviewer_enabled = record.review_rounds > 0;
    let engine = manager.engine();
    let generation = {
        let mut locked = engine.lock().await;
        locked
            .start_generation(provider_config, reviewer_enabled)
            .await
    };
    let dispatch = match generation {
        Err(message) => Err(message),
        Ok(_) => {
            let mut run_context = manager.provider_run_context(state.workspace_runs.clone());
            run_context.connection_id = None;
            let (outbound_tx, _detached_outbound_rx) = tokio::sync::mpsc::channel(1);
            let run_kind =
                ProviderRunKind::work_item_plan_author_for_durable_flow(record.flow_kind);
            spawn_provider_run_claiming_idle(run_context, run_kind, outbound_tx).await
        }
    };
    match dispatch {
        Ok(true) => Ok((
            StatusCode::OK,
            Json(retry_status(&request, &failed_node_id, "accepted")),
        )),
        Ok(false) => {
            // 认领窗口内活 run 被他人启动：本命令未派发，交由其持有者驱动；
            // 认领保留，人工可观察，不隐式重试。
            mark_needs_human(
                &lifecycle,
                &session_id,
                &failed_node_id,
                &request.command_id,
            );
            Ok((
                StatusCode::OK,
                Json(retry_status(&request, &failed_node_id, "needs_human")),
            ))
        }
        Err(_message) => {
            // 派发外部副作用是否发生不可证明：保留认领、停人工分诊。
            mark_needs_human(
                &lifecycle,
                &session_id,
                &failed_node_id,
                &request.command_id,
            );
            Ok((
                StatusCode::OK,
                Json(retry_status(&request, &failed_node_id, "needs_human")),
            ))
        }
    }
}

fn retry_status(
    request: &RetryFailedScRunRequest,
    failed_node_id: &str,
    state: &str,
) -> RetryFailedScRunStatus {
    RetryFailedScRunStatus {
        command_id: request.command_id.clone(),
        failed_node_id: failed_node_id.to_string(),
        state: state.to_string(),
    }
}

fn mark_needs_human(
    lifecycle: &LifecycleStore,
    session_id: &str,
    failed_node_id: &str,
    command_id: &str,
) {
    if let Err(error) =
        lifecycle.mark_failed_sc_recovery_needs_human(session_id, failed_node_id, command_id)
    {
        eprintln!(
            "sc recovery needs-human mark failed for {session_id}: {error:?} (claim retained)"
        );
    }
}

fn latest_failed_timeline_node(
    lifecycle: &LifecycleStore,
    record: &crate::product::models::WorkspaceSessionRecord,
) -> Option<String> {
    let nodes = lifecycle
        .load_timeline_nodes_for_issue_session(&record.project_id, &record.issue_id, &record.id)
        .ok()?;
    nodes
        .iter()
        .filter(|node| node.status == TimelineNodeStatus::Failed)
        .max_by(|left, right| left.started_at.cmp(&right.started_at))
        .map(|node| node.node_id.clone())
}

// ─── 批 2 P4（REQ-WIGA-10/12）：plan 阶段失败继续与只读 control 投影 ───
//
// 停点分类（`PlanStop` / `resolve_plan_stop` / `plan_stop_key`）与 control
// 事实派生已**纯移动**到 `crate::web::plan_stop`（零行为变化）：HTTP control
// 与 WS `plan_continue_available` 两面共用同一份 durable 事实判定。

use crate::web::plan_stop::{PlanStop, plan_stop_key, resolve_plan_stop};

fn continue_status(
    request_command_id: &str,
    session_id: &str,
    state: &str,
    reason_code: Option<&str>,
) -> crate::web::types::AutomationContinuePlanStatus {
    crate::web::types::AutomationContinuePlanStatus {
        command_id: request_command_id.to_string(),
        state: state.to_string(),
        session_id: session_id.to_string(),
        reason_code: reason_code.map(str::to_string),
    }
}

/// 批 2 P4（REQ-WIGA-10）：plan 阶段失败继续（同一 plan 对话窗口补说明＋
/// 继续）。固定步骤：核当前授权及未 terminated → manager 互斥且无活 run →
/// 持久认领命令（同键同负载重放读首次结果、异负载拒绝）→ 写 context →
/// 按事实分支推进；派发前重验由 `start_plan_generation_once` 的准入承担。
/// 绝不自动改 Interactive、跳门或吞异常；收尾事实写失败不假成功。
pub async fn post_automation_continue_plan(
    State(state): State<WebAppState>,
    Path((project_id, issue_id)): Path<(String, String)>,
    Json(request): Json<crate::web::types::AutomationContinuePlanRequest>,
) -> ApiResult<(
    StatusCode,
    Json<crate::web::types::AutomationContinuePlanStatus>,
)> {
    if request.command_id.trim().is_empty() {
        return Err(ApiError::validation(
            "invalid_command_id",
            "command_id must not be blank",
        ));
    }
    let paths = product_app_paths(&state);
    let store = IssueAutomationStore::new(paths.clone());
    let lifecycle = LifecycleStore::new(paths.clone());
    let enrollment = store
        .get(&project_id, &issue_id)
        .map_err(product_store_api_error)?
        .ok_or_else(|| {
            ApiError::runtime(
                "automation_enrollment_not_found",
                "automation enrollment is missing for continue-plan",
                json!({ "project_id": project_id, "issue_id": issue_id }),
            )
        })?;
    // terminated（N0 纯字段非空）拒绝继续与派发，零副作用。
    if let Some(terminated_at) = enrollment.terminated_at.clone() {
        return Err(ApiError::runtime(
            "automation_plan_terminated",
            "automation plan is terminated; continue is rejected",
            json!({
                "terminated_at": terminated_at,
                "enrollment_id": enrollment.enrollment_id,
            }),
        ));
    }
    if !enrollment.enabled {
        return Err(ApiError::runtime(
            "automation_continue_rejected",
            "automation enrollment is disabled",
            json!({ "enrollment_id": enrollment.enrollment_id }),
        ));
    }
    let binding = crate::web::advance_plan::load_current_enrollment_binding(&enrollment).map_err(
        |message| {
            ApiError::runtime(
                "automation_continue_rejected",
                message,
                json!({ "enrollment_id": enrollment.enrollment_id }),
            )
        },
    )?;
    if request.expected_binding != binding {
        return Err(ApiError::runtime(
            "automation_continue_binding_mismatch",
            "expected_binding does not match the current enrollment binding",
            json!({ "enrollment_id": enrollment.enrollment_id }),
        ));
    }
    // 命令账本先行：同键同负载重放读首次 durable 结果（不重复写 note/派发，
    // 也不再重判当前停点/活 run——首次结果已是权威）；异负载 fail-closed。
    let digest = request.payload_digest();
    match store.find_command_result(&project_id, &issue_id, &request.command_id, &digest) {
        Ok(Some(entry)) => {
            let state_name =
                if entry.state == crate::product::models::automation::OperationState::NeedsHuman {
                    "needs_human"
                } else {
                    "replayed"
                };
            return Ok((
                StatusCode::OK,
                Json(continue_status(
                    &request.command_id,
                    &binding.session_id,
                    state_name,
                    None,
                )),
            ));
        }
        Ok(None) => {}
        Err(crate::product::models::automation::EnrollmentError::Conflict { .. }) => {
            return Err(ApiError::runtime(
                "automation_continue_command_conflict",
                "command_id was already recorded with a different payload",
                json!({ "command_id": request.command_id }),
            ));
        }
        Err(error) => {
            return Err(ApiError::runtime(
                "automation_continue_ledger_unavailable",
                format!("command ledger read failed: {error}"),
                json!({ "command_id": request.command_id }),
            ));
        }
    }
    let session_id = binding.session_id.clone();
    let manager = state
        .workspace_sessions
        .get_or_create(&session_id, || {
            WorkspaceSessionManager::create(&state, &session_id)
        })
        .await
        .map_err(|message| {
            ApiError::runtime(
                "automation_continue_manager_unavailable",
                message,
                json!({ "session_id": session_id }),
            )
        })?;
    if manager.is_active_run() {
        return Err(ApiError::runtime(
            "automation_continue_busy",
            "session has an active provider run; continue must wait",
            json!({ "session_id": session_id }),
        ));
    }
    let record = lifecycle
        .get_workspace_session(&session_id)
        .map_err(product_store_api_error)?;
    let intent_phase = crate::web::plan_generation::generation_intent(&state, &enrollment)
        .ok()
        .flatten()
        .and_then(|read| match read {
            crate::product::issue_automation_store::PlanGenerationIntentRead::Current(intent) => {
                Some(intent.phase)
            }
            crate::product::issue_automation_store::PlanGenerationIntentRead::LegacyUnbound {
                ..
            } => None,
        });
    let stop = resolve_plan_stop(&record, intent_phase);
    let Some(stop_key) = plan_stop_key(&stop, &enrollment.enrollment_id, &binding) else {
        return Err(ApiError::runtime(
            "automation_continue_not_stopped",
            "session has no continuable plan stop",
            json!({ "session_id": session_id, "status": record.status }),
        ));
    };
    if request.expected_stop_key != stop_key {
        return Err(ApiError::runtime(
            "automation_continue_stop_key_mismatch",
            "expected_stop_key does not match the current durable stop",
            json!({
                "session_id": session_id,
                "expected": request.expected_stop_key,
                "actual": stop_key,
            }),
        ));
    }
    let context = request
        .context
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string);
    // 评审超限（StoppedNeedsHuman）必须在对话输入区写说明再继续。
    if stop == PlanStop::StoppedHumanGate && context.is_none() {
        return Err(ApiError::validation(
            "automation_continue_context_required",
            "a non-empty context note is required to continue a stopped human gate",
        ));
    }
    // 批 2 P4 fix（k3 波3 发现1）：命令账本在收尾时落真实 durable 状态——
    // accepted 收尾写 Accepted、needs_human 收尾写 NeedsHuman（同文件 SC
    // retry 的 mark_needs_human 先例）；同键重放臂按首次 durable 状态分派，
    // 人工分诊停等不会被读成成功重放。append 同键同负载幂等返回既有条目
    //（首结果权威），异负载 Conflict（fail-closed）。
    let commit_ledger = |operation: crate::product::models::automation::OperationState| {
        store
            .append_command_result(
                &project_id,
                &issue_id,
                &request.command_id,
                &digest,
                operation,
            )
            .map(|entry| entry.state)
            .map_err(|error| {
                let code = if matches!(
                    error,
                    crate::product::models::automation::EnrollmentError::Conflict { .. }
                ) {
                    "automation_continue_command_conflict"
                } else {
                    "automation_continue_ledger_unavailable"
                };
                ApiError::runtime(
                    code,
                    format!("continue command ledger write failed: {error}"),
                    json!({ "command_id": request.command_id }),
                )
            })
    };
    // 回执状态＝首次 durable 结果（并发竞态下以先落盘的权威结果为准）。
    let ledger_state_name =
        |operation: crate::product::models::automation::OperationState| match operation {
            crate::product::models::automation::OperationState::NeedsHuman => "needs_human",
            _ => "accepted",
        };
    let needs_human = |reason_code: &str| -> Result<
        (
            StatusCode,
            Json<crate::web::types::AutomationContinuePlanStatus>,
        ),
        ApiError,
    > {
        let committed =
            commit_ledger(crate::product::models::automation::OperationState::NeedsHuman)?;
        Ok((
            StatusCode::OK,
            Json(continue_status(
                &request.command_id,
                &session_id,
                ledger_state_name(committed),
                Some(reason_code),
            )),
        ))
    };
    // 成功收尾：resolve 对应停点事实（通知消失）＋ wake 编排器；resolve 写
    // 失败不假成功（needs_human 收尾，账本同步落 NeedsHuman）。
    let finish_accepted = |keys: &[String]| -> Result<
        (
            StatusCode,
            Json<crate::web::types::AutomationContinuePlanStatus>,
        ),
        ApiError,
    > {
        for key in keys {
            if let Err(message) = crate::web::plan_generation::resolve_plan_attention_fact(
                &paths,
                &project_id,
                &issue_id,
                &binding,
                key,
            ) {
                return needs_human(&format!("automation_continue_resolve_failed: {message}"));
            }
        }
        let committed =
            commit_ledger(crate::product::models::automation::OperationState::Accepted)?;
        let _ = state.autopilot_wake.send(true);
        Ok((
            StatusCode::OK,
            Json(continue_status(
                &request.command_id,
                &session_id,
                ledger_state_name(committed),
                None,
            )),
        ))
    };

    match stop {
        PlanStop::FailedProvider => {
            // context 追加一次（写失败保留停点，不派发）。
            if let Some(context) = context.as_deref() {
                let engine = manager.engine();
                let mut locked = engine.lock().await;
                if let Err(message) = locked.append_context_note(context.to_string()).await {
                    drop(locked);
                    return needs_human(&format!(
                        "automation_continue_context_note_failed: {message}"
                    ));
                }
            }
            // 旧 generation intent 重置 Claimed（无 checkpoint 时交由
            // start_plan_generation_once 重新认领），再按真实启动推进。
            if let Err(error) = store.mark_plan_generation_phase(
                &project_id,
                &issue_id,
                &enrollment.enrollment_id,
                crate::product::models::automation::PlanGenerationPhase::Claimed,
            ) && !matches!(
                error,
                crate::product::models::automation::EnrollmentError::InvalidScope(_)
            ) {
                return needs_human("automation_continue_intent_reset_failed");
            }
            // SC 失败重臂（session 文件锁内 Failed→Prepare/Open）。
            let provider_config = ProviderConfigSnapshot {
                author: record.author_provider.clone(),
                reviewer: record.reviewer_provider.clone(),
                review_rounds: record.review_rounds,
                permission_modes: record.permission_modes.clone(),
            };
            let reviewer_enabled = record.review_rounds > 0;
            {
                let engine = manager.engine();
                let mut locked = engine.lock().await;
                if let Err(message) = locked
                    .start_generation(provider_config, reviewer_enabled)
                    .await
                {
                    drop(locked);
                    return needs_human(&format!("automation_continue_rearm_failed: {message}"));
                }
            }
            match crate::web::plan_generation::start_plan_generation_once(&state, &enrollment).await
            {
                Ok(crate::web::plan_generation::PlanGenerationOutcome::Running) => {
                    finish_accepted(&[
                        crate::web::plan_generation::plan_failed_node_fact_key(&session_id),
                        crate::web::plan_generation::plan_generation_failed_fact_key(
                            &enrollment.enrollment_id,
                            &binding.plan_id,
                        ),
                    ])
                }
                _ => needs_human("automation_continue_dispatch_not_running"),
            }
        }
        PlanStop::StoppedHumanGate => {
            let context = context.expect("context validated above");
            // 同 session CAS StoppedNeedsHuman→WaitingForHuman（预算已零只开
            // 一个新说明回合），内存恢复 HumanConfirm 后走既有候选修订。
            let saved = match lifecycle.reopen_stopped_needs_human_for_bound_continuation(&record) {
                Ok(saved) => saved,
                Err(_) => {
                    return needs_human("automation_continue_gate_reopen_conflict");
                }
            };
            {
                let engine = manager.engine();
                let mut locked = engine.lock().await;
                locked.reload_session_from_record(saved);
            }
            let effect = crate::web::workspace_ws_handler::apply_human_gate_feedback(
                manager.engine(),
                crate::product::workspace_engine::HumanGateFeedbackInput {
                    command_id: request.command_id.clone(),
                    feedback: context,
                },
            )
            .await;
            match effect {
                crate::web::workspace_ws_handler::HumanGateFeedbackEffect::TurnOpened {
                    turn,
                    prompt,
                    ..
                } => {
                    // 已有 human-gate run 派发（唯一 manager run 面，非 superseding）。
                    let mut run_context =
                        manager.provider_run_context(state.workspace_runs.clone());
                    run_context.connection_id = None;
                    let (outbound_tx, _dropped_outbound_rx) = tokio::sync::mpsc::channel(1);
                    match crate::web::workspace_ws_handler::spawn_provider_run_claiming_idle(
                        run_context,
                        ProviderRunKind::HumanGateScManualRevision {
                            turn_id: turn.turn_id,
                            prompt,
                        },
                        outbound_tx,
                    )
                    .await
                    {
                        Ok(true) => {
                            finish_accepted(&[crate::web::plan_generation::plan_gate_fact_key(
                                &session_id,
                            )])
                        }
                        Ok(false) => needs_human("automation_continue_gate_run_busy"),
                        Err(message) => {
                            needs_human(&format!("automation_continue_gate_run_failed: {message}"))
                        }
                    }
                }
                crate::web::workspace_ws_handler::HumanGateFeedbackEffect::Replayed { .. } => {
                    needs_human("automation_continue_gate_turn_replayed")
                }
                crate::web::workspace_ws_handler::HumanGateFeedbackEffect::Busy { .. } => {
                    needs_human("automation_continue_gate_busy")
                }
                crate::web::workspace_ws_handler::HumanGateFeedbackEffect::Rejected {
                    code,
                    ..
                } => needs_human(&format!("automation_continue_gate_rejected: {code}")),
                crate::web::workspace_ws_handler::HumanGateFeedbackEffect::EngineError(message) => {
                    needs_human(&format!("automation_continue_gate_engine_error: {message}"))
                }
            }
        }
        PlanStop::GenerationNeedsHuman => {
            // 旧 intent 重置 Claimed 再按真实启动推进（零重复 provider：spawn
            // 仍走 claim-idle 单飞）。
            if let Err(error) = store.mark_plan_generation_phase(
                &project_id,
                &issue_id,
                &enrollment.enrollment_id,
                crate::product::models::automation::PlanGenerationPhase::Claimed,
            ) && !matches!(
                error,
                crate::product::models::automation::EnrollmentError::InvalidScope(_)
            ) {
                return needs_human("automation_continue_intent_reset_failed");
            }
            match crate::web::plan_generation::start_plan_generation_once(&state, &enrollment).await
            {
                Ok(crate::web::plan_generation::PlanGenerationOutcome::Running) => {
                    finish_accepted(&[
                        crate::web::plan_generation::plan_generation_failed_fact_key(
                            &enrollment.enrollment_id,
                            &binding.plan_id,
                        ),
                    ])
                }
                _ => needs_human("automation_continue_dispatch_not_running"),
            }
        }
        PlanStop::WaitingExisting | PlanStop::None => {
            needs_human("automation_continue_not_stopped")
        }
    }
}

/// 批 2 P4（REQ-WIGA-10）：只读 control 管理投影。按 durable session/门/
/// 生成检查点及 N0 持久事实判定真实 stop_key/stage/allowed_actions/
/// context_required；初版 allowed_actions 只含 continue_plan 与既有门动作，
/// 不含 takeover；GET 不写任何事实。
pub async fn get_automation_enrollment_control(
    State(state): State<WebAppState>,
    Path((project_id, issue_id)): Path<(String, String)>,
) -> ApiResult<Json<crate::web::types::AutomationControlStatus>> {
    let unmanaged = || crate::web::types::AutomationControlStatus {
        managed: false,
        binding: None,
        plan_id: None,
        session_id: None,
        terminated_at: None,
        stop_key: None,
        stage: "unmanaged".to_string(),
        allowed_actions: Vec::new(),
        context_required: false,
    };
    let paths = product_app_paths(&state);
    let store = IssueAutomationStore::new(paths.clone());
    let Some(enrollment) = store
        .get(&project_id, &issue_id)
        .map_err(product_store_api_error)?
    else {
        return Ok(Json(unmanaged()));
    };
    let binding = if enrollment.enabled {
        crate::web::advance_plan::load_current_enrollment_binding(&enrollment).ok()
    } else {
        None
    };
    let Some(binding) = binding else {
        return Ok(Json(unmanaged()));
    };
    let lifecycle = LifecycleStore::new(paths.clone());
    // 绑定会话已被删除（实体级联删除清盘，NotFound）→ 降级为 unmanaged
    // 投影，不把已删 id 抛成 404；真实 IO/JSON 错误仍上抛。
    let record = match lifecycle.get_workspace_session(&binding.session_id) {
        Ok(record) => record,
        Err(crate::product::json_store::ProductStoreError::NotFound { .. }) => {
            return Ok(Json(unmanaged()));
        }
        Err(other) => return Err(product_store_api_error(other)),
    };
    let intent_phase = crate::web::plan_stop::generation_intent_phase(&paths, &enrollment);
    // control 事实与 WS `plan_continue_available` **同源**（同一
    // `plan_control_facts` 派生）——两面不可能互相矛盾。
    let facts = crate::web::plan_stop::plan_control_facts(Some(&enrollment), &record, intent_phase);
    Ok(Json(crate::web::types::AutomationControlStatus {
        managed: facts.managed,
        binding: facts.binding,
        plan_id: facts.plan_id,
        session_id: facts.session_id,
        terminated_at: facts.terminated_at,
        stop_key: facts.stop_key,
        stage: facts.stage,
        allowed_actions: facts.allowed_actions,
        context_required: facts.context_required,
    }))
}

#[cfg(test)]
mod tests {
    use crate::cross_cutting::provider_adapter::ProviderAdapterError;
    use crate::product::lifecycle_store::LifecycleStore;
    use crate::product::models::{ProviderName, SingleCandidatePhase, WorkspaceSessionStatus};
    use crate::product::work_item_plan_policy::RunPolicy;
    use crate::web::state::WebAppState;
    use crate::web::wiga_gate_fixture::EnrolledGateFixture;
    use crate::web::workspace_session::WorkspaceSessionManager;
    use axum::http::StatusCode;
    use tower::ServiceExt;

    async fn post_json(
        app: &axum::Router,
        uri: String,
        body: &serde_json::Value,
    ) -> (StatusCode, serde_json::Value) {
        let response = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method(axum::http::Method::POST)
                    .uri(uri)
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(axum::body::Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let payload = if bytes.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
        };
        (status, payload)
    }

    fn retry_uri(session_id: &str, failed_node_id: &str) -> String {
        format!("/api/workspace-sessions/{session_id}/failed-sc-runs/{failed_node_id}/retry")
    }

    fn durable(
        state: &WebAppState,
        session_id: &str,
    ) -> crate::product::models::WorkspaceSessionRecord {
        let paths =
            crate::product::app_paths::ProductAppPaths::new(state.workspace_root.join(".aria"));
        LifecycleStore::new(paths)
            .get_workspace_session(session_id)
            .expect("durable session")
    }

    /// SC Evaluate reviewer 失败收敛 durable Failed 后，人工显式重驱：
    /// 活 run 前置 409、错误节点 409、正确节点 200 accepted（真实 provider
    /// 启动落账 +1）、重复同键 replayed 不再增加 provider_start_ledger。
    #[tokio::test]
    async fn retry_failed_sc_run_requires_latest_failed_node() {
        let fixture = EnrolledGateFixture::new().await;
        let state = fixture.state.clone();
        let session_id = fixture.session_id.clone();
        let manager = state
            .workspace_sessions
            .get_or_create(&session_id, || {
                WorkspaceSessionManager::create(&state, &session_id)
            })
            .await
            .expect("registry manager");

        // 构造 SC Evaluate reviewer 失败现场（真实 manager engine 面）：
        // durable phase=Evaluate + 引擎投影同步 + 真实 start_review/驱动失败。
        let failed_node_id = {
            let engine_arc = manager.engine();
            let mut engine = engine_arc.lock().await;
            let lifecycle = LifecycleStore::new(crate::product::app_paths::ProductAppPaths::new(
                state.workspace_root.join(".aria"),
            ));
            let mut record = lifecycle
                .get_workspace_session(&session_id)
                .expect("load session");
            record.flow_kind =
                crate::product::work_item_plan_policy::WorkItemPlanFlowKind::SingleCandidate;
            record.run_policy = RunPolicy::Interactive;
            record.single_candidate_phase = Some(SingleCandidatePhase::Evaluate);
            crate::product::json_store::write_json(&fixture.session_path(&record.id), &record)
                .expect("persist evaluate session");
            engine.session =
                crate::product::workspace_engine::WorkspaceSession::from_record(record);
            // 评审失败现场要求非 Fake reviewer（Fake 是快速跳过路径）。
            engine.session.reviewer_provider = Some(ProviderName::ClaudeCode);
            engine.start_review().await;
            let failed_node_id = engine.active_node_id.clone().unwrap();
            let (_tx, rx) = tokio::sync::mpsc::channel(1);
            engine
                .drive_reviewer_provider_session(
                    Err(ProviderAdapterError::provider_unavailable(
                        "503 No available accounts",
                    )),
                    rx,
                    ProviderName::ClaudeCode,
                )
                .await;
            failed_node_id
        };
        {
            let record = durable(&state, &session_id);
            assert_eq!(
                record.single_candidate_phase,
                Some(SingleCandidatePhase::Failed)
            );
            assert_eq!(record.status, WorkspaceSessionStatus::Failed);
        }

        let app = crate::web::app::build_web_router(state.clone());
        let body = serde_json::json!({
            "command_id": "cmd-sc-retry-1",
            "expected_phase": "failed",
        });

        // 前置 1：fixture 的生成 run 仍注册为活 run → 409 busy，零副作用。
        assert!(manager.is_active_run());
        let (status, payload) =
            post_json(&app, retry_uri(&session_id, &failed_node_id), &body).await;
        assert_eq!(status, StatusCode::CONFLICT, "busy run must 409: {payload}");
        assert!(manager.abort_active_run().await, "abort fixture run");

        // 前置 2：错误失败节点 → 409，零副作用。
        let (status, payload) = post_json(
            &app,
            retry_uri(&session_id, "node_not_a_failed_node"),
            &body,
        )
        .await;
        assert_eq!(
            status,
            StatusCode::CONFLICT,
            "non-latest failed node must 409: {payload}"
        );
        assert_eq!(
            durable(&state, &session_id).single_candidate_phase,
            Some(SingleCandidatePhase::Failed),
            "rejected retries must not mutate durable phase"
        );

        // 正确节点：200 accepted。
        let ledger_before = durable(&state, &session_id).provider_start_ledger.len();
        let (status, payload) =
            post_json(&app, retry_uri(&session_id, &failed_node_id), &body).await;
        assert_eq!(status, StatusCode::OK, "correct node retry: {payload}");
        assert_eq!(payload["state"], "accepted", "{payload}");
        assert_eq!(payload["command_id"], "cmd-sc-retry-1", "{payload}");

        // 真实重驱必须恰好新落一条 provider start（有界等待后台 run 落账）。
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let count = durable(&state, &session_id).provider_start_ledger.len();
            if count == ledger_before + 1 {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "accepted retry must persist exactly one provider start"
            );
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }

        // 重复同键：replayed 读原结果，不增加 provider_start_ledger。
        let (status, payload) =
            post_json(&app, retry_uri(&session_id, &failed_node_id), &body).await;
        assert_eq!(status, StatusCode::OK, "same-key replay: {payload}");
        assert_eq!(payload["state"], "replayed", "{payload}");
        assert_eq!(
            durable(&state, &session_id).provider_start_ledger.len(),
            ledger_before + 1,
            "same-key replay must not start a second provider run"
        );

        // 异键 → 409（认领已被 (failed_node_id, cmd-sc-retry-1) 冻结）。
        let other = serde_json::json!({
            "command_id": "cmd-sc-retry-2",
            "expected_phase": "failed",
        });
        let (status, payload) =
            post_json(&app, retry_uri(&session_id, &failed_node_id), &other).await;
        assert_eq!(
            status,
            StatusCode::CONFLICT,
            "different command on claimed session must 409: {payload}"
        );
    }

    // ─── 批 2 P4（REQ-WIGA-10/12）：continue-plan/control/plan 事实红测 ───

    include!("workspace_sc_recovery/b2_p4.inc.rs");
}
