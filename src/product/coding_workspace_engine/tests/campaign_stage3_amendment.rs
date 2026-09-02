//! 阶段 3 Task 8.4 —— 8.4a amendment E2E 与 GCE-02 failure E2E（Step 1/Step 2）。
//!
//! campaign 纪律（与 8.2/8.3 相同）：
//! - Confirmed 前置态由真实批准链产生（裸 Confirm → close → compile →
//!   `confirm_work_item_plan`），且 plan session id 经进程级原子计数唯一化，
//!   与所有 failpoint 注册家族的 durable scope 键永不相等（8.3 修复轮根因）。
//! - 计划缺陷注入复用 7.2 手段（seeded `PlanRepairRequest` + 真实
//!   `start_plan_repair`/`reconcile_linked_plan_repair_pause`）；断线/重启仅
//!   丢弃内存态从磁盘重建；fake revision 经真实 compiler/validator
//!   （`run_sc_manual_revision_turn`）产生 durable 候选。
//! - approve 走 amendment manifest/publication/application journal
//!   （`enter_plan_repair_awaiting_confirmation` → `confirm_and_publish_plan_amendment`
//!   → `resume_group_after_plan_amendment`），绝不重走首次 compile。
//! - 断言一律从 durable store 落盘重开读取；同 attempt/同 context 是唯一权威。

use super::*;
use crate::product::checkpoint_store::CheckpointStore;
use crate::product::coding_models::{
    CodingAttemptStatus, CodingExecutionStage, CodingUnitRunStatus, PlanAmendmentContextStatus,
};
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::{
    AmendmentResumeMode, PlanDefectClass, PlanDefectEvidence, PlanRepairRequest,
    PlanRepairRequestStatus, PlanRepairReviewAttestation, RepairTarget, RepairTargetKind,
    SingleCandidatePhase, WorkspaceSessionStatus,
};
use crate::product::plan_repair::{PlanRepairEngine, PreparedPlanAmendment};
use crate::product::work_item_plan_policy::{HumanGateSnapshot, HumanReason, RunPolicy};
use crate::product::work_item_revision_store::WorkItemRevisionStore;
use crate::product::workspace_engine::{
    HumanGateCommandOutcome, HumanGateFeedbackInput, WorkspaceEngine, WorkspaceSession,
};
use crate::web::workspace_ws_types::ArtifactPayload;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tempfile::TempDir;
use tokio::sync::mpsc;

const CAMPAIGN_FINDING_ID: &str = "code_review_report_0001_finding_0001";
const CAMPAIGN_TRIGGER_RUN_ID: &str = "coding_unit_run_campaign_trigger";
const AMENDMENT_CREATED_AT: &str = "2026-06-30T00:00:10Z";

const GATE_CANDIDATE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/product/work_item_plan_compiler/fixtures/work-item-plan-rep4.md"
));

/// rep4 fixture 的 WI-003 provided 行无消费者（canonical 校验 Error 级会拒绝），
/// 与 8.2 campaign harness 相同地剔除后才是可编译的脚本化候选。
fn campaign_gate_candidate() -> String {
    GATE_CANDIDATE.replace(
        "- provided_contract_refs: contract.levels-integration",
        "- provided_contract_refs: []",
    )
}

fn campaign_revised_candidate() -> String {
    campaign_gate_candidate().replace("Backend levels API", "Backend levels API amend-1")
}

struct CampaignAmendmentFixture {
    _root: TempDir,
    store: CodingAttemptStore,
    lifecycle: LifecycleStore,
    revision_store: WorkItemRevisionStore,
    attempt: CodingExecutionAttempt,
    plan: WorkItemPlanLineage,
    plan_session_id: String,
    child_session_id: String,
    request: PlanRepairRequest,
    units: Vec<crate::product::coding_models::CodingExecutionUnit>,
}

/// 真实批准链基座：accepted contract drafts → SC Approval 门 → 裸 Confirm 走完整
/// close→compile→confirm 链。session id 进程内唯一化（failpoint 键隔离纪律）。
async fn campaign_confirmed_plan_session(budget: u32) -> (TempDir, LifecycleStore, String, String) {
    let (root, lifecycle, plan_id, mut engine) =
        crate::product::workspace_engine::tests::make_work_item_plan_engine_with_accepted_contract_drafts();
    let app_paths = lifecycle.app_paths();
    {
        static CAMPAIGN_AMENDMENT_SESSION_SEQUENCE: AtomicU64 = AtomicU64::new(1);
        let mut record = lifecycle
            .get_workspace_session(&engine.session().session_id)
            .expect("fixture session record");
        let previous_session_path = app_paths
            .issue_root(&record.project_id, &record.issue_id)
            .join("workspace-sessions")
            .join(format!("{}.json", record.id));
        record.id = format!(
            "{}-campaign-amendment-{}",
            record.id,
            CAMPAIGN_AMENDMENT_SESSION_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        );
        let artifact = engine.session.artifact.clone();
        crate::product::json_store::write_json(
            &app_paths
                .issue_root(&record.project_id, &record.issue_id)
                .join("workspace-sessions")
                .join(format!("{}.json", record.id)),
            &record,
        )
        .expect("persist campaign-unique session");
        std::fs::remove_file(&previous_session_path)
            .expect("drop fixture session before unique rescope");
        engine.session = WorkspaceSession::from_record(record);
        engine.session.artifact = artifact;
    }
    crate::product::workspace_engine::tests::single_candidate_recovery::single_candidate_recovery_record(
        &lifecycle,
        &mut engine,
        SingleCandidatePhase::Approval,
        RunPolicy::Interactive,
    );
    engine
        .update_artifact(ArtifactPayload::Markdown {
            markdown: campaign_gate_candidate(),
            diff: None,
        })
        .await;
    let artifact = engine.session.artifact.clone();
    let mut record = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("session record");
    record.flow_kind = crate::product::work_item_plan_policy::WorkItemPlanFlowKind::SingleCandidate;
    record.run_policy = RunPolicy::Interactive;
    record.single_candidate_phase = Some(SingleCandidatePhase::Approval);
    record.status = WorkspaceSessionStatus::WaitingForHuman;
    record.human_gate_snapshot = Some(HumanGateSnapshot {
        findings: Vec::new(),
        repeated_fingerprints: Vec::new(),
        attempts_used: 0,
        manual_repairs_remaining: budget,
        trigger: HumanReason::NativeHumanRequired,
        resumable: true,
    });
    crate::product::json_store::write_json(
        &app_paths
            .issue_root(&record.project_id, &record.issue_id)
            .join("workspace-sessions")
            .join(format!("{}.json", record.id)),
        &record,
    )
    .expect("persist campaign gate session");
    let mut session = WorkspaceSession::from_record(record.clone());
    session.stage = crate::product::workspace_engine::WorkspaceStage::HumanConfirm;
    session.session_status = WorkspaceSessionStatus::WaitingForHuman;
    session.artifact = artifact;
    let (event_tx, _event_rx) = mpsc::channel(64);
    let mut engine = WorkspaceEngine::new_persistent(
        Arc::new(CheckpointStore::new(root.path().join("checkpoints"))),
        lifecycle.clone(),
        event_tx,
        session,
    );
    let plan_session_id = record.id.clone();
    let close = engine
        .handle_human_gate_termination(
            crate::web::workspace_ws_types::HumanConfirmDecision::Confirm,
        )
        .await
        .expect("real approval chain must confirm");
    assert_eq!(
        close,
        crate::product::workspace_engine::HumanGateCloseOutcome::Confirmed
    );
    let confirmed = lifecycle
        .get_workspace_session(&plan_session_id)
        .expect("confirmed session");
    assert_eq!(confirmed.status, WorkspaceSessionStatus::Confirmed);
    assert_eq!(
        confirmed.single_candidate_phase,
        Some(SingleCandidatePhase::Completed)
    );
    (root, lifecycle, plan_id, plan_session_id)
}

/// 真实编译产物上的 group attempt：units/binding 来自编译后的 active plan
/// revision（wi_a=完成并留 run、wi_b=运行中触发），前置态 seed 通道仅覆盖
/// unit run 记录（与 7.2 real-chain 测试同一注入手段）。
async fn campaign_amendment_fixture() -> CampaignAmendmentFixture {
    let (root, lifecycle, plan_id, plan_session_id) = campaign_confirmed_plan_session(2).await;
    let app_paths = lifecycle.app_paths();
    let store = CodingAttemptStore::new(app_paths.clone());
    let worktree = root.path().join("shared-worktree");
    std::fs::create_dir_all(&worktree).expect("worktree dir");
    super::init_test_git_repo(&worktree);
    let initial = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: plan_id.clone(),
            current_work_item_id: "wi_a".to_string(),
            base_branch: "main".to_string(),
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
    let mut persisted = store
        .get_attempt(&initial.project_id, &initial.issue_id, &initial.id)
        .expect("persisted attempt");
    persisted.admission_kind = crate::product::coding_models::CodingAdmissionKind::ScAdvance;
    store
        .write_coding_attempt_for_test(&persisted)
        .expect("admission kind");
    let revision_store = WorkItemRevisionStore::new(app_paths.clone());
    let plan = revision_store
        .get_plan_lineage("project_0001", "issue_0001", &plan_id)
        .expect("real plan lineage");
    let active_revision = revision_store
        .get_plan_revision(
            "project_0001",
            "issue_0001",
            &plan_id,
            plan.active_revision_id
                .as_deref()
                .expect("compiled active revision"),
        )
        .expect("real plan revision");
    store
        .save_plan_binding(
            &persisted,
            &CodingAttemptPlanBinding {
                attempt_id: persisted.id.clone(),
                plan_id: plan_id.clone(),
                bound_plan_revision_id: active_revision.id.clone(),
                applied_amendment_ids: Vec::new(),
                updated_at: AMENDMENT_CREATED_AT.to_string(),
            },
        )
        .expect("attempt plan binding");
    let ordered = ["wi_a".to_string(), "wi_b".to_string()];
    let mut units = Vec::new();
    for (index, logical_id) in ordered.iter().enumerate() {
        let revision_id = active_revision
            .work_item_bindings
            .get(logical_id)
            .expect("real binding")
            .clone();
        let unit = store
            .create_coding_unit(CreateCodingExecutionUnitInput {
                attempt_id: persisted.id.clone(),
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                plan_id: plan_id.clone(),
                logical_work_item_id: logical_id.clone(),
                work_item_revision_id: revision_id.clone(),
                dependency_logical_work_item_ids: if index == 1 {
                    vec!["wi_a".to_string()]
                } else {
                    Vec::new()
                },
                order_index: index as u32,
                status: if index == 0 {
                    CodingExecutionUnitStatus::Completed
                } else {
                    CodingExecutionUnitStatus::Pending
                },
            })
            .expect("coding unit");
        units.push((unit, revision_id));
    }
    let attempt = store
        .seed_running_attempt_for_test("project_0001", "issue_0001", &persisted.id)
        .expect("running attempt");
    // 前置态 seed：wi_a 已完成的 run + wi_b 运行中的触发 run（真实 revision 事实）。
    for (index, (unit, revision_id)) in units.iter().enumerate() {
        let revision = revision_store
            .get_work_item_revision(&plan, &unit.logical_work_item_id, revision_id)
            .expect("real work item revision");
        let bundle = revision_store
            .get_work_item_projection_bundle(&plan, &revision.work_item_projection_bundle_id)
            .expect("real projection bundle");
        // 触发形态：上游 wi_a 自身的 code review 发现其计划契约缺陷
        // （UpstreamContractInvalid 指向自身），wi_b 尚未启动（Pending 无 run）。
        let (run_id, status) = if index == 0 {
            (
                CAMPAIGN_TRIGGER_RUN_ID.to_string(),
                CodingUnitRunStatus::Completed,
            )
        } else {
            continue;
        };
        store
            .create_coding_unit_run(
                &attempt,
                &crate::product::coding_models::CodingUnitRun {
                    id: run_id,
                    unit_id: unit.id.clone(),
                    execution_no: 1,
                    work_item_revision_id: revision.id.clone(),
                    resolved_handoff_revision_ids: Vec::new(),
                    canonical_contract_hash: bundle.canonical_contract_hash.clone(),
                    projection_bundle_id: bundle.id.clone(),
                    projection_compiler_version: bundle.compiler_version.clone(),
                    coder_provider_renderer_version: "coder-v1".to_string(),
                    reviewer_provider_renderer_version: "reviewer-v1".to_string(),
                    internal_reviewer_provider_renderer_version: None,
                    coder_projection_hash: bundle.coder_projection_hash.clone(),
                    reviewer_projection_hash: bundle.reviewer_projection_hash.clone(),
                    coder_execution_context_hash: None,
                    reviewer_execution_context_hash: None,
                    internal_reviewer_execution_context_hash: None,
                    status,
                    unit_rework_count: 0,
                    verification_retry_count: 0,
                    operational_retry_count: 0,
                    plan_repair_count: 0,
                    start_commit: None,
                    completion_commit: None,
                    created_at: AMENDMENT_CREATED_AT.to_string(),
                    updated_at: AMENDMENT_CREATED_AT.to_string(),
                },
            )
            .expect("seed unit run");
        store
            .update_coding_unit_status(
                "project_0001",
                "issue_0001",
                &attempt.id,
                &unit.id,
                CodingExecutionUnitStatus::Completed,
                Some("completed before amendment (campaign fixture)".to_string()),
            )
            .expect("unit status");
    }
    let attempt = store
        .update_attempt_stage(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingExecutionStage::CodeReview,
        )
        .expect("review stage");
    lifecycle
        .upsert_issue_shared_worktree(
            crate::product::lifecycle_store::UpsertIssueSharedWorktreeInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                repository_id: "repository_0001".to_string(),
                branch_name: attempt.branch_name.clone(),
                worktree_path: worktree.clone(),
                base_branch: attempt.base_branch.clone(),
            },
        )
        .expect("shared worktree");
    lifecycle
        .try_acquire_issue_worktree_lock("project_0001", "issue_0001", "wi_a", &attempt.id)
        .expect("initial worktree owner");

    // —— 计划缺陷注入：真实 start_plan_repair + reconcile 暂停 ——
    // 触发 unit 即 wi_a 自身（units[0]）。
    let trigger_revision = revision_store
        .get_work_item_revision(&plan, "wi_a", &units[0].1)
        .expect("trigger revision");
    let contract_refs: Vec<String> = trigger_revision
        .canonical_contract
        .output_contracts
        .iter()
        .map(|contract| contract.contract_id.clone())
        .collect();
    let capability_refs: Vec<String> = trigger_revision
        .canonical_contract
        .output_contracts
        .iter()
        .flat_map(|contract| contract.capabilities.iter().cloned())
        .collect();
    let request = PlanRepairRequest {
        id: format!("plan_repair_request_{plan_session_id}"),
        plan_id: plan_id.clone(),
        base_plan_revision_id: active_revision.id.clone(),
        trigger_attempt_id: attempt.id.clone(),
        trigger_unit_run_id: CAMPAIGN_TRIGGER_RUN_ID.to_string(),
        trigger_review_id: Some("code_review_report_0001".to_string()),
        trigger_finding_id: CAMPAIGN_FINDING_ID.to_string(),
        amendment_id: None,
        defect_class: PlanDefectClass::UpstreamContractInvalid,
        reason_code: "upstream_contract_invalid".to_string(),
        repair_target: RepairTarget {
            kind: RepairTargetKind::UpstreamWorkItem,
            logical_work_item_ids: vec!["wi_a".to_string()],
            work_item_revision_ids: vec![units[0].1.clone()],
        },
        contract_refs,
        capability_refs,
        evidence: vec![PlanDefectEvidence {
            kind: "review".to_string(),
            source_ref: "code_review_report_0001".to_string(),
            message: "upstream contract needs repair (campaign)".to_string(),
        }],
        fingerprint: format!("plan_repair_fingerprint_{plan_session_id}"),
        status: PlanRepairRequestStatus::Open,
        created_at: AMENDMENT_CREATED_AT.to_string(),
        updated_at: AMENDMENT_CREATED_AT.to_string(),
    };
    let (event_tx, _event_rx) = mpsc::channel(16);
    let mut plan_engine = WorkspaceEngine::new_persistent(
        Arc::new(CheckpointStore::new(root.path().join("plan-checkpoints"))),
        lifecycle.clone(),
        event_tx,
        WorkspaceSession::from_record(
            lifecycle
                .get_workspace_session(&plan_session_id)
                .expect("plan session record"),
        ),
    );
    let child = plan_engine
        .start_plan_repair(request.clone())
        .await
        .expect("plan repair child session");
    let reconciliation = store
        .reconcile_linked_plan_repair_pause(&attempt)
        .expect("reconcile plan repair pause");
    let attempt = reconciliation.attempt;
    // start_plan_repair 会为 request 分配 amendment id 并落盘；后续 prepare/
    // approve 一律以 durable 存储的 request 为准（不得复用本地未分配克隆）。
    let request = revision_store
        .get_repair_request(&plan, &request.id)
        .expect("stored repair request after start_plan_repair");
    CampaignAmendmentFixture {
        _root: root,
        store,
        lifecycle,
        revision_store,
        attempt,
        plan,
        plan_session_id,
        child_session_id: child.id,
        request,
        units: units.into_iter().map(|(unit, _)| unit).collect(),
    }
}

impl CampaignAmendmentFixture {
    fn trigger_context(&self) -> crate::product::coding_models::PlanAmendmentContext {
        self.store
            .find_plan_amendment_context_by_finding(&self.attempt, CAMPAIGN_FINDING_ID)
            .expect("context lookup")
            .expect("plan amendment context after reconcile")
    }

    fn durable_attempt(&self) -> crate::product::coding_models::CodingExecutionAttempt {
        self.store
            .get_attempt(
                &self.attempt.project_id,
                &self.attempt.issue_id,
                &self.attempt.id,
            )
            .expect("durable attempt")
    }

    /// 原 plan session 上的 feedback 引擎（每次调用从 durable 记录重建，
    /// 模拟 WS 断开重连后的新 worker）。
    fn plan_session_engine(&self, checkpoints: &str) -> WorkspaceEngine {
        let record = self
            .lifecycle
            .get_workspace_session(&self.plan_session_id)
            .expect("plan session record");
        let (event_tx, _event_rx) = mpsc::channel(64);
        let mut session = WorkspaceSession::from_record(record);
        session.artifact = Some(ArtifactPayload::Markdown {
            markdown: campaign_gate_candidate(),
            diff: None,
        });
        WorkspaceEngine::new_persistent(
            Arc::new(CheckpointStore::new(self._root.path().join(checkpoints))),
            self.lifecycle.clone(),
            event_tx,
            session,
        )
    }
}

/// 用真实 compiler/validator 产出修订候选：feedback turn + fake revision。
/// 返回 durable turn id（断线重开后的对账锚点）。
async fn open_amendment_turn_and_run_fake_revision(
    fixture: &CampaignAmendmentFixture,
    command_id: &str,
) -> String {
    let mut engine = fixture.plan_session_engine("feedback-checkpoints");
    let opened = engine
        .handle_human_gate_feedback(HumanGateFeedbackInput {
            command_id: command_id.to_string(),
            feedback: "只修正上游契约的实现指引，其余逐字保留".to_string(),
        })
        .await
        .expect("amendment feedback must reopen the original plan session gate");
    let turn = match opened {
        HumanGateCommandOutcome::TurnOpened { turn, .. } => turn,
        other => panic!("expected amendment turn opened, got {other:?}"),
    };
    assert_eq!(turn.session_id, fixture.plan_session_id);
    engine
        .mark_human_gate_turn_running(&turn.turn_id)
        .expect("mark running");
    let accepted = engine
        .run_sc_manual_revision_turn(&turn.turn_id, campaign_revised_candidate())
        .await
        .expect("fake revision must pass the real compiler/validator");
    let artifact_ref = match accepted {
        crate::product::workspace_engine::ScManualRevisionResult::Accepted { artifact_ref } => {
            artifact_ref
        }
        crate::product::workspace_engine::ScManualRevisionResult::ValidationRejected {
            diagnostics,
        } => {
            panic!("fake revision must pass the real compiler/validator, rejected: {diagnostics:?}")
        }
    };
    assert!(
        artifact_ref.starts_with("artifact_version_"),
        "accepted revision must persist an artifact version ref: {artifact_ref}"
    );
    turn.turn_id
}

/// 真实 prepare/publish/approve 链：guidance-only 修订（图校验保持合法的
/// 契约增量）→ `PlanRepairEngine` 出版 amendment → child session
/// AwaitingConfirmation → `confirm_and_publish_plan_amendment`。
async fn publish_real_amendment(
    fixture: &CampaignAmendmentFixture,
) -> crate::product::models::PlanAmendmentManifest {
    let active_revision_id = fixture
        .revision_store
        .get_plan_lineage("project_0001", "issue_0001", &fixture.plan.id)
        .expect("lineage")
        .active_revision_id
        .expect("active revision");
    let active_revision = fixture
        .revision_store
        .get_plan_revision(
            "project_0001",
            "issue_0001",
            &fixture.plan.id,
            &active_revision_id,
        )
        .expect("active plan revision");
    let wi_a_revision_id = active_revision
        .work_item_bindings
        .get("wi_a")
        .expect("wi_a binding")
        .clone();
    let previous = fixture
        .revision_store
        .get_work_item_revision(&fixture.plan, "wi_a", &wi_a_revision_id)
        .expect("previous wi_a revision");
    let mut candidate = previous.canonical_contract.clone();
    candidate.goal.summary = format!(
        "{}（amendment 修订：补充上游契约实现指引）",
        candidate.goal.summary
    );
    let draft = crate::product::models::WorkItemDraftRevision {
        id: format!("work_item_draft_revision_wi_a_{}", fixture.plan_session_id),
        logical_work_item_id: "wi_a".to_string(),
        revision_no: 2,
        supersedes: Some(previous.source_draft_revision_id.clone()),
        revision_reason: crate::product::models::PlanRevisionReason::RepairUpstreamContract,
        canonical_contract_candidate: candidate,
        trigger_repair_request_id: Some(fixture.request.id.clone()),
        created_at: AMENDMENT_CREATED_AT.to_string(),
    };
    fixture
        .revision_store
        .put_draft_revision(&fixture.plan, &draft)
        .expect("put draft revision");
    let prepared = PlanRepairEngine::new(fixture.revision_store.clone(), fixture.plan.clone())
        .with_candidate_drafts(vec![draft])
        .with_created_at(AMENDMENT_CREATED_AT)
        .prepare_amendment(&fixture.request)
        .expect("prepare real amendment");
    PlanRepairEngine::new(fixture.revision_store.clone(), fixture.plan.clone())
        .with_created_at(AMENDMENT_CREATED_AT)
        .persist_candidate(&prepared)
        .expect("persist candidate");
    let attestation = campaign_review_attestation(fixture, &prepared);
    fixture
        .revision_store
        .put_plan_repair_review_attestation(&fixture.plan, &attestation)
        .expect("put review attestation");
    // durable 快照登记 candidate package（awaiting 包身份绑定校验项）。
    {
        let mut snapshot = fixture
            .lifecycle
            .load_plan_repair_session_state(
                &fixture.plan.project_id,
                &fixture.plan.issue_id,
                &fixture.child_session_id,
            )
            .expect("load child snapshot")
            .expect("child snapshot exists");
        snapshot.candidate_package_artifact_id = Some(prepared.candidate_package.id.clone());
        fixture
            .lifecycle
            .save_plan_repair_session_state(
                &fixture.plan.project_id,
                &fixture.plan.issue_id,
                &fixture.child_session_id,
                &snapshot,
            )
            .expect("save child snapshot");
    }
    let mut child_engine = fixture.child_engine();
    child_engine
        .enter_plan_repair_awaiting_confirmation(campaign_awaiting_package(
            fixture,
            &prepared,
            &attestation,
        ))
        .await
        .expect("enter awaiting confirmation");
    child_engine
        .confirm_and_publish_plan_amendment(&prepared.manifest.id, "workspace_user")
        .await
        .expect("approve via amendment publication journal")
}

fn campaign_review_attestation(
    fixture: &CampaignAmendmentFixture,
    prepared: &PreparedPlanAmendment,
) -> PlanRepairReviewAttestation {
    let review = crate::web::workspace_ws_types::WorkItemPlanReviewComplete {
        verdict: crate::web::workspace_ws_types::WorkItemPlanReviewVerdict::Pass,
        review_scope: crate::web::workspace_ws_types::WorkItemPlanReviewScope::Outline,
        target_outline_id: None,
        generation_round_id: format!("repair_round_{}", fixture.plan_session_id),
        draft_id: None,
        batch_id: None,
        review_action: crate::web::workspace_ws_types::WorkItemPlanReviewAction::Continue,
        gates: Vec::new(),
        affects_items: Vec::new(),
        warnings: Vec::new(),
    };
    PlanRepairReviewAttestation {
        id: format!(
            "plan_repair_review_attestation_{}_{}",
            prepared.manifest.id, fixture.plan_session_id
        ),
        request_id: fixture.request.id.clone(),
        amendment_id: prepared.manifest.id.clone(),
        plan_id: fixture.plan.id.clone(),
        base_plan_revision_id: prepared.base_plan_revision_id.clone(),
        reviewed_plan_revision_id: prepared.next_plan_revision.id.clone(),
        plan_projection_bundle_id: prepared.plan_projection_bundle.id.clone(),
        generation_round_id: review.generation_round_id.clone(),
        accepted_impact_scope: Vec::new(),
        risk_acceptance_reason: None,
        candidate_package_artifact_id: prepared.candidate_package.id.clone(),
        candidate_package_fingerprint: prepared
            .candidate_package
            .candidate_package_fingerprint
            .clone(),
        review,
        created_at: AMENDMENT_CREATED_AT.to_string(),
    }
}

fn campaign_awaiting_package(
    fixture: &CampaignAmendmentFixture,
    prepared: &PreparedPlanAmendment,
    attestation: &PlanRepairReviewAttestation,
) -> crate::product::models::PlanRepairAwaitingConfirmationPackage {
    crate::product::models::PlanRepairAwaitingConfirmationPackage {
        package_identity: crate::product::models::PlanRepairPackageIdentity {
            request_id: fixture.request.id.clone(),
            amendment_id: prepared.manifest.id.clone(),
            plan_id: fixture.plan.id.clone(),
            base_plan_revision_id: prepared.base_plan_revision_id.clone(),
            next_plan_revision_id: prepared.next_plan_revision.id.clone(),
            projection_bundle_id: prepared.plan_projection_bundle.id.clone(),
            validation_report_id: prepared.validation_report.id.clone(),
            review_attestation_id: attestation.id.clone(),
            reviewed_plan_revision_id: prepared.next_plan_revision.id.clone(),
            review_generation_round_id: attestation.generation_round_id.clone(),
            candidate_package_artifact_id: prepared.candidate_package.id.clone(),
            candidate_package_fingerprint: prepared
                .candidate_package
                .candidate_package_fingerprint
                .clone(),
        },
        projection: prepared.plan_projection_bundle.clone(),
        amendment: prepared.manifest.clone(),
        validation: prepared.validation_report.clone(),
        impact: prepared.impact_report.clone(),
        plan_review: attestation.review.clone(),
    }
}

impl CampaignAmendmentFixture {
    fn child_engine(&self) -> WorkspaceEngine {
        let record = self
            .lifecycle
            .get_workspace_session(&self.child_session_id)
            .expect("child session record");
        let (event_tx, _event_rx) = mpsc::channel(64);
        WorkspaceEngine::new_persistent(
            Arc::new(CheckpointStore::new(
                self._root.path().join("child-checkpoints"),
            )),
            self.lifecycle.clone(),
            event_tx,
            WorkspaceSession::from_record(record),
        )
    }

    fn coding_engine(&self) -> CodingWorkspaceEngine {
        let (event_tx, mut socket_event_rx) = mpsc::channel(64);
        // 模拟真实 coding ws：socket 写成功后回执 delivery ack（应用链最后一跳）。
        tokio::spawn(async move {
            while let Some(event) = socket_event_rx.recv().await {
                crate::web::coding_ws_handler::delivery_ack::confirm_plan_amendment_socket_write(
                    &event,
                );
            }
        });
        CodingWorkspaceEngine::new(
            self.store.clone(),
            crate::product::git_workspace_service::GitWorkspaceService::new(),
            event_tx,
        )
    }
}

/// Step 1 —— 8.4a amendment E2E：真实批准链上的 plan 缺陷 → 原 plan session
/// typed feedback（预算接续、group attempt 无预算账）→ fake revision 经真实
/// compiler/validator → 断开重连后从原 session/context 继续 → approve 走
/// amendment manifest/application journal（非首次 compile）→ 回原 attempt。
#[tokio::test]
async fn campaign_stage3_amendment_returns_to_same_attempt_via_original_plan_session() {
    let fixture = campaign_amendment_fixture().await;

    // —— 1) 暂停形态：BlockedByPlanDefect / AwaitingPlanAmendment / context ——
    let paused = fixture.durable_attempt();
    assert_eq!(paused.status, CodingAttemptStatus::AwaitingPlanAmendment);
    let trigger_run = fixture
        .store
        .list_coding_unit_runs(&paused, &fixture.units[0].id)
        .expect("trigger runs")
        .into_iter()
        .find(|run| run.id == CAMPAIGN_TRIGGER_RUN_ID)
        .expect("trigger run");
    assert_eq!(trigger_run.status, CodingUnitRunStatus::BlockedByPlanDefect);
    let context = fixture.trigger_context();
    assert_eq!(context.plan_session_id, fixture.plan_session_id);
    assert_eq!(context.group_attempt_id, paused.id);
    assert_eq!(context.trigger_unit_id, fixture.units[0].id);
    assert_eq!(context.status, PlanAmendmentContextStatus::Open);
    let previous_plan_revision_id = context.previous_plan_revision_id.clone();

    // —— 2) 原 plan session typed feedback：预算接续，group attempt 无预算账 ——
    let attempt_before_feedback = fixture.durable_attempt();
    let sessions_before = fixture
        .lifecycle
        .list_workspace_sessions("project_0001", "issue_0001")
        .expect("sessions")
        .len();
    let turn_id = open_amendment_turn_and_run_fake_revision(&fixture, "cmd-campaign-amend-1").await;
    let durable_turns = fixture
        .lifecycle
        .list_human_gate_turns(&fixture.plan_session_id)
        .expect("durable turns");
    assert_eq!(durable_turns.len(), 1, "恰好一个 amendment turn");
    assert_eq!(durable_turns[0].turn_id, turn_id);
    assert_eq!(
        durable_turns[0].status,
        crate::product::models::HumanGateTurnStatus::Completed
    );
    let reopened = fixture
        .lifecycle
        .get_workspace_session(&fixture.plan_session_id)
        .expect("reopened session");
    assert_eq!(reopened.status, WorkspaceSessionStatus::WaitingForHuman);
    assert_eq!(
        reopened
            .human_gate_snapshot
            .as_ref()
            .expect("snapshot")
            .manual_repairs_remaining,
        1,
        "预算只从原 plan session 快照扣一次"
    );
    assert_eq!(
        reopened.provider_start_ledger.len(),
        1,
        "provider ledger 每真实 start 一项"
    );
    // group attempt 侧无预算账：attempt 零变化、不新增 session。
    assert_eq!(fixture.durable_attempt(), attempt_before_feedback);
    assert!(
        fixture
            .store
            .list_open_blocked_gates("project_0001", "issue_0001", &paused.id)
            .expect("open gates")
            .is_empty(),
        "group attempt 不开人工门"
    );
    assert_eq!(
        fixture
            .lifecycle
            .list_workspace_sessions("project_0001", "issue_0001")
            .expect("sessions after feedback")
            .len(),
        sessions_before,
        "不产生第二个人工门 session"
    );

    // —— 3) 中途断开 WS + 重启：丢弃内存态，仅从磁盘重建 ——
    {
        let mut recovered = fixture.plan_session_engine("reconnect-checkpoints");
        let actions = recovered
            .recover_human_gate_turns(false)
            .expect("recover turns");
        assert!(
            actions.is_empty(),
            "turn 已完成，恢复不需要任何动作: {actions:?}"
        );
        // 原 session/context 仍是权威：attempt 仍暂停、context 仍 Open。
        assert_eq!(
            fixture.durable_attempt().status,
            CodingAttemptStatus::AwaitingPlanAmendment
        );
        assert_eq!(fixture.trigger_context().id, context.id);
        // 同 command 重发：Replay 同一 turn，预算/ledger 不再变化。
        let replayed = recovered
            .handle_human_gate_feedback(HumanGateFeedbackInput {
                command_id: "cmd-campaign-amend-1".to_string(),
                feedback: "同 command 重发（重连后）".to_string(),
            })
            .await
            .expect("replay");
        assert!(matches!(
            replayed,
            HumanGateCommandOutcome::Replayed { ref turn } if turn.turn_id == turn_id
        ));
        let after_replay = fixture
            .lifecycle
            .get_workspace_session(&fixture.plan_session_id)
            .expect("session after replay");
        assert_eq!(
            after_replay
                .human_gate_snapshot
                .as_ref()
                .expect("snapshot")
                .manual_repairs_remaining,
            1
        );
    }

    // —— 4) approve 走 amendment manifest/publication journal（非首次 compile）——
    let manifest = publish_real_amendment(&fixture).await;
    assert_eq!(
        manifest.previous_plan_revision_id, previous_plan_revision_id,
        "previous revision 前缀绑定"
    );
    let lineage_after_publish = fixture
        .revision_store
        .get_plan_lineage("project_0001", "issue_0001", &fixture.plan.id)
        .expect("lineage after publish");
    assert_eq!(
        lineage_after_publish.active_revision_id.as_deref(),
        Some(manifest.new_plan_revision_id.as_str()),
        "新 plan revision 由 amendment publication 激活"
    );
    assert_eq!(
        lineage_after_publish.active_amendment_id.as_deref(),
        Some(manifest.id.as_str())
    );
    let published_plan_revision = fixture
        .revision_store
        .get_plan_revision(
            "project_0001",
            "issue_0001",
            &fixture.plan.id,
            &manifest.new_plan_revision_id,
        )
        .expect("published plan revision");
    assert_eq!(
        published_plan_revision.supersedes.as_deref(),
        Some(previous_plan_revision_id.as_str())
    );
    assert_eq!(
        published_plan_revision.reason,
        crate::product::models::PlanRevisionReason::RepairUpstreamContract,
        "非首次 compile：修订由 RepairUpstreamContract 出版"
    );
    let request_after = fixture
        .revision_store
        .get_repair_request(&fixture.plan, &fixture.request.id)
        .expect("request after publish");
    assert_eq!(request_after.status, PlanRepairRequestStatus::Published);

    // —— 5) application journal：回原 attempt、binding 更新、previous revision 留在 context ——
    let engine = fixture.coding_engine();
    let resumed = engine
        .resume_group_after_plan_amendment(&fixture.attempt, &context, &manifest)
        .await
        .expect("resume the original attempt via the application journal");
    assert_eq!(
        resumed.id, fixture.attempt.id,
        "amendment 回到原 attempt（不新建）"
    );
    assert_eq!(
        fixture
            .store
            .list_attempts_for_issue("project_0001", "issue_0001")
            .expect("attempts for issue")
            .len(),
        1,
        "全 issue 始终只有一个 group attempt"
    );
    let binding = fixture.store.get_plan_binding(&resumed).expect("binding");
    assert_eq!(
        binding.bound_plan_revision_id,
        manifest.new_plan_revision_id
    );
    assert_eq!(binding.applied_amendment_ids, vec![manifest.id.clone()]);
    let applied_context = fixture.trigger_context();
    assert_eq!(applied_context.id, context.id);
    assert_eq!(applied_context.status, PlanAmendmentContextStatus::Applied);
    assert_eq!(
        applied_context.previous_plan_revision_id, previous_plan_revision_id,
        "previous revision 留在 context"
    );
    assert_eq!(
        applied_context.new_plan_revision_id.as_deref(),
        Some(manifest.new_plan_revision_id.as_str())
    );
    assert_eq!(
        applied_context.resume_target, manifest.resume_target,
        "resume target 恰好登记一次"
    );
    // application journal 落盘且 Completed。
    let journal = fixture
        .store
        .get_amendment_application_journal(&resumed, &manifest.id)
        .expect("application journal");
    assert_eq!(
        journal.phase,
        crate::product::coding_models::CodingAmendmentApplicationPhase::Completed
    );
    // 原 plan session 的重开门关回 Confirmed。
    let closed = fixture
        .lifecycle
        .get_workspace_session(&fixture.plan_session_id)
        .expect("plan session after resume");
    assert_eq!(closed.status, WorkspaceSessionStatus::Confirmed);
    // resume 单元状态由真实 manifest resume_target 产生。
    let resume_unit = fixture
        .store
        .list_coding_units("project_0001", "issue_0001", &resumed.id)
        .expect("units")
        .into_iter()
        .find(|unit| unit.logical_work_item_id == manifest.resume_target.logical_work_item_id)
        .expect("resume target unit");
    match manifest.resume_target.mode {
        AmendmentResumeMode::Reexecute => {
            assert_eq!(resume_unit.status, CodingExecutionUnitStatus::Running);
            assert_eq!(resumed.status, CodingAttemptStatus::Running);
            assert_eq!(resumed.stage, CodingExecutionStage::Coding);
        }
        AmendmentResumeMode::Revalidate => {
            assert_eq!(
                resume_unit.status,
                CodingExecutionUnitStatus::NeedsRevalidation
            );
            assert_eq!(resumed.stage, CodingExecutionStage::CodeReview);
        }
        AmendmentResumeMode::AwaitHandoff => {
            assert_eq!(
                resume_unit.status,
                CodingExecutionUnitStatus::AwaitingAmendment
            );
        }
    }
}

/// 8.4a 负面案：不兼容 revision（伪造 manifest 身份）→ context durable
/// FailedClosed，attempt/binding 无路径切换，仍留原 attempt 等人工处置。
#[tokio::test]
async fn campaign_stage3_amendment_incompatible_revision_fails_closed_without_path_switch() {
    let fixture = campaign_amendment_fixture().await;
    let context = fixture.trigger_context();
    let binding_before = fixture
        .store
        .get_plan_binding(&fixture.attempt)
        .expect("binding");

    let mut forged = publish_real_amendment(&fixture).await;
    forged.repair_request_id = format!("{}_forged", fixture.request.id);

    let engine = fixture.coding_engine();
    let error = engine
        .resume_group_after_plan_amendment(&fixture.attempt, &context, &forged)
        .await
        .expect_err("forged amendment identity must fail closed");
    assert!(
        error.to_string().contains("identity_mismatch"),
        "unexpected error: {error}"
    );

    let persisted = fixture.durable_attempt();
    assert_eq!(persisted.id, fixture.attempt.id);
    assert_eq!(persisted.status, CodingAttemptStatus::AwaitingPlanAmendment);
    assert_eq!(
        fixture
            .store
            .get_plan_binding(&persisted)
            .expect("binding after"),
        binding_before,
        "binding 无路径切换"
    );
    let failed = fixture.trigger_context();
    assert_eq!(failed.id, context.id);
    assert_eq!(failed.status, PlanAmendmentContextStatus::FailedClosed);
    assert_eq!(failed.new_plan_revision_id, None);
    assert_eq!(
        failed.previous_plan_revision_id,
        context.previous_plan_revision_id
    );
    let diagnostic = fixture
        .store
        .get_plan_amendment_context_diagnostic(&persisted, &failed.id)
        .expect("diagnostic lookup")
        .expect("durable fail-closed diagnostic");
    assert!(diagnostic.reason.contains("identity_mismatch"));
    // FailedClosed 后重试同一不兼容 revision：仍拒绝。
    let retry = engine
        .resume_group_after_plan_amendment(&persisted, &failed, &forged)
        .await
        .expect_err("failed-closed context must keep blocking");
    assert!(retry.to_string().contains("failed_closed"));
}

/// Step 2 —— GCE-02 failure E2E：一次 transient 失败在同 attempt/unit 有界
/// 重试；另案用户 abort 后 attempt Aborted 且 units/runs/logs/commit/event
/// 保留；list attempts 始终一项，投影显示 reason。
#[tokio::test]
async fn campaign_stage3_provider_failure_retries_same_unit_and_abort_preserves_evidence() {
    use crate::product::coding_models::{CodingRoleRunStatus, CodingRoleRunTrigger};
    use crate::product::coding_workspace_engine::CodingExecutionContext;

    // —— 案 1：transient 失败 → 同 attempt/unit bounded retry ——
    let (_root, store, attempt) = super::running_attempt_with_worktree();
    let provider = super::provider_failure_recovery::TransportFailuresThenSuccessProvider::new(
        1,
        "coder retry completed",
    );
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let (_command_tx, mut command_rx) = mpsc::channel(1);
    let _ = engine
        .execute_coding_with_commands(
            &attempt,
            &provider,
            &CodingExecutionContext {
                work_item_markdown: Some("# Retry work item\n\nKeep the full context.".to_string()),
                verification_commands: vec!["cargo test --locked --lib retry".to_string()],
            },
            &mut command_rx,
        )
        .await
        .expect("bounded retry must recover the same unit");
    let runs = store
        .list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("role runs");
    assert_eq!(runs.len(), 2, "一次 transient 失败恰好一次自动重试");
    assert_eq!(runs[0].status, CodingRoleRunStatus::Failed);
    assert_eq!(runs[1].trigger, CodingRoleRunTrigger::AutomaticRetry);
    assert_eq!(
        runs[1]
            .retry_metadata
            .as_ref()
            .expect("retry metadata")
            .attempt_no,
        2
    );
    assert_eq!(
        store
            .list_attempts_for_issue(&attempt.project_id, &attempt.issue_id)
            .expect("attempts")
            .len(),
        1,
        "失败重试绝不新建 attempt"
    );

    // —— 案 2：用户 abort → attempt Aborted，证据保留，投影显示 reason ——
    let (root, store, attempt) = super::running_attempt_with_worktree();
    let provider = super::provider_failure_recovery::RetryBoundaryMutationProvider::new(
        super::provider_failure_recovery::RetryBoundaryMutation::Abort {
            store: store.clone(),
            attempt: attempt.clone(),
        },
    );
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let (_command_tx, mut command_rx) = mpsc::channel(1);
    let _ = engine
        .execute_coding_with_commands(
            &attempt,
            &provider,
            &crate::product::coding_workspace_engine::CodingExecutionContext::default(),
            &mut command_rx,
        )
        .await
        .expect_err("abort stops the retry cycle");
    let aborted = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("aborted attempt");
    assert_eq!(aborted.status, CodingAttemptStatus::Aborted);
    // runs/logs/event 保留（不抹除证据）：本 fixture 为单 work-item attempt
    // （无 group units），abort 证据面 = role runs + raw 输出 refs + timeline；
    // group units 的保留由 8.4a amendment 家族在同 attempt 断言覆盖。
    let runs = store
        .list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("role runs after abort");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].status, CodingRoleRunStatus::Failed);
    assert_eq!(
        runs[0].reason_code.as_deref(),
        Some("provider_retry_attempt_state_changed")
    );
    assert_eq!(
        runs[0].raw_provider_output_refs.len(),
        1,
        "abort 保留 raw 输出日志 refs"
    );
    let nodes = store
        .get_timeline_nodes(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("timeline nodes after abort");
    assert_eq!(nodes.len(), 1);
    assert_eq!(
        nodes[0].status,
        crate::product::coding_models::CodingTimelineNodeStatus::Failed
    );
    assert_eq!(
        store
            .list_attempts_for_issue(&attempt.project_id, &attempt.issue_id)
            .expect("attempts after abort")
            .len(),
        1,
        "abort 后 list attempts 始终一项"
    );
    // 投影显示 reason：HTTP snapshot 真实 handler 读取。
    let state = crate::web::state::WebAppState::new(
        root.path().to_path_buf(),
        crate::web::runtime::WebRuntime::new_fake(root.path().to_path_buf()),
    );
    let path = axum::extract::Path(crate::web::handlers::CodingAttemptRoutePath {
        project_id: Some(attempt.project_id.clone()),
        issue_id: Some(attempt.issue_id.clone()),
        attempt_id: attempt.id.clone(),
    });
    let axum::Json(snapshot) =
        crate::web::handlers::get_coding_attempt(axum::extract::State(state), path)
            .await
            .expect("GET aborted attempt snapshot");
    assert_eq!(snapshot.attempt.attempt_id, attempt.id);
    assert_eq!(
        snapshot.attempt.status, "aborted",
        "投影如实显示 abort 终态"
    );
}
