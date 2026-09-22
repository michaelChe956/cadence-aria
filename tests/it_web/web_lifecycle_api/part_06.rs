// ── change plan-compile-gate-visibility（WP-B Task 3）─────────────────────────
// HTTP confirm 端点在 WorkItemPlan 批次确认门（`AuthorConfirm` + active
// `WorkItemBatchConfirm`）上必须走引擎既有确认执行面（`confirm_work_item_plan`：
// plan Draft→Confirmed + 建子 WorkItem 会话 + stage→Completed + Completed 节点），
// 而不是端点自身的兜底通路。
//
// 背景实测（2026-09-22）：端点对 WorkItemPlan 一律 `NotHandled`，落到
// `confirm_workspace_entity` 的 WorkItemPlan 臂 → `work_item_plan_confirm_not_supported`
// （未映射稳定码 → 500）。因此批次确认门在今天**没有任何可用确认通路**（用户可见
// 的「无操作可行」），本任务把既有 typed HTTP confirm 接到既有引擎确认面，
// 不新增状态机、不改 action 枚举语义。
mod http_confirm_batch_gate {
    use axum::http::{Method, StatusCode};
    use cadence_aria::product::app_paths::ProductAppPaths;
    use cadence_aria::product::lifecycle_store::{
        CreateWorkItemInput, CreateWorkspaceSessionInput, LifecycleStore,
        WorkItemPlanSessionOptions,
    };
    use cadence_aria::product::models::{
        IssueWorkItemPlanOptions, IssueWorkItemPlanStatus, ProviderName, SingleCandidatePhase,
        WorkItemPlanStatus, WorkspaceSessionStatus, WorkspaceType,
    };
    use cadence_aria::product::work_item_plan_policy::{RunPolicy, WorkItemPlanFlowKind};
    use cadence_aria::web::app::build_web_router;
    use cadence_aria::web::runtime::WebRuntime;
    use cadence_aria::web::state::WebAppState;
    use cadence_aria::web::workspace_ws_types::{
        ProviderConfigSnapshot, TimelineNode, TimelineNodeStatus, TimelineNodeType, WorkspaceStage,
    };
    use futures_util::StreamExt;
    use serde_json::{Value, json};
    use tempfile::TempDir;
    use tokio::net::TcpListener;
    use tokio::time::{Duration, timeout};
    use tokio_tungstenite::connect_async;
    use tokio_tungstenite::tungstenite::Message;

    const BATCH_PLAN_ID: &str = "issue_work_item_plan_batch_confirm";
    const BATCH_NODE_ID: &str = "timeline_node_batch_confirm";
    const HUMAN_CONFIRM_NODE_ID: &str = "timeline_node_human_confirm";

    struct BatchConfirmFixture {
        _root: TempDir,
        _repo: TempDir,
        session_id: String,
        app_paths: ProductAppPaths,
        app: axum::Router,
    }

    impl BatchConfirmFixture {
        fn lifecycle(&self) -> LifecycleStore {
            LifecycleStore::new(self.app_paths.clone())
        }

        fn plan_status(&self) -> IssueWorkItemPlanStatus {
            self.lifecycle()
                .get_issue_work_item_plan("project_0001", "issue_0001", BATCH_PLAN_ID)
                .expect("load plan")
                .status
        }

        fn session(&self) -> cadence_aria::product::models::WorkspaceSessionRecord {
            self.lifecycle()
                .get_workspace_session(&self.session_id)
                .expect("load session")
        }

        fn work_item_child_sessions(
            &self,
        ) -> Vec<cadence_aria::product::models::WorkspaceSessionRecord> {
            self.lifecycle()
                .list_workspace_sessions("project_0001", "issue_0001")
                .expect("list sessions")
                .into_iter()
                .filter(|session| session.workspace_type == WorkspaceType::WorkItem)
                .collect()
        }

        fn timeline_nodes(&self) -> Vec<TimelineNode> {
            self.lifecycle()
                .load_timeline_nodes(&self.session_id)
                .expect("load timeline nodes")
        }

        fn confirm_uri(&self) -> String {
            format!("/api/workspace-sessions/{}/confirm", self.session_id)
        }

        /// 建 WS 连接（会话工厂随之创建 live engine——HTTP confirm 的引擎裁决依赖它）。
        async fn connect_ws(
            &self,
        ) -> (
            tokio_tungstenite::WebSocketStream<
                tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
            >,
            tokio::task::JoinHandle<()>,
        ) {
            let app = self.app.clone();
            let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
            let addr = listener.local_addr().expect("local addr");
            let server = tokio::spawn(async move {
                axum::serve(listener, app).await.expect("serve");
            });
            let (mut ws, _) = connect_async(format!(
                "ws://{addr}/api/workspace-sessions/{}/ws",
                self.session_id
            ))
            .await
            .expect("connect ws");
            let initial = timeout(Duration::from_secs(5), ws.next())
                .await
                .expect("initial frame timeout")
                .expect("initial frame")
                .expect("initial frame ok");
            let Message::Text(text) = initial else {
                panic!("expected text ws frame, got {initial:?}");
            };
            let value: Value = serde_json::from_str(&text).expect("initial frame json");
            assert_eq!(value["type"], "session_state", "{value}");
            (ws, server)
        }
    }

    fn batch_gate_node(
        node_id: &str,
        node_type: TimelineNodeType,
        stage: WorkspaceStage,
    ) -> TimelineNode {
        TimelineNode {
            node_id: node_id.to_string(),
            node_type,
            agent: None,
            stage,
            round: None,
            status: TimelineNodeStatus::Active,
            title: "Work Item Batch 确认".to_string(),
            summary: Some("等待整组 Work Item Draft 确认".to_string()),
            started_at: "2026-09-22T00:00:00Z".to_string(),
            completed_at: None,
            duration_ms: None,
            artifact_ref: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Fake,
                reviewer: None,
                review_rounds: 0,
                permission_modes:
                    cadence_aria::product::models::WorkspaceRolePermissionModes::default(),
            },
            retry: None,
        }
    }

    /// project/repository/issue 经 HTTP 建好（WS 会话工厂要求三者可解析），plan 与
    /// compiled WorkItems 经 store 落地（`work_item_ids` 非空是 `confirm_work_item_plan`
    /// 的前置），会话铺 single_candidate 流 + 给定门节点。
    async fn create_batch_confirm_fixture(
        node_id: &str,
        node_type: TimelineNodeType,
        node_stage: WorkspaceStage,
    ) -> BatchConfirmFixture {
        let root = tempfile::tempdir().expect("root");
        let repo = super::git_repo();
        let app = build_web_router(WebAppState::new(
            root.path().to_path_buf(),
            WebRuntime::new_fake(root.path().to_path_buf()),
        ));
        super::request_json(
            app.clone(),
            Method::POST,
            "/api/projects",
            json!({"name":"BatchConfirm","description":null}),
        )
        .await;
        crate::create_repository_and_wait(
            app.clone(),
            "project_0001",
            json!({"name":"Repo","path":repo.path()}),
        )
        .await;
        let (status, issue) = super::request_json(
            app.clone(),
            Method::POST,
            "/api/projects/project_0001/issues",
            json!({"title":"批次确认门","description":null,"repository_id":"repository_0001"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{issue}");

        let app_paths = ProductAppPaths::new(root.path().join(".aria"));
        let lifecycle = LifecycleStore::new(app_paths.clone());
        let mut work_item_ids = Vec::new();
        for index in 1..=2 {
            let work_item = lifecycle
                .create_work_item(CreateWorkItemInput {
                    id: Some(format!("work_item_batch_000{index}")),
                    project_id: "project_0001".to_string(),
                    issue_id: "issue_0001".to_string(),
                    repository_id: "repository_0001".to_string(),
                    title: format!("批次工作项 {index}"),
                    plan_status: WorkItemPlanStatus::Draft,
                    ..Default::default()
                })
                .expect("create compiled work item");
            work_item_ids.push(work_item.id);
        }
        let plan = lifecycle
            .create_issue_work_item_plan(
                cadence_aria::product::lifecycle_store::CreateIssueWorkItemPlanInput {
                    id: Some(BATCH_PLAN_ID.to_string()),
                    project_id: "project_0001".to_string(),
                    issue_id: "issue_0001".to_string(),
                    source_story_spec_ids: vec![],
                    source_design_spec_ids: vec![],
                    options: IssueWorkItemPlanOptions {
                        include_integration_tests: false,
                        include_e2e_tests: false,
                        force_frontend_backend_split: false,
                        require_execution_plan_confirm: false,
                    },
                    status: IssueWorkItemPlanStatus::Draft,
                    work_item_ids,
                    repository_profile_ref: None,
                    verification_plan_ids: vec![],
                    dependency_graph: vec![],
                    created_from_provider_run: None,
                    validator_findings: vec![],
                },
            )
            .expect("create plan");
        let session = lifecycle
            .create_workspace_session(CreateWorkspaceSessionInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                entity_id: plan.id.clone(),
                workspace_type: WorkspaceType::WorkItemPlan,
                author_provider: ProviderName::Fake,
                reviewer_provider: ProviderName::Fake,
                review_rounds: 0,
                superpowers_enabled: false,
                openspec_enabled: false,
                work_item_plan_options: Some(WorkItemPlanSessionOptions {
                    flow_kind: WorkItemPlanFlowKind::SingleCandidate,
                    run_policy: RunPolicy::Interactive,
                    rollout_snapshot: true,
                }),
            })
            .expect("create plan session");
        let mut record = lifecycle
            .get_workspace_session(&session.id)
            .expect("load plan session");
        record.status = WorkspaceSessionStatus::WaitingForHuman;
        record.single_candidate_phase = Some(SingleCandidatePhase::Approval);
        let session_path = app_paths
            .issue_root(&record.project_id, &record.issue_id)
            .join("workspace-sessions")
            .join(format!("{}.json", record.id));
        cadence_aria::product::json_store::write_json(&session_path, &record)
            .expect("persist plan session");
        lifecycle
            .save_timeline_nodes(
                &session.id,
                &[batch_gate_node(node_id, node_type, node_stage)],
            )
            .expect("persist gate timeline");

        BatchConfirmFixture {
            _root: root,
            _repo: repo,
            session_id: session.id,
            app_paths,
            app,
        }
    }

    /// 等一条 session_state 帧并取出 stage（端点 200 后的权威收敛帧）。
    async fn await_session_state_stage(
        ws: &mut tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    ) -> String {
        for _ in 0..40 {
            let frame = timeout(Duration::from_secs(5), ws.next())
                .await
                .expect("post-confirm frame timeout")
                .expect("post-confirm frame")
                .expect("post-confirm frame ok");
            let Message::Text(text) = frame else {
                continue;
            };
            let value: Value = serde_json::from_str(&text).expect("post-confirm frame json");
            if value["type"] == "session_state" {
                return value["stage"].as_str().unwrap_or_default().to_string();
            }
        }
        panic!("no session_state frame after confirm");
    }

    // 场景 a + d：批次确认门 → 引擎执行面确认（plan Confirmed + 子会话 + Completed +
    // stage 投影 completed），重复 confirm 走既有终态守卫且零副作用。
    #[tokio::test]
    async fn http_confirm_on_work_item_plan_batch_gate_runs_engine_then_is_idempotent() {
        let fixture = create_batch_confirm_fixture(
            BATCH_NODE_ID,
            TimelineNodeType::WorkItemBatchConfirm,
            WorkspaceStage::AuthorConfirm,
        )
        .await;
        let (mut ws, server) = fixture.connect_ws().await;

        let (status, body) = super::request_json(
            fixture.app.clone(),
            Method::POST,
            &fixture.confirm_uri(),
            json!({"confirmed_by":"user"}),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "WorkItemPlan 批次确认门必须经引擎确认面成功: {body}"
        );
        assert_eq!(body["status"], "confirmed", "{body}");

        // 引擎执行面：plan Draft→Confirmed、子 WorkItem 会话已建、门节点收口 + Completed 节点。
        assert_eq!(fixture.plan_status(), IssueWorkItemPlanStatus::Confirmed);
        let child_sessions = fixture.work_item_child_sessions();
        assert_eq!(
            child_sessions.len(),
            2,
            "confirm 必须为整组 Work Item 建子会话: {child_sessions:?}"
        );
        let nodes = fixture.timeline_nodes();
        assert_eq!(
            nodes
                .iter()
                .find(|node| node.node_id == BATCH_NODE_ID)
                .expect("batch gate node")
                .status,
            TimelineNodeStatus::Completed,
            "批次门节点必须收口: {nodes:?}"
        );
        assert!(
            nodes
                .iter()
                .any(|node| node.node_type == TimelineNodeType::Completed
                    && node.status == TimelineNodeStatus::Completed),
            "必须落 Completed 节点: {nodes:?}"
        );
        assert_eq!(fixture.session().status, WorkspaceSessionStatus::Confirmed);

        // stage 投影收敛为 completed——矩阵对 SC 会话只在 completed 放行 advance，
        // 这是「确认后 advance 可达」的服务端权威前置（不再停在 author_confirm）。
        assert_eq!(await_session_state_stage(&mut ws).await, "completed");

        // 场景 d：重复 confirm 走既有终态守卫（已 Confirmed 幂等返回），零副作用。
        let (status, body) = super::request_json(
            fixture.app.clone(),
            Method::POST,
            &fixture.confirm_uri(),
            json!({"confirmed_by":"user"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "已 Confirmed 必须幂等返回: {body}");
        assert_eq!(body["status"], "confirmed", "{body}");
        assert_eq!(
            fixture.work_item_child_sessions().len(),
            2,
            "重复 confirm 不得重复建子会话"
        );
        assert_eq!(
            fixture.timeline_nodes().len(),
            nodes.len(),
            "重复 confirm 不得追加节点"
        );
        assert_eq!(fixture.plan_status(), IssueWorkItemPlanStatus::Confirmed);
        assert_eq!(fixture.session().status, WorkspaceSessionStatus::Confirmed);

        drop(ws);
        server.abort();
    }

    // 场景 b：WorkItemPlan 无送审语义 → with_review=true 如实 422，且不触碰任何 durable 状态。
    #[tokio::test]
    async fn http_confirm_with_review_on_work_item_plan_batch_gate_is_rejected() {
        let fixture = create_batch_confirm_fixture(
            BATCH_NODE_ID,
            TimelineNodeType::WorkItemBatchConfirm,
            WorkspaceStage::AuthorConfirm,
        )
        .await;
        let (ws, server) = fixture.connect_ws().await;

        let (status, body) = super::request_json(
            fixture.app.clone(),
            Method::POST,
            &fixture.confirm_uri(),
            json!({"confirmed_by":"user","with_review":true}),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "WorkItemPlan 不支持送审，必须如实 422: {body}"
        );
        assert_eq!(
            body["code"], "workspace_session_review_not_enabled",
            "{body}"
        );

        assert_eq!(
            fixture.plan_status(),
            IssueWorkItemPlanStatus::Draft,
            "422 不得推进 plan"
        );
        assert!(
            fixture.work_item_child_sessions().is_empty(),
            "422 不得建子会话"
        );
        assert_eq!(
            fixture.session().status,
            WorkspaceSessionStatus::WaitingForHuman,
            "422 不得改会话状态"
        );

        drop(ws);
        server.abort();
    }

    // 场景 c：非批次门的 WorkItemPlan 形态（此处 SC 对话门 HumanConfirm）维持既有通路语义
    // ——`NotHandled` → `confirm_workspace_entity` 的 WorkItemPlan 臂原样拒绝
    // （`work_item_plan_confirm_not_supported`，未映射稳定码 → 500），零副作用。
    #[tokio::test]
    async fn http_confirm_on_work_item_plan_human_confirm_keeps_existing_semantics() {
        let fixture = create_batch_confirm_fixture(
            HUMAN_CONFIRM_NODE_ID,
            TimelineNodeType::HumanConfirm,
            WorkspaceStage::HumanConfirm,
        )
        .await;
        let (ws, server) = fixture.connect_ws().await;

        let (status, body) = super::request_json(
            fixture.app.clone(),
            Method::POST,
            &fixture.confirm_uri(),
            json!({"confirmed_by":"user"}),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::INTERNAL_SERVER_ERROR,
            "非批次门形态维持既有拒绝通路: {body}"
        );
        assert_eq!(
            body["code"], "work_item_plan_confirm_not_supported",
            "{body}"
        );

        assert_eq!(
            fixture.plan_status(),
            IssueWorkItemPlanStatus::Draft,
            "非批次门形态不得确认 plan"
        );
        assert!(
            fixture.work_item_child_sessions().is_empty(),
            "非批次门形态不得建子会话"
        );
        assert_eq!(
            fixture.session().status,
            WorkspaceSessionStatus::WaitingForHuman,
            "非批次门形态不得改会话状态"
        );

        drop(ws);
        server.abort();
    }
}
