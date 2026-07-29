use super::*;
use crate::product::models::{
    TrustedDraftVerificationCommand, WorkItemDraftCandidate, WorkItemOutline,
    WorkItemOutlineDependencyEdge, WorkItemOutlineSessionFit,
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
        outline_id: outline.outline_id.clone(),
        logical_work_item_id: outline.logical_work_item_id.clone(),
        verification_plan: crate::product::models::WorkItemDraftVerificationPlan {
            checks: contract.verification_checks.clone(),
        },
        canonical_contract_candidate: contract,
    }
}
