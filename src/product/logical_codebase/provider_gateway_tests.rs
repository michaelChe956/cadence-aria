use super::*;
use crate::cross_cutting::session_launch::{
    ValidatedAdapterInput, ValidatedStreamingProviderInput,
};
use crate::product::app_paths::ProductAppPaths;
use crate::product::logical_codebase::policy::PolicyTarget;
use crate::product::logical_codebase::store::LogicalCodebaseManifest;
use crate::protocol::contracts::AdapterInput;

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

/// 测试用 capability source:version 与 resume/write 分格可调,以驱动 spawn
/// 前复验指纹漂移与 resume/write-boundary fail-closed(Task 2b 分格门)。
struct StaticCapabilitySource {
    version: std::sync::Mutex<String>,
    resume_cell: std::sync::Mutex<ProviderCapabilityEvidence>,
    write_boundary_cell: std::sync::Mutex<ProviderCapabilityEvidence>,
}

impl StaticCapabilitySource {
    fn new(version: &str) -> Self {
        Self {
            version: std::sync::Mutex::new(version.to_string()),
            resume_cell: std::sync::Mutex::new(ProviderCapabilityEvidence::Confirmed),
            write_boundary_cell: std::sync::Mutex::new(ProviderCapabilityEvidence::Confirmed),
        }
    }

    fn set_version(&self, version: &str) {
        *self.version.lock().unwrap() = version.to_string();
    }

    /// 直接设置三态 resume 证据(Task 9a 显式 resume Unknown 决策测试;
    /// parking_lot 非本 crate 依赖,沿用本文件 std Mutex 约定)。
    fn set_resume_evidence_state(&self, state: ProviderCapabilityEvidence) {
        *self.resume_cell.lock().unwrap() = state;
    }
    fn set_write_boundary_cell(&self, evidence: ProviderCapabilityEvidence) {
        *self.write_boundary_cell.lock().unwrap() = evidence;
    }

    fn capability(
        &self,
        provider: &ProviderRef,
        action: SessionPolicyAction,
    ) -> ProviderCapability {
        let version = self.version.lock().unwrap().clone();
        let resume = self.resume_cell.lock().unwrap().clone();
        let write_boundary = self.write_boundary_cell.lock().unwrap().clone();
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
            version,
            adapter_dialect,
            wire_dialect,
            capability_snapshot_ref: provider.capability_snapshot_ref.clone(),
            action_capability: ProviderActionCapability {
                action,
                launch: ProviderCapabilityEvidence::Confirmed,
                resume,
                write_boundary,
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
        action: SessionPolicyAction,
    ) -> Result<ProviderCapability, ProviderGatewayError> {
        Ok(self.capability(provider, action))
    }

    fn require_resume_supported(
        &self,
        provider: &ProviderRef,
        action: SessionPolicyAction,
    ) -> Result<ProviderCapability, ProviderGatewayError> {
        let capability = self.capability(provider, action);
        if capability.action_capability.resume != ProviderCapabilityEvidence::Confirmed {
            return Err(ProviderGatewayError::ResumeNotSupported);
        }
        Ok(capability)
    }

    fn require_write_boundary(
        &self,
        provider: &ProviderRef,
        action: SessionPolicyAction,
    ) -> Result<ProviderCapability, ProviderGatewayError> {
        let capability = self.capability(provider, action);
        if capability.action_capability.write_boundary != ProviderCapabilityEvidence::Confirmed {
            return Err(ProviderGatewayError::UnsupportedCapability(
                PROVIDER_CAPABILITY_WRITE_BOUNDARY_NOT_CONFIRMED.to_string(),
            ));
        }
        Ok(capability)
    }

    fn require_root_recipe_supported(
        &self,
        provider: &ProviderRef,
        _credential: &crate::product::logical_codebase::provider_admission_preflight::BootstrapPhaseCredential,
    ) -> Result<ProviderCapability, ProviderGatewayError> {
        // 测试 double:固定 Claude recipe 事实的基础形态(durable 重核验归
        // StoreBacked source 的 for_lc 通道,由 admission 侧测试覆盖)。
        Ok(self.capability(provider, SessionPolicyAction::PlanningReadOnly))
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
        Ok(crate::cross_cutting::streaming_provider::ProviderSession {
            events,
            commands,
            native_session_id: None,
        })
    }

    /// Task 7 分流收口:gateway `start_streaming` 只走 validated trait,
    /// 计数并入同一 `start_count`(「复验失败不触达 registry」断言不变)。
    async fn start_validated(
        &self,
        _input: crate::cross_cutting::session_launch::ValidatedStreamingProviderInput,
        _cancel: tokio_util::sync::CancellationToken,
    ) -> Result<
        crate::cross_cutting::streaming_provider::ProviderSession,
        crate::cross_cutting::provider_adapter::ProviderAdapterError,
    > {
        self.start_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let (_event_tx, events) = tokio::sync::mpsc::channel(1);
        let (commands, _command_rx) = tokio::sync::mpsc::channel(1);
        Ok(crate::cross_cutting::streaming_provider::ProviderSession {
            events,
            commands,
            native_session_id: None,
        })
    }
}

/// 测试用 streaming adapter:记录每次 validated start 收到的 tool_policy
/// (r61 kimi 例外映射对照断言用),返回最小空会话。
struct RecordingStreamingAdapter {
    observed_tool_policies:
        std::sync::Mutex<Vec<Option<crate::cross_cutting::streaming_provider::ProviderToolPolicy>>>,
}

impl Default for RecordingStreamingAdapter {
    fn default() -> Self {
        Self {
            observed_tool_policies: std::sync::Mutex::new(Vec::new()),
        }
    }
}

impl RecordingStreamingAdapter {
    fn observed_tool_policies(
        &self,
    ) -> Vec<Option<crate::cross_cutting::streaming_provider::ProviderToolPolicy>> {
        self.observed_tool_policies.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl crate::cross_cutting::streaming_provider::StreamingProviderAdapter
    for RecordingStreamingAdapter
{
    async fn start(
        &self,
        _input: crate::cross_cutting::streaming_provider::StreamingProviderInput,
        _cancel: tokio_util::sync::CancellationToken,
    ) -> Result<
        crate::cross_cutting::streaming_provider::ProviderSession,
        crate::cross_cutting::provider_adapter::ProviderAdapterError,
    > {
        let (_event_tx, events) = tokio::sync::mpsc::channel(1);
        let (commands, _command_rx) = tokio::sync::mpsc::channel(1);
        Ok(crate::cross_cutting::streaming_provider::ProviderSession {
            events,
            commands,
            native_session_id: None,
        })
    }

    async fn start_validated(
        &self,
        input: crate::cross_cutting::session_launch::ValidatedStreamingProviderInput,
        _cancel: tokio_util::sync::CancellationToken,
    ) -> Result<
        crate::cross_cutting::streaming_provider::ProviderSession,
        crate::cross_cutting::provider_adapter::ProviderAdapterError,
    > {
        let (input, _launch) = input.into_parts();
        self.observed_tool_policies
            .lock()
            .unwrap()
            .push(input.tool_policy.clone());
        let (_event_tx, events) = tokio::sync::mpsc::channel(1);
        let (commands, _command_rx) = tokio::sync::mpsc::channel(1);
        Ok(crate::cross_cutting::streaming_provider::ProviderSession {
            events,
            commands,
            native_session_id: None,
        })
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
        stub_sync_output()
    }

    /// Task 7 分流收口:gateway `run_sync` 只走 validated trait。
    fn run_validated(
        &self,
        _launch: crate::cross_cutting::session_launch::ValidatedAdapterInput,
    ) -> Result<
        crate::protocol::contracts::AdapterOutput,
        crate::cross_cutting::provider_adapter::ProviderAdapterError,
    > {
        stub_sync_output()
    }
}

fn stub_sync_output() -> Result<
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
        providers: [
            ProviderName::ClaudeCode,
            ProviderName::Codex,
            ProviderName::Pi,
            ProviderName::KimiCode,
        ]
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
        // 权威根 = manifest.provider_context_root(temp dir),构造时 canonicalize。
        let authority_root = std::fs::canonicalize(self.manifest().provider_context_root)
            .expect("fixture provider context root exists");
        LogicalCodebaseProviderGateway::with_audit(
            self.policy_store(),
            self.capabilities.clone(),
            self.targets.clone(),
            Arc::new(registry),
            self.sync_adapter.clone(),
            self.gate.clone(),
            self.audit.clone(),
            authority_root,
        )
    }

    /// r61 kimi 例外映射测试构形:ClaudeCode/KimiCode 注册到同一 recording
    /// adapter,支撑「kimi 收 None、claude 对照收 Some(deny)」的对照断言。
    fn gateway_with_recording_streaming_adapter(
        &self,
        adapter: Arc<RecordingStreamingAdapter>,
    ) -> LogicalCodebaseProviderGateway {
        let mut registry = ProviderRegistry::new();
        registry.register(ProviderName::ClaudeCode, adapter.clone());
        registry.register(ProviderName::KimiCode, adapter);
        // 权威根 = manifest.provider_context_root(temp dir),构造时 canonicalize。
        let authority_root = std::fs::canonicalize(self.manifest().provider_context_root)
            .expect("fixture provider context root exists");
        LogicalCodebaseProviderGateway::with_audit(
            self.policy_store(),
            self.capabilities.clone(),
            self.targets.clone(),
            Arc::new(registry),
            self.sync_adapter.clone(),
            self.gate.clone(),
            self.audit.clone(),
            authority_root,
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
            working_directory: worktree.clone(),
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
            working_directory: None,
            baseline_tree: None,
            tool_policy: None,
            audit_sink: None,
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
            working_directory: target.worktree.clone(),
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
            env_vars: Default::default(),
            working_directory: None,
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

#[path = "provider_gateway_tests/anchor_t09_prepare_contract.rs"]
mod anchor_t09_prepare_contract;
#[path = "provider_gateway_tests/anchor_t10_convergence.rs"]
mod anchor_t10_convergence;
#[path = "provider_gateway_tests/anchor_t10_support.rs"]
pub(crate) mod anchor_t10_support;
#[path = "provider_gateway_tests/b2t2_trusted_execution.rs"]
mod b2t2_trusted_execution;
#[path = "provider_gateway_tests/coverage_tests.rs"]
mod coverage_tests;
#[path = "provider_gateway_tests/gateway_contract_tests.rs"]
mod gateway_contract_tests;
#[path = "provider_gateway_tests/lcg_t01_provider_mapping.rs"]
mod lcg_t01_provider_mapping;
#[path = "provider_gateway_tests/lcg_t09a_resume_fingerprint.rs"]
mod lcg_t09a_resume_fingerprint;
#[cfg(test)]
#[path = "provider_gateway_tests/published_policy_body_authority_tests.rs"]
mod published_policy_body_authority_tests;
#[path = "provider_gateway_tests/t10_dev01_dual_stage.rs"]
mod t10_dev01_dual_stage;
#[path = "provider_gateway_tests/task13_gateway_hardening.rs"]
mod task13_gateway_hardening;
#[path = "provider_gateway_tests/task28_gateway_root_authority.rs"]
mod task28_gateway_root_authority;
