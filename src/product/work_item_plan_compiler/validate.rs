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
                // D5 事后（REQ-WSC-06 场景「越界路径回灌最近候选」）：把
                // 基线树中最近可用的现存路径回灌进 message——同父目录优先，
                // 无则同名（`baseline_candidates` 口径）；候选逐条 ∈ 基线树
                // （平台不得臆造路径），同一候选只回灌一次；无候选时如实说明。
                let mut candidates: Vec<&str> = Vec::new();
                for path in &missing {
                    for candidate in super::baseline_candidates::nearest_baseline_candidates(
                        path,
                        baseline_tree,
                    ) {
                        if !candidates.contains(&candidate) {
                            candidates.push(candidate);
                        }
                    }
                }
                let candidate_hint = if candidates.is_empty() {
                    "；基线树中无同目录或同名可用路径".to_string()
                } else {
                    format!("；基线树中同目录最近可用路径：{}", candidates.join("、"))
                };
                findings.push(WorkItemSplitFinding {
                    severity: WorkItemSplitFindingSeverity::Error,
                    code: "acceptance_path_not_in_baseline".to_string(),
                    message: format!(
                        "work item {work_item_id} 的验收标准/验证计划引用路径 [{}] 不存在于 plan 基线树；修复动作：{}{}",
                        missing.join("、"),
                        super::types::ACCEPTANCE_PATH_NOT_IN_BASELINE_REPAIR_ACTION,
                        candidate_hint
                    ),
                    work_item_ids: vec![work_item_id],
                    // F1.2：preflight 族 finding 暂无 typed payload（None=
                    // Unknown 不自动；显式 kind 工厂归 F2b）。
                    kind: None,
                });
            }
        }
    }

    // bug45/t25（P1 计划自洽）：本工作项的检查目标落在本项 forbidden 写域
    // 内 → Error finding（preflight 族，回灌修订）。自洽性核对不依赖基线
    // 树可用性（域冲突是 plan 内部矛盾）。
    findings.extend(super::check_target_forbidden::check_target_forbidden_findings(ir));
    // REQ-WSC-10（写范围×被授权仓归属，owner 2026-10-30 真机现场）：多仓
    //（LC 聚合根）会话的 exclusive 写面必须落在本 WI 被授权成员仓的
    // checkout 内——聚合根非成员区（根不是 git 仓库）与其他成员 checkout
    // 本项执行期均无法写入。只判归属不判存在（新建文件不误伤）；前缀表
    // 缺席（单仓/Manual）不触发（零回归）；表在场而被授权仓不在表内 →
    // fail-closed（不以跳过吞错）。属 preflight 族（Error 留在报告里经
    // 既有修订轮回灌，不硬失败 author 轮次）。
    if let Some(member_checkout_prefixes) = context.member_checkout_prefixes {
        findings.extend(
            super::write_scope_bound::write_scope_bound_repository_findings(
                ir,
                member_checkout_prefixes,
            ),
        );
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
                // F1.2：preflight 族 finding 暂无 typed payload（None=
                // Unknown 不自动；显式 kind 工厂归 F2b）。
                kind: None,
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
                        // F1.2：line=0 诊断不伪行、未 typed 不猜 kind。
                        kind: None,
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
                // 批 2 R1（REQ-WIGA-01）：集合 intent 不能作为单 WI 归属
                //（lowering 同判；此处兜底拒绝，A/B 各 WI 仍落各物理仓）。
                if matches!(
                    &intent.target,
                    crate::product::logical_codebase::EnrollmentTarget::LogicalCodebaseMembers { .. }
                ) {
                    diagnostics.push(unexecutable(
                        &work_item_id,
                        "intent target 是集合（logical_codebase_members）；单个 work item \
                         的 intent 必须声明恰一成员"
                            .to_string(),
                    ));
                }
                // target 等值核对只在 enrollment 绑定 target 在场时执行
                //（REQ-C1-PLAN-01 的“target 不符”属绑定层事实；非 enrolled/
                // Manual 会话零回归）。logical target 的两级存在性由 lowering
                // fail-closed 保证。
                //
                // 批 2 R1：bound 为集合时不再等值比较，而是授权上界核对——
                // 同 LC、成员 ∈ 授权集；IR 物理 ID 用成员归属映射核对；
                // physical intent 反查唯一授权成员。映射缺失 fail-closed
                // 出诊断，不以 None 吞错。
                if let Some(bound) = context.enrollment_target
                    && let Some(message) =
                        intent_target_bound_violation(&intent.target, bound, context, item)
                {
                    diagnostics.push(unexecutable(&work_item_id, message));
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
        // F1.2：line=0 诊断不伪行、未 typed 不猜 kind。
        kind: None,
    }
}

/// 批 2 R1（REQ-WIGA-01）：intent target 与 enrollment 绑定的核对。
/// - 旧单成员/单仓绑定：保持既有等值语义（零回归）；
/// - 集合绑定：授权上界——同 LC、成员 ∈ 授权集，IR 物理 ID 经成员归属
///   映射核对；physical intent 反查唯一授权成员。返回 `Some(message)`
///   表示违规（诊断）；映射缺失也按违规 fail-closed，不以 None 吞错。
fn intent_target_bound_violation(
    intent_target: &crate::product::logical_codebase::EnrollmentTarget,
    bound: &crate::product::logical_codebase::EnrollmentTarget,
    context: &PlanCandidateValidationContext<'_>,
    item: &super::PlanCandidateItemIr,
) -> Option<String> {
    use crate::product::logical_codebase::{EnrollmentTarget, LogicalRepositoryId};

    let member_targets = || {
        context.member_targets.ok_or_else(|| {
            "集合绑定缺少成员归属映射（member_targets）；无法核对 intent 的成员物理仓".to_string()
        })
    };
    let EnrollmentTarget::LogicalCodebaseMembers {
        logical_codebase_id: bound_codebase,
        logical_repository_ids: authorized,
    } = bound
    else {
        // 旧绑定：既有等值语义（logical UUID 与 IR 物理 ID 的历史比较口径
        // 保持不变，零回归）。
        let target_matches = match (intent_target, bound) {
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
            (EnrollmentTarget::LogicalCodebaseMembers { .. }, _) => {
                return Some(SET_INTENT_MESSAGE.to_string());
            }
        };
        return (!target_matches).then(|| {
            format!(
                "intent target 与 enrollment 绑定目标不符：期望 repository {}，实际 {}",
                intent_target_repository(intent_target),
                item.target_repository_id
            )
        });
    };
    match intent_target {
        EnrollmentTarget::LogicalCodebase {
            logical_codebase_id,
            logical_repository_id,
        } => {
            if logical_codebase_id != bound_codebase {
                return Some(format!(
                    "intent target 逻辑代码库 {logical_codebase_id} 不在授权逻辑代码库 \
                     {bound_codebase} 内；单 WI 归属必须落在授权集合的成员上"
                ));
            }
            if !authorized.contains(logical_repository_id) {
                return Some(format!(
                    "intent target 成员 {logical_repository_id:?} 不在 enrollment 授权集合内"
                ));
            }
            match member_targets() {
                Err(message) => Some(message),
                Ok(targets) => match targets.physical_by_logical.get(logical_repository_id) {
                    Some(physical) if physical == &item.target_repository_id => None,
                    Some(physical) => Some(format!(
                        "intent target 成员 {logical_repository_id:?} 的物理仓 {physical} 与 \
                         IR target {} 不符",
                        item.target_repository_id
                    )),
                    None => Some(format!(
                        "intent target 成员 {logical_repository_id:?} 不在成员归属映射内"
                    )),
                },
            }
        }
        EnrollmentTarget::SingleRepository { repository_id } => {
            if repository_id != &item.target_repository_id {
                return Some(format!(
                    "intent target 与 enrollment 绑定目标不符：期望 repository {}，实际 {}",
                    repository_id, item.target_repository_id
                ));
            }
            // physical intent 反查：该物理仓必须映射自授权集合中的恰一成员。
            match member_targets() {
                Err(message) => Some(message),
                Ok(targets) => {
                    let matched: Vec<LogicalRepositoryId> = authorized
                        .iter()
                        .filter(|logical| {
                            targets
                                .physical_by_logical
                                .get(logical)
                                .is_some_and(|physical| physical == repository_id)
                        })
                        .copied()
                        .collect();
                    match matched.as_slice() {
                        [_] => None,
                        [] => Some(format!(
                            "intent target 物理仓 {repository_id} 不映射自授权集合的任何成员"
                        )),
                        _ => Some(format!(
                            "intent target 物理仓 {repository_id} 映射自授权集合的多个成员"
                        )),
                    }
                }
            }
        }
        EnrollmentTarget::LogicalCodebaseMembers { .. } => Some(SET_INTENT_MESSAGE.to_string()),
    }
}

/// 批 2 R1：集合 intent 的稳定违规文案（入口早拒与绑定核对兜底共用）。
const SET_INTENT_MESSAGE: &str =
    "intent target 是集合（logical_codebase_members）；单个 work item 的 intent 必须声明恰一成员";

fn intent_target_repository(target: &crate::product::logical_codebase::EnrollmentTarget) -> String {
    use crate::product::logical_codebase::EnrollmentTarget;
    match target {
        EnrollmentTarget::SingleRepository { repository_id } => repository_id.clone(),
        EnrollmentTarget::LogicalCodebase {
            logical_repository_id,
            ..
        } => logical_repository_id.0.to_string(),
        EnrollmentTarget::LogicalCodebaseMembers {
            logical_repository_ids,
            ..
        } => format!(
            "logical_codebase_members({} 个成员)",
            logical_repository_ids.len()
        ),
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
        // F1.2：validator 投影无 source 行事实（line=0），不伪行、不猜 kind；
        // finding 自身的可选 kind 由 split 侧显式携带（F2b）。
        kind: None,
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
        // D1.4：候选投影内存 record；root 持久化由 create_issue_work_item_plan 决定。
        ever_confirmed: None,
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
