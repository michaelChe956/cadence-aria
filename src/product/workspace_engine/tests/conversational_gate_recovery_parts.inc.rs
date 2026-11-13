// SG6b 结构恢复分域基座（从 conversational_gate_recovery.rs 整块搬移，逐字未改；
// 主模块已近行数上限，按职责搬移不压行）。
/// Reviewing/BlockedByLcRecovery 分域的通用基座：真实 reserve（物化 hold）
/// + Running turn + 注入指定 phase 的活动 grant + current refs 轮换到真实
///   落盘的「已交付 candidate」（合法中间/交付 hash ≠ 原授权 hash 的现场
///   形态）。返回 (turn_id, session_path, delivered_source_ref)。
fn sg06b_structural_recovery_fixture(
    lifecycle: &crate::product::lifecycle_store::LifecycleStore,
    engine: &mut crate::product::workspace_engine::WorkspaceEngine,
    plan_id: &str,
    phase: crate::product::models::workspace::StructuralGrantPhase,
    turn_label: &str,
    turn_source_hash: &str,
) -> (String, std::path::PathBuf, String) {
    use crate::product::json_store::write_json;
    use crate::product::models::workspace::{StructuralGovernanceState, StructuralRevisionGrant};
    use crate::product::models::{
        HumanGateReservation, HumanGateTurnStatus, SingleCandidatePhase, WorkspaceSessionStatus,
    };
    use crate::product::work_item_plan_policy::{HumanGateSnapshot, HumanReason};
    use crate::product::work_item_plan_source_store::{
        SourceRevisionRecord, WorkItemPlanSourceStore,
    };
    use sha2::{Digest, Sha256};

    let mut session = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("session");
    session.flow_kind = WorkItemPlanFlowKind::SingleCandidate;
    session.single_candidate_phase = Some(SingleCandidatePhase::Approval);
    session.status = WorkspaceSessionStatus::WaitingForHuman;
    session.human_gate_snapshot = Some(HumanGateSnapshot {
        candidate_recovery: None,
        findings: Vec::new(),
        repeated_fingerprints: Vec::new(),
        attempts_used: 0,
        manual_repairs_remaining: 3,
        trigger: HumanReason::NativeHumanRequired,
        resumable: true,
        accepted_feedback_turns: None,
    });
    session.work_item_plan_source_revision_ref = None;
    session.plan_candidate_ir_ref = None;
    session.mechanical_report_ref = None;
    session.approval_attempt_id = None;
    session.approved_at = None;
    session.compile_reservation = None;
    session.publication_provenance_ref = None;
    let session_path = lifecycle
        .app_paths()
        .issue_root(&session.project_id, &session.issue_id)
        .join("workspace-sessions")
        .join(format!("{}.json", session.id));
    write_json(&session_path, &session).expect("persist gate session");

    let now = "2026-10-14T00:00:00Z".to_string();
    let turn_id = format!("turn_sg6b_{turn_label}");
    let command_id = format!("cmd_sg6b_{turn_label}");
    let reserved_turn = HumanGateTurn {
        turn_id: turn_id.clone(),
        session_id: session.id.clone(),
        command_id: command_id.clone(),
        feedback_text: "SG6b 恢复分域原反馈".to_string(),
        status: HumanGateTurnStatus::Reserved,
        attempt_no: 1,
        budget_reserved: 1,
        source_hash: turn_source_hash.to_string(),
        result_artifact_ref: None,
        failure_class: None,
        created_at: now.clone(),
        updated_at: now.clone(),
    };
    let reservation = HumanGateReservation {
        command_id,
        turn_id: turn_id.clone(),
        provider_start_idempotency_key: format!("human_gate:{turn_id}:attempt:1"),
        reserved_at: now,
    };
    let (reserved_session, _) = lifecycle
        .compare_and_reserve_human_gate_turn(&session, reserved_turn, reservation)
        .expect("reserve durable turn");
    let running_turn = HumanGateTurn {
        status: HumanGateTurnStatus::Running,
        ..lifecycle
            .get_human_gate_turn(&session.id, &turn_id)
            .expect("reserved turn")
    };
    lifecycle
        .update_human_gate_turn(&reserved_session, running_turn)
        .expect("mark running");

    // 已交付 candidate：真实 source store 落盘 + refs 轮换（合法中间/
    // 交付 hash，允许不同于原授权 turn.source_hash）。
    let delivered_source = format!("# SG6b delivered candidate {turn_label}\n\n## 交付正文\n");
    let delivered_hash = hex::encode(Sha256::digest(delivered_source.as_bytes()));
    let mut source = SourceRevisionRecord {
        id: format!("source-sg6b-{turn_label}"),
        source: delivered_source,
        source_revision_hash: delivered_hash.clone(),
        content_hash: String::new(),
    };
    source.content_hash = source.content_hash().expect("source content hash");
    let source_store = WorkItemPlanSourceStore::new(lifecycle.app_paths());
    let delivered_ref = source_store
        .put_source_revision(&session.project_id, &session.issue_id, plan_id, &source)
        .expect("persist delivered source");

    let mut seeded = lifecycle
        .get_workspace_session(&session.id)
        .expect("reload for grant seed");
    seeded.work_item_plan_source_revision_ref = Some(delivered_ref);
    let state = seeded
        .structural_governance_state
        .get_or_insert_with(StructuralGovernanceState::default);
    state.current_grant = Some(StructuralRevisionGrant {
        grant_id: format!("structural_grant_{turn_id}"),
        authorization_hash: "f".repeat(64),
        turn_id: Some(turn_id.clone()),
        command_id: Some(format!("cmd_sg6b_{turn_label}")),
        candidate_source_revision_ref: None,
        plan_candidate_ir_ref: None,
        mechanical_report_ref: None,
        phase: Some(phase),
        attempt_no: 2,
        prompt_ref: Some(format!("structural-prompt://{turn_id}")),
        prompt_digest: Some("d".repeat(64)),
        instruction_digest: Some("b".repeat(64)),
        failing_source_hash: Some("a".repeat(64)),
        review_identity: None,
        granted_at: "2026-10-14T00:00:00Z".to_string(),
    });
    write_json(&session_path, &seeded).expect("persist structural phase fixture");
    (turn_id, session_path, delivered_hash)
}
