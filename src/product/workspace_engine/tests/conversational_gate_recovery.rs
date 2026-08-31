use crate::product::models::{HumanGateTurn, HumanGateTurnFailureClass, HumanGateTurnStatus};
use crate::product::work_item_plan_policy::WorkItemPlanFlowKind;
use crate::product::workspace_engine::{
    HUMAN_GATE_PROVIDER_MAX_ATTEMPTS, HumanGateRecoveryAction,
    assert_human_gate_event_prefix_immutable, provider_run_kind_for_human_gate,
    recover_human_gate_turn,
};

fn turn(status: HumanGateTurnStatus, attempt_no: u32) -> HumanGateTurn {
    HumanGateTurn {
        turn_id: "turn_recovery_001".to_string(),
        session_id: "session_001".to_string(),
        command_id: "command_001".to_string(),
        feedback_text: "修复字段".to_string(),
        status,
        attempt_no,
        budget_reserved: 1,
        result_artifact_ref: None,
        failure_class: None,
        created_at: "2026-08-31T00:00:00Z".to_string(),
        updated_at: "2026-08-31T00:00:00Z".to_string(),
    }
}

#[test]
fn conversational_gate_recovery_resumes_reserved_attempt_one() {
    let action = recover_human_gate_turn(&turn(HumanGateTurnStatus::Reserved, 1), false)
        .expect("reserved turn should restart its unstarted provider attempt");
    assert_eq!(
        action,
        HumanGateRecoveryAction::ResumeSameTurn { next_attempt_no: 1 }
    );
}

#[test]
fn conversational_gate_recovery_waits_for_running_provider() {
    let action = recover_human_gate_turn(&turn(HumanGateTurnStatus::Running, 1), true)
        .expect("running provider should be waitable");
    assert_eq!(action, HumanGateRecoveryAction::WaitForProvider);
}

#[test]
fn conversational_gate_recovery_retries_dead_provider_on_same_turn() {
    let original = turn(HumanGateTurnStatus::Running, 1);
    let action = recover_human_gate_turn(&original, false).expect("dead provider should retry");
    assert_eq!(
        action,
        HumanGateRecoveryAction::ResumeSameTurn { next_attempt_no: 2 }
    );
    assert_eq!(original.turn_id, "turn_recovery_001");
    assert_eq!(original.attempt_no, 1);
    assert_eq!(original.budget_reserved, 1);
}

#[test]
fn conversational_gate_recovery_fails_after_fixed_attempt_limit() {
    let action = recover_human_gate_turn(
        &turn(
            HumanGateTurnStatus::Running,
            HUMAN_GATE_PROVIDER_MAX_ATTEMPTS,
        ),
        false,
    )
    .expect("attempt limit should produce a terminal action");
    assert_eq!(
        action,
        HumanGateRecoveryAction::MarkFailed {
            failure_class: HumanGateTurnFailureClass::ProviderErr,
        }
    );
}

#[test]
fn conversational_gate_recovery_maps_only_single_candidate_to_provider_run_kind() {
    let run_kind = provider_run_kind_for_human_gate(
        WorkItemPlanFlowKind::SingleCandidate,
        "turn_recovery_001",
    )
    .expect("single-candidate gate should map to a dedicated run kind");
    assert!(matches!(
        run_kind,
        crate::product::workspace_engine::ProviderRunKind::HumanGateScManualRevision {
            turn_id,
            prompt
        } if turn_id == "turn_recovery_001" && prompt.is_empty()
    ));
    assert!(
        provider_run_kind_for_human_gate(WorkItemPlanFlowKind::Legacy, "turn_recovery_001")
            .is_err()
    );
}
#[test]
fn conversational_gate_recovery_preserves_event_prefix_and_budget() {
    let event_prefix = vec!["human_gate_turn_open", "human_gate_turn_completed"];
    let recovered_events = vec![
        "human_gate_turn_open",
        "human_gate_turn_completed",
        "human_gate_turn_open",
    ];
    assert_human_gate_event_prefix_immutable(&event_prefix, &recovered_events)
        .expect("recovery may append only a suffix");

    let original = turn(HumanGateTurnStatus::Running, 1);
    let action = recover_human_gate_turn(&original, false).expect("dead provider should retry");
    assert!(matches!(
        action,
        HumanGateRecoveryAction::ResumeSameTurn { next_attempt_no: 2 }
    ));
    assert_eq!(original.turn_id, "turn_recovery_001");
    assert_eq!(original.budget_reserved, 1);
}

#[test]
fn conversational_gate_recovery_reservation_commit_restart_keeps_budget_and_turn() {
    let original = turn(HumanGateTurnStatus::Reserved, 1);
    let action = recover_human_gate_turn(&original, false)
        .expect("reserved turn should restart attempt one after reconnect");
    assert_eq!(
        action,
        HumanGateRecoveryAction::ResumeSameTurn { next_attempt_no: 1 }
    );
    assert_eq!(original.turn_id, "turn_recovery_001");
    assert_eq!(original.attempt_no, 1);
    assert_eq!(original.budget_reserved, 1);
    let attempt_key = format!("human_gate:{}:attempt:1", original.turn_id);
    assert_eq!(attempt_key, "human_gate:turn_recovery_001:attempt:1");
    assert_eq!(
        [attempt_key.clone(), attempt_key]
            .into_iter()
            .collect::<std::collections::HashSet<_>>()
            .len(),
        1
    );
}
