//! C1 Task 2：AC×基线树核对（REQ-WSC-02 场景 13，F-56）。
//!
//! F-56 根因：issue_0002 的 AC-023 引用兄弟分支 issue_0001 才有的
//! `status.html`（statement 内 `GET /status.html` URL 路径形态），缺口直到
//! final confirm 的 exclusive 覆盖门才代偿性暴露。本文件锁定新契约：候选
//! 校验期以受限路径提取（方法前缀路由 / 引号与反引号 span / 命令 token）
//! × plan 基线树交叉核对，缺失即产出 Error 级 finding（附路径清单与三条
//! 修复建议），属 preflight 族（机械回灌修订，不把 author 轮硬失败）；
//! 无扩展名路由（`/api/status`）与裸 prose 一律不触发（防误报）；
//! 基线不可用（None）不触发核对（fail-safe）。

use super::*;
use crate::product::models::{IssueWorkItemPlanOptions, WorkItemSplitFindingSeverity};
use crate::product::work_item_plan_compiler::{
    PlanCandidateValidationContext, validate_plan_candidate_ir,
};
use std::collections::BTreeSet;

/// F-56 现场原文（issue_0002 / source-109fb57a7629868c / AC-023）：
/// `GET /api/status`（无扩展名，必须不触发）与 `GET /status.html`
/// （兄弟分支才有的基线文件，必须被拦）同句出现。
const F56_AC_STATEMENT: &str = "WHEN 请求 GET /api/status 与 GET /status.html 与未知路径及非 GET 请求 THE SYSTEM SHALL 分别返回既有 200 JSON 与既有 200 text/html 与 404 text/plain; charset=utf-8。";

/// 单 WI backend 候选：AC 换成 F-56 原文，验证命令带仓库内测试文件路径
/// （`node --test test/status.test.js`），任务语句保留 `GET /api/levels`
/// 无扩展名路由作为反例。
fn f56_candidate() -> String {
    let source = REP4_FIXTURE
        .split("## Work Item WI-002:")
        .next()
        .expect("fixture 必须包含 WI-001");
    source
        .replacen(
            "- statement: WHEN GET /api/levels is requested THE SYSTEM SHALL return the configured levels JSON.",
            &format!("- statement: {F56_AC_STATEMENT}"),
            1,
        )
        .replacen(
            "- command: cargo test --locked --lib levels_api",
            "- command: node --test test/status.test.js",
            1,
        )
        .replacen("- provided_contract_refs: contract.levels-api", "- provided_contract_refs: []", 1)
        .trim_end()
        .to_string()
        + "\n"
}

struct BaselineFixture {
    story_ids: Vec<String>,
    design_ids: Vec<String>,
    options: IssueWorkItemPlanOptions,
    baseline: Option<BTreeSet<String>>,
}

impl BaselineFixture {
    fn new(baseline: Option<BTreeSet<String>>) -> Self {
        Self {
            story_ids: vec!["story_spec_levels_0001".to_string()],
            design_ids: vec!["design_spec_levels_0001".to_string()],
            options: IssueWorkItemPlanOptions {
                include_integration_tests: false,
                include_e2e_tests: false,
                force_frontend_backend_split: false,
                require_execution_plan_confirm: false,
            },
            baseline,
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
            baseline_tree: self.baseline.as_ref(),
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

/// REQ-WSC-02 场景 13：F-56 真实形态被拦——`GET /status.html` 产出 Error
/// finding（preflight 族，留在报告里回灌修订），路径清单含 status.html、
/// 不含无扩展名路由 api/status，消息附三条修复建议。
#[test]
fn f56_route_reference_outside_baseline_is_blocked_with_repair_advice() {
    let ir = compile_candidate(&f56_candidate());
    let fixture = BaselineFixture::new(Some(
        ["package.json", "server.js"]
            .map(str::to_string)
            .into_iter()
            .collect(),
    ));
    let report = validate_plan_candidate_ir(&ir, &fixture.context())
        .expect("AC 路径×基线树缺口必须留在报告里（机械回灌修订），不得硬失败 author 轮");

    let finding = report
        .findings
        .iter()
        .find(|finding| finding.code == "acceptance_path_not_in_baseline")
        .expect("基线外路径引用必须产出显式 finding");
    assert_eq!(finding.severity, WorkItemSplitFindingSeverity::Error);
    assert_eq!(finding.work_item_ids, vec!["WI-001".to_string()]);
    assert!(
        finding.message.contains("status.html"),
        "finding 必须列出缺失路径：{}",
        finding.message
    );
    assert!(
        !finding.message.contains("api/status"),
        "无扩展名路由不是文件路径，不得误报：{}",
        finding.message
    );
    assert!(
        finding.message.contains("删除"),
        "修复建议一（删除该 AC）：{}",
        finding.message
    );
    assert!(
        finding.message.contains("基线树"),
        "修复建议二（改用基线内路径）：{}",
        finding.message
    );
    assert!(
        finding.message.contains("exclusive_scopes"),
        "修复建议三（声明基线恢复依赖并纳入写范围）：{}",
        finding.message
    );
}

/// REQ-WSC-02 场景 13 放行面：引用路径都在基线树内时不产出 finding。
#[test]
fn baseline_covered_references_do_not_produce_findings() {
    let ir = compile_candidate(&f56_candidate());
    let fixture = BaselineFixture::new(Some(
        [
            "package.json",
            "server.js",
            "status.html",
            "test/status.test.js",
        ]
        .map(str::to_string)
        .into_iter()
        .collect(),
    ));
    let report = validate_plan_candidate_ir(&ir, &fixture.context()).expect("基线内引用必须放行");
    assert!(
        report
            .findings
            .iter()
            .all(|finding| finding.code != "acceptance_path_not_in_baseline"),
        "基线内路径不得产出 finding：{:?}",
        report.findings
    );
}

/// 防误报负例（受限提取纪律）：非路径文本——反引号 span 中的词法名
/// （`CommonJS`）、下划线标识符（`require_all`）、带冒号的 URL、裸 prose
/// （无引号无方法前缀）、flag（`--test`）与 `$` 变量——一律不触发。
#[test]
fn non_path_text_is_not_extracted() {
    let base = f56_candidate();
    let source = base
        .replacen(
            F56_AC_STATEMENT,
            "WHEN 集成运行 `CommonJS` 与 `require_all` 及 \"https://example.com/x.js\" 与 'node:http' 同用 THE SYSTEM SHALL 保持 main.js 于裸 prose 与 `--test` 与 `$PATH` 语义。",
            1,
        )
        .replacen(
            "- command: node --test test/status.test.js",
            "- command: node --test",
            1,
        );
    let ir = compile_candidate(&source);
    let fixture = BaselineFixture::new(Some(BTreeSet::new()));
    let report =
        validate_plan_candidate_ir(&ir, &fixture.context()).expect("非路径文本不得触发核对失败");
    assert!(
        report
            .findings
            .iter()
            .all(|finding| finding.code != "acceptance_path_not_in_baseline"),
        "URL/词法名/flag/变量/裸 prose 不得误报：{:?}",
        report.findings
    );
}

/// REQ-WSC-02 场景 13 验证计划面：trusted command 中的仓库内文件路径
/// （`node --test test/status.test.js`）同样参与基线树核对。
#[test]
fn trusted_command_path_outside_baseline_is_blocked() {
    let ir = compile_candidate(&f56_candidate());
    // AC 面放行（status.html 在基线内），命令面缺失（test/status.test.js 不在）。
    let fixture = BaselineFixture::new(Some(
        ["status.html"].map(str::to_string).into_iter().collect(),
    ));
    let report = validate_plan_candidate_ir(&ir, &fixture.context())
        .expect("preflight 族缺口留在报告里回灌修订");
    let finding = report
        .findings
        .iter()
        .find(|finding| finding.code == "acceptance_path_not_in_baseline")
        .expect("命令中的基线外路径必须产出 finding");
    assert!(
        finding.message.contains("test/status.test.js"),
        "finding 必须列出命令引用的缺失路径：{}",
        finding.message
    );
}

/// 反引号 span 的正例：引号内的仓库相对路径（可带 GET 前缀）被提取核对。
#[test]
fn quoted_span_reference_outside_baseline_is_blocked() {
    let base = f56_candidate();
    let source = base.replacen(
        F56_AC_STATEMENT,
        "WHEN 请求 `web/assets/status.html` 与 \"GET /api/legacy.json\" THE SYSTEM SHALL 返回既有内容。",
        1,
    );
    let ir = compile_candidate(&source);
    let fixture = BaselineFixture::new(Some(
        ["status.html", "test/status.test.js"]
            .map(str::to_string)
            .into_iter()
            .collect(),
    ));
    let report = validate_plan_candidate_ir(&ir, &fixture.context())
        .expect("preflight 族缺口留在报告里回灌修订");
    let finding = report
        .findings
        .iter()
        .find(|finding| finding.code == "acceptance_path_not_in_baseline")
        .expect("引号 span 内的基线外路径必须产出 finding");
    assert!(
        finding.message.contains("web/assets/status.html"),
        "反引号 span 路径必须被提取：{}",
        finding.message
    );
    assert!(
        finding.message.contains("api/legacy.json"),
        "双引号 span（带 GET 前缀）路径必须被提取：{}",
        finding.message
    );
}

/// `baseline_tree=None`（issue 无仓，逻辑代码库 Non-Goal 面）不触发 AC 路径
/// 核对。注意（REQ-PIB-03 软限制 pivot）：加载器 `plan_baseline_tree` 的
/// 「不可解析 → None 跳过」fail-safe 已废弃——基线不可解析现在为 Err
/// fail-closed（见 workspace_engine::plan_preflight 测试），本用例仅钉住
/// 校验器层对 None 的既有契约（三族 options 预检不受影响）。
#[test]
fn unavailable_baseline_skips_acceptance_path_check() {
    let ir = compile_candidate(&f56_candidate());
    let fixture = BaselineFixture::new(None);
    let report =
        validate_plan_candidate_ir(&ir, &fixture.context()).expect("基线不可用时不得新增失败面");
    assert!(
        report
            .findings
            .iter()
            .all(|finding| finding.code != "acceptance_path_not_in_baseline"),
        "基线不可用（None）时不得产出 AC 路径 finding：{:?}",
        report.findings
    );
}
