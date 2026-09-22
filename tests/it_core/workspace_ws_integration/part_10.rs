use cadence_aria::web::workspace_ws_types::TimelineNode;

// ── change plan-compile-gate-visibility（WP-B Task 2）─────────────────────────
// SC compile recovery action 的 WS 集成矩阵：REQ-CG-02「合法 recovery action 与
// 人工门命令并存」+「非 recovery 状态拒绝 recovery action」（spec
// openspec/changes/plan-compile-gate-visibility/specs/work-item-plan-conversational-gate）。
//
// fixture 形态与 `enter_work_item_plan_compile_recovery`
// （`src/product/workspace_engine/compile.rs:548`）落盘结果等价：会话为
// work_item_plan + single_candidate 流 + waiting_for_human，timeline 上唯一 active
// 的 `work_item_plan_compile_recovery` 节点，加一条 `RecoveryRequired` compile
// transaction。**真 compile 制品**（source/IR/mechanical-report/provenance 四 refs）
// 由 in-crate fixture（`workspace_engine::tests::single_candidate_recovery`）从
// outline/accepted-draft 现场生成，集成层无法复现，故 `continue` 的完整成功链由引擎
// 层用例（single_candidate_recovery.rs:727/:850/:1020、part_10）锁定；本文件的
// `continue` 行只钉「协议面放行 + 引擎 fail-closed 零副作用」。

const SC_RECOVERY_PLAN_ID: &str = "issue_work_item_plan_sc_recovery";
const SC_RECOVERY_COMPILE_ID: &str = "compile_recovery_0001";
const SC_RECOVERY_NODE_ID: &str = "timeline_node_compile_recovery";

struct ScCompileRecoveryFixture {
    session_id: String,
    app_paths: ProductAppPaths,
}

impl ScCompileRecoveryFixture {
    fn lifecycle(&self) -> LifecycleStore {
        LifecycleStore::new(self.app_paths.clone())
    }

    fn compile_transactions(
        &self,
    ) -> Vec<cadence_aria::product::models::WorkItemPlanCompileTransaction> {
        cadence_aria::product::work_item_plan_store::WorkItemPlanStore::new(self.app_paths.clone())
            .list_compile_transactions("project_0001", "issue_0001", SC_RECOVERY_PLAN_ID)
            .expect("list compile transactions")
    }

    fn timeline_nodes(&self) -> Vec<TimelineNode> {
        self.lifecycle()
            .load_timeline_nodes(&self.session_id)
            .expect("load timeline nodes")
    }

    fn session_status(&self) -> cadence_aria::product::models::WorkspaceSessionStatus {
        self.lifecycle()
            .get_workspace_session(&self.session_id)
            .expect("load workspace session")
            .status
    }
}

fn sc_recovery_node(
    node_id: &str,
    node_type: TimelineNodeType,
    stage: cadence_aria::web::workspace_ws_types::WorkspaceStage,
    status: TimelineNodeStatus,
) -> TimelineNode {
    TimelineNode {
        node_id: node_id.to_string(),
        node_type,
        agent: None,
        stage,
        round: None,
        status,
        title: "SC Final Compile".to_string(),
        summary: Some("fixture node".to_string()),
        started_at: "2026-09-22T00:00:00Z".to_string(),
        completed_at: None,
        duration_ms: None,
        artifact_ref: None,
        provider_config_snapshot: ProviderConfigSnapshot {
            author: ProviderName::Fake,
            reviewer: None,
            review_rounds: 0,
            permission_modes: cadence_aria::product::models::WorkspaceRolePermissionModes::default(
            ),
        },
        retry: None,
    }
}

/// 构造一个 SC WorkItemPlan 会话，铺上给定 timeline 节点（以及可选的
/// `RecoveryRequired` compile transaction）。
///
/// 前置复用 `create_workspace_session_fixture`：WS 会话工厂
/// （`WorkspaceSessionManager::create`）要求 project/repository/issue 三者可解析
/// （workspace context + `workspace_repository_for_session`），该夹具是既有集成层
/// 唯一已证实可达的铺法（part_04/part_08 同源）。
async fn create_sc_compile_recovery_fixture(
    root: &TempDir,
    nodes: Vec<TimelineNode>,
    session_status: cadence_aria::product::models::WorkspaceSessionStatus,
    recovery_transaction: bool,
) -> ScCompileRecoveryFixture {
    create_workspace_session_fixture(root).await;
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let plan = lifecycle
        .create_issue_work_item_plan(
            cadence_aria::product::lifecycle_store::CreateIssueWorkItemPlanInput {
                id: Some(SC_RECOVERY_PLAN_ID.to_string()),
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                source_story_spec_ids: vec![],
                source_design_spec_ids: vec![],
                options: cadence_aria::product::models::IssueWorkItemPlanOptions {
                    include_integration_tests: false,
                    include_e2e_tests: false,
                    force_frontend_backend_split: false,
                    require_execution_plan_confirm: false,
                },
                status: cadence_aria::product::models::IssueWorkItemPlanStatus::Draft,
                work_item_ids: vec![],
                repository_profile_ref: None,
                verification_plan_ids: vec![],
                dependency_graph: vec![],
                created_from_provider_run: None,
                validator_findings: vec![],
            },
        )
        .expect("create SC work item plan");
    let session = lifecycle
        .create_workspace_session(
            cadence_aria::product::lifecycle_store::CreateWorkspaceSessionInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                entity_id: plan.id.clone(),
                workspace_type: cadence_aria::product::models::WorkspaceType::WorkItemPlan,
                author_provider: ProviderName::Fake,
                reviewer_provider: ProviderName::Fake,
                review_rounds: 0,
                superpowers_enabled: false,
                openspec_enabled: false,
                work_item_plan_options: Some(
                    cadence_aria::product::lifecycle_store::WorkItemPlanSessionOptions {
                        flow_kind: cadence_aria::product::work_item_plan_policy::WorkItemPlanFlowKind::SingleCandidate,
                        run_policy: cadence_aria::product::work_item_plan_policy::RunPolicy::Interactive,
                        rollout_snapshot: true,
                    },
                ),
            },
        )
        .expect("create SC work item plan session");
    let mut record = lifecycle
        .get_workspace_session(&session.id)
        .expect("load SC plan session");
    record.status = session_status;
    record.single_candidate_phase =
        Some(cadence_aria::product::models::SingleCandidatePhase::Approval);
    let session_path = app_paths
        .issue_root(&record.project_id, &record.issue_id)
        .join("workspace-sessions")
        .join(format!("{}.json", record.id));
    cadence_aria::product::json_store::write_json(&session_path, &record)
        .expect("persist SC plan session");
    lifecycle
        .save_timeline_nodes(&session.id, &nodes)
        .expect("persist fixture timeline");
    if recovery_transaction {
        let now = "2026-09-22T00:00:00Z".to_string();
        cadence_aria::product::work_item_plan_store::WorkItemPlanStore::new(app_paths.clone())
            .put_compile_transaction(&cadence_aria::product::models::WorkItemPlanCompileTransaction {
                compile_id: SC_RECOVERY_COMPILE_ID.to_string(),
                project_id: record.project_id.clone(),
                issue_id: record.issue_id.clone(),
                plan_id: plan.id.clone(),
                flow_kind: Some(
                    cadence_aria::product::work_item_plan_policy::WorkItemPlanFlowKind::SingleCandidate,
                ),
                source_revision_id: None,
                source_revision_ref: None,
                plan_candidate_ir_ref: None,
                mechanical_report_ref: None,
                publication_provenance_ref: None,
                publication_provenance_content_hash: None,
                generation_round_id: "generation_round_recovery".to_string(),
                outline_version_ref: "outline_version_recovery".to_string(),
                active_draft_ids: vec![],
                status: cadence_aria::product::models::WorkItemPlanCompileStatus::RecoveryRequired,
                plan_commit_state:
                    cadence_aria::product::models::WorkItemPlanCommitState::NotStarted,
                step_cursor: "compile_failed".to_string(),
                outline_to_work_item_id: std::collections::BTreeMap::new(),
                outline_to_verification_plan_id: std::collections::BTreeMap::new(),
                created_work_item_ids: vec![],
                created_verification_plan_ids: vec![],
                child_session_ids: vec![],
                validator_findings: vec![],
                abort_requested_at: None,
                failure_reason: Some("final compile failed".to_string()),
                previous_plan_snapshot: plan.clone(),
                created_at: now.clone(),
                updated_at: now,
                committed_at: None,
            })
            .expect("persist recovery compile transaction");
    }
    ScCompileRecoveryFixture {
        session_id: session.id,
        app_paths,
    }
}

async fn sc_recovery_ws_harness(
    root: &TempDir,
    session_id: &str,
) -> (
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    tokio::task::JoinHandle<()>,
) {
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    let (mut ws, _) = connect_async(format!(
        "ws://{addr}/api/workspace-sessions/{session_id}/ws"
    ))
    .await
    .expect("connect ws");
    let initial = recv_json(&mut ws).await;
    assert!(
        matches!(initial, WsOutMessage::SessionState { .. }),
        "unexpected initial frame: {initial:?}"
    );
    (ws, server)
}

/// 收帧到「结果信号」为止：拒绝路径等 `ProtocolError`，合法路径等首个业务帧
/// （stage_change / session_state）。两条路径都收到信号即返回，因此不依赖固定观察窗口。
async fn sc_recovery_await_outcome(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    expect_rejection: bool,
) -> WsOutMessage {
    for _ in 0..80 {
        let frame = recv_json(ws).await;
        match frame {
            WsOutMessage::ProtocolError { .. } if expect_rejection => return frame,
            WsOutMessage::ProtocolError { .. } => {
                panic!("recovery action must not be rejected, got {frame:?}")
            }
            WsOutMessage::Pong => {}
            other if !expect_rejection => return other,
            _ => {}
        }
    }
    panic!("no ws outcome observed (expect_rejection={expect_rejection})");
}

async fn sc_recovery_send_continue(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) {
    send_json(
        ws,
        &WsInMessage::WorkItemPlanCompileRecoveryAction {
            action:
                cadence_aria::web::workspace_ws_types::WorkItemPlanCompileRecoveryActionDto::Continue,
            reason: None,
        },
    )
    .await;
}

fn sc_recovery_assert_rejected_as_stage_error(frame: &WsOutMessage, expected_stage: &str) {
    match frame {
        WsOutMessage::ProtocolError {
            code,
            message,
            context,
        } => {
            assert_eq!(
                code, "INVALID_MESSAGE_FOR_STAGE",
                "非 HumanConfirm 阶段必须按阶段矩阵拒收（stage-specific protocol error）: {message}"
            );
            assert!(
                message.contains("work_item_plan_compile_recovery_action")
                    && message.contains(expected_stage),
                "拒绝必须可诊断到消息与阶段: {message}"
            );
            let context = context.as_ref().expect("stage rejection context");
            assert_eq!(context["stage"].as_str(), Some(expected_stage));
            assert_eq!(
                context["received"].as_str(),
                Some("work_item_plan_compile_recovery_action")
            );
        }
        other => panic!("expected stage-specific protocol error, got {other:?}"),
    }
}

fn sc_recovery_active_node_ids(nodes: &[TimelineNode]) -> Vec<String> {
    nodes
        .iter()
        .filter(|node| node.status == TimelineNodeStatus::Active)
        .map(|node| node.node_id.clone())
        .collect()
}

// 场景 1a（合法 recovery action：human_triage）：协议面放行 → 引擎按既有 recovery
// 语义执行 → 不新建第二个 compile，恢复节点收口、门回 human_confirm。
#[tokio::test]
async fn workspace_ws_sc_compile_recovery_human_triage_runs_through_engine() {
    use cadence_aria::product::models::{WorkItemPlanCompileStatus, WorkspaceSessionStatus};
    use cadence_aria::web::workspace_ws_types::{
        TimelineNodeStatus, TimelineNodeType, WorkItemPlanCompileRecoveryActionDto, WorkspaceStage,
    };

    let root = tempdir().expect("root");
    let fixture = create_sc_compile_recovery_fixture(
        &root,
        vec![sc_recovery_node(
            SC_RECOVERY_NODE_ID,
            TimelineNodeType::WorkItemPlanCompileRecovery,
            WorkspaceStage::HumanConfirm,
            TimelineNodeStatus::Active,
        )],
        WorkspaceSessionStatus::WaitingForHuman,
        true,
    )
    .await;
    let (mut ws, server) = sc_recovery_ws_harness(&root, &fixture.session_id).await;

    send_json(
        &mut ws,
        &WsInMessage::WorkItemPlanCompileRecoveryAction {
            action: WorkItemPlanCompileRecoveryActionDto::HumanTriage,
            reason: Some("需要人工整理依赖".to_string()),
        },
    )
    .await;
    let outcome = sc_recovery_await_outcome(&mut ws, false).await;
    assert!(
        !matches!(outcome, WsOutMessage::ProtocolError { .. }),
        "合法 SC recovery action 不得被协议面拒收: {outcome:?}"
    );

    // 引擎既有语义：同一 compile transaction 落原因（不新建第二个 compile），
    // 恢复节点收口，门回 human_confirm。
    let transactions = fixture.compile_transactions();
    assert_eq!(
        transactions.len(),
        1,
        "recovery 必须复用原 compile transaction，不得创建第二个 compile"
    );
    assert_eq!(
        transactions[0].status,
        WorkItemPlanCompileStatus::RecoveryRequired
    );
    assert_eq!(
        transactions[0].failure_reason.as_deref(),
        Some("需要人工整理依赖")
    );
    assert_eq!(transactions[0].step_cursor, "compile_failed");

    let nodes = fixture.timeline_nodes();
    let recovery = nodes
        .iter()
        .find(|node| node.node_id == SC_RECOVERY_NODE_ID)
        .expect("recovery node");
    assert_eq!(
        recovery.status,
        TimelineNodeStatus::Completed,
        "恢复节点必须在动作后收口"
    );
    assert!(
        nodes.iter().any(|node| {
            node.node_type == TimelineNodeType::HumanConfirm
                && node.status == TimelineNodeStatus::Active
        }),
        "恢复完成后必须回到 human_confirm 门: {nodes:?}"
    );
    assert_eq!(
        fixture.session_status(),
        WorkspaceSessionStatus::WaitingForHuman
    );

    drop(ws);
    server.abort();
}

// 场景 1b（合法 recovery action：abort_and_rollback）：同一合法门态下的回滚动作也
// 完整走引擎（事务落 Failed/rolled_back + previous plan 快照回写），证明放行不是
// 「静默吞掉」而是真接到既有执行面。
#[tokio::test]
async fn workspace_ws_sc_compile_recovery_abort_and_rollback_runs_through_engine() {
    use cadence_aria::product::models::{WorkItemPlanCompileStatus, WorkspaceSessionStatus};
    use cadence_aria::web::workspace_ws_types::{
        TimelineNodeStatus, TimelineNodeType, WorkItemPlanCompileRecoveryActionDto, WorkspaceStage,
    };

    let root = tempdir().expect("root");
    let fixture = create_sc_compile_recovery_fixture(
        &root,
        vec![sc_recovery_node(
            SC_RECOVERY_NODE_ID,
            TimelineNodeType::WorkItemPlanCompileRecovery,
            WorkspaceStage::HumanConfirm,
            TimelineNodeStatus::Active,
        )],
        WorkspaceSessionStatus::WaitingForHuman,
        true,
    )
    .await;
    let (mut ws, server) = sc_recovery_ws_harness(&root, &fixture.session_id).await;

    send_json(
        &mut ws,
        &WsInMessage::WorkItemPlanCompileRecoveryAction {
            action: WorkItemPlanCompileRecoveryActionDto::AbortAndRollback,
            reason: Some("放弃本次编译并回滚旧 Plan".to_string()),
        },
    )
    .await;
    let outcome = sc_recovery_await_outcome(&mut ws, false).await;
    assert!(
        !matches!(outcome, WsOutMessage::ProtocolError { .. }),
        "合法 SC recovery action 不得被协议面拒收: {outcome:?}"
    );

    let transactions = fixture.compile_transactions();
    assert_eq!(
        transactions.len(),
        1,
        "回滚不得创建第二个 compile transaction"
    );
    assert_eq!(transactions[0].status, WorkItemPlanCompileStatus::Failed);
    assert_eq!(transactions[0].step_cursor, "rolled_back");
    assert_eq!(
        transactions[0].failure_reason.as_deref(),
        Some("放弃本次编译并回滚旧 Plan")
    );
    assert!(
        fixture
            .lifecycle()
            .get_issue_work_item_plan("project_0001", "issue_0001", SC_RECOVERY_PLAN_ID)
            .is_ok(),
        "回滚必须把 previous plan 快照回写为现存 plan"
    );

    let nodes = fixture.timeline_nodes();
    assert_eq!(
        nodes
            .iter()
            .find(|node| node.node_id == SC_RECOVERY_NODE_ID)
            .expect("recovery node")
            .status,
        TimelineNodeStatus::Completed
    );
    assert!(
        nodes.iter().any(|node| {
            node.node_type == TimelineNodeType::HumanConfirm
                && node.status == TimelineNodeStatus::Active
        }),
        "回滚完成后必须回到 human_confirm 门: {nodes:?}"
    );

    drop(ws);
    server.abort();
}

// 场景 1c（continue）：协议面放行后由引擎裁决。集成层无法造出真 compile 制品
// （四 refs 由 in-crate fixture 生成），引擎在此 fail-closed——关键是收到的是
// 引擎错误码 `INVALID_COMPILE_RECOVERY_ACTION` 而非阶段错误，且零副作用。
#[tokio::test]
async fn workspace_ws_sc_compile_recovery_continue_reaches_engine_without_side_effects() {
    use cadence_aria::product::models::{WorkItemPlanCompileStatus, WorkspaceSessionStatus};
    use cadence_aria::web::workspace_ws_types::{
        TimelineNodeStatus, TimelineNodeType, WorkspaceStage,
    };

    let root = tempdir().expect("root");
    let fixture = create_sc_compile_recovery_fixture(
        &root,
        vec![sc_recovery_node(
            SC_RECOVERY_NODE_ID,
            TimelineNodeType::WorkItemPlanCompileRecovery,
            WorkspaceStage::HumanConfirm,
            TimelineNodeStatus::Active,
        )],
        WorkspaceSessionStatus::WaitingForHuman,
        true,
    )
    .await;
    let (mut ws, server) = sc_recovery_ws_harness(&root, &fixture.session_id).await;

    sc_recovery_send_continue(&mut ws).await;
    let outcome = sc_recovery_await_outcome(&mut ws, true).await;
    match &outcome {
        WsOutMessage::ProtocolError { code, message, .. } => {
            assert_eq!(
                code, "INVALID_COMPILE_RECOVERY_ACTION",
                "HumanConfirm + recovery 事实必须进入引擎裁决（而非阶段拒收）: {message}"
            );
            assert!(
                !message.contains("not allowed in stage"),
                "不得退化为阶段错误: {message}"
            );
            assert!(
                message.contains("source ref is missing"),
                "durable-only fixture 必须在引擎的单候选事务校验上 fail-closed: {message}"
            );
        }
        other => panic!("expected engine-level rejection, got {other:?}"),
    }

    // 零副作用：事务与 timeline 原样。
    let transactions = fixture.compile_transactions();
    assert_eq!(transactions.len(), 1, "fail-closed 不得创建第二个 compile");
    assert_eq!(
        transactions[0].status,
        WorkItemPlanCompileStatus::RecoveryRequired
    );
    assert_eq!(transactions[0].step_cursor, "compile_failed");
    assert_eq!(
        transactions[0].failure_reason.as_deref(),
        Some("final compile failed")
    );
    let nodes = fixture.timeline_nodes();
    assert_eq!(
        sc_recovery_active_node_ids(&nodes),
        vec![SC_RECOVERY_NODE_ID.to_string()],
        "fail-closed 不得改动 timeline: {nodes:?}"
    );

    drop(ws);
    server.abort();
}

// 场景 2/3/4（非 recovery 状态拒绝）：generate(Running) / 普通 HumanConfirm（缺
// recovery 事实）/ 终态 Confirmed 三类都必须零副作用拒收。
#[tokio::test]
async fn workspace_ws_compile_recovery_action_rejected_outside_recovery_state() {
    use cadence_aria::product::models::WorkspaceSessionStatus;
    use cadence_aria::web::workspace_ws_types::{
        TimelineNodeStatus, TimelineNodeType, WorkspaceStage,
    };

    // 非 recovery 状态的三类代表行：(1) 非门阶段（生成前 prepare_context）、
    // (2) AuthorConfirm 批次门、(3) 终态 Completed——矩阵均不放行 recovery action，
    // 必须零副作用拒收。
    //
    // 说明：Running/CrossReview/Revision 等 in-flight 阶段无法由 durable fixture 铺出
    // ——manager 创建期的 F-23 僵尸整流（`recover_stale_run_if_zombie`）会把「无活跃
    // run 却在 Running/CrossReview/Revision」的会话整回 prepare_context，故此处以
    // prepare_context 作非门阶段代表；in-flight 阶段的矩阵拒绝（返回 false）由 Task 1
    // 单测 `sc_human_confirm_accepts_compile_recovery_action_only_in_recovery_stage`
    // 逐阶段钉住。
    for (node_id, node_type, node_status, ws_stage, session_status, expected_stage) in [
        (
            "timeline_node_prepare_context",
            TimelineNodeType::PrepareContext,
            TimelineNodeStatus::Active,
            WorkspaceStage::PrepareContext,
            WorkspaceSessionStatus::Open,
            "prepare_context",
        ),
        (
            "timeline_node_batch_confirm",
            TimelineNodeType::WorkItemBatchConfirm,
            TimelineNodeStatus::Active,
            WorkspaceStage::AuthorConfirm,
            WorkspaceSessionStatus::WaitingForHuman,
            "author_confirm",
        ),
        (
            "timeline_node_completed",
            TimelineNodeType::Completed,
            TimelineNodeStatus::Completed,
            WorkspaceStage::Completed,
            WorkspaceSessionStatus::Confirmed,
            "completed",
        ),
    ] {
        let root = tempdir().expect("root");
        let fixture = create_sc_compile_recovery_fixture(
            &root,
            vec![sc_recovery_node(node_id, node_type, ws_stage, node_status)],
            session_status.clone(),
            false,
        )
        .await;
        let (mut ws, server) = sc_recovery_ws_harness(&root, &fixture.session_id).await;

        sc_recovery_send_continue(&mut ws).await;
        let outcome = sc_recovery_await_outcome(&mut ws, true).await;
        sc_recovery_assert_rejected_as_stage_error(&outcome, expected_stage);

        assert!(
            fixture.compile_transactions().is_empty(),
            "阶段拒收不得创建 compile transaction"
        );
        let nodes = fixture.timeline_nodes();
        assert_eq!(
            nodes.len(),
            1,
            "阶段拒收必须零副作用（timeline 不变）: {nodes:?}"
        );
        assert_eq!(fixture.session_status(), session_status);

        drop(ws);
        server.abort();
    }
}

// 场景 3（普通 HumanConfirm 门：无 recovery 事实）：矩阵放行（SC HumanConfirm 臂）
// → 引擎 durable 守卫 fail-closed `INVALID_COMPILE_RECOVERY_ACTION`，零副作用。
#[tokio::test]
async fn workspace_ws_human_confirm_without_recovery_fact_rejects_recovery_action() {
    use cadence_aria::product::models::WorkspaceSessionStatus;
    use cadence_aria::web::workspace_ws_types::{
        TimelineNodeStatus, TimelineNodeType, WorkspaceStage,
    };

    let root = tempdir().expect("root");
    let fixture = create_sc_compile_recovery_fixture(
        &root,
        vec![sc_recovery_node(
            "timeline_node_human_confirm",
            TimelineNodeType::HumanConfirm,
            WorkspaceStage::HumanConfirm,
            TimelineNodeStatus::Active,
        )],
        WorkspaceSessionStatus::WaitingForHuman,
        false,
    )
    .await;
    let (mut ws, server) = sc_recovery_ws_harness(&root, &fixture.session_id).await;

    sc_recovery_send_continue(&mut ws).await;
    let outcome = sc_recovery_await_outcome(&mut ws, true).await;
    match &outcome {
        WsOutMessage::ProtocolError { code, message, .. } => {
            assert_eq!(code, "INVALID_COMPILE_RECOVERY_ACTION");
            assert!(
                message.contains("requires active work_item_plan_compile_recovery node"),
                "缺 recovery 事实必须由引擎 durable 守卫拒收: {message}"
            );
        }
        other => panic!("expected engine-level rejection, got {other:?}"),
    }

    // 零副作用：无新节点、门/阶段不变、无 compile transaction、无子会话。
    let nodes = fixture.timeline_nodes();
    assert_eq!(nodes.len(), 1, "零副作用（timeline 不变）: {nodes:?}");
    assert_eq!(
        fixture.session_status(),
        WorkspaceSessionStatus::WaitingForHuman
    );
    assert!(fixture.compile_transactions().is_empty());
    let child_sessions: Vec<_> = fixture
        .lifecycle()
        .list_workspace_sessions("project_0001", "issue_0001")
        .expect("list workspace sessions")
        .into_iter()
        .filter(|session| {
            session.workspace_type == cadence_aria::product::models::WorkspaceType::WorkItem
        })
        .collect();
    assert!(
        child_sessions.is_empty(),
        "零副作用：不得创建子 WorkItem 会话: {child_sessions:?}"
    );

    drop(ws);
    server.abort();
}
