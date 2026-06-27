use axum::http::Method;
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
    app_with_confirmed_story_and_design_revision_and_test_providers, request_json,
    valid_revision_redo_output, valid_split_output,
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

async fn recv_ws_until<F>(ws: &mut WsStream, timeout_after: Duration, predicate: F) -> Vec<Value>
where
    F: Fn(&[Value]) -> bool,
{
    let mut messages = Vec::new();
    let deadline = tokio::time::Instant::now() + timeout_after;
    while tokio::time::Instant::now() < deadline {
        let remaining = deadline - tokio::time::Instant::now();
        match timeout(remaining, ws.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => match serde_json::from_str(&text) {
                Ok(value) => {
                    messages.push(value);
                    if predicate(&messages) {
                        break;
                    }
                }
                Err(error) => panic!("ws json parse error: {error}\n{text}"),
            },
            Ok(Some(Ok(Message::Close(_)))) => break,
            Ok(Some(Ok(other))) => {
                eprintln!("ignoring non-text ws message: {other:?}");
                continue;
            }
            Ok(Some(Err(error))) => panic!("ws error: {error}"),
            Ok(None) => break,
            Err(_) => break,
        }
    }
    messages
}

async fn prepare_and_start_generation(
    app: &axum::Router,
) -> (String, String, WsStream, Vec<Value>) {
    let (status, prepare_resp) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-item-plans:prepare",
        json!({
            "title": "登录拆分 Work Item Plan",
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
            "provider_config": { "author": "fake", "reviewer": "codex", "review_rounds": 1 },
            "reviewer_enabled": true
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send start_generation");

    let messages = recv_ws_until(&mut ws, Duration::from_secs(15), |msgs| {
        msgs.iter().any(|m| m["type"] == "artifact_update")
            && msgs
                .iter()
                .any(|m| m["type"] == "stage_change" && m["stage"] == "author_confirm")
    })
    .await;

    (session_id, plan_id, ws, messages)
}

#[tokio::test]
#[ignore = "legacy full-candidate WorkItemPlan flow is superseded by WP2 outline generation; WP8 will replace this end-to-end coverage"]
async fn work_item_plan_full_flow() {
    let _guard = WS_TEST_LOCK.lock().await;
    let (app, repo) = app_with_confirmed_story_and_design_revision_and_test_providers(
        valid_split_output(),
        valid_revision_redo_output(),
    )
    .await;
    let (session_id, plan_id, mut ws, prepare_messages) = prepare_and_start_generation(&app).await;

    // 1. 初始 candidate
    let initial_artifact = prepare_messages
        .iter()
        .find(|m| m["type"] == "artifact_update")
        .expect("initial artifact_update");
    let initial_version = initial_artifact["version"].as_u64().unwrap();
    let initial_count = initial_artifact["candidate"]["work_items"]
        .as_array()
        .unwrap()
        .len();
    let first_work_item_id = initial_artifact["candidate"]["work_items"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();

    // 2. revert 第一个 work_item
    ws.send(Message::Text(
        json!({
            "type": "revert_work_item",
            "work_item_id": first_work_item_id,
            "feedback": "拆得太粗",
            "clear": false
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send revert_work_item");

    let revert_messages = recv_ws_until(&mut ws, Duration::from_secs(5), |msgs| {
        msgs.iter().any(|m| {
            m["type"] == "artifact_update" && m["version"].as_u64() == Some(initial_version)
        })
    })
    .await;
    let revert_artifact = revert_messages
        .iter()
        .find(|m| m["type"] == "artifact_update")
        .expect("artifact_update after revert");
    assert_eq!(
        revert_artifact["version"].as_u64().unwrap(),
        initial_version
    );
    let reverted_item = revert_artifact["candidate"]["work_items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["id"] == first_work_item_id)
        .expect("reverted work item still in candidate");
    assert_eq!(reverted_item["meta"]["reverted"], true);
    assert_eq!(reverted_item["meta"]["revert_feedback"], "拆得太粗");

    // 3. request_revision：重做被标记的项
    ws.send(Message::Text(
        json!({
            "type": "request_revision",
            "feedback": {
                "feedback_types": ["other"],
                "description": "重做被标记的",
                "target_artifact_version": null
            }
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send request_revision");

    let revision_messages = recv_ws_until(&mut ws, Duration::from_secs(15), |msgs| {
        msgs.iter()
            .any(|m| m["type"] == "stage_change" && m["stage"] == "author_confirm")
    })
    .await;
    assert!(
        revision_messages.iter().all(|m| m["type"] != "error"),
        "request_revision should not produce error messages: {:?}",
        revision_messages
            .iter()
            .filter(|m| m["type"] == "error")
            .collect::<Vec<_>>()
    );
    let revision_artifact = revision_messages
        .iter()
        .rfind(|m| m["type"] == "artifact_update")
        .expect("artifact_update after revision");
    assert!(
        revision_artifact["version"].as_u64().unwrap() > initial_version,
        "revision must produce a new artifact version"
    );
    let revision_work_items = revision_artifact["candidate"]["work_items"]
        .as_array()
        .unwrap();
    assert_eq!(
        revision_work_items.len(),
        initial_count,
        "revision must keep the same total work item count"
    );
    assert!(
        !revision_work_items
            .iter()
            .any(|w| w["id"] == first_work_item_id),
        "reverted work item must be replaced with a new id"
    );

    // 4. author_decision accept -> review（review_rounds=1，使用 pass fixture）
    let (status, _response) = request_json(
        app.clone(),
        Method::POST,
        &format!("/api/test/workspace-sessions/{session_id}/review-fixture"),
        json!({
            "verdict": "pass",
            "summary": "审核通过",
            "comments": "覆盖核心路径",
            "findings": []
        }),
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::OK);

    ws.send(Message::Text(
        json!({ "type": "author_decision", "decision": "accept" })
            .to_string()
            .into(),
    ))
    .await
    .expect("send author_decision accept");

    let human_confirm_messages = recv_ws_until(&mut ws, Duration::from_secs(15), |msgs| {
        msgs.iter()
            .any(|m| m["type"] == "stage_change" && m["stage"] == "human_confirm")
    })
    .await;
    assert!(
        human_confirm_messages
            .iter()
            .any(|m| m["type"] == "stage_change" && m["stage"] == "human_confirm"),
        "expected stage_change to human_confirm, got {human_confirm_messages:?}"
    );

    // 7. human_confirm confirm -> completed
    ws.send(Message::Text(
        json!({ "type": "human_confirm", "decision": "confirm" })
            .to_string()
            .into(),
    ))
    .await
    .expect("send human_confirm confirm");

    let completed_messages = recv_ws_until(&mut ws, Duration::from_secs(15), |msgs| {
        msgs.iter()
            .any(|m| m["type"] == "stage_change" && m["stage"] == "completed")
    })
    .await;
    assert!(
        completed_messages
            .iter()
            .any(|m| m["type"] == "stage_change" && m["stage"] == "completed"),
        "expected stage_change to completed, got {completed_messages:?}"
    );

    // 8. 持久化状态校验
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
