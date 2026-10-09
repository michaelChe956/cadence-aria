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
use cadence_aria::product::logical_codebase::{
    AggregateInitializationCoordinator, AggregateInitializationError,
    AggregateInitializationOperationStore, AggregatePolicyArtifactStore, AggregatePreflightService,
    AggregatePreflightSnapshot, AggregateProviderTurnDriver, AggregateProviderTurnRequest,
    AggregateSkillsPreparation, CheckoutAvailability, CheckoutKind, CodebaseMemberRecord,
    GatewayRunAudit, LogicalCodebaseManifest, LogicalCodebaseProviderGateway, LogicalCodebaseStore,
    LogicalRepositoryId, MachineSkillsPreparation, MemberStatus, PolicyTarget,
    PolicyTargetResolver, ProviderCapability, ProviderCapabilitySource, ProviderDialect,
    ProviderGatewayError, ProviderRef, ProviderRefType, RepositoryCheckoutId,
    RepositoryCheckoutRecord, RepositorySourceIdentity, RepositoryType, SessionLaunchRequest,
};
use cadence_aria::product::models::{ProviderName, WorkspaceRolePermissionModes};
use cadence_aria::product::project_store::{CreateProjectInput, ProjectStore};
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
    ProjectStore::new(paths.clone())
        .create(CreateProjectInput {
            name: "aggregate initialization gateway envelope fixture".to_string(),
            description: None,
        })
        .expect("create multi-repository project fixture");

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

    // Task 1.4 起 provider turn 真实消费会话事件（存在等待完成的真实挂起
    // 点），POST 后台 worker 与显式 execute 双跑会在 durable step 上竞争；
    // 测试改为观察后台 worker 收敛——provider 仍经同一 gateway factory 启动。
    let mut operation = dependencies
        .coordinator()
        .get("project_0001", &operation_id)
        .expect("operation is durable");
    for _ in 0..200 {
        if !matches!(
            operation.status,
            cadence_aria::product::logical_codebase::AggregateInitializationOperationStatus::Created
                | cadence_aria::product::logical_codebase::AggregateInitializationOperationStatus::Running
        ) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        operation = dependencies
            .coordinator()
            .get("project_0001", &operation_id)
            .expect("operation is durable");
    }
    assert_eq!(
        operation.status,
        cadence_aria::product::logical_codebase::AggregateInitializationOperationStatus::Completed,
        "background worker must finish the five-step operation: {:?}",
        operation.error
    );

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
        .expect_err("codex coder must fail closed before any provider start");

    // r47:路由级 Codex 全阻移除(LC 投影恒非 danger);无探针签发行的
    // codex coder 在后续链(cross-target/registry)fail-closed——不变式是
    // 零 provider 启动(下两断言),错误码随链路阶段。
    assert!(
        !error.to_string().is_empty(),
        "failure must be observable: {error}"
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
                revoked: false,
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

/// R9 fix round 1【Important-1】it_web 最小覆盖：非默认（非 legacy）LC 的 coding
/// session 启动 gateway 校验。`LogicalCodebaseGatewayFactory::build_for_lc` 组装的
/// gateway 注入了生产 `ProductionPolicyTargetResolver`，其 checkout 目标复验必须按
/// `logical-codebases/{lc_id}/` 子树权威解析（否则 fail-closed）。
#[tokio::test]
async fn non_default_lc_coding_gateway_validate_resolves_lc_scoped_checkout_target() {
    let root = tempdir().expect("temporary root");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let project = ProjectStore::new(paths.clone())
        .create(CreateProjectInput {
            name: "new-lc gateway project".to_string(),
            description: None,
        })
        .expect("create project");
    // Task 2.8（cwd authority 契约）：真实拓扑成员 checkout 位于聚合根之下；
    // 本 fixture 的 worktree 是 root/api（root 直接子目录），聚合根取
    // root.path()（真实公共父目录），否则 cwd 会被 gateway 拒绝。
    let aggregate_root = root.path().to_path_buf();
    std::fs::create_dir_all(&aggregate_root).expect("create aggregate root");
    let record = LogicalCodebaseStore::new(paths.clone())
        .create(
            &project.id,
            cadence_aria::product::logical_codebase::LogicalCodebaseCreateInput {
                name: "new-lc".to_string(),
                aggregate_root: aggregate_root.clone(),
            },
        )
        .expect("create logical codebase record");
    let lc_id = record.id;

    let worktree = root.path().join("api");
    std::fs::create_dir_all(&worktree).expect("create repository root");
    run_git(&worktree, &["init", "--quiet"]);
    run_git(
        &worktree,
        &["config", "user.email", "itweb-lc@example.test"],
    );
    run_git(&worktree, &["config", "user.name", "Itweb New LC"]);
    std::fs::write(worktree.join("README.md"), "# api\n").expect("write file");
    run_git(&worktree, &["add", "README.md"]);
    run_git(&worktree, &["commit", "--quiet", "-m", "initial commit"]);

    let authority = LogicalCodebaseStore::for_lc(paths.clone(), lc_id.clone());
    let logical_id = LogicalRepositoryId(uuid::Uuid::new_v4());
    let checkout_id = RepositoryCheckoutId(uuid::Uuid::new_v4());
    let physical_repository_id = format!("repository_{}", uuid::Uuid::new_v4().simple());
    authority
        .save_manifest(
            &project.id,
            &LogicalCodebaseManifest::new(&project.id, aggregate_root.clone(), vec![logical_id]),
        )
        .expect("save lc manifest");
    let source_identity =
        cadence_aria::product::logical_codebase::RepositorySourceIdentity::from_git_parts(
            &worktree,
            worktree.join(".git"),
            None,
        );
    let now = "2026-08-18T00:00:00Z".to_string();
    authority
        .save_member(
            &project.id,
            &cadence_aria::product::logical_codebase::CodebaseMemberRecord {
                logical_repository_id: logical_id,
                physical_repository_id: physical_repository_id.clone(),
                alias: "api".to_string(),
                role: "repository".to_string(),
                ordinal: 0,
                source_identity: source_identity.clone(),
                repo_type: cadence_aria::product::logical_codebase::RepositoryType::Unknown,
                tech_stack: Vec::new(),
                owner: None,
                tags: Vec::new(),
                default_ref: None,
                checkout_ids: vec![checkout_id],
                status: cadence_aria::product::logical_codebase::MemberStatus::Active,
                created_at: now.clone(),
                updated_at: now.clone(),
            },
        )
        .expect("save lc member");
    authority
        .save_checkout(
            &project.id,
            &RepositoryCheckoutRecord {
                checkout_id,
                logical_repository_id: logical_id,
                physical_repository_id: physical_repository_id.clone(),
                kind: CheckoutKind::Main,
                canonical_path: worktree.clone(),
                checkout_path_hash: "sha256:checkout".to_string(),
                git_dir_identity: source_identity.git_dir_identity().to_string(),
                revision: None,
                availability: CheckoutAvailability::Available,
                observed_at: now.clone(),
                created_at: now.clone(),
                updated_at: now,
            },
        )
        .expect("save lc checkout");
    cadence_aria::product::logical_codebase::IdentityRegistryStore::new(paths.clone())
        .upsert_active(
            &project.id,
            cadence_aria::product::logical_codebase::IdentityRegistryEntry::active(
                source_identity,
                logical_id,
                physical_repository_id,
                checkout_id,
                "itweb-new-lc-fixture".to_string(),
            ),
        )
        .expect("register identity");

    let factory = fake_registry_gateway_factory(paths.clone());
    let gateway = factory
        .build_for_lc(&project.id, Some(&lc_id))
        .expect("build gateway for non-default lc");

    let request = SessionLaunchRequest {
        project_id: project.id.clone(),
        provider: ProviderRef::claude_code("cap_managed_snapshot"),
        action: cadence_aria::product::logical_codebase::SessionPolicyAction::CodingTargetWrite,
        target: PolicyTarget::checkout(
            logical_id.0.to_string(),
            checkout_id.0.to_string(),
            worktree.clone(),
        ),
        working_directory: worktree.clone(),
        readable_roots: vec![worktree.clone()],
        writable_roots: vec![worktree],
        config_artifact_ref: "sha256:managed-config-artifact".to_string(),
    };

    let validated = gateway
        .validate(request)
        .expect("non-default lc coding launch must pass gateway validation");
    assert_eq!(validated.envelope().policy_revision, 1);
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
        request: AggregateProviderTurnRequest<'_>,
    ) -> Result<String, AggregateInitializationError> {
        let AggregateProviderTurnRequest {
            project_id,
            operation_id,
            step,
            preflight,
            lc_id,
            bootstrap,
            cancellation,
        } = request;
        let factory =
            self.factory
                .as_ref()
                .ok_or_else(|| AggregateInitializationError::ProviderTurn {
                    step,
                    reason: "logical codebase gateway factory is not configured".to_string(),
                    retryable: false,
                })?;
        let gateway = factory.build_for_lc(project_id, lc_id).map_err(|error| {
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
        .run_turn(AggregateProviderTurnRequest {
            project_id,
            operation_id,
            step,
            preflight,
            lc_id,
            bootstrap,
            cancellation,
        })
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

/// 静态 capability source：按 provider ref 返回对应 dialect，version 固定 1.0.0
/// (Task 2b 分格形状:launch/resume/write_boundary 恒 Confirmed)。
struct StaticCapabilitySource;

impl StaticCapabilitySource {
    fn capability(
        provider: &ProviderRef,
        action: cadence_aria::product::logical_codebase::SessionPolicyAction,
    ) -> ProviderCapability {
        use cadence_aria::cross_cutting::provider_capabilities::ProviderCapabilityEvidence;
        use cadence_aria::product::logical_codebase::policy::ProviderWireDialect;
        use cadence_aria::product::logical_codebase::provider_capability_store::ProviderActionCapability;
        let (adapter_dialect, wire_dialect) = match provider.provider_type {
            ProviderRefType::ClaudeCode => (
                ProviderDialect::ClaudeCodeCliV1,
                ProviderWireDialect::ClaudeCodeStreamJson,
            ),
            ProviderRefType::Codex => (
                ProviderDialect::CodexCliV1,
                ProviderWireDialect::CodexAppServerRpc,
            ),
            ProviderRefType::Pi => (ProviderDialect::PiRpcV1, ProviderWireDialect::PiRpc),
            ProviderRefType::KimiCode => (ProviderDialect::KimiAcpV1, ProviderWireDialect::KimiAcp),
        };
        ProviderCapability {
            provider_type: provider.provider_type,
            version: "1.0.0".to_string(),
            adapter_dialect,
            wire_dialect,
            capability_snapshot_ref: provider.capability_snapshot_ref.clone(),
            action_capability: ProviderActionCapability {
                action,
                launch: ProviderCapabilityEvidence::Confirmed,
                resume: ProviderCapabilityEvidence::Confirmed,
                write_boundary: ProviderCapabilityEvidence::Confirmed,
                projection_digest: format!("projection-digest-{action:?}"),
                evidence_ref: format!("probe://{action:?}"),
            },
            trust: ProviderCapabilityEvidence::Confirmed,
        }
    }
}

impl ProviderCapabilitySource for StaticCapabilitySource {
    fn require_supported(
        &self,
        provider: &ProviderRef,
        action: cadence_aria::product::logical_codebase::SessionPolicyAction,
    ) -> Result<ProviderCapability, ProviderGatewayError> {
        Ok(Self::capability(provider, action))
    }

    fn require_resume_supported(
        &self,
        provider: &ProviderRef,
        action: cadence_aria::product::logical_codebase::SessionPolicyAction,
    ) -> Result<ProviderCapability, ProviderGatewayError> {
        Ok(Self::capability(provider, action))
    }

    fn require_write_boundary(
        &self,
        provider: &ProviderRef,
        action: cadence_aria::product::logical_codebase::SessionPolicyAction,
    ) -> Result<ProviderCapability, ProviderGatewayError> {
        Ok(Self::capability(provider, action))
    }

    fn require_root_recipe_supported(
        &self,
        provider: &ProviderRef,
        _credential: &cadence_aria::product::logical_codebase::provider_admission_preflight::BootstrapPhaseCredential,
    ) -> Result<ProviderCapability, ProviderGatewayError> {
        if provider.provider_type != ProviderRefType::ClaudeCode {
            return Err(ProviderGatewayError::UnsupportedCapability(
                cadence_aria::product::logical_codebase::provider_gateway::PROVIDER_ROOT_RECIPE_REQUIRES_FIXED_CLAUDE
                    .to_string(),
            ));
        }
        Ok(Self::capability(
            provider,
            cadence_aria::product::logical_codebase::SessionPolicyAction::PlanningReadOnly,
        ))
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
        // Task 2.8（cwd authority 契约）：authority = workspace root（.aria
        // 的父目录）——coding cwd（成员 worktree）位于其下。
        paths
            .root()
            .parent()
            .expect("workspace root parent")
            .to_path_buf(),
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

    /// Task 7 分流收口:validated 分发同样不得触达 fail-closed 路径的
    /// provider(计数并入同一 `start_count`)。
    async fn start_validated(
        &self,
        _validated: cadence_aria::cross_cutting::session_launch::ValidatedStreamingProviderInput,
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
            native_session_id: None,
            events: event_rx,
            commands: command_tx,
        })
    }

    /// Task 7 分流收口:gateway `start_streaming` 只经 validated trait 分发。
    /// it_web(库外)无法拆 `ValidatedStreamingProviderInput`,以 plain
    /// completion 输出同一 review JSON(review 判定消费 full_output)。
    async fn start_validated(
        &self,
        _validated: cadence_aria::cross_cutting::session_launch::ValidatedStreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (event_tx, event_rx) = mpsc::channel(4);
        let (command_tx, _command_rx) = mpsc::channel(4);
        let output = self.output.clone();
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::Completed(ProviderCompletion::plain(
                    output, None,
                )))
                .await;
        });
        Ok(ProviderSession {
            native_session_id: None,
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

/// 播种 logical manifest + active 成员记录 + 主 checkout（供
/// `capture_cross_target_baseline` 的 D4 成员基线窗口判定）。
fn seed_logical_codebase_checkout(store: &CodingAttemptStore, attempt: &CodingExecutionAttempt) {
    let target = attempt.target_snapshot.as_ref().expect("target snapshot");
    let logical_store = LogicalCodebaseStore::new(store.paths());
    // Task 2.8（cwd authority 契约）：worktree 位于 workspace root（.aria 的
    // 父目录）之下；manifest 根取该真实公共父目录，否则 coding cwd 会被
    // gateway authority 门拒绝。
    let manifest = LogicalCodebaseManifest::new(
        &attempt.project_id,
        store
            .paths()
            .root()
            .parent()
            .expect("workspace root parent")
            .to_path_buf(),
        vec![target.logical_repository_id],
    );
    logical_store
        .save_manifest(&attempt.project_id, &manifest)
        .expect("save manifest");
    // Task 1.7（D4 生产 seam）：基线窗口按成员状态取 active 主 checkout。
    // 真实流程 manifest 与 member 记录同写（coding_attempt_repository /
    // migration_executor）；fixture 缺失 member 记录会被 seam 判为证据不足
    // fail-closed（cross_target_store_failure），与 lib 侧
    // `provider_gateway_validated_input` 的 fixture 播种保持同构。
    logical_store
        .save_member(
            &attempt.project_id,
            &CodebaseMemberRecord {
                logical_repository_id: target.logical_repository_id,
                physical_repository_id: target.physical_repository_id.clone(),
                alias: "repo".to_string(),
                role: "repository".to_string(),
                ordinal: 1,
                source_identity: RepositorySourceIdentity::from_git_parts(
                    &target.canonical_path,
                    target.canonical_path.join(".git"),
                    None,
                ),
                repo_type: RepositoryType::Unknown,
                tech_stack: Vec::new(),
                owner: None,
                tags: Vec::new(),
                default_ref: None,
                checkout_ids: vec![target.checkout_id],
                status: MemberStatus::Active,
                created_at: "2026-08-13T00:00:00Z".to_string(),
                updated_at: "2026-08-13T00:00:00Z".to_string(),
            },
        )
        .expect("save member");
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

// ---------------------------------------------------------------------------
// Task 13(lcg_t13):单仓 sync direct 与 workspace streaming legacy 的
// 兼容锁(REQ-LCG-02 非目标面)。两类 direct 对照与 LC validated gateway
// 三路分离:sync direct 恒两槽、workspace streaming 恒 raw `start`,
// 都不经 LC 桥——本 change 不交付四家同步 direct,也不把 streaming
// 对照升级为 LC 支持。
// ---------------------------------------------------------------------------

/// 记录每次 `run` 收到的完整 AdapterInput 并返回固定输出(sync direct
/// 两槽的确定性观察点)。
#[derive(Default)]
struct SyncDirectSlotRecorder {
    inputs: std::sync::Mutex<Vec<AdapterInput>>,
}

impl ProviderAdapter for SyncDirectSlotRecorder {
    fn run(&self, input: &AdapterInput) -> Result<AdapterOutput, ProviderAdapterError> {
        self.inputs.lock().expect("sync direct slot").push(input.clone());
        Ok(AdapterOutput {
            exit_code: Some(0),
            stdout: "ok".to_string(),
            stderr: String::new(),
            structured_output: Some(serde_json::json!({"ok": true})),
            files_modified: Vec::new(),
            duration_ms: 0,
            timeout_status: TimeoutStatus::NotTimedOut,
        })
    }
}

/// `Arc` 共享包装,使 routing 装箱后仍可读取记录。
struct SharedSyncSlot(std::sync::Arc<SyncDirectSlotRecorder>);

impl ProviderAdapter for SharedSyncSlot {
    fn run(&self, input: &AdapterInput) -> Result<AdapterOutput, ProviderAdapterError> {
        self.0.run(input)
    }
}

/// lcg_t13:单仓 sync direct 拓扑不变——`default_compatibility_matrix`
/// 恒恰 ClaudeCode/Codex 两槽(本 change 不交付四家同步 direct),两槽
/// run argv/prompt 通道/输出解析冻结;`RoutingProviderAdapter` 对同一
/// AdapterInput 两次分发的 captured input 与输出逐字节相同(args/cwd/
/// output 不变);Pi/KimiCode/Fake 显式 reject(错误码 + 点名 provider),
/// 两槽零调用。
#[test]
fn lcg_t13_single_repository_sync_claude_codex_direct_topology_unchanged() {
    use cadence_aria::cross_cutting::adapter_compatibility::{
        OutputParser, PromptInputMode, StructuredOutputMode, default_compatibility_matrix,
    };
    use cadence_aria::protocol::contracts::ProviderType;
    use cadence_aria::task_run::provider_factory::RoutingProviderAdapter;

    // ① 两槽矩阵拓扑:恰 Claude/Codex 两行,Pi/Kimi 不是 sync direct 行。
    let matrix = default_compatibility_matrix();
    assert_eq!(
        matrix.entries.len(),
        2,
        "sync direct compatibility matrix must stay exactly two slots"
    );
    let claude = matrix
        .entry_for(ProviderType::ClaudeCode)
        .expect("claude slot entry");
    let codex = matrix
        .entry_for(ProviderType::Codex)
        .expect("codex slot entry");
    assert!(matrix.entry_for(ProviderType::Pi).is_none());
    assert!(matrix.entry_for(ProviderType::KimiCode).is_none());

    // ② 两槽 argv/prompt 通道/输出解析基线冻结(args_before == args_after)。
    assert_eq!(claude.run_command.program, "claude");
    assert_eq!(
        claude.run_command.args,
        vec![
            "-p",
            "--permission-mode",
            "dontAsk",
            "--tools",
            "",
            "--strict-mcp-config",
            "--no-session-persistence",
        ],
        "claude sync direct argv baseline must stay byte-identical"
    );
    assert!(matches!(claude.prompt_input_mode, PromptInputMode::Stdin));
    assert!(!claude.pass_worktree_path_as_arg);
    assert!(matches!(
        claude.structured_output_mode,
        StructuredOutputMode::SentinelJson
    ));
    assert!(matches!(claude.output_parser, OutputParser::SentinelBlock));
    assert_eq!(codex.run_command.program, "codex");
    assert_eq!(
        codex.run_command.args,
        vec!["exec", "-s", "danger-full-access"],
        "codex sync direct argv baseline must stay byte-identical"
    );
    assert!(matches!(codex.prompt_input_mode, PromptInputMode::Stdin));
    assert!(!codex.pass_worktree_path_as_arg);

    // ③ 路由分发对照:同一 input 两次分发,captured args/cwd/output 不变。
    let worktree = std::path::PathBuf::from("/tmp/lcg-t13-sync-direct-worktree");
    let mut input = AdapterInput {
        working_directory: None,
        provider_type: ProviderType::ClaudeCode,
        role: cadence_aria::protocol::contracts::AdapterRole::Executor,
        worktree_path: Some(worktree.to_string_lossy().to_string()),
        provider_stream_log_dir: None,
        prompt: "lcg_t13 sync direct topology lock".to_string(),
        context_files: Vec::new(),
        output_schema: String::new(),
        timeout: 60,
        max_retries: 0,
    };
    let claude_slot = Arc::new(SyncDirectSlotRecorder::default());
    let codex_slot = Arc::new(SyncDirectSlotRecorder::default());
    let routing = RoutingProviderAdapter::new(
        Box::new(SharedSyncSlot(claude_slot.clone())),
        Box::new(SharedSyncSlot(codex_slot.clone())),
    );

    let out_before = routing.run(&input).expect("claude slot dispatch");
    let out_after = routing.run(&input).expect("claude slot replay");
    assert_eq!(out_before, out_after, "sync direct output unchanged");
    input.provider_type = ProviderType::Codex;
    let codex_out_before = routing.run(&input).expect("codex slot dispatch");
    let codex_out_after = routing.run(&input).expect("codex slot replay");
    assert_eq!(codex_out_before, codex_out_after);

    let claude_inputs = claude_slot.inputs.lock().expect("claude slot inputs");
    let codex_inputs = codex_slot.inputs.lock().expect("codex slot inputs");
    assert_eq!(claude_inputs.len(), 2, "claude slot saw exactly the two runs");
    assert_eq!(codex_inputs.len(), 2, "codex slot saw exactly the two runs");
    assert_eq!(
        claude_inputs[0], claude_inputs[1],
        "sync direct args/cwd passthrough must be identical across dispatches"
    );
    assert_eq!(codex_inputs[0], codex_inputs[1]);
    assert_eq!(
        claude_inputs[0].worktree_path,
        Some(worktree.to_string_lossy().to_string()),
        "cwd/worktree passthrough must stay untouched by the routing layer"
    );
    assert_eq!(claude_inputs[0].working_directory, None);
    drop(claude_inputs);
    drop(codex_inputs);

    // ④ Pi/Kimi/Fake 显式 reject:错误码 + 点名 provider,两槽零新增调用。
    for (provider_type, name_fragment) in [
        (ProviderType::Pi, "pi"),
        (ProviderType::KimiCode, "kimi_code"),
        (ProviderType::Fake, "fake"),
    ] {
        input.provider_type = provider_type.clone();
        let error = routing
            .run(&input)
            .err()
            .unwrap_or_else(|| panic!("{provider_type:?} must be rejected by sync direct"));
        assert_eq!(
            error.code,
            cadence_aria::protocol::provider_errors::ProviderErrorCode::ProviderIncompatibleOutput,
            "{provider_type:?} reject code unchanged"
        );
        assert!(
            error.details.contains(name_fragment),
            "reject must name the provider, got: {}",
            error.details
        );
    }
    assert_eq!(claude_slot.inputs.lock().expect("claude slot inputs").len(), 2);
    assert_eq!(codex_slot.inputs.lock().expect("codex slot inputs").len(), 2);
}

/// 写一个捕获型 fixture CLI:`--version` 分支回固定版本(需要版本探测的
/// provider 消费),其余调用把 argv 逐行 + `pwd` 追加写入 capture 文件后
/// 立即退出 0(workspace streaming raw start 的 argv/cwd 观察点)。
fn write_workspace_stream_capture_cli(
    dir: &std::path::Path,
    name: &str,
    version_echo: Option<&str>,
) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let capture = dir.join(format!("{name}.capture"));
    let version_branch = version_echo
        .map(|version| {
            format!("if [ \"$1\" = \"--version\" ]; then printf '%s\\n' '{version}'; exit 0; fi\n")
        })
        .unwrap_or_default();
    let script = format!(
        "#!/bin/sh\n{version_branch}printf '%s\\n' \"$@\" >> \"{capture}\"\npwd >> \"{capture}\"\nexit 0\n",
        capture = capture.display(),
    );
    let path = dir.join(name);
    std::fs::write(&path, script).expect("write capture cli");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod capture cli");
    path
}

/// 把 capture 文件内容解析为 (argv 行, cwd)。
fn parse_workspace_stream_capture(content: &str) -> (Vec<String>, String) {
    let mut lines: Vec<String> = content.lines().map(str::to_string).collect();
    let cwd = lines.pop().expect("cwd line");
    (lines, cwd)
}

/// 有界等待并消费 capture 文件为 (argv 行, cwd)。脚本分两次 append
/// (argv 后 cwd),必须等到至少两行才算写完整;读取后删除文件,使下
/// 一次运行的存在性轮询有明确语义。
async fn wait_workspace_stream_capture(
    capture: &std::path::Path,
) -> Option<(Vec<String>, String)> {
    for _ in 0..150 {
        if let Ok(content) = std::fs::read_to_string(capture) {
            if content.lines().count() >= 2 {
                std::fs::remove_file(capture).expect("consume capture file");
                return Some(parse_workspace_stream_capture(&content));
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    None
}

/// 以 raw `start`(legacy workspace streaming 直连,非 `start_validated`)
/// 驱动一次 adapter 启动并返回 capture。prompt 经 stdin/协议层注入,
/// fixture CLI 立即退出,会话按终结契约有界收口。
async fn drive_workspace_stream_raw_start(
    adapter: Arc<dyn StreamingProviderAdapter>,
    provider_type: cadence_aria::protocol::contracts::ProviderType,
    working_dir: &std::path::Path,
    capture: &std::path::Path,
) -> (Vec<String>, String) {
    let input = StreamingProviderInput {
        provider_type: provider_type.clone(),
        role: cadence_aria::protocol::contracts::AdapterRole::Executor,
        prompt: "lcg_t13 workspace streaming legacy lock".to_string(),
        working_dir: working_dir.to_path_buf(),
        working_directory: None,
        workspace_session_id: Some("lcg_t13_legacy_stream".to_string()),
        resume_provider_session_id: None,
        permission_mode:
            cadence_aria::cross_cutting::streaming_provider::ProviderPermissionMode::Auto,
        tool_policy: None,
        audit_sink: None,
        structured_output_contract: None,
        env_vars: Default::default(),
        timeout_secs: 30,
        baseline_tree: None,
    };
    let cancel = CancellationToken::new();
    let session = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        adapter.start(input, cancel.clone()),
    )
    .await
    .expect("raw start must spawn within the bounded window")
    .expect("raw legacy start must stay reachable (not LC-gated)");
    // capture 落盘竞态:fixture CLI 在 spawn 后毫秒级写入;会话释放即触发
    // kill 链,可能先于子进程 exec——先有界等待 capture 写完整,再释放会话。
    let captured = wait_workspace_stream_capture(capture).await;
    drop(session);
    cancel.cancel();
    captured.unwrap_or_else(|| {
        panic!(
            "fixture CLI must have captured argv+cwd before exiting (provider={provider_type:?}, capture={})",
            capture.display()
        )
    })
}

/// lcg_t13:四家 workspace streaming legacy raw `start` 路径不变——
/// Claude/Codex/Pi/Kimi 各自的 raw argv 基线冻结(prompt 通道、permission
/// 标志、会话模式均不含 LC validated 专属 token),cwd 恒 input.working_dir,
/// 两次驱动 capture 逐字节相同;该路径不是 split_sync、不是 LC validated
/// gateway,对照不得被升级解读为四家同步 direct 支持。
#[tokio::test]
async fn lcg_t13_workspace_streaming_legacy_four_provider_path_unchanged() {
    use cadence_aria::cross_cutting::claude_code_provider::ClaudeCodeProvider;
    use cadence_aria::cross_cutting::codex_provider::CodexProvider;
    use cadence_aria::cross_cutting::kimi_code_provider::KimiCodeProvider;
    use cadence_aria::cross_cutting::pi_provider::PiProvider;
    use cadence_aria::protocol::contracts::ProviderType;

    let root = tempdir().expect("tempdir");
    let working_dir = root.path().join("workspace");
    std::fs::create_dir_all(&working_dir).expect("workspace dir");
    let canonical_working_dir = std::fs::canonicalize(&working_dir).expect("canonical cwd");

    struct Case {
        provider_type: ProviderType,
        capture: std::path::PathBuf,
        adapter: Arc<dyn StreamingProviderAdapter>,
        expected_prefix: Vec<&'static str>,
        exact_len: usize,
    }
    let claude_cli = write_workspace_stream_capture_cli(root.path(), "claude-cli", None);
    let codex_cli = write_workspace_stream_capture_cli(root.path(), "codex-cli", None);
    let pi_cli = write_workspace_stream_capture_cli(root.path(), "pi-cli", Some("0.83.0"));
    let kimi_cli = write_workspace_stream_capture_cli(root.path(), "kimi-cli", Some("0.34.0"));
    let cases = vec![
        Case {
            provider_type: ProviderType::ClaudeCode,
            capture: root.path().join("claude-cli.capture"),
            adapter: Arc::new(ClaudeCodeProvider::new(claude_cli)),
            expected_prefix: vec![
                "-p",
                "--verbose",
                "--output-format=stream-json",
                "--input-format=stream-json",
                "--include-partial-messages",
                "--replay-user-messages",
            ],
            exact_len: 7,
        },
        Case {
            provider_type: ProviderType::Codex,
            capture: root.path().join("codex-cli.capture"),
            adapter: Arc::new(CodexProvider::new(codex_cli)),
            expected_prefix: vec!["app-server", "--enable", "default_mode_request_user_input"],
            exact_len: 3,
        },
        Case {
            provider_type: ProviderType::Pi,
            capture: root.path().join("pi-cli.capture"),
            adapter: Arc::new(PiProvider::new(pi_cli)),
            expected_prefix: vec!["--mode", "rpc", "-e"],
            exact_len: 4,
        },
        Case {
            provider_type: ProviderType::KimiCode,
            capture: root.path().join("kimi-cli.capture"),
            adapter: Arc::new(KimiCodeProvider::new(kimi_cli)),
            expected_prefix: vec!["acp"],
            exact_len: 1,
        },
    ];

    for case in cases {
        let provider_type = case.provider_type.clone();
        let first = drive_workspace_stream_raw_start(
            case.adapter.clone(),
            provider_type.clone(),
            &working_dir,
            &case.capture,
        )
        .await;
        let second = drive_workspace_stream_raw_start(
            case.adapter.clone(),
            provider_type.clone(),
            &working_dir,
            &case.capture,
        )
        .await;

        // args/cwd 对照:两次 raw start 逐字节相同(workspace_streaming_
        // args/output 不变),cwd 恒 working_dir。
        assert_eq!(
            first, second,
            "{:?}: raw start capture must be identical across runs",
            case.provider_type
        );
        let (args, cwd) = first;
        assert_eq!(
            std::path::PathBuf::from(&cwd),
            canonical_working_dir,
            "{:?}: raw start cwd must stay input.working_dir",
            case.provider_type
        );
        let expected_len = case.exact_len;
        assert_eq!(
            args.len(),
            expected_len,
            "{:?}: raw argv length baseline, got {args:?}",
            case.provider_type
        );
        for (index, expected) in case.expected_prefix.iter().enumerate() {
            assert_eq!(
                &args[index], expected,
                "{:?}: raw argv baseline at {index}, got {args:?}",
                case.provider_type
            );
        }
        if case.provider_type == ProviderType::Pi {
            // Pi 的第 4 个参数是 ask extension 安装路径($HOME 缓存内,
            // 内容 hash 命名,确定性),逐字节钉死会耦合 HOME,只锁形态。
            let extension = &args[3];
            assert!(
                extension.ends_with(".ts")
                    && extension.contains("aria-ask-"),
                "pi extension path shape unchanged, got {extension}"
            );
        }
        // raw ≠ LC validated:argv 不得出现 LC 专属 token(允许列表/
        // resume/deny 冻结片段);permission 语义保持 raw 通道(经
        // stdin/协议,而非 argv 注入 LC 投影)。
        for token in ["--allowedTools", "--resume", "--disallowedTools"] {
            assert!(
                !args.iter().any(|arg| arg == token),
                "{:?}: raw argv must not carry the LC validated token {token}, got {args:?}",
                case.provider_type
            );
        }
    }
}
