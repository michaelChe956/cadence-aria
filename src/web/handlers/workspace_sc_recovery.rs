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

/// durable 事实派生的 plan 停点分类：continue-plan 按此分支，stop_key 按
/// 同一分类派生（continue 与 control 同源，防把旧说明送给新停点）。
#[derive(Debug, Clone, PartialEq, Eq)]
enum PlanStop {
    /// 会话 durable Failed（SC phase Failed；provider 失败收敛）。
    FailedProvider,
    /// 评审超限/人工停点（StoppedNeedsHuman + resumable 门快照）。
    StoppedHumanGate,
    /// 生成检查点派发前失败（intent NeedsHuman、无活 run）。
    GenerationNeedsHuman,
    /// 既有 choice/门等待（照旧走既有端点，continue 不受理）。
    WaitingExisting,
    /// 无可继续停点（活跃/终态/未停）。
    None,
}

fn resolve_plan_stop(
    record: &crate::product::models::WorkspaceSessionRecord,
    intent_phase: Option<crate::product::models::automation::PlanGenerationPhase>,
) -> PlanStop {
    if record.workspace_type != WorkspaceType::WorkItemPlan {
        return PlanStop::None;
    }
    match record.status {
        WorkspaceSessionStatus::Failed => {
            if record.single_candidate_phase == Some(SingleCandidatePhase::Failed) {
                PlanStop::FailedProvider
            } else {
                PlanStop::None
            }
        }
        WorkspaceSessionStatus::StoppedNeedsHuman => {
            if record
                .human_gate_snapshot
                .as_ref()
                .is_some_and(|snapshot| snapshot.resumable)
            {
                PlanStop::StoppedHumanGate
            } else {
                PlanStop::None
            }
        }
        WorkspaceSessionStatus::WaitingForHuman | WorkspaceSessionStatus::ChangeRequested => {
            PlanStop::WaitingExisting
        }
        _ => {
            if intent_phase
                == Some(crate::product::models::automation::PlanGenerationPhase::NeedsHuman)
            {
                PlanStop::GenerationNeedsHuman
            } else {
                PlanStop::None
            }
        }
    }
}

fn plan_stop_key(
    stop: &PlanStop,
    enrollment_id: &str,
    binding: &crate::product::models::automation::EnrollmentBindingIdentity,
) -> Option<String> {
    match stop {
        PlanStop::FailedProvider => Some(crate::web::plan_generation::plan_failed_node_fact_key(
            &binding.session_id,
        )),
        PlanStop::StoppedHumanGate => Some(crate::web::plan_generation::plan_gate_fact_key(
            &binding.session_id,
        )),
        PlanStop::GenerationNeedsHuman => Some(
            crate::web::plan_generation::plan_generation_failed_fact_key(
                enrollment_id,
                &binding.plan_id,
            ),
        ),
        PlanStop::WaitingExisting | PlanStop::None => None,
    }
}

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
    let terminated_at = enrollment.terminated_at.clone();
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
    let stop_key = plan_stop_key(&stop, &enrollment.enrollment_id, &binding);
    let (stage, allowed_actions, context_required) = if terminated_at.is_some() {
        ("terminated".to_string(), Vec::new(), false)
    } else {
        match &stop {
            PlanStop::FailedProvider => (
                "failed".to_string(),
                vec!["continue_plan".to_string()],
                false,
            ),
            PlanStop::StoppedHumanGate => (
                "stopped_needs_human".to_string(),
                vec!["continue_plan".to_string()],
                true,
            ),
            PlanStop::GenerationNeedsHuman => (
                "generation_needs_human".to_string(),
                vec!["continue_plan".to_string()],
                false,
            ),
            PlanStop::WaitingExisting => (
                "waiting_for_human".to_string(),
                vec![
                    "choice_response".to_string(),
                    "human_gate_feedback".to_string(),
                    "human_gate_close".to_string(),
                ],
                false,
            ),
            PlanStop::None => (serde_stage(&record.status), Vec::new(), false),
        }
    };
    Ok(Json(crate::web::types::AutomationControlStatus {
        managed: true,
        binding: Some(binding),
        plan_id: enrollment.plan_id.clone(),
        session_id: enrollment.session_id.clone(),
        terminated_at,
        stop_key,
        stage,
        allowed_actions,
        context_required,
    }))
}

/// `WorkspaceSessionStatus` 的稳定 stage 投影（与 wire snake_case 同口径）。
fn serde_stage(status: &WorkspaceSessionStatus) -> String {
    serde_json::to_value(status)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_else(|| "unknown".to_string())
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

    mod b2_p4 {
        use super::*;
        use crate::product::issue_automation_store::IssueAutomationStore;
        use crate::product::models::automation::{
            AutomationAttentionFact, AutomationAttentionKind, AutomationAttentionStage,
            EnrollmentOptions, EnrollmentSource, EnrollmentWriteCommand, PlanApprovalPolicy,
            PlanGenerationPhase,
        };
        use crate::product::models::{WorkspaceMessageRecord, WorkspaceType};
        use crate::product::work_item_plan_policy::{
            HumanGateSnapshot, HumanReason, RunPolicy, WorkItemPlanFlowKind,
        };
        use crate::web::handlers::automation_enrollment_test_support::{
            ISSUE_ID, PROJECT_ID, seed_fixture,
        };

        struct ContinueFixture {
            _root: tempfile::TempDir,
            state: WebAppState,
            lifecycle: LifecycleStore,
            store: IssueAutomationStore,
            binding: crate::product::models::automation::EnrollmentBindingIdentity,
            enrollment_id: String,
            plan_id: String,
            session_id: String,
        }

        impl ContinueFixture {
            fn enrollment(&self) -> crate::product::models::automation::IssueAutomationEnrollment {
                self.store
                    .get(PROJECT_ID, ISSUE_ID)
                    .expect("read enrollment")
                    .expect("enrollment present")
            }

            fn record(&self) -> crate::product::models::WorkspaceSessionRecord {
                self.lifecycle
                    .get_workspace_session(&self.session_id)
                    .expect("durable record")
            }

            fn write_record(&self, record: &crate::product::models::WorkspaceSessionRecord) {
                crate::product::json_store::write_json(
                    &self
                        .lifecycle
                        .app_paths()
                        .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
                        .join("workspace-sessions")
                        .join(format!("{}.json", record.id)),
                    record,
                )
                .expect("persist session json");
            }

            fn binding_json(&self) -> serde_json::Value {
                serde_json::to_value(&self.binding).expect("binding json")
            }

            async fn manager(
                &self,
            ) -> std::sync::Arc<crate::web::workspace_session::WorkspaceSessionManager>
            {
                self.state
                    .workspace_sessions
                    .get_or_create(&self.session_id, || {
                        crate::web::workspace_session::WorkspaceSessionManager::create(
                            &self.state,
                            &self.session_id,
                        )
                    })
                    .await
                    .expect("manager")
            }
        }

        /// gate-passing WorkItemPlan 候选（唯一通过 artifact fence 选择器，
        /// 供修订回合取候选基线）。
        fn candidate_message() -> WorkspaceMessageRecord {
            let markdown = "# Work Item Plan\n\n\
                ## 计划范围\n覆盖 provider 失败后的自动化继续。source id: story_spec_0001。\n\n\
                ## 任务拆分\n- [TASK-001] 恢复链。\n\n\
                ## 依赖图\n无。\n\n\
                ## 验证计划\ncargo test --locked。\n\n\
                ## 执行顺序\n先恢复。\n\n\
                ## 风险\n无。\n\n\
                ## 追踪关系\nsource ids: Story Spec story_spec_0001, Design Spec design_spec_0001。\n\
                [TASK-001] -> [REQ-001]\n";
            WorkspaceMessageRecord {
                role: "assistant".to_string(),
                content: format!("```artifact\n{markdown}```\n"),
                created_at: "2026-10-16T00:00:00Z".to_string(),
            }
        }

        fn gate_snapshot(budget: u32) -> HumanGateSnapshot {
            HumanGateSnapshot {
                candidate_recovery: None,
                findings: Vec::new(),
                repeated_fingerprints: Vec::new(),
                attempts_used: 0,
                manual_repairs_remaining: budget,
                trigger: HumanReason::NativeHumanRequired,
                resumable: true,
                accepted_feedback_turns: Some(0),
            }
        }

        /// auto 授权 + AutoIfValid SC 会话 + 精确绑定的最小 durable 现场。
        async fn continue_fixture(
            approval: PlanApprovalPolicy,
            status: crate::product::models::WorkspaceSessionStatus,
            phase: crate::product::models::SingleCandidatePhase,
            snapshot: Option<HumanGateSnapshot>,
            with_candidate: bool,
        ) -> ContinueFixture {
            let inner = seed_fixture(1, true);
            let root_path = inner._root.path().to_path_buf();
            let state = WebAppState::new(
                root_path.clone(),
                crate::web::runtime::WebRuntime::new_fake(root_path.clone()),
            );
            let paths = crate::product::app_paths::ProductAppPaths::new(root_path.join(".aria"));
            let store = IssueAutomationStore::new(paths.clone());
            let lifecycle = LifecycleStore::new(paths.clone());
            store
                .compare_and_set(
                    PROJECT_ID,
                    ISSUE_ID,
                    None,
                    EnrollmentWriteCommand::Enable {
                        selection_key: "continue-fixture".to_string(),
                        source: EnrollmentSource {
                            stories: Vec::new(),
                            designs: Vec::new(),
                        },
                        options: EnrollmentOptions {
                            author_provider: crate::product::models::ProviderName::Fake,
                            reviewer_provider: crate::product::models::ProviderName::Fake,
                            review_rounds: 0,
                            superpowers_enabled: false,
                            openspec_enabled: false,
                            plan_approval: approval,
                            plan_options: crate::product::models::IssueWorkItemPlanOptions {
                                include_integration_tests: false,
                                include_e2e_tests: false,
                                force_frontend_backend_split: false,
                                require_execution_plan_confirm: false,
                            },
                        },
                        target:
                            crate::product::logical_codebase::EnrollmentTarget::SingleRepository {
                                repository_id: "repo-1".to_string(),
                            },
                    },
                )
                .expect("enable enrollment");
            let plan_id = "issue_work_item_plan_continue_1".to_string();
            // manager 创建前置（ensure_workspace_context_message）需要 plan 记录。
            lifecycle
                .create_issue_work_item_plan(
                    crate::product::lifecycle_store::CreateIssueWorkItemPlanInput {
                        id: Some(plan_id.clone()),
                        project_id: PROJECT_ID.to_string(),
                        issue_id: ISSUE_ID.to_string(),
                        source_story_spec_ids: vec![inner.story_id.clone()],
                        source_design_spec_ids: Vec::new(),
                        options: crate::product::models::IssueWorkItemPlanOptions {
                            include_integration_tests: false,
                            include_e2e_tests: false,
                            force_frontend_backend_split: false,
                            require_execution_plan_confirm: false,
                        },
                        status: crate::product::models::IssueWorkItemPlanStatus::Draft,
                        work_item_ids: Vec::new(),
                        repository_profile_ref: None,
                        verification_plan_ids: Vec::new(),
                        dependency_graph: Vec::new(),
                        created_from_provider_run: None,
                        validator_findings: Vec::new(),
                    },
                )
                .expect("create plan record");
            let mut record = lifecycle
                .create_workspace_session(
                    crate::product::lifecycle_store::CreateWorkspaceSessionInput {
                        project_id: PROJECT_ID.to_string(),
                        issue_id: ISSUE_ID.to_string(),
                        entity_id: plan_id.clone(),
                        workspace_type: WorkspaceType::WorkItemPlan,
                        author_provider: crate::product::models::ProviderName::Fake,
                        reviewer_provider: None,
                        review_rounds: 0,
                        superpowers_enabled: false,
                        openspec_enabled: false,
                        work_item_plan_options: Some(
                            crate::product::lifecycle_store::WorkItemPlanSessionOptions {
                                flow_kind: WorkItemPlanFlowKind::SingleCandidate,
                                run_policy: RunPolicy::AutoIfValid,
                                rollout_snapshot: false,
                            },
                        ),
                    },
                )
                .expect("create session");
            record.status = status;
            record.single_candidate_phase = Some(phase);
            record.human_gate_snapshot = snapshot;
            if with_candidate {
                record.messages = vec![candidate_message()];
            }
            crate::product::json_store::write_json(
                &lifecycle
                    .app_paths()
                    .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
                    .join("workspace-sessions")
                    .join(format!("{}.json", record.id)),
                &record,
            )
            .expect("persist session json");
            store
                .bind_plan(PROJECT_ID, ISSUE_ID, 1, &plan_id, &record.id)
                .expect("bind plan");
            let enrollment = store
                .get(PROJECT_ID, ISSUE_ID)
                .expect("read enrollment")
                .expect("enrollment present");
            let binding = crate::web::advance_plan::load_current_enrollment_binding(&enrollment)
                .expect("current binding");
            ContinueFixture {
                _root: inner._root,
                state,
                lifecycle,
                store,
                binding,
                enrollment_id: enrollment.enrollment_id.clone(),
                plan_id,
                session_id: record.id,
            }
        }

        fn continue_uri() -> String {
            format!(
                "/api/projects/{PROJECT_ID}/issues/{ISSUE_ID}/automation-enrollment/continue-plan"
            )
        }

        fn control_uri() -> String {
            format!("/api/projects/{PROJECT_ID}/issues/{ISSUE_ID}/automation-enrollment/control")
        }

        async fn get_json(app: &axum::Router, uri: String) -> (StatusCode, serde_json::Value) {
            let response = app
                .clone()
                .oneshot(
                    axum::http::Request::builder()
                        .method(axum::http::Method::GET)
                        .uri(uri)
                        .body(axum::body::Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            let status = response.status();
            let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            let payload = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
            (status, payload)
        }

        async fn wait_for<
            F: Fn(&crate::product::models::automation::IssueAutomationEnrollment) -> bool,
        >(
            fixture: &ContinueFixture,
            predicate: F,
            label: &str,
        ) {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            loop {
                let enrollment = fixture.enrollment();
                if predicate(&enrollment) {
                    return;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "timed out waiting for {label}"
                );
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        }

        /// TDD ①⑨：Failed 复用 SC 失败重臂＋start_generation；旧 intent 重置
        /// Claimed 再按真实启动推进；继续成功 resolve 停点事实＋wake。
        #[tokio::test]
        async fn continue_plan_failed_session_rearms_resets_intent_and_resolves_fact() {
            let fixture = continue_fixture(
                PlanApprovalPolicy::Auto,
                crate::product::models::WorkspaceSessionStatus::Failed,
                crate::product::models::SingleCandidatePhase::Failed,
                None,
                false,
            )
            .await;
            // 预置旧 generation intent（ProviderDispatched——恢复后必须先重置）。
            fixture
                .store
                .claim_plan_generation(PROJECT_ID, ISSUE_ID, &fixture.enrollment_id)
                .expect("claim intent");
            fixture
                .store
                .mark_plan_generation_phase(
                    PROJECT_ID,
                    ISSUE_ID,
                    &fixture.enrollment_id,
                    PlanGenerationPhase::ProviderDispatched,
                )
                .expect("stale intent phase");
            // 预置 router 已写的 provider 失败事实（继续成功即 resolve）。
            let failed_key =
                crate::web::plan_generation::plan_failed_node_fact_key(&fixture.session_id);
            fixture
                .store
                .upsert_automation_attention_fact(
                    PROJECT_ID,
                    ISSUE_ID,
                    &fixture.binding,
                    AutomationAttentionFact {
                        key: failed_key.clone(),
                        kind: AutomationAttentionKind::AutomationFailed,
                        stage: AutomationAttentionStage::Plan,
                        plan_id: Some(fixture.plan_id.clone()),
                        session_id: Some(fixture.session_id.clone()),
                        attempt_id: None,
                        reason_code: "plan_provider_failed".to_string(),
                        occurred_at: "2026-10-16T00:00:00Z".to_string(),
                        choice_id: None,
                        gate_id: None,
                    },
                )
                .expect("seed failed fact");

            let app = crate::web::app::build_web_router(fixture.state.clone());
            let body = serde_json::json!({
                "command_id": "cmd-continue-1",
                "expected_binding": fixture.binding_json(),
                "expected_stop_key": failed_key,
            });
            let (status, payload) = post_json(&app, continue_uri(), &body).await;
            assert_eq!(status, StatusCode::OK, "continue accepted: {payload}");
            assert_eq!(payload["state"], "accepted", "{payload}");
            assert_eq!(payload["session_id"], fixture.session_id, "{payload}");

            // 旧 intent 已按真实启动推进（越过 Claimed；Fake run 真实落账）。
            let ledger_before = fixture.record().provider_start_ledger.len();
            wait_for(
                &fixture,
                |_| fixture.record().provider_start_ledger.len() > ledger_before,
                "provider start after continue",
            )
            .await;
            let enrollment = fixture.enrollment();
            let intent =
                crate::web::plan_generation::generation_intent(&fixture.state, &enrollment)
                    .expect("intent read")
                    .expect("intent present");
            match intent {
                crate::product::issue_automation_store::PlanGenerationIntentRead::Current(
                    intent,
                ) => {
                    assert!(
                        matches!(
                            intent.phase,
                            PlanGenerationPhase::EngineStarted
                                | PlanGenerationPhase::ProviderDispatched
                        ),
                        "intent must be re-driven from Claimed, got {:?}",
                        intent.phase
                    );
                }
                crate::product::issue_automation_store::PlanGenerationIntentRead::LegacyUnbound {
                    ..
                } => panic!("legacy intent without target cannot drive automatic chains"),
            }
            // 继续成功 resolve：失败事实消失。
            wait_for(
                &fixture,
                |enrollment| {
                    !enrollment
                        .attention_facts
                        .iter()
                        .any(|fact| fact.key == failed_key)
                },
                "failed fact resolved",
            )
            .await;
        }

        /// TDD ④：同键同负载重放读首次结果（零重复派发）；同键异负载 409；
        /// 停点键不符 409 且零副作用（无账本项）。
        #[tokio::test]
        async fn continue_plan_command_ledger_replay_and_conflict() {
            let fixture = continue_fixture(
                PlanApprovalPolicy::Auto,
                crate::product::models::WorkspaceSessionStatus::Failed,
                crate::product::models::SingleCandidatePhase::Failed,
                None,
                false,
            )
            .await;
            fixture
                .store
                .claim_plan_generation(PROJECT_ID, ISSUE_ID, &fixture.enrollment_id)
                .expect("claim intent");
            let failed_key =
                crate::web::plan_generation::plan_failed_node_fact_key(&fixture.session_id);
            let app = crate::web::app::build_web_router(fixture.state.clone());

            // 停点键不符 → 409、零副作用（命令账本不落项）。
            let wrong = serde_json::json!({
                "command_id": "cmd-continue-mismatch",
                "expected_binding": fixture.binding_json(),
                "expected_stop_key": "plan_gate:not_the_stop:human_confirm",
            });
            let (status, payload) = post_json(&app, continue_uri(), &wrong).await;
            assert_eq!(status, StatusCode::CONFLICT, "stop key mismatch: {payload}");
            assert_eq!(payload["code"], "automation_continue_stop_key_mismatch");
            assert!(
                fixture.enrollment().command_ledger.is_empty(),
                "mismatch must not claim the command"
            );

            let body = serde_json::json!({
                "command_id": "cmd-continue-ledger",
                "expected_binding": fixture.binding_json(),
                "expected_stop_key": failed_key,
            });
            let (status, payload) = post_json(&app, continue_uri(), &body).await;
            assert_eq!(status, StatusCode::OK, "first continue: {payload}");
            let ledger_after_first = fixture.record().provider_start_ledger.len();
            wait_for(
                &fixture,
                |_| fixture.record().provider_start_ledger.len() > ledger_after_first,
                "first continue drives",
            )
            .await;
            let ledger_count = fixture.record().provider_start_ledger.len();

            // 同键同负载 → replayed，不再派发。
            let (status, payload) = post_json(&app, continue_uri(), &body).await;
            assert_eq!(status, StatusCode::OK, "replay: {payload}");
            assert_eq!(payload["state"], "replayed", "{payload}");
            assert_eq!(
                fixture.record().provider_start_ledger.len(),
                ledger_count,
                "replay must not dispatch again"
            );

            // 同键异负载 → 409。
            let mutated = serde_json::json!({
                "command_id": "cmd-continue-ledger",
                "expected_binding": fixture.binding_json(),
                "expected_stop_key": failed_key,
                "context": "不同的负载",
            });
            let (status, payload) = post_json(&app, continue_uri(), &mutated).await;
            assert_eq!(
                status,
                StatusCode::CONFLICT,
                "same command different payload must 409: {payload}"
            );
            assert_eq!(payload["code"], "automation_continue_command_conflict");
        }

        /// TDD ⑤：terminated（N0 字段非空）拒绝继续与派发，零副作用。
        #[tokio::test]
        async fn continue_plan_rejects_terminated_enrollment() {
            let fixture = continue_fixture(
                PlanApprovalPolicy::Auto,
                crate::product::models::WorkspaceSessionStatus::Failed,
                crate::product::models::SingleCandidatePhase::Failed,
                None,
                false,
            )
            .await;
            let mut enrollment = fixture.enrollment();
            enrollment.terminated_at = Some("2026-10-16T01:00:00Z".to_string());
            crate::product::json_store::write_json(
                &fixture
                    .lifecycle
                    .app_paths()
                    .issue_root(PROJECT_ID, ISSUE_ID)
                    .join("automation-enrollment.json"),
                &enrollment,
            )
            .expect("mark terminated");
            let failed_key =
                crate::web::plan_generation::plan_failed_node_fact_key(&fixture.session_id);
            let app = crate::web::app::build_web_router(fixture.state.clone());
            let body = serde_json::json!({
                "command_id": "cmd-continue-term",
                "expected_binding": fixture.binding_json(),
                "expected_stop_key": failed_key,
            });
            let (status, payload) = post_json(&app, continue_uri(), &body).await;
            assert_eq!(status, StatusCode::CONFLICT, "terminated: {payload}");
            assert_eq!(payload["code"], "automation_plan_terminated");
            assert!(
                fixture.enrollment().command_ledger.is_empty(),
                "terminated continue must be claim-free"
            );
            assert_eq!(
                fixture.record().single_candidate_phase,
                Some(crate::product::models::SingleCandidatePhase::Failed),
                "terminated continue must not rearm"
            );
        }

        /// TDD ②：评审超限（StoppedNeedsHuman）同 session CAS WaitingForHuman＋
        /// 预算耗尽显式继续只开一个新说明回合（TurnOpened）；成功 resolve
        /// 门事实；说明为空拒绝。
        #[tokio::test]
        async fn continue_plan_stopped_gate_reopens_single_turn_and_resolves_fact() {
            let fixture = continue_fixture(
                PlanApprovalPolicy::Auto,
                crate::product::models::WorkspaceSessionStatus::StoppedNeedsHuman,
                crate::product::models::SingleCandidatePhase::Approval,
                Some(gate_snapshot(0)),
                true,
            )
            .await;
            let gate_key = crate::web::plan_generation::plan_gate_fact_key(&fixture.session_id);
            fixture
                .store
                .upsert_automation_attention_fact(
                    PROJECT_ID,
                    ISSUE_ID,
                    &fixture.binding,
                    AutomationAttentionFact {
                        key: gate_key.clone(),
                        kind: AutomationAttentionKind::ReviewLimit,
                        stage: AutomationAttentionStage::Plan,
                        plan_id: Some(fixture.plan_id.clone()),
                        session_id: Some(fixture.session_id.clone()),
                        attempt_id: None,
                        reason_code: "plan_review_manual_budget_exhausted".to_string(),
                        occurred_at: "2026-10-16T00:00:00Z".to_string(),
                        choice_id: None,
                        gate_id: None,
                    },
                )
                .expect("seed gate fact");
            let app = crate::web::app::build_web_router(fixture.state.clone());

            // control 只读投影：真实 stop_key/stage/allowed/context_required。
            let (status, payload) = get_json(&app, control_uri()).await;
            assert_eq!(status, StatusCode::OK, "control: {payload}");
            assert_eq!(payload["managed"], true, "{payload}");
            assert_eq!(payload["stage"], "stopped_needs_human", "{payload}");
            assert_eq!(payload["stop_key"], gate_key, "{payload}");
            assert_eq!(
                payload["allowed_actions"],
                serde_json::json!(["continue_plan"]),
                "{payload}"
            );
            assert_eq!(payload["context_required"], true, "{payload}");
            assert!(
                fixture
                    .enrollment()
                    .attention_facts
                    .iter()
                    .any(|fact| fact.key == gate_key),
                "control GET must not write facts"
            );

            // 说明为空 → 422，零副作用。
            let blank = serde_json::json!({
                "command_id": "cmd-gate-blank",
                "expected_binding": fixture.binding_json(),
                "expected_stop_key": gate_key,
            });
            let (status, payload) = post_json(&app, continue_uri(), &blank).await;
            assert_eq!(
                status,
                StatusCode::UNPROCESSABLE_ENTITY,
                "blank context must 422: {payload}"
            );
            assert_eq!(payload["code"], "automation_continue_context_required");
            assert_eq!(
                fixture.record().status,
                crate::product::models::WorkspaceSessionStatus::StoppedNeedsHuman,
                "blank context must not reopen the gate"
            );

            let body = serde_json::json!({
                "command_id": "cmd-gate-continue",
                "expected_binding": fixture.binding_json(),
                "expected_stop_key": gate_key,
                "context": "评审口径按新约束放宽，继续按说明修订。",
            });
            let (status, payload) = post_json(&app, continue_uri(), &body).await;
            assert_eq!(status, StatusCode::OK, "gate continue: {payload}");
            assert_eq!(payload["state"], "accepted", "{payload}");

            // 同 session 原地重开（WaitingForHuman）＋单说明回合（TurnOpened
            // 的 durable turn 携带 command/说明）。
            let record = fixture.record();
            assert_eq!(
                record.status,
                crate::product::models::WorkspaceSessionStatus::WaitingForHuman
            );
            let turn = fixture
                .lifecycle
                .get_human_gate_turn_by_command_id(&fixture.session_id, "cmd-gate-continue")
                .expect("turn read")
                .expect("turn opened by continue");
            assert_eq!(turn.budget_reserved, 1, "explicit continue opens one turn");

            // 成功 resolve：门事实消失。
            wait_for(
                &fixture,
                |enrollment| {
                    !enrollment
                        .attention_facts
                        .iter()
                        .any(|fact| fact.key == gate_key)
                },
                "gate fact resolved",
            )
            .await;
        }

        /// k3 波3 发现1 fix：needs_human 收尾把命令账本落 NeedsHuman，同键
        /// 重放回执 state=needs_human（不把人工分诊停等读成成功重放）；
        /// accepted 收尾的重放语义不变（见账本重放用例）。
        #[tokio::test]
        async fn continue_plan_needs_human_outcome_replays_needs_human() {
            let fixture = continue_fixture(
                PlanApprovalPolicy::Auto,
                crate::product::models::WorkspaceSessionStatus::StoppedNeedsHuman,
                crate::product::models::SingleCandidatePhase::Approval,
                Some(gate_snapshot(1)),
                // 不带候选产物：修订回合因候选缺失被引擎 typed 拒绝 →
                // continue 收尾必然 needs_human（确定性停等现场）。
                false,
            )
            .await;
            let gate_key = crate::web::plan_generation::plan_gate_fact_key(&fixture.session_id);
            let app = crate::web::app::build_web_router(fixture.state.clone());
            let body = serde_json::json!({
                "command_id": "cmd-gate-needs-human",
                "expected_binding": fixture.binding_json(),
                "expected_stop_key": gate_key,
                "context": "按新口径继续修订。",
            });
            let (status, payload) = post_json(&app, continue_uri(), &body).await;
            assert_eq!(status, StatusCode::OK, "needs_human finish: {payload}");
            assert_eq!(payload["state"], "needs_human", "{payload}");

            // durable 账本状态＝NeedsHuman（SC retry mark_needs_human 先例）。
            let enrollment_after = fixture.enrollment();
            let entry = enrollment_after
                .command_ledger
                .iter()
                .find(|entry| entry.command_id == "cmd-gate-needs-human")
                .expect("ledger entry committed");
            assert_eq!(
                entry.state,
                crate::product::models::automation::OperationState::NeedsHuman
            );

            // 同键同负载重放：读首次 durable 结果 → needs_human（非 replayed）。
            let (status, payload) = post_json(&app, continue_uri(), &body).await;
            assert_eq!(
                status,
                StatusCode::OK,
                "replay after needs_human: {payload}"
            );
            assert_eq!(
                payload["state"], "needs_human",
                "replay must surface the durable needs_human result, not a fake replay: {payload}"
            );
        }

        /// TDD ⑦：plan_generation intent NeedsHuman / EngineStarted(ProviderDispatched)
        /// 无活 run 两分支写 automation_failed（稳定 key、手动零写入）。
        #[tokio::test]
        async fn plan_generation_needs_human_branches_write_automation_failed_fact() {
            let fixture = continue_fixture(
                PlanApprovalPolicy::Auto,
                crate::product::models::WorkspaceSessionStatus::Open,
                crate::product::models::SingleCandidatePhase::Prepare,
                None,
                false,
            )
            .await;
            fixture
                .store
                .claim_plan_generation(PROJECT_ID, ISSUE_ID, &fixture.enrollment_id)
                .expect("claim intent");
            fixture
                .store
                .mark_plan_generation_phase(
                    PROJECT_ID,
                    ISSUE_ID,
                    &fixture.enrollment_id,
                    PlanGenerationPhase::NeedsHuman,
                )
                .expect("intent needs human");
            let enrollment = fixture.enrollment();
            let outcome = crate::web::plan_generation::start_plan_generation_once(
                &fixture.state,
                &enrollment,
            )
            .await
            .expect("observe");
            assert_eq!(
                outcome,
                crate::web::plan_generation::PlanGenerationOutcome::NeedsHuman
            );
            let key = crate::web::plan_generation::plan_generation_failed_fact_key(
                &fixture.enrollment_id,
                &fixture.plan_id,
            );
            let facts = fixture.enrollment().attention_facts;
            let fact = facts
                .iter()
                .find(|fact| fact.key == key)
                .expect("automation_failed fact written");
            assert_eq!(fact.kind, AutomationAttentionKind::AutomationFailed);
            assert_eq!(fact.stage, AutomationAttentionStage::Plan);
            assert_eq!(fact.reason_code, "plan_generation_needs_human");
            assert_eq!(
                fact.session_id.as_deref(),
                Some(fixture.session_id.as_str())
            );

            // 第二分支：EngineStarted 无活 run → 转 NeedsHuman 并写事实。
            let fixture2 = continue_fixture(
                PlanApprovalPolicy::Auto,
                crate::product::models::WorkspaceSessionStatus::Open,
                crate::product::models::SingleCandidatePhase::Prepare,
                None,
                false,
            )
            .await;
            fixture2
                .store
                .claim_plan_generation(PROJECT_ID, ISSUE_ID, &fixture2.enrollment_id)
                .expect("claim intent");
            fixture2
                .store
                .mark_plan_generation_phase(
                    PROJECT_ID,
                    ISSUE_ID,
                    &fixture2.enrollment_id,
                    PlanGenerationPhase::EngineStarted,
                )
                .expect("engine started without run");
            let enrollment2 = fixture2.enrollment();
            let outcome2 = crate::web::plan_generation::start_plan_generation_once(
                &fixture2.state,
                &enrollment2,
            )
            .await
            .expect("observe");
            assert_eq!(
                outcome2,
                crate::web::plan_generation::PlanGenerationOutcome::NeedsHuman
            );
            let facts2 = fixture2.enrollment().attention_facts;
            let fact2 = facts2
                .iter()
                .find(|fact| {
                    fact.key
                        == crate::web::plan_generation::plan_generation_failed_fact_key(
                            &fixture2.enrollment_id,
                            &fixture2.plan_id,
                        )
                })
                .expect("dispatch-unproven fact written");
            assert_eq!(fact2.reason_code, "plan_generation_dispatch_unproven");

            // 手动零写入：manual 冻结的同形状停点不落事实。
            let fixture3 = continue_fixture(
                PlanApprovalPolicy::Manual,
                crate::product::models::WorkspaceSessionStatus::Open,
                crate::product::models::SingleCandidatePhase::Prepare,
                None,
                false,
            )
            .await;
            // 手动授权会话为 Interactive：改写策略后重绑一致性由准入拒绝，
            // 此处只验事实面——intent NeedsHuman 不写事实。
            let enrollment3 = fixture3.enrollment();
            let _ = crate::web::plan_generation::start_plan_generation_once(
                &fixture3.state,
                &enrollment3,
            )
            .await;
            assert!(
                fixture3.enrollment().attention_facts.is_empty(),
                "manual enrollment must not receive plan attention facts"
            );
        }

        /// TDD ⑧：router 观察 choice/HumanGateOpened/durable Failed 写
        /// plan_question/review_limit/automation_failed；HumanGateClosed resolve；
        /// 禁用 enrollment 零写入。
        #[tokio::test]
        async fn router_plan_stop_facts_upsert_and_resolve() {
            let fixture = continue_fixture(
                PlanApprovalPolicy::Auto,
                crate::product::models::WorkspaceSessionStatus::StoppedNeedsHuman,
                crate::product::models::SingleCandidatePhase::Approval,
                Some(gate_snapshot(0)),
                false,
            )
            .await;
            let manager = fixture.manager().await;
            let engine_tx = manager.engine_tx();
            let gate_key = crate::web::plan_generation::plan_gate_fact_key(&fixture.session_id);

            // HumanGateOpened + 预算耗尽 → review_limit。
            engine_tx
                .send(
                    crate::product::workspace_engine::EngineEvent::HumanGateOpened {
                        stage: "human_confirm".to_string(),
                    },
                )
                .await
                .expect("send gate open");
            wait_for(
                &fixture,
                |enrollment| {
                    enrollment
                        .attention_facts
                        .iter()
                        .any(|fact| fact.key == gate_key)
                },
                "gate fact written",
            )
            .await;
            let fact = fixture
                .enrollment()
                .attention_facts
                .into_iter()
                .find(|fact| fact.key == gate_key)
                .unwrap();
            assert_eq!(fact.kind, AutomationAttentionKind::ReviewLimit);

            // 预算未耗尽（重写快照）→ 同 key upsert 为 plan_question（不增项）。
            let mut record = fixture.record();
            record.human_gate_snapshot = Some(gate_snapshot(2));
            fixture.write_record(&record);
            engine_tx
                .send(
                    crate::product::workspace_engine::EngineEvent::HumanGateOpened {
                        stage: "human_confirm".to_string(),
                    },
                )
                .await
                .expect("send gate open again");
            wait_for(
                &fixture,
                |enrollment| {
                    enrollment.attention_facts.iter().any(|fact| {
                        fact.key == gate_key && fact.kind == AutomationAttentionKind::PlanQuestion
                    })
                },
                "gate fact refined to plan_question",
            )
            .await;
            assert_eq!(
                fixture
                    .enrollment()
                    .attention_facts
                    .iter()
                    .filter(|fact| fact.key == gate_key)
                    .count(),
                1,
                "stable key must not duplicate facts"
            );

            // 挂起 choice → plan_question（focus choice_id）。
            engine_tx
                .send(crate::product::workspace_engine::EngineEvent::ChoiceRequest {
                    id: "choice-p4-1".to_string(),
                    prompt: "选择实现路径".to_string(),
                    options: Vec::new(),
                    allow_multiple: false,
                    allow_free_text: true,
                    questions: Vec::new(),
                    source: crate::cross_cutting::streaming_provider::ChoiceRequestSource::AskUserQuestion,
                })
                .await
                .expect("send choice");
            let choice_key = crate::web::plan_generation::plan_choice_fact_key(
                &fixture.session_id,
                "choice-p4-1",
            );
            wait_for(
                &fixture,
                |enrollment| {
                    enrollment
                        .attention_facts
                        .iter()
                        .any(|fact| fact.key == choice_key)
                },
                "choice fact written",
            )
            .await;
            let choice_fact = fixture
                .enrollment()
                .attention_facts
                .into_iter()
                .find(|fact| fact.key == choice_key)
                .unwrap();
            assert_eq!(choice_fact.kind, AutomationAttentionKind::PlanQuestion);
            assert_eq!(choice_fact.choice_id.as_deref(), Some("choice-p4-1"));

            // 门关闭 → 门事实 resolve（choice 事实保留）。
            engine_tx
                .send(
                    crate::product::workspace_engine::EngineEvent::HumanGateClosed {
                        decision: "abandon".to_string(),
                        stage: "human_confirm".to_string(),
                    },
                )
                .await
                .expect("send gate close");
            wait_for(
                &fixture,
                |enrollment| {
                    !enrollment
                        .attention_facts
                        .iter()
                        .any(|fact| fact.key == gate_key)
                },
                "gate fact resolved on close",
            )
            .await;
            assert!(
                fixture
                    .enrollment()
                    .attention_facts
                    .iter()
                    .any(|fact| fact.key == choice_key),
                "choice fact must survive gate close"
            );

            // durable Failed 收敛 → automation_failed（provider 失败）。
            let mut record = fixture.record();
            record.status = crate::product::models::WorkspaceSessionStatus::Failed;
            record.single_candidate_phase =
                Some(crate::product::models::SingleCandidatePhase::Failed);
            fixture.write_record(&record);
            engine_tx
                .send(
                    crate::product::workspace_engine::EngineEvent::TimelineNodeUpdated {
                        node_id: "node_failed_p4".to_string(),
                        status: crate::web::workspace_ws_types::TimelineNodeStatus::Failed,
                        summary: Some("provider unavailable".to_string()),
                        completed_at: None,
                    },
                )
                .await
                .expect("send failed node");
            let failed_key =
                crate::web::plan_generation::plan_failed_node_fact_key(&fixture.session_id);
            wait_for(
                &fixture,
                |enrollment| {
                    enrollment
                        .attention_facts
                        .iter()
                        .any(|fact| fact.key == failed_key)
                },
                "provider failure fact written",
            )
            .await;

            // 禁用 enrollment → 零写入（后续门开不落新事实）。
            let mut enrollment = fixture.enrollment();
            enrollment.enabled = false;
            crate::product::json_store::write_json(
                &fixture
                    .lifecycle
                    .app_paths()
                    .issue_root(PROJECT_ID, ISSUE_ID)
                    .join("automation-enrollment.json"),
                &enrollment,
            )
            .expect("disable enrollment");
            let before = fixture.enrollment().attention_facts.len();
            engine_tx
                .send(
                    crate::product::workspace_engine::EngineEvent::HumanGateOpened {
                        stage: "human_confirm".to_string(),
                    },
                )
                .await
                .expect("send gate open on disabled");
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            assert_eq!(
                fixture.enrollment().attention_facts.len(),
                before,
                "disabled enrollment must not receive new plan facts"
            );
        }

        /// TDD ③：control 无 enrollment → managed=false；GET 全程零写入。
        #[tokio::test]
        async fn control_reports_unmanaged_without_enrollment() {
            let inner = seed_fixture(1, true);
            let root_path = inner._root.path().to_path_buf();
            let state = WebAppState::new(
                root_path.clone(),
                crate::web::runtime::WebRuntime::new_fake(root_path.clone()),
            );
            let app = crate::web::app::build_web_router(state);
            let (status, payload) = get_json(&app, control_uri()).await;
            assert_eq!(status, StatusCode::OK, "control unmanaged: {payload}");
            assert_eq!(payload["managed"], false, "{payload}");
            assert_eq!(payload["stage"], "unmanaged", "{payload}");
            assert!(payload["stop_key"].is_null(), "{payload}");
            assert!(payload["allowed_actions"].as_array().unwrap().is_empty());
        }

        /// X 修复（读取方降级）：绑定会话已被删除（实体级联删除清盘）时，
        /// control 投影降级为 unmanaged，不得把已删 id 抛成 404/500。
        #[tokio::test]
        async fn control_degrades_when_bound_session_deleted() {
            let fixture = continue_fixture(
                PlanApprovalPolicy::Auto,
                crate::product::models::WorkspaceSessionStatus::Open,
                crate::product::models::SingleCandidatePhase::Prepare,
                None,
                false,
            )
            .await;
            let root_path = fixture._root.path().to_path_buf();
            let state = WebAppState::new(
                root_path.clone(),
                crate::web::runtime::WebRuntime::new_fake(root_path.clone()),
            );
            let app = crate::web::app::build_web_router(state);
            // 模拟级联删除后的磁盘状态：会话文件已清、enrollment 仍指向它。
            std::fs::remove_file(
                fixture
                    .lifecycle
                    .app_paths()
                    .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
                    .join("workspace-sessions")
                    .join(format!("{}.json", fixture.session_id)),
            )
            .expect("remove bound session file");

            let (status, payload) = get_json(&app, control_uri()).await;
            assert_eq!(status, StatusCode::OK, "control degrade: {payload}");
            assert_eq!(payload["managed"], false, "{payload}");
            assert_eq!(payload["stage"], "unmanaged", "{payload}");
        }
    }
}
