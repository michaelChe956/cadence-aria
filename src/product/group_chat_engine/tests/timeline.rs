use std::fs;

use crate::cross_cutting::streaming_provider::ProviderPermissionMode;
use crate::product::app_paths::ProductAppPaths;
use crate::product::group_chat_engine::timeline;
use crate::product::group_chat_engine::types::{
    ArtifactLine, GroupChatRoleKey, GroupChatSessionRecord, GroupChatSessionStatus, RoleInstance,
    RoomEvent,
};
use crate::product::group_chat_store::GroupChatStore;
use crate::product::json_store::read_json;
use crate::product::models::ProviderName;
use tempfile::tempdir;

fn session() -> GroupChatSessionRecord {
    GroupChatSessionRecord {
        id: "session-1".into(),
        project_id: "project-1".into(),
        issue_id: "issue-1".into(),
        status: GroupChatSessionStatus::Active,
        roles: vec![
            RoleInstance {
                id: "author-1".into(),
                role_key: GroupChatRoleKey::Author,
                provider: ProviderName::ClaudeCode,
                display_name: "作者".into(),
                permission_mode: ProviderPermissionMode::Auto,
                seen_cursor: 0,
                injection_watermark: 0,
            },
            RoleInstance {
                id: "reviewer-1".into(),
                role_key: GroupChatRoleKey::Reviewer,
                provider: ProviderName::Codex,
                display_name: "审查者".into(),
                permission_mode: ProviderPermissionMode::Supervised,
                seen_cursor: 0,
                injection_watermark: 0,
            },
        ],
        artifact_lines: Vec::<ArtifactLine>::new(),
        created_at: "2026-08-18T12:00:00Z".into(),
        updated_at: "2026-08-18T12:00:00Z".into(),
    }
}

#[test]
fn group_chat_store_appends_events_and_replays_authoritative_cursors() {
    let root = tempdir().expect("临时目录");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let store = GroupChatStore::new(paths.clone());
    let snapshot = session();
    store
        .save_session_snapshot(&snapshot)
        .expect("保存初始快照");

    assert_eq!(
        store
            .append_event(
                "project-1",
                "issue-1",
                "session-1",
                RoomEvent::UserMessage {
                    text: "请开始讨论".into(),
                    mentions: vec!["author-1".into()],
                },
            )
            .expect("追加第一条事件"),
        1
    );
    assert_eq!(
        store
            .append_event(
                "project-1",
                "issue-1",
                "session-1",
                RoomEvent::AgentMessage {
                    role_instance_id: "author-1".into(),
                    text: "我会起草 Story。".into(),
                    artifact_ref: None,
                    cursor_after: 1,
                },
            )
            .expect("追加第二条事件"),
        2
    );
    assert_eq!(
        store
            .append_event(
                "project-1",
                "issue-1",
                "session-1",
                RoomEvent::HeldEvent {
                    role_instance_id: "reviewer-1".into(),
                    reason: "等待最新上下文".into(),
                    cursor_after: 2,
                },
            )
            .expect("追加第三条事件"),
        3
    );

    let loaded = store
        .load_session("project-1", "issue-1", "session-1")
        .expect("重放会话");
    assert_eq!(loaded.roles[0].seen_cursor, 1);
    assert_eq!(loaded.roles[1].seen_cursor, 2);
    assert_eq!(loaded.roles[0].injection_watermark, 0);
    assert_eq!(loaded.roles[1].injection_watermark, 0);

    let snapshot_path = paths
        .group_chat_session_root("project-1", "issue-1", "session-1")
        .join("session.json");
    let cached: GroupChatSessionRecord = read_json(&snapshot_path).expect("读取快照缓存");
    assert_eq!(cached.roles[0].seen_cursor, 1);
    assert_eq!(cached.roles[1].seen_cursor, 2);

    let timeline_path = paths
        .group_chat_session_root("project-1", "issue-1", "session-1")
        .join("timeline.jsonl");
    let timeline = fs::read_to_string(timeline_path).expect("读取时间线");
    assert!(timeline.contains("请开始讨论"));
    assert!(timeline.contains("我会起草 Story。"));
    assert!(timeline.contains("等待最新上下文"));
}

#[test]
fn group_chat_store_replays_event_written_before_crash_without_new_snapshot() {
    let root = tempdir().expect("临时目录");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let store = GroupChatStore::new(paths.clone());
    let snapshot = session();
    store
        .save_session_snapshot(&snapshot)
        .expect("保存崩溃前快照");

    let timeline_path = paths
        .group_chat_session_root("project-1", "issue-1", "session-1")
        .join("timeline.jsonl");
    timeline::append_event(
        &timeline_path,
        1,
        &RoomEvent::AgentMessage {
            role_instance_id: "author-1".into(),
            text: "事件已经持久化，但快照尚未来得及更新。".into(),
            artifact_ref: None,
            cursor_after: 9,
        },
    )
    .expect("模拟已落盘事件");

    let recovered = store
        .load_session("project-1", "issue-1", "session-1")
        .expect("从崩溃态恢复");
    assert_eq!(recovered.roles[0].seen_cursor, 9);
    assert_eq!(recovered.roles[0].injection_watermark, 0);
}
