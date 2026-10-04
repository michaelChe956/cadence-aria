// Task 3.2（REQ-ENV-09/D7）：pi 策略会话的有界握手（id 预生成/传入）、
// provider_start durable 写入、append 失败 kill 链与缺 sink fail-closed 测试。

use std::collections::BTreeMap;
use std::sync::atomic::Ordering;

use tokio_util::sync::CancellationToken;

use crate::cross_cutting::streaming_provider::{
    ProviderEvent, ProviderPermissionMode, ProviderToolPolicy, ProviderVersionSupplier,
    StreamingProviderAdapter, StreamingProviderInput,
};
use crate::cross_cutting::tool_policy_audit::test_support::RecordingToolPolicyAuditSink;
use crate::cross_cutting::tool_policy_audit::{DurableToolPolicyEvent, ToolPolicyAuditSink};
use crate::protocol::contracts::{AdapterRole, ProviderType};

use super::*;

fn policy_version_supplier() -> ProviderVersionSupplier {
    std::sync::Arc::new(|| Ok("pi 0.83.0-policy-fixture".to_string()))
}

fn policy_pi_input(
    resume_id: Option<String>,
    audit_sink: Option<std::sync::Arc<dyn ToolPolicyAuditSink>>,
) -> StreamingProviderInput {
    let mut input = streaming_input_for_test(resume_id);
    input.role = AdapterRole::Orchestrator;
    input.tool_policy = Some(ProviderToolPolicy::deny_file_write_builtins());
    input.audit_sink = audit_sink;
    input
}

/// fake pi：--version 打印兼容版本；rpc 模式下登记 PID（kill 链断言）并读 stdin
/// 到 EOF 后退出。
#[cfg(unix)]
fn policy_pi_fixture(marker: &std::path::Path) -> PathBuf {
    write_executable(
        marker.parent().expect("marker parent"),
        "fake-policy-pi",
        &format!(
            "if [ \"$1\" = \"--version\" ]; then echo 0.83.0; exit 0; fi\necho $$ > {}\nwhile IFS= read -r line; do :; done",
            marker.display()
        ),
    )
}

fn plain_policy_pi_fixture() -> PathBuf {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = write_executable(
        temp.path(),
        "fake-policy-pi-plain",
        "if [ \"$1\" = \"--version\" ]; then echo 0.83.0; exit 0; fi\nwhile IFS= read -r line; do :; done",
    );
    let _ = temp.keep();
    path
}

#[cfg(unix)]
fn assert_process_exits(pid: u32) {
    let deadline = std::time::Duration::from_secs(5);
    let start = std::time::Instant::now();
    loop {
        let alive = std::process::Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false);
        if !alive {
            return;
        }
        assert!(
            start.elapsed() < deadline,
            "policy session child (pid {pid}) must be terminated after append failure"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

#[cfg(unix)]
fn read_marker_pid(marker: &std::path::Path) -> Option<u32> {
    // 等待 fixture 登记 PID；append 失败 kill 可能快于 fixture 写 marker——
    // 此时子进程已死（marker 永不出现），同样证明 kill 链生效。
    let deadline = std::time::Duration::from_secs(2);
    let start = std::time::Instant::now();
    loop {
        if let Ok(content) = std::fs::read_to_string(marker)
            && let Ok(pid) = content.trim().parse::<u32>()
        {
            return Some(pid);
        }
        if start.elapsed() >= deadline {
            return None;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

/// P2-2：策略会话的版本探测必须走进程内缓存——同一命令路径的第二次策略启动
/// 不得重复执行 `--version`（计数 fixture：预检每次 start 各 1 次 + 策略探测仅
/// 首次 1 次，共 3 次；无缓存时为 4 次）。
#[cfg(unix)]
#[tokio::test]
async fn pi_policy_version_probe_uses_process_cache_across_starts() {
    let temp = tempfile::tempdir().expect("tempdir");
    let counter = temp.path().join("version-probe.count");
    let fixture = temp.path().join("fake-pi-counting");
    std::fs::write(
        &fixture,
        format!(
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then\n  n=$(cat {counter} 2>/dev/null || echo 0)\n  echo $((n + 1)) > {counter}\n  echo 0.83.0\n  exit 0\nfi\nwhile IFS= read -r line; do :; done\n",
            counter = counter.display()
        ),
    )
    .expect("write fixture");
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(&fixture).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&fixture, permissions).expect("chmod fixture");
    }
    let provider = PiProvider::new(fixture);
    let sink = RecordingToolPolicyAuditSink::new();
    for _ in 0..2 {
        let session = provider
            .start(
                policy_pi_input(None, Some(sink.clone().bound())),
                CancellationToken::new(),
            )
            .await
            .expect("policy session starts with real probing");
        drop(session);
    }
    let invocations: u32 = std::fs::read_to_string(&counter)
        .expect("counter file")
        .trim()
        .parse()
        .expect("counter value");
    assert_eq!(
        invocations, 3,
        "两次策略启动：预检各 1 次 + 策略探测仅首次（后续命中缓存）"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn pi_policy_start_pregenerates_session_id_and_writes_provider_start() {
    let sink = RecordingToolPolicyAuditSink::new();
    let provider =
        PiProvider::new(plain_policy_pi_fixture()).with_version_supplier(policy_version_supplier());
    let input = policy_pi_input(None, Some(sink.clone().bound()));

    let session = provider
        .start(input, CancellationToken::new())
        .await
        .expect("pi policy session starts");

    // pi 握手 = id 预生成：fresh 策略会话必须返回预生成的 native session id。
    let native_id = session
        .native_session_id
        .clone()
        .expect("pi policy session must carry a pre-generated native session id");
    assert!(!native_id.trim().is_empty());

    // provider_start：首条 durable 事件，含冻结 argv（--session-id + denylist）、
    // digest、version（supplier 注入）与 dialect。
    let events = sink.events();
    assert_eq!(events.len(), 1, "only provider_start is written at start");
    let DurableToolPolicyEvent::ProviderStart(record) = &events[0] else {
        panic!("expected provider_start");
    };
    assert_eq!(record.provider, "pi");
    // P1-8：durable 审计保留真实 AdapterRole 序列化值（orchestrator），不与
    // usage 展示层归一（usage 把 Orchestrator 归一为 author）。
    assert_eq!(record.role, "orchestrator");
    assert_eq!(record.adapter_dialect, PI_POLICY_DIALECT);
    assert_eq!(record.provider_version, "pi 0.83.0-policy-fixture");
    assert_eq!(record.provider_session_id, native_id);
    assert!(record.argv.contains(&"--session-id".to_string()));
    assert!(record.argv.contains(&native_id));
    assert!(record.argv.contains(&"--exclude-tools".to_string()));
    assert!(record.argv.contains(&"edit,write".to_string()));
    assert!(!record.tool_policy_canonical_digest.is_empty());
    assert_eq!(record.sandbox, None);
    assert_eq!(record.approval_policy, None);
}

#[cfg(unix)]
#[tokio::test]
async fn pi_policy_start_reuses_resume_session_id_as_native_id() {
    let sink = RecordingToolPolicyAuditSink::new();
    // Task 3.3：resume 前置冻结三元组比对——预置一致记录使决策为 Resume。
    let canonical = crate::cross_cutting::streaming_provider::canonical_tool_policy(
        TOOL_POLICY_PROVIDER_NAME,
        &ProviderToolPolicy::deny_file_write_builtins(),
    )
    .expect("canonical policy");
    sink.with_stored_provider_start(
        crate::cross_cutting::tool_policy_audit::ProviderStartAudit {
            provider: "pi".to_string(),
            workspace_session_id: "ws-test".to_string(),
            role: "author".to_string(),
            tool_policy_canonical_digest: canonical.digest,
            argv: Vec::new(),
            sandbox: None,
            approval_policy: None,
            provider_version: "pi 0.83.0-policy-fixture".to_string(),
            adapter_dialect: PI_POLICY_DIALECT.to_string(),
            provider_session_id: "pi-session-resume-policy".to_string(),
            lc_projection: None,
        },
    );
    let provider =
        PiProvider::new(plain_policy_pi_fixture()).with_version_supplier(policy_version_supplier());
    let input = policy_pi_input(
        Some("pi-session-resume-policy".to_string()),
        Some(sink.clone().bound()),
    );

    let session = provider
        .start(input, CancellationToken::new())
        .await
        .expect("pi policy resume session starts");
    assert_eq!(
        session.native_session_id.as_deref(),
        Some("pi-session-resume-policy")
    );
    let DurableToolPolicyEvent::ProviderStart(record) = &sink.events()[0] else {
        panic!("expected provider_start");
    };
    assert_eq!(record.provider_session_id, "pi-session-resume-policy");
}

#[cfg(unix)]
#[tokio::test]
async fn pi_policy_start_without_sink_or_version_fails_closed() {
    let provider = PiProvider::new(plain_policy_pi_fixture());
    // 缺 sink：fail-closed。
    let Err(error) = provider
        .start(policy_pi_input(None, None), CancellationToken::new())
        .await
    else {
        panic!("policy session without sink must fail closed");
    };
    assert!(
        error.details.contains("audit sink is required"),
        "unexpected error: {}",
        error.details
    );

    // 版本不可得（注入失败 supplier，模拟探测 Unavailable）：fail-closed
    //（Task 3.3 默认路径为真实 CLI 探测+缓存，此处验证错误传播）。
    let provider =
        PiProvider::new(plain_policy_pi_fixture()).with_version_supplier(std::sync::Arc::new(
            || Err(crate::cross_cutting::streaming_provider::VersionProbeError::Unavailable),
        ));
    let Err(error) = provider
        .start(
            policy_pi_input(None, Some(RecordingToolPolicyAuditSink::new().bound())),
            CancellationToken::new(),
        )
        .await
    else {
        panic!("policy session without version must fail closed");
    };
    assert!(
        error.details.contains("provider version unavailable"),
        "unexpected error: {}",
        error.details
    );
}

#[cfg(unix)]
#[tokio::test]
async fn pi_policy_resume_compares_frozen_triple_and_supersedes_on_drift() {
    // Task 3.3：匹配存档 → 保留 resume id；digest drift → 追加 superseded 终止
    // 审计并以预生成新 id 新建会话。
    let canonical = crate::cross_cutting::streaming_provider::canonical_tool_policy(
        TOOL_POLICY_PROVIDER_NAME,
        &ProviderToolPolicy::deny_file_write_builtins(),
    )
    .expect("canonical policy");
    let base_record =
        |digest: String| crate::cross_cutting::tool_policy_audit::ProviderStartAudit {
            provider: "pi".to_string(),
            workspace_session_id: "ws-test".to_string(),
            role: "author".to_string(),
            tool_policy_canonical_digest: digest,
            argv: Vec::new(),
            sandbox: None,
            approval_policy: None,
            provider_version: "pi 0.83.0-policy-fixture".to_string(),
            adapter_dialect: PI_POLICY_DIALECT.to_string(),
            provider_session_id: "pi-session-resume-policy".to_string(),
            lc_projection: None,
        };

    // 匹配：resume id 保留为 native id，无 superseded 审计。
    let sink = RecordingToolPolicyAuditSink::new();
    sink.with_stored_provider_start(base_record(canonical.digest.clone()));
    let provider =
        PiProvider::new(plain_policy_pi_fixture()).with_version_supplier(policy_version_supplier());
    let session = provider
        .start(
            policy_pi_input(
                Some("pi-session-resume-policy".to_string()),
                Some(sink.clone().bound()),
            ),
            CancellationToken::new(),
        )
        .await
        .expect("matching resume must be preserved");
    assert_eq!(
        session.native_session_id.as_deref(),
        Some("pi-session-resume-policy")
    );
    assert!(
        !sink
            .events()
            .iter()
            .any(|event| event.event_type() == "session_terminated")
    );

    // drift：digest 不一致 → superseded + 新 id。
    let sink = RecordingToolPolicyAuditSink::new();
    sink.with_stored_provider_start(base_record("sha256:drifted".to_string()));
    let provider =
        PiProvider::new(plain_policy_pi_fixture()).with_version_supplier(policy_version_supplier());
    let session = provider
        .start(
            policy_pi_input(
                Some("pi-session-resume-policy".to_string()),
                Some(sink.clone().bound()),
            ),
            CancellationToken::new(),
        )
        .await
        .expect("drifted resume must start a fresh session");
    let native_id = session
        .native_session_id
        .expect("fresh session carries a pre-generated id");
    assert_ne!(native_id, "pi-session-resume-policy");
    let events = sink.events();
    let superseded_index = events
        .iter()
        .position(|event| event.event_type() == "session_terminated")
        .expect("drifted resume must be marked superseded");
    assert!(
        serde_json::to_string(&events[superseded_index])
            .unwrap()
            .contains("superseded_policy_drift")
    );
    let start_index = events
        .iter()
        .position(|event| event.event_type() == "provider_start")
        .expect("fresh provider_start");
    assert!(superseded_index < start_index);
    let crate::cross_cutting::tool_policy_audit::DurableToolPolicyEvent::ProviderStart(record) =
        &events[start_index]
    else {
        panic!("expected provider_start");
    };
    assert_eq!(record.provider_session_id, native_id);
}

/// 带内收集首个 ToolPolicyWarning（带 2s 界限；warning 在 start 返回前已入队，
/// 通常首个非状态事件即命中）。
async fn recv_tool_policy_warning(
    events: &mut mpsc::Receiver<ProviderEvent>,
) -> Option<crate::cross_cutting::streaming_provider::CodexProtocolWarningEvent> {
    loop {
        match tokio::time::timeout(std::time::Duration::from_secs(2), events.recv()).await {
            Err(_) => return None,
            Ok(None) => return None,
            Ok(Some(ProviderEvent::ToolPolicyWarning(warning))) => return Some(warning),
            Ok(Some(_)) => {}
        }
    }
}

/// GC9：resume 记录缺失（find_provider_start → None）与 drift 同路径处置——
/// 清除 resume id、以全新会话（新预生成 id+provider_start）启动；「标记 superseded」
/// 仅带内 ToolPolicyWarning（🔴 无旧文件可写，不得伪造无 provider_start 首行的
/// durable 文件）。
#[cfg(unix)]
#[tokio::test]
async fn pi_policy_resume_with_missing_record_starts_fresh_and_warns_in_band() {
    let sink = RecordingToolPolicyAuditSink::new();
    let provider =
        PiProvider::new(plain_policy_pi_fixture()).with_version_supplier(policy_version_supplier());
    let mut session = provider
        .start(
            policy_pi_input(
                Some("pi-session-resume-policy".to_string()),
                Some(sink.clone().bound()),
            ),
            CancellationToken::new(),
        )
        .await
        .expect("missing record must start a fresh session");

    // 全新会话：native id 为预生成新 id，不复用 resume id。
    let native_id = session
        .native_session_id
        .clone()
        .expect("fresh session carries a pre-generated id");
    assert_ne!(native_id, "pi-session-resume-policy");

    // 🔴 不得伪造 durable 事件：无 session_terminated，仅新会话 provider_start。
    let events = sink.events();
    assert_eq!(events.len(), 1, "only the fresh provider_start is written");
    let DurableToolPolicyEvent::ProviderStart(record) = &events[0] else {
        panic!("expected provider_start");
    };
    assert_eq!(record.provider_session_id, native_id);

    // 带内标记：ToolPolicyWarning(superseded_policy_record_missing)。
    let warning = recv_tool_policy_warning(&mut session.events)
        .await
        .expect("missing record must emit an in-band superseded warning");
    assert_eq!(
        warning.reason_code, "superseded_policy_record_missing",
        "unexpected warning: {warning:?}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn pi_policy_start_append_failure_kills_child_and_fails_closed() {
    let temp = tempfile::tempdir().expect("tempdir");
    let marker = temp.path().join("policy-pi.pid");
    let provider = PiProvider::new(policy_pi_fixture(&marker))
        .with_version_supplier(policy_version_supplier());
    // 首次 append 即失败（provider_start 写入失败）。
    let sink = RecordingToolPolicyAuditSink::failing_after(0);

    let Err(error) = provider
        .start(
            policy_pi_input(None, Some(sink.clone().bound())),
            CancellationToken::new(),
        )
        .await
    else {
        panic!("provider_start append failure must fail the session");
    };
    assert!(
        error.details.contains("provider_start audit append failed"),
        "unexpected error: {}",
        error.details
    );

    // adapter 在返回前终止子进程（kill 链）：marker 已写则等待进程退出；
    // marker 未写则子进程在登记前即被终止。
    if let Some(pid) = read_marker_pid(&marker) {
        assert_process_exits(pid);
    }
}

#[test]
fn pi_policy_input_fixture_keeps_env_vars_bounded() {
    // 静态守卫：测试 helper 不携带环境注入（避免 fixture 漂移）。
    let input = policy_pi_input(None, None);
    assert_eq!(input.env_vars, BTreeMap::new());
    assert_eq!(input.timeout_secs, 60);
    assert!(input.permission_mode == ProviderPermissionMode::Auto);
    assert_eq!(input.provider_type, ProviderType::Pi);
    let _ = Ordering::SeqCst;
}

// ==== Task 4b:Pi LC 权限投影与统一 launch audit ====

use crate::cross_cutting::pi_provider::{PiPolicyProjector, projection};
use crate::product::logical_codebase::policy::{
    PolicyTarget, ProviderDialect, ProviderWireDialect, SessionPolicyAction, SessionPolicyEnvelope,
};
use crate::product::logical_codebase::provider_gateway::ProviderRef;
use crate::product::logical_codebase::provider_projection::{
    ProviderPolicyProjector, ProviderProjectionInput,
};

fn lc_projection_envelope(
    action: SessionPolicyAction,
    target_worktree: PathBuf,
    config_artifact_ref: &str,
    config_digest: &str,
) -> SessionPolicyEnvelope {
    let writable_roots = match action {
        SessionPolicyAction::CodingTargetWrite => vec![target_worktree.clone()],
        SessionPolicyAction::PlanningReadOnly | SessionPolicyAction::ReviewReadOnly => Vec::new(),
    };
    SessionPolicyEnvelope {
        policy_id: "policy-lc-0001".to_string(),
        policy_revision: 1,
        policy_digest: "sha256:policy-lc-0001".to_string(),
        action,
        target: PolicyTarget::checkout("logical_repo_0001", "checkout_0001", target_worktree),
        working_directory: PathBuf::from("/lc/lc-root"),
        readable_roots: vec![PathBuf::from("/lc/lc-root")],
        writable_roots,
        provider_dialect: ProviderDialect::PiRpcV1,
        config_artifact_ref: config_artifact_ref.to_string(),
        config_digest: config_digest.to_string(),
        created_at: "2026-10-03T00:00:00Z".to_string(),
        authority_root: PathBuf::from("/lc/lc-root"),
    }
}

fn lc_projection_input(
    envelope: SessionPolicyEnvelope,
    role: AdapterRole,
    tool_policy: Option<ProviderToolPolicy>,
    trust_digest: &str,
) -> ProviderProjectionInput {
    ProviderProjectionInput::new(
        envelope.clone(),
        ProviderRef::pi("cap_pi_lc_fixture"),
        envelope.action,
        role,
        crate::cross_cutting::streaming_provider::ProviderPermissionMode::Auto,
        tool_policy,
        projection::PI_LC_APPROVAL_POLICY.to_string(),
        String::new(),
        "sha256:cfg-a".to_string(),
        trust_digest.to_string(),
        None,
    )
}

/// Task 4b Step 1(断言组 305 逐字 + 300-301 写面):target/role/tool/config
/// 任一漂移都会改变会话全投影 digest;capability profile 摘要按「证据摘要
/// 分层」不随单次 role/config 变化。Coding 投影恰一个可写 target(root/
/// 其它成员不可写);read-only action 无任何可写根。
#[test]
fn lcg_t04_projection_digest_changes_on_target_role_tool_or_config() {
    let projector = PiPolicyProjector::new("pi 0.83.0-lc-fixture");
    let deny = ProviderToolPolicy::deny_file_write_builtins();

    let baseline_input = lc_projection_input(
        lc_projection_envelope(
            SessionPolicyAction::PlanningReadOnly,
            PathBuf::from("/lc/member-a"),
            "sha256:cfg-a",
            "sha256:cfg-digest-a",
        ),
        AdapterRole::Reviewer,
        Some(deny.clone()),
        "sha256:trust-1",
    );
    let baseline = projector
        .project(&baseline_input)
        .expect("pi lc planning projection is produced");
    let original_projection_digest = baseline.projection_digest().to_string();
    let original_capability_digest = baseline.capability_projection_digest().to_string();

    // digest 形状:sha256: 前缀 + 64 位小写 hex(与 2c shape validator 同构)。
    assert_eq!(original_projection_digest.len(), 71);
    assert!(original_projection_digest.starts_with("sha256:"));
    assert_eq!(original_capability_digest.len(), 71);
    assert!(original_capability_digest.starts_with("sha256:"));

    // read-only action:cwd 冻结为 canonical root,无任何可写根。
    assert_eq!(
        baseline.working_directory(),
        std::path::Path::new("/lc/lc-root")
    );
    assert!(baseline.writable_roots().is_empty());
    assert_eq!(baseline.wire_dialect(), ProviderWireDialect::PiRpc);
    assert_eq!(baseline.exact_version(), "pi 0.83.0-lc-fixture");
    assert_eq!(
        baseline.approval_policy(),
        projection::PI_LC_APPROVAL_POLICY
    );

    // 1) target 漂移 → 会话 digest 变化。
    let changed = projector
        .project(&lc_projection_input(
            lc_projection_envelope(
                SessionPolicyAction::PlanningReadOnly,
                PathBuf::from("/lc/member-b"),
                "sha256:cfg-a",
                "sha256:cfg-digest-a",
            ),
            AdapterRole::Reviewer,
            Some(deny.clone()),
            "sha256:trust-1",
        ))
        .expect("projection with drifted target");
    assert_ne!(original_projection_digest, changed.projection_digest());

    // 2) role 漂移 → 会话 digest 变化;profile 摘要不随单次 role 变化。
    let changed = projector
        .project(&lc_projection_input(
            lc_projection_envelope(
                SessionPolicyAction::PlanningReadOnly,
                PathBuf::from("/lc/member-a"),
                "sha256:cfg-a",
                "sha256:cfg-digest-a",
            ),
            AdapterRole::Orchestrator,
            Some(deny.clone()),
            "sha256:trust-1",
        ))
        .expect("projection with drifted role");
    assert_ne!(original_projection_digest, changed.projection_digest());
    assert_eq!(
        original_capability_digest,
        changed.capability_projection_digest()
    );

    // 3) tool policy 漂移(Some→None)→ 会话 digest 变化。
    let changed = projector
        .project(&lc_projection_input(
            lc_projection_envelope(
                SessionPolicyAction::PlanningReadOnly,
                PathBuf::from("/lc/member-a"),
                "sha256:cfg-a",
                "sha256:cfg-digest-a",
            ),
            AdapterRole::Reviewer,
            None,
            "sha256:trust-1",
        ))
        .expect("projection without generic tool policy");
    assert_ne!(original_projection_digest, changed.projection_digest());
    assert!(changed.tool_policy().is_none());

    // 4) config 漂移 → 会话 digest 变化;profile 摘要不含 config,保持不变。
    let changed = projector
        .project(&lc_projection_input(
            lc_projection_envelope(
                SessionPolicyAction::PlanningReadOnly,
                PathBuf::from("/lc/member-a"),
                "sha256:cfg-b",
                "sha256:cfg-digest-b",
            ),
            AdapterRole::Reviewer,
            Some(deny.clone()),
            "sha256:trust-1",
        ))
        .expect("projection with drifted config");
    assert_ne!(original_projection_digest, changed.projection_digest());
    assert_eq!(
        original_capability_digest,
        changed.capability_projection_digest()
    );

    // Coding 投影(断言组 300-301):恰一个可写 root=target worktree;
    // root 与其它成员均不可写;boundary 引用非空。
    let coding = projector
        .project(&lc_projection_input(
            lc_projection_envelope(
                SessionPolicyAction::CodingTargetWrite,
                PathBuf::from("/lc/member-a"),
                "sha256:cfg-a",
                "sha256:cfg-digest-a",
            ),
            AdapterRole::Executor,
            None,
            "sha256:trust-1",
        ))
        .expect("pi lc coding projection is produced");
    assert_eq!(coding.writable_roots(), [PathBuf::from("/lc/member-a")]);
    assert!(!coding.writable_roots().iter().any(
        |root| root == &PathBuf::from("/lc/lc-root") || root == &PathBuf::from("/lc/member-b")
    ));
    assert!(!coding.boundary_evidence_ref().is_empty());
    assert_eq!(coding.sandbox(), "target-write-only");
    assert_eq!(baseline.sandbox(), "read-only");

    // profile 摘要随 tool 控制规范整体变化(exclude-tools 序列属于 profile)。
    let coding_capability = coding.capability_projection_digest();
    assert_ne!(original_capability_digest, coding_capability);
}

/// Task 4b Step 1(断言组 299-303 的 Pi 对照形态):无通用 tool policy 的
/// LC Coding 启动(Executor)同样执行 exact version 解析、原生会话(id
/// 预生成)与统一 `ProviderStartAudit.lc_projection` 落盘;进程 cwd=
/// canonical root。
#[cfg(unix)]
#[tokio::test]
async fn lcg_t04_no_generic_tool_policy_still_records_version_audit_and_native_session() {
    let fixture = LcLaunchFixture::new();
    let sink = RecordingToolPolicyAuditSink::new();
    let marker = fixture.paths.root().join("lc-cwd-marker");
    let mut raw = fixture.lc_streaming_input(
        AdapterRole::Executor,
        None,
        Some(sink.clone().bound()),
        None,
    );
    raw.env_vars
        .insert("LC_CWD_MARKER".to_string(), marker.display().to_string());

    let provider = PiProvider::new(lc_cwd_pi_fixture(&marker))
        .with_version_supplier(policy_version_supplier());

    let session = provider
        .start_validated(
            fixture.validated_coding_input(raw),
            CancellationToken::new(),
        )
        .await
        .expect("lc validated start succeeds without generic tool policy");

    // 原生会话 id 来自预生成握手(无 tool_policy 也不跳过)。
    let native_id = session
        .native_session_id
        .clone()
        .expect("pi lc validated session must carry a pre-generated native session id");
    assert!(!native_id.trim().is_empty());

    let events = sink.events();
    assert_eq!(events.len(), 1, "exactly one provider_start is written");
    let DurableToolPolicyEvent::ProviderStart(record) = &events[0] else {
        panic!("expected provider_start");
    };
    assert_eq!(record.provider, "pi");
    assert_eq!(record.provider_version, "pi 0.83.0-policy-fixture");
    assert_eq!(record.adapter_dialect, PI_POLICY_DIALECT);
    assert_eq!(record.provider_session_id, native_id);
    assert_eq!(record.role, "executor");
    assert_eq!(record.workspace_session_id, "ws-lc-fixture-1");
    // 无通用 tool policy 也有 canonical digest(空 token 序列的 canonical 形态)。
    assert!(!record.tool_policy_canonical_digest.is_empty());
    // argv:rpc 模式与 extension 在;exclude-tools 不在(Executor/Coding 无
    // 通用策略)。
    assert!(record.argv.windows(2).any(|p| p == ["--mode", "rpc"]));
    assert!(record.argv.iter().any(|arg| arg == "-e"));
    assert!(!record.argv.contains(&"--exclude-tools".to_string()));
    assert!(
        record
            .argv
            .windows(2)
            .any(|p| p == ["--session-id", native_id.as_str()])
    );

    // 统一 lc_projection 落盘:分层双 digest + wire/action/boundary 引用。
    let lc_projection = record
        .lc_projection
        .as_ref()
        .expect("lc projection audit is recorded even without generic tool policy");
    assert_eq!(lc_projection.action, "coding_target_write");
    assert_eq!(lc_projection.wire_dialect, "pi-rpc");
    assert!(lc_projection.projection_digest.starts_with("sha256:"));
    assert!(
        lc_projection
            .capability_projection_digest
            .starts_with("sha256:")
    );
    assert!(!lc_projection.boundary_evidence_ref.is_empty());

    // 进程 cwd = canonical LC root(envelope 冻结,不是 target worktree)。
    let observed_cwd = wait_for_lc_cwd_marker(&marker);
    assert_eq!(
        observed_cwd.trim(),
        fixture.canonical_root().to_string_lossy(),
        "provider process must spawn at the canonical LC root"
    );
}

// ==== Task 9a:LC 显式 resume 的 child 前全 LC audit 比对 ====

use crate::cross_cutting::tool_policy_audit::{
    LcProjectionAudit, ProviderStartAudit, ResumeDecision,
};

/// 以与 adapter 同源的材料(gateway validate 的 envelope + 相同投影输入)
/// 预计算 LC 启动会得到的投影摘要,供存档记录构造。
fn t09a_expected_lc_projection(
    fixture: &LcLaunchFixture,
) -> crate::product::logical_codebase::provider_projection::ProviderPolicyProjection {
    let validated = fixture
        .gateway()
        .validate(fixture.coding_request())
        .expect("lc coding launch validates");
    let envelope = validated.envelope().clone();
    let boundary = projection::lc_boundary_plan(&envelope).expect("boundary plan");
    let projection_input =
        crate::product::logical_codebase::provider_projection::ProviderProjectionInput::new(
            envelope.clone(),
            crate::product::logical_codebase::provider_gateway::ProviderRef::pi(
                validated.capability_snapshot_ref(),
            ),
            envelope.action,
            AdapterRole::Executor,
            ProviderPermissionMode::Auto,
            None,
            projection::PI_LC_APPROVAL_POLICY.to_string(),
            String::new(),
            envelope.config_artifact_ref.clone(),
            String::new(),
            Some(boundary),
        );
    PiPolicyProjector::new("pi 0.83.0-policy-fixture")
        .project(&projection_input)
        .expect("expected lc projection")
}

/// Task 9a:LC 显式 resume 在 child 前比较全 LC audit(完整字面量含投影)。
/// 匹配的完整 LC 存档照常续接(native id 保持、child 正常启动);旧 audit
/// 缺 projection 或投影摘要漂移都拒绝 resume——拒绝发生在
/// ProcessManager::spawn 之前(零 child、零新 provider_start),漂移存档
/// 追加 superseded 终止审计;绝不在 adapter 内清 resume id 后静默 fresh。
#[cfg(unix)]
#[tokio::test]
async fn lcg_t09a_pi_lc_resume_audit_gate_zero_child_on_legacy_or_drift() {
    let fixture = LcLaunchFixture::new();
    let expected = t09a_expected_lc_projection(&fixture);
    let matching_lc_projection = LcProjectionAudit {
        action: projection::action_text(expected.action()).to_string(),
        wire_dialect: projection::wire_dialect_text(expected.wire_dialect()).to_string(),
        capability_projection_digest: expected.capability_projection_digest().to_string(),
        projection_digest: expected.projection_digest().to_string(),
        boundary_evidence_ref: expected.boundary_evidence_ref().to_string(),
    };
    let stored_record = |lc_projection: Option<LcProjectionAudit>| ProviderStartAudit {
        provider: "pi".to_string(),
        role: "executor".to_string(),
        workspace_session_id: "ws-lc-fixture-1".to_string(),
        provider_session_id: "pi-session-resume-t09a".to_string(),
        tool_policy_canonical_digest: "sha256:seed".to_string(),
        argv: Vec::new(),
        sandbox: None,
        approval_policy: None,
        provider_version: "pi 0.83.0-policy-fixture".to_string(),
        adapter_dialect: PI_POLICY_DIALECT.to_string(),
        lc_projection,
    };

    // 1)匹配的完整 LC 存档 → 续接:9c 起续接由 RPC get_state 真实确认同 id
    //    (fixture 以 PI_SESSION_ID 应答),native id 即 resume id,child 正常
    //    启动于 canonical root。
    {
        let sink = RecordingToolPolicyAuditSink::new();
        sink.with_stored_provider_start(stored_record(Some(matching_lc_projection.clone())));
        let marker = fixture.paths.root().join("t09a-pi-matching-cwd-marker");
        let mut raw = fixture.lc_streaming_input(
            AdapterRole::Executor,
            None,
            Some(sink.clone().bound()),
            Some("pi-session-resume-t09a".to_string()),
        );
        // marker 经 env 注入(lc_pi_rpc_resume_fixture 读取 $LC_CWD_MARKER)。
        raw.env_vars
            .insert("LC_CWD_MARKER".to_string(), marker.display().to_string());
        raw.env_vars.insert(
            "PI_SESSION_ID".to_string(),
            "pi-session-resume-t09a".to_string(),
        );
        let provider = PiProvider::new(lc_pi_rpc_resume_fixture(
            marker
                .parent()
                .expect("marker parent hosts the rpc fixture"),
        ))
        .with_version_supplier(policy_version_supplier());
        let session = provider
            .start_validated(
                fixture.validated_coding_input(raw),
                CancellationToken::new(),
            )
            .await
            .expect("matching full LC audit must resume the native session");
        assert_eq!(
            session.native_session_id.as_deref(),
            Some("pi-session-resume-t09a")
        );
        // 续接确实启动了 child(正向观测)。
        let observed_cwd = wait_for_lc_cwd_marker(&marker);
        assert_eq!(
            observed_cwd.trim(),
            fixture.canonical_root().to_string_lossy(),
            "resumed pi session must spawn at the canonical LC root"
        );
        let events = sink.events();
        assert_eq!(
            events.len(),
            1,
            "resumed run writes exactly one provider_start"
        );
        assert!(matches!(
            &events[0],
            DurableToolPolicyEvent::ProviderStart(record)
                if record.provider_session_id == "pi-session-resume-t09a"
                    && record.lc_projection.is_some()
        ));
    }

    // 2)旧 audit 缺 projection → 不能 resume LC;投影摘要漂移同理:
    // 零 child、零新 provider_start,不清 id 转 fresh。
    for (case, record) in [
        ("legacy", stored_record(None)),
        (
            "drifted",
            stored_record(Some(LcProjectionAudit {
                projection_digest: "sha256:session-projection-drifted".to_string(),
                ..matching_lc_projection.clone()
            })),
        ),
    ] {
        let sink = RecordingToolPolicyAuditSink::new();
        sink.with_stored_provider_start(record);
        let marker = fixture
            .paths
            .root()
            .join(format!("t09a-pi-{case}-cwd-marker"));
        let mut raw = fixture.lc_streaming_input(
            AdapterRole::Executor,
            None,
            Some(sink.clone().bound()),
            Some("pi-session-resume-t09a".to_string()),
        );
        raw.env_vars
            .insert("LC_CWD_MARKER".to_string(), marker.display().to_string());
        let provider = PiProvider::new(lc_cwd_pi_fixture(&marker))
            .with_version_supplier(policy_version_supplier());

        let rejected = match provider
            .start_validated(
                fixture.validated_coding_input(raw),
                CancellationToken::new(),
            )
            .await
        {
            Err(error) => error,
            Ok(_) => panic!("legacy or drifted LC audit must refuse resume ({case})"),
        };
        assert!(
            rejected.details.contains("resume audit"),
            "rejection must name the resume audit gate: {rejected:?}"
        );
        // 零 child:拒绝发生在 ProcessManager::spawn 之前。
        assert!(
            !marker.exists(),
            "no pi child may spawn after a refused LC resume ({case})"
        );
        // 零新 provider_start;legacy 与漂移存档同样被标记 superseded(旧 run
        // 追加终止审计,不启动 fresh provider_start)。
        let events = sink.events();
        assert_eq!(
            events.len(),
            1,
            "{case} record is marked superseded without a fresh provider_start"
        );
        assert!(
            matches!(
                &events[0],
                DurableToolPolicyEvent::SessionTerminated(terminated)
                    if terminated.reason_code == "superseded_policy_drift"
            ),
            "{case} resume must mark the old run superseded"
        );
    }
}

// ==== Task 9c:Pi LC 原生 resume 的 get_state 真实同 id 确认(错/缺 id 绝不 fresh)====

/// Task 9c fixture(fake pi RPC):`--version` 打印兼容版本;rpc 循环前把进程
/// `pwd -P` 写入 `LC_CWD_MARKER`、pid 写入 `PI_PID_MARKER`(kill/reap 断言);
/// rpc 循环内按 `PI_SESSION_ID` 应答 get_state(非空 → `data.sessionId`;
/// 空 → success 但无 sessionId),prompt 应答后送 text_delta 与
/// agent_settled;不自行退出——终止必须来自 adapter 的 kill 链。
#[cfg(unix)]
fn lc_pi_rpc_resume_fixture(dir: &std::path::Path) -> PathBuf {
    write_executable(
        dir,
        "fake-lc-pi-rpc-resume",
        r#"#!/usr/bin/env bash
if [ "$1" = "--version" ]; then echo 0.83.0; exit 0; fi
pwd -P > "${LC_CWD_MARKER:-/dev/null}"
echo $$ > "${PI_PID_MARKER:-/dev/null}"
while IFS= read -r line; do
  id="$(printf '%s' "$line" | sed -n 's/.*"id"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')"
  if [[ "$line" == *'"get_state"'* ]]; then
    if [[ -n "${PI_SESSION_ID:-}" ]]; then
      echo "{\"type\":\"response\",\"id\":\"${id:-pi-1}\",\"command\":\"get_state\",\"success\":true,\"data\":{\"sessionId\":\"${PI_SESSION_ID}\"}}"
    else
      echo "{\"type\":\"response\",\"id\":\"${id:-pi-1}\",\"command\":\"get_state\",\"success\":true,\"data\":{}}"
    fi
  elif [[ "$line" == *'"prompt"'* ]]; then
    echo "{\"type\":\"response\",\"id\":\"${id:-pi-2}\",\"success\":true}"
    echo '{"type":"message_update","assistantMessageEvent":{"type":"text_delta","contentIndex":0,"delta":"lc resume done"}}'
    echo '{"type":"agent_settled"}'
  fi
done
"#,
    )
}

/// Task 9c kill 链断言辅助:轮询读取 fixture 登记的子进程 pid,再轮询确认该
/// pid 已从进程表消失(kill+reap 都发生后 zombie 不复存在)。
#[cfg(unix)]
fn t09c_child_killed_and_reaped(pid_marker: &std::path::Path) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let pid = loop {
        if let Some(pid) = std::fs::read_to_string(pid_marker)
            .ok()
            .and_then(|content| content.trim().parse::<u32>().ok())
        {
            break pid;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "fixture must register its pid at {}",
            pid_marker.display()
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    loop {
        let alive = std::process::Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false);
        if !alive {
            return true;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "pi child (pid {pid}) must be killed and reaped after an unconfirmed native resume"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

/// 有界等待终态事件:Completed 返回产物全文,Failed 即 panic(5s 上界)。
#[cfg(unix)]
async fn t09c_recv_pi_completed_text(
    events: &mut tokio::sync::mpsc::Receiver<ProviderEvent>,
) -> String {
    let deadline = std::time::Duration::from_secs(5);
    loop {
        let event = tokio::time::timeout(deadline, events.recv())
            .await
            .expect("pi session must reach a terminal event within timeout")
            .expect("pi session event channel stays open");
        match event {
            ProviderEvent::Completed(completion) => return completion.full_output,
            ProviderEvent::Failed { message } => {
                panic!("confirmed pi resume must complete, failed instead: {message}")
            }
            _ => {}
        }
    }
}

/// Task 9c(Step 1 断言组 438-440 逐字):Pi LC 显式 resume 的原生确认——
/// `--session-id` 传入后必须由 RPC `get_state` 应答真实核对同 id:
/// - 应答同 id(无存档记录,9a 的「无存档→依赖 native 握手」由本测试补
///   真实确认)→ 续接,confirmed==requested,provider_start 落确认 id;
/// - 缺 id(get_state success 但无 `data.sessionId`)→ Err;错 id(应答不同
///   会话 id)→ Err——两者都是已启动 child 后的 runtime 失败:child 被
///   kill/reap、错误记录「未恢复」(不伪称零 spawn),不回填请求 id、不清
///   id 转 fresh、零新 provider_start。
#[cfg(unix)]
#[tokio::test]
async fn lcg_t09_pi_missing_or_wrong_native_id_never_fresh() {
    let fixture = LcLaunchFixture::new();
    let requested_native_id = "pi-session-resume-t09c".to_string();

    // 1) get_state 应答同 id(无存档记录):真实确认后续接,provider_start
    //    落确认 id,握手消耗 get_state 行后会话流照常完成。
    {
        let marker_dir = tempfile::tempdir().expect("t09c matching marker dir");
        let sink = RecordingToolPolicyAuditSink::new();
        let mut raw = fixture.lc_streaming_input(
            AdapterRole::Executor,
            None,
            Some(sink.clone().bound()),
            Some(requested_native_id.clone()),
        );
        raw.env_vars
            .insert("PI_SESSION_ID".to_string(), requested_native_id.clone());
        let provider = PiProvider::new(lc_pi_rpc_resume_fixture(marker_dir.path()))
            .with_version_supplier(policy_version_supplier());
        let mut session = provider
            .start_validated(
                fixture.validated_coding_input(raw),
                CancellationToken::new(),
            )
            .await
            .expect("a native-confirmed resume must continue the LC session");
        let confirmed_native_id = session.native_session_id.clone().unwrap_or_default();
        assert_eq!(confirmed_native_id, requested_native_id);
        let events = sink.events();
        assert_eq!(
            events.len(),
            1,
            "confirmed resume writes exactly one provider_start"
        );
        assert!(matches!(
            &events[0],
            DurableToolPolicyEvent::ProviderStart(record)
                if record.provider_session_id == requested_native_id
        ));
        let completed = t09c_recv_pi_completed_text(&mut session.events).await;
        assert_eq!(completed, "lc resume done");
    }

    // 2) 缺 id:get_state success 但应答无 sessionId → runtime 失败:Err +
    //    kill/reap + 「未恢复」记录,零新 provider_start。
    {
        let marker_dir = tempfile::tempdir().expect("t09c missing-id marker dir");
        let pid_marker = marker_dir.path().join("pi-t09c-missing-id.pid");
        let sink = RecordingToolPolicyAuditSink::new();
        let mut raw = fixture.lc_streaming_input(
            AdapterRole::Executor,
            None,
            Some(sink.clone().bound()),
            Some(requested_native_id.clone()),
        );
        raw.env_vars
            .insert("PI_SESSION_ID".to_string(), String::new());
        raw.env_vars.insert(
            "PI_PID_MARKER".to_string(),
            pid_marker.display().to_string(),
        );
        let provider = PiProvider::new(lc_pi_rpc_resume_fixture(marker_dir.path()))
            .with_version_supplier(policy_version_supplier());
        let native_id_missing_result = provider
            .start_validated(
                fixture.validated_coding_input(raw),
                CancellationToken::new(),
            )
            .await;
        assert!(native_id_missing_result.is_err());
        let Err(rejected) = native_id_missing_result else {
            panic!("unconfirmed resume must fail");
        };
        assert!(
            rejected.details.contains("session NOT resumed")
                && rejected.details.contains("not a zero-spawn refusal"),
            "rejection must record the not-resumed outcome without claiming zero spawn: {rejected:?}"
        );
        assert!(
            sink.events().is_empty(),
            "no fresh provider_start may be written for a resume the native side never confirmed"
        );
        let started_child_was_killed_and_reaped = t09c_child_killed_and_reaped(&pid_marker);
        assert!(started_child_was_killed_and_reaped);
    }

    // 3) 错 id:get_state 应答不同的原生会话 id → 同为 runtime 失败:Err +
    //    kill/reap,绝不采纳陌生 id 续接、绝不清请求 id 转 fresh。
    {
        let marker_dir = tempfile::tempdir().expect("t09c wrong-id marker dir");
        let pid_marker = marker_dir.path().join("pi-t09c-wrong-id.pid");
        let sink = RecordingToolPolicyAuditSink::new();
        let wrong_native_id = "pi-session-native-wrong-t09c".to_string();
        let mut raw = fixture.lc_streaming_input(
            AdapterRole::Executor,
            None,
            Some(sink.clone().bound()),
            Some(requested_native_id.clone()),
        );
        raw.env_vars
            .insert("PI_SESSION_ID".to_string(), wrong_native_id.clone());
        raw.env_vars.insert(
            "PI_PID_MARKER".to_string(),
            pid_marker.display().to_string(),
        );
        let provider = PiProvider::new(lc_pi_rpc_resume_fixture(marker_dir.path()))
            .with_version_supplier(policy_version_supplier());
        let wrong_native_id_result = provider
            .start_validated(
                fixture.validated_coding_input(raw),
                CancellationToken::new(),
            )
            .await;
        let Err(rejected) = wrong_native_id_result else {
            panic!("a wrong native id must fail the resume");
        };
        assert!(
            rejected.details.contains("pi-session-native-wrong-t09c")
                && rejected.details.contains(&requested_native_id)
                && rejected.details.contains("session NOT resumed"),
            "rejection must name both ids and record the not-resumed outcome: {rejected:?}"
        );
        assert!(
            sink.events().is_empty(),
            "no fresh provider_start may be written for a mismatched native resume confirmation"
        );
        let started_child_was_killed_and_reaped = t09c_child_killed_and_reaped(&pid_marker);
        assert!(started_child_was_killed_and_reaped);
    }
}
