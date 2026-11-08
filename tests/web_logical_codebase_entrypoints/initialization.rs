use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::http::{Method, StatusCode};
use cadence_aria::cross_cutting::aria_state_paths::AriaStatePaths;
use cadence_aria::cross_cutting::bounded_command_runner::{
    BoundedCommandError, BoundedCommandRequest, BoundedCommandResult, BoundedCommandRunner,
    TokioBoundedCommandRunner,
};
use cadence_aria::cross_cutting::provider_availability_gate::ProviderAvailabilityGate;
use cadence_aria::cross_cutting::provider_health::{
    ProviderHealthService, SystemProviderHealthClock,
};
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::logical_codebase::aggregate_initialization_coordinator::{
    AggregateInitializationCoordinator, AggregateInitializationError, AggregatePreflightService,
    AggregatePreflightSnapshot, AggregateProviderTurnDriver, AggregateProviderTurnRequest,
    AggregateSkillsPreparation, MachineSkillsPreparation,
};
use cadence_aria::product::logical_codebase::aggregate_initialization_store::AggregateInitializationOperationStore;
use cadence_aria::product::logical_codebase::store::{
    LogicalCodebaseManifest, LogicalCodebaseStore,
};
use cadence_aria::product::project_store::{CreateProjectInput, ProjectStore};
use cadence_aria::web::app::build_web_router;
use cadence_aria::web::handlers::AggregateInitializationDependencies;
use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::state::{InitializationRunRegistry, WebAppState};
use tempfile::TempDir;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

/// 四家 provider 的版本探测一律成功的 fake 命令 runner。
///
/// 只认「provider 可执行文件 + `--version`」这一种组合 ⇒ 其它命令（codegraph、
/// git 等）原样委托生产 runner ⇒ 夹具的执行路径与注入前完全一致。
struct AllProvidersAvailableRunner {
    inner: TokioBoundedCommandRunner,
}

#[async_trait]
impl BoundedCommandRunner for AllProvidersAvailableRunner {
    async fn run(
        &self,
        request: BoundedCommandRequest,
    ) -> Result<BoundedCommandResult, BoundedCommandError> {
        // 版本取各家最低兼容版本（与 lib 侧 `selector_seed.inc.rs` 的 stub 同款），
        // 避免「低于最低版本」把可用性改写（kimi 的最低版本是 0.34.0）。
        let version = match (
            request.executable.as_str(),
            request.argv.first().map(String::as_str),
        ) {
            ("claude", Some("--version")) => "1.0",
            ("codex", Some("--version")) => "2.0",
            ("pi", Some("--version")) => "0.83.0",
            ("kimi", Some("--version")) => "0.34.0",
            _ => return self.inner.run(request).await,
        };
        Ok(BoundedCommandResult {
            exit_code: Some(0),
            stdout: format!("{version}\n"),
            stderr: String::new(),
            timed_out: false,
            cancelled: false,
            stdout_truncated: false,
            stderr_truncated: false,
            duration_ms: 0,
        })
    }
}

/// 夹具专用的 provider health 注入：让「provider 命令可用」成为**夹具的显式事实**。
///
/// 生产同构的创建期 provider 选择门（`select_aggregate_provider`，REQ-BOOT-07）
/// 有两层判据：L1 = provider health 快照的 `available`（真机上跑
/// `claude`/`codex`/`pi`/`kimi --version` 的**真实**探测）、L2 = capability
/// 记录的 launch 分格（由 `capability_fixture::seed_confirmed_capabilities`
/// 物化）。`WebAppState::new` 注入的是真实探测服务 ⇒ 夹具的 L1 结论取决于
/// **宿主 PATH 上装没装 provider CLI**：CI runner 上四家全不可用 ⇒ 创建期
/// 500 `aggregate_initialization_no_provider_available`（202 变 500）。
///
/// 本 helper 把 L1 也变成夹具的显式事实：注入带 fake runner 的
/// `ProviderHealthService`（四家版本探测一律成功）⇒ 选择结论与宿主环境无关
/// （hermetic），与 L2 的显式播种同款做法。
///
/// 先例（照抄，不自创写法）：
/// - `tests/web_logical_codebase_entrypoints/planning_p0.rs`（`P0CommandRunner`
///   + `ProviderHealthService::with_dependencies` + `state.with_provider_health(...)`）；
/// - `src/web/handlers/aggregate_initialization/tests_parts/selector_seed.inc.rs`
///   （lib 侧同款 `with_provider_health_all_available`：stub runner 让四家版本
///   探测全部成功）。
///
/// 与先例的**有意差别**：先例的 stub 只回 `claude`/`codex`，`pi`/`kimi` 仍落
/// 真实执行 ⇒ 「pi 是否可用」仍取决于宿主 PATH；本 helper 让**四家**版本探测
/// 一律成功 ⇒ 候选表的 `cli_available` 与宿主环境完全无关。
pub(crate) fn with_provider_health_all_available(state: WebAppState, root: &Path) -> WebAppState {
    let runner: Arc<dyn BoundedCommandRunner> = Arc::new(AllProvidersAvailableRunner {
        inner: TokioBoundedCommandRunner,
    });
    let health = Arc::new(ProviderHealthService::with_dependencies(
        AriaStatePaths::from_workspace_root(root),
        runner.clone(),
        Arc::new(SystemProviderHealthClock),
        Duration::from_secs(1),
        4096,
    ));
    let gate = Arc::new(ProviderAvailabilityGate::new(health.clone()));
    state.with_provider_health(health, gate, runner)
}

struct NoopSkills;

#[async_trait]
impl AggregateSkillsPreparation for NoopSkills {
    async fn prepare_skills(
        &self,
        _project_id: &str,
        _operation_id: &str,
        _cancellation: CancellationToken,
    ) -> Result<MachineSkillsPreparation, AggregateInitializationError> {
        Ok(MachineSkillsPreparation {
            source_digest: "sha256:test-source".to_string(),
            link_digest: "sha256:test-links".to_string(),
            skills_root: PathBuf::from("/test-skills"),
            warnings: Vec::new(),
        })
    }
}

struct NoopPreflight;

impl AggregatePreflightService for NoopPreflight {
    fn inspect(
        &self,
        _project_id: &str,
        manifest: &LogicalCodebaseManifest,
        cancellation: &CancellationToken,
    ) -> Result<AggregatePreflightSnapshot, AggregateInitializationError> {
        if cancellation.is_cancelled() {
            return Err(AggregateInitializationError::Cancelled);
        }
        Ok(AggregatePreflightSnapshot {
            aggregate_root: manifest
                .provider_context_root
                .to_string_lossy()
                .into_owned(),
            index_excludes_assets: true,
            members: Vec::new(),
            manifest_revision: manifest.membership_revision,
            manifest_digest: "sha256:test-manifest".to_string(),
            ..Default::default()
        })
    }
}

struct NoopProvider {
    started: Option<Arc<Notify>>,
    release: Option<Arc<Notify>>,
}

#[async_trait]
impl AggregateProviderTurnDriver for NoopProvider {
    async fn run_turn(
        &self,
        request: AggregateProviderTurnRequest<'_>,
    ) -> Result<String, AggregateInitializationError> {
        let AggregateProviderTurnRequest { cancellation, .. } = request;
        if cancellation.is_cancelled() {
            return Err(AggregateInitializationError::Cancelled);
        }
        if let Some(started) = &self.started {
            started.notify_one();
        }
        if let Some(release) = &self.release {
            release.notified().await;
        }
        Ok("turn complete".to_string())
    }
}

struct AggregateInitializationHttpFixture {
    _temp: TempDir,
    root: PathBuf,
    app: axum::Router,
    blocked_started: Option<Arc<Notify>>,
    blocked_release: Option<Arc<Notify>>,
}

impl AggregateInitializationHttpFixture {
    async fn new() -> Self {
        Self::new_with_blocked_provider(false).await
    }

    async fn new_with_blocked_provider(blocked: bool) -> Self {
        let temp = tempfile::tempdir().expect("fixture root");
        let root = temp.path().to_path_buf();
        let paths = ProductAppPaths::new(root.join(".aria"));
        ProjectStore::new(paths.clone())
            .create(CreateProjectInput {
                name: "aggregate initialization http test".to_string(),
                description: None,
            })
            .expect("create project");
        // 创建期 provider 选择门（判定前移）要求被选 provider 能力已 Confirmed ⇒
        // 夹具显式物化「已探测」事实（capability 记录作用域 = 项目级全局）。
        crate::capability_fixture::seed_confirmed_capabilities(&paths, "project_0001");
        let aggregate_root = root.join("aggregate-root");
        std::fs::create_dir_all(&aggregate_root).expect("aggregate root");
        LogicalCodebaseStore::new(paths.clone())
            .save_manifest(
                "project_0001",
                &LogicalCodebaseManifest::new("project_0001", aggregate_root, Vec::new()),
            )
            .expect("save manifest");

        let blocked_started = blocked.then(|| Arc::new(Notify::new()));
        let blocked_release = blocked.then(|| Arc::new(Notify::new()));
        let coordinator = AggregateInitializationCoordinator::new(
            paths,
            AggregateInitializationOperationStore::new(ProductAppPaths::new(root.join(".aria"))),
            Arc::new(NoopSkills),
            Arc::new(NoopPreflight),
            Arc::new(NoopProvider {
                started: blocked_started.clone(),
                release: blocked_release.clone(),
            }),
            Arc::new(|| "2026-08-18T00:00:00Z".to_string()),
        );
        let dependencies = AggregateInitializationDependencies::new(
            Arc::new(coordinator),
            InitializationRunRegistry::default(),
        );
        let state = WebAppState::new(root.clone(), WebRuntime::new_fake(root.clone()))
            .with_aggregate_initialization_dependencies(dependencies);
        // 创建期 provider 选择门（判定前移）的 L1 判据是 provider health 快照的
        // `available` ⇒ 默认（真实探测）会让夹具结论取决于宿主 PATH 上装没装
        // provider CLI ⇒ 显式注入 fake provider health，让「四家 provider 都可用」
        // 成为夹具的显式事实（先例与理由见 `with_provider_health_all_available`）。
        let state = with_provider_health_all_available(state, &root);
        Self {
            _temp: temp,
            root,
            app: build_web_router(state),
            blocked_started,
            blocked_release,
        }
    }

    async fn start(&self, idempotency_key: &str) -> serde_json::Value {
        let (status, body) = super::request(
            &self.app,
            Method::POST,
            "/api/projects/project_0001/logical-codebase/initializations",
            serde_json::json!({"idempotency_key": idempotency_key}),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED, "{body}");
        body
    }

    async fn start_with_blocked_provider(&self, idempotency_key: &str) -> serde_json::Value {
        let accepted = self.start(idempotency_key).await;
        self.blocked_started
            .as_ref()
            .expect("blocked provider")
            .notified()
            .await;
        accepted
    }

    async fn cancel(&self, operation_id: &str) -> serde_json::Value {
        let (status, body) = super::request(
            &self.app,
            Method::POST,
            &format!(
                "/api/projects/project_0001/logical-codebase/initializations/{operation_id}/cancel"
            ),
            serde_json::json!({"reason":"user_cancelled"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body
    }

    async fn release_and_poll(&self, operation_id: &str) -> serde_json::Value {
        self.blocked_release
            .as_ref()
            .expect("blocked provider")
            .notify_one();
        self.poll_until_terminal(operation_id).await
    }

    fn seed_running_without_lease(&self, operation_id: &str) {
        let paths = ProductAppPaths::new(self.root.join(".aria"));
        let store = AggregateInitializationOperationStore::new(paths);
        let operation =
            cadence_aria::product::logical_codebase::AggregateInitializationOperation::new(
                operation_id.to_string(),
                "project_0001".to_string(),
                cadence_aria::product::logical_codebase::AggregateInitializationOperationInput {
                    idempotency_key: format!("seed-{operation_id}"),
                    manifest_revision: 0,
                    policy_digest: "sha256:test-policy".to_string(),
                    profile_evidence_digest: None,
                    provider_context_root: self.root.join("aggregate-root"),
                    provider: "claude_code".to_string(),
                },
                "2026-08-18T00:00:00Z".to_string(),
            );
        store.create_idempotent(operation).expect("seed operation");
        store
            .mark_running(
                "project_0001",
                operation_id,
                "2026-08-18T00:00:01Z".to_string(),
            )
            .expect("seed running operation");
    }

    async fn poll_until_terminal(&self, operation_id: &str) -> serde_json::Value {
        for _ in 0..100 {
            let (_, body) = super::request(
                &self.app,
                Method::GET,
                &format!(
                    "/api/projects/project_0001/logical-codebase/initializations/{operation_id}"
                ),
                serde_json::Value::Null,
            )
            .await;
            if matches!(
                body["status"].as_str(),
                Some("completed" | "failed" | "cancelled")
            ) {
                return body;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("operation {operation_id} did not reach a terminal state")
    }
}

#[tokio::test]
async fn aggregate_initialization_post_spawns_worker_until_completed() {
    let fixture = AggregateInitializationHttpFixture::new().await;
    let accepted = fixture.start("key-a").await;
    let operation_id = accepted["operation_id"].as_str().unwrap();
    let completed = fixture.poll_until_terminal(operation_id).await;
    assert_eq!(completed["status"], "completed", "{completed}");
}

#[tokio::test]
async fn aggregate_initialization_cancel_stops_at_provider_step_boundary() {
    let fixture = AggregateInitializationHttpFixture::new_with_blocked_provider(true).await;
    let accepted = fixture.start_with_blocked_provider("key-b").await;
    let operation_id = accepted["operation_id"].as_str().unwrap();
    assert_eq!(fixture.cancel(operation_id).await["status"], "cancelled");
    assert_eq!(
        fixture.release_and_poll(operation_id).await["status"],
        "cancelled"
    );
}

#[tokio::test]
async fn aggregate_initialization_get_recovers_running_operation_without_lease() {
    let fixture = AggregateInitializationHttpFixture::new().await;
    fixture.seed_running_without_lease("operation_interrupted");
    // 中断残留 staging：显式恢复动作应清理，GET 不得清理。
    let store = AggregateInitializationOperationStore::new(ProductAppPaths::new(
        fixture.root.join(".aria"),
    ));
    let staging_root = store
        .staging_path("project_0001", "operation_interrupted")
        .expect("staging path");
    std::fs::create_dir_all(&staging_root).expect("staging dir");
    std::fs::write(staging_root.join("partial-member-projection.json"), "{}")
        .expect("staging residue");

    let uri = "/api/projects/project_0001/logical-codebase/initializations/operation_interrupted";
    // 两段式 (a)：连续两次 GET（legacy 两段路由）是纯投影——status 保持
    // running，staging 原样保留（C4 Task 6 起 GET 禁止 recovery）。
    for _ in 0..2 {
        let (status, body) =
            super::request(&fixture.app, Method::GET, uri, serde_json::Value::Null).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(
            body["status"], "running",
            "GET must not recover the operation"
        );
    }
    assert!(staging_root.join("partial-member-projection.json").exists());

    // 两段式 (b)：显式 POST bootstrap action continue 才落盘中断事实——
    // projection member_index=failed + failure.reason_code=
    // aggregate_initialization_interrupted + staging 已清除。
    let (status, body) = super::request(
        &fixture.app,
        Method::POST,
        "/api/projects/project_0001/logical-codebase/bootstrap/actions",
        serde_json::json!({
            "command_id": "cmd-get-interrupted-continue-1",
            "step": "member_index",
            "action": "continue",
            "expected_revision": null,
            "expected_object_id": "operation_interrupted",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "explicit continue action: {body}");
    let member_index = body["projection"]["steps"]
        .as_array()
        .expect("projection steps")
        .iter()
        .find(|step| step["step"] == "member_index")
        .expect("member_index step")
        .clone();
    assert_eq!(member_index["status"], "failed");
    assert_eq!(
        member_index["failure"]["reason_code"],
        "aggregate_initialization_interrupted"
    );
    assert!(!staging_root.exists(), "explicit recovery clears staging");
}

#[tokio::test]
async fn aggregate_initialization_terminal_replay_returns_conflict() {
    let fixture = AggregateInitializationHttpFixture::new().await;
    let first = fixture.start("key-terminal-replay").await;
    let operation_id = first["operation_id"].as_str().unwrap();
    fixture.poll_until_terminal(operation_id).await;

    let replay = super::request(
        &fixture.app,
        Method::POST,
        "/api/projects/project_0001/logical-codebase/initializations",
        serde_json::json!({"idempotency_key": "key-terminal-replay"}),
    )
    .await;
    super::assert_error(
        replay,
        StatusCode::CONFLICT,
        "aggregate_initialization_conflict",
    );
}

#[tokio::test]
async fn aggregate_initialization_retry_uses_a_new_deterministic_operation_id() {
    let fixture = AggregateInitializationHttpFixture::new().await;
    let first = fixture.start("key-c").await;
    let second = fixture.start("key-d").await;
    assert_ne!(first["operation_id"], second["operation_id"]);
}
