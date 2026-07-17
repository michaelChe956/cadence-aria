use super::canonical_contract_fixture;
use crate::product::work_item_contract::{
    ContractFindingSeverity, ContractValidationFinding, ContractValidationReport,
    validate_canonical_contract,
};

fn finding<'a>(report: &'a ContractValidationReport, code: &str) -> &'a ContractValidationFinding {
    report
        .findings
        .iter()
        .find(|finding| finding.code == code)
        .unwrap_or_else(|| panic!("expected finding {code}, got {:?}", report.findings))
}

#[test]
fn canonical_work_item_validation_accepts_a_complete_contract() {
    let report = validate_canonical_contract(&canonical_contract_fixture("WI-01"));

    assert!(report.findings.is_empty());
    assert!(report.is_valid());
}

#[test]
fn canonical_work_item_validation_report_roundtrips_through_serde() {
    let report = ContractValidationReport {
        findings: vec![ContractValidationFinding {
            code: "diagnostic".to_string(),
            severity: ContractFindingSeverity::Warning,
            logical_work_item_id: Some("WI-01".to_string()),
            contract_ref: Some("contract.workflow".to_string()),
            capability_ref: Some("workflow_explicit_completion".to_string()),
            message: "diagnostic warning".to_string(),
        }],
    };

    let value = serde_json::to_value(&report).unwrap();
    assert_eq!(
        serde_json::from_value::<ContractValidationReport>(value).unwrap(),
        report
    );
    assert!(report.is_valid());
}

#[test]
fn canonical_work_item_validation_report_is_invalid_when_it_contains_an_error() {
    let report = ContractValidationReport {
        findings: vec![ContractValidationFinding {
            code: "invalid".to_string(),
            severity: ContractFindingSeverity::Error,
            logical_work_item_id: None,
            contract_ref: None,
            capability_ref: None,
            message: "invalid contract".to_string(),
        }],
    };

    assert!(!report.is_valid());
}

#[test]
fn canonical_work_item_validation_rejects_duplicate_task_ids() {
    let mut contract = canonical_contract_fixture("WI-01");
    contract.tasks.push(contract.tasks[0].clone());

    let report = validate_canonical_contract(&contract);
    let finding = finding(&report, "duplicate_task_id");

    assert_eq!(finding.logical_work_item_id.as_deref(), Some("WI-01"));
    assert_eq!(finding.contract_ref.as_deref(), Some("task_1"));
    assert_eq!(finding.severity, ContractFindingSeverity::Error);
    assert!(!finding.message.is_empty());
}

#[test]
fn canonical_work_item_validation_rejects_duplicate_acceptance_criterion_ids() {
    let mut contract = canonical_contract_fixture("WI-01");
    contract
        .acceptance_criteria
        .push(contract.acceptance_criteria[0].clone());

    let report = validate_canonical_contract(&contract);
    let finding = finding(&report, "duplicate_acceptance_criterion_id");

    assert_eq!(finding.logical_work_item_id.as_deref(), Some("WI-01"));
    assert_eq!(finding.contract_ref.as_deref(), Some("AC-001"));
}

#[test]
fn canonical_work_item_validation_rejects_duplicate_verification_check_ids() {
    let mut contract = canonical_contract_fixture("WI-01");
    contract
        .verification_checks
        .push(contract.verification_checks[0].clone());

    let report = validate_canonical_contract(&contract);
    let finding = finding(&report, "duplicate_verification_check_id");

    assert_eq!(finding.logical_work_item_id.as_deref(), Some("WI-01"));
    assert_eq!(finding.contract_ref.as_deref(), Some("check_canonical"));
}

#[test]
fn canonical_work_item_validation_rejects_unknown_done_when_refs() {
    let mut contract = canonical_contract_fixture("WI-01");
    contract.tasks[0].done_when_refs = vec!["AC-404".to_string()];

    let report = validate_canonical_contract(&contract);
    let finding = finding(&report, "unknown_done_when_ref");

    assert_eq!(finding.logical_work_item_id.as_deref(), Some("WI-01"));
    assert_eq!(finding.contract_ref.as_deref(), Some("AC-404"));
    assert!(finding.message.contains("task_1"));
}

#[test]
fn canonical_work_item_validation_rejects_unknown_requirement_refs() {
    let mut contract = canonical_contract_fixture("WI-01");
    contract.tasks[0].requirement_refs = vec!["REQ-404".to_string()];

    let report = validate_canonical_contract(&contract);
    let finding = finding(&report, "unknown_requirement_ref");

    assert_eq!(finding.logical_work_item_id.as_deref(), Some("WI-01"));
    assert_eq!(finding.contract_ref.as_deref(), Some("REQ-404"));
    assert!(finding.message.contains("task_1"));
}

#[test]
fn canonical_work_item_validation_rejects_unknown_reviewer_check_refs() {
    let mut contract = canonical_contract_fixture("WI-01");
    contract
        .handoff_contract
        .reviewer_check_refs
        .push("AC-404".to_string());

    let report = validate_canonical_contract(&contract);
    let finding = finding(&report, "unknown_reviewer_check_ref");

    assert_eq!(finding.logical_work_item_id.as_deref(), Some("WI-01"));
    assert_eq!(finding.contract_ref.as_deref(), Some("AC-404"));
}

#[test]
fn canonical_work_item_validation_requires_reviewer_check_for_every_acceptance_criterion() {
    let mut contract = canonical_contract_fixture("WI-01");
    contract.handoff_contract.reviewer_check_refs.clear();

    let report = validate_canonical_contract(&contract);
    let finding = finding(&report, "acceptance_criterion_without_reviewer_check");

    assert_eq!(finding.logical_work_item_id.as_deref(), Some("WI-01"));
    assert_eq!(finding.contract_ref.as_deref(), Some("AC-001"));
}

#[test]
fn canonical_work_item_validation_rejects_empty_required_write_scope() {
    let mut contract = canonical_contract_fixture("WI-01");
    contract
        .write_policy
        .exclusive_scopes
        .push("   ".to_string());

    let report = validate_canonical_contract(&contract);
    let finding = finding(&report, "empty_required_write_scope");

    assert_eq!(finding.logical_work_item_id.as_deref(), Some("WI-01"));
    assert_eq!(finding.contract_ref.as_deref(), Some("   "));
}

#[test]
fn canonical_work_item_validation_rejects_overlapping_write_scopes() {
    let mut contract = canonical_contract_fixture("WI-01");
    contract
        .write_policy
        .forbidden_scopes
        .push("src/product/work_item_contract".to_string());

    let report = validate_canonical_contract(&contract);
    let finding = finding(&report, "overlapping_exclusive_and_forbidden_scope");

    assert_eq!(finding.logical_work_item_id.as_deref(), Some("WI-01"));
    assert_eq!(
        finding.contract_ref.as_deref(),
        Some("src/product/work_item_contract")
    );
}

#[test]
fn canonical_work_item_validation_rejects_required_check_without_command() {
    let mut contract = canonical_contract_fixture("WI-01");
    contract.verification_checks[0].command = None;
    contract.verification_checks[0].manual_instruction = Some("Inspect output".to_string());

    let report = validate_canonical_contract(&contract);
    let finding = finding(&report, "missing_required_verification_command");

    assert_eq!(finding.logical_work_item_id.as_deref(), Some("WI-01"));
    assert_eq!(finding.contract_ref.as_deref(), Some("check_canonical"));
}

#[test]
fn canonical_work_item_validation_rejects_required_check_with_blank_command() {
    let mut contract = canonical_contract_fixture("WI-01");
    contract.verification_checks[0].command = Some("  ".to_string());

    let report = validate_canonical_contract(&contract);

    assert_eq!(
        finding(&report, "missing_required_verification_command")
            .contract_ref
            .as_deref(),
        Some("check_canonical")
    );
}

#[test]
fn canonical_work_item_validation_rejects_blocker_without_target_contract() {
    let mut contract = canonical_contract_fixture("WI-01");
    contract.blocker_rules[0].target_contract_refs.clear();

    let report = validate_canonical_contract(&contract);
    let finding = finding(&report, "stage_blocker_without_target_contract");

    assert_eq!(finding.logical_work_item_id.as_deref(), Some("WI-01"));
    assert_eq!(finding.contract_ref, None);
    assert!(finding.message.contains("contract_invalid"));
}

#[test]
fn canonical_work_item_validation_rejects_blocker_with_unknown_target_contract() {
    let mut contract = canonical_contract_fixture("WI-01");
    contract.blocker_rules[0].target_contract_refs = vec!["contract.unknown".to_string()];

    let report = validate_canonical_contract(&contract);
    let finding = finding(&report, "stage_blocker_without_target_contract");

    assert_eq!(finding.logical_work_item_id.as_deref(), Some("WI-01"));
    assert_eq!(finding.contract_ref.as_deref(), Some("contract.unknown"));
}

#[test]
fn canonical_work_item_validation_orders_findings_deterministically() {
    let mut contract = canonical_contract_fixture("WI-01");
    contract.tasks.push(contract.tasks[0].clone());
    contract.tasks[0].done_when_refs = vec!["AC-404".to_string()];
    contract.tasks[0].requirement_refs = vec!["REQ-404".to_string()];
    contract.verification_checks[0].command = None;

    let first = validate_canonical_contract(&contract);
    let second = validate_canonical_contract(&contract);
    let codes = first
        .findings
        .iter()
        .map(|finding| finding.code.as_str())
        .collect::<Vec<_>>();

    assert_eq!(first, second);
    assert!(codes.windows(2).all(|pair| pair[0] <= pair[1]));
}
