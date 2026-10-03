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
#[test]
fn resume_fingerprint_changes_when_working_directory_drifts() {
    use crate::product::logical_codebase::policy::{
        AggregatePolicyArtifact, PolicyTarget, ProviderDialect, SessionPolicyAction,
        SessionPolicyEnvelope,
    };
    use crate::product::logical_codebase::provider_gateway::SessionResumeFingerprint;

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

    let fingerprint = |envelope: &SessionPolicyEnvelope| {
        SessionResumeFingerprint::from_envelope(
            envelope,
            "1.4.0",
            ProviderDialect::ClaudeCodeCliV1,
            "cap_claude_code_1_4_0",
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

/// sync 探针：记录 spawn 时 input 的独立 cwd 与 worktree target 并计数，
/// 返回最小 structured output。
struct SplitRootCwdSyncProbe {
    runs: Arc<AtomicUsize>,
    cwd_at_runs: Arc<Mutex<Option<PathBuf>>>,
    worktree_at_runs: Arc<Mutex<Option<String>>>,
}

impl SplitRootCwdSyncProbe {
    fn new() -> Self {
        Self {
            runs: Arc::new(AtomicUsize::new(0)),
            cwd_at_runs: Arc::new(Mutex::new(None)),
            worktree_at_runs: Arc::new(Mutex::new(None)),
        }
    }
}

impl ProviderAdapter for SplitRootCwdSyncProbe {
    fn run(&self, input: &AdapterInput) -> Result<AdapterOutput, ProviderAdapterError> {
        self.runs.fetch_add(1, Ordering::SeqCst);
        *self.cwd_at_runs.lock().expect("split cwd probe") = input.working_directory.clone();
        *self.worktree_at_runs.lock().expect("split worktree probe") = input.worktree_path.clone();
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

/// gateway 测试 capability source:按 provider ref 返回对应 dialect
/// (Task 2b 分格形状:launch/resume/write_boundary 恒 Confirmed)。
struct SplitRootCwdCapabilitySource;

impl SplitRootCwdCapabilitySource {
    fn capability(provider: &ProviderRef, action: SessionPolicyAction) -> ProviderCapability {
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
                resume: ProviderCapabilityEvidence::Confirmed,
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
        Ok(Self::capability(provider, action))
    }

    fn require_resume_supported(
        &self,
        provider: &ProviderRef,
        action: SessionPolicyAction,
    ) -> Result<ProviderCapability, ProviderGatewayError> {
        Ok(Self::capability(provider, action))
    }

    fn require_write_boundary(
        &self,
        provider: &ProviderRef,
        action: SessionPolicyAction,
    ) -> Result<ProviderCapability, ProviderGatewayError> {
        Ok(Self::capability(provider, action))
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
        Ok(Self::capability(
            provider,
            SessionPolicyAction::PlanningReadOnly,
        ))
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
        Arc::new(SplitRootCwdCapabilitySource),
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
            &gateway,
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
