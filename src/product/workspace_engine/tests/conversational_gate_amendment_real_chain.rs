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
fn real_approval_fixture(budget: u32) -> (TempDir, LifecycleStore, String, WorkspaceEngine) {
    let (tmp, lifecycle, plan_id, mut engine) =
        super::make_work_item_plan_engine_with_accepted_contract_drafts();
    super::single_candidate_recovery::single_candidate_recovery_record(
        &lifecycle,
        &mut engine,
        SingleCandidatePhase::Approval,
        RunPolicy::Interactive,
    );
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

#[tokio::test]
async fn group_amendment_reachable_from_real_approval_chain() {
    let _serial = crate::product::workspace_engine::single_candidate_compile_test_lock().await;

    // ---- 1) 真批准链：裸 Confirm → close → compile → confirm_work_item_plan ----
    let (root, lifecycle, plan_id, mut engine) = real_approval_fixture(2);
    let (event_tx, _event_rx) = mpsc::channel(32);
    engine.event_tx = event_tx;
    let plan_session_id = engine.session().session_id.clone();
    let project_id = engine.session().project_id.clone();
    let issue_id = engine.session().issue_id.clone();
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

    // ---- 2) 编码侧：真实编译产物上的 group attempt + 7.2 既有计划缺陷注入 ----
    let store = CodingAttemptStore::new(lifecycle.app_paths());
    let worktree = root.path().join("worktree");
    std::fs::create_dir_all(&worktree).unwrap();
    let initial = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: project_id.clone(),
            issue_id: issue_id.clone(),
            plan_id: plan_id.clone(),
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
        .get_plan_lineage(&project_id, &issue_id, &plan_id)
        .expect("real plan lineage");
    let active_revision = revision_store
        .get_plan_revision(
            &project_id,
            &issue_id,
            &plan_id,
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
                plan_id: plan_id.clone(),
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
        plan_id: plan_id.clone(),
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
        Arc::new(CheckpointStore::new(root.path().join("checkpoints-feedback"))),
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
        .get_attempt(&project_id, &issue_id, &paused.id)
        .expect("attempt after feedback");
    assert_eq!(attempt_after.status, CodingAttemptStatus::AwaitingPlanAmendment);
    assert_ne!(child.id, plan_session_id);
}
