use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::product::models::{TrustedDraftVerificationCommand, WorkItemKind};
use crate::protocol::contracts::ProviderType;

pub const MIN_RELEASE_SCENARIOS: usize = 30;
pub const DEFAULT_RUNS_PER_SCENARIO: usize = 10;
pub const MIN_OVERALL_FIRST_PASS_PERCENT: usize = 95;
pub const MIN_SCENARIO_FIRST_PASS_PERCENT: usize = 90;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct DraftEvaluationScenarioFile {
    pub schema_version: u32,
    pub scenarios: Vec<DraftEvaluationScenario>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct DraftEvaluationScenario {
    pub scenario_id: String,
    pub relative_worktree_path: String,
    pub outline: DraftEvaluationScenarioOutline,
    #[serde(default)]
    pub accepted_dependency_summaries: Vec<DraftEvaluationDependencySummary>,
    #[serde(default)]
    pub trusted_verification_command_catalog: Vec<TrustedDraftVerificationCommand>,
    pub expected_coverage_categories: Vec<String>,
    #[serde(default)]
    pub scenario_traits: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_feedback: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct DraftEvaluationScenarioOutline {
    pub outline_id: String,
    pub logical_work_item_id: String,
    pub title: String,
    pub kind: WorkItemKind,
    pub goal: String,
    pub scope: Vec<String>,
    pub non_goals: Vec<String>,
    pub exclusive_write_scopes: Vec<String>,
    pub forbidden_write_scopes: Vec<String>,
    pub depends_on: Vec<String>,
    pub verification_intent: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct DraftEvaluationDependencySummary {
    pub outline_id: String,
    pub logical_work_item_id: String,
    pub title: String,
    pub promised_contract_refs: Vec<String>,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftEvaluationError {
    pub code: String,
    pub message: String,
}

impl DraftEvaluationError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

impl std::fmt::Display for DraftEvaluationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for DraftEvaluationError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftEvaluationOutcome {
    pub scenario_id: String,
    pub first_passed: bool,
    pub repair_attempted: bool,
    pub repaired_passed: bool,
    pub error_codes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftEvaluationReportInput {
    pub provider: ProviderType,
    pub prompt_version: String,
    pub scenario_set_hash: String,
    pub scenario_count: usize,
    pub runs_per_scenario: usize,
    pub non_release_smoke: bool,
    pub outcomes: Vec<DraftEvaluationOutcome>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct DraftEvaluationReport {
    pub provider: ProviderType,
    pub prompt_version: String,
    pub scenario_set_hash: String,
    pub scenario_count: usize,
    pub runs_per_scenario: usize,
    pub total_runs: usize,
    pub first_pass_rate_percent: usize,
    pub repaired_pass_rate_percent: usize,
    pub per_scenario_first_pass_rates: BTreeMap<String, usize>,
    pub error_code_histogram: BTreeMap<String, usize>,
    pub release_gate_passed: bool,
    pub non_release_smoke: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct DraftEvaluationComparison {
    pub provider: ProviderType,
    pub prompt_version: String,
    pub scenario_set_hash: String,
    pub report_count: usize,
    pub release_gate_passed: bool,
}

pub fn build_report(
    input: DraftEvaluationReportInput,
) -> Result<DraftEvaluationReport, DraftEvaluationError> {
    let expected_total = input
        .scenario_count
        .checked_mul(input.runs_per_scenario)
        .ok_or_else(|| {
            DraftEvaluationError::new(
                "draft_eval_run_count_overflow",
                "scenario_count * runs_per_scenario overflowed",
            )
        })?;
    if input.outcomes.len() != expected_total {
        return Err(DraftEvaluationError::new(
            "draft_eval_run_count_mismatch",
            format!(
                "expected {expected_total} outcomes but received {}",
                input.outcomes.len()
            ),
        ));
    }

    let mut scenario_totals = BTreeMap::<String, usize>::new();
    let mut scenario_first_passes = BTreeMap::<String, usize>::new();
    let mut error_code_histogram = BTreeMap::<String, usize>::new();
    let mut first_passes = 0usize;
    let mut repair_attempts = 0usize;
    let mut repaired_passes = 0usize;

    for outcome in &input.outcomes {
        if outcome.first_passed && outcome.repair_attempted
            || outcome.repaired_passed && !outcome.repair_attempted
        {
            return Err(DraftEvaluationError::new(
                "draft_eval_outcome_invalid",
                "repair state is inconsistent with first-pass state",
            ));
        }
        *scenario_totals
            .entry(outcome.scenario_id.clone())
            .or_default() += 1;
        if outcome.first_passed {
            first_passes += 1;
            *scenario_first_passes
                .entry(outcome.scenario_id.clone())
                .or_default() += 1;
        } else if outcome.repair_attempted {
            repair_attempts += 1;
            if outcome.repaired_passed {
                repaired_passes += 1;
            }
        }
        for code in &outcome.error_codes {
            *error_code_histogram.entry(code.clone()).or_default() += 1;
        }
    }

    if scenario_totals.len() != input.scenario_count
        || scenario_totals
            .values()
            .any(|total| *total != input.runs_per_scenario)
    {
        return Err(DraftEvaluationError::new(
            "draft_eval_scenario_run_shape_mismatch",
            "every scenario must contribute exactly runs_per_scenario outcomes",
        ));
    }

    let per_scenario_first_pass_rates = scenario_totals
        .iter()
        .map(|(scenario_id, total)| {
            let passes = scenario_first_passes
                .get(scenario_id)
                .copied()
                .unwrap_or_default();
            (scenario_id.clone(), percentage(passes, *total))
        })
        .collect::<BTreeMap<_, _>>();
    let first_pass_rate_percent = percentage(first_passes, expected_total);
    let repaired_pass_rate_percent = if repair_attempts == 0 {
        0
    } else {
        percentage(repaired_passes, repair_attempts)
    };
    let release_gate_passed = !input.non_release_smoke
        && input.scenario_count >= MIN_RELEASE_SCENARIOS
        && input.runs_per_scenario >= DEFAULT_RUNS_PER_SCENARIO
        && first_pass_rate_percent >= MIN_OVERALL_FIRST_PASS_PERCENT
        && per_scenario_first_pass_rates
            .values()
            .all(|rate| *rate >= MIN_SCENARIO_FIRST_PASS_PERCENT);

    Ok(DraftEvaluationReport {
        provider: input.provider,
        prompt_version: input.prompt_version,
        scenario_set_hash: input.scenario_set_hash,
        scenario_count: input.scenario_count,
        runs_per_scenario: input.runs_per_scenario,
        total_runs: expected_total,
        first_pass_rate_percent,
        repaired_pass_rate_percent,
        per_scenario_first_pass_rates,
        error_code_histogram,
        release_gate_passed,
        non_release_smoke: input.non_release_smoke,
    })
}

pub fn compare_reports(
    first: &DraftEvaluationReport,
    second: &DraftEvaluationReport,
) -> Result<DraftEvaluationComparison, DraftEvaluationError> {
    if first.provider != second.provider {
        return Err(DraftEvaluationError::new(
            "draft_eval_provider_mismatch",
            "reports use different providers",
        ));
    }
    if first.prompt_version != second.prompt_version {
        return Err(DraftEvaluationError::new(
            "draft_eval_prompt_version_mismatch",
            "reports use different prompt versions",
        ));
    }
    if first.scenario_set_hash != second.scenario_set_hash {
        return Err(DraftEvaluationError::new(
            "draft_eval_scenario_set_mismatch",
            "reports use different scenario sets",
        ));
    }
    if first.scenario_count != second.scenario_count
        || first.runs_per_scenario != second.runs_per_scenario
    {
        return Err(DraftEvaluationError::new(
            "draft_eval_run_shape_mismatch",
            "reports use different scenario or run counts",
        ));
    }

    Ok(DraftEvaluationComparison {
        provider: first.provider.clone(),
        prompt_version: first.prompt_version.clone(),
        scenario_set_hash: first.scenario_set_hash.clone(),
        report_count: 2,
        release_gate_passed: first.release_gate_passed && second.release_gate_passed,
    })
}

fn percentage(numerator: usize, denominator: usize) -> usize {
    numerator
        .saturating_mul(100)
        .checked_div(denominator)
        .unwrap_or(0)
}
