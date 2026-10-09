//! Task 12b:两类 direct 对照(实施计划 Task 12 段 517/519/521-534 行;
//! REQ-LCG-02)。对照合同(与 Task 13 兼容锁同源,不升级交付面):
//!
//! 1. **sync direct 两槽**:`task_run/provider_factory.rs` 的
//!    `RoutingProviderAdapter` 只保留 Claude/Codex 两个同步 direct 槽;
//!    Pi/Kimi 无同步 direct 槽,以 `incompatible_output`("does not
//!    schedule pi/kimi_code")reject——即计划 533 行概念判别
//!    `Err(DirectProviderUnsupported)` 的生产错误形态。两槽的
//!    args(prompt/worktree/timeout)与 output 端到端透传;LC 网关工作
//!    (缺发布 fail-closed)不得扰动该拓扑(计划 531 行 before==after)。
//! 2. **workspace streaming legacy 四家**:四家真实 adapter 的 raw
//!    `start`(`StreamingProviderAdapter::start`——workspace streaming
//!    legacy 路径,仅保留单仓 direct,不是 LC validated 入口)保持各自
//!    独立形态,不因本 change 被对照升级为「四家同步 direct 支持」
//!    (计划 532 行 before==after)。
//!
//! 单测观测面说明(argv/cwd/permission/output baseline):raw `start` 在
//! 配置的 CLI 命令不可解析时于 spawn 入口 fail-closed(`command_missing`
//! 携带精确命令名),单测据此钉死「raw 路径进入 + argv[0] 即配置命令 +
//! 错误类别」;cwd/permission/output 的真实 spawn 基线只能在真实 CLI
//! 现场观测,归 Task 10/11 live 矩阵 cell 与
//! `lcg_live_fresh_lc_policy_publication_recipe_and_direct_comparison`
//! 现场,不以单测冒充。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use cadence_aria::cross_cutting::claude_code_provider::ClaudeCodeProvider;
use cadence_aria::cross_cutting::codex_provider::CodexProvider;
use cadence_aria::cross_cutting::kimi_code_provider::KimiCodeProvider;
use cadence_aria::cross_cutting::pi_provider::PiProvider;
use cadence_aria::cross_cutting::provider_adapter::{ProviderAdapter, ProviderAdapterError};
use cadence_aria::cross_cutting::streaming_provider::{
    ProviderPermissionMode, StreamingProviderAdapter, StreamingProviderInput,
};
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::logical_codebase::{
    AggregatePolicyArtifactStore, ProviderGatewayError, ProviderRef,
};

use cadence_aria::protocol::contracts::{
    AdapterInput, AdapterOutput, AdapterRole, ProviderType, TimeoutStatus,
};
use cadence_aria::protocol::provider_errors::ProviderErrorCode;
use cadence_aria::task_run::provider_factory::RoutingProviderAdapter;
use tempfile::tempdir;
use tokio_util::sync::CancellationToken;

use super::policy_dependency::{t12_gateway, t12_planning_request};

/// 计划 533 行概念判别 `DirectProviderUnsupported`:生产错误形态 =
/// `ProviderErrorCode::ProviderIncompatibleOutput` + "does not schedule"
/// (RoutingProviderAdapter 对 Pi/Kimi/Fake 的固定 reject)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SyncDirectRejection {
    DirectProviderUnsupported,
}

fn classify_sync_direct_rejection(error: &ProviderAdapterError) -> Result<(), SyncDirectRejection> {
    if error.code == ProviderErrorCode::ProviderIncompatibleOutput
        && error.details.contains("does not schedule")
    {
        return Err(SyncDirectRejection::DirectProviderUnsupported);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 对照 1:sync direct 两槽拓扑
// ---------------------------------------------------------------------------

/// 单槽探针:记录派发事实(prompt/worktree/timeout 透传 + 计数 + 固定 output)。
struct SlotProbe {
    slot: &'static str,
    runs: AtomicUsize,
    last_prompt: Mutex<String>,
    last_worktree: Mutex<Option<String>>,
    last_timeout: AtomicUsize,
}

impl SlotProbe {
    fn new(slot: &'static str) -> Arc<Self> {
        Arc::new(Self {
            slot,
            runs: AtomicUsize::new(0),
            last_prompt: Mutex::new(String::new()),
            last_worktree: Mutex::new(None),
            last_timeout: AtomicUsize::new(0),
        })
    }
}

impl ProviderAdapter for SlotProbe {
    fn run(&self, input: &AdapterInput) -> Result<AdapterOutput, ProviderAdapterError> {
        self.runs.fetch_add(1, Ordering::SeqCst);
        *self.last_prompt.lock().expect("slot probe prompt lock") = input.prompt.clone();
        *self.last_worktree.lock().expect("slot probe worktree lock") = input.worktree_path.clone();
        self.last_timeout
            .store(input.timeout as usize, Ordering::SeqCst);
        Ok(AdapterOutput {
            exit_code: Some(0),
            stdout: format!("direct-slot:{}", self.slot),
            stderr: String::new(),
            structured_output: None,
            files_modified: Vec::new(),
            duration_ms: 0,
            timeout_status: TimeoutStatus::NotTimedOut,
        })
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
struct SyncSlotDispatch {
    slot: &'static str,
    runs: usize,
    prompt_passthrough: String,
    worktree_passthrough: Option<String>,
    timeout_passthrough: u64,
    output_stdout: String,
}

#[derive(Debug, PartialEq, Eq, Clone)]
struct SyncRejection {
    provider: &'static str,
    code: &'static str,
    details: String,
    /// 活错误上的计划概念判别(计划 533 行 `Err(DirectProviderUnsupported)`)。
    classified: Result<(), SyncDirectRejection>,
}

/// sync direct 两槽拓扑观测(可整体 before==after 比较,计划 531 行)。
#[derive(Debug, PartialEq, Eq, Clone)]
pub(crate) struct SyncDirectTopology {
    claude: SyncSlotDispatch,
    codex: SyncSlotDispatch,
    pi: SyncRejection,
    kimi: SyncRejection,
    fake: SyncRejection,
}

fn sync_input(provider_type: ProviderType, prompt: &str, worktree: &PathBuf) -> AdapterInput {
    AdapterInput {
        provider_type,
        role: AdapterRole::Executor,
        worktree_path: Some(worktree.to_string_lossy().into_owned()),
        working_directory: None,
        provider_stream_log_dir: None,
        prompt: prompt.to_string(),
        context_files: Vec::new(),
        output_schema: String::new(),
        timeout: 17,
        max_retries: 0,
    }
}

/// 归一化 worktree 透传观测:保留文件名(透传语义),剥离每次捕获的
/// tempdir 前缀,使 before/after 拓扑可整体比较。
fn normalize_worktree(raw: Option<String>) -> Option<String> {
    raw.map(|path| {
        std::path::Path::new(&path)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or(path)
    })
}

/// 采集一次两槽拓扑:Claude/Codex 各派发一次,Pi/Kimi/Fake 各请求一次。
pub(crate) fn capture_sync_direct_topology() -> SyncDirectTopology {
    let worktree_root = tempdir().expect("sync direct worktree root");
    let worktree = worktree_root.path().join("member-checkout");
    std::fs::create_dir_all(&worktree).expect("member worktree");

    let claude_probe = SlotProbe::new("claude");
    let codex_probe = SlotProbe::new("codex");
    let routing = RoutingProviderAdapter::new(
        // SlotProbe 不携带 Send+Sync 约束的 Box 包装歧义:显式装箱。
        Box::new(SlotAdapter {
            probe: claude_probe.clone(),
        }),
        Box::new(SlotAdapter {
            probe: codex_probe.clone(),
        }),
    );

    let claude_output = routing
        .run(&sync_input(
            ProviderType::ClaudeCode,
            "t12 sync direct claude prompt",
            &worktree,
        ))
        .expect("claude 两槽同步 direct 必须派发到 claude 槽");
    assert_eq!(
        codex_probe.runs.load(Ordering::SeqCst),
        0,
        "claude 输入不得派发到 codex 槽"
    );
    let claude = SyncSlotDispatch {
        slot: "claude",
        runs: claude_probe.runs.load(Ordering::SeqCst),
        prompt_passthrough: claude_probe
            .last_prompt
            .lock()
            .expect("claude prompt lock")
            .clone(),
        worktree_passthrough: normalize_worktree(
            claude_probe
                .last_worktree
                .lock()
                .expect("claude worktree lock")
                .clone(),
        ),
        timeout_passthrough: claude_probe.last_timeout.load(Ordering::SeqCst) as u64,
        output_stdout: claude_output.stdout,
    };

    let codex_output = routing
        .run(&sync_input(
            ProviderType::Codex,
            "t12 sync direct codex prompt",
            &worktree,
        ))
        .expect("codex 两槽同步 direct 必须派发到 codex 槽");
    assert_eq!(
        claude_probe.runs.load(Ordering::SeqCst),
        1,
        "codex 输入不得派发到 claude 槽"
    );
    let codex = SyncSlotDispatch {
        slot: "codex",
        runs: codex_probe.runs.load(Ordering::SeqCst),
        prompt_passthrough: codex_probe
            .last_prompt
            .lock()
            .expect("codex prompt lock")
            .clone(),
        worktree_passthrough: normalize_worktree(
            codex_probe
                .last_worktree
                .lock()
                .expect("codex worktree lock")
                .clone(),
        ),
        timeout_passthrough: codex_probe.last_timeout.load(Ordering::SeqCst) as u64,
        output_stdout: codex_output.stdout,
    };

    let pi = sync_rejection(&routing, ProviderType::Pi, &worktree, "pi");
    let kimi = sync_rejection(&routing, ProviderType::KimiCode, &worktree, "kimi_code");
    let fake = sync_rejection(&routing, ProviderType::Fake, &worktree, "fake");

    SyncDirectTopology {
        claude,
        codex,
        pi,
        kimi,
        fake,
    }
}

fn sync_rejection(
    routing: &RoutingProviderAdapter,
    provider_type: ProviderType,
    worktree: &PathBuf,
    label: &'static str,
) -> SyncRejection {
    let error = routing
        .run(&sync_input(
            provider_type,
            "t12 sync direct probe",
            worktree,
        ))
        .expect_err(&format!("{label} 无同步 direct 槽必须 reject"));
    let classified = classify_sync_direct_rejection(&error);
    SyncRejection {
        provider: label,
        code: error.code.as_str(),
        details: error.details.clone(),
        classified,
    }
}

/// Box 层:RoutingProviderAdapter 持有 `Box<dyn ProviderAdapter + Send + Sync>`。
struct SlotAdapter {
    probe: Arc<SlotProbe>,
}

impl ProviderAdapter for SlotAdapter {
    fn run(&self, input: &AdapterInput) -> Result<AdapterOutput, ProviderAdapterError> {
        self.probe.run(input)
    }
}

/// 计划 519 行 `lcg_t12_single_repository_sync_direct_two_slot_comparison`:
/// 两槽派发/透传/输出 + Pi/Kimi `Err(DirectProviderUnsupported)` + LC 网关
/// 缺发布 fail-closed 前后拓扑不变(计划 531 行)。
#[test]
fn lcg_t12_single_repository_sync_direct_two_slot_comparison() {
    let sync_direct_before = capture_sync_direct_topology();

    // 两槽各自恰好派发一次,槽间零串扰。
    assert_eq!(sync_direct_before.claude.runs, 1, "claude 槽恰好一次派发");
    assert_eq!(sync_direct_before.codex.runs, 1, "codex 槽恰好一次派发");
    // args(prompt/worktree/timeout)与 output 端到端透传(计划 Interfaces:
    // 两类 direct 对照记录 argv/cwd/output baseline)。
    assert_eq!(
        sync_direct_before.claude.prompt_passthrough,
        "t12 sync direct claude prompt"
    );
    assert!(
        sync_direct_before
            .claude
            .worktree_passthrough
            .as_ref()
            .is_some_and(|path| path.ends_with("member-checkout"))
    );
    assert_eq!(sync_direct_before.claude.timeout_passthrough, 17);
    assert_eq!(
        sync_direct_before.claude.output_stdout, "direct-slot:claude",
        "claude 输入的 output 必须来自 claude 槽"
    );
    assert_eq!(
        sync_direct_before.codex.output_stdout, "direct-slot:codex",
        "codex 输入的 output 必须来自 codex 槽"
    );

    // 计划 533 行:Pi/Kimi 同步 direct = Err(DirectProviderUnsupported)。
    let pi_kimi_sync_result = [
        (&sync_direct_before.pi, "pi"),
        (&sync_direct_before.kimi, "kimi"),
    ];
    for (rejection, label) in pi_kimi_sync_result {
        assert_eq!(
            rejection.classified,
            Err(SyncDirectRejection::DirectProviderUnsupported),
            "计划 533 行:{label} pi_kimi_sync_result == Err(DirectProviderUnsupported)"
        );
    }
    // reject 文案钉死生产槽位语义(不冒充四家同步 direct 支持)。
    assert!(
        sync_direct_before
            .pi
            .details
            .contains("does not schedule pi"),
        "pi reject 文案必须钉死无槽语义(得到 {})",
        sync_direct_before.pi.details
    );
    assert!(
        sync_direct_before
            .kimi
            .details
            .contains("does not schedule kimi_code"),
        "kimi reject 文案必须钉死无槽语义(得到 {})",
        sync_direct_before.kimi.details
    );

    // LC 邻接操作:LC 网关缺发布 fail-closed(REQ-LCG 侧新链)不得扰动
    // 单仓 sync direct 拓扑(计划 531 行 before==after)。
    let root_guard = tempdir().expect("lc-adjacent root");
    let canonical_root = root_guard.path().canonicalize().expect("canonical root");
    let paths = ProductAppPaths::new(canonical_root.join(".aria"));
    let fixture = t12_gateway(
        AggregatePolicyArtifactStore::for_lc(paths.clone(), "lc_0001"),
        None,
        &canonical_root,
    );
    let error = fixture
        .gateway
        .validate(t12_planning_request(
            ProviderRef::claude_code("t12-cap-claude"),
            &canonical_root,
        ))
        .expect_err("LC 侧缺发布必须 fail-closed");
    assert!(
        matches!(&error, ProviderGatewayError::PolicyMissing(_)),
        "LC 邻接操作应保持 PolicyMissing(得到 {error:?})"
    );

    let sync_direct_after = capture_sync_direct_topology();
    assert_eq!(
        sync_direct_after, sync_direct_before,
        "计划 531 行:sync direct 两槽拓扑在 LC 网关工作前后不变"
    );
}

// ---------------------------------------------------------------------------
// 对照 2:workspace streaming legacy 四家 raw start 形态
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq, Clone)]
pub(crate) struct LegacyStartObservation {
    provider: &'static str,
    code: &'static str,
    details: String,
}

/// 四家真实 adapter 的 raw `start`(legacy 路径,非 LC validated 入口)
/// 观测:构造与生产 registry 同款 adapter(`Provider::new(command)`),
/// command 使用确定不可解析的绝对路径——raw 路径在 spawn 入口以
/// `command_missing`(携带精确命令名)fail-closed,kimi 的 raw 路径先过
/// 版本门(探测失败 → 版本不满足),二者都是「raw start 被进入」的
/// 确定性证据(见模块头观测面说明)。
pub(crate) async fn capture_workspace_streaming_legacy_shape() -> Vec<LegacyStartObservation> {
    let root_guard = tempdir().expect("legacy streaming workdir root");
    let working_dir = root_guard.path().to_path_buf();

    let claude: Arc<dyn StreamingProviderAdapter> = Arc::new(ClaudeCodeProvider::new(
        PathBuf::from("/nonexistent/lcg-t12-legacy/claude"),
    ));
    let codex: Arc<dyn StreamingProviderAdapter> = Arc::new(CodexProvider::new(PathBuf::from(
        "/nonexistent/lcg-t12-legacy/codex",
    )));
    let pi: Arc<dyn StreamingProviderAdapter> = Arc::new(PiProvider::new(PathBuf::from(
        "/nonexistent/lcg-t12-legacy/pi",
    )));
    let kimi: Arc<dyn StreamingProviderAdapter> = Arc::new(KimiCodeProvider::new(PathBuf::from(
        "/nonexistent/lcg-t12-legacy/kimi",
    )));

    let cases: [(
        &'static str,
        Arc<dyn StreamingProviderAdapter>,
        ProviderType,
    ); 4] = [
        ("claude_code", claude, ProviderType::ClaudeCode),
        ("codex", codex, ProviderType::Codex),
        ("pi", pi, ProviderType::Pi),
        ("kimi_code", kimi, ProviderType::KimiCode),
    ];

    let mut observations = Vec::with_capacity(cases.len());
    for (label, adapter, provider_type) in cases {
        let input = StreamingProviderInput {
            provider_type,
            role: AdapterRole::Executor,
            prompt: "t12 workspace streaming legacy probe".to_string(),
            working_dir: working_dir.clone(),
            working_directory: None,
            workspace_session_id: Some("t12_legacy_probe_0001".to_string()),
            resume_provider_session_id: None,
            permission_mode: ProviderPermissionMode::Auto,
            tool_policy: None,
            audit_sink: None,
            structured_output_contract: None,
            env_vars: BTreeMap::new(),
            timeout_secs: 5,
            baseline_tree: None,
        };
        // raw `start`:workspace streaming legacy 唯一入口(非 start_validated,
        // 非 split_sync,非 LC validated gateway)。
        let outcome = adapter.start(input, CancellationToken::new()).await;
        let observation = match outcome {
            Ok(_session) => {
                panic!("{label} raw start 在不可解析命令下不得成功返回(单测禁止真实 spawn)")
            }
            Err(error) => LegacyStartObservation {
                provider: label,
                code: error.code.as_str(),
                details: error.details,
            },
        };
        observations.push(observation);
    }
    observations
}

/// 计划 519 行 `lcg_t12_workspace_streaming_legacy_four_provider_comparison`:
/// 四家 raw start 形态(进入 raw 路径 + argv[0]=配置命令 + 错误类别)在
/// LC 邻接操作前后不变(计划 532 行);Pi/Kimi 不得因对照被升级为同步
/// direct 支持(与对照 1 的 reject 断言互证)。
#[tokio::test]
async fn lcg_t12_workspace_streaming_legacy_four_provider_comparison() {
    let workspace_streaming_before = capture_workspace_streaming_legacy_shape().await;

    // 四家形态逐一钉死(raw 路径进入的确定性证据)。
    let claude = &workspace_streaming_before[0];
    let codex = &workspace_streaming_before[1];
    let pi = &workspace_streaming_before[2];
    let kimi = &workspace_streaming_before[3];
    assert_eq!(claude.provider, "claude_code");
    assert_eq!(
        claude.code,
        ProviderErrorCode::ProviderCommandMissing.as_str(),
        "claude raw start 必须到达 spawn 入口(得到 {}:{})",
        claude.code,
        claude.details
    );
    assert!(
        claude
            .details
            .contains("/nonexistent/lcg-t12-legacy/claude"),
        "claude spawn argv[0] 必须是配置命令(得到 {})",
        claude.details
    );
    assert_eq!(codex.provider, "codex");
    assert_eq!(
        codex.code,
        ProviderErrorCode::ProviderCommandMissing.as_str(),
        "codex raw start 必须到达 spawn 入口(得到 {}:{})",
        codex.code,
        codex.details
    );
    assert!(
        codex.details.contains("/nonexistent/lcg-t12-legacy/codex"),
        "codex spawn argv[0] 必须是配置命令(得到 {})",
        codex.details
    );
    assert_eq!(pi.provider, "pi");
    assert_eq!(
        pi.code,
        ProviderErrorCode::ProviderCommandMissing.as_str(),
        "pi raw start 必须到达 spawn 入口(得到 {}:{})",
        pi.code,
        pi.details
    );
    assert!(
        pi.details.contains("/nonexistent/lcg-t12-legacy/pi"),
        "pi spawn argv[0] 必须是配置命令(得到 {})",
        pi.details
    );
    assert_eq!(kimi.provider, "kimi_code");
    assert_eq!(
        kimi.code,
        ProviderErrorCode::ProviderParseError.as_str(),
        "kimi raw start 先过版本门(探测失败→版本不满足;得到 {}:{})",
        kimi.code,
        kimi.details
    );
    assert!(
        kimi.details.contains("Kimi version"),
        "kimi raw start 的版本门文案必须钉死(得到 {})",
        kimi.details
    );

    // LC 邻接操作(同对照 1 的缺发布 fail-closed)不得扰动 legacy 形态
    // (计划 532 行 before==after)。
    let root_guard = tempdir().expect("lc-adjacent root");
    let canonical_root = root_guard.path().canonicalize().expect("canonical root");
    let paths = ProductAppPaths::new(canonical_root.join(".aria"));
    let fixture = t12_gateway(
        AggregatePolicyArtifactStore::for_lc(paths.clone(), "lc_0001"),
        None,
        &canonical_root,
    );
    let error = fixture
        .gateway
        .validate(t12_planning_request(
            ProviderRef::claude_code("t12-cap-claude"),
            &canonical_root,
        ))
        .expect_err("LC 侧缺发布必须 fail-closed");
    assert!(
        matches!(&error, ProviderGatewayError::PolicyMissing(_)),
        "LC 邻接操作应保持 PolicyMissing(得到 {error:?})"
    );

    let workspace_streaming_after = capture_workspace_streaming_legacy_shape().await;
    assert_eq!(
        workspace_streaming_after, workspace_streaming_before,
        "计划 532 行:四家 workspace streaming legacy 形态前后不变"
    );
}
