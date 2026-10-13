//! P0 1.3（REQ-WIGA-05）Task 10：无 driver 的人工门与 compile recovery REST。
//!
//! 与 WS 共用同一 manager run 与事件路由（`provider_run_context` 的
//! `connection_id=None`），不挂 attachment、不抢 driver lease、不新建第二个
//! engine。`expected_gate_id` 与当前 active gate/timeline node 比对（不匹配
//! 409 零副作用）；忙态 409；引擎语义拒绝/compile 失败 422（不宣称
//! Confirmed）；受理（含幂等重放与 AlreadyClosed）200。

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde_json::json;

use crate::product::workspace_engine::{HumanGateCloseDecision, HumanGateFeedbackInput};
use crate::web::error::{ApiError, ApiResult};
use crate::web::handlers::workspace_choice::resolve_session_manager;
use crate::web::state::WebAppState;
use crate::web::types::{HumanActionRequest, HumanActionState, HumanActionStatus};
use crate::web::workspace_ws_handler::{
    HumanGateFeedbackEffect, HumanGateTerminationEffect, ProviderRunKind,
    apply_human_gate_feedback, apply_human_gate_termination, confirmation_trust_waiting_projection,
    spawn_provider_run_from_handler,
};

pub async fn post_workspace_human_action(
    State(state): State<WebAppState>,
    Path(session_id): Path<String>,
    Json(request): Json<HumanActionRequest>,
) -> ApiResult<axum::response::Response> {
    let manager = resolve_session_manager(&state, &session_id).await?;
    let (command_id, expected_gate_id) = command_and_gate(&request);
    if command_id.trim().is_empty() {
        return Err(ApiError::runtime(
            "invalid_command_id",
            "command_id must not be blank",
            json!({ "session_id": session_id }),
        ));
    }
    // 门身份比对在引擎调用前完成：不匹配零副作用（不误答过期门）。
    let gate_id = {
        let engine = manager.engine();
        let engine = engine.lock().await;
        engine.active_timeline_node_id()
    };
    if gate_id.as_deref() != Some(expected_gate_id.as_str()) {
        return Err(ApiError::runtime(
            "human_action_gate_mismatch",
            "expected_gate_id does not match the current active gate",
            json!({
                "expected_gate_id": expected_gate_id,
                "current_gate_id": gate_id,
                "session_id": session_id,
            }),
        ));
    }
    let gate_id = gate_id.unwrap_or_default();
    let engine = manager.engine();
    // C1 Task 4：CandidateRecovery 携带专用响应体（facts/missing 投影），
    // 与既有 HumanActionStatus 族并存于同一路由（唯一 additive variant）。
    if matches!(request, HumanActionRequest::CandidateRecovery { .. }) {
        let result =
            handle_candidate_recovery(engine, gate_id, command_id, &session_id, request).await?;
        return Ok((StatusCode::OK, Json(result)).into_response());
    }
    handle_request(&state, &manager, &session_id, engine, gate_id, request)
        .await
        .map(|status| {
            let code = if status.state == HumanActionState::Busy {
                StatusCode::CONFLICT
            } else {
                StatusCode::OK
            };
            (code, Json(status)).into_response()
        })
}

/// C1 Task 4（REQ-C1-GATE-01/02）：候选门恢复动作的薄入口——只调用引擎
/// 应用服务 `recover_candidate_gate`，不承载判定逻辑。
async fn handle_candidate_recovery(
    engine: std::sync::Arc<tokio::sync::Mutex<crate::product::workspace_engine::WorkspaceEngine>>,
    gate_id: String,
    command_id: String,
    session_id: &str,
    request: HumanActionRequest,
) -> ApiResult<crate::web::types::CandidateRecoveryResult> {
    use crate::product::models::OperationState;
    use crate::product::workspace_engine::CandidateRecoveryCommand;
    use crate::web::types::CandidateRecoveryResult;

    let HumanActionRequest::CandidateRecovery { action, .. } = request else {
        return Err(ApiError::runtime(
            "candidate_recovery_rejected",
            "not a candidate recovery request".to_string(),
            json!({ "command_id": command_id, "session_id": session_id }),
        ));
    };
    let outcome = {
        let mut engine = engine.lock().await;
        engine
            .recover_candidate_gate(CandidateRecoveryCommand {
                command_id: command_id.clone(),
                expected_gate_id: gate_id.clone(),
                action: action.into(),
            })
            .await
    };
    let projection = |state: OperationState,
                      complete: bool,
                      missing: Vec<String>,
                      completed_steps: Vec<String>| CandidateRecoveryResult {
        command_id: command_id.clone(),
        gate_id: gate_id.clone(),
        state,
        complete,
        missing,
        completed_steps,
    };
    match outcome {
        Ok(crate::product::workspace_engine::CandidateRecoveryOutcome::Accepted { facts }) => {
            Ok(projection(
                OperationState::Accepted,
                facts.complete,
                facts.missing,
                facts.completed_steps,
            ))
        }
        Ok(crate::product::workspace_engine::CandidateRecoveryOutcome::NeedsHuman { facts }) => {
            Ok(projection(
                OperationState::NeedsHuman,
                facts.complete,
                facts.missing,
                facts.completed_steps,
            ))
        }
        Ok(crate::product::workspace_engine::CandidateRecoveryOutcome::Replayed {
            facts, ..
        }) => Ok(projection(
            OperationState::Replayed,
            facts.complete,
            facts.missing,
            facts.completed_steps,
        )),
        Err(message) => {
            let code = if message.contains("GATE_MISMATCH") {
                "candidate_recovery_gate_mismatch"
            } else if message.contains("COMMAND_CONFLICT") {
                "candidate_recovery_command_conflict"
            } else {
                "candidate_recovery_rejected"
            };
            Err(ApiError::runtime(
                code,
                message,
                json!({
                    "command_id": command_id,
                    "gate_id": gate_id,
                    "session_id": session_id,
                }),
            ))
        }
    }
}

async fn handle_request(
    state: &WebAppState,
    manager: &std::sync::Arc<crate::web::workspace_session::WorkspaceSessionManager>,
    session_id: &str,
    engine: std::sync::Arc<tokio::sync::Mutex<crate::product::workspace_engine::WorkspaceEngine>>,
    gate_id: String,
    request: HumanActionRequest,
) -> ApiResult<HumanActionStatus> {
    let (command_id, _expected_gate_id) = command_and_gate(&request);
    match request {
        HumanActionRequest::Feedback {
            command_id,
            feedback,
            ..
        } => {
            let effect = apply_human_gate_feedback(
                engine,
                HumanGateFeedbackInput {
                    command_id: command_id.clone(),
                    feedback,
                },
            )
            .await;
            match effect {
                HumanGateFeedbackEffect::TurnOpened { turn, prompt, .. } => {
                    // 经唯一 manager run 与事件路由驱动修订 turn（connection_id=None：
                    // 不挂 attachment、不抢 lease）。outbound 汇入逐帧打点 drain
                    //（蒸发链修法③：原先的丢弃通道令 run 内 HumanGateTurnFailed
                    // 等错误帧全部蒸发，对 Replayed/Busy 类零落盘效果无 durable 可
                    // 查）；观察者侧事实仍由 event router 的 EngineEvent 广播承载。
                    let sink_tx = rest_feedback_outbound_drain(session_id);
                    let run_context = manager.provider_run_context(state.workspace_runs.clone());
                    spawn_provider_run_from_handler(
                        run_context,
                        ProviderRunKind::HumanGateScManualRevision {
                            turn_id: turn.turn_id,
                            prompt,
                        },
                        sink_tx,
                    )
                    .await
                    .map_err(|message| {
                        ApiError::runtime(
                            "human_action_run_spawn_failed",
                            message,
                            json!({ "command_id": command_id, "session_id": session_id }),
                        )
                    })?;
                    Ok(accepted(command_id, gate_id))
                }
                HumanGateFeedbackEffect::Replayed { .. } => Ok(accepted(command_id, gate_id)),
                HumanGateFeedbackEffect::Busy { .. } => Ok(busy(command_id, gate_id)),
                HumanGateFeedbackEffect::Rejected { code, reason } => Err(ApiError::runtime(
                    "human_action_rejected",
                    reason,
                    json!({
                        "rejection_code": code,
                        "command_id": command_id,
                        "gate_id": gate_id,
                    }),
                )),
                HumanGateFeedbackEffect::EngineError(message) => Err(ApiError::runtime(
                    "human_action_engine_error",
                    message,
                    json!({ "command_id": command_id, "session_id": session_id }),
                )),
            }
        }
        request @ (HumanActionRequest::Approve { .. } | HumanActionRequest::Abandon { .. }) => {
            let decision = match &request {
                HumanActionRequest::Approve { .. } => HumanGateCloseDecision::Approve,
                _ => HumanGateCloseDecision::Abandon,
            };
            let effect = apply_human_gate_termination(engine, decision).await;
            match effect {
                HumanGateTerminationEffect::Confirmed
                | HumanGateTerminationEffect::Abandoned
                | HumanGateTerminationEffect::AlreadyClosed { .. } => {
                    Ok(accepted(command_id, gate_id))
                }
                HumanGateTerminationEffect::Busy { .. } => Ok(busy(command_id, gate_id)),
                HumanGateTerminationEffect::CompileFailed {
                    message,
                    findings_context,
                } => Err(ApiError::runtime(
                    "single_candidate_approval_compile_failed",
                    message,
                    findings_context
                        .unwrap_or_else(|| json!({ "command_id": command_id, "gate_id": gate_id })),
                )),
                // T5.1（REQ-REG-15/D4）：确认 trust 未 Ready → 非 success
                // ApiError（422，不宣称 Confirmed、不落 generic engine
                // error）；稳定 code 与 message/context 同源投影（与 WS
                // ProtocolError 共用 `confirmation_trust_waiting_projection`）。
                HumanGateTerminationEffect::TrustWaiting { waiting } => {
                    let projection = confirmation_trust_waiting_projection(&waiting);
                    Err(ApiError::runtime(
                        projection.code,
                        projection.message,
                        projection.context,
                    ))
                }
                HumanGateTerminationEffect::EngineError(message) => {
                    // C1（REQ-C1-GATE-01）：缺完整候选快照的裸 approve 以
                    // 专用 code 上抛（422），不混入引擎噪音通道。
                    let code = if message.contains("CANDIDATE_SNAPSHOT_INCOMPLETE") {
                        "candidate_snapshot_incomplete"
                    } else {
                        "human_action_engine_error"
                    };
                    Err(ApiError::runtime(
                        code,
                        message,
                        json!({ "command_id": command_id, "session_id": session_id }),
                    ))
                }
            }
        }
        HumanActionRequest::CompileRecovery {
            command_id,
            action,
            reason,
            ..
        } => {
            let outcome = {
                let mut engine = engine.lock().await;
                engine
                    .handle_work_item_plan_compile_recovery_action(action, reason)
                    .await
            };
            match outcome {
                Ok(_) => Ok(accepted(command_id, gate_id)),
                Err(message) => Err(ApiError::runtime(
                    "invalid_compile_recovery_action",
                    message,
                    json!({ "command_id": command_id, "gate_id": gate_id }),
                )),
            }
        }
        // C1 Task 4：CandidateRecovery 在 post_workspace_human_action 中
        // 先行分流到 handle_candidate_recovery（专用响应体），此处不可达。
        HumanActionRequest::CandidateRecovery { command_id, .. } => Err(ApiError::runtime(
            "candidate_recovery_rejected",
            "candidate recovery is dispatched before handle_request".to_string(),
            json!({ "command_id": command_id, "gate_id": gate_id }),
        )),
    }
}

fn command_and_gate(request: &HumanActionRequest) -> (String, String) {
    match request {
        HumanActionRequest::Approve {
            command_id,
            expected_gate_id,
        }
        | HumanActionRequest::Abandon {
            command_id,
            expected_gate_id,
        } => (command_id.clone(), expected_gate_id.clone()),
        HumanActionRequest::Feedback {
            command_id,
            expected_gate_id,
            ..
        }
        | HumanActionRequest::CompileRecovery {
            command_id,
            expected_gate_id,
            ..
        }
        | HumanActionRequest::CandidateRecovery {
            command_id,
            expected_gate_id,
            ..
        } => (command_id.clone(), expected_gate_id.clone()),
    }
}

fn accepted(command_id: String, gate_id: String) -> HumanActionStatus {
    HumanActionStatus {
        command_id,
        state: HumanActionState::Accepted,
        gate_id,
    }
}

fn busy(command_id: String, gate_id: String) -> HumanActionStatus {
    HumanActionStatus {
        command_id,
        state: HumanActionState::Busy,
        gate_id,
    }
}

/// 蒸发链修法③（诊断 local://diag-manual-revision-evaporate.md §a-②）：
/// REST 反馈 spawn 的 outbound 通道——无连接可投递，改为常驻 drain 任务逐帧
/// eprintln 打点（对齐 aria-cancellation 风格），使 run 内 HumanGateTurnFailed
/// 等错误帧至少服务端日志可见，不再静默蒸发。返回发送端交给
/// spawn_provider_run_from_handler。
fn rest_feedback_outbound_drain(
    session_id: &str,
) -> tokio::sync::mpsc::Sender<crate::web::workspace_ws_handler::OutboundControl> {
    let (sink_tx, mut sink_rx) = tokio::sync::mpsc::channel(1);
    let session_id = session_id.to_string();
    tokio::spawn(async move {
        while let Some(control) = sink_rx.recv().await {
            eprintln!(
                "[aria-rest-feedback] workspace human_action outbound_dropped session_id={session_id} frame={control:?}"
            );
        }
    });
    sink_tx
}

// —— T5.1（REQ-REG-15/D4）：确认 trust typed 停等的 REST 边界投影 ——
// 真实 handler/request 测试锚（build_web_router + oneshot）：REST Approve 与
// WS 共用 apply_human_gate_termination effect，TrustWaiting 必须映射为非
// success ApiError（稳定 code=workspace_confirmation_trust_waiting，不宣称
// Confirmed）；Ready 沿原链推进；Abandon 脱困不变。
#[cfg(test)]
mod confirmation_trust_waiting {
    use super::*;
    use crate::product::lifecycle_store::{
        CreateWorkspaceSessionInput, LifecycleStore, WorkItemPlanSessionOptions,
    };
    use crate::product::logical_codebase::ProviderTrustPreparationResult;
    use crate::product::logical_codebase::ProviderTrustWaiting;
    use crate::product::models::{
        ProviderName, SingleCandidatePhase, WorkspaceSessionStatus, WorkspaceType,
    };
    use crate::product::work_item_plan_policy::{HumanGateSnapshot, HumanReason, RunPolicy};
    use crate::product::workspace_engine::{WorkspaceEngine, WorkspaceSession};
    use crate::web::app::build_web_router;
    use crate::web::handlers::automation_enrollment_test_support::seed_fixture;
    use crate::web::state::WebAppState;
    use crate::web::workspace_session::WorkspaceSessionManager;
    use serde_json::json;

    /// 剧本 precondition：默认 Ready，`script_waiting` 排队一条 HomeWaiting；
    /// 记录 operation_id 调用（恰一次/零调用断言）。
    #[derive(Default)]
    struct ScriptedTrustPrecondition {
        calls: std::sync::Mutex<Vec<String>>,
        scripted: std::sync::Mutex<std::collections::VecDeque<ProviderTrustPreparationResult>>,
    }

    impl ScriptedTrustPrecondition {
        fn script_waiting(&self, reason_code: &str) {
            self.scripted
                .lock()
                .unwrap()
                .push_back(ProviderTrustPreparationResult::Waiting {
                    waiting: ProviderTrustWaiting {
                        provider: ProviderName::Codex,
                        canonical_root: std::path::PathBuf::from("/fixture-trust-root"),
                        trust_key: "fixture-trust-key".to_string(),
                        reason_code: reason_code.to_string(),
                        message: "fixture 等待原因（home 读取失败）".to_string(),
                        retry_action: "修复后重新确认".to_string(),
                        manual_guidance: "fixture 人工指导：修复 Codex home 配置".to_string(),
                        command_markdown: None,
                        recorded_at: "2026-10-11T00:00:00Z".to_string(),
                    },
                });
        }

        fn call_count(&self) -> usize {
            self.calls.lock().unwrap().len()
        }
    }

    impl crate::product::logical_codebase::ProviderTrustPrecondition for ScriptedTrustPrecondition {
        fn ensure_ready(
            &self,
            _project_id: &str,
            operation_id: &str,
            _lc_id: &str,
            _canonical_root: &std::path::Path,
            _providers: &[ProviderName],
            _scenario: crate::product::logical_codebase::provider_trust::ProviderTrustScenario,
        ) -> ProviderTrustPreparationResult {
            self.calls.lock().unwrap().push(operation_id.to_string());
            self.scripted.lock().unwrap().pop_front().unwrap_or(
                ProviderTrustPreparationResult::Ready {
                    registrations: Vec::new(),
                },
            )
        }
    }

    struct HumanActionTrustFixture {
        _root: tempfile::TempDir,
        lifecycle: LifecycleStore,
        router: axum::Router,
        manager: std::sync::Arc<WorkspaceSessionManager>,
        session_id: String,
        gate_id: String,
        trust: std::sync::Arc<ScriptedTrustPrecondition>,
    }

    impl HumanActionTrustFixture {
        async fn post_human_action(
            &self,
            body: serde_json::Value,
        ) -> (axum::http::StatusCode, serde_json::Value) {
            use tower::ServiceExt;
            let uri = format!("/api/workspace-sessions/{}/human-actions", self.session_id);
            let response = self
                .router
                .clone()
                .oneshot(
                    axum::http::Request::builder()
                        .method("POST")
                        .uri(uri)
                        .header("content-type", "application/json")
                        .body(axum::body::Body::from(body.to_string()))
                        .unwrap(),
                )
                .await
                .unwrap();
            let status = response.status();
            let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
            (status, json)
        }

        fn durable(&self) -> crate::product::models::WorkspaceSessionRecord {
            self.lifecycle
                .get_workspace_session(&self.session_id)
                .expect("durable session")
        }
    }

    static T5_REST_SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

    /// LC SC Approval 门 + 真实 manager/router：seed LC 环境（issue/manifest/
    /// members）→ SC WorkItemPlan 会话（相位 Approval + 快照静态资格）→
    /// `enter_human_confirm` 落 durable 门节点（REST expected_gate_id 锚）→
    /// WebAppState deps 注入剧本 precondition（生产 manager 注入链）。
    async fn lc_sc_approval_gate_fixture(
        trust: std::sync::Arc<ScriptedTrustPrecondition>,
    ) -> HumanActionTrustFixture {
        let fixture = seed_fixture(2, true);
        let root_path = fixture._root.path().to_path_buf();
        let lifecycle = fixture.lifecycle.clone();
        let seq = T5_REST_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        // 真实 plan 记录（Draft 空 plan）：manager create 的 repository 解析
        // 对 WorkItemPlan 会话先读 plan；2 成员无唯一 focus → 聚合根视图
        //（logical_repository_id=Some → gateway/确认上下文装配段可达）。
        let plan_id = format!("plan_t5_rest_{seq}");
        lifecycle
            .create_issue_work_item_plan(
                crate::product::lifecycle_store::CreateIssueWorkItemPlanInput {
                    id: Some(plan_id.clone()),
                    project_id: "project_0001".to_string(),
                    issue_id: "issue_0001".to_string(),
                    source_story_spec_ids: vec![fixture.story_id.clone()],
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
            .expect("create fixture work item plan");
        let created = lifecycle
            .create_workspace_session(CreateWorkspaceSessionInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                entity_id: plan_id,
                workspace_type: WorkspaceType::WorkItemPlan,
                author_provider: ProviderName::Codex,
                reviewer_provider: None,
                review_rounds: 0,
                superpowers_enabled: false,
                openspec_enabled: false,
                work_item_plan_options: Some(WorkItemPlanSessionOptions {
                    flow_kind: crate::product::work_item_plan_policy::WorkItemPlanFlowKind::SingleCandidate,
                    run_policy: RunPolicy::Interactive,
                    rollout_snapshot: true,
                }),
            })
            .expect("create SC session");
        // 进程内唯一 session id（close CAS 计数不互窜）+ 开态门 durable 事实。
        let session_id = format!("workspace_session_t5_rest_{seq}");
        let mut record = lifecycle
            .get_workspace_session(&created.id)
            .expect("load created session");
        let old_id = record.id.clone();
        record.id = session_id.clone();
        record.status = WorkspaceSessionStatus::WaitingForHuman;
        record.single_candidate_phase = Some(SingleCandidatePhase::Approval);
        record.human_gate_snapshot = Some(HumanGateSnapshot {
            candidate_recovery: None,
            findings: Vec::new(),
            repeated_fingerprints: Vec::new(),
            attempts_used: 0,
            manual_repairs_remaining: 1,
            trigger: HumanReason::NativeHumanRequired,
            resumable: false,
            accepted_feedback_turns: None,
        });
        let session_dir = fixture
            .paths
            .issue_root(&record.project_id, &record.issue_id)
            .join("workspace-sessions");
        crate::product::json_store::write_json(
            &session_dir.join(format!("{}.json", record.id)),
            &record,
        )
        .expect("persist t5 rest fixture session");
        std::fs::remove_file(session_dir.join(format!("{old_id}.json"))).ok();

        // durable 门节点：REST 的 expected_gate_id 以 active timeline node 为准
        //（manager 引擎晚于该 durable 事实构建，与 campaign REST fixture 同款）。
        let (event_tx, _event_rx) = tokio::sync::mpsc::channel(8);
        let session = WorkspaceSession::from_record(record);
        let mut gate_engine = WorkspaceEngine::new_persistent(
            std::sync::Arc::new(crate::product::checkpoint_store::CheckpointStore::new(
                root_path.join("checkpoints"),
            )),
            lifecycle.clone(),
            event_tx,
            session,
        );
        gate_engine
            .enter_human_confirm(Some("T5 REST 人工确认门".to_string()))
            .await;

        // C1 候选完整性 label 不在本测试面（本测试锁 trust 接线）：
        // enter_human_confirm 会按 SC Approval 门落 candidate_recovery 评估
        // label，approve 前的候选完整性重估将因 fixture 缺 plan 候选工件
        // fail-closed（CANDIDATE_SNAPSHOT_INCOMPLETE，与 trust 无关）。
        // 清掉该 additive label，走「label 缺席（旧会话/legacy 门）保持既有
        // approve 语义」的零回归路径（与 WS 面夹具同形）。
        let mut labeled = lifecycle
            .get_workspace_session(&session_id)
            .expect("labeled fixture session");
        if let Some(snapshot) = labeled.human_gate_snapshot.as_mut() {
            snapshot.candidate_recovery = None;
        }
        crate::product::json_store::write_json(
            &session_dir.join(format!("{session_id}.json")),
            &labeled,
        )
        .expect("clear candidate recovery label");

        let registry = crate::cross_cutting::provider_registry::ProviderRegistry::new();
        let state = WebAppState::with_provider_registry(
            root_path.clone(),
            crate::web::runtime::WebRuntime::new_fake(root_path.clone()),
            registry,
        );
        // 生产 manager 注入链：deps 的 trust precondition 换成剧本实例
        //（coordinator/runs/index 原样），manager create 冻结为确认上下文。
        let deps = state
            .aggregate_initialization_dependencies()
            .with_trust(trust.clone());
        let state = state.with_aggregate_initialization_dependencies(deps);
        let manager = state
            .workspace_sessions
            .get_or_create(&session_id, || {
                WorkspaceSessionManager::create(&state, &session_id)
            })
            .await
            .expect("manager for t5 rest fixture");
        let gate_id = {
            let engine = manager.engine();
            let engine = engine.lock().await;
            engine
                .active_timeline_node_id()
                .expect("durable human gate node")
        };
        HumanActionTrustFixture {
            _root: fixture._root,
            lifecycle,
            router: build_web_router(state),
            manager,
            session_id,
            gate_id,
            trust,
        }
    }

    /// REST Approve 的 HomeWaiting：非 success ApiError（422），稳定 code=
    /// workspace_confirmation_trust_waiting（不宣称 Confirmed、不落入 generic
    /// engine error）；message 同源含原因/人工处置/门未关闭/重新确认短语；
    /// details 携带已知身份与 retry_action/manual_guidance/retryable:true；
    /// durable 全不变、close CAS 零推进、ensure 恰一次。
    #[tokio::test]
    async fn rest_approve_trust_waiting_is_non_success_stable_code() {
        let trust = std::sync::Arc::new(ScriptedTrustPrecondition::default());
        trust.script_waiting("fixture_home_failure");
        let fixture = lc_sc_approval_gate_fixture(trust.clone()).await;
        let before = serde_json::to_vec(&fixture.durable()).expect("serialize before");
        let cas_before = crate::product::workspace_engine::conversational_gate::close_cas_attempts(
            &fixture.session_id,
        );

        let (status, body) = fixture
            .post_human_action(json!({
                "type": "approve",
                "command_id": "cmd_t5_rest_approve_waiting",
                "expected_gate_id": fixture.gate_id,
            }))
            .await;

        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body["code"], json!("workspace_confirmation_trust_waiting"));
        let message = body["message"].as_str().expect("message");
        assert!(
            message.contains("fixture 等待原因（home 读取失败）"),
            "{message}"
        );
        assert!(
            message.contains("fixture 人工指导：修复 Codex home 配置"),
            "{message}"
        );
        assert!(message.contains("修复后重新确认"), "{message}");
        assert!(message.contains("门保持未关闭"), "{message}");
        assert!(
            message.contains("完成后请重新选择该门并确认；失败不自动重试"),
            "{message}"
        );
        let details = &body["details"];
        assert_eq!(details["session_id"], json!(fixture.session_id));
        assert_eq!(details["project_id"], json!("project_0001"));
        assert_eq!(details["provider"], json!("codex"));
        assert_eq!(details["trust_key"], json!("fixture-trust-key"));
        assert_eq!(details["reason_code"], json!("fixture_home_failure"));
        assert_eq!(details["retry_action"], json!("修复后重新确认"));
        assert_eq!(
            details["manual_guidance"],
            json!("fixture 人工指导：修复 Codex home 配置")
        );
        assert_eq!(details["retryable"], json!(true));
        assert_eq!(
            serde_json::to_vec(&fixture.durable()).expect("serialize after"),
            before,
            "waiting 前后 durable 会话记录完全不变（门开态、零定稿副作用）"
        );
        assert_eq!(
            crate::product::workspace_engine::conversational_gate::close_cas_attempts(
                &fixture.session_id
            ),
            cas_before,
            "准备失败不推进原 close CAS"
        );
        assert_eq!(trust.call_count(), 1, "missing 才进入 ensure（恰一次）");
    }

    /// REST Approve 的 Ready 对照：trust Ready 后沿原链推进（close CAS 恰
    /// +1），结果不再落 trust waiting——「REST Ready 才 accepted」的等待侧
    /// 对照（accepted 全链由既有 campaign REST approve 用例锁定）。
    #[tokio::test]
    async fn rest_approve_ready_trust_proceeds_past_waiting_arm() {
        let trust = std::sync::Arc::new(ScriptedTrustPrecondition::default());
        let fixture = lc_sc_approval_gate_fixture(trust.clone()).await;
        let cas_before = crate::product::workspace_engine::conversational_gate::close_cas_attempts(
            &fixture.session_id,
        );

        let (status, body) = fixture
            .post_human_action(json!({
                "type": "approve",
                "command_id": "cmd_t5_rest_approve_ready",
                "expected_gate_id": fixture.gate_id,
            }))
            .await;

        assert_ne!(
            status,
            StatusCode::OK,
            "fixture 缺 plan/compile 记录，原链如实失败（不伪成功）"
        );
        assert_ne!(
            body["code"],
            json!("workspace_confirmation_trust_waiting"),
            "Ready 不得落 trust waiting 臂"
        );
        assert_eq!(
            crate::product::workspace_engine::conversational_gate::close_cas_attempts(
                &fixture.session_id
            ),
            cas_before + 1,
            "Ready 才沿原链推进原带写 close CAS（恰一次）"
        );
        assert_eq!(trust.call_count(), 1, "Ready 也恰一次 ensure");
    }

    /// SG3b（REQ-SG-03、REQ-CG-04）：REST Approve 后 compile 失败的 typed
    /// PlanningCompileFailure（failure_id/approval/source context 字段）经
    /// 422 details 直达客户端；同场景种入的旧 Failed tx（不同 failure）
    /// 不得被捞为本次失败事实。本 fixture 无候选 refs——refs 缺失失败，
    /// 字段以 null 序列化、合同不缺。
    #[tokio::test]
    async fn sg_03b_rest_approve_compile_failure_typed_failure_details() {
        use crate::product::models::{
            IssueWorkItemDependencyEdge, IssueWorkItemPlan, IssueWorkItemPlanOptions,
            IssueWorkItemPlanStatus, WorkItemPlanCommitState, WorkItemPlanCompileStatus,
            WorkItemPlanCompileTransaction, WorkItemSplitFinding, WorkItemSplitFindingSeverity,
        };
        use crate::product::work_item_plan_policy::WorkItemPlanFlowKind;
        use crate::product::work_item_plan_store::WorkItemPlanStore;

        let trust = std::sync::Arc::new(ScriptedTrustPrecondition::default());
        let fixture = lc_sc_approval_gate_fixture(trust.clone()).await;
        let durable = fixture.durable();
        // 种入旧 Failed tx（strict validator 失败，不同 failure）。
        WorkItemPlanStore::new(fixture.lifecycle.app_paths())
            .put_compile_transaction(&WorkItemPlanCompileTransaction {
                compile_id: "compile_sg3b_rest_stale".to_string(),
                project_id: durable.project_id.clone(),
                issue_id: durable.issue_id.clone(),
                plan_id: durable.entity_id.clone(),
                flow_kind: Some(WorkItemPlanFlowKind::SingleCandidate),
                source_revision_id: None,
                source_revision_ref: None,
                plan_candidate_ir_ref: None,
                mechanical_report_ref: None,
                publication_provenance_ref: None,
                publication_provenance_content_hash: None,
                generation_round_id: "round_sg3b_rest_stale".to_string(),
                outline_version_ref: "outline_sg3b_rest_stale".to_string(),
                active_draft_ids: Vec::new(),
                status: WorkItemPlanCompileStatus::Failed,
                plan_commit_state: WorkItemPlanCommitState::NotStarted,
                step_cursor: "validating".to_string(),
                outline_to_work_item_id: Default::default(),
                outline_to_verification_plan_id: Default::default(),
                created_work_item_ids: Vec::new(),
                created_verification_plan_ids: Vec::new(),
                child_session_ids: Vec::new(),
                validator_findings: vec![WorkItemSplitFinding {
                    severity: WorkItemSplitFindingSeverity::Error,
                    code: "WI_STALE_SG3B_REST".to_string(),
                    message: "stale failure of a different compile run".to_string(),
                    work_item_ids: vec!["wi_stale".to_string()],
                    kind: None,
                }],
                abort_requested_at: None,
                failure_reason: Some("stale strict validator failure".to_string()),
                previous_plan_snapshot: IssueWorkItemPlan {
                    id: durable.entity_id.clone(),
                    project_id: durable.project_id.clone(),
                    issue_id: durable.issue_id.clone(),
                    source_story_spec_ids: Vec::new(),
                    source_design_spec_ids: Vec::new(),
                    options: IssueWorkItemPlanOptions {
                        include_integration_tests: false,
                        include_e2e_tests: false,
                        force_frontend_backend_split: false,
                        require_execution_plan_confirm: false,
                    },
                    status: IssueWorkItemPlanStatus::Draft,
                    work_item_ids: Vec::new(),
                    repository_profile_ref: None,
                    verification_plan_ids: Vec::new(),
                    dependency_graph: vec![IssueWorkItemDependencyEdge {
                        from_work_item_id: "wi_stale".to_string(),
                        to_work_item_id: "wi_stale_2".to_string(),
                    }],
                    created_from_provider_run: None,
                    validator_findings: Vec::new(),
                    review_summary: None,
                    created_at: "2026-10-13T00:00:00Z".to_string(),
                    updated_at: "2026-10-13T00:00:00Z".to_string(),
                },
                created_at: "2026-10-13T00:00:01Z".to_string(),
                updated_at: "2026-10-13T00:00:01Z".to_string(),
                committed_at: None,
            })
            .expect("seed stale failed compile transaction");

        let (status, body) = fixture
            .post_human_action(json!({
                "type": "approve",
                "command_id": "cmd_sg3b_rest_approve_compile_failure",
                "expected_gate_id": fixture.gate_id,
            }))
            .await;

        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(
            body["code"],
            json!("single_candidate_approval_compile_failed")
        );
        let message = body["message"].as_str().expect("message");
        assert!(message.contains("human gate remains open"), "{message}");
        assert!(message.contains("SOURCE_STORE_MALFORMED_REF"), "{message}");
        assert!(!message.contains("WI_STALE_SG3B_REST"), "{message}");
        let details = &body["details"];
        assert!(
            details["failure_id"]
                .as_str()
                .is_some_and(|id| !id.is_empty()),
            "{details}"
        );
        assert_eq!(details["kind"], json!("operational"));
        assert_eq!(details["stable_code"], json!("SOURCE_STORE_MALFORMED_REF"));
        for field in [
            "approval_attempt_id",
            "source_revision_ref",
            "plan_candidate_ir_ref",
            "mechanical_report_ref",
        ] {
            assert!(details.get(field).is_some(), "{field} missing: {details}");
        }
        assert!(
            details
                .get("validator_findings")
                .map(serde_json::Value::is_null)
                .unwrap_or(true),
            "stale failed tx findings must not be salvaged: {details}"
        );
    }

    /// REST Abandon 脱困不变：LC 会话确认 trust 未 Ready 也不影响 abandon
    /// 逃生门——200 accepted + durable Terminated + 零 trust 调用。
    #[tokio::test]
    async fn rest_abandon_escape_unchanged_under_trust_waiting() {
        let trust = std::sync::Arc::new(ScriptedTrustPrecondition::default());
        trust.script_waiting("fixture_home_failure");
        let fixture = lc_sc_approval_gate_fixture(trust.clone()).await;

        let (status, body) = fixture
            .post_human_action(json!({
                "type": "abandon",
                "command_id": "cmd_t5_rest_abandon",
                "expected_gate_id": fixture.gate_id,
            }))
            .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["state"], json!("accepted"));
        assert_eq!(
            fixture.durable().status,
            WorkspaceSessionStatus::Terminated,
            "abandon 脱困：durable Terminated"
        );
        assert_eq!(
            trust.call_count(),
            0,
            "Abandon 不登记（脱困不受 trust 等待影响）"
        );
    }
}

// 蒸发链修法③（诊断 local://diag-manual-revision-evaporate.md §a-②）：
// REST 反馈 spawn 的 outbound 此前接丢弃通道（`_sink_rx` 立即 drop）——
// run 内 HumanGateTurnFailed 等错误帧全部蒸发，注释里「错误细节以 durable
// 状态为准」对 Replayed/Busy 类零落盘效果不成立。改为常驻 drain 任务逐帧
// eprintln 打点（对齐 aria-cancellation 风格），通道不再静默丢弃。
#[cfg(test)]
mod rest_feedback_outbound_drain {
    use super::*;

    #[tokio::test]
    async fn rest_feedback_outbound_drain_consumes_run_frames_instead_of_dropping() {
        let sender = rest_feedback_outbound_drain("session_drain_probe");
        // 通道容量为 1：若无 drain 任务消费，第二帧起 send 将永久挂起。
        for index in 0..3 {
            sender
                .send(crate::web::workspace_ws_handler::OutboundControl::Text(
                    format!("{{\"drain_probe\":{index}}}"),
                ))
                .await
                .expect("drain must keep consuming REST feedback outbound frames");
        }
    }
}
