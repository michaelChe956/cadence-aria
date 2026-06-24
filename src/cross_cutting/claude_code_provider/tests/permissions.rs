use serde_json::json;

use crate::cross_cutting::streaming_provider::{
    ProviderCommand, ProviderEvent, ProviderSession, ProviderStatus, StreamingProviderAdapter,
};

use super::*;

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
