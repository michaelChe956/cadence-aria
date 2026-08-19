//! Task R4：初始化/索引/成员/指针端点换形 `/logical-codebases/{lc_id}/…` 集成测试。
//!
//! 契约来源：v1.3 §4——
//! - 初始化三端点（create/get/cancel）与索引两端点（active/rebuild）按 lc_id 解析
//!   operation store / index store 子树（D3/D5 语义逐字保留）
//! - guard：lc_id 不存在 → 404 logical_codebase_not_found
//! - 旧路径 `/logical-codebase/{initializations,aggregate-indexes,members,pointer-publications}`
//!   保留为"默认第一个逻辑代码库"兼容别名
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use cadence_aria::cross_cutting::bounded_command_runner::{
    BoundedCommandError, BoundedCommandRequest, BoundedCommandResult, BoundedCommandRunner,
};
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::logical_codebase::AggregateInitializationStepKind;
use cadence_aria::product::logical_codebase::aggregate_index::{
    AggregateIndexOperation, AggregateIndexStore, CodeGraphCli, CodeGraphExcludeGenerator,
};
use cadence_aria::product::logical_codebase::aggregate_initialization_coordinator::{
    AggregateInitializationCoordinator, AggregateInitializationError, AggregatePreflightService,
    AggregatePreflightSnapshot, AggregateProviderTurnDriver, AggregateSkillsPreparation,
    MachineSkillsPreparation,
};
use cadence_aria::product::logical_codebase::aggregate_initialization_store::AggregateInitializationOperationStore;
use cadence_aria::product::logical_codebase::store::{
    LogicalCodebaseManifest, LogicalCodebaseStore,
};
use cadence_aria::web::app::build_web_router;
use cadence_aria::web::events::EventHub;
use cadence_aria::web::handlers::AggregateInitializationDependencies;
use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::state::{InitializationRunRegistry, WebAppState};
use serde_json::{Value, json};
use tempfile::tempdir;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

const PROJECT_ID: &str = "project_0001";

// ---------------------------------------------------------------------------
// Noop aggregate-initialization drivers (D3 五步不依赖真实 claude/codegraph)。
// ---------------------------------------------------------------------------

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
        _project_id: &str,
        _operation_id: &str,
        _step: AggregateInitializationStepKind,
        _preflight: &AggregatePreflightSnapshot,
        cancellation: CancellationToken,
    ) -> Result<String, AggregateInitializationError> {
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

// ---------------------------------------------------------------------------
// Fake CodeGraph CLI: exercises the D5 acceptance assertions deterministically.
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct FakeCodeGraphCli {
    member_names: Vec<String>,
}

#[async_trait]
impl BoundedCommandRunner for FakeCodeGraphCli {
    async fn run(
        &self,
        request: BoundedCommandRequest,
    ) -> Result<BoundedCommandResult, BoundedCommandError> {
        let command = request.argv.first().map(String::as_str).unwrap_or_default();
        let ok = |stdout: String| BoundedCommandResult {
            exit_code: Some(0),
            stdout,
            stderr: String::new(),
            timed_out: false,
            cancelled: false,
            stdout_truncated: false,
            stderr_truncated: false,
            duration_ms: 0,
        };
        match command {
            "--version" => Ok(ok("1.5.0\n".to_string())),
            "init" | "sync" => Ok(ok(String::new())),
            "files" => {
                let files = self
                    .member_names
                    .iter()
                    .map(|name| format!(r#"{{"path":"{name}/lib.rs"}}"#))
                    .collect::<Vec<_>>()
                    .join(",");
                Ok(ok(format!("[{files}]")))
            }
            "query" => {
                let query = request.argv.get(1).map(String::as_str).unwrap_or_default();
                if query == "crossRepoGreeting" {
                    let hits = self
                        .member_names
                        .iter()
                        .map(|name| format!(r#"{{"path":"{name}/lib.rs"}}"#))
                        .collect::<Vec<_>>()
                        .join(",");
                    Ok(ok(format!("[{hits}]")))
                } else {
                    Ok(ok("[]".to_string()))
                }
            }
            _ => Err(BoundedCommandError::Io {
                details: format!("unexpected fake codegraph command: {command}"),
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// HTTP helper。
// ---------------------------------------------------------------------------

async fn request_json(
    app: &axum::Router,
    method: Method,
    uri: &str,
    body: Value,
) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .expect("request"),
        )
        .await
        .expect("response");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, value)
}

fn git_repo_at(path: &std::path::Path) {
    std::fs::create_dir_all(path).expect("create repo dir");
    run_git(path, &["init", "-q"]);
    run_git(path, &["config", "user.email", "test@example.com"]);
    run_git(path, &["config", "user.name", "Test User"]);
}

/// 为成员仓配置一个本地 bare `origin` 远端，供 pointer-publication 流水 push。
fn add_origin_remote(repo: &std::path::Path, remote: &std::path::Path) {
    std::fs::create_dir_all(remote).expect("create remote dir");
    run_git(remote, &["init", "--bare", "-q"]);
    run_git(
        repo,
        &[
            "remote",
            "add",
            "origin",
            remote.to_str().expect("remote path"),
        ],
    );
    run_git(repo, &["push", "-q", "-u", "origin", "HEAD"]);
}

fn run_git(path: &std::path::Path, arguments: &[&str]) {
    let status = std::process::Command::new("git")
        .args(arguments)
        .current_dir(path)
        .status()
        .expect("git");
    assert!(status.success(), "git {arguments:?} failed");
}

fn commit(path: &std::path::Path, message: &str) {
    std::fs::write(path.join("lib.rs"), message).expect("write");
    run_git(path, &["add", "."]);
    run_git(
        path,
        &[
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-q",
            "-m",
            message,
        ],
    );
}

// ---------------------------------------------------------------------------
// Fixture：真实 project + 一个经新登记端点注册的逻辑代码库（manifest 落 per-LC 子树）。
// ---------------------------------------------------------------------------

struct LcOperationsFixture {
    _root: tempfile::TempDir,
    _aggregate_root: tempfile::TempDir,
    app: axum::Router,
    lc_id: String,
}

impl LcOperationsFixture {
    async fn new(blocked_provider: bool, fake_index_member_names: Option<&[&str]>) -> Self {
        let root = tempdir().expect("root");
        let aggregate_root = tempdir().expect("aggregate root");
        let member_a = aggregate_root.path().join("alpha");
        let member_b = aggregate_root.path().join("beta");
        git_repo_at(&member_a);
        git_repo_at(&member_b);
        commit(
            &member_a,
            "pub fn cross_repo_greeting() -> &'static str { \"alpha\" }",
        );
        commit(
            &member_b,
            "pub fn cross_repo_greeting() -> &'static str { \"beta\" }",
        );
        // 为 pointer-publication 新路径用例准备可 push 的 bare origin。
        let remote_a = root.path().join("remotes/alpha.git");
        let remote_b = root.path().join("remotes/beta.git");
        add_origin_remote(&member_a, &remote_a);
        add_origin_remote(&member_b, &remote_b);

        let paths = ProductAppPaths::new(root.path().join(".aria"));
        let blocked_started = blocked_provider.then(|| Arc::new(Notify::new()));
        let blocked_release = blocked_provider.then(|| Arc::new(Notify::new()));
        let coordinator = AggregateInitializationCoordinator::new(
            paths.clone(),
            AggregateInitializationOperationStore::new(paths.clone()),
            Arc::new(NoopSkills),
            Arc::new(NoopPreflight),
            Arc::new(NoopProvider {
                started: blocked_started.clone(),
                release: blocked_release.clone(),
            }),
            Arc::new(|| "2026-08-19T00:00:00Z".to_string()),
        );
        let dependencies = AggregateInitializationDependencies::new(
            Arc::new(coordinator),
            InitializationRunRegistry::default(),
        );

        let state = WebAppState::with_events(
            root.path().to_path_buf(),
            WebRuntime::new_fake(root.path().to_path_buf()),
            EventHub::new(),
        )
        .with_aggregate_initialization_dependencies(dependencies.clone());

        let state = if let Some(member_names) = fake_index_member_names {
            let index = Arc::new(AggregateIndexOperation::with_snapshot_dependencies(
                LogicalCodebaseStore::new(paths.clone()),
                AggregateIndexStore::new(paths.clone()),
                CodeGraphCli::new(
                    Arc::new(FakeCodeGraphCli {
                        member_names: member_names.iter().map(|s| s.to_string()).collect(),
                    }),
                    "fake-codegraph".to_string(),
                ),
                CodeGraphExcludeGenerator,
                cadence_aria::product::logical_codebase::aggregate_index::
                    AggregateIndexSnapshotCollector::for_paths(paths.clone()),
            ));
            state.with_aggregate_index_operation(index)
        } else {
            state
        };

        let app = build_web_router(state);

        let (status, _) = request_json(
            &app,
            Method::POST,
            "/api/projects",
            json!({"name":"R4 operations","description":null}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        // 创建逻辑代码库。
        let (status, logical) = request_json(
            &app,
            Method::POST,
            "/api/projects/project_0001/logical-codebases",
            json!({"name":"Platform","aggregate_root":aggregate_root.path()}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{logical}");
        let lc_id = logical["id"].as_str().expect("logical id").to_string();

        // 经新登记端点注册两个成员（manifest/members/checkouts 落 per-LC 子树）。
        let (status, preflight) = request_json(
            &app,
            Method::POST,
            &format!("/api/projects/project_0001/logical-codebases/{lc_id}/registrations/preflight"),
            json!({"aggregate_root": aggregate_root.path(), "candidate_paths": [], "auto_discover": true}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{preflight:?}");
        let preflight_id = preflight["preflight_id"]
            .as_str()
            .expect("preflight id")
            .to_string();
        let (status, batch) = request_json(
            &app,
            Method::POST,
            &format!("/api/projects/project_0001/logical-codebases/{lc_id}/registrations"),
            json!({
                "preflight_id": preflight_id,
                "aggregate_root": aggregate_root.path(),
                "confirmed_paths": [
                    aggregate_root.path().join("alpha").display().to_string(),
                    aggregate_root.path().join("beta").display().to_string(),
                ],
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{batch}");
        assert_eq!(batch["status"], "completed");

        let app = app.clone();
        // 把 blocked provider 的释放信号保留在 fixture 外由测试驱动（用 app 无关的
        // Notify 已经在 coordinator 中共享）。此处仅返回构造结果。
        let _ = (&blocked_started, &blocked_release);
        Self {
            _root: root,
            _aggregate_root: aggregate_root,
            app,
            lc_id,
        }
    }

    async fn start_initialization(&self, idempotency_key: &str) -> Value {
        let (status, body) = request_json(
            &self.app,
            Method::POST,
            &format!(
                "/api/projects/{PROJECT_ID}/logical-codebases/{}/initializations",
                self.lc_id
            ),
            json!({"idempotency_key": idempotency_key}),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED, "{body}");
        body
    }

    async fn get_initialization(&self, operation_id: &str) -> (StatusCode, Value) {
        request_json(
            &self.app,
            Method::GET,
            &format!(
                "/api/projects/{PROJECT_ID}/logical-codebases/{}/initializations/{operation_id}",
                self.lc_id
            ),
            json!({}),
        )
        .await
    }

    async fn cancel_initialization(&self, operation_id: &str) -> (StatusCode, Value) {
        request_json(
            &self.app,
            Method::POST,
            &format!(
                "/api/projects/{PROJECT_ID}/logical-codebases/{}/initializations/{operation_id}/cancel",
                self.lc_id
            ),
            json!({"reason":"user_cancelled"}),
        )
        .await
    }

    async fn poll_until_terminal(&self, operation_id: &str) -> Value {
        for _ in 0..400 {
            let (_, body) = self.get_initialization(operation_id).await;
            if matches!(
                body["status"].as_str(),
                Some("completed" | "failed" | "cancelled")
            ) {
                return body;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("operation {operation_id} did not reach a terminal state");
    }
}

// ---------------------------------------------------------------------------
// 新路径：初始化 completed + cancel（D3 语义逐字保留）。
// ---------------------------------------------------------------------------

#[tokio::test]
async fn lc_initialization_new_path_spawns_until_completed() {
    let fixture = LcOperationsFixture::new(false, None).await;
    let accepted = fixture.start_initialization("key-completed").await;
    let operation_id = accepted["operation_id"].as_str().unwrap();
    let completed = fixture.poll_until_terminal(operation_id).await;
    assert_eq!(completed["status"], "completed", "{completed}");
}

#[tokio::test]
async fn lc_initialization_new_path_cancel_stops_at_provider_boundary() {
    let fixture = LcOperationsFixture::new(true, None).await;
    let accepted = fixture.start_initialization("key-cancel").await;
    let operation_id = accepted["operation_id"].as_str().unwrap();
    let (status, cancelled) = fixture.cancel_initialization(operation_id).await;
    assert_eq!(status, StatusCode::OK, "{cancelled}");
    assert_eq!(cancelled["status"], "cancelled");
}

// ---------------------------------------------------------------------------
// 新路径：索引 active / rebuild / 409。
// ---------------------------------------------------------------------------

#[tokio::test]
async fn lc_aggregate_index_new_path_active_rebuild_and_conflict() {
    let fixture = LcOperationsFixture::new(false, Some(&["alpha", "beta"])).await;
    let active_uri = format!(
        "/api/projects/{PROJECT_ID}/logical-codebases/{}/aggregate-indexes/active",
        fixture.lc_id
    );
    let rebuild_uri = format!(
        "/api/projects/{PROJECT_ID}/logical-codebases/{}/aggregate-indexes/rebuild",
        fixture.lc_id
    );

    // 尚无索引 → missing。
    let (status, body) = request_json(&fixture.app, Method::GET, &active_uri, json!({})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["state"], "missing");

    // rebuild → active（D5 三层触发 + acceptance 校验）。
    let (status, body) = request_json(&fixture.app, Method::POST, &rebuild_uri, json!({})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["state"], "active");

    let (status, body) = request_json(&fixture.app, Method::GET, &active_uri, json!({})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["state"], "active");

    // 404：lc_id 不存在。
    let (status, body) = request_json(
        &fixture.app,
        Method::POST,
        &format!(
            "/api/projects/{PROJECT_ID}/logical-codebases/logical_codebase_missing/aggregate-indexes/rebuild"
        ),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["code"], "logical_codebase_not_found");
}

#[tokio::test]
async fn lc_aggregate_index_rebuild_new_path_conflicts_while_lc_registered() {
    // 前置注册该 LC 的 in-memory rebuild lease → 新路径 rebuild 直接 409。
    let root = tempdir().expect("root");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    cadence_aria::product::project_store::ProjectStore::new(paths.clone())
        .create(cadence_aria::product::project_store::CreateProjectInput {
            name: "rebuild conflict".to_string(),
            description: None,
        })
        .expect("create project");
    let lc = LogicalCodebaseStore::new(paths.clone())
        .create(
            PROJECT_ID,
            cadence_aria::product::logical_codebase::LogicalCodebaseCreateInput {
                name: "Default".to_string(),
                aggregate_root: root.path().to_path_buf(),
            },
        )
        .expect("create logical codebase");

    let state = WebAppState::with_events(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
        EventHub::new(),
    );
    let lease = state
        .aggregate_index_rebuilds
        .try_register(&format!("{PROJECT_ID}/{}", lc.id))
        .expect("register rebuild lease");
    let app = build_web_router(state);

    let (status, body) = request_json(
        &app,
        Method::POST,
        &format!(
            "/api/projects/{PROJECT_ID}/logical-codebases/{}/aggregate-indexes/rebuild",
            lc.id
        ),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "aggregate_index_rebuild_in_progress");
    drop(lease);
}

// ---------------------------------------------------------------------------
// 新路径成员 + 旧路径别名回归。
// ---------------------------------------------------------------------------

#[tokio::test]
async fn lc_members_new_path_and_legacy_alias_route_to_default_lc() {
    let fixture = LcOperationsFixture::new(false, None).await;

    // 新路径成员。
    let (status, body) = request_json(
        &fixture.app,
        Method::GET,
        &format!(
            "/api/projects/{PROJECT_ID}/logical-codebases/{}/members",
            fixture.lc_id
        ),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["members"].as_array().expect("members").len(), 2);

    // 旧路径成员别名（默认第一个逻辑代码库）。
    let (status, body) = request_json(
        &fixture.app,
        Method::GET,
        &format!("/api/projects/{PROJECT_ID}/logical-codebase/members"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["members"].as_array().expect("members").len(), 2);

    // 旧路径索引 active 别名。
    let (status, body) = request_json(
        &fixture.app,
        Method::GET,
        &format!("/api/projects/{PROJECT_ID}/logical-codebase/aggregate-indexes/active"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["state"], "missing");

    // 旧路径初始化别名：仍可发起并在完成态结束。
    let (status, body) = request_json(
        &fixture.app,
        Method::POST,
        &format!("/api/projects/{PROJECT_ID}/logical-codebase/initializations"),
        json!({"idempotency_key":"legacy-alias-init"}),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let operation_id = body["operation_id"].as_str().unwrap();
    let completed = fixture.poll_until_terminal(operation_id).await;
    assert_eq!(completed["status"], "completed", "{completed}");

    // 无任何 LC 时旧路径成员 → 404 logical_codebase_not_found（不再 feature_disabled）。
    let empty_root = tempdir().expect("empty root");
    let empty_state = WebAppState::with_events(
        empty_root.path().to_path_buf(),
        WebRuntime::new_fake(empty_root.path().to_path_buf()),
        EventHub::new(),
    );
    let empty_app = build_web_router(empty_state);
    let (status, _) = request_json(
        &empty_app,
        Method::POST,
        "/api/projects",
        json!({"name":"Empty","description":null}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = request_json(
        &empty_app,
        Method::GET,
        &format!("/api/projects/{PROJECT_ID}/logical-codebase/members"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["code"], "logical_codebase_not_found");
}

// ---------------------------------------------------------------------------
// 新路径：pointer-publications list/create（v1.3 §4 per-LC 寻址）。
// ---------------------------------------------------------------------------

#[tokio::test]
async fn lc_pointer_publications_new_path_list_and_create() {
    let fixture = LcOperationsFixture::new(false, None).await;
    let uri = format!(
        "/api/projects/{PROJECT_ID}/logical-codebases/{}/pointer-publications",
        fixture.lc_id
    );

    // 新路径 list：尚无发布批次 → 空数组。
    let (status, body) = request_json(&fixture.app, Method::GET, &uri, json!({})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body.as_array().expect("publications array").len(), 0);

    // 新路径 create：full 批次 → 两个成员仓均推送完成。
    let (status, publication) = request_json(
        &fixture.app,
        Method::POST,
        &uri,
        json!({ "batch_kind": "full" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{publication}");
    assert_eq!(publication["status"], "completed_all", "{publication}");
    let entries = publication["entries"].as_array().expect("entries");
    assert_eq!(entries.len(), 2, "{publication}");
    assert!(
        entries
            .iter()
            .all(|entry| entry["state"] == "review_created"),
        "{publication}"
    );

    // 新路径 list：现在返回一个批次。
    let (status, body) = request_json(&fixture.app, Method::GET, &uri, json!({})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body.as_array().expect("publications array").len(), 1);
}
