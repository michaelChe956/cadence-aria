use crate::cross_cutting::streaming_provider::ProviderPermissionMode;
use crate::product::app_paths::ProductAppPaths;
use crate::product::group_chat_engine::finalize::{FinalizeInput, FinalizeService};
use crate::product::group_chat_engine::types::{
    ArtifactDraft, ArtifactLine, ArtifactLineKind, DraftSlot, DraftSlotKey, GroupChatRoleKey,
    GroupChatSessionRecord, GroupChatSessionStatus, RoleInstance, RoomEvent,
};
use crate::product::group_chat_store::GroupChatStore;
use crate::product::issue_store::{CreateProductIssueInput, IssueStore};
use crate::product::json_store::read_json;
use crate::product::lifecycle_store::{CreateStorySpecInput, LifecycleStore};
use crate::product::models::{LifecycleConfirmationStatus, ProviderName, SessionOrigin};
use tempfile::TempDir;

fn setup() -> (TempDir, FinalizeService, GroupChatStore) {
    let temp = tempfile::tempdir().expect("临时目录");
    let paths = ProductAppPaths::new(temp.path());
    let lifecycle = LifecycleStore::new(paths.clone());
    let issue_store = IssueStore::new(paths.clone());
    let group_chat = GroupChatStore::new(paths);
    issue_store
        .create(CreateProductIssueInput {
            project_id: "project-1".into(),
            repo_id: Some("repo-1".into()),
            title: "实现群聊定稿".into(),
            description: Some("旧描述".into()),
            change_id: None,
        })
        .expect("创建 Issue");
    (
        temp,
        FinalizeService::new(lifecycle, issue_store, group_chat.clone()),
        group_chat,
    )
}

fn role(key: GroupChatRoleKey, id: &str) -> RoleInstance {
    RoleInstance {
        id: id.into(),
        role_key: key,
        provider: ProviderName::Fake,
        display_name: id.into(),
        permission_mode: ProviderPermissionMode::Auto,
        seen_cursor: 0,
        injection_watermark: 0,
    }
}

fn session(line: ArtifactLine) -> GroupChatSessionRecord {
    GroupChatSessionRecord {
        id: "room-1".into(),
        project_id: "project-1".into(),
        issue_id: "issue_0001".into(),
        status: GroupChatSessionStatus::Active,
        roles: vec![role(GroupChatRoleKey::Author, "author-1")],
        artifact_lines: vec![line],
        triage_provider: None,
        created_at: "2026-08-18T00:00:00Z".into(),
        updated_at: "2026-08-18T00:00:00Z".into(),
    }
}

fn line(kind: ArtifactLineKind, slot: &str, markdown: &str) -> ArtifactLine {
    ArtifactLine {
        kind,
        drafts: vec![DraftSlot {
            slot_key: DraftSlotKey(slot.into()),
            current: Some(ArtifactDraft {
                version: 1,
                markdown: markdown.into(),
                author_role_id: "author-1".into(),
                based_on_events: 1,
            }),
            claim: None,
        }],
        finalized_versions: vec![],
        entity_id: None,
        bridge_session_id: None,
    }
}

fn input(kind: ArtifactLineKind) -> FinalizeInput {
    FinalizeInput {
        project_id: "project-1".into(),
        issue_id: "issue_0001".into(),
        session_id: "room-1".into(),
        line_kind: kind,
        included_slots_override: None,
        confirmed_by: Some("human".into()),
        provider_run_refs: vec!["turn-1".into()],
        review_refs: vec![],
    }
}

#[test]
fn story定稿写入实体和桥接并翻转旧artifact_current() {
    let (_temp, service, store) = setup();
    store
        .save_session_snapshot(&session(line(
            ArtifactLineKind::StorySpec,
            "story_full",
            "Story v1",
        )))
        .expect("保存群聊会话");

    let event = service
        .finalize_line(input(ArtifactLineKind::StorySpec))
        .expect("定稿");
    let RoomEvent::FinalizeEvent { version, .. } = event else {
        panic!("应返回定稿事件");
    };
    let stories = service
        .lifecycle
        .list_story_specs("project-1", "issue_0001")
        .expect("读取 Story");
    assert_eq!(stories.len(), 1);
    assert_eq!(
        stories[0].confirmation_status,
        LifecycleConfirmationStatus::Confirmed
    );
    assert_eq!(
        service
            .lifecycle
            .list_versions("project-1", "issue_0001", &stories[0].id)
            .unwrap()
            .len(),
        1
    );
    let session = service
        .lifecycle
        .list_workspace_sessions("project-1", "issue_0001")
        .unwrap();
    assert_eq!(session.len(), 1);
    assert_eq!(session[0].origin, Some(SessionOrigin::GroupChat));
    assert_eq!(
        service
            .lifecycle
            .list_artifact_versions(&session[0].id)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(version, "version_0001");
}

#[test]
fn story定稿后保持active并允许design在同一会话定稿() {
    let (_temp, service, store) = setup();
    let mut room = session(line(
        ArtifactLineKind::StorySpec,
        "story_full",
        "# Story Spec",
    ));
    room.artifact_lines.push(line(
        ArtifactLineKind::DesignSpec,
        "design_summary",
        "# Design Spec",
    ));
    store.save_session_snapshot(&room).expect("保存群聊会话");

    service
        .finalize_line(input(ArtifactLineKind::StorySpec))
        .expect("定稿 Story Spec");
    assert_eq!(
        store
            .load_session("project-1", "issue_0001", "room-1")
            .expect("读取 Story 定稿后的会话")
            .status,
        GroupChatSessionStatus::Active
    );

    let event = service
        .finalize_line(input(ArtifactLineKind::DesignSpec))
        .expect("在同一会话定稿 Design Spec");
    assert!(matches!(
        event,
        RoomEvent::FinalizeEvent {
            artifact_line: ArtifactLineKind::DesignSpec,
            ..
        }
    ));

    let room = store
        .load_session("project-1", "issue_0001", "room-1")
        .expect("读取 Design 定稿后的会话");
    assert_eq!(room.status, GroupChatSessionStatus::Active);
    assert_eq!(room.artifact_lines[0].finalized_versions.len(), 1);
    assert_eq!(room.artifact_lines[1].finalized_versions.len(), 1);
    let designs = service
        .lifecycle
        .list_design_specs("project-1", "issue_0001")
        .expect("读取 Design Spec");
    assert_eq!(designs.len(), 1);
    assert_eq!(
        designs[0].confirmation_status,
        LifecycleConfirmationStatus::Confirmed
    );
}

#[test]
fn 二次定稿复用桥接且旧版本不再current() {
    let (_temp, service, store) = setup();
    store
        .save_session_snapshot(&session(line(
            ArtifactLineKind::StorySpec,
            "story_full",
            "v1",
        )))
        .unwrap();
    service
        .finalize_line(input(ArtifactLineKind::StorySpec))
        .unwrap();
    let mut current = store
        .load_session("project-1", "issue_0001", "room-1")
        .unwrap();
    current.artifact_lines[0].drafts[0]
        .current
        .as_mut()
        .unwrap()
        .markdown = "v2".into();
    store.save_session_snapshot(&current).unwrap();
    service
        .finalize_line(input(ArtifactLineKind::StorySpec))
        .unwrap();
    let sessions = service
        .lifecycle
        .list_workspace_sessions("project-1", "issue_0001")
        .unwrap();
    assert_eq!(sessions.len(), 1);
    let versions = service
        .lifecycle
        .list_artifact_versions(&sessions[0].id)
        .unwrap();
    assert_eq!(versions.len(), 2);
    assert!(!versions[0].is_current);
    assert!(versions[1].is_current);
}

#[test]
fn design在story未确认时被派生约束拒绝() {
    let (_temp, service, store) = setup();
    let story = service
        .lifecycle
        .create_story_spec(CreateStorySpecInput {
            project_id: "project-1".into(),
            issue_id: "issue_0001".into(),
            repository_id: "repo-1".into(),
            title: "Story".into(),
        })
        .unwrap();
    let mut design = line(ArtifactLineKind::DesignSpec, "design_summary", "Design");
    design.entity_id = None;
    store
        .save_session_snapshot(&GroupChatSessionRecord {
            roles: vec![role(GroupChatRoleKey::Author, "author-1")],
            ..session(design)
        })
        .unwrap();
    let error = service
        .finalize_line(input(ArtifactLineKind::DesignSpec))
        .unwrap_err();
    assert!(error.to_string().contains("story_spec_not_confirmed"));
    assert_eq!(
        story.confirmation_status,
        LifecycleConfirmationStatus::Draft
    );
}

#[test]
fn issue澄清更新描述并产生修订历史() {
    let (_temp, service, store) = setup();
    store
        .save_session_snapshot(&session(line(
            ArtifactLineKind::IssueRefinement,
            "issue_full",
            "新描述",
        )))
        .unwrap();
    service
        .finalize_line(input(ArtifactLineKind::IssueRefinement))
        .unwrap();
    assert_eq!(
        service
            .issue_store
            .get("project-1", "issue_0001")
            .unwrap()
            .description
            .as_deref(),
        Some("新描述")
    );
    assert_eq!(
        service
            .issue_store
            .list_description_revisions("project-1", "issue_0001")
            .unwrap()
            .len(),
        1
    );
    assert!(
        service
            .lifecycle
            .list_workspace_sessions("project-1", "issue_0001")
            .unwrap()
            .is_empty()
    );
}

#[test]
fn 删除桥接后可按既有实体版本补建() {
    let (temp, service, store) = setup();
    store
        .save_session_snapshot(&session(line(
            ArtifactLineKind::StorySpec,
            "story_full",
            "v1",
        )))
        .unwrap();
    service
        .finalize_line(input(ArtifactLineKind::StorySpec))
        .unwrap();
    let mut room = store
        .load_session("project-1", "issue_0001", "room-1")
        .unwrap();
    let bridge_id = service
        .lifecycle
        .list_workspace_sessions("project-1", "issue_0001")
        .unwrap()[0]
        .id
        .clone();
    std::fs::remove_dir_all(
        temp.path()
            .join("projects/project-1/issues/issue_0001/workspace-sessions"),
    )
    .unwrap();
    room.artifact_lines[0].bridge_session_id = Some(bridge_id);
    store.save_session_snapshot(&room).unwrap();
    assert!(
        service
            .repair_bridge_if_missing(
                "project-1",
                "issue_0001",
                "room-1",
                ArtifactLineKind::StorySpec
            )
            .unwrap()
    );
    let bridge = service
        .lifecycle
        .list_workspace_sessions("project-1", "issue_0001")
        .unwrap();
    assert_eq!(bridge.len(), 1);
    let artifacts = service
        .lifecycle
        .list_artifact_versions(&bridge[0].id)
        .unwrap();
    assert_eq!(artifacts.len(), 1);
    let _: serde_json::Value = read_json(
        &temp
            .path()
            .join("projects/project-1/issues/issue_0001/workspace-sessions")
            .join(format!("{}.json", bridge[0].id)),
    )
    .unwrap();
}
