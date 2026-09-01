use crate::product::advance_store::{AdvanceOutcome, AdvanceRecord, AdvanceStatus};
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
