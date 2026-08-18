use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::cross_cutting::streaming_provider::{
    PermissionRequestData, ProviderCompletion, ProviderEvent, ProviderPermissionMode,
    ProviderSession, RiskLevel, ScriptedFakeProvider, ScriptedReply, StreamingProviderAdapter,
    StreamingProviderInput,
};
use crate::product::app_paths::ProductAppPaths;
use crate::product::group_chat_engine::agent_turn::HoldRetryPolicy;
use crate::product::group_chat_engine::coordinator::{
    Coordinator, CoordinatorConfig, ProviderAdapterRegistry,
};
use crate::product::group_chat_engine::roles::STORY_FULL_SLOT;
use crate::product::group_chat_engine::triage::{TriageInput, TriageOutput, TriageRouter};
use crate::product::group_chat_engine::types::{
    ArtifactLine, ArtifactLineKind, ArtifactRef, DraftSlot, DraftSlotKey, GroupChatRoleKey,
    GroupChatSessionRecord, GroupChatSessionStatus, RoleInstance, RoomEvent,
};
use crate::product::group_chat_store::GroupChatStore;
use crate::product::models::ProviderName;
use tempfile::TempDir;

#[derive(Clone, Copy)]
enum Script {
    AuthorThenReviewerThenNoOne,
    AlternateForever,
    ReviewerOnly,
}

impl TriageRouter for Script {
    fn route(&self, input: &TriageInput) -> TriageOutput {
        match self {
            Self::AuthorThenReviewerThenNoOne => {
                if input.last_speaker.is_none() {
                    TriageOutput::RespondTo(vec!["author-1".into()])
                } else if input.last_speaker.as_deref() == Some("author-1") {
                    TriageOutput::RespondTo(vec!["reviewer-1".into()])
                } else {
                    TriageOutput::NoOneNeedsToRespond
                }
            }
            Self::AlternateForever => match input.last_speaker.as_deref() {
                Some("author-1") => TriageOutput::RespondTo(vec!["reviewer-1".into()]),
                _ => TriageOutput::RespondTo(vec!["author-1".into()]),
            },
            Self::ReviewerOnly => TriageOutput::NoOneNeedsToRespond,
        }
    }
}

struct RequestDenyFailureProvider;

#[async_trait::async_trait]
impl StreamingProviderAdapter for RequestDenyFailureProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (event_tx, event_rx) = mpsc::channel(1);
        let (command_tx, command_rx) = mpsc::channel(1);
        drop(command_rx);
        event_tx
            .send(ProviderEvent::PermissionRequest(PermissionRequestData {
                id: "permission-1".into(),
                tool_name: "write_file".into(),
                description: "写草稿".into(),
                risk_level: RiskLevel::Medium,
            }))
            .await
            .expect("事件接收端仍打开");
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }
}

fn role(id: &str, role_key: GroupChatRoleKey, display_name: &str) -> RoleInstance {
    RoleInstance {
        id: id.into(),
        role_key,
        provider: ProviderName::Fake,
        display_name: display_name.into(),
        permission_mode: ProviderPermissionMode::Auto,
        seen_cursor: 0,
        injection_watermark: 0,
    }
}

fn session(roles: Vec<RoleInstance>) -> GroupChatSessionRecord {
    GroupChatSessionRecord {
        id: "session-1".into(),
        project_id: "project-1".into(),
        issue_id: "issue-1".into(),
        status: GroupChatSessionStatus::Active,
        roles,
        artifact_lines: vec![ArtifactLine {
            kind: ArtifactLineKind::StorySpec,
            drafts: vec![DraftSlot {
                slot_key: DraftSlotKey(STORY_FULL_SLOT.into()),
                current: None,
                claim: None,
            }],
            finalized_versions: vec![],
        }],
        created_at: "2026-08-18T00:00:00Z".into(),
        updated_at: "2026-08-18T00:00:00Z".into(),
    }
}

fn completed(text: &str) -> Vec<ProviderEvent> {
    vec![ProviderEvent::Completed(ProviderCompletion::plain(
        text, None,
    ))]
}

fn adapters() -> ProviderAdapterRegistry {
    let fake = ScriptedFakeProvider::new(vec![
        ScriptedReply {
            match_prompt_contains: "角色：作者".into(),
            events: completed("作者已提交草稿"),
        },
        ScriptedReply {
            match_prompt_contains: "角色：审稿人".into(),
            events: completed("审稿人给出证据化意见"),
        },
    ]);
    HashMap::from([(
        ProviderName::Fake,
        Arc::new(fake) as Arc<dyn StreamingProviderAdapter>,
    )])
}

fn config() -> CoordinatorConfig {
    CoordinatorConfig {
        // 硬上限场景必须隔离空转熔断，空转合取条件由独立测试覆盖。
        stall_window: 100,
        spawn_interval: Duration::ZERO,
        rate_limit_backoff: Duration::ZERO,
        hold_retry_policy: HoldRetryPolicy::without_delay(),
        ..CoordinatorConfig::default()
    }
}

fn fixture(roles: Vec<RoleInstance>, script: Script) -> (TempDir, GroupChatStore, Coordinator) {
    let temp = tempfile::tempdir().expect("临时目录");
    let store = GroupChatStore::new(ProductAppPaths::new(temp.path()));
    store
        .save_session_snapshot(&session(roles))
        .expect("保存会话");
    let coordinator =
        Coordinator::new(store.clone(), adapters(), Box::new(script)).with_config(config());
    (temp, store, coordinator)
}

#[tokio::test]
async fn 用户消息经_triage_驱动作者和审稿人并自然结束() {
    let (_temp, store, mut coordinator) = fixture(
        vec![
            role("author-1", GroupChatRoleKey::Author, "作者"),
            role("reviewer-1", GroupChatRoleKey::Reviewer, "审稿人"),
        ],
        Script::AuthorThenReviewerThenNoOne,
    );

    let summary = coordinator
        .on_user_message("project-1", "issue-1", "session-1", "请起草 Story", vec![])
        .await
        .expect("协调完成");
    let events = store
        .load_events("project-1", "issue-1", "session-1")
        .expect("读取时间线");

    assert!(summary.no_one_notice);
    assert!(!summary.circuit_break);
    assert_eq!(
        events
            .iter()
            .filter_map(|event| match event {
                RoomEvent::AgentMessage {
                    role_instance_id, ..
                } => Some(role_instance_id.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>(),
        vec!["author-1", "reviewer-1"]
    );
    assert!(events.iter().any(|event| {
        matches!(event, RoomEvent::SystemNotice { text } if text == "当前讨论暂无待响应方")
    }));
}

#[tokio::test]
async fn 达到_hard_loop_cap_后写入熔断提示且不再发言() {
    let (_temp, store, mut coordinator) = fixture(
        vec![
            role("author-1", GroupChatRoleKey::Author, "作者"),
            role("reviewer-1", GroupChatRoleKey::Reviewer, "审稿人"),
        ],
        Script::AlternateForever,
    );

    let summary = coordinator
        .on_user_message("project-1", "issue-1", "session-1", "开始持续讨论", vec![])
        .await
        .expect("协调完成");
    let events = store
        .load_events("project-1", "issue-1", "session-1")
        .expect("读取时间线");

    assert!(summary.circuit_break);
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, RoomEvent::AgentMessage { .. }))
            .count(),
        12
    );
    assert!(events.iter().any(|event| {
        matches!(event, RoomEvent::SystemNotice { text } if text == "讨论已暂停，等待你的输入")
    }));
}

#[test]
fn 连续四条消息但草稿槽版本持续变化时不触发空转熔断() {
    let mut progress_events = vec![
        RoomEvent::UserMessage {
            text: "继续完善草稿".into(),
            mentions: vec![],
        },
        RoomEvent::AgentMessage {
            role_instance_id: "author-1".into(),
            text: "草稿 v1".into(),
            artifact_ref: Some(ArtifactRef {
                line: ArtifactLineKind::StorySpec,
                slot: DraftSlotKey(STORY_FULL_SLOT.into()),
                version: 1,
            }),
            cursor_after: 1,
        },
    ];
    progress_events.extend((2..=5).map(|version| RoomEvent::AgentMessage {
        role_instance_id: "author-1".into(),
        text: format!("草稿 v{version}"),
        artifact_ref: Some(ArtifactRef {
            line: ArtifactLineKind::StorySpec,
            slot: DraftSlotKey(STORY_FULL_SLOT.into()),
            version,
        }),
        cursor_after: version as u64,
    }));

    assert!(!Coordinator::stall_circuit_breaks(&progress_events, 4));
}

#[test]
fn 连续四条同一参与者且无草稿变化时触发空转熔断() {
    let mut stalled_events = vec![
        RoomEvent::UserMessage {
            text: "继续讨论".into(),
            mentions: vec![],
        },
        RoomEvent::AgentMessage {
            role_instance_id: "author-1".into(),
            text: "初始发言".into(),
            artifact_ref: None,
            cursor_after: 1,
        },
    ];
    stalled_events.extend((2..=5).map(|cursor_after| RoomEvent::AgentMessage {
        role_instance_id: "author-1".into(),
        text: "没有新增意见".into(),
        artifact_ref: None,
        cursor_after,
    }));

    assert!(Coordinator::stall_circuit_breaks(&stalled_events, 4));
}

#[tokio::test]
async fn request_deny_failed_记录_provider_error_held_且角色本轮停止() {
    let temp = tempfile::tempdir().expect("临时目录");
    let store = GroupChatStore::new(ProductAppPaths::new(temp.path()));
    store
        .save_session_snapshot(&session(vec![role(
            "author-1",
            GroupChatRoleKey::Author,
            "作者",
        )]))
        .expect("保存会话");
    let adapters = HashMap::from([(
        ProviderName::Fake,
        Arc::new(RequestDenyFailureProvider) as Arc<dyn StreamingProviderAdapter>,
    )]);
    let mut coordinator = Coordinator::new(store.clone(), adapters, Box::new(Script::ReviewerOnly))
        .with_config(config());

    let summary = coordinator
        .on_user_message(
            "project-1",
            "issue-1",
            "session-1",
            "请处理",
            vec!["author-1".into()],
        )
        .await
        .expect("协调完成");
    let events = store
        .load_events("project-1", "issue-1", "session-1")
        .expect("读取时间线");

    assert_eq!(summary.held_events, 1);
    assert!(events.iter().any(|event| {
        matches!(event, RoomEvent::HeldEvent { role_instance_id, reason, .. }
            if role_instance_id == "author-1" && reason == "provider_error")
    }));
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, RoomEvent::AgentMessage { .. }))
    );
}

#[tokio::test]
async fn 点名审稿人时绕过_triage_且只有审稿人发言() {
    let (_temp, store, mut coordinator) = fixture(
        vec![
            role("author-1", GroupChatRoleKey::Author, "作者"),
            role("reviewer-1", GroupChatRoleKey::Reviewer, "审稿人"),
        ],
        Script::ReviewerOnly,
    );

    let summary = coordinator
        .on_user_message(
            "project-1",
            "issue-1",
            "session-1",
            "@审稿人 请审查",
            vec!["reviewer-1".into()],
        )
        .await
        .expect("协调完成");
    let events = store
        .load_events("project-1", "issue-1", "session-1")
        .expect("读取时间线");

    assert!(summary.no_one_notice);
    assert_eq!(
        events
            .iter()
            .filter_map(|event| match event {
                RoomEvent::AgentMessage {
                    role_instance_id, ..
                } => Some(role_instance_id.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>(),
        vec!["reviewer-1"]
    );
}

#[tokio::test]
async fn 点名三个角色时所有被点名角色都发言() {
    let (_temp, store, mut coordinator) = fixture(
        vec![
            role("author-1", GroupChatRoleKey::Author, "作者"),
            role("reviewer-1", GroupChatRoleKey::Reviewer, "审稿人"),
            role("researcher-1", GroupChatRoleKey::Researcher, "研究员"),
        ],
        Script::ReviewerOnly,
    );

    let summary = coordinator
        .on_user_message(
            "project-1",
            "issue-1",
            "session-1",
            "请三位一起处理",
            vec![
                "author-1".into(),
                "reviewer-1".into(),
                "researcher-1".into(),
            ],
        )
        .await
        .expect("协调完成");
    let events = store
        .load_events("project-1", "issue-1", "session-1")
        .expect("读取时间线");

    assert!(summary.no_one_notice);
    let mut speakers = events
        .iter()
        .filter_map(|event| match event {
            RoomEvent::AgentMessage {
                role_instance_id, ..
            } => Some(role_instance_id.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    speakers.sort_unstable();
    assert_eq!(speakers, vec!["author-1", "researcher-1", "reviewer-1"]);
}
