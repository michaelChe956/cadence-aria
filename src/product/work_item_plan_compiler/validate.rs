//! 编译 IR 到既有三层 work item validator 输入的确定性投影。
//!
//! 本模块只适配输入形状；membership、dependency、scope、semantics 与 verification
//! 规则全部复用 `work_item_split_validator`。

use crate::product::models::{
    IssueWorkItemDependencyEdge, IssueWorkItemPlan, IssueWorkItemPlanOptions,
    IssueWorkItemPlanStatus, LifecycleWorkItemRecord, RepositoryProfileConfidence,
    VerificationCommand, VerificationCommandSafety, VerificationCommandSource,
    VerificationFallbackPolicy, VerificationManualCheck, VerificationPlan, VerificationScope,
    WorkItemContextBudget, WorkItemDraftCandidate, WorkItemKind, WorkItemOutline,
    WorkItemOutlineSessionFit, WorkItemPlanOutline, WorkItemPlanStatus, WorkItemSplitFinding,
    WorkItemSplitFindingSeverity, WorkItemStatus,
};
use crate::product::work_item_split_validator::{
    WorkItemDraftLocalValidator, WorkItemPlanOutlineValidator, WorkItemSplitValidator,
};

use super::{
    lower::PlanCandidateIr,
    types::{CompilerDiagnostic, PlanCandidateMechanicalReport, PlanCandidateValidationContext},
};

pub fn validate_plan_candidate_ir(
    ir: &PlanCandidateIr,
    context: &PlanCandidateValidationContext<'_>,
) -> Result<PlanCandidateMechanicalReport, Vec<CompilerDiagnostic>> {
    let outline = project_outline(ir, context);
    let drafts = project_drafts(ir, context);
    let plan = project_issue_work_item_plan(ir, context);
    let work_items = project_lifecycle_work_items(ir, context);
    let verification_plans = project_verification_plans(ir, context);

    let mut findings = WorkItemPlanOutlineValidator::validate(&outline).findings;
    for (index, draft) in drafts.iter().enumerate() {
        let accepted_dependencies = drafts
            .iter()
            .enumerate()
            .filter_map(|(candidate_index, candidate)| {
                (candidate_index != index
                    && ir.items[index]
                        .contract
                        .depends_on
                        .contains(&candidate.logical_work_item_id))
                .then_some(candidate.clone())
            })
            .collect::<Vec<_>>();
        findings.extend(
            WorkItemDraftLocalValidator::validate(draft, &accepted_dependencies, &outline).findings,
        );
    }
    findings.extend(
        WorkItemSplitValidator::validate(
            &plan,
            &work_items,
            context.repository_profile,
            &verification_plans,
        )
        .findings,
    );
    findings.sort_by(|left, right| {
        left.severity
            .as_str()
            .cmp(right.severity.as_str())
            .then(left.code.cmp(&right.code))
            .then(left.work_item_ids.cmp(&right.work_item_ids))
            .then(left.message.cmp(&right.message))
    });

    let report = PlanCandidateMechanicalReport {
        source_revision_hash: ir.source_revision_hash.clone(),
        compiler_version: ir.compiler_version.clone(),
        findings,
    };
    if report.has_errors() {
        return Err(report
            .findings
            .iter()
            .filter(|finding| finding.severity == WorkItemSplitFindingSeverity::Error)
            .map(validator_diagnostic)
            .collect());
    }
    Ok(report)
}

fn validator_diagnostic(finding: &WorkItemSplitFinding) -> CompilerDiagnostic {
    CompilerDiagnostic {
        code: finding.code.clone(),
        line: 0,
        field: finding.work_item_ids.join(","),
        message: finding.message.clone(),
        repair_example: format!("修复 validator finding `{}`。", finding.code),
    }
}

fn project_outline(
    ir: &PlanCandidateIr,
    context: &PlanCandidateValidationContext<'_>,
) -> WorkItemPlanOutline {
    let target_repository_id = context
        .repository_profile
        .and_then(|profile| profile.logical_repository_id);
    let mut outline = WorkItemPlanOutline {
        id: context.plan_id.to_string(),
        project_id: context.project_id.to_string(),
        issue_id: context.issue_id.to_string(),
        source_story_spec_ids: context.source_story_spec_ids.to_vec(),
        source_design_spec_ids: context.source_design_spec_ids.to_vec(),
        strategy_summary: "由编译后的 PlanCandidateIr 投影".to_string(),
        work_item_outlines: ir
            .items
            .iter()
            .map(|item| WorkItemOutline {
                target_repository_id,
                outline_id: item.contract.identity.logical_work_item_id.clone(),
                logical_work_item_id: item.contract.identity.logical_work_item_id.clone(),
                title: item.contract.identity.title.clone(),
                kind: work_item_kind(&item.contract.identity.kind),
                goal: item.contract.goal.summary.clone(),
                scope: item.contract.write_policy.exclusive_scopes.clone(),
                non_goals: item.contract.non_goals.clone(),
                estimated_context_tokens: Some(30_000),
                session_fit: Some(WorkItemOutlineSessionFit::FitsSingleAgentSession),
                source_story_spec_ids: context.source_story_spec_ids.to_vec(),
                source_design_spec_ids: context.source_design_spec_ids.to_vec(),
                exclusive_write_scopes: item.contract.write_policy.exclusive_scopes.clone(),
                forbidden_write_scopes: item.contract.write_policy.forbidden_scopes.clone(),
                depends_on: item.contract.depends_on.clone(),
                verification_intent: item
                    .verification_plan
                    .checks
                    .iter()
                    .filter_map(|check| {
                        check
                            .command
                            .as_deref()
                            .or(check.manual_instruction.as_deref())
                            .map(str::to_string)
                    })
                    .collect(),
                trusted_verification_commands: item.trusted_commands.clone(),
                handoff_notes: item.contract.handoff_contract.required_fields.join(", "),
            })
            .collect(),
        dependency_graph: Vec::new(),
        risks: Vec::new(),
        handoff_strategy: "由 canonical handoff schema 投影".to_string(),
        status: "draft".to_string(),
    };
    outline.normalize_dependency_graph_from_depends_on();
    outline
}

fn project_drafts(
    ir: &PlanCandidateIr,
    context: &PlanCandidateValidationContext<'_>,
) -> Vec<WorkItemDraftCandidate> {
    let target_repository_id = context
        .repository_profile
        .and_then(|profile| profile.logical_repository_id);
    ir.items
        .iter()
        .map(|item| WorkItemDraftCandidate {
            target_repository_id,
            outline_id: item.contract.identity.logical_work_item_id.clone(),
            logical_work_item_id: item.contract.identity.logical_work_item_id.clone(),
            canonical_contract_candidate: item.contract.clone(),
            verification_plan: item.verification_plan.clone(),
        })
        .collect()
}

fn project_issue_work_item_plan(
    ir: &PlanCandidateIr,
    context: &PlanCandidateValidationContext<'_>,
) -> IssueWorkItemPlan {
    let work_item_ids = ir
        .items
        .iter()
        .map(|item| item.contract.identity.logical_work_item_id.clone())
        .collect::<Vec<_>>();
    let verification_plan_ids = ir
        .items
        .iter()
        .map(|item| verification_plan_id(&item.contract.identity.logical_work_item_id))
        .collect::<Vec<_>>();
    let dependency_graph = ir
        .items
        .iter()
        .flat_map(|item| {
            item.contract
                .depends_on
                .iter()
                .map(|dependency| IssueWorkItemDependencyEdge {
                    from_work_item_id: dependency.clone(),
                    to_work_item_id: item.contract.identity.logical_work_item_id.clone(),
                })
        })
        .collect();

    IssueWorkItemPlan {
        id: context.plan_id.to_string(),
        project_id: context.project_id.to_string(),
        issue_id: context.issue_id.to_string(),
        source_story_spec_ids: context.source_story_spec_ids.to_vec(),
        source_design_spec_ids: context.source_design_spec_ids.to_vec(),
        options: IssueWorkItemPlanOptions {
            include_integration_tests: ir.items.iter().any(|item| {
                work_item_kind(&item.contract.identity.kind) == WorkItemKind::Integration
            }),
            include_e2e_tests: ir
                .items
                .iter()
                .any(|item| work_item_kind(&item.contract.identity.kind) == WorkItemKind::E2e),
            force_frontend_backend_split: ir
                .items
                .iter()
                .any(|item| work_item_kind(&item.contract.identity.kind) == WorkItemKind::Backend)
                && ir.items.iter().any(|item| {
                    work_item_kind(&item.contract.identity.kind) == WorkItemKind::Frontend
                }),
            require_execution_plan_confirm: false,
        },
        status: IssueWorkItemPlanStatus::Draft,
        work_item_ids,
        repository_profile_ref: context.repository_profile.map(|profile| profile.id.clone()),
        verification_plan_ids,
        dependency_graph,
        created_from_provider_run: None,
        validator_findings: Vec::new(),
        review_summary: None,
        created_at: context.now.to_string(),
        updated_at: context.now.to_string(),
    }
}

fn project_lifecycle_work_items(
    ir: &PlanCandidateIr,
    context: &PlanCandidateValidationContext<'_>,
) -> Vec<LifecycleWorkItemRecord> {
    let target_repository_id = context
        .repository_profile
        .and_then(|profile| profile.logical_repository_id);
    ir.items
        .iter()
        .enumerate()
        .map(|(index, item)| LifecycleWorkItemRecord {
            id: item.contract.identity.logical_work_item_id.clone(),
            project_id: context.project_id.to_string(),
            issue_id: context.issue_id.to_string(),
            repository_id: item.target_repository_id.clone(),
            target_repository_id,
            story_spec_ids: context.source_story_spec_ids.to_vec(),
            design_spec_ids: context.source_design_spec_ids.to_vec(),
            title: item.contract.identity.title.clone(),
            plan_status: WorkItemPlanStatus::Draft,
            execution_status: WorkItemStatus::Pending,
            worktree_path: None,
            work_item_set_id: Some(context.plan_id.to_string()),
            source_work_item_plan_id: Some(context.plan_id.to_string()),
            source_outline_id: Some(item.contract.identity.logical_work_item_id.clone()),
            source_draft_id: None,
            planned_implementation_context: None,
            kind: work_item_kind(&item.contract.identity.kind),
            sequence_hint: Some(index as u32 + 1),
            depends_on: item.contract.depends_on.clone(),
            exclusive_write_scopes: item.contract.write_policy.exclusive_scopes.clone(),
            forbidden_write_scopes: item.contract.write_policy.forbidden_scopes.clone(),
            context_budget: WorkItemContextBudget::default(),
            verification_plan_ref: Some(verification_plan_id(
                &item.contract.identity.logical_work_item_id,
            )),
            require_execution_plan_confirm: false,
            execution_plan_status: Default::default(),
            completion_commit: None,
            completion_diff_summary_ref: None,
            created_at: context.now.to_string(),
            updated_at: context.now.to_string(),
        })
        .collect()
}

fn project_verification_plans(
    ir: &PlanCandidateIr,
    context: &PlanCandidateValidationContext<'_>,
) -> Vec<VerificationPlan> {
    let profile_ref = context.repository_profile.map(|profile| profile.id.clone());
    let confidence = context
        .repository_profile
        .map(|profile| profile.confidence.clone())
        .unwrap_or(RepositoryProfileConfidence::High);
    ir.items
        .iter()
        .map(|item| {
            let mut commands = Vec::new();
            let mut manual_checks = Vec::new();
            let mut required_gates = Vec::new();
            for check in &item.verification_plan.checks {
                if let Some(command) = &check.command {
                    let command_id = format!("{}:command", check.check_id);
                    let trusted = item
                        .trusted_commands
                        .iter()
                        .find(|entry| entry.command == *command);
                    commands.push(VerificationCommand {
                        id: command_id.clone(),
                        label: check.check_id.clone(),
                        command: command.clone(),
                        cwd: trusted.map(|entry| entry.cwd.clone()).unwrap_or_default(),
                        purpose: trusted
                            .map(|entry| entry.purpose.clone())
                            .unwrap_or_else(|| check.check_id.clone()),
                        required: check.required,
                        timeout_seconds: 300,
                        source: VerificationCommandSource::Provider,
                        safety: VerificationCommandSafety::Approved,
                    });
                    if check.required {
                        required_gates.push(command_id);
                    }
                }
                if let Some(instructions) = &check.manual_instruction {
                    let manual_id = format!("{}:manual", check.check_id);
                    manual_checks.push(VerificationManualCheck {
                        id: manual_id.clone(),
                        label: check.check_id.clone(),
                        instructions: instructions.clone(),
                        required: check.required,
                    });
                    if check.required {
                        required_gates.push(manual_id);
                    }
                }
            }

            VerificationPlan {
                id: verification_plan_id(&item.contract.identity.logical_work_item_id),
                project_id: context.project_id.to_string(),
                issue_id: context.issue_id.to_string(),
                work_item_id: item.contract.identity.logical_work_item_id.clone(),
                repository_profile_ref: profile_ref.clone(),
                provider_run_ref: None,
                scope: verification_scope(&item.contract.identity.kind),
                commands,
                manual_checks,
                required_gates,
                risk_notes: Vec::new(),
                confidence: confidence.clone(),
                fallback_policy: VerificationFallbackPolicy::ManualGate,
                created_at: context.now.to_string(),
                updated_at: context.now.to_string(),
            }
        })
        .collect()
}

fn verification_plan_id(logical_work_item_id: &str) -> String {
    format!("verification_plan_{logical_work_item_id}")
}

fn work_item_kind(value: &str) -> WorkItemKind {
    match value {
        "backend" => WorkItemKind::Backend,
        "frontend" => WorkItemKind::Frontend,
        "integration" => WorkItemKind::Integration,
        "e2e" => WorkItemKind::E2e,
        "docs" => WorkItemKind::Docs,
        "infra" => WorkItemKind::Infra,
        _ => WorkItemKind::Other,
    }
}

fn verification_scope(kind: &str) -> VerificationScope {
    match work_item_kind(kind) {
        WorkItemKind::Integration => VerificationScope::Integration,
        WorkItemKind::E2e => VerificationScope::E2e,
        WorkItemKind::Backend | WorkItemKind::Frontend => VerificationScope::Unit,
        WorkItemKind::Docs | WorkItemKind::Infra | WorkItemKind::Other => VerificationScope::Custom,
    }
}
