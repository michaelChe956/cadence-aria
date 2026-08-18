use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::cross_cutting::streaming_provider::{
    ChoiceRequestData, ChoiceRequestSource, ProviderCommand, ProviderCompletion, ProviderEvent,
    ProviderPermissionMode, ProviderSession, RiskLevel, ScriptedFakeProvider, ScriptedReply,
    StreamingProviderAdapter, StreamingProviderInput,
};
use crate::product::group_chat_engine::agent_turn::{
    AgentTurnFinalStatus, AgentTurnRuntime, HoldRetryPolicy, SleepFuture, run_agent_turn,
};
use crate::product::group_chat_engine::context::TurnContext;
use crate::product::group_chat_engine::types::{GroupChatRoleKey, RoleInstance, RoomEvent};
use crate::product::models::ProviderName;

fn role() -> RoleInstance {
    RoleInstance {
        id: "author-1".into(),
        role_key: GroupChatRoleKey::Author,
        provider: ProviderName::Fake,
        display_name: "作者".into(),
        permission_mode: ProviderPermissionMode::Auto,
        seen_cursor: 0,
        injection_watermark: 1,
    }
}

fn context() -> TurnContext {
    TurnContext {
        unread_events: vec!["请给出初稿".into()],
        ..TurnContext::default()
    }
}

fn rebuild_context(_: &[RoomEvent], _: &mut RoleInstance) -> TurnContext {
    context()
}

fn immediate_sleep(_: Duration) -> SleepFuture {
    Box::pin(async {})
}

fn completed(text: &str) -> Vec<ProviderEvent> {
    vec![
        ProviderEvent::TextDelta {
            content: text.into(),
        },
        ProviderEvent::Completed(ProviderCompletion::plain(text, None)),
    ]
}

fn scripted_provider(replies: Vec<(&str, &str)>) -> ScriptedFakeProvider {
    ScriptedFakeProvider::new(
        replies
            .into_iter()
            .map(|(marker, text)| ScriptedReply {
                match_prompt_contains: marker.into(),
                events: completed(text),
            })
            .collect(),
    )
}

struct InteractiveRequestProvider {
    received_commands: Arc<Mutex<Vec<ProviderCommand>>>,
}

impl InteractiveRequestProvider {
    fn new() -> Self {
        Self {
            received_commands: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for InteractiveRequestProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, mut command_rx) = mpsc::channel(8);
        let received_commands = self.received_commands.clone();

        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::PermissionRequest(
                    crate::cross_cutting::streaming_provider::PermissionRequestData {
                        id: "permission-1".into(),
                        tool_name: "write_file".into(),
                        description: "写入草稿".into(),
                        risk_level: RiskLevel::Medium,
                    },
                ))
                .await;
            if let Some(command) = command_rx.recv().await {
                received_commands
                    .lock()
                    .expect("received command lock")
                    .push(command);
            }

            let _ = event_tx
                .send(ProviderEvent::ChoiceRequest(ChoiceRequestData {
                    id: "choice-1".into(),
                    prompt: "请选择写入范围".into(),
                    options: vec![],
                    allow_multiple: false,
                    allow_free_text: false,
                    questions: vec![],
                    source: ChoiceRequestSource::ProviderChoice,
                }))
                .await;
            if let Some(command) = command_rx.recv().await {
                received_commands
                    .lock()
                    .expect("received command lock")
                    .push(command);
            }

            let _ = event_tx
                .send(ProviderEvent::Completed(ProviderCompletion::plain(
                    "请求已拒绝后继续完成发言",
                    None,
                )))
                .await;
        });

        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }
}

#[tokio::test]
async fn 正常产出_agent_message_并推进角色_cursor() {
    let provider = scripted_provider(vec![("群聊重试轮次：0", "这是作者的初稿")]);
    let timeline = Arc::new(Mutex::new(vec![RoomEvent::UserMessage {
        text: "请给出初稿".into(),
        mentions: vec!["author-1".into()],
    }]));
    let mut author = role();

    let mut read_events = {
        let timeline = timeline.clone();
        move || timeline.lock().expect("timeline lock").clone()
    };
    let mut publish_event = {
        let timeline = timeline.clone();
        move |event| timeline.lock().expect("timeline lock").push(event)
    };
    let mut rebuild_context = rebuild_context;
    let result = run_agent_turn(
        &mut author,
        context(),
        &provider,
        AgentTurnRuntime {
            events_len_at_start: 1,
            read_events: &mut read_events,
            publish_event: &mut publish_event,
            rebuild_context: &mut rebuild_context,
            retry_policy: HoldRetryPolicy::without_delay(),
            sleep: &mut immediate_sleep,
        },
    )
    .await
    .expect("agent turn must complete");

    assert_eq!(result.status, AgentTurnFinalStatus::Published);
    assert_eq!(result.provider_attempts, 1);
    assert_eq!(author.seen_cursor, 1);
    // 发布路径只更新 seen_cursor；注入水位由 assemble_turn_context 保守推进。
    assert_eq!(author.injection_watermark, 1);
    assert_eq!(
        result.events,
        vec![RoomEvent::AgentMessage {
            role_instance_id: "author-1".into(),
            text: "这是作者的初稿".into(),
            artifact_ref: None,
            cursor_after: 1,
        }]
    );
}

#[tokio::test]
async fn 发布路径不会覆盖组装阶段保守保留的注入水位() {
    let provider = scripted_provider(vec![("群聊重试轮次：0", "这是截断后发布的回复")]);
    let timeline = Arc::new(Mutex::new(vec![RoomEvent::UserMessage {
        text: "请继续处理".into(),
        mentions: vec![],
    }]));
    let mut author = role();
    author.injection_watermark = 0;

    let mut read_events = {
        let timeline = timeline.clone();
        move || timeline.lock().expect("timeline lock").clone()
    };
    let mut publish_event = {
        let timeline = timeline.clone();
        move |event| timeline.lock().expect("timeline lock").push(event)
    };
    let mut rebuild_context = rebuild_context;
    run_agent_turn(
        &mut author,
        context(),
        &provider,
        AgentTurnRuntime {
            events_len_at_start: 1,
            read_events: &mut read_events,
            publish_event: &mut publish_event,
            rebuild_context: &mut rebuild_context,
            retry_policy: HoldRetryPolicy::without_delay(),
            sleep: &mut immediate_sleep,
        },
    )
    .await
    .expect("agent turn must complete");

    assert_eq!(author.seen_cursor, 1);
    assert_eq!(author.injection_watermark, 0);
}

#[tokio::test]
async fn 快照后出现新事件时写入_hold_并带新上下文重试() {
    let provider = scripted_provider(vec![
        ("群聊重试轮次：0", "过期回复"),
        ("群聊重试轮次：1", "读取新消息后的回复"),
    ]);
    let timeline = Arc::new(Mutex::new(vec![RoomEvent::UserMessage {
        text: "请给出初稿".into(),
        mentions: vec![],
    }]));
    let read_count = Arc::new(Mutex::new(0usize));
    let mut author = role();

    let mut read_events = {
        let timeline = timeline.clone();
        let read_count = read_count.clone();
        move || {
            let mut count = read_count.lock().expect("read count lock");
            *count += 1;
            if *count == 1 {
                timeline
                    .lock()
                    .expect("timeline lock")
                    .push(RoomEvent::UserMessage {
                        text: "请补充验收条件".into(),
                        mentions: vec![],
                    });
            }
            timeline.lock().expect("timeline lock").clone()
        }
    };
    let mut publish_event = {
        let timeline = timeline.clone();
        move |event| timeline.lock().expect("timeline lock").push(event)
    };
    let mut rebuild_context = rebuild_context;
    let result = run_agent_turn(
        &mut author,
        context(),
        &provider,
        AgentTurnRuntime {
            events_len_at_start: 1,
            read_events: &mut read_events,
            publish_event: &mut publish_event,
            rebuild_context: &mut rebuild_context,
            retry_policy: HoldRetryPolicy::without_delay(),
            sleep: &mut immediate_sleep,
        },
    )
    .await
    .expect("agent turn must retry after freshness hold");

    assert_eq!(result.status, AgentTurnFinalStatus::Published);
    assert_eq!(result.provider_attempts, 2);
    assert!(matches!(
        result.events.first(),
        Some(RoomEvent::HeldEvent { reason, cursor_after, .. })
            if reason == "freshness" && *cursor_after == 2
    ));
    assert!(matches!(
        result.events.last(),
        Some(RoomEvent::AgentMessage { text, cursor_after, .. })
            if text == "读取新消息后的回复" && *cursor_after == 3
    ));
    assert_eq!(author.seen_cursor, 3);
}

#[tokio::test]
async fn 与最近他人消息逐字相同的产出必须_hold_且不可绕过() {
    let provider = scripted_provider(vec![("群聊 Agent Turn", "完全相同")]);
    let timeline = Arc::new(Mutex::new(vec![
        RoomEvent::UserMessage {
            text: "请给出意见".into(),
            mentions: vec![],
        },
        RoomEvent::AgentMessage {
            role_instance_id: "reviewer-1".into(),
            text: "完全相同".into(),
            artifact_ref: None,
            cursor_after: 1,
        },
    ]));
    let mut author = role();

    let mut read_events = {
        let timeline = timeline.clone();
        move || timeline.lock().expect("timeline lock").clone()
    };
    let mut publish_event = {
        let timeline = timeline.clone();
        move |event| timeline.lock().expect("timeline lock").push(event)
    };
    let mut rebuild_context = rebuild_context;
    let result = run_agent_turn(
        &mut author,
        context(),
        &provider,
        AgentTurnRuntime {
            events_len_at_start: 2,
            read_events: &mut read_events,
            publish_event: &mut publish_event,
            rebuild_context: &mut rebuild_context,
            retry_policy: HoldRetryPolicy::without_delay(),
            sleep: &mut immediate_sleep,
        },
    )
    .await
    .expect("duplicate responses must be held rather than published");

    assert_eq!(result.status, AgentTurnFinalStatus::RetryExhausted);
    assert_eq!(result.provider_attempts, 4);
    assert!(
        !result
            .events
            .iter()
            .any(|event| matches!(event, RoomEvent::AgentMessage { .. }))
    );
    assert!(matches!(
        result.events.first(),
        Some(RoomEvent::HeldEvent { reason, .. }) if reason == "verbatim_duplicate"
    ));
    assert!(matches!(
        result.events.last(),
        Some(RoomEvent::HeldEvent { reason, .. }) if reason == "retry_exhausted"
    ));
}

#[tokio::test]
async fn 带首尾空白的逐字重复产出仍必须_hold() {
    let provider = scripted_provider(vec![("群聊 Agent Turn", "  完全相同\n")]);
    let timeline = Arc::new(Mutex::new(vec![RoomEvent::AgentMessage {
        role_instance_id: "reviewer-1".into(),
        text: "完全相同".into(),
        artifact_ref: None,
        cursor_after: 0,
    }]));
    let mut author = role();
    let mut read_events = {
        let timeline = timeline.clone();
        move || timeline.lock().expect("timeline lock").clone()
    };
    let mut publish_event = {
        let timeline = timeline.clone();
        move |event| timeline.lock().expect("timeline lock").push(event)
    };
    let mut rebuild_context = rebuild_context;

    let result = run_agent_turn(
        &mut author,
        context(),
        &provider,
        AgentTurnRuntime {
            events_len_at_start: 1,
            read_events: &mut read_events,
            publish_event: &mut publish_event,
            rebuild_context: &mut rebuild_context,
            retry_policy: HoldRetryPolicy::without_delay(),
            sleep: &mut immediate_sleep,
        },
    )
    .await
    .expect("trimmed duplicate must be held");

    assert_eq!(result.status, AgentTurnFinalStatus::RetryExhausted);
    assert_eq!(result.denied_requests, 0);
    assert!(matches!(
        result.events.first(),
        Some(RoomEvent::HeldEvent { reason, .. }) if reason == "verbatim_duplicate"
    ));
}

#[tokio::test]
async fn 自动拒绝权限与选择请求并在结果中记录计数() {
    let provider = InteractiveRequestProvider::new();
    let received_commands = provider.received_commands.clone();
    let timeline = Arc::new(Mutex::new(Vec::<RoomEvent>::new()));
    let mut author = role();
    let mut read_events = {
        let timeline = timeline.clone();
        move || timeline.lock().expect("timeline lock").clone()
    };
    let mut publish_event = {
        let timeline = timeline.clone();
        move |event| timeline.lock().expect("timeline lock").push(event)
    };
    let mut rebuild_context = rebuild_context;

    let result = run_agent_turn(
        &mut author,
        context(),
        &provider,
        AgentTurnRuntime {
            events_len_at_start: 0,
            read_events: &mut read_events,
            publish_event: &mut publish_event,
            rebuild_context: &mut rebuild_context,
            retry_policy: HoldRetryPolicy::without_delay(),
            sleep: &mut immediate_sleep,
        },
    )
    .await
    .expect("rejected interactive requests must allow completion");

    assert_eq!(result.status, AgentTurnFinalStatus::Published);
    assert_eq!(result.denied_requests, 2);
    assert_eq!(
        *received_commands.lock().expect("received command lock"),
        vec![
            ProviderCommand::PermissionResponse {
                id: "permission-1".into(),
                approved: false,
                reason: Some("群聊 Agent Turn 暂不支持人工审批，已自动拒绝".into()),
            },
            ProviderCommand::ChoiceResponse {
                id: "choice-1".into(),
                selected_option_ids: vec![],
                free_text: Some("群聊 Agent Turn 暂不支持人工审批，已自动拒绝".into()),
                answers: vec![],
            },
        ]
    );
}

#[tokio::test]
async fn freshness_hold_连续三次重试后以_retry_exhausted_结束() {
    let provider = scripted_provider(vec![("群聊 Agent Turn", "每次都会过期")]);
    let timeline = Arc::new(Mutex::new(Vec::<RoomEvent>::new()));
    let read_count = Arc::new(Mutex::new(0usize));
    let mut author = role();

    let mut read_events = {
        let timeline = timeline.clone();
        let read_count = read_count.clone();
        move || {
            let mut count = read_count.lock().expect("read count lock");
            *count += 1;
            timeline
                .lock()
                .expect("timeline lock")
                .push(RoomEvent::SystemNotice {
                    text: format!("并发事件-{count}"),
                });
            timeline.lock().expect("timeline lock").clone()
        }
    };
    let mut publish_event = {
        let timeline = timeline.clone();
        move |event| timeline.lock().expect("timeline lock").push(event)
    };
    let slept = Arc::new(Mutex::new(Vec::new()));
    let mut sleep = {
        let slept = slept.clone();
        move |duration| -> SleepFuture {
            slept.lock().expect("sleep log lock").push(duration);
            Box::pin(async {})
        }
    };
    let mut rebuild_context = rebuild_context;
    let result = run_agent_turn(
        &mut author,
        context(),
        &provider,
        AgentTurnRuntime {
            events_len_at_start: 0,
            read_events: &mut read_events,
            publish_event: &mut publish_event,
            rebuild_context: &mut rebuild_context,
            retry_policy: HoldRetryPolicy::default(),
            sleep: &mut sleep,
        },
    )
    .await
    .expect("stale outputs must exhaust retries deterministically");

    assert_eq!(result.status, AgentTurnFinalStatus::RetryExhausted);
    assert_eq!(result.provider_attempts, 4);
    assert_eq!(
        result
            .events
            .iter()
            .filter(|event| matches!(event, RoomEvent::HeldEvent { .. }))
            .count(),
        4
    );
    assert!(matches!(
        result.events.last(),
        Some(RoomEvent::HeldEvent { reason, .. }) if reason == "retry_exhausted"
    ));
    assert_eq!(
        *slept.lock().expect("sleep log lock"),
        vec![
            Duration::from_secs(1),
            Duration::from_secs(2),
            Duration::from_secs(4),
        ]
    );
}
