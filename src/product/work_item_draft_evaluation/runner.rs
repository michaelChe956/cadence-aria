use std::path::{Component, Path};

use sha2::{Digest, Sha256};

use crate::cross_cutting::provider_adapter::{DEFAULT_PROVIDER_TIMEOUT_SECS, ProviderAdapter};
use crate::cross_cutting::structured_output::parse_last_structured_output_value;
use crate::product::models::{
    WorkItemDraftCandidate, WorkItemDraftRecord, WorkItemDraftStatus, WorkItemGenerationMode,
    WorkItemOutline, WorkItemOutlineDependencyEdge, WorkItemOutlineSessionFit, WorkItemPlanOutline,
};
use crate::product::work_item_contract::{
    AcceptanceCriterion, DesignTraceabilityRef, EvidenceKind, HandoffContract,
    PromisedOutputContract, VerificationCheck, WorkItemContractIdentity, WorkItemGoal,
    WorkItemTask, WorkItemWritePolicy,
};
use crate::product::work_item_draft_evaluation::types::{
    DEFAULT_RUNS_PER_SCENARIO, DraftEvaluationDependencySummary, DraftEvaluationError,
    DraftEvaluationOutcome, DraftEvaluationReport, DraftEvaluationReportInput,
    DraftEvaluationScenario, DraftEvaluationScenarioFile, MIN_RELEASE_SCENARIOS, build_report,
    is_safe_scenario_id,
};
use crate::product::work_item_split_engine::{
    WORK_ITEM_DRAFT_OUTPUT_SCHEMA, WORK_ITEM_DRAFT_PROMPT_VERSION,
    build_work_item_draft_invocation, parse_work_item_draft_output,
};
use crate::product::work_item_split_validator::WorkItemDraftLocalValidator;
use crate::product::workspace_engine::{
    combine_draft_validation_feedback, work_item_split_findings_to_dto,
};
use crate::protocol::contracts::{AdapterInput, AdapterRole, ProviderType};

pub fn load_scenarios_from_str(
    raw: &str,
) -> Result<Vec<DraftEvaluationScenario>, DraftEvaluationError> {
    let file: DraftEvaluationScenarioFile = serde_json::from_str(raw).map_err(|error| {
        DraftEvaluationError::new(
            "draft_eval_scenario_parse_error",
            format!("failed to parse scenario file: {error}"),
        )
    })?;
    if file.schema_version != 1 {
        return Err(DraftEvaluationError::new(
            "draft_eval_scenario_schema_unsupported",
            format!(
                "unsupported scenario schema version {}",
                file.schema_version
            ),
        ));
    }
    Ok(file.scenarios)
}

pub fn run_evaluation_with_adapter(
    adapter: &(dyn ProviderAdapter + Send + Sync),
    provider: ProviderType,
    workspace: &Path,
    scenarios: &[DraftEvaluationScenario],
    runs_per_scenario: usize,
    non_release_smoke: bool,
) -> Result<DraftEvaluationReport, DraftEvaluationError> {
    validate_evaluation_request(scenarios, runs_per_scenario, non_release_smoke)?;

    let mut outcomes = Vec::with_capacity(scenarios.len().saturating_mul(runs_per_scenario));
    for scenario in scenarios {
        let (outline, accepted_drafts) = materialize_scenario(scenario);
        let accepted_candidates = accepted_drafts
            .iter()
            .map(|draft| draft.candidate.clone())
            .collect::<Vec<_>>();
        for _ in 0..runs_per_scenario {
            outcomes.push(evaluate_one(
                adapter,
                provider.clone(),
                workspace,
                scenario,
                &outline,
                &accepted_drafts,
                &accepted_candidates,
            ));
        }
    }

    build_report(DraftEvaluationReportInput {
        provider,
        prompt_version: WORK_ITEM_DRAFT_PROMPT_VERSION.to_string(),
        scenario_set_hash: scenario_set_hash(scenarios)?,
        scenario_count: scenarios.len(),
        runs_per_scenario,
        non_release_smoke,
        outcomes,
    })
}

pub const REQUIRED_RELEASE_COVERAGE_CATEGORIES: [&str; 10] = [
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
];

pub fn validate_evaluation_request(
    scenarios: &[DraftEvaluationScenario],
    runs_per_scenario: usize,
    non_release_smoke: bool,
) -> Result<(), DraftEvaluationError> {
    validate_run_shape(scenarios.len(), runs_per_scenario, non_release_smoke)?;
    scenarios
        .len()
        .checked_mul(runs_per_scenario)
        .ok_or_else(|| {
            DraftEvaluationError::new(
                "draft_eval_run_count_overflow",
                "scenario_count * runs_per_scenario overflowed",
            )
        })?;
    validate_scenarios(scenarios, non_release_smoke)
}

pub fn validate_evaluation_scenario_corpus(
    scenarios: &[DraftEvaluationScenario],
) -> Result<(), DraftEvaluationError> {
    validate_scenarios(scenarios, true)
}

fn evaluate_one(
    adapter: &(dyn ProviderAdapter + Send + Sync),
    provider: ProviderType,
    workspace: &Path,
    scenario: &DraftEvaluationScenario,
    outline: &WorkItemPlanOutline,
    accepted_drafts: &[WorkItemDraftRecord],
    accepted_candidates: &[WorkItemDraftCandidate],
) -> DraftEvaluationOutcome {
    let first = run_provider_once(
        adapter,
        provider.clone(),
        workspace,
        scenario,
        outline,
        accepted_drafts,
        scenario.user_feedback.as_deref(),
    );
    let candidate = match first {
        Ok(candidate) => candidate,
        Err(error_codes) => return failed_outcome(scenario, error_codes),
    };
    let validation =
        WorkItemDraftLocalValidator::validate(&candidate, accepted_candidates, outline);
    if !validation.has_errors() {
        return DraftEvaluationOutcome {
            scenario_id: scenario.scenario_id.clone(),
            first_passed: true,
            repair_attempted: false,
            repaired_passed: false,
            error_codes: vec![],
        };
    }

    let mut error_codes = finding_codes(&validation.findings);
    let feedback = combine_draft_validation_feedback(
        scenario.user_feedback.as_deref(),
        &work_item_split_findings_to_dto(&validation.findings),
    );
    let repaired = run_provider_once(
        adapter,
        provider,
        workspace,
        scenario,
        outline,
        accepted_drafts,
        Some(&feedback),
    );
    let repaired_passed = match repaired {
        Ok(candidate) => {
            let repaired_validation =
                WorkItemDraftLocalValidator::validate(&candidate, accepted_candidates, outline);
            if repaired_validation.has_errors() {
                error_codes.extend(finding_codes(&repaired_validation.findings));
                false
            } else {
                true
            }
        }
        Err(repair_errors) => {
            error_codes.extend(repair_errors);
            false
        }
    };

    DraftEvaluationOutcome {
        scenario_id: scenario.scenario_id.clone(),
        first_passed: false,
        repair_attempted: true,
        repaired_passed,
        error_codes,
    }
}

fn run_provider_once(
    adapter: &(dyn ProviderAdapter + Send + Sync),
    provider: ProviderType,
    workspace: &Path,
    scenario: &DraftEvaluationScenario,
    outline: &WorkItemPlanOutline,
    accepted_drafts: &[WorkItemDraftRecord],
    feedback: Option<&str>,
) -> Result<WorkItemDraftCandidate, Vec<String>> {
    let invocation = build_work_item_draft_invocation(
        outline,
        &scenario.outline.outline_id,
        WorkItemGenerationMode::Serial,
        accepted_drafts,
        feedback,
    )
    .map_err(|error| vec![error.code])?;
    let worktree_path = workspace.join(&scenario.relative_worktree_path);
    let output = adapter
        .run(&AdapterInput {
            provider_type: provider,
            role: AdapterRole::WorkItemSplitter,
            worktree_path: Some(worktree_path.to_string_lossy().into_owned()),
            prompt: invocation.prompt,
            context_files: vec![],
            output_schema: WORK_ITEM_DRAFT_OUTPUT_SCHEMA.to_string(),
            timeout: DEFAULT_PROVIDER_TIMEOUT_SECS,
            max_retries: 0,
        })
        .map_err(|error| vec![error.code.as_str().to_string()])?;
    let value = parse_last_structured_output_value(&output.stdout)
        .map_err(|error| vec![error.code.as_str().to_string()])?
        .ok_or_else(|| vec!["structured_output_missing".to_string()])?;
    parse_work_item_draft_output(value).map_err(|error| vec![error.code])
}

fn failed_outcome(
    scenario: &DraftEvaluationScenario,
    error_codes: Vec<String>,
) -> DraftEvaluationOutcome {
    DraftEvaluationOutcome {
        scenario_id: scenario.scenario_id.clone(),
        first_passed: false,
        repair_attempted: false,
        repaired_passed: false,
        error_codes,
    }
}

fn finding_codes(findings: &[crate::product::models::WorkItemSplitFinding]) -> Vec<String> {
    findings
        .iter()
        .filter(|finding| {
            finding.severity == crate::product::models::WorkItemSplitFindingSeverity::Error
        })
        .map(|finding| finding.code.clone())
        .collect()
}

fn validate_run_shape(
    scenario_count: usize,
    runs_per_scenario: usize,
    non_release_smoke: bool,
) -> Result<(), DraftEvaluationError> {
    if runs_per_scenario == 0 {
        return Err(DraftEvaluationError::new(
            "draft_eval_runs_invalid",
            "runs_per_scenario must be greater than zero",
        ));
    }
    if non_release_smoke {
        if scenario_count == 0 || scenario_count > 2 || runs_per_scenario > 2 {
            return Err(DraftEvaluationError::new(
                "draft_eval_smoke_limit_exceeded",
                "smoke evaluation allows one or two scenarios and at most two runs per scenario",
            ));
        }
    } else if scenario_count < MIN_RELEASE_SCENARIOS {
        return Err(DraftEvaluationError::new(
            "draft_eval_scenario_count_too_small",
            format!("release evaluation requires at least {MIN_RELEASE_SCENARIOS} scenarios"),
        ));
    } else if runs_per_scenario < DEFAULT_RUNS_PER_SCENARIO {
        return Err(DraftEvaluationError::new(
            "draft_eval_runs_too_small",
            format!(
                "release evaluation requires at least {DEFAULT_RUNS_PER_SCENARIO} runs per scenario"
            ),
        ));
    }
    Ok(())
}

fn validate_scenarios(
    scenarios: &[DraftEvaluationScenario],
    non_release_smoke: bool,
) -> Result<(), DraftEvaluationError> {
    let mut ids = std::collections::BTreeSet::new();
    let mut category_counts = std::collections::BTreeMap::<&str, usize>::new();
    for scenario in scenarios {
        if !is_safe_scenario_id(&scenario.scenario_id) {
            return Err(DraftEvaluationError::new(
                "draft_eval_scenario_id_invalid",
                "scenario_id must be 1-64 lowercase ASCII letters, digits, underscores, or hyphens",
            ));
        }
        if !ids.insert(&scenario.scenario_id) {
            return Err(DraftEvaluationError::new(
                "draft_eval_scenario_id_duplicate",
                format!("duplicate scenario id {}", scenario.scenario_id),
            ));
        }
        let path = Path::new(&scenario.relative_worktree_path);
        if path.is_absolute()
            || path
                .components()
                .any(|component| matches!(component, Component::ParentDir | Component::RootDir))
        {
            return Err(DraftEvaluationError::new(
                "draft_eval_worktree_path_invalid",
                format!(
                    "scenario {} must use a workspace-relative worktree path",
                    scenario.scenario_id
                ),
            ));
        }
        if scenario.expected_coverage_categories.is_empty() {
            return Err(DraftEvaluationError::new(
                "draft_eval_coverage_category_required",
                format!(
                    "scenario {} must declare expected coverage categories",
                    scenario.scenario_id
                ),
            ));
        }
        let mut scenario_categories = std::collections::BTreeSet::new();
        for category in &scenario.expected_coverage_categories {
            let Some(required) = REQUIRED_RELEASE_COVERAGE_CATEGORIES
                .iter()
                .find(|required| **required == category)
                .copied()
            else {
                return Err(DraftEvaluationError::new(
                    "draft_eval_coverage_category_invalid",
                    format!("scenario {} uses unknown category", scenario.scenario_id),
                ));
            };
            if !scenario_categories.insert(required) {
                return Err(DraftEvaluationError::new(
                    "draft_eval_duplicate_scenario_category",
                    format!(
                        "scenario {} repeats coverage category {required}",
                        scenario.scenario_id
                    ),
                ));
            }
        }
        for category in scenario_categories {
            *category_counts.entry(category).or_default() += 1;
        }
    }
    if !non_release_smoke
        && REQUIRED_RELEASE_COVERAGE_CATEGORIES
            .iter()
            .any(|category| category_counts.get(category).copied().unwrap_or_default() < 2)
    {
        return Err(DraftEvaluationError::new(
            "draft_eval_required_category_coverage",
            "release evaluation requires every required category at least twice",
        ));
    }
    Ok(())
}

fn scenario_set_hash(
    scenarios: &[DraftEvaluationScenario],
) -> Result<String, DraftEvaluationError> {
    let bytes = serde_json::to_vec(scenarios).map_err(|error| {
        DraftEvaluationError::new(
            "draft_eval_scenario_hash_failed",
            format!("failed to serialize scenarios for hashing: {error}"),
        )
    })?;
    Ok(format!("sha256:{}", hex::encode(Sha256::digest(bytes))))
}

fn materialize_scenario(
    scenario: &DraftEvaluationScenario,
) -> (WorkItemPlanOutline, Vec<WorkItemDraftRecord>) {
    let mut outlines = scenario
        .accepted_dependency_summaries
        .iter()
        .map(dependency_outline)
        .collect::<Vec<_>>();
    outlines.push(WorkItemOutline {
        outline_id: scenario.outline.outline_id.clone(),
        logical_work_item_id: scenario.outline.logical_work_item_id.clone(),
        title: scenario.outline.title.clone(),
        kind: scenario.outline.kind.clone(),
        goal: scenario.outline.goal.clone(),
        scope: scenario.outline.scope.clone(),
        non_goals: scenario.outline.non_goals.clone(),
        estimated_context_tokens: Some(8_000),
        session_fit: Some(WorkItemOutlineSessionFit::FitsSingleAgentSession),
        source_story_spec_ids: vec!["story_placeholder".to_string()],
        source_design_spec_ids: vec!["design_placeholder".to_string()],
        exclusive_write_scopes: scenario.outline.exclusive_write_scopes.clone(),
        forbidden_write_scopes: scenario.outline.forbidden_write_scopes.clone(),
        depends_on: scenario.outline.depends_on.clone(),
        verification_intent: scenario.outline.verification_intent.clone(),
        trusted_verification_commands: scenario.trusted_verification_command_catalog.clone(),
        handoff_notes: "Provide an abstract handoff summary".to_string(),
    });
    let dependency_graph = scenario
        .outline
        .depends_on
        .iter()
        .map(|dependency| WorkItemOutlineDependencyEdge {
            from_outline_id: dependency.clone(),
            to_outline_id: scenario.outline.outline_id.clone(),
        })
        .collect();
    let outline = WorkItemPlanOutline {
        id: format!("eval_plan_{}", scenario.scenario_id),
        project_id: "project_anonymous".to_string(),
        issue_id: "context_anonymous".to_string(),
        source_story_spec_ids: vec!["story_placeholder".to_string()],
        source_design_spec_ids: vec!["design_placeholder".to_string()],
        strategy_summary: "Apply abstract placeholder design semantics".to_string(),
        work_item_outlines: outlines,
        dependency_graph,
        risks: vec![],
        handoff_strategy: "Use accepted abstract dependency summaries".to_string(),
        status: "draft".to_string(),
    };
    let accepted = scenario
        .accepted_dependency_summaries
        .iter()
        .map(accepted_dependency_record)
        .collect();
    (outline, accepted)
}

fn dependency_outline(summary: &DraftEvaluationDependencySummary) -> WorkItemOutline {
    WorkItemOutline {
        outline_id: summary.outline_id.clone(),
        logical_work_item_id: summary.logical_work_item_id.clone(),
        title: summary.title.clone(),
        kind: crate::product::models::WorkItemKind::Backend,
        goal: summary.summary.clone(),
        scope: vec![format!("modules/{}", summary.logical_work_item_id)],
        non_goals: vec![],
        estimated_context_tokens: Some(4_000),
        session_fit: Some(WorkItemOutlineSessionFit::FitsSingleAgentSession),
        source_story_spec_ids: vec!["story_placeholder".to_string()],
        source_design_spec_ids: vec!["design_placeholder".to_string()],
        exclusive_write_scopes: vec![format!("modules/{}/**", summary.logical_work_item_id)],
        forbidden_write_scopes: vec![],
        depends_on: vec![],
        verification_intent: vec!["Dependency verification already accepted".to_string()],
        trusted_verification_commands: vec![],
        handoff_notes: summary.summary.clone(),
    }
}

fn accepted_dependency_record(summary: &DraftEvaluationDependencySummary) -> WorkItemDraftRecord {
    let output_contracts = summary
        .promised_contract_refs
        .iter()
        .map(|contract_id| PromisedOutputContract {
            contract_id: contract_id.clone(),
            capabilities: vec!["placeholder_capability".to_string()],
        })
        .collect::<Vec<_>>();
    let verification_checks = vec![VerificationCheck {
        check_id: "check_dependency_handoff".to_string(),
        command: None,
        manual_instruction: Some("Inspect the accepted dependency handoff".to_string()),
        required: false,
        non_zero_test_execution_required: false,
    }];
    let candidate = WorkItemDraftCandidate {
        outline_id: summary.outline_id.clone(),
        logical_work_item_id: summary.logical_work_item_id.clone(),
        canonical_contract_candidate:
            crate::product::work_item_contract::CanonicalWorkItemContract {
                schema_version: 1,
                identity: WorkItemContractIdentity {
                    logical_work_item_id: summary.logical_work_item_id.clone(),
                    title: summary.title.clone(),
                    kind: "backend".to_string(),
                },
                goal: WorkItemGoal {
                    summary: summary.summary.clone(),
                },
                non_goals: vec![],
                input_contracts: vec![],
                output_contracts,
                tasks: vec![WorkItemTask {
                    task_id: "task_dependency".to_string(),
                    statement: summary.summary.clone(),
                    requirement_refs: vec!["REQ-PLACEHOLDER-001".to_string()],
                    done_when_refs: vec!["AC-DEPENDENCY-001".to_string()],
                }],
                write_policy: WorkItemWritePolicy {
                    exclusive_scopes: vec![format!("modules/{}/**", summary.logical_work_item_id)],
                    forbidden_scopes: vec![],
                },
                acceptance_criteria: vec![AcceptanceCriterion {
                    criterion_id: "AC-DEPENDENCY-001".to_string(),
                    statement: "Accepted dependency handoff is available".to_string(),
                    required_evidence: vec![EvidenceKind::HandoffField],
                }],
                verification_checks: verification_checks.clone(),
                handoff_contract: HandoffContract {
                    required_fields: vec!["summary".to_string()],
                    provided_contract_refs: summary.promised_contract_refs.clone(),
                    reviewer_check_refs: vec!["AC-DEPENDENCY-001".to_string()],
                },
                blocker_rules: vec![],
                design_traceability: vec![DesignTraceabilityRef {
                    source_type: "design_spec".to_string(),
                    source_id: "design_placeholder".to_string(),
                    requirement_id: "REQ-PLACEHOLDER-001".to_string(),
                }],
            },
        verification_plan: crate::product::models::WorkItemDraftVerificationPlan {
            checks: verification_checks,
        },
    };
    WorkItemDraftRecord {
        project_id: "project_anonymous".to_string(),
        issue_id: "context_anonymous".to_string(),
        plan_id: "eval_plan".to_string(),
        draft_id: format!("draft_{}", summary.outline_id),
        outline_id: summary.outline_id.clone(),
        generation_round_id: "round_1".to_string(),
        batch_id: None,
        attempt_index: 1,
        outline_version_ref: "outline_v1".to_string(),
        generation_mode: WorkItemGenerationMode::Serial,
        generation_diagnostics: None,
        candidate,
        status: WorkItemDraftStatus::Accepted,
        active: true,
        superseded_by_draft_id: None,
        supersede_reason: None,
        copied_from_draft_id: None,
        review_node_id: None,
        review_verdict_ref: None,
        generated_from_node_id: "node_placeholder".to_string(),
        accepted_at: Some("2026-01-01T00:00:00Z".to_string()),
        superseded_at: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    }
}
