use super::support;

use cadence_aria::cross_cutting::adapter_compatibility::{
    PromptInputMode, StructuredOutputMode, default_compatibility_matrix,
    fixture_compatibility_entry,
};
use cadence_aria::cross_cutting::cli_adapter::{CliAdapterConfig, CliProviderAdapter};
use cadence_aria::cross_cutting::provider_adapter::ProviderAdapter;
use cadence_aria::cross_cutting::provider_capabilities::ProviderCapabilityProbe;
use cadence_aria::protocol::contracts::{AdapterInput, AdapterRole, ProviderType, TimeoutStatus};
use serde_json::json;
use std::fs;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

static CLI_ADAPTER_TEST_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn fixture_provider_command_can_be_probed_without_real_claude_or_codex() {
    let _guard = cli_adapter_test_guard();
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
        claude.run_command.args,
        vec![
            "-p".to_string(),
            "--permission-mode".to_string(),
            "dontAsk".to_string(),
            "--tools".to_string(),
            "".to_string(),
            "--strict-mcp-config".to_string(),
            "--no-session-persistence".to_string(),
        ]
    );
    assert_eq!(
        codex.run_command.args,
        vec![
            "exec".to_string(),
            "-s".to_string(),
            "danger-full-access".to_string()
        ]
    );
    assert!(!claude.pass_worktree_path_as_arg);
    assert!(!codex.pass_worktree_path_as_arg);
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
    let _guard = cli_adapter_test_guard();
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
        output_sink: None,
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
fn cli_adapter_can_run_without_passing_worktree_as_positional_arg() {
    let _guard = cli_adapter_test_guard();
    let tempdir = tempfile::tempdir().expect("tempdir");
    let command = support::write_executable_script(
        tempdir.path(),
        "fixture-provider",
        r#"#!/bin/sh
set -eu
if [ "${2:-}" != "" ]; then
  echo "unexpected worktree arg: $2" >&2
  exit 7
fi
cat >/dev/null
echo "<ARIA_STRUCTURED_OUTPUT>"
echo '{"artifact_kind":"clarification_record","goal_summary":"fixture goal","constraints":[],"open_questions":[],"assumptions":[],"suggested_scope":"fixture scope"}'
echo "</ARIA_STRUCTURED_OUTPUT>"
"#,
    );
    let worktree = tempfile::tempdir().expect("worktree");
    let mut compatibility = fixture_compatibility_entry(ProviderType::Fake, command);
    compatibility.pass_worktree_path_as_arg = false;
    let adapter = CliProviderAdapter::new(CliAdapterConfig {
        compatibility,
        expected_artifact_kind: Some("clarification_record".to_string()),
        output_sink: None,
    });

    let output = adapter
        .run(&adapter_input(worktree.path()))
        .expect("cli run");

    assert_eq!(
        output
            .structured_output
            .as_ref()
            .expect("structured output")["artifact_kind"],
        json!("clarification_record")
    );
}

#[test]
fn cli_adapter_drains_large_provider_output_while_waiting_for_exit() {
    let _guard = cli_adapter_test_guard();
    let tempdir = tempfile::tempdir().expect("tempdir");
    let command = support::write_executable_script(
        tempdir.path(),
        "chatty-provider",
        r#"#!/bin/sh
set -eu
case "${1:-run}" in
  probe|version|auth)
    echo "ok"
    exit 0
    ;;
  run)
    cat >/dev/null
    i=0
    while [ "$i" -lt 20000 ]; do
      echo "provider log line $i with enough bytes to fill stdout pipe before process exit"
      i=$((i + 1))
    done
    echo "<ARIA_STRUCTURED_OUTPUT>"
    echo '{"artifact_kind":"clarification_record","goal_summary":"fixture goal","constraints":[],"open_questions":[],"assumptions":[],"suggested_scope":"fixture scope"}'
    echo "</ARIA_STRUCTURED_OUTPUT>"
    exit 0
    ;;
esac
"#,
    );
    let worktree = tempfile::tempdir().expect("worktree");
    let compatibility = fixture_compatibility_entry(ProviderType::Fake, command);
    let adapter = CliProviderAdapter::new(CliAdapterConfig {
        compatibility,
        expected_artifact_kind: Some("clarification_record".to_string()),
        output_sink: None,
    });

    let output = adapter
        .run(&adapter_input(worktree.path()))
        .expect("cli run should not deadlock on large stdout");

    assert!(output.stdout.contains("provider log line 19999"));
    assert_eq!(
        output
            .structured_output
            .as_ref()
            .expect("structured output")["artifact_kind"],
        json!("clarification_record")
    );
}

#[test]
fn cli_adapter_finishes_after_structured_output_even_if_provider_keeps_running() {
    let _guard = cli_adapter_test_guard();
    let tempdir = tempfile::tempdir().expect("tempdir");
    let command = support::write_executable_script(
        tempdir.path(),
        "slow-exit-provider",
        r#"#!/bin/sh
set -eu
case "${1:-run}" in
  probe|version|auth)
    echo "ok"
    exit 0
    ;;
  run)
    cat >/dev/null
    echo "<ARIA_STRUCTURED_OUTPUT>"
    echo '{"artifact_kind":"clarification_record","goal_summary":"fixture goal","constraints":[],"open_questions":[],"assumptions":[],"suggested_scope":"fixture scope"}'
    echo "</ARIA_STRUCTURED_OUTPUT>"
    sleep 30
    exit 0
    ;;
esac
"#,
    );
    let worktree = tempfile::tempdir().expect("worktree");
    let compatibility = fixture_compatibility_entry(ProviderType::Fake, command);
    let adapter = CliProviderAdapter::new(CliAdapterConfig {
        compatibility,
        expected_artifact_kind: Some("clarification_record".to_string()),
        output_sink: None,
    });

    let started = Instant::now();
    let output = adapter
        .run(&adapter_input(worktree.path()))
        .expect("cli run should use complete structured output without waiting for process exit");

    assert!(
        started.elapsed().as_secs() < 2,
        "adapter waited too long after structured output"
    );
    assert_eq!(output.exit_code, Some(0));
    assert_eq!(output.timeout_status, TimeoutStatus::NotTimedOut);
    assert_eq!(
        output
            .structured_output
            .as_ref()
            .expect("structured output")["artifact_kind"],
        json!("clarification_record")
    );
}

#[test]
fn cli_adapter_streams_provider_stdout_and_stderr_to_runtime_logs() {
    let _guard = cli_adapter_test_guard();
    let tempdir = tempfile::tempdir().expect("tempdir");
    let command = support::write_executable_script(
        tempdir.path(),
        "streaming-provider",
        r#"#!/bin/sh
set -eu
case "${1:-run}" in
  probe|version|auth)
    echo "ok"
    exit 0
    ;;
  run)
    cat >/dev/null
    echo "stdout before structured output"
    echo "stderr before structured output" >&2
    echo "<ARIA_STRUCTURED_OUTPUT>"
    echo '{"artifact_kind":"clarification_record","goal_summary":"fixture goal","constraints":[],"open_questions":[],"assumptions":[],"suggested_scope":"fixture scope"}'
    echo "</ARIA_STRUCTURED_OUTPUT>"
    exit 0
    ;;
esac
"#,
    );
    let worktree = tempfile::tempdir().expect("worktree");
    let compatibility = fixture_compatibility_entry(ProviderType::Fake, command);
    let adapter = CliProviderAdapter::new(CliAdapterConfig {
        compatibility,
        expected_artifact_kind: Some("clarification_record".to_string()),
        output_sink: None,
    });

    let output = adapter
        .run(&adapter_input(worktree.path()))
        .expect("cli run");

    let stream_dir = worktree.path().join(".aria/runtime/provider-streams");
    let mut combined_logs = String::new();
    for entry in fs::read_dir(&stream_dir)
        .unwrap_or_else(|error| panic!("read {}: {error}", stream_dir.display()))
    {
        let entry = entry.expect("stream entry");
        if entry.path().is_file() {
            combined_logs
                .push_str(&fs::read_to_string(entry.path()).expect("read provider stream log"));
        }
    }
    assert!(combined_logs.contains("stdout before structured output"));
    assert!(combined_logs.contains("stderr before structured output"));
    assert_eq!(
        output.files_modified,
        Vec::<String>::new(),
        "runtime stream logs must not be reported as target file changes"
    );
}

#[test]
fn cli_adapter_emits_stdout_and_stderr_chunks_to_stream_sink() {
    let _guard = cli_adapter_test_guard();
    let tempdir = tempfile::tempdir().expect("tempdir");
    let command = support::write_executable_script(
        tempdir.path(),
        "stream-sink-provider",
        r#"#!/bin/sh
set -eu
case "${1:-run}" in
  probe|version|auth)
    echo "ok"
    exit 0
    ;;
  run)
    cat >/dev/null
    echo "stdout chunk for browser"
    echo "stderr chunk for browser" >&2
    echo "<ARIA_STRUCTURED_OUTPUT>"
    echo '{"artifact_kind":"clarification_record","goal_summary":"fixture goal","constraints":[],"open_questions":[],"assumptions":[],"suggested_scope":"fixture scope"}'
    echo "</ARIA_STRUCTURED_OUTPUT>"
    exit 0
    ;;
esac
"#,
    );
    let worktree = tempfile::tempdir().expect("worktree");
    let compatibility = fixture_compatibility_entry(ProviderType::Fake, command);
    let chunks = Arc::new(Mutex::new(Vec::new()));
    let sink_chunks = Arc::clone(&chunks);
    let adapter = CliProviderAdapter::new(CliAdapterConfig {
        compatibility,
        expected_artifact_kind: Some("clarification_record".to_string()),
        output_sink: Some(Arc::new(move |chunk| {
            sink_chunks
                .lock()
                .expect("chunks")
                .push((chunk.stream, chunk.text));
        })),
    });

    adapter
        .run(&adapter_input(worktree.path()))
        .expect("cli run");

    let chunks = chunks.lock().expect("chunks");
    assert!(
        chunks
            .iter()
            .any(|(stream, text)| stream == "stdout" && text.contains("stdout chunk for browser")),
        "stdout chunk should be streamed to sink: {chunks:?}"
    );
    assert!(
        chunks
            .iter()
            .any(|(stream, text)| stream == "stderr" && text.contains("stderr chunk for browser")),
        "stderr chunk should be streamed to sink: {chunks:?}"
    );
}

#[test]
fn missing_provider_command_is_diagnostic_not_a_panic() {
    let _guard = cli_adapter_test_guard();
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

fn cli_adapter_test_guard() -> MutexGuard<'static, ()> {
    CLI_ADAPTER_TEST_LOCK
        .lock()
        .expect("cli adapter test lock poisoned")
}
