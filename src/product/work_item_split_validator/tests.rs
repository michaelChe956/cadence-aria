use std::collections::BTreeMap;

use super::*;
use crate::product::models::{
    IssueWorkItemDependencyEdge, IssueWorkItemPlanOptions, IssueWorkItemPlanStatus,
    TrustedDraftVerificationCommand, VerificationCommand, VerificationFallbackPolicy,
    VerificationScope, WorkItemContextBudget, WorkItemDraftCandidate, WorkItemExecutionPlanStatus,
    WorkItemOutline, WorkItemOutlineDependencyEdge, WorkItemOutlineSessionFit, WorkItemPlanStatus,
    WorkItemStatus,
};
use crate::product::planning_failure::{
    PlanningFailure, PlanningFailureEvidence, PlanningFailureKind, PlanningFailureOrigin,
    PlanningFailureSet, PlanningRecoveryDisposition, PlanningStage, StructuralAuthorizationFacts,
    StructuralAuthorizationKind, classify_failure_set,
};

type VerificationMutation = fn(&mut WorkItemDraftCandidate);

#[test]
fn work_item_plan_draft_validator_maps_canonical_contract_findings() {
    let outline = valid_outline();
    let mut candidate = canonical_draft_candidate(&outline.work_item_outlines[0]);
    candidate
        .canonical_contract_candidate
        .tasks
        .push(candidate.canonical_contract_candidate.tasks[0].clone());

    let report = WorkItemDraftLocalValidator::validate(&candidate, &[], &outline);

    let finding = report
        .findings
        .iter()
        .find(|finding| finding.code == "duplicate_task_id")
        .expect("canonical validator finding must be mapped");
    assert_eq!(finding.severity, WorkItemSplitFindingSeverity::Error);
}

#[test]
fn work_item_plan_draft_validator_keeps_process_evidence_warning_non_blocking() {
    let outline = valid_outline();
    let mut candidate = canonical_draft_candidate(&outline.work_item_outlines[0]);
    candidate.canonical_contract_candidate.acceptance_criteria[0].criterion_id =
        "ac_tdd_red_evidence".to_string();
    candidate.canonical_contract_candidate.acceptance_criteria[0].statement =
        "先失败的测试提交必须存在".to_string();
    candidate.canonical_contract_candidate.tasks[0].done_when_refs =
        vec!["ac_tdd_red_evidence".to_string()];
    candidate
        .canonical_contract_candidate
        .handoff_contract
        .reviewer_check_refs = vec!["ac_tdd_red_evidence".to_string()];

    let report = WorkItemDraftLocalValidator::validate(&candidate, &[], &outline);
    let finding = report
        .findings
        .iter()
        .find(|finding| finding.code == "process_evidence_acceptance_criterion")
        .expect("process-evidence warning must be visible in the local validator report");

    assert_eq!(finding.severity, WorkItemSplitFindingSeverity::Warning);
    assert!(!report.has_errors());
}

#[test]
fn work_item_plan_draft_validator_rejects_missing_output_provider() {
    let outline = valid_outline();
    let mut candidate = canonical_draft_candidate(&outline.work_item_outlines[1]);
    candidate.canonical_contract_candidate.input_contracts[0].provider_logical_work_item_id =
        "wi_missing".to_string();

    let report = WorkItemDraftLocalValidator::validate(&candidate, &[], &outline);

    assert_has_code(&report, "unknown_provider_logical_work_item");
}

#[test]
fn work_item_plan_draft_validator_rejects_verification_plan_full_typed_drift() {
    let outline = valid_outline();
    let base = canonical_draft_candidate(&outline.work_item_outlines[0]);
    let mutations: [(&str, VerificationMutation); 8] = [
        ("check_id", |candidate| {
            candidate.verification_plan.checks[0].check_id = "check_drift".to_string();
        }),
        ("command", |candidate| {
            candidate.verification_plan.checks[0].command = Some("cargo check".to_string());
        }),
        ("manual_instruction", |candidate| {
            candidate.verification_plan.checks[0].manual_instruction =
                Some("manual drift".to_string());
        }),
        ("required", |candidate| {
            candidate.verification_plan.checks[0].required = false;
        }),
        ("non_zero", |candidate| {
            candidate.verification_plan.checks[0].non_zero_test_execution_required = false;
        }),
        ("missing", |candidate| {
            candidate.verification_plan.checks.pop();
        }),
        ("extra", |candidate| {
            let mut extra = candidate.verification_plan.checks[0].clone();
            extra.check_id = "check_extra".to_string();
            candidate.verification_plan.checks.push(extra);
        }),
        ("order", |candidate| {
            candidate.verification_plan.checks.swap(0, 1);
        }),
    ];

    for (field, mutate) in mutations {
        let mut candidate = base.clone();
        mutate(&mut candidate);

        let report = WorkItemDraftLocalValidator::validate(&candidate, &[], &outline);

        assert!(
            has_code(&report, "verification_plan_not_derived_from_contract"),
            "expected verification drift for {field}, got {:?}",
            report.findings
        );
    }
}

#[test]
fn work_item_plan_draft_validator_accepts_clean_canonical_candidate() {
    let outline = valid_outline();
    let candidate = canonical_draft_candidate(&outline.work_item_outlines[0]);

    let report = WorkItemDraftLocalValidator::validate(&candidate, &[], &outline);

    assert!(
        !report.has_errors(),
        "expected clean candidate, got {:?}",
        report.findings
    );
}

#[test]
fn work_item_plan_draft_validator_rejects_required_command_outside_confirmed_catalog() {
    let outline = valid_outline();
    let mut candidate = canonical_draft_candidate(&outline.work_item_outlines[0]);
    candidate.canonical_contract_candidate.verification_checks[0].command =
        Some("pnpm --dir web test".to_string());
    candidate.verification_plan.checks = candidate
        .canonical_contract_candidate
        .verification_checks
        .clone();

    let report = WorkItemDraftLocalValidator::validate(&candidate, &[], &outline);

    assert_has_code(&report, "untrusted_required_verification_command");
}

#[test]
fn work_item_plan_draft_validator_trusted_command_matrix_has_one_target_finding() {
    type Mutation = fn(&mut WorkItemPlanOutline, &mut WorkItemDraftCandidate);

    let cases: [(&str, Mutation); 2] = [
        ("untrusted_required_verification_command", |outline, _| {
            outline.work_item_outlines[0].trusted_verification_commands[0].command =
                "pnpm --dir web test".to_string();
        }),
        (
            "missing_trusted_verification_command_catalog",
            |outline, _| {
                outline.work_item_outlines[0]
                    .trusted_verification_commands
                    .clear();
            },
        ),
    ];

    for (expected_code, mutate) in cases {
        let mut outline = valid_outline();
        let mut candidate = canonical_draft_candidate(&outline.work_item_outlines[0]);
        mutate(&mut outline, &mut candidate);

        let report = WorkItemDraftLocalValidator::validate(&candidate, &[], &outline);

        assert!(report.has_errors(), "{expected_code} must reject the Draft");
        assert_eq!(
            report
                .findings
                .iter()
                .filter(|finding| finding.code == expected_code)
                .count(),
            1,
            "expected exactly one {expected_code}, got {:?}",
            report.findings
        );
        for unrelated_identity_code in [
            "unknown_provider_logical_work_item",
            "draft_outline_identity_mismatch",
            "verification_plan_not_derived_from_contract",
        ] {
            assert!(
                !has_code(&report, unrelated_identity_code),
                "{expected_code} must not create unrelated identity finding {unrelated_identity_code}: {:?}",
                report.findings
            );
        }
    }
}

#[test]
fn p4_legacy_draft_trusted_catalog_rules_remain_active() {
    let mut outline = valid_outline();
    outline.work_item_outlines[0]
        .trusted_verification_commands
        .clear();
    let candidate = canonical_draft_candidate(&outline.work_item_outlines[0]);
    let report = WorkItemDraftLocalValidator::validate(&candidate, &[], &outline);
    assert_has_code(&report, "missing_trusted_verification_command_catalog");
}

#[test]
fn p4_sc_outline_budget_rules_remain_active() {
    let mut outline = valid_outline();
    outline.work_item_outlines[0].estimated_context_tokens = Some(50_001);
    outline.work_item_outlines[0].session_fit = Some(WorkItemOutlineSessionFit::TooLargeMustSplit);
    let report = WorkItemPlanOutlineValidator::validate_for_single_candidate(&outline);
    for code in [
        "outline_exceeds_single_session_budget",
        "outline_too_large_must_split",
    ] {
        assert_has_code(&report, code);
    }
    assert!(report.findings.iter().all(|finding| {
        !finding
            .code
            .starts_with("trusted_verification_command_catalog_")
    }));
}

#[test]
fn work_item_plan_draft_validator_requires_operational_gate_when_catalog_is_empty() {
    let mut outline = valid_outline();
    outline.work_item_outlines[0]
        .trusted_verification_commands
        .clear();
    let candidate = canonical_draft_candidate(&outline.work_item_outlines[0]);

    let report = WorkItemDraftLocalValidator::validate(&candidate, &[], &outline);

    assert_has_code(&report, "missing_trusted_verification_command_catalog");
}

#[test]
fn work_item_plan_draft_validator_allows_empty_catalog_with_operational_gate() {
    let mut outline = valid_outline();
    outline.work_item_outlines[0]
        .trusted_verification_commands
        .clear();
    let mut candidate = canonical_draft_candidate(&outline.work_item_outlines[0]);
    for check in &mut candidate.canonical_contract_candidate.verification_checks {
        check.command = None;
        check.required = false;
        check.non_zero_test_execution_required = false;
    }
    candidate.canonical_contract_candidate.blocker_rules[0].route =
        crate::product::work_item_contract::BlockerRoute::OperationalGate;
    candidate.canonical_contract_candidate.blocker_rules[0]
        .target_contract_refs
        .clear();
    candidate.verification_plan.checks = candidate
        .canonical_contract_candidate
        .verification_checks
        .clone();

    let report = WorkItemDraftLocalValidator::validate(&candidate, &[], &outline);

    assert!(
        !has_code(&report, "missing_trusted_verification_command_catalog"),
        "operational gate must make an empty catalog routable: {:?}",
        report.findings
    );
}

#[test]
fn work_item_plan_outline_validator_rejects_trusted_command_catalog_over_budget() {
    let mut outline = valid_outline();
    let command = outline.work_item_outlines[0].trusted_verification_commands[0].clone();
    outline.work_item_outlines[0].trusted_verification_commands = vec![command; 4];

    let report = WorkItemPlanOutlineValidator::validate(&outline);

    assert_has_code(&report, "trusted_verification_command_catalog_too_large");
}

#[test]
fn work_item_plan_outline_validator_rejects_overlong_trusted_command_catalog_fields() {
    for field in ["command", "cwd", "purpose", "source_ref"] {
        let mut outline = valid_outline();
        let command = &mut outline.work_item_outlines[0].trusted_verification_commands[0];
        match field {
            "command" => command.command = "c".repeat(49),
            "cwd" => command.cwd = "w".repeat(17),
            "purpose" => command.purpose = "p".repeat(33),
            "source_ref" => command.source_ref = "s".repeat(33),
            _ => unreachable!("only trusted catalog fields are checked"),
        }

        let report = WorkItemPlanOutlineValidator::validate(&outline);

        assert_has_code(
            &report,
            "trusted_verification_command_catalog_field_too_large",
        );
    }
}

#[test]
fn work_item_plan_outline_validator_rejects_trusted_catalog_utf8_projection_over_budget() {
    let mut outline = valid_outline();
    let mut command = outline.work_item_outlines[0].trusted_verification_commands[0].clone();
    command.command = "界".repeat(48);
    command.cwd.clear();
    command.purpose.clear();
    command.source_ref.clear();
    outline.work_item_outlines[0].trusted_verification_commands = vec![command; 3];

    let report = WorkItemPlanOutlineValidator::validate(&outline);

    assert_has_code(
        &report,
        "trusted_verification_command_catalog_projection_too_large",
    );
}

#[test]
fn work_item_plan_draft_validator_fails_closed_when_candidate_outline_is_missing() {
    let outline = valid_outline();
    let mut candidate = canonical_draft_candidate(&outline.work_item_outlines[0]);
    candidate.outline_id = "outline_missing".to_string();

    let report = WorkItemDraftLocalValidator::validate(&candidate, &[], &outline);

    assert_has_code(&report, "draft_outline_not_found");
}

#[test]
fn work_item_plan_draft_validator_fails_closed_on_duplicate_outline_or_logical_identity() {
    let mut duplicate_outline = valid_outline();
    duplicate_outline.work_item_outlines[1].outline_id =
        duplicate_outline.work_item_outlines[0].outline_id.clone();
    let candidate = canonical_draft_candidate(&duplicate_outline.work_item_outlines[0]);

    let report = WorkItemDraftLocalValidator::validate(&candidate, &[], &duplicate_outline);

    assert_has_code(&report, "duplicate_outline_id");

    let mut duplicate_logical = valid_outline();
    duplicate_logical.work_item_outlines[1].logical_work_item_id = duplicate_logical
        .work_item_outlines[0]
        .logical_work_item_id
        .clone();
    let candidate = canonical_draft_candidate(&duplicate_logical.work_item_outlines[0]);

    let report = WorkItemDraftLocalValidator::validate(&candidate, &[], &duplicate_logical);

    assert_has_code(&report, "duplicate_logical_work_item_identity");
}

#[test]
fn work_item_plan_draft_validator_rejects_candidate_and_outline_identity_mismatch() {
    let outline = valid_outline();
    let mut logical_mismatch = canonical_draft_candidate(&outline.work_item_outlines[0]);
    logical_mismatch.logical_work_item_id = "wi_other".to_string();

    let report = WorkItemDraftLocalValidator::validate(&logical_mismatch, &[], &outline);

    assert_has_code(&report, "draft_logical_identity_mismatch");

    let mut outline_mismatch = canonical_draft_candidate(&outline.work_item_outlines[0]);
    outline_mismatch.canonical_contract_candidate.identity.title = "Other title".to_string();

    let report = WorkItemDraftLocalValidator::validate(&outline_mismatch, &[], &outline);

    assert_has_code(&report, "draft_outline_identity_mismatch");
}

#[test]
fn outline_validator_rejects_duplicate_outline_ids() {
    let mut outline = valid_outline();
    outline.work_item_outlines[1].outline_id = "outline_backend".to_string();

    let report = WorkItemPlanOutlineValidator::validate(&outline);

    assert_has_code(&report, "duplicate_outline_id");
}

#[test]
fn outline_validator_rejects_missing_dependency() {
    let mut outline = valid_outline();
    outline.work_item_outlines[1].depends_on = vec!["outline_missing".to_string()];

    let report = WorkItemPlanOutlineValidator::validate(&outline);

    assert_has_code(&report, "dependency_not_in_outline");
}

#[test]
fn outline_validator_rejects_dependency_cycle() {
    let mut outline = valid_outline();
    outline.work_item_outlines[0].depends_on = vec!["outline_frontend".to_string()];

    let report = WorkItemPlanOutlineValidator::validate(&outline);

    assert_has_code(&report, "dependency_cycle");
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.message.contains("depends_on")),
        "cycle diagnostic should identify depends_on as the source, got {:?}",
        report.findings
    );
}

#[test]
fn outline_validator_reports_reversed_dependency_graph_without_cycle_noise() {
    let mut outline = valid_outline();
    outline.dependency_graph = vec![WorkItemOutlineDependencyEdge {
        from_outline_id: "outline_frontend".to_string(),
        to_outline_id: "outline_backend".to_string(),
    }];

    let report = WorkItemPlanOutlineValidator::validate(&outline);

    assert_has_code(&report, "dependency_graph_direction_reversed");
    assert!(
        !has_code(&report, "dependency_cycle"),
        "reversed derived graph should be diagnosed as mismatch, got {:?}",
        report.findings
    );
}

#[test]
fn outline_validator_requires_traceability_and_write_scopes() {
    let mut outline = valid_outline();
    outline.work_item_outlines[0].source_story_spec_ids.clear();
    outline.work_item_outlines[0].source_design_spec_ids.clear();
    outline.work_item_outlines[0].goal.clear();
    outline.work_item_outlines[0].scope.clear();
    outline.work_item_outlines[0].exclusive_write_scopes.clear();

    let report = WorkItemPlanOutlineValidator::validate(&outline);

    assert_has_code(&report, "traceability_required");
    assert_has_code(&report, "outline_goal_required");
    assert_has_code(&report, "outline_scope_required");
    assert_has_code(&report, "write_scope_required");
}

#[test]
fn outline_validator_requires_single_session_budget() {
    let mut outline = valid_outline();
    outline.work_item_outlines[0].estimated_context_tokens = None;
    outline.work_item_outlines[0].session_fit = None;
    outline.work_item_outlines[1].estimated_context_tokens = Some(50_001);
    outline.work_item_outlines[1].session_fit = Some(WorkItemOutlineSessionFit::TooLargeMustSplit);

    let report = WorkItemPlanOutlineValidator::validate(&outline);

    assert_has_code(&report, "outline_budget_required");
    assert_has_code(&report, "outline_session_fit_required");
    assert_has_code(&report, "outline_exceeds_single_session_budget");
    assert_has_code(&report, "outline_too_large_must_split");
}

#[test]
fn outline_validator_accepts_soft_and_hard_session_budget_boundaries() {
    for value in [40_000, 40_001, 50_000] {
        let mut outline = valid_outline();
        outline.work_item_outlines[0].estimated_context_tokens = Some(value);

        let report = WorkItemPlanOutlineValidator::validate(&outline);

        assert!(
            !has_code(&report, "outline_exceeds_single_session_budget"),
            "budget {value} should be accepted, got {:?}",
            report.findings
        );
    }
}

#[test]
fn outline_validator_detects_direct_scope_conflict() {
    let mut outline = valid_outline();
    outline.work_item_outlines[1].depends_on.clear();
    outline.dependency_graph.clear();
    outline.work_item_outlines[1].exclusive_write_scopes = vec!["src/product/api.rs".to_string()];

    let report = WorkItemPlanOutlineValidator::validate(&outline);

    assert_has_code(&report, "parallel_scope_overlap");
}

#[test]
fn outline_validator_detects_dependent_scope_conflict() {
    let mut outline = valid_outline();
    outline.work_item_outlines[1].exclusive_write_scopes = vec!["src/product/api.rs".to_string()];

    let report = WorkItemPlanOutlineValidator::validate(&outline);

    assert_has_code(&report, "write_scope_conflict");
}

#[test]
fn local_validator_allows_valid_single_draft() {
    let outline = valid_outline();
    let dependency = canonical_draft_candidate(&outline.work_item_outlines[0]);
    let current = canonical_draft_candidate(&outline.work_item_outlines[1]);

    let report = WorkItemDraftLocalValidator::validate(&current, &[dependency], &outline);

    assert!(
        !report.has_errors(),
        "expected valid local draft, got {:?}",
        report.findings
    );
}

#[test]
fn local_validator_blocks_missing_write_scope() {
    let outline = valid_outline();
    let mut current = canonical_draft_candidate(&outline.work_item_outlines[0]);
    current
        .canonical_contract_candidate
        .write_policy
        .exclusive_scopes
        .clear();

    let report = WorkItemDraftLocalValidator::validate(&current, &[], &outline);

    assert_has_code(&report, "write_scope_required");
}

#[test]
fn local_validator_blocks_verification_plan_drift() {
    let outline = valid_outline();
    let mut current = canonical_draft_candidate(&outline.work_item_outlines[0]);
    current.verification_plan.checks[0].check_id = "check_missing".to_string();

    let report = WorkItemDraftLocalValidator::validate(&current, &[], &outline);

    assert_has_code(&report, "verification_plan_not_derived_from_contract");
}

#[test]
fn local_validator_blocks_scope_conflict_with_direct_dependency() {
    let outline = valid_outline();
    let dependency = canonical_draft_candidate(&outline.work_item_outlines[0]);
    let mut current = canonical_draft_candidate(&outline.work_item_outlines[1]);
    current
        .canonical_contract_candidate
        .write_policy
        .exclusive_scopes = vec!["src/product/api.rs".to_string()];

    let report = WorkItemDraftLocalValidator::validate(&current, &[dependency], &outline);

    assert_has_code(&report, "direct_dependency_scope_conflict");
}

fn assert_has_code(report: &WorkItemSplitValidationReport, code: &str) {
    assert!(
        has_code(report, code),
        "expected code {code}, got {:?}",
        report.findings
    );
}

fn has_code(report: &WorkItemSplitValidationReport, code: &str) -> bool {
    report.findings.iter().any(|finding| finding.code == code)
}

fn valid_outline() -> WorkItemPlanOutline {
    WorkItemPlanOutline {
        id: "outline_artifact_1".to_string(),
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        source_story_spec_ids: vec!["story_spec_0001".to_string()],
        source_design_spec_ids: vec!["design_spec_0001".to_string()],
        strategy_summary: "后端先行，前端随后接入".to_string(),
        work_item_outlines: vec![
            WorkItemOutline {
                target_repository_id: None,
                outline_id: "outline_backend".to_string(),
                logical_work_item_id: "wi_backend".to_string(),
                title: "后端 API".to_string(),
                kind: WorkItemKind::Backend,
                goal: "实现 API".to_string(),
                scope: vec!["src/product".to_string()],
                non_goals: vec![],
                estimated_context_tokens: Some(12_000),
                session_fit: Some(WorkItemOutlineSessionFit::FitsSingleAgentSession),
                source_story_spec_ids: vec!["story_spec_0001".to_string()],
                source_design_spec_ids: vec!["design_spec_0001".to_string()],
                exclusive_write_scopes: vec!["src/product/api.rs".to_string()],
                forbidden_write_scopes: vec!["web/**".to_string()],
                depends_on: vec![],
                verification_intent: vec!["cargo test --locked --lib api".to_string()],
                trusted_verification_commands: vec![TrustedDraftVerificationCommand {
                    command: "cargo test --locked --lib canonical_work_item_".to_string(),
                    cwd: ".".to_string(),
                    purpose: "验证 canonical contract".to_string(),
                    source_ref: "design_spec_0001#verification".to_string(),
                }],
                handoff_notes: "提供 API contract".to_string(),
            },
            WorkItemOutline {
                target_repository_id: None,
                outline_id: "outline_frontend".to_string(),
                logical_work_item_id: "wi_frontend".to_string(),
                title: "前端 UI".to_string(),
                kind: WorkItemKind::Frontend,
                goal: "接入 API".to_string(),
                scope: vec!["web/src".to_string()],
                non_goals: vec![],
                estimated_context_tokens: Some(10_000),
                session_fit: Some(WorkItemOutlineSessionFit::FitsSingleAgentSession),
                source_story_spec_ids: vec!["story_spec_0001".to_string()],
                source_design_spec_ids: vec!["design_spec_0001".to_string()],
                exclusive_write_scopes: vec!["web/src/session.ts".to_string()],
                forbidden_write_scopes: vec!["src/product/**".to_string()],
                depends_on: vec!["outline_backend".to_string()],
                verification_intent: vec!["pnpm -C web test".to_string()],
                trusted_verification_commands: vec![TrustedDraftVerificationCommand {
                    command: "cargo test --locked --lib canonical_work_item_".to_string(),
                    cwd: ".".to_string(),
                    purpose: "验证 canonical contract".to_string(),
                    source_ref: "design_spec_0001#verification".to_string(),
                }],
                handoff_notes: "消费 API contract".to_string(),
            },
        ],
        dependency_graph: vec![WorkItemOutlineDependencyEdge {
            from_outline_id: "outline_backend".to_string(),
            to_outline_id: "outline_frontend".to_string(),
        }],
        risks: vec![],
        handoff_strategy: "后端输出 contract 给前端".to_string(),
        status: "draft".to_string(),
    }
}

fn canonical_draft_candidate(outline: &WorkItemOutline) -> WorkItemDraftCandidate {
    let mut contract = crate::product::work_item_contract::canonical_contract_fixture(
        &outline.logical_work_item_id,
    );
    contract.identity.title = outline.title.clone();
    contract.identity.kind = outline.kind.as_str().to_string();
    contract.write_policy.exclusive_scopes = outline.exclusive_write_scopes.clone();
    contract.write_policy.forbidden_scopes = outline.forbidden_write_scopes.clone();
    if outline.depends_on.is_empty() {
        contract.input_contracts.clear();
    } else {
        contract.input_contracts[0].provider_logical_work_item_id = "wi_backend".to_string();
    }
    contract
        .verification_checks
        .push(crate::product::work_item_contract::VerificationCheck {
            check_id: "check_manual".to_string(),
            command: None,
            manual_instruction: Some("Inspect the generated contract".to_string()),
            required: false,
            non_zero_test_execution_required: false,
        });

    WorkItemDraftCandidate {
        target_repository_id: None,
        outline_id: outline.outline_id.clone(),
        logical_work_item_id: outline.logical_work_item_id.clone(),
        verification_plan: crate::product::models::WorkItemDraftVerificationPlan {
            checks: contract.verification_checks.clone(),
        },
        canonical_contract_candidate: contract,
    }
}

// ---------------------------------------------------------------------------
// F2b（REQ-SG-01/02/02A）：split/preflight 显式 typed 工厂与授权排除。
// 断言基于真实 fixture 观察 findings 的 kind payload 与 classify_failure_set
// disposition，不只匹配中文文案。
// ---------------------------------------------------------------------------

fn sg_f2b_plan_work_item(id: &str, kind: WorkItemKind) -> LifecycleWorkItemRecord {
    LifecycleWorkItemRecord {
        id: id.to_string(),
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        repository_id: "repo_0001".to_string(),
        target_repository_id: None,
        story_spec_ids: vec!["story_spec_0001".to_string()],
        design_spec_ids: vec!["design_spec_0001".to_string()],
        title: id.to_string(),
        plan_status: WorkItemPlanStatus::Confirmed,
        execution_status: WorkItemStatus::Pending,
        worktree_path: None,
        work_item_set_id: Some("work_item_set_0001".to_string()),
        source_work_item_plan_id: None,
        source_outline_id: None,
        source_draft_id: None,
        planned_implementation_context: None,
        kind,
        sequence_hint: None,
        depends_on: Vec::new(),
        exclusive_write_scopes: vec![format!("src/product/{id}.rs")],
        forbidden_write_scopes: Vec::new(),
        context_budget: WorkItemContextBudget::default(),
        verification_plan_ref: Some(format!("verification_plan_{id}")),
        require_execution_plan_confirm: false,
        execution_plan_status: WorkItemExecutionPlanStatus::NotStarted,
        completion_commit: None,
        completion_diff_summary_ref: None,
        created_at: "2026-10-13T00:00:00Z".to_string(),
        updated_at: "2026-10-13T00:00:00Z".to_string(),
    }
}

fn sg_f2b_split_plan(
    options: IssueWorkItemPlanOptions,
    work_item_ids: Vec<&str>,
    dependency_graph: Vec<(&str, &str)>,
) -> IssueWorkItemPlan {
    let work_item_ids: Vec<String> = work_item_ids.into_iter().map(str::to_string).collect();
    let verification_plan_ids = work_item_ids
        .iter()
        .map(|id| format!("verification_plan_{id}"))
        .collect();
    IssueWorkItemPlan {
        id: "work_item_set_0001".to_string(),
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        source_story_spec_ids: vec!["story_spec_0001".to_string()],
        source_design_spec_ids: vec!["design_spec_0001".to_string()],
        options,
        status: IssueWorkItemPlanStatus::Draft,
        work_item_ids,
        repository_profile_ref: Some("repository_profile_0001".to_string()),
        verification_plan_ids,
        dependency_graph: dependency_graph
            .into_iter()
            .map(|(from, to)| IssueWorkItemDependencyEdge {
                from_work_item_id: from.to_string(),
                to_work_item_id: to.to_string(),
            })
            .collect(),
        created_from_provider_run: None,
        validator_findings: Vec::new(),
        review_summary: None,
        ever_confirmed: None,
        created_at: "2026-10-13T00:00:00Z".to_string(),
        updated_at: "2026-10-13T00:00:00Z".to_string(),
    }
}

fn sg_f2b_repository_profile() -> RepositoryProfile {
    RepositoryProfile {
        id: "repository_profile_0001".to_string(),
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        repository_id: "repo_0001".to_string(),
        logical_repository_id: None,
        membership_revision: 0,
        provider_run_ref: Some("provider_run_0001".to_string()),
        languages: vec!["rust".to_string()],
        frameworks: vec!["axum".to_string()],
        package_managers: vec!["cargo".to_string()],
        test_frameworks: vec!["cargo-test".to_string()],
        build_systems: vec!["cargo".to_string()],
        verification_capabilities: vec!["unit".to_string()],
        detected_layers: vec!["backend".to_string()],
        split_recommendation: "frontend_backend".to_string(),
        confidence: RepositoryProfileConfidence::High,
        uncertainties: Vec::new(),
        created_at: "2026-10-13T00:00:00Z".to_string(),
        updated_at: "2026-10-13T00:00:00Z".to_string(),
    }
}

fn sg_f2b_verification_command(id: &str, command: &str) -> VerificationCommand {
    VerificationCommand {
        id: id.to_string(),
        label: format!("{id} label"),
        command: command.to_string(),
        cwd: ".".to_string(),
        purpose: id.to_string(),
        required: true,
        timeout_seconds: 120,
        source: VerificationCommandSource::Provider,
        safety: VerificationCommandSafety::Approved,
    }
}

fn sg_f2b_verification_plan(
    item_id: &str,
    commands: Vec<VerificationCommand>,
) -> crate::product::models::VerificationPlan {
    crate::product::models::VerificationPlan {
        id: format!("verification_plan_{item_id}"),
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        work_item_id: item_id.to_string(),
        repository_profile_ref: Some("repository_profile_0001".to_string()),
        provider_run_ref: Some("provider_run_0001".to_string()),
        scope: VerificationScope::Unit,
        commands,
        manual_checks: Vec::new(),
        required_gates: Vec::new(),
        risk_notes: Vec::new(),
        confidence: RepositoryProfileConfidence::High,
        fallback_policy: VerificationFallbackPolicy::ManualGate,
        created_at: "2026-10-13T00:00:00Z".to_string(),
        updated_at: "2026-10-13T00:00:00Z".to_string(),
    }
}

fn sg_f2b_active_sc_facts() -> StructuralAuthorizationFacts {
    StructuralAuthorizationFacts {
        authorization_id: "grant-sc-f2b".to_string(),
        authorization_kind: StructuralAuthorizationKind::ScTurn,
        parent_id: "turn-f2b".to_string(),
        source_hash: "f2b-source-hash-0000000000000000000000000000000000000000".to_string(),
        context_digest: "ctx-f2b".to_string(),
        active: true,
        anchor_waiting: false,
        launch_unknown: false,
        attempt_no: 1,
        max_attempts: 2,
    }
}

/// 把 split 预检 report 的 Error findings 包装成失败集合（warnings 是
/// advisory，不进入分类）；kind 缺失按 Unknown 投影——F2b 后所有 split
/// producer 都显式写 kind，None 只可能来自旧 JSON 记录。
fn sg_f2b_failure_set_from_report(report: &WorkItemSplitValidationReport) -> PlanningFailureSet {
    PlanningFailureSet {
        failures: report
            .findings
            .iter()
            .filter(|finding| finding.severity == WorkItemSplitFindingSeverity::Error)
            .map(|finding| PlanningFailure {
                kind: finding
                    .kind
                    .clone()
                    .unwrap_or_else(|| PlanningFailureKind::Unknown {
                        code: finding.code.clone(),
                    }),
                origin: PlanningFailureOrigin::SplitPreflight,
                stage: PlanningStage::Plan,
                diagnostic_ref: format!("diag-f2b-{}", finding.code),
                source_ref: "plan-source/f2b".to_string(),
                source_hash: "f2b-source-hash-0000000000000000000000000000000000000000".to_string(),
                evidence: PlanningFailureEvidence {
                    affected_ids: finding.work_item_ids.clone(),
                    details: BTreeMap::new(),
                },
            })
            .collect(),
    }
}

#[test]
fn sg_f2b_required_item_gaps_carry_confirmed_options_without_machine_authorization() {
    let options_on = IssueWorkItemPlanOptions {
        include_integration_tests: true,
        include_e2e_tests: true,
        force_frontend_backend_split: true,
        require_execution_plan_confirm: false,
    };
    let items = vec![sg_f2b_plan_work_item("wi_backend", WorkItemKind::Backend)];
    // 声明图引用缺失 to 端：plan 级 dependency 分支一并进入扫描。
    let plan = sg_f2b_split_plan(
        options_on,
        vec!["wi_backend"],
        vec![("wi_backend", "wi_missing")],
    );
    let profile = sg_f2b_repository_profile();
    let verification = sg_f2b_verification_plan(
        "wi_backend",
        vec![sg_f2b_verification_command(
            "verify_unit",
            "custom-verify unit",
        )],
    );

    let report = WorkItemSplitValidator::validate(&plan, &items, Some(&profile), &[verification]);

    for (code, option_key, repair_action) in [
        (
            "frontend_backend_split_required",
            "force_frontend_backend_split",
            FRONTEND_BACKEND_SPLIT_REQUIRED_REPAIR_ACTION,
        ),
        (
            "integration_work_item_required",
            "include_integration_tests",
            INTEGRATION_WORK_ITEM_REQUIRED_REPAIR_ACTION,
        ),
        (
            "e2e_work_item_required",
            "include_e2e_tests",
            E2E_WORK_ITEM_REQUIRED_REPAIR_ACTION,
        ),
    ] {
        let finding = report
            .findings
            .iter()
            .find(|finding| finding.code == code)
            .unwrap_or_else(|| panic!("{code} must be reported"));
        // 缺口 finding 携带已确认 options 的业务事实（选项键＋共享修复路径）。
        assert!(
            finding.message.contains(option_key),
            "{code} message must carry confirmed option {option_key}"
        );
        assert!(
            finding.message.contains(repair_action),
            "{code} message must carry the shared repair action"
        );
        // options×items 预检缺口是业务混合事实：折叠 Unknown，不免费 L1，
        // 机器不得替用户开/关已确认 options。
        assert_eq!(
            finding.kind.as_ref(),
            Some(&PlanningFailureKind::Unknown {
                code: code.to_string()
            })
        );
        assert_eq!(finding.severity, WorkItemSplitFindingSeverity::Error);
    }
    // 活动授权下整批仍 fail-closed：业务门不被机器越过。
    assert_eq!(
        classify_failure_set(
            &sg_f2b_failure_set_from_report(&report),
            &sg_f2b_active_sc_facts()
        ),
        PlanningRecoveryDisposition::UnknownStop
    );

    // 原 options 权限不变：选项关闭时同一 fixture 不再产生这些缺口。
    let options_off = IssueWorkItemPlanOptions {
        include_integration_tests: false,
        include_e2e_tests: false,
        force_frontend_backend_split: false,
        require_execution_plan_confirm: false,
    };
    let plan_off = sg_f2b_split_plan(options_off, vec!["wi_backend"], Vec::new());
    let report_off = WorkItemSplitValidator::validate(&plan_off, &items, Some(&profile), &[]);
    for code in [
        "frontend_backend_split_required",
        "integration_work_item_required",
        "e2e_work_item_required",
    ] {
        assert!(
            !has_code(&report_off, code),
            "{code} must depend on confirmed options"
        );
    }
}

#[test]
fn sg_f2b_required_command_authorization_is_operational_and_not_machine_granted() {
    // draft 级：required command 不在已确认可信目录——security operational
    // 分支，机器不得补权限（不产生任何 L1 kind）。
    let outline = valid_outline();
    let mut candidate = canonical_draft_candidate(&outline.work_item_outlines[0]);
    candidate.canonical_contract_candidate.verification_checks[0].command =
        Some("pnpm --dir web test".to_string());
    candidate.verification_plan.checks = candidate
        .canonical_contract_candidate
        .verification_checks
        .clone();

    let report = WorkItemDraftLocalValidator::validate(&candidate, &[], &outline);

    let untrusted = report
        .findings
        .iter()
        .find(|finding| finding.code == "untrusted_required_verification_command")
        .expect("untrusted required command must be reported");
    assert!(matches!(
        untrusted.kind.as_ref(),
        Some(PlanningFailureKind::Operational { stable_code, .. })
            if stable_code == "untrusted_required_verification_command"
    ));
    assert_eq!(
        classify_failure_set(
            &sg_f2b_failure_set_from_report(&report),
            &sg_f2b_active_sc_facts()
        ),
        PlanningRecoveryDisposition::OperationalWait
    );

    // draft 级：可信目录整体缺失（无 operational_gate blocker）同样 operational。
    let mut empty_catalog_outline = valid_outline();
    empty_catalog_outline.work_item_outlines[0]
        .trusted_verification_commands
        .clear();
    let empty_catalog_candidate =
        canonical_draft_candidate(&empty_catalog_outline.work_item_outlines[0]);
    let report = WorkItemDraftLocalValidator::validate(
        &empty_catalog_candidate,
        &[],
        &empty_catalog_outline,
    );
    let missing_catalog = report
        .findings
        .iter()
        .find(|finding| finding.code == "missing_trusted_verification_command_catalog")
        .expect("missing trusted catalog must be reported");
    assert!(matches!(
        missing_catalog.kind.as_ref(),
        Some(PlanningFailureKind::Operational { stable_code, .. })
            if stable_code == "missing_trusted_verification_command_catalog"
    ));

    // plan 级：unsafe / cwd 逃逸 / 必需命令待人工复核，全部 security
    // operational，stable code 逐分支保留。
    let items = vec![sg_f2b_plan_work_item("wi_backend", WorkItemKind::Backend)];
    let options_off = IssueWorkItemPlanOptions {
        include_integration_tests: false,
        include_e2e_tests: false,
        force_frontend_backend_split: false,
        require_execution_plan_confirm: false,
    };
    let plan = sg_f2b_split_plan(options_off, vec!["wi_backend"], Vec::new());
    let mut cwd_escape = sg_f2b_verification_command("verify_cwd", "custom-verify unit");
    cwd_escape.cwd = "../escape".to_string();
    let mut manual_review = sg_f2b_verification_command("verify_manual", "custom-verify unit");
    manual_review.safety = VerificationCommandSafety::NeedsManualReview;
    let verification = sg_f2b_verification_plan(
        "wi_backend",
        vec![
            sg_f2b_verification_command("verify_unsafe", "git reset --hard main"),
            cwd_escape,
            manual_review,
        ],
    );

    let report = WorkItemSplitValidator::validate(
        &plan,
        &items,
        Some(&sg_f2b_repository_profile()),
        &[verification],
    );

    for code in [
        "verification_command_unsafe",
        "verification_command_cwd_outside_repository",
    ] {
        let finding = report
            .findings
            .iter()
            .find(|finding| finding.code == code)
            .unwrap_or_else(|| panic!("{code} must be reported"));
        assert!(
            matches!(
                finding.kind.as_ref(),
                Some(PlanningFailureKind::Operational { stable_code, .. }) if stable_code == code
            ),
            "{code} must carry typed Operational payload with stable code"
        );
    }
    // 必需命令待人工复核是 warning：advisory 语义不变，kind 仍显式 operational。
    let manual = report
        .findings
        .iter()
        .find(|finding| finding.code == "verification_command_needs_manual_review")
        .expect("manual review warning must be reported");
    assert_eq!(manual.severity, WorkItemSplitFindingSeverity::Warning);
    assert!(matches!(
        manual.kind.as_ref(),
        Some(PlanningFailureKind::Operational { stable_code, .. })
            if stable_code == "verification_command_needs_manual_review"
    ));
    // 整批：活动授权下 operational 等待，不借 L1 启动 author。
    assert_eq!(
        classify_failure_set(
            &sg_f2b_failure_set_from_report(&report),
            &sg_f2b_active_sc_facts()
        ),
        PlanningRecoveryDisposition::OperationalWait
    );
}

#[test]
fn sg_f2b_acceptance_path_does_not_delete_acceptance_criteria_to_fake_pass() {
    let outline = valid_outline();
    let mut candidate = canonical_draft_candidate(&outline.work_item_outlines[0]);
    candidate.canonical_contract_candidate.tasks[0].done_when_refs = vec!["ac_missing".to_string()];

    let report = WorkItemDraftLocalValidator::validate(&candidate, &[], &outline);

    let finding = report
        .findings
        .iter()
        .find(|finding| finding.code == "unknown_done_when_ref")
        .expect("AC reference gap must survive the canonical->split mapping");
    assert_eq!(finding.severity, WorkItemSplitFindingSeverity::Error);
    // AC 缺口折叠 Unknown：不存在“删 AC 假通过”的 L1 机器指令。
    assert_eq!(
        finding.kind.as_ref(),
        Some(&PlanningFailureKind::Unknown {
            code: "unknown_done_when_ref".to_string()
        })
    );
    // 被引用的 AC 事实不因检出被删除（affected ids 保留引用）。
    assert!(finding.work_item_ids.contains(&"ac_missing".to_string()));
    assert!(
        report.has_errors(),
        "broken AC references must block acceptance"
    );
    assert_eq!(
        classify_failure_set(
            &sg_f2b_failure_set_from_report(&report),
            &sg_f2b_active_sc_facts()
        ),
        PlanningRecoveryDisposition::UnknownStop
    );
}

#[test]
fn sg_f2b_mixed_operational_waits_whole_batch_without_partial_authorization() {
    let outline = valid_outline();
    let mut candidate = canonical_draft_candidate(&outline.work_item_outlines[1]);
    // 唯一错误是 outline 级供需缺口：provider/consumer/contract 三元 ID 完整，
    // 活动授权 + 剩余 attempt 下才有 L1（StructuralAuthor）资格。
    candidate.canonical_contract_candidate.input_contracts[0].provider_logical_work_item_id =
        "wi_missing".to_string();

    let l1_only = WorkItemDraftLocalValidator::validate(&candidate, &[], &outline);

    let provider = l1_only
        .findings
        .iter()
        .find(|finding| finding.code == "unknown_provider_logical_work_item")
        .expect("unknown provider must be reported");
    assert_eq!(
        provider.kind.as_ref(),
        Some(&PlanningFailureKind::UnknownProvider {
            provider: "wi_missing".to_string(),
            consumer: "wi_frontend".to_string(),
            contract_id: "contract.source".to_string(),
        })
    );
    assert_eq!(
        classify_failure_set(
            &sg_f2b_failure_set_from_report(&l1_only),
            &sg_f2b_active_sc_facts()
        ),
        PlanningRecoveryDisposition::StructuralAuthor
    );

    // 混入 security operational：整批等待，不部分自动。
    candidate.canonical_contract_candidate.verification_checks[0].command =
        Some("pnpm --dir web test".to_string());
    candidate.verification_plan.checks = candidate
        .canonical_contract_candidate
        .verification_checks
        .clone();
    let mixed_operational = WorkItemDraftLocalValidator::validate(&candidate, &[], &outline);
    assert_eq!(
        classify_failure_set(
            &sg_f2b_failure_set_from_report(&mixed_operational),
            &sg_f2b_active_sc_facts()
        ),
        PlanningRecoveryDisposition::OperationalWait
    );

    // 再混入缺事实 Unknown（identity 缺口）：fail-closed 停止面优先。
    candidate.logical_work_item_id = "wi_other".to_string();
    let mixed_unknown = WorkItemDraftLocalValidator::validate(&candidate, &[], &outline);
    assert!(has_code(&mixed_unknown, "draft_logical_identity_mismatch"));
    assert_eq!(
        classify_failure_set(
            &sg_f2b_failure_set_from_report(&mixed_unknown),
            &sg_f2b_active_sc_facts()
        ),
        PlanningRecoveryDisposition::UnknownStop
    );
}

#[test]
fn sg_f2b_all_split_findings_carry_explicit_kind_with_stable_code_identity() {
    // outline 族：环 + 图投影失配 + 溯源/预算缺口。
    let mut outline = valid_outline();
    outline.work_item_outlines[0].depends_on = vec!["outline_frontend".to_string()];
    outline.work_item_outlines[0].source_story_spec_ids.clear();
    outline.work_item_outlines[0].estimated_context_tokens = None;
    let outline_report = WorkItemPlanOutlineValidator::validate(&outline);
    assert!(outline_report.has_errors());

    // draft 族：identity 缺口 + 写域缺失 + 未授权必需命令 + AC 引用缺口。
    let mut draft_outline = valid_outline();
    let mut candidate = canonical_draft_candidate(&draft_outline.work_item_outlines[0]);
    candidate.canonical_contract_candidate.identity.title = "Other title".to_string();
    candidate
        .canonical_contract_candidate
        .write_policy
        .exclusive_scopes
        .clear();
    candidate.canonical_contract_candidate.tasks[0].done_when_refs = vec!["ac_missing".to_string()];
    candidate.canonical_contract_candidate.verification_checks[0].command =
        Some("pnpm --dir web test".to_string());
    candidate.verification_plan.checks = candidate
        .canonical_contract_candidate
        .verification_checks
        .clone();
    let draft_report = WorkItemDraftLocalValidator::validate(&candidate, &[], &draft_outline);
    assert!(draft_report.has_errors());

    // plan 族：options 缺口 + 声明图缺口 + unsafe 命令。
    let options_on = IssueWorkItemPlanOptions {
        include_integration_tests: true,
        include_e2e_tests: true,
        force_frontend_backend_split: true,
        require_execution_plan_confirm: false,
    };
    let items = vec![sg_f2b_plan_work_item("wi_backend", WorkItemKind::Backend)];
    let plan = sg_f2b_split_plan(
        options_on,
        vec!["wi_backend"],
        vec![("wi_backend", "wi_missing")],
    );
    let verification = sg_f2b_verification_plan(
        "wi_backend",
        vec![sg_f2b_verification_command(
            "verify_unsafe",
            "git reset --hard main",
        )],
    );
    let plan_report = WorkItemSplitValidator::validate(
        &plan,
        &items,
        Some(&sg_f2b_repository_profile()),
        &[verification],
    );
    assert!(plan_report.has_errors());

    for (source, report) in [
        ("outline", &outline_report),
        ("draft", &draft_report),
        ("plan", &plan_report),
    ] {
        for finding in &report.findings {
            let kind = finding.kind.as_ref().unwrap_or_else(|| {
                panic!("{source} finding {} must carry explicit kind", finding.code)
            });
            // 显式 kind 的 stable code 与 finding code 单源一致（含 Unknown
            // 折影与 Operational 分支），不出现第二套身份。
            assert_eq!(
                kind.stable_code(),
                finding.code,
                "{source} finding {} kind/code identity mismatch",
                finding.code
            );
        }
    }
}
