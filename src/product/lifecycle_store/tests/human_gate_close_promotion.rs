// SC 人工门「Evaluate 进门 × Approval 关门」死锁修复（oracle 裁决 A，3.6 矩阵族⑤）
// 的 store 层回归：`compare_and_save_human_gate_close` 前置改为三 conjunct——
// `WaitingForHuman ∧ phase∈{Approval,Evaluate} ∧ human_gate_snapshot 在场`；
// confirm（status=Running）在锁内判等通过后原子提升 phase→Approval（人工权威
// 升级），terminate 关门成功但不提升相位。快照在场是反伪造判据：
// `update_workspace_session_status` 只在终态清快照、从不创建快照，Evaluate 门
// 若没有快照在场就不是本 CAS 应当关闭的门形态。禁止 trigger/resumable 判据：
// NativeHumanRequired 跨两族复用、resumable 是策略派生值；Completed 必须继续拒
// （amendment 重开门不得流经 approval compile）。

use crate::product::models::{SingleCandidatePhase, WorkspaceSessionStatus};

/// 铺一个 SingleCandidate WorkItemPlan 的 durable 门形态记录并返回（写入后与
/// 传入 CAS 的 expected 判等成立）。三 refs（source/IR/mechanical report）一并
/// 种入，验证 close CAS 不动批准链输入。
fn gate_close_session(
    store: &LifecycleStore,
    phase: Option<SingleCandidatePhase>,
    status: WorkspaceSessionStatus,
    with_snapshot: bool,
) -> crate::product::models::WorkspaceSessionRecord {
    let mut record = create_session(store, "work_item_plan_0001", WorkspaceType::WorkItemPlan);
    record.flow_kind = crate::product::work_item_plan_policy::WorkItemPlanFlowKind::SingleCandidate;
    record.status = status;
    record.single_candidate_phase = phase;
    record.human_gate_snapshot = with_snapshot.then(|| HumanGateSnapshot {
        findings: Vec::new(),
        repeated_fingerprints: Vec::new(),
        attempts_used: 0,
        manual_repairs_remaining: 1,
        trigger: HumanReason::NativeHumanRequired,
        resumable: true,
    });
    record.work_item_plan_source_revision_ref = Some("source_revision_ref_0001".to_string());
    record.plan_candidate_ir_ref = Some("plan_candidate_ir_ref_0001".to_string());
    record.mechanical_report_ref = Some("mechanical_report_ref_0001".to_string());
    write_json(
        &store
            .app_paths()
            .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
            .join("workspace-sessions")
            .join(format!("{}.json", record.id)),
        &record,
    )
    .unwrap();
    record
}

fn assert_conflict_kind(
    result: Result<crate::product::models::WorkspaceSessionRecord, ProductStoreError>,
    kind: &'static str,
) {
    assert!(
        matches!(result, Err(ProductStoreError::Conflict { kind: actual, .. }) if actual == kind),
        "expected Conflict{{kind: {kind}}}, got {result:?}"
    );
}

/// ①核心修复：Evaluate 进门 + 快照在场 + confirm → Ok，锁内原子提升
/// phase→Approval、status→Running；批准链三 refs 与门快照原样保留。
#[test]
fn human_gate_close_confirm_at_evaluate_promotes_phase_atomically() {
    let (_tmp, store) = setup();
    let expected = gate_close_session(
        &store,
        Some(SingleCandidatePhase::Evaluate),
        WorkspaceSessionStatus::WaitingForHuman,
        true,
    );

    let saved = store
        .compare_and_save_human_gate_close(&expected, WorkspaceSessionStatus::Running)
        .expect("confirm at an Evaluate gate must close via human-authority promotion");
    assert_eq!(saved.status, WorkspaceSessionStatus::Running);
    assert_eq!(saved.single_candidate_phase, Some(SingleCandidatePhase::Approval));
    assert!(saved.human_gate_snapshot.is_some(), "Running 关门不清门快照");
    assert_eq!(
        saved.work_item_plan_source_revision_ref,
        Some("source_revision_ref_0001".to_string())
    );
    assert_eq!(
        saved.plan_candidate_ir_ref,
        Some("plan_candidate_ir_ref_0001".to_string())
    );
    assert_eq!(
        saved.mechanical_report_ref,
        Some("mechanical_report_ref_0001".to_string())
    );

    let durable = store.get_workspace_session(&expected.id).unwrap();
    assert_eq!(durable.status, WorkspaceSessionStatus::Running);
    assert_eq!(durable.single_candidate_phase, Some(SingleCandidatePhase::Approval));
    assert!(durable.human_gate_snapshot.is_some());
    assert_eq!(durable.work_item_plan_source_revision_ref, expected.work_item_plan_source_revision_ref);
    assert_eq!(durable.plan_candidate_ir_ref, expected.plan_candidate_ir_ref);
    assert_eq!(durable.mechanical_report_ref, expected.mechanical_report_ref);
}

/// ②反伪造：Evaluate 相位但快照缺席 → Conflict（不是本 CAS 应关的门形态）。
#[test]
fn human_gate_close_evaluate_without_snapshot_stays_conflict() {
    let (_tmp, store) = setup();
    let expected = gate_close_session(
        &store,
        Some(SingleCandidatePhase::Evaluate),
        WorkspaceSessionStatus::WaitingForHuman,
        false,
    );
    assert_conflict_kind(
        store.compare_and_save_human_gate_close(&expected, WorkspaceSessionStatus::Running),
        "human_gate_close",
    );
}

/// ③amendment 门拒：Completed 相位（快照在场）不得经 approval compile 关门。
#[test]
fn human_gate_close_completed_phase_stays_conflict() {
    let (_tmp, store) = setup();
    let expected = gate_close_session(
        &store,
        Some(SingleCandidatePhase::Completed),
        WorkspaceSessionStatus::WaitingForHuman,
        true,
    );
    assert_conflict_kind(
        store.compare_and_save_human_gate_close(&expected, WorkspaceSessionStatus::Running),
        "human_gate_close",
    );
}

/// ④非门相位：Prepare/Generate/Failed/None → Conflict。
#[test]
fn human_gate_close_rejects_non_gate_phases() {
    let (_tmp, store) = setup();
    for phase in [
        Some(SingleCandidatePhase::Prepare),
        Some(SingleCandidatePhase::Generate),
        Some(SingleCandidatePhase::Failed),
        None,
    ] {
        let expected = gate_close_session(&store, phase, WorkspaceSessionStatus::WaitingForHuman, true);
        assert_conflict_kind(
            store.compare_and_save_human_gate_close(&expected, WorkspaceSessionStatus::Running),
            "human_gate_close",
        );
    }
}

/// ⑤status≠WaitingForHuman → Conflict（不变）。
#[test]
fn human_gate_close_requires_waiting_for_human_status() {
    let (_tmp, store) = setup();
    for status in [
        WorkspaceSessionStatus::Running,
        WorkspaceSessionStatus::Confirmed,
        WorkspaceSessionStatus::Terminated,
    ] {
        let expected = gate_close_session(&store, Some(SingleCandidatePhase::Approval), status, true);
        assert_conflict_kind(
            store.compare_and_save_human_gate_close(&expected, WorkspaceSessionStatus::Running),
            "human_gate_close",
        );
    }
}

/// ⑥Evaluate 门 terminate → Ok：status=terminated、快照与 reservation 清空，
/// 但相位不提升（terminate 不是批准权威）。
#[test]
fn human_gate_close_terminate_at_evaluate_closes_without_phase_promotion() {
    let (_tmp, store) = setup();
    let mut expected = gate_close_session(
        &store,
        Some(SingleCandidatePhase::Evaluate),
        WorkspaceSessionStatus::WaitingForHuman,
        true,
    );
    expected.human_gate_reservation = Some(crate::product::models::HumanGateReservation {
        command_id: "command_0001".to_string(),
        turn_id: "turn_0001".to_string(),
        provider_start_idempotency_key: "human_gate_start_0001".to_string(),
        reserved_at: "2026-09-04T00:00:00Z".to_string(),
    });
    write_json(
        &store
            .app_paths()
            .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
            .join("workspace-sessions")
            .join(format!("{}.json", expected.id)),
        &expected,
    )
    .unwrap();

    let saved = store
        .compare_and_save_human_gate_close(&expected, WorkspaceSessionStatus::Terminated)
        .expect("terminate must close an Evaluate gate without promotion");
    assert_eq!(saved.status, WorkspaceSessionStatus::Terminated);
    assert_eq!(
        saved.single_candidate_phase,
        Some(SingleCandidatePhase::Evaluate),
        "terminate 不提升相位"
    );
    assert_eq!(saved.human_gate_snapshot, None);
    assert_eq!(saved.human_gate_reservation, None);

    let durable = store.get_workspace_session(&expected.id).unwrap();
    assert_eq!(durable.status, WorkspaceSessionStatus::Terminated);
    assert_eq!(durable.single_candidate_phase, Some(SingleCandidatePhase::Evaluate));
    assert_eq!(durable.human_gate_snapshot, None);
    assert_eq!(durable.human_gate_reservation, None);
}

/// ⑦陈旧 expected → Conflict{kind:"workspace_session"}（判等语义不变）。
#[test]
fn human_gate_close_stale_expected_record_keeps_workspace_session_conflict() {
    let (_tmp, store) = setup();
    let expected = gate_close_session(
        &store,
        Some(SingleCandidatePhase::Approval),
        WorkspaceSessionStatus::WaitingForHuman,
        true,
    );
    // 模拟另一个 worker 已推进 durable（expected 陈旧）。
    let mut drifted = expected.clone();
    drifted.provider_start_ledger.push(
        crate::product::work_item_plan_policy::ProviderStartLedgerEntry {
            provider_start_idempotency_key: "stale_start_0001".to_string(),
            started: true,
        },
    );
    write_json(
        &store
            .app_paths()
            .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
            .join("workspace-sessions")
            .join(format!("{}.json", drifted.id)),
        &drifted,
    )
    .unwrap();
    assert_conflict_kind(
        store.compare_and_save_human_gate_close(&expected, WorkspaceSessionStatus::Running),
        "workspace_session",
    );
}
