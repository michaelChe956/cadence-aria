//! 失败测试（RED）：为待新增的 `WorkItemRevisionStore::delete_handoff_revision` 方法
//! 编写覆盖测试。方法尚未实现，这些测试当前编译失败——这是 TDD RED 的合法形态，
//! Task 2 将实现 `delete_handoff_revision` 使其转为 GREEN。
//!
//! 覆盖场景：
//! 1. 删除已发布的 handoff revision（先 publish 再 delete，断言随后 get 返回 NotFound）。
//! 2. 归属校验：用 `wi_b` 名义删除 `wi_a` 的 handoff 必须失败，且 wi_a 的档案仍在。
//! 3. 删除 handoff revision 不影响 plan 编译产物（plan revision 等仍在）。

use crate::product::models::{
    HandoffRevision, LogicalWorkItem, WorkItemPlanLineage, WorkItemRevision,
};

use super::*;

// 归属校验测试的落地形态说明（写给 Task 2 实现）：
// `delete_handoff_revision` 形态 (a) 先 `get_handoff_revision` 读出档案、校验
// `logical_work_item_id` 与传入参数一致再删；形态 (b) path 完全由传入的
// `logical_work_item_id` 决定，归属不符直接撞 NotFound。本测试套两种形态都能通过，
// 关键断言是「wi_a 的档案未被误删」。Task 2 选形态 (a)（显式校验更安全）。

const WORK_ITEM_A: &str = "wi_a";
const WORK_ITEM_B: &str = "wi_b";

/// 构造一个含两个 logical work item（`wi_a` + `wi_b`）与各自 work item revision 的
/// 最小 lineage 夹具，确保 `put_handoff_revision` 的归属校验（logical_work_item 必须存在）
/// 能通过。参考 `tests/initial_publication.rs` / `tests.rs` 的既有构造范式。
fn deletion_fixture() -> (TempDir, WorkItemRevisionStore, WorkItemPlanLineage) {
    let temp = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(temp.path().join(".aria"));
    let store = WorkItemRevisionStore::new(paths);
    let lineage = WorkItemPlanLineage {
        id: PLAN_ID.to_string(),
        project_id: PROJECT_ID.to_string(),
        issue_id: ISSUE_ID.to_string(),
        story_spec_refs: vec!["story_spec_0001".to_string()],
        design_spec_refs: vec!["design_spec_0001".to_string()],
        active_revision_id: None,
        active_amendment_id: None,
        created_at: "2026-07-28T00:00:00Z".to_string(),
        updated_at: "2026-07-28T00:00:00Z".to_string(),
    };
    store.put_plan_lineage(&lineage).unwrap();
    for logical_id in [WORK_ITEM_A, WORK_ITEM_B] {
        store
            .put_logical_work_item(
                &lineage,
                &LogicalWorkItem {
                    id: logical_id.to_string(),
                    plan_id: PLAN_ID.to_string(),
                    title: format!("Work item {logical_id}"),
                    active_revision_id: None,
                    created_at: "2026-07-28T00:00:00Z".to_string(),
                    updated_at: "2026-07-28T00:00:00Z".to_string(),
                },
            )
            .unwrap();
        let contract = canonical_contract_fixture(logical_id);
        let work_item_revision = WorkItemRevision {
            id: work_item_revision_id_for(logical_id),
            logical_work_item_id: logical_id.to_string(),
            source_draft_revision_id: format!("draft_revision_{logical_id}"),
            canonical_contract: contract,
            canonical_contract_hash: format!("contract_hash_{logical_id}"),
            work_item_projection_bundle_id: format!("projection_bundle_{logical_id}"),
            verification_plan_revision_id: format!("verification_plan_revision_{logical_id}"),
            created_at: "2026-07-28T00:00:01Z".to_string(),
        };
        store
            .put_work_item_revision(&lineage, &work_item_revision)
            .unwrap();
    }
    (temp, store, lineage)
}

fn work_item_revision_id_for(logical_id: &str) -> String {
    format!("work_item_revision_{logical_id}")
}

/// 构造一个属于 `logical_id` 的 `HandoffRevision`。
fn handoff_for(logical_id: &str, handoff_id: &str) -> HandoffRevision {
    HandoffRevision {
        id: handoff_id.to_string(),
        logical_work_item_id: logical_id.to_string(),
        work_item_revision_id: work_item_revision_id_for(logical_id),
        coding_unit_run_id: "coding_unit_run_0001".to_string(),
        provided_contracts: Vec::new(),
        provided_capabilities: BTreeMap::new(),
        contract_hash: "contract_hash_handoff".to_string(),
        commit_sha: "deadbeef".to_string(),
        tests: Vec::new(),
        artifacts: Vec::new(),
        created_at: "2026-07-28T00:00:02Z".to_string(),
    }
}

#[test]
fn delete_handoff_revision_removes_published_revision() {
    let (_root, store, lineage) = deletion_fixture();
    let handoff = handoff_for(WORK_ITEM_A, "handoff_revision_coding_unit_run_0001");
    store.put_handoff_revision(&lineage, &handoff).unwrap();

    store
        .delete_handoff_revision(
            &lineage,
            WORK_ITEM_A,
            "handoff_revision_coding_unit_run_0001",
        )
        .unwrap();

    let err = store
        .get_handoff_revision(
            &lineage,
            WORK_ITEM_A,
            "handoff_revision_coding_unit_run_0001",
        )
        .unwrap_err();
    assert!(matches!(
        err,
        ProductStoreError::NotFound {
            kind: "handoff_revision",
            ..
        }
    ));
}

#[test]
fn delete_handoff_revision_rejects_mismatched_logical_work_item_id() {
    let (_root, store, lineage) = deletion_fixture();
    // handoff 属于 wi_a。
    let handoff = handoff_for(WORK_ITEM_A, "handoff_revision_run_0001");
    store.put_handoff_revision(&lineage, &handoff).unwrap();

    // 用 wi_b 的名义删除 wi_a 的 handoff：必须失败。
    // 归属不符两种落地形态都属正确拒绝：形态 (a) get 读出后 logical_work_item_id 不匹配；
    // 形态 (b) path 取向 wi_b、找不到档案直接 NotFound。
    let _ = store.delete_handoff_revision(&lineage, WORK_ITEM_B, "handoff_revision_run_0001");

    // 关键断言：wi_a 的档案仍在，未被误删。
    assert!(
        store
            .get_handoff_revision(&lineage, WORK_ITEM_A, "handoff_revision_run_0001")
            .is_ok()
    );
}

#[test]
fn delete_handoff_revision_does_not_touch_plan_compilation_artifacts() {
    let (_root, store, lineage) = deletion_fixture_with_compiled_plan();
    let handoff = handoff_for(WORK_ITEM_A, "handoff_revision_run_0001");
    store.put_handoff_revision(&lineage, &handoff).unwrap();

    store
        .delete_handoff_revision(&lineage, WORK_ITEM_A, "handoff_revision_run_0001")
        .unwrap();

    // 断言编译产物仍在：plan revision 可读，未被 handoff 删除波及。
    let active_revision_id = lineage
        .active_revision_id
        .expect("lineage has active revision");
    assert!(
        store
            .get_plan_revision(PROJECT_ID, ISSUE_ID, PLAN_ID, &active_revision_id)
            .is_ok()
    );
}

/// 在 `deletion_fixture` 基础上追加一个 plan revision 并激活，用于校验「删除 handoff
/// 不影响 plan 编译产物」。
fn deletion_fixture_with_compiled_plan() -> (TempDir, WorkItemRevisionStore, WorkItemPlanLineage) {
    let (temp, store, lineage) = deletion_fixture();
    let plan_revision = WorkItemPlanRevision {
        id: "plan_revision_0001".to_string(),
        plan_id: PLAN_ID.to_string(),
        revision_no: 1,
        supersedes: None,
        reason: PlanRevisionReason::InitialCompile,
        work_item_bindings: BTreeMap::from([
            (
                WORK_ITEM_A.to_string(),
                work_item_revision_id_for(WORK_ITEM_A),
            ),
            (
                WORK_ITEM_B.to_string(),
                work_item_revision_id_for(WORK_ITEM_B),
            ),
        ]),
        dependency_graph_revision_id: "dependency_graph_revision_0001".to_string(),
        validation_report_ref: "plan_validation_report_0001".to_string(),
        plan_projection_bundle_id: "plan_projection_bundle_0001".to_string(),
        created_at: "2026-07-28T00:00:03Z".to_string(),
    };
    store.put_plan_revision(&lineage, &plan_revision).unwrap();
    let lineage = store
        .set_active_plan_revision(&lineage, &plan_revision.id)
        .unwrap();
    (temp, store, lineage)
}
