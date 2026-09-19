use crate::product::advance_store::{
    AdvanceOutcome, AdvanceRecord, AdvanceStatus, AdvanceTargetAttemptBinding,
};
use crate::web::workspace_ws_types::WsOutMessage;

use super::map_advance_outcome;

fn replay_record(status: AdvanceStatus) -> AdvanceRecord {
    AdvanceRecord {
        id: "advance_map_test".to_string(),
        command_id: "command_map_test".to_string(),
        project_id: "project_map_test".to_string(),
        issue_id: "issue_map_test".to_string(),
        plan_id: "plan_map_test".to_string(),
        plan_revision_id: "revision_map_test".to_string(),
        attempt_id: Some("attempt_map_test".to_string()),
        target_attempts: Vec::new(),
        status,
        workspace_entry: Some("/worktree/map-test".to_string()),
        error: None,
        created_at: "2026-08-31T00:00:00Z".to_string(),
        updated_at: "2026-08-31T00:00:01Z".to_string(),
    }
}

#[test]
fn ready_replay_maps_to_completed_message() {
    let message = map_advance_outcome(
        "command_map_test".to_string(),
        AdvanceOutcome::Replayed {
            record: replay_record(AdvanceStatus::Ready),
        },
    );
    assert!(matches!(message, WsOutMessage::AdvanceCompleted { .. }));
}

fn advance_replay_record_with_status(status: AdvanceStatus) -> AdvanceRecord {
    let mut record = replay_record(AdvanceStatus::Ready);
    record.status = status;
    record
}

#[test]
fn advance_replay_mapping_emits_only_completed_for_ready_durable_record() {
    for status in [
        AdvanceStatus::Initializing,
        AdvanceStatus::Running,
        AdvanceStatus::AwaitingPlanAmendment,
        AdvanceStatus::Completed,
        AdvanceStatus::Failed,
        AdvanceStatus::Aborted,
    ] {
        let message = map_advance_outcome(
            "command_map_test".to_string(),
            AdvanceOutcome::Replayed {
                record: advance_replay_record_with_status(status.clone()),
            },
        );
        assert!(
            matches!(message, WsOutMessage::AdvanceRejected { ref code, .. } if code == "ADVANCE_REPLAY_NOT_READY"),
            "{status:?} must not emit advance_completed"
        );
    }
}

fn multi_target_replay_record() -> AdvanceRecord {
    let mut record = replay_record(AdvanceStatus::Ready);
    record.attempt_id = Some("attempt_first".to_string());
    record.target_attempts = vec![
        AdvanceTargetAttemptBinding {
            target_repository_id: "11111111-1111-1111-1111-111111111111".to_string(),
            attempt_id: "attempt_first".to_string(),
        },
        AdvanceTargetAttemptBinding {
            target_repository_id: "22222222-2222-2222-2222-222222222222".to_string(),
            attempt_id: "attempt_second".to_string(),
        },
    ];
    record
}

#[test]
fn multi_target_ready_replay_maps_to_completed_with_target_attempts_payload() {
    // k3 F3 必改点二：多 target Ready record 不因单值 attempt_id 误落
    // ADVANCE_REPLAY_INCOMPLETE——完备判据=「集合非空+workspace_entry 在」。
    let message = map_advance_outcome(
        "command_map_test".to_string(),
        AdvanceOutcome::Replayed {
            record: multi_target_replay_record(),
        },
    );
    match message {
        WsOutMessage::AdvanceCompleted {
            attempt_id,
            workspace_entry,
            target_attempts,
            ..
        } => {
            assert_eq!(attempt_id, "attempt_first");
            assert_eq!(workspace_entry, "/worktree/map-test");
            assert_eq!(target_attempts.len(), 2, "full target set on the wire");
        }
        other => panic!("multi-target replay must complete, got {other:?}"),
    }
}

#[test]
fn multi_target_ready_replay_without_workspace_entry_stays_incomplete() {
    let mut record = multi_target_replay_record();
    record.workspace_entry = None;
    let message = map_advance_outcome(
        "command_map_test".to_string(),
        AdvanceOutcome::Replayed { record },
    );
    assert!(
        matches!(message, WsOutMessage::AdvanceRejected { ref code, .. } if code == "ADVANCE_REPLAY_INCOMPLETE"),
        "workspace_entry missing must stay incomplete even for multi-target records"
    );
}

#[test]
fn single_target_replay_without_attempt_id_stays_incomplete() {
    // 单 target 判据零变化：空集合时仍要求 (attempt_id, workspace_entry) 双在场。
    let mut record = replay_record(AdvanceStatus::Ready);
    record.attempt_id = None;
    let message = map_advance_outcome(
        "command_map_test".to_string(),
        AdvanceOutcome::Replayed { record },
    );
    assert!(
        matches!(message, WsOutMessage::AdvanceRejected { ref code, .. } if code == "ADVANCE_REPLAY_INCOMPLETE")
    );
}

#[test]
fn single_target_completed_wire_omits_target_attempts() {
    // 单 target wire 零变化：空集不发送 target_attempts 键。
    let message = map_advance_outcome(
        "command_map_test".to_string(),
        AdvanceOutcome::Completed {
            record: replay_record(AdvanceStatus::Ready),
            attempt_id: "attempt_map_test".to_string(),
            workspace_entry: "/worktree/map-test".to_string(),
            target_attempts: Vec::new(),
        },
    );
    let serialized = serde_json::to_value(&message).unwrap();
    // WsOutMessage 为 internally tagged（type=advance_completed，字段内联）。
    assert_eq!(serialized["type"], "advance_completed");
    assert!(
        serialized.get("target_attempts").is_none(),
        "wire must omit empty target_attempts: {serialized}"
    );
}
