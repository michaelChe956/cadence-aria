use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Map, Value};
use tokio::io::AsyncBufReadExt;
use tokio::sync::{Mutex, mpsc};
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::streaming_provider::{
    ProviderEvent, ProviderPermissionMode, StreamingProviderInput,
};
use crate::protocol::contracts::{AdapterInput, AdapterRole, ProviderType};

use super::ClaudeCodeProvider;

mod args;
mod ask_user_question;
mod live_claude;
mod permissions;
mod policy_session;
mod process;
mod streaming;
mod version_probe;

const TEST_TIMEOUT: Duration = Duration::from_secs(15);

fn executable_fixture(relative_path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(relative_path)
}

#[test]
fn executable_fixture_uses_unmodified_checked_in_script() {
    let relative_path = "tests/fixtures/provider/claude_ask_user_question_fixture.sh";
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative_path);
    let before = std::fs::metadata(&source).expect("read source fixture metadata");

    let fixture = executable_fixture(relative_path);

    let after = std::fs::metadata(&source).expect("read fixture metadata after lookup");
    assert_eq!(fixture, source);
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        assert_ne!(after.mode() & 0o111, 0);
        assert_eq!(before.ctime(), after.ctime());
        assert_eq!(before.ctime_nsec(), after.ctime_nsec());
    }
}
fn streaming_input(
    provider_type: ProviderType,
    permission_mode: ProviderPermissionMode,
) -> StreamingProviderInput {
    // 非策略 legacy 路径 fixture：守卫（Task 3.1）要求非策略会话使用非策略角色，
    // 故测试 helper 用 Executor（Coder/聚合初始化同侧），与生产行为一致。
    StreamingProviderInput {
        working_directory: None,
        baseline_tree: None,
        tool_policy: None,
        audit_sink: None,
        provider_type,
        role: AdapterRole::Executor,
        prompt: "Run the fixture provider".to_string(),
        working_dir: std::env::current_dir().unwrap(),
        workspace_session_id: None,
        resume_provider_session_id: None,
        permission_mode,
        structured_output_contract: None,
        env_vars: BTreeMap::new(),
        timeout_secs: 60,
    }
}
async fn recv_completed(events: &mut mpsc::Receiver<ProviderEvent>) -> String {
    loop {
        match tokio::time::timeout(TEST_TIMEOUT, events.recv())
            .await
            .expect("provider should emit completion")
            .expect("provider event channel should stay open")
        {
            ProviderEvent::Completed(completion) => return completion.full_output,
            ProviderEvent::StatusChanged(_)
            | ProviderEvent::Execution(_)
            | ProviderEvent::TextDelta { .. }
            | ProviderEvent::PermissionRequest(_)
            | ProviderEvent::ChoiceRequest(_)
            | ProviderEvent::ToolCall(_)
            | ProviderEvent::ToolResult(_) => {}
            ProviderEvent::Failed { message } => panic!("provider failed: {message}"),
            ProviderEvent::ProtocolError { message, .. } => {
                panic!("provider protocol error: {message}")
            }
            ProviderEvent::UsageReport(_)
            | ProviderEvent::ToolPolicyDecision(_)
            | ProviderEvent::ToolPolicyWarning(_)
            | ProviderEvent::ToolPolicyTerminated(_) => {}
            ProviderEvent::PermissionTimeout { permission_id } => {
                panic!("provider permission timed out: {permission_id}")
            }
        }
    }
}
async fn wait_for_receiver_closed<T>(rx: &mpsc::Receiver<T>) {
    for _ in 0..poll_attempts_for_test_timeout() {
        if rx.is_closed() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("receiver did not close after cancellation");
}
async fn wait_for_buffer_len<T>(rx: &mpsc::Receiver<T>, expected_len: usize) {
    for _ in 0..poll_attempts_for_test_timeout() {
        if rx.len() >= expected_len {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!(
        "receiver buffer did not reach {expected_len} items; actual len is {}",
        rx.len()
    );
}
async fn wait_for_file(path: &Path) {
    for _ in 0..poll_attempts_for_test_timeout() {
        if path.exists() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("file did not appear: {}", path.display());
}

fn poll_attempts_for_test_timeout() -> usize {
    (TEST_TIMEOUT.as_millis() / 5).max(1) as usize
}
#[cfg(target_os = "linux")]
async fn wait_for_process_absent(pid: u32) {
    let proc_path = PathBuf::from(format!("/proc/{pid}"));
    for _ in 0..200 {
        if !proc_path.exists() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("process {pid} was not reaped after cancellation");
}
fn adapter_input(prompt: &str) -> AdapterInput {
    // 非策略 legacy 直连 fixture（Task 3.1/3.2）：策略角色经 bridge 会派生策略并
    // 要求 durable sink；bridge 机制测试使用 Executor 保持非策略路径。
    AdapterInput {
        working_directory: None,
        provider_type: ProviderType::ClaudeCode,
        role: AdapterRole::Executor,
        worktree_path: Some(
            std::env::current_dir()
                .unwrap()
                .to_string_lossy()
                .to_string(),
        ),
        provider_stream_log_dir: None,
        prompt: prompt.to_string(),
        context_files: Vec::new(),
        output_schema: String::new(),
        timeout: 60,
        max_retries: 0,
    }
}
fn write_fixture(relative_path: &str, body: &str) -> PathBuf {
    let path = tempfile::tempdir()
        .expect("fixture dir")
        .keep()
        .join(relative_path);
    std::fs::write(&path, body).expect("write fixture");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = std::fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&path, permissions).expect("chmod fixture");
    }
    path
}
async fn capture_tool_control_response(approved: bool, reason: Option<String>) -> Value {
    let mut child = tokio::process::Command::new("cat")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn cat fixture");
    let stdin = Arc::new(Mutex::new(child.stdin.take().expect("child stdin")));
    let stdout = child.stdout.take().expect("child stdout");

    ClaudeCodeProvider::write_control_response(&stdin, "perm_req_001", approved, reason)
        .await
        .expect("write control response");
    drop(stdin);

    let mut lines = tokio::io::BufReader::new(stdout).lines();
    let line = tokio::time::timeout(TEST_TIMEOUT, lines.next_line())
        .await
        .expect("control response line timeout")
        .expect("read control response line")
        .expect("control response line");
    let _ = tokio::time::timeout(TEST_TIMEOUT, child.wait())
        .await
        .expect("cat wait timeout")
        .expect("cat status");
    serde_json::from_str(&line).expect("control response json")
}
async fn capture_choice_control_response(
    original_input: Value,
    answers: Map<String, Value>,
) -> Value {
    let mut child = tokio::process::Command::new("cat")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn cat fixture");
    let stdin = Arc::new(Mutex::new(child.stdin.take().expect("child stdin")));
    let stdout = child.stdout.take().expect("child stdout");

    ClaudeCodeProvider::write_choice_control_response(
        &stdin,
        "ask_req_001",
        &original_input,
        answers,
    )
    .await
    .expect("write choice control response");
    drop(stdin);

    let mut lines = tokio::io::BufReader::new(stdout).lines();
    let line = tokio::time::timeout(TEST_TIMEOUT, lines.next_line())
        .await
        .expect("choice control response line timeout")
        .expect("read choice control response line")
        .expect("choice control response line");
    let _ = tokio::time::timeout(TEST_TIMEOUT, child.wait())
        .await
        .expect("cat wait timeout")
        .expect("cat status");
    serde_json::from_str(&line).expect("choice control response json")
}

// ==== Task 4a:LC validated launch 测试 fixture ====
// 经真实 `LogicalCodebaseProviderGateway::validate` 链产出
// `ValidatedStreamingProviderInput`(envelope 由 bootstrap 政策冻结),
// 供 `start_validated` 的 LC 审计/原生会话测试消费。

use crate::cross_cutting::provider_availability_gate::ProviderAvailabilityGate;
use crate::cross_cutting::provider_registry::ProviderRegistry;
use crate::cross_cutting::session_launch::ValidatedStreamingProviderInput;
use crate::cross_cutting::streaming_provider::StreamingProviderAdapter;
use crate::cross_cutting::tool_policy_audit::ToolPolicyAuditSink;
use crate::product::app_paths::ProductAppPaths;
use crate::product::logical_codebase::policy::{
    AggregatePolicyArtifactStore, PolicyTarget, ProviderDialect, ProviderWireDialect,
    SessionPolicyAction,
};
use crate::product::logical_codebase::provider_capability_store::ProviderActionCapability;
use crate::product::logical_codebase::provider_gateway::{
    GatewayRunAudit, LogicalCodebaseProviderGateway, PolicyTargetResolver, ProviderCapability,
    ProviderCapabilitySource, ProviderGatewayError, ProviderRef, ProviderRefType,
    SessionLaunchRequest,
};
use crate::product::logical_codebase::store::LogicalCodebaseManifest;
use crate::product::models::ProviderName;
use crate::protocol::contracts::{AdapterOutput, TimeoutStatus};

/// LC fixture 的 capability 源:仅返回 Claude Code 的已实测快照(三格
/// Confirmed 的 fixture 事实;真实 probe/evidence 归 6c/2d)。
struct LcStaticCapabilitySource;

fn lc_claude_capability() -> ProviderCapability {
    ProviderCapability {
        provider_type: ProviderRefType::ClaudeCode,
        version: "claude 2.0.4-lc-fixture".to_string(),
        adapter_dialect: ProviderDialect::ClaudeCodeCliV1,
        wire_dialect: ProviderWireDialect::ClaudeCodeStreamJson,
        capability_snapshot_ref: "cap_claude_code_lc_fixture".to_string(),
        action_capability: ProviderActionCapability {
            action: SessionPolicyAction::CodingTargetWrite,
            launch:
                crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence::Confirmed,
            resume:
                crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence::Confirmed,
            write_boundary:
                crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence::Confirmed,
            projection_digest: String::new(),
            evidence_ref: String::new(),
        },
        trust: crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence::Confirmed,
    }
}

impl ProviderCapabilitySource for LcStaticCapabilitySource {
    fn require_supported(
        &self,
        _provider: &ProviderRef,
        _action: SessionPolicyAction,
    ) -> Result<ProviderCapability, ProviderGatewayError> {
        Ok(lc_claude_capability())
    }

    fn require_resume_supported(
        &self,
        _provider: &ProviderRef,
        _action: SessionPolicyAction,
    ) -> Result<ProviderCapability, ProviderGatewayError> {
        Ok(lc_claude_capability())
    }

    fn require_write_boundary(
        &self,
        _provider: &ProviderRef,
        _action: SessionPolicyAction,
    ) -> Result<ProviderCapability, ProviderGatewayError> {
        Ok(lc_claude_capability())
    }

    fn require_root_recipe_supported(
        &self,
        _provider: &ProviderRef,
        _credential: &crate::product::logical_codebase::provider_admission_preflight::BootstrapPhaseCredential,
    ) -> Result<ProviderCapability, ProviderGatewayError> {
        Err(ProviderGatewayError::UnsupportedCapability(
            "lc fixture has no root recipe facts".to_string(),
        ))
    }
}

/// LC fixture 的 target resolver:直接返回请求冻结的 target(validate 链
/// 的 target 复验语义由 gateway 域测试覆盖)。
struct LcTargetResolver;

impl PolicyTargetResolver for LcTargetResolver {
    fn resolve_and_revalidate(
        &self,
        request: &SessionLaunchRequest,
    ) -> Result<PolicyTarget, ProviderGatewayError> {
        Ok(request.target.clone())
    }
}

/// 占位 streaming adapter:validate 不触 registry,仅为 gateway 构造提供槽位。
struct LcNoopStreamingAdapter;

#[async_trait::async_trait]
impl StreamingProviderAdapter for LcNoopStreamingAdapter {}

/// 占位 sync adapter:gateway 构造参数,LC validated 测试不调用。
struct LcNoopSyncAdapter;

impl crate::cross_cutting::provider_adapter::ProviderAdapter for LcNoopSyncAdapter {
    fn run(
        &self,
        _input: &crate::protocol::contracts::AdapterInput,
    ) -> Result<AdapterOutput, crate::cross_cutting::provider_adapter::ProviderAdapterError> {
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

fn lc_available_gate() -> Arc<ProviderAvailabilityGate> {
    use crate::cross_cutting::provider_availability_gate::ProviderHealthSource;
    use crate::cross_cutting::provider_health::{ProviderHealthEntry, ProviderHealthSnapshot};
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

struct LcLaunchFixture {
    _root: tempfile::TempDir,
    paths: ProductAppPaths,
    audit: Arc<GatewayRunAudit>,
}

impl LcLaunchFixture {
    fn new() -> Self {
        let root = tempfile::tempdir().expect("lc fixture root");
        let paths = ProductAppPaths::new(root.path());
        let manifest =
            LogicalCodebaseManifest::new("project_0001", root.path().to_path_buf(), vec![]);
        AggregatePolicyArtifactStore::new(paths.clone())
            .ensure_bootstrap(&manifest)
            .expect("install lc bootstrap policy");
        let fixture = Self {
            _root: root,
            paths,
            audit: Arc::new(GatewayRunAudit::new()),
        };
        fixture.target_worktree();
        fixture
    }

    /// canonical LC root(= manifest provider_context_root)。
    fn canonical_root(&self) -> PathBuf {
        std::fs::canonicalize(self.paths.root()).expect("lc fixture root exists")
    }

    /// 唯一可写 target(成员 worktree;与 canonical root 分离)。
    fn target_worktree(&self) -> PathBuf {
        let worktree = self.paths.root().join("member-worktree");
        std::fs::create_dir_all(&worktree).expect("create member worktree");
        worktree
    }

    fn gateway(&self) -> LogicalCodebaseProviderGateway {
        let mut registry = ProviderRegistry::new();
        registry.register(ProviderName::ClaudeCode, Arc::new(LcNoopStreamingAdapter));
        LogicalCodebaseProviderGateway::with_audit(
            AggregatePolicyArtifactStore::new(self.paths.clone()),
            Arc::new(LcStaticCapabilitySource),
            Arc::new(LcTargetResolver),
            Arc::new(registry),
            Arc::new(LcNoopSyncAdapter),
            lc_available_gate(),
            self.audit.clone(),
            self.canonical_root(),
        )
    }

    /// LC coding 请求:cwd=canonical root,target=成员 worktree,恰一个
    /// 可写 root=target(read-only 语义由 planning 请求变体覆盖)。
    fn coding_request(&self) -> SessionLaunchRequest {
        let manifest =
            LogicalCodebaseManifest::new("project_0001", self.paths.root().to_path_buf(), vec![]);
        let worktree = self.target_worktree();
        SessionLaunchRequest {
            project_id: manifest.project_id,
            provider: ProviderRef::claude_code("cap_claude_code_lc_fixture"),
            action: SessionPolicyAction::CodingTargetWrite,
            target: PolicyTarget::checkout("logical_repo_0001", "checkout_0001", worktree.clone()),
            working_directory: self.canonical_root(),
            readable_roots: vec![self.canonical_root()],
            writable_roots: vec![worktree],
            config_artifact_ref: "sha256:managed-config-artifact".to_string(),
        }
    }

    /// LC streaming input(Executor/Coder 形态默认无通用 tool policy)。
    fn lc_streaming_input(
        &self,
        role: AdapterRole,
        tool_policy: Option<crate::cross_cutting::streaming_provider::ProviderToolPolicy>,
        audit_sink: Option<Arc<dyn ToolPolicyAuditSink>>,
        resume_id: Option<String>,
    ) -> StreamingProviderInput {
        StreamingProviderInput {
            working_directory: Some(self.canonical_root()),
            baseline_tree: None,
            tool_policy,
            audit_sink,
            provider_type: ProviderType::ClaudeCode,
            role,
            prompt: "Run the LC fixture provider".to_string(),
            working_dir: self.target_worktree(),
            workspace_session_id: Some("ws-lc-fixture-1".to_string()),
            resume_provider_session_id: resume_id,
            permission_mode: crate::cross_cutting::streaming_provider::ProviderPermissionMode::Auto,
            structured_output_contract: None,
            env_vars: BTreeMap::new(),
            timeout_secs: 60,
        }
    }

    /// 经真实 gateway validate 产出 coding validated input。
    fn validated_coding_input(
        &self,
        raw: StreamingProviderInput,
    ) -> ValidatedStreamingProviderInput {
        let validated = self
            .gateway()
            .validate(self.coding_request())
            .expect("lc coding launch validates");
        ValidatedStreamingProviderInput::new(raw, validated)
    }
}
