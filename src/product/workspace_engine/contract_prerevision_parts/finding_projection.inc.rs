// REQ-WSC-12：契约预修订 finding 的 wire 投影（纯移动自
// `contract_prerevision.rs`，控制该文件行数门禁）。
fn contract_finding_to_review_finding(
    finding: &ContractValidationFinding,
    graph: Option<&DependencyContractGraph>,
    machine_repair: Option<MachineRepairDto>,
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
        // SG7（§0.6.3）：machine_repair 只由服务端已证 L1 的同 source/
        // context 显式配对产生（`machine_repair_dto_for_instruction`），
        // 不从 message/class_hint/severity 猜资格；无配对恒 None。
        machine_repair,
        owner_question: None,
        owner_options: None,
        // REQ-WSC-12：disposition 留 None（语义＝actionable）；Warning 已是
        // Suggestion + class_hint=Advisory，本就不阻断。
        disposition: None,
        target_work_item_id: None,
        finding_key: None,
    }
}
