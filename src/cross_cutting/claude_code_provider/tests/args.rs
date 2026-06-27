use std::path::PathBuf;

use crate::cross_cutting::streaming_provider::{ProviderPermissionMode, StreamingProviderAdapter};

use super::super::ClaudeCodeProvider;

#[test]
fn claude_code_provider_supports_provider_driven_testing() {
    let provider = ClaudeCodeProvider::new(PathBuf::from("claude"));

    assert!(provider.supports_provider_driven_testing());
}
#[test]
fn claude_args_include_resume_when_provider_session_is_available() {
    let provider = ClaudeCodeProvider::new(PathBuf::from("claude"));
    let args = provider.build_args(
        ProviderPermissionMode::Supervised,
        Some("claude-session-123"),
    );

    assert!(args.contains(&"--resume".to_string()));
    assert!(args.contains(&"claude-session-123".to_string()));
    assert!(!args.contains(&"--continue".to_string()));
    assert!(!args.contains(&"--fork-session".to_string()));
}
#[test]
fn claude_args_do_not_include_resume_without_provider_session() {
    let provider = ClaudeCodeProvider::new(PathBuf::from("claude"));
    let args = provider.build_args(ProviderPermissionMode::Supervised, None);

    assert!(!args.contains(&"--resume".to_string()));
    assert!(!args.contains(&"--continue".to_string()));
}
