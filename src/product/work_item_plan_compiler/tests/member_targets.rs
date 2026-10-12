//! B1 铁律（多仓 plan 聚合视图缺口）：聚合根视图 ID 不得流入 IR target。
//! 多仓 plan 会话（member_targets 在场）每 WI 按 Plan Intent 成员归属落
//! 成员物理仓 ID；无归属 WI 按确定性回落（focus/首仓），回落缺失或归属
//! 失效 → `work_item_target_missing` 语义错 fail-closed；member_targets
//! 缺席（legacy 单仓）语义零变化。

use super::*;
use crate::product::logical_codebase::LogicalRepositoryId;
use crate::product::work_item_plan_compiler::{PlanCandidateIr, PlanMemberTargets};

const AGGREGATE_VIEW_ID: &str = "lc_aggregate_root_view_00000000-0000-4000-8000-0000000000aa";

fn logical(id: &str) -> LogicalRepositoryId {
    LogicalRepositoryId(id.parse().expect("logical member uuid"))
}

/// 成员 0001→phys-busi、0002→phys-web、0003→phys-int（rep4 fixture 的
/// WI-003 已声明 logical_codebase→0003）。
fn member_targets_fixture(fallback: Option<&str>) -> PlanMemberTargets {
    let mut physical_by_logical = std::collections::BTreeMap::new();
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
        fallback_physical_id: fallback.map(str::to_string),
    }
}

fn aggregate_context(member_targets: Option<PlanMemberTargets>) -> WorkItemPlanSourceContext {
    WorkItemPlanSourceContext {
        target_repository_id: AGGREGATE_VIEW_ID.to_string(),
        member_targets,
    }
}

fn owned_source() -> String {
    // WI-001：single_repository 归属成员 phys-busi；WI-002：logical_codebase
    // 归属成员 0002；WI-003 保持 fixture 自带的 logical_codebase→0003。
    REP4_FIXTURE.replacen(
        "- intent_target_kind: single_repository\n- intent_target_repo: repo-levels",
        "- intent_target_kind: single_repository\n- intent_target_repo: phys-busi",
        1,
    )
    .replacen(
        "- intent_target_kind: single_repository\n- intent_target_repo: repo-levels",
        "- intent_target_kind: logical_codebase\n- intent_target_codebase: lc-levels\n- intent_target_logical_repo: 00000000-0000-4000-8000-000000000002",
        1,
    )
}

fn unowned_wi002_source() -> String {
    // WI-002 无 Plan Intent（无成员归属声明）。
    owned_source().replacen(
        "### Plan Intent\n- intent: create\n- provider_work_item_id: WI-001\n- intent_target_kind: logical_codebase\n- intent_target_codebase: lc-levels\n- intent_target_logical_repo: 00000000-0000-4000-8000-000000000002\n\n",
        "",
        1,
    )
}

fn targets_of(ir: &PlanCandidateIr) -> Vec<&str> {
    ir.items
        .iter()
        .map(|item| item.target_repository_id.as_str())
        .collect()
}

#[test]
fn member_targets_land_per_work_item_member_physical_repositories() {
    let ir = compile_work_item_plan(
        &owned_source(),
        &aggregate_context(Some(member_targets_fixture(None))),
    )
    .expect("per-WI 成员归属必须 lower 成功");
    assert_eq!(
        targets_of(&ir),
        vec!["phys-busi", "phys-web", "phys-int"],
        "每 WI 的 IR target 必须是其归属成员的物理仓 ID"
    );
}

#[test]
fn member_targets_fallback_lands_for_unowned_work_item() {
    let ir = compile_work_item_plan(
        &unowned_wi002_source(),
        &aggregate_context(Some(member_targets_fixture(Some("phys-first")))),
    )
    .expect("无归属 WI 有确定性回落时必须 lower 成功");
    assert_eq!(
        targets_of(&ir),
        vec!["phys-busi", "phys-first", "phys-int"],
        "无归属 WI 必须落确定性回落物理仓（focus/首仓）"
    );
}

#[test]
fn member_targets_reject_unowned_work_item_without_fallback() {
    let diagnostics = compile_work_item_plan(
        &unowned_wi002_source(),
        &aggregate_context(Some(member_targets_fixture(None))),
    )
    .expect_err("无归属且无回落必须语义错 fail-closed");
    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "work_item_target_missing"
                && diagnostic.field == "WI-002"
                && diagnostic.message.contains("成员归属")
        }),
        "诊断必须指向无归属 WI 并说明成员归属缺失：{diagnostics:?}"
    );
}

#[test]
fn member_targets_reject_unknown_logical_member() {
    let source = owned_source().replace(
        "00000000-0000-4000-8000-000000000003",
        "00000000-0000-4000-8000-000000000009",
    );
    let diagnostics = compile_work_item_plan(
        &source,
        &aggregate_context(Some(member_targets_fixture(None))),
    )
    .expect_err("归属成员不在有效成员集必须语义错");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "work_item_target_missing"
                && diagnostic.field == "WI-003"),
        "诊断必须指向归属失效的 WI-003：{diagnostics:?}"
    );
}

#[test]
fn member_targets_reject_non_member_physical_repository() {
    let source = owned_source().replacen(
        "- intent_target_repo: phys-busi",
        "- intent_target_repo: repo-outsider",
        1,
    );
    let diagnostics = compile_work_item_plan(
        &source,
        &aggregate_context(Some(member_targets_fixture(None))),
    )
    .expect_err("声明非成员物理仓必须语义错");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "work_item_target_missing"
                && diagnostic.field == "WI-001"),
        "诊断必须指向声明非成员仓的 WI-001：{diagnostics:?}"
    );
}

#[test]
fn member_targets_absent_keeps_session_target_unchanged() {
    let ir = compile_work_item_plan(
        REP4_FIXTURE,
        &WorkItemPlanSourceContext {
            target_repository_id: "repo-levels".to_string(),
            member_targets: None,
        },
    )
    .expect("legacy 单仓语义零变化");
    assert_eq!(
        targets_of(&ir),
        vec!["repo-levels", "repo-levels", "repo-levels"],
        "member_targets 缺席时全部 WI 沿用会话 target（零回归）"
    );
}

#[test]
fn aggregate_view_id_never_lands_in_ir_target_without_member_targets() {
    // B1 兜底守卫：即使调用方漏传 member_targets，聚合视图 ID 也必须在
    // 写入点 fail-closed，绝不流入 IR target（否则 Final Compile 以
    // work_item_target_missing 卡死 owner 验收——session_0003 现场）。
    let diagnostics = compile_work_item_plan(REP4_FIXTURE, &aggregate_context(None))
        .expect_err("聚合视图 ID 作 target 必须在 lowering 拒绝");
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code == "work_item_target_missing"),
        "守卫诊断必须稳定：{diagnostics:?}"
    );
}
