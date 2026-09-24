//! C1 Task 1：options×items 三族预检（REQ-WSC-02 场景 11-12）。
//!
//! F-51 根因：`project_issue_work_item_plan` 从 IR items 反推 options，使
//! `integration_work_item_required` 等三族校验在 generate/evaluate 期结构性
//! 不可触发。本文件锁定新契约：校验上下文携带存储 options（不从 IR 反推），
//! 三族缺口在候选校验产出 Error 级 finding（附修复动作），并作为可回灌修订的
//! preflight 族——不把 author 轮次硬失败（区别于结构性 grammar Error）。

use super::*;
use crate::product::models::IssueWorkItemPlanOptions;
use crate::product::work_item_plan_compiler::{
    PlanCandidateValidationContext, validate_plan_candidate_ir,
};

/// 仅含 WI-001（backend）的单项候选：三族 kind 全缺，handoff 显式为空避免
/// unconsumed_required_handoff 干扰（终端 WI 空提供是合法形态）。
fn single_backend_candidate() -> String {
    REP4_FIXTURE
        .split("## Work Item WI-002:")
        .next()
        .expect("fixture 必须包含 WI-001")
        .trim_end()
        .to_string()
        + "\n"
}

/// 把 WI-001 的 provided_contract_refs 置空，使单项候选成为合法终端 WI。
fn terminal_single_backend_candidate() -> String {
    single_backend_candidate().replacen(
        "- provided_contract_refs: contract.levels-api",
        "- provided_contract_refs: []",
        1,
    )
}

fn all_flags(integration: bool, e2e: bool, split: bool) -> IssueWorkItemPlanOptions {
    IssueWorkItemPlanOptions {
        include_integration_tests: integration,
        include_e2e_tests: e2e,
        force_frontend_backend_split: split,
        require_execution_plan_confirm: false,
    }
}

/// 持有 context 借用的所有数据（spec ids/options），避免悬垂引用；fixture
/// 显式构造 options（F-51 纪律：禁默认值丢意图）。
struct PreflightFixture {
    story_ids: Vec<String>,
    design_ids: Vec<String>,
    options: IssueWorkItemPlanOptions,
}

impl PreflightFixture {
    fn new(integration: bool, e2e: bool, split: bool) -> Self {
        Self {
            story_ids: vec!["story_spec_levels_0001".to_string()],
            design_ids: vec!["design_spec_levels_0001".to_string()],
            options: all_flags(integration, e2e, split),
        }
    }

    fn context(&self) -> PlanCandidateValidationContext<'_> {
        PlanCandidateValidationContext {
            project_id: "project_levels_0001",
            issue_id: "issue_levels_0001",
            plan_id: "plan_levels_0001",
            source_story_spec_ids: &self.story_ids,
            source_design_spec_ids: &self.design_ids,
            repository_profile: None,
            plan_options: &self.options,
            baseline_tree: None,
            now: "2026-08-27T00:00:00Z",
        }
    }
}

fn compile_candidate(
    source: &str,
) -> crate::product::work_item_plan_compiler::lower::PlanCandidateIr {
    compile_work_item_plan(
        source,
        &WorkItemPlanSourceContext {
            target_repository_id: "repo-levels".to_string(),
        },
    )
    .expect("candidate must lower to typed IR")
}

/// REQ-WSC-02 场景 11：integration=true 而候选无 integration WI——首轮即产出
/// Error finding（附修复动作），不得从 IR 反推 options 使校验沉默。
#[test]
fn integration_option_gap_produces_error_finding_with_repair_action() {
    let ir = compile_candidate(&terminal_single_backend_candidate());
    let fixture = PreflightFixture::new(true, false, false);
    let report = validate_plan_candidate_ir(&ir, &fixture.context()).expect(
        "options preflight findings must stay in the report (revision-loop routed), not hard-fail the author round",
    );

    let finding = report
        .findings
        .iter()
        .find(|finding| finding.code == "integration_work_item_required")
        .expect("integration option gap must surface as an explicit finding");
    assert_eq!(
        finding.severity,
        crate::product::models::WorkItemSplitFindingSeverity::Error
    );
    assert!(
        finding.message.contains("kind=integration"),
        "finding 必须附可照抄修复动作（新增 kind=integration Work Item）：{}",
        finding.message
    );
    assert!(
        finding
            .message
            .contains("回创建计划选项关闭 include_integration_tests"),
        "finding 必须给出第二条修复路径（关闭 flag）：{}",
        finding.message
    );
}

/// REQ-WSC-02 场景 11 同构：e2e 与 frontend_backend_split 两族同样前移。
#[test]
fn e2e_and_split_option_gaps_surface_as_error_findings() {
    let ir = compile_candidate(&terminal_single_backend_candidate());
    let fixture = PreflightFixture::new(false, true, true);
    let report = validate_plan_candidate_ir(&ir, &fixture.context())
        .expect("preflight findings must not hard-fail the author round");

    for code in ["e2e_work_item_required", "frontend_backend_split_required"] {
        let finding = report
            .findings
            .iter()
            .find(|finding| finding.code == code)
            .unwrap_or_else(|| panic!("{code} 必须产出 Error finding：{:#?}", report.findings));
        assert_eq!(
            finding.severity,
            crate::product::models::WorkItemSplitFindingSeverity::Error
        );
        assert!(
            finding.message.contains("修复动作"),
            "{code} finding 必须附修复动作：{}",
            finding.message
        );
    }
}

/// REQ-WSC-02 场景 12：三族 flag 同时启用而候选各有缺口——一次校验报告全部
/// 缺口（单项 backend 候选缺 integration/e2e/frontend 三类）。
#[test]
fn all_three_option_gaps_are_reported_together() {
    let ir = compile_candidate(&terminal_single_backend_candidate());
    let fixture = PreflightFixture::new(true, true, true);
    let report = validate_plan_candidate_ir(&ir, &fixture.context())
        .expect("preflight findings must ride the mechanical report");

    let codes = report
        .findings
        .iter()
        .filter(|finding| {
            finding.severity == crate::product::models::WorkItemSplitFindingSeverity::Error
        })
        .map(|finding| finding.code.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        codes,
        [
            "e2e_work_item_required",
            "frontend_backend_split_required",
            "integration_work_item_required",
        ],
        "三族缺口必须一次同报（按 code 排序）：{:#?}",
        report.findings
    );
}

/// REQ-WSC-02 场景 12：flag=false 保持既有 skipped-risk warning，不产生 Error。
#[test]
fn disabled_flags_keep_skipped_risk_warning_without_errors() {
    let ir = compile_candidate(&terminal_single_backend_candidate());
    let fixture = PreflightFixture::new(false, false, false);
    let report = validate_plan_candidate_ir(&ir, &fixture.context())
        .expect("all-disabled options must validate cleanly");

    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.code == "integration_or_e2e_skipped_risk"
                && finding.severity
                    == crate::product::models::WorkItemSplitFindingSeverity::Warning),
        "flag=false 必须保持既有 skipped-risk warning：{:#?}",
        report.findings
    );
    assert!(
        !report.has_errors(),
        "flag=false 不得产生任何 Error：{:#?}",
        report.findings
    );
}

/// 存储 options 满足时（候选含对应 kind）不产 Error——反向防误报。REP4 自带
/// backend+frontend+integration 三类，split=true 满足。
#[test]
fn satisfied_options_do_not_produce_preflight_errors() {
    let ir = compile_candidate(REP4_FIXTURE);
    let fixture = PreflightFixture::new(false, false, true);
    let report = validate_plan_candidate_ir(&ir, &fixture.context())
        .expect("rep4 satisfies split options");

    for code in [
        "integration_work_item_required",
        "e2e_work_item_required",
        "frontend_backend_split_required",
    ] {
        assert!(
            !report.findings.iter().any(|finding| finding.code == code),
            "{code} 不应出现：{:#?}",
            report.findings
        );
    }
}

/// options 反推防护：即使 IR 自带 integration item，校验也 MUST 消费存储
/// options（integration=true + 候选有 integration item → 无 Error），与
/// terminal_single_backend 用例共同锁定「存储 options 是唯一事实源」。
#[test]
fn stored_options_are_not_inferred_from_ir_items() {
    let ir = compile_candidate(REP4_FIXTURE);
    let fixture = PreflightFixture::new(true, false, false);
    let report = validate_plan_candidate_ir(&ir, &fixture.context())
        .expect("rep4 with matching integration option validates");

    assert!(
        !report
            .findings
            .iter()
            .any(|finding| finding.code == "integration_work_item_required"),
        "存储 options 与 items 一致时不得产生 Error：{:#?}",
        report.findings
    );
}
