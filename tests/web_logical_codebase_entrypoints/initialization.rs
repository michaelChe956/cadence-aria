use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::http::{Method, StatusCode};
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::logical_codebase::AggregateInitializationStepKind;
use cadence_aria::product::logical_codebase::aggregate_initialization_coordinator::{
    AggregateInitializationCoordinator, AggregateInitializationError, AggregatePreflightService,
    AggregatePreflightSnapshot, AggregateProviderTurnDriver, AggregateSkillsPreparation,
    MachineSkillsPreparation,
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
        _lc_id: Option<&str>,
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
    let (status, body) = super::request(
        &fixture.app,
        Method::GET,
        "/api/projects/project_0001/logical-codebase/initializations/operation_interrupted",
        serde_json::Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "failed");
    assert_eq!(
        body["error"]["code"],
        "aggregate_initialization_interrupted"
    );
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
