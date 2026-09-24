// SC 人工门「Evaluate 进门 × Approval 关门」死锁修复（oracle 裁决 A，3.6 矩阵族⑤）
// 的 store 层回归：`compare_and_save_human_gate_close` 前置改为三 conjunct——
// `WaitingForHuman ∧ phase∈{Approval,Evaluate} ∧ human_gate_snapshot 在场`；
// confirm（status=Running）在锁内判等通过后原子提升 phase→Approval（人工权威
// 升级），terminate 关门成功但不提升相位。快照在场是反伪造判据：
// `update_workspace_session_status` 只在终态清快照、从不创建快照，Evaluate 门
// 若没有快照在场就不是本 CAS 应当关闭的门形态。禁止 trigger/resumable 判据：
// NativeHumanRequired 跨两族复用、resumable 是策略派生值；Completed 必须继续拒
//（amendment 重开门不得流经 approval compile）。
// F-54 fix round：terminate 放行面扩展到 Failed 相位（用户脱困权），confirm
// 授权面 {Approval, Evaluate} 一字不动（见 ⑩⑪）。

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
    assert_eq!(
        saved.single_candidate_phase,
        Some(SingleCandidatePhase::Approval)
    );
    assert!(
        saved.human_gate_snapshot.is_some(),
        "Running 关门不清门快照"
    );
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
    assert_eq!(
        durable.single_candidate_phase,
        Some(SingleCandidatePhase::Approval)
    );
    assert!(durable.human_gate_snapshot.is_some());
    assert_eq!(
        durable.work_item_plan_source_revision_ref,
        expected.work_item_plan_source_revision_ref
    );
    assert_eq!(
        durable.plan_candidate_ir_ref,
        expected.plan_candidate_ir_ref
    );
    assert_eq!(
        durable.mechanical_report_ref,
        expected.mechanical_report_ref
    );
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
        let expected =
            gate_close_session(&store, phase, WorkspaceSessionStatus::WaitingForHuman, true);
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
        let expected =
            gate_close_session(&store, Some(SingleCandidatePhase::Approval), status, true);
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
    assert_eq!(
        durable.single_candidate_phase,
        Some(SingleCandidatePhase::Evaluate)
    );
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
            provider: None,
            started_at: None,
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

// F-21 fix round（controller 裁定）：plan 会话（SC 流）除终审门外还会停在
// human_confirm——context blocker 门（prepare 相位，plan_outline/authoring.rs
// enter_work_item_plan_context_blocker）与 author 连续 validate 失败门
//（generate 相位，decisions.rs enter_human_confirm_for_work_item_plan_author_
// failure）。这两个入口只置 stage+WaitingForHuman，不写快照不提相位；矩阵
// HumanConfirm SC 臂放行 AbandonHumanGate，引擎 close_human_gate 已在内存校验
// stage==HumanConfirm——CAS 对 terminate（Terminated）放行这三形态（Prepare/
// Generate/None），confirm（Running）前置一字不动（反伪造与 Completed 拒收
// 维持）。

// F-54 fix round：Failed 相位加入 terminate 放行面——compile 失败残留相位 ×
// 门重开 WaitingForHuman（0009 形态）此前 abandon 也被拒＝全通路死锁。
// fail-closed 只锁 confirm，永不锁 terminate（用户脱困权）。
/// ⑧F-21：terminate 放行 context blocker/author 失败/缺相位门（无快照）。
#[test]
fn human_gate_close_terminate_relaxes_non_approval_gate_shapes() {
    let (_tmp, store) = setup();
    for phase in [
        Some(SingleCandidatePhase::Prepare),
        Some(SingleCandidatePhase::Generate),
        None,
    ] {
        let expected = gate_close_session(
            &store,
            phase.clone(),
            WorkspaceSessionStatus::WaitingForHuman,
            false,
        );
        let saved = store
            .compare_and_save_human_gate_close(&expected, WorkspaceSessionStatus::Terminated)
            .unwrap_or_else(|error| panic!("phase {phase:?} terminate must close: {error:?}"));
        assert_eq!(saved.status, WorkspaceSessionStatus::Terminated);
        assert_eq!(saved.single_candidate_phase, phase, "terminate 不改相位");
        assert_eq!(saved.human_gate_snapshot, None);

        let durable = store.get_workspace_session(&expected.id).unwrap();
        assert_eq!(durable.status, WorkspaceSessionStatus::Terminated);
    }
}

/// ⑨F-21 边界：confirm（Running）对这些形态维持 Conflict——approve 仍必须是
/// Evaluate/Approval+快照在场的批准链门（反伪造判据不动）。
#[test]
fn human_gate_close_confirm_keeps_rejecting_non_approval_gate_shapes() {
    let (_tmp, store) = setup();
    for phase in [
        Some(SingleCandidatePhase::Prepare),
        Some(SingleCandidatePhase::Generate),
        None,
    ] {
        let expected =
            gate_close_session(&store, phase, WorkspaceSessionStatus::WaitingForHuman, false);
        assert_conflict_kind(
            store.compare_and_save_human_gate_close(&expected, WorkspaceSessionStatus::Running),
            "human_gate_close",
        );
    }
}

/// ⑩F-54 B3：terminate 放行 Failed 相位门（快照在场，0009 形态）。fail-closed
/// 只锁 confirm（防未授权推进），永不锁 terminate——用户脱困权。terminate 不
/// 提升相位，快照与 reservation 照常清空。
#[test]
fn human_gate_close_terminate_relaxes_failed_phase_gate() {
    let (_tmp, store) = setup();
    let expected = gate_close_session(
        &store,
        Some(SingleCandidatePhase::Failed),
        WorkspaceSessionStatus::WaitingForHuman,
        true,
    );
    let closed = store
        .compare_and_save_human_gate_close(&expected, WorkspaceSessionStatus::Terminated)
        .expect("F-54: failed-phase gate must stay terminable (user egress)");
    assert_eq!(closed.status, WorkspaceSessionStatus::Terminated);
    assert_eq!(
        closed.single_candidate_phase,
        Some(SingleCandidatePhase::Failed),
        "terminate 不改相位"
    );
    assert_eq!(closed.human_gate_snapshot, None);
    assert_eq!(closed.human_gate_reservation, None);

    let durable = store.get_workspace_session(&expected.id).unwrap();
    assert_eq!(durable.status, WorkspaceSessionStatus::Terminated);
}

/// ⑪F-54 B3 边界：confirm（Running）对 Failed 相位门维持 Conflict——放行的
/// 只有 terminate（脱困），approve 授权面 {Approval, Evaluate} 一字不动。
#[test]
fn human_gate_close_confirm_still_rejects_failed_phase() {
    let (_tmp, store) = setup();
    let expected = gate_close_session(
        &store,
        Some(SingleCandidatePhase::Failed),
        WorkspaceSessionStatus::WaitingForHuman,
        true,
    );
    assert_conflict_kind(
        store.compare_and_save_human_gate_close(&expected, WorkspaceSessionStatus::Running),
        "human_gate_close",
    );
}
