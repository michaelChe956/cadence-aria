//! T10a:聚合规划 author 启动点经 gateway 的 handler 级单测。
//!
//! 覆盖:
//! - `resolve_plan_author_launch` + `start_work_item_plan_author(Logical, ...)` 经 gateway 启动并留 audit;
//! - `resolve_plan_author_launch` 无 gateway 时返回 Legacy,`start_work_item_plan_author(Legacy, ...)` 原样走 provider.start(Legacy 零变化);
//! - WorkspaceEngine 两个最小公开访问器。

use super::*;

use std::sync::atomic::{AtomicUsize, Ordering};

use crate::cross_cutting::provider_adapter::{ProviderAdapter, ProviderAdapterError};
use crate::cross_cutting::provider_availability_gate::{
    ProviderAvailabilityGate, ProviderHealthSource,
};
use crate::cross_cutting::provider_health::{ProviderHealthEntry, ProviderHealthSnapshot};
use crate::cross_cutting::streaming_provider::{
    ProviderPermissionMode, ProviderSession, StreamingProviderInput,
};
use crate::product::logical_codebase::{
    LogicalCodebaseManifest, LogicalCodebaseProviderGateway, LogicalCodebaseStore,
};
use crate::protocol::contracts::{AdapterOutput, AdapterRole, ProviderType, TimeoutStatus};
use crate::web::gateway_factory::LogicalCodebaseGatewayFactory;

struct StubSyncAdapter;

impl ProviderAdapter for StubSyncAdapter {
    fn run(
        &self,
        _input: &crate::protocol::contracts::AdapterInput,
    ) -> Result<AdapterOutput, ProviderAdapterError> {
        Ok(AdapterOutput {
            exit_code: Some(0),
            stdout: "ok".to_string(),
            stderr: String::new(),
            structured_output: None,
            files_modified: Vec::new(),
            duration_ms: 0,
            timeout_status: TimeoutStatus::NotTimedOut,
        })
    }
}

struct CountingStreamingAdapter {
    starts: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for CountingStreamingAdapter {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        let (_event_tx, events) = mpsc::channel(1);
        let (commands, _command_rx) = mpsc::channel(1);
        Ok(ProviderSession { events, commands })
    }
}

fn always_available_gate() -> Arc<ProviderAvailabilityGate> {
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
        providers: [ProviderName::ClaudeCode, ProviderName::Codex]
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

struct GatewayFixture {
    _temp: tempfile::TempDir,
    paths: ProductAppPaths,
    gateway: Arc<LogicalCodebaseProviderGateway>,
    aggregate_root: std::path::PathBuf,
}

fn gateway_fixture() -> GatewayFixture {
    let root = tempfile::tempdir().expect("temporary product root");
    let paths = ProductAppPaths::new(root.path().join(".aria"));

    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::ClaudeCode,
        Arc::new(CountingStreamingAdapter {
            starts: Arc::new(AtomicUsize::new(0)),
        }),
    );

    let factory = LogicalCodebaseGatewayFactory::new(
        paths.clone(),
        Arc::new(registry),
        Arc::new(StubSyncAdapter),
        always_available_gate(),
    );

    let aggregate_root = root.path().join("aggregate-root");
    std::fs::create_dir_all(&aggregate_root).expect("create aggregate root");
    let manifest = LogicalCodebaseManifest::new("project_0001", aggregate_root.clone(), Vec::new());
    LogicalCodebaseStore::new(paths.clone())
        .save_manifest("project_0001", &manifest)
        .expect("save manifest");

    let gateway = Arc::new(factory.build("project_0001").expect("build gateway"));

    GatewayFixture {
        _temp: root,
        paths,
        gateway,
        aggregate_root,
    }
}

fn streaming_input(working_dir: std::path::PathBuf) -> StreamingProviderInput {
    StreamingProviderInput {
        provider_type: ProviderType::ClaudeCode,
        role: AdapterRole::WorkItemSplitter,
        prompt: "work item plan outline".to_string(),
        working_dir,
        workspace_session_id: Some("session_0001".to_string()),
        resume_provider_session_id: None,
        permission_mode: ProviderPermissionMode::Auto,
        structured_output_contract: None,
        env_vars: std::collections::BTreeMap::new(),
        timeout_secs: 3600,
    }
}

fn workspace_session(repository_path: std::path::PathBuf) -> WorkspaceSession {
    WorkspaceSession {
        session_id: "session_0001".to_string(),
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        entity_id: "work_item_plan_0001".to_string(),
        workspace_type: WorkspaceType::WorkItemPlan,
        stage: WorkspaceStage::Running,
        messages: vec![],
        artifact: None,
        author_provider: ProviderName::ClaudeCode,
        reviewer_provider: Some(ProviderName::Codex),
        review_rounds: 1,
        permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
        superpowers_enabled: false,
        openspec_enabled: false,
        provider_conversations: vec![],
        repository_path: Some(repository_path),
    }
}

fn workspace_engine(fixture: &GatewayFixture, with_gateway: bool) -> WorkspaceEngine {
    let (event_tx, _event_rx) = mpsc::channel::<crate::product::workspace_engine::EngineEvent>(8);
    let engine = WorkspaceEngine::new(
        Arc::new(CheckpointStore::new(
            fixture.paths.root().join("checkpoints"),
        )),
        event_tx,
        workspace_session(fixture.aggregate_root.clone()),
    );
    if with_gateway {
        engine.with_logical_provider_gateway(fixture.gateway.clone())
    } else {
        engine
    }
}

#[tokio::test]
async fn start_work_item_plan_author_routes_logical_through_gateway_and_records_audit() {
    let fixture = gateway_fixture();
    let audit = fixture.gateway.audit();
    assert_eq!(audit.stream_launches(), 0);

    let provider: Arc<dyn StreamingProviderAdapter> = Arc::new(CountingStreamingAdapter {
        starts: Arc::new(AtomicUsize::new(0)),
    });
    let engine = workspace_engine(&fixture, true);
    let plan_launch =
        resolve_plan_author_launch(&engine, None, None).expect("resolve logical launch");
    let input = streaming_input(fixture.aggregate_root.clone());

    let session =
        start_work_item_plan_author(plan_launch, provider, input, CancellationToken::new()).await;

    assert!(
        session.is_ok(),
        "gateway launch failed: {:?}",
        session.as_ref().err()
    );
    assert_eq!(audit.stream_launches(), 1);
}

#[tokio::test]
async fn start_work_item_plan_author_none_uses_legacy_provider_start_unchanged() {
    let fixture = gateway_fixture();
    let audit = fixture.gateway.audit();
    assert_eq!(audit.stream_launches(), 0);

    let starts = Arc::new(AtomicUsize::new(0));
    let provider: Arc<dyn StreamingProviderAdapter> = Arc::new(CountingStreamingAdapter {
        starts: starts.clone(),
    });
    let engine = workspace_engine(&fixture, false);
    let plan_launch = resolve_plan_author_launch(&engine, None, None).expect("legacy launch");
    let input = streaming_input(fixture.aggregate_root.clone());

    let session =
        start_work_item_plan_author(plan_launch, provider, input, CancellationToken::new()).await;

    assert!(
        session.is_ok(),
        "legacy start failed: {:?}",
        session.as_ref().err()
    );
    assert_eq!(
        starts.load(Ordering::SeqCst),
        1,
        "legacy provider.start must run"
    );
    assert_eq!(
        audit.stream_launches(),
        0,
        "legacy path must not touch gateway audit"
    );
}

#[test]
fn workspace_engine_accessors_expose_logical_launch() {
    let fixture = gateway_fixture();
    let engine = workspace_engine(&fixture, true);

    assert!(engine.logical_provider_gateway().is_some());
    assert_eq!(
        engine.logical_planning_launch(),
        Some(("project_0001".to_string(), fixture.aggregate_root.clone()))
    );
}
