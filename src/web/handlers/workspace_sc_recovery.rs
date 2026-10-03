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

use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::{SingleCandidatePhase, WorkspaceSessionStatus, WorkspaceType};
use crate::product::work_item_plan_policy::{RunPolicy, WorkItemPlanFlowKind};
use crate::web::error::{ApiError, ApiResult};
use crate::web::handlers::support::{product_app_paths, product_store_api_error};
use crate::web::handlers::workspace_choice::resolve_session_manager;
use crate::web::state::WebAppState;
use crate::web::types::{RetryFailedScRunRequest, RetryFailedScRunStatus};
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
}
