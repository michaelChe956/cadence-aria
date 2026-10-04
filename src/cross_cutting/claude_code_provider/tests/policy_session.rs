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

/// claude 2.1.247 实测形态：stdout 首行先吐非 JSON 日志行（
/// `[claude-code:unrecognized_model] ...`），system/init 在第二行。握手必须
/// 容忍此类行直到 deadline（D② latent bug 回归）。
fn init_after_log_line_fixture() -> PathBuf {
    write_fixture(
        "claude_policy_init_after_log_fixture.sh",
        "#!/usr/bin/env bash\nwhile IFS= read -r line; do\n  if [[ \"$line\" == *'\"type\":\"user\"'* ]]; then\n    echo '[claude-code:unrecognized_model] model=claude-sonnet-4-5 fallback applied'\n    echo '{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"sess-policy-2\"}'\n    echo '{\"type\":\"result\",\"subtype\":\"success\",\"result\":\"policy done\",\"session_id\":\"sess-policy-2\"}'\n    exit 0\n  fi\ndone\n",
    )
}

/// D③ fixture：输出一条 stderr 后无 init 即退出（EOF 失败路径，stderr 须随错
/// 误返回）。
fn stderr_then_exit_without_init_fixture() -> PathBuf {
    write_fixture(
        "claude_policy_stderr_exit_fixture.sh",
        "#!/usr/bin/env bash\nwhile IFS= read -r line; do\n  if [[ \"$line\" == *'\"type\":\"user\"'* ]]; then\n    echo '[claude-code:unrecognized_model] stderr evidence line' >&2\n    exit 7\n  fi\ndone\n",
    )
}

/// D③ fixture：写入一条 stderr 后不读 stdin 挂起（外层 bound 超时路径，stderr
/// 须随超时错误返回）。256KiB prompt 超管道缓冲，初始写入阻塞至外层超时。
fn stderr_then_block_stdin_fixture() -> PathBuf {
    write_fixture(
        "claude_policy_stderr_block_fixture.sh",
        "#!/usr/bin/env bash\necho '[claude-code:unrecognized_model] blocked stderr evidence' >&2\nexec sleep 300\n",
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

/// D②（红→绿）：握手首行非 JSON 日志行（`[claude-code:...]` 等）不得立即判
/// invalid Claude init JSON——须跳过继续等待，init 在第二行时握手成功。
#[cfg(unix)]
#[tokio::test]
async fn claude_policy_start_tolerates_non_json_log_lines_before_init() {
    let sink = RecordingToolPolicyAuditSink::new();
    let provider = ClaudeCodeProvider::new(init_after_log_line_fixture())
        .with_version_supplier(policy_version_supplier());
    let mut session = provider
        .start(
            policy_claude_input(None, Some(sink.clone().bound())),
            CancellationToken::new(),
        )
        .await
        .expect("claude policy handshake must skip non-JSON log lines and accept init");

    assert_eq!(session.native_session_id.as_deref(), Some("sess-policy-2"));
    let events = sink.events();
    assert_eq!(events.len(), 1, "only provider_start is written at start");
    let DurableToolPolicyEvent::ProviderStart(record) = &events[0] else {
        panic!("expected provider_start");
    };
    assert_eq!(record.provider_session_id, "sess-policy-2");
    assert_eq!(recv_completed(&mut session.events).await, "policy done");
}

/// D③（红→绿）：握手失败（EOF 无 init）时 stderr 快照须并入错误 details 与
/// stderr 字段，claude 子进程死因可见。
#[cfg(unix)]
#[tokio::test]
async fn claude_policy_handshake_failure_includes_bounded_stderr_snapshot() {
    let provider = ClaudeCodeProvider::new(stderr_then_exit_without_init_fixture())
        .with_version_supplier(policy_version_supplier());
    let sink = RecordingToolPolicyAuditSink::new();

    let Err(error) = provider
        .start(
            policy_claude_input(None, Some(sink.clone().bound())),
            CancellationToken::new(),
        )
        .await
    else {
        panic!("fixture without init must fail the policy handshake");
    };

    assert!(
        error
            .details
            .contains("claude stream ended before init event"),
        "unexpected error: {}",
        error.details
    );
    assert!(
        error.details.contains("stderr evidence line"),
        "handshake failure must carry stderr snapshot in details: {}",
        error.details
    );
    assert!(
        error.stderr.contains("stderr evidence line"),
        "handshake failure must carry stderr snapshot in stderr field"
    );
    // 失败路径不落 durable 事件。
    assert!(sink.events().is_empty());
}

/// D③（红→绿）：外层 bound 超时（初始写入阻塞）路径同样携带 stderr 快照。
#[cfg(unix)]
#[tokio::test]
async fn claude_policy_bounded_start_timeout_includes_bounded_stderr_snapshot() {
    let provider = ClaudeCodeProvider::new(stderr_then_block_stdin_fixture())
        .with_version_supplier(policy_version_supplier());
    let sink = RecordingToolPolicyAuditSink::new();
    let mut input = policy_claude_input(None, Some(sink.clone().bound()));
    input.prompt = "x".repeat(256 * 1024);

    let Err(error) = provider.start(input, CancellationToken::new()).await else {
        panic!("blocked initial write must fail the policy session");
    };

    assert!(
        error.details.contains("timed out"),
        "unexpected error: {}",
        error.details
    );
    assert!(
        error.details.contains("blocked stderr evidence"),
        "bounded start timeout must carry stderr snapshot in details: {}",
        error.details
    );
    assert!(
        error.stderr.contains("blocked stderr evidence"),
        "bounded start timeout must carry stderr snapshot in stderr field"
    );
    assert!(sink.events().is_empty());
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
            // direct 策略会话 fixture:无 LC 投影(字段仅 LC validated 启动落盘)。
            lc_projection: None,
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

// ==== Task 4a:LC 权限投影与统一 launch audit ====

use crate::cross_cutting::claude_code_provider::{ClaudePolicyProjector, projection};
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
        provider_dialect: ProviderDialect::ClaudeCodeCliV1,
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
        ProviderRef::claude_code("cap_claude_code_lc_fixture"),
        envelope.action,
        role,
        crate::cross_cutting::streaming_provider::ProviderPermissionMode::Auto,
        tool_policy,
        projection::CLAUDE_LC_APPROVAL_POLICY.to_string(),
        String::new(),
        "sha256:cfg-a".to_string(),
        trust_digest.to_string(),
        None,
    )
}

/// Task 4a Step 1(断言组 305 逐字 + 300-301 写面):target/role/tool/config
/// 任一漂移都会改变会话全投影 digest;capability profile 摘要按「证据摘要
/// 分层」不随单次 role/config 变化。Coding 投影恰一个可写 target(root/
/// 其它成员不可写);read-only action 无任何可写根。
#[test]
fn lcg_t04_projection_digest_changes_on_target_role_tool_or_config() {
    let projector = ClaudePolicyProjector::new("claude 2.0.4-lc-fixture");
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
        .expect("claude lc planning projection is produced");
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
    assert_eq!(
        baseline.wire_dialect(),
        ProviderWireDialect::ClaudeCodeStreamJson
    );
    assert_eq!(baseline.exact_version(), "claude 2.0.4-lc-fixture");
    assert_eq!(
        baseline.approval_policy(),
        projection::CLAUDE_LC_APPROVAL_POLICY
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
        .expect("claude lc coding projection is produced");
    assert_eq!(coding.writable_roots(), [PathBuf::from("/lc/member-a")]);
    assert!(!coding.writable_roots().iter().any(
        |root| root == &PathBuf::from("/lc/lc-root") || root == &PathBuf::from("/lc/member-b")
    ));
    assert!(!coding.boundary_evidence_ref().is_empty());
    assert_eq!(coding.sandbox(), "target-write-only");
    assert_eq!(baseline.sandbox(), "read-only");

    // profile 摘要随 tool 控制规范整体变化(allowlist/deny 序列属于 profile)。
    let coding_capability = coding.capability_projection_digest();
    assert_ne!(original_capability_digest, coding_capability);
}

/// 初始化即完成并登记进程 cwd 的 LC fixture:stdout 首行 init(原生会话 id),
/// 随后 result;`pwd -P` 写入 `$LC_CWD_MARKER` 供 cwd 断言。
fn lc_init_result_cwd_fixture() -> PathBuf {
    write_fixture(
        "claude_lc_init_cwd_fixture.sh",
        "#!/usr/bin/env bash\npwd -P > \"$LC_CWD_MARKER\"\nwhile IFS= read -r line; do\n  if [[ \"$line\" == *'\"type\":\"user\"'* ]]; then\n    echo '{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"sess-lc-native-1\"}'\n    echo '{\"type\":\"result\",\"subtype\":\"success\",\"result\":\"lc done\",\"session_id\":\"sess-lc-native-1\"}'\n    exit 0\n  fi\ndone\n",
    )
}

/// Task 4a Step 1(断言组 299-304 的 Claude 对照形态):无通用 tool policy
/// 的 LC Coding 启动(Executor)同样执行 exact version 解析、原生 init 握手
/// 与统一 `ProviderStartAudit.lc_projection` 落盘;进程 cwd=canonical root。
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

    let provider = ClaudeCodeProvider::new(lc_init_result_cwd_fixture())
        .with_version_supplier(policy_version_supplier());

    let mut session = provider
        .start_validated(
            fixture.validated_coding_input(raw),
            CancellationToken::new(),
        )
        .await
        .expect("lc validated start succeeds without generic tool policy");

    // 原生会话 id 来自 init 握手(无 tool_policy 也不跳过握手)。
    assert_eq!(
        session.native_session_id.as_deref(),
        Some("sess-lc-native-1")
    );

    let events = sink.events();
    assert_eq!(events.len(), 1, "exactly one provider_start is written");
    let DurableToolPolicyEvent::ProviderStart(record) = &events[0] else {
        panic!("expected provider_start");
    };
    assert_eq!(record.provider, "claude-code");
    assert_eq!(record.provider_version, "claude 1.0.99-policy-fixture");
    assert_eq!(record.adapter_dialect, "claude-stream-json");
    assert_eq!(record.provider_session_id, "sess-lc-native-1");
    assert_eq!(record.role, "executor");
    assert_eq!(record.workspace_session_id, "ws-lc-fixture-1");
    // 无通用 tool policy 也有 canonical digest(空 token 序列的 canonical 形态)。
    assert!(!record.tool_policy_canonical_digest.is_empty());
    // argv:headless allowlist 在;deny token 不在(Executor/Coding 无通用策略)。
    assert!(record.argv.iter().any(|arg| arg == "--allowedTools"));
    assert!(!record.argv.contains(&"--disallowedTools".to_string()));

    // 统一 lc_projection 落盘:分层双 digest + wire/action/boundary 引用。
    let lc_projection = record
        .lc_projection
        .as_ref()
        .expect("lc projection audit is recorded even without generic tool policy");
    assert_eq!(lc_projection.action, "coding_target_write");
    assert_eq!(lc_projection.wire_dialect, "claude-stream-json");
    assert!(lc_projection.projection_digest.starts_with("sha256:"));
    assert!(
        lc_projection
            .capability_projection_digest
            .starts_with("sha256:")
    );
    assert!(!lc_projection.boundary_evidence_ref.is_empty());

    // 进程 cwd = canonical LC root(envelope 冻结,不是 target worktree)。
    let observed_cwd = std::fs::read_to_string(&marker).expect("cwd marker is written");
    assert_eq!(
        observed_cwd.trim(),
        fixture.canonical_root().to_string_lossy(),
        "provider process must spawn at the canonical LC root"
    );

    // 握手消耗 init 行后,流式续读不受影响。
    assert_eq!(recv_completed(&mut session.events).await, "lc done");
}

// ==== Task 9a:LC 显式 resume 的 child 前全 LC audit 比对 ====

use crate::cross_cutting::tool_policy_audit::{
    LcProjectionAudit, LcProviderStartAudit, ProviderStartAudit, ResumeDecision,
    resume_with_lc_start_record,
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
            crate::product::logical_codebase::provider_gateway::ProviderRef::claude_code(
                validated.capability_snapshot_ref(),
            ),
            envelope.action,
            AdapterRole::Executor,
            crate::cross_cutting::streaming_provider::ProviderPermissionMode::Auto,
            None,
            projection::CLAUDE_LC_APPROVAL_POLICY.to_string(),
            String::new(),
            envelope.config_artifact_ref.clone(),
            String::new(),
            Some(boundary),
        );
    ClaudePolicyProjector::new("claude 1.0.99-policy-fixture")
        .project(&projection_input)
        .expect("expected lc projection")
}

/// Task 9a:LC 显式 resume 在 child 前比较全 LC audit(`LcProviderStartAudit`
/// 完整字面量含投影)。匹配的完整 LC 存档照常续接(native id 保持);旧
/// audit 缺 projection(direct/Task 4 前形态)或投影摘要漂移都拒绝 resume、
/// 拒绝发生在 ProcessManager::spawn 之前(零 child、零新 provider_start),
/// superseded 终止审计追加到被取代旧 run——绝不在 adapter 内清 resume id
/// 后静默 fresh。
#[cfg(unix)]
#[tokio::test]
async fn lcg_t09a_claude_lc_resume_audit_gate_zero_child_on_legacy_or_drift() {
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
        provider: "claude-code".to_string(),
        role: "executor".to_string(),
        workspace_session_id: "ws-lc-fixture-1".to_string(),
        provider_session_id: "sess-lc-resume-t09a".to_string(),
        tool_policy_canonical_digest: "sha256:seed".to_string(),
        argv: Vec::new(),
        sandbox: None,
        approval_policy: None,
        provider_version: "claude 1.0.99-policy-fixture".to_string(),
        adapter_dialect: "claude-stream-json".to_string(),
        lc_projection,
    };

    // 1)匹配的完整 LC 存档 → 续接:native id 即 resume id,新 run 落
    // provider_start,无 superseded。
    {
        let sink = RecordingToolPolicyAuditSink::new();
        sink.with_stored_provider_start(stored_record(Some(matching_lc_projection.clone())));
        let mut raw = fixture.lc_streaming_input(
            AdapterRole::Executor,
            None,
            Some(sink.clone().bound()),
            Some("sess-lc-resume-t09a".to_string()),
        );
        let marker = fixture.paths.root().join("t09a-matching-cwd-marker");
        raw.env_vars
            .insert("LC_CWD_MARKER".to_string(), marker.display().to_string());
        let provider = ClaudeCodeProvider::new(lc_init_result_cwd_fixture())
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
            Some("sess-lc-resume-t09a")
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
                if record.provider_session_id == "sess-lc-resume-t09a"
                    && record.lc_projection.is_some()
        ));
    }

    // 2)旧 audit 缺 projection → 不能 resume LC:零 child、零 provider_start,
    // 不清 id 转 fresh。
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
        let mut raw = fixture.lc_streaming_input(
            AdapterRole::Executor,
            None,
            Some(sink.clone().bound()),
            Some("sess-lc-resume-t09a".to_string()),
        );
        let marker = fixture.paths.root().join(format!("t09a-{case}-cwd-marker"));
        raw.env_vars
            .insert("LC_CWD_MARKER".to_string(), marker.display().to_string());
        let provider = ClaudeCodeProvider::new(lc_init_result_cwd_fixture())
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
            "no claude child may spawn after a refused LC resume ({case})"
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

// ==== Task 9b:LC 原生 resume 的 native 会话确认(错/缺 id 绝不 fresh)====

/// Task 9b fixture:以指定原生会话 id 应答 init 并完成的 LC fixture(与
/// `lc_init_result_cwd_fixture` 同构,init/result 的 session_id 参数化——
/// resume 确认路径的真实 native 应答形态;`pwd -P` 落 LC_CWD_MARKER 保持
/// 零 child 断言的观测点)。
fn lc_init_result_cwd_fixture_for_session(session_id: &str, result_text: &str) -> PathBuf {
    write_fixture(
        "claude_lc_init_cwd_session_fixture.sh",
        &format!(
            "#!/usr/bin/env bash\npwd -P > \"$LC_CWD_MARKER\"\nwhile IFS= read -r line; do\n  if [[ \"$line\" == *'\"type\":\"user\"'* ]]; then\n    echo '{{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"{}\"}}'\n    echo '{{\"type\":\"result\",\"subtype\":\"success\",\"result\":\"{}\",\"session_id\":\"{}\"}}'\n    exit 0\n  fi\ndone\n",
            session_id, result_text, session_id
        ),
    )
}

/// Task 9b fixture:登记 pid 并以「与请求不同的原生会话 id」应答 init,随后
/// 存活等待 kill 链终止(不自行退出——kill/reap 必须来自 adapter 返回前)。
#[cfg(unix)]
fn lc_pid_registering_init_fixture(marker: &std::path::Path, session_id: &str) -> PathBuf {
    write_fixture(
        "claude_lc_pid_init_fixture.sh",
        &format!(
            "#!/usr/bin/env bash\nwhile IFS= read -r line; do\n  if [[ \"$line\" == *'\"type\":\"user\"'* ]]; then\n    echo $$ > {}\n    echo '{{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"{}\"}}'\n    while IFS= read -r line; do :; done\n    exit 0\n  fi\ndone\n",
            marker.display(),
            session_id
        ),
    )
}

/// Task 9b kill 链断言辅助:轮询读取 fixture 登记的子进程 pid,再轮询确认
/// 该 pid 已退出且被回收(zombie 仍响应 `kill -0`,故「从进程表消失」才能
/// 证明 kill+reap 都发生了)。
#[cfg(unix)]
fn t09b_child_killed_and_reaped(pid_marker: &std::path::Path) -> bool {
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
            "claude child (pid {pid}) must be killed and reaped after an unconfirmed native resume"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

/// Task 9b(Step 1 断言组 438-440 逐字):LC 显式 resume 的真实 native 会话
/// 确认——Claude 侧 resume 同样等待 init 握手,原生应答 id 必须与请求
/// resume id 一致:
/// - 一致(无存档记录,9a 的「无存档→依赖 native 握手」由本测试补真实
///   确认)→ 续接,confirmed==requested;
/// - 缺 id(init 应答空白 session_id)→ Err;错 id(应答不同会话 id)→
///   Err——两者都是已启动 child 后的 runtime 失败:child 被 kill/reap、
///   错误记录「未恢复」(不伪称零 spawn),不回填请求 id、不清 id 转
///   fresh、零新 provider_start。
#[cfg(unix)]
#[tokio::test]
async fn lcg_t09_claude_missing_or_wrong_native_id_never_fresh() {
    let fixture = LcLaunchFixture::new();
    let requested_native_id = "sess-lc-resume-t09b".to_string();

    // 1) native 应答同 id(无存档记录):真实确认后续接,provider_start 落
    //    确认 id,握手消耗 init 行后流式续读不受影响。
    {
        let sink = RecordingToolPolicyAuditSink::new();
        let mut raw = fixture.lc_streaming_input(
            AdapterRole::Executor,
            None,
            Some(sink.clone().bound()),
            Some(requested_native_id.clone()),
        );
        let marker_dir = tempfile::tempdir().expect("t09b matching marker dir");
        raw.env_vars.insert(
            "LC_CWD_MARKER".to_string(),
            marker_dir
                .path()
                .join("t09b-matching-cwd-marker")
                .display()
                .to_string(),
        );
        let provider = ClaudeCodeProvider::new(lc_init_result_cwd_fixture_for_session(
            &requested_native_id,
            "lc resume done",
        ))
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
        assert_eq!(recv_completed(&mut session.events).await, "lc resume done");
    }

    // 2) 缺 id:init 应答空白 session_id → runtime 失败:Err + kill/reap +
    //    「未恢复」记录,零新 provider_start。
    {
        let marker_dir = tempfile::tempdir().expect("t09b missing-id marker dir");
        let pid_marker = marker_dir.path().join("claude-t09b-missing-id.pid");
        let sink = RecordingToolPolicyAuditSink::new();
        let raw = fixture.lc_streaming_input(
            AdapterRole::Executor,
            None,
            Some(sink.clone().bound()),
            Some(requested_native_id.clone()),
        );
        let provider = ClaudeCodeProvider::new(pid_registering_blank_init_fixture(&pid_marker, ""))
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
        assert!(t09b_child_killed_and_reaped(&pid_marker));
    }

    // 3) 错 id:init 应答不同的原生会话 id → 同为 runtime 失败:Err +
    //    kill/reap,绝不采纳陌生 id 续接、绝不清请求 id 转 fresh。
    {
        let marker_dir = tempfile::tempdir().expect("t09b wrong-id marker dir");
        let pid_marker = marker_dir.path().join("claude-t09b-wrong-id.pid");
        let sink = RecordingToolPolicyAuditSink::new();
        let raw = fixture.lc_streaming_input(
            AdapterRole::Executor,
            None,
            Some(sink.clone().bound()),
            Some(requested_native_id.clone()),
        );
        let provider = ClaudeCodeProvider::new(lc_pid_registering_init_fixture(
            &pid_marker,
            "sess-lc-native-wrong-t09b",
        ))
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
            rejected.details.contains("sess-lc-native-wrong-t09b")
                && rejected.details.contains(&requested_native_id)
                && rejected.details.contains("session NOT resumed"),
            "rejection must name both ids and record the not-resumed outcome: {rejected:?}"
        );
        assert!(
            sink.events().is_empty(),
            "no fresh provider_start may be written for a mismatched native resume confirmation"
        );
        let started_child_was_killed_and_reaped = t09b_child_killed_and_reaped(&pid_marker);
        assert!(started_child_was_killed_and_reaped);
    }
}
