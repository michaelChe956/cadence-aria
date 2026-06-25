#[cfg(target_os = "linux")]
use super::*;
#[cfg(target_os = "linux")]
use crate::cross_cutting::streaming_provider::StreamingProviderAdapter;

#[tokio::test]
#[cfg(target_os = "linux")]
async fn claude_provider_cancel_kills_and_reaps_hanging_process() {
    let fixture = write_fixture(
        "claude_hanging_after_output_fixture.sh",
        "#!/usr/bin/env bash\nset -euo pipefail\nprintf '%s\\n' \"$$\" > \"$ARIA_FIXTURE_PID\"\nwhile IFS= read -r line; do\n  if [[ \"$line\" == *'\"user\"'* ]]; then\n    echo '{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"started\"}]},\"session_id\":\"claude_fixture_session\"}'\n    sleep 30\n  fi\ndone\n",
    );
    let provider = ClaudeCodeProvider::new(fixture);
    let pid_path = tempfile::NamedTempFile::new()
        .expect("pid file")
        .into_temp_path();
    let mut input = streaming_input(ProviderType::ClaudeCode, ProviderPermissionMode::Auto);
    input.env_vars.insert(
        "ARIA_FIXTURE_PID".to_string(),
        pid_path.to_string_lossy().to_string(),
    );
    let cancel = CancellationToken::new();

    let mut session = provider
        .start(input, cancel.clone())
        .await
        .expect("start provider");
    loop {
        match tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
            .await
            .expect("provider should emit startup event")
            .expect("provider channel should stay open until cancellation")
        {
            ProviderEvent::TextDelta { content } if content == "started" => break,
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
            | ProviderEvent::ChoiceRequest(_)
            | ProviderEvent::ToolCall(_)
            | ProviderEvent::ToolResult(_)
            | ProviderEvent::Completed { .. } => {}
        }
    }

    let pid = std::fs::read_to_string(&pid_path)
        .expect("fixture pid")
        .trim()
        .parse::<u32>()
        .expect("fixture pid number");
    cancel.cancel();
    drop(session);
    wait_for_process_absent(pid).await;
}
