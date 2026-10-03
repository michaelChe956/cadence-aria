//! 编译 IR 到既有三层 work item validator 输入的确定性投影。
//!
//! 本模块只适配输入形状；membership、dependency、scope、semantics 与 verification
//! 规则全部复用 `work_item_split_validator`。

use crate::product::models::{
    IssueWorkItemDependencyEdge, IssueWorkItemPlan, IssueWorkItemPlanStatus,
    LifecycleWorkItemRecord, RepositoryProfileConfidence, VerificationCommand,
    VerificationCommandSafety, VerificationCommandSource, VerificationFallbackPolicy,
    VerificationManualCheck, VerificationPlan, VerificationScope, WorkItemContextBudget,
    WorkItemDraftCandidate, WorkItemKind, WorkItemOutline, WorkItemOutlineSessionFit,
    WorkItemPlanOutline, WorkItemPlanStatus, WorkItemSplitFinding, WorkItemSplitFindingSeverity,
    WorkItemStatus,
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

    let mut findings =
        WorkItemPlanOutlineValidator::validate_for_single_candidate(&outline).findings;
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
            WorkItemDraftLocalValidator::validate_for_single_candidate(
                draft,
                &accepted_dependencies,
                &outline,
            )
            .findings,
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
    // REQ-WSC-02 场景 13（F-56）：AC/验证计划引用的仓库内文件路径与 plan
    // 基线树交叉核对——受限提取（ac_paths，不做全文模糊匹配），缺失即
    // Error finding（preflight 族，附三条修复建议）；基线不可用不触发。
    if let Some(baseline_tree) = context.baseline_tree {
        for (work_item_id, refs) in super::ac_paths::extract_acceptance_path_refs(ir) {
            let missing: Vec<&str> = refs
                .iter()
                .map(String::as_str)
                .filter(|path| !baseline_tree.contains(*path))
                .collect();
            if !missing.is_empty() {
                findings.push(WorkItemSplitFinding {
                    severity: WorkItemSplitFindingSeverity::Error,
                    code: "acceptance_path_not_in_baseline".to_string(),
                    message: format!(
                        "work item {work_item_id} 的验收标准/验证计划引用路径 [{}] 不存在于 plan 基线树；修复动作：{}",
                        missing.join("、"),
                        super::types::ACCEPTANCE_PATH_NOT_IN_BASELINE_REPAIR_ACTION
                    ),
                    work_item_ids: vec![work_item_id],
                });
            }
        }
    }

    // C1 Task 5（REQ-C1-PLAN-01）：existing/create 意图合同校验——未声明
    //（intent_undeclared）与不能执行（intent_unexecutable）分开停等修订，
    // 作为 preflight 族 Error 保留在报告里（不清既有 finding、不硬失败）。
    if let Err(intent_diagnostics) = validate_work_item_intent_contract(ir, context) {
        for diagnostic in intent_diagnostics {
            findings.push(WorkItemSplitFinding {
                severity: WorkItemSplitFindingSeverity::Error,
                code: diagnostic.code.clone(),
                message: diagnostic.message.clone(),
                work_item_ids: vec![diagnostic.field.clone()],
            });
        }
    }
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
    // F-51（REQ-WSC-02 场景 11-13）：options×items 三族与 AC 路径×基线树核对
    // 属 preflight 族——Error 级结果保留在报告里、经既有机械 ReviewVerdict
    // 回灌修订（contract_prerevision 先例），不得把 author 轮次硬失败；结构性
    // Error（grammar/契约图等）保持 fail-closed 原样。
    if report.findings.iter().any(|finding| {
        finding.severity == WorkItemSplitFindingSeverity::Error
            && !super::types::PREFLIGHT_FINDING_CODES.contains(&finding.code.as_str())
    }) {
        return Err(report
            .findings
            .iter()
            .filter(|finding| {
                finding.severity == WorkItemSplitFindingSeverity::Error
                    && !super::types::PREFLIGHT_FINDING_CODES.contains(&finding.code.as_str())
            })
            .map(validator_diagnostic)
            .collect());
    }
    Ok(report)
}

/// C1 Task 5（REQ-C1-PLAN-01）：existing/create 意图合同校验。
///
/// - 未声明（`intent_undeclared`）：enrolled 会话（enrollment target 在场）
///   中 item 的 exclusive 写面涉及基线外新建路径但未声明 `create` 意图——
///   停等修订，不得自动补声明或扩大写范围；基线不可用（None）或非
///   enrolled（enrollment target 缺席）不触发——Manual/旧路径零回归。
/// - 不能执行（`intent_unexecutable`）：声明在场但 provider Work Item 不可
///   解析（create→本 plan items；existing→durable 既有集）、depends_on 悬空、
///   exclusive/forbidden scope 冲突、或 target 与 compile 上下文不符——
///   拒绝 compile，既有 finding 不清空。
pub fn validate_work_item_intent_contract(
    ir: &PlanCandidateIr,
    context: &PlanCandidateValidationContext<'_>,
) -> Result<(), Vec<CompilerDiagnostic>> {
    use crate::product::logical_codebase::EnrollmentTarget;
    use crate::product::work_item_contract::{WorkItemIntent, WorkItemIntentContract};

    let plan_item_ids: Vec<&str> = ir
        .items
        .iter()
        .map(|item| item.contract.identity.logical_work_item_id.as_str())
        .collect();
    let mut diagnostics = Vec::new();

    for item in &ir.items {
        let work_item_id = item.contract.identity.logical_work_item_id.clone();
        let intent: Option<&WorkItemIntentContract> = item.contract.intent_contract.as_ref();
        match intent {
            None => {
                // C1（REQ-C1-PLAN-01）：未声明判定只对 enrolled 会话生效
                //（enrollment target 在场）；基线不可用同样不触发。
                if context.enrollment_target.is_none() {
                    continue;
                }
                let Some(baseline_tree) = context.baseline_tree else {
                    continue;
                };
                let undeclared: Vec<&str> = item
                    .contract
                    .write_policy
                    .exclusive_scopes
                    .iter()
                    .map(String::as_str)
                    .filter(|scope| scope_absent_from_baseline(scope, baseline_tree))
                    .collect();
                if !undeclared.is_empty() {
                    diagnostics.push(CompilerDiagnostic {
                        code: "intent_undeclared".to_string(),
                        line: 0,
                        field: work_item_id.clone(),
                        message: format!(
                            "work item {work_item_id} 的写入面 [{}] 完全不在 plan 基线树（新建写面）但未声明 create 意图；修复动作：显式声明 Plan Intent（intent: create）或改用基线内路径，不得由系统自动补声明",
                            undeclared.join("、")
                        ),
                        repair_example: format!(
                            "### Plan Intent\n- intent: create\n- provider_work_item_id: {}\n- intent_target_kind: single_repository\n- intent_target_repo: <repo>",
                            plan_item_ids.first().copied().unwrap_or("WI-001"),
                        ),
                    });
                }
            }
            Some(intent) => {
                let provider_resolves = match intent.intent {
                    WorkItemIntent::Create => {
                        plan_item_ids.contains(&intent.provider_work_item_id.as_str())
                    }
                    WorkItemIntent::Existing => context
                        .existing_work_item_ids
                        .contains(&intent.provider_work_item_id),
                };
                if !provider_resolves {
                    diagnostics.push(unexecutable(
                        &work_item_id,
                        format!(
                            "provider Work Item {} 不能解析：create 必须指向本 plan 的 work item，existing 必须指向 durable 既有 work item",
                            intent.provider_work_item_id
                        ),
                    ));
                }
                let dangling: Vec<&str> = intent
                    .depends_on
                    .iter()
                    .map(String::as_str)
                    .filter(|dep| {
                        !plan_item_ids.contains(dep)
                            && !context.existing_work_item_ids.iter().any(|id| id == dep)
                    })
                    .collect();
                if !dangling.is_empty() {
                    diagnostics.push(unexecutable(
                        &work_item_id,
                        format!(
                            "依赖闭包悬空：[{}] 不在本 plan 也不在既有 work item 集",
                            dangling.join("、")
                        ),
                    ));
                }
                let conflict: Vec<&str> = intent
                    .exclusive_scopes
                    .iter()
                    .map(String::as_str)
                    .filter(|scope| intent.forbidden_scopes.iter().any(|f| f == scope))
                    .collect();
                if !conflict.is_empty() {
                    diagnostics.push(unexecutable(
                        &work_item_id,
                        format!(
                            "exclusive/forbidden scope 冲突：[{}] 同时出现在两表",
                            conflict.join("、")
                        ),
                    ));
                }
                // target 等值核对只在 enrollment 绑定 target 在场时执行
                //（REQ-C1-PLAN-01 的“target 不符”属绑定层事实；非 enrolled/
                // Manual 会话零回归）。logical target 的两级存在性由 lowering
                // fail-closed 保证。
                if let Some(bound) = context.enrollment_target {
                    let target_matches = match (&intent.target, bound) {
                        (EnrollmentTarget::SingleRepository { repository_id }, _) => {
                            repository_id == &item.target_repository_id
                        }
                        (
                            EnrollmentTarget::LogicalCodebase {
                                logical_repository_id,
                                ..
                            },
                            _,
                        ) => logical_repository_id.0.to_string() == item.target_repository_id,
                    };
                    if !target_matches {
                        diagnostics.push(unexecutable(
                            &work_item_id,
                            format!(
                                "intent target 与 enrollment 绑定目标不符：期望 repository {}，实际 {}",
                                intent_target_repository(&intent.target),
                                item.target_repository_id
                            ),
                        ));
                    }
                }
            }
        }
    }

    if diagnostics.is_empty() {
        Ok(())
    } else {
        diagnostics.sort_by(|left, right| {
            left.code
                .cmp(&right.code)
                .then(left.field.cmp(&right.field))
                .then(left.message.cmp(&right.message))
        });
        Err(diagnostics)
    }
}

fn unexecutable(work_item_id: &str, message: String) -> CompilerDiagnostic {
    CompilerDiagnostic {
        code: "intent_unexecutable".to_string(),
        line: 0,
        field: work_item_id.to_string(),
        message,
        repair_example: "修正 Plan Intent 的 provider/依赖/scope/target 声明后重新提交".to_string(),
    }
}

fn intent_target_repository(target: &crate::product::logical_codebase::EnrollmentTarget) -> String {
    use crate::product::logical_codebase::EnrollmentTarget;
    match target {
        EnrollmentTarget::SingleRepository { repository_id } => repository_id.clone(),
        EnrollmentTarget::LogicalCodebase {
            logical_repository_id,
            ..
        } => logical_repository_id.0.to_string(),
    }
}

/// scope（glob 如 `src/x/**`）与基线树无任何交集＝纯新建写面。
fn scope_absent_from_baseline(
    scope: &str,
    baseline_tree: &std::collections::BTreeSet<String>,
) -> bool {
    let prefix = scope.strip_suffix("/**").unwrap_or(scope);
    if prefix.is_empty() {
        return false;
    }
    !baseline_tree
        .range(prefix.to_string()..)
        .take_while(|path| path.starts_with(prefix))
        .any(|path| path == scope || path.starts_with(&format!("{prefix}/")))
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
        // F-51：候选校验消费存储 options（context 显式提供），不得从 IR items
        // 反推——反推使三族 options 预检结构性不可触发。
        options: context.plan_options.clone(),
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
