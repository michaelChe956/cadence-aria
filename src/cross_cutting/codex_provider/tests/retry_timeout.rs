// 超时/空转/重试族：turn 停止事件超时、resume stall、空输出 in-session
// 单次重试与 429 失败透传（不落空输出 guard、不耗重试）的 wire 级验证。
// 从 tests/mod.rs 拆出以满足 large_file_guard 的 1200 行上限。

use super::*;

#[tokio::test]
async fn codex_provider_times_out_when_turn_stops_emitting_events() {
    let fixture =
        executable_fixture("tests/fixtures/provider/codex_app_server_hanging_turn_fixture.sh");
    let provider = CodexProvider::new(fixture);
    let mut input = streaming_input(ProviderType::Codex, ProviderPermissionMode::Auto);
    input.timeout_secs = 1;
    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .unwrap();

    loop {
        match tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
            .await
            .expect("provider should emit timeout failure")
            .expect("provider event channel should stay open until failure")
        {
            ProviderEvent::Failed { message } => {
                assert!(
                    message.contains("timed out") || message.contains("timeout"),
                    "unexpected failure message: {message}"
                );
                return;
            }
            ProviderEvent::StatusChanged(_)
            | ProviderEvent::Execution(_)
            | ProviderEvent::TextDelta { .. }
            | ProviderEvent::PermissionRequest(_)
            | ProviderEvent::ChoiceRequest(_)
            | ProviderEvent::ToolCall(_)
            | ProviderEvent::ToolResult(_) => {}
            ProviderEvent::Completed(completion) => {
                let full_output = completion.full_output;
                panic!("provider completed unexpectedly: {full_output}")
            }
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

#[tokio::test]
async fn codex_provider_reports_resume_stall_when_resumed_turn_emits_no_events() {
    let fixture = executable_fixture(
        "tests/fixtures/provider/codex_app_server_resume_hanging_turn_fixture.sh",
    );
    let provider = CodexProvider::new(fixture);
    let mut input = streaming_input(ProviderType::Codex, ProviderPermissionMode::Auto);
    input.resume_provider_session_id = Some("codex-thread-stale".to_string());
    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .unwrap();

    loop {
        match tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
            .await
            .expect("provider should emit resume stall failure")
            .expect("provider event channel should stay open until failure")
        {
            ProviderEvent::Failed { message } => {
                assert!(
                    message.contains("Codex resume stalled before provider progress"),
                    "unexpected failure message: {message}"
                );
                return;
            }
            ProviderEvent::StatusChanged(_)
            | ProviderEvent::Execution(_)
            | ProviderEvent::TextDelta { .. }
            | ProviderEvent::PermissionRequest(_)
            | ProviderEvent::ChoiceRequest(_)
            | ProviderEvent::ToolCall(_)
            | ProviderEvent::ToolResult(_) => {}
            ProviderEvent::Completed(completion) => {
                let full_output = completion.full_output;
                panic!("provider completed unexpectedly: {full_output}")
            }
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

#[tokio::test]
async fn codex_provider_empty_turn_output_retries_once_and_completes() {
    let fixture = executable_fixture(
        "tests/fixtures/provider/codex_app_server_empty_output_retry_success_fixture.sh",
    );
    let provider = CodexProvider::new(fixture);
    let input = streaming_input(ProviderType::Codex, ProviderPermissionMode::Auto);
    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .unwrap();

    let mut saw_retry_audit = false;
    let completion = loop {
        match tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
            .await
            .expect("provider should emit completion")
            .expect("provider event channel should stay open")
        {
            ProviderEvent::Execution(event)
                if event.kind == ProviderExecutionEventKind::Turn
                    && event.status == ProviderExecutionEventStatus::Running
                    && event.title.contains("retry") =>
            {
                saw_retry_audit = true;
            }
            ProviderEvent::Completed(completion) => break completion,
            ProviderEvent::StatusChanged(_)
            | ProviderEvent::Execution(_)
            | ProviderEvent::TextDelta { .. }
            | ProviderEvent::PermissionRequest(_)
            | ProviderEvent::ChoiceRequest(_)
            | ProviderEvent::ToolCall(_)
            | ProviderEvent::UsageReport(_)
            | ProviderEvent::ToolPolicyDecision(_)
            | ProviderEvent::ToolPolicyWarning(_)
            | ProviderEvent::ToolPolicyTerminated(_)
            | ProviderEvent::ToolResult(_) => {}
            ProviderEvent::Failed { message } => panic!("provider failed: {message}"),
            ProviderEvent::ProtocolError { message, .. } => {
                panic!("provider protocol error: {message}")
            }
            ProviderEvent::PermissionTimeout { permission_id } => {
                panic!("provider permission timed out: {permission_id}")
            }
        }
    };

    assert!(
        saw_retry_audit,
        "empty-output retry should leave a provider-layer audit event"
    );
    assert_eq!(completion.full_output, "Retry output recovered");
}

#[tokio::test]
async fn codex_provider_empty_output_retry_recovers_when_retry_turn_reuses_item_id() {
    let fixture = executable_fixture(
        "tests/fixtures/provider/codex_app_server_empty_output_retry_same_item_id_fixture.sh",
    );
    let provider = CodexProvider::new(fixture);
    let input = streaming_input(ProviderType::Codex, ProviderPermissionMode::Auto);
    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .unwrap();

    // 首轮 agentMessage 仅含空白且已被去重集合登记；重试 turn 复用同一 item id 时，
    // 重试必须先清理去重集合，否则恢复内容会被当作重复 item 丢弃。
    let completion = loop {
        match tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
            .await
            .expect("provider should emit completion")
            .expect("provider event channel should stay open")
        {
            ProviderEvent::Completed(completion) => break completion,
            ProviderEvent::StatusChanged(_)
            | ProviderEvent::Execution(_)
            | ProviderEvent::TextDelta { .. }
            | ProviderEvent::PermissionRequest(_)
            | ProviderEvent::ChoiceRequest(_)
            | ProviderEvent::ToolCall(_)
            | ProviderEvent::UsageReport(_)
            | ProviderEvent::ToolPolicyDecision(_)
            | ProviderEvent::ToolPolicyWarning(_)
            | ProviderEvent::ToolPolicyTerminated(_)
            | ProviderEvent::ToolResult(_) => {}
            ProviderEvent::Failed { message } => panic!("provider failed: {message}"),
            ProviderEvent::ProtocolError { message, .. } => {
                panic!("provider protocol error: {message}")
            }
            ProviderEvent::PermissionTimeout { permission_id } => {
                panic!("provider permission timed out: {permission_id}")
            }
        }
    };

    assert_eq!(
        completion.full_output, "Recovered via reused item id",
        "retry turn reusing the streamed item id must not be dropped by the dedup set"
    );
}

#[tokio::test]
async fn codex_provider_empty_turn_output_fails_with_provider_empty_output_after_single_retry() {
    let fixture = executable_fixture(
        "tests/fixtures/provider/codex_app_server_empty_output_twice_fixture.sh",
    );
    let provider = CodexProvider::new(fixture);
    let input = streaming_input(ProviderType::Codex, ProviderPermissionMode::Auto);
    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .unwrap();

    let mut saw_retry_audit = false;
    loop {
        match tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
            .await
            .expect("provider should emit terminal failure")
            .expect("provider event channel should stay open until failure")
        {
            ProviderEvent::Failed { message } => {
                assert!(
                    message.contains("provider_empty_output"),
                    "unexpected failure message: {message}"
                );
                assert!(
                    saw_retry_audit,
                    "empty-output retry should leave a provider-layer audit event"
                );
                return;
            }
            ProviderEvent::Execution(event)
                if event.kind == ProviderExecutionEventKind::Turn
                    && event.status == ProviderExecutionEventStatus::Running
                    && event.title.contains("retry") =>
            {
                saw_retry_audit = true;
            }
            ProviderEvent::StatusChanged(_)
            | ProviderEvent::Execution(_)
            | ProviderEvent::TextDelta { .. }
            | ProviderEvent::PermissionRequest(_)
            | ProviderEvent::ChoiceRequest(_)
            | ProviderEvent::ToolCall(_)
            | ProviderEvent::UsageReport(_)
            | ProviderEvent::ToolPolicyDecision(_)
            | ProviderEvent::ToolPolicyWarning(_)
            | ProviderEvent::ToolPolicyTerminated(_)
            | ProviderEvent::ToolResult(_) => {}
            ProviderEvent::Completed(completion) => {
                let full_output = completion.full_output;
                panic!("provider completed with empty output: {full_output:?}");
            }
            ProviderEvent::ProtocolError { message, .. } => {
                panic!("provider protocol error: {message}")
            }
            ProviderEvent::PermissionTimeout { permission_id } => {
                panic!("provider permission timed out: {permission_id}")
            }
        }
    }
}

// E1 定因回放（spike T2-resume-writes.jsonl）：上游 429 限流时 app-server 发送
// turn/completed{turn.status:"failed"}，error.message 含原始 429 文案、零 agent 输出。
// 失败轮必须以原始 429 文案失败；绝不落入空输出 guard（provider_empty_output 吞错误），
// 也不得触发 in-session 空输出重试（重试只服务真实空输出场景）。
#[tokio::test]
async fn codex_provider_turn_failed_429_surfaces_upstream_error_not_empty_output() {
    let fixture =
        executable_fixture("tests/fixtures/provider/codex_app_server_turn_failed_429_fixture.sh");
    let provider = CodexProvider::new(fixture);
    let input = streaming_input(ProviderType::Codex, ProviderPermissionMode::Auto);
    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .unwrap();

    let mut saw_empty_output_retry = false;
    loop {
        match tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
            .await
            .expect("provider should emit terminal failure")
            .expect("provider event channel should stay open until failure")
        {
            ProviderEvent::Failed { message } => {
                assert!(
                    message.contains("exceeded retry limit, last status: 429 Too Many Requests"),
                    "failed turn must surface the verbatim upstream 429 message: {message}"
                );
                assert!(
                    !message.contains("provider_empty_output"),
                    "failed turn must not be misrouted to the empty-output guard: {message}"
                );
                assert!(
                    !saw_empty_output_retry,
                    "failed turn must not consume the in-session empty-output retry"
                );
                return;
            }
            ProviderEvent::Execution(event)
                if event.kind == ProviderExecutionEventKind::Turn
                    && event.status == ProviderExecutionEventStatus::Running
                    && event.title.contains("retry") =>
            {
                saw_empty_output_retry = true;
            }
            ProviderEvent::StatusChanged(_)
            | ProviderEvent::Execution(_)
            | ProviderEvent::TextDelta { .. }
            | ProviderEvent::PermissionRequest(_)
            | ProviderEvent::ChoiceRequest(_)
            | ProviderEvent::ToolCall(_)
            | ProviderEvent::UsageReport(_)
            | ProviderEvent::ToolPolicyDecision(_)
            | ProviderEvent::ToolPolicyWarning(_)
            | ProviderEvent::ToolPolicyTerminated(_)
            | ProviderEvent::ToolResult(_) => {}
            ProviderEvent::Completed(completion) => {
                let full_output = completion.full_output;
                panic!("failed turn must not complete: {full_output:?}");
            }
            ProviderEvent::ProtocolError { message, .. } => {
                panic!("provider protocol error: {message}")
            }
            ProviderEvent::PermissionTimeout { permission_id } => {
                panic!("provider permission timed out: {permission_id}")
            }
        }
    }
}

#[tokio::test]
async fn codex_provider_nonempty_turn_output_completes_without_retry() {
    let fixture = executable_fixture(
        "tests/fixtures/provider/codex_app_server_nonempty_output_no_retry_fixture.sh",
    );
    let provider = CodexProvider::new(fixture);
    let input = streaming_input(ProviderType::Codex, ProviderPermissionMode::Auto);
    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .unwrap();

    let completion = recv_completion(&mut session.events).await;

    assert_eq!(completion.full_output, "Primary output delivered");
}
