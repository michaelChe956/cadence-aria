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
    assert_eq!(record.dialect, "claude-stream-json");
    assert_eq!(record.provider_version, "claude 1.0.99-policy-fixture");
    assert_eq!(record.native_session_id, "sess-policy-1");
    assert!(record.argv.contains(&"--disallowedTools".to_string()));
    assert!(record.argv.contains(&"Edit,Write,NotebookEdit".to_string()));
    assert!(!record.tool_policy_digest.is_empty());
    assert_eq!(record.sandbox, None);
    assert_eq!(record.approval_policy, None);

    // 握手消耗了 init 行，但流式续读不受影响：result 仍可送达。
    assert_eq!(recv_completed(&mut session.events).await, "policy done");
}

#[cfg(unix)]
#[tokio::test]
async fn claude_policy_resume_uses_known_native_id_without_waiting_for_init() {
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
        .expect("claude policy resume session starts");

    // resume 已知：native id 即 resume id，不等 init。
    assert_eq!(
        session.native_session_id.as_deref(),
        Some("claude-session-resume-policy")
    );
    let DurableToolPolicyEvent::ProviderStart(record) = &sink.events()[0] else {
        panic!("expected provider_start");
    };
    assert_eq!(record.native_session_id, "claude-session-resume-policy");
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

    let provider = ClaudeCodeProvider::new(init_then_result_fixture());
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
        error.details.contains("version is unavailable"),
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
