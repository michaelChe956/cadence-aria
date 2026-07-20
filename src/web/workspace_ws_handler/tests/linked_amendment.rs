use super::*;
use crate::product::models::{
    WorkspaceReturnContext, WorkspaceSessionLink, WorkspaceSessionLinkTrigger,
    WorkspaceSessionRelation, WorkspaceSessionStatus,
};
use crate::product::workspace_engine::{
    LinkedWorkspaceAmendmentTarget, LinkedWorkspaceSessionSnapshot,
};

fn target() -> LinkedWorkspaceAmendmentTarget {
    LinkedWorkspaceAmendmentTarget {
        entity_id: "story_spec_0001".to_string(),
        workspace_type: WorkspaceType::Story,
        relation: WorkspaceSessionRelation::StoryAmendment,
    }
}

fn snapshot() -> LinkedWorkspaceSessionSnapshot {
    LinkedWorkspaceSessionSnapshot {
        link: WorkspaceSessionLink {
            id: "workspace_session_link_story_amendment_0001".to_string(),
            relation: WorkspaceSessionRelation::StoryAmendment,
            parent_session_id: "workspace_session_plan_amendment_0001".to_string(),
            child_session_id: "workspace_session_story_amendment_0001".to_string(),
            trigger: WorkspaceSessionLinkTrigger {
                attempt_id: "coding_attempt_0001".to_string(),
                unit_run_id: "coding_unit_run_0001".to_string(),
                review_id: Some("code_review_0001".to_string()),
                finding_id: "finding_0001".to_string(),
                repair_request_id: "plan_repair_request_0001".to_string(),
                amendment_id: "plan_amendment_0001".to_string(),
                fingerprint: "fingerprint_0001".to_string(),
                base_plan_revision_id: "plan_revision_0001".to_string(),
            },
            return_context: WorkspaceReturnContext {
                original_attempt_id: "coding_attempt_0001".to_string(),
                original_unit_run_id: "coding_unit_run_0001".to_string(),
                timeline_anchor_id: "finding_0001".to_string(),
                original_route: "/workbench/workspace/workspace_session_plan_amendment_0001"
                    .to_string(),
            },
            created_at: "2026-07-20T00:00:00Z".to_string(),
        },
        workspace_type: WorkspaceType::Story,
        artifact_version_id: Some(2),
        timeline_nodes: Vec::new(),
        selected_timeline_node_id: None,
        human_confirm_state: WorkspaceSessionStatus::WaitingForHuman,
    }
}

#[test]
fn workspace_session_link_upgrade_command_is_typed_and_stage_scoped() {
    let message = WsInMessage::StartLinkedWorkspaceAmendment { target: target() };
    assert_eq!(
        serde_json::to_value(&message).unwrap(),
        serde_json::json!({
            "type": "start_linked_workspace_amendment",
            "target": {
                "entity_id": "story_spec_0001",
                "workspace_type": "story",
                "relation": "story_amendment"
            }
        })
    );
    assert_eq!(
        serde_json::from_value::<WsInMessage>(serde_json::to_value(&message).unwrap()).unwrap(),
        message
    );
    assert!(is_message_valid_for_stage(
        &message,
        &WorkspaceStage::Running
    ));
    assert!(is_message_valid_for_stage(
        &message,
        &WorkspaceStage::HumanConfirm
    ));
    assert!(!is_message_valid_for_stage(
        &message,
        &WorkspaceStage::PrepareContext
    ));
    assert_eq!(message_type(&message), "start_linked_workspace_amendment");
}

#[test]
fn workspace_session_link_upgrade_response_preserves_shared_recovery_binding() {
    let message = WsOutMessage::LinkedWorkspaceAmendmentCreated {
        snapshot: snapshot(),
    };
    let value = serde_json::to_value(&message).unwrap();
    assert_eq!(value["type"], "linked_workspace_amendment_created");
    assert_eq!(value["snapshot"]["workspace_type"], "story");
    assert_eq!(value["snapshot"]["artifact_version_id"], 2);
    assert_eq!(
        serde_json::from_value::<WsOutMessage>(value).unwrap(),
        message
    );
}
