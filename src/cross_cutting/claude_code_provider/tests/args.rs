use std::path::PathBuf;

use crate::cross_cutting::streaming_provider::{
    ProviderPermissionMode, ProviderToolPolicy, StreamingProviderAdapter,
};
use tokio_util::sync::CancellationToken;

use super::*;

#[test]
fn claude_policy_args_include_frozen_denylist_with_resume() {
    let provider = ClaudeCodeProvider::new(PathBuf::from("claude"));
    let deny = ProviderToolPolicy::deny_file_write_builtins();
    let args = provider.build_args(Some("claude-session-7"), Some(&deny));
    assert!(
        args.windows(2)
            .any(|w| w == ["--disallowedTools", "Edit,Write,NotebookEdit"])
    );
    assert!(
        args.windows(2)
            .any(|w| w == ["--resume", "claude-session-7"])
    );
    assert_eq!(
        args.iter()
            .filter(|arg| arg.as_str() == "--disallowedTools")
            .count(),
        1
    );
    let fresh = provider.build_args(None, Some(&deny));
    assert!(
        fresh
            .windows(2)
            .any(|w| w == ["--disallowedTools", "Edit,Write,NotebookEdit"])
    );
    assert!(!fresh.contains(&"--resume".to_string()));
}

#[test]
fn claude_args_include_resume_when_provider_session_is_available() {
    let provider = ClaudeCodeProvider::new(PathBuf::from("claude"));
    let args = provider.build_args(Some("claude-session-123"), None);

    assert!(args.contains(&"--resume".to_string()));
    assert!(args.contains(&"claude-session-123".to_string()));
    assert!(!args.contains(&"--continue".to_string()));
    assert!(!args.contains(&"--fork-session".to_string()));
}
#[test]
fn claude_args_do_not_include_resume_without_provider_session() {
    let provider = ClaudeCodeProvider::new(PathBuf::from("claude"));
    let args = provider.build_args(None, None);

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

/// Task 4a Step 1(断言组 296-297 逐字):LC validated argv 必须携带冻结的
/// deny token 片段与 headless MCP allowlist;无通用 tool policy 的 Coding
/// 角色无 deny token 但 allowlist 仍在。
#[test]
fn lcg_t04_claude_projection_has_headless_mcp_allowlist_and_deny_tokens() {
    let provider = ClaudeCodeProvider::new(PathBuf::from("claude"));
    let deny = ProviderToolPolicy::deny_file_write_builtins();

    // 策略角色(Planning/Review):deny token 冻结片段逐字 + allowlist。
    let claude_args = provider.build_lc_validated_args(Some("sess-lc-7"), Some(&deny));
    assert!(
        claude_args
            .windows(2)
            .any(|p| p == ["--disallowedTools", "Edit,Write,NotebookEdit"])
    );
    assert!(claude_args.iter().any(|arg| arg == "--allowedTools"));
    // allowlist 值成对跟随,且只列既有合法只读内建工具(不含写通道/MCP 写工具)。
    assert!(claude_args.windows(2).any(|p| p
        == [
            "--allowedTools",
            crate::cross_cutting::claude_code_provider::projection::CLAUDE_LC_ALLOWED_TOOLS
        ]));

    // resume 片段沿用;deny/allowlist 互不干扰。
    assert!(claude_args.contains(&"--resume".to_string()));
    assert!(claude_args.contains(&"sess-lc-7".to_string()));

    // 无通用策略的 Coding 角色:无 deny token,allowlist 仍在。
    let coding_args = provider.build_lc_validated_args(None, None);
    assert!(coding_args.iter().any(|arg| arg == "--allowedTools"));
    assert!(!coding_args.contains(&"--disallowedTools".to_string()));
    assert!(!coding_args.contains(&"--resume".to_string()));
    assert!(coding_args.contains(&"--permission-prompt-tool=stdio".to_string()));
}
