use cadence_aria::cli::{CliOutput, run_cli, run_cli_async};
use cadence_aria::product::work_item_draft_evaluation::{
    DraftEvaluationOutcome, DraftEvaluationReport, DraftEvaluationReportInput, build_report,
    compare_reports,
};
use cadence_aria::protocol::contracts::ProviderType;
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
fn draft_eval_compare_help_requires_help_to_be_the_only_argument() {
    let output = run_cli(["work-item-draft-eval", "compare", "--help"]).expect("compare help");
    let CliOutput::Text(help) = output;
    assert!(help.contains("work-item-draft-eval compare"));

    let error = run_cli(["work-item-draft-eval", "compare", "--unknown", "--help"])
        .expect_err("help must not hide an unknown compare argument");
    assert_eq!(error.code, "draft_eval_unknown_arg");
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
    build_report(DraftEvaluationReportInput {
        provider,
        prompt_version: prompt_version.to_string(),
        scenario_set_hash: "sha256:scenario".to_string(),
        scenario_count: 30,
        runs_per_scenario: 10,
        non_release_smoke: false,
        outcomes: (0..30)
            .flat_map(|scenario_index| {
                (0..10).map(move |_| DraftEvaluationOutcome {
                    scenario_id: format!("scenario_{scenario_index:02}"),
                    first_passed: true,
                    repair_attempted: false,
                    repaired_passed: false,
                    error_codes: vec![],
                })
            })
            .collect(),
    })
    .expect("comparison report")
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

#[test]
fn draft_eval_compare_rejects_the_same_persisted_run_twice() {
    let directory = tempdir().expect("reports");
    let report_path = directory.path().join("report.json");
    let report = comparison_report(ProviderType::Codex, "work_item_draft_v2");
    std::fs::write(&report_path, serde_json::to_vec(&report).expect("report"))
        .expect("write report");

    let error = run_cli([
        "work-item-draft-eval",
        "compare",
        "--reports",
        &format!("{},{}", report_path.display(), report_path.display()),
    ])
    .expect_err("same run must not count twice");

    assert_eq!(error.code, "draft_eval_same_run");
}

#[test]
fn draft_eval_compare_rejects_tampered_persisted_gate() {
    let directory = tempdir().expect("reports");
    let first_path = directory.path().join("first.json");
    let second_path = directory.path().join("second.json");
    let first = comparison_report(ProviderType::Codex, "work_item_draft_v2");
    let mut second = comparison_report(ProviderType::Codex, "work_item_draft_v2");
    second.release_gate_passed = false;
    std::fs::write(&first_path, serde_json::to_vec(&first).expect("first")).expect("write first");
    std::fs::write(&second_path, serde_json::to_vec(&second).expect("second"))
        .expect("write second");

    let error = run_cli([
        "work-item-draft-eval",
        "compare",
        "--reports",
        &format!("{},{}", first_path.display(), second_path.display()),
    ])
    .expect_err("tampered gate must be rejected");

    assert_eq!(error.code, "draft_eval_report_gate_mismatch");
}

fn draft_eval_run_args(report: &str) -> Vec<&str> {
    vec![
        "work-item-draft-eval",
        "run",
        "--workspace",
        ".",
        "--provider",
        "codex",
        "--scenario-file",
        "tests/fixtures/work_item_draft_eval/scenarios.v1.json",
        "--real-provider",
        "--report",
        report,
    ]
}

#[test]
fn draft_eval_preflight_rejects_existing_report_without_overwrite_or_provider_setup() {
    let directory = tempdir().expect("report directory");
    let report = directory.path().join("report.json");
    std::fs::write(&report, "sentinel").expect("existing report");

    let error = run_cli(draft_eval_run_args(report.to_str().expect("path")))
        .expect_err("existing report target must fail closed");

    assert_eq!(error.code, "draft_eval_report_target_exists");
    assert_eq!(
        std::fs::read_to_string(report).expect("sentinel"),
        "sentinel"
    );
}

#[cfg(unix)]
#[test]
fn draft_eval_preflight_rejects_symlink_report_without_touching_target() {
    use std::os::unix::fs::symlink;

    let directory = tempdir().expect("report directory");
    let protected = directory.path().join("protected.txt");
    let report = directory.path().join("report.json");
    std::fs::write(&protected, "protected").expect("protected target");
    symlink(&protected, &report).expect("report symlink");

    let error = run_cli(draft_eval_run_args(report.to_str().expect("path")))
        .expect_err("symlink report target must fail closed");

    assert_eq!(error.code, "draft_eval_report_target_exists");
    assert_eq!(
        std::fs::read_to_string(protected).expect("protected"),
        "protected"
    );
}

#[test]
fn draft_eval_cli_rejects_flag_where_report_value_is_required() {
    let error = run_cli([
        "work-item-draft-eval",
        "run",
        "--workspace",
        ".",
        "--provider",
        "codex",
        "--scenario-file",
        "tests/fixtures/work_item_draft_eval/scenarios.v1.json",
        "--real-provider",
        "--report",
        "--smoke",
    ])
    .expect_err("a flag is not a report path");

    assert_eq!(error.code, "draft_eval_report_required");
}

#[test]
fn draft_eval_cli_rejects_unknown_and_duplicate_arguments() {
    let unknown = run_cli(
        [
            draft_eval_run_args("report.json").as_slice(),
            &["--unknown"],
        ]
        .concat(),
    )
    .expect_err("unknown argument");
    assert_eq!(unknown.code, "draft_eval_unknown_arg");

    let duplicate = run_cli(
        [
            draft_eval_run_args("report.json").as_slice(),
            &["--report", "second.json"],
        ]
        .concat(),
    )
    .expect_err("duplicate report argument");
    assert_eq!(duplicate.code, "draft_eval_duplicate_arg");

    let unknown_with_help = run_cli(
        [
            draft_eval_run_args("report.json").as_slice(),
            &["--unknown", "--help"],
        ]
        .concat(),
    )
    .expect_err("help must not hide an unknown argument");
    assert_eq!(unknown_with_help.code, "draft_eval_unknown_arg");

    let missing_value_with_help = run_cli(["work-item-draft-eval", "run", "--report", "--help"])
        .expect_err("help must not hide a missing value");
    assert_eq!(missing_value_with_help.code, "draft_eval_report_required");

    let duplicate_with_help = run_cli(
        [
            draft_eval_run_args("report.json").as_slice(),
            &["--report", "second.json", "--help"],
        ]
        .concat(),
    )
    .expect_err("help must not hide a duplicate argument");
    assert_eq!(duplicate_with_help.code, "draft_eval_duplicate_arg");
}

#[test]
fn draft_eval_cli_preflights_release_corpus_before_async_provider_setup() {
    let directory = tempdir().expect("preflight");
    let scenario_file = directory.path().join("invalid-scenarios.json");
    let report = directory.path().join("report.json");
    let mut fixture: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string("tests/fixtures/work_item_draft_eval/scenarios.v1.json")
            .expect("fixture"),
    )
    .expect("fixture json");
    for scenario in fixture["scenarios"].as_array_mut().expect("scenarios") {
        scenario["expected_coverage_categories"] = serde_json::json!(["valid_control"]);
    }
    std::fs::write(
        &scenario_file,
        serde_json::to_vec(&fixture).expect("invalid fixture"),
    )
    .expect("write invalid fixture");

    let error = run_cli([
        "work-item-draft-eval",
        "run",
        "--workspace",
        ".",
        "--provider",
        "codex",
        "--scenario-file",
        scenario_file.to_str().expect("scenario path"),
        "--real-provider",
        "--report",
        report.to_str().expect("report path"),
    ])
    .expect_err("invalid release corpus must fail before async provider setup");

    assert_eq!(error.code, "draft_eval_required_category_coverage");
}

#[test]
fn draft_eval_smoke_validates_the_full_input_corpus_before_truncating() {
    let directory = tempdir().expect("preflight");
    let scenario_file = directory.path().join("invalid-smoke-scenarios.json");
    let report = directory.path().join("report.json");
    let mut fixture: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string("tests/fixtures/work_item_draft_eval/scenarios.v1.json")
            .expect("fixture"),
    )
    .expect("fixture json");
    fixture["scenarios"][2]["scenario_id"] = serde_json::json!("unsafe/path");
    std::fs::write(
        &scenario_file,
        serde_json::to_vec(&fixture).expect("invalid fixture"),
    )
    .expect("write invalid fixture");

    let error = run_cli([
        "work-item-draft-eval",
        "run",
        "--workspace",
        ".",
        "--provider",
        "codex",
        "--scenario-file",
        scenario_file.to_str().expect("scenario path"),
        "--real-provider",
        "--report",
        report.to_str().expect("report path"),
        "--smoke",
    ])
    .expect_err("smoke must validate scenarios that are not selected for execution");

    assert_eq!(error.code, "draft_eval_scenario_id_invalid");
}
