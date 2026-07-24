use super::*;
use crate::cross_cutting::provider_adapter::{ProviderAdapter, ProviderAdapterError};
use crate::product::models::{TrustedDraftVerificationCommand, WorkItemKind};
use crate::protocol::contracts::{
    AdapterInput, AdapterOutput, AdapterRole, ProviderType, TimeoutStatus,
};
use crate::protocol::provider_errors::ProviderErrorCode;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

fn outcomes_with_first_passes(
    scenario_count: usize,
    runs_per_scenario: usize,
    first_passes_by_scenario: impl Fn(usize) -> usize,
) -> Vec<DraftEvaluationOutcome> {
    (0..scenario_count)
        .flat_map(|scenario_index| {
            let first_passes = first_passes_by_scenario(scenario_index);
            (0..runs_per_scenario).map(move |run_index| DraftEvaluationOutcome {
                scenario_id: format!("scenario_{scenario_index:02}"),
                first_passed: run_index < first_passes,
                repair_attempted: run_index >= first_passes,
                repaired_passed: run_index >= first_passes,
                error_codes: if run_index < first_passes {
                    vec![]
                } else {
                    vec!["missing_required_verification_command".to_string()]
                },
            })
        })
        .collect()
}

fn report_from(outcomes: Vec<DraftEvaluationOutcome>) -> DraftEvaluationReport {
    build_report(DraftEvaluationReportInput {
        provider: ProviderType::Codex,
        prompt_version: "work_item_draft_v2".to_string(),
        scenario_set_hash: "scenario_hash_v1".to_string(),
        scenario_count: 30,
        runs_per_scenario: 10,
        non_release_smoke: false,
        outcomes,
    })
    .expect("report")
}

#[test]
fn draft_evaluation_report_passes_at_95_percent_over_30_by_10() {
    let outcomes = outcomes_with_first_passes(
        30,
        10,
        |scenario_index| {
            if scenario_index < 15 { 10 } else { 9 }
        },
    );

    let report = report_from(outcomes);

    assert_eq!(report.total_runs, 300);
    assert_eq!(report.first_pass_rate_percent, 95);
    assert!(report.release_gate_passed);
    assert_eq!(
        report.error_code_histogram,
        BTreeMap::from([("missing_required_verification_command".to_string(), 15)])
    );
}

#[test]
fn draft_evaluation_report_rejects_94_percent_overall() {
    let outcomes = outcomes_with_first_passes(
        30,
        10,
        |scenario_index| {
            if scenario_index < 12 { 10 } else { 9 }
        },
    );

    let report = report_from(outcomes);

    assert_eq!(report.first_pass_rate_percent, 94);
    assert!(!report.release_gate_passed);
}

#[test]
fn draft_evaluation_report_rejects_any_scenario_below_90_percent() {
    let outcomes = outcomes_with_first_passes(
        30,
        10,
        |scenario_index| {
            if scenario_index == 0 { 8 } else { 10 }
        },
    );

    let report = report_from(outcomes);

    assert_eq!(report.per_scenario_first_pass_rates["scenario_00"], 80);
    assert!(!report.release_gate_passed);
}

#[test]
fn repaired_pass_rate_only_counts_runs_that_really_attempted_semantic_repair() {
    let mut outcomes = outcomes_with_first_passes(30, 10, |_| 10);
    outcomes[0] = DraftEvaluationOutcome {
        scenario_id: "scenario_00".to_string(),
        first_passed: false,
        repair_attempted: true,
        repaired_passed: true,
        error_codes: vec!["missing_required_verification_command".to_string()],
    };
    outcomes[10] = DraftEvaluationOutcome {
        scenario_id: "scenario_01".to_string(),
        first_passed: false,
        repair_attempted: false,
        repaired_passed: false,
        error_codes: vec!["provider_unavailable".to_string()],
    };

    let report = report_from(outcomes);

    assert_eq!(report.repaired_pass_rate_percent, 100);
}

#[test]
fn compare_requires_both_independent_reports_to_pass() {
    let passing = report_from(outcomes_with_first_passes(30, 10, |scenario_index| {
        if scenario_index < 15 { 10 } else { 9 }
    }));
    let failing = report_from(outcomes_with_first_passes(30, 10, |scenario_index| {
        if scenario_index < 12 { 10 } else { 9 }
    }));

    let comparison = compare_reports(&passing, &failing).expect("comparable reports");

    assert!(!comparison.release_gate_passed);
}

#[test]
fn compare_rejects_different_provider_prompt_or_scenario_hash() {
    let baseline = report_from(outcomes_with_first_passes(30, 10, |_| 10));

    let mut different_provider = baseline.clone();
    different_provider.provider = ProviderType::ClaudeCode;
    assert_eq!(
        compare_reports(&baseline, &different_provider)
            .expect_err("provider mismatch")
            .code,
        "draft_eval_provider_mismatch"
    );

    let mut different_prompt = baseline.clone();
    different_prompt.prompt_version = "work_item_draft_v3".to_string();
    assert_eq!(
        compare_reports(&baseline, &different_prompt)
            .expect_err("prompt mismatch")
            .code,
        "draft_eval_prompt_version_mismatch"
    );

    let mut different_hash = baseline.clone();
    different_hash.scenario_set_hash = "scenario_hash_v2".to_string();
    assert_eq!(
        compare_reports(&baseline, &different_hash)
            .expect_err("scenario mismatch")
            .code,
        "draft_eval_scenario_set_mismatch"
    );
}

#[test]
fn draft_evaluation_report_serialization_is_audited_and_rejects_raw_fields() {
    let report = report_from(outcomes_with_first_passes(30, 10, |_| 10));
    let value = serde_json::to_value(&report).expect("serialize audited report");
    let keys = value
        .as_object()
        .expect("report object")
        .keys()
        .map(String::as_str)
        .collect::<std::collections::BTreeSet<_>>();

    assert_eq!(
        keys,
        std::collections::BTreeSet::from([
            "error_code_histogram",
            "first_pass_rate_percent",
            "non_release_smoke",
            "per_scenario_first_pass_rates",
            "prompt_version",
            "provider",
            "release_gate_passed",
            "repaired_pass_rate_percent",
            "runs_per_scenario",
            "scenario_count",
            "scenario_set_hash",
            "total_runs",
        ])
    );
    let mut unsafe_value = value;
    unsafe_value
        .as_object_mut()
        .expect("report object")
        .insert("raw_prompt".to_string(), serde_json::json!("secret"));
    assert!(serde_json::from_value::<DraftEvaluationReport>(unsafe_value).is_err());
}

fn scenario(index: usize) -> DraftEvaluationScenario {
    DraftEvaluationScenario {
        scenario_id: format!("scenario_{index:02}"),
        relative_worktree_path: "sandbox/repository".to_string(),
        outline: DraftEvaluationScenarioOutline {
            outline_id: "outline_component".to_string(),
            logical_work_item_id: "wi_component".to_string(),
            title: "Implement abstract component".to_string(),
            kind: WorkItemKind::Backend,
            goal: "Implement placeholder design semantics".to_string(),
            scope: vec!["modules/component".to_string()],
            non_goals: vec!["Do not change unrelated modules".to_string()],
            exclusive_write_scopes: vec!["modules/component/**".to_string()],
            forbidden_write_scopes: vec!["modules/other/**".to_string()],
            depends_on: vec![],
            verification_intent: vec!["Run the trusted component check".to_string()],
        },
        accepted_dependency_summaries: vec![],
        trusted_verification_command_catalog: vec![TrustedDraftVerificationCommand {
            command: "cargo test --locked --lib component".to_string(),
            cwd: ".".to_string(),
            purpose: "verify component".to_string(),
            source_ref: "design_placeholder#verification".to_string(),
        }],
        expected_coverage_categories: vec!["valid_control".to_string()],
        scenario_traits: vec!["no_dependency".to_string()],
        user_feedback: None,
    }
}

fn provider_candidate(invalid: bool) -> serde_json::Value {
    let mut contract =
        crate::product::work_item_contract::canonical_contract_fixture("wi_component");
    contract.identity.title = "Implement abstract component".to_string();
    contract.identity.kind = "backend".to_string();
    contract.input_contracts.clear();
    contract.write_policy.exclusive_scopes = vec!["modules/component/**".to_string()];
    contract.write_policy.forbidden_scopes = vec!["modules/other/**".to_string()];
    contract.verification_checks[0].command = if invalid {
        None
    } else {
        Some("cargo test --locked --lib component".to_string())
    };
    let verification_plan = serde_json::json!({
        "checks": contract.verification_checks.clone(),
    });
    serde_json::json!({
        "draft": {
            "outline_id": "outline_component",
            "logical_work_item_id": "wi_component",
            "canonical_contract": contract,
            "verification_plan": verification_plan,
        }
    })
}

fn output_for(input: &AdapterInput, value: &serde_json::Value) -> AdapterOutput {
    let marker = "<ARIA_STRUCTURED_OUTPUT nonce=\"";
    let start = input.prompt.rfind(marker).expect("formal prompt nonce") + marker.len();
    let nonce = &input.prompt[start..start + 8];
    AdapterOutput {
        exit_code: Some(0),
        stdout: format!(
            "<ARIA_STRUCTURED_OUTPUT nonce=\"{nonce}\">{value}</ARIA_STRUCTURED_OUTPUT nonce=\"{nonce}\">"
        ),
        stderr: String::new(),
        structured_output: None,
        files_modified: vec![],
        duration_ms: 1,
        timeout_status: TimeoutStatus::NotTimedOut,
    }
}

struct ThresholdFakeAdapter {
    first_attempts: AtomicUsize,
    repair_attempts: AtomicUsize,
}

impl ProviderAdapter for ThresholdFakeAdapter {
    fn run(&self, input: &AdapterInput) -> Result<AdapterOutput, ProviderAdapterError> {
        assert_eq!(input.provider_type, ProviderType::Codex);
        assert_eq!(input.role, AdapterRole::WorkItemSplitter);
        assert_eq!(
            input.output_schema,
            crate::product::work_item_split_engine::WORK_ITEM_DRAFT_OUTPUT_SCHEMA
        );
        assert_eq!(input.max_retries, 0);
        assert!(
            input
                .worktree_path
                .as_deref()
                .is_some_and(|path| path.ends_with("sandbox/repository"))
        );

        let repairing = input.prompt.contains("[draft_validation_findings]");
        let invalid = if repairing {
            self.repair_attempts.fetch_add(1, Ordering::SeqCst);
            false
        } else {
            let first_attempt = self.first_attempts.fetch_add(1, Ordering::SeqCst);
            let scenario_index = first_attempt / 10;
            scenario_index >= 15 && first_attempt % 10 == 9
        };
        Ok(output_for(input, &provider_candidate(invalid)))
    }
}

#[test]
fn draft_evaluation_runner_uses_fake_adapter_for_30_by_10_threshold_run() {
    let adapter = ThresholdFakeAdapter {
        first_attempts: AtomicUsize::new(0),
        repair_attempts: AtomicUsize::new(0),
    };
    let scenarios = (0..30).map(scenario).collect::<Vec<_>>();

    let report = run_evaluation_with_adapter(
        &adapter,
        ProviderType::Codex,
        Path::new("/tmp/draft-eval-workspace"),
        &scenarios,
        10,
        false,
    )
    .expect("fake evaluation");

    assert_eq!(report.total_runs, 300);
    assert_eq!(report.first_pass_rate_percent, 95);
    assert_eq!(report.repaired_pass_rate_percent, 100);
    assert!(report.release_gate_passed);
    assert_eq!(adapter.first_attempts.load(Ordering::SeqCst), 300);
    assert_eq!(adapter.repair_attempts.load(Ordering::SeqCst), 15);
}

struct ProviderErrorFakeAdapter {
    calls: AtomicUsize,
}

impl ProviderAdapter for ProviderErrorFakeAdapter {
    fn run(&self, _input: &AdapterInput) -> Result<AdapterOutput, ProviderAdapterError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(ProviderAdapterError::provider_unavailable(
            "fake unavailable",
        ))
    }
}

#[test]
fn draft_evaluation_runner_counts_provider_errors_without_retrying_or_faking_success() {
    let adapter = ProviderErrorFakeAdapter {
        calls: AtomicUsize::new(0),
    };

    let report = run_evaluation_with_adapter(
        &adapter,
        ProviderType::Codex,
        Path::new("/tmp/draft-eval-workspace"),
        &[scenario(0)],
        1,
        true,
    )
    .expect("provider failures belong in the report");

    assert_eq!(adapter.calls.load(Ordering::SeqCst), 1);
    assert_eq!(report.first_pass_rate_percent, 0);
    assert_eq!(report.repaired_pass_rate_percent, 0);
    assert_eq!(
        report.error_code_histogram[ProviderErrorCode::ProviderUnavailable.as_str()],
        1
    );
    assert!(!report.release_gate_passed);
    assert!(report.non_release_smoke);
}

struct ParseErrorFakeAdapter {
    calls: AtomicUsize,
}

impl ProviderAdapter for ParseErrorFakeAdapter {
    fn run(&self, _input: &AdapterInput) -> Result<AdapterOutput, ProviderAdapterError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(AdapterOutput {
            exit_code: Some(0),
            stdout: "<ARIA_STRUCTURED_OUTPUT nonce=\"deadbeef\">{invalid}</ARIA_STRUCTURED_OUTPUT nonce=\"deadbeef\">".to_string(),
            stderr: String::new(),
            structured_output: None,
            files_modified: vec![],
            duration_ms: 1,
            timeout_status: TimeoutStatus::NotTimedOut,
        })
    }
}

struct AlwaysSemanticFailureFakeAdapter {
    calls: AtomicUsize,
}

impl ProviderAdapter for AlwaysSemanticFailureFakeAdapter {
    fn run(&self, input: &AdapterInput) -> Result<AdapterOutput, ProviderAdapterError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(output_for(input, &provider_candidate(true)))
    }
}

#[test]
fn draft_evaluation_runner_stops_after_one_failed_semantic_repair() {
    let adapter = AlwaysSemanticFailureFakeAdapter {
        calls: AtomicUsize::new(0),
    };

    let report = run_evaluation_with_adapter(
        &adapter,
        ProviderType::Codex,
        Path::new("/tmp/draft-eval-workspace"),
        &[scenario(0)],
        1,
        true,
    )
    .expect("semantic failure belongs in the report");

    assert_eq!(adapter.calls.load(Ordering::SeqCst), 2);
    assert_eq!(report.repaired_pass_rate_percent, 0);
    assert!(!report.release_gate_passed);
}

#[test]
fn draft_evaluation_runner_counts_parse_errors_without_semantic_retry() {
    let adapter = ParseErrorFakeAdapter {
        calls: AtomicUsize::new(0),
    };

    let report = run_evaluation_with_adapter(
        &adapter,
        ProviderType::Codex,
        Path::new("/tmp/draft-eval-workspace"),
        &[scenario(0)],
        1,
        true,
    )
    .expect("parse failures belong in the report");

    assert_eq!(adapter.calls.load(Ordering::SeqCst), 1);
    assert_eq!(report.error_code_histogram["invalid_json"], 1);
    assert_eq!(report.repaired_pass_rate_percent, 0);
    assert!(!report.release_gate_passed);
}

#[test]
fn draft_evaluation_fixture_is_exactly_30_sanitized_auditable_scenarios() {
    let raw = include_str!("../../../tests/fixtures/work_item_draft_eval/scenarios.v1.json");
    let scenarios = load_scenarios_from_str(raw).expect("scenario fixture");

    assert_eq!(scenarios.len(), 30);
    let mut coverage = BTreeMap::<String, usize>::new();
    let mut traits = BTreeMap::<String, usize>::new();
    for scenario in &scenarios {
        assert!(!scenario.scenario_id.is_empty());
        assert!(!scenario.outline.goal.is_empty());
        assert!(!scenario.relative_worktree_path.starts_with('/'));
        assert!(!scenario.expected_coverage_categories.is_empty());
        for category in &scenario.expected_coverage_categories {
            *coverage.entry(category.clone()).or_default() += 1;
        }
        for scenario_trait in &scenario.scenario_traits {
            *traits.entry(scenario_trait.clone()).or_default() += 1;
        }
    }
    for category in [
        "valid_control",
        "missing_required_verification_command",
        "unknown_done_when_ref",
        "unknown_requirement_ref",
        "unknown_reviewer_check_ref",
        "acceptance_criterion_without_reviewer_check",
        "stage_blocker_without_target_contract",
        "verification_plan_not_derived_from_contract",
        "untrusted_required_verification_command",
        "missing_trusted_verification_command_catalog",
    ] {
        assert!(
            coverage.get(category).copied().unwrap_or_default() >= 2,
            "missing coverage for {category}"
        );
    }
    for required_trait in [
        "with_dependency",
        "no_dependency",
        "empty_catalog_blocker",
        "multiple_acceptance_criteria",
        "with_user_feedback",
    ] {
        assert!(
            traits.get(required_trait).copied().unwrap_or_default() > 0,
            "missing trait {required_trait}"
        );
    }
    for forbidden in [
        "/home/",
        "/Users/",
        "C:\\\\",
        "<ARIA_STRUCTURED_OUTPUT",
        "customer",
        "issue title",
    ] {
        assert!(
            !raw.contains(forbidden),
            "fixture leaks forbidden token {forbidden}"
        );
    }
}
