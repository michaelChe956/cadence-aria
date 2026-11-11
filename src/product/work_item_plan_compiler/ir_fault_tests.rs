//! 工作包 2.6（MODIFIED REQ-WSC-09）：四类 IR 诊断 typed 载荷的定向测试。
//!
//! 断言口径：
//! - 载荷与 message 无关：断言前先把 message 中出现过的 task/criterion/
//!   requirement 编号抹掉，`ir_fault` 各字段仍与 fixture 一致（不从文本
//!   反解析）。
//! - `source_line` 只认 `parse_work_item_plan` 的 AST span（经
//!   `attach_source_lines` 补全）；contract 层无行号时保持 `None`，
//!   不得以 0 冒充。
//! - 旧 serde 记录（无 `ir_fault` 键）必须原样可反序列化，`None` 再序列化
//!   字节不变（哈希兼容纪律）。

use crate::product::models::{RepositoryProfile, RepositoryProfileConfidence};
use crate::product::work_item_plan_compiler::CompilerDiagnostic;
use crate::product::work_item_plan_compiler::{
    IrFault, IrFaultKind, IrFaultLocator, PlanCandidateValidationContext,
    WorkItemPlanSourceContext, attach_source_lines, compile_work_item_plan,
    validate_plan_candidate_ir,
};

const REP4_FIXTURE: &str = include_str!("fixtures/work-item-plan-rep4.md");

fn rep4_repository_profile() -> RepositoryProfile {
    RepositoryProfile {
        id: "repository_profile_levels_0001".to_string(),
        project_id: "project_levels_0001".to_string(),
        issue_id: "issue_levels_0001".to_string(),
        repository_id: "repo-levels".to_string(),
        logical_repository_id: None,
        membership_revision: 0,
        provider_run_ref: Some("provider_run_levels_0001".to_string()),
        languages: vec!["rust".to_string(), "typescript".to_string()],
        frameworks: vec!["axum".to_string(), "react".to_string()],
        package_managers: vec!["cargo".to_string(), "pnpm".to_string()],
        test_frameworks: vec!["cargo-test".to_string(), "vitest".to_string()],
        build_systems: vec!["cargo".to_string(), "vite".to_string()],
        verification_capabilities: vec!["unit".to_string(), "integration".to_string()],
        detected_layers: vec!["backend".to_string(), "frontend".to_string()],
        split_recommendation: "frontend_backend".to_string(),
        confidence: RepositoryProfileConfidence::High,
        uncertainties: Vec::new(),
        created_at: "2026-08-27T00:00:00Z".to_string(),
        updated_at: "2026-08-27T00:00:00Z".to_string(),
    }
}

fn rep4_validation_context<'a>(
    profile: Option<&'a RepositoryProfile>,
    story_ids: &'a [String],
    design_ids: &'a [String],
) -> PlanCandidateValidationContext<'a> {
    // F-51：fixture 显式构造 options——rep4 自带 integration+backend+frontend，
    // 存储意图为「不要求 integration/e2e」（false），不从 IR 反推。
    static REP4_OPTIONS: std::sync::LazyLock<crate::product::models::IssueWorkItemPlanOptions> =
        std::sync::LazyLock::new(|| crate::product::models::IssueWorkItemPlanOptions {
            include_integration_tests: false,
            include_e2e_tests: false,
            force_frontend_backend_split: false,
            require_execution_plan_confirm: false,
        });
    PlanCandidateValidationContext {
        project_id: "project_levels_0001",
        issue_id: "issue_levels_0001",
        plan_id: "plan_levels_0001",
        source_story_spec_ids: story_ids,
        source_design_spec_ids: design_ids,
        repository_profile: profile,
        plan_options: &REP4_OPTIONS,
        baseline_tree: None,
        existing_work_item_ids: &[],
        enrollment_target: None,
        member_targets: None,
        member_checkout_prefixes: None,
        now: "2026-08-27T00:00:00Z",
    }
}

/// 把 source 编译为 IR 并走既有机械校验，收集 fail-closed 诊断
/// （四类交叉引用错误都是 Error 且非 preflight 族，必须走 Err 出口）。
fn compile_and_collect_diagnostics(source: &str) -> Vec<CompilerDiagnostic> {
    let ir = compile_work_item_plan(
        source,
        &WorkItemPlanSourceContext {
            target_repository_id: "repo-levels".to_string(),
            member_targets: None,
        },
    )
    .expect("交叉引用错误必须可 lower 为供 validator 拒绝的 typed IR");
    let profile = rep4_repository_profile();
    let story_ids = vec!["story_spec_levels_0001".to_string()];
    let design_ids = vec!["design_spec_levels_0001".to_string()];
    validate_plan_candidate_ir(
        &ir,
        &rep4_validation_context(Some(&profile), &story_ids, &design_ids),
    )
    .expect_err("四类 IR 交叉引用错误必须使候选校验 fail-closed")
}

/// 载荷无关性断言口径：抹掉 message 中出现过的全部编号字样后再读
/// `ir_fault`——字段仍与 fixture 一致（REQ-WSC-09：不从 message 反解析）。
fn scrub_message(diagnostic: &CompilerDiagnostic, ids: &[&str]) -> CompilerDiagnostic {
    let mut scrubbed = diagnostic.clone();
    for id in ids {
        scrubbed.message = scrubbed.message.replace(id, "<已抹除>");
    }
    scrubbed
}

fn faults_for(source: &str, code: &str) -> Vec<IrFault> {
    compile_and_collect_diagnostics(source)
        .iter()
        .filter(|diagnostic| diagnostic.code == code)
        .filter_map(|diagnostic| diagnostic.ir_fault.clone())
        .collect()
}

#[test]
fn unknown_done_when_ref_fault_payload_matches_fixture() {
    let source = REP4_FIXTURE.replacen("- done_when_refs: AC-001", "- done_when_refs: AC-999", 1);
    let diagnostics = compile_and_collect_diagnostics(&source);
    let diagnostic = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "unknown_done_when_ref")
        .expect("unknown_done_when_ref 必须出现在 fail-closed 诊断里");
    let scrubbed = scrub_message(diagnostic, &["TASK-001", "AC-999"]);
    let fault = scrubbed
        .ir_fault
        .as_ref()
        .expect("四码诊断必须携带 typed 载荷");
    assert_eq!(fault.kind, IrFaultKind::UnknownDoneWhenRef);
    assert_eq!(fault.work_item_id, "WI-001");
    assert_eq!(
        fault.locator,
        IrFaultLocator::Task {
            task_id: "TASK-001".to_string(),
        }
    );
    assert_eq!(fault.offending_ref, "AC-999");
    // 合法集合 = 本 WI 全部 criterion id。
    assert_eq!(fault.legal_refs, vec!["AC-001".to_string()]);
    // contract 层无行号事实：保持 None，不得以 0 冒充。
    assert_eq!(fault.source_line, None);
    assert_eq!(
        fault.expected_shape,
        IrFaultKind::UnknownDoneWhenRef.expected_shape()
    );
    assert!(fault.expected_shape.contains("done_when_refs"));
}

#[test]
fn unknown_requirement_ref_fault_payload_matches_fixture() {
    let source = REP4_FIXTURE.replacen(
        "- requirement_refs: REQ-WSC-02",
        "- requirement_refs: REQ-777",
        1,
    );
    let diagnostics = compile_and_collect_diagnostics(&source);
    let diagnostic = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "unknown_requirement_ref")
        .expect("unknown_requirement_ref 必须出现在 fail-closed 诊断里");
    let scrubbed = scrub_message(diagnostic, &["TASK-001", "REQ-777"]);
    let fault = scrubbed
        .ir_fault
        .as_ref()
        .expect("四码诊断必须携带 typed 载荷");
    assert_eq!(fault.kind, IrFaultKind::UnknownRequirementRef);
    assert_eq!(fault.work_item_id, "WI-001");
    assert_eq!(
        fault.locator,
        IrFaultLocator::Task {
            task_id: "TASK-001".to_string(),
        }
    );
    assert_eq!(fault.offending_ref, "REQ-777");
    // 合法集合 = 本 WI Traceability 已声明的 requirement_id。
    assert_eq!(fault.legal_refs, vec!["REQ-WSC-02".to_string()]);
    assert_eq!(fault.source_line, None);
    assert_eq!(
        fault.expected_shape,
        IrFaultKind::UnknownRequirementRef.expected_shape()
    );
    assert!(fault.expected_shape.contains("requirement_refs"));
}

#[test]
fn unknown_reviewer_check_ref_fault_payload_matches_fixture() {
    let source = REP4_FIXTURE.replacen(
        "- reviewer_check_refs: AC-001",
        "- reviewer_check_refs: AC-888",
        1,
    );
    let diagnostics = compile_and_collect_diagnostics(&source);
    let diagnostic = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "unknown_reviewer_check_ref")
        .expect("unknown_reviewer_check_ref 必须出现在 fail-closed 诊断里");
    let scrubbed = scrub_message(diagnostic, &["AC-888"]);
    let fault = scrubbed
        .ir_fault
        .as_ref()
        .expect("四码诊断必须携带 typed 载荷");
    assert_eq!(fault.kind, IrFaultKind::UnknownReviewerCheckRef);
    assert_eq!(fault.work_item_id, "WI-001");
    // 定位规则钉死：reviewer check ref 归 Handoff。
    assert_eq!(fault.locator, IrFaultLocator::Handoff);
    assert_eq!(fault.offending_ref, "AC-888");
    // 合法集合 = 本 WI 全部 criterion id。
    assert_eq!(fault.legal_refs, vec!["AC-001".to_string()]);
    assert_eq!(fault.source_line, None);
    assert_eq!(
        fault.expected_shape,
        IrFaultKind::UnknownReviewerCheckRef.expected_shape()
    );
    assert!(fault.expected_shape.contains("reviewer_check_refs"));
}

#[test]
fn acceptance_criterion_without_reviewer_check_fault_payload_matches_fixture() {
    let source = REP4_FIXTURE.replacen(
        "- reviewer_check_refs: AC-001",
        "- reviewer_check_refs: []",
        1,
    );
    let diagnostics = compile_and_collect_diagnostics(&source);
    let diagnostic = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "acceptance_criterion_without_reviewer_check")
        .expect("acceptance_criterion_without_reviewer_check 必须出现在 fail-closed 诊断里");
    let scrubbed = scrub_message(diagnostic, &["AC-001"]);
    let fault = scrubbed
        .ir_fault
        .as_ref()
        .expect("四码诊断必须携带 typed 载荷");
    assert_eq!(
        fault.kind,
        IrFaultKind::AcceptanceCriterionWithoutReviewerCheck
    );
    assert_eq!(fault.work_item_id, "WI-001");
    // 定位规则钉死：未被覆盖的 criterion 归 Criterion{criterion_id}。
    assert_eq!(
        fault.locator,
        IrFaultLocator::Criterion {
            criterion_id: "AC-001".to_string(),
        }
    );
    assert_eq!(fault.offending_ref, "AC-001");
    // 合法集合 = 本 WI 实际声明的 reviewer_check_refs（空即空，不臆造）。
    assert_eq!(fault.legal_refs, Vec::<String>::new());
    assert_eq!(fault.source_line, None);
    assert_eq!(
        fault.expected_shape,
        IrFaultKind::AcceptanceCriterionWithoutReviewerCheck.expected_shape()
    );
    assert!(fault.expected_shape.contains("reviewer_check_refs"));
}

#[test]
fn attach_source_lines_fills_line_or_none() {
    let done_when_source =
        REP4_FIXTURE.replacen("- done_when_refs: AC-001", "- done_when_refs: AC-999", 1);
    let requirement_source = REP4_FIXTURE.replacen(
        "- requirement_refs: REQ-WSC-02",
        "- requirement_refs: REQ-777",
        1,
    );
    let reviewer_source = REP4_FIXTURE.replacen(
        "- reviewer_check_refs: AC-001",
        "- reviewer_check_refs: AC-888",
        1,
    );

    // 行号必须来自 parse_work_item_plan 的 AST span：rep4 fixture 中
    // WI-001 的四个 ref 行固定在 35/36/43/58 行（1-based）。
    let mut done_when = faults_for(&done_when_source, "unknown_done_when_ref");
    attach_source_lines(&mut done_when, &done_when_source);
    assert_eq!(done_when[0].source_line, Some(36));

    let mut requirement = faults_for(&requirement_source, "unknown_requirement_ref");
    attach_source_lines(&mut requirement, &requirement_source);
    assert_eq!(requirement[0].source_line, Some(35));

    let mut reviewer = faults_for(&reviewer_source, "unknown_reviewer_check_ref");
    attach_source_lines(&mut reviewer, &reviewer_source);
    assert_eq!(reviewer[0].source_line, Some(58));

    let mut criterion = faults_for(
        &reviewer_source,
        "acceptance_criterion_without_reviewer_check",
    );
    attach_source_lines(&mut criterion, &reviewer_source);
    assert_eq!(criterion[0].source_line, Some(43));

    // source 中无该行（对不含错误 ref 的原始 fixture 定位）→ 保持 None，
    // 不得以 0 冒充（REQ-WSC-09：无原行即不授资格）。
    let mut missing = faults_for(&done_when_source, "unknown_done_when_ref");
    attach_source_lines(&mut missing, REP4_FIXTURE);
    assert_eq!(missing[0].source_line, None);
}

#[test]
fn non_ir_code_gets_no_ir_fault() {
    // 非四码的既有诊断（duplicate_task_id 在 grammar 层即被
    // invalid_work_item_id 终态拒绝、到不了校验层，故用
    // overlapping_exclusive_and_forbidden_scope）：不得携带 ir_fault。
    let source = REP4_FIXTURE.replacen(
        "- forbidden_scopes: web/**",
        "- forbidden_scopes: src/product/levels/**",
        1,
    );
    let diagnostics = compile_and_collect_diagnostics(&source);
    let overlap = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "overlapping_exclusive_and_forbidden_scope")
        .expect("exclusive/forbidden 重叠必须仍由既有非四码诊断拒绝");
    assert_eq!(overlap.ir_fault, None);
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic.ir_fault.is_none()),
        "非四码诊断一律不得携带 ir_fault：{diagnostics:#?}"
    );
}

#[test]
fn old_serde_record_without_ir_field_still_deserializes() {
    use crate::product::work_item_contract::ContractValidationFinding;

    // 旧 JSON（无 ir_fault 键）必须原样可反序列化为 None。
    let legacy = concat!(
        r#"{"code":"unknown_done_when_ref","severity":"Error","#,
        r#""logical_work_item_id":"WI-001","contract_ref":"AC-999","#,
        r#""capability_ref":null,"message":"旧记录"}"#,
    );
    let finding: ContractValidationFinding =
        serde_json::from_str(legacy).expect("旧 JSON 必须可反序列化");
    assert_eq!(finding.ir_fault, None);
    assert!(
        !serde_json::to_string(&finding)
            .unwrap()
            .contains("ir_fault"),
        "ir_fault=None 时再序列化不得出现该键（哈希兼容纪律）"
    );

    // 新载荷可 round-trip；IrFaultKind 的 serde 形状与 PlanningFailureKind
    // 同形（tag="kind"、snake_case 变体名）。
    let mut with_fault = finding;
    with_fault.ir_fault = Some(IrFault {
        kind: IrFaultKind::UnknownDoneWhenRef,
        work_item_id: "WI-001".to_string(),
        locator: IrFaultLocator::Task {
            task_id: "TASK-001".to_string(),
        },
        offending_ref: "AC-999".to_string(),
        legal_refs: vec!["AC-001".to_string()],
        source_line: None,
        expected_shape: IrFaultKind::UnknownDoneWhenRef.expected_shape().to_string(),
    });
    let json = serde_json::to_string(&with_fault).unwrap();
    assert!(
        json.contains(r#""kind":"unknown_done_when_ref""#),
        "tag+snake_case 形状必须钉死：{json}"
    );
    let round_tripped: ContractValidationFinding = serde_json::from_str(&json).unwrap();
    assert_eq!(round_tripped, with_fault);
}
