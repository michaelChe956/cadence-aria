use serde::{Serialize, de::DeserializeOwned};
use serde_json::json;

use crate::cross_cutting::streaming_provider::ProviderPermissionMode;
use crate::product::group_chat_engine::types::{
    ArtifactDraft, ArtifactLine, ArtifactLineKind, ArtifactRef, Claim, DraftSlot, DraftSlotKey,
    GroupChatRoleKey, GroupChatSessionRecord, GroupChatSessionStatus, RoleInstance, RoomEvent,
};
use crate::product::models::ProviderName;

fn assert_round_trip<T>(value: T)
where
    T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let serialized = serde_json::to_string(&value).unwrap();
    let deserialized: T = serde_json::from_str(&serialized).unwrap();

    assert_eq!(deserialized, value);
}

fn draft_slot_key(value: &str) -> DraftSlotKey {
    DraftSlotKey(value.into())
}

fn role_instance() -> RoleInstance {
    RoleInstance {
        id: "author-1".into(),
        role_key: GroupChatRoleKey::Author,
        provider: ProviderName::ClaudeCode,
        display_name: "作者".into(),
        permission_mode: ProviderPermissionMode::Auto,
        seen_cursor: 3,
        injection_watermark: 2,
    }
}

fn draft_slot() -> DraftSlot {
    DraftSlot {
        slot_key: draft_slot_key("design_frontend"),
        current: Some(ArtifactDraft {
            version: 2,
            markdown: "# 前端设计".into(),
            author_role_id: "frontend-1".into(),
            based_on_events: 5,
        }),
        claim: Some(Claim {
            holder_role_id: "frontend-1".into(),
            claimed_at: "2026-08-18T12:00:00Z".into(),
        }),
    }
}

#[test]
fn group_chat_types_round_trip_through_serde() {
    for role_key in [
        GroupChatRoleKey::Author,
        GroupChatRoleKey::FrontendDesign,
        GroupChatRoleKey::BackendDesign,
        GroupChatRoleKey::Reviewer,
        GroupChatRoleKey::Researcher,
    ] {
        assert_round_trip(role_key);
    }

    for slot_key in [
        draft_slot_key("issue_full"),
        draft_slot_key("story_full"),
        draft_slot_key("design_frontend"),
        draft_slot_key("design_backend"),
        draft_slot_key("design_summary"),
    ] {
        assert_round_trip(slot_key);
    }

    for line_kind in [
        ArtifactLineKind::IssueRefinement,
        ArtifactLineKind::StorySpec,
        ArtifactLineKind::DesignSpec,
    ] {
        assert_round_trip(line_kind);
    }

    for status in [
        GroupChatSessionStatus::Active,
        GroupChatSessionStatus::Finalized,
        GroupChatSessionStatus::Archived,
    ] {
        assert_round_trip(status);
    }

    assert_round_trip(role_instance());
    assert_round_trip(ArtifactDraft {
        version: 1,
        markdown: "# Story".into(),
        author_role_id: "author-1".into(),
        based_on_events: 4,
    });
    assert_round_trip(Claim {
        holder_role_id: "author-1".into(),
        claimed_at: "2026-08-18T12:00:00Z".into(),
    });
    assert_round_trip(draft_slot());
    assert_round_trip(ArtifactLine {
        kind: ArtifactLineKind::DesignSpec,
        drafts: vec![draft_slot()],
        finalized_versions: vec!["version_1".into()],
    });
    assert_round_trip(ArtifactRef {
        line: ArtifactLineKind::DesignSpec,
        slot: draft_slot_key("design_frontend"),
        version: 2,
    });
    assert_round_trip(GroupChatSessionRecord {
        id: "room-1".into(),
        project_id: "project-1".into(),
        issue_id: "issue-1".into(),
        status: GroupChatSessionStatus::Active,
        roles: vec![role_instance()],
        artifact_lines: vec![ArtifactLine {
            kind: ArtifactLineKind::DesignSpec,
            drafts: vec![draft_slot()],
            finalized_versions: vec!["version_1".into()],
        }],
        created_at: "2026-08-18T12:00:00Z".into(),
        updated_at: "2026-08-18T12:01:00Z".into(),
    });
}

#[test]
fn group_chat_types_room_events_round_trip_through_serde() {
    let events = vec![
        RoomEvent::UserMessage {
            text: "请完善设计".into(),
            mentions: vec!["author-1".into()],
        },
        RoomEvent::AgentMessage {
            role_instance_id: "author-1".into(),
            text: "我会起草。".into(),
            artifact_ref: Some(ArtifactRef {
                line: ArtifactLineKind::StorySpec,
                slot: draft_slot_key("story_full"),
                version: 1,
            }),
            cursor_after: 6,
        },
        RoomEvent::ClaimEvent {
            role_instance_id: "author-1".into(),
            line: ArtifactLineKind::StorySpec,
            slot_key: draft_slot_key("story_full"),
            claimed: true,
        },
        RoomEvent::HeldEvent {
            role_instance_id: "reviewer-1".into(),
            reason: "上下文已过期".into(),
            cursor_after: 7,
        },
        RoomEvent::FinalizeEvent {
            artifact_line: ArtifactLineKind::DesignSpec,
            version: "version_2".into(),
            included_slots: vec![
                draft_slot_key("design_frontend"),
                draft_slot_key("design_backend"),
                draft_slot_key("design_summary"),
            ],
        },
        RoomEvent::SystemNotice {
            text: "当前讨论暂无待响应方".into(),
        },
    ];

    for event in events {
        assert_round_trip(event);
    }
}

#[test]
fn group_chat_types_room_events_use_snake_case_type_tags() {
    let events = [
        (
            RoomEvent::UserMessage {
                text: "消息".into(),
                mentions: vec![],
            },
            "user_message",
        ),
        (
            RoomEvent::AgentMessage {
                role_instance_id: "author-1".into(),
                text: "回复".into(),
                artifact_ref: None,
                cursor_after: 1,
            },
            "agent_message",
        ),
        (
            RoomEvent::ClaimEvent {
                role_instance_id: "author-1".into(),
                line: ArtifactLineKind::StorySpec,
                slot_key: draft_slot_key("story_full"),
                claimed: true,
            },
            "claim_event",
        ),
        (
            RoomEvent::HeldEvent {
                role_instance_id: "author-1".into(),
                reason: "过期".into(),
                cursor_after: 1,
            },
            "held_event",
        ),
        (
            RoomEvent::FinalizeEvent {
                artifact_line: ArtifactLineKind::StorySpec,
                version: "version_1".into(),
                included_slots: vec![draft_slot_key("story_full")],
            },
            "finalize_event",
        ),
        (
            RoomEvent::SystemNotice {
                text: "通知".into(),
            },
            "system_notice",
        ),
    ];

    for (event, expected_type) in events {
        let value = serde_json::to_value(event).unwrap();
        assert_eq!(value["type"], json!(expected_type));
    }
}
