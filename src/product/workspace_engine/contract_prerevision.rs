//! F5 回灌扩展：canonical 契约机械校验前移进 SC 修订循环（3.6 矩阵 codex×重 根治）。
//!
//! 根因（codex×重 6 连败）：上游 WI 输出契约 capabilities 不逐字覆盖下游
//! require_all——该缺口此前只在 Approval 阶段的 initial plan publication compile
//! （`plan_projection.rs` 的 `prepare_initial_plan_publication`）里由
//! `validate_dependency_contract_graph` 机械比对发现；修订循环内的 reviewer
//! （flash）抓不住这类缺口，循环空转收敛后仍要等到 Approval compile 才失败落人工门。
//!
//! 本模块把同一份校验器前移到 SC author 每轮落盘时
//! （`complete_single_candidate_work_item_plan_author`，`validate_plan_candidate_ir`
//! 之后、`put_mechanical_report`/CAS 之前）：对 IR 既有 canonical 契约
//! （`PlanCandidateIr.items[].contract`）`build_dependency_contract_graph` →
//! `validate_dependency_contract_graph`，Error 级 findings 适配成机械
//! `ReviewVerdict`（verdict=Revise、review_gate=RequiresRevision），经
//! `complete_review` 既有 ingestion 路由（与 reviewer verdict 完全同一条链）：
//!
//! - F5-A 回灌：`complete_review` 注入 `latest_review_verdict`，下一轮 SC author
//!   重跑由 `single_candidate_pending_revision_verdict` 判定为修订轮，机械
//!   findings 逐条回灌进返修 prompt（`[review_findings]`，required_action 模板
//!   指明「provider X 的 contract Y 需逐字补 capability Z」）。
//! - 预算/收敛：policy 路由把机械缺口按 `ContractGap`+`Repairable` 分类进
//!   `TriggerAggregateRepair`（`repairs_used` 计数与 reviewer 返修轮同池），
//!   自动重驱 SC author；连续 2 轮同指纹（`FindingFingerprint` 与 F5-B 闸门
//!   同一 `classify_finding` 口径）由既有 `RepeatedFingerprint` 人工门兜底，
//!   不新增任何终止逻辑。
//! - Warning 级 findings 以 Suggestion+Advisory 拼进同一 verdict（可见但不参与
//!   闸门与预算）；首轮无 Error 缺口路径零变化（不产生 verdict、不改路由）。
//!
//! 确定性契约：`validate_dependency_contract_graph` 的 findings 经
//! `sorted_report` 稳定排序，本模块的 message/evidence/required_action 均为
//! 确定性 format（无时间戳/随机源），同输入同文本——保证跨轮指纹稳定，
//! 闸门语义可复现。

use crate::product::work_item_contract::{
    CanonicalWorkItemContract, ContractFindingSeverity, ContractValidationFinding,
    ContractValidationReport, DependencyContractGraph, build_dependency_contract_graph,
    validate_dependency_contract_graph,
};
use crate::product::work_item_plan_compiler::PlanCandidateIr;
use crate::product::work_item_plan_policy::{FindingClassHint, ReviewFindingCategory};
use crate::web::workspace_ws_types::{
    ReviewFinding, ReviewFindingSeverity, ReviewGate, ReviewVerdict, ReviewVerdictType,
};

/// 对 SC 本轮 IR 的 canonical 契约跑 approval 同源机械校验；存在 Error 级
/// findings 时返回机械返修 verdict，否则返回 None（首轮/干净候选零变化）。
pub(crate) fn single_candidate_contract_prerevision_verdict(
    ir: &PlanCandidateIr,
) -> Option<ReviewVerdict> {
    let contracts = ir
        .items
        .iter()
        .map(|item| item.contract.clone())
        .collect::<Vec<_>>();
    contract_prerevision_verdict_for(&contracts)
}

fn contract_prerevision_verdict_for(
    contracts: &[CanonicalWorkItemContract],
) -> Option<ReviewVerdict> {
    let graph = match build_dependency_contract_graph(contracts) {
        Ok(graph) => graph,
        Err(report) => return verdict_from_report(&report, None),
    };
    let report = validate_dependency_contract_graph(&graph);
    verdict_from_report(&report, Some(&graph))
}

/// Error 级 findings 存在时才产生机械返修 verdict；Warning 级仅在已有
/// verdict 时搭车（干净/仅告警候选零变化）。
fn verdict_from_report(
    report: &ContractValidationReport,
    graph: Option<&DependencyContractGraph>,
) -> Option<ReviewVerdict> {
    let has_error = report
        .findings
        .iter()
        .any(|finding| finding.severity == ContractFindingSeverity::Error);
    if !has_error {
        return None;
    }
    // validate_dependency_contract_graph 的 findings 已由 sorted_report 稳定
    // 排序；适配层保持同序，保证同输入同文本。
    let findings = report
        .findings
        .iter()
        .map(|finding| contract_finding_to_review_finding(finding, graph))
        .collect::<Vec<_>>();
    let error_count = findings
        .iter()
        .filter(|finding| finding.severity == ReviewFindingSeverity::MustFix)
        .count();
    let summary = format!("canonical 契约机械校验发现 {error_count} 项 Error 级缺口，候选必须返修");
    let mut comments = String::new();
    comments.push_str(&format!(
        "canonical 契约机械校验（validate_dependency_contract_graph，与 Approval compile 同源）发现 {error_count} 项 Error 级缺口：\n"
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

/// 机械 verdict 的确定性可读全文（`complete_review` 的
/// `record_review_message` 载体，进 timeline 消息流供人审阅）。
pub(crate) fn contract_prerevision_readable_output(verdict: &ReviewVerdict) -> String {
    let mut output = String::new();
    output.push_str("[contract_prerevision] 机械 canonical 契约校验未通过。\n");
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

/// `ContractValidationFinding` → `ReviewFinding` 适配：
/// - Error → MustFix + category=ContractGap + class_hint=Repairable（policy 侧
///   归入自动返修通道，与 reviewer 契约缺口同池消费预算）；
/// - Warning → Suggestion + class_hint=Advisory（advisory：不参与指纹闸门/预算）；
/// - `code` 拼进 message 前缀（机械可识别）；`logical_work_item_id`/
///   `contract_ref`/`capability_ref` 拼进 evidence；`required_action` 按 code
///   模板化（provider 从 graph 反查补全）。
fn contract_finding_to_review_finding(
    finding: &ContractValidationFinding,
    graph: Option<&DependencyContractGraph>,
) -> ReviewFinding {
    let logical_id = finding.logical_work_item_id.as_deref();
    let contract_ref = finding.contract_ref.as_deref();
    let capability_ref = finding.capability_ref.as_deref();
    let provider = graph.and_then(|graph| {
        let consumer = logical_id?;
        let contract_id = contract_ref?;
        provider_for_consumer_contract(graph, consumer, contract_id)
    });
    let (severity, category, class_hint) = match finding.severity {
        ContractFindingSeverity::Error => (
            ReviewFindingSeverity::MustFix,
            Some(ReviewFindingCategory::ContractGap),
            Some(FindingClassHint::Repairable),
        ),
        ContractFindingSeverity::Warning => (
            ReviewFindingSeverity::Suggestion,
            None,
            Some(FindingClassHint::Advisory),
        ),
    };
    ReviewFinding {
        severity,
        message: format!("{}: {}", finding.code, finding.message),
        evidence: evidence_text(logical_id, contract_ref, capability_ref),
        required_action: required_action_text(finding, provider),
        category,
        class_hint,
        contract_field: contract_field_text(finding),
    }
}

/// 定位三元组拼进 evidence（确定性键值对，机械可识别）。
fn evidence_text(
    logical_work_item_id: Option<&str>,
    contract_ref: Option<&str>,
    capability_ref: Option<&str>,
) -> String {
    let mut parts = Vec::new();
    if let Some(id) = logical_work_item_id {
        parts.push(format!("work_item={id}"));
    }
    if let Some(contract_ref) = contract_ref {
        parts.push(format!("contract={contract_ref}"));
    }
    if let Some(capability) = capability_ref {
        parts.push(format!("capability={capability}"));
    }
    parts.join(" ")
}

/// 跨轮指纹定位器：有 category 时 fingerprint=hash(category, contract_field)，
/// 缺能力缺口必须精确到 capability 级，避免同一契约的两个不同缺口共享指纹。
fn contract_field_text(finding: &ContractValidationFinding) -> Option<String> {
    match (
        finding.code.as_str(),
        &finding.contract_ref,
        &finding.capability_ref,
    ) {
        ("required_capability_missing", Some(contract_id), Some(capability)) => {
            Some(format!("{contract_id}::{capability}"))
        }
        (_, Some(contract_id), _) => Some(contract_id.clone()),
        _ => finding.code.clone().into(),
    }
}

/// required_action 按 code 模板化；provider 从 graph 反查补全（对
/// required_capability_missing/required_contract_missing，finding 的
/// logical_work_item_id 是消费方，provider 只存在于图边）。
fn required_action_text(finding: &ContractValidationFinding, provider: Option<&str>) -> String {
    let consumer = finding.logical_work_item_id.as_deref();
    let contract_ref = finding.contract_ref.as_deref();
    let capability = finding.capability_ref.as_deref();
    match finding.code.as_str() {
        "required_capability_missing" => {
            let provider = provider.unwrap_or("（provider）");
            let contract_ref = contract_ref.unwrap_or("（contract）");
            format!(
                "provider {provider} 的 contract {contract_ref} 需逐字补 capability {}（覆盖消费方 {} 的 require_all）",
                capability.unwrap_or("（capability）"),
                consumer.unwrap_or("（consumer）"),
            )
        }
        "required_contract_missing" => format!(
            "provider {} 需在输出契约中提供 {}（消费方 {} 依赖该契约）",
            provider.unwrap_or("（provider）"),
            contract_ref.unwrap_or("（contract）"),
            consumer.unwrap_or("（consumer）"),
        ),
        "unknown_provider_logical_work_item" => format!(
            "消费方引用的 provider {} 不在本 plan 内：修正 provider_logical_work_item_id 或补齐对应 work item（契约 {}）",
            consumer.unwrap_or("（provider）"),
            contract_ref.unwrap_or("（contract）"),
        ),
        "duplicate_dependency_contract_edge" => format!(
            "同一依赖边只声明一次契约 {}：合并重复声明",
            contract_ref.unwrap_or("（contract）"),
        ),
        "dependency_cycle" => {
            format!(
                "拆除依赖环（{}）：调整 depends_on 打破循环",
                finding.message
            )
        }
        "unconsumed_required_handoff" => format!(
            "终端 work item 的 handoff 契约 {} 无消费方：从 provided_contract_refs 移除该引用，或补齐下游消费方",
            contract_ref.unwrap_or("（contract）"),
        ),
        "duplicate_logical_work_item_identity" => {
            "logical_work_item_identity 重复：合并或重命名重复的 work item".to_string()
        }
        _ => format!("按机械校验结论修复契约声明：{}", finding.message),
    }
}

/// 反查 (consumer, contract_id) 所在依赖边的 provider。
fn provider_for_consumer_contract<'a>(
    graph: &'a DependencyContractGraph,
    consumer: &str,
    contract_id: &str,
) -> Option<&'a str> {
    graph
        .edges
        .iter()
        .find(|edge| {
            edge.to == consumer
                && edge
                    .required_contracts
                    .iter()
                    .any(|required| required.contract_id == contract_id)
        })
        .map(|edge| edge.from.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compiled_gap_ir() -> PlanCandidateIr {
        // rep4 基线：终端 WI-003 的 handoff 提供行按 campaign 口径清空
        // （unconsumed_required_handoff 是 Error 级，混入会干扰缺口断言），
        // 再给 WI-002 的 require_all 塞一个 WI-001 未提供的 capability。
        let source = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/product/work_item_plan_compiler/fixtures/work-item-plan-rep4.md"
        ))
        .replace(
            "- provided_contract_refs: contract.levels-integration",
            "- provided_contract_refs: []",
        )
        .replace(
            "- required_capabilities: api.levels.read\n",
            "- required_capabilities: api.levels.read, api.levels.write\n",
        );
        crate::product::work_item_plan_compiler::compile_work_item_plan(
            &source,
            &crate::product::work_item_plan_compiler::WorkItemPlanSourceContext {
                target_repository_id: "repo_fixture".to_string(),
            },
        )
        .expect("compile gap fixture")
    }

    fn compiled_clean_ir() -> PlanCandidateIr {
        let source = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/product/work_item_plan_compiler/fixtures/work-item-plan-rep4.md"
        ))
        .replace(
            "- provided_contract_refs: contract.levels-integration",
            "- provided_contract_refs: []",
        );
        crate::product::work_item_plan_compiler::compile_work_item_plan(
            &source,
            &crate::product::work_item_plan_compiler::WorkItemPlanSourceContext {
                target_repository_id: "repo_fixture".to_string(),
            },
        )
        .expect("compile clean fixture")
    }

    /// E3-1 适配层映射：Error→MustFix+ContractGap+Repairable，code 进 message
    /// 前缀，定位三元组进 evidence，required_action 模板含 provider/contract/
    /// capability 逐字补齐指令。
    #[test]
    fn contract_prerevision_verdict_maps_error_findings_into_review_verdict() {
        let verdict = single_candidate_contract_prerevision_verdict(&compiled_gap_ir())
            .expect("capability gap must produce a mechanical revise verdict");
        assert_eq!(verdict.verdict, ReviewVerdictType::Revise);
        assert_eq!(verdict.review_gate, ReviewGate::RequiresRevision);
        assert!(verdict.work_item_plan_review.is_none());
        assert!(verdict.structured_output_diagnostic.is_none());

        let finding = verdict
            .findings
            .iter()
            .find(|finding| finding.message.contains("required_capability_missing"))
            .expect("capability gap finding must be present");
        assert_eq!(finding.severity, ReviewFindingSeverity::MustFix);
        assert_eq!(finding.category, Some(ReviewFindingCategory::ContractGap));
        assert_eq!(finding.class_hint, Some(FindingClassHint::Repairable));
        assert!(
            finding.message.starts_with("required_capability_missing: "),
            "mechanical code must prefix the message: {}",
            finding.message
        );
        assert!(
            finding.message.contains("api.levels.write"),
            "message must name the missing capability: {}",
            finding.message
        );
        for locator in ["WI-002", "contract.levels-api", "api.levels.write"] {
            assert!(
                finding.evidence.contains(locator),
                "evidence must carry locator {locator}: {}",
                finding.evidence
            );
        }
        assert!(
            finding
                .required_action
                .contains("provider WI-001 的 contract contract.levels-api"),
            "required_action must follow the provider/contract template: {}",
            finding.required_action
        );
        assert!(
            finding.required_action.contains("api.levels.write"),
            "required_action must name the capability to add: {}",
            finding.required_action
        );
        assert!(
            finding
                .contract_field
                .as_deref()
                .is_some_and(|field| field.contains("contract.levels-api")
                    && field.contains("api.levels.write")),
            "contract_field must locate the capability gap: {:?}",
            finding.contract_field
        );
    }

    /// E3-1 干净候选零变化：无 Error findings 时不产生任何 verdict。
    #[test]
    fn contract_prerevision_verdict_stays_none_for_clean_contracts() {
        assert!(
            single_candidate_contract_prerevision_verdict(&compiled_clean_ir()).is_none(),
            "clean candidate must not produce a mechanical verdict"
        );
    }

    /// E3-1/勘察 C：Warning 级 findings 以 Suggestion+Advisory 拼进同一 verdict，
    /// 不单独触发 verdict、不参与闸门口径。
    #[test]
    fn contract_prerevision_warnings_ride_along_as_advisory_suggestions() {
        let graph = build_dependency_contract_graph(
            &compiled_clean_ir()
                .items
                .iter()
                .map(|item| item.contract.clone())
                .collect::<Vec<_>>(),
        )
        .expect("clean fixture builds a graph");
        let warning = ContractValidationFinding {
            code: "process_evidence_acceptance_criterion".to_string(),
            severity: ContractFindingSeverity::Warning,
            logical_work_item_id: Some("WI-001".to_string()),
            contract_ref: Some("AC-001".to_string()),
            capability_ref: None,
            message: "acceptance criterion AC-001 must describe an observable result".to_string(),
        };
        let finding = contract_finding_to_review_finding(&warning, Some(&graph));
        assert_eq!(finding.severity, ReviewFindingSeverity::Suggestion);
        assert_eq!(finding.class_hint, Some(FindingClassHint::Advisory));
        assert_eq!(finding.category, None);
        assert!(
            finding
                .message
                .starts_with("process_evidence_acceptance_criterion: "),
            "warning code must prefix the message: {}",
            finding.message
        );

        // Warning-only 报告不产生 verdict（零变化门槛）。
        let warning_only = ContractValidationReport {
            findings: vec![warning],
        };
        assert!(verdict_from_report(&warning_only, Some(&graph)).is_none());
    }

    /// E3-4/勘察 D：同输入同文本（findings 排序稳定、模板确定性）——
    /// 机械 message 确定性是跨轮指纹稳定与闸门可复现的前提。
    #[test]
    fn contract_prerevision_verdict_is_deterministic_for_identical_input() {
        let first =
            single_candidate_contract_prerevision_verdict(&compiled_gap_ir()).expect("gap verdict");
        let second = single_candidate_contract_prerevision_verdict(&compiled_gap_ir())
            .expect("gap verdict rebuilt");
        assert_eq!(
            first, second,
            "identical IR must yield byte-identical mechanical verdicts"
        );
        assert_eq!(
            contract_prerevision_readable_output(&first),
            contract_prerevision_readable_output(&second)
        );
    }

    /// E3-1：build 阶段 Err（duplicate identity）同样转机械 verdict，不丢缺口。
    #[test]
    fn contract_prerevision_covers_duplicate_identity_build_failure() {
        let mut contracts = compiled_clean_ir()
            .items
            .iter()
            .map(|item| item.contract.clone())
            .collect::<Vec<_>>();
        let duplicated = contracts[0].clone();
        contracts.push(duplicated);
        let verdict = contract_prerevision_verdict_for(&contracts)
            .expect("duplicate identity must produce a mechanical revise verdict");
        assert!(verdict.findings.iter().any(|finding| {
            finding
                .message
                .contains("duplicate_logical_work_item_identity")
        }));
    }
}
