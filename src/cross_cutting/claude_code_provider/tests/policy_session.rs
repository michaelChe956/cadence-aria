// Task 3.2（REQ-ENV-09/D7）：claude 策略会话的有界握手（fresh=等首个
// system/init 事件；resume 已知）、provider_start durable 写入、append 失败 kill
// 链、握手超时与缺 sink fail-closed 测试。

use tokio_util::sync::CancellationToken;

use crate::cross_cutting::streaming_provider::{
    ProviderEvent, ProviderToolPolicy, ProviderVersionSupplier, StreamingProviderAdapter,
    StreamingProviderInput,
};
use crate::cross_cutting::tool_policy_audit::test_support::RecordingToolPolicyAuditSink;
use crate::cross_cutting::tool_policy_audit::{DurableToolPolicyEvent, ToolPolicyAuditSink};
use crate::protocol::contracts::AdapterRole;

use super::*;

fn policy_version_supplier() -> ProviderVersionSupplier {
    std::sync::Arc::new(|| Ok("claude 1.0.99-policy-fixture".to_string()))
}

fn policy_claude_input(
    resume_id: Option<String>,
    audit_sink: Option<std::sync::Arc<dyn ToolPolicyAuditSink>>,
) -> StreamingProviderInput {
    let mut input = streaming_input(
        crate::protocol::contracts::ProviderType::ClaudeCode,
        crate::cross_cutting::streaming_provider::ProviderPermissionMode::Auto,
    );
    input.role = AdapterRole::Reviewer;
    input.tool_policy = Some(ProviderToolPolicy::deny_file_write_builtins());
    input.audit_sink = audit_sink;
    if let Some(resume_id) = resume_id {
        input.resume_provider_session_id = Some(resume_id);
    }
    input
}

/// 初始化即完成的策略 fixture：收到首条 user 消息后输出 system/init 与 result。
fn init_then_result_fixture() -> PathBuf {
    write_fixture(
        "claude_policy_init_fixture.sh",
        "#!/usr/bin/env bash\nwhile IFS= read -r line; do\n  if [[ \"$line\" == *'\"type\":\"user\"'* ]]; then\n    echo '{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"sess-policy-1\"}'\n    echo '{\"type\":\"result\",\"subtype\":\"success\",\"result\":\"policy done\",\"session_id\":\"sess-policy-1\"}'\n    exit 0\n  fi\ndone\n",
    )
}

/// 挂起 fixture：收到 user 消息后永不输出 init（验证有界握手超时 fail-closed）。
fn hanging_fixture() -> PathBuf {
    write_fixture(
        "claude_policy_hanging_fixture.sh",
        "#!/usr/bin/env bash\nwhile IFS= read -r line; do\n  if [[ \"$line\" == *'\"type\":\"user\"'* ]]; then\n    sleep 30\n    exit 0\n  fi\ndone\n",
    )
}

async fn recv_completed(events: &mut mpsc::Receiver<ProviderEvent>) -> String {
    loop {
        match tokio::time::timeout(TEST_TIMEOUT, events.recv())
            .await
            .expect("provider should emit completion")
            .expect("provider event channel should stay open")
        {
            ProviderEvent::Completed(completion) => return completion.full_output,
            ProviderEvent::Failed { message } => panic!("provider failed: {message}"),
            _ => {}
        }
    }
}

#[cfg(unix)]
#[tokio::test]
async fn claude_policy_start_waits_for_init_writes_provider_start_and_returns_native_id() {
    let sink = RecordingToolPolicyAuditSink::new();
    let provider = ClaudeCodeProvider::new(init_then_result_fixture())
        .with_version_supplier(policy_version_supplier());
    let mut session = provider
        .start(
            policy_claude_input(None, Some(sink.clone().bound())),
            CancellationToken::new(),
        )
        .await
        .expect("claude policy session starts");

    // fresh 策略会话：native id 来自有界等待的首个 system/init 事件。
    assert_eq!(session.native_session_id.as_deref(), Some("sess-policy-1"));

    let events = sink.events();
    assert_eq!(events.len(), 1, "only provider_start is written at start");
    let DurableToolPolicyEvent::ProviderStart(record) = &events[0] else {
        panic!("expected provider_start");
    };
    assert_eq!(record.provider, "claude-code");
    assert_eq!(record.adapter_dialect, "claude-stream-json");
    assert_eq!(record.provider_version, "claude 1.0.99-policy-fixture");
    assert_eq!(record.provider_session_id, "sess-policy-1");
    assert!(record.argv.contains(&"--disallowedTools".to_string()));
    assert!(record.argv.contains(&"Edit,Write,NotebookEdit".to_string()));
    assert!(!record.tool_policy_canonical_digest.is_empty());
    assert_eq!(record.sandbox, None);
    assert_eq!(record.approval_policy, None);

    // 握手消耗了 init 行，但流式续读不受影响：result 仍可送达。
    assert_eq!(recv_completed(&mut session.events).await, "policy done");
}

#[cfg(unix)]
#[tokio::test]
async fn claude_policy_resume_uses_known_native_id_without_waiting_for_init() {
    let sink = RecordingToolPolicyAuditSink::new();
    // Task 3.3：resume 前置冻结三元组比对——预置一致记录使决策为 Resume。
    let canonical = crate::cross_cutting::streaming_provider::canonical_tool_policy(
        crate::cross_cutting::claude_code_provider::TOOL_POLICY_PROVIDER_NAME,
        &ProviderToolPolicy::deny_file_write_builtins(),
    )
    .expect("canonical policy");
    sink.with_stored_provider_start(
        crate::cross_cutting::tool_policy_audit::ProviderStartAudit {
            provider: "claude-code".to_string(),
            workspace_session_id: "ws-test".to_string(),
            role: "reviewer".to_string(),
            tool_policy_canonical_digest: canonical.digest,
            argv: Vec::new(),
            sandbox: None,
            approval_policy: None,
            provider_version: "claude 1.0.99-policy-fixture".to_string(),
            adapter_dialect: "claude-stream-json".to_string(),
            provider_session_id: "claude-session-resume-policy".to_string(),
        },
    );
    let provider = ClaudeCodeProvider::new(init_then_result_fixture())
        .with_version_supplier(policy_version_supplier());
    let mut session = provider
        .start(
            policy_claude_input(
                Some("claude-session-resume-policy".to_string()),
                Some(sink.clone().bound()),
            ),
            CancellationToken::new(),
        )
        .await
        .expect("claude policy resume session starts");

    // resume 已知：native id 即 resume id，不等 init。
    assert_eq!(
        session.native_session_id.as_deref(),
        Some("claude-session-resume-policy")
    );
    let DurableToolPolicyEvent::ProviderStart(record) = &sink.events()[0] else {
        panic!("expected provider_start");
    };
    assert_eq!(record.provider_session_id, "claude-session-resume-policy");
    assert!(record.argv.contains(&"--resume".to_string()));
    assert!(
        record
            .argv
            .contains(&"claude-session-resume-policy".to_string())
    );
    assert_eq!(recv_completed(&mut session.events).await, "policy done");
}

#[cfg(unix)]
#[tokio::test]
async fn claude_policy_start_without_sink_or_version_fails_closed() {
    let provider = ClaudeCodeProvider::new(init_then_result_fixture());
    let Err(error) = provider
        .start(policy_claude_input(None, None), CancellationToken::new())
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
    let provider = ClaudeCodeProvider::new(init_then_result_fixture()).with_version_supplier(
        std::sync::Arc::new(|| {
            Err(crate::cross_cutting::streaming_provider::VersionProbeError::Unavailable)
        }),
    );
    let Err(error) = provider
        .start(
            policy_claude_input(None, Some(RecordingToolPolicyAuditSink::new().bound())),
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
async fn claude_policy_start_times_out_waiting_for_init_and_fails_closed() {
    let sink = RecordingToolPolicyAuditSink::new();
    let provider =
        ClaudeCodeProvider::new(hanging_fixture()).with_version_supplier(policy_version_supplier());
    let Err(error) = provider
        .start(
            policy_claude_input(None, Some(sink.clone().bound())),
            CancellationToken::new(),
        )
        .await
    else {
        panic!("policy session without init must time out and fail closed");
    };
    assert!(
        error.details.contains("handshake")
            && (error.details.contains("timed out")
                || error.details.contains("waiting for init event")),
        "unexpected error: {}",
        error.details
    );
    // 握手失败不落任何 durable 事件。
    assert!(sink.events().is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn claude_policy_start_append_failure_fails_closed() {
    let provider = ClaudeCodeProvider::new(init_then_result_fixture())
        .with_version_supplier(policy_version_supplier());
    // provider_start append 注入失败：cancel 触发任务内 kill 链。
    let sink = RecordingToolPolicyAuditSink::failing_after(0);
    let Err(error) = provider
        .start(
            policy_claude_input(None, Some(sink.clone().bound())),
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
}

/// 带内收集首个 ToolPolicyWarning（带 2s 界限；warning 在 start 返回前已入队）。
async fn recv_tool_policy_warning(
    events: &mut mpsc::Receiver<ProviderEvent>,
) -> Option<crate::cross_cutting::streaming_provider::CodexProtocolWarningEvent> {
    loop {
        match tokio::time::timeout(TEST_TIMEOUT, events.recv()).await {
            Err(_) => return None,
            Ok(None) => return None,
            Ok(Some(ProviderEvent::ToolPolicyWarning(warning))) => return Some(warning),
            Ok(Some(_)) => {}
        }
    }
}

/// GC9：resume 记录缺失（find_provider_start → None）与 drift 同路径处置——
/// 清除 resume id、以全新会话（fresh init 握手+provider_start）启动；「标记 superseded」
/// 仅带内 ToolPolicyWarning（🔴 无旧文件可写，不得伪造无 provider_start 首行的
/// durable 文件）。
#[cfg(unix)]
#[tokio::test]
async fn claude_policy_resume_with_missing_record_starts_fresh_and_warns_in_band() {
    let sink = RecordingToolPolicyAuditSink::new();
    let provider = ClaudeCodeProvider::new(init_then_result_fixture())
        .with_version_supplier(policy_version_supplier());
    let mut session = provider
        .start(
            policy_claude_input(
                Some("claude-session-resume-policy".to_string()),
                Some(sink.clone().bound()),
            ),
            CancellationToken::new(),
        )
        .await
        .expect("missing record must start a fresh session");

    // 全新会话：native id 来自首个 init 事件（非 resume id），argv 不携带 --resume。
    assert_eq!(session.native_session_id.as_deref(), Some("sess-policy-1"));
    let events = sink.events();
    assert_eq!(events.len(), 1, "only the fresh provider_start is written");
    let DurableToolPolicyEvent::ProviderStart(record) = &events[0] else {
        panic!("expected provider_start");
    };
    assert!(!record.argv.contains(&"--resume".to_string()));
    assert_eq!(record.provider_session_id, "sess-policy-1");

    // 带内标记：ToolPolicyWarning(superseded_policy_record_missing)。
    let warning = recv_tool_policy_warning(&mut session.events)
        .await
        .expect("missing record must emit an in-band superseded warning");
    assert_eq!(
        warning.reason_code, "superseded_policy_record_missing",
        "unexpected warning: {warning:?}"
    );
    // 全新会话功能不受影响：result 仍可送达。
    assert_eq!(recv_completed(&mut session.events).await, "policy done");
}

/// 登记 PID 并发 init 后存活的 fixture（kill 链断言用：子进程不自行退出）。
#[cfg(unix)]
fn pid_registering_init_fixture(marker: &std::path::Path) -> PathBuf {
    write_fixture(
        "claude_policy_pid_fixture.sh",
        &format!(
            "#!/usr/bin/env bash\nwhile IFS= read -r line; do\n  if [[ \"$line\" == *'\"type\":\"user\"'* ]]; then\n    echo $$ > {}\n    echo '{{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"sess-policy-1\"}}'\n    while IFS= read -r line; do :; done\n    exit 0\n  fi\ndone\n",
            marker.display()
        ),
    )
}

/// F2（最终审）fixture：登记 pid 后发出携带空白 session_id（空串/全空白）的
/// system/init 事件，随后存活等待 kill 链终止（不自行退出）。
#[cfg(unix)]
fn pid_registering_blank_init_fixture(marker: &std::path::Path, session_id: &str) -> PathBuf {
    write_fixture(
        "claude_policy_blank_init_fixture.sh",
        &format!(
            "#!/usr/bin/env bash\nwhile IFS= read -r line; do\n  if [[ \"$line\" == *'\"type\":\"user\"'* ]]; then\n    echo $$ > {}\n    echo '{{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"{}\"}}'\n    while IFS= read -r line; do :; done\n    exit 0\n  fi\ndone\n",
            marker.display(),
            session_id
        ),
    )
}

/// F2（最终审，红→绿）：`system/init` 的 session_id 为空串/全空白时不是有效
/// 原生会话 id——握手 fail-closed（无成功 ProviderSession 返回）、子进程被 kill、
/// 无 provider_start 落盘（空白 id 不得进入 durable 审计）。旧实现只查字符串
/// 不查 trim 空，会接受空白 id 并写出 provider_start。
#[cfg(unix)]
#[tokio::test]
async fn claude_policy_blank_init_session_id_fails_handshake_and_kills_child() {
    for blank in ["", "   "] {
        let marker_dir = tempfile::tempdir().expect("marker dir");
        let marker = marker_dir.path().join("claude-blank-init-child.pid");
        let provider = ClaudeCodeProvider::new(pid_registering_blank_init_fixture(&marker, blank))
            .with_version_supplier(policy_version_supplier());
        let sink = RecordingToolPolicyAuditSink::new();
        let Err(error) = provider
            .start(
                policy_claude_input(None, Some(sink.clone().bound())),
                CancellationToken::new(),
            )
            .await
        else {
            panic!("blank init session_id ({blank:?}) must fail the policy handshake");
        };
        assert!(
            error.details.contains("session_id"),
            "unexpected error: {}",
            error.details
        );
        assert!(
            sink.events().is_empty(),
            "no provider_start may be written for a blank native session id"
        );
        let pid = std::fs::read_to_string(&marker)
            .expect("fixture registers its pid")
            .trim()
            .parse::<u32>()
            .expect("pid");
        let alive = std::process::Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false);
        assert!(
            !alive,
            "claude policy child (pid {pid}) must be killed for blank init session_id"
        );
    }
}

/// F2（最终审，红→绿）：解析层函数级锁定——`parse_claude_init_session_id` 对
/// 空串/全空白/非字符串 session_id 返回 None，仅接受非空白字符串。
#[test]
fn claude_parse_init_session_id_rejects_blank_and_non_string_ids() {
    use crate::cross_cutting::claude_code_provider::stream::parse_claude_init_session_id;

    let valid = serde_json::json!({
        "type": "system",
        "subtype": "init",
        "session_id": "sess-1",
    });
    assert_eq!(
        parse_claude_init_session_id(&valid),
        Some("sess-1".to_string())
    );
    for blank in ["", "   \t"] {
        let value = serde_json::json!({
            "type": "system",
            "subtype": "init",
            "session_id": blank,
        });
        assert_eq!(
            parse_claude_init_session_id(&value),
            None,
            "blank session_id ({blank:?}) must not parse as a native session id"
        );
    }
    let non_string = serde_json::json!({
        "type": "system",
        "subtype": "init",
        "session_id": 42,
    });
    assert_eq!(parse_claude_init_session_id(&non_string), None);
    let missing = serde_json::json!({"type": "system", "subtype": "init"});
    assert_eq!(parse_claude_init_session_id(&missing), None);
    let not_init = serde_json::json!({
        "type": "system",
        "subtype": "other",
        "session_id": "sess-1",
    });
    assert_eq!(parse_claude_init_session_id(&not_init), None);
}

/// P1-7：append 失败时 adapter 必须同步 kill+wait 后再返回错误——start 返回
/// Err 的瞬间子进程已终止（不等异步 cancel 链）。
#[cfg(unix)]
#[tokio::test]
async fn claude_policy_append_failure_kills_child_before_returning() {
    let marker_dir = tempfile::tempdir().expect("marker dir");
    let marker = marker_dir.path().join("claude-policy-child.pid");
    let provider = ClaudeCodeProvider::new(pid_registering_init_fixture(&marker))
        .with_version_supplier(policy_version_supplier());
    let sink = RecordingToolPolicyAuditSink::failing_after(0);
    let Err(error) = provider
        .start(
            policy_claude_input(None, Some(sink.clone().bound())),
            CancellationToken::new(),
        )
        .await
    else {
        panic!("provider_start append failure must fail the session");
    };
    assert!(error.details.contains("provider_start audit append failed"));
    let pid = std::fs::read_to_string(&marker)
        .expect("fixture registers its pid")
        .trim()
        .parse::<u32>()
        .expect("pid");
    let alive = std::process::Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    assert!(
        !alive,
        "claude policy child (pid {pid}) must be terminated before start returns the append-failure error"
    );
}

/// 登记 PID 但从不读 stdin 的 fixture：巨大的初始 user 消息超过管道缓冲后，
// 初始写入阻塞在不可取消 await（stdin write 不 select cancel）——P1-7 round 2
// 裁决针对的卡死形态。
#[cfg(unix)]
fn pid_registering_blind_fixture(marker: &std::path::Path) -> PathBuf {
    write_fixture(
        "claude_policy_blind_fixture.sh",
        &format!(
            "#!/usr/bin/env bash\necho $$ > {}\nsleep 30\n",
            marker.display()
        ),
    )
}

/// P1-7 round 2（controller 裁决）：策略路径的「子进程所有权+握手+provider_start
/// 写入」留在 start() 内——初始写入卡死在不可取消 await 时，start() 返回错误的
/// 瞬间子进程必须已终止且 exit status 已回收（同步 kill()+wait()，非异步任务
/// 兑底）。256KiB prompt 超过管道缓冲（64KiB），fixture 不读 stdin → 写入阻塞。
#[cfg(unix)]
#[tokio::test]
async fn claude_policy_blocked_initial_write_kills_child_before_returning() {
    let marker_dir = tempfile::tempdir().expect("marker dir");
    let marker = marker_dir.path().join("claude-policy-blind-child.pid");
    let provider = ClaudeCodeProvider::new(pid_registering_blind_fixture(&marker))
        .with_version_supplier(policy_version_supplier());
    let sink = RecordingToolPolicyAuditSink::new();
    let mut input = policy_claude_input(None, Some(sink.clone().bound()));
    input.prompt = "x".repeat(256 * 1024);

    let Err(error) = provider.start(input, CancellationToken::new()).await else {
        panic!("blocked initial write must fail the policy session");
    };
    assert!(
        error.details.contains("timed out") || error.details.is_empty(),
        "unexpected error: {error:?}"
    );

    let pid = std::fs::read_to_string(&marker)
        .expect("fixture registers its pid")
        .trim()
        .parse::<u32>()
        .expect("pid");
    // kill -0 对僵尸（已死未回收）仍返回成功：此处立即失败即证明 start() 已
    // 同步 wait() 回收 exit status，而非仅发出 kill 信号后交给后台任务。
    let alive = std::process::Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    assert!(
        !alive,
        "claude policy child (pid {pid}) must be reaped before start returns the blocked-write error"
    );
}
