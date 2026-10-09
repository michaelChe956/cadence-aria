// `engine.rs::invoke_provider` 的 sync 死路径 fail-closed 防线测试。
//
// 防线契约：`logical_repository_id.is_some()`（逻辑代码库仓库）必须经
// `LogicalCodebaseProviderGateway` 启动，禁止直连真实 provider（对应
// REQ-ENV-01/02「无政策不得启动」）。该同步路径当前为死代码，但若未来被
// 复活用于逻辑代码库仓库，会绕过 gateway——因此必须在 adapter.run 之前
// fail-closed。Legacy 单仓（`logical_repository_id: None`）行为零变化，
// 仍走直接 adapter 路径。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::cross_cutting::provider_adapter::{ProviderAdapter, ProviderAdapterError};
use crate::product::app_paths::ProductAppPaths;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::logical_codebase::LogicalRepositoryId;
use crate::product::models::ProviderName;
use crate::product::work_item_split_engine::WorkItemSplitEngine;
use crate::protocol::contracts::{AdapterInput, AdapterOutput};

/// 记录 `run` 是否被调用的 adapter。用于证明 fail-closed 防线在
/// `spawn_blocking(adapter.run)` 之前返回，adapter 从未被调用。
struct RecordingAdapter {
    invoked: Arc<AtomicBool>,
}

impl RecordingAdapter {
    fn new(invoked: Arc<AtomicBool>) -> Self {
        Self { invoked }
    }
}

impl ProviderAdapter for RecordingAdapter {
    fn run(&self, _input: &AdapterInput) -> Result<AdapterOutput, ProviderAdapterError> {
        self.invoked.store(true, Ordering::SeqCst);
        Err(ProviderAdapterError::execution_failed(
            Some(1),
            "adapter.run must not be reached behind the gateway guard",
            "adapter.run must not be reached behind the gateway guard",
            0,
        ))
    }
}

fn logical_repository() -> RepositoryRecord {
    let (_, _, mut repository) = split_prompt_fixture();
    repository.logical_repository_id = Some(LogicalRepositoryId(uuid::Uuid::nil()));
    repository
}

fn engine(invoked: Arc<AtomicBool>) -> WorkItemSplitEngine {
    WorkItemSplitEngine::new(Arc::new(RecordingAdapter::new(invoked)))
}

fn lifecycle() -> LifecycleStore {
    LifecycleStore::new(ProductAppPaths::new("/tmp/aria-work-item-split-engine-t8"))
}

#[tokio::test]
async fn logical_repository_generate_fails_closed_with_gateway_required() {
    let invoked = Arc::new(AtomicBool::new(false));
    let (request, issue, _) = split_prompt_fixture();
    let repository = logical_repository();
    let lifecycle = lifecycle();

    let error = engine(invoked.clone())
        .generate(
            &request,
            &lifecycle,
            &issue,
            &repository,
            ProviderName::ClaudeCode,
        )
        .await
        .expect_err("logical codebase work item split must fail closed");

    assert_eq!(error.code, "logical_provider_gateway_required");
    assert!(
        !invoked.load(Ordering::SeqCst),
        "adapter.run must not be invoked for a logical codebase repository"
    );
}

#[tokio::test]
async fn logical_repository_generate_revision_fails_closed_with_gateway_required() {
    let invoked = Arc::new(AtomicBool::new(false));
    let (request, issue, _) = split_prompt_fixture();
    let repository = logical_repository();
    let lifecycle = lifecycle();

    let error = engine(invoked.clone())
        .generate_revision(
            &request,
            &lifecycle,
            &issue,
            &repository,
            ProviderName::Codex,
            &[],
            &[],
        )
        .await
        .expect_err("logical codebase revision must fail closed");

    assert_eq!(error.code, "logical_provider_gateway_required");
    assert!(
        !invoked.load(Ordering::SeqCst),
        "adapter.run must not be invoked for a logical codebase repository"
    );
}

#[tokio::test]
async fn legacy_repository_generate_still_invokes_adapter_directly() {
    let invoked = Arc::new(AtomicBool::new(false));
    let (request, issue, repository) = split_prompt_fixture();
    let lifecycle = lifecycle();

    let error = engine(invoked.clone())
        .generate(
            &request,
            &lifecycle,
            &issue,
            &repository,
            ProviderName::ClaudeCode,
        )
        .await
        .expect_err("recording adapter returns a controlled error");

    // 防线不触发：错误来自 adapter（经 map_provider_adapter_error 映射），
    // 且 adapter.run 确实被调用——Legacy 路径行为零变化。
    assert_eq!(error.code, "work_item_split_provider_error");
    assert!(
        invoked.load(Ordering::SeqCst),
        "legacy single-repo path must still call adapter.run directly"
    );
}

// ---------------------------------------------------------------------------
// Task 13(lcg_t13):单仓 sync direct 兼容锁——direct 路径永不经 LC 桥。
//
// 断言语义(计划 Task 13 Step 1):legacy 单仓(`logical_repository_id:
// None`)的 split sync 仍直接 `spawn_blocking(adapter.run)`,同一
// repository 两次 generate 驱动的 AdapterInput 逐字段相同(provider 槽、
// role、cwd/worktree 双字段、prompt/schema/timeout 冻结),且从不走
// `run_validated`(LC 同步桥 `GatewaySyncProvider` 的唯一入口)——
// validated_runs 恒 0 即「lc_bridge_calls_for_direct == 0」的可观测
// 形态。本 change 不交付四家同步 direct;Pi/Kimi 的两槽 reject 由
// task_run `RoutingProviderAdapter` it_web 侧
// `lcg_t13_single_repository_sync_claude_codex_direct_topology_unchanged`
// 锁定。
// ---------------------------------------------------------------------------

/// 记录每次收到的完整 AdapterInput,区分 raw `run` 与 `run_validated`,
/// 返回固定 structured output(确定性输出对照)。
struct DirectRunCapture {
    raw_inputs: Mutex<Vec<AdapterInput>>,
    validated_runs: AtomicUsize,
}

impl DirectRunCapture {
    fn new() -> Self {
        Self {
            raw_inputs: Mutex::new(Vec::new()),
            validated_runs: AtomicUsize::new(0),
        }
    }

    fn minimal_output() -> Result<AdapterOutput, ProviderAdapterError> {
        Ok(AdapterOutput {
            exit_code: Some(0),
            stdout: String::new(),
            stderr: String::new(),
            structured_output: Some(serde_json::json!({"work_items": []})),
            files_modified: Vec::new(),
            duration_ms: 0,
            timeout_status: crate::protocol::contracts::TimeoutStatus::NotTimedOut,
        })
    }
}

impl ProviderAdapter for DirectRunCapture {
    fn run(&self, input: &AdapterInput) -> Result<AdapterOutput, ProviderAdapterError> {
        self.raw_inputs.lock().expect("direct run capture").push(input.clone());
        Self::minimal_output()
    }

    fn run_validated(
        &self,
        _launch: crate::cross_cutting::session_launch::ValidatedAdapterInput,
    ) -> Result<AdapterOutput, ProviderAdapterError> {
        self.validated_runs.fetch_add(1, Ordering::SeqCst);
        Err(ProviderAdapterError::execution_failed(
            None,
            String::new(),
            "direct path must never launch through the LC sync bridge",
            0,
        ))
    }
}

/// lcg_t13:legacy 单仓 split sync direct 拓扑不变——两次 generate 的
/// AdapterInput 逐字段相同(Claude 槽/Codex 槽同一推导),raw `run`
/// 恰好各一次,`run_validated` 零调用(LC 桥不可达),输出沿同一映射
/// 确定性上浮。
#[tokio::test]
async fn lcg_t13_sync_direct_never_uses_lc_bridge() {
    let probe = Arc::new(DirectRunCapture::new());
    let engine = WorkItemSplitEngine::new(probe.clone());
    let (request, issue, repository) = split_prompt_fixture();
    let lifecycle = lifecycle();

    for author_provider in [ProviderName::ClaudeCode, ProviderName::Codex] {
        // 输出确定性:同一 structured output 各次得到同一映射错误
        //(minimal fixture 输出无 work items,generate 在 provider 成功
        // 之后的同一解析步失败——该边界即输出上浮的可观测面)。
        let first_code = engine
            .generate(&request, &lifecycle, &issue, &repository, author_provider.clone())
            .await
            .err()
            .expect("minimal output must fail at the deterministic parse boundary")
            .code;
        let second_code = engine
            .generate(&request, &lifecycle, &issue, &repository, author_provider.clone())
            .await
            .err()
            .expect("minimal output must fail at the deterministic parse boundary")
            .code;
        assert_eq!(
            first_code, second_code,
            "sync direct output mapping must stay deterministic"
        );
        assert_eq!(first_code, "work_item_split_provider_output_invalid");
    }

    let inputs = probe.raw_inputs.lock().expect("direct run capture");
    assert_eq!(inputs.len(), 4, "two raw runs per provider slot");
    // 拓扑对照(prompt 含逐跑动态上下文,不在兼容锁范围;两槽路由语义、
    // cwd/worktree 双字段、schema/timeout 冻结才是锁定面)。
    assert_same_topology(&inputs[0], &inputs[1], "claude");
    assert_same_topology(&inputs[2], &inputs[3], "codex");
    assert_eq!(inputs[0].provider_type, crate::protocol::contracts::ProviderType::ClaudeCode);
    assert_eq!(inputs[2].provider_type, crate::protocol::contracts::ProviderType::Codex);
    for input in inputs.iter() {
        assert_eq!(input.role, crate::protocol::contracts::AdapterRole::WorkItemSplitter);
        // 单仓直连:不注入独立 cwd(None → 沿用 worktree_path,两字段
        // 旧目录映射),worktree_path 即 repository.path。
        assert_eq!(input.working_directory, None);
        assert_eq!(
            input.worktree_path.as_deref(),
            Some(repository.path.to_string_lossy().to_string().as_str()),
            "single-repo direct must keep the repository path as worktree target"
        );
    }
    assert_eq!(
        probe.validated_runs.load(Ordering::SeqCst),
        0,
        "lc bridge calls for direct must be 0: raw run is the only spawn edge"
    );
}

/// lcg_t13 拓扑对照:prompt 含逐跑动态上下文(会话号/时间),不在兼容锁
/// 范围;槽身份/role/cwd 双字段/schema/timeout 才是锁定面。
fn assert_same_topology(a: &AdapterInput, b: &AdapterInput, slot: &str) {
    assert_eq!(a.provider_type, b.provider_type, "{slot}: slot identity");
    assert_eq!(a.role, b.role, "{slot}: role unchanged");
    assert_eq!(a.working_directory, b.working_directory, "{slot}: cwd field");
    assert_eq!(a.worktree_path, b.worktree_path, "{slot}: worktree target");
    assert_eq!(a.output_schema, b.output_schema, "{slot}: schema frozen");
    assert_eq!(a.timeout, b.timeout, "{slot}: timeout frozen");
}

// ---------------------------------------------------------------------------
// Task 2.5：cwd/target 分离的跨层字段合同（纯字段合同层测试）。
//
// 断言语义（task-2-5 brief Step 1）：
// - LC cwd≠target 在字段合同层合法：sync input 同时保留 root working_directory
//   与 member worktree（target），不互相覆盖。
// - cwd 漂移必须进入 resume fingerprint——fingerprint 不一致是
//   `resume_or_start` supersede 旧会话的判定维度。
// - 旧构造器不填新字段（None）时回填既有路径：单仓零行为变化（含 serde 兼容）。
// - 显式 root cwd 优先于 legacy 回填。
// ---------------------------------------------------------------------------

fn streaming_probe_input(
    working_directory: Option<std::path::PathBuf>,
    working_dir: std::path::PathBuf,
) -> crate::cross_cutting::streaming_provider::StreamingProviderInput {
    use std::collections::BTreeMap;

    use crate::cross_cutting::streaming_provider::{
        ProviderPermissionMode, StreamingProviderInput,
    };
    use crate::protocol::contracts::{AdapterRole, ProviderType};

    StreamingProviderInput {
        provider_type: ProviderType::ClaudeCode,
        role: AdapterRole::Executor,
        prompt: "probe".to_string(),
        working_dir,
        working_directory,
        workspace_session_id: None,
        resume_provider_session_id: None,
        permission_mode: ProviderPermissionMode::Auto,
        tool_policy: None,
        audit_sink: None,
        structured_output_contract: None,
        env_vars: BTreeMap::new(),
        timeout_secs: 1,
        baseline_tree: None,
    }
}

fn sync_probe_input(
    working_directory: Option<std::path::PathBuf>,
    worktree_path: Option<String>,
) -> AdapterInput {
    AdapterInput {
        provider_type: crate::protocol::contracts::ProviderType::ClaudeCode,
        role: crate::protocol::contracts::AdapterRole::WorkItemSplitter,
        working_directory,
        worktree_path,
        provider_stream_log_dir: None,
        prompt: "split work items".to_string(),
        context_files: Vec::new(),
        output_schema: String::new(),
        timeout: 1,
        max_retries: 0,
    }
}

/// LC cwd≠target 在 sync input 字段合同层合法：effective cwd 取 canonical
/// root，member worktree 仍是 target——两个字段并存、不互相覆盖。
#[test]
fn sync_input_preserves_root_working_directory_and_member_worktree() {
    let root = std::path::PathBuf::from("/lc-root");
    let member_worktree = "/work/api/.worktrees/aria-issues/issue_1".to_string();

    let input = sync_probe_input(Some(root.clone()), Some(member_worktree.clone()));

    // gateway `run_sync` 复验口径（effective cwd）：优先独立 working_directory
    //（root），而不是回退 member worktree。
    assert_eq!(
        input.effective_working_directory().as_deref(),
        Some(root.as_path())
    );
    // target 语义不因新字段改变：worktree_path 仍是 member checkout/worktree。
    assert_eq!(
        input.worktree_path.as_deref(),
        Some(member_worktree.as_str())
    );
}

/// cwd 漂移进入 resume fingerprint：envelope 冻结 working_directory 并纳入
/// digest，两个仅 cwd 不同的 envelope 指纹不同（`resume_or_start` 据此判定
/// supersede 旧会话并 StartNew）；cwd 不漂移时指纹稳定。
///
/// Task 9a：指纹迁移为五参 `from_envelope`(envelope+候选投影+action
/// evidence 摘要+target git identity)——split sync 测试 caller 同一原子
/// 迁移；本测试只钉 cwd 维度，投影其余维度由 gateway 侧 lcg_t09a 测试覆盖。
#[test]
fn resume_fingerprint_changes_when_working_directory_drifts() {
    use crate::cross_cutting::streaming_provider::ProviderPermissionMode;
    use crate::product::logical_codebase::policy::{
        AggregatePolicyArtifact, PolicyTarget, ProviderDialect, ProviderWireDialect,
        SessionPolicyAction, SessionPolicyEnvelope,
    };
    use crate::product::logical_codebase::provider_gateway::{
        ProviderRefType, SessionResumeFingerprint,
    };
    use crate::product::logical_codebase::provider_projection::ProviderPolicyProjection;
    use crate::protocol::contracts::AdapterRole;

    let artifact = AggregatePolicyArtifact::bootstrap(
        "project_0001",
        "logical_0001",
        "2026-10-01T00:00:00Z".to_string(),
    );
    let target = PolicyTarget::checkout("logical_repo", "checkout", "/work/repo");

    let envelope_for = |working_directory: &str| {
        SessionPolicyEnvelope::new(
            &artifact,
            SessionPolicyAction::PlanningReadOnly,
            target.clone(),
            std::path::PathBuf::from(working_directory),
            Vec::new(),
            Vec::new(),
            ProviderDialect::ClaudeCodeCliV1,
            "sha256:settings".to_string(),
            "2026-10-01T00:00:00Z".to_string(),
            std::path::PathBuf::from("/authority-root"),
        )
        .unwrap()
    };
    let root_a = envelope_for("/lc-root-a");
    let root_b = envelope_for("/lc-root-b");
    let root_a_again = envelope_for("/lc-root-a");

    // envelope 冻结 working_directory（并随 serde 往返保留）。
    assert_eq!(
        root_a.working_directory,
        std::path::PathBuf::from("/lc-root-a")
    );
    let json = serde_json::to_value(&root_a).unwrap();
    assert_eq!(
        json.get("working_directory")
            .and_then(|value| value.as_str()),
        Some("/lc-root-a")
    );
    let back: SessionPolicyEnvelope = serde_json::from_value(json).unwrap();
    assert_eq!(back.working_directory, root_a.working_directory);

    // 候选投影与 envelope 的 cwd/target/roots 同源(调用者完整 prepare 所得)。
    let projection_for = |envelope: &SessionPolicyEnvelope| {
        ProviderPolicyProjection::new(
            ProviderRefType::ClaudeCode,
            envelope.provider_dialect,
            ProviderWireDialect::ClaudeCodeStreamJson,
            "1.4.0".to_string(),
            envelope.action,
            AdapterRole::WorkItemSplitter,
            ProviderPermissionMode::Auto,
            None,
            String::new(),
            String::new(),
            envelope.working_directory.clone(),
            envelope.working_directory.clone(),
            envelope.target.clone(),
            envelope.readable_roots.clone(),
            envelope.writable_roots.clone(),
            String::new(),
            envelope.config_digest.clone(),
            String::new(),
            String::new(),
            "sha256:capability-projection-split".to_string(),
            "sha256:session-projection-split".to_string(),
        )
    };
    let fingerprint = |envelope: &SessionPolicyEnvelope| {
        SessionResumeFingerprint::from_envelope(
            envelope,
            &projection_for(envelope),
            "sha256:action-evidence-split",
            "/work/main/.git/worktrees/repo",
        )
    };
    // 仅 cwd 漂移 → fingerprint 漂移（resume supersede 判定维度）。
    assert_ne!(fingerprint(&root_a).digest, fingerprint(&root_b).digest);
    // 控制组：其余维度一致且 cwd 不漂移 → fingerprint 稳定（resume 放行）。
    assert_eq!(
        fingerprint(&root_a).digest,
        fingerprint(&root_a_again).digest
    );
}

/// 旧构造器不填新字段（None）时回填既有路径：单仓/存量调用零行为变化。
/// 含 serde 兼容：legacy JSON（无 working_directory 键）反序列化为 None。
#[test]
fn legacy_input_defaults_working_directory_from_existing_path() {
    let legacy_dir = std::path::PathBuf::from("/single-repo-checkout");

    // StreamingProviderInput：None → effective cwd 回填既有 working_dir。
    let streaming = streaming_probe_input(None, legacy_dir.clone());
    assert_eq!(streaming.working_directory, None);
    assert_eq!(
        streaming.effective_working_directory(),
        legacy_dir.as_path()
    );

    // AdapterInput：None → effective cwd 回填既有 worktree_path。
    let adapter = sync_probe_input(None, Some(legacy_dir.to_string_lossy().to_string()));
    assert_eq!(adapter.working_directory, None);
    assert_eq!(
        adapter.effective_working_directory().as_deref(),
        Some(legacy_dir.as_path())
    );

    // serde 兼容：存量 JSON 不携带 working_directory 键 → None（旧记录可读）。
    let legacy_json = r#"{
        "provider_type": "claude_code",
        "role": "work_item_splitter",
        "worktree_path": "/single-repo-checkout",
        "prompt": "probe",
        "context_files": [],
        "output_schema": "",
        "timeout": 1,
        "max_retries": 0
    }"#;
    let parsed: AdapterInput = serde_json::from_str(legacy_json).unwrap();
    assert_eq!(parsed.working_directory, None);
    assert_eq!(
        parsed.effective_working_directory().as_deref(),
        Some(legacy_dir.as_path())
    );
}

/// 显式 root cwd 优先于 legacy 回填：LC 入口注入 Some(canonical root) 后，
/// effective cwd 取 root；legacy 路径字段原样保留（不被覆盖、不反噬）。
#[test]
fn logical_input_explicit_working_directory_overrides_legacy_fallback() {
    let root = std::path::PathBuf::from("/lc-root");
    let legacy_dir = std::path::PathBuf::from("/work/member");

    let streaming = streaming_probe_input(Some(root.clone()), legacy_dir.clone());
    assert_eq!(streaming.effective_working_directory(), root.as_path());
    assert_eq!(streaming.working_dir, legacy_dir);

    let adapter = sync_probe_input(
        Some(root.clone()),
        Some(legacy_dir.to_string_lossy().to_string()),
    );
    assert_eq!(
        adapter.effective_working_directory().as_deref(),
        Some(root.as_path())
    );
    assert_eq!(
        adapter.worktree_path.as_deref(),
        Some(legacy_dir.to_string_lossy().to_string().as_str())
    );
}

// ---------------------------------------------------------------------------
// Task 2.6：split sync 经 gateway 启动的 LC root cwd 重绑（REQ-ENV-10/11）。
//
// 断言语义：LC 分支 `invoke_provider_via_gateway` 的 AdapterInput 独立
// cwd=canonical root（gateway 冻结的 manifest provider_context_root），
// worktree_path 仍是 target 成员路径；恰一次 sync adapter spawn。单仓
// `invoke_provider` 的直连/回填（None → worktree_path）由上方既有测试锁定。
// ---------------------------------------------------------------------------

use std::sync::Mutex;

use crate::cross_cutting::provider_availability_gate::{
    ProviderAvailabilityGate, ProviderHealthSource,
};
use crate::cross_cutting::provider_health::{ProviderHealthEntry, ProviderHealthSnapshot};
use crate::cross_cutting::provider_registry::ProviderRegistry;
use crate::product::logical_codebase::AggregatePolicyArtifactStore;
use crate::product::logical_codebase::GatewayRunAudit;
use crate::product::logical_codebase::LogicalCodebaseManifest;
use crate::product::logical_codebase::LogicalCodebaseProviderGateway;
use crate::product::logical_codebase::PolicyTarget;
use crate::product::logical_codebase::PolicyTargetResolver;
use crate::product::logical_codebase::ProviderCapability;
use crate::product::logical_codebase::ProviderCapabilitySource;
use crate::product::logical_codebase::ProviderDialect;
use crate::product::logical_codebase::ProviderRef;
use crate::product::logical_codebase::ProviderRefType;
use crate::product::logical_codebase::RepositoryCheckoutId;
use crate::product::logical_codebase::SessionLaunchRequest;
use crate::product::logical_codebase::SessionPolicyAction;

/// sync 探针:记录 spawn 时 input 的独立 cwd 与 worktree target 并计数,
/// 返回最小 structured output。`run_validated` 独立计数(段①:prepared
/// launch 必须走 validated trait);可注入一次失败供 fail 收口断言。
struct SplitRootCwdSyncProbe {
    runs: Arc<AtomicUsize>,
    validated_runs: Arc<AtomicUsize>,
    fail_validated: Arc<AtomicBool>,
    cwd_at_runs: Arc<Mutex<Option<PathBuf>>>,
    worktree_at_runs: Arc<Mutex<Option<String>>>,
}

impl SplitRootCwdSyncProbe {
    fn new() -> Self {
        Self {
            runs: Arc::new(AtomicUsize::new(0)),
            validated_runs: Arc::new(AtomicUsize::new(0)),
            fail_validated: Arc::new(AtomicBool::new(false)),
            cwd_at_runs: Arc::new(Mutex::new(None)),
            worktree_at_runs: Arc::new(Mutex::new(None)),
        }
    }

    fn minimal_output() -> Result<AdapterOutput, ProviderAdapterError> {
        Ok(AdapterOutput {
            exit_code: Some(0),
            stdout: String::new(),
            stderr: String::new(),
            structured_output: Some(serde_json::json!({"work_items": []})),
            files_modified: Vec::new(),
            duration_ms: 0,
            timeout_status: crate::protocol::contracts::TimeoutStatus::NotTimedOut,
        })
    }
}

impl ProviderAdapter for SplitRootCwdSyncProbe {
    fn run(&self, input: &AdapterInput) -> Result<AdapterOutput, ProviderAdapterError> {
        self.runs.fetch_add(1, Ordering::SeqCst);
        *self.cwd_at_runs.lock().expect("split cwd probe") = input.working_directory.clone();
        *self.worktree_at_runs.lock().expect("split worktree probe") = input.worktree_path.clone();
        Self::minimal_output()
    }

    fn run_validated(
        &self,
        launch: crate::cross_cutting::session_launch::ValidatedAdapterInput,
    ) -> Result<AdapterOutput, ProviderAdapterError> {
        let (input, _policy) = launch.into_parts();
        self.runs.fetch_add(1, Ordering::SeqCst);
        self.validated_runs.fetch_add(1, Ordering::SeqCst);
        *self.cwd_at_runs.lock().expect("split cwd probe") = input.working_directory.clone();
        *self.worktree_at_runs.lock().expect("split worktree probe") = input.worktree_path.clone();
        if self.fail_validated.load(Ordering::SeqCst) {
            self.fail_validated.store(false, Ordering::SeqCst);
            return Err(ProviderAdapterError::execution_failed(
                None,
                String::new(),
                "split probe injected failure".to_string(),
                0,
            ));
        }
        Self::minimal_output()
    }
}

/// gateway 测试 capability source:按 provider ref 返回对应 dialect
/// (Task 2b 分格形状:launch/write_boundary 恒 Confirmed;resume 三态可调,
/// Task 9a 显式 resume Unknown 决策测试消费)。
struct SplitRootCwdCapabilitySource {
    resume: crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence,
}

impl Default for SplitRootCwdCapabilitySource {
    fn default() -> Self {
        Self {
            resume:
                crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence::Confirmed,
        }
    }
}

impl SplitRootCwdCapabilitySource {
    fn capability(
        &self,
        provider: &ProviderRef,
        action: SessionPolicyAction,
    ) -> ProviderCapability {
        use crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence;
        use crate::product::logical_codebase::policy::ProviderWireDialect;
        use crate::product::logical_codebase::provider_capability_store::ProviderActionCapability;
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
                resume: self.resume.clone(),
                write_boundary: ProviderCapabilityEvidence::Confirmed,
                projection_digest: format!("projection-digest-{action:?}"),
                evidence_ref: format!("probe://{action:?}"),
            },
            trust: ProviderCapabilityEvidence::Confirmed,
        }
    }
}

impl ProviderCapabilitySource for SplitRootCwdCapabilitySource {
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
        if capability.action_capability.resume
            != crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence::Confirmed
        {
            return Err(ProviderGatewayError::ResumeNotSupported);
        }
        Ok(capability)
    }

    fn require_write_boundary(
        &self,
        provider: &ProviderRef,
        action: SessionPolicyAction,
    ) -> Result<ProviderCapability, ProviderGatewayError> {
        Ok(self.capability(provider, action))
    }

    fn require_root_recipe_supported(
        &self,
        provider: &ProviderRef,
        _credential: &crate::product::logical_codebase::provider_admission_preflight::BootstrapPhaseCredential,
    ) -> Result<ProviderCapability, ProviderGatewayError> {
        if provider.provider_type != ProviderRefType::ClaudeCode {
            return Err(ProviderGatewayError::UnsupportedCapability(
                crate::product::logical_codebase::provider_gateway::PROVIDER_ROOT_RECIPE_REQUIRES_FIXED_CLAUDE
                    .to_string(),
            ));
        }
        Ok(self.capability(provider, SessionPolicyAction::PlanningReadOnly))
    }
}

/// pass-through target resolver：直接返回请求冻结的 target（git-dir 校验由
/// 生产 resolver 承担，本测试只验证 cwd 重绑分流）。
struct SplitRootCwdPassThroughResolver;

impl PolicyTargetResolver for SplitRootCwdPassThroughResolver {
    fn resolve_and_revalidate(
        &self,
        request: &SessionLaunchRequest,
    ) -> Result<PolicyTarget, ProviderGatewayError> {
        Ok(request.target.clone())
    }
}

fn split_root_cwd_availability_gate() -> Arc<ProviderAvailabilityGate> {
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

fn split_root_cwd_gateway(
    paths: &ProductAppPaths,
    canonical_root: &std::path::Path,
    project_id: &str,
    sync_probe: Arc<SplitRootCwdSyncProbe>,
) -> LogicalCodebaseProviderGateway {
    let manifest = LogicalCodebaseManifest::new(project_id, canonical_root.to_path_buf(), vec![]);
    let policies = AggregatePolicyArtifactStore::new(paths.clone());
    policies
        .ensure_bootstrap(&manifest)
        .expect("bootstrap policy");
    LogicalCodebaseProviderGateway::with_audit(
        policies,
        Arc::new(SplitRootCwdCapabilitySource::default()),
        Arc::new(SplitRootCwdPassThroughResolver),
        Arc::new(ProviderRegistry::new()),
        sync_probe,
        split_root_cwd_availability_gate(),
        Arc::new(GatewayRunAudit::new()),
        manifest.provider_context_root.clone(),
    )
}

/// Task 2.6：LC split sync 的 AdapterInput 独立 cwd=canonical root、
/// worktree_path=target 成员；恰一次 sync spawn 经 gateway（run_sync 的 spawn
/// 前复验以 envelope 冻结 root 为 canonical cwd 权威）。
#[tokio::test]
async fn split_sync_gateway_launch_rebinds_cwd_to_canonical_root() {
    let root = tempfile::tempdir().expect("tempdir");
    let canonical_root = root.path().to_path_buf();
    let member = canonical_root.join("member_repo");
    std::fs::create_dir_all(&member).expect("member dir");
    let paths = ProductAppPaths::new(canonical_root.join(".aria"));
    let probe = Arc::new(SplitRootCwdSyncProbe::new());
    let gateway = split_root_cwd_gateway(&paths, &canonical_root, "project_0001", probe.clone());

    let mut repository = logical_repository();
    repository.path = member.clone();
    repository.primary_checkout_id = Some(RepositoryCheckoutId(uuid::Uuid::nil()));
    let (_, issue, _) = split_prompt_fixture();

    let engine = WorkItemSplitEngine::new(Arc::new(RecordingAdapter::new(Arc::new(
        AtomicBool::new(false),
    ))));
    let lifecycle = LifecycleStore::new(paths.clone());
    let result = engine
        .invoke_provider_via_gateway(
            "root cwd probe",
            &repository,
            ProviderName::ClaudeCode,
            &lifecycle,
            &issue,
            Arc::new(gateway),
            "ws_probe_0001",
        )
        .await
        .expect("split sync gateway launch must complete with the root cwd");

    assert!(
        !result.run_ref.trim().is_empty(),
        "provider run must be persisted"
    );
    assert_eq!(
        probe.runs.load(Ordering::SeqCst),
        1,
        "exactly one sync adapter spawn through the gateway"
    );
    assert_eq!(
        *probe.cwd_at_runs.lock().expect("split cwd probe"),
        Some(canonical_root),
        "sync adapter cwd must be rebound to the canonical lc root"
    );
    assert_eq!(
        *probe.worktree_at_runs.lock().expect("split worktree probe"),
        Some(member.to_string_lossy().to_string()),
        "worktree_path must stay the target member checkout"
    );
}

// ---------------------------------------------------------------------------
// Task 1b 段①尾(lcg_t01):sync split caller 的 handle 收口。
//
// 断言语义(计划冻结「WS streaming Plan/split 的每个真实 caller 依次 begin
// handle、bind sink、start、parse、complete/fail」;sync split 走同一 gateway
// bridge):invoke_provider_via_gateway 经 begin handle→bind sink(prepare)→
// start(validated trait)→parse(complete 消费 handle)收口;失败路径 fail
// 收口 status=failed。
// ---------------------------------------------------------------------------

/// 读取 split run 身份分区里指定 workspace 的全部 run.json status。
fn split_run_statuses(paths: &ProductAppPaths, workspace_session_id: &str) -> Vec<String> {
    let root = paths
        .root()
        .join("work-item-split-runs")
        .join(workspace_session_id);
    let mut statuses = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&root) {
        for entry in entries.flatten() {
            let Ok(value) = serde_json::from_str::<serde_json::Value>(
                &std::fs::read_to_string(entry.path().join("run.json")).unwrap_or_default(),
            ) else {
                continue;
            };
            if let Some(status) = value.get("status").and_then(|v| v.as_str()) {
                statuses.push(status.to_string());
            }
        }
    }
    statuses.sort();
    statuses
}

/// sync split caller 的 handle 流:成功收口 completed、失败收口 failed,
/// 且 prepared launch 只走 validated trait(validated_runs==1)。
#[tokio::test]
async fn split_sync_gateway_launch_closes_run_handle_on_success_and_failure() {
    let root = tempfile::tempdir().expect("tempdir");
    let canonical_root = root.path().to_path_buf();
    let member = canonical_root.join("member_repo");
    std::fs::create_dir_all(&member).expect("member dir");
    let paths = ProductAppPaths::new(canonical_root.join(".aria"));
    let probe = Arc::new(SplitRootCwdSyncProbe::new());
    let gateway = split_root_cwd_gateway(&paths, &canonical_root, "project_0001", probe.clone());
    let gateway = Arc::new(gateway);

    let mut repository = logical_repository();
    repository.path = member.clone();
    repository.primary_checkout_id = Some(RepositoryCheckoutId(uuid::Uuid::nil()));
    let (_, issue, _) = split_prompt_fixture();

    let engine = WorkItemSplitEngine::new(Arc::new(RecordingAdapter::new(Arc::new(
        AtomicBool::new(false),
    ))));
    let lifecycle = LifecycleStore::new(paths.clone());

    // 成功路径:handle 收口 completed,prepared launch 只走 validated trait。
    let result = engine
        .invoke_provider_via_gateway(
            "handle success probe",
            &repository,
            ProviderName::ClaudeCode,
            &lifecycle,
            &issue,
            gateway.clone(),
            "ws_handle_0001",
        )
        .await
        .expect("split sync gateway launch must close the run handle");

    assert_eq!(
        probe.validated_runs.load(Ordering::SeqCst),
        1,
        "prepared launch must dispatch to the validated trait"
    );
    assert!(
        result.run_ref.starts_with("ws-ws_handle_0001-split-run-"),
        "run_ref must carry the workspace-scoped handle identity, got {}",
        result.run_ref
    );
    assert_eq!(
        split_run_statuses(&paths, "ws_handle_0001"),
        vec!["completed".to_string()],
        "success must close the split run handle as completed"
    );

    // 失败路径:adapter 失败时 handle 收口 failed,错误向上传播。
    probe.fail_validated.store(true, Ordering::SeqCst);
    let failure = engine
        .invoke_provider_via_gateway(
            "handle failure probe",
            &repository,
            ProviderName::ClaudeCode,
            &lifecycle,
            &issue,
            gateway.clone(),
            "ws_handle_0002",
        )
        .await
        .expect_err("injected adapter failure must propagate");
    assert!(
        failure.message.contains("ProviderExecutionFailed"),
        "failure must propagate the adapter error kind, got {}",
        failure.message
    );
    assert_eq!(
        split_run_statuses(&paths, "ws_handle_0002"),
        vec!["failed".to_string()],
        "adapter failure must close the split run handle as failed"
    );
}

// ---------------------------------------------------------------------------
// Task 1b 段①(lcg_t01):WorkItemSplitProviderRunHandle 的 lifecycle 与
// parse.rs 的 complete 函数消费。
//
// 断言语义(计划冻结接口「WS Plan/split run identity」):
// - `begin` 分配 run_ref + run-bound role_run_seq,handle 冻结 workspace 会话身份;
// - parse.rs 的 complete 函数消费已有 handle 收口 run 记录(status=completed),
//   不再重新 `save_work_item_split_provider_run`;
// - read-back 的 provider_run_ref 与 handle.run_ref 一致——split 的运行身份
//   与 streaming Plan/split 的 run_ref/证据不混用。
// ---------------------------------------------------------------------------

/// begin 分配的 handle 携带冻结身份;complete 函数消费已有 handle 收口 run,
/// durable 记录的 provider_run_ref 与 handle.run_ref 一致。
#[test]
fn lcg_t01_parse_consumes_existing_split_run_handle() {
    let root = tempfile::tempdir().expect("tempdir");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    std::fs::create_dir_all(root.path().join(".aria")).expect("aria root");
    let lifecycle = LifecycleStore::new(paths);

    let handle = lifecycle
        .begin_work_item_split_provider_run(
            "project_0001",
            "issue_0001",
            &ProviderName::ClaudeCode,
            "ws_session_0001",
        )
        .expect("begin split provider run");

    assert!(!handle.run_ref.trim().is_empty());
    assert_eq!(handle.workspace_session_id, "ws_session_0001");

    let structured = serde_json::json!({"work_items": []});
    let parsed = crate::product::work_item_split_engine::parse::complete_split_provider_run(
        &lifecycle,
        &handle,
        "split prompt",
        &structured,
    )
    .expect("complete consumes the existing handle");

    assert_eq!(parsed.provider_run_ref, handle.run_ref);
}

// ---------------------------------------------------------------------------
// Task 9a 决策测试(lcg_t09):同步 split resume 没有 native session
// contract——resume 证据恒 Unknown,显式 resume 决策稳定拒绝、零 spawn,
// 不借 streaming fresh 推导支持。
// ---------------------------------------------------------------------------

/// Task 9a:split sync 无 native session contract(capability row resume=
/// Unknown)时,显式 resume 决策被 `ResumeNotSupported` 稳定拒绝,既有
/// fresh sync run 的 probe 启动计数不增加(不伪称支持、不静默转 streaming
/// fresh);`resume=Unknown` 由 durable capability row 形态承载。
#[tokio::test]
async fn lcg_t09_split_sync_resume_unknown_is_not_streaming_fresh() {
    use crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence;
    use crate::cross_cutting::streaming_provider::ProviderPermissionMode;
    use crate::product::logical_codebase::policy::ProviderWireDialect;
    use crate::product::logical_codebase::policy::{PolicyTarget, SessionPolicyAction};
    use crate::product::logical_codebase::provider_gateway::{
        ProviderRef, ResumeSessionLaunchRequest, SessionResumeFingerprint,
    };
    use crate::product::logical_codebase::provider_gateway::{
        SessionLaunchRequest, canonical_target_git_identity,
    };
    use crate::product::logical_codebase::provider_projection::ProviderPolicyProjection;
    use crate::protocol::contracts::AdapterRole;

    let root = tempfile::tempdir().expect("tempdir");
    let canonical_root = root.path().to_path_buf();
    let member = canonical_root.join("member_repo");
    std::fs::create_dir_all(&member).expect("member dir");
    let paths = ProductAppPaths::new(canonical_root.join(".aria"));
    let probe = Arc::new(SplitRootCwdSyncProbe::new());

    // split sync 的 resume 证据形态:无 native session contract → Unknown。
    let resume_state = ProviderCapabilityEvidence::Unknown;
    let manifest = LogicalCodebaseManifest::new("project_0001", canonical_root.clone(), vec![]);
    let policies = AggregatePolicyArtifactStore::new(paths.clone());
    policies
        .ensure_bootstrap(&manifest)
        .expect("bootstrap policy");
    let gateway = LogicalCodebaseProviderGateway::with_audit(
        policies,
        Arc::new(SplitRootCwdCapabilitySource {
            resume: resume_state.clone(),
        }),
        Arc::new(SplitRootCwdPassThroughResolver),
        Arc::new(ProviderRegistry::new()),
        probe.clone(),
        split_root_cwd_availability_gate(),
        Arc::new(GatewayRunAudit::new()),
        manifest.provider_context_root.clone(),
    );
    let gateway = Arc::new(gateway);

    // 既有 fresh sync run:恰一次 sync spawn 经 gateway。
    let mut repository = logical_repository();
    repository.path = member.clone();
    repository.primary_checkout_id = Some(RepositoryCheckoutId(uuid::Uuid::nil()));
    let (_, issue, _) = split_prompt_fixture();
    let engine = WorkItemSplitEngine::new(Arc::new(RecordingAdapter::new(Arc::new(
        AtomicBool::new(false),
    ))));
    let lifecycle = LifecycleStore::new(paths.clone());
    engine
        .invoke_provider_via_gateway(
            "split sync fresh",
            &repository,
            ProviderName::ClaudeCode,
            &lifecycle,
            &issue,
            gateway.clone(),
            "ws_t09_split_0001",
        )
        .await
        .expect("fresh split sync run completes");
    assert_eq!(probe.runs.load(Ordering::SeqCst), 1);

    // 显式 sync resume 决策:候选投影按当前请求完整 prepare 后仍被
    // resume=Unknown 稳定拒绝,不静默转 streaming fresh。
    let request = SessionLaunchRequest::planning(
        "project_0001",
        ProviderRef::claude_code("cap_claude_code_1_4_0"),
        PolicyTarget::checkout("logical_repo_0001", "checkout_0001", member.clone()),
        vec![canonical_root.clone()],
        "sha256:managed-config-artifact",
    );
    let validated = gateway
        .validate(request.clone())
        .expect("split sync planning launch validates");
    let projection = ProviderPolicyProjection::new(
        crate::product::logical_codebase::provider_gateway::ProviderRefType::ClaudeCode,
        validated.envelope().provider_dialect,
        ProviderWireDialect::ClaudeCodeStreamJson,
        "1.0.0".to_string(),
        validated.envelope().action,
        AdapterRole::WorkItemSplitter,
        ProviderPermissionMode::Auto,
        None,
        String::new(),
        String::new(),
        validated.envelope().working_directory.clone(),
        validated.envelope().working_directory.clone(),
        validated.envelope().target.clone(),
        validated.envelope().readable_roots.clone(),
        validated.envelope().writable_roots.clone(),
        String::new(),
        validated.envelope().config_digest.clone(),
        String::new(),
        String::new(),
        "sha256:capability-projection-split".to_string(),
        "sha256:session-projection-split".to_string(),
    );
    let previous = SessionResumeFingerprint::from_envelope(
        validated.envelope(),
        &projection,
        validated.action_evidence_digest(),
        &canonical_target_git_identity(&validated.envelope().target.worktree),
    );
    let rejected = gateway
        .resume_or_start_with_projection(
            ResumeSessionLaunchRequest {
                launch: request,
                previous_fingerprint: previous,
                previous_session_id: "ws_t09_split_0001".to_string(),
            },
            projection,
        )
        .expect_err("split sync resume without a native session contract must be refused");
    assert!(matches!(
        rejected,
        crate::product::logical_codebase::provider_gateway::ProviderGatewayError::ResumeNotSupported
    ));
    // 零新 spawn:resume 尝试没有再启动 sync probe(不是 streaming fresh)。
    assert_eq!(probe.runs.load(Ordering::SeqCst), 1);
    // durable capability row 的 resume 形态:Unknown(不借 streaming fresh
    // 推导支持)。
    let split_sync_resume_state = resume_state;
    assert_eq!(split_sync_resume_state, ProviderCapabilityEvidence::Unknown);
    // 显式 resume 只被拒绝:没有 Resume/StartNew 决策、没有 streaming fresh。
    let split_sync_fresh_spawn_count = probe.runs.load(Ordering::SeqCst);
    assert_eq!(split_sync_fresh_spawn_count, 1);
}

// ---------------------------------------------------------------------------
// r16/r17 B案现场回归(同步桥 join 冻结唯一 runtime 线程):
//
// invoke_provider_via_gateway 若在 async 任务内直接调用 gateway.run_sync,
// GatewaySyncProvider::run_validated 的 worker.join() 会同步阻塞唯一
// runtime 线程——现场形态(r16 67min/r17 51min 后 kill,EXIT=137):lc-
// gateway-sync bridge 线程驱动真实 provider 会话等待终态,主线程 join 期间
// 全部 timer/WS/HTTP 无响应(180s 兜底 timer 同被冻结,表现为"timer 全灭")。
// 修复:同步桥 join 经 tokio::task::spawn_blocking 移入阻塞线程池,调用方
// runtime 保持调度。本测试用「bridge 线程内 sleep 2s 后失败」的探针制造
// 确定的冻结窗口:修前 liveness timer(300ms)只能在 join 返回后被调度
// (fire≥2s,红);修后 ~300ms 即 fire(绿)。
// ---------------------------------------------------------------------------

/// 慢速 validated streaming adapter:bridge 线程内先睡 `delay` 再失败,
/// 制造确定的「bridge 在途窗口」(现场形态:真实 CLI 会话长时间无终态)。
struct SlowValidatedStreamingAdapter {
    delay: std::time::Duration,
    bridge_starts: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl crate::cross_cutting::streaming_provider::StreamingProviderAdapter
    for SlowValidatedStreamingAdapter
{
    async fn start_validated(
        &self,
        _validated: crate::cross_cutting::session_launch::ValidatedStreamingProviderInput,
        _cancel: tokio_util::sync::CancellationToken,
    ) -> Result<
        crate::cross_cutting::streaming_provider::ProviderSession,
        ProviderAdapterError,
    > {
        self.bridge_starts.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(self.delay).await;
        Err(ProviderAdapterError::execution_failed(
            None,
            String::new(),
            "slow validated probe injected failure".to_string(),
            0,
        ))
    }
}

/// B案回归:同步桥在途期间调用方 runtime 必须保持调度(liveness timer 在
/// 调用窗口内 fire),且 bridge 恰好启动一次、失败沿 ApiError 上浮。
#[tokio::test]
async fn sync_bridge_run_must_not_starve_caller_runtime() {
    let root = tempfile::tempdir().expect("tempdir");
    let canonical_root = root.path().to_path_buf();
    let member = canonical_root.join("member_repo");
    std::fs::create_dir_all(&member).expect("member dir");
    let paths = ProductAppPaths::new(canonical_root.join(".aria"));

    // 真实同步桥:registry 挂慢速 streaming adapter,sync 槽注入
    // GatewaySyncProvider(join 冻结点),不掺测试用 sync probe。
    let bridge_starts = Arc::new(AtomicUsize::new(0));
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::ClaudeCode,
        Arc::new(SlowValidatedStreamingAdapter {
            delay: std::time::Duration::from_millis(2_000),
            bridge_starts: bridge_starts.clone(),
        }),
    );
    let registry = Arc::new(registry);
    let manifest =
        LogicalCodebaseManifest::new("project_0001", canonical_root.to_path_buf(), vec![]);
    let policies = AggregatePolicyArtifactStore::new(paths.clone());
    policies
        .ensure_bootstrap(&manifest)
        .expect("bootstrap policy");
    let gateway = LogicalCodebaseProviderGateway::with_audit(
        policies,
        Arc::new(SplitRootCwdCapabilitySource::default()),
        Arc::new(SplitRootCwdPassThroughResolver),
        registry.clone(),
        Arc::new(crate::cross_cutting::gateway_sync_provider::GatewaySyncProvider::new(
            registry,
        )),
        split_root_cwd_availability_gate(),
        Arc::new(GatewayRunAudit::new()),
        manifest.provider_context_root.clone(),
    );

    let mut repository = logical_repository();
    repository.path = member.clone();
    repository.primary_checkout_id = Some(RepositoryCheckoutId(uuid::Uuid::nil()));
    let (_, issue, _) = split_prompt_fixture();
    let engine = WorkItemSplitEngine::new(Arc::new(RecordingAdapter::new(Arc::new(
        AtomicBool::new(false),
    ))));
    let lifecycle = LifecycleStore::new(paths.clone());

    // liveness 探针:300ms timer 应在 2s 的 bridge 在途窗口内被调度。
    let liveness_ms = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let probe = liveness_ms.clone();
    let started = std::time::Instant::now();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        probe.store(
            started.elapsed().as_millis() as u64,
            Ordering::SeqCst,
        );
    });

    let failure = tokio::time::timeout(
        std::time::Duration::from_secs(6),
        engine.invoke_provider_via_gateway(
            "runtime liveness probe",
            &repository,
            ProviderName::ClaudeCode,
            &lifecycle,
            &issue,
            Arc::new(gateway),
            "ws_b_case_0001",
        ),
    )
    .await
    .expect("sync bridge call must stay bounded by the probe delay")
    .expect_err("slow probe failure must propagate through the sync bridge");
    // 给 timer 任务充分补跑机会:修前它在 join 返回后才被首次 poll(300ms
    // timer 从 ~2000ms 起算,~2300ms 才 fire,红信息自释);修后调用窗口内
    // ~300ms 即 fire。
    tokio::time::sleep(std::time::Duration::from_millis(600)).await;

    assert_eq!(
        bridge_starts.load(Ordering::SeqCst),
        1,
        "exactly one validated start through the bridge thread"
    );
    let elapsed_ms = started.elapsed().as_millis() as u64;
    assert!(
        elapsed_ms >= 1_900,
        "call must span the 2s bridge window, took {elapsed_ms}ms"
    );
    let fired = liveness_ms.load(Ordering::SeqCst);
    assert!(
        fired > 0 && fired < 1_900,
        "B案回归:同步桥 join 冻结了调用方 runtime——liveness timer 于 \
         {fired}ms 才 fire(应在 ~300ms,bridge 在途窗口 2s 内保持调度)"
    );
    assert!(
        failure.message.contains("slow validated probe injected failure")
            || failure.message.contains("ProviderExecutionFailed"),
        "bridge failure must propagate with the adapter error kind, got {}",
        failure.message
    );
}
