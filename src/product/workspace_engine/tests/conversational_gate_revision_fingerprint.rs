use crate::product::models::WorkspaceType;
use crate::product::work_item_plan_policy::{
    FindingFingerprint, HumanReason, ReviewInvocationScope, RunBudgets, RunHistory, RunPolicy,
};
use crate::product::workspace_engine::review::policy_routing::{
    GateSnapshotContext, RoutingAction, route_outcome, route_repeated_human_gate_fingerprint,
};
use crate::product::workspace_engine::{
    HumanGateCommandOutcome, HumanGateFeedbackInput, WorkspaceStage,
    artifact_constraint_spec_call_count, artifact_constraint_spec_for,
    reset_artifact_constraint_spec_call_count,
};

fn feedback(command_id: &str) -> HumanGateFeedbackInput {
    HumanGateFeedbackInput {
        command_id: command_id.to_string(),
        feedback: "只修正这个字段".to_string(),
    }
}

#[test]
fn conversational_gate_revision_repeated_fingerprint_returns_same_gate() {
    let fingerprint = FindingFingerprint::new("a".repeat(64)).expect("fingerprint");
    let snapshot = crate::product::work_item_plan_policy::HumanGateSnapshot {
        findings: Vec::new(),
        repeated_fingerprints: vec![fingerprint.clone()],
        attempts_used: 2,
        manual_repairs_remaining: 2,
        trigger: HumanReason::RepeatedFingerprint,
        resumable: false,
    };
    assert!(route_repeated_human_gate_fingerprint(&snapshot, &fingerprint).is_ok());
    let context = GateSnapshotContext {
        history: RunHistory {
            manual_repairs_used: 1,
            ..RunHistory::default()
        },
        budgets: RunBudgets {
            max_manual_repairs: 2,
            ..RunBudgets::default()
        },
        invocation: ReviewInvocationScope::initial("revision_001"),
        findings: Vec::new(),
        repeated_fingerprints: vec![fingerprint.clone()],
        trigger: HumanReason::RepeatedFingerprint,
    };
    let action = route_outcome(
        crate::product::work_item_plan_policy::PlanOutcome::HumanRequired {
            findings: Vec::new(),
            repeated_fingerprints: vec![fingerprint],
            reason: HumanReason::RepeatedFingerprint,
        },
        RunPolicy::Interactive,
        context,
    );
    let RoutingAction::EnterHumanGate { snapshot: routed } = action else {
        panic!("repeated finding must remain in the existing gate");
    };
    assert_eq!(routed.manual_repairs_remaining, 1);
}

#[test]
fn conversational_gate_revision_legacy_constraint_spy_positive_control() {
    reset_artifact_constraint_spec_call_count();
    let _ = artifact_constraint_spec_for(&WorkspaceType::Story);
    assert!(artifact_constraint_spec_call_count() > 0);
}
#[tokio::test]
async fn conversational_gate_revision_attempt_no_has_fixed_upper_bound() {
    let (_root, lifecycle, mut engine) = super::conversational_gate::gate_fixture(2);
    let opened = engine
        .handle_human_gate_feedback(feedback("attempt_limit"))
        .await
        .expect("open turn");
    let turn_id = match opened {
        HumanGateCommandOutcome::TurnOpened { turn, .. } => turn.turn_id,
        other => panic!("expected turn, got {other:?}"),
    };
    engine
        .mark_human_gate_turn_running(&turn_id)
        .expect("running");
    let expected = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("session");
    let mut invalid = lifecycle
        .get_human_gate_turn(engine.session().session_id.as_str(), &turn_id)
        .expect("turn");
    invalid.attempt_no = 3;
    assert!(
        lifecycle
            .update_human_gate_turn(&expected, invalid)
            .is_err()
    );
}

#[tokio::test]
async fn conversational_gate_revision_ledger_counts_real_starts_only() {
    // Same command replay: no second logical budget or provider start.
    let (_root, lifecycle, mut engine) = super::conversational_gate::gate_fixture(3);
    let opened = engine
        .handle_human_gate_feedback(feedback("ledger_replay"))
        .await
        .expect("open turn");
    let turn_id = match opened {
        HumanGateCommandOutcome::TurnOpened { turn, .. } => turn.turn_id,
        other => panic!("expected turn, got {other:?}"),
    };
    let before = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("before replay");
    let replay = engine
        .handle_human_gate_feedback(feedback("ledger_replay"))
        .await
        .expect("replay");
    assert!(matches!(replay, HumanGateCommandOutcome::Replayed { .. }));
    let after_replay = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("after replay");
    assert_eq!(
        after_replay.provider_start_ledger,
        before.provider_start_ledger
    );
    assert_eq!(after_replay.human_gate_snapshot, before.human_gate_snapshot);

    // Same-turn reconnect: idempotent running transition does not start again.
    engine
        .mark_human_gate_turn_running(&turn_id)
        .expect("first running transition");
    engine
        .mark_human_gate_turn_running(&turn_id)
        .expect("reconnect running transition");
    let after_reconnect = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("after reconnect");
    assert_eq!(after_reconnect.provider_start_ledger.len(), 1);

    // Transport retry: exactly one additional attempt key, duplicate retry rejected.
    let mut retry = lifecycle
        .get_human_gate_turn(engine.session().session_id.as_str(), &turn_id)
        .expect("running turn");
    retry.attempt_no = 2;
    let retry_session = lifecycle
        .update_human_gate_turn(&after_reconnect, retry.clone())
        .expect("attempt two");
    assert_eq!(retry_session.provider_start_ledger.len(), 2);
    assert_eq!(
        retry_session
            .provider_start_ledger
            .iter()
            .map(|entry| entry.provider_start_idempotency_key.as_str())
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        2
    );
    assert!(
        lifecycle
            .update_human_gate_turn(&retry_session, retry)
            .is_ok()
    );
    let final_session = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("final session");
    assert_eq!(final_session.provider_start_ledger.len(), 2);
}

#[tokio::test]
async fn legacy_revision_paths_remain_unchanged_when_conversational_gate_is_enabled() {
    let (_root, lifecycle, engine) = super::conversational_gate::gate_fixture(1);
    let before = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("before");
    assert_eq!(before.workspace_type, WorkspaceType::WorkItemPlan);
    assert_eq!(engine.current_stage(), WorkspaceStage::HumanConfirm);
    let story_spec =
        crate::product::workspace_engine::artifact_constraint_spec_for(&WorkspaceType::Story);
    let design_spec =
        crate::product::workspace_engine::artifact_constraint_spec_for(&WorkspaceType::Design);
    assert!(!story_spec.required_headings.is_empty());
    assert!(!design_spec.required_headings.is_empty());
    assert_eq!(
        lifecycle
            .get_workspace_session(engine.session().session_id.as_str())
            .expect("after")
            .provider_start_ledger,
        before.provider_start_ledger
    );
}
