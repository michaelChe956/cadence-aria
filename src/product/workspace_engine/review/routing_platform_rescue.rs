//! A3 / REQ-WSC-25（owner 2026-10-31 第 3 条，controller 批复）：平台合成
//! 编译失败 verdict 在 Verification 相位下的 origin 并入与返修资格复原。
//!
//! 拆文件原因：large_file_guard 对 `review/routing.rs`（登记 1391）与
//! `review/routing_scope.rs`（1200 硬线）不得增长——本组纯函数连同从
//! routing.rs 平移出的两个既有自由助手独立成文（story_terminate.rs 先例）。
//!
//! 现场 2adc5e41 物证链（为什么需要这两个函数）：初评通过的候选再次进评审
//! 时 cycle 已计 `initial_count=1` → 相位判 Verification；平台合成的编译失败
//! finding 不在 invocation 的 original_fingerprints（初评通过 → 空集）→ 撞上
//! new-findings 守卫；即使守卫放行，`evaluate` 的 `Repairable` 也只在 Initial
//! 相位产出 → `HumanRequired{RepairBudgetExhausted}` → AutoIfValid 下
//! StopNeedsHuman——durable `auto_rewrite.phase=running` 的承诺 0.67 秒内直落
//! 人工门、零 author 重写。F5-A 机械返修不踩坑只是因为它在初评前跑
//! （initial_count=0 → Initial）。

use super::policy_routing::RoutingAction;
use crate::product::work_item_plan_policy::classify::classify_finding;
use crate::product::work_item_plan_policy::{
    EvaluationDecision, FatalReason, FindingClass, FindingFingerprint, HumanReason, PlanOutcome,
    PolicyDiagnostic, ReviewCycleState, ReviewFindingCategory, ReviewInvocationScope, ReviewPhase,
    RunBudgets,
};
use crate::product::workspace_engine::compile_failure_surface::PLATFORM_COMPILE_FAILURE_FINDING_PREFIX;
use crate::web::workspace_ws_types::{ReviewVerdict, WorkItemPlanReviewVerdict};

/// A3 / REQ-WSC-25（owner 2026-10-31 第 3 条，controller 批复）：平台合成
/// 的编译失败 verdict（`compile_failure_surface::auto_rewrite_verdict` 单点
/// 生产）在 Verification 相位下的 origin 并入。
///
/// 红线（不得放宽）：
/// - 判定单源 = finding `message` 的平台前缀
///   [`PLATFORM_COMPILE_FAILURE_FINDING_PREFIX`]，按 finding 逐条生效——
///   不得改成按 kind/severity 或按轮次豁免；
/// - **只**把前缀 finding 的分类指纹并入 Verification scope 的
///   `original_fingerprints`，**不改** `evaluate` 的 new-findings 守卫本身；
/// - 模型产出的 finding（无前缀）不并入：同一轮里它们照常被守卫拦下
///   （`HumanRequired{VerificationNewFindings}` → 人工门）。
///
/// 语义诚实性：编译失败轮就是该返修链的 origin（此前 cycle 的初评集合为
/// 空是因为初评通过、零 findings），把它自己的指纹并入「本轮闭合边界」与
/// Initial 相位把 actionable 指纹写入 cycle `original_fingerprints`
/// （evaluate.rs `review_count_delta`）是同一语义，不是绕过守卫。
pub(super) fn merge_platform_compile_failure_origin(
    invocation: ReviewInvocationScope,
    phase: ReviewPhase,
    verdict: &ReviewVerdict,
) -> ReviewInvocationScope {
    if phase != ReviewPhase::Verification {
        return invocation;
    }
    let ReviewInvocationScope::Verification {
        original_fingerprints,
        repaired_revision_id,
        mechanical_report_ref,
        cycle_anchor_revision_id,
        ..
    } = invocation
    else {
        return invocation;
    };
    let mut original_fingerprints = original_fingerprints;
    for finding in &verdict.findings {
        if !finding.is_actionable()
            || !finding
                .message
                .starts_with(PLATFORM_COMPILE_FAILURE_FINDING_PREFIX)
        {
            continue;
        }
        let classified = classify_finding(verdict.verdict.clone(), finding);
        original_fingerprints.insert(classified.fingerprint);
    }
    ReviewInvocationScope::verification(
        original_fingerprints,
        repaired_revision_id,
        mechanical_report_ref,
        cycle_anchor_revision_id,
    )
}

/// A3 / REQ-WSC-25（owner 2026-10-31 第 3 条，controller 批复）：纯平台
/// 编译失败轮的返修资格复原。
///
/// 复原条件**全部**结构性成立才生效（controller 红线）：
/// - 仅 Verification 相位、且 outcome 是 `RepairBudgetExhausted`（守卫触发
///   的 `VerificationNewFindings` / `RepeatedFingerprint` 一律不复原——
///   模型产出的新 finding 与同环重复仍必落人工门）；
/// - verdict 的**全部** actionable finding 都带平台前缀（单源标识；混入
///   任何模型 finding → 不复原，守卫已在 evaluate 内把它们拦下）；
/// - 本 cycle 的返修预算确实还有（`repairs_used < max_repairs`）——预算
///   耗尽时 `RepairBudgetExhausted` 是真实结论，必须交人工门。
pub(super) fn rescue_platform_compile_failure_repair(
    decision: EvaluationDecision,
    phase: ReviewPhase,
    cycle: &ReviewCycleState,
    verdict: &ReviewVerdict,
) -> EvaluationDecision {
    if phase != ReviewPhase::Verification
        || !matches!(
            decision.outcome,
            PlanOutcome::HumanRequired {
                reason: HumanReason::RepairBudgetExhausted,
                ..
            }
        )
    {
        return decision;
    }
    let actionable = verdict
        .findings
        .iter()
        .filter(|finding| finding.is_actionable())
        .collect::<Vec<_>>();
    if actionable.is_empty()
        || !actionable.iter().all(|finding| {
            finding
                .message
                .starts_with(PLATFORM_COMPILE_FAILURE_FINDING_PREFIX)
        })
    {
        return decision;
    }
    if cycle.repairs_used >= RunBudgets::default().max_repairs {
        return decision;
    }
    let PlanOutcome::HumanRequired { findings, .. } = decision.outcome else {
        unreachable!("guarded by the matches! above");
    };
    let repairable = findings
        .into_iter()
        .filter(|finding| {
            matches!(
                finding.class,
                FindingClass::MechanicalError | FindingClass::Repairable
            )
        })
        .collect::<Vec<_>>();
    if repairable.is_empty() {
        // 守卫放行但没有可修 finding：维持人工门（不虚构可修性）。
        return EvaluationDecision {
            outcome: PlanOutcome::HumanRequired {
                findings: Vec::new(),
                repeated_fingerprints: Vec::new(),
                reason: HumanReason::RepairBudgetExhausted,
            },
            history_delta: decision.history_delta,
        };
    }
    EvaluationDecision {
        outcome: PlanOutcome::Repairable {
            findings: repairable,
        },
        history_delta: decision.history_delta,
    }
}

// ============ 以下两个自由助手自 routing.rs 原样平移（large_file_guard 拆分） ============

pub(super) fn legacy_work_item_plan_repair_finding(
    verdict: &ReviewVerdict,
) -> Option<crate::product::work_item_plan_policy::ClassifiedFinding> {
    let review = verdict.work_item_plan_review.as_ref()?;
    let (kind, contract_field, required_action) = match review.verdict {
        WorkItemPlanReviewVerdict::ReviseBatch => (
            "revise_batch",
            "work_item_plan.batch",
            "rewrite the reviewed Work Item batch",
        ),
        WorkItemPlanReviewVerdict::PlanReopenRequired => (
            "plan_reopen_required",
            "work_item_plan.outline",
            "regenerate the Work Item plan outline",
        ),
        _ => return None,
    };
    let message = format!("{kind}: {}", verdict.summary);
    let (fingerprint, identity_unstable) = FindingFingerprint::identity_for_finding(
        Some(ReviewFindingCategory::Other),
        FindingClass::Repairable,
        &message,
        Some(contract_field),
    );
    Some(crate::product::work_item_plan_policy::ClassifiedFinding {
        class: FindingClass::Repairable,
        fingerprint,
        identity_unstable,
        category: Some(ReviewFindingCategory::Other),
        severity: "must_fix".to_string(),
        message,
        evidence: (!verdict.comments.trim().is_empty()).then(|| verdict.comments.clone()),
        required_action: Some(required_action.to_string()),
        contract_field: Some(contract_field.to_string()),
    })
}

pub(super) fn classification_error_action(
    error: crate::product::work_item_plan_policy::ClassificationError,
) -> RoutingAction {
    let (reason, code, message) = match error {
        crate::product::work_item_plan_policy::ClassificationError::UnknownCategory(raw) => (
            FatalReason::UnknownCategory,
            "unknown_finding_category",
            format!("unknown finding category: {raw}"),
        ),
        crate::product::work_item_plan_policy::ClassificationError::UnknownClassHint(raw) => (
            FatalReason::UnknownClassHint,
            "unknown_class_hint",
            format!("unknown class hint: {raw}"),
        ),
        crate::product::work_item_plan_policy::ClassificationError::InvalidFinding(details) => (
            FatalReason::InvalidStructuredOutput,
            "invalid_finding_field",
            details,
        ),
        crate::product::work_item_plan_policy::ClassificationError::VerificationScopeViolation(
            details,
        ) => (
            FatalReason::ProtocolViolation,
            "verification_scope_violation",
            details,
        ),
    };
    RoutingAction::AbortFatal {
        reason,
        diagnostics: vec![PolicyDiagnostic {
            code: code.to_string(),
            message,
            field: Some("findings".to_string()),
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::web::workspace_ws_types::ReviewVerdictType;
    use crate::web::workspace_ws_types::review::{ReviewFinding, ReviewFindingSeverity};

    /// 红线：origin 并入只认平台前缀 finding（模型 finding 不并入）。
    #[test]
    fn platform_compile_failure_origin_merges_only_prefixed_actionable_findings() {
        let ir_ref = "project/project_0001/issue/issue_0001/plan/plan_0001/plan_candidate_ir/ir-340fe12ab34c5678";
        let scope = ReviewInvocationScope::verification(
            std::collections::BTreeSet::new(),
            ir_ref,
            "report-001",
            Some(ir_ref.to_string()),
        );
        let platform_finding = finding(&format!(
            "candidate_compile_failure:final_compile_prepare_failed:{}（候选依赖图仍需消除该环）",
            "a".repeat(64)
        ));
        let model_finding = finding("模型复评发现的新问题");
        let verdict = verdict_of(vec![platform_finding.clone(), model_finding]);

        let merged =
            merge_platform_compile_failure_origin(scope, ReviewPhase::Verification, &verdict);
        let ReviewInvocationScope::Verification {
            original_fingerprints,
            ..
        } = &merged
        else {
            panic!("merged scope must stay Verification");
        };
        let platform_fingerprint =
            classify_finding(verdict.verdict.clone(), &platform_finding).fingerprint;
        assert!(
            original_fingerprints.contains(&platform_fingerprint),
            "前缀 finding 的指纹必须并入 origin：{original_fingerprints:?}"
        );
        assert_eq!(
            original_fingerprints.len(),
            1,
            "模型 finding（无前缀）不得并入：{original_fingerprints:?}"
        );
        assert!(
            merged.validate_digest().is_ok(),
            "重建后的 scope digest 必须有效"
        );
    }

    /// 红线：Initial 相位（F5-A 机械返修常规形态）不并入、原样返回。
    #[test]
    fn platform_compile_failure_origin_keeps_other_phases_untouched() {
        let scope = ReviewInvocationScope::initial("ir-001");
        let verdict = verdict_of(vec![finding(&format!(
            "candidate_compile_failure:final_compile_prepare_failed:{}",
            "b".repeat(64)
        ))]);
        assert_eq!(
            merge_platform_compile_failure_origin(scope.clone(), ReviewPhase::Initial, &verdict),
            scope
        );
    }

    fn finding(message: &str) -> ReviewFinding {
        ReviewFinding {
            severity: ReviewFindingSeverity::MustFix,
            message: message.to_string(),
            evidence: "final compile rejected the candidate".to_string(),
            required_action: "按重写要点修正".to_string(),
            category: Some(ReviewFindingCategory::ContractGap),
            class_hint: Some(crate::product::work_item_plan_policy::FindingClassHint::Repairable),
            contract_field: None,
            machine_repair: None,
            owner_question: None,
            owner_options: None,
            disposition: None,
            target_work_item_id: None,
            finding_key: None,
        }
    }

    fn verdict_of(findings: Vec<ReviewFinding>) -> ReviewVerdict {
        ReviewVerdict {
            verdict: ReviewVerdictType::Revise,
            comments: String::new(),
            summary: String::new(),
            findings,
            review_gate: crate::web::workspace_ws_types::ReviewGate::RequiresRevision,
            work_item_plan_review: None,
            structured_output_diagnostic: None,
        }
    }
}
