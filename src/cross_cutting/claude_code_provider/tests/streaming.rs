use crate::cross_cutting::streaming_provider::{
    ProviderEvent, ProviderExecutionEventStatus, ProviderPermissionMode, StreamingProviderAdapter,
};

use super::*;

#[tokio::test]
async fn claude_provider_emits_assistant_final_text_after_stream_delta_when_result_is_empty() {
    let fixture = write_fixture(
        "claude_partial_then_final_assistant_empty_result_fixture.sh",
        r##"#!/usr/bin/env bash
set -euo pipefail

if [[ "${1:-}" == "--version" ]]; then
  echo "claude 2.1.160"
  exit 0
fi

while IFS= read -r line; do
  if [[ "$line" == *'"initialize"'* ]]; then
    continue
  fi
  if [[ "$line" == *'"set_permission_mode"'* ]]; then
    continue
  fi
  if [[ "$line" == *'"user"'* ]]; then
    echo '{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"我先调研。\n"}},"session_id":"claude_fixture_session"}'
    echo '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"```artifact\n# 依赖自检查 Design Spec\n\n## 设计范围\n- [DEC-001] 设计入口。\n\n## 设计决策\n- [DEC-001] 使用轻量探测。\n\n## 公共组件\n- [CMP-001] ProviderDependencyDialog。\n\n## API 契约\n- [API-001] GET /api/provider-dependencies。\n\n## 数据模型\n- repository provider settings。\n\n## 风险\n- npm 缺失。\n\n## 追踪关系\n- REQ-001 -> DEC-001\n```"}]},"session_id":"claude_fixture_session"}'
    echo '{"type":"result","subtype":"success","is_error":false,"result":"","session_id":"claude_fixture_session"}'
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
    let mut streamed = String::new();
    let completed = loop {
        match tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
            .await
            .expect("provider should emit completion")
            .expect("provider event channel should stay open")
        {
            ProviderEvent::TextDelta { content } => streamed.push_str(&content),
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

    assert_eq!(completed, streamed);
    assert!(
        completed.contains("# 依赖自检查 Design Spec"),
        "completed output should fall back to visible assistant text, got: {completed}"
    );
    assert!(
        streamed.contains("# 依赖自检查 Design Spec"),
        "final assistant artifact should be emitted as stream text, got: {streamed}"
    );
}
#[tokio::test]
async fn claude_provider_reports_failure_when_process_exits_without_result() {
    let fixture = write_fixture(
        "claude_fail_fixture.sh",
        "#!/usr/bin/env bash\nset -euo pipefail\nexec 0<&-\necho 'not authenticated' >&2\nexit 7\n",
    );
    let provider = ClaudeCodeProvider::new(fixture);
    let input = streaming_input(ProviderType::ClaudeCode, ProviderPermissionMode::Auto);

    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .expect("start provider");

    let mut failed = None;
    while let Some(event) = tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
        .await
        .expect("provider should emit failure")
    {
        if let ProviderEvent::Failed { message } = event {
            failed = Some(message);
            break;
        }
    }

    let failed = failed.expect("provider should emit failed event");
    assert!(
        failed.contains("exited without result") || failed.contains("exit status"),
        "unexpected failure message: {failed}"
    );
    assert!(
        failed.contains("not authenticated"),
        "unexpected failure message: {failed}"
    );
}
#[tokio::test]
async fn claude_provider_truncates_multibyte_tool_result_preview_without_panicking() {
    let long_output = "通".repeat(180);
    let tool_use_line = serde_json::json!({
        "type": "assistant",
        "message": {
            "role": "assistant",
            "content": [{
                "type": "tool_use",
                "id": "toolu_utf8",
                "name": "Bash",
                "input": { "command": "printf unicode output" }
            }]
        },
        "session_id": "claude_fixture_session"
    })
    .to_string();
    let tool_result_line = serde_json::json!({
        "type": "user",
        "message": {
            "role": "user",
            "content": [{
                "type": "tool_result",
                "tool_use_id": "toolu_utf8",
                "content": long_output
            }]
        },
        "session_id": "claude_fixture_session"
    })
    .to_string();
    let result_line = serde_json::json!({
        "type": "result",
        "subtype": "success",
        "is_error": false,
        "result": "done",
        "session_id": "claude_fixture_session"
    })
    .to_string();
    let body = format!(
        "#!/usr/bin/env bash\nset -euo pipefail\nwhile IFS= read -r line; do\n  if [[ \"$line\" == *'\"user\"'* ]]; then\n    printf '%s\\n' '{tool_use_line}'\n    printf '%s\\n' '{tool_result_line}'\n    printf '%s\\n' '{result_line}'\n    exit 0\n  fi\ndone\n"
    );
    let fixture = write_fixture("claude_utf8_tool_result_fixture.sh", &body);
    let provider = ClaudeCodeProvider::new(fixture);
    let input = streaming_input(ProviderType::ClaudeCode, ProviderPermissionMode::Auto);

    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .expect("start provider");
    let mut preview = None;
    let mut completed = None;

    while let Some(event) = tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
        .await
        .expect("provider should not hang while handling utf8 tool result")
    {
        match event {
            ProviderEvent::Execution(event) => {
                if event.event_id == "toolu_utf8"
                    && event.status == ProviderExecutionEventStatus::Completed
                {
                    preview = event.output;
                }
            }
            ProviderEvent::Completed { full_output, .. } => {
                completed = Some(full_output);
                break;
            }
            ProviderEvent::Failed { message } => panic!("provider failed: {message}"),
            ProviderEvent::ProtocolError { message, .. } => {
                panic!("provider protocol error: {message}")
            }
            ProviderEvent::PermissionTimeout { permission_id } => {
                panic!("provider permission timed out: {permission_id}")
            }
            ProviderEvent::StatusChanged(_)
            | ProviderEvent::TextDelta { .. }
            | ProviderEvent::PermissionRequest(_)
            | ProviderEvent::ChoiceRequest(_)
            | ProviderEvent::ToolCall(_)
            | ProviderEvent::ToolResult(_) => {}
        }
    }

    assert_eq!(completed.as_deref(), Some("done"));
    let preview = preview.expect("tool result preview");
    assert!(preview.ends_with("..."), "preview should be truncated");
    assert!(
        preview.len() <= 503,
        "preview should be capped to 500 bytes plus suffix, got {} bytes",
        preview.len()
    );
}
#[tokio::test]
async fn claude_provider_run_streaming_cancel_closes_backpressured_bridge() {
    let mut body = String::from("#!/usr/bin/env bash\nset -euo pipefail\n");
    body.push_str("while IFS= read -r line; do\n");
    body.push_str("  if [[ \"$line\" == *'\"user\"'* ]]; then\n");
    for index in 0..80 {
        body.push_str(&format!(
                "    echo '{{\"type\":\"assistant\",\"message\":{{\"role\":\"assistant\",\"content\":[{{\"type\":\"text\",\"text\":\"chunk {index}\"}}]}},\"session_id\":\"backpressure\"}}'\n"
            ));
    }
    body.push_str("    sleep 5\n");
    body.push_str("  fi\n");
    body.push_str("done\n");

    let fixture = write_fixture("claude_backpressure_fixture.sh", &body);
    let provider = ClaudeCodeProvider::new(fixture);
    let cancel = CancellationToken::new();
    let rx = provider
        .run_streaming(&adapter_input("trigger backpressure"), cancel.clone())
        .await
        .expect("run streaming");

    wait_for_buffer_len(&rx, 32).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    cancel.cancel();
    tokio::time::timeout(TEST_TIMEOUT, wait_for_receiver_closed(&rx))
        .await
        .expect("stream receiver should close after cancellation");
}
