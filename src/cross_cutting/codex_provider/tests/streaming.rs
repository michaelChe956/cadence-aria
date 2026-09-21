// 当前协议/流式族：agent message delta、completed-only 输出与命令执行
// 事件的 wire 级验证。
// 从 tests/mod.rs 拆出以满足 large_file_guard 的 1200 行上限。

use super::*;

#[tokio::test]
async fn codex_provider_handles_current_app_server_protocol_and_agent_message_delta() {
    let fixture = executable_fixture("tests/fixtures/provider/codex_app_server_current_fixture.sh");
    let provider = CodexProvider::new(fixture);
    let input = streaming_input(ProviderType::Codex, ProviderPermissionMode::Auto);
    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .unwrap();

    let completed = recv_completed(&mut session.events).await;

    assert!(completed.contains("# Story Spec"));
    assert!(completed.contains("## 功能需求"));
    assert!(completed.contains("## 成功标准"));
}

#[tokio::test]
async fn codex_provider_streams_completed_only_agent_messages() {
    let fixture =
        executable_fixture("tests/fixtures/provider/codex_app_server_completed_only_fixture.sh");
    let provider = CodexProvider::new(fixture);
    let input = streaming_input(ProviderType::Codex, ProviderPermissionMode::Auto);
    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .unwrap();

    let mut saw_text_delta = false;
    let completed = loop {
        match tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
            .await
            .expect("provider should emit completed-only text")
            .expect("provider event channel should stay open")
        {
            ProviderEvent::TextDelta { content } => {
                assert_eq!(content, "Codex completed-only chunk");
                saw_text_delta = true;
            }
            ProviderEvent::Completed(completion) => break completion.full_output,
            ProviderEvent::StatusChanged(_)
            | ProviderEvent::Execution(_)
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
    };

    assert!(saw_text_delta);
    assert_eq!(completed, "Codex completed-only chunk");
}

#[tokio::test]
async fn codex_provider_emits_command_execution_events_from_current_protocol() {
    let fixture = executable_fixture("tests/fixtures/provider/codex_app_server_current_fixture.sh");
    let provider = CodexProvider::new(fixture);
    let input = streaming_input(ProviderType::Codex, ProviderPermissionMode::Auto);
    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .unwrap();

    let mut saw_started = false;
    let mut saw_completed = false;
    for _ in 0..20 {
        match tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
            .await
            .expect("provider should emit execution events")
            .expect("provider event channel should stay open")
        {
            ProviderEvent::Execution(event)
                if event.kind == ProviderExecutionEventKind::Command
                    && event.status == ProviderExecutionEventStatus::Started =>
            {
                assert_eq!(event.event_id, "command_cmd_001");
                assert_eq!(event.command.as_deref(), Some("pwd"));
                assert!(event.cwd.is_some());
                saw_started = true;
            }
            ProviderEvent::Execution(event)
                if event.kind == ProviderExecutionEventKind::Command
                    && event.status == ProviderExecutionEventStatus::Completed =>
            {
                assert_eq!(event.event_id, "command_cmd_001");
                assert_eq!(event.command.as_deref(), Some("pwd"));
                assert_eq!(event.exit_code, Some(0));
                assert!(event.output.as_deref().unwrap_or_default().contains('/'));
                saw_completed = true;
            }
            ProviderEvent::Completed(_) if saw_started && saw_completed => return,
            ProviderEvent::Failed { message } => panic!("provider failed: {message}"),
            _ => {}
        }
    }

    assert!(saw_started, "command started event was not emitted");
    assert!(saw_completed, "command completed event was not emitted");
}
