//! 7.2 快照断裂追修：REQ-GCE-03 修订链真链路触达测试。
//!
//! 与 `coding_workspace_engine::tests::group_amendment_chain` 的手工终态 fixture
//! 不同，本文件的 plan session 终态（Confirmed+Completed+门快照在场）完全由
//! 真实引擎批准链产生（裸 Confirm → close_human_gate → compile →
//! confirm_work_item_plan），编码侧计划缺陷复用 7.2 既有注入手段（seeded
//! PlanRepairRequest + trigger unit run），随后向原 plan session 发 typed
//! feedback，断言 probe 放行、CAS 重开成功且预算只从原快照扣一次。

use super::*;
use crate::product::coding_attempt_store::{
    CodingAttemptStore, CreateCodingExecutionUnitInput, CreateGroupCodingAttemptInput,
};
use crate::product::coding_models::{
    CodingAttemptPlanBinding, CodingAttemptStatus, CodingExecutionUnitStatus,
    PlanAmendmentContextStatus,
};
use crate::product::models::{
    PlanDefectClass, PlanDefectEvidence, PlanRepairRequest, PlanRepairRequestStatus,
    SingleCandidatePhase, WorkspaceSessionStatus,
};
use crate::product::work_item_plan_policy::{HumanGateSnapshot, HumanReason, RunPolicy};
use crate::product::work_item_revision_store::WorkItemRevisionStore;
use crate::product::workspace_engine::{HumanGateCommandOutcome, HumanGateFeedbackInput};
use tempfile::TempDir;

const TRIGGER_FINDING_ID: &str = "code_review_report_0001_finding_0001";
const TRIGGER_RUN_ID: &str = "coding_unit_run_blocked";

/// 真实批准链 fixture：accepted contract drafts → SingleCandidate Approval 门
/// （快照在场、预算 `budget`）→ 裸 Confirm 走完整 close→compile→confirm 链。
async fn real_approval_fixture(budget: u32) -> (TempDir, LifecycleStore, String, WorkspaceEngine) {
    let (tmp, lifecycle, plan_id, mut engine) =
        super::make_work_item_plan_engine_with_accepted_contract_drafts();
    super::single_candidate_recovery::single_candidate_recovery_record(
        &lifecycle,
        &mut engine,
        SingleCandidatePhase::Approval,
        RunPolicy::Interactive,
    );
    // 生产 SC 流在门开启前会经 update_artifact(Markdown{source}) 把候选文本持久化
    // 为 artifact version（single_candidate 流程）；recovery 式 fixture 需补齐这
    // 一 durable 形态，否则批准链 compile 后 durable versions 完全无 Markdown，
    // 修订回落的「批准时计划文本」无从取材。
    engine
        .update_artifact(ArtifactPayload::Markdown {
            markdown: "# Work Item Plan\n\n## Work Item WI-001: candidate\n".to_string(),
            diff: None,
        })
        .await;
    let mut record = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("session");
    record.flow_kind = WorkItemPlanFlowKind::SingleCandidate;
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
        &lifecycle
            .app_paths()
            .issue_root(&record.project_id, &record.issue_id)
            .join("workspace-sessions")
            .join(format!("{}.json", record.id)),
        &record,
    )
    .expect("persist approval session");
    let artifact = engine.session.artifact.clone();
    engine.session = WorkspaceSession::from_record(record);
    engine.session.stage = WorkspaceStage::HumanConfirm;
    engine.session.session_status = WorkspaceSessionStatus::WaitingForHuman;
    engine.session.artifact = artifact;
    (tmp, lifecycle, plan_id, engine)
}

fn seed_real_unit_run(
    store: &CodingAttemptStore,
    plan: &crate::product::models::WorkItemPlanLineage,
    attempt: &crate::product::coding_models::CodingExecutionAttempt,
    unit: &crate::product::coding_models::CodingExecutionUnit,
    work_item_revision_id: &str,
    run_id: &str,
    status: crate::product::coding_models::CodingUnitRunStatus,
) {
    let revisions = WorkItemRevisionStore::new(store.paths());
    let revision = revisions
        .get_work_item_revision(plan, &unit.logical_work_item_id, work_item_revision_id)
        .expect("real work item revision");
    let bundle = revisions
        .get_work_item_projection_bundle(plan, &revision.work_item_projection_bundle_id)
        .expect("real projection bundle");
    store
        .create_coding_unit_run(
            attempt,
            &crate::product::coding_models::CodingUnitRun {
                id: run_id.to_string(),
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
                created_at: "2026-08-31T00:00:00Z".to_string(),
                updated_at: "2026-08-31T00:00:00Z".to_string(),
            },
        )
        .expect("seed real unit run");
}

/// 既有 `group_amendment_reachable_from_real_approval_chain` 用例的播种段提取：
/// 真实编译产物上建 group attempt + 两个 unit + seeded runs，注入 7.2 计划缺陷
/// （PlanRepairRequest → start_plan_repair → reconcile），使 group attempt 停在
/// AwaitingPlanAmendment 且存在指向本 plan session 的 Open PlanAmendmentContext。
/// 返回 (coding store, 暂停 attempt id, child repair session id)。
async fn seed_awaiting_plan_amendment(
    root: &tempfile::TempDir,
    lifecycle: &LifecycleStore,
    engine: &mut WorkspaceEngine,
    plan_id: &str,
    plan_session_id: &str,
) -> (CodingAttemptStore, String, String) {
    let project_id = engine.session().project_id.clone();
    let issue_id = engine.session().issue_id.clone();
    // ---- 2) 编码侧：真实编译产物上的 group attempt + 7.2 既有计划缺陷注入 ----
    let store = CodingAttemptStore::new(lifecycle.app_paths());
    let worktree = root.path().join("worktree");
    std::fs::create_dir_all(&worktree).unwrap();
    let initial = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: project_id.clone(),
            issue_id: issue_id.clone(),
            plan_id: plan_id.to_string(),
            current_work_item_id: "wi_a".to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: Some(worktree),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
                permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
            },
            target_snapshot: None,
            max_auto_rework: 2,
        })
        .expect("group attempt");
    let revision_store = WorkItemRevisionStore::new(lifecycle.app_paths());
    let plan = revision_store
        .get_plan_lineage(&project_id, &issue_id, plan_id)
        .expect("real plan lineage");
    let active_revision = revision_store
        .get_plan_revision(
            &project_id,
            &issue_id,
            plan_id,
            plan.active_revision_id
                .as_deref()
                .expect("compiled active revision"),
        )
        .expect("real plan revision");
    let ordered_logical_ids = ["wi_a".to_string(), "wi_b".to_string()];
    let mut units = Vec::new();
    for (index, logical_id) in ordered_logical_ids.iter().enumerate() {
        let revision_id = active_revision
            .work_item_bindings
            .get(logical_id)
            .expect("real binding")
            .clone();
        let unit = store
            .create_coding_unit(CreateCodingExecutionUnitInput {
                attempt_id: initial.id.clone(),
                project_id: project_id.clone(),
                issue_id: issue_id.clone(),
                plan_id: plan_id.to_string(),
                logical_work_item_id: logical_id.clone(),
                work_item_revision_id: revision_id.clone(),
                dependency_logical_work_item_ids: if index == 1 {
                    vec!["wi_a".to_string()]
                } else {
                    Vec::new()
                },
                order_index: index as u32,
                status: if index == 0 {
                    CodingExecutionUnitStatus::Running
                } else {
                    CodingExecutionUnitStatus::Pending
                },
            })
            .expect("coding unit");
        units.push((unit, revision_id));
    }
    store
        .save_plan_binding(
            &initial,
            &CodingAttemptPlanBinding {
                attempt_id: initial.id.clone(),
                plan_id: plan_id.to_string(),
                bound_plan_revision_id: active_revision.id.clone(),
                applied_amendment_ids: Vec::new(),
                updated_at: "2026-08-31T00:00:00Z".to_string(),
            },
        )
        .expect("attempt plan binding");
    let attempt = store
        .seed_running_attempt_for_test(&project_id, &issue_id, &initial.id)
        .expect("running attempt");
    seed_real_unit_run(
        &store,
        &plan,
        &attempt,
        &units[0].0,
        &units[0].1,
        "coding_unit_run_completed",
        crate::product::coding_models::CodingUnitRunStatus::Completed,
    );
    seed_real_unit_run(
        &store,
        &plan,
        &attempt,
        &units[1].0,
        &units[1].1,
        TRIGGER_RUN_ID,
        crate::product::coding_models::CodingUnitRunStatus::Running,
    );
    store
        .update_coding_unit_status(
            &project_id,
            &issue_id,
            &attempt.id,
            &units[0].0.id,
            CodingExecutionUnitStatus::Completed,
            Some("completed before amendment".to_string()),
        )
        .expect("complete first unit");
    store
        .update_coding_unit_status(
            &project_id,
            &issue_id,
            &attempt.id,
            &units[1].0.id,
            CodingExecutionUnitStatus::Running,
            None,
        )
        .expect("running trigger unit");

    let trigger_revision = revision_store
        .get_work_item_revision(&plan, "wi_b", &units[1].1)
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
        id: "plan_repair_request_0001".to_string(),
        plan_id: plan_id.to_string(),
        base_plan_revision_id: active_revision.id.clone(),
        trigger_attempt_id: attempt.id.clone(),
        trigger_unit_run_id: TRIGGER_RUN_ID.to_string(),
        trigger_review_id: Some("code_review_report_0001".to_string()),
        trigger_finding_id: TRIGGER_FINDING_ID.to_string(),
        amendment_id: None,
        defect_class: PlanDefectClass::UpstreamContractInvalid,
        reason_code: "upstream_contract_invalid".to_string(),
        repair_target: crate::product::models::RepairTarget {
            kind: crate::product::models::RepairTargetKind::UpstreamWorkItem,
            logical_work_item_ids: vec!["wi_a".to_string()],
            work_item_revision_ids: vec![units[0].1.clone()],
        },
        contract_refs,
        capability_refs,
        evidence: vec![PlanDefectEvidence {
            kind: "review".to_string(),
            source_ref: "code_review_report_0001".to_string(),
            message: "upstream contract needs repair".to_string(),
        }],
        fingerprint: "plan_repair_fingerprint_real_0001".to_string(),
        status: PlanRepairRequestStatus::Open,
        created_at: "2026-08-31T00:00:00Z".to_string(),
        updated_at: "2026-08-31T00:00:00Z".to_string(),
    };
    let child = engine
        .start_plan_repair(request)
        .await
        .expect("plan repair child session");

    // ---- 3) 计划缺陷暂停：AwaitingPlanAmendment + PlanAmendmentContext ----
    let reconciliation = store
        .reconcile_linked_plan_repair_pause(&attempt)
        .expect("reconcile plan repair pause");
    let paused = reconciliation.attempt;
    assert_eq!(paused.status, CodingAttemptStatus::AwaitingPlanAmendment);
    let context = store
        .find_plan_amendment_context_by_finding(&paused, TRIGGER_FINDING_ID)
        .expect("find context")
        .expect("plan amendment context must exist after reconcile");
    assert_eq!(context.plan_session_id, plan_session_id);
    assert_eq!(context.status, PlanAmendmentContextStatus::Open);
    assert_eq!(context.trigger_unit_id, units[1].0.id);
    assert_eq!(
        context.previous_plan_revision_id, active_revision.id,
        "context binds the real compiled revision"
    );
    (store, paused.id, child.id)
}

#[tokio::test]
async fn group_amendment_reachable_from_real_approval_chain() {
    let _serial = crate::product::workspace_engine::single_candidate_compile_test_lock().await;

    // ---- 1) 真批准链：裸 Confirm → close → compile → confirm_work_item_plan ----
    let (root, lifecycle, plan_id, mut engine) = real_approval_fixture(2).await;
    let (event_tx, _event_rx) = mpsc::channel(32);
    engine.event_tx = event_tx;
    let plan_session_id = engine.session().session_id.clone();
    let close = engine
        .handle_human_gate_termination(
            crate::web::workspace_ws_types::HumanConfirmDecision::Confirm,
        )
        .await
        .expect("real approval chain must confirm");
    assert_eq!(close, HumanGateCloseOutcome::Confirmed);
    let approved = lifecycle
        .get_workspace_session(&plan_session_id)
        .expect("approved session");
    assert_eq!(approved.status, WorkspaceSessionStatus::Confirmed);
    assert_eq!(
        approved.single_candidate_phase,
        Some(SingleCandidatePhase::Completed)
    );
    let approved_snapshot = approved
        .human_gate_snapshot
        .clone()
        .expect("real approval must retain the gate snapshot (D11/D16)");
    assert_eq!(approved_snapshot.manual_repairs_remaining, 2);

    let (store, paused_attempt_id, child_session_id) =
        seed_awaiting_plan_amendment(&root, &lifecycle, &mut engine, &plan_id, &plan_session_id)
            .await;
    let project_id = engine.session().project_id.clone();
    let issue_id = engine.session().issue_id.clone();

    // ---- 4) typed feedback → probe 放行 → CAS 重开 → 预算从原快照扣一次 ----
    let record = lifecycle
        .get_workspace_session(&plan_session_id)
        .expect("plan session record");
    let (feedback_tx, _feedback_rx) = mpsc::channel(8);
    let mut session = WorkspaceSession::from_record(record);
    session.artifact = Some(ArtifactPayload::Markdown {
        markdown: "# Work Item Plan\n\n## Work Item WI-001: candidate\n".to_string(),
        diff: None,
    });
    let mut feedback_engine = WorkspaceEngine::new_persistent(
        Arc::new(CheckpointStore::new(
            root.path().join("checkpoints-feedback"),
        )),
        lifecycle.clone(),
        feedback_tx,
        session,
    );
    let opened = feedback_engine
        .handle_human_gate_feedback(HumanGateFeedbackInput {
            command_id: "cmd_real_chain_amendment_feedback".to_string(),
            feedback: "只修正 WI-001 的 Outputs，其余逐字保留".to_string(),
        })
        .await
        .expect("amendment feedback must reopen the original plan session gate");
    let (turn, remaining_budget) = match opened {
        HumanGateCommandOutcome::TurnOpened {
            turn,
            remaining_budget,
            ..
        } => (turn, remaining_budget),
        other => panic!("expected amendment turn opened, got {other:?}"),
    };
    assert_eq!(turn.session_id, plan_session_id);
    assert_eq!(remaining_budget, 1);

    let durable = lifecycle
        .get_workspace_session(&plan_session_id)
        .expect("reopened session");
    assert_eq!(durable.status, WorkspaceSessionStatus::WaitingForHuman);
    let snapshot = durable.human_gate_snapshot.as_ref().expect("gate snapshot");
    assert_eq!(
        snapshot.manual_repairs_remaining, 1,
        "budget decremented exactly once from the real approval snapshot"
    );
    assert_eq!(snapshot.attempts_used, approved_snapshot.attempts_used);
    assert_eq!(snapshot.findings, approved_snapshot.findings);
    let turns = lifecycle
        .list_human_gate_turns(&plan_session_id)
        .expect("turns");
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].turn_id, turn.turn_id);

    // group attempt 侧不产生第二预算账；child repair session 是独立会话。
    let attempt_after = store
        .get_attempt(&project_id, &issue_id, &paused_attempt_id)
        .expect("attempt after feedback");
    assert_eq!(
        attempt_after.status,
        CodingAttemptStatus::AwaitingPlanAmendment
    );
    assert_ne!(child_session_id, plan_session_id);
}

/// I-1 回归链共用前缀：真批准链（budget 进入 retained 快照）→ 注入计划缺陷暂停
/// → 以 `review_rounds` 配置构造 amendment feedback engine → typed feedback 重开
/// 原门并预留一个 turn（预算 budget→budget-1，turn 置 Running）。
/// review_rounds=0 走无 reviewer 的本地 synthetic Pass Evaluate 路由；
/// review_rounds>=1 走重启评审分支（reviewer=ClaudeCode）。
/// 额外返回 (coding store, 暂停的 group attempt id) 供轮 3 用例操纵
/// PlanAmendmentContext / attempt 状态（损坏、应用窗口模拟）。
#[allow(clippy::type_complexity)]
async fn amendment_revision_chain_prefix(
    tag: &str,
    budget: u32,
    review_rounds: u32,
) -> (
    tempfile::TempDir,
    LifecycleStore,
    WorkspaceEngine,
    String, /* plan_session_id */
    String, /* running turn_id */
    CodingAttemptStore,
    String, /* paused group attempt id */
) {
    let (root, lifecycle, plan_id, mut approval_engine) = real_approval_fixture(budget).await;
    let (event_tx, _event_rx) = mpsc::channel(32);
    approval_engine.event_tx = event_tx;
    let plan_session_id = approval_engine.session().session_id.clone();
    let close = approval_engine
        .handle_human_gate_termination(
            crate::web::workspace_ws_types::HumanConfirmDecision::Confirm,
        )
        .await
        .expect("real approval chain must confirm");
    assert_eq!(close, HumanGateCloseOutcome::Confirmed);
    let (store, paused_attempt_id, _child_session_id) = seed_awaiting_plan_amendment(
        &root,
        &lifecycle,
        &mut approval_engine,
        &plan_id,
        &plan_session_id,
    )
    .await;

    let mut record = lifecycle
        .get_workspace_session(&plan_session_id)
        .expect("plan session record");
    record.reviewer_provider = ProviderName::ClaudeCode;
    record.review_rounds = review_rounds;
    crate::product::json_store::write_json(
        &lifecycle
            .app_paths()
            .issue_root(&record.project_id, &record.issue_id)
            .join("workspace-sessions")
            .join(format!("{}.json", record.id)),
        &record,
    )
    .expect("persist reviewer config");
    let (feedback_tx, _feedback_rx) = mpsc::channel(8);
    let mut session = WorkspaceSession::from_record(record);
    session.artifact = Some(ArtifactPayload::Markdown {
        markdown: real_chain_candidate_markdown().to_string(),
        diff: None,
    });
    let mut engine = WorkspaceEngine::new_persistent(
        Arc::new(CheckpointStore::new(
            root.path().join(format!("{tag}-checkpoints")),
        )),
        lifecycle.clone(),
        feedback_tx,
        session,
    );
    let opened = engine
        .handle_human_gate_feedback(HumanGateFeedbackInput {
            command_id: format!("cmd_{tag}_amendment_feedback"),
            feedback: "只修正 WI-001 的标题，其余逐字保留".to_string(),
        })
        .await
        .expect("amendment feedback must reopen the gate");
    let (turn, remaining_budget) = match opened {
        HumanGateCommandOutcome::TurnOpened {
            turn,
            remaining_budget,
            ..
        } => (turn, remaining_budget),
        other => panic!("expected amendment turn opened, got {other:?}"),
    };
    assert_eq!(remaining_budget, budget - 1, "预留后预算恰减一次");
    engine
        .mark_human_gate_turn_running(&turn.turn_id)
        .expect("mark running");
    (
        root,
        lifecycle,
        engine,
        plan_session_id,
        turn.turn_id,
        store,
        paused_attempt_id,
    )
}

fn real_chain_candidate_markdown() -> &'static str {
    "# Work Item Plan\n\n## Work Item WI-001: candidate\n"
}

/// I-1 回归（无 reviewer 路径）：amendment typed feedback 修订成功后，Evaluate
/// policy route 重建 Approval 快照时预算 MUST 接续原 human_gate_snapshot 的
/// manual_repairs_remaining——2→1 之后经重建仍为 1，不得回退到重建重置语义
/// （默认 3 − run_history 计数）。普通 SC 修订门的重建重置语义由
/// campaign_stage3_interactive 既有用例锚定，本用例只锁 amendment 分叉。
#[tokio::test]
async fn amendment_revision_rebuild_without_reviewer_continues_gate_budget() {
    let _serial = crate::product::workspace_engine::single_candidate_compile_test_lock().await;
    let (_root, lifecycle, mut engine, plan_session_id, turn_id, _store, _attempt_id) =
        amendment_revision_chain_prefix("amendment_rebuild_no_reviewer", 2, 0).await;

    let result = engine
        .run_sc_manual_revision_turn(
            &turn_id,
            super::conversational_gate_revision::handoff_clean_rep4_v2(),
        )
        .await
        .expect("amendment revision must complete");
    assert!(matches!(
        result,
        crate::product::workspace_engine::ScManualRevisionResult::Accepted { .. }
    ));
    let turn = lifecycle
        .get_human_gate_turn(&plan_session_id, &turn_id)
        .expect("durable turn");
    assert_eq!(
        turn.status,
        crate::product::models::HumanGateTurnStatus::Completed
    );
    let routed = lifecycle
        .get_workspace_session(&plan_session_id)
        .expect("routed session");
    assert_eq!(routed.status, WorkspaceSessionStatus::WaitingForHuman);
    assert_eq!(
        routed.single_candidate_phase,
        Some(SingleCandidatePhase::Approval),
        "amendment 修订成功后必须经 Evaluate 重建 Approval 门"
    );
    let snapshot = routed.human_gate_snapshot.as_ref().expect("gate snapshot");
    assert_eq!(
        snapshot.manual_repairs_remaining, 1,
        "I-1: amendment 门预算接续——2→1 后经重建仍为 1，不得重置回默认 3"
    );
    assert_eq!(engine.session().stage, WorkspaceStage::HumanConfirm);
}

/// I-1 回归（有 reviewer 路径）：amendment 修订成功后重启评审，reviewer Pass 经
/// 同一 Approval snapshot builder 重建门快照时，预算同样 MUST 接续（2→1 后仍为
/// 1，不回 3）。重开中的 amendment 门（phase Completed + WaitingForHuman）不得
/// 被终态评审守卫当作已完结会话丢弃 verdict。
#[tokio::test]
async fn amendment_revision_reviewer_pass_rebuild_continues_gate_budget() {
    let _serial = crate::product::workspace_engine::single_candidate_compile_test_lock().await;
    let (_root, lifecycle, mut engine, plan_session_id, turn_id, _store, _attempt_id) =
        amendment_revision_chain_prefix("amendment_rebuild_reviewer", 2, 1).await;

    let result = engine
        .run_sc_manual_revision_turn(
            &turn_id,
            super::conversational_gate_revision::handoff_clean_rep4_v2(),
        )
        .await
        .expect("amendment revision must complete");
    assert!(matches!(
        result,
        crate::product::workspace_engine::ScManualRevisionResult::Accepted { .. }
    ));
    assert_eq!(
        engine.session().stage,
        WorkspaceStage::CrossReview,
        "有 reviewer 时 amendment 修订后必须重启评审，不得直接跳 Approval"
    );

    let verdict = crate::web::workspace_ws_types::ReviewVerdict {
        verdict: crate::web::workspace_ws_types::ReviewVerdictType::Pass,
        comments: "review pass".to_string(),
        summary: "review pass".to_string(),
        findings: Vec::new(),
        review_gate: crate::web::workspace_ws_types::ReviewGate::UserConfirmAllowed,
        work_item_plan_review: None,
        structured_output_diagnostic: None,
    };
    engine
        .complete_review(
            crate::cross_cutting::streaming_provider::ProviderCompletion::plain(
                "review".to_string(),
                None,
            ),
            verdict,
        )
        .await;

    let routed = lifecycle
        .get_workspace_session(&plan_session_id)
        .expect("routed session");
    assert_eq!(routed.status, WorkspaceSessionStatus::WaitingForHuman);
    assert_eq!(
        routed.single_candidate_phase,
        Some(SingleCandidatePhase::Approval),
        "reviewer Pass 后必须路由回 Approval 门"
    );
    let snapshot = routed.human_gate_snapshot.as_ref().expect("gate snapshot");
    assert_eq!(
        snapshot.manual_repairs_remaining, 1,
        "I-1: reviewer 路径 amendment 门预算接续——2→1 后经重建仍为 1，不得重置回默认 3"
    );
    assert_eq!(engine.session().stage, WorkspaceStage::HumanConfirm);
}

/// I-1 round2 伪造场景共用前缀：真批准链（budget=2，即 Confirmed + phase
/// Completed，且保留快照 remaining=2）之后**不注入任何 amendment 上下文**，
/// 直接经通用 `update_workspace_session_status` 把会话写成 WaitingForHuman——
/// 复现「重开三元组可伪造」攻击面（该通用写入与 `enter_human_confirm`、
/// `provider_workspace_runner` 同源，均不校验 phase、不清快照）。返回按
/// `review_rounds` 配置好的 amendment 判别目标 engine。
async fn forged_reopen_signature_prefix(
    tag: &str,
    review_rounds: u32,
) -> (
    tempfile::TempDir,
    LifecycleStore,
    WorkspaceEngine,
    String, /* plan_session_id */
) {
    let (root, lifecycle, _plan_id, mut approval_engine) = real_approval_fixture(2).await;
    let (event_tx, _event_rx) = mpsc::channel(32);
    approval_engine.event_tx = event_tx;
    let plan_session_id = approval_engine.session().session_id.clone();
    let close = approval_engine
        .handle_human_gate_termination(
            crate::web::workspace_ws_types::HumanConfirmDecision::Confirm,
        )
        .await
        .expect("real approval chain must confirm");
    assert_eq!(close, HumanGateCloseOutcome::Confirmed);
    let mut record = lifecycle
        .get_workspace_session(&plan_session_id)
        .expect("approved record");
    assert_eq!(record.status, WorkspaceSessionStatus::Confirmed);
    assert_eq!(
        record.single_candidate_phase,
        Some(SingleCandidatePhase::Completed)
    );
    assert_eq!(
        record
            .human_gate_snapshot
            .as_ref()
            .expect("retained snapshot")
            .manual_repairs_remaining,
        2
    );
    // 伪造：通用状态写入 WaitingForHuman，不经过 compare_and_reopen_amendment_gate，
    // 也不存在任何 PlanAmendmentContext。
    lifecycle
        .update_workspace_session_status(&plan_session_id, WorkspaceSessionStatus::WaitingForHuman)
        .expect("forge waiting_for_human via the generic status write");
    record = lifecycle
        .get_workspace_session(&plan_session_id)
        .expect("forged record");
    record.reviewer_provider = ProviderName::ClaudeCode;
    record.review_rounds = review_rounds;
    crate::product::json_store::write_json(
        &lifecycle
            .app_paths()
            .issue_root(&record.project_id, &record.issue_id)
            .join("workspace-sessions")
            .join(format!("{}.json", record.id)),
        &record,
    )
    .expect("persist reviewer config");
    let (feedback_tx, _feedback_rx) = mpsc::channel(8);
    let mut session = WorkspaceSession::from_record(record);
    session.artifact = Some(ArtifactPayload::Markdown {
        markdown: real_chain_candidate_markdown().to_string(),
        diff: None,
    });
    let engine = WorkspaceEngine::new_persistent(
        Arc::new(CheckpointStore::new(
            root.path().join(format!("{tag}-checkpoints")),
        )),
        lifecycle.clone(),
        feedback_tx,
        session,
    );
    (root, lifecycle, engine, plan_session_id)
}

/// I-1 round2 回归（伪造场景·终态守卫）：普通 SC 会话被通用状态写入伪造成
/// Completed+WaitingForHuman 三元组（无任何 amendment context）时，迟到的评审
/// verdict 仍 MUST 被终态守卫丢弃——不得因三元组巧合命中 amendment 判别而绕过
/// 守卫处理 verdict 并重路由。
#[tokio::test]
async fn forged_reopen_signature_without_amendment_context_keeps_terminal_guard() {
    let _serial = crate::product::workspace_engine::single_candidate_compile_test_lock().await;
    let (_root, lifecycle, mut engine, plan_session_id) =
        forged_reopen_signature_prefix("forged_guard", 1).await;

    engine
        .complete_review(
            crate::cross_cutting::streaming_provider::ProviderCompletion::plain(
                "review".to_string(),
                None,
            ),
            crate::web::workspace_ws_types::ReviewVerdict {
                verdict: crate::web::workspace_ws_types::ReviewVerdictType::Pass,
                comments: "review pass".to_string(),
                summary: "review pass".to_string(),
                findings: Vec::new(),
                review_gate: crate::web::workspace_ws_types::ReviewGate::UserConfirmAllowed,
                work_item_plan_review: None,
                structured_output_diagnostic: None,
            },
        )
        .await;

    assert!(
        engine.latest_review_verdict.is_none(),
        "伪造三元组不得绕过终态守卫——迟到 verdict 必须被原样丢弃"
    );
    let routed = lifecycle
        .get_workspace_session(&plan_session_id)
        .expect("durable session");
    assert_eq!(
        routed.single_candidate_phase,
        Some(SingleCandidatePhase::Completed),
        "伪造场景不得重路由（不得进入 Approval 重建）"
    );
    assert_eq!(routed.status, WorkspaceSessionStatus::WaitingForHuman);
    let snapshot = routed
        .human_gate_snapshot
        .as_ref()
        .expect("generic write retains the snapshot");
    assert_eq!(
        snapshot.manual_repairs_remaining, 2,
        "伪造场景不得经 amendment 继承分支改写旧快照"
    );
}

/// I-1 round2 回归（伪造场景·不继承）：伪造三元组（无 amendment context）命中
/// Evaluate 重建时，快照预算 MUST 走普通门重置公式（默认 3 − run_history 计数），
/// 不得继承被伪造会话遗留的旧快照预算（remaining=2）。
#[tokio::test]
async fn forged_reopen_signature_rebuild_uses_reset_formula() {
    let _serial = crate::product::workspace_engine::single_candidate_compile_test_lock().await;
    let (_root, lifecycle, mut engine, plan_session_id) =
        forged_reopen_signature_prefix("forged_reset", 0).await;

    engine
        .route_single_candidate_evaluate_without_reviewer()
        .await;

    let routed = lifecycle
        .get_workspace_session(&plan_session_id)
        .expect("routed session");
    assert_eq!(
        routed.single_candidate_phase,
        Some(SingleCandidatePhase::Approval),
        "伪造场景仍按普通门语义路由进 Approval"
    );
    let snapshot = routed.human_gate_snapshot.as_ref().expect("gate snapshot");
    assert_eq!(
        snapshot.manual_repairs_remaining, 3,
        "伪造场景必须走普通门重置公式（默认 3 − 计数 = 3），不得继承旧快照的 2"
    );
}

/// I-1 round2 回归（fail-closed）：amendment 判别命中（durable 重开签名 + Open
/// PlanAmendmentContext 在场）但原快照缺失时，Approval 重建 MUST fail-closed
/// 报错（AbortFatal{PersistenceFailure}），不得回退普通重置公式重建。
#[tokio::test]
async fn amendment_reopen_signature_missing_snapshot_fails_closed() {
    let _serial = crate::product::workspace_engine::single_candidate_compile_test_lock().await;
    let (_root, lifecycle, mut engine, plan_session_id, _turn_id, _store, _attempt_id) =
        amendment_revision_chain_prefix("amendment_missing_snapshot", 2, 0).await;

    // 破坏性剪除 durable 快照（模拟损坏/外部删改）：判别所需的 durable 事实仍在
    // （WaitingForHuman + phase Completed + Open context），但快照缺席。
    let mut stripped = lifecycle
        .get_workspace_session(&plan_session_id)
        .expect("reopened record");
    stripped.human_gate_snapshot = None;
    crate::product::json_store::write_json(
        &lifecycle
            .app_paths()
            .issue_root(&stripped.project_id, &stripped.issue_id)
            .join("workspace-sessions")
            .join(format!("{}.json", stripped.id)),
        &stripped,
    )
    .expect("strip durable snapshot");

    engine
        .route_single_candidate_evaluate_without_reviewer()
        .await;

    let routed = lifecycle
        .get_workspace_session(&plan_session_id)
        .expect("durable session");
    assert_eq!(
        routed.single_candidate_phase,
        Some(SingleCandidatePhase::Completed),
        "fail-closed：不得以重置公式写回 Approval"
    );
    assert!(
        routed.human_gate_snapshot.is_none(),
        "fail-closed：不得落任何重建快照"
    );
    assert_eq!(
        engine.session().single_candidate_phase,
        Some(SingleCandidatePhase::Failed),
        "fail-closed： AbortFatal 后会话必须进 Failed"
    );
    assert!(
        engine
            .session()
            .policy_diagnostics
            .iter()
            .any(|diagnostic| diagnostic
                .message
                .contains("human_gate_amendment_snapshot_missing")),
        "fail-closed 必须以诊断显式报错，不得静默回退"
    );
    // I-1 round3（F-C）：fail-closed 诊断必须写回 durable record（重读可见），
    // 不得仅存内存——进程重启后诊断无从复原。
    assert!(
        routed.policy_diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("human_gate_amendment_snapshot_missing")),
        "F-C: fail-closed 诊断必须 durable 可见（重读 record），不得仅存内存"
    );
}

/// I-1 round3 F-A 辅助：把 durable PlanAmendmentContext 文件写坏（非法 JSON），
/// 模拟复审指出的「context 文件损坏/目录读失败/coding store 瞬时 I/O 错误」——
/// 判别所需的 durable 事实无法读取/校验。
fn corrupt_open_plan_amendment_context(
    store: &CodingAttemptStore,
    project_id: &str,
    issue_id: &str,
    attempt_id: &str,
) {
    let attempt = store
        .get_attempt(project_id, issue_id, attempt_id)
        .expect("paused attempt");
    let context = store
        .find_plan_amendment_context_by_finding(&attempt, TRIGGER_FINDING_ID)
        .expect("list contexts")
        .expect("open context present");
    let context_path = store
        .plan_amendment_contexts_root(project_id, issue_id, attempt_id)
        .join(format!("{}.json", context.id));
    std::fs::write(&context_path, "{ corrupted context payload (not json)")
        .expect("corrupt the durable plan amendment context");
}

/// I-1 round3 F-B 辅助：按 `apply_plan_amendment_from_journal` 的落盘顺序
/// （amendment.rs：先 context Open→Applying，后 attempt→ApplyingPlanAmendment）
/// 把真 amendment 链推进到应用窗口的稳态：context=Applying 且 attempt 已离开
/// AwaitingPlanAmendment——probe 在此窗口不放行，判别也不得按 amendment 门接续。
fn advance_amendment_into_applying_window(
    store: &CodingAttemptStore,
    project_id: &str,
    issue_id: &str,
    attempt_id: &str,
) {
    let attempt = store
        .get_attempt(project_id, issue_id, attempt_id)
        .expect("paused attempt");
    let context = store
        .find_plan_amendment_context_by_finding(&attempt, TRIGGER_FINDING_ID)
        .expect("list contexts")
        .expect("open context present");
    store
        .transition_plan_amendment_context_to_applying(&attempt, &context.id)
        .expect("context Open→Applying (apply 链顺序第 1 步)");
    store
        .update_attempt_status(
            project_id,
            issue_id,
            attempt_id,
            CodingAttemptStatus::ApplyingPlanAmendment,
        )
        .expect("attempt → ApplyingPlanAmendment (apply 链顺序第 2 步)");
}

fn late_pass_review_verdict() -> crate::web::workspace_ws_types::ReviewVerdict {
    crate::web::workspace_ws_types::ReviewVerdict {
        verdict: crate::web::workspace_ws_types::ReviewVerdictType::Pass,
        comments: "review pass".to_string(),
        summary: "review pass".to_string(),
        findings: Vec::new(),
        review_gate: crate::web::workspace_ws_types::ReviewGate::UserConfirmAllowed,
        work_item_plan_review: None,
        structured_output_diagnostic: None,
    }
}

/// I-1 round3 回归（F-A·重建路径）：真 amendment 门重开在场的会话，其判别所需
/// durable 事实（PlanAmendmentContext）无法读取/校验（文件损坏/权限/瞬时 I/O）
/// 时，Evaluate 重建 MUST 以持久化失败 fail-closed 终止（AbortFatal
/// {PersistenceFailure}），不得把「无法读取」当作「无 amendment 事实」走进普通
/// 门重置公式——那会凭空恢复已耗预算。轮 2 代码下该路径走重置重建（本用例红）。
#[tokio::test]
async fn amendment_context_read_failure_fails_closed_on_rebuild() {
    let _serial = crate::product::workspace_engine::single_candidate_compile_test_lock().await;
    let (_root, lifecycle, mut engine, plan_session_id, _turn_id, store, attempt_id) =
        amendment_revision_chain_prefix("amendment_ctx_read_fail_rebuild", 2, 0).await;
    let project_id = engine.session().project_id.clone();
    let issue_id = engine.session().issue_id.clone();
    corrupt_open_plan_amendment_context(&store, &project_id, &issue_id, &attempt_id);

    engine
        .route_single_candidate_evaluate_without_reviewer()
        .await;

    let routed = lifecycle
        .get_workspace_session(&plan_session_id)
        .expect("durable session");
    assert_eq!(
        routed.status,
        WorkspaceSessionStatus::Failed,
        "F-A: 判别读失败必须 fail-closed 终止，不得静默重置重建"
    );
    assert_eq!(
        routed.single_candidate_phase,
        Some(SingleCandidatePhase::Completed),
        "F-A: 不得以重置公式写回 Approval（不得把无法校验当「无 amendment 事实」）"
    );
    assert!(
        routed.human_gate_snapshot.is_none(),
        "F-A: fail-closed 不得落任何重建快照"
    );
    assert_eq!(
        engine.session().single_candidate_phase,
        Some(SingleCandidatePhase::Failed),
        "F-A: AbortFatal 后内存会话必须进 Failed"
    );
    assert!(
        engine
            .session()
            .policy_diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "persistence_failure"),
        "F-A: 判别读失败必须以 persistence_failure 诊断显式报错"
    );
    // I-1 round3（F-C）：fail-closed 诊断必须写回 durable record（重读可见）。
    assert!(
        routed
            .policy_diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "persistence_failure"),
        "F-C: fail-closed 诊断必须 durable 可见（重读 record），不得仅存内存"
    );
}

/// I-1 round3 回归（F-A·守卫路径）：重开中 amendment 门会话的判别 durable 事实
/// 无法读取/校验时，迟到的 review verdict 也不得被静默丢弃后当作可继续会话，
/// 更不得放行进重置公式——无法校验 = 持久化失败，MUST 显式 fail-closed 终止
/// （status=Failed + durable 诊断）。轮 2 代码下该路径静默丢弃、会话保持
/// WaitingForHuman（本用例红）。
#[tokio::test]
async fn amendment_context_read_failure_fails_closed_on_late_verdict() {
    let _serial = crate::product::workspace_engine::single_candidate_compile_test_lock().await;
    let (_root, lifecycle, mut engine, plan_session_id, _turn_id, store, attempt_id) =
        amendment_revision_chain_prefix("amendment_ctx_read_fail_verdict", 2, 1).await;
    let project_id = engine.session().project_id.clone();
    let issue_id = engine.session().issue_id.clone();
    corrupt_open_plan_amendment_context(&store, &project_id, &issue_id, &attempt_id);

    engine
        .complete_review(
            crate::cross_cutting::streaming_provider::ProviderCompletion::plain(
                "review".to_string(),
                None,
            ),
            late_pass_review_verdict(),
        )
        .await;

    assert!(
        engine.latest_review_verdict.is_none(),
        "F-A: 无法读取/校验判别事实时不得放行迟到 verdict 进路由"
    );
    let routed = lifecycle
        .get_workspace_session(&plan_session_id)
        .expect("durable session");
    assert_eq!(
        routed.status,
        WorkspaceSessionStatus::Failed,
        "F-A: 无法校验 = 持久化失败，必须显式 fail-closed 终止，不得静默丢弃保持 WaitingForHuman"
    );
    assert_eq!(
        routed.single_candidate_phase,
        Some(SingleCandidatePhase::Completed),
        "F-A: 守卫侧 fail-closed 不得写回 Approval"
    );
    assert!(
        routed.human_gate_snapshot.is_none(),
        "F-A: fail-closed 不得落任何重建快照"
    );
    assert_eq!(
        engine.session().single_candidate_phase,
        Some(SingleCandidatePhase::Failed),
        "F-A: 守卫侧 fail-closed 后内存会话必须进 Failed"
    );
    assert!(
        routed
            .policy_diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "persistence_failure"
                && diagnostic.field.as_deref() == Some("plan_amendment_context")),
        "F-C: 守卫侧 fail-closed 诊断必须 durable 可见且可定位到判别读失败"
    );
}

/// I-1 round3 回归（F-B·对称性）：判别必须复用 probe 放行重开的同一完整谓词
/// （Open/Applying context **且** group attempt 处于 AwaitingPlanAmendment 并绑定
/// 本 plan session entity）。context 先行 Open→Applying 而 attempt 已离开
/// AwaitingPlanAmendment 的应用窗口内（amendment.rs 落盘顺序），probe 不放行，
/// 判别同样不得按 amendment 门：(a) 迟到 verdict 必须被终态守卫丢弃；
/// (b) Evaluate 重建必须走普通门重置公式，不得接续快照预算。轮 2 判别只看
/// context 状态，两条均被绕过（本用例红）。
#[tokio::test]
async fn amendment_applying_window_not_discriminated_as_amendment_gate() {
    let _serial = crate::product::workspace_engine::single_candidate_compile_test_lock().await;
    let (_root, lifecycle, mut engine, plan_session_id, _turn_id, store, attempt_id) =
        amendment_revision_chain_prefix("amendment_applying_window", 2, 1).await;
    let project_id = engine.session().project_id.clone();
    let issue_id = engine.session().issue_id.clone();
    advance_amendment_into_applying_window(&store, &project_id, &issue_id, &attempt_id);

    // (a) 迟到 verdict：应用窗口内不得按 amendment 门绕过终态守卫
    engine
        .complete_review(
            crate::cross_cutting::streaming_provider::ProviderCompletion::plain(
                "review".to_string(),
                None,
            ),
            late_pass_review_verdict(),
        )
        .await;
    assert!(
        engine.latest_review_verdict.is_none(),
        "F-B: 应用窗口（context=Applying 且 attempt 已离开 AwaitingPlanAmendment）判别不得按 amendment 门绕过终态守卫"
    );
    let after_verdict = lifecycle
        .get_workspace_session(&plan_session_id)
        .expect("durable session");
    assert_eq!(
        after_verdict.single_candidate_phase,
        Some(SingleCandidatePhase::Completed),
        "F-B: 迟到 verdict 被丢弃，不得重路由进 Approval"
    );
    assert_eq!(
        after_verdict.status,
        WorkspaceSessionStatus::WaitingForHuman
    );
    let snapshot = after_verdict
        .human_gate_snapshot
        .as_ref()
        .expect("snapshot retained");
    assert_eq!(
        snapshot.manual_repairs_remaining, 1,
        "F-B: 不得经接续分支改写快照"
    );

    // (b) Evaluate 重建：不得按 amendment 接续，必须走普通门重置公式
    engine
        .route_single_candidate_evaluate_without_reviewer()
        .await;
    let routed = lifecycle
        .get_workspace_session(&plan_session_id)
        .expect("routed session");
    assert_eq!(
        routed.single_candidate_phase,
        Some(SingleCandidatePhase::Approval),
        "F-B: 应用窗口按普通门语义路由进 Approval"
    );
    let rebuilt = routed.human_gate_snapshot.as_ref().expect("gate snapshot");
    assert_eq!(
        rebuilt.manual_repairs_remaining, 3,
        "F-B: 应用窗口不得按 amendment 接续（不得继承 1），按普通门重置公式 3"
    );
}
