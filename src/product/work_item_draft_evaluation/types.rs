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
    pub run_id: String,
    pub provider: ProviderType,
    pub prompt_version: String,
    pub scenario_set_hash: String,
    pub scenario_count: usize,
    pub runs_per_scenario: usize,
    pub total_runs: usize,
    pub total_first_pass_count: usize,
    pub repair_attempt_count: usize,
    pub repaired_pass_count: usize,
    pub first_pass_rate_percent: usize,
    pub repaired_pass_rate_percent: usize,
    pub per_scenario_stats: BTreeMap<String, DraftEvaluationScenarioStats>,
    pub per_scenario_first_pass_rates: BTreeMap<String, usize>,
    pub error_code_histogram: BTreeMap<String, usize>,
    pub release_gate_passed: bool,
    pub non_release_smoke: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct DraftEvaluationScenarioStats {
    pub attempts: usize,
    pub first_pass_count: usize,
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
    let per_scenario_stats = scenario_totals
        .iter()
        .map(|(scenario_id, total)| {
            (
                scenario_id.clone(),
                DraftEvaluationScenarioStats {
                    attempts: *total,
                    first_pass_count: scenario_first_passes
                        .get(scenario_id)
                        .copied()
                        .unwrap_or_default(),
                },
            )
        })
        .collect();
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
        run_id: uuid::Uuid::new_v4().simple().to_string(),
        provider: input.provider,
        prompt_version: input.prompt_version,
        scenario_set_hash: input.scenario_set_hash,
        scenario_count: input.scenario_count,
        runs_per_scenario: input.runs_per_scenario,
        total_runs: expected_total,
        total_first_pass_count: first_passes,
        repair_attempt_count: repair_attempts,
        repaired_pass_count: repaired_passes,
        first_pass_rate_percent,
        repaired_pass_rate_percent,
        per_scenario_stats,
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

    let first_gate = validate_report_invariants(first)?;
    let second_gate = validate_report_invariants(second)?;
    if first.run_id == second.run_id {
        return Err(DraftEvaluationError::new(
            "draft_eval_same_run",
            "reports must come from two independent evaluation runs",
        ));
    }

    Ok(DraftEvaluationComparison {
        provider: first.provider.clone(),
        prompt_version: first.prompt_version.clone(),
        scenario_set_hash: first.scenario_set_hash.clone(),
        report_count: 2,
        release_gate_passed: first_gate && second_gate,
    })
}

fn validate_report_invariants(
    report: &DraftEvaluationReport,
) -> Result<bool, DraftEvaluationError> {
    if report.provider == ProviderType::Fake {
        return Err(DraftEvaluationError::new(
            "draft_eval_report_provider_invalid",
            "release reports must use codex or claude_code",
        ));
    }
    if report.run_id.len() != 32
        || !report
            .run_id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(DraftEvaluationError::new(
            "draft_eval_report_run_id_invalid",
            "report run_id must be a 32-character lowercase hexadecimal identifier",
        ));
    }
    let expected_total = report
        .scenario_count
        .checked_mul(report.runs_per_scenario)
        .ok_or_else(|| {
            DraftEvaluationError::new(
                "draft_eval_report_run_count_overflow",
                "scenario_count * runs_per_scenario overflowed",
            )
        })?;
    if report.total_runs != expected_total {
        return Err(DraftEvaluationError::new(
            "draft_eval_report_total_mismatch",
            "report total_runs does not match its run shape",
        ));
    }
    if report.per_scenario_stats.len() != report.scenario_count
        || report.per_scenario_first_pass_rates.len() != report.scenario_count
        || report
            .per_scenario_stats
            .keys()
            .ne(report.per_scenario_first_pass_rates.keys())
    {
        return Err(DraftEvaluationError::new(
            "draft_eval_report_scenario_stats_incomplete",
            "report must contain complete matching per-scenario statistics",
        ));
    }

    let mut total_attempts = 0usize;
    let mut total_first_passes = 0usize;
    let mut recomputed_rates = BTreeMap::new();
    for (scenario_id, stats) in &report.per_scenario_stats {
        if !is_safe_scenario_id(scenario_id) {
            return Err(DraftEvaluationError::new(
                "draft_eval_report_scenario_id_invalid",
                "report contains an unsafe scenario identifier",
            ));
        }
        if stats.attempts != report.runs_per_scenario || stats.first_pass_count > stats.attempts {
            return Err(DraftEvaluationError::new(
                "draft_eval_report_scenario_stats_invalid",
                "per-scenario attempts and first-pass counts are inconsistent",
            ));
        }
        total_attempts = total_attempts.checked_add(stats.attempts).ok_or_else(|| {
            DraftEvaluationError::new(
                "draft_eval_report_count_overflow",
                "per-scenario attempt count overflowed",
            )
        })?;
        total_first_passes = total_first_passes
            .checked_add(stats.first_pass_count)
            .ok_or_else(|| {
                DraftEvaluationError::new(
                    "draft_eval_report_count_overflow",
                    "per-scenario first-pass count overflowed",
                )
            })?;
        recomputed_rates.insert(
            scenario_id.clone(),
            percentage(stats.first_pass_count, stats.attempts),
        );
    }
    if total_attempts != report.total_runs {
        return Err(DraftEvaluationError::new(
            "draft_eval_report_attempt_count_mismatch",
            "per-scenario attempts do not sum to total_runs",
        ));
    }
    if total_first_passes != report.total_first_pass_count {
        return Err(DraftEvaluationError::new(
            "draft_eval_report_first_pass_count_mismatch",
            "per-scenario first-pass counts do not match the aggregate count",
        ));
    }
    if report.repaired_pass_count > report.repair_attempt_count {
        return Err(DraftEvaluationError::new(
            "draft_eval_report_repair_count_mismatch",
            "repaired pass count exceeds repair attempts",
        ));
    }
    let recomputed_first_pass_rate = percentage(total_first_passes, report.total_runs);
    let recomputed_repaired_pass_rate = if report.repair_attempt_count == 0 {
        0
    } else {
        percentage(report.repaired_pass_count, report.repair_attempt_count)
    };
    if report.first_pass_rate_percent != recomputed_first_pass_rate
        || report.repaired_pass_rate_percent != recomputed_repaired_pass_rate
        || report.per_scenario_first_pass_rates != recomputed_rates
    {
        return Err(DraftEvaluationError::new(
            "draft_eval_report_percentage_mismatch",
            "persisted percentages do not match the safe aggregate counts",
        ));
    }
    let recomputed_gate = !report.non_release_smoke
        && report.scenario_count >= MIN_RELEASE_SCENARIOS
        && report.runs_per_scenario >= DEFAULT_RUNS_PER_SCENARIO
        && recomputed_first_pass_rate >= MIN_OVERALL_FIRST_PASS_PERCENT
        && recomputed_rates
            .values()
            .all(|rate| *rate >= MIN_SCENARIO_FIRST_PASS_PERCENT);
    if report.release_gate_passed != recomputed_gate {
        return Err(DraftEvaluationError::new(
            "draft_eval_report_gate_mismatch",
            "persisted release gate does not match recomputed report invariants",
        ));
    }
    Ok(recomputed_gate)
}

pub fn is_safe_scenario_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
        })
}

fn percentage(numerator: usize, denominator: usize) -> usize {
    numerator
        .saturating_mul(100)
        .checked_div(denominator)
        .unwrap_or(0)
}
