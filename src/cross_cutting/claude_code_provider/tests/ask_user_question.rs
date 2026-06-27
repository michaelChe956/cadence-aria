use serde_json::json;

use crate::cross_cutting::streaming_provider::{
    ProviderCommand, ProviderEvent, ProviderSession, ProviderStatus, StreamingProviderAdapter,
};

use super::*;

#[test]
fn claude_control_request_reads_top_level_tool_use_id() {
    let value = json!({
        "type": "control_request",
        "request_id": "ask_req_001",
        "tool_use_id": "toolu_question",
        "request": {
            "subtype": "can_use_tool",
            "tool_name": "AskUserQuestion",
            "input": {
                "questions": [{
                    "question": "Drink?",
                    "options": [{ "label": "Tea" }]
                }]
            }
        }
    });

    let request = ClaudeCodeProvider::parse_control_request(&value).expect("control request");

    assert_eq!(request.request_id, "ask_req_001");
    assert_eq!(request.tool_name, "AskUserQuestion");
    assert_eq!(request.tool_use_id.as_deref(), Some("toolu_question"));
}
#[tokio::test]
async fn claude_provider_continues_same_session_after_ask_user_question_choice() {
    let fixture = executable_fixture("tests/fixtures/provider/claude_ask_user_question_fixture.sh");
    let provider = ClaudeCodeProvider::new(fixture);
    let input = streaming_input(ProviderType::ClaudeCode, ProviderPermissionMode::Supervised);

    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .expect("start provider");

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
            ProviderEvent::ProtocolError { message, .. } => {
                panic!("provider protocol error before choice: {message}")
            }
            ProviderEvent::PermissionTimeout { permission_id } => {
                panic!("provider permission timed out before choice: {permission_id}")
            }
            ProviderEvent::StatusChanged(_)
            | ProviderEvent::Execution(_)
            | ProviderEvent::TextDelta { .. }
            | ProviderEvent::PermissionRequest(_)
            | ProviderEvent::ToolCall(_)
            | ProviderEvent::ToolResult(_)
            | ProviderEvent::Completed { .. } => {}
        }
    };
    assert_eq!(choice.id, "ask_req_001");
    assert_eq!(choice.options[0].id, "opt_0");

    session
        .commands
        .send(ProviderCommand::ChoiceResponse {
            id: choice.id,
            selected_option_ids: vec!["opt_0".to_string()],
            free_text: None,
        })
        .await
        .expect("send choice response");

    let completed = recv_completed(&mut session.events).await;
    assert!(completed.contains("# Story Spec"));
    assert!(completed.contains("## 功能需求"));
    assert!(completed.contains("## 成功标准"));
}
#[tokio::test]
async fn claude_provider_deduplicates_assistant_then_control_ask_user_question() {
    let fixture = write_fixture(
        "claude_assistant_then_control_ask_user_question_fixture.sh",
        r##"#!/usr/bin/env bash
set -euo pipefail

if [[ "${1:-}" == "--version" ]]; then
  echo "claude 2.1.160"
  exit 0
fi

sent_question=0
while IFS= read -r line; do
  if [[ "$line" == *'"initialize"'* ]]; then
    continue
  fi
  if [[ "$line" == *'"set_permission_mode"'* ]]; then
    continue
  fi
  if [[ "$sent_question" == "0" && "$line" == *'"user"'* ]]; then
    sent_question=1
    echo '{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"toolu_question","name":"AskUserQuestion","input":{"questions":[{"question":"Scope?","options":[{"label":"Global"},{"label":"Project"}]}]}}]}}'
    continue
  fi
  if [[ "$line" == *'"tool_result"'* ]]; then
    if [[ "$line" != *'"tool_use_id":"toolu_question"'* || "$line" != *'Scope?'* || "$line" != *'Global'* ]]; then
      echo "missing AskUserQuestion tool_result: $line" >&2
      exit 44
    fi
    echo '{"type":"control_request","request_id":"ask_req_duplicate","request":{"subtype":"can_use_tool","tool_name":"AskUserQuestion","input":{"questions":[{"question":"Scope?","options":[{"label":"Global"},{"label":"Project"}]}]},"tool_use_id":"toolu_question"}}'
    continue
  fi
  if [[ "$line" == *'"control_response"'* ]]; then
    if [[ "$line" != *'"request_id":"ask_req_duplicate"'* || "$line" != *'"updatedInput"'* || "$line" != *'Scope?'* || "$line" != *'Global'* ]]; then
      echo "missing reused AskUserQuestion control_response: $line" >&2
      exit 45
    fi
    echo '{"type":"result","subtype":"success","is_error":false,"result":"# Story Spec\n\n## 功能需求\n- [REQ-001] Duplicate AskUserQuestion events are answered once.\n\n## 成功标准\n- [AC-001] The provider completes without a second frontend choice.","session_id":"claude_fixture_session"}'
    exit 0
  fi
done
"##,
    );
    let provider = ClaudeCodeProvider::new(fixture);
    let input = streaming_input(ProviderType::ClaudeCode, ProviderPermissionMode::Supervised);

    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .expect("start provider");

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
            ProviderEvent::ProtocolError { message, .. } => {
                panic!("provider protocol error before choice: {message}")
            }
            ProviderEvent::PermissionTimeout { permission_id } => {
                panic!("provider permission timed out before choice: {permission_id}")
            }
            ProviderEvent::StatusChanged(_)
            | ProviderEvent::Execution(_)
            | ProviderEvent::TextDelta { .. }
            | ProviderEvent::PermissionRequest(_)
            | ProviderEvent::ToolCall(_)
            | ProviderEvent::ToolResult(_)
            | ProviderEvent::Completed { .. } => {}
        }
    };
    assert_eq!(choice.id, "toolu_question");

    session
        .commands
        .send(ProviderCommand::ChoiceResponse {
            id: choice.id,
            selected_option_ids: vec!["opt_0".to_string()],
            free_text: None,
        })
        .await
        .expect("send choice response");

    loop {
        match tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
            .await
            .expect("provider should emit completion")
            .expect("provider event channel should stay open")
        {
            ProviderEvent::Completed { full_output, .. } => {
                assert!(full_output.contains("# Story Spec"));
                break;
            }
            ProviderEvent::ChoiceRequest(choice) => {
                panic!(
                    "duplicate AskUserQuestion should not be emitted: {}",
                    choice.id
                )
            }
            ProviderEvent::Failed { message } => panic!("provider failed: {message}"),
            ProviderEvent::ProtocolError { message, .. } => {
                panic!("provider protocol error: {message}")
            }
            ProviderEvent::PermissionTimeout { permission_id } => {
                panic!("provider permission timed out: {permission_id}")
            }
            ProviderEvent::StatusChanged(_)
            | ProviderEvent::Execution(_)
            | ProviderEvent::TextDelta { .. }
            | ProviderEvent::PermissionRequest(_)
            | ProviderEvent::ToolCall(_)
            | ProviderEvent::ToolResult(_) => {}
        }
    }
}
#[tokio::test]
async fn claude_provider_abort_during_ask_user_question_closes_without_completion() {
    let fixture = write_fixture(
        "claude_ask_user_question_abort_fixture.sh",
        r#"#!/usr/bin/env bash
set -euo pipefail

if [[ "${1:-}" == "--version" ]]; then
  echo "claude 2.1.160"
  exit 0
fi

printf '%s\n' "$$" > "$ARIA_FIXTURE_PID"

while IFS= read -r line; do
  if [[ "$line" == *'"initialize"'* ]]; then
    continue
  fi
  if [[ "$line" == *'"set_permission_mode"'* ]]; then
    continue
  fi
  if [[ "$line" == *'"user"'* ]]; then
    echo '{"type":"control_request","request_id":"ask_abort_001","request":{"subtype":"can_use_tool","tool_name":"AskUserQuestion","input":{"questions":[{"question":"Continue?","options":[{"label":"Yes"},{"label":"No"}]}]},"tool_use_id":"toolu_abort_question"}}'
  fi
  if [[ "$line" == *'"control_response"'* ]]; then
    if [[ "$line" != *'"subtype":"success"'* ]]; then
      echo "missing SDK success subtype during abort: $line" >&2
      exit 42
    fi
    if [[ "$line" != *'"updatedInput"'* || "$line" != *'"answers"'* || "$line" != *'"Continue?"'* || "$line" != *'"aborted"'* ]]; then
      echo "missing aborted answer: $line" >&2
      exit 43
    fi
    printf 'aborted-control-response\n' > "$ARIA_ABORT_MARKER"
    sleep 30
  fi
done
"#,
    );
    let marker_dir = tempfile::tempdir().expect("marker dir");
    let marker_path = marker_dir.path().join("abort_marker");
    let pid_path = tempfile::NamedTempFile::new()
        .expect("pid file")
        .into_temp_path();
    let provider = ClaudeCodeProvider::new(fixture);
    let mut input = streaming_input(ProviderType::ClaudeCode, ProviderPermissionMode::Supervised);
    input.env_vars.insert(
        "ARIA_ABORT_MARKER".to_string(),
        marker_path.to_string_lossy().to_string(),
    );
    input.env_vars.insert(
        "ARIA_FIXTURE_PID".to_string(),
        pid_path.to_string_lossy().to_string(),
    );
    let cancel = CancellationToken::new();

    let ProviderSession {
        mut events,
        commands,
    } = provider
        .start(input, cancel.clone())
        .await
        .expect("start provider");

    let choice = loop {
        match tokio::time::timeout(TEST_TIMEOUT, events.recv())
            .await
            .expect("provider should emit choice")
            .expect("provider event channel should stay open")
        {
            ProviderEvent::ChoiceRequest(choice) => break choice,
            ProviderEvent::Failed { message } => {
                panic!("provider failed before choice: {message}")
            }
            ProviderEvent::ProtocolError { message, .. } => {
                panic!("provider protocol error before choice: {message}")
            }
            ProviderEvent::PermissionTimeout { permission_id } => {
                panic!("provider permission timed out before choice: {permission_id}")
            }
            ProviderEvent::StatusChanged(_)
            | ProviderEvent::Execution(_)
            | ProviderEvent::TextDelta { .. }
            | ProviderEvent::PermissionRequest(_)
            | ProviderEvent::ToolCall(_)
            | ProviderEvent::ToolResult(_)
            | ProviderEvent::Completed { .. } => {}
        }
    };
    assert_eq!(choice.id, "ask_abort_001");

    commands
        .send(ProviderCommand::Abort)
        .await
        .expect("send abort command");
    drop(commands);
    wait_for_file(&marker_path).await;

    cancel.cancel();
    let mut saw_aborted = false;
    loop {
        let event = tokio::time::timeout(TEST_TIMEOUT, events.recv())
                .await
                .unwrap_or_else(|_| {
                    panic!(
                        "provider receiver did not close after abort cancellation; saw_aborted={saw_aborted}"
                    )
                });
        let Some(event) = event else {
            break;
        };
        match event {
            ProviderEvent::StatusChanged(ProviderStatus::Aborted) => saw_aborted = true,
            ProviderEvent::Completed { .. } => {
                panic!("aborted AskUserQuestion provider should not complete")
            }
            ProviderEvent::Failed { message } => {
                panic!("provider failed during abort: {message}")
            }
            ProviderEvent::ProtocolError { message, .. } => {
                panic!("provider protocol error during abort: {message}")
            }
            ProviderEvent::PermissionTimeout { permission_id } => {
                panic!("provider permission timed out during abort: {permission_id}")
            }
            ProviderEvent::StatusChanged(_)
            | ProviderEvent::Execution(_)
            | ProviderEvent::TextDelta { .. }
            | ProviderEvent::PermissionRequest(_)
            | ProviderEvent::ChoiceRequest(_)
            | ProviderEvent::ToolCall(_)
            | ProviderEvent::ToolResult(_) => {}
        }
    }
    assert!(saw_aborted);

    #[cfg(target_os = "linux")]
    {
        let pid = std::fs::read_to_string(&pid_path)
            .expect("fixture pid")
            .trim()
            .parse::<u32>()
            .expect("fixture pid number");
        wait_for_process_absent(pid).await;
    }
}
#[tokio::test]
async fn claude_provider_ask_user_question_emits_protocol_error_on_bridge_failure() {
    let fixture = executable_fixture("tests/fixtures/provider/claude_ask_user_question_fixture.sh");
    let provider = ClaudeCodeProvider::new(fixture);
    let input = streaming_input(ProviderType::ClaudeCode, ProviderPermissionMode::Supervised);
    let cancel = CancellationToken::new();

    let mut session = provider
        .start(input, cancel.clone())
        .await
        .expect("start provider");

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
            ProviderEvent::ProtocolError { message, .. } => {
                panic!("provider protocol error before choice: {message}")
            }
            _ => {}
        }
    };

    // 取消会话，让 bridge 的 request_choice 返回错误，同时保持 receiver 打开以接收 ProtocolError。
    cancel.cancel();

    // provider 应该抛出 ProtocolError，而不是通用的 Failed。
    let mut saw_protocol_error = false;
    while let Some(event) = tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
        .await
        .unwrap_or(None)
    {
        if let ProviderEvent::ProtocolError {
            code,
            message,
            context,
        } = event
        {
            assert_eq!(code, "ask_user_question_unresolved");
            assert!(
                message.contains("AskUserQuestion"),
                "message should mention AskUserQuestion: {message}"
            );
            assert!(
                message.contains("unresolved"),
                "message should mention unresolved: {message}"
            );
            let ctx = context.expect("context should be present");
            assert_eq!(ctx["request_id"], "ask_req_001");
            assert_eq!(ctx["tool_use_id"], "toolu_question");
            saw_protocol_error = true;
            break;
        }
    }
    assert!(
        saw_protocol_error,
        "expected ask_user_question_unresolved protocol error after bridge failure"
    );
}
#[tokio::test]
async fn claude_provider_ask_user_question_tool_use_emits_protocol_error_on_bridge_failure() {
    let fixture = executable_fixture(
        "tests/fixtures/provider/claude_ask_user_question_tool_use_bridge_failure_fixture.sh",
    );
    let provider = ClaudeCodeProvider::new(fixture);
    let input = streaming_input(ProviderType::ClaudeCode, ProviderPermissionMode::Supervised);
    let cancel = CancellationToken::new();

    let mut session = provider
        .start(input, cancel.clone())
        .await
        .expect("start provider");

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
            ProviderEvent::ProtocolError { message, .. } => {
                panic!("provider protocol error before choice: {message}")
            }
            _ => {}
        }
    };

    cancel.cancel();

    let mut saw_protocol_error = false;
    while let Some(event) = tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
        .await
        .unwrap_or(None)
    {
        if let ProviderEvent::ProtocolError {
            code,
            message,
            context,
        } = event
        {
            assert_eq!(code, "ask_user_question_unresolved");
            assert!(
                message.contains("AskUserQuestion"),
                "message should mention AskUserQuestion: {message}"
            );
            assert!(
                message.contains("unresolved"),
                "message should mention unresolved: {message}"
            );
            let ctx = context.expect("context should be present");
            assert_eq!(ctx["tool_use_id"], "toolu_question");
            saw_protocol_error = true;
            break;
        }
    }
    assert!(
        saw_protocol_error,
        "expected ask_user_question_unresolved protocol error after tool_use bridge failure"
    );
}
#[tokio::test]
async fn claude_provider_ask_user_question_emits_protocol_error_on_tool_result_error() {
    let fixture = executable_fixture(
        "tests/fixtures/provider/claude_ask_user_question_tool_error_fixture.sh",
    );
    let provider = ClaudeCodeProvider::new(fixture);
    let input = streaming_input(ProviderType::ClaudeCode, ProviderPermissionMode::Supervised);

    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .expect("start provider");

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
    assert_eq!(choice.id, "toolu_question");

    session
        .commands
        .send(ProviderCommand::ChoiceResponse {
            id: choice.id,
            selected_option_ids: vec!["opt_0".to_string()],
            free_text: None,
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
            } if code == "ask_user_question_unresolved" => {
                assert!(
                    message.contains("AskUserQuestion"),
                    "message should mention AskUserQuestion: {message}"
                );
                assert!(
                    message.contains("unresolved"),
                    "message should mention unresolved: {message}"
                );
                let ctx = context.expect("context should be present");
                assert_eq!(ctx["tool_use_id"], "toolu_question");
                assert_eq!(ctx["output"], "User refused to answer");
                saw_protocol_error = true;
                break;
            }
            ProviderEvent::Completed { .. } => {
                panic!("provider should not complete after AskUserQuestion tool_result error")
            }
            _ => {}
        }
    }
    assert!(
        saw_protocol_error,
        "expected ask_user_question_unresolved protocol error on tool_result is_error"
    );
}
