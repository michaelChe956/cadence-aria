use crate::product::models::{
    HumanGateReservation, HumanGateTurn, HumanGateTurnStatus,
};

#[test]
fn human_gate_recovery_attempt_updates_same_turn_and_terminal_replay_reserves_again() {
    let (_tmp, store) = setup();
    let mut session = create_session(&store, "work_item_plan_attempt", WorkspaceType::WorkItemPlan);
    session.human_gate_snapshot = Some(HumanGateSnapshot {
        findings: Vec::new(),
        repeated_fingerprints: Vec::new(),
        attempts_used: 0,
        manual_repairs_remaining: 2,
        trigger: HumanReason::NativeHumanRequired,
        resumable: true,
    });
    let session_path = store
        .app_paths()
        .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
        .join("workspace-sessions")
        .join(format!("{}.json", session.id));
    write_json(&session_path, &session).unwrap();

    let make_turn = |turn_id: &str, command_id: &str, status| HumanGateTurn {
        turn_id: turn_id.to_string(),
        session_id: session.id.clone(),
        command_id: command_id.to_string(),
        feedback_text: "recover provider".to_string(),
        status,
        attempt_no: 1,
        budget_reserved: 1,
        result_artifact_ref: None,
        failure_class: None,
        created_at: "2026-08-31T00:00:00Z".to_string(),
        updated_at: "2026-08-31T00:00:00Z".to_string(),
    };
    let reservation = |turn: &HumanGateTurn| HumanGateReservation {
        command_id: turn.command_id.clone(),
        turn_id: turn.turn_id.clone(),
        provider_start_idempotency_key: format!("human_gate:{}:attempt:1", turn.turn_id),
        reserved_at: turn.created_at.clone(),
    };

    let first_reserved = make_turn("turn_attempt_0001", "command_attempt_0001", HumanGateTurnStatus::Reserved);
    let (reserved, _) = store
        .compare_and_reserve_human_gate_turn(&session, first_reserved.clone(), reservation(&first_reserved))
        .unwrap();
    let mut running = first_reserved.clone();
    running.status = HumanGateTurnStatus::Running;
    let running_session = store
        .update_human_gate_turn(&reserved, running.clone())
        .unwrap();
    assert_eq!(
        store
            .get_human_gate_turn(&session.id, &first_reserved.turn_id)
            .unwrap()
            .status,
        HumanGateTurnStatus::Running
    );
    assert_eq!(running_session.provider_start_ledger.len(), 1);
    let mut resumed = running.clone();
    resumed.attempt_no = 2;
    let resumed_session = store.update_human_gate_turn(&running_session, resumed.clone()).unwrap();
    assert_eq!(store.get_human_gate_turn(&session.id, &first_reserved.turn_id).unwrap(), resumed);
    assert_eq!(resumed_session.human_gate_snapshot.as_ref().unwrap().manual_repairs_remaining, 1);
    assert_eq!(resumed_session.provider_start_ledger.len(), 2);
    assert!(resumed_session.provider_start_ledger.iter().any(|entry| {
        entry.provider_start_idempotency_key == "human_gate:turn_attempt_0001:attempt:2"
    }));

    let mut failed = resumed.clone();
    failed.status = HumanGateTurnStatus::Failed;
    failed.failure_class = Some(crate::product::models::HumanGateTurnFailureClass::ProviderErr);
    let terminal_session = store.update_human_gate_turn(&resumed_session, failed.clone()).unwrap();
    let second_reserved = make_turn("turn_attempt_0002", "command_attempt_0002", HumanGateTurnStatus::Reserved);
    let (second_session, second_turn) = store
        .compare_and_reserve_human_gate_turn(
            &terminal_session,
            second_reserved.clone(),
            reservation(&second_reserved),
        )
        .unwrap();
    assert_eq!(second_turn.turn_id, second_reserved.turn_id);
    assert_eq!(second_session.human_gate_snapshot.as_ref().unwrap().manual_repairs_remaining, 0);
    assert_eq!(
        store.get_human_gate_turn_by_command_id(&session.id, &first_reserved.command_id).unwrap(),
        Some(failed)
    );

    let nonterminal = make_turn("turn_attempt_0003", "command_attempt_0003", HumanGateTurnStatus::Reserved);
    assert!(matches!(
        store.compare_and_reserve_human_gate_turn(
            &second_session,
            nonterminal.clone(),
            reservation(&nonterminal),
        ),
        Err(ProductStoreError::Conflict { kind: "human_gate_reservation", .. })
    ));
}
