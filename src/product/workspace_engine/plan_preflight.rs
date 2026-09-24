//! C1 Task 1/2：SC 候选 preflight 族的机械 ReviewVerdict 适配（F-51/F-56）。
//!
//! preflight 族 = options×items 三族缺口 + AC 引用路径×基线树缺口。此类缺口
//! 是「用户创建意图 × 候选实际交付」的确定性事实，与 canonical 契约缺口
//! （`contract_prerevision`）同族：SHALL 在 generate/evaluate 期经既有机械
//! ReviewVerdict 回灌修订循环（`complete_review` ingestion → F5-A 修订轮回灌
//! → policy TriggerAggregateRepair 重驱 author），SHALL NOT 把 author 轮次硬
//! 失败，更不得延迟至 Approval/Final Compile 才首次出现。
//!
//! finding 消息由 `WorkItemSplitValidator::validate` 三族语义产出（禁复制
//! 规则）；`required_action`/`contract_field` 从共享常量与 code 确定性映射，
//! 同输入同文本——保证跨轮指纹稳定（REQ-TOP-04 结构化 identity 消费同一
//! `classify_finding` 口径）。skipped-risk Warning 仅在已有 Error verdict 时
//! 搭车为 Suggestion+Advisory（可见不参与闸门/预算），与 contract_prerevision
//! 行为一致。

use crate::product::models::WorkItemSplitFinding;
use crate::product::work_item_plan_compiler::PlanCandidateMechanicalReport;
use crate::product::work_item_split_validator::{
    E2E_WORK_ITEM_REQUIRED_REPAIR_ACTION, FRONTEND_BACKEND_SPLIT_REQUIRED_REPAIR_ACTION,
    INTEGRATION_WORK_ITEM_REQUIRED_REPAIR_ACTION,
};
use crate::web::workspace_ws_types::{
    ReviewFinding, ReviewFindingSeverity, ReviewGate, ReviewVerdict, ReviewVerdictType,
};
/// 机械 preflight verdict 的确定性可读全文（`complete_review` 的
/// `record_review_message` 载体，进 timeline 消息流供人审阅）。
pub(crate) fn preflight_readable_output(verdict: &ReviewVerdict) -> String {
    let mut output = String::new();
    output.push_str("[plan_preflight] 机械 options/基线路径预检未通过。\n");
    for finding in &verdict.findings {
        output.push_str(&format!(
            "- [{}] {}\n",
            match finding.severity {
                ReviewFindingSeverity::Blocking => "blocking",
                ReviewFindingSeverity::MustFix => "must_fix",
                ReviewFindingSeverity::Suggestion => "suggestion",
            },
            finding.message
        ));
    }
    output
}

/// 从候选机械报告中收集 preflight 族 findings 并适配为机械返修 verdict；
/// 无 Error 级 preflight 缺口时返回 None（干净/仅告警候选零变化）。
pub(crate) fn preflight_review_verdict(
    report: &PlanCandidateMechanicalReport,
) -> Option<ReviewVerdict> {
    let preflight: Vec<&WorkItemSplitFinding> = report
        .findings
        .iter()
        .filter(|finding| {
            crate::product::work_item_plan_compiler::PREFLIGHT_FINDING_CODES
                .contains(&finding.code.as_str())
        })
        .collect();
    let has_error = preflight.iter().any(|finding| {
        finding.severity == crate::product::models::WorkItemSplitFindingSeverity::Error
    });
    if !has_error {
        return None;
    }
    let findings = preflight
        .iter()
        .map(|finding| preflight_finding_to_review_finding(finding))
        .collect::<Vec<_>>();
    let error_count = findings
        .iter()
        .filter(|finding| finding.severity == ReviewFindingSeverity::MustFix)
        .count();
    let summary = format!(
        "计划 preflight 机械校验发现 {error_count} 项 Error 级缺口（options×items / AC 路径×基线树），候选必须返修"
    );
    let mut comments = String::new();
    comments.push_str(&format!(
        "计划 preflight 机械校验（存储 plan options 与基线树 × 候选交付交叉核对）发现 {error_count} 项 Error 级缺口：\n"
    ));
    for finding in &findings {
        if finding.severity == ReviewFindingSeverity::MustFix {
            comments.push_str(&format!("- {}\n", finding.message));
        }
    }
    comments
        .push_str("以上缺口均为机械比对结论（非 reviewer 判断）；逐条按 required_action 修复。");
    Some(ReviewVerdict {
        verdict: ReviewVerdictType::Revise,
        comments,
        summary,
        findings,
        review_gate: ReviewGate::RequiresRevision,
        work_item_plan_review: None,
        structured_output_diagnostic: None,
    })
}

/// 合并两路机械返修 verdict（canonical 契约缺口 × preflight 族）共用一次
/// `complete_review` ingestion：findings 顺序拼接（契约缺口在前，preflight 在
/// 后），verdict/gate 保持 Revise/RequiresRevision，comments 确定性拼接。
pub(crate) fn merge_revision_verdicts(
    contract: ReviewVerdict,
    preflight: ReviewVerdict,
) -> ReviewVerdict {
    let mut findings = contract.findings;
    findings.extend(preflight.findings);
    let comments = format!("{}\n{}", contract.comments, preflight.comments);
    let summary = format!("{}；{}", contract.summary, preflight.summary);
    ReviewVerdict {
        verdict: ReviewVerdictType::Revise,
        comments,
        summary,
        findings,
        review_gate: ReviewGate::RequiresRevision,
        work_item_plan_review: None,
        structured_output_diagnostic: None,
    }
}

/// preflight code → 确定性身份定位器（跨轮指纹稳定：category+contract_field）。
fn preflight_contract_field(code: &str) -> String {
    match code {
        "integration_work_item_required" => "plan_options.include_integration_tests".to_string(),
        "e2e_work_item_required" => "plan_options.include_e2e_tests".to_string(),
        "frontend_backend_split_required" => {
            "plan_options.force_frontend_backend_split".to_string()
        }
        other => format!("plan_preflight.{other}"),
    }
}

/// preflight code → 与校验器消息同源的修复路径（REQ-WSC-06 口径一致纪律）。
fn preflight_required_action(code: &str) -> String {
    match code {
        "integration_work_item_required" => INTEGRATION_WORK_ITEM_REQUIRED_REPAIR_ACTION.to_string(),
        "e2e_work_item_required" => E2E_WORK_ITEM_REQUIRED_REPAIR_ACTION.to_string(),
        "frontend_backend_split_required" => {
            FRONTEND_BACKEND_SPLIT_REQUIRED_REPAIR_ACTION.to_string()
        }
        other => format!("按 finding 消息修复 {other}"),
    }
}

/// `WorkItemSplitFinding` → `ReviewFinding` 适配：
/// - Error → MustFix + category=ContractGap + class_hint=Repairable（policy 侧
///   归入自动返修通道，与 reviewer 契约缺口同池消费预算）；
/// - Warning → Suggestion + class_hint=Advisory（仅在已有 verdict 时搭车）。
fn preflight_finding_to_review_finding(finding: &WorkItemSplitFinding) -> ReviewFinding {
    use crate::product::models::WorkItemSplitFindingSeverity;
    let is_error = finding.severity == WorkItemSplitFindingSeverity::Error;
    ReviewFinding {
        severity: if is_error {
            ReviewFindingSeverity::MustFix
        } else {
            ReviewFindingSeverity::Suggestion
        },
        message: format!("[{}] {}", finding.code, finding.message),
        evidence: format!(
            "plan preflight mechanical finding；work_items: [{}]",
            finding.work_item_ids.join(", ")
        ),
        required_action: if is_error {
            preflight_required_action(&finding.code)
        } else {
            String::new()
        },
        category: Some(if is_error {
            crate::product::work_item_plan_policy::ReviewFindingCategory::ContractGap
        } else {
            crate::product::work_item_plan_policy::ReviewFindingCategory::Completeness
        }),
        class_hint: Some(if is_error {
            crate::product::work_item_plan_policy::FindingClassHint::Repairable
        } else {
            crate::product::work_item_plan_policy::FindingClassHint::Advisory
        }),
        contract_field: Some(preflight_contract_field(&finding.code)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report_with(code: &str, message: &str) -> PlanCandidateMechanicalReport {
        PlanCandidateMechanicalReport {
            source_revision_hash: "a".repeat(64),
            compiler_version: "work_item_plan_compiler/v1".to_string(),
            findings: vec![WorkItemSplitFinding {
                code: code.to_string(),
                message: message.to_string(),
                work_item_ids: Vec::new(),
                severity: crate::product::models::WorkItemSplitFindingSeverity::Error,
            }],
        }
    }

    #[test]
    fn clean_report_produces_no_verdict() {
        let report = PlanCandidateMechanicalReport {
            source_revision_hash: "a".repeat(64),
            compiler_version: "work_item_plan_compiler/v1".to_string(),
            findings: Vec::new(),
        };
        assert!(preflight_review_verdict(&report).is_none());
    }

    #[test]
    fn options_gap_adapts_to_mechanical_revision_verdict() {
        let report = report_with(
            "integration_work_item_required",
            "include_integration_tests is enabled but the plan does not contain an integration work item；修复动作：新增一个 kind=integration 的 Work Item",
        );
        let verdict = preflight_review_verdict(&report).expect("gap must produce verdict");
        assert_eq!(verdict.verdict, ReviewVerdictType::Revise);
        assert_eq!(verdict.review_gate, ReviewGate::RequiresRevision);
        let finding = &verdict.findings[0];
        assert_eq!(finding.severity, ReviewFindingSeverity::MustFix);
        assert_eq!(
            finding.contract_field.as_deref(),
            Some("plan_options.include_integration_tests")
        );
        assert!(
            finding.required_action.contains("kind=integration"),
            "required_action 与校验器消息同源：{}",
            finding.required_action
        );
        assert!(preflight_readable_output(&verdict).contains("[plan_preflight]"));
    }

    #[test]
    fn non_preflight_errors_do_not_produce_verdict() {
        let report = report_with("traceability_refs_required", "unrelated structural error");
        assert!(preflight_review_verdict(&report).is_none());
    }
}
