use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::streaming_provider::{
    ChoiceAnswerData, ProviderCommand, ProviderEvent, ProviderExecutionEventKind,
    ProviderExecutionEventStatus, ProviderPermissionMode, StreamingProviderAdapter,
    StreamingProviderInput,
};
use crate::protocol::contracts::{AdapterRole, ProviderType};

use super::CodexProvider;

const TEST_TIMEOUT: Duration = Duration::from_secs(5);

#[test]
fn codex_provider_supports_provider_driven_testing() {
    let provider = CodexProvider::new(PathBuf::from("codex"));

    assert!(provider.supports_provider_driven_testing());
}

fn executable_fixture(relative_path: &str) -> PathBuf {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative_path);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = std::fs::metadata(&path)
            .unwrap_or_else(|error| panic!("fixture metadata {}: {error}", path.display()))
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&path, permissions)
            .unwrap_or_else(|error| panic!("chmod fixture {}: {error}", path.display()));
    }
    path
}

fn streaming_input(
    provider_type: ProviderType,
    permission_mode: ProviderPermissionMode,
) -> StreamingProviderInput {
    StreamingProviderInput {
        provider_type,
        role: AdapterRole::Orchestrator,
        prompt: "fixture prompt".to_string(),
        working_dir: std::env::current_dir().unwrap(),
        workspace_session_id: None,
        resume_provider_session_id: None,
        permission_mode,
        env_vars: BTreeMap::new(),
        timeout_secs: 60,
    }
}

async fn recv_completed(events: &mut mpsc::Receiver<ProviderEvent>) -> String {
    loop {
        match tokio::time::timeout(TEST_TIMEOUT, events.recv())
            .await
            .expect("provider should emit completion")
            .expect("provider event channel should stay open")
        {
            ProviderEvent::Completed { full_output, .. } => return full_output,
            ProviderEvent::StatusChanged(_)
            | ProviderEvent::Execution(_)
            | ProviderEvent::TextDelta { .. }
            | ProviderEvent::PermissionRequest(_)
            | ProviderEvent::ChoiceRequest(_)
            | ProviderEvent::ToolCall(_)
            | ProviderEvent::ToolResult(_) => {}
            ProviderEvent::Failed { message } => panic!("provider failed: {message}"),
            ProviderEvent::ProtocolError { message, .. } => {
                panic!("provider protocol error: {message}")
            }
            ProviderEvent::PermissionTimeout { permission_id } => {
                panic!("provider permission timed out: {permission_id}")
            }
        }
    }
}

#[test]
fn codex_provider_enables_default_mode_request_user_input_feature() {
    let provider = CodexProvider::new(PathBuf::from("codex"));

    assert_eq!(
        provider.build_args(),
        vec![
            "app-server".to_string(),
            "--enable".to_string(),
            "default_mode_request_user_input".to_string(),
        ]
    );
}

#[tokio::test]
async fn codex_resume_uses_existing_thread_without_starting_new_thread() {
    let fixture = executable_fixture("tests/fixtures/provider/codex_app_server_resume_fixture.sh");
    let provider = CodexProvider::new(fixture);
    let mut input = streaming_input(ProviderType::Codex, ProviderPermissionMode::Auto);
    input.resume_provider_session_id = Some("codex-thread-123".to_string());
    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .unwrap();

    let completed = recv_completed(&mut session.events).await;

    assert_eq!(completed, "resumed done");
}

#[tokio::test]
async fn codex_thread_start_creates_persistent_thread_for_later_resume() {
    let fixture =
        executable_fixture("tests/fixtures/provider/codex_app_server_persistent_thread_fixture.sh");
    let provider = CodexProvider::new(fixture);
    let input = streaming_input(ProviderType::Codex, ProviderPermissionMode::Auto);
    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .unwrap();

    let completed = recv_completed(&mut session.events).await;

    assert_eq!(completed, "persistent thread done");
}

#[tokio::test]
async fn codex_thread_start_requests_danger_full_access_sandbox() {
    let fixture = executable_fixture("tests/fixtures/provider/codex_app_server_sandbox_fixture.sh");
    let provider = CodexProvider::new(fixture);
    let input = streaming_input(ProviderType::Codex, ProviderPermissionMode::Supervised);
    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .unwrap();

    let completed = recv_completed(&mut session.events).await;

    assert_eq!(completed, "sandbox disabled done");
}

#[tokio::test]
async fn codex_thread_resume_requests_danger_full_access_sandbox() {
    let fixture =
        executable_fixture("tests/fixtures/provider/codex_app_server_resume_sandbox_fixture.sh");
    let provider = CodexProvider::new(fixture);
    let mut input = streaming_input(ProviderType::Codex, ProviderPermissionMode::Auto);
    input.resume_provider_session_id = Some("codex-thread-123".to_string());
    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .unwrap();

    let completed = recv_completed(&mut session.events).await;

    assert_eq!(completed, "resume sandbox disabled done");
}

#[tokio::test]
async fn codex_provider_bridges_permission_and_completes() {
    let fixture = executable_fixture("tests/fixtures/provider/codex_app_server_fixture.sh");
    let provider = CodexProvider::new(fixture);
    let input = streaming_input(ProviderType::Codex, ProviderPermissionMode::Supervised);
    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .unwrap();

    let mut saw_text = false;
    let permission_id = loop {
        match session.events.recv().await.unwrap() {
            ProviderEvent::TextDelta { content } => {
                saw_text = content.contains("Codex fixture chunk");
            }
            ProviderEvent::PermissionRequest(data) => break data.id,
            ProviderEvent::StatusChanged(_)
            | ProviderEvent::Execution(_)
            | ProviderEvent::ChoiceRequest(_)
            | ProviderEvent::ToolCall(_)
            | ProviderEvent::ToolResult(_) => {}
            other => panic!("unexpected event before permission: {other:?}"),
        }
    };
    assert!(saw_text);

    session
        .commands
        .send(ProviderCommand::PermissionResponse {
            id: permission_id,
            approved: true,
            reason: None,
        })
        .await
        .unwrap();

    let completed = recv_completed(&mut session.events).await;
    assert_eq!(completed, "Codex fixture chunk");
}

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
async fn codex_provider_responds_to_current_command_approval_with_json_rpc_result() {
    let fixture = executable_fixture(
        "tests/fixtures/provider/codex_app_server_current_permission_fixture.sh",
    );
    let provider = CodexProvider::new(fixture);
    let input = streaming_input(ProviderType::Codex, ProviderPermissionMode::Supervised);
    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .unwrap();

    let permission = loop {
        match tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
            .await
            .expect("provider should emit current command approval")
            .expect("provider event channel should stay open")
        {
            ProviderEvent::PermissionRequest(request) => break request,
            ProviderEvent::StatusChanged(_)
            | ProviderEvent::Execution(_)
            | ProviderEvent::TextDelta { .. }
            | ProviderEvent::ChoiceRequest(_)
            | ProviderEvent::ToolCall(_)
            | ProviderEvent::ToolResult(_) => {}
            ProviderEvent::Completed { full_output, .. } => {
                panic!("provider completed before permission request: {full_output}")
            }
            ProviderEvent::Failed { message } => panic!("provider failed: {message}"),
            ProviderEvent::ProtocolError { message, .. } => {
                panic!("provider protocol error: {message}")
            }
            ProviderEvent::PermissionTimeout { permission_id } => {
                panic!("provider permission timed out: {permission_id}")
            }
        }
    };
    assert_eq!(permission.tool_name, "command");
    assert!(permission.description.contains("pnpm -C web install"));

    session
        .commands
        .send(ProviderCommand::PermissionResponse {
            id: permission.id,
            approved: true,
            reason: None,
        })
        .await
        .unwrap();

    let completed = recv_completed(&mut session.events).await;
    assert_eq!(completed, "permission accepted");
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
            ProviderEvent::Completed { full_output, .. } => break full_output,
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
            ProviderEvent::PermissionTimeout { permission_id } => {
                panic!("provider permission timed out: {permission_id}")
            }
        }
    };

    assert!(saw_text_delta);
    assert_eq!(completed, "Codex completed-only chunk");
}

#[tokio::test]
async fn codex_provider_bridges_request_user_input_and_completes() {
    let fixture =
        executable_fixture("tests/fixtures/provider/codex_app_server_user_input_fixture.sh");
    let provider = CodexProvider::new(fixture);
    let input = streaming_input(ProviderType::Codex, ProviderPermissionMode::Auto);
    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .unwrap();

    let choice = loop {
        match tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
            .await
            .expect("provider should emit a choice request")
            .expect("provider event channel should stay open")
        {
            ProviderEvent::ChoiceRequest(request) => break request,
            ProviderEvent::StatusChanged(_)
            | ProviderEvent::Execution(_)
            | ProviderEvent::TextDelta { .. }
            | ProviderEvent::PermissionRequest(_)
            | ProviderEvent::ToolCall(_)
            | ProviderEvent::ToolResult(_) => {}
            ProviderEvent::Completed { full_output, .. } => {
                panic!("provider completed before choice request: {full_output}")
            }
            ProviderEvent::Failed { message } => panic!("provider failed: {message}"),
            ProviderEvent::ProtocolError { message, .. } => {
                panic!("provider protocol error: {message}")
            }
            ProviderEvent::PermissionTimeout { permission_id } => {
                panic!("provider permission timed out: {permission_id}")
            }
        }
    };

    assert_eq!(choice.id, "77");
    assert_eq!(choice.prompt, "请选择复杂度");
    assert_eq!(choice.options[0].id, "O(n)");
    assert_eq!(choice.options[0].description.as_deref(), Some("线性复杂度"));

    session
        .commands
        .send(ProviderCommand::ChoiceResponse {
            id: choice.id,
            selected_option_ids: vec!["O(n)".to_string()],
            free_text: None,
            answers: vec![],
        })
        .await
        .unwrap();

    let completed = recv_completed(&mut session.events).await;
    assert_eq!(completed, "Codex received O(n)");
}

#[tokio::test]
async fn codex_provider_bridges_all_request_user_input_questions() {
    let fixture =
        executable_fixture("tests/fixtures/provider/codex_app_server_multi_user_input_fixture.sh");
    let provider = CodexProvider::new(fixture);
    let input = streaming_input(ProviderType::Codex, ProviderPermissionMode::Auto);
    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .unwrap();

    let choice = loop {
        match tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
            .await
            .expect("provider should emit a choice request")
            .expect("provider event channel should stay open")
        {
            ProviderEvent::ChoiceRequest(request) => break request,
            ProviderEvent::StatusChanged(_)
            | ProviderEvent::Execution(_)
            | ProviderEvent::TextDelta { .. }
            | ProviderEvent::PermissionRequest(_)
            | ProviderEvent::ToolCall(_)
            | ProviderEvent::ToolResult(_) => {}
            ProviderEvent::Completed { full_output, .. } => {
                panic!("provider completed before choice request: {full_output}")
            }
            ProviderEvent::Failed { message } => panic!("provider failed: {message}"),
            ProviderEvent::ProtocolError { message, .. } => {
                panic!("provider protocol error: {message}")
            }
            ProviderEvent::PermissionTimeout { permission_id } => {
                panic!("provider permission timed out: {permission_id}")
            }
        }
    };

    assert_eq!(choice.id, "91");
    assert_eq!(choice.questions.len(), 3);
    assert_eq!(choice.questions[0].id, "startup");
    assert_eq!(choice.questions[0].prompt, "启动自检策略？");
    assert_eq!(choice.questions[1].id, "scope");
    assert_eq!(choice.questions[1].prompt, "影响范围？");
    assert_eq!(choice.questions[2].id, "mcp_events");
    assert_eq!(choice.questions[2].prompt, "MCP 事件输出？");

    session
        .commands
        .send(ProviderCommand::ChoiceResponse {
            id: choice.id,
            selected_option_ids: vec![],
            free_text: None,
            answers: vec![
                ChoiceAnswerData {
                    question_id: "startup".to_string(),
                    selected_option_ids: vec!["每次启动都自检".to_string()],
                    free_text: None,
                },
                ChoiceAnswerData {
                    question_id: "scope".to_string(),
                    selected_option_ids: vec!["Story/Design/Work Item 共享链路".to_string()],
                    free_text: None,
                },
                ChoiceAnswerData {
                    question_id: "mcp_events".to_string(),
                    selected_option_ids: vec!["输出 MCP 事件".to_string()],
                    free_text: None,
                },
            ],
        })
        .await
        .unwrap();

    let completed = recv_completed(&mut session.events).await;
    assert_eq!(completed, "Codex received all answers");
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
            ProviderEvent::Completed { .. } if saw_started && saw_completed => return,
            ProviderEvent::Failed { message } => panic!("provider failed: {message}"),
            _ => {}
        }
    }

    assert!(saw_started, "command started event was not emitted");
    assert!(saw_completed, "command completed event was not emitted");
}

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
            ProviderEvent::Completed { full_output, .. } => {
                panic!("provider completed unexpectedly: {full_output}")
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
            ProviderEvent::Completed { full_output, .. } => {
                panic!("provider completed unexpectedly: {full_output}")
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
async fn codex_provider_request_user_input_emits_protocol_error_on_bridge_failure() {
    let fixture =
        executable_fixture("tests/fixtures/provider/codex_app_server_user_input_fixture.sh");
    let provider = CodexProvider::new(fixture);
    let input = streaming_input(ProviderType::Codex, ProviderPermissionMode::Auto);
    let cancel = CancellationToken::new();
    let mut session = provider.start(input, cancel.clone()).await.unwrap();

    let _choice = loop {
        match tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
            .await
            .expect("provider should emit choice")
            .expect("provider event channel should stay open")
        {
            ProviderEvent::ChoiceRequest(choice) => break choice,
            ProviderEvent::Failed { message } => {
                panic!("provider failed before choice: {message}")
            }
            _ => {}
        }
    };

    // 取消 provider 以强制 bridge 失败。
    cancel.cancel();

    let mut saw_protocol_error = false;
    while let Some(event) = tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
        .await
        .unwrap_or(None)
    {
        match event {
            ProviderEvent::ProtocolError {
                code,
                message,
                context,
            } if code == "request_user_input_unresolved" => {
                assert!(
                    message.contains("requestUserInput"),
                    "unexpected message: {message}"
                );
                assert!(
                    message.contains("unresolved"),
                    "unexpected message: {message}"
                );
                let context = context.expect("protocol error should include context");
                assert_eq!(
                    context.get("question_id").and_then(Value::as_str),
                    Some("complexity")
                );
                saw_protocol_error = true;
                break;
            }
            ProviderEvent::Completed { .. } => {
                panic!("expected protocol error before completion")
            }
            ProviderEvent::Failed { message } => {
                panic!("expected protocol error before failure: {message}")
            }
            _ => {}
        }
    }
    assert!(
        saw_protocol_error,
        "expected request_user_input_unresolved protocol error after bridge failure"
    );
}

#[tokio::test]
async fn codex_provider_request_user_input_emits_protocol_error_on_write_failure() {
    let fixture = executable_fixture(
        "tests/fixtures/provider/codex_request_user_input_peer_closes_fixture.sh",
    );
    let provider = CodexProvider::new(fixture);
    let input = streaming_input(ProviderType::Codex, ProviderPermissionMode::Auto);

    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .unwrap();

    let choice = loop {
        match tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
            .await
            .expect("provider should emit choice")
            .expect("provider event channel should stay open")
        {
            ProviderEvent::ChoiceRequest(choice) => break choice,
            ProviderEvent::Failed { message } => {
                panic!("provider failed before choice: {message}")
            }
            _ => {}
        }
    };

    session
        .commands
        .send(ProviderCommand::ChoiceResponse {
            id: choice.id,
            selected_option_ids: vec!["是".to_string()],
            free_text: None,
            answers: vec![],
        })
        .await
        .expect("send choice response");

    let mut saw_protocol_error = false;
    while let Some(event) = tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
        .await
        .expect("provider should emit events")
    {
        match event {
            ProviderEvent::ProtocolError {
                code,
                message,
                context,
            } if code == "request_user_input_unresolved" => {
                assert!(
                    message.contains("requestUserInput"),
                    "unexpected message: {message}"
                );
                assert!(
                    message.contains("unresolved"),
                    "unexpected message: {message}"
                );
                let context = context.expect("protocol error should include context");
                assert_eq!(
                    context.get("question_id").and_then(Value::as_str),
                    Some("confirm")
                );
                saw_protocol_error = true;
                break;
            }
            ProviderEvent::Completed { .. } => {
                panic!("expected protocol error before completion")
            }
            ProviderEvent::Failed { message } => {
                panic!("expected protocol error before failure: {message}")
            }
            _ => {}
        }
    }
    assert!(
        saw_protocol_error,
        "expected request_user_input_unresolved protocol error when JSON-RPC response write fails"
    );
}
