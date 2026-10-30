//! 写范围 × 被授权仓归属核对（REQ-WSC-10，owner 2026-10-30 真机现场）。
//!
//! 现场形态：多仓（LC 聚合根）plan 会话中 WI-004 的 exclusive_scopes 声明
//! `scripts/dev.sh`——该路径位于聚合根本地文件区（根目录不是 git 仓库，仅
//! 各成员仓入库），任何成员仓的执行期写入都到不了。本文件锁定新契约：
//! 候选校验期逐 WI 判写面归属（只判归属不判存在），越界产出稳定错误码
//! `write_scope_outside_bound_repository` 的 Error finding（preflight 族，
//! 机械回灌修订，不硬失败 author 轮）；成员 checkout 前缀表缺席（单仓/
//! Manual）不触发；表在场而被授权仓不在表内 fail-closed。

use super::*;
use crate::product::logical_codebase::{EnrollmentTarget, LogicalRepositoryId};
use crate::product::models::IssueWorkItemPlanOptions;
use crate::product::work_item_plan_compiler::{
    PlanCandidateValidationContext, PlanMemberTargets,
    WRITE_SCOPE_OUTSIDE_BOUND_REPOSITORY_REPAIR_ACTION, validate_plan_candidate_ir,
};
use std::collections::{BTreeMap, BTreeSet};

const AGGREGATE_VIEW_ID: &str = "lc_aggregate_root_view_00000000-0000-4000-8000-0000000000aa";
const BOUND_CODEBASE_ID: &str = "lc-levels";

fn logical(id: &str) -> LogicalRepositoryId {
    LogicalRepositoryId(id.parse().expect("logical member uuid"))
}

/// 与 member_targets.rs 同源 fixture：0001→phys-busi、0002→phys-web、
/// 0003→phys-int（WI-003 声明 logical_codebase→0003）。
fn member_targets_fixture() -> PlanMemberTargets {
    let mut physical_by_logical = BTreeMap::new();
    physical_by_logical.insert(
        logical("00000000-0000-4000-8000-000000000001"),
        "phys-busi".to_string(),
    );
    physical_by_logical.insert(
        logical("00000000-0000-4000-8000-000000000002"),
        "phys-web".to_string(),
    );
    physical_by_logical.insert(
        logical("00000000-0000-4000-8000-000000000003"),
        "phys-int".to_string(),
    );
    PlanMemberTargets {
        physical_by_logical,
        fallback_physical_id: None,
    }
}

/// 成员 checkout 前缀表：phys-busi→busi、phys-web→web、phys-int→integration。
fn checkout_prefixes_fixture() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("phys-busi".to_string(), "busi".to_string()),
        ("phys-web".to_string(), "web".to_string()),
        ("phys-int".to_string(), "integration".to_string()),
    ])
}

/// 三 WI 多仓源：WI-001→phys-busi、WI-002→phys-web（logical 0002）、
/// WI-003→phys-int；写面全部落在各自被授权成员 checkout 内（含新建路径）。
fn bound_source() -> String {
    REP4_FIXTURE
        .replacen(
            "- intent_target_kind: single_repository\n- intent_target_repo: repo-levels",
            "- intent_target_kind: single_repository\n- intent_target_repo: phys-busi",
            1,
        )
        .replacen(
            "- intent_target_kind: single_repository\n- intent_target_repo: repo-levels",
            "- intent_target_kind: logical_codebase\n- intent_target_codebase: lc-levels\n- intent_target_logical_repo: 00000000-0000-4000-8000-000000000002",
            1,
        )
        .replacen(
            "- exclusive_scopes: src/product/levels/**",
            "- exclusive_scopes: busi/src/new_feature/**",
            1,
        )
        .replacen(
            "- exclusive_scopes: tests/integration/**",
            "- exclusive_scopes: integration/tests/**",
            1,
        )
        .trim_end()
        .to_string()
        + "\n"
}

fn bound_set_target() -> EnrollmentTarget {
    EnrollmentTarget::LogicalCodebaseMembers {
        logical_codebase_id: BOUND_CODEBASE_ID.to_string(),
        logical_repository_ids: vec![
            logical("00000000-0000-4000-8000-000000000001"),
            logical("00000000-0000-4000-8000-000000000002"),
            logical("00000000-0000-4000-8000-000000000003"),
        ],
    }
}

fn options() -> IssueWorkItemPlanOptions {
    IssueWorkItemPlanOptions {
        include_integration_tests: false,
        include_e2e_tests: false,
        force_frontend_backend_split: false,
        require_execution_plan_confirm: false,
    }
}

fn compile_bound(source: &str) -> crate::product::work_item_plan_compiler::PlanCandidateIr {
    compile_work_item_plan(
        source,
        &WorkItemPlanSourceContext {
            target_repository_id: AGGREGATE_VIEW_ID.to_string(),
            member_targets: Some(member_targets_fixture()),
        },
    )
    .expect("多仓源必须 lower 成功")
}

fn validate_bound(
    source: &str,
    member_checkout_prefixes: Option<&BTreeMap<String, String>>,
) -> crate::product::work_item_plan_compiler::types::PlanCandidateMechanicalReport {
    let ir = compile_bound(source);
    let member_targets = member_targets_fixture();
    let bound = bound_set_target();
    let story_ids = vec!["story_spec_levels_0001".to_string()];
    let design_ids = vec!["design_spec_levels_0001".to_string()];
    let plan_options = options();
    let baseline: Option<BTreeSet<String>> = None;
    validate_plan_candidate_ir(
        &ir,
        &PlanCandidateValidationContext {
            project_id: "project_levels_0001",
            issue_id: "issue_levels_0001",
            plan_id: "plan_levels_0001",
            source_story_spec_ids: &story_ids,
            source_design_spec_ids: &design_ids,
            repository_profile: None,
            plan_options: &plan_options,
            baseline_tree: baseline.as_ref(),
            existing_work_item_ids: &[],
            enrollment_target: Some(&bound),
            member_targets: Some(&member_targets),
            member_checkout_prefixes,
            now: "2026-10-30T00:00:00Z",
        },
    )
    .expect("归属缺口属 preflight 族，必须留在报告里（机械回灌修订）")
}

fn bound_findings<'a>(
    report: &'a crate::product::work_item_plan_compiler::types::PlanCandidateMechanicalReport,
    work_item_id: &str,
) -> Vec<&'a crate::product::models::WorkItemSplitFinding> {
    report
        .findings
        .iter()
        .filter(|finding| {
            finding.code == "write_scope_outside_bound_repository"
                && finding.work_item_ids == [work_item_id.to_string()]
        })
        .collect()
}

/// 用例 1（owner 现场原文形态）：WI-004→WI-001 同构——被授权 busi 的 WI
/// 声明聚合根非成员区路径 `scripts/dev.sh` → Error finding 点名路径与非成员
/// 区归属事实，附修复建议；preflight 族不硬失败。
#[test]
fn non_member_area_scope_is_blocked_with_stable_code() {
    let source = bound_source().replacen(
        "- exclusive_scopes: busi/src/new_feature/**",
        "- exclusive_scopes: scripts/dev.sh",
        1,
    );
    let report = validate_bound(&source, Some(&checkout_prefixes_fixture()));

    let findings = bound_findings(&report, "WI-001");
    assert_eq!(
        findings.len(),
        1,
        "非成员区写面必须产出恰一条指向 WI-001 的 finding：{findings:?}"
    );
    let finding = findings[0];
    assert_eq!(
        finding.severity,
        crate::product::models::WorkItemSplitFindingSeverity::Error
    );
    assert!(
        finding.message.contains("scripts/dev.sh"),
        "{}",
        finding.message
    );
    assert!(
        finding.message.contains("非成员区"),
        "消息必须点名聚合根非成员区归属事实：{}",
        finding.message
    );
    assert!(
        finding
            .message
            .contains(WRITE_SCOPE_OUTSIDE_BOUND_REPOSITORY_REPAIR_ACTION),
        "消息必须附共享修复建议（REQ-WSC-06 同源）：{}",
        finding.message
    );
}

/// 用例 2（跨成员写）：被授权 busi 的 WI 声明 `web/assets/**`（落在另一
/// 成员 phys-web 的 checkout）→ 同码 Error finding，消息点名实际归属成员
/// checkout 与本项被授权仓（既有授权语义：每 WI 恰落一被授权成员仓）。
#[test]
fn cross_member_scope_is_blocked_naming_owning_checkout() {
    let source = bound_source().replacen(
        "- exclusive_scopes: busi/src/new_feature/**",
        "- exclusive_scopes: web/assets/**",
        1,
    );
    let report = validate_bound(&source, Some(&checkout_prefixes_fixture()));

    let findings = bound_findings(&report, "WI-001");
    assert_eq!(
        findings.len(),
        1,
        "跨成员写面必须产出恰一条指向 WI-001 的 finding：{findings:?}"
    );
    let message = &findings[0].message;
    assert!(message.contains("web/assets/**"), "{message}");
    assert!(
        message.contains("web") && message.contains("phys-web"),
        "消息必须点名写面实际落在的成员 checkout 与归属物理仓：{message}"
    );
    assert!(
        message.contains("phys-busi"),
        "消息必须点名本项被授权仓：{message}"
    );
    assert!(
        message.contains(WRITE_SCOPE_OUTSIDE_BOUND_REPOSITORY_REPAIR_ACTION),
        "{message}"
    );
}

/// 用例 3（只判归属不判存在）：被授权 checkout 内的新建路径（基线树不可用、
/// 路径不存在）不误伤；成员内既有写法（`web/src/levels/**`、目录前缀形
/// `busi`）同样放行；整份多仓候选零本码 finding。
#[test]
fn new_paths_inside_bound_checkout_are_not_blocked() {
    let source = bound_source().replacen(
        "- exclusive_scopes: busi/src/new_feature/**",
        "- exclusive_scopes: busi/src/new_feature/**\n- exclusive_scopes: busi",
        1,
    );
    // 基线树不可用（None）时也放行——本规则不消费基线树。
    let report = validate_bound(&source, Some(&checkout_prefixes_fixture()));

    assert!(
        report
            .findings
            .iter()
            .all(|finding| finding.code != "write_scope_outside_bound_repository"),
        "被授权 checkout 内的写面（含新建与目录前缀形）一律放行：{:?}",
        report.findings
    );
}

/// 用例 4（前缀表缺席零回归）：单仓/Manual 会话（member_checkout_prefixes
/// 为 None）即使声明非成员区写面也不触发本规则——与 enrollment target
/// 缺席时既有核对不触发的惯例同构。
#[test]
fn absent_prefix_map_keeps_single_repo_behavior_unchanged() {
    let source = bound_source().replacen(
        "- exclusive_scopes: busi/src/new_feature/**",
        "- exclusive_scopes: scripts/dev.sh",
        1,
    );
    let report = validate_bound(&source, None);

    assert!(
        report
            .findings
            .iter()
            .all(|finding| finding.code != "write_scope_outside_bound_repository"),
        "前缀表缺席时本规则不得触发（零回归）：{:?}",
        report.findings
    );
}

/// 用例 5（映射缺失 fail-closed）：前缀表在场而被授权仓不在表内（成员归属
/// 映射缺失/漂移）→ Error finding 说明「无法核对归属」，不以跳过吞错。
#[test]
fn bound_repository_missing_from_prefix_map_fails_closed() {
    let mut prefixes = checkout_prefixes_fixture();
    assert!(prefixes.remove("phys-busi").is_some());
    let source = bound_source();
    let report = validate_bound(&source, Some(&prefixes));

    let findings = bound_findings(&report, "WI-001");
    assert_eq!(
        findings.len(),
        1,
        "被授权仓不在前缀表内必须 fail-closed 产出 finding：{findings:?}"
    );
    let message = &findings[0].message;
    assert!(
        message.contains("phys-busi") && message.contains("无法核对"),
        "消息必须点名缺失的被授权仓与 fail-closed 理由：{message}"
    );
}

/// 无固定路径锚点的 scope（`*`）不触发本规则（无确定路径域可判归属）。
/// 空 exclusive 列表无法经 grammar 表达（Write Policy 必填字段，缺失即
/// missing_section 拒收），实现侧以 scopes.is_empty() 早退兜底。
#[test]
fn unanchored_scopes_do_not_trigger() {
    let single_wi = bound_source()
        .split("## Work Item WI-002:")
        .next()
        .expect("fixture 必须包含 WI-001")
        .trim_end()
        .to_string()
        + "\n";
    let source = single_wi.replacen(
        "- exclusive_scopes: busi/src/new_feature/**",
        "- exclusive_scopes: *",
        1,
    );
    let report = validate_bound(&source, Some(&checkout_prefixes_fixture()));
    assert!(
        report
            .findings
            .iter()
            .all(|finding| finding.code != "write_scope_outside_bound_repository"),
        "scope `*` 无固定锚点不得触发本规则：{:?}",
        report.findings
    );
}

/// 用例（F2，仓内相对写法）：绑 busi 的 WI 声明 `src/lib.rs`（仓内相对
/// 写法，运行期 `scope_allows_member_path` 兼容放行）。plan 期归属核对按
/// 聚合根基准拦截（与运行期兼容语义的分工），但 finding 文案 MUST 说清
/// 两种可能归属与改写指引（加成员目录前缀），不得武断说成「非成员区」
/// ——「看不懂/看错的话」正是本 change 立项要治的病。
#[test]
fn repo_relative_scope_is_blocked_with_rewrite_guidance_not_misclassified() {
    let source = bound_source().replacen(
        "- exclusive_scopes: busi/src/new_feature/**",
        "- exclusive_scopes: src/lib.rs",
        1,
    );
    let report = validate_bound(&source, Some(&checkout_prefixes_fixture()));

    let findings = bound_findings(&report, "WI-001");
    assert_eq!(
        findings.len(),
        1,
        "仓内相对写法在 plan 期按聚合根基准判越界（运行期才兼容）：{findings:?}"
    );
    let message = &findings[0].message;
    assert!(
        message.contains("src/lib.rs"),
        "消息必须点名该写范围：{message}"
    );
    assert!(
        message.contains("仓内相对"),
        "文案必须区分「仓内相对写法」的可能归属（F2）：{message}"
    );
    assert!(
        message.contains("busi/src"),
        "文案必须给出带成员目录前缀的改写示例（如 busi/src/...）：{message}"
    );
}

/// 用例（F4，`..` 相对段）：`busi/../scripts/dev.sh` 字面前缀命中 busi 但
/// 实际落点是聚合根非成员区——机械核对不得被字面前缀绕过，含 `..` 段
/// 一律 fail-closed 出 finding（不展开、不猜）。
#[test]
fn dot_dot_segments_are_rejected_fail_closed() {
    let source = bound_source().replacen(
        "- exclusive_scopes: busi/src/new_feature/**",
        "- exclusive_scopes: busi/../scripts/dev.sh",
        1,
    );
    let report = validate_bound(&source, Some(&checkout_prefixes_fixture()));

    let findings = bound_findings(&report, "WI-001");
    assert_eq!(
        findings.len(),
        1,
        "含 .. 相对段的写范围必须被拦，不得以字面前缀放行：{findings:?}"
    );
    let message = &findings[0].message;
    assert!(
        message.contains("busi/../scripts/dev.sh"),
        "消息必须点名原始写范围：{message}"
    );
    assert!(
        message.contains(".."),
        "文案必须说明 .. 段无法机械判归属：{message}"
    );
}

/// 用例（顿号方言在归属核对下的表现）：同串并存本仓段与他仓段
///（`busi/src/**、web/src/**`）——运行期只命中本仓段，plan 期归属核对
/// 对他仓段逐段点名：恰一条 finding、含 web/src/**、不含 busi/src 段
/// 条目（本仓段放行，不重复报）。
#[test]
fn dun_dialect_reports_only_foreign_segments() {
    let source = bound_source().replacen(
        "- exclusive_scopes: busi/src/new_feature/**",
        "- exclusive_scopes: busi/src/**、web/src/**",
        1,
    );
    let report = validate_bound(&source, Some(&checkout_prefixes_fixture()));

    let findings = bound_findings(&report, "WI-001");
    assert_eq!(
        findings.len(),
        1,
        "顿号同串的本仓+他仓段必须聚合为一条 finding：{findings:?}"
    );
    let message = &findings[0].message;
    assert!(message.contains("web/src/**"), "{message}");
    assert!(
        !message.contains("busi/src/**（"),
        "本仓段（busi/src/**）必须放行、不得进越界清单：{message}"
    );
}
