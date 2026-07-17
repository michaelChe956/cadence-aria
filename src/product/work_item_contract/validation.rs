use std::{cmp::Ordering, collections::BTreeSet};

use serde::{Deserialize, Serialize};

use super::{BlockerRoute, CanonicalWorkItemContract};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractValidationReport {
    pub findings: Vec<ContractValidationFinding>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractValidationFinding {
    pub code: String,
    pub severity: ContractFindingSeverity,
    pub logical_work_item_id: Option<String>,
    pub contract_ref: Option<String>,
    pub capability_ref: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContractFindingSeverity {
    Warning,
    Error,
}

impl ContractValidationReport {
    pub fn is_valid(&self) -> bool {
        self.findings
            .iter()
            .all(|finding| finding.severity != ContractFindingSeverity::Error)
    }
}

pub fn validate_canonical_contract(
    contract: &CanonicalWorkItemContract,
) -> ContractValidationReport {
    let logical_work_item_id = &contract.identity.logical_work_item_id;
    let mut findings = Vec::new();

    report_blank_ids(
        std::iter::once(logical_work_item_id.as_str()),
        "blank_logical_work_item_id",
        "logical work item id",
        logical_work_item_id,
        &mut findings,
    );
    report_blank_ids(
        contract
            .input_contracts
            .iter()
            .map(|input| input.contract_id.as_str()),
        "blank_input_contract_id",
        "input contract id",
        logical_work_item_id,
        &mut findings,
    );
    report_blank_ids(
        contract
            .input_contracts
            .iter()
            .map(|input| input.provider_logical_work_item_id.as_str()),
        "blank_provider_logical_work_item_id",
        "provider logical work item id",
        logical_work_item_id,
        &mut findings,
    );
    report_blank_ids(
        contract
            .output_contracts
            .iter()
            .map(|output| output.contract_id.as_str()),
        "blank_output_contract_id",
        "output contract id",
        logical_work_item_id,
        &mut findings,
    );
    report_blank_ids(
        contract.tasks.iter().map(|task| task.task_id.as_str()),
        "blank_task_id",
        "task id",
        logical_work_item_id,
        &mut findings,
    );
    report_blank_ids(
        contract
            .acceptance_criteria
            .iter()
            .map(|criterion| criterion.criterion_id.as_str()),
        "blank_acceptance_criterion_id",
        "acceptance criterion id",
        logical_work_item_id,
        &mut findings,
    );
    report_blank_ids(
        contract
            .verification_checks
            .iter()
            .map(|check| check.check_id.as_str()),
        "blank_verification_check_id",
        "verification check id",
        logical_work_item_id,
        &mut findings,
    );
    report_blank_ids(
        contract
            .blocker_rules
            .iter()
            .map(|blocker| blocker.reason_code.as_str()),
        "blank_blocker_reason_code",
        "blocker reason code",
        logical_work_item_id,
        &mut findings,
    );
    report_blank_ids(
        contract
            .handoff_contract
            .required_fields
            .iter()
            .map(String::as_str),
        "blank_handoff_required_field",
        "handoff required field",
        logical_work_item_id,
        &mut findings,
    );
    report_blank_ids(
        contract
            .handoff_contract
            .provided_contract_refs
            .iter()
            .map(String::as_str),
        "blank_handoff_provided_contract_ref",
        "handoff provided contract ref",
        logical_work_item_id,
        &mut findings,
    );
    report_blank_ids(
        contract
            .handoff_contract
            .reviewer_check_refs
            .iter()
            .map(String::as_str),
        "blank_handoff_reviewer_check_ref",
        "handoff reviewer check ref",
        logical_work_item_id,
        &mut findings,
    );

    report_duplicate_ids(
        contract
            .input_contracts
            .iter()
            .map(|input| input.contract_id.as_str())
            .chain(
                contract
                    .output_contracts
                    .iter()
                    .map(|output| output.contract_id.as_str()),
            ),
        "duplicate_contract_id",
        "contract",
        logical_work_item_id,
        &mut findings,
    );
    report_duplicate_ids(
        contract.tasks.iter().map(|task| task.task_id.as_str()),
        "duplicate_task_id",
        "task",
        logical_work_item_id,
        &mut findings,
    );
    report_duplicate_ids(
        contract
            .blocker_rules
            .iter()
            .map(|blocker| blocker.reason_code.as_str()),
        "duplicate_blocker_reason_code",
        "blocker reason code",
        logical_work_item_id,
        &mut findings,
    );
    report_duplicate_ids(
        contract
            .handoff_contract
            .required_fields
            .iter()
            .map(String::as_str),
        "duplicate_handoff_required_field",
        "handoff required field",
        logical_work_item_id,
        &mut findings,
    );
    report_duplicate_ids(
        contract
            .handoff_contract
            .provided_contract_refs
            .iter()
            .map(String::as_str),
        "duplicate_handoff_provided_contract_ref",
        "handoff provided contract ref",
        logical_work_item_id,
        &mut findings,
    );
    report_duplicate_ids(
        contract
            .handoff_contract
            .reviewer_check_refs
            .iter()
            .map(String::as_str),
        "duplicate_handoff_reviewer_check_ref",
        "handoff reviewer check ref",
        logical_work_item_id,
        &mut findings,
    );
    report_duplicate_ids(
        contract
            .acceptance_criteria
            .iter()
            .map(|criterion| criterion.criterion_id.as_str()),
        "duplicate_acceptance_criterion_id",
        "acceptance criterion",
        logical_work_item_id,
        &mut findings,
    );
    report_duplicate_ids(
        contract
            .verification_checks
            .iter()
            .map(|check| check.check_id.as_str()),
        "duplicate_verification_check_id",
        "verification check",
        logical_work_item_id,
        &mut findings,
    );

    let acceptance_criterion_ids = contract
        .acceptance_criteria
        .iter()
        .map(|criterion| criterion.criterion_id.as_str())
        .collect::<BTreeSet<_>>();
    let requirement_ids = contract
        .design_traceability
        .iter()
        .map(|traceability| traceability.requirement_id.as_str())
        .collect::<BTreeSet<_>>();

    for task in &contract.tasks {
        for reference in &task.done_when_refs {
            if !acceptance_criterion_ids.contains(reference.as_str()) {
                findings.push(error_finding(
                    "unknown_done_when_ref",
                    logical_work_item_id,
                    Some(reference),
                    None,
                    format!(
                        "task {} references unknown acceptance criterion {reference}",
                        task.task_id
                    ),
                ));
            }
        }
        for reference in &task.requirement_refs {
            if !requirement_ids.contains(reference.as_str()) {
                findings.push(error_finding(
                    "unknown_requirement_ref",
                    logical_work_item_id,
                    Some(reference),
                    None,
                    format!(
                        "task {} references unknown design requirement {reference}",
                        task.task_id
                    ),
                ));
            }
        }
    }

    let reviewer_check_refs = contract
        .handoff_contract
        .reviewer_check_refs
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    for reference in &contract.handoff_contract.reviewer_check_refs {
        if !acceptance_criterion_ids.contains(reference.as_str()) {
            findings.push(error_finding(
                "unknown_reviewer_check_ref",
                logical_work_item_id,
                Some(reference),
                None,
                format!("handoff reviewer check references unknown criterion {reference}"),
            ));
        }
    }
    for criterion_id in &acceptance_criterion_ids {
        if !reviewer_check_refs.contains(criterion_id) {
            findings.push(error_finding(
                "acceptance_criterion_without_reviewer_check",
                logical_work_item_id,
                Some(criterion_id),
                None,
                format!("acceptance criterion {criterion_id} has no handoff reviewer check"),
            ));
        }
    }

    for scope in &contract.write_policy.exclusive_scopes {
        if scope.trim().is_empty() {
            findings.push(error_finding(
                "empty_required_write_scope",
                logical_work_item_id,
                Some(scope),
                None,
                "exclusive write scope must not be blank".to_string(),
            ));
        }
    }
    let forbidden_scopes = contract
        .write_policy
        .forbidden_scopes
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    for scope in contract
        .write_policy
        .exclusive_scopes
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>()
        .intersection(&forbidden_scopes)
    {
        findings.push(error_finding(
            "overlapping_exclusive_and_forbidden_scope",
            logical_work_item_id,
            Some(scope),
            None,
            format!("write scope {scope} is both exclusive and forbidden"),
        ));
    }

    for check in &contract.verification_checks {
        if check.required
            && check
                .command
                .as_deref()
                .is_none_or(|command| command.trim().is_empty())
        {
            findings.push(error_finding(
                "missing_required_verification_command",
                logical_work_item_id,
                Some(&check.check_id),
                None,
                format!(
                    "required verification check {} must define a command",
                    check.check_id
                ),
            ));
        }
    }

    let relevant_contract_refs = contract
        .input_contracts
        .iter()
        .map(|input| input.contract_id.as_str())
        .chain(
            contract
                .output_contracts
                .iter()
                .map(|output| output.contract_id.as_str()),
        )
        .collect::<BTreeSet<_>>();
    for blocker in &contract.blocker_rules {
        if blocker.target_contract_refs.is_empty()
            && matches!(
                blocker.route,
                BlockerRoute::PlanRepairCurrent
                    | BlockerRoute::PlanRepairUpstream
                    | BlockerRoute::SubgraphReplan
            )
        {
            findings.push(error_finding(
                "stage_blocker_without_target_contract",
                logical_work_item_id,
                None,
                None,
                format!(
                    "blocker {} must target at least one input or output contract",
                    blocker.reason_code
                ),
            ));
        }
        for reference in &blocker.target_contract_refs {
            if !relevant_contract_refs.contains(reference.as_str()) {
                findings.push(error_finding(
                    "stage_blocker_without_target_contract",
                    logical_work_item_id,
                    Some(reference),
                    None,
                    format!(
                        "blocker {} targets unknown contract {reference}",
                        blocker.reason_code
                    ),
                ));
            }
        }
    }

    sorted_report(findings)
}

pub(crate) fn sorted_report(
    mut findings: Vec<ContractValidationFinding>,
) -> ContractValidationReport {
    findings.sort_by(compare_findings);
    findings.dedup();
    ContractValidationReport { findings }
}

pub(crate) fn error_finding(
    code: &str,
    logical_work_item_id: &str,
    contract_ref: Option<&str>,
    capability_ref: Option<&str>,
    message: String,
) -> ContractValidationFinding {
    ContractValidationFinding {
        code: code.to_string(),
        severity: ContractFindingSeverity::Error,
        logical_work_item_id: Some(logical_work_item_id.to_string()),
        contract_ref: contract_ref.map(str::to_string),
        capability_ref: capability_ref.map(str::to_string),
        message,
    }
}

fn report_duplicate_ids<'a>(
    ids: impl IntoIterator<Item = &'a str>,
    code: &str,
    label: &str,
    logical_work_item_id: &str,
    findings: &mut Vec<ContractValidationFinding>,
) {
    let mut seen = BTreeSet::new();
    for id in ids {
        if !seen.insert(id) {
            findings.push(error_finding(
                code,
                logical_work_item_id,
                Some(id),
                None,
                format!("duplicate {label} id {id}"),
            ));
        }
    }
}

fn report_blank_ids<'a>(
    ids: impl IntoIterator<Item = &'a str>,
    code: &str,
    label: &str,
    logical_work_item_id: &str,
    findings: &mut Vec<ContractValidationFinding>,
) {
    for id in ids {
        if id.trim().is_empty() {
            findings.push(error_finding(
                code,
                logical_work_item_id,
                Some(id),
                None,
                format!("{label} must not be blank"),
            ));
        }
    }
}

fn compare_findings(
    left: &ContractValidationFinding,
    right: &ContractValidationFinding,
) -> Ordering {
    (
        &left.code,
        &left.logical_work_item_id,
        &left.contract_ref,
        &left.capability_ref,
        &left.message,
    )
        .cmp(&(
            &right.code,
            &right.logical_work_item_id,
            &right.contract_ref,
            &right.capability_ref,
            &right.message,
        ))
}
