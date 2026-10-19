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
    // 批2 Task 11：deny 是 Author 岗位纪律的 wire 物理投影——策略会话
    // fixture 采用 Author 承载角色（Orchestrator）；Reviewer 不再带 deny。
    input.role = AdapterRole::Orchestrator;
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

use crate::cross_cutting::tool_policy_audit::{LcProjectionAudit, ProviderStartAudit};

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
/// 完整字面量含投影)。匹配的完整 LC 存档照常续接(9b 起 resume 亦等 init
/// 握手、native 应答确认同 id);旧 audit 缺 projection(direct/Task 4 前
/// 形态)或投影摘要漂移都拒绝 resume、拒绝发生在 ProcessManager::spawn
/// 之前(零 child、零新 provider_start),superseded 终止审计追加到被取代
/// 旧 run——绝不在 adapter 内清 resume id 后静默 fresh。
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

    // 1)匹配的完整 LC 存档 → 续接:9b 起 init 握手确认同 id 后新 run 落
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
        let provider = ClaudeCodeProvider::new(lc_init_result_cwd_fixture_for_session(
            "sess-lc-resume-t09a",
            "lc done",
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

// ==== Task 10 fix:RootRecipe 相位 marker 的确定性 canonical 化 ====

use crate::cross_cutting::streaming_provider::ToolPolicyIntent;
use crate::product::logical_codebase::aggregate_initialization::AggregateInitializationStepKind;
use crate::product::logical_codebase::provider_admission_preflight::{
    BootstrapExecutorMarker, BootstrapPhaseCredential,
};

/// 构造完整四要素的 BootstrapExecutorMarker 策略(canonical 层测试材料;
/// 生产面凭据只能经 `from_running_operation` 派生,此处用 for_test seam)。
fn t10_bootstrap_marker_policy(operation_id: &str, receipt_context: &str) -> ProviderToolPolicy {
    let canonical_root = PathBuf::from("/lc/bootstrap-root");
    let credential = BootstrapPhaseCredential::for_test(
        "project_0001",
        "logical_codebase_0001",
        operation_id,
        AggregateInitializationStepKind::PreCheck,
        "sha256:test-input-digest",
        canonical_root.clone(),
    );
    let marker = BootstrapExecutorMarker::new(
        credential,
        SessionPolicyAction::CodingTargetWrite,
        canonical_root,
        receipt_context,
    )
    .expect("complete bootstrap executor marker");
    ProviderToolPolicy {
        intent: ToolPolicyIntent::BootstrapExecutorMarker(marker),
    }
}

/// Task 10 fix(root recipe 生产启动经 LC validated 链):RootRecipe 相位的
/// BootstrapExecutorMarker 可确定性 canonical 化(固定 schema 序列化+长度
/// 分隔+域前缀),供会话投影与统一 launch audit 落盘;Normal 相位的 marker
/// 在 canonical 层独立拒绝(T7 guard 相位门之外的二层防线,普通 Coder 不可
/// 借 marker);deny/None 的既有 canonical 语义与相位无关,零回归。
#[test]
fn lcg_t10_claude_root_recipe_marker_canonicalization_is_deterministic_and_phase_gated() {
    // 1) RootRecipe 相位:可翻译且确定性(同材料两次一致,digest 形状与
    //    LC 分层摘要同构:sha256: 前缀 + 64 位小写 hex)。
    let marker_policy = t10_bootstrap_marker_policy(
        "aggregate_initialization_0001",
        "root-recipe:pre_check:command-1",
    );
    let canonical = projection::lc_tool_policy_canonical_digest(Some(&marker_policy), true)
        .expect("root recipe phase marker must canonicalize");
    assert!(canonical.starts_with("sha256:"));
    assert_eq!(canonical.len(), 71);
    let again = projection::lc_tool_policy_canonical_digest(Some(&marker_policy), true)
        .expect("canonicalization is deterministic");
    assert_eq!(canonical, again);

    // 2) 漂移敏感:凭据 operation id / receipt context 任一变化 → digest 变化。
    let drifted_operation = t10_bootstrap_marker_policy(
        "aggregate_initialization_0002",
        "root-recipe:pre_check:command-1",
    );
    let drifted_receipt = t10_bootstrap_marker_policy(
        "aggregate_initialization_0001",
        "root-recipe:rule_and_mcp_config:command-2",
    );
    assert_ne!(
        canonical,
        projection::lc_tool_policy_canonical_digest(Some(&drifted_operation), true)
            .expect("drifted operation still canonicalizes")
    );
    assert_ne!(
        canonical,
        projection::lc_tool_policy_canonical_digest(Some(&drifted_receipt), true)
            .expect("drifted receipt still canonicalizes")
    );

    // 3) canonical 层防线:Normal 相位不可传 marker(普通 Coder 不可借)。
    let rejected = projection::lc_tool_policy_canonical_digest(Some(&marker_policy), false)
        .expect_err("normal phase marker must be rejected at the canonical layer");
    assert!(
        rejected.to_string().contains("root recipe phase"),
        "unexpected rejection: {rejected:?}"
    );

    // 4) deny/None 与相位无关(既有 canonical 语义零回归)。
    let deny = ProviderToolPolicy::deny_file_write_builtins();
    assert_eq!(
        projection::lc_tool_policy_canonical_digest(Some(&deny), true)
            .expect("deny canonicalizes in root recipe phase"),
        projection::lc_tool_policy_canonical_digest(Some(&deny), false)
            .expect("deny canonicalizes in normal phase")
    );
    assert_eq!(
        projection::lc_tool_policy_canonical_digest(None, true).expect("none in root recipe"),
        projection::lc_tool_policy_canonical_digest(None, false).expect("none in normal")
    );

    // 5) projector 层:RootRecipe 相位投影成功且确定性(Normal 相位的 marker
    //    经 T7 guard 先拦,此处验证 projector/canonical 层独立 fail-closed)。
    let marker_input = lc_projection_input(
        lc_projection_envelope(
            SessionPolicyAction::CodingTargetWrite,
            PathBuf::from("/lc/member-a"),
            "sha256:cfg-a",
            "sha256:cfg-digest-a",
        ),
        AdapterRole::Executor,
        Some(marker_policy.clone()),
        "sha256:trust-1",
    );
    let root_recipe_projector =
        ClaudePolicyProjector::new("claude 2.0.4-lc-fixture").with_root_recipe_phase(true);
    let projected = root_recipe_projector
        .project(&marker_input)
        .expect("root recipe projector accepts the bootstrap marker");
    assert!(projected.tool_policy().is_some());
    assert!(projected.projection_digest().starts_with("sha256:"));
    assert_eq!(
        projected.projection_digest(),
        root_recipe_projector
            .project(&marker_input)
            .expect("projection is deterministic")
            .projection_digest()
    );

    let normal_projector = ClaudePolicyProjector::new("claude 2.0.4-lc-fixture");
    let rejected_projection = normal_projector
        .project(&marker_input)
        .expect_err("normal phase projector must refuse the marker");
    assert!(
        rejected_projection
            .to_string()
            .contains("root recipe phase"),
        "unexpected projection rejection: {rejected_projection:?}"
    );
}

// ==== Task 3.3(B0b):Claude 真实 prepare facts 与 full 投影/审计同源 ====

use crate::cross_cutting::claude_code_provider::{
    CLAUDE_POLICY_DIALECT, TOOL_POLICY_PROVIDER_NAME,
};
use crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence;
use crate::product::logical_codebase::provider_gateway::{
    GatewaySessionDisposition, PreparedLcStreamingCandidate, ProviderLaunchAuditContext,
    ResumeSessionLaunchRequest, SessionResumeFingerprint, ValidatedSessionLaunchPolicy,
    canonical_target_git_identity,
};
use crate::product::logical_codebase::provider_projection::UnprovisionedProviderPolicyProjector;

/// t33 真实 CLI fixture 版本(`--version` 输出与 capability 快照同值,经
/// admission 的 provider_version 一致门)。
const T33_CLI_VERSION: &str = "claude 1.0.33-t33-fixture";

/// t33 真实 CLI fixture 体:`--version` 分支应答指定版本(真实 `--version`
/// 探测面);会话模式先把 `pwd -P` 落入 LC_CWD_MARKER(零 child 观测点),
/// 随后以指定原生会话 id 应答 init/result。
fn t33_cli_body(version: &str, session_id: &str, result_text: &str) -> String {
    format!(
        "#!/usr/bin/env bash\nif [[ \"${{1:-}}\" == \"--version\" ]]; then\n  echo '{version}'\n  exit 0\nfi\npwd -P > \"$LC_CWD_MARKER\"\nwhile IFS= read -r line; do\n  if [[ \"$line\" == *'\"type\":\"user\"'* ]]; then\n    echo '{{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"{session_id}\"}}'\n    echo '{{\"type\":\"result\",\"subtype\":\"success\",\"result\":\"{result_text}\",\"session_id\":\"{session_id}\"}}'\n    exit 0\n  fi\ndone\n"
    )
}

/// t33 错 ID fixture 体:登记 pid 并以「与请求不同的原生会话 id」应答 init,
/// 随后存活等待 kill 链终止(kill/reap 必须来自 adapter 返回前)。
fn t33_wrong_id_cli_body(version: &str, pid_marker: &Path, wrong_session_id: &str) -> String {
    format!(
        "#!/usr/bin/env bash\nif [[ \"${{1:-}}\" == \"--version\" ]]; then\n  echo '{version}'\n  exit 0\nfi\nwhile IFS= read -r line; do\n  if [[ \"$line\" == *'\"type\":\"user\"'* ]]; then\n    echo $$ > {}\n    echo '{{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"{wrong_session_id}\"}}'\n    while IFS= read -r line; do :; done\n    exit 0\n  fi\ndone\n",
        pid_marker.display()
    )
}

/// 写入(或重写)t33 CLI fixture 并保持可执行(重写同一路径是漂移用例的
/// 真实观测点:executable identity 变化 → probe key 换新再观测)。
fn t33_write_cli(path: &Path, body: &str) {
    std::fs::write(path, body).expect("write t33 cli fixture");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = std::fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(path, permissions).expect("chmod t33 cli fixture");
    }
}

fn t33_cli_path(name: &str, body: &str) -> PathBuf {
    let dir = tempfile::tempdir().expect("t33 fixture dir");
    let path = dir.keep().join(name);
    t33_write_cli(&path, body);
    path
}

/// t33 capability 源:version 与真实 CLI `--version` 输出同值;action row 的
/// `projection_digest` 以生产同源 Claude projector 推导 capability profile
/// 摘要(证据摘要分层:profile 只依赖 provider/dialect/wire/version/action/
/// boundary mode/approval/MCP 控制规范,与单次 cwd/target/trust/config 无关),
/// 保证 admission 的 profile 摘要门对真实投影放行。
fn t33_claude_profile_digest(action: SessionPolicyAction) -> String {
    let envelope = lc_projection_envelope(
        action,
        PathBuf::from("/t33/member-worktree"),
        "sha256:t33-config-artifact",
        "sha256:t33-config-digest",
    );
    let boundary = projection::lc_boundary_plan(&envelope).expect("t33 boundary plan");
    let input = ProviderProjectionInput::new(
        envelope.clone(),
        ProviderRef::claude_code("cap_claude_code_lc_fixture"),
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
    ClaudePolicyProjector::new(T33_CLI_VERSION)
        .project(&input)
        .expect("t33 profile projection derives the capability profile digest")
        .capability_projection_digest()
        .to_string()
}

struct T33CapabilitySource {
    profile_digest: String,
}

impl T33CapabilitySource {
    fn capability(&self, action: SessionPolicyAction) -> ProviderCapability {
        ProviderCapability {
            provider_type: ProviderRefType::ClaudeCode,
            version: T33_CLI_VERSION.to_string(),
            adapter_dialect: ProviderDialect::ClaudeCodeCliV1,
            wire_dialect: ProviderWireDialect::ClaudeCodeStreamJson,
            capability_snapshot_ref: "cap_claude_code_lc_fixture".to_string(),
            action_capability: ProviderActionCapability {
                action,
                launch: ProviderCapabilityEvidence::Confirmed,
                resume: ProviderCapabilityEvidence::Confirmed,
                write_boundary: ProviderCapabilityEvidence::Confirmed,
                projection_digest: self.profile_digest.clone(),
                evidence_ref: String::new(),
            },
            trust: ProviderCapabilityEvidence::Confirmed,
        }
    }
}

impl ProviderCapabilitySource for T33CapabilitySource {
    fn require_supported(
        &self,
        _provider: &ProviderRef,
        action: SessionPolicyAction,
    ) -> Result<ProviderCapability, ProviderGatewayError> {
        Ok(self.capability(action))
    }

    fn require_resume_supported(
        &self,
        _provider: &ProviderRef,
        action: SessionPolicyAction,
    ) -> Result<ProviderCapability, ProviderGatewayError> {
        Ok(self.capability(action))
    }

    fn require_write_boundary(
        &self,
        _provider: &ProviderRef,
        action: SessionPolicyAction,
    ) -> Result<ProviderCapability, ProviderGatewayError> {
        Ok(self.capability(action))
    }

    fn require_root_recipe_supported(
        &self,
        _provider: &ProviderRef,
        _credential: &crate::product::logical_codebase::provider_admission_preflight::BootstrapPhaseCredential,
    ) -> Result<ProviderCapability, ProviderGatewayError> {
        Err(ProviderGatewayError::UnsupportedCapability(
            "t33 fixture has no root recipe facts".to_string(),
        ))
    }
}

/// 以真实 ClaudeCodeProvider 装配 gateway(registry gated wrapper 透传
/// `prepare_lc_facts`;capability 源与真实 CLI 版本/profile 摘要对齐)。
fn t33_gateway(
    fixture: &LcLaunchFixture,
    adapter: Arc<ClaudeCodeProvider>,
) -> LogicalCodebaseProviderGateway {
    let mut registry = ProviderRegistry::new();
    registry
        .register_gated(
            ProviderName::ClaudeCode,
            adapter,
            Arc::new(UnprovisionedProviderPolicyProjector),
            lc_available_gate(),
        )
        .expect("register the real claude adapter for anchor t33");
    LogicalCodebaseProviderGateway::with_audit(
        AggregatePolicyArtifactStore::new(fixture.paths.clone()),
        Arc::new(T33CapabilitySource {
            profile_digest: t33_claude_profile_digest(SessionPolicyAction::CodingTargetWrite),
        }),
        Arc::new(LcTargetResolver),
        Arc::new(registry),
        Arc::new(LcNoopSyncAdapter),
        lc_available_gate(),
        fixture.audit.clone(),
        fixture.canonical_root(),
    )
}

fn t33_audit_context(sink: &Arc<RecordingToolPolicyAuditSink>) -> ProviderLaunchAuditContext {
    ProviderLaunchAuditContext {
        workspace_session_id: "ws-lc-fixture-1".to_string(),
        role_run_seq: 1,
        audit_sink: sink.clone(),
    }
}

/// 真实 prepare→full-matching resume 链:gateway 冻结 opaque 候选(内部经
/// registry wrapper 调真实 adapter `prepare_lc_facts`),再以候选 full
/// projection 走 `resume_or_start_with_projection` 得到携带冻结投影的
/// validated(完整上下文指纹一致 → Resume)。
async fn t33_prepared_resume_validated(
    gateway: &LogicalCodebaseProviderGateway,
    fixture: &LcLaunchFixture,
    sink: &Arc<RecordingToolPolicyAuditSink>,
    resume_id: &str,
) -> (PreparedLcStreamingCandidate, ValidatedSessionLaunchPolicy) {
    let input = fixture.lc_streaming_input(
        AdapterRole::Executor,
        None,
        None,
        Some(resume_id.to_string()),
    );
    let candidate = gateway
        .prepare_lc_streaming_candidate(
            input,
            fixture.coding_request(),
            t33_audit_context(sink),
            CancellationToken::new(),
        )
        .await
        .expect("gateway freezes the real claude full candidate");
    let previous = SessionResumeFingerprint::from_envelope(
        candidate.validated().envelope(),
        candidate.full_projection(),
        candidate.validated().action_evidence_digest(),
        &canonical_target_git_identity(&candidate.validated().envelope().target.worktree),
    );
    let disposition = gateway
        .resume_or_start_with_projection(
            ResumeSessionLaunchRequest {
                launch: fixture.coding_request(),
                previous_fingerprint: previous,
                previous_session_id: resume_id.to_string(),
            },
            candidate.full_projection().clone(),
        )
        .expect("full-matching fingerprint decision");
    match disposition {
        GatewaySessionDisposition::Resume(validated) => (candidate, validated),
        GatewaySessionDisposition::StartNew { .. } => {
            panic!("full-matching projection must resume, not start new")
        }
    }
}

/// Task 3.3(B0b) Step 1:仅 Claude——真实 `prepare_lc_facts`(复用本家 CLI
/// `--version` 探测与工具 translator)与 gateway 冻结 full projection、
/// adapter 审计同源:resume 判定产出的 validated 携带冻结 full projection,
/// `start_validated` 以同一 facts 消费它;原生 `--resume` 同 ID 确认后续接,
/// provider_start 的 lc_projection 摘要与 prepared full projection 逐字相等;
/// 错 ID 属 child 后 runtime 失败:kill/reap,不伪称零 child。
#[cfg(unix)]
#[tokio::test]
async fn anchor_t33_provider_prepare_and_audit_share_full_fingerprint() {
    let fixture = LcLaunchFixture::new();
    let resume_id = "sess-lc-resume-t33".to_string();

    // 1) 真实 adapter facts 只读观测(无 supplier:真实 CLI `--version`)。
    let cli = t33_cli_path(
        "claude_t33_full_fingerprint_fixture.sh",
        &t33_cli_body(T33_CLI_VERSION, &resume_id, "lc t33 done"),
    );
    let provider = Arc::new(ClaudeCodeProvider::new(cli));
    let gateway = t33_gateway(&fixture, provider.clone());
    let sink = RecordingToolPolicyAuditSink::new();

    let (candidate, validated) =
        t33_prepared_resume_validated(&gateway, &fixture, &sink, &resume_id).await;

    // facts 真实可观察(非 default unsupported 虚绿):真实 CLI `--version`
    // 值、probe key、approval 与 argv 投影同源。
    let facts = provider
        .prepare_lc_facts(
            candidate.input(),
            candidate.validated(),
            CancellationToken::new(),
        )
        .await
        .expect("real claude prepare_lc_facts observes the native CLI --version");
    assert_eq!(facts.exact_version(), T33_CLI_VERSION);
    assert_eq!(facts.version_probe_key().provider, ProviderName::ClaudeCode);
    assert_eq!(
        facts.version_probe_key().argv,
        vec!["--version".to_string()]
    );
    assert!(
        facts
            .version_probe_key()
            .executable_identity
            .starts_with("claude-cli:"),
        "probe key must freeze a real executable identity: {:?}",
        facts.version_probe_key().executable_identity
    );
    assert_eq!(
        facts.approval_policy(),
        projection::CLAUDE_LC_APPROVAL_POLICY
    );
    assert!(facts.version_observed_at() <= std::time::Instant::now());
    // gateway 冻结 full projection 与 facts 同版本同源(同版本 projector)。
    assert_eq!(
        candidate.full_projection().exact_version(),
        facts.exact_version()
    );

    // 2) start_validated 消费冻结 prepared projection:真实 spawn,原生
    // --resume 同 ID 确认后续接。
    let marker_dir = tempfile::tempdir().expect("t33 cwd marker dir");
    let mut spawn_input = fixture.lc_streaming_input(
        AdapterRole::Executor,
        None,
        Some(sink.clone().bound()),
        Some(resume_id.clone()),
    );
    spawn_input.env_vars.insert(
        "LC_CWD_MARKER".to_string(),
        marker_dir
            .path()
            .join("t33-cwd-marker")
            .display()
            .to_string(),
    );
    let mut session = provider
        .start_validated(
            ValidatedStreamingProviderInput::new(spawn_input, validated),
            CancellationToken::new(),
        )
        .await
        .expect("prepared full projection must drive the real claude session");
    assert_eq!(
        session.native_session_id.as_deref(),
        Some(resume_id.as_str())
    );
    assert_eq!(recv_completed(&mut session.events).await, "lc t33 done");

    // 3) 审计同源:provider_start 的 lc_projection 摘要与 prepared full
    // projection 逐字相等;版本/工具 translator 字面量同源。
    let prepared_projection_digest = candidate.full_projection().projection_digest().to_string();
    let events = sink.events();
    let provider_start = events
        .iter()
        .find_map(|event| match event {
            DurableToolPolicyEvent::ProviderStart(record) => Some(record),
            _ => None,
        })
        .expect("real claude spawn writes provider_start");
    let audit_projection_digest = provider_start
        .lc_projection
        .as_ref()
        .expect("provider_start carries the lc projection audit")
        .projection_digest
        .clone();
    assert_eq!(prepared_projection_digest, audit_projection_digest);
    assert_eq!(provider_start.provider, TOOL_POLICY_PROVIDER_NAME);
    assert_eq!(provider_start.adapter_dialect, CLAUDE_POLICY_DIALECT);
    assert_eq!(provider_start.provider_version, T33_CLI_VERSION);
    assert!(provider_start.argv.contains(&"--resume".to_string()));
    assert!(provider_start.argv.contains(&resume_id));
    assert!(
        provider_start
            .argv
            .contains(&projection::CLAUDE_LC_ALLOWED_TOOLS.to_string())
    );

    // 4) 错 ID(冻结投影同版本):child 已启动的 runtime 失败 → kill/reap,
    // 零新 provider_start(不伪称零 child)。
    let wrong_marker_dir = tempfile::tempdir().expect("t33 wrong-id marker dir");
    let pid_marker = wrong_marker_dir.path().join("claude-t33-wrong-id.pid");
    let wrong_cli = t33_cli_path(
        "claude_t33_wrong_id_fixture.sh",
        &t33_wrong_id_cli_body(T33_CLI_VERSION, &pid_marker, "sess-lc-native-wrong-t33"),
    );
    let wrong_provider = Arc::new(ClaudeCodeProvider::new(wrong_cli));
    let wrong_gateway = t33_gateway(&fixture, wrong_provider.clone());
    let wrong_sink = RecordingToolPolicyAuditSink::new();
    let (_wrong_candidate, wrong_validated) =
        t33_prepared_resume_validated(&wrong_gateway, &fixture, &wrong_sink, &resume_id).await;

    let wrong_input = fixture.lc_streaming_input(
        AdapterRole::Executor,
        None,
        Some(wrong_sink.clone().bound()),
        Some(resume_id.clone()),
    );
    let wrong_result = wrong_provider
        .start_validated(
            ValidatedStreamingProviderInput::new(wrong_input, wrong_validated),
            CancellationToken::new(),
        )
        .await;
    let Err(rejected) = wrong_result else {
        panic!("a wrong native id must fail the resume");
    };
    assert!(
        rejected.details.contains("sess-lc-native-wrong-t33")
            && rejected.details.contains(&resume_id)
            && rejected.details.contains("session NOT resumed"),
        "rejection must name both ids and record the not-resumed outcome: {rejected:?}"
    );
    assert!(
        wrong_sink.events().is_empty(),
        "no fresh provider_start may be written for a mismatched native resume confirmation"
    );
    let wrong_native_child_killed_and_reaped = t09b_child_killed_and_reaped(&pid_marker);
    assert!(wrong_native_child_killed_and_reaped);
}

/// Task 3.3(B0b) Step 1:仅 Claude——prepare→spawn 之间的真实版本漂移(重写
/// 同一 CLI,`--version` 改答新版;executable identity 变化 → probe key 换新
/// 再观测,不命中旧缓存)必须在 session child 之前被拒:零 provider_start
/// (零 adapter start)、零 child,错误显式命名两侧版本。
#[cfg(unix)]
#[tokio::test]
async fn anchor_t33_real_drift_zero_session_child() {
    let fixture = LcLaunchFixture::new();
    let resume_id = "sess-lc-resume-t33-drift".to_string();
    let drifted_version = "claude 2.0.777-t33-drifted".to_string();

    let cli_dir = tempfile::tempdir().expect("t33 drift cli dir");
    let cli_path = cli_dir.keep().join("claude_t33_drift_fixture.sh");
    t33_write_cli(
        &cli_path,
        &t33_cli_body(T33_CLI_VERSION, &resume_id, "lc t33 drift done"),
    );
    let provider = Arc::new(ClaudeCodeProvider::new(cli_path.clone()));
    let gateway = t33_gateway(&fixture, provider.clone());
    let sink = RecordingToolPolicyAuditSink::new();

    let (candidate, validated) =
        t33_prepared_resume_validated(&gateway, &fixture, &sink, &resume_id).await;
    assert_eq!(candidate.full_projection().exact_version(), T33_CLI_VERSION);

    // 真实漂移:重写同一 CLI(`--version` 改答新版;长度与 mtime 都变,
    // executable identity 变化 → 新 key 再观测)。
    std::thread::sleep(std::time::Duration::from_millis(20));
    t33_write_cli(
        &cli_path,
        &t33_cli_body(&drifted_version, &resume_id, "lc t33 drift done"),
    );

    let marker_dir = tempfile::tempdir().expect("t33 drift cwd marker dir");
    let cwd_marker = marker_dir.path().join("t33-drift-cwd-marker");
    let mut spawn_input = fixture.lc_streaming_input(
        AdapterRole::Executor,
        None,
        Some(sink.clone().bound()),
        Some(resume_id.clone()),
    );
    spawn_input.env_vars.insert(
        "LC_CWD_MARKER".to_string(),
        cwd_marker.display().to_string(),
    );

    let rejected = provider
        .start_validated(
            ValidatedStreamingProviderInput::new(spawn_input, validated),
            CancellationToken::new(),
        )
        .await;
    let Err(error) = rejected else {
        panic!("real version drift must refuse the spawn before any session child");
    };
    assert!(
        error.details.contains(T33_CLI_VERSION)
            && error.details.contains(&drifted_version)
            && error.details.contains("drift"),
        "drift rejection must name both observed versions: {error:?}"
    );

    // 零 session child:拒绝发生在 ProcessManager::spawn 之前。
    let mismatch_children = usize::from(cwd_marker.exists());
    assert_eq!(mismatch_children, 0);
    // 零 adapter start:漂移拒绝不落任何 provider_start,也不写其它审计。
    let mismatch_adapter_starts = sink
        .events()
        .iter()
        .filter(|event| matches!(event, DurableToolPolicyEvent::ProviderStart(_)))
        .count();
    assert_eq!(mismatch_adapter_starts, 0);
    assert!(sink.events().is_empty());
}
