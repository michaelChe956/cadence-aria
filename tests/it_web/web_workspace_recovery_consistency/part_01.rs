use axum::http::{Method, StatusCode};
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::checkpoint_store::CheckpointStore;
use cadence_aria::product::lifecycle_store::LifecycleStore;
use cadence_aria::product::models::{ProviderName, WorkspaceType};
use cadence_aria::product::workspace_engine::{EngineEvent, WorkspaceEngine, WorkspaceSession};
use cadence_aria::web::workspace_ws_types::{
    ArtifactPayload, ProviderConfigSnapshot, TimelineNode, TimelineNodeStatus, TimelineNodeType,
    ValidatorFindingDto, VerificationCommandDto, VerificationManualCheckDto, VerificationPlanDto,
    WorkItemCandidateDto, WorkItemCandidateMetaDto, WorkItemDependencyEdgeDto,
    WorkItemPlanCandidateDto, WorkItemPlanDto, WorkItemSplitOptionsDto, WorkspaceStage,
    WsCheckpointDto, WsMessageDto, WsOutMessage, WsProviderConfig,
};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::time::{Duration, timeout};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

use cadence_aria::cross_cutting::provider_adapter::{ProviderAdapter, ProviderAdapterError};
use cadence_aria::cross_cutting::provider_availability_gate::{
    ProviderAvailabilityGate, ProviderHealthSource,
};
use cadence_aria::cross_cutting::provider_health::{ProviderHealthEntry, ProviderHealthSnapshot};
use cadence_aria::product::logical_codebase::LogicalCodebaseStore;
use cadence_aria::protocol::contracts::{AdapterInput, AdapterOutput};
use cadence_aria::web::app::build_web_router;
use cadence_aria::web::gateway_factory::LogicalCodebaseGatewayFactory;
use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::state::WebAppState;

use crate::web_work_item_generation::request_json;

static WS_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;



async fn connect_ws(app: axum::Router, session_id: &str) -> WsStream {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/api/ws/workspace/{session_id}");
    let (ws, _) = connect_async(url).await.expect("connect ws");

    tokio::spawn(async move {
        server.await.ok();
    });

    ws
}

async fn recv_ws_until<F>(ws: &mut WsStream, timeout_after: Duration, predicate: F) -> Vec<Value>
where
    F: Fn(&[Value]) -> bool,
{
    let mut messages = Vec::new();
    let deadline = tokio::time::Instant::now() + timeout_after;
    while tokio::time::Instant::now() < deadline {
        let remaining = deadline - tokio::time::Instant::now();
        match timeout(remaining, ws.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => match serde_json::from_str(&text) {
                Ok(value) => {
                    messages.push(value);
                    if predicate(&messages) {
                        break;
                    }
                }
                Err(error) => panic!("ws json parse error: {error}\n{text}"),
            },
            Ok(Some(Ok(Message::Close(_)))) => break,
            Ok(Some(Ok(other))) => {
                eprintln!("ignoring non-text ws message: {other:?}");
                continue;
            }
            Ok(Some(Err(error))) => panic!("ws error: {error}"),
            Ok(None) => break,
            Err(_) => break,
        }
    }
    messages
}


// ---------------------------------------------------------------------------
// LC 收敛后的 design author 落盘链夹具（本模块专用）
// ---------------------------------------------------------------------------
// 生产契约（方案 X 阶段1收敛 + lc-root-initialization）：issue 恒挂 logical
// codebase 归属（块3），story/design 的 author 轮必经 LC provider gateway 启动
// （provider_run.rs `resolve_author_root_launch` → `drive_author_provider_
// session_via_gateway`）。gateway 方言表只认 ClaudeCode/Codex/Pi/KimiCode——
// Fake 一律 fail-closed `provider_unsupported_for_gateway_launch`（见
// provider_gateway.rs `ProviderRef::from_provider_name`），绝不静默回退。
// 共享夹具（web_work_item_generation）的 state 缺 gateway 启动前置（默认
// health 服务无刷新通道恒 degraded、capability 行恒 Unknown），且该文件已
// 定稿，故按 web_coding_ws_handler/fixtures.rs 的 gateway 先例本地补齐：
//   ① author 走 claude_code 方言槽位——registry 的 TestControlled 槽位回落
//      FakeStreamingProvider，对「Workspace 类型: Design Spec」prompt 产出
//      确定性 `# Design Spec` markdown（fake.rs 设计分支满足 Design gate：
//      七个必需标题 + [DEC-*]/[CMP-*]/[API-*] + source id）；
//   ② capability 行播种 Confirmed（provider_capability_fixture 共享助手，
//      LC 子树 + project legacy 子树双种，同 web_lc_operations_api 先例；
//      gateway launch/write_boundary 分格需 Confirmed，bootstrap 行恒 Unknown）；
//   ③ gateway factory 直挂恒健康 availability gate（spawn 前复验 #9 消费；
//      测试环境无真实 provider 二进制，默认 health 服务无磁盘加载路径）。
struct RecoveryStubSyncAdapter;

impl ProviderAdapter for RecoveryStubSyncAdapter {
    fn run(&self, _input: &AdapterInput) -> Result<AdapterOutput, ProviderAdapterError> {
        Err(ProviderAdapterError::provider_unavailable(
            "recovery design fixture sync adapter is not expected to run",
        ))
    }
}

fn recovery_always_healthy_gate() -> Arc<ProviderAvailabilityGate> {
    struct AlwaysHealthy(Arc<ProviderHealthSnapshot>);

    impl ProviderHealthSource for AlwaysHealthy {
        fn snapshot(&self) -> Arc<ProviderHealthSnapshot> {
            self.0.clone()
        }

        fn degraded(&self) -> bool {
            false
        }
    }

    let checked_at = chrono::Utc::now();
    let snapshot = Arc::new(ProviderHealthSnapshot {
        schema_version: 1,
        generation: 1,
        checked_at,
        providers: [ProviderName::ClaudeCode]
            .into_iter()
            .map(|provider| ProviderHealthEntry {
                provider,
                command: "stub".to_string(),
                available: true,
                version: Some("1.0".to_string()),
                reason_code: None,
                reason: None,
                checked_at,
            })
            .collect(),
    });
    Arc::new(ProviderAvailabilityGate::new(Arc::new(AlwaysHealthy(
        snapshot,
    ))))
}

/// 与共享夹具同序的 HTTP bootstrap（project → repo init → 身份迁移 → 聚合
/// 索引重建 → issue → story/design generate+confirm），差异仅在 state 的
/// gateway factory 与 capability 播种；bootstrap 会话不驱动 author 轮，
/// store-only confirm 路径不受 gateway 影响。
async fn app_with_gateway_launchable_design_history() -> (axum::Router, tempfile::TempDir) {
    let root = tempfile::tempdir().expect("root");
    let repo = root.path().join("repo");
    // REQ-PIB-02：夹具对齐生产不变量（main 分支+初始提交）；聚合索引布局
    // 规则要求每成员至少 1 个可索引源码文件（aggregate_index_member_coverage_
    // failed），空提交不算。
    std::fs::create_dir_all(repo.join("src")).expect("repo src dir");
    std::fs::write(repo.join("src/main.rs"), "fn main() {}\n").expect("repo source");
    let status = std::process::Command::new("git")
        .args(["init", "-b", "main"])
        .current_dir(&repo)
        .status()
        .expect("git init");
    assert!(status.success());
    for args in [
        vec!["config", "user.email", "test@example.com"],
        vec!["config", "user.name", "Test User"],
        vec!["add", "."],
        vec!["commit", "-m", "fixture baseline"],
    ] {
        let status = std::process::Command::new("git")
            .args(&args)
            .current_dir(&repo)
            .status()
            .expect("git fixture");
        assert!(status.success());
    }

    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    // 默认 fake 模式 registry（全 provider 名 → TestControlled →
    // FakeStreamingProvider 回落）即为 gateway 方言槽位；只重建 gateway
    // factory（恒健康 gate + 同一 registry——注入 registry 后须同步重建
    // factory 的坑见 state.rs `with_events_and_provider_registry` 注释）。
    let state = WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    );
    let registry = state.provider_registry.clone();
    let state = state.with_gateway_factory(Arc::new(LogicalCodebaseGatewayFactory::new(
        app_paths.clone(),
        registry,
        Arc::new(RecoveryStubSyncAdapter),
        recovery_always_healthy_gate(),
    )));
    let app = build_web_router(state);

    let (status, response) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects",
        json!({"name":"Lifecycle","description":null}),
    )
    .await;
    assert!(status.is_success(), "bootstrap project failed: {response}");
    let (status, response) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/repositories",
        json!({"name":"Repo","path":repo}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::ACCEPTED,
        "bootstrap repository initialization start failed: {response}"
    );
    let operation_id = response["operation_id"]
        .as_str()
        .expect("bootstrap repository initialization operation id")
        .to_string();
    let operation_uri =
        format!("/api/projects/project_0001/repository-initializations/{operation_id}");
    let mut last_snapshot = response;
    let mut completed = None;
    for _ in 0..100 {
        let (status, snapshot) =
            request_json(app.clone(), Method::GET, &operation_uri, json!({})).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "bootstrap repository initialization poll failed: {snapshot}"
        );
        match snapshot["status"].as_str() {
            Some("completed") => {
                completed = Some(snapshot);
                break;
            }
            Some("failed") => panic!("bootstrap repository initialization failed: {snapshot}"),
            _ => last_snapshot = snapshot,
        }
        tokio::task::yield_now().await;
    }
    let completed = completed.unwrap_or_else(|| {
        panic!("bootstrap repository initialization did not complete: {last_snapshot}")
    });
    let repository_id = completed["result"]["repository"]["repository_id"]
        .as_str()
        .expect("bootstrap initialized repository id")
        .to_string();

    // 块3：issue 必须挂 logical_codebase_id 归属；按生产 v1.2→v1.3 身份迁移
    // 把 repository 升成 LC 权威，再经**生产聚合索引操作 + 假 CodeGraph CLI**
    // （`aggregate_index_fixture`）建 active 聚合索引。夹具不真跑 `codegraph sync .`
    // （本机重活：抢全局 `/tmp/.codegraph` 锁 + 吃 30s 增量预算 ⇒ 422
    // codegraph_timeout）；假 CLI 只替换 CLI 应答，真实 git 成员快照 / 成员覆盖校验 /
    // durable 世代落盘 / active 投影全走生产链路。rebuild 端点本身的行为覆盖由
    // `web_aggregate_index_api` 承担。
    let involved: Vec<String> = {
        let _codegraph_guard = crate::CODEGRAPH_FIXTURE_LOCK.lock().await;
        cadence_aria::product::logical_codebase::IdentityMigrationExecutor::new(
            app_paths.clone(),
        )
        .ensure_identity_schema("project_0001")
        .expect("migrate fixture repository into logical codebase authority");
        let lc_id =
            cadence_aria::product::logical_codebase::store::legacy_logical_codebase_id(
                "project_0001",
            );
        crate::aggregate_index_fixture::seed_active_aggregate_index(
            &app_paths,
            "project_0001",
            &lc_id,
        );
        let involved: Vec<String> = LogicalCodebaseStore::new(app_paths.clone())
            .list_members("project_0001")
            .expect("list fixture logical members")
            .into_iter()
            .map(|member| member.logical_repository_id.0.to_string())
            .collect();
        let (status, response) = request_json(
            app.clone(),
            Method::POST,
            "/api/projects/project_0001/issues",
            json!({"title":"登录会话过期","description":"描述","repository_id":repository_id,"logical_codebase_id":lc_id}),
        )
        .await;
        assert!(status.is_success(), "bootstrap issue failed: {response}");
        // capability 播种：作用域 = 项目级全局（owner 2026-11-04）⇒ 只种一次。
        crate::provider_capability_fixture::seed_confirmed_claude(&app_paths, "project_0001");
        involved
    };

    let (status, response) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/story-specs:generate",
        json!({
            "title":"登录会话过期提示",
            "involved_repository_ids":involved,
            "author_provider":"fake",
            "reviewer_provider":"codex",
            "review_rounds":3,
            "superpowers_enabled":false,
            "openspec_enabled":true
        }),
    )
    .await;
    assert!(
        status.is_success(),
        "bootstrap story generation failed: {response}"
    );
    let (status, response) = request_json(
        app.clone(),
        Method::POST,
        "/api/workspace-sessions/workspace_session_0001/confirm",
        json!({"confirmed_by":"human"}),
    )
    .await;
    assert!(
        status.is_success(),
        "bootstrap story confirmation failed: {response}"
    );

    let (status, response) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/design-specs:generate",
        json!({
            "title":"会话过期后端设计",
            "story_spec_ids":["story_spec_0001"],
            "involved_repository_ids":involved,
            "author_provider":"codex",
            "reviewer_provider":"claude_code",
            "review_rounds":2,
            "superpowers_enabled":true,
            "openspec_enabled":true
        }),
    )
    .await;
    assert!(
        status.is_success(),
        "bootstrap design generation failed: {response}"
    );
    let (status, response) = request_json(
        app.clone(),
        Method::POST,
        "/api/workspace-sessions/workspace_session_0002/confirm",
        json!({"confirmed_by":"human"}),
    )
    .await;
    assert!(
        status.is_success(),
        "bootstrap design confirmation failed: {response}"
    );

    (app, root)
}

async fn generate_session_to_author_confirm(
    app: &axum::Router,
    endpoint: &str,
    body: Value,
) -> String {
    let (_status, resp) = request_json(app.clone(), Method::POST, endpoint, body).await;
    let session_id = resp["workspace_session"]["workspace_session_id"]
        .as_str()
        .unwrap()
        .to_string();

    let mut ws = connect_ws(app.clone(), &session_id).await;
    ws.send(Message::Text(
        json!({
            "type": "start_generation",
            "provider_config": { "author": "claude_code", "reviewer": null, "review_rounds": 0 },
            "reviewer_enabled": false
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send start_generation");

    let _messages = recv_ws_until(&mut ws, Duration::from_secs(15), |msgs| {
        msgs.iter().any(|m| m["type"] == "artifact_update")
            && msgs
                .iter()
                .any(|m| m["type"] == "stage_change" && m["stage"] == "author_confirm")
    })
    .await;

    ws.close(None).await.ok();
    session_id
}


fn recover_engine(repo: &tempfile::TempDir, session_id: &str) -> WorkspaceEngine {
    let app_paths = ProductAppPaths::new(repo.path().join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let checkpoint_store = Arc::new(CheckpointStore::new(
        app_paths.issue_lifecycle_root("project_0001", "issue_0001"),
    ));
    let (event_tx, _event_rx) = mpsc::channel::<EngineEvent>(1);
    let session_record = lifecycle
        .get_workspace_session(session_id)
        .expect("session record exists");
    let mut session = WorkspaceSession::from_record(session_record);
    session.repository_path = Some(repo.path().join("repo"));
    WorkspaceEngine::new_persistent(checkpoint_store, lifecycle, event_tx, session)
}

// 退役留档（T5/REQ-RET-02）：`story_design_work_item_plan_recovery_consistency` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。


// 退役留档（T5/REQ-RET-02）：`story_workspace_review_sentinel_fallback_still_passes` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

#[tokio::test]
async fn design_workspace_artifact_history_still_loads_markdown() {
    let _guard = WS_TEST_LOCK.lock().await;
    let (app, repo) = app_with_gateway_launchable_design_history().await;

    let design_session_id = generate_session_to_author_confirm(
        &app,
        "/api/projects/project_0001/issues/issue_0001/design-specs:generate",
        json!({
            "title": "Design artifact history",
            "story_spec_ids": ["story_spec_0001"],
            "author_provider": "claude_code",
            "reviewer_provider": null,
            "review_rounds": 1,
            "superpowers_enabled": false,
            "openspec_enabled": true
        }),
    )
    .await;

    let app_paths = ProductAppPaths::new(repo.path().join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths);
    let versions = lifecycle
        .list_artifact_versions(&design_session_id)
        .expect("list design artifact versions");
    assert!(
        versions
            .iter()
            .any(|version| version.is_current && version.markdown().contains("# Design Spec")),
        "Design artifact history should keep readable markdown versions: {versions:?}"
    );

    let design_engine = recover_engine(&repo, &design_session_id);
    match design_engine.build_session_state() {
        WsOutMessage::SessionState {
            workspace_type,
            artifact,
            artifact_versions,
            artifact_version_summaries,
            ..
        } => {
            assert_eq!(workspace_type, WorkspaceType::Design);
            assert!(artifact_versions.is_empty());
            assert!(
                artifact_version_summaries
                    .iter()
                    .any(|summary| summary.is_current && summary.markdown_size > 0),
                "SessionState should expose Design artifact history summaries"
            );
            let markdown = artifact
                .as_ref()
                .and_then(|payload| payload.markdown())
                .expect("recovered design artifact markdown");
            assert!(markdown.contains("# Design Spec"));
        }
        other => panic!("expected Design SessionState, got {other:?}"),
    }
}

// 退役留档（T5/REQ-RET-02）：`ordinary_work_item_workspace_review_unaffected` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。
