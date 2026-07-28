//! Handoff revision 删除测试：覆盖 `WorkItemRevisionStore::delete_handoff_revision`。
//!
//! 覆盖场景：
//! 1. 删除已发布的 handoff revision（先 publish 再 delete，断言随后 get 返回 NotFound）。
//! 2. 归属校验：用 `wi_b` 名义删除 `wi_a` 的 handoff 必须失败，且 wi_a 的档案仍在。
//! 3. 删除 handoff revision 不影响 plan 编译产物：spec requirement 4 要求
//!    plan revision、work item revision、projection bundle、verification plan revision、
//!    dependency graph revision 全部保持存在且内容不变。本测试在夹具中真实写入
//!    这 5 类编译产物，删除 handoff 后逐一断言仍可 get（is_ok）。

use crate::product::models::{
    DependencyGraphRevision, HandoffRevision, LogicalWorkItem, PlanProjectionBundle,
    PlanValidationReportArtifact, VerificationPlanRevision, WorkItemPlanLineage,
    WorkItemProjectionBundle, WorkItemRevision,
};
use crate::product::work_item_contract::{
    ContractValidationReport, build_dependency_contract_graph,
};
use crate::product::work_item_projection::{
    PlanProjectionCompileInput, PlanProjectionCompiler, ProjectionValidationReport,
    WorkItemProjectionCompiler, projection_hashes,
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
    // 归属相符两种落地形态都属正确拒绝：形态 (a) get 读出后 logical_work_item_id 不匹配；
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

    // spec requirement 4：删除 handoff 不得波及以下 5 类 plan 编译产物。
    let active_revision_id = lineage
        .active_revision_id
        .clone()
        .expect("lineage has active revision");
    // 1. plan revision
    assert!(
        store
            .get_plan_revision(PROJECT_ID, ISSUE_ID, PLAN_ID, &active_revision_id)
            .is_ok()
    );
    // 2. work item revision（每个 logical work item 一个）
    assert!(
        store
            .get_work_item_revision(
                &lineage,
                WORK_ITEM_A,
                &work_item_revision_id_for(WORK_ITEM_A)
            )
            .is_ok()
    );
    assert!(
        store
            .get_work_item_revision(
                &lineage,
                WORK_ITEM_B,
                &work_item_revision_id_for(WORK_ITEM_B)
            )
            .is_ok()
    );
    // 3. work item projection bundle（每个 logical work item 一个）
    assert!(
        store
            .get_work_item_projection_bundle(&lineage, "projection_bundle_wi_a")
            .is_ok()
    );
    assert!(
        store
            .get_work_item_projection_bundle(&lineage, "projection_bundle_wi_b")
            .is_ok()
    );
    // 4. verification plan revision（每个 logical work item 一个）
    assert!(
        store
            .get_verification_plan_revision(&lineage, "verification_plan_revision_wi_a")
            .is_ok()
    );
    assert!(
        store
            .get_verification_plan_revision(&lineage, "verification_plan_revision_wi_b")
            .is_ok()
    );
    // 5. dependency graph revision / plan projection bundle / plan validation report
    assert!(
        store
            .get_dependency_graph_revision(&lineage, "dependency_graph_revision_0001")
            .is_ok()
    );
    assert!(
        store
            .get_plan_projection_bundle(&lineage, "plan_projection_bundle_0001")
            .is_ok()
    );
    assert!(
        store
            .get_plan_validation_report(&lineage, "plan_validation_report_0001")
            .is_ok()
    );
}

/// 在 `deletion_fixture` 基础上追加一个 plan revision 及其全部编译产物，用于校验
/// 「删除 handoff 不影响 plan 编译产物」。编译产物通过编译器从 canonical contract
/// 真实生成（参考 `tests/projection_artifacts.rs`），确保 get 断言可读：
/// - work item projection bundle（每个 logical work item 一个）
/// - verification plan revision（每个 logical work item 一个）
/// - dependency graph revision（输入合约清空，wi_a/wi_b 无相互依赖）
/// - plan projection bundle
/// - plan validation report
fn deletion_fixture_with_compiled_plan() -> (TempDir, WorkItemRevisionStore, WorkItemPlanLineage) {
    let (temp, store, lineage) = deletion_fixture();

    // 收集每个 logical work item 的编译产物与依赖图契约。canonical_contract_fixture
    // 的 input_contracts 指向外部 `wi_upstream` 且 handoff provided_contract_refs 无消费者，
    // 会触发 dependency graph 校验失败（unknown_provider / unconsumed_handoff）。
    // 这里清空 input_contracts 与 provided_contract_refs，使 wi_a/wi_b 成为两个互不依赖的
    // 独立工作项，构造出合法的最小依赖图。
    let mut compiled_work_items = BTreeMap::new();
    let mut expected_work_item_revision_ids = BTreeMap::new();
    let mut work_item_projection_bundle_refs = Vec::new();
    let mut plan_contracts = Vec::new();

    for logical_id in [WORK_ITEM_A, WORK_ITEM_B] {
        let work_item_revision_id = work_item_revision_id_for(logical_id);
        let work_item_revision = store
            .get_work_item_revision(&lineage, logical_id, &work_item_revision_id)
            .unwrap();

        // canonical_contract_fixture 的 input_contracts 指向外部 `wi_upstream` 且
        // handoff provided_contract_refs 无消费者，会触发 dependency graph 校验失败
        // （unknown_provider / unconsumed_handoff）以及 plan projection 校验失败
        // （invented_contract_ref）。这里清空 input_contracts 与 provided_contract_refs，
        // 使 wi_a/wi_b 成为两个互不依赖的独立工作项，后续 work item projection 与
        // dependency graph 都基于这份一致的清空输入契约编译。
        let mut plan_contract = work_item_revision.canonical_contract.clone();
        plan_contract.input_contracts.clear();
        plan_contract
            .handoff_contract
            .provided_contract_refs
            .clear();

        // 编译 work item projection（基于清空输入后的契约，保持与依赖图一致）。
        let compiled_work_item = WorkItemProjectionCompiler
            .compile(&plan_contract, &work_item_revision_id)
            .unwrap();
        let work_item_hashes = projection_hashes(&compiled_work_item).unwrap();
        let work_item_projection_bundle_id =
            work_item_revision.work_item_projection_bundle_id.clone();
        let work_item_projection = WorkItemProjectionBundle {
            id: work_item_projection_bundle_id.clone(),
            work_item_revision_id: work_item_revision_id.clone(),
            canonical_contract_hash: work_item_revision.canonical_contract_hash.clone(),
            projection_schema_version: 1,
            compiler_version: "compiler-v1".to_string(),
            human_projection: compiled_work_item.human.clone(),
            coder_projection: compiled_work_item.coder.clone(),
            reviewer_projection: compiled_work_item.reviewer.clone(),
            human_projection_hash: work_item_hashes.human,
            coder_projection_hash: work_item_hashes.coder,
            reviewer_projection_hash: work_item_hashes.reviewer,
            created_at: "2026-07-28T00:00:04Z".to_string(),
        };
        store
            .put_work_item_projection_bundle(&lineage, &work_item_projection)
            .unwrap();

        // verification plan revision（复用 work item revision 的 id 与 verification_checks）。
        let verification_plan_revision = VerificationPlanRevision {
            id: work_item_revision.verification_plan_revision_id.clone(),
            logical_work_item_id: logical_id.to_string(),
            source_draft_revision_id: work_item_revision.source_draft_revision_id.clone(),
            verification_checks: work_item_revision
                .canonical_contract
                .verification_checks
                .clone(),
            created_at: "2026-07-28T00:00:05Z".to_string(),
        };
        store
            .put_verification_plan_revision(&lineage, &verification_plan_revision)
            .unwrap();

        plan_contracts.push(plan_contract);
        compiled_work_items.insert(logical_id.to_string(), compiled_work_item);
        expected_work_item_revision_ids.insert(logical_id.to_string(), work_item_revision_id);
        work_item_projection_bundle_refs.push(work_item_projection_bundle_id);
    }

    // 编译 plan projection（dependency graph + 全部 work item projection）。
    let graph = build_dependency_contract_graph(&plan_contracts).unwrap();
    let compiled_plan = PlanProjectionCompiler
        .compile(PlanProjectionCompileInput {
            plan_id: PLAN_ID,
            goal: "Delete handoff without touching plan artifacts",
            split_reason: "Two independent work items",
            source_refs: vec!["design_spec_0001".to_string()],
            dependency_graph: &graph,
            work_item_projections: &compiled_work_items,
            expected_work_item_revision_ids,
        })
        .unwrap();

    let plan_projection_bundle = PlanProjectionBundle {
        id: "plan_projection_bundle_0001".to_string(),
        plan_revision_id: "plan_revision_0001".to_string(),
        dependency_graph_revision_id: "dependency_graph_revision_0001".to_string(),
        work_item_projection_bundle_refs,
        human_group_projection: compiled_plan.human,
        coder_group_context: compiled_plan.coder,
        reviewer_group_matrix: compiled_plan.reviewer,
        human_group_projection_hash: "human_group_hash".to_string(),
        coder_group_context_hash: "coder_group_hash".to_string(),
        reviewer_group_matrix_hash: "reviewer_group_hash".to_string(),
        compiler_version: "compiler-v1".to_string(),
        created_at: "2026-07-28T00:00:06Z".to_string(),
    };
    store
        .put_plan_projection_bundle(&lineage, &plan_projection_bundle)
        .unwrap();

    // plan validation report（findings 为空，表示通过）。
    let plan_validation_report = PlanValidationReportArtifact {
        id: "plan_validation_report_0001".to_string(),
        plan_id: PLAN_ID.to_string(),
        plan_revision_id: "plan_revision_0001".to_string(),
        plan_projection_bundle_id: plan_projection_bundle.id.clone(),
        contract_validation: ContractValidationReport { findings: vec![] },
        projection_validation: ProjectionValidationReport { findings: vec![] },
        created_at: "2026-07-28T00:00:07Z".to_string(),
    };
    store
        .put_plan_validation_report(&lineage, &plan_validation_report)
        .unwrap();

    // dependency graph revision（边为空：wi_a/wi_b 互不依赖）。
    let dependency_graph_revision = DependencyGraphRevision {
        id: "dependency_graph_revision_0001".to_string(),
        plan_id: PLAN_ID.to_string(),
        edges: graph.edges.clone(),
        created_at: "2026-07-28T00:00:08Z".to_string(),
    };
    store
        .put_dependency_graph_revision(&lineage, &dependency_graph_revision)
        .unwrap();

    // plan revision 激活。
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
