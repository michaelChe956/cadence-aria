// T10b:规划 review via-gateway 骨架的 lib 层单测。
//
// 覆盖:
// - 注入 fake-registry gateway 后调用 `drive_review_session_via_gateway` →
//   会话驱动正常(reviewer pass → HumanConfirm)+ gateway audit `stream_launches` +1。
// - 既有 `drive_review_session`(Legacy,未注入 gateway)路径在其余 part 中保持
//   零变化,本文件不重复覆盖。

use crate::cross_cutting::provider_adapter::ProviderAdapter;
use crate::cross_cutting::provider_availability_gate::{
    ProviderAvailabilityGate, ProviderHealthSource,
};
use crate::cross_cutting::provider_health::{ProviderHealthEntry, ProviderHealthSnapshot};
use crate::cross_cutting::provider_registry::ProviderRegistry;
use crate::product::logical_codebase::provider_gateway::ResumeEvidenceState;
use crate::product::logical_codebase::{
    AggregatePolicyArtifactStore, GatewayRunAudit, LogicalCodebaseManifest,
    LogicalCodebaseProviderGateway, PolicyTarget, PolicyTargetResolver, ProviderCapability,
    ProviderCapabilitySource, ProviderDialect, ProviderGatewayError, ProviderRef, ProviderRefType,
    SessionLaunchRequest, SessionPolicyAction,
};
use crate::protocol::contracts::{AdapterOutput, TimeoutStatus};

/// 同步 adapter stub:`run` 返回最小成功输出,供 gateway 构造。
struct ReviewStubSyncAdapter;

impl ProviderAdapter for ReviewStubSyncAdapter {
    fn run(&self, _input: &AdapterInput) -> Result<AdapterOutput, ProviderAdapterError> {
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

/// 测试用 capability source:返回固定 capability,resume 证据恒为 `Confirmed`,
/// 使 review repair 的 resume 启动也能通过 spawn 前复验。同时记录每次
/// `require_supported` 收到的 `ProviderRef`,供断言「launch request 的 provider
/// ref 随 session.reviewer_provider 透传」(C-2 身份契约)。
#[derive(Default)]
struct ReviewStaticCapabilitySource {
    seen: Arc<Mutex<Vec<ProviderRef>>>,
}

impl ReviewStaticCapabilitySource {
    /// 已收到的 provider ref 快照(启动身份审计用)。
    fn seen_provider_refs(&self) -> Vec<ProviderRef> {
        self.seen.lock().unwrap().clone()
    }
}

impl ProviderCapabilitySource for ReviewStaticCapabilitySource {
    fn require_supported(
        &self,
        provider: &ProviderRef,
        _action: SessionPolicyAction,
    ) -> Result<ProviderCapability, ProviderGatewayError> {
        self.seen.lock().unwrap().push(provider.clone());
        let adapter_dialect = match provider.provider_type {
            ProviderRefType::ClaudeCode => ProviderDialect::ClaudeCodeCliV1,
            ProviderRefType::Codex => ProviderDialect::CodexCliV1,
        };
        Ok(ProviderCapability {
            provider_type: provider.provider_type,
            version: "1.4.0".to_string(),
            adapter_dialect,
            capability_snapshot_ref: provider.capability_snapshot_ref.clone(),
            resume_evidence: ResumeEvidenceState::Confirmed,
        })
    }
}

/// 测试用 target resolver:直接透传 request target。aggregate_root target 的
/// worktree 与 input.working_dir 相同,spawn 前 canonicalize 复验可通过对等比较。
struct ReviewPassThroughTargetResolver;

impl PolicyTargetResolver for ReviewPassThroughTargetResolver {
    fn resolve_and_revalidate(
        &self,
        request: &SessionLaunchRequest,
    ) -> Result<PolicyTarget, ProviderGatewayError> {
        Ok(request.target.clone())
    }
}

fn review_always_available_gate() -> Arc<ProviderAvailabilityGate> {
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

struct ReviewGatewayFixture {
    _root: tempfile::TempDir,
    paths: ProductAppPaths,
    gateway: Arc<LogicalCodebaseProviderGateway>,
    capabilities: Arc<ReviewStaticCapabilitySource>,
    audit: Arc<GatewayRunAudit>,
    worktree: std::path::PathBuf,
}

fn review_gateway_fixture() -> ReviewGatewayFixture {
    let root = tempfile::tempdir().expect("temporary product root");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let worktree = root.path().join("worktree");
    std::fs::create_dir_all(&worktree).expect("create review worktree");

    let manifest = LogicalCodebaseManifest::new("project_0001", worktree.clone(), Vec::new());
    let policy_store = AggregatePolicyArtifactStore::new(paths.clone());
    policy_store
        .ensure_bootstrap(&manifest)
        .expect("bootstrap aggregate policy");

    // fake registry:ClaudeCode 映射到「输出 pass verdict」的 streaming adapter。
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::ClaudeCode,
        Arc::new(ReviewVerdictStreamingProvider {
            output: "审核通过。\n\n```json\n{\"verdict\":\"pass\",\"summary\":\"可以确认\"}\n```",
            provider_type: Arc::new(Mutex::new(None)),
            prompt: Arc::new(Mutex::new(None)),
        }),
    );

    let capabilities = Arc::new(ReviewStaticCapabilitySource::default());
    let audit = Arc::new(GatewayRunAudit::new());
    let gateway = Arc::new(LogicalCodebaseProviderGateway::with_audit(
        policy_store,
        capabilities.clone(),
        Arc::new(ReviewPassThroughTargetResolver),
        Arc::new(registry),
        Arc::new(ReviewStubSyncAdapter),
        review_always_available_gate(),
        audit.clone(),
        worktree.clone(),
    ));

    ReviewGatewayFixture {
        _root: root,
        paths,
        gateway,
        capabilities,
        audit,
        worktree,
    }
}

#[tokio::test]
async fn drive_review_session_via_gateway_records_audit_and_completes() {
    let fixture = review_gateway_fixture();
    let audit = fixture.audit.clone();
    assert_eq!(audit.stream_launches(), 0);

    let (event_tx, _event_rx) = mpsc::channel(64);
    let mut session = make_session("sess_review_via_gateway");
    session.review_rounds = 2;
    session.reviewer_provider = Some(ProviderName::ClaudeCode);
    session.artifact = Some(artifact_payload("# Artifact\n\n可以确认"));
    session.repository_path = Some(fixture.worktree.clone());

    let mut engine = WorkspaceEngine::new(
        Arc::new(CheckpointStore::new(
            fixture.paths.root().join("checkpoints"),
        )),
        event_tx,
        session,
    )
    .with_logical_provider_gateway(fixture.gateway.clone());

    engine.start_review_or_skip().await;
    engine
        .drive_review_session_via_gateway(empty_provider_commands())
        .await;

    assert_eq!(
        audit.stream_launches(),
        1,
        "logical review session must start via gateway once"
    );
    // C-2 身份契约:launch request 的 provider ref 必须随 session.reviewer_provider
    // (此处为 ClaudeCode)透传到 gateway validate,不得硬编码别的 provider。
    // (validate/spawn 复验会多次咨询 capability source,故断言「全部一致」而非次数。)
    let seen = fixture.capabilities.seen_provider_refs();
    assert!(
        !seen.is_empty(),
        "gateway must consult capability for the session-configured reviewer provider"
    );
    assert!(
        seen.iter().all(|r| r == &ProviderRef::claude_code("cap_managed_snapshot")),
        "every gateway validate/revalidate must use the session-configured reviewer, got {seen:?}"
    );
    assert_eq!(
        engine.session().stage,
        WorkspaceStage::AuthorConfirm,
        "Story/Design pass verdict must return its report to author confirmation"
    );
    assert!(
        engine.timeline_nodes.iter().any(|node| {
            node.node_type == TimelineNodeType::ReviewerRun
                && node.status == TimelineNodeStatus::Completed
                && node.summary.as_deref() == Some("Review 完成，报告已进入对话流")
        }),
        "Story/Design reviewer run must complete before its report returns to author confirmation"
    );
}

/// C-2(组3):reviewer 配置 Kimi 时不得静默回退 Claude。logical review 启动必须
/// 在集中映射处 fail-closed:错误事件含判别码与 provider 名、gateway audit 零启动、
/// capability source 零调用(映射失败发生在 gateway validate 之前)。回归锁定:
/// 修复前该配置会静默用 ClaudeCode 跑完整 review。
#[tokio::test]
async fn drive_review_session_via_gateway_fails_closed_for_unsupported_reviewer() {
    let fixture = review_gateway_fixture();
    let audit = fixture.audit.clone();
    assert_eq!(audit.stream_launches(), 0);

    let (event_tx, mut event_rx) = mpsc::channel(64);
    let mut session = make_session("sess_review_kimi_via_gateway");
    session.review_rounds = 2;
    session.reviewer_provider = Some(ProviderName::KimiCode);
    session.artifact = Some(artifact_payload("# Artifact\n\n可以确认"));
    session.repository_path = Some(fixture.worktree.clone());

    let mut engine = WorkspaceEngine::new(
        Arc::new(CheckpointStore::new(
            fixture.paths.root().join("checkpoints"),
        )),
        event_tx,
        session,
    )
    .with_logical_provider_gateway(fixture.gateway.clone());

    engine.start_review_or_skip().await;
    engine
        .drive_review_session_via_gateway(empty_provider_commands())
        .await;

    let mut start_failure = None;
    while let Ok(event) = event_rx.try_recv() {
        if let crate::product::workspace_engine::EngineEvent::Error { message } = event {
            start_failure = Some(message);
        }
    }
    let message = start_failure.expect("unsupported reviewer must surface an error event");
    assert!(
        message.contains("provider_unsupported_for_gateway_launch"),
        "expected provider_unsupported_for_gateway_launch, got: {message}"
    );
    assert!(
        message.contains("KimiCode"),
        "error must name the configured reviewer provider, got: {message}"
    );
    assert_eq!(
        audit.stream_launches(),
        0,
        "unsupported reviewer must not start any gateway session"
    );
    // 集中映射失败发生在 review launch request 组装处:KimiCode 不存在对应的
    // `ProviderRefType`,绝不可能出现在 capability 咨询记录里(记录中只允许
    // author 侧路由投影的 ClaudeCode)。
    assert!(
        fixture
            .capabilities
            .seen_provider_refs()
            .iter()
            .all(|r| r.provider_type == ProviderRefType::ClaudeCode),
        "KimiCode must never reach gateway validate as a provider ref"
    );
}

/// C-2(组2):reviewer 配置 Codex 时，review 启动必须命中 REQ-ENV-05 路由级硬门
/// (codex_danger_full_access_unsupported)，不得静默改成 Claude 跑 review。
#[tokio::test]
async fn drive_review_session_via_gateway_blocks_codex_reviewer_at_route() {
    let fixture = review_gateway_fixture();
    let audit = fixture.audit.clone();
    assert_eq!(audit.stream_launches(), 0);

    let (event_tx, mut event_rx) = mpsc::channel(64);
    let mut session = make_session("sess_review_codex_via_gateway");
    session.review_rounds = 2;
    session.reviewer_provider = Some(ProviderName::Codex);
    session.artifact = Some(artifact_payload("# Artifact\n\n可以确认"));
    session.repository_path = Some(fixture.worktree.clone());

    let mut engine = WorkspaceEngine::new(
        Arc::new(CheckpointStore::new(
            fixture.paths.root().join("checkpoints"),
        )),
        event_tx,
        session,
    )
    .with_logical_provider_gateway(fixture.gateway.clone());

    engine.start_review_or_skip().await;
    engine
        .drive_review_session_via_gateway(empty_provider_commands())
        .await;

    let mut start_failure = None;
    while let Ok(event) = event_rx.try_recv() {
        if let crate::product::workspace_engine::EngineEvent::Error { message } = event {
            start_failure = Some(message);
        }
    }
    let message = start_failure.expect("codex reviewer must surface an error event");
    assert!(
        message.contains("codex_danger_full_access_unsupported"),
        "expected codex_danger_full_access_unsupported, got: {message}"
    );
    assert_eq!(
        audit.stream_launches(),
        0,
        "route-blocked reviewer must not start any gateway session"
    );
    assert!(
        fixture
            .capabilities
            .seen_provider_refs()
            .iter()
            .any(|r| r.provider_type == ProviderRefType::Codex),
        "the launch must have validated the session-configured Codex reviewer"
    );
}

/// 测试用 target resolver：透传 request target 并记录之，用于验证
/// `routing_reference_context` 构造 aggregate_root target 时镜像 factory 的
/// canonicalize 语义。
#[derive(Default)]
struct RecordingTargetResolver {
    seen: Arc<Mutex<Option<PolicyTarget>>>,
}

impl PolicyTargetResolver for RecordingTargetResolver {
    fn resolve_and_revalidate(
        &self,
        request: &SessionLaunchRequest,
    ) -> Result<PolicyTarget, ProviderGatewayError> {
        *self.seen.lock().unwrap() = Some(request.target.clone());
        Ok(request.target.clone())
    }
}

/// 测试用 target resolver：恒失败，用于验证 gateway validate Err → Legacy 行为不变。
struct FailingTargetResolver;

impl PolicyTargetResolver for FailingTargetResolver {
    fn resolve_and_revalidate(
        &self,
        _request: &SessionLaunchRequest,
    ) -> Result<PolicyTarget, ProviderGatewayError> {
        Err(ProviderGatewayError::Target("stub failure".to_string()))
    }
}

/// 构造一个最小 gateway：静态 capability + 指定 target resolver + 真实 bootstrap 的
/// aggregate policy。`routing_reference_context` 只走 validate（政策 + capability +
/// target），不触发真实 provider run。
fn routing_context_gateway<T: PolicyTargetResolver + 'static>(
    root: &tempfile::TempDir,
    resolver: T,
) -> (Arc<LogicalCodebaseProviderGateway>, std::path::PathBuf) {
    routing_context_gateway_with_capability(
        root,
        resolver,
        Arc::new(ReviewStaticCapabilitySource::default()),
    )
}

/// 同 `routing_context_gateway`，但调用方持有 capability source，供断言
/// 「投影 request 的 provider ref 随 session.author_provider」（C-2）。
fn routing_context_gateway_with_capability<T: PolicyTargetResolver + 'static>(
    root: &tempfile::TempDir,
    resolver: T,
    capabilities: Arc<ReviewStaticCapabilitySource>,
) -> (Arc<LogicalCodebaseProviderGateway>, std::path::PathBuf) {
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let worktree = root.path().join("worktree");
    std::fs::create_dir_all(&worktree).expect("create worktree");

    let manifest = LogicalCodebaseManifest::new("project_0001", worktree.clone(), Vec::new());
    let policy_store = AggregatePolicyArtifactStore::new(paths.clone());
    policy_store
        .ensure_bootstrap(&manifest)
        .expect("bootstrap aggregate policy");

    let gateway = Arc::new(LogicalCodebaseProviderGateway::with_audit(
        policy_store,
        capabilities,
        Arc::new(resolver),
        Arc::new(ProviderRegistry::new()),
        Arc::new(ReviewStubSyncAdapter),
        review_always_available_gate(),
        Arc::new(GatewayRunAudit::new()),
        worktree.clone(),
    ));
    (gateway, worktree)
}

#[test]
fn routing_reference_context_returns_legacy_without_gateway() {
    let root = tempfile::tempdir().expect("temporary product root");
    let (event_tx, _event_rx) = mpsc::channel(64);
    let engine = WorkspaceEngine::new(
        Arc::new(CheckpointStore::new(root.path().join("checkpoints"))),
        event_tx,
        make_session("sess_no_gateway"),
    );

    assert!(matches!(
        engine.routing_reference_context(),
        RoutingReferenceContext::Legacy
    ));
}

#[test]
fn routing_reference_context_canonicalizes_aggregate_root_target() {
    let root = tempfile::tempdir().expect("temporary product root");
    let resolver = RecordingTargetResolver::default();
    let seen = resolver.seen.clone();
    let (gateway, worktree) = routing_context_gateway(&root, resolver);

    // repository_path 故意带 `..`，验证 target worktree 在构造时被 canonicalize。
    let non_canonical = worktree.join("..").join("worktree");
    let canonical = std::fs::canonicalize(&non_canonical).expect("canonicalize non-canonical path");

    let (event_tx, _event_rx) = mpsc::channel(64);
    let mut session = make_session("sess_logical");
    session.repository_path = Some(non_canonical);
    let engine = WorkspaceEngine::new(
        Arc::new(CheckpointStore::new(root.path().join("checkpoints"))),
        event_tx,
        session,
    )
    .with_logical_provider_gateway(gateway);

    assert!(matches!(
        engine.routing_reference_context(),
        RoutingReferenceContext::Logical(_)
    ));

    let recorded = seen
        .lock()
        .unwrap()
        .clone()
        .expect("resolver must record target");
    assert_eq!(recorded.worktree, canonical);
}

/// C-2(组2):author 配置 Codex 时，投影 request 必须以 session 配置的 Codex ref
/// 校验(被 REQ-ENV-05 路由级硬门阻断后回落 Legacy)——不得硬编码 ClaudeCode
/// 假装校验通过。
#[test]
fn routing_reference_context_projects_session_codex_author_not_hardcoded_claude() {
    let root = tempfile::tempdir().expect("temporary product root");
    let capabilities = Arc::new(ReviewStaticCapabilitySource::default());
    let (gateway, worktree) = routing_context_gateway_with_capability(
        &root,
        ReviewPassThroughTargetResolver,
        capabilities.clone(),
    );

    let (event_tx, _event_rx) = mpsc::channel(64);
    let mut session = make_session("sess_author_codex");
    session.author_provider = ProviderName::Codex;
    session.repository_path = Some(worktree);
    let engine = WorkspaceEngine::new(
        Arc::new(CheckpointStore::new(root.path().join("checkpoints"))),
        event_tx,
        session,
    )
    .with_logical_provider_gateway(gateway);

    // Codex 被路由级硬门阻断 → validate 失败 → 回落 Legacy(既有 fail-open 契约)。
    assert!(matches!(
        engine.routing_reference_context(),
        RoutingReferenceContext::Legacy
    ));
    let seen = capabilities.seen_provider_refs();
    assert!(
        seen.iter().any(|r| r.provider_type == ProviderRefType::Codex
            && r.capability_snapshot_ref == "cap_managed_snapshot"),
        "projection must validate the session-configured Codex author, got {seen:?}"
    );
}

/// C-2(组3):author 配置 Pi 时投影无 gateway dialect → 回落 Legacy(prompt 路由
/// 引用不假装 Logical，也不触达 gateway validate)；真实启动在集中映射处
/// fail-closed(由 gateway_start 测试锁定)。
#[test]
fn routing_reference_context_unsupported_author_falls_back_to_legacy() {
    let root = tempfile::tempdir().expect("temporary product root");
    let capabilities = Arc::new(ReviewStaticCapabilitySource::default());
    let (gateway, worktree) = routing_context_gateway_with_capability(
        &root,
        ReviewPassThroughTargetResolver,
        capabilities.clone(),
    );

    let (event_tx, _event_rx) = mpsc::channel(64);
    let mut session = make_session("sess_author_pi");
    session.author_provider = ProviderName::Pi;
    session.repository_path = Some(worktree);
    let engine = WorkspaceEngine::new(
        Arc::new(CheckpointStore::new(root.path().join("checkpoints"))),
        event_tx,
        session,
    )
    .with_logical_provider_gateway(gateway);

    assert!(matches!(
        engine.routing_reference_context(),
        RoutingReferenceContext::Legacy
    ));
    assert!(
        capabilities.seen_provider_refs().is_empty(),
        "unsupported author must not reach gateway validate from the prompt projection"
    );
}

#[test]
fn routing_reference_context_returns_legacy_when_validate_fails() {
    let root = tempfile::tempdir().expect("temporary product root");
    let (gateway, worktree) = routing_context_gateway(&root, FailingTargetResolver);

    let (event_tx, _event_rx) = mpsc::channel(64);
    let mut session = make_session("sess_validate_fails");
    session.repository_path = Some(worktree);
    let engine = WorkspaceEngine::new(
        Arc::new(CheckpointStore::new(root.path().join("checkpoints"))),
        event_tx,
        session,
    )
    .with_logical_provider_gateway(gateway);

    // validate Err → Legacy 行为不变。`tracing::warn!` 是日志级副作用，
    // 无稳定可捕获的断言方式，此处不测试日志输出本身。
    assert!(matches!(
        engine.routing_reference_context(),
        RoutingReferenceContext::Legacy
    ));
}
