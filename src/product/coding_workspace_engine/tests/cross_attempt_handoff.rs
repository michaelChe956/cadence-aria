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
    /// P3-a：owner 桶 worktree 绝对路径（`cross_fixture_with_owner_worktree`
    /// 变体创建；缺省 None）。
    owner_worktree: Option<PathBuf>,
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
    cross_fixture_with_owner_worktree(false)
}

/// P3-a（REQ-COD-07）注入面变体：owner 桶携带真实 worktree 绝对路径
///（上游交付段的 path 事实来源）。
fn cross_fixture_with_owner_worktree(with_owner_worktree: bool) -> CrossFixture {
    let root = tempdir().expect("tempdir");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    init_test_git_repo(&worktree);
    let start_head = git_stdout(&worktree, &["rev-parse", "HEAD"])
        .trim()
        .to_string();
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));

    let owner_worktree = with_owner_worktree.then(|| {
        let path = root.path().join("owner-worktree");
        fs::create_dir_all(&path).expect("owner worktree dir");
        path
    });
    let owner = create_bucket_attempt(&store, "api", OWNER_WI, owner_worktree.clone());
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
        owner_worktree,
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
    publish_completed_handoff_with_contracts(
        store,
        attempt,
        logical_work_item_id,
        completion_commit,
        &[],
        &[],
    )
}

/// P3-a 变体：发布的 handoff 携带 provided_contracts/provided_capabilities
///（上游交付段的契约与能力清单事实来源）。
fn publish_completed_handoff_with_contracts(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    logical_work_item_id: &str,
    completion_commit: &str,
    provided_contracts: &[&str],
    provided_capabilities: &[(&str, &[&str])],
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
        provided_contracts: provided_contracts
            .iter()
            .map(|contract| contract.to_string())
            .collect(),
        provided_capabilities: provided_capabilities
            .iter()
            .map(|(contract, capabilities)| {
                (
                    contract.to_string(),
                    capabilities
                        .iter()
                        .map(|capability| capability.to_string())
                        .collect(),
                )
            })
            .collect(),
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

// ---------------------------------------------------------------------------
// P3-a（REQ-COD-07）：跨仓上游交付注入 coder prompt。
// ---------------------------------------------------------------------------

use crate::cross_cutting::provider_registry::ProviderRegistry;
use crate::cross_cutting::streaming_provider::{ProviderCompletion, ProviderEvent};
use crate::product::coding_models::provider_config::CodingRoleProviderConfigSnapshot;
use crate::product::logical_codebase::GatewayRunAudit;
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
// Task 7 既有 LC gateway 装置复用（worktree_discipline 同款）。
use super::provider_gateway_validated_input::{
    build_gateway_with_registry, engine as gateway_engine, seed_logical_codebase_checkout,
};

const UPSTREAM_SECTION_HEADER: &str = "[resolved_upstream_deliveries]";

/// P3-a 注入面 scripted provider：按调用序弹出预置输出；前
/// `failures_before_success` 次以 Failed 事件模拟传输失败（触发 retry 周期
/// 的 fresh 重启），记录每次 StreamingProviderInput 供 prompt 变体断言。
struct PromptCapturingProvider {
    outputs: Mutex<VecDeque<String>>,
    failures_before_success: usize,
    starts: AtomicUsize,
    inputs: Mutex<Vec<StreamingProviderInput>>,
}

impl PromptCapturingProvider {
    fn new(outputs: Vec<&str>, failures_before_success: usize) -> Arc<Self> {
        Arc::new(Self {
            outputs: Mutex::new(outputs.into_iter().map(str::to_string).collect()),
            failures_before_success,
            starts: AtomicUsize::new(0),
            inputs: Mutex::new(Vec::new()),
        })
    }

    fn recorded_inputs(&self) -> Vec<StreamingProviderInput> {
        self.inputs.lock().expect("provider inputs").clone()
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for PromptCapturingProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: tokio_util::sync::CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let start_no = self.starts.fetch_add(1, Ordering::SeqCst) + 1;
        let output = self
            .outputs
            .lock()
            .expect("provider outputs")
            .pop_front()
            .unwrap_or_else(|| "{\"plan_defect_findings\": []}".to_string());
        self.inputs
            .lock()
            .expect("provider inputs")
            .push(input.clone());
        let (event_tx, event_rx) = mpsc::channel(2);
        let (command_tx, _command_rx) = mpsc::channel(2);
        let failures_before_success = self.failures_before_success;
        tokio::spawn(async move {
            let event = if start_no <= failures_before_success {
                ProviderEvent::Failed {
                    message: "connection reset by peer".to_string(),
                }
            } else {
                ProviderEvent::Completed(ProviderCompletion::from_output(
                    output,
                    input.structured_output_contract.as_ref(),
                    Some(format!("provider-session-{start_no}")),
                ))
            };
            let _ = event_tx.send(event).await;
        });
        Ok(ProviderSession {
            native_session_id: None,
            events: event_rx,
            commands: command_tx,
        })
    }

    /// LC gateway 分发走 validated 启动（Task 1a 冻结签名）：拆出 input 后
    /// 与裸 `start` 同路径（fake.rs 同款委托）。
    async fn start_validated(
        &self,
        validated: crate::cross_cutting::session_launch::ValidatedStreamingProviderInput,
        cancel: tokio_util::sync::CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (input, _launch) = validated.into_parts();
        self.start(input, cancel).await
    }
}

fn owner_worktree_string(f: &CrossFixture) -> String {
    f.owner_worktree
        .as_ref()
        .expect("owner worktree")
        .display()
        .to_string()
}

/// P3-a 注入面的 LC gateway 引擎装置（worktree_discipline 先例）：target
/// snapshot attempt 的 provider run 必须 validated launch，无 gateway 直接
/// 拒（logical_provider_gateway_required）。consumer 的 target 快照覆写为
/// nil UUID（对齐 seed_logical_codebase_checkout 播种的成员主 checkout），
/// coder 覆写 ClaudeCode（避开 Codex danger-full-access 路由级硬门）。
fn lc_prompt_capture_engine(
    store: &CodingAttemptStore,
    consumer: &CodingExecutionAttempt,
    adapter: &Arc<PromptCapturingProvider>,
) -> CodingWorkspaceEngine {
    let mut consumer = consumer.clone();
    let worktree = consumer.worktree_path.clone().expect("consumer worktree");
    let snapshot = consumer.target_snapshot.clone().expect("target snapshot");
    consumer.target_snapshot = Some(AttemptTargetSnapshot {
        logical_repository_id: LogicalRepositoryId(Uuid::nil()),
        checkout_id: RepositoryCheckoutId(Uuid::nil()),
        canonical_path: worktree,
        ..snapshot
    });
    store
        .write_coding_attempt_for_test(&consumer)
        .expect("nil-uuid target snapshot");
    seed_logical_codebase_checkout(store, &consumer);
    store
        .update_role_provider_config_snapshot(
            &consumer.project_id,
            &consumer.issue_id,
            &consumer.id,
            CodingRoleProviderConfigSnapshot {
                coder: ProviderName::ClaudeCode,
                code_reviewer: Some(ProviderName::ClaudeCode),
                internal_reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
                permission_modes:
                    crate::product::coding_models::provider_config::CodingRolePermissionModes::default(
                    ),
            },
        )
        .expect("coder provider ClaudeCode");
    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::ClaudeCode, adapter.clone());
    let gateway = build_gateway_with_registry(
        &store.paths(),
        &consumer.project_id,
        Arc::new(registry),
        Arc::new(GatewayRunAudit::new()),
    );
    gateway_engine(store, Some(gateway))
}

/// 红测①：上游 unit 已完成且 handoff 已发布，而 owner attempt 停在
/// WaitingForHuman（待人工确认）——注入不等待人工确认，resolver 与段文本
/// 直接给出 owner path / SHA / 契约（CT-001）与能力清单。
#[test]
fn cross_bucket_upstream_delivery_resolves_owner_facts_without_owner_confirmation() {
    let f = cross_fixture_with_owner_worktree(true);
    let handoff = publish_completed_handoff_with_contracts(
        &f.store,
        &f.owner,
        OWNER_WI,
        "commit_owner_0001",
        &["CT-001"],
        &[("CT-001", &["cap_shared_state", "cap_user_profile_api"])],
    );
    f.store
        .seed_running_attempt_for_test(PROJECT_ID, ISSUE_ID, &f.owner.id)
        .expect("owner running");
    // Running -> WaitingForHuman：owner unit 已完成、attempt 待人工确认。
    f.store
        .update_attempt_status(
            PROJECT_ID,
            ISSUE_ID,
            &f.owner.id,
            CodingAttemptStatus::WaitingForHuman,
        )
        .expect("owner attempt awaiting human confirmation");

    let deliveries = f
        .engine
        .resolved_upstream_deliveries_for_attempt(&f.consumer)
        .expect("upstream deliveries resolve without owner confirmation");
    assert_eq!(deliveries.len(), 1);
    let delivery = &deliveries[0];
    assert_eq!(delivery.logical_work_item_id, OWNER_WI);
    assert_eq!(delivery.owner_attempt_id, f.owner.id);
    assert_eq!(
        delivery.owner_worktree_path.as_deref(),
        f.owner_worktree.as_deref()
    );
    assert_eq!(delivery.handoff_revision_id, handoff.id);
    assert_eq!(delivery.commit_sha, "commit_owner_0001");
    assert_eq!(delivery.provided_contracts, vec!["CT-001".to_string()]);

    let section = resolved_upstream_deliveries_section(&deliveries);
    assert!(section.contains(UPSTREAM_SECTION_HEADER), "{section}");
    assert!(section.contains(&owner_worktree_string(&f)), "{section}");
    assert!(section.contains("commit_owner_0001"), "{section}");
    assert!(section.contains("CT-001"), "{section}");
    assert!(section.contains("cap_shared_state"), "{section}");
    assert!(section.contains("只读"), "{section}");
    assert!(section.contains("本仓"), "{section}");
    assert!(section.contains("operational_blocker"), "{section}");
    assert!(section.contains("operational_gate"), "{section}");
    assert!(section.contains("upstream_source_unavailable"), "{section}");
}

/// 红测③（resolver 面）：单仓与「同桶承载依赖」的 group attempt 都不在
/// 注入面——同仓上游就在本 worktree，无需跨仓交付段。
#[test]
fn single_repo_and_local_dependency_attempts_have_no_upstream_delivery() {
    let f = cross_fixture();
    let single = test_attempt("coding_attempt_single_repo");
    assert!(
        f.engine
            .resolved_upstream_deliveries_for_attempt(&single)
            .expect("single repo resolves empty")
            .is_empty()
    );

    // 同桶承载：WI-001 与消费 WI-002 同 attempt，上游交付就在本 worktree。
    let local = create_bucket_attempt(&f.store, "web2", OWNER_WI, None);
    create_unit(
        &f.store,
        &local,
        OWNER_WI,
        "work_item_revision_0001",
        &[],
        CodingExecutionUnitStatus::Completed,
    );
    publish_completed_handoff(&f.store, &local, OWNER_WI, "commit_local_0001");
    create_unit(
        &f.store,
        &local,
        CONSUMER_WI,
        "work_item_revision_0002",
        &[OWNER_WI],
        CodingExecutionUnitStatus::Running,
    );
    assert!(
        f.engine
            .resolved_upstream_deliveries_for_attempt(&local)
            .expect("local dependency resolves empty")
            .is_empty()
    );

    let section = resolved_upstream_deliveries_section(&[]);
    assert!(section.is_empty());
    assert!(!section.contains(UPSTREAM_SECTION_HEADER));
}

/// 红测②fresh 面＋红测③哈希面：跨桶下游 coder 的 fresh prompt 携带段，
/// 而冻结渲染 envelope 与 coder 执行上下文哈希不含段、注入后保持不变。
#[tokio::test]
async fn cross_bucket_coder_fresh_prompt_carries_upstream_section_and_keeps_context_hash() {
    let f = cross_fixture_with_owner_worktree(true);
    publish_completed_handoff_with_contracts(
        &f.store,
        &f.owner,
        OWNER_WI,
        "commit_owner_0001",
        &["CT-001"],
        &[("CT-001", &["cap_shared_state"])],
    );
    f.store
        .seed_running_attempt_for_test(PROJECT_ID, ISSUE_ID, &f.consumer.id)
        .expect("running consumer");
    let consumer = f
        .store
        .get_attempt(PROJECT_ID, ISSUE_ID, &f.consumer.id)
        .expect("consumer attempt");

    let provider = PromptCapturingProvider::new(vec!["{\"plan_defect_findings\": []}"], 0);
    let engine = lc_prompt_capture_engine(&f.store, &consumer, &provider);
    let (_command_tx, mut command_rx) = mpsc::channel(1);
    engine
        .execute_coding_with_commands(
            &consumer,
            provider.as_ref(),
            &CodingExecutionContext::default(),
            &mut command_rx,
        )
        .await
        .expect("coding completes");

    let inputs = provider.recorded_inputs();
    assert_eq!(inputs.len(), 1);
    let prompt = &inputs[0].prompt;
    assert!(prompt.contains(UPSTREAM_SECTION_HEADER), "{prompt}");
    assert!(prompt.contains(&owner_worktree_string(&f)), "{prompt}");
    assert!(prompt.contains("commit_owner_0001"), "{prompt}");
    assert!(prompt.contains("CT-001"), "{prompt}");

    // 冻结渲染面不含注入段（段只进实际 prompt，不进 renderer 哈希链）；
    // execute 内 freeze start_commit 后已 bind 的哈希，此处幂等重渲染
    // （同 envelope 同 hash，bind 直接复用）证明注入不动执行上下文哈希。
    let rendered = engine
        .render_coder_unit_run_context(&consumer, &ProviderName::ClaudeCode)
        .expect("re-render cross-bucket coder context")
        .expect("group context");
    assert!(
        !rendered.text.contains(UPSTREAM_SECTION_HEADER),
        "{}",
        rendered.text
    );
    let run_after = f
        .store
        .get_active_unit_run(&consumer)
        .expect("unit run after coding");
    assert_eq!(
        run_after.coder_execution_context_hash.as_deref(),
        Some(rendered.content_hash.as_str())
    );
}

/// 红测③（prompt 面）：legacy 单仓 coder 的实际 prompt 不出现注入段。
#[tokio::test]
async fn single_repo_coder_prompt_has_no_upstream_section() {
    let (_root, _store, attempt) = running_attempt_with_worktree();
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(_store.clone(), GitWorkspaceService::new(), tx);
    let provider = PromptCapturingProvider::new(vec!["{\"plan_defect_findings\": []}"], 0);
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    engine
        .execute_coding_with_commands(
            &attempt,
            provider.as_ref(),
            &CodingExecutionContext::default(),
            &mut command_rx,
        )
        .await
        .expect("single repo coding completes");

    let inputs = provider.recorded_inputs();
    assert_eq!(inputs.len(), 1);
    assert!(
        !inputs[0].prompt.contains(UPSTREAM_SECTION_HEADER),
        "{}",
        inputs[0].prompt
    );
}

/// 红测②delta 面：resume 会话续跑的 delta-only prompt 同样携带段。
#[tokio::test]
async fn cross_bucket_coder_delta_resume_prompt_carries_upstream_section() {
    let f = cross_fixture_with_owner_worktree(true);
    publish_completed_handoff_with_contracts(
        &f.store,
        &f.owner,
        OWNER_WI,
        "commit_owner_0001",
        &["CT-001"],
        &[("CT-001", &["cap_shared_state"])],
    );
    f.store
        .seed_running_attempt_for_test(PROJECT_ID, ISSUE_ID, &f.consumer.id)
        .expect("running consumer");
    // head_commit 先落：run 物化以 attempt.head_commit 为 start_commit，
    // 二轮 freeze 才不会回填改写 envelope（执行上下文哈希稳定）。
    f.store
        .update_attempt_head_commit(
            PROJECT_ID,
            ISSUE_ID,
            &f.consumer.id,
            Some(f.start_head.clone()),
        )
        .expect("consumer start head");
    // unit started_at 先落（resume 判定需要活跃 unit 的启动事实）。
    let consumer_unit = unit_of(&f.store, &f.consumer, CONSUMER_WI);
    f.store
        .update_coding_unit_status(
            PROJECT_ID,
            ISSUE_ID,
            &f.consumer.id,
            &consumer_unit.id,
            CodingExecutionUnitStatus::Running,
            None,
        )
        .expect("unit running with started_at");
    let consumer = f
        .store
        .get_attempt(PROJECT_ID, ISSUE_ID, &f.consumer.id)
        .expect("consumer attempt");
    let provider = PromptCapturingProvider::new(vec!["{\"plan_defect_findings\": []}"], 0);
    let engine = lc_prompt_capture_engine(&f.store, &consumer, &provider);
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    // 第一轮：fresh（无 resume 会话）。
    let persisted = engine
        .execute_coding_with_commands(
            &consumer,
            provider.as_ref(),
            &CodingExecutionContext::default(),
            &mut command_rx,
        )
        .await
        .expect("first round completes");

    // 第二轮：resume 会话续跑 → delta-only prompt。
    let (_command_tx2, mut command_rx2) = mpsc::channel(1);
    engine
        .execute_coding_with_commands(
            &persisted,
            provider.as_ref(),
            &CodingExecutionContext::default(),
            &mut command_rx2,
        )
        .await
        .expect("second round completes");

    let inputs = provider.recorded_inputs();
    assert_eq!(inputs.len(), 2);
    assert_eq!(inputs[0].resume_provider_session_id, None);
    assert_eq!(
        inputs[1].resume_provider_session_id.as_deref(),
        Some("provider-session-1")
    );
    assert!(
        inputs[1].prompt.contains(UPSTREAM_SECTION_HEADER),
        "{}",
        inputs[1].prompt
    );
    assert!(
        inputs[1].prompt.contains(&owner_worktree_string(&f)),
        "{}",
        inputs[1].prompt
    );
}

/// 红测②retry 面：传输失败后的 retry 重启（fresh_prompt）同样携带段。
#[tokio::test]
async fn cross_bucket_coder_retry_prompt_carries_upstream_section() {
    let f = cross_fixture_with_owner_worktree(true);
    publish_completed_handoff_with_contracts(
        &f.store,
        &f.owner,
        OWNER_WI,
        "commit_owner_0001",
        &["CT-001"],
        &[],
    );
    f.store
        .seed_running_attempt_for_test(PROJECT_ID, ISSUE_ID, &f.consumer.id)
        .expect("running consumer");
    let consumer = f
        .store
        .get_attempt(PROJECT_ID, ISSUE_ID, &f.consumer.id)
        .expect("consumer attempt");
    let provider = PromptCapturingProvider::new(vec!["{\"plan_defect_findings\": []}"], 1);
    let engine = lc_prompt_capture_engine(&f.store, &consumer, &provider);
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    engine
        .execute_coding_with_commands(
            &consumer,
            provider.as_ref(),
            &CodingExecutionContext::default(),
            &mut command_rx,
        )
        .await
        .expect("coding completes after retry");

    let inputs = provider.recorded_inputs();
    assert_eq!(inputs.len(), 2, "first invocation failed, retry restarted");
    assert!(
        inputs[1].prompt.contains(UPSTREAM_SECTION_HEADER),
        "{}",
        inputs[1].prompt
    );
    assert!(
        inputs[1].prompt.contains("commit_owner_0001"),
        "{}",
        inputs[1].prompt
    );
}

/// 红测②teaching 面：coder 完成报告解析失败的恰一次教学重驱 prompt
/// 同样携带段（delta 面补注；fresh 面经 full_prompt 已含）。
#[tokio::test]
async fn cross_bucket_coder_teaching_reredrive_prompt_carries_upstream_section() {
    let f = cross_fixture_with_owner_worktree(true);
    publish_completed_handoff_with_contracts(
        &f.store,
        &f.owner,
        OWNER_WI,
        "commit_owner_0001",
        &["CT-001"],
        &[],
    );
    f.store
        .seed_running_attempt_for_test(PROJECT_ID, ISSUE_ID, &f.consumer.id)
        .expect("running consumer");
    let consumer = f
        .store
        .get_attempt(PROJECT_ID, ISSUE_ID, &f.consumer.id)
        .expect("consumer attempt");
    let invalid_output = "{\"plan_defect_findings\": [{\"finding_id\": \"f1\", \
         \"severity\": \"error\", \"defect_class\": \"plan_defect\", \
         \"reason_code\": \"upstream_plan_invalid\", \"message\": \"上游计划契约无法满足\", \
         \"evidence\": [], \"contract_refs\": [], \"capability_refs\": [], \
         \"repair_target\": null, \"recommended_route\": \"plan_repair\", \
         \"confidence\": \"low\"}]}";
    let provider = PromptCapturingProvider::new(vec![invalid_output], 0);
    let engine = lc_prompt_capture_engine(&f.store, &consumer, &provider);
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    engine
        .execute_coding_with_commands(
            &consumer,
            provider.as_ref(),
            &CodingExecutionContext::default(),
            &mut command_rx,
        )
        .await
        .expect("teaching reredrive recovers");

    let inputs = provider.recorded_inputs();
    assert_eq!(inputs.len(), 2);
    assert!(
        inputs[1].prompt.contains("上一轮已结束"),
        "{}",
        inputs[1].prompt
    );
    assert!(
        inputs[1].prompt.contains(UPSTREAM_SECTION_HEADER),
        "{}",
        inputs[1].prompt
    );
    assert!(
        inputs[1].prompt.contains(&owner_worktree_string(&f)),
        "{}",
        inputs[1].prompt
    );
}

/// 红测②rework 面：code review 修复轮（rework.rs 装配点）的 full/delta
/// prompt 同样携带段；此处驱动 fresh 落点（无 resume 会话时 FullConversation）。
#[tokio::test]
async fn cross_bucket_coder_rework_prompt_carries_upstream_section() {
    let f = cross_fixture_with_owner_worktree(true);
    publish_completed_handoff_with_contracts(
        &f.store,
        &f.owner,
        OWNER_WI,
        "commit_owner_0001",
        &["CT-001"],
        &[],
    );
    f.store
        .seed_running_attempt_for_test(PROJECT_ID, ISSUE_ID, &f.consumer.id)
        .expect("running consumer");
    let consumer = f
        .store
        .get_attempt(PROJECT_ID, ISSUE_ID, &f.consumer.id)
        .expect("consumer attempt");
    let provider = PromptCapturingProvider::new(vec!["{\"plan_defect_findings\": []}"], 0);
    let engine = lc_prompt_capture_engine(&f.store, &consumer, &provider);
    // review report 需要 unit run 事实：与 rework 装配同一 provider 口径
    //（ClaudeCode）先行物化，避免跨 renderer 版本的执行上下文失配。
    let run = f
        .store
        .get_active_unit_run(&consumer)
        .map(|run| run)
        .unwrap_or_else(|_| {
            engine
                .render_coder_unit_run_context(&consumer, &ProviderName::ClaudeCode)
                .expect("render materializes unit run");
            f.store
                .get_active_unit_run(&consumer)
                .expect("unit run after render")
        });
    let report = group_final_readiness_support::review_report(
        &consumer,
        "code_review_report_rework",
        &run,
        "2026-10-14T00:00:00Z",
        ReviewVerdict::RequestChanges,
        "round 1 requires changes",
    );
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    engine
        .execute_coder_fix_from_review_outcome(
            &consumer,
            &report,
            &CodingExecutionContext::default(),
            provider.as_ref(),
            &mut command_rx,
        )
        .await
        .expect("rework round completes");

    let inputs = provider.recorded_inputs();
    assert_eq!(inputs.len(), 1);
    assert!(
        inputs[0].prompt.contains(UPSTREAM_SECTION_HEADER),
        "{}",
        inputs[0].prompt
    );
    assert!(
        inputs[0].prompt.contains("commit_owner_0001"),
        "{}",
        inputs[0].prompt
    );
}
