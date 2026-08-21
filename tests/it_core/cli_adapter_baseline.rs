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
    assert!(
        matrix.entry_for(ProviderType::Pi).is_none(),
        "Pi is a streaming-only provider and must not enter Task Runner's CLI compatibility matrix"
    );
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
echo "<ARIA_STRUCTURED_OUTPUT nonce=\"fix00001\">"
echo '{"nonce":"fix00001","artifact_kind":"clarification_record","goal_summary":"fixture goal","constraints":[],"open_questions":[],"assumptions":[],"suggested_scope":"fixture scope"}'
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
    echo "<ARIA_STRUCTURED_OUTPUT nonce=\"fix00001\">"
    echo '{"nonce":"fix00001","artifact_kind":"clarification_record","goal_summary":"fixture goal","constraints":[],"open_questions":[],"assumptions":[],"suggested_scope":"fixture scope"}'
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
    echo "<ARIA_STRUCTURED_OUTPUT nonce=\"fix00001\">"
    echo '{"nonce":"fix00001","artifact_kind":"clarification_record","goal_summary":"fixture goal","constraints":[],"open_questions":[],"assumptions":[],"suggested_scope":"fixture scope"}'
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
    echo "<ARIA_STRUCTURED_OUTPUT nonce=\"fix00001\">"
    echo '{"nonce":"fix00001","artifact_kind":"clarification_record","goal_summary":"fixture goal","constraints":[],"open_questions":[],"assumptions":[],"suggested_scope":"fixture scope"}'
    echo "</ARIA_STRUCTURED_OUTPUT>"
    exit 0
    ;;
esac
"#,
    );
    let worktree = tempfile::tempdir().expect("worktree");
    let stream_log_dir = tempfile::tempdir().expect("stream log dir");
    let compatibility = fixture_compatibility_entry(ProviderType::Fake, command);
    let adapter = CliProviderAdapter::new(CliAdapterConfig {
        compatibility,
        expected_artifact_kind: Some("clarification_record".to_string()),
        output_sink: None,
    });

    let output = adapter
        .run(&adapter_input_with_stream_log_dir(
            worktree.path(),
            stream_log_dir.path(),
        ))
        .expect("cli run");

    let combined_logs = read_stream_logs(stream_log_dir.path());
    assert!(combined_logs.contains("stdout before structured output"));
    assert!(combined_logs.contains("stderr before structured output"));
    assert!(
        !worktree.path().join(".aria").exists(),
        "provider stream logs must not be written into the target repository"
    );
    assert_eq!(
        output.files_modified,
        Vec::<String>::new(),
        "runtime stream logs must not be reported as target file changes"
    );
}

/// 未提供流日志目录时不得写流日志，且不得回退到 provider 工作目录。
#[test]
fn cli_adapter_does_not_write_stream_logs_without_log_dir() {
    let _guard = cli_adapter_test_guard();
    let tempdir = tempfile::tempdir().expect("tempdir");
    let command = support::write_executable_script(
        tempdir.path(),
        "no-log-dir-provider",
        r#"#!/bin/sh
set -eu
case "${1:-run}" in
  probe|version|auth)
    echo "ok"
    exit 0
    ;;
  run)
    cat >/dev/null
    echo "stdout without log dir"
    echo "stderr without log dir" >&2
    echo "<ARIA_STRUCTURED_OUTPUT nonce=\"fix00001\">"
    echo '{"nonce":"fix00001","artifact_kind":"clarification_record","goal_summary":"fixture goal","constraints":[],"open_questions":[],"assumptions":[],"suggested_scope":"fixture scope"}'
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
        .expect("cli run must succeed without a stream log dir");

    assert!(
        !worktree.path().join(".aria").exists(),
        "missing stream log dir must not fall back to the provider working directory"
    );
    assert!(output.structured_output.is_some());
    assert_eq!(output.files_modified, Vec::<String>::new());
}

/// 命名规则不得改变，且先前执行的日志不得被覆盖。
///
/// 追加写入模式无法在此验证：跨进程执行的 child_id 不同，两次执行落到不同文件。
/// 追加语义由 `cli_adapter::provider_stream_tests` 直接锁住。
#[test]
fn cli_adapter_stream_log_naming_and_previous_runs_are_preserved() {
    let _guard = cli_adapter_test_guard();
    let tempdir = tempfile::tempdir().expect("tempdir");
    let command = support::write_executable_script(
        tempdir.path(),
        "naming-provider",
        r#"#!/bin/sh
set -eu
case "${1:-run}" in
  probe|version|auth)
    echo "ok"
    exit 0
    ;;
  run)
    cat >/dev/null
    echo "stdout marker line"
    echo "stderr marker line" >&2
    echo "<ARIA_STRUCTURED_OUTPUT nonce=\"fix00001\">"
    echo '{"nonce":"fix00001","artifact_kind":"clarification_record","goal_summary":"fixture goal","constraints":[],"open_questions":[],"assumptions":[],"suggested_scope":"fixture scope"}'
    echo "</ARIA_STRUCTURED_OUTPUT>"
    exit 0
    ;;
esac
"#,
    );
    let worktree = tempfile::tempdir().expect("worktree");
    let stream_log_dir = tempfile::tempdir().expect("stream log dir");
    let compatibility = fixture_compatibility_entry(ProviderType::Fake, command);
    let adapter = CliProviderAdapter::new(CliAdapterConfig {
        compatibility,
        expected_artifact_kind: Some("clarification_record".to_string()),
        output_sink: None,
    });
    let input = adapter_input_with_stream_log_dir(worktree.path(), stream_log_dir.path());

    adapter.run(&input).expect("first cli run");

    let names = fs::read_dir(stream_log_dir.path())
        .expect("read stream log dir")
        .map(|entry| {
            entry
                .expect("stream entry")
                .file_name()
                .to_string_lossy()
                .to_string()
        })
        .collect::<Vec<_>>();
    assert!(
        names.iter().any(|name| name.contains("naming-provider")),
        "stream log file name must retain the provider name: {names:?}"
    );
    assert!(
        names.iter().any(|name| name.ends_with("-stdout.log")),
        "stdout stream log must be named by stream: {names:?}"
    );
    assert!(
        names.iter().any(|name| name.ends_with("-stderr.log")),
        "stderr stream log must be named by stream: {names:?}"
    );

    // 第二次执行落在新的 child_id 上，因此两次执行的日志共存于同一目录，
    // 既有文件不被覆盖。
    adapter.run(&input).expect("second cli run");

    let combined_logs = read_stream_logs(stream_log_dir.path());
    assert_eq!(
        combined_logs.matches("stdout marker line").count(),
        2,
        "logs from both runs must coexist; earlier files must not be overwritten"
    );
    assert_eq!(
        combined_logs.matches("stderr marker line").count(),
        2,
        "logs from both runs must coexist; earlier files must not be overwritten"
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
    echo "<ARIA_STRUCTURED_OUTPUT nonce=\"fix00001\">"
    echo '{"nonce":"fix00001","artifact_kind":"clarification_record","goal_summary":"fixture goal","constraints":[],"open_questions":[],"assumptions":[],"suggested_scope":"fixture scope"}'
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

/// 无 coding attempt 上下文的执行路径不得提供流日志目录，从而按 adapter 的缺省
/// 行为不写流日志。`build_provider_context` 服务全部 runtime unit，取其作为代表：
/// 它是这些路径构造 `AdapterInput` 的唯一入口。
#[test]
fn runtime_unit_adapter_input_carries_no_provider_stream_log_dir() {
    use cadence_aria::cross_cutting::provider_context_builder::{
        ProviderContextBuilderInput, build_provider_context,
    };
    use cadence_aria::runtime_units::prompt_template_registry::all_planning_node_ids;

    let node_id = all_planning_node_ids()
        .first()
        .expect("at least one planning node")
        .to_string();
    let worktree = tempfile::tempdir().expect("worktree");
    let result = build_provider_context(ProviderContextBuilderInput {
        session_id: "session_001".to_string(),
        task_id: "task_001".to_string(),
        node_id,
        canonical_inputs: json!({
            "artifact_refs": ["art_ref_spec_0001"],
            "risk_registry_ref": "risk_registry_001"
        }),
        canonical_input_summary: "canonical summary for test".to_string(),
        projection_refs: vec!["proj_spec_projection_art_spec_001_0001".to_string()],
        projection_summary: "projection summary for test".to_string(),
        constraint_bundle_ref: "constraint_bundle_openspec_sample-change_0001".to_string(),
        constraint_summary: "constraint summary for test".to_string(),
        context_files: vec!["tests/fixtures/artifacts/spec.md".to_string()],
        worktree_path: Some(worktree.path().to_string_lossy().to_string()),
        routing_reference_context: Default::default(),
    })
    .expect("context package");

    assert!(
        result.adapter_input.worktree_path.is_some(),
        "the runtime unit still executes inside the target worktree"
    );
    assert!(
        result.adapter_input.provider_stream_log_dir.is_none(),
        "runtime unit inputs must not carry a stream log dir"
    );
}

fn adapter_input(worktree_path: &std::path::Path) -> AdapterInput {
    AdapterInput {
        provider_type: ProviderType::Fake,
        role: AdapterRole::Orchestrator,
        worktree_path: Some(worktree_path.to_string_lossy().to_string()),
        provider_stream_log_dir: None,
        prompt: "fixture prompt".to_string(),
        context_files: Vec::new(),
        output_schema: "clarification_record.v1".to_string(),
        timeout: 3,
        max_retries: 1,
    }
}

fn adapter_input_with_stream_log_dir(
    worktree_path: &std::path::Path,
    stream_log_dir: &std::path::Path,
) -> AdapterInput {
    AdapterInput {
        provider_stream_log_dir: Some(stream_log_dir.to_string_lossy().to_string()),
        ..adapter_input(worktree_path)
    }
}

fn read_stream_logs(stream_dir: &std::path::Path) -> String {
    let mut combined_logs = String::new();
    for entry in fs::read_dir(stream_dir)
        .unwrap_or_else(|error| panic!("read {}: {error}", stream_dir.display()))
    {
        let entry = entry.expect("stream entry");
        if entry.path().is_file() {
            combined_logs
                .push_str(&fs::read_to_string(entry.path()).expect("read provider stream log"));
        }
    }
    combined_logs
}

fn temp_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(name)
}

fn cli_adapter_test_guard() -> MutexGuard<'static, ()> {
    CLI_ADAPTER_TEST_LOCK
        .lock()
        .expect("cli adapter test lock poisoned")
}
