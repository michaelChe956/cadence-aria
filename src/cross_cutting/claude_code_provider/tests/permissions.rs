use serde_json::json;

use crate::cross_cutting::claude_code_provider::permission_mode_for_claude;
use crate::cross_cutting::streaming_provider::{
    ProviderCommand, ProviderEvent, ProviderSession, ProviderStatus, StreamingProviderAdapter,
};

use super::*;

async fn capture_initial_messages(permission_mode: ProviderPermissionMode) -> Vec<Value> {
    let mut child = tokio::process::Command::new("cat")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("spawn cat fixture");
    let stdin = Arc::new(Mutex::new(child.stdin.take().expect("child stdin")));
    let stdout = child.stdout.take().expect("child stdout");
    let input = streaming_input(ProviderType::ClaudeCode, permission_mode);

    ClaudeCodeProvider::write_initial_messages(&stdin, &input)
        .await
        .expect("write initial messages");
    drop(stdin);

    let mut lines = tokio::io::BufReader::new(stdout).lines();
    let mut messages = Vec::new();
    while let Some(line) = tokio::time::timeout(TEST_TIMEOUT, lines.next_line())
        .await
        .expect("initial message line timeout")
        .expect("read initial message line")
    {
        messages.push(serde_json::from_str(&line).expect("initial message json"));
    }
    let _ = tokio::time::timeout(TEST_TIMEOUT, child.wait())
        .await
        .expect("cat wait timeout")
        .expect("cat status");
    messages
}

#[test]
fn claude_supervised_permission_mode_maps_to_default() {
    assert_eq!(
        permission_mode_for_claude(&ProviderPermissionMode::Supervised),
        "default"
    );
}

#[test]
fn claude_auto_permission_mode_uses_default_so_aria_remains_authoritative() {
    assert_eq!(
        permission_mode_for_claude(&ProviderPermissionMode::Auto),
        "default"
    );
}

#[tokio::test]
async fn claude_initial_messages_send_only_valid_permission_modes() {
    for permission_mode in [
        ProviderPermissionMode::Auto,
        ProviderPermissionMode::Supervised,
    ] {
        let messages = capture_initial_messages(permission_mode).await;
        let request = &messages[1]["request"];
        assert_eq!(request["subtype"], "set_permission_mode");
        assert_eq!(request["mode"], "default");
        assert_ne!(request["mode"], "supervised");
    }
}

#[tokio::test]
async fn claude_auto_mode_routes_permission_request_through_auto_approval_bridge() {
    let fixture = write_fixture(
        "claude_auto_permission_fixture.sh",
        r##"#!/usr/bin/env bash
set -euo pipefail

if [[ "${1:-}" == "--version" ]]; then
  echo "claude 2.1.160"
  exit 0
fi

while IFS= read -r line; do
  if [[ "$line" == *'"initialize"'* || "$line" == *'"set_permission_mode"'* ]]; then
    continue
  fi
  if [[ "$line" == *'"user"'* ]]; then
    echo '{"type":"control_request","request_id":"perm_req_001","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"pwd","description":"Print current directory"},"tool_use_id":"toolu_bash"}}'
    continue
  fi
  if [[ "$line" == *'"control_response"'* ]]; then
    if [[ "$line" != *'"request_id":"perm_req_001"'* || "$line" != *'"behavior":"allow"'* ]]; then
      echo "missing auto approval control response: $line" >&2
      exit 41
    fi
    echo '{"type":"result","subtype":"success","is_error":false,"result":"auto approval completed","session_id":"claude_auto_permission_session"}'
    exit 0
  fi
done
"##,
    );
    let provider = ClaudeCodeProvider::new(fixture);
    let input = streaming_input(ProviderType::ClaudeCode, ProviderPermissionMode::Auto);
    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .expect("start provider");
    let mut saw_auto_approval = false;

    loop {
        match tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
            .await
            .expect("provider should complete")
            .expect("provider event channel should stay open")
        {
            ProviderEvent::Execution(event) if event.title == "Auto approval" => {
                saw_auto_approval = true;
            }
            ProviderEvent::PermissionRequest(request) => {
                panic!("Auto mode must not emit a user permission request: {request:?}")
            }
            ProviderEvent::Completed(completion) => {
                assert_eq!(completion.full_output, "auto approval completed");
                break;
            }
            ProviderEvent::Failed { message } => panic!("provider failed: {message}"),
            ProviderEvent::ProtocolError { message, .. } => {
                panic!("provider protocol error: {message}")
            }
            ProviderEvent::UsageReport(_) => {}
            ProviderEvent::PermissionTimeout { permission_id } => {
                panic!("provider permission timed out: {permission_id}")
            }
            ProviderEvent::StatusChanged(_)
            | ProviderEvent::Execution(_)
            | ProviderEvent::TextDelta { .. }
            | ProviderEvent::ChoiceRequest(_)
            | ProviderEvent::ToolCall(_)
            | ProviderEvent::ToolResult(_) => {}
        }
    }

    assert!(saw_auto_approval, "Auto approval must be audited");
}

#[tokio::test]
async fn claude_control_response_uses_sdk_success_envelope_for_approved_tool() {
    let payload = capture_tool_control_response(true, None).await;

    assert_eq!(payload["type"], "control_response");
    assert!(payload.get("request_id").is_none());
    assert_eq!(payload["response"]["subtype"], "success");
    assert_eq!(payload["response"]["request_id"], "perm_req_001");
    assert_eq!(payload["response"]["response"]["behavior"], "allow");
    assert!(payload["response"]["response"]["message"].is_null());
}
#[tokio::test]
async fn claude_control_response_uses_sdk_success_envelope_for_denied_tool() {
    let payload = capture_tool_control_response(false, Some("用户拒绝执行".to_string())).await;

    assert_eq!(payload["type"], "control_response");
    assert!(payload.get("request_id").is_none());
    assert_eq!(payload["response"]["subtype"], "success");
    assert_eq!(payload["response"]["request_id"], "perm_req_001");
    assert_eq!(payload["response"]["response"]["behavior"], "deny");
    assert_eq!(payload["response"]["response"]["message"], "用户拒绝执行");
}
#[tokio::test]
async fn claude_choice_control_response_uses_sdk_success_envelope_with_answers() {
    let original_input = json!({
        "questions": [{
            "question": "Drink?",
            "options": [
                { "label": "Tea" },
                { "label": "Coffee" }
            ]
        }]
    });
    let mut answers = Map::new();
    answers.insert("Drink?".to_string(), Value::String("Tea".to_string()));

    let payload = capture_choice_control_response(original_input, answers).await;

    assert_eq!(payload["type"], "control_response");
    assert!(payload.get("request_id").is_none());
    assert_eq!(payload["response"]["subtype"], "success");
    assert_eq!(payload["response"]["request_id"], "ask_req_001");
    assert_eq!(payload["response"]["response"]["behavior"], "allow");
    assert_eq!(
        payload["response"]["response"]["updatedInput"]["answers"]["Drink?"],
        "Tea"
    );
}
#[tokio::test]
async fn claude_provider_bridges_permission_and_completes() {
    let fixture = executable_fixture("tests/fixtures/provider/claude_stream_json_fixture.sh");
    let provider = ClaudeCodeProvider::new(fixture);
    let input = streaming_input(ProviderType::ClaudeCode, ProviderPermissionMode::Supervised);

    let mut session: ProviderSession = provider
        .start(input, CancellationToken::new())
        .await
        .unwrap();

    assert!(matches!(
        session.events.recv().await.unwrap(),
        ProviderEvent::StatusChanged(ProviderStatus::Starting)
    ));

    let mut saw_text = false;
    let permission_id = loop {
        match session.events.recv().await.unwrap() {
            ProviderEvent::TextDelta { content } => {
                saw_text = content.contains("# Story Spec")
                    && content.contains("## 功能需求")
                    && content.contains("## 成功标准");
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
    assert!(completed.contains("# Story Spec"));
    assert!(completed.contains("## 功能需求"));
    assert!(completed.contains("## 成功标准"));
}
