//! 阶段 3 Task 8.3 —— SC 依赖门 campaign 用例（Step 2 依赖门实证 + 8.3a
//! admission/status/journal 隔离）。
//!
//! campaign 纪律：前置态（unit 状态/handoff 指针/依赖图 JSON）可 seed，
//! 但 Waiting/Running/FailedClosed 等推进终态一律由真实
//! `CodingWorkspaceEngine::advance_to_next_group_unit`/`select_next_sc_group_unit`
//! 路径产生；断言一律从 durable store 落盘重开读取；provider ledger
//! （`list_role_runs`）在每次门判定前后必须零新增——依赖门永不启动 provider。

use std::collections::BTreeMap;

use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::{CodingAttemptStore, CreateGroupCodingAttemptInput};
use crate::product::coding_models::{
    CodingAdmissionKind, CodingExecutionAttempt, CodingExecutionUnit, CodingExecutionUnitStatus,
    CodingUnitRun, CodingUnitRunStatus,
};
use crate::product::coding_workspace_engine::{CodingWorkspaceEngine, group_dependency_gate};
use crate::product::git_workspace_service::GitWorkspaceService;
use crate::product::lifecycle_store::{LifecycleStore, UpsertIssueSharedWorktreeInput};
use crate::product::models::{HandoffRevision, ProviderName};
use crate::product::work_item_revision_store::WorkItemRevisionStore;
use crate::web::workspace_ws_types::ProviderConfigSnapshot;
use tokio::sync::mpsc;

struct DependencyFixture {
    _root: tempfile::TempDir,
    store: CodingAttemptStore,
    lifecycle: LifecycleStore,
    engine: CodingWorkspaceEngine,
    attempt: CodingExecutionAttempt,
}

fn sc_dependency_fixture(with_dependency: bool) -> DependencyFixture {
    group_dependency_fixture(with_dependency, CodingAdmissionKind::ScAdvance)
}

fn group_dependency_fixture(
    with_dependency: bool,
    admission_kind: CodingAdmissionKind,
) -> DependencyFixture {
    let root = tempfile::tempdir().expect("root");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let worktree = root.path().join("shared-worktree");
    std::fs::create_dir_all(&worktree).expect("worktree dir");
    super::init_test_git_repo(&worktree);
    let attempt = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_work_item_id: "work_item_0001".to_string(),
            base_branch: "HEAD".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: Some(worktree.clone()),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Fake,
                reviewer: Some(ProviderName::Fake),
                review_rounds: 1,
                permission_modes: Default::default(),
            },
            target_snapshot: None,
            max_auto_rework: 2,
        })
        .expect("group attempt");
    super::seed_group_attempt_fixture(&store, &attempt, true, with_dependency);
    let mut persisted = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("persisted attempt");
    persisted.admission_kind = admission_kind;
    store
        .write_coding_attempt_for_test(&persisted)
        .expect("admission kind");
    let lifecycle = LifecycleStore::new(store.paths());
    lifecycle
        .upsert_issue_shared_worktree(UpsertIssueSharedWorktreeInput {
            project_id: persisted.project_id.clone(),
            issue_id: persisted.issue_id.clone(),
            repository_id: "repository_0001".to_string(),
            branch_name: persisted.branch_name.clone(),
            worktree_path: worktree.clone(),
            base_branch: persisted.base_branch.clone(),
        })
        .expect("shared worktree");
    lifecycle
        .try_acquire_issue_worktree_lock(
            &persisted.project_id,
            &persisted.issue_id,
            "work_item_0001",
            &persisted.id,
        )
        .expect("initial worktree owner");
    let (event_tx, _event_rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), event_tx);
    DependencyFixture {
        _root: root,
        store,
        lifecycle,
        engine,
        attempt: persisted,
    }
}

fn unit(fixture: &DependencyFixture, logical_work_item_id: &str) -> CodingExecutionUnit {
    fixture
        .store
        .list_coding_units(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .expect("units")
        .into_iter()
        .find(|unit| unit.logical_work_item_id == logical_work_item_id)
        .expect("unit")
}

fn durable_attempt(fixture: &DependencyFixture) -> CodingExecutionAttempt {
    fixture
        .store
        .get_attempt(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .expect("durable attempt")
}

fn provider_ledger_count(fixture: &DependencyFixture) -> usize {
    fixture
        .store
        .list_role_runs(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .expect("provider ledger")
        .len()
}

/// 完成 unit 但不发布 handoff(A 完成、B 仍缺输入的真实前置态)。
fn complete_without_handoff(fixture: &DependencyFixture, logical_work_item_id: &str) {
    let target = unit(fixture, logical_work_item_id);
    fixture
        .store
        .update_coding_unit_status(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
            &target.id,
            CodingExecutionUnitStatus::Completed,
            Some("completed without handoff (campaign fixture)".to_string()),
        )
        .expect("completed unit without handoff");
}

/// 完成 unit 并发布绑定匹配的 handoff revision(真实 completed-run 链)。
fn complete_with_matching_handoff(fixture: &DependencyFixture, logical_work_item_id: &str) {
    let target = unit(fixture, logical_work_item_id);
    let revision_store = WorkItemRevisionStore::new(fixture.store.paths());
    let lineage = revision_store
        .get_plan_lineage(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &target.plan_id,
        )
        .expect("lineage");
    let revision = revision_store
        .get_work_item_revision(
            &lineage,
            &target.logical_work_item_id,
            &target.work_item_revision_id,
        )
        .expect("work item revision");
    let bundle = revision_store
        .get_work_item_projection_bundle(&lineage, &revision.work_item_projection_bundle_id)
        .expect("projection bundle");
    let run = CodingUnitRun {
        id: format!("{}_run_0001", target.id),
        unit_id: target.id.clone(),
        execution_no: 1,
        work_item_revision_id: target.work_item_revision_id.clone(),
        resolved_handoff_revision_ids: Vec::new(),
        canonical_contract_hash: bundle.canonical_contract_hash,
        projection_bundle_id: bundle.id,
        projection_compiler_version: bundle.compiler_version,
        coder_projection_hash: bundle.coder_projection_hash,
        reviewer_projection_hash: bundle.reviewer_projection_hash,
        coder_provider_renderer_version: "test-renderer-v1".to_string(),
        reviewer_provider_renderer_version: "test-renderer-v1".to_string(),
        internal_reviewer_provider_renderer_version: None,
        coder_execution_context_hash: None,
        reviewer_execution_context_hash: None,
        internal_reviewer_execution_context_hash: None,
        status: CodingUnitRunStatus::Completed,
        unit_rework_count: 0,
        verification_retry_count: 0,
        operational_retry_count: 0,
        plan_repair_count: 0,
        start_commit: Some("start_commit_0001".to_string()),
        completion_commit: Some("commit_dependency_0001".to_string()),
        created_at: "2026-08-31T00:00:00Z".to_string(),
        updated_at: "2026-08-31T00:00:00Z".to_string(),
    };
    fixture
        .store
        .create_coding_unit_run(&fixture.attempt, &run)
        .expect("completed run");
    fixture
        .store
        .update_coding_unit_completion_commit(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
            &target.id,
            Some("commit_dependency_0001".to_string()),
        )
        .expect("completion commit");
    let handoff = HandoffRevision {
        id: format!("handoff_revision_{}", run.id),
        logical_work_item_id: target.logical_work_item_id.clone(),
        work_item_revision_id: target.work_item_revision_id.clone(),
        coding_unit_run_id: run.id.clone(),
        provided_contracts: Vec::new(),
        provided_capabilities: BTreeMap::new(),
        contract_hash: "contract_hash_dependency_0001".to_string(),
        commit_sha: "commit_dependency_0001".to_string(),
        created_at: "2026-08-31T00:00:00Z".to_string(),
    };
    revision_store
        .put_handoff_revision(&lineage, &handoff)
        .expect("handoff");
    fixture
        .store
        .update_coding_unit_latest_handoff_revision_id(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
            &target.id,
            Some(handoff.id),
        )
        .expect("handoff pointer");
    fixture
        .store
        .update_coding_unit_status(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
            &target.id,
            CodingExecutionUnitStatus::Completed,
            Some("completed with matching handoff (campaign fixture)".to_string()),
        )
        .expect("completed unit");
}

/// 直接改写 durable coding unit JSON 的依赖列表(前置态 seed 通道)。
fn set_unit_dependencies(
    fixture: &DependencyFixture,
    logical_work_item_id: &str,
    dependencies: &[&str],
) {
    let target = unit(fixture, logical_work_item_id);
    let path = fixture.store.coding_unit_path(
        &fixture.attempt.project_id,
        &fixture.attempt.issue_id,
        &fixture.attempt.id,
        &target.id,
    );
    let mut unit: CodingExecutionUnit =
        serde_json::from_slice(&std::fs::read(&path).expect("coding unit JSON"))
            .expect("coding unit JSON value");
    unit.dependency_logical_work_item_ids = dependencies
        .iter()
        .map(|dependency| (*dependency).to_string())
        .collect();
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&unit).expect("coding unit JSON encoding"),
    )
    .expect("coding unit JSON");
}

/// 同步改写权威 dependency graph edges(unit 声明与 graph 必须一致,
/// 否则门会先以 SC_GROUP_DEPENDENCY_UNKNOWN 拒绝而不是命中目标场景)。
/// graph revision 经 store 是不可变的;这里作为前置态 seed 直接改写
/// durable JSON 文件(与 set_unit_dependencies 同一 seed 通道)。
fn set_graph_edges(fixture: &DependencyFixture, edges: Vec<(String, String)>) {
    let revision_store = WorkItemRevisionStore::new(fixture.store.paths());
    let lineage = revision_store
        .get_plan_lineage(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            "work_item_plan_0001",
        )
        .expect("lineage");
    let plan_revision = revision_store
        .get_plan_revision(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            "work_item_plan_0001",
            lineage
                .active_revision_id
                .as_deref()
                .expect("active revision"),
        )
        .expect("plan revision");
    let graph = revision_store
        .get_dependency_graph_revision(&lineage, &plan_revision.dependency_graph_revision_id)
        .expect("dependency graph");
    let graph_path = fixture
        .store
        .paths()
        .issue_root(&fixture.attempt.project_id, &fixture.attempt.issue_id)
        .join("work-item-revisions")
        .join("work_item_plan_0001")
        .join("dependency-graph-revisions")
        .join(format!("{}.json", graph.id));
    let mut graph = graph;
    graph.edges = edges
        .into_iter()
        .map(
            |(from, to)| crate::product::work_item_contract::DependencyContractEdge {
                from,
                to,
                required_contracts: Vec::new(),
            },
        )
        .collect();
    std::fs::write(
        &graph_path,
        serde_json::to_vec_pretty(&graph).expect("dependency graph JSON encoding"),
    )
    .expect("dependency graph JSON rewrite");
}

/// Step 2 —— A→B:完成 A 但不发 handoff,B 保持 Pending、attempt Waiting;
/// 发布匹配 binding 的 handoff 后 B 才 Running;并发推进仍只有一个 active。
#[tokio::test]
async fn campaign_stage3_dependency_gate_never_skips_unready_unit() {
    let fixture = sc_dependency_fixture(true);
    // 先完成无消费者的 work_item_0003(不发布 handoff),把门收敛到 A→B 单边;
    // A(work_item_0001)初始即 active/Running,完成 A 但不发 handoff。
    complete_without_handoff(&fixture, "work_item_0003");
    complete_without_handoff(&fixture, "work_item_0001");
    let before = provider_ledger_count(&fixture);

    let waiting = fixture
        .engine
        .advance_to_next_group_unit(&durable_attempt(&fixture))
        .await
        .expect("waiting advance");
    let consumer = unit(&fixture, "work_item_0002");
    assert_eq!(
        consumer.status,
        CodingExecutionUnitStatus::Pending,
        "B 在依赖 handoff 未发布前必须保持 Pending"
    );
    assert!(
        waiting.active_unit_id.is_none(),
        "attempt 不得有 active unit"
    );
    let snapshot = fixture
        .store
        .get_group_dependency_gate_snapshot(&waiting)
        .expect("gate snapshot lookup")
        .expect("durable gate snapshot");
    assert_eq!(
        snapshot.status,
        crate::product::coding_models::GroupDependencyGateStatus::Waiting
    );
    assert_eq!(
        snapshot.pending_unit_ids,
        vec![consumer.id.clone()],
        "gate 只等 B"
    );
    assert!(
        fixture
            .store
            .list_coding_unit_runs(&waiting, &consumer.id)
            .expect("consumer runs")
            .is_empty(),
        "B 未 Ready 前不得有任何 run(不得抢跑)"
    );
    assert_eq!(provider_ledger_count(&fixture), before);

    // 发布匹配 binding 的 handoff 后,B 才可 Running;并发推进(两个 engine
    // 同时 advance)仍只有一个 active unit。
    complete_with_matching_handoff(&fixture, "work_item_0001");
    let second_engine = {
        let (event_tx, _event_rx) = mpsc::channel(8);
        CodingWorkspaceEngine::new(fixture.store.clone(), GitWorkspaceService::new(), event_tx)
    };
    let concurrent_attempt = durable_attempt(&fixture);
    let engine_handle = {
        let engine = fixture.engine.clone();
        let attempt = concurrent_attempt.clone();
        tokio::spawn(async move { engine.advance_to_next_group_unit(&attempt).await })
    };
    let second_handle = {
        let attempt = concurrent_attempt.clone();
        tokio::spawn(async move { second_engine.advance_to_next_group_unit(&attempt).await })
    };
    let (first_result, second_result) = tokio::join!(engine_handle, second_handle);
    for result in [first_result, second_result] {
        let updated = result
            .expect("concurrent advance task")
            .expect("concurrent advance");
        assert!(
            updated.active_unit_id.is_some(),
            "并发推进收敛后仍保持恰一个 active unit"
        );
    }
    let consumer_after = unit(&fixture, "work_item_0002");
    assert_eq!(
        durable_attempt(&fixture).active_unit_id,
        Some(consumer_after.id.clone()),
        "handoff 发布后 B 成为唯一 active unit"
    );
    assert_eq!(consumer_after.status, CodingExecutionUnitStatus::Running);
    let running_units = fixture
        .store
        .list_coding_units(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .expect("units after concurrency")
        .iter()
        .filter(|unit| unit.status == CodingExecutionUnitStatus::Running)
        .count();
    assert_eq!(running_units, 1, "并发推进后 Running unit 恰一个");
    let worktree = fixture
        .lifecycle
        .get_issue_shared_worktree(&fixture.attempt.project_id, &fixture.attempt.issue_id)
        .expect("worktree")
        .expect("worktree record");
    assert_eq!(
        worktree.current_active_work_item_id.as_deref(),
        Some("work_item_0002")
    );
    assert_eq!(
        provider_ledger_count(&fixture),
        before,
        "全程零 provider start"
    );
}

/// Step 2 —— 表驱动 fail-closed:环/未知/自依赖/handoff mismatch,
/// 逐案读取 durable reason 且 provider ledger 零新增。
#[tokio::test]
async fn campaign_stage3_dependency_invalid_graph_and_handoff_mismatch_fail_closed() {
    struct InvalidGraphCase {
        label: &'static str,
        mutate: fn(&DependencyFixture),
        expected_reason: &'static str,
    }
    let cases = [
        InvalidGraphCase {
            label: "cycle",
            mutate: |fixture| {
                // 环:0001↔0002;unit 声明与权威 graph 同步改写,命中拓扑环检测。
                set_graph_edges(
                    fixture,
                    vec![
                        ("work_item_0001".to_string(), "work_item_0002".to_string()),
                        ("work_item_0002".to_string(), "work_item_0001".to_string()),
                    ],
                );
                set_unit_dependencies(fixture, "work_item_0001", &["work_item_0002"]);
                set_unit_dependencies(fixture, "work_item_0002", &["work_item_0001"]);
            },
            expected_reason: "SC_GROUP_DEPENDENCY_CYCLE",
        },
        InvalidGraphCase {
            label: "unknown_dependency",
            mutate: |fixture| {
                set_unit_dependencies(fixture, "work_item_0002", &["work_item_unknown"]);
            },
            expected_reason: "SC_GROUP_DEPENDENCY_UNKNOWN",
        },
        InvalidGraphCase {
            label: "self_dependency",
            mutate: |fixture| {
                set_unit_dependencies(fixture, "work_item_0002", &["work_item_0002"]);
            },
            expected_reason: "SC_GROUP_DEPENDENCY_SELF",
        },
        InvalidGraphCase {
            label: "handoff_binding_mismatch",
            mutate: |fixture| {
                complete_with_matching_handoff(fixture, "work_item_0001");
                let dependency = unit(fixture, "work_item_0001");
                let revision_store = WorkItemRevisionStore::new(fixture.store.paths());
                let lineage = revision_store
                    .get_plan_lineage(
                        &fixture.attempt.project_id,
                        &fixture.attempt.issue_id,
                        &dependency.plan_id,
                    )
                    .expect("lineage");
                let mismatched = HandoffRevision {
                    id: "handoff_mismatch_campaign".to_string(),
                    logical_work_item_id: dependency.logical_work_item_id.clone(),
                    work_item_revision_id: "work_item_revision_wrong_0001".to_string(),
                    coding_unit_run_id: format!("{}_run_0001", dependency.id),
                    provided_contracts: Vec::new(),
                    provided_capabilities: BTreeMap::new(),
                    contract_hash: "contract_hash_dependency_0001".to_string(),
                    commit_sha: "commit_dependency_0001".to_string(),
                    created_at: "2026-08-31T00:00:00Z".to_string(),
                };
                revision_store
                    .put_handoff_revision(&lineage, &mismatched)
                    .expect("mismatched handoff");
                fixture
                    .store
                    .update_coding_unit_latest_handoff_revision_id(
                        &fixture.attempt.project_id,
                        &fixture.attempt.issue_id,
                        &fixture.attempt.id,
                        &dependency.id,
                        Some(mismatched.id),
                    )
                    .expect("mismatched pointer");
            },
            expected_reason: "SC_GROUP_HANDOFF_PLAN_BINDING_MISMATCH",
        },
    ];
    for case in cases {
        let fixture = sc_dependency_fixture(true);
        (case.mutate)(&fixture);
        let before = provider_ledger_count(&fixture);

        let updated = fixture
            .engine
            .advance_to_next_group_unit(&durable_attempt(&fixture))
            .await
            .unwrap_or_else(|error| panic!("{}: advance failed: {error}", case.label));
        let snapshot = fixture
            .store
            .get_group_dependency_gate_snapshot(&updated)
            .unwrap_or_else(|error| panic!("{}: snapshot lookup failed: {error}", case.label))
            .unwrap_or_else(|| panic!("{}: durable gate snapshot expected", case.label));
        assert_eq!(
            snapshot.status,
            crate::product::coding_models::GroupDependencyGateStatus::FailedClosed,
            "{}: 门必须 fail-closed",
            case.label
        );
        assert_eq!(
            snapshot.reason_code.as_deref(),
            Some(case.expected_reason),
            "{}: durable reason 逐案落盘",
            case.label
        );
        assert!(
            updated.active_unit_id.is_none(),
            "{}: fail-closed 后不得有 active unit",
            case.label
        );
        let consumer = unit(&fixture, "work_item_0002");
        assert_eq!(
            consumer.status,
            CodingExecutionUnitStatus::Pending,
            "{}: 消费者 unit 状态不被 fail-closed 改写",
            case.label
        );
        assert!(
            fixture
                .store
                .list_coding_unit_runs(&updated, &consumer.id)
                .expect("consumer runs")
                .is_empty(),
            "{}: fail-closed 不得产生任何 run",
            case.label
        );
        assert_eq!(
            provider_ledger_count(&fixture),
            before,
            "{}: provider ledger 零新增",
            case.label
        );
    }
}

/// 8.3a —— admission 隔离:同一 graph fixture 下 SC 等依赖门,legacy 按既有
/// order_index 直接运行;flow_kind/旧消息形状零变化(attempt JSON 除推进
/// 必然改写的字段外逐字段不变,legacy 不落 SC 门快照)。
#[tokio::test]
async fn campaign_stage3_admission_kind_separates_sc_dependency_gate_from_legacy_order() {
    /// 剥离推进必然改写的字段后比对 attempt JSON(其余字段零变化):
    /// version 是乐观并发计数,admission_ticket_consumed_at 是推进即沿袭的
    // 门票据语义,两者与 active/current/stage/status/updated_at 同属推进字段。
    fn stable_shape(attempt: &CodingExecutionAttempt) -> serde_json::Value {
        let mut value = serde_json::to_value(attempt).expect("attempt JSON");
        let object = value.as_object_mut().expect("attempt object");
        for field in [
            "updated_at",
            "active_unit_id",
            "current_work_item_id",
            "stage",
            "status",
            "version",
            "admission_ticket_consumed_at",
        ] {
            object.remove(field);
        }
        value
    }

    // —— SC:同 graph(先收敛到 A→B 单边),依赖未满足 → Waiting,B 不运行 ——
    let sc = sc_dependency_fixture(true);
    complete_without_handoff(&sc, "work_item_0003");
    complete_without_handoff(&sc, "work_item_0001");
    let sc_before = stable_shape(&durable_attempt(&sc));
    let sc_updated = sc
        .engine
        .advance_to_next_group_unit(&durable_attempt(&sc))
        .await
        .expect("SC waiting advance");
    assert!(sc_updated.active_unit_id.is_none(), "SC 必须等依赖门");
    assert_eq!(
        unit(&sc, "work_item_0002").status,
        CodingExecutionUnitStatus::Pending
    );
    assert!(
        sc.store
            .get_group_dependency_gate_snapshot(&sc_updated)
            .expect("SC gate snapshot lookup")
            .is_some(),
        "SC 路径落 durable 依赖门快照"
    );
    assert_eq!(stable_shape(&sc_updated), sc_before, "SC:旧字段零变化");

    // —— legacy:同一 graph fixture,直接按 order_index 运行下一个 unit ——
    let legacy = group_dependency_fixture(true, CodingAdmissionKind::LegacyGroup);
    let legacy_before = stable_shape(&durable_attempt(&legacy));
    complete_without_handoff(&legacy, "work_item_0003");
    complete_without_handoff(&legacy, "work_item_0001");
    let legacy_updated = legacy
        .engine
        .advance_to_next_group_unit(&durable_attempt(&legacy))
        .await
        .expect("legacy advance");
    assert_eq!(
        legacy_updated.admission_kind,
        CodingAdmissionKind::LegacyGroup
    );
    let legacy_next = unit(&legacy, "work_item_0002");
    assert_eq!(
        legacy_updated.active_unit_id.as_deref(),
        Some(legacy_next.id.as_str()),
        "legacy 按 order_index 运行 B,不理会依赖门"
    );
    assert_eq!(legacy_next.status, CodingExecutionUnitStatus::Running);
    assert!(
        legacy
            .store
            .get_group_dependency_gate_snapshot(&legacy_updated)
            .expect("legacy gate snapshot lookup")
            .is_none(),
        "legacy 路径不落 SC 依赖门快照"
    );
    assert!(!group_dependency_gate::dependency_gate_applies(
        &legacy_updated
    ));
    assert_eq!(
        stable_shape(&legacy_updated),
        legacy_before,
        "legacy:旧字段零变化"
    );
    let legacy_units = legacy
        .store
        .list_coding_units(
            &legacy.attempt.project_id,
            &legacy.attempt.issue_id,
            &legacy.attempt.id,
        )
        .expect("legacy units");
    let mut order_indices: Vec<u32> = legacy_units.iter().map(|unit| unit.order_index).collect();
    order_indices.sort();
    assert_eq!(
        order_indices,
        vec![0, 1, 2],
        "legacy unit 保留既有 order_index 语义(输入顺序不变)"
    );
    assert_eq!(provider_ledger_count(&sc), 0);
    assert_eq!(provider_ledger_count(&legacy), 0);
}
