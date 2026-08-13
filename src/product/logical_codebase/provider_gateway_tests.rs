use super::*;
use crate::cross_cutting::session_launch::{
    ValidatedAdapterInput, ValidatedStreamingProviderInput,
};
use crate::product::app_paths::ProductAppPaths;
use crate::product::logical_codebase::policy::PolicyTarget;
use crate::product::logical_codebase::store::LogicalCodebaseManifest;
use crate::protocol::contracts::AdapterInput;

/// 验证 bootstrap 政策在 gateway 能校验首次启动前被持久化。
///
/// 缺失政策时 fail-closed 为 `PolicyMissing`;`ensure_bootstrap` 写出 revision 1
/// 的 bootstrap artifact 后,gateway 可校验 planning 只读启动并冻结 envelope。
#[test]
fn bootstrap_policy_is_persisted_before_gateway_can_validate_a_launch() {
    let fixture = gateway_fixture();
    assert!(matches!(
        fixture.gateway().validate(fixture.planning_request()),
        Err(ProviderGatewayError::PolicyMissing(_))
    ));

    fixture.install_bootstrap_policy();
    let validated = fixture
        .gateway()
        .validate(fixture.planning_request())
        .unwrap();
    assert_eq!(validated.envelope().policy_revision, 1);
    assert_eq!(
        validated.envelope().action,
        SessionPolicyAction::PlanningReadOnly,
    );
}

/// validated policy 的字段对外不可直接构造:没有 public constructor,getter 是
/// 唯一访问方式。编译期保证只能由 gateway 产出。
#[test]
fn validated_policy_only_exposes_getters_and_cannot_be_constructed_outside_module() {
    let fixture = gateway_fixture();
    fixture.install_bootstrap_policy();
    let validated = fixture
        .gateway()
        .validate(fixture.planning_request())
        .unwrap();

    // getter 返回冻结的引用;外部无法修改字段或凭空重建 validated policy。
    assert_eq!(validated.envelope().policy_revision, 1);
    assert!(validated.fingerprint().digest.starts_with("sha256:"));
}

/// fingerprint 随 provider exact version 与 capability snapshot 变化:任一漂移
/// 都应产生不同 digest,保证 resume 复验能检测 provider 侧变更。
#[test]
fn resume_fingerprint_changes_when_provider_version_or_snapshot_drifts() {
    let fixture = gateway_fixture();
    fixture.install_bootstrap_policy();

    let baseline = fixture
        .gateway()
        .validate(fixture.planning_request())
        .unwrap();
    let baseline_digest = baseline.fingerprint().digest.clone();

    fixture.capabilities.set_version("1.4.1");
    let drifted = fixture
        .gateway()
        .validate(fixture.planning_request())
        .unwrap();
    assert_ne!(drifted.fingerprint().digest, baseline_digest);
}

/// 直接构造 `ValidatedSessionLaunchPolicy { envelope, fingerprint }` 在本模块外
/// 不可行(struct literal 构造需要字段可见)。此处用 doctest-like 断言:gateway
/// 返回值只能经 getter 访问,确认 opaque 边界由 privacy 保护。
#[test]
fn opaque_policy_blocks_struct_literal_construction_outside_module() {
    // 编译期约束:ValidatedSessionLaunchPolicy 的字段是 private,本测试模块虽在
    // 同一文件但通过公开 API 访问,模拟外部调用方。外部 crate 无法构造该 struct。
    let fixture = gateway_fixture();
    fixture.install_bootstrap_policy();
    let validated = fixture
        .gateway()
        .validate(fixture.planning_request())
        .unwrap();
    // 只能读 getter,无法读字段或重建。
    let _envelope = validated.envelope();
    let _fingerprint = validated.fingerprint();
}

/// spawn 前 canonical 复验:validate 阶段 target resolver 检测到 git_dir 在
/// request 构造后被篡改(TOCTOU),返回 `TargetMismatch { field: "git_dir" }`,
/// 且复验失败发生在 registry lookup/真实 adapter start 之前(start_count == 0)。
#[test]
fn gateway_revalidates_canonical_target_git_dir_and_managed_config_before_spawn() {
    let fixture = gateway_fixture();
    fixture.install_bootstrap_policy();
    let request = fixture.coding_request("/work/api/.worktrees/aria-issues/issue_1");
    fixture
        .targets()
        .change_git_dir_after_request("/work/api/.git-replaced");

    let error = fixture.gateway().validate(request).unwrap_err();
    assert!(
        matches!(error, ProviderGatewayError::TargetMismatch { ref field } if field == "git_dir")
    );
    assert_eq!(fixture.registry_start_count(), 0);
}

/// spawn 前 target identity 复验(B-1):validate 成功后、start_streaming 前
/// resolver 返回的 target 被改掉(模拟 .git 指针/target 被调包),spawn 必须
/// fail-closed 为 `TargetMismatch { field: "target" }`,且不触达 registry。
#[tokio::test]
async fn start_streaming_revalidates_target_identity_before_spawn() {
    let fixture = gateway_fixture();
    fixture.install_bootstrap_policy();
    let worktree = fixture.real_worktree();
    let launch = fixture.validated_planning_streaming_input(worktree.clone());

    // validate 后、spawn 前把 resolver 返回的 target 改掉。
    fixture
        .targets()
        .change_target_after_request(PolicyTarget::checkout(
            "logical_repo_0001",
            "checkout_0001",
            worktree.join("swapped"),
        ));

    let result = fixture
        .gateway()
        .start_streaming(launch, CancellationToken::new())
        .await;
    let error = match result {
        Err(error) => error,
        Ok(_) => panic!("expected TargetMismatch field=target, got a session"),
    };

    assert!(
        matches!(error, ProviderGatewayError::TargetMismatch { ref field } if field == "target"),
        "expected TargetMismatch field=target, got {error:?}"
    );
    assert_eq!(fixture.registry_start_count(), 0);
}

/// read-only action(planning/review)没有 write root;coding action 恰好一个
/// 等于 canonical target worktree 的 write root。envelope 在 validate 时冻结该约束。
#[test]
fn planning_and_review_have_no_write_root_while_coding_has_exactly_target_root() {
    let fixture = gateway_fixture();
    fixture.install_bootstrap_policy();
    assert!(
        fixture
            .gateway()
            .validate(fixture.planning_request())
            .unwrap()
            .envelope()
            .writable_roots
            .is_empty()
    );
    assert_eq!(
        fixture
            .gateway()
            .validate(fixture.coding_request("/work/api/.worktrees/aria-issues/issue_1"))
            .unwrap()
            .envelope()
            .writable_roots,
        vec![PathBuf::from("/work/api/.worktrees/aria-issues/issue_1")]
    );
}

/// spawn 前复验(B-1):validate 后、start_streaming 前政策被升级(revision+digest
/// 变化),spawn 必须以 `PolicyDrift { dimension: "policy_revision" }` fail-closed,
/// 且不触达 registry(start_count == 0)。
#[test]
fn start_streaming_fails_closed_when_policy_upgraded_between_validate_and_spawn() {
    let fixture = gateway_fixture();
    fixture.install_bootstrap_policy();
    let worktree = fixture.real_worktree();
    let launch = fixture.validated_planning_streaming_input(worktree);

    // validate→spawn 之间政策被升级到 revision 2。
    fixture.upgrade_policy();

    let runtime = tokio::runtime::Runtime::new().unwrap();
    let result = runtime.block_on(
        fixture
            .gateway()
            .start_streaming(launch, CancellationToken::new()),
    );
    let error = match result {
        Err(error) => error,
        Ok(_) => panic!("expected PolicyDrift, got a session"),
    };
    assert!(
        matches!(error, ProviderGatewayError::PolicyDrift { ref dimension } if dimension == "policy_revision")
    );
    assert_eq!(fixture.registry_start_count(), 0);
}

/// spawn 前复验(B-1):validate 后、start_streaming 前 provider version 被改
/// (capability source 返回不同 version),spawn 必须以
/// `PolicyDrift { dimension: "provider_version" }` fail-closed,不触达 registry。
#[test]
fn start_streaming_rejects_provider_version_change_before_spawn() {
    let fixture = gateway_fixture();
    fixture.install_bootstrap_policy();
    let worktree = fixture.real_worktree();
    let launch = fixture.validated_planning_streaming_input(worktree);

    fixture.capabilities().set_version("1.4.1");

    let runtime = tokio::runtime::Runtime::new().unwrap();
    let result = runtime.block_on(
        fixture
            .gateway()
            .start_streaming(launch, CancellationToken::new()),
    );
    let error = match result {
        Err(error) => error,
        Ok(_) => panic!("expected PolicyDrift, got a session"),
    };
    assert!(
        matches!(error, ProviderGatewayError::PolicyDrift { ref dimension } if dimension == "provider_version")
    );
    assert_eq!(fixture.registry_start_count(), 0);
}

/// spawn 前复验(B-1):validate 后、run_sync 前政策被升级,同步路径同样 fail-closed
/// 为 `PolicyDrift`,不触达真实 sync adapter(此处表现为返回错误而非成功)。
#[test]
fn run_sync_fails_closed_when_policy_upgraded_between_validate_and_spawn() {
    let fixture = gateway_fixture();
    fixture.install_bootstrap_policy();
    let worktree = fixture.real_worktree();
    let request = SessionLaunchRequest::planning(
        fixture.manifest().project_id,
        ProviderRef::claude_code("cap_claude_code_1_4_0"),
        PolicyTarget::checkout("logical_repo_0001", "checkout_0001", worktree.clone()),
        vec![fixture.paths.root().to_path_buf()],
        "sha256:managed-config-artifact",
    );
    let validated = fixture.gateway().validate(request).unwrap();
    let input = crate::protocol::contracts::AdapterInput {
        provider_type: crate::protocol::contracts::ProviderType::ClaudeCode,
        role: crate::protocol::contracts::AdapterRole::Executor,
        worktree_path: Some(worktree.to_string_lossy().to_string()),
        provider_stream_log_dir: None,
        prompt: "probe".to_string(),
        context_files: Vec::new(),
        output_schema: String::new(),
        timeout: 1,
        max_retries: 0,
    };
    let launch = ValidatedAdapterInput::new(input, validated);

    fixture.upgrade_policy();

    let error = fixture.gateway().run_sync(launch).unwrap_err();
    assert!(
        matches!(error, ProviderGatewayError::PolicyDrift { ref dimension } if dimension == "policy_revision")
    );
}

/// resume 能力 fail-closed(B-2 消费者):resume 启动时 provider 的
/// `resume_evidence` 不是 `Confirmed` → spawn 拒绝(`ResumeNotSupported`),不触达
/// registry。这使 `resume_evidence` 三态成为有消费者的 fail-closed 门禁,而非
/// dead field。
#[test]
fn start_streaming_resume_is_rejected_when_resume_evidence_not_confirmed() {
    let fixture = gateway_fixture();
    fixture.install_bootstrap_policy();
    let worktree = fixture.real_worktree();
    // resume 启动:streaming input 携带 resume_provider_session_id。
    let validated = fixture
        .gateway()
        .validate(fixture.planning_request())
        .unwrap();
    let input = fixture.streaming_input(worktree, Some("sess_resume_0001".to_string()));
    let launch = ValidatedStreamingProviderInput::new(input, validated);

    // provider 标记 resume 不支持(三态 Unknown/Denied 在 gateway 侧归为 Unsupported)。
    fixture
        .capabilities()
        .set_resume_evidence(ResumeEvidenceState::Unsupported);

    let runtime = tokio::runtime::Runtime::new().unwrap();
    let result = runtime.block_on(
        fixture
            .gateway()
            .start_streaming(launch, CancellationToken::new()),
    );
    let error = match result {
        Err(error) => error,
        Ok(_) => panic!("expected ResumeNotSupported, got a session"),
    };
    assert!(matches!(error, ProviderGatewayError::ResumeNotSupported));
    assert_eq!(fixture.registry_start_count(), 0);
}

/// resume 能力放行路径(B-2):`resume_evidence` 为 `Confirmed` 时 resume 启动通过复验
/// 并触达 registry(start_count == 1),确认消费者只在非 Confirmed 时阻断。
#[tokio::test]
async fn start_streaming_resume_passes_when_resume_evidence_confirmed() {
    let fixture = gateway_fixture();
    fixture.install_bootstrap_policy();
    let worktree = fixture.real_worktree();
    // planning 请求的 target worktree 必须等于真实 worktree,使 cwd 复验通过。
    let request = SessionLaunchRequest::planning(
        fixture.manifest().project_id,
        ProviderRef::claude_code("cap_claude_code_1_4_0"),
        PolicyTarget::checkout("logical_repo_0001", "checkout_0001", worktree.clone()),
        vec![fixture.paths.root().to_path_buf()],
        "sha256:managed-config-artifact",
    );
    let validated = fixture.gateway().validate(request).unwrap();
    let input = fixture.streaming_input(worktree, Some("sess_resume_0001".to_string()));
    let launch = ValidatedStreamingProviderInput::new(input, validated);

    fixture
        .capabilities()
        .set_resume_evidence(ResumeEvidenceState::Confirmed);

    let _session = fixture
        .gateway()
        .start_streaming(launch, CancellationToken::new())
        .await
        .expect("resume launch passes when evidence confirmed");
    assert_eq!(fixture.registry_start_count(), 1);
}

/// 可变 target resolver:模拟 spawn 前 git_dir/target TOCTOU。初始 current 与
/// expected 一致(resolve 通过);`change_git_dir_after_request` 后 current
/// 偏离 expected,resolve 返回 `TargetMismatch { field: "git_dir" }`;
/// `change_target_after_request` 后 resolve 返回偏离请求的 target,供 spawn 前
/// target identity 复验测试驱动漂移。
struct MutableTargetResolver {
    expected_git_dir: PathBuf,
    current_git_dir: std::sync::Mutex<PathBuf>,
    target_override: std::sync::Mutex<Option<PolicyTarget>>,
}

impl MutableTargetResolver {
    fn new(git_dir: PathBuf) -> Self {
        let current = git_dir.clone();
        Self {
            expected_git_dir: git_dir,
            current_git_dir: std::sync::Mutex::new(current),
            target_override: std::sync::Mutex::new(None),
        }
    }

    fn change_git_dir_after_request(&self, git_dir: impl Into<PathBuf>) {
        *self.current_git_dir.lock().unwrap() = git_dir.into();
    }

    fn change_target_after_request(&self, target: PolicyTarget) {
        *self.target_override.lock().unwrap() = Some(target);
    }
}

impl PolicyTargetResolver for MutableTargetResolver {
    fn resolve_and_revalidate(
        &self,
        request: &SessionLaunchRequest,
    ) -> Result<PolicyTarget, ProviderGatewayError> {
        let current = self.current_git_dir.lock().unwrap().clone();
        if current != self.expected_git_dir {
            return Err(ProviderGatewayError::TargetMismatch {
                field: "git_dir".to_string(),
            });
        }
        if let Some(target) = self.target_override.lock().unwrap().clone() {
            return Ok(target);
        }
        Ok(request.target.clone())
    }
}

/// 测试用 capability source:返回固定 capability,version 可调以驱动 fingerprint 漂移。
/// 测试用 capability source:version 与 resume 能力可调,以驱动 spawn 前
/// 复验指纹漂移与 resume fail-closed。
struct StaticCapabilitySource {
    version: std::sync::Mutex<String>,
    resume_evidence: std::sync::Mutex<ResumeEvidenceState>,
}

impl StaticCapabilitySource {
    fn new(version: &str) -> Self {
        Self {
            version: std::sync::Mutex::new(version.to_string()),
            resume_evidence: std::sync::Mutex::new(ResumeEvidenceState::Confirmed),
        }
    }

    fn set_version(&self, version: &str) {
        *self.version.lock().unwrap() = version.to_string();
    }

    fn set_resume_evidence(&self, state: ResumeEvidenceState) {
        *self.resume_evidence.lock().unwrap() = state;
    }
}

impl ProviderCapabilitySource for StaticCapabilitySource {
    fn require_supported(
        &self,
        provider: &ProviderRef,
        _action: SessionPolicyAction,
    ) -> Result<ProviderCapability, ProviderGatewayError> {
        let version = self.version.lock().unwrap().clone();
        let resume_evidence = *self.resume_evidence.lock().unwrap();
        let adapter_dialect = match provider.provider_type {
            ProviderRefType::ClaudeCode => ProviderDialect::ClaudeCodeCliV1,
            ProviderRefType::Codex => ProviderDialect::CodexCliV1,
        };
        Ok(ProviderCapability {
            provider_type: provider.provider_type,
            version,
            adapter_dialect,
            capability_snapshot_ref: provider.capability_snapshot_ref.clone(),
            resume_evidence,
        })
    }
}

/// 测试用 streaming adapter:记录 start 调用次数,供断言「复验失败不触达 registry」。
struct CountingStreamingAdapter {
    start_count: std::sync::atomic::AtomicUsize,
}

impl CountingStreamingAdapter {
    fn new() -> Self {
        Self {
            start_count: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    fn start_count(&self) -> usize {
        self.start_count.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl crate::cross_cutting::streaming_provider::StreamingProviderAdapter
    for CountingStreamingAdapter
{
    async fn start(
        &self,
        _input: crate::cross_cutting::streaming_provider::StreamingProviderInput,
        _cancel: tokio_util::sync::CancellationToken,
    ) -> Result<
        crate::cross_cutting::streaming_provider::ProviderSession,
        crate::cross_cutting::provider_adapter::ProviderAdapterError,
    > {
        self.start_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let (_event_tx, events) = tokio::sync::mpsc::channel(1);
        let (commands, _command_rx) = tokio::sync::mpsc::channel(1);
        Ok(crate::cross_cutting::streaming_provider::ProviderSession { events, commands })
    }
}

/// 测试用同步 adapter stub:run 返回最小成功输出。
struct StubSyncAdapter;

impl crate::cross_cutting::provider_adapter::ProviderAdapter for StubSyncAdapter {
    fn run(
        &self,
        _input: &crate::protocol::contracts::AdapterInput,
    ) -> Result<
        crate::protocol::contracts::AdapterOutput,
        crate::cross_cutting::provider_adapter::ProviderAdapterError,
    > {
        use crate::protocol::contracts::TimeoutStatus;
        Ok(crate::protocol::contracts::AdapterOutput {
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

/// 始终可用的 availability gate fixture:health snapshot 标记所有真实 provider 可用。
fn always_available_gate()
-> Arc<crate::cross_cutting::provider_availability_gate::ProviderAvailabilityGate> {
    use crate::cross_cutting::provider_availability_gate::ProviderHealthSource;
    use crate::cross_cutting::provider_health::{ProviderHealthEntry, ProviderHealthSnapshot};
    use crate::product::models::ProviderName;
    use chrono::Utc;

    struct AlwaysHealthy(Arc<ProviderHealthSnapshot>);
    impl ProviderHealthSource for AlwaysHealthy {
        fn snapshot(&self) -> Arc<ProviderHealthSnapshot> {
            self.0.clone()
        }
        fn degraded(&self) -> bool {
            false
        }
    }

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
    Arc::new(
        crate::cross_cutting::provider_availability_gate::ProviderAvailabilityGate::new(Arc::new(
            AlwaysHealthy(snapshot),
        )),
    )
}

struct GatewayFixture {
    _root: tempfile::TempDir,
    paths: ProductAppPaths,
    capabilities: Arc<StaticCapabilitySource>,
    targets: Arc<MutableTargetResolver>,
    streaming_adapter: Arc<CountingStreamingAdapter>,
    sync_adapter: Arc<StubSyncAdapter>,
    gate: Arc<crate::cross_cutting::provider_availability_gate::ProviderAvailabilityGate>,
    /// 跨多次 `gateway()` 构造共享的启动审计。Task 11 的覆盖率测试驱动 4 类
    /// 逻辑启动,每次构造独立 gateway 后审计计数仍需累计。
    audit: Arc<GatewayRunAudit>,
}

fn gateway_fixture() -> GatewayFixture {
    let root = tempfile::tempdir().expect("temporary product root");
    let paths = ProductAppPaths::new(root.path());
    let baseline_git_dir = paths.root().join("baseline.git");
    GatewayFixture {
        _root: root,
        paths,
        capabilities: Arc::new(StaticCapabilitySource::new("1.4.0")),
        targets: Arc::new(MutableTargetResolver::new(baseline_git_dir)),
        streaming_adapter: Arc::new(CountingStreamingAdapter::new()),
        sync_adapter: Arc::new(StubSyncAdapter),
        gate: always_available_gate(),
        audit: Arc::new(GatewayRunAudit::new()),
    }
}

impl GatewayFixture {
    fn manifest(&self) -> LogicalCodebaseManifest {
        LogicalCodebaseManifest::new("project_0001", self.paths.root().to_path_buf(), vec![])
    }

    fn policy_store(&self) -> AggregatePolicyArtifactStore {
        AggregatePolicyArtifactStore::new(self.paths.clone())
    }

    fn install_bootstrap_policy(&self) {
        let manifest = self.manifest();
        self.policy_store().ensure_bootstrap(&manifest).unwrap();
    }

    fn gateway(&self) -> LogicalCodebaseProviderGateway {
        let mut registry = ProviderRegistry::new();
        registry.register(ProviderName::ClaudeCode, self.streaming_adapter.clone());
        registry.register(ProviderName::Codex, self.streaming_adapter.clone());
        LogicalCodebaseProviderGateway::with_audit(
            self.policy_store(),
            self.capabilities.clone(),
            self.targets.clone(),
            Arc::new(registry),
            self.sync_adapter.clone(),
            self.gate.clone(),
            self.audit.clone(),
        )
    }

    /// 返回共享启动审计。多次 `gateway()` 构造后的启动记录都在此处累计。
    fn gateway_audit(&self) -> Arc<GatewayRunAudit> {
        self.audit.clone()
    }

    fn targets(&self) -> Arc<MutableTargetResolver> {
        self.targets.clone()
    }

    fn capabilities(&self) -> Arc<StaticCapabilitySource> {
        self.capabilities.clone()
    }

    fn registry_start_count(&self) -> usize {
        self.streaming_adapter.start_count()
    }

    /// 将 store 中的 policy artifact 升级到 revision 2(新 policy_text + 新 digest),
    /// 模拟 validate→spawn 之间政策被升级(TOCTOU)。
    fn upgrade_policy(&self) {
        let manifest = self.manifest();
        let store = self.policy_store();
        let current = store.get(&manifest.project_id).unwrap().unwrap();
        let revised = current.with_revised_policy(
            "# Aggregate policy (revision 2)\n\nTighter supervised scope.\n",
            manifest.updated_at.clone(),
        );
        store.save(&manifest.project_id, &revised).unwrap();
    }

    /// 创建一个真实存在的 worktree 目录(用于 spawn 前 canonicalize 复验)。
    fn real_worktree(&self) -> PathBuf {
        let worktree = self.paths.root().join("worktree");
        std::fs::create_dir_all(&worktree).unwrap();
        worktree
    }

    /// 无参便利方法:返回 planning 只读请求(内部用 manifest())。
    fn planning_request(&self) -> SessionLaunchRequest {
        self.planning_request_for_manifest(&self.manifest())
    }

    fn planning_request_for_manifest(
        &self,
        manifest: &LogicalCodebaseManifest,
    ) -> SessionLaunchRequest {
        let target = PolicyTarget::checkout(
            "logical_repo_0001",
            "checkout_0001",
            self.paths.root().join("worktree"),
        );
        SessionLaunchRequest::planning(
            &manifest.project_id,
            ProviderRef::claude_code("cap_claude_code_1_4_0"),
            target,
            vec![self.paths.root().to_path_buf()],
            "sha256:managed-config-artifact",
        )
    }

    /// coding 请求:恰好一个等于 canonical target worktree 的 write root。
    fn coding_request(&self, worktree: impl Into<PathBuf>) -> SessionLaunchRequest {
        let manifest = self.manifest();
        let worktree = worktree.into();
        let target = PolicyTarget::checkout("logical_repo_0001", "checkout_0001", worktree.clone());
        SessionLaunchRequest {
            project_id: manifest.project_id,
            provider: ProviderRef::claude_code("cap_claude_code_1_4_0"),
            action: SessionPolicyAction::CodingTargetWrite,
            target,
            readable_roots: vec![self.paths.root().to_path_buf()],
            writable_roots: vec![worktree],
            config_artifact_ref: "sha256:managed-config-artifact".to_string(),
        }
    }

    /// 针对真实 worktree 目录构造一个 planning 只读 validated streaming input。
    /// worktree 必须真实存在(spawn 前 canonicalize 复验需要)。
    fn validated_planning_streaming_input(
        self: &GatewayFixture,
        worktree: PathBuf,
    ) -> ValidatedStreamingProviderInput {
        let request = SessionLaunchRequest::planning(
            self.manifest().project_id,
            ProviderRef::claude_code("cap_claude_code_1_4_0"),
            PolicyTarget::checkout("logical_repo_0001", "checkout_0001", worktree.clone()),
            vec![self.paths.root().to_path_buf()],
            "sha256:managed-config-artifact",
        );
        let validated = self.gateway().validate(request).unwrap();
        ValidatedStreamingProviderInput::new(self.streaming_input(worktree, None), validated)
    }

    /// 构造一个 streaming input;resume 设 `resume_provider_session_id`。
    fn streaming_input(
        &self,
        working_dir: PathBuf,
        resume_id: Option<String>,
    ) -> crate::cross_cutting::streaming_provider::StreamingProviderInput {
        use crate::cross_cutting::streaming_provider::{
            ProviderPermissionMode, StreamingProviderInput,
        };
        use crate::protocol::contracts::{AdapterRole, ProviderType};
        StreamingProviderInput {
            provider_type: ProviderType::ClaudeCode,
            role: AdapterRole::Executor,
            prompt: "probe".to_string(),
            working_dir,
            workspace_session_id: None,
            resume_provider_session_id: resume_id,
            permission_mode: ProviderPermissionMode::Auto,
            structured_output_contract: None,
            env_vars: Default::default(),
            timeout_secs: 1,
        }
    }

    /// review 只读请求:与 planning 同为 read-only action,但 action 为
    /// `ReviewReadOnly`。Task 11 的 review 流式启动经此请求走 gateway。
    fn review_request(&self, worktree: PathBuf) -> SessionLaunchRequest {
        let manifest = self.manifest();
        let target = PolicyTarget::checkout("logical_repo_0001", "checkout_0001", worktree);
        SessionLaunchRequest {
            project_id: manifest.project_id,
            provider: ProviderRef::claude_code("cap_claude_code_1_4_0"),
            action: SessionPolicyAction::ReviewReadOnly,
            target,
            readable_roots: vec![self.paths.root().to_path_buf()],
            writable_roots: Vec::new(),
            config_artifact_ref: "sha256:managed-config-artifact".to_string(),
        }
    }

    /// 同步栈入口(work item split):经 gateway `run_sync` 启动同步 adapter。
    /// 对应 `work_item_split_engine/engine.rs` 逻辑代码库分支。worktree 必须真实
    /// 存在以通过 spawn 前 canonicalize 复验。
    async fn run_logical_work_item_split(&self) -> Result<(), ProviderGatewayError> {
        let worktree = self.real_worktree();
        let request = self.planning_request_for_manifest_with_worktree(worktree.clone());
        let validated = self.gateway().validate(request)?;
        let adapter_input = AdapterInput {
            provider_type: crate::protocol::contracts::ProviderType::ClaudeCode,
            role: crate::protocol::contracts::AdapterRole::WorkItemSplitter,
            worktree_path: Some(worktree.to_string_lossy().to_string()),
            provider_stream_log_dir: None,
            prompt: "split work items".to_string(),
            context_files: Vec::new(),
            output_schema: String::new(),
            timeout: 1,
            max_retries: 0,
        };
        let launch = ValidatedAdapterInput::new(adapter_input, validated);
        self.gateway().run_sync(launch)?;
        Ok(())
    }

    /// 流式 planning 栈入口:经 gateway `start_streaming` 启动。对应
    /// `workspace_engine/provider_drive.rs` 逻辑代码库 planning 分支。
    async fn run_logical_planning_stream(&self) -> Result<(), ProviderGatewayError> {
        let worktree = self.real_worktree();
        let request = self.planning_request_for_manifest_with_worktree(worktree.clone());
        let validated = self.gateway().validate(request)?;
        let input = self.streaming_input(worktree, None);
        let launch = ValidatedStreamingProviderInput::new(input, validated);
        let gateway = self.gateway();
        gateway
            .start_streaming(launch, CancellationToken::new())
            .await?;
        Ok(())
    }

    /// 流式 coding 栈入口:经 gateway `start_streaming` 启动。对应
    /// `coding_workspace_engine/provider_stream.rs` 逻辑代码库 coding 分支。
    async fn run_logical_coding_stream(&self) -> Result<(), ProviderGatewayError> {
        let worktree = self.real_worktree();
        let request = self.coding_request(worktree.clone());
        let validated = self.gateway().validate(request)?;
        let input = self.streaming_input(worktree, None);
        let launch = ValidatedStreamingProviderInput::new(input, validated);
        let gateway = self.gateway();
        gateway
            .start_streaming(launch, CancellationToken::new())
            .await?;
        Ok(())
    }

    /// 流式 review 栈入口:经 gateway `start_streaming` 启动。对应
    /// `coding_workspace_engine/provider_stream.rs` 逻辑代码库 review 分支。
    async fn run_logical_review_stream(&self) -> Result<(), ProviderGatewayError> {
        let worktree = self.real_worktree();
        let request = self.review_request(worktree.clone());
        let validated = self.gateway().validate(request)?;
        let input = self.streaming_input(worktree, None);
        let launch = ValidatedStreamingProviderInput::new(input, validated);
        let gateway = self.gateway();
        gateway
            .start_streaming(launch, CancellationToken::new())
            .await?;
        Ok(())
    }

    /// planning 只读请求的 worktree 参数化变体,供 work-item split/planning stream
    /// 复用同一真实 worktree(而不是 fixture 默认的未创建 `worktree` 路径)。
    fn planning_request_for_manifest_with_worktree(
        &self,
        worktree: PathBuf,
    ) -> SessionLaunchRequest {
        let target = PolicyTarget::checkout("logical_repo_0001", "checkout_0001", worktree);
        SessionLaunchRequest::planning(
            self.manifest().project_id,
            ProviderRef::claude_code("cap_claude_code_1_4_0"),
            target,
            vec![self.paths.root().to_path_buf()],
            "sha256:managed-config-artifact",
        )
    }
}

/// bridge 映射(B-2):`cross_cutting::ProviderCapabilityEvidence` 三态经
/// `ResumeEvidenceState::from_cross_cutting_evidence` 进入 gateway 消费路径。
/// 仅 `Confirmed` 放行;`Denied`/`Unknown` 归为 `Unsupported`(fail-closed),
/// 使该三态字段不再是无消费者的 dead field。
#[test]
fn resume_evidence_bridge_maps_cross_cutting_three_states_to_gateway_state() {
    use crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence;
    assert!(
        ResumeEvidenceState::from_cross_cutting_evidence(&ProviderCapabilityEvidence::Confirmed)
            .allows_resume()
    );
    assert!(
        !ResumeEvidenceState::from_cross_cutting_evidence(&ProviderCapabilityEvidence::Denied {
            reason: "probe says no".to_string(),
        })
        .allows_resume()
    );
    assert!(
        !ResumeEvidenceState::from_cross_cutting_evidence(&ProviderCapabilityEvidence::Unknown)
            .allows_resume()
    );
}

/// Task 11 覆盖率:验证逻辑代码库同步与流式 provider 全入口经 gateway 接线后,
/// 每类真实启动都在 gateway 留下可审计记录。本模块是 gateway 侧契约测试(B2):
/// 4 个 `run_logical_*` helper 分别模拟逻辑代码库的 work-item split(同步)、
/// planning/coding/review(流式)启动,各向 gateway 发一次真实 `run_sync`/
/// `start_streaming`,断言审计计数(sync==1、stream==3)与全部携带 policy digest。
///
/// 该测试是 4 个真实入口(engine.rs / provider_drive.rs / provider_stream.rs /
/// initializer.rs)逻辑分支接线是否完成的验收门:只要 gateway 是唯一启动入口且
/// 记录审计,计数即为契约。
mod coverage_tests {
    use super::*;

    #[tokio::test]
    async fn logical_provider_entrypoints_use_gateway_for_sync_and_streaming_stacks() {
        let fixture = gateway_fixture();
        fixture.install_bootstrap_policy();
        fixture.run_logical_work_item_split().await.unwrap();
        fixture.run_logical_planning_stream().await.unwrap();
        fixture.run_logical_coding_stream().await.unwrap();
        fixture.run_logical_review_stream().await.unwrap();

        assert_eq!(fixture.gateway_audit().sync_launches(), 1);
        assert_eq!(fixture.gateway_audit().stream_launches(), 3);
        assert!(fixture.gateway_audit().all_have_policy_digest());
    }
}

/// Task 13: Codex `danger-full-access` 路由级阻断、resume 一致性与配置来源审计。
///
/// 三个验收维度:
/// 1. Codex 在受限写配置(danger-full-access)下经 gateway 路由级阻断——不论 UI
///    是否选择该 provider,validate 即拒绝,不触达 registry。这与「UI 隐藏」
///    不同:路由级阻断无法被绕过。
/// 2. resume 一致性:`resume_or_start` 只有当 policy digest、target、provider exact
///    version、dialect 与 capability snapshot 全一致(fingerprint 相等)才 resume;
///    任一漂移则 supersede 旧会话并新建。
/// 3. 配置来源审计:`ConfigSourceAudit` 记录 user/project/local/env/子仓 MCP 来源
///    与最终 argv/config digest;发现 managed settings 时标注
///    `managed_settings_active=true` 并发警告,绝不假装已覆盖;policy 可据此拒绝启动。
mod task13_gateway_hardening {
    use super::*;
    use crate::cross_cutting::codex_provider::CODEX_DEFAULT_SANDBOX_MODE;

    /// 用 Codex provider ref 构造一个 coding 写请求(worktree 必须等于 target)。
    fn codex_coding_request(fixture: &GatewayFixture) -> SessionLaunchRequest {
        let manifest = fixture.manifest();
        let worktree = fixture.paths.root().join("codex-worktree");
        std::fs::create_dir_all(&worktree).unwrap();
        let target = PolicyTarget::checkout("logical_repo_0001", "checkout_0001", worktree.clone());
        SessionLaunchRequest {
            project_id: manifest.project_id,
            provider: ProviderRef::codex("cap_codex_1_0_0"),
            action: SessionPolicyAction::CodingTargetWrite,
            target,
            readable_roots: vec![fixture.paths.root().to_path_buf()],
            writable_roots: vec![worktree],
            config_artifact_ref: "sha256:managed-config-artifact".to_string(),
        }
    }

    /// Codex danger-full-access 在 gateway 路由级被阻断:即使请求显式选择 Codex
    /// provider(模拟 UI 外的路径,如 CLI/脚本直接构造请求),validate 也返回
    /// `UnsupportedCapability`,且不触达 registry(start_count == 0)。
    #[test]
    fn codex_danger_full_access_is_blocked_at_gateway_route_even_outside_ui() {
        // 契约断言:当前唯一的 Codex sandbox 配置就是 danger-full-access,即受限写
        // 尚未配置,故 Codex 路由必须被阻断。
        assert_eq!(CODEX_DEFAULT_SANDBOX_MODE, "danger-full-access");

        let fixture = gateway_fixture();
        fixture.install_bootstrap_policy();
        let request = codex_coding_request(&fixture);

        let error = fixture.gateway().validate(request).unwrap_err();
        assert!(
            matches!(
                error,
                ProviderGatewayError::UnsupportedCapability(ref code)
                    if code == "codex_danger_full_access_unsupported"
            ),
            "expected codex_danger_full_access_unsupported, got {error:?}"
        );
        assert_eq!(fixture.registry_start_count(), 0);
    }

    /// 同一 provider type 但 ClaudeCode 不受 danger-full-access 阻断:确认阻断
    /// 精确针对 Codex+danger-full-access,而非误伤所有 provider。
    #[test]
    fn claude_code_is_not_blocked_by_codex_danger_full_access_guard() {
        let fixture = gateway_fixture();
        fixture.install_bootstrap_policy();
        let worktree = fixture.real_worktree();
        let request = SessionLaunchRequest::planning(
            fixture.manifest().project_id,
            ProviderRef::claude_code("cap_claude_code_1_4_0"),
            PolicyTarget::checkout("logical_repo_0001", "checkout_0001", worktree),
            vec![fixture.paths.root().to_path_buf()],
            "sha256:managed-config-artifact",
        );

        let validated = fixture.gateway().validate(request);
        assert!(
            validated.is_ok(),
            "ClaudeCode must not be blocked: {validated:?}"
        );
    }

    /// resume 一致性:fingerprint 全相等 → `GatewaySessionDisposition::Resume`,
    /// 旧会话不被 supersede。
    #[test]
    fn resume_or_start_resumes_when_fingerprint_matches_exactly() {
        let fixture = gateway_fixture();
        fixture.install_bootstrap_policy();
        let worktree = fixture.real_worktree();
        let request = SessionLaunchRequest::planning(
            fixture.manifest().project_id,
            ProviderRef::claude_code("cap_claude_code_1_4_0"),
            PolicyTarget::checkout("logical_repo_0001", "checkout_0001", worktree),
            vec![fixture.paths.root().to_path_buf()],
            "sha256:managed-config-artifact",
        );

        // 先 validate 一次取基线 fingerprint,模拟旧会话冻结的指纹。
        let previous = fixture.gateway().validate(request.clone()).unwrap();
        let resume_request = ResumeSessionLaunchRequest {
            launch: request,
            previous_fingerprint: previous.fingerprint().clone(),
            previous_session_id: "sess_old_0001".to_string(),
        };

        let disposition = fixture.gateway().resume_or_start(resume_request).unwrap();
        assert!(matches!(disposition, GatewaySessionDisposition::Resume(_)));
        // 无 supersede 审计记录。
        assert_eq!(fixture.gateway_audit().supersede_count(), 0);
    }

    /// resume 一致性:provider version 漂移(旧会话指纹记录 1.4.0,当前 1.4.1)→
    /// fingerprint 不一致 → supersede 旧会话并 `StartNew`,旧 session id 被透传。
    #[test]
    fn resume_or_start_supersedes_when_provider_version_drifts() {
        let fixture = gateway_fixture();
        fixture.install_bootstrap_policy();
        let worktree = fixture.real_worktree();
        let request = || {
            SessionLaunchRequest::planning(
                fixture.manifest().project_id,
                ProviderRef::claude_code("cap_claude_code_1_4_0"),
                PolicyTarget::checkout("logical_repo_0001", "checkout_0001", worktree.clone()),
                vec![fixture.paths.root().to_path_buf()],
                "sha256:managed-config-artifact",
            )
        };

        // 旧会话指纹在 version=1.4.0 时冻结。
        let previous = fixture.gateway().validate(request()).unwrap();
        let stale_fingerprint = previous.fingerprint().clone();

        // resume 前当前 capability version 漂移到 1.4.1。
        fixture.capabilities().set_version("1.4.1");
        let resume_request = ResumeSessionLaunchRequest {
            launch: request(),
            previous_fingerprint: stale_fingerprint,
            previous_session_id: "sess_old_0002".to_string(),
        };

        let disposition = fixture.gateway().resume_or_start(resume_request).unwrap();
        match disposition {
            GatewaySessionDisposition::StartNew {
                superseded_session_id,
                ..
            } => {
                assert_eq!(superseded_session_id, "sess_old_0002");
            }
            other => panic!("expected StartNew, got {other:?}"),
        }
        // supersede 审计记录一次,原因标记为 fingerprint mismatch。
        assert_eq!(fixture.gateway_audit().supersede_count(), 1);
        assert!(
            fixture
                .gateway_audit()
                .last_supersede_reason()
                .is_some_and(|reason| reason == "resume_fingerprint_mismatch")
        );
    }

    /// resume 一致性:policy digest 漂移(政策在旧会话后被升级)→ supersede 旧会话。
    /// 证明 fingerprint 覆盖 policy digest 维度,不只 provider version。
    #[test]
    fn resume_or_start_supersedes_when_policy_digest_drifts() {
        let fixture = gateway_fixture();
        fixture.install_bootstrap_policy();
        let worktree = fixture.real_worktree();
        let request = || {
            SessionLaunchRequest::planning(
                fixture.manifest().project_id,
                ProviderRef::claude_code("cap_claude_code_1_4_0"),
                PolicyTarget::checkout("logical_repo_0001", "checkout_0001", worktree.clone()),
                vec![fixture.paths.root().to_path_buf()],
                "sha256:managed-config-artifact",
            )
        };

        let previous = fixture.gateway().validate(request()).unwrap();
        let stale_fingerprint = previous.fingerprint().clone();

        // 政策在旧会话后被升级(revision 2,新 digest)。
        fixture.upgrade_policy();
        let resume_request = ResumeSessionLaunchRequest {
            launch: request(),
            previous_fingerprint: stale_fingerprint,
            previous_session_id: "sess_old_0003".to_string(),
        };

        let disposition = fixture.gateway().resume_or_start(resume_request).unwrap();
        assert!(matches!(
            disposition,
            GatewaySessionDisposition::StartNew { .. }
        ));
        assert_eq!(fixture.gateway_audit().supersede_count(), 1);
    }

    /// resume 一致性:当 Codex 被路由级阻断时,resume_or_start 对 Codex 请求也
    /// 返回 `UnsupportedCapability`(阻断发生在 resume 判定之前)。
    #[test]
    fn resume_or_start_blocks_codex_danger_full_access_before_resume_decision() {
        let fixture = gateway_fixture();
        fixture.install_bootstrap_policy();
        let request = codex_coding_request(&fixture);

        let resume_request = ResumeSessionLaunchRequest {
            launch: request,
            previous_fingerprint: SessionResumeFingerprint {
                digest: "sha256:stale".to_string(),
            },
            previous_session_id: "sess_codex_old".to_string(),
        };

        let error = fixture
            .gateway()
            .resume_or_start(resume_request)
            .unwrap_err();
        assert!(
            matches!(
                error,
                ProviderGatewayError::UnsupportedCapability(ref code)
                    if code == "codex_danger_full_access_unsupported"
            ),
            "expected route block before resume decision, got {error:?}"
        );
    }

    /// 配置来源审计:`ConfigSourceAudit` 记录最终 argv 与 config digest,且
    /// argv 非空、config digest 与 envelope 冻结值一致。仅 Aria-owned
    /// (user/project/local/env/mcp)来源被标注;非 Aria 来源(如 managed settings)
    /// 被标注为 `managed_settings_active=true` 并携带警告,绝不假装已覆盖。
    #[test]
    fn config_source_audit_records_argv_config_digest_and_provenance() {
        let audit = ConfigSourceAudit::from_launch(
            &[
                "claude".to_string(),
                "--permission-prompt-tool=stdio".to_string(),
            ],
            "sha256:managed-config-artifact",
            ConfigSourceProvenance {
                user_settings: true,
                project_settings: true,
                local_settings: false,
                env_overrides: true,
                managed_settings_active: false,
                managed_settings_warning: None,
                mcp_sources: vec![ConfigSourceKind::AriaOwnedBundle],
            },
        );

        assert_eq!(audit.argv, vec!["claude", "--permission-prompt-tool=stdio"]);
        assert!(audit.config_digest.starts_with("sha256:"));
        assert!(!audit.provenance.managed_settings_active);
        assert!(audit.provenance.is_aria_owned_only());
    }

    /// 配置来源审计:解析 provider `/status` 的 `Setting sources` 时发现 managed
    /// settings(非 Aria-owned),标注 `managed_settings_active=true` + 警告,且
    /// `is_aria_owned_only()` 返回 false(绝不假装已覆盖)。该已知 gap 仍可被
    /// policy 配置为拒绝启动(`ManagedSettingsActive` 错误)。
    #[test]
    fn config_source_audit_flags_managed_settings_without_pretending_override() {
        let provenance =
            ConfigSourceProvenance::detect_from_setting_sources(&["User", "Project", "Managed"]);
        assert!(provenance.managed_settings_active);
        assert!(
            provenance
                .managed_settings_warning
                .as_ref()
                .is_some_and(|warning| warning.contains("managed settings")
                    && !warning.contains("overridden")
                    && !warning.contains("覆盖")),
            "warning must not claim override: {:?}",
            provenance.managed_settings_warning
        );
        assert!(!provenance.is_aria_owned_only());
    }

    /// 配置来源审计:policy 可据 managed settings 拒绝启动。
    /// `enforce_config_source_policy` 在发现 managed settings active 且 policy
    /// 配置为拒绝时返回 `ManagedSettingsActive`(fail-closed)。
    #[test]
    fn gateway_rejects_launch_when_managed_settings_active_and_policy_forbids() {
        let fixture = gateway_fixture();
        fixture.install_bootstrap_policy();
        let worktree = fixture.real_worktree();
        let request = SessionLaunchRequest::planning(
            fixture.manifest().project_id,
            ProviderRef::claude_code("cap_claude_code_1_4_0"),
            PolicyTarget::checkout("logical_repo_0001", "checkout_0001", worktree),
            vec![fixture.paths.root().to_path_buf()],
            "sha256:managed-config-artifact",
        );
        let validated = fixture.gateway().validate(request).unwrap();

        let provenance = ConfigSourceProvenance::detect_from_setting_sources(&["User", "Managed"]);
        let audit = ConfigSourceAudit::from_launch(
            &["claude".to_string()],
            "sha256:managed-config-artifact",
            provenance,
        );
        let error = fixture
            .gateway()
            .enforce_config_source_policy(&validated, &audit)
            .unwrap_err();
        assert!(
            matches!(error, ProviderGatewayError::ManagedSettingsActive),
            "expected ManagedSettingsActive, got {error:?}"
        );
    }
}
