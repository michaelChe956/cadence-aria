//! REQ-ENV-01~06 场景验收 + 全入口 gateway audit 断言（it_web 侧）。
//!
//! 分层策略（controller 裁决）：
//! - ENV-01（启动经 envelope 校验；缺失 fail-closed）：it_web 聚合初始化 provider
//!   turn 经 `LogicalCodebaseGatewayFactory` 组装 gateway 启动（Stream +3 审计）；
//!   coding 路径政策缺失 fail-closed 经 engine 级断言（`provider_gateway_policy_missing`）。
//!   底层 envelope 校验 / `PolicyMissing` 的 lib 断言见
//!   `provider_gateway_tests::bootstrap_policy_is_persisted_before_gateway_can_validate_a_launch`。
//! - ENV-02（无政策启动被拒；Fake 经 registry 分层）：it_web 增加 coding 逻辑
//!   attempt 无 gateway 拒绝断言（`logical_provider_gateway_required`，Fake 不豁免）。
//! - ENV-03/04/06：lib 已有（T2 resolver+复验 / gateway resume / T11 config digest +
//!   managed-settings 标注），it_web 不重复，报告给出映射。
//! - ENV-05（Codex 路由级阻断）：it_web 增加 coding 逻辑 attempt + Codex coder 的
//!   路由级阻断断言（`codex_danger_full_access_unsupported`）。
//!
//! 全入口 audit：it_web 覆盖「聚合初始化（Stream +3）」与「coding 内部审查
//! （`CodingProviderStreamRun` 经 gateway，Stream +1）」。coding 入口与 lib 侧
//! seed 工具同构（`provider_gateway_validated_input.rs`），在 it_web crate 内以
//! 公开 API 构造 logical attempt fixture 后驱动 engine；完整 WS + 生产
//! `ProductionPolicyTargetResolver` 三层身份 fixture 成本超预算，规划 author/
//! 规划 review 的 it_web 逻辑会话 fixture 同样超预算，均已记录为报告缺口（lib
//! 层 `coverage_tests::logical_provider_entrypoints_use_gateway_for_sync_and_streaming_stacks`
//! 已覆盖 split（Sync）/planning/coding/review 四类入口）。

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use cadence_aria::cross_cutting::provider_adapter::{ProviderAdapter, ProviderAdapterError};
use cadence_aria::cross_cutting::provider_availability_gate::{
    ProviderAvailabilityGate, ProviderHealthSource,
};
use cadence_aria::cross_cutting::provider_health::{ProviderHealthEntry, ProviderHealthSnapshot};
use cadence_aria::cross_cutting::provider_registry::ProviderRegistry;
use cadence_aria::cross_cutting::streaming_provider::{
    FakeStreamingProvider, ProviderCompletion, ProviderEvent, ProviderSession,
    StreamingProviderAdapter, StreamingProviderInput,
};
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::coding_attempt_store::{CodingAttemptStore, CreateCodingAttemptInput};
use cadence_aria::product::coding_models::{
    AttemptTargetSnapshot, CodingExecutionAttempt, PushStatus, RemoteKind, ReviewRequest,
    ReviewRequestKind, ReviewRequestOwnerKind, ReviewVerdict,
};
use cadence_aria::product::coding_workspace_engine::{
    CodingExecutionContext, CodingWorkspaceEngine,
};
use cadence_aria::product::git_workspace_service::GitWorkspaceService;
use cadence_aria::product::logical_codebase::aggregate_initialization_coordinator::GatewayBackedAggregateProviderTurnDriver;
use cadence_aria::product::logical_codebase::provider_gateway::ResumeEvidenceState;
use cadence_aria::product::logical_codebase::{
    AggregateInitializationCoordinator, AggregateInitializationError,
    AggregateInitializationOperationStore, AggregateInitializationStepKind,
    AggregatePolicyArtifactStore, AggregatePreflightService, AggregatePreflightSnapshot,
    AggregateProviderTurnDriver, AggregateSkillsPreparation, CheckoutAvailability, CheckoutKind,
    GatewayRunAudit, LogicalCodebaseManifest, LogicalCodebaseProviderGateway, LogicalCodebaseStore,
    LogicalRepositoryId, MachineSkillsPreparation, PolicyTarget, PolicyTargetResolver,
    ProviderCapability, ProviderCapabilitySource, ProviderDialect, ProviderGatewayError,
    ProviderRef, ProviderRefType, RepositoryCheckoutId, RepositoryCheckoutRecord,
    SessionLaunchRequest,
};
use cadence_aria::product::models::{ProviderName, WorkspaceRolePermissionModes};
use cadence_aria::protocol::contracts::{AdapterInput, AdapterOutput, TimeoutStatus};
use cadence_aria::web::app::build_web_router;
use cadence_aria::web::gateway_factory::LogicalCodebaseGatewayFactory;
use cadence_aria::web::handlers::AggregateInitializationDependencies;
use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::state::{InitializationRunRegistry, WebAppState};
use cadence_aria::web::workspace_ws_types::ProviderConfigSnapshot;
use chrono::Utc;
use serde_json::json;
use tempfile::tempdir;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

/// ENV-01（聚合初始化 provider turn 经 envelope 校验）+ 全入口聚合初始化 audit：
/// 三个 provider turn 都经 `LogicalCodebaseGatewayFactory` 现组装 gateway 启动，
/// 在共享 `GatewayRunAudit` 留下 3 条 Stream 记录，且全部携带 policy digest。
#[tokio::test]
async fn aggregate_initialization_provider_turns_validate_envelope_and_audit_three_streams() {
    let root = tempdir().expect("root");
    let root_path = root.path().to_path_buf();
    let paths = ProductAppPaths::new(root_path.join(".aria"));

    // 聚合根必须真实存在：生产 `ProductionPolicyTargetResolver` 会 canonicalize。
    let aggregate_root = root_path.join("aggregate-root");
    std::fs::create_dir_all(&aggregate_root).expect("aggregate root");
    let manifest = LogicalCodebaseManifest::new("project_0001", aggregate_root.clone(), Vec::new());
    LogicalCodebaseStore::new(paths.clone())
        .save_manifest("project_0001", &manifest)
        .expect("save manifest");

    let factory = fake_registry_gateway_factory(paths);
    let dependencies = aggregate_dependencies(root_path.clone(), Some(factory.clone()));
    let state = WebAppState::new(root_path.clone(), WebRuntime::new_fake(root_path.clone()))
        .with_aggregate_initialization_dependencies(dependencies.clone());
    // 泄漏 tempdir：manifest 在测试期间保持落盘。
    std::mem::forget(root);
    let app = build_web_router(state);

    let created = post_json(
        &app,
        "/api/projects/project_0001/logical-codebase/initializations",
        json!({"idempotency_key": "init-env-01"}),
    )
    .await;
    assert_eq!(created.status(), StatusCode::ACCEPTED);
    let body = axum::body::to_bytes(created.into_body(), 1024 * 1024)
        .await
        .expect("create body");
    let value: serde_json::Value = serde_json::from_slice(&body).expect("create json");
    assert_eq!(value["steps"].as_array().expect("steps").len(), 5);
    let operation_id = value["operation_id"]
        .as_str()
        .expect("operation_id")
        .to_string();

    dependencies
        .coordinator()
        .execute("project_0001", &operation_id, CancellationToken::new())
        .await
        .expect("aggregate initialization executes through gateway factory");

    // 三个 provider turn 经 factory 组装的 gateway 启动（machine_skills /
    // aggregate_preflight 是确定性 Cadence 代码，不产生启动）。
    assert_eq!(factory.audit().stream_launches(), 3);
    assert_eq!(factory.audit().sync_launches(), 0);
    assert!(
        factory.audit().all_have_policy_digest(),
        "aggregate provider turns must leave policy digest in audit"
    );
}

/// ENV-01（coding 路径政策缺失 fail-closed）：逻辑 attempt + 注入空政策 store 的
/// gateway → `validated_streaming_input_for_role` 在 `gateway.validate` 处
/// fail-closed 为 `provider_gateway_policy_missing`，不触达真实 provider。
#[tokio::test]
async fn coding_logical_attempt_with_missing_policy_fails_closed() {
    let (root, store, logical_attempt) = logical_coding_attempt_plain();
    let _root = root;

    // 空政策 store：不 ensure_bootstrap，validate 返回 PolicyMissing。
    let audit = Arc::new(GatewayRunAudit::new());
    let gateway = build_gateway_with_registry(
        &store.paths(),
        &logical_attempt.project_id,
        Arc::new(ProviderRegistry::new()),
        audit.clone(),
        false,
    );
    let never_start = NeverStartAdapter::new();
    let engine = engine_with_gateway(&store, Some(gateway));

    let error = engine
        .execute_coding(
            &logical_attempt,
            &never_start,
            &CodingExecutionContext::default(),
        )
        .await
        .expect_err("missing policy must fail closed");

    assert!(
        error
            .to_string()
            .contains("provider_gateway_policy_missing"),
        "expected provider_gateway_policy_missing, got: {error}"
    );
    assert_eq!(never_start.start_count(), 0);
    assert_eq!(audit.stream_launches(), 0);
    assert_eq!(audit.sync_launches(), 0);
}

/// ENV-02（无政策启动被拒；Fake 经 registry 分层）：逻辑 attempt + 未注入 gateway
/// → `validated_input` 为 None，provider run fail-closed 为
/// `logical_provider_gateway_required`，即使 coder 是 Fake 也不裸 `start`。
#[tokio::test]
async fn coding_logical_attempt_without_gateway_is_rejected_even_for_fake_provider() {
    let (root, store, mut logical_attempt) = logical_coding_attempt_plain();
    let _root = root;
    set_coder_provider(&store, &logical_attempt, ProviderName::Fake);
    logical_attempt = store
        .get_attempt(
            &logical_attempt.project_id,
            &logical_attempt.issue_id,
            &logical_attempt.id,
        )
        .expect("reload attempt");

    let never_start = NeverStartAdapter::new();
    let engine = engine_with_gateway(&store, None);

    let error = engine
        .execute_coding(
            &logical_attempt,
            &never_start,
            &CodingExecutionContext::default(),
        )
        .await
        .expect_err("logical target without gateway must be rejected");

    assert!(
        error
            .to_string()
            .contains("logical_provider_gateway_required"),
        "expected logical_provider_gateway_required, got: {error}"
    );
    assert_eq!(
        never_start.start_count(),
        0,
        "Fake provider must not be started directly for logical targets"
    );
}

/// ENV-05（Codex 路由级阻断）：逻辑 attempt + Codex coder + 注入 gateway →
/// `gateway.validate` 路由级硬门 fail-closed 为 `codex_danger_full_access_unsupported`。
#[tokio::test]
async fn coding_logical_attempt_with_codex_coder_is_blocked_at_gateway_route() {
    let (root, store, mut logical_attempt) = logical_coding_attempt_plain();
    let _root = root;
    set_coder_provider(&store, &logical_attempt, ProviderName::Codex);
    logical_attempt = store
        .get_attempt(
            &logical_attempt.project_id,
            &logical_attempt.issue_id,
            &logical_attempt.id,
        )
        .expect("reload attempt");

    let audit = Arc::new(GatewayRunAudit::new());
    let gateway = build_gateway_with_registry(
        &store.paths(),
        &logical_attempt.project_id,
        Arc::new(ProviderRegistry::new()),
        audit.clone(),
        true,
    );
    let never_start = NeverStartAdapter::new();
    let engine = engine_with_gateway(&store, Some(gateway));

    let error = engine
        .execute_coding(
            &logical_attempt,
            &never_start,
            &CodingExecutionContext::default(),
        )
        .await
        .expect_err("Codex danger-full-access must be blocked at gateway route");

    assert!(
        error
            .to_string()
            .contains("codex_danger_full_access_unsupported"),
        "expected codex_danger_full_access_unsupported, got: {error}"
    );
    assert_eq!(never_start.start_count(), 0);
    assert_eq!(audit.stream_launches(), 0);
}

/// 全入口 coding audit：逻辑 attempt 的 internal review（`CodingProviderStreamRun`）
/// 经 gateway `start_streaming` 启动，在共享 `GatewayRunAudit` 留下 Stream +1，
/// 且 policy digest 非空。与 lib 侧
/// `provider_gateway_validated_input::logical_internal_review_launches_through_gateway`
/// 同构，但经 it_web crate 的公开 API 驱动。
#[tokio::test]
async fn coding_internal_review_launches_through_gateway_and_audits_stream_launch() {
    let (root, store, logical_attempt) = logical_coding_attempt_with_git_and_checkout();
    let _root = root;

    let commit_sha = git_stdout(
        logical_attempt.worktree_path.as_ref().expect("worktree"),
        &["rev-parse", "HEAD"],
    );
    store
        .save_review_request(
            &logical_attempt,
            &ReviewRequest {
                id: "review_request_0001".to_string(),
                attempt_id: logical_attempt.id.clone(),
                kind: ReviewRequestKind::GitBranchOnly,
                remote_kind: RemoteKind::GenericGit,
                remote: "origin".to_string(),
                base_branch: logical_attempt.base_branch.clone(),
                branch_name: logical_attempt.branch_name.clone(),
                commit_sha,
                push_status: PushStatus::Pushed,
                external_url: None,
                manual_instructions: Vec::new(),
                created_at: "2026-08-13T00:00:00Z".to_string(),
                updated_at: "2026-08-13T00:00:00Z".to_string(),
                push_error: None,
                owner_kind: ReviewRequestOwnerKind::Attempt,
                pointer_publication_id: None,
            },
        )
        .expect("review request");

    let audit = Arc::new(GatewayRunAudit::new());
    let adapter = Arc::new(ReviewStreamingAdapter::new(
        json!({
            "verdict": "approve",
            "summary": "logical review ok",
            "findings": []
        })
        .to_string(),
    ));
    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::ClaudeCode, adapter.clone());
    let gateway = build_gateway_with_registry(
        &store.paths(),
        &logical_attempt.project_id,
        Arc::new(registry),
        audit.clone(),
        true,
    );

    let engine = engine_with_gateway(&store, Some(gateway));
    let review = engine
        .execute_internal_pr_review(&logical_attempt, adapter.as_ref())
        .await
        .expect("logical internal review must launch through gateway");

    assert_eq!(review.verdict, ReviewVerdict::Approve);
    assert_eq!(audit.stream_launches(), 1);
    assert!(audit.all_have_policy_digest());
}

// ---------------------------------------------------------------------------
// fixtures / helpers
// ---------------------------------------------------------------------------

/// 测试用 gateway factory：fake streaming provider + 恒可用 gate + stub sync adapter。
fn fake_registry_gateway_factory(paths: ProductAppPaths) -> Arc<LogicalCodebaseGatewayFactory> {
    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::ClaudeCode, Arc::new(FakeStreamingProvider));
    Arc::new(LogicalCodebaseGatewayFactory::new(
        paths,
        Arc::new(registry),
        Arc::new(StubSyncAdapter),
        always_available_gate(),
    ))
}

/// 与 `src/web/handlers/aggregate_initialization.rs` 单测同构的依赖组装：用注入的
/// factory 包装出 gateway-backed provider turn driver，再拼出 coordinator。
fn aggregate_dependencies(
    root: std::path::PathBuf,
    factory: Option<Arc<LogicalCodebaseGatewayFactory>>,
) -> AggregateInitializationDependencies {
    let paths = ProductAppPaths::new(root.join(".aria"));
    let operations = AggregateInitializationOperationStore::new(paths.clone());
    let clock: Arc<dyn Fn() -> String + Send + Sync> = Arc::new(|| Utc::now().to_rfc3339());
    let provider: Arc<dyn AggregateProviderTurnDriver> =
        Arc::new(FactoryBackedProviderTurnDriver::new(factory));
    let coordinator = AggregateInitializationCoordinator::new(
        paths,
        operations,
        Arc::new(NoopSkills),
        Arc::new(NoopPreflight),
        provider,
        clock,
    );
    AggregateInitializationDependencies::new(
        Arc::new(coordinator),
        InitializationRunRegistry::default(),
    )
}

/// 把 factory 组装出的 gateway 委托给 gateway-backed 驱动，复刻 web 层
/// `GatewayFactoryProviderTurnDriver` 的行为（it_web 无法访问该私有类型）。
struct FactoryBackedProviderTurnDriver {
    factory: Option<Arc<LogicalCodebaseGatewayFactory>>,
}

impl FactoryBackedProviderTurnDriver {
    fn new(factory: Option<Arc<LogicalCodebaseGatewayFactory>>) -> Self {
        Self { factory }
    }
}

#[async_trait::async_trait]
impl AggregateProviderTurnDriver for FactoryBackedProviderTurnDriver {
    async fn run_turn(
        &self,
        project_id: &str,
        operation_id: &str,
        step: AggregateInitializationStepKind,
        preflight: &AggregatePreflightSnapshot,
        cancellation: CancellationToken,
    ) -> Result<String, AggregateInitializationError> {
        let factory =
            self.factory
                .as_ref()
                .ok_or_else(|| AggregateInitializationError::ProviderTurn {
                    step,
                    reason: "logical codebase gateway factory is not configured".to_string(),
                    retryable: false,
                })?;
        let gateway = factory.build(project_id).map_err(|error| {
            AggregateInitializationError::ProviderTurn {
                step,
                reason: format!("logical codebase gateway factory build failed: {error}"),
                retryable: true,
            }
        })?;
        GatewayBackedAggregateProviderTurnDriver::claude_code(
            Arc::new(gateway),
            "cap_managed_snapshot",
        )
        .run_turn(project_id, operation_id, step, preflight, cancellation)
        .await
    }
}

struct NoopSkills;

#[async_trait::async_trait]
impl AggregateSkillsPreparation for NoopSkills {
    async fn prepare_skills(
        &self,
        _project_id: &str,
        _operation_id: &str,
        _cancellation: CancellationToken,
    ) -> Result<MachineSkillsPreparation, AggregateInitializationError> {
        Ok(MachineSkillsPreparation {
            source_digest: "sha256:noop".to_string(),
            link_digest: "sha256:noop".to_string(),
            skills_root: PathBuf::from("/skills"),
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
        _cancellation: &CancellationToken,
    ) -> Result<AggregatePreflightSnapshot, AggregateInitializationError> {
        Ok(AggregatePreflightSnapshot {
            aggregate_root: manifest
                .provider_context_root
                .to_string_lossy()
                .into_owned(),
            index_excludes_assets: true,
            members: Vec::new(),
            manifest_revision: manifest.membership_revision,
            manifest_digest: "sha256:manifest".to_string(),
        })
    }
}

/// 恒定健康源：ClaudeCode 与 Codex 均可用。
struct AlwaysHealthy(Arc<ProviderHealthSnapshot>);

impl ProviderHealthSource for AlwaysHealthy {
    fn snapshot(&self) -> Arc<ProviderHealthSnapshot> {
        self.0.clone()
    }

    fn degraded(&self) -> bool {
        false
    }
}

fn always_available_gate() -> Arc<ProviderAvailabilityGate> {
    let checked_at = Utc::now();
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

struct StubSyncAdapter;

impl ProviderAdapter for StubSyncAdapter {
    fn run(&self, _input: &AdapterInput) -> Result<AdapterOutput, ProviderAdapterError> {
        Ok(AdapterOutput {
            exit_code: Some(0),
            stdout: String::new(),
            stderr: String::new(),
            structured_output: None,
            files_modified: Vec::new(),
            duration_ms: 0,
            timeout_status: TimeoutStatus::NotTimedOut,
        })
    }
}

/// pass-through target resolver：直接返回请求中的 target。it_web 侧不重复
/// `ProductionPolicyTargetResolver` 的三层身份复验（T2/T6 lib 已覆盖）。
struct PassThroughTargetResolver;

impl PolicyTargetResolver for PassThroughTargetResolver {
    fn resolve_and_revalidate(
        &self,
        request: &SessionLaunchRequest,
    ) -> Result<PolicyTarget, ProviderGatewayError> {
        Ok(request.target.clone())
    }
}

/// 静态 capability source：按 provider ref 返回对应 dialect，version 固定 1.0.0。
struct StaticCapabilitySource;

impl ProviderCapabilitySource for StaticCapabilitySource {
    fn require_supported(
        &self,
        provider: &ProviderRef,
        _action: cadence_aria::product::logical_codebase::SessionPolicyAction,
    ) -> Result<ProviderCapability, ProviderGatewayError> {
        let adapter_dialect = match provider.provider_type {
            ProviderRefType::ClaudeCode => ProviderDialect::ClaudeCodeCliV1,
            ProviderRefType::Codex => ProviderDialect::CodexCliV1,
        };
        Ok(ProviderCapability {
            provider_type: provider.provider_type,
            version: "1.0.0".to_string(),
            adapter_dialect,
            capability_snapshot_ref: provider.capability_snapshot_ref.clone(),
            resume_evidence: ResumeEvidenceState::Confirmed,
        })
    }
}

/// 组装 gateway。`bootstrap` 为 true 时先 ensure_bootstrap policy（放行路径）；
/// false 时保持空政策 store（ENV-01 coding fail-closed）。
fn build_gateway_with_registry(
    paths: &ProductAppPaths,
    project_id: &str,
    registry: Arc<ProviderRegistry>,
    audit: Arc<GatewayRunAudit>,
    bootstrap: bool,
) -> LogicalCodebaseProviderGateway {
    let policies = AggregatePolicyArtifactStore::new(paths.clone());
    if bootstrap {
        let manifest = LogicalCodebaseManifest::new(project_id, paths.root().to_path_buf(), vec![]);
        policies
            .ensure_bootstrap(&manifest)
            .expect("bootstrap policy");
    }
    LogicalCodebaseProviderGateway::with_audit(
        policies,
        Arc::new(StaticCapabilitySource),
        Arc::new(PassThroughTargetResolver),
        registry,
        Arc::new(StubSyncAdapter),
        always_available_gate(),
        audit,
        paths.root().to_path_buf(),
    )
}

/// `start` 若被调用则计数并立即失败——用于断言 fail-closed 路径不触达 provider。
struct NeverStartAdapter {
    starts: AtomicUsize,
}

impl NeverStartAdapter {
    fn new() -> Self {
        Self {
            starts: AtomicUsize::new(0),
        }
    }

    fn start_count(&self) -> usize {
        self.starts.load(Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for NeverStartAdapter {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        Err(ProviderAdapterError::execution_failed(
            None,
            String::new(),
            "unexpected provider start for fail-closed path",
            0,
        ))
    }
}

/// internal review 用的完成型 streaming adapter：立即输出固定 review JSON。
struct ReviewStreamingAdapter {
    output: String,
}

impl ReviewStreamingAdapter {
    fn new(output: impl Into<String>) -> Self {
        Self {
            output: output.into(),
        }
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for ReviewStreamingAdapter {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let structured_output_contract = input.structured_output_contract.clone();
        let (event_tx, event_rx) = mpsc::channel(4);
        let (command_tx, _command_rx) = mpsc::channel(4);
        let output = self.output.clone();
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::Completed(ProviderCompletion::from_output(
                    output,
                    structured_output_contract.as_ref(),
                    None,
                )))
                .await;
        });
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }
}

/// 构造一个 running + 逻辑 target 的 coding attempt（worktree 目录存在但非 git）。
/// 供 ENV-01/02/05 的 fail-closed 断言使用：这些断言在 provider 启动前即失败，
/// 不触达 `capture_cross_target_baseline` 的 git/manifest 需求。
fn logical_coding_attempt_plain() -> (
    tempfile::TempDir,
    CodingAttemptStore,
    CodingExecutionAttempt,
) {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    std::fs::create_dir_all(&worktree).expect("worktree dir");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = create_running_attempt(&store, worktree);
    let logical = with_target_snapshot(&store, &attempt);
    (root, store, logical)
}

/// 构造 running + 逻辑 target 的 coding attempt，worktree 为真实 git repo，并播种
/// logical manifest + 主 checkout，供 internal review（Stream +1 audit）使用。
fn logical_coding_attempt_with_git_and_checkout() -> (
    tempfile::TempDir,
    CodingAttemptStore,
    CodingExecutionAttempt,
) {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    std::fs::create_dir_all(&worktree).expect("worktree dir");
    init_git_repo(&worktree);
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = create_running_attempt(&store, worktree);
    let logical = with_target_snapshot(&store, &attempt);
    seed_logical_codebase_checkout(&store, &logical);
    (root, store, logical)
}

fn create_running_attempt(store: &CodingAttemptStore, worktree: PathBuf) -> CodingExecutionAttempt {
    let created = store
        .create_attempt(CreateCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: "work_item_0001".to_string(),
            base_branch: "HEAD".to_string(),
            branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
            worktree_path: Some(worktree),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::ClaudeCode,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
                permission_modes: WorkspaceRolePermissionModes::default(),
            },
            target_snapshot: None,
            max_auto_rework: 2,
        })
        .expect("create attempt");
    crate::seed_coding_attempt_running(store, &created.project_id, &created.issue_id, &created.id)
}

/// 覆写 attempt 的 `target_snapshot` 为逻辑代码库 target 并落盘。
fn with_target_snapshot(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
) -> CodingExecutionAttempt {
    let mut logical = attempt.clone();
    logical.target_snapshot = Some(AttemptTargetSnapshot {
        logical_repository_id: LogicalRepositoryId(uuid::Uuid::new_v4()),
        checkout_id: RepositoryCheckoutId(uuid::Uuid::new_v4()),
        physical_repository_id: "repository_0001".to_string(),
        canonical_path: logical.worktree_path.clone().expect("worktree"),
        git_dir_identity: "git-dir-identity".to_string(),
        revision: None,
        policy_digest: String::new(),
        membership_revision: 1,
        captured_at: "2026-08-13T00:00:00Z".to_string(),
        capture_source: "it_web".to_string(),
    });
    crate::seed_coding_attempt_record(store, &logical);
    logical
}

/// 播种 logical manifest + 主 checkout（供 `capture_cross_target_baseline`）。
fn seed_logical_codebase_checkout(store: &CodingAttemptStore, attempt: &CodingExecutionAttempt) {
    let target = attempt.target_snapshot.as_ref().expect("target snapshot");
    let logical_store = LogicalCodebaseStore::new(store.paths());
    let manifest = LogicalCodebaseManifest::new(
        &attempt.project_id,
        store.paths().root().to_path_buf(),
        vec![target.logical_repository_id],
    );
    logical_store
        .save_manifest(&attempt.project_id, &manifest)
        .expect("save manifest");
    logical_store
        .save_checkout(
            &attempt.project_id,
            &RepositoryCheckoutRecord {
                checkout_id: target.checkout_id,
                logical_repository_id: target.logical_repository_id,
                physical_repository_id: target.physical_repository_id.clone(),
                kind: CheckoutKind::Main,
                canonical_path: target.canonical_path.clone(),
                checkout_path_hash: "checkout-path-hash".to_string(),
                git_dir_identity: target.git_dir_identity.clone(),
                revision: None,
                availability: CheckoutAvailability::Available,
                observed_at: "2026-08-13T00:00:00Z".to_string(),
                created_at: "2026-08-13T00:00:00Z".to_string(),
                updated_at: "2026-08-13T00:00:00Z".to_string(),
            },
        )
        .expect("save checkout");
}

fn set_coder_provider(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    coder: ProviderName,
) {
    let mut config = store
        .get_role_provider_config_snapshot(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("role provider config");
    config.coder = coder;
    store
        .update_role_provider_config_snapshot(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            config,
        )
        .expect("update role provider config");
}

fn engine_with_gateway(
    store: &CodingAttemptStore,
    gateway: Option<LogicalCodebaseProviderGateway>,
) -> CodingWorkspaceEngine {
    let (tx, _rx) = mpsc::channel(32);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    match gateway {
        Some(gateway) => engine.with_logical_provider_gateway(Arc::new(gateway)),
        None => engine,
    }
}

async fn post_json(
    app: &axum::Router,
    uri: &str,
    body: serde_json::Value,
) -> axum::http::Response<Body> {
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .expect("request"),
        )
        .await
        .expect("response")
}

fn init_git_repo(repo: &std::path::Path) {
    run_git(repo, &["init", "--quiet"]);
    run_git(repo, &["config", "user.name", "Aria Test"]);
    run_git(repo, &["config", "user.email", "aria@example.test"]);
    std::fs::write(repo.join("README.md"), "fixture\n").expect("readme");
    run_git(repo, &["add", "README.md"]);
    run_git(repo, &["commit", "--quiet", "-m", "fixture"]);
}

fn git_stdout(cwd: &std::path::Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("git command");
    assert!(output.status.success(), "git {args:?}");
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn run_git(cwd: &std::path::Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .status()
        .expect("git command");
    assert!(status.success(), "git {args:?}");
}
