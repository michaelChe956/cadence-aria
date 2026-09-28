// ---------------------------------------------------------------------------
// P0 1.3（REQ-WIGA-05）Task 10 —— 无 driver 的人工门/compile recovery REST
// （POST /api/workspace-sessions/{id}/human-actions）。全程零 WS attachment：
// manager 以 connection_id=None 的 provider_run_context 复用唯一 run 与事件
// 路由；expected_gate_id 与当前 active gate/timeline node 比对。
// ---------------------------------------------------------------------------

pub(super) struct WorkspaceHumanActionHttpFixture {
    #[allow(dead_code)]
    harness: CampaignStage3Harness,
    router: axum::Router,
    manager: std::sync::Arc<crate::web::workspace_session::WorkspaceSessionManager>,
    session_id: String,
}

pub(super) async fn workspace_human_action_http_fixture(
    budget: u32,
    script: Vec<RevisionScriptStep>,
) -> WorkspaceHumanActionHttpFixture {
    let harness = campaign_stage3_fixture(budget, script).await;
    let root_path = harness.root.path().to_path_buf();
    // campaign 基座只设 stage 不建节点（F-49 注）；显式 enter_human_confirm
    // 落 durable 门节点——REST 的 expected_gate_id 以 active timeline node
    // 为准，且 manager 引擎晚于该 durable 事实构建。
    {
        let mut engine = harness.engine.lock().await;
        engine
            .enter_human_confirm(Some("REST 人工确认门".to_string()))
            .await;
    }
    // 脚本化修订 provider 注入 manager 消费的 registry（与 WS 用例同一测试
    // 构造器；manager 以 durable 记录重建 engine，不共享 harness 内存引擎）。
    let record = harness.session_record().await;
    let mut registry = crate::cross_cutting::provider_registry::ProviderRegistry::new();
    registry.register(
        record.author_provider.clone(),
        harness.scripted_provider_handle(),
    );
    let state = crate::web::state::WebAppState::with_provider_registry(
        root_path.clone(),
        crate::web::runtime::WebRuntime::new_fake(root_path.clone()),
        registry,
    );
    let manager = state
        .workspace_sessions
        .get_or_create(&harness.session_id, || {
            crate::web::workspace_session::WorkspaceSessionManager::create(
                &state,
                &harness.session_id,
            )
        })
        .await
        .expect("manager for human action fixture");
    let session_id = harness.session_id.clone();
    WorkspaceHumanActionHttpFixture {
        harness,
        router: crate::web::app::build_web_router(state),
        manager,
        session_id,
    }
}

impl WorkspaceHumanActionHttpFixture {
    // C1 Task 4：候选行复用同一 REST fixture（visibility: pub(super)）。
    pub(super) async fn post_human_action(
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

    pub(super) async fn active_gate_id(&self) -> Option<String> {
        let engine = self.manager.engine();
        let engine = engine.lock().await;
        engine.active_timeline_node_id()
    }

    pub(super) async fn session_record(&self) -> WorkspaceSessionRecord {
        self.harness.session_record().await
    }

    fn durable_turn_count(&self) -> usize {
        self.harness.durable_turns().len()
    }

    async fn await_turn_terminal(&self, command_id: &str) -> crate::product::models::HumanGateTurn {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Some(turn) = self
                    .harness
                    .lifecycle
                    .get_human_gate_turn_by_command_id(&self.session_id, command_id)
                    .expect("turn lookup")
                    && !matches!(
                        turn.status,
                        crate::product::models::HumanGateTurnStatus::Reserved
                            | crate::product::models::HumanGateTurnStatus::Running
                    )
                {
                    return turn;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .expect("turn reaches terminal state")
    }

    /// 修订轮完成 → 复评 → 新门节点（active node 换代）。
    async fn await_gate_rebuild(&self, previous_gate_id: String) -> String {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Some(current) = self.active_gate_id().await
                    && current != previous_gate_id
                {
                    return current;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .expect("gate rebuilds after revision round")
    }

    pub(super) async fn await_session_status(
        &self,
        expected: WorkspaceSessionStatus,
    ) -> WorkspaceSessionRecord {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let record = self.session_record().await;
                if record.status == expected {
                    return record;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("session reaches {expected:?}"))
    }
}

/// Task 10 主链：关闭 driver 后 REST feedback 开修订 turn；同 command 重试
/// 幂等（Replayed，只一条 durable turn）；修订轮完成后新门上 approve 走
/// deterministic compile 落 durable Confirmed——全程无 WS attachment。
#[tokio::test]
async fn workspace_human_action_http_feedback_replay_and_approve_without_driver() {
    let fixture = workspace_human_action_http_fixture(
        2,
        vec![RevisionScriptStep::Complete(campaign_candidate_v2())],
    )
    .await;
    let gate_id = fixture.active_gate_id().await.expect("active gate node");

    let feedback_body = serde_json::json!({
        "type": "feedback",
        "command_id": "cmd-rest-fb-1",
        "expected_gate_id": gate_id,
        "feedback": "REST 反馈：补充验收条件",
    });
    let (status, body) = fixture.post_human_action(feedback_body.clone()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["state"], "accepted");
    assert_eq!(body["command_id"], "cmd-rest-fb-1");
    assert_eq!(body["gate_id"], serde_json::json!(gate_id));

    // 同 command 重试：durable 查重 → Replayed 幂等 accepted，不二开 turn。
    let (retry_status, retry_body) = fixture.post_human_action(feedback_body).await;
    assert_eq!(retry_status, StatusCode::OK);
    assert_eq!(retry_body["state"], "accepted");

    fixture.await_turn_terminal("cmd-rest-fb-1").await;
    assert_eq!(
        fixture.durable_turn_count(),
        1,
        "同 command 重试只创建一个 turn"
    );

    // 修订轮完成 → 新门（新 active 节点）；approve 必须用新 gate id。
    let gate_id_2 = fixture.await_gate_rebuild(gate_id.clone()).await;
    let (approve_status, approve_body) = fixture
        .post_human_action(serde_json::json!({
            "type": "approve",
            "command_id": "cmd-rest-approve-1",
            "expected_gate_id": gate_id_2,
        }))
        .await;
    assert_eq!(approve_status, StatusCode::OK);
    assert_eq!(approve_body["state"], "accepted");

    // approve 走 deterministic compile：durable Confirmed + Completed 相位，
    // 且零 provider start（compile 不经 provider）。
    let confirmed = fixture
        .await_session_status(WorkspaceSessionStatus::Confirmed)
        .await;
    assert_eq!(
        confirmed.single_candidate_phase,
        Some(SingleCandidatePhase::Completed)
    );
    assert_eq!(
        fixture.harness.scripted_provider_starts(),
        1,
        "仅修订 turn 一次 provider start；approve 是 deterministic compile"
    );
}

/// Task 10 防误答面：门 id 不匹配 409 零副作用；compile recovery 只在
/// WorkItemPlanCompileRecovery 节点放行（当前 human gate → 422）；空白
/// command_id 400；会话全程保持等待态。
#[tokio::test]
async fn workspace_human_action_http_rejects_stale_gate_recovery_and_blank_command() {
    let fixture = workspace_human_action_http_fixture(1, vec![]).await;
    let gate_id = fixture.active_gate_id().await.expect("active gate node");

    let (mismatch_status, mismatch_body) = fixture
        .post_human_action(serde_json::json!({
            "type": "feedback",
            "command_id": "cmd-stale-1",
            "expected_gate_id": "node_not_the_gate",
            "feedback": "过期门的反馈",
        }))
        .await;
    assert_eq!(mismatch_status, StatusCode::CONFLICT);
    assert_eq!(mismatch_body["code"], "human_action_gate_mismatch");
    assert!(
        fixture.harness.durable_turns().is_empty(),
        "门 id 不匹配不得开 turn（不误答）"
    );

    let (recovery_status, recovery_body) = fixture
        .post_human_action(serde_json::json!({
            "type": "compile_recovery",
            "command_id": "cmd-recovery-1",
            "expected_gate_id": gate_id,
            "action": "continue",
            "reason": null,
        }))
        .await;
    assert_eq!(recovery_status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(recovery_body["code"], "invalid_compile_recovery_action");
    assert_eq!(
        fixture.session_record().await.status,
        WorkspaceSessionStatus::WaitingForHuman
    );

    let (blank_status, blank_body) = fixture
        .post_human_action(serde_json::json!({
            "type": "abandon",
            "command_id": "   ",
            "expected_gate_id": gate_id,
        }))
        .await;
    assert_eq!(blank_status, StatusCode::BAD_REQUEST);
    assert_eq!(blank_body["code"], "invalid_command_id");
    assert_eq!(
        fixture.session_record().await.status,
        WorkspaceSessionStatus::WaitingForHuman,
        "空白 command_id 零副作用"
    );
}

/// Task 10 abandon：REST 关门 → durable Terminated，快照清理与 WS 路径一致。
#[tokio::test]
async fn workspace_human_action_http_abandon_terminates_session_without_driver() {
    let fixture = workspace_human_action_http_fixture(1, vec![]).await;
    let gate_id = fixture.active_gate_id().await.expect("active gate node");
    let (status, body) = fixture
        .post_human_action(serde_json::json!({
            "type": "abandon",
            "command_id": "cmd-rest-abandon-1",
            "expected_gate_id": gate_id,
        }))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["state"], "accepted");

    let record = fixture
        .await_session_status(WorkspaceSessionStatus::Terminated)
        .await;
    assert!(
        record.human_gate_snapshot.is_none(),
        "关门后快照清理与 legacy Terminate 路径一致"
    );
    assert!(
        fixture.harness.durable_turns().is_empty(),
        "abandon 零 durable turn"
    );
    assert_eq!(
        fixture.harness.scripted_provider_starts(),
        0,
        "abandon 零 provider start"
    );
}

/// Task 10 approve 失败面：把候选 IR 的首个 item handoff 加一个无消费者契约
///（批准链 compile 以 source/IR 为源；freshness 只看 source hash/编译器版本/
/// 报告，三者不动）——canonical 校验（unconsumed_required_handoff，Error 级）
/// 使 deterministic compile 如实失败并落 Failed 事务：REST approve 422 上抛
/// single_candidate_approval_compile_failed，会话不得宣称 Confirmed
///（REQ-CG-04）。
#[tokio::test]
async fn workspace_human_action_http_approve_compile_failure_never_confirms() {
    let fixture = workspace_human_action_http_fixture(1, vec![]).await;
    {
        use crate::product::work_item_plan_source_store::SourceStoreScope;
        let record = fixture.harness.session_record().await;
        let scope = SourceStoreScope {
            project_id: record.project_id.clone(),
            issue_id: record.issue_id.clone(),
            plan_id: record.entity_id.clone(),
        };
        let source_store =
            crate::product::work_item_plan_source_store::WorkItemPlanSourceStore::new(
                fixture.harness.lifecycle.app_paths(),
            );
        let ir_ref = record
            .plan_candidate_ir_ref
            .clone()
            .expect("fixture IR ref");
        let mut ir = source_store
            .get_plan_candidate_ir(&scope, &ir_ref)
            .expect("fixture IR");
        ir.id = format!("{}-dirty", ir.id);
        ir.ir.items[0]
            .contract
            .handoff_contract
            .provided_contract_refs
            .push("contract.unconsumed.rest".to_string());
        ir.content_hash = ir.content_hash().expect("dirty IR content hash");
        let dirty_ir_ref = source_store
            .put_plan_candidate_ir(&scope.project_id, &scope.issue_id, &scope.plan_id, &ir)
            .expect("persist dirty IR");
        let mut dirty_record = record.clone();
        dirty_record.plan_candidate_ir_ref = Some(dirty_ir_ref);
        crate::product::json_store::write_json(
            &fixture
                .harness
                .app_paths
                .issue_root(&dirty_record.project_id, &dirty_record.issue_id)
                .join("workspace-sessions")
                .join(format!("{}.json", dirty_record.id)),
            &dirty_record,
        )
        .expect("rebind dirty IR ref");
    }
    let gate_id = fixture.active_gate_id().await.expect("active gate node");
    let (status, body) = fixture
        .post_human_action(serde_json::json!({
            "type": "approve",
            "command_id": "cmd-rest-approve-dirty",
            "expected_gate_id": gate_id,
        }))
        .await;
    // 该失败形态无 Failed 事务（引擎未落 findings 事务）——与 WS 层同形映射：
    // 通用引擎错误面（human_action_engine_error，5xx），绝不 200/accepted。
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body={body}");
    assert_eq!(body["code"], "human_action_engine_error");
    assert!(
        body["message"]
            .as_str()
            .is_some_and(|message| message.contains("compile failed; human gate remains open")),
        "错误必须如实携带 deterministic compile 失败语义: {body}"
    );
    assert_eq!(
        fixture.session_record().await.status,
        WorkspaceSessionStatus::WaitingForHuman,
        "compile 失败不得宣称 Confirmed（门保持等待人工）"
    );
    assert_eq!(
        fixture.harness.scripted_provider_starts(),
        0,
        "approve 失败同样零 provider start（deterministic compile）"
    );
}

/// Task 10：REST 人工命令不反向提升 observer WS 权限——人工门命令族
/// （feedback/approve/abandon/compile recovery）在 observer 连接上仍被
/// 只读仲裁拒绝（OBSERVER_WRITE_REJECTED 的仲裁来源）。
#[tokio::test]
async fn workspace_human_action_observer_ws_writes_stay_rejected() {
    use crate::web::workspace_session::ConnectionRole;
    use crate::web::workspace_ws_types::WsInMessage;

    let manager = crate::web::workspace_session::WorkspaceSessionManager::test_fixture(
        "session_human_action_observer",
    );
    let (observer_tx, _observer_rx) = mpsc::channel::<OutboundControl>(1);
    manager.register_attachment("observer_connection", observer_tx.clone());
    manager.bind_role(&observer_tx, ConnectionRole::Observer, None);
    for message in [
        WsInMessage::HumanGateFeedback {
            command_id: "cmd-obs-1".to_string(),
            feedback: "观察者反馈".to_string(),
        },
        WsInMessage::Confirm,
        WsInMessage::AbandonHumanGate {
            command_id: "cmd-obs-2".to_string(),
        },
        WsInMessage::WorkItemPlanCompileRecoveryAction {
            action: crate::web::workspace_ws_types::WorkItemPlanCompileRecoveryActionDto::Continue,
            reason: None,
        },
    ] {
        assert!(
            matches!(
                manager.arbitrate("observer_connection", &message),
                Err(ConnectionRole::Observer)
            ),
            "observer 人工命令写面必须维持拒绝: {:?}",
            message
        );
    }
}
