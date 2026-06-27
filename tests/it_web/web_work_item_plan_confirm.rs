use axum::http::{Method, StatusCode};
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::lifecycle_store::LifecycleStore;
use cadence_aria::product::models::{
    IssueWorkItemPlanStatus, WorkspaceSessionStatus, WorkspaceType,
};
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

type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn connect_ws(app: axum::Router, session_id: &str) -> WsStream {
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
    ws: &mut WsStream,
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

async fn recv_until_stage(ws: &mut WsStream, stage: &str, timeout_after: Duration) -> Vec<Value> {
    let mut messages = Vec::new();
    let deadline = tokio::time::Instant::now() + timeout_after;
    while tokio::time::Instant::now() < deadline {
        let remaining = deadline - tokio::time::Instant::now();
        match timeout(remaining, ws.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => {
                let value: Value = serde_json::from_str(&text).expect("ws json");
                messages.push(value);
                if messages
                    .iter()
                    .any(|m| m["type"] == "stage_change" && m["stage"] == stage)
                {
                    break;
                }
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

async fn prepare_and_start_generation(app: &axum::Router) -> (String, String, WsStream) {
    let (status, prepare_resp) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-item-plans:prepare",
        json!({
            "title": "登录拆分",
            "story_spec_ids": ["story_spec_0001"],
            "design_spec_ids": ["design_spec_0001"],
            "author_provider": "fake",
            "reviewer_provider": "codex",
            "review_rounds": 1,
            "superpowers_enabled": true,
            "openspec_enabled": true,
            "include_integration_tests": true,
            "include_e2e_tests": false,
            "force_frontend_backend_split": true,
            "require_execution_plan_confirm": false
        }),
    )
    .await;
    assert_eq!(
        status,
        axum::http::StatusCode::OK,
        "prepare failed: {prepare_resp}"
    );

    let session_id = prepare_resp["workspace_session"]["workspace_session_id"]
        .as_str()
        .unwrap()
        .to_string();
    let plan_id = prepare_resp["workspace_session"]["entity_id"]
        .as_str()
        .unwrap()
        .to_string();

    let mut ws = connect_ws(app.clone(), &session_id).await;
    ws.send(Message::Text(
        json!({
            "type": "start_generation",
            "provider_config": { "author": "fake", "reviewer": null, "review_rounds": 0 },
            "reviewer_enabled": false
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send start_generation");

    let _messages = recv_until_stage(&mut ws, "author_confirm", Duration::from_secs(10)).await;

    (session_id, plan_id, ws)
}

#[tokio::test]
#[ignore = "legacy full-candidate confirm flow is superseded by WP2 outline generation; WP6 final compile will replace this coverage"]
async fn confirm_creates_child_work_item_sessions() {
    let _guard = WS_TEST_LOCK.lock().await;
    let (app, repo) = app_with_confirmed_story_and_design(valid_split_output()).await;
    let (session_id, plan_id, mut ws) = prepare_and_start_generation(&app).await;

    ws.send(Message::Text(
        json!({"type": "author_decision", "decision": "accept"})
            .to_string()
            .into(),
    ))
    .await
    .expect("send author_decision accept");

    let _messages = recv_until_stage(&mut ws, "human_confirm", Duration::from_secs(10)).await;

    ws.send(Message::Text(
        json!({"type": "human_confirm", "decision": "confirm"})
            .to_string()
            .into(),
    ))
    .await
    .expect("send human_confirm confirm");

    let messages = recv_ws_messages_with_timeout(&mut ws, Duration::from_secs(10), 8).await;
    assert!(
        messages
            .iter()
            .any(|m| m["type"] == "stage_change" && m["stage"] == "completed"),
        "expected stage_change to completed, got {messages:?}"
    );

    let lifecycle = LifecycleStore::new(ProductAppPaths::new(repo.path().join(".aria")));
    let plan = lifecycle
        .get_issue_work_item_plan("project_0001", "issue_0001", &plan_id)
        .unwrap();
    assert_eq!(plan.status, IssueWorkItemPlanStatus::Confirmed);

    let work_items = lifecycle
        .list_work_items("project_0001", "issue_0001")
        .unwrap();
    let sessions = lifecycle
        .list_workspace_sessions("project_0001", "issue_0001")
        .unwrap();
    for wi in &work_items {
        let has_session = sessions
            .iter()
            .any(|s| s.workspace_type == WorkspaceType::WorkItem && s.entity_id == wi.id);
        assert!(
            has_session,
            "work_item {} should have a child WorkItem session",
            wi.id
        );
    }

    let parent_session = sessions
        .iter()
        .find(|s| s.id == session_id)
        .expect("parent session exists");
    assert_eq!(parent_session.status, WorkspaceSessionStatus::Confirmed);

    ws.close(None).await.ok();
}

#[tokio::test]
#[ignore = "legacy full-candidate confirm flow is superseded by WP2 outline generation; WP6 final compile will replace this coverage"]
async fn confirm_uses_session_entity_plan_id() {
    let _guard = WS_TEST_LOCK.lock().await;
    let (app, repo) = app_with_confirmed_story_and_design(valid_split_output()).await;
    let (session_id, plan_id, mut ws) = prepare_and_start_generation(&app).await;

    let lifecycle = LifecycleStore::new(ProductAppPaths::new(repo.path().join(".aria")));
    let session_record = lifecycle.get_workspace_session(&session_id).unwrap();
    let _ = session_id;
    assert_eq!(session_record.entity_id, plan_id);
    assert_eq!(session_record.workspace_type, WorkspaceType::WorkItemPlan);

    ws.send(Message::Text(
        json!({"type": "author_decision", "decision": "accept"})
            .to_string()
            .into(),
    ))
    .await
    .expect("send author_decision accept");

    let _messages = recv_until_stage(&mut ws, "human_confirm", Duration::from_secs(10)).await;

    ws.send(Message::Text(
        json!({"type": "human_confirm", "decision": "confirm"})
            .to_string()
            .into(),
    ))
    .await
    .expect("send human_confirm confirm");

    let messages = recv_ws_messages_with_timeout(&mut ws, Duration::from_secs(10), 8).await;
    assert!(
        messages
            .iter()
            .any(|m| m["type"] == "stage_change" && m["stage"] == "completed")
    );

    let plan = lifecycle
        .get_issue_work_item_plan("project_0001", "issue_0001", &plan_id)
        .unwrap();
    assert_eq!(plan.status, IssueWorkItemPlanStatus::Confirmed);

    ws.close(None).await.ok();
}

#[tokio::test]
#[ignore = "legacy full-candidate confirm flow is superseded by WP2 outline generation; WP6 final compile will replace this coverage"]
async fn confirm_is_idempotent_on_retry() {
    let _guard = WS_TEST_LOCK.lock().await;
    let (app, repo) = app_with_confirmed_story_and_design(valid_split_output()).await;
    let (_session_id, plan_id, mut ws) = prepare_and_start_generation(&app).await;

    ws.send(Message::Text(
        json!({"type": "author_decision", "decision": "accept"})
            .to_string()
            .into(),
    ))
    .await
    .expect("send author_decision accept");

    let _messages = recv_until_stage(&mut ws, "human_confirm", Duration::from_secs(10)).await;

    ws.send(Message::Text(
        json!({"type": "human_confirm", "decision": "confirm"})
            .to_string()
            .into(),
    ))
    .await
    .expect("send human_confirm confirm first time");

    let messages = recv_ws_messages_with_timeout(&mut ws, Duration::from_secs(10), 8).await;
    assert!(
        messages
            .iter()
            .any(|m| m["type"] == "stage_change" && m["stage"] == "completed")
    );

    let lifecycle = LifecycleStore::new(ProductAppPaths::new(repo.path().join(".aria")));
    let sessions_after_first = lifecycle
        .list_workspace_sessions("project_0001", "issue_0001")
        .unwrap();
    let child_count_after_first = sessions_after_first
        .iter()
        .filter(|s| s.workspace_type == WorkspaceType::WorkItem)
        .count();

    ws.send(Message::Text(
        json!({"type": "human_confirm", "decision": "confirm"})
            .to_string()
            .into(),
    ))
    .await
    .expect("send human_confirm confirm second time");

    let retry_messages = recv_ws_messages_with_timeout(&mut ws, Duration::from_secs(5), 4).await;
    assert!(
        retry_messages.iter().any(|m| m["type"] == "protocol_error"),
        "second confirm should return protocol_error, got {retry_messages:?}"
    );

    let sessions_after_second = lifecycle
        .list_workspace_sessions("project_0001", "issue_0001")
        .unwrap();
    let child_count_after_second = sessions_after_second
        .iter()
        .filter(|s| s.workspace_type == WorkspaceType::WorkItem)
        .count();
    assert_eq!(
        child_count_after_first, child_count_after_second,
        "retry should not create additional child sessions"
    );

    let plan = lifecycle
        .get_issue_work_item_plan("project_0001", "issue_0001", &plan_id)
        .unwrap();
    assert_eq!(plan.status, IssueWorkItemPlanStatus::Confirmed);

    ws.close(None).await.ok();
}

#[tokio::test]
async fn delete_legacy_rest_routes_returns_404() {
    let (app, _repo) = app_with_confirmed_story_and_design(valid_split_output()).await;

    let (status, _) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items:generate",
        json!({
            "title": "x",
            "story_spec_ids": ["story_spec_0001"],
            "design_spec_ids": ["design_spec_0001"]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, _) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-item-plans/some_plan/confirm",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, _) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-item-plans/some_plan/change-request",
        json!({"feedback": "x"}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
