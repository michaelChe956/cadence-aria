use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use tempfile::tempdir;
use tokio::net::TcpListener;
use tokio::time::{Duration, timeout};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message as TungsteniteMessage;

use crate::product::app_paths::ProductAppPaths;
use crate::product::group_chat_engine::types::RoomEvent;
use crate::product::issue_store::{CreateProductIssueInput, IssueStore};
use crate::product::project_store::{CreateProjectInput, ProjectStore};
use crate::web::app::build_web_router;
use crate::web::group_chat_ws_types::{GroupChatWsInMessage, GroupChatWsOutMessage};
use crate::web::runtime::WebRuntime;
use crate::web::state::WebAppState;

#[test]
fn protocol_messages_use_snake_case_tags() {
    let input: GroupChatWsInMessage = serde_json::from_value(serde_json::json!({
        "type": "ping"
    }))
    .expect("ping input");
    assert_eq!(input, GroupChatWsInMessage::Ping);

    let output = serde_json::to_value(GroupChatWsOutMessage::Pong).expect("pong output");
    assert_eq!(output["type"], "pong");

    let input: GroupChatWsInMessage = serde_json::from_value(serde_json::json!({
        "type": "send_message",
        "text": "请起草",
        "mentions": ["role_1"],
        "draft_slot": "story_full"
    }))
    .expect("send message input");
    assert!(matches!(input, GroupChatWsInMessage::SendMessage { .. }));

    let output = serde_json::to_value(GroupChatWsOutMessage::RoomEvent {
        seq: 7,
        event: RoomEvent::SystemNotice {
            text: "ready".to_owned(),
        },
    })
    .expect("room event output");
    assert_eq!(output["type"], "room_event");
    assert_eq!(output["seq"], 7);
    assert_eq!(output["event"]["type"], "system_notice");
}

#[tokio::test]
async fn websocket_send_message_starts_turn_then_publishes_user_event() {
    let root = tempdir().expect("temp root");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let project = ProjectStore::new(paths.clone())
        .create(CreateProjectInput {
            name: "WS send test".to_owned(),
            description: None,
        })
        .expect("project");
    let issue = IssueStore::new(paths.clone())
        .create(CreateProductIssueInput {
            project_id: project.id.clone(),
            repo_id: None,
            title: "WS send test issue".to_owned(),
            description: None,
            change_id: None,
        })
        .expect("issue");
    let state = WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    );
    let engine = Arc::clone(state.group_chat_engine.as_ref().expect("group chat engine"));
    let (session, _) = engine
        .create_or_get_session(&project.id, &issue.id)
        .expect("group chat session");
    let app = build_web_router(state);
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let address = listener.local_addr().expect("local address");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    let (mut socket, _) = connect_async(format!("ws://{address}/ws/group-chat/{}", session.id))
        .await
        .expect("connect");
    socket
        .send(TungsteniteMessage::Text(
            serde_json::to_string(&GroupChatWsInMessage::SendMessage {
                text: "开始讨论".to_owned(),
                mentions: vec!["role_1".to_owned()],
                draft_slot: None,
            })
            .expect("send json")
            .into(),
        ))
        .await
        .expect("send message");
    let started = timeout(Duration::from_secs(3), socket.next())
        .await
        .expect("turn started timeout")
        .expect("turn started frame")
        .expect("turn started websocket frame");
    let started: GroupChatWsOutMessage = match started {
        TungsteniteMessage::Text(text) => serde_json::from_str(&text).expect("started json"),
        other => panic!("unexpected frame: {other:?}"),
    };
    assert_eq!(
        started,
        GroupChatWsOutMessage::TurnStarted {
            role_instance_id: "role_1".to_owned()
        }
    );
    let room_events = timeout(Duration::from_secs(30), async {
        let mut events = Vec::new();
        let expected_count = loop {
            let frame = socket
                .next()
                .await
                .expect("event frame")
                .expect("event websocket");
            let TungsteniteMessage::Text(text) = frame else {
                continue;
            };
            let event: GroupChatWsOutMessage = serde_json::from_str(&text).expect("event json");
            let GroupChatWsOutMessage::RoomEvent { seq, event } = event else {
                continue;
            };
            events.push((seq, event));
            if events.len() == 1 {
                assert!(matches!(events[0].1, RoomEvent::UserMessage { .. }));
                let expected = engine
                    .store
                    .load_event_entries(&project.id, &issue.id, &session.id)
                    .expect("load completed timeline")
                    .len();
                assert!(expected >= 2, "fake provider should produce an agent event");
                break expected;
            }
        };
        while events.len() < expected_count {
            let frame = socket
                .next()
                .await
                .expect("remaining event frame")
                .expect("remaining event websocket");
            let TungsteniteMessage::Text(text) = frame else {
                continue;
            };
            let GroupChatWsOutMessage::RoomEvent { seq, event } =
                serde_json::from_str(&text).expect("remaining event json")
            else {
                continue;
            };
            events.push((seq, event));
        }
        events
    })
    .await
    .expect("room event sequence timeout");
    assert!(
        room_events
            .windows(2)
            .all(|events| events[0].0 < events[1].0)
    );
    assert!(matches!(room_events[0].1, RoomEvent::UserMessage { .. }));
    let agent_index = room_events
        .iter()
        .position(|(_, event)| matches!(event, RoomEvent::AgentMessage { .. }))
        .expect("fake provider agent event");
    assert!(agent_index > 0, "agent event must follow user event");
    socket.close(None).await.expect("close socket");
    server.abort();
}

#[tokio::test]
async fn websocket_replays_events_after_client_cursor() {
    let root = tempdir().expect("temp root");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let project = ProjectStore::new(paths.clone())
        .create(CreateProjectInput {
            name: "WS test".to_owned(),
            description: None,
        })
        .expect("project");
    let issue = IssueStore::new(paths.clone())
        .create(CreateProductIssueInput {
            project_id: project.id.clone(),
            repo_id: None,
            title: "WS test issue".to_owned(),
            description: None,
            change_id: None,
        })
        .expect("issue");
    let state = WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    );
    let engine = Arc::clone(state.group_chat_engine.as_ref().expect("group chat engine"));
    let (session, _) = engine
        .create_or_get_session(&project.id, &issue.id)
        .expect("group chat session");
    let seq = engine
        .store
        .append_event(
            &project.id,
            &issue.id,
            &session.id,
            RoomEvent::UserMessage {
                text: "reconnect me".to_owned(),
                mentions: vec![],
            },
        )
        .expect("timeline event");

    let app = build_web_router(state);
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let address = listener.local_addr().expect("local address");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{address}/ws/group-chat/{}", session.id);
    let (mut socket, _) = connect_async(&url).await.expect("connect");
    let first = timeout(Duration::from_secs(2), socket.next())
        .await
        .expect("first event timeout")
        .expect("first event")
        .expect("first websocket frame");
    let first: GroupChatWsOutMessage = match first {
        TungsteniteMessage::Text(text) => serde_json::from_str(&text).expect("room event json"),
        other => panic!("unexpected frame: {other:?}"),
    };
    assert!(matches!(
        first,
        GroupChatWsOutMessage::RoomEvent {
            seq: event_seq,
            event: RoomEvent::UserMessage { .. }
        } if event_seq == seq
    ));
    socket.close(None).await.expect("close first socket");

    let reconnect_url = format!("{url}?after_seq={seq}");
    let (mut reconnected, _) = connect_async(reconnect_url).await.expect("reconnect");
    reconnected
        .send(TungsteniteMessage::Text(
            serde_json::to_string(&GroupChatWsInMessage::Ping)
                .expect("ping json")
                .into(),
        ))
        .await
        .expect("send ping");
    let pong = timeout(Duration::from_secs(2), reconnected.next())
        .await
        .expect("pong timeout")
        .expect("pong frame")
        .expect("pong websocket frame");
    let pong: GroupChatWsOutMessage = match pong {
        TungsteniteMessage::Text(text) => serde_json::from_str(&text).expect("pong json"),
        other => panic!("unexpected pong frame: {other:?}"),
    };
    assert_eq!(pong, GroupChatWsOutMessage::Pong);
    reconnected.close(None).await.expect("close reconnect");
    server.abort();
}
