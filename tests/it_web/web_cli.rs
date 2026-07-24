use cadence_aria::cli::{CliOutput, run_cli, run_cli_async};
use cadence_aria::product::work_item_draft_evaluation::{DraftEvaluationReport, compare_reports};
use cadence_aria::protocol::contracts::ProviderType;
use std::collections::BTreeMap;
use tempfile::tempdir;

#[test]
fn web_check_reports_workspace_and_bind_address() {
    let workspace = tempdir().expect("workspace");
    let output = run_cli([
        "web",
        "--workspace",
        workspace.path().to_str().expect("path"),
        "--host",
        "127.0.0.1",
        "--port",
        "4317",
        "--check",
    ])
    .expect("cli");

    assert_eq!(
        output,
        CliOutput::Text(format!(
            "web_check_ok:{}:127.0.0.1:4317",
            workspace.path().display()
        ))
    );
}

#[tokio::test]
async fn async_web_check_uses_same_parser() {
    let workspace = tempdir().expect("workspace");
    let output = run_cli_async([
        "web",
        "--workspace",
        workspace.path().to_str().expect("path"),
        "--check",
    ])
    .await
    .expect("cli");

    assert!(matches!(output, CliOutput::Text(text) if text.starts_with("web_check_ok:")));
}

#[tokio::test]
async fn draft_eval_run_fails_closed_without_real_provider_authorization() {
    let workspace = tempdir().expect("workspace");
    let error = run_cli_async([
        "work-item-draft-eval",
        "run",
        "--workspace",
        workspace.path().to_str().expect("path"),
        "--provider",
        "codex",
        "--scenario-file",
        "tests/fixtures/work_item_draft_eval/scenarios.v1.json",
        "--report",
        "report.json",
    ])
    .await
    .expect_err("real provider must require explicit authorization");

    assert_eq!(error.code, "draft_eval_real_provider_required");
}

#[tokio::test]
async fn draft_eval_run_requires_caller_selected_report_path() {
    let workspace = tempdir().expect("workspace");
    let error = run_cli_async([
        "work-item-draft-eval",
        "run",
        "--workspace",
        workspace.path().to_str().expect("path"),
        "--provider",
        "codex",
        "--scenario-file",
        "tests/fixtures/work_item_draft_eval/scenarios.v1.json",
        "--real-provider",
    ])
    .await
    .expect_err("report path must be explicit");

    assert_eq!(error.code, "draft_eval_report_required");
}

#[test]
fn draft_eval_run_help_is_explicit_and_does_not_require_async_provider_setup() {
    let output = run_cli(["work-item-draft-eval", "run", "--help"]).expect("help");
    let CliOutput::Text(help) = output;

    for flag in [
        "--real-provider",
        "--runs-per-scenario",
        "--scenario-file",
        "--report",
    ] {
        assert!(help.contains(flag), "missing {flag} in {help}");
    }
}

#[test]
fn draft_eval_smoke_rejects_more_than_two_runs_before_provider_setup() {
    let error = run_cli([
        "work-item-draft-eval",
        "run",
        "--workspace",
        ".",
        "--provider",
        "codex",
        "--scenario-file",
        "tests/fixtures/work_item_draft_eval/scenarios.v1.json",
        "--runs-per-scenario",
        "3",
        "--smoke",
        "--real-provider",
        "--report",
        "report.json",
    ])
    .expect_err("smoke run limit");

    assert_eq!(error.code, "draft_eval_smoke_limit_exceeded");
}

fn comparison_report(provider: ProviderType, prompt_version: &str) -> DraftEvaluationReport {
    DraftEvaluationReport {
        provider,
        prompt_version: prompt_version.to_string(),
        scenario_set_hash: "sha256:scenario".to_string(),
        scenario_count: 30,
        runs_per_scenario: 10,
        total_runs: 300,
        first_pass_rate_percent: 95,
        repaired_pass_rate_percent: 100,
        per_scenario_first_pass_rates: BTreeMap::from([("scenario_00".to_string(), 100)]),
        error_code_histogram: BTreeMap::new(),
        release_gate_passed: true,
        non_release_smoke: false,
    }
}

#[test]
fn draft_eval_compare_reads_only_audited_reports_and_rejects_prompt_mismatch() {
    let directory = tempdir().expect("reports");
    let first_path = directory.path().join("first.json");
    let second_path = directory.path().join("second.json");
    let first = comparison_report(ProviderType::Codex, "work_item_draft_v2");
    let second = comparison_report(ProviderType::Codex, "work_item_draft_v3");
    std::fs::write(
        &first_path,
        serde_json::to_vec(&first).expect("first report"),
    )
    .expect("write first");
    std::fs::write(
        &second_path,
        serde_json::to_vec(&second).expect("second report"),
    )
    .expect("write second");

    let error = run_cli([
        "work-item-draft-eval",
        "compare",
        "--reports",
        &format!("{},{}", first_path.display(), second_path.display()),
    ])
    .expect_err("prompt mismatch");

    assert_eq!(error.code, "draft_eval_prompt_version_mismatch");
    assert!(compare_reports(&first, &second).is_err());
}
