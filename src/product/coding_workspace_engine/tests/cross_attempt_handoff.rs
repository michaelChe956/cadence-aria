//! S1：跨 attempt（跨 target 桶）依赖 handoff 解析。
//!
//! 多 target 拆分增殖形态下，一个 plan 的 WI 分属多个 target-attempt 桶；WI-002（web 桶）
//! 依赖 WI-001（api 桶）时，权威 handoff 解析必须能越过本桶读到 owner attempt 的
//! 已完成 handoff（同桶命中仍走原逻辑），且全程 fail-closed。

use super::*;
use crate::product::coding_models::{
    AttemptTargetSnapshot, CodingExecutionUnit, CodingStartRunPolicy, GroupFinalReadinessStatus,
    ReviewVerdict,
};
use crate::product::logical_codebase::{LogicalRepositoryId, RepositoryCheckoutId};
use std::path::PathBuf;
use uuid::Uuid;

const PROJECT_ID: &str = "project_0001";
const ISSUE_ID: &str = "issue_0001";
const PLAN_ID: &str = "work_item_plan_0001";
const OWNER_WI: &str = "work_item_0001";
const CONSUMER_WI: &str = "work_item_0002";

struct CrossFixture {
    _root: tempfile::TempDir,
    worktree: PathBuf,
    start_head: String,
    store: CodingAttemptStore,
    engine: CodingWorkspaceEngine,
    /// api 桶：承载 WI-001（Completed，尚未发布 handoff）。
    owner: CodingExecutionAttempt,
    /// web 桶：承载 WI-002（Running，依赖桶外 WI-001）。
    consumer: CodingExecutionAttempt,
}

fn target_snapshot(tag: &str) -> AttemptTargetSnapshot {
    AttemptTargetSnapshot {
        logical_repository_id: LogicalRepositoryId(Uuid::new_v4()),
        checkout_id: RepositoryCheckoutId(Uuid::new_v4()),
        physical_repository_id: format!("repo_{tag}"),
        canonical_path: PathBuf::from(format!("/nonexistent/checkout-{tag}")),
        git_dir_identity: format!("git-dir-{tag}"),
        revision: Some(format!("rev-{tag}")),
        policy_digest: "policy-digest".to_string(),
        membership_revision: 1,
        captured_at: "2026-10-14T00:00:00Z".to_string(),
        capture_source: "test".to_string(),
    }
}

fn create_bucket_attempt(
    store: &CodingAttemptStore,
    tag: &str,
    current_work_item_id: &str,
    worktree_path: Option<PathBuf>,
) -> CodingExecutionAttempt {
    let snapshot = target_snapshot(tag);
    store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            plan_id: PLAN_ID.to_string(),
            current_work_item_id: current_work_item_id.to_string(),
            base_branch: "HEAD".to_string(),
            branch_name: format!(
                "aria/issues/{ISSUE_ID}/{}",
                snapshot.logical_repository_id.0
            ),
            worktree_path,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
                permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
            },
            target_snapshot: Some(snapshot),
            max_auto_rework: 2,
            start_run_policy: CodingStartRunPolicy::Manual,
        })
        .expect("bucket attempt")
}

fn create_unit(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    logical_work_item_id: &str,
    work_item_revision_id: &str,
    dependencies: &[&str],
    status: CodingExecutionUnitStatus,
) -> CodingExecutionUnit {
    store
        .create_coding_unit(CreateCodingExecutionUnitInput {
            attempt_id: attempt.id.clone(),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            plan_id: PLAN_ID.to_string(),
            logical_work_item_id: logical_work_item_id.to_string(),
            work_item_revision_id: work_item_revision_id.to_string(),
            dependency_logical_work_item_ids: dependencies
                .iter()
                .map(|dependency| dependency.to_string())
                .collect(),
            order_index: 0,
            status,
        })
        .expect("coding unit")
}

fn save_binding(store: &CodingAttemptStore, attempt: &CodingExecutionAttempt) {
    store
        .save_plan_binding(
            attempt,
            &CodingAttemptPlanBinding {
                attempt_id: attempt.id.clone(),
                plan_id: PLAN_ID.to_string(),
                bound_plan_revision_id: "plan_revision_0001".to_string(),
                applied_amendment_ids: Vec::new(),
                updated_at: "2026-10-14T00:00:00Z".to_string(),
            },
        )
        .expect("plan binding");
}

fn cross_fixture() -> CrossFixture {
    let root = tempdir().expect("tempdir");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    init_test_git_repo(&worktree);
    let start_head = git_stdout(&worktree, &["rev-parse", "HEAD"])
        .trim()
        .to_string();
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));

    let owner = create_bucket_attempt(&store, "api", OWNER_WI, None);
    // 只播种 plan/revision 事实（含 0001→0002 依赖边），units 按桶手工物化。
    seed_group_attempt_fixture(&store, &owner, false, true);
    save_binding(&store, &owner);
    create_unit(
        &store,
        &owner,
        OWNER_WI,
        "work_item_revision_0001",
        &[],
        CodingExecutionUnitStatus::Completed,
    );

    let consumer = create_bucket_attempt(&store, "web", CONSUMER_WI, Some(worktree.clone()));
    save_binding(&store, &consumer);
    create_unit(
        &store,
        &consumer,
        CONSUMER_WI,
        "work_item_revision_0002",
        &[OWNER_WI],
        CodingExecutionUnitStatus::Running,
    );
    let consumer = store
        .get_attempt(PROJECT_ID, ISSUE_ID, &consumer.id)
        .expect("consumer attempt");

    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    CrossFixture {
        _root: root,
        worktree,
        start_head,
        store,
        engine,
        owner,
        consumer,
    }
}

fn unit_of(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    logical_work_item_id: &str,
) -> CodingExecutionUnit {
    store
        .list_coding_units(PROJECT_ID, ISSUE_ID, &attempt.id)
        .expect("units")
        .into_iter()
        .find(|unit| unit.logical_work_item_id == logical_work_item_id)
        .expect("unit")
}

/// 为 `attempt` 内的 `logical_work_item_id` 补齐一条权威的已完成 run + handoff +
/// completion_commit（unit 状态沿用创建时的值）。
fn publish_completed_handoff(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    logical_work_item_id: &str,
    completion_commit: &str,
) -> HandoffRevision {
    let unit = unit_of(store, attempt, logical_work_item_id);
    let revision_store = WorkItemRevisionStore::new(store.paths());
    let lineage = revision_store
        .get_plan_lineage(PROJECT_ID, ISSUE_ID, PLAN_ID)
        .expect("lineage");
    let revision = revision_store
        .get_work_item_revision(&lineage, logical_work_item_id, &unit.work_item_revision_id)
        .expect("work item revision");
    let bundle = revision_store
        .get_work_item_projection_bundle(&lineage, &revision.work_item_projection_bundle_id)
        .expect("projection bundle");
    let run = CodingUnitRun {
        id: format!("run_{}_{}", attempt.id, unit.id),
        unit_id: unit.id.clone(),
        execution_no: 1,
        work_item_revision_id: unit.work_item_revision_id.clone(),
        resolved_handoff_revision_ids: Vec::new(),
        canonical_contract_hash: bundle.canonical_contract_hash,
        projection_bundle_id: bundle.id,
        projection_compiler_version: bundle.compiler_version,
        coder_provider_renderer_version: "test-renderer-v1".to_string(),
        reviewer_provider_renderer_version: "test-renderer-v1".to_string(),
        internal_reviewer_provider_renderer_version: None,
        coder_projection_hash: bundle.coder_projection_hash,
        reviewer_projection_hash: bundle.reviewer_projection_hash,
        coder_execution_context_hash: None,
        reviewer_execution_context_hash: None,
        internal_reviewer_execution_context_hash: None,
        status: CodingUnitRunStatus::Completed,
        unit_rework_count: 0,
        verification_retry_count: 0,
        operational_retry_count: 0,
        plan_repair_count: 0,
        start_commit: Some("start_commit_0001".to_string()),
        completion_commit: Some(completion_commit.to_string()),
        created_at: "2026-10-14T00:00:00Z".to_string(),
        updated_at: "2026-10-14T00:00:00Z".to_string(),
    };
    store
        .create_coding_unit_run(attempt, &run)
        .expect("completed run");
    store
        .update_coding_unit_completion_commit(
            PROJECT_ID,
            ISSUE_ID,
            &attempt.id,
            &unit.id,
            Some(completion_commit.to_string()),
        )
        .expect("completion commit");
    let handoff = HandoffRevision {
        id: format!("handoff_revision_{}", run.id),
        logical_work_item_id: logical_work_item_id.to_string(),
        work_item_revision_id: unit.work_item_revision_id.clone(),
        coding_unit_run_id: run.id,
        provided_contracts: Vec::new(),
        provided_capabilities: std::collections::BTreeMap::new(),
        contract_hash: "contract_hash_0001".to_string(),
        commit_sha: completion_commit.to_string(),
        created_at: "2026-10-14T00:00:00Z".to_string(),
    };
    revision_store
        .put_handoff_revision(&lineage, &handoff)
        .expect("handoff");
    store
        .update_coding_unit_latest_handoff_revision_id(
            PROJECT_ID,
            ISSUE_ID,
            &attempt.id,
            &unit.id,
            Some(handoff.id.clone()),
        )
        .expect("handoff pointer");
    handoff
}

fn resolve(
    f: &CrossFixture,
    unit: &CodingExecutionUnit,
) -> Result<Vec<String>, CodingWorkspaceEngineError> {
    let lineage = WorkItemRevisionStore::new(f.store.paths())
        .get_plan_lineage(PROJECT_ID, ISSUE_ID, PLAN_ID)
        .expect("lineage");
    f.engine
        .authoritative_resolved_handoff_revision_ids(&f.consumer, unit, &lineage)
}

fn handoff_missing_message(result: Result<Vec<String>, CodingWorkspaceEngineError>) -> String {
    match result {
        Err(CodingWorkspaceEngineError::WorkItemHandoffMissing(message)) => message,
        other => panic!("expected WorkItemHandoffMissing, got {other:?}"),
    }
}

#[test]
fn cross_bucket_completed_dependency_resolves_owner_handoff_id() {
    let f = cross_fixture();
    let handoff = publish_completed_handoff(&f.store, &f.owner, OWNER_WI, "commit_owner_0001");
    let consumer_unit = unit_of(&f.store, &f.consumer, CONSUMER_WI);

    let ids = resolve(&f, &consumer_unit).expect("cross-bucket dependency resolves");

    assert_eq!(ids, vec![handoff.id]);
}

#[test]
fn cross_bucket_prefers_highest_attempt_no_owner() {
    let f = cross_fixture();
    publish_completed_handoff(&f.store, &f.owner, OWNER_WI, "commit_owner_0001");
    // 第三个桶 attempt 同样承载 WI-001（attempt_no 更大）——取最新 attempt 的 handoff。
    let newer = create_bucket_attempt(&f.store, "ops", OWNER_WI, None);
    assert!(newer.attempt_no > f.owner.attempt_no);
    create_unit(
        &f.store,
        &newer,
        OWNER_WI,
        "work_item_revision_0001",
        &[],
        CodingExecutionUnitStatus::Completed,
    );
    let newer_handoff = publish_completed_handoff(&f.store, &newer, OWNER_WI, "commit_owner_0002");
    let consumer_unit = unit_of(&f.store, &f.consumer, CONSUMER_WI);

    let ids = resolve(&f, &consumer_unit).expect("latest owner attempt resolves");

    assert_eq!(ids, vec![newer_handoff.id]);
}

#[test]
fn cross_bucket_dependency_not_completed_in_owner_fails_closed_with_status() {
    let f = cross_fixture();
    publish_completed_handoff(&f.store, &f.owner, OWNER_WI, "commit_owner_0001");
    let owner_unit = unit_of(&f.store, &f.owner, OWNER_WI);
    f.store
        .update_coding_unit_status(
            PROJECT_ID,
            ISSUE_ID,
            &f.owner.id,
            &owner_unit.id,
            CodingExecutionUnitStatus::Running,
            Some("owner unit still running".to_string()),
        )
        .expect("owner unit running");
    let consumer_unit = unit_of(&f.store, &f.consumer, CONSUMER_WI);

    let message = handoff_missing_message(resolve(&f, &consumer_unit));

    assert!(message.contains(OWNER_WI), "{message}");
    assert!(message.contains(&f.owner.id), "{message}");
    assert!(message.contains("Running"), "{message}");
}

#[test]
fn cross_bucket_completed_dependency_without_published_handoff_fails_closed() {
    let f = cross_fixture();
    // 夹具默认：owner WI-001 Completed 但从未发布 handoff。
    let consumer_unit = unit_of(&f.store, &f.consumer, CONSUMER_WI);

    let message = handoff_missing_message(resolve(&f, &consumer_unit));

    assert!(message.contains(OWNER_WI), "{message}");
    assert!(message.contains(&f.owner.id), "{message}");
}

#[test]
fn cross_bucket_stale_work_item_revision_is_not_a_candidate() {
    let f = cross_fixture();
    publish_completed_handoff(&f.store, &f.owner, OWNER_WI, "commit_owner_0001");
    let mut owner_unit = unit_of(&f.store, &f.owner, OWNER_WI);
    owner_unit.work_item_revision_id = "work_item_revision_stale".to_string();
    fs::write(
        f.store
            .coding_unit_path(PROJECT_ID, ISSUE_ID, &f.owner.id, &owner_unit.id),
        serde_json::to_vec(&owner_unit).expect("unit json"),
    )
    .expect("rewrite owner unit revision");
    let consumer_unit = unit_of(&f.store, &f.consumer, CONSUMER_WI);

    let message = handoff_missing_message(resolve(&f, &consumer_unit));

    assert!(message.contains(OWNER_WI), "{message}");
    assert!(message.contains("work_item_revision_stale"), "{message}");
}

#[test]
fn cross_bucket_completion_commit_mismatch_fails_closed() {
    let f = cross_fixture();
    publish_completed_handoff(&f.store, &f.owner, OWNER_WI, "commit_owner_0001");
    let owner_unit = unit_of(&f.store, &f.owner, OWNER_WI);
    f.store
        .update_coding_unit_completion_commit(
            PROJECT_ID,
            ISSUE_ID,
            &f.owner.id,
            &owner_unit.id,
            Some("commit_diverged".to_string()),
        )
        .expect("diverge unit completion commit");
    let consumer_unit = unit_of(&f.store, &f.consumer, CONSUMER_WI);

    let error = resolve(&f, &consumer_unit).expect_err("commit mismatch must fail closed");

    assert!(
        error
            .to_string()
            .contains("unit_run_handoff_binding_mismatch"),
        "{error}"
    );
}

#[test]
fn cross_bucket_dependency_hosted_by_no_attempt_fails_closed() {
    let f = cross_fixture();
    publish_completed_handoff(&f.store, &f.owner, OWNER_WI, "commit_owner_0001");
    // 依赖 work_item_0003：全 plan 没有任何 attempt 承载它。
    let mut consumer_unit = unit_of(&f.store, &f.consumer, CONSUMER_WI);
    consumer_unit.dependency_logical_work_item_ids = vec!["work_item_0003".to_string()];

    let message = handoff_missing_message(resolve(&f, &consumer_unit));

    assert!(message.contains("work_item_0003"), "{message}");
}

#[tokio::test]
async fn cross_bucket_dependency_flows_through_render_completion_and_final_readiness() {
    let f = cross_fixture();
    let handoff = publish_completed_handoff(&f.store, &f.owner, OWNER_WI, "commit_owner_0001");
    f.store
        .update_attempt_head_commit(
            PROJECT_ID,
            ISSUE_ID,
            &f.consumer.id,
            Some(f.start_head.clone()),
        )
        .expect("consumer start head");
    let consumer = f
        .store
        .get_attempt(PROJECT_ID, ISSUE_ID, &f.consumer.id)
        .expect("consumer attempt");

    // 1. 渲染入口：跨桶依赖 handoff 进入 unit run 与 prompt。
    let rendered = f
        .engine
        .render_coder_unit_run_context(&consumer, &ProviderName::Codex)
        .expect("render cross-bucket coder context")
        .expect("group context");
    assert!(rendered.text.contains(&handoff.id));
    let run = f
        .store
        .get_active_unit_run(&consumer)
        .expect("consumer unit run");
    assert_eq!(run.resolved_handoff_revision_ids, vec![handoff.id.clone()]);

    // 2. 完成 preflight：同一解析与 run 冻结的 handoff 绑定一致，完成链走通。
    fs::write(f.worktree.join("unit2.txt"), "consumer change\n").expect("coder change");
    run_test_git(&f.worktree, &["add", "unit2.txt"]);
    run_test_git(
        &f.worktree,
        &["commit", "-m", "coder completes consumer unit"],
    );
    f.store
        .seed_running_attempt_for_test(PROJECT_ID, ISSUE_ID, &consumer.id)
        .expect("running attempt");
    let consumer = f
        .store
        .update_attempt_stage(
            PROJECT_ID,
            ISSUE_ID,
            &consumer.id,
            CodingExecutionStage::ReviewRequest,
        )
        .expect("review request stage");
    f.engine
        .complete_group_unit_after_code_review(&consumer)
        .await
        .expect("cross-bucket group completion preflight passes");

    // 3. 评审绑定（第三处调用点）与 final readiness：桶内 unit 全部事实齐备即 Complete。
    let consumer = f
        .store
        .get_attempt(PROJECT_ID, ISSUE_ID, &consumer.id)
        .expect("completed consumer attempt");
    let bindings = f
        .engine
        .authoritative_group_reviewer_bindings(&consumer)
        .expect("reviewer bindings resolve the cross-bucket handoff");
    assert_eq!(bindings.len(), 1);
    assert_eq!(
        bindings[0].run.resolved_handoff_revision_ids,
        vec![handoff.id]
    );

    let completed_run = f
        .store
        .list_coding_unit_runs(&consumer, &unit_of(&f.store, &consumer, CONSUMER_WI).id)
        .expect("consumer runs")
        .into_iter()
        .next()
        .expect("consumer run");
    f.store
        .save_code_review_report(
            &consumer,
            &group_final_readiness_support::review_report(
                &consumer,
                "code_review_report_0001",
                &completed_run,
                "2026-10-14T00:00:00Z",
                ReviewVerdict::Approve,
                "approved",
            ),
        )
        .expect("review report");
    let snapshot = f
        .engine
        .build_group_final_readiness_snapshot(&consumer)
        .await
        .expect("final readiness snapshot");
    assert_eq!(
        snapshot.status,
        GroupFinalReadinessStatus::Complete,
        "{:?}",
        snapshot.diagnostics
    );
}
