mod support;

use cadence_aria::cross_cutting::adapter_compatibility::{
    PromptInputMode, StructuredOutputMode, default_compatibility_matrix,
    fixture_compatibility_entry,
};
use cadence_aria::cross_cutting::cli_adapter::{CliAdapterConfig, CliProviderAdapter};
use cadence_aria::cross_cutting::provider_adapter::ProviderAdapter;
use cadence_aria::cross_cutting::provider_capabilities::ProviderCapabilityProbe;
use cadence_aria::protocol::contracts::{AdapterInput, AdapterRole, ProviderType, TimeoutStatus};
use serde_json::json;

#[test]
fn fixture_provider_command_can_be_probed_without_real_claude_or_codex() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let command = support::write_executable_script(
        tempdir.path(),
        "fixture-provider",
        support::successful_provider_script(),
    );
    let compatibility = fixture_compatibility_entry(ProviderType::Fake, command.clone());

    let capability = ProviderCapabilityProbe::new(compatibility)
        .probe()
        .expect("probe fixture provider");

    assert_eq!(capability.provider_type, ProviderType::Fake);
    assert!(capability.provider_capability_ref.starts_with("cap_fake_"));
    assert_eq!(capability.command_path, command.to_string_lossy());
    assert_eq!(capability.version, "fixture-provider 1.2.3");
    assert_eq!(capability.install_source, "user_local_cli");
    assert_eq!(capability.supported_output_modes, vec!["sentinel_json"]);
    assert!(!capability.supports_session);
    assert!(!capability.supports_resume);
    assert!(!capability.probed_at.is_empty());
}

#[test]
fn default_matrix_contains_claude_code_and_codex_cli_entries() {
    let matrix = default_compatibility_matrix();
    let claude = matrix
        .entry_for(ProviderType::ClaudeCode)
        .expect("claude code entry");
    let codex = matrix.entry_for(ProviderType::Codex).expect("codex entry");

    assert_eq!(claude.provider_type, ProviderType::ClaudeCode);
    assert_eq!(codex.provider_type, ProviderType::Codex);
    assert!(!claude.matrix_version.is_empty());
    assert!(!codex.matrix_version.is_empty());
    assert_eq!(claude.prompt_input_mode, PromptInputMode::Stdin);
    assert_eq!(codex.prompt_input_mode, PromptInputMode::Stdin);
    assert_eq!(
        claude.structured_output_mode,
        StructuredOutputMode::SentinelJson
    );
    assert_eq!(
        codex.structured_output_mode,
        StructuredOutputMode::SentinelJson
    );
    assert!(
        claude
            .unauthorized_patterns
            .iter()
            .any(|pattern| pattern.contains("not logged in"))
    );
    assert!(
        codex
            .permission_denied_patterns
            .iter()
            .any(|pattern| pattern.contains("permission denied"))
    );
}

#[test]
fn cli_adapter_spawns_fixture_command_parses_sentinel_and_detects_modified_files() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let command = support::write_executable_script(
        tempdir.path(),
        "fixture-provider",
        support::successful_provider_script(),
    );
    let worktree = tempfile::tempdir().expect("worktree");
    let compatibility = fixture_compatibility_entry(ProviderType::Fake, command);
    let adapter = CliProviderAdapter::new(CliAdapterConfig {
        compatibility,
        expected_artifact_kind: Some("clarification_record".to_string()),
    });

    let output = adapter
        .run(&adapter_input(worktree.path()))
        .expect("cli run");

    assert_eq!(output.exit_code, Some(0));
    assert_eq!(output.timeout_status, TimeoutStatus::NotTimedOut);
    assert_eq!(output.stderr, "");
    assert_eq!(
        output
            .structured_output
            .as_ref()
            .expect("structured output")["artifact_kind"],
        json!("clarification_record")
    );
    assert_eq!(output.files_modified, vec!["generated.txt".to_string()]);
}

#[test]
fn missing_provider_command_is_diagnostic_not_a_panic() {
    let compatibility = fixture_compatibility_entry(
        ProviderType::Fake,
        temp_path("missing-provider-command-never-created"),
    );

    let error = ProviderCapabilityProbe::new(compatibility)
        .probe()
        .expect_err("missing command should be diagnostic");

    assert_eq!(error.code.as_str(), "provider_command_missing");
    assert!(error.details.contains("missing-provider-command"));
}

fn adapter_input(worktree_path: &std::path::Path) -> AdapterInput {
    AdapterInput {
        provider_type: ProviderType::Fake,
        role: AdapterRole::Orchestrator,
        worktree_path: Some(worktree_path.to_string_lossy().to_string()),
        prompt: "fixture prompt".to_string(),
        context_files: Vec::new(),
        output_schema: "clarification_record.v1".to_string(),
        timeout: 3,
        max_retries: 1,
    }
}

fn temp_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(name)
}
