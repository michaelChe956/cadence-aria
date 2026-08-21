use std::path::PathBuf;

use crate::cross_cutting::streaming_provider::{ProviderPermissionMode, StreamingProviderAdapter};
use tokio_util::sync::CancellationToken;

use super::*;

#[test]
fn claude_args_include_resume_when_provider_session_is_available() {
    let provider = ClaudeCodeProvider::new(PathBuf::from("claude"));
    let args = provider.build_args(Some("claude-session-123"));

    assert!(args.contains(&"--resume".to_string()));
    assert!(args.contains(&"claude-session-123".to_string()));
    assert!(!args.contains(&"--continue".to_string()));
    assert!(!args.contains(&"--fork-session".to_string()));
}
#[test]
fn claude_args_do_not_include_resume_without_provider_session() {
    let provider = ClaudeCodeProvider::new(PathBuf::from("claude"));
    let args = provider.build_args(None);

    assert!(!args.contains(&"--resume".to_string()));
    assert!(!args.contains(&"--continue".to_string()));
}

#[tokio::test]
async fn claude_args_always_include_stdio_permission_prompt() {
    for (mode, name) in [
        (ProviderPermissionMode::Auto, "auto"),
        (ProviderPermissionMode::Supervised, "supervised"),
    ] {
        let fixture = write_fixture(
            &format!("claude_{name}_stdio_args_fixture.sh"),
            r##"#!/usr/bin/env bash
set -euo pipefail

if [[ "${1:-}" == "--version" ]]; then
  echo "claude 2.1.160"
  exit 0
fi

stdio_count=0
for arg in "$@"; do
  if [[ "$arg" == "--permission-prompt-tool=stdio" ]]; then
    stdio_count=$((stdio_count + 1))
  fi
done
if [[ "$stdio_count" != "1" ]]; then
  echo "expected exactly one stdio permission callback, got $stdio_count: $*" >&2
  exit 41
fi

while IFS= read -r line; do
  if [[ "$line" == *'"user"'* ]]; then
    echo '{"type":"result","subtype":"success","is_error":false,"result":"stdio callback registered","session_id":"claude_args_session"}'
    exit 0
  fi
done
"##,
        );
        let provider = ClaudeCodeProvider::new(fixture);
        let input = streaming_input(ProviderType::ClaudeCode, mode);
        let mut session = provider
            .start(input, CancellationToken::new())
            .await
            .expect("start provider");

        assert_eq!(
            recv_completed(&mut session.events).await,
            "stdio callback registered",
            "{name} mode must register stdio exactly once"
        );
    }
}
