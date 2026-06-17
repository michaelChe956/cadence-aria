use axum::http::Method;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::time::{Duration, timeout};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

use crate::web_work_item_generation::{
    app_with_confirmed_story_and_design, request_json, valid_split_output,
};

static WS_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn connect_ws(
    app: axum::Router,
    session_id: &str,
) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/api/ws/workspace/{session_id}");
    let (ws, _) = connect_async(url).await.expect("connect ws");

    tokio::spawn(async move {
        server.await.ok();
    });

    ws
}

async fn recv_ws_messages_with_timeout(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    timeout_after: Duration,
    max_messages: usize,
) -> Vec<Value> {
    let mut messages = Vec::new();
    let deadline = tokio::time::Instant::now() + timeout_after;
    while messages.len() < max_messages && tokio::time::Instant::now() < deadline {
        let remaining = deadline - tokio::time::Instant::now();
        match timeout(remaining, ws.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => {
                messages.push(serde_json::from_str(&text).expect("ws json"));
            }
            Ok(Some(Ok(Message::Close(_)))) => break,
            Ok(Some(Ok(other))) => panic!("expected text ws message, got {other:?}"),
            Ok(Some(Err(error))) => panic!("ws error: {error}"),
            Ok(None) => break,
            Err(_) => break,
        }
    }
    messages
}

#[tokio::test]
async fn revert_work_item_is_valid_in_author_confirm_only() {
    let _guard = WS_TEST_LOCK.lock().await;
    let (app, _repo) = app_with_confirmed_story_and_design(valid_split_output()).await;

    let (_status, prepare_resp) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-item-plans:prepare",
        json!({
            "title": "登录拆分",
            "story_spec_ids": ["story_spec_0001"],
            "design_spec_ids": ["design_spec_0001"],
            "include_integration_tests": true,
            "include_e2e_tests": false,
            "force_frontend_backend_split": true,
            "require_execution_plan_confirm": false,
            "review_rounds": 1
        }),
    )
    .await;

    let session_id = prepare_resp["workspace_session"]["workspace_session_id"]
        .as_str()
        .unwrap()
        .to_string();

    let mut ws = connect_ws(app, &session_id).await;
    ws.send(Message::Text(
        json!({
            "type": "start_generation",
            "provider_config": { "author": "fake", "reviewer": null, "review_rounds": 1 },
            "reviewer_enabled": false
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send start_generation");

    let messages = recv_ws_messages_with_timeout(&mut ws, Duration::from_secs(10), 8).await;
    let initial_artifact = messages
        .iter()
        .find(|m| m["type"] == "artifact_update")
        .expect("initial artifact_update");
    let initial_version = initial_artifact["version"].as_u64().unwrap();

    ws.send(Message::Text(
        json!({
            "type": "revert_work_item",
            "work_item_id": "work_item_0001",
            "feedback": "拆得太粗",
            "clear": false
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send revert_work_item");

    let messages = recv_ws_messages_with_timeout(&mut ws, Duration::from_secs(5), 8).await;
    assert!(
        messages.iter().all(|m| m["type"] != "error"),
        "revert_work_item should not produce error messages"
    );

    let artifact = messages
        .iter()
        .find(|m| m["type"] == "artifact_update")
        .expect("artifact_update after revert");
    assert_eq!(
        artifact["version"].as_u64().unwrap(),
        initial_version,
        "revert mark must not create a new artifact version"
    );

    let wi = artifact["candidate"]["work_items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["id"] == "work_item_0001")
        .expect("work_item_0001 in candidate");
    assert_eq!(wi["meta"]["reverted"], true);
    assert_eq!(wi["meta"]["revert_feedback"], "拆得太粗");

    ws.close(None).await.ok();
}

#[tokio::test]
async fn revert_work_item_clear_removes_mark() {
    let _guard = WS_TEST_LOCK.lock().await;
    let (app, _repo) = app_with_confirmed_story_and_design(valid_split_output()).await;

    let (_status, prepare_resp) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-item-plans:prepare",
        json!({
            "title": "登录拆分",
            "story_spec_ids": ["story_spec_0001"],
            "design_spec_ids": ["design_spec_0001"],
            "include_integration_tests": true,
            "include_e2e_tests": false,
            "force_frontend_backend_split": true,
            "require_execution_plan_confirm": false,
            "review_rounds": 1
        }),
    )
    .await;

    let session_id = prepare_resp["workspace_session"]["workspace_session_id"]
        .as_str()
        .unwrap()
        .to_string();

    let mut ws = connect_ws(app, &session_id).await;
    ws.send(Message::Text(
        json!({
            "type": "start_generation",
            "provider_config": { "author": "fake", "reviewer": null, "review_rounds": 1 },
            "reviewer_enabled": false
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send start_generation");

    let messages = recv_ws_messages_with_timeout(&mut ws, Duration::from_secs(10), 8).await;
    let initial_artifact = messages
        .iter()
        .find(|m| m["type"] == "artifact_update")
        .expect("initial artifact_update");
    let initial_version = initial_artifact["version"].as_u64().unwrap();

    ws.send(Message::Text(
        json!({
            "type": "revert_work_item",
            "work_item_id": "work_item_0001",
            "feedback": "拆得太粗",
            "clear": false
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send revert_work_item");

    let messages = recv_ws_messages_with_timeout(&mut ws, Duration::from_secs(5), 8).await;
    let artifact = messages
        .iter()
        .find(|m| m["type"] == "artifact_update")
        .expect("artifact_update after revert");
    let wi = artifact["candidate"]["work_items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["id"] == "work_item_0001")
        .unwrap();
    assert_eq!(wi["meta"]["reverted"], true);
    assert_eq!(wi["meta"]["revert_feedback"], "拆得太粗");
    assert_eq!(artifact["version"].as_u64().unwrap(), initial_version);

    ws.send(Message::Text(
        json!({
            "type": "revert_work_item",
            "work_item_id": "work_item_0001",
            "feedback": null,
            "clear": true
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send revert_work_item clear");

    let messages = recv_ws_messages_with_timeout(&mut ws, Duration::from_secs(5), 8).await;
    assert!(
        messages.iter().all(|m| m["type"] != "error"),
        "clear revert should not produce error messages"
    );

    let artifact = messages
        .iter()
        .find(|m| m["type"] == "artifact_update")
        .expect("artifact_update after clear");
    assert_eq!(
        artifact["version"].as_u64().unwrap(),
        initial_version,
        "clear revert must not create a new artifact version"
    );

    let wi = artifact["candidate"]["work_items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["id"] == "work_item_0001")
        .expect("work_item_0001 in candidate");
    assert_eq!(wi["meta"]["reverted"], false);
    assert!(
        wi["meta"].get("revert_feedback").is_none(),
        "revert_feedback should be omitted after clear"
    );

    ws.close(None).await.ok();
}
