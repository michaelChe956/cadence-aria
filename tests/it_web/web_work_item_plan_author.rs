use axum::http::Method;
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::lifecycle_store::LifecycleStore;
use cadence_aria::product::models::{WorkItemPlanStatus, WorkspaceType};
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
async fn work_item_plan_start_generation_returns_candidate_artifact() {
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

    let artifact_update = messages
        .iter()
        .find(|m| m["type"] == "artifact_update")
        .expect("artifact_update");
    assert!(artifact_update["candidate"]["work_items"].is_array());
    assert!(
        !artifact_update["candidate"]["work_items"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let _author_confirm_stage = messages
        .iter()
        .filter(|m| m["type"] == "stage_change")
        .find(|m| m["stage"] == "author_confirm")
        .expect("stage_change to author_confirm");

    ws.close(None).await.ok();
}

#[tokio::test]
async fn work_item_plan_author_persists_draft_candidate_records_without_child_sessions() {
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

    let artifact_update = messages
        .iter()
        .find(|m| m["type"] == "artifact_update")
        .expect("artifact_update");
    assert!(
        !artifact_update["candidate"]["work_items"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let lifecycle = LifecycleStore::new(ProductAppPaths::new(_repo.path().join(".aria")));
    let sessions = lifecycle
        .list_workspace_sessions("project_0001", "issue_0001")
        .unwrap();
    assert!(
        sessions
            .iter()
            .all(|s| s.workspace_type != WorkspaceType::WorkItem)
    );

    let work_items = lifecycle
        .list_work_items("project_0001", "issue_0001")
        .unwrap();
    assert!(!work_items.is_empty());
    assert!(
        work_items
            .iter()
            .all(|wi| wi.plan_status == WorkItemPlanStatus::Draft)
    );

    ws.close(None).await.ok();
}
