use super::canonical_contract_fixture;
use crate::product::work_item_contract::{
    BlockerRoute, CanonicalWorkItemContract, ContractFindingSeverity, ContractValidationFinding,
    ContractValidationReport, validate_canonical_contract,
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
fn canonical_work_item_validation_warns_for_process_evidence_acceptance_criterion() {
    let mut contract = canonical_contract_fixture("WI-01");
    contract.acceptance_criteria[0].criterion_id = "ac_tdd_red_evidence".to_string();
    contract.acceptance_criteria[0].statement = "先失败的测试提交必须存在".to_string();
    contract.tasks[0].done_when_refs = vec!["ac_tdd_red_evidence".to_string()];
    contract.handoff_contract.reviewer_check_refs = vec!["ac_tdd_red_evidence".to_string()];

    let report = validate_canonical_contract(&contract);
    let finding = finding(&report, "process_evidence_acceptance_criterion");

    assert_eq!(finding.severity, ContractFindingSeverity::Warning);
    assert_eq!(finding.contract_ref.as_deref(), Some("ac_tdd_red_evidence"));
    assert!(report.is_valid());
}

#[test]
fn canonical_work_item_validation_checks_process_evidence_in_criterion_id() {
    let mut contract = canonical_contract_fixture("WI-01");
    contract.acceptance_criteria[0].criterion_id = "ac_red_commit_history".to_string();
    contract.acceptance_criteria[0].statement = "Feature behavior is observable".to_string();
    contract.tasks[0].done_when_refs = vec!["ac_red_commit_history".to_string()];
    contract.handoff_contract.reviewer_check_refs = vec!["ac_red_commit_history".to_string()];

    let report = validate_canonical_contract(&contract);
    let finding = finding(&report, "process_evidence_acceptance_criterion");

    assert_eq!(finding.severity, ContractFindingSeverity::Warning);
    assert_eq!(
        finding.contract_ref.as_deref(),
        Some("ac_red_commit_history")
    );
}

#[test]
fn canonical_work_item_validation_warns_for_process_evidence_git_commit_history_without_red_token()
{
    let mut contract = canonical_contract_fixture("WI-01");
    contract.acceptance_criteria[0].criterion_id = "ac_git_commit_history".to_string();
    contract.acceptance_criteria[0].statement = "Evidence is available".to_string();
    contract.tasks[0].done_when_refs = vec!["ac_git_commit_history".to_string()];
    contract.handoff_contract.reviewer_check_refs = vec!["ac_git_commit_history".to_string()];

    let report = validate_canonical_contract(&contract);
    let finding = finding(&report, "process_evidence_acceptance_criterion");

    assert_eq!(finding.severity, ContractFindingSeverity::Warning);
    assert_eq!(
        finding.contract_ref.as_deref(),
        Some("ac_git_commit_history")
    );
}

#[test]
fn canonical_work_item_validation_ignores_incomplete_or_observable_process_evidence_matches() {
    let cases = [
        ("提交验证结果", "AC-COMMIT"),
        ("red 状态已记录", "AC-RED"),
        ("commit credentials remain valid", "AC-CREDENTIALS"),
        ("用户提交表单后，结果按创建时间顺序展示", "AC-FORM-ORDER"),
        ("事务按提交顺序持久化", "ac_transaction_commit"),
        ("验证命令实际运行了非零数量的测试", "AC-OBSERVABLE"),
        // 真实误报：中文「提交」在前端语境指提交按钮/控件，不是 git commit。
        // 该 statement 同时含「提交控件」「代码」「链路」，旧规则三条件全中。
        (
            "在任意 HTTP 静态服务器下打开 demo/index.html，于秒数输入框输入 3661 后无需点击任何提交控件，结果区即显示 01:01:01；页面中不存在参与该更新链路的提交按钮",
            "ac_demo_realtime_result",
        ),
        (
            "表单提交按钮在测试代码校验失败时保持禁用，且按提交顺序展示错误",
            "ac_form_submit_guard",
        ),
    ];

    for (statement, criterion_id) in cases {
        let mut contract = canonical_contract_fixture("WI-01");
        contract.acceptance_criteria[0].criterion_id = criterion_id.to_string();
        contract.acceptance_criteria[0].statement = statement.to_string();
        contract.tasks[0].done_when_refs = vec![criterion_id.to_string()];
        contract.handoff_contract.reviewer_check_refs = vec![criterion_id.to_string()];

        let report = validate_canonical_contract(&contract);

        assert!(
            report
                .findings
                .iter()
                .all(|finding| finding.code != "process_evidence_acceptance_criterion"),
            "{statement} must not create a process-evidence warning: {:?}",
            report.findings
        );
        assert!(report.is_valid(), "{statement} must remain valid");
    }
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
fn canonical_work_item_validation_enforces_stable_non_blank_identity_fields() {
    let mut cases = Vec::new();

    let mut contract = canonical_contract_fixture("WI-01");
    contract.identity.logical_work_item_id = "  ".to_string();
    cases.push(("blank_logical_work_item_id", contract));

    let mut contract = canonical_contract_fixture("WI-01");
    contract.input_contracts[0].contract_id = "  ".to_string();
    cases.push(("blank_input_contract_id", contract));

    let mut contract = canonical_contract_fixture("WI-01");
    contract.input_contracts[0].provider_logical_work_item_id = "  ".to_string();
    cases.push(("blank_provider_logical_work_item_id", contract));

    let mut contract = canonical_contract_fixture("WI-01");
    contract.output_contracts[0].contract_id = "  ".to_string();
    cases.push(("blank_output_contract_id", contract));

    let mut contract = canonical_contract_fixture("WI-01");
    contract.tasks[0].task_id = "  ".to_string();
    cases.push(("blank_task_id", contract));

    let mut contract = canonical_contract_fixture("WI-01");
    contract.acceptance_criteria[0].criterion_id = "  ".to_string();
    cases.push(("blank_acceptance_criterion_id", contract));

    let mut contract = canonical_contract_fixture("WI-01");
    contract.verification_checks[0].check_id = "  ".to_string();
    cases.push(("blank_verification_check_id", contract));

    let mut contract = canonical_contract_fixture("WI-01");
    contract.blocker_rules[0].reason_code = "  ".to_string();
    cases.push(("blank_blocker_reason_code", contract));

    for (expected_code, contract) in cases {
        finding(&validate_canonical_contract(&contract), expected_code);
    }
}

#[test]
fn canonical_work_item_validation_enforces_stable_unique_contract_and_blocker_identities() {
    let mut contract = canonical_contract_fixture("WI-01");
    contract
        .input_contracts
        .push(contract.input_contracts[0].clone());
    contract
        .output_contracts
        .push(contract.output_contracts[0].clone());
    contract
        .blocker_rules
        .push(contract.blocker_rules[0].clone());

    let report = validate_canonical_contract(&contract);
    let duplicate_contract_refs = report
        .findings
        .iter()
        .filter(|finding| finding.code == "duplicate_contract_id")
        .filter_map(|finding| finding.contract_ref.as_deref())
        .collect::<Vec<_>>();

    assert_eq!(
        duplicate_contract_refs,
        vec!["contract.canonical", "contract.source"]
    );
    assert_eq!(
        finding(&report, "duplicate_blocker_reason_code")
            .contract_ref
            .as_deref(),
        Some("contract_invalid")
    );
}

#[test]
fn canonical_work_item_validation_enforces_stable_non_blank_unique_handoff_references() {
    let mut contract = canonical_contract_fixture("WI-01");
    contract.handoff_contract.required_fields = vec![
        "  ".to_string(),
        "commit_sha".to_string(),
        "commit_sha".to_string(),
    ];
    contract.handoff_contract.provided_contract_refs = vec![
        "  ".to_string(),
        "contract.canonical".to_string(),
        "contract.canonical".to_string(),
    ];
    contract.handoff_contract.reviewer_check_refs =
        vec!["  ".to_string(), "AC-001".to_string(), "AC-001".to_string()];

    let report = validate_canonical_contract(&contract);
    for code in [
        "blank_handoff_required_field",
        "duplicate_handoff_required_field",
        "blank_handoff_provided_contract_ref",
        "duplicate_handoff_provided_contract_ref",
        "blank_handoff_reviewer_check_ref",
        "duplicate_handoff_reviewer_check_ref",
    ] {
        finding(&report, code);
    }
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

/// required check 必须有可执行依据，但人工核对以 manual_instruction 为依据。
///
/// 实测缺陷：校验把「required」等同于「必须有 command」，于是没有测试框架的
/// outline（如纯静态页面）无法把任何人工核对设为必需——而 reviewer 按 outline 的
/// verification_intent 要求它们必需，author 与校验层因此互相否决。
#[test]
fn canonical_work_item_validation_rejects_required_check_without_command_or_manual_instruction() {
    let mut contract = canonical_contract_fixture("WI-01");
    contract.verification_checks[0].command = None;
    contract.verification_checks[0].manual_instruction = None;

    let report = validate_canonical_contract(&contract);
    let finding = finding(&report, "missing_required_verification_command");

    assert_eq!(finding.logical_work_item_id.as_deref(), Some("WI-01"));
    assert_eq!(finding.contract_ref.as_deref(), Some("check_canonical"));
}

#[test]
fn canonical_work_item_validation_allows_required_manual_check_without_command() {
    let mut contract = canonical_contract_fixture("WI-01");
    contract.verification_checks[0].command = None;
    contract.verification_checks[0].manual_instruction =
        Some("在静态服务器下打开页面并核对 7 条示例".to_string());
    contract.verification_checks[0].required = true;

    let report = validate_canonical_contract(&contract);

    assert!(
        report
            .findings
            .iter()
            .all(|item| item.code != "missing_required_verification_command"),
        "required manual check with a manual_instruction must be accepted: {:?}",
        report.findings
    );
    assert!(report.is_valid());
}

#[test]
fn canonical_work_item_validation_rejects_required_check_with_blank_manual_instruction() {
    let mut contract = canonical_contract_fixture("WI-01");
    contract.verification_checks[0].command = None;
    contract.verification_checks[0].manual_instruction = Some("   ".to_string());

    let report = validate_canonical_contract(&contract);

    assert_eq!(
        finding(&report, "missing_required_verification_command")
            .contract_ref
            .as_deref(),
        Some("check_canonical")
    );
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
fn canonical_work_item_validation_rejects_each_draft_only_reference_mutation_once() {
    type Mutation = fn(&mut CanonicalWorkItemContract);

    let cases: [(&str, Mutation); 9] = [
        ("missing_required_verification_command", |contract| {
            contract.verification_checks[0].command = None;
        }),
        ("unknown_done_when_ref", |contract| {
            contract.tasks[0].done_when_refs = vec!["check_canonical".to_string()];
        }),
        ("unknown_reviewer_check_ref", |contract| {
            contract.handoff_contract.reviewer_check_refs = vec!["check_canonical".to_string()];
        }),
        ("stage_blocker_without_target_contract", |contract| {
            contract.blocker_rules[0].target_contract_refs = vec!["REQ-CANONICAL-001".to_string()];
        }),
        ("stage_blocker_without_target_contract", |contract| {
            contract.blocker_rules[0].target_contract_refs = vec!["NFR-001".to_string()];
        }),
        ("stage_blocker_without_target_contract", |contract| {
            contract.blocker_rules[0].target_contract_refs = vec!["check_canonical".to_string()];
        }),
        ("stage_blocker_without_target_contract", |contract| {
            contract.blocker_rules[0].target_contract_refs =
                vec!["src/product/work_item_contract".to_string()];
        }),
        ("unknown_requirement_ref", |contract| {
            contract.tasks[0].requirement_refs = vec!["REQ-NOT-IN-DESIGN".to_string()];
        }),
        ("stage_blocker_without_target_contract", |contract| {
            contract.blocker_rules[0].target_contract_refs.clear();
        }),
    ];

    for (expected_code, mutate) in cases {
        let mut contract = canonical_contract_fixture("WI-01");
        mutate(&mut contract);

        let report = validate_canonical_contract(&contract);
        assert!(!report.is_valid(), "{expected_code} must reject the Draft");
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
            "blank_logical_work_item_id",
            "blank_input_contract_id",
            "blank_output_contract_id",
            "duplicate_task_id",
            "duplicate_acceptance_criterion_id",
            "duplicate_verification_check_id",
        ] {
            assert!(
                report
                    .findings
                    .iter()
                    .all(|finding| finding.code != unrelated_identity_code),
                "{expected_code} must not create unrelated identity finding {unrelated_identity_code}: {:?}",
                report.findings
            );
        }
    }
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
fn canonical_work_item_validation_requires_empty_target_only_for_plan_repair_routes() {
    let cases = [
        (BlockerRoute::CoderRework, false),
        (BlockerRoute::VerificationRetry, false),
        (BlockerRoute::PlanRepairCurrent, true),
        (BlockerRoute::PlanRepairUpstream, true),
        (BlockerRoute::SubgraphReplan, true),
        (BlockerRoute::StoryAmendment, false),
        (BlockerRoute::DesignAmendment, false),
        (BlockerRoute::OperationalGate, false),
    ];

    for (route, requires_target) in cases {
        let mut contract = canonical_contract_fixture("WI-01");
        contract.blocker_rules[0].route = route.clone();
        contract.blocker_rules[0].target_contract_refs.clear();

        let count = validate_canonical_contract(&contract)
            .findings
            .iter()
            .filter(|finding| finding.code == "stage_blocker_without_target_contract")
            .count();

        assert_eq!(
            count,
            usize::from(requires_target),
            "unexpected empty-target result for {route:?}"
        );
    }
}

#[test]
fn canonical_work_item_validation_rejects_invalid_explicit_target_for_every_blocker_route() {
    let routes = [
        BlockerRoute::CoderRework,
        BlockerRoute::VerificationRetry,
        BlockerRoute::PlanRepairCurrent,
        BlockerRoute::PlanRepairUpstream,
        BlockerRoute::SubgraphReplan,
        BlockerRoute::StoryAmendment,
        BlockerRoute::DesignAmendment,
        BlockerRoute::OperationalGate,
    ];

    for route in routes {
        let mut contract = canonical_contract_fixture("WI-01");
        contract.blocker_rules[0].route = route.clone();
        contract.blocker_rules[0].target_contract_refs = vec!["contract.unknown".to_string()];

        let findings = validate_canonical_contract(&contract)
            .findings
            .into_iter()
            .filter(|finding| finding.code == "stage_blocker_without_target_contract")
            .collect::<Vec<_>>();

        assert_eq!(findings.len(), 1, "invalid target ignored for {route:?}");
        assert_eq!(
            findings[0].contract_ref.as_deref(),
            Some("contract.unknown")
        );
    }
}

#[test]
fn canonical_work_item_validation_accepts_valid_target_for_plan_repair_routes() {
    for route in [
        BlockerRoute::PlanRepairCurrent,
        BlockerRoute::PlanRepairUpstream,
        BlockerRoute::SubgraphReplan,
    ] {
        let mut contract = canonical_contract_fixture("WI-01");
        contract.blocker_rules[0].route = route.clone();
        contract.blocker_rules[0].target_contract_refs = vec!["contract.canonical".to_string()];

        assert!(
            validate_canonical_contract(&contract)
                .findings
                .iter()
                .all(|finding| finding.code != "stage_blocker_without_target_contract"),
            "valid target rejected for {route:?}"
        );
    }
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
