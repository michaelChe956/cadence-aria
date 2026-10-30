//! 工作项「检查目标 × 本项 forbidden 写域」自洽校验（bug45/t25 诊断，P1）。
//!
//! 根因形态（kimi WI-002 现场）：`forbidden_scopes` 声明 `lib.rs` 禁写、
//! Non Goals 明说不改 lib.rs，而 AC/Verification 又把「审阅 lib.rs 新增 pub
//! 项文档注释」列为必需检查——自相矛盾的 plan 让编码会话两种结局都红
//!（写 → 违反禁写域；不写 → 检查不过），只能在返修循环里耗尽预算。
//! 本模块在机械校验层前置拦截：同一工作项的验收面引用的路径若落在**本项**
//! `forbidden_scopes` 写域内 → Error finding（preflight 族，回灌修订）。
//!
//! 零误伤边界：
//! - 检查目标不在本项 forbidden 写域内 → 一律不报（读其他文件合法，
//!   「超出 exclusive_scopes」不是本规则的错因）；
//! - 整仓命令（`cargo test` 等无路径 token）天然无可提取路径，不报；
//! - 跨工作项引用（路径只落在**别的**工作项 forbidden 内）不报——
//!   本规则只对「本项检查 × 本项禁写」的自洽性负责。

use crate::cross_cutting::worktree::scope_allows_path;
use crate::product::models::{WorkItemSplitFinding, WorkItemSplitFindingSeverity};

use super::ac_paths::{collect_command_paths, collect_statement_paths};
use super::lower::{PlanCandidateIr, PlanCandidateItemIr};
use super::types::WORK_ITEM_CHECK_TARGET_FORBIDDEN_REPAIR_ACTION;

/// 逐工作项产出「检查目标 × 本项 forbidden 写域」冲突 finding（每 WI 至多
/// 一条，冲突项聚合；顺序稳定：AC → verification check、声明序）。
///
/// 提取面复用 ac_paths 的受限形态规则（方法前缀路由/引号 span/命令 token，
/// 不做全文模糊匹配）；trusted_commands 由 lowering 从 verification checks
/// 逐字派生（`trusted_commands_for_checks`），走 checks 即全覆盖，不重复
/// 提取。scope 匹配复用仓内权威 [`scope_allows_path`]（含 F-25 顿号方言
/// 处理），case_sensitive 与 split validator 既有调用面一致。
pub(crate) fn check_target_forbidden_findings(ir: &PlanCandidateIr) -> Vec<WorkItemSplitFinding> {
    ir.items.iter().filter_map(item_conflict_finding).collect()
}

fn item_conflict_finding(item: &PlanCandidateItemIr) -> Option<WorkItemSplitFinding> {
    let forbidden = &item.contract.write_policy.forbidden_scopes;
    if forbidden.is_empty() {
        return None;
    }
    let work_item_id = &item.contract.identity.logical_work_item_id;
    // 冲突项文案：`<检查来源> → <引用路径>（forbidden: <命中 scope>）`。
    let mut conflicts: Vec<String> = Vec::new();
    let mut consider = |source: &str, text: &str, is_command: bool| {
        let mut refs = Vec::new();
        if is_command {
            collect_command_paths(text, &mut refs);
        } else {
            collect_statement_paths(text, &mut refs);
        }
        for path in refs {
            if let Some(scope) = forbidden
                .iter()
                .find(|scope| scope_allows_path(scope, &path, true))
            {
                let entry = format!("{source} → {path}（forbidden: {scope}）");
                // 同一检查来源内收集器已去重；跨来源同路径各自点名（不同
                // 检查须各自修订），仅完全相同条目（同源 command 与
                // manual_instruction 重复引用）收敛为一条。
                if !conflicts.contains(&entry) {
                    conflicts.push(entry);
                }
            }
        }
    };
    for criterion in &item.contract.acceptance_criteria {
        consider(&criterion.criterion_id, &criterion.statement, false);
    }
    for check in &item.verification_plan.checks {
        if let Some(command) = &check.command {
            consider(&check.check_id, command, true);
        }
        if let Some(instruction) = &check.manual_instruction {
            consider(&check.check_id, instruction, false);
        }
    }
    if conflicts.is_empty() {
        return None;
    }
    Some(WorkItemSplitFinding {
        severity: WorkItemSplitFindingSeverity::Error,
        code: "work_item_check_target_forbidden".to_string(),
        message: format!(
            "work item {work_item_id} 的检查目标 [{}] 落在本项 forbidden 写域内：plan 一边禁止本项写这些路径、一边要求本项检查它们，两种执行结局都无法通过；修复动作：{WORK_ITEM_CHECK_TARGET_FORBIDDEN_REPAIR_ACTION}",
            conflicts.join("、")
        ),
        work_item_ids: vec![work_item_id.clone()],
        // F1.2：preflight 族 finding 暂无 typed payload（None=Unknown 不
        // 自动；显式 kind 工厂归 F2b）。
        kind: None,
    })
}

#[cfg(test)]
mod tests {
    use super::WORK_ITEM_CHECK_TARGET_FORBIDDEN_REPAIR_ACTION;
    use crate::product::models::{IssueWorkItemPlanOptions, WorkItemSplitFindingSeverity};
    use crate::product::work_item_plan_compiler::{
        PlanCandidateValidationContext, WorkItemPlanSourceContext, compile_work_item_plan,
        validate_plan_candidate_ir,
    };

    const REP4_FIXTURE: &str = include_str!("fixtures/work-item-plan-rep4.md");

    /// bug45/t25 kimi 形态：WI-001 forbidden 写域含 `src/lib.rs`，AC 反引号
    /// 引用 `src/lib.rs`、验证命令 token 引用 `src/lib.rs`。
    fn self_conflicting_candidate() -> String {
        wi001_only()
            .replacen("- forbidden_scopes: web/**", "- forbidden_scopes: src/lib.rs", 1)
            .replacen(
                "- statement: WHEN GET /api/levels is requested THE SYSTEM SHALL return the configured levels JSON.",
                "- statement: WHEN the levels API ships THE SYSTEM SHALL review the new pub docs in `src/lib.rs` and return the configured levels JSON.",
                1,
            )
            .replacen(
                "- command: cargo test --locked --lib levels_api",
                "- command: rustdoc src/lib.rs",
                1,
            )
            .trim_end()
            .to_string()
                + "\n"
    }

    /// 放行面基线：仅改 forbidden 与引用路径（引用落在域外），其余同
    /// self_conflicting_candidate。
    fn legal_reference_candidate() -> String {
        wi001_only()
            .replacen("- forbidden_scopes: web/**", "- forbidden_scopes: src/lib.rs", 1)
            .replacen(
                "- statement: WHEN GET /api/levels is requested THE SYSTEM SHALL return the configured levels JSON.",
                "- statement: WHEN the levels API ships THE SYSTEM SHALL review the docs in `docs/levels.md` and return the configured levels JSON.",
                1,
            )
            .replacen(
                "- command: cargo test --locked --lib levels_api",
                "- command: node --test test/levels.test.js",
                1,
            )
            .trim_end()
            .to_string()
                + "\n"
    }

    /// 仅取 REP4 的 WI-001 段（单工作项候选，preflight_baseline_paths 先例）。
    fn wi001_only() -> String {
        REP4_FIXTURE
            .split("## Work Item WI-002:")
            .next()
            .expect("fixture 必须包含 WI-001")
            .to_string()
    }

    struct ValidationFixture {
        story_ids: Vec<String>,
        design_ids: Vec<String>,
        options: IssueWorkItemPlanOptions,
    }

    impl ValidationFixture {
        fn new() -> Self {
            Self {
                story_ids: vec!["story_spec_levels_0001".to_string()],
                design_ids: vec!["design_spec_levels_0001".to_string()],
                options: IssueWorkItemPlanOptions {
                    include_integration_tests: false,
                    include_e2e_tests: false,
                    force_frontend_backend_split: false,
                    require_execution_plan_confirm: false,
                },
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
                existing_work_item_ids: &[],
                enrollment_target: None,
                member_targets: None,
                member_checkout_prefixes: None,
                now: "2026-10-21T00:00:00Z",
            }
        }
    }

    fn compile_candidate(source: &str) -> crate::product::work_item_plan_compiler::PlanCandidateIr {
        compile_work_item_plan(
            source,
            &WorkItemPlanSourceContext {
                target_repository_id: "repo-levels".to_string(),
                member_targets: None,
            },
        )
        .expect("candidate must lower to typed IR")
    }

    /// 冲突正例：AC 反引号路径与验证命令路径都落在本项 forbidden 写域内 →
    /// Error finding 留在报告里（preflight 族回灌修订，不硬失败 author 轮），
    /// 文案指名工作项、检查（AC/CHECK）、冲突路径与命中 scope。
    #[test]
    fn check_target_inside_own_forbidden_write_scope_is_blocked() {
        let ir = compile_candidate(&self_conflicting_candidate());
        let fixture = ValidationFixture::new();
        let report = validate_plan_candidate_ir(&ir, &fixture.context())
            .expect("自洽缺口必须留在报告里（机械回灌修订），不得硬失败 author 轮");
        let finding = report
            .findings
            .iter()
            .find(|finding| finding.code == "work_item_check_target_forbidden")
            .expect("检查目标×本项 forbidden 写域冲突必须产出显式 finding");
        assert_eq!(finding.severity, WorkItemSplitFindingSeverity::Error);
        assert_eq!(finding.work_item_ids, vec!["WI-001".to_string()]);
        for required in [
            "WI-001",
            "AC-001 → src/lib.rs（forbidden: src/lib.rs）",
            "CHECK-001 → src/lib.rs（forbidden: src/lib.rs）",
        ] {
            assert!(
                finding.message.contains(required),
                "finding 文案必须指名 {required}：{}",
                finding.message
            );
        }
        assert!(
            finding
                .message
                .ends_with(WORK_ITEM_CHECK_TARGET_FORBIDDEN_REPAIR_ACTION),
            "finding 消息与共享常量逐字同源（REQ-WSC-06 口径一致）：{}",
            finding.message
        );
    }

    /// 冲突正例（glob 写域）：forbidden `src/**` 覆盖 `src/lib.rs` 同样被拦
    ///（scope 匹配复用仓内权威 scope_allows_path，非仅字面相等）。
    #[test]
    fn glob_forbidden_scope_covers_check_target() {
        let source = self_conflicting_candidate().replacen(
            "- forbidden_scopes: src/lib.rs",
            "- forbidden_scopes: src/**",
            1,
        );
        let ir = compile_candidate(&source);
        let fixture = ValidationFixture::new();
        let report = validate_plan_candidate_ir(&ir, &fixture.context())
            .expect("preflight 族缺口留在报告里回灌修订");
        assert!(
            report
                .findings
                .iter()
                .any(|finding| finding.code == "work_item_check_target_forbidden"
                    && finding.message.contains("src/lib.rs")),
            "glob 写域 src/** 覆盖的检查目标必须被拦：{:?}",
            report.findings
        );
    }

    /// 人工检查面：manual_instruction 的反引号路径落在本项 forbidden 内同样
    /// 被拦（检查目标不因载体是人工说明而豁免自洽性）。
    #[test]
    fn manual_instruction_quoted_path_inside_own_forbidden_is_blocked() {
        let source = wi001_only()
            .replacen(
                "- forbidden_scopes: web/**",
                "- forbidden_scopes: src/lib.rs",
                1,
            )
            .replacen(
                "- manual_instruction: Confirm the endpoint returns the configured levels JSON.",
                "- manual_instruction: Confirm the pub docs in `src/lib.rs` cover the new items.",
                1,
            )
            .trim_end()
            .to_string()
            + "\n";
        let ir = compile_candidate(&source);
        let fixture = ValidationFixture::new();
        let report = validate_plan_candidate_ir(&ir, &fixture.context())
            .expect("preflight 族缺口留在报告里回灌修订");
        assert!(
            report
                .findings
                .iter()
                .any(|finding| finding.code == "work_item_check_target_forbidden"
                    && finding.message.contains("CHECK-001")
                    && finding.message.contains("src/lib.rs")),
            "manual_instruction 的 forbidden 路径引用必须被拦：{:?}",
            report.findings
        );
    }

    /// 放行面：检查目标落在本项 forbidden 写域之外 → 不报（读其他文件合法）。
    #[test]
    fn legal_reference_outside_own_forbidden_produces_no_finding() {
        let ir = compile_candidate(&legal_reference_candidate());
        let fixture = ValidationFixture::new();
        let report =
            validate_plan_candidate_ir(&ir, &fixture.context()).expect("合法候选不得因新规则失败");
        assert!(
            report
                .findings
                .iter()
                .all(|finding| finding.code != "work_item_check_target_forbidden"),
            "检查目标不在本项 forbidden 写域内不得误报：{:?}",
            report.findings
        );
    }

    /// 放行面：整仓命令（无路径 token）与裸 prose（无引号/方法前缀）一律
    /// 不报（受限提取纪律，ac_paths 同一形态规则）。
    #[test]
    fn whole_repo_commands_and_bare_prose_produce_no_finding() {
        let ir = compile_candidate(&wi001_only());
        let fixture = ValidationFixture::new();
        let report = validate_plan_candidate_ir(&ir, &fixture.context())
            .expect("REP4 WI-001 既有合法形态零回归");
        assert!(
            report
                .findings
                .iter()
                .all(|finding| finding.code != "work_item_check_target_forbidden"),
            "整仓命令与裸 prose 不得触发新 finding：{:?}",
            report.findings
        );
    }

    /// 放行面（跨工作项引用）：路径只落在**别的**工作项 forbidden 写域内
    ///（WI-001 引用 `src/product/levels/api.rs` 属 WI-002 的 forbidden，不属
    /// WI-001 自己的）→ 不报；全量 REP4 候选同样零新 finding（既有合法用例
    /// 回归：WI-002/WI-003 的 AC 均为裸 prose、命令均无路径 token）。
    #[test]
    fn cross_work_item_reference_and_full_rep4_produce_no_finding() {
        let cross_wi = REP4_FIXTURE.replacen(
            "- statement: WHEN GET /api/levels is requested THE SYSTEM SHALL return the configured levels JSON.",
            "- statement: WHEN the levels API ships THE SYSTEM SHALL keep `src/product/levels/api.rs` documented and return the configured levels JSON.",
            1,
        );
        for source in [cross_wi.as_str(), REP4_FIXTURE] {
            let ir = compile_candidate(source);
            let fixture = ValidationFixture::new();
            let report = validate_plan_candidate_ir(&ir, &fixture.context())
                .unwrap_or_else(|diagnostics| panic!("合法候选不得硬失败：{diagnostics:?}"));
            assert!(
                report
                    .findings
                    .iter()
                    .all(|finding| finding.code != "work_item_check_target_forbidden"),
                "跨工作项引用/既有 REP4 合法用例不得误报：{:?}",
                report.findings
            );
        }
    }
}
