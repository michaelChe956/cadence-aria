use std::collections::HashSet;

use axum::http::{Method, StatusCode};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::time::{Duration, timeout};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

use crate::web_work_item_generation::{
    app_with_confirmed_story_and_design, app_with_confirmed_story_and_design_and_revision_output,
    invalid_split_output_missing_e2e, request_json, valid_revision_redo_output, valid_split_output,
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

async fn prepare_and_start_generation(app: &axum::Router) -> String {
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

    prepare_resp["workspace_session"]["workspace_session_id"]
        .as_str()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn revert_work_item_triggers_local_redo_in_revision() {
    let _guard = WS_TEST_LOCK.lock().await;
    let (app, _repo) = app_with_confirmed_story_and_design_and_revision_output(
        valid_split_output(),
        valid_revision_redo_output(),
    )
    .await;
    let session_id = prepare_and_start_generation(&app).await;
    let mut ws = connect_ws(app.clone(), &session_id).await;

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

    let messages = recv_ws_messages_with_timeout(&mut ws, Duration::from_secs(10), 10).await;
    let initial_artifact = messages
        .iter()
        .find(|m| m["type"] == "artifact_update")
        .expect("initial artifact_update");
    let initial_version = initial_artifact["version"].as_u64().unwrap();
    let initial_count = initial_artifact["candidate"]["work_items"]
        .as_array()
        .unwrap()
        .len();

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

    let messages = recv_ws_messages_with_timeout(&mut ws, Duration::from_secs(10), 12).await;
    assert!(
        messages.iter().all(|m| m["type"] != "error"),
        "request_revision should not produce error messages: {:?}",
        messages
            .iter()
            .filter(|m| m["type"] == "error")
            .collect::<Vec<_>>()
    );

    let artifact = messages
        .iter()
        .rfind(|m| m["type"] == "artifact_update")
        .expect("artifact_update after revision");
    assert!(
        artifact["version"].as_u64().unwrap() > initial_version,
        "revision must produce a new artifact version"
    );
    let work_items = artifact["candidate"]["work_items"].as_array().unwrap();
    assert_eq!(
        work_items.len(),
        initial_count,
        "revision must keep the same total work item count"
    );
    assert!(
        !work_items.iter().any(|w| w["id"] == "work_item_0001"),
        "reverted work item must be replaced with a new id"
    );

    let stage = messages
        .iter()
        .find(|m| m["type"] == "stage_change" && m["stage"] == "author_confirm")
        .expect("stage_change back to author_confirm");
    assert_eq!(stage["stage"], "author_confirm");

    ws.close(None).await.ok();
}

#[tokio::test]
async fn revision_replaces_draft_candidate_without_touching_confirmed_records() {
    let _guard = WS_TEST_LOCK.lock().await;
    let (app, _repo) = app_with_confirmed_story_and_design_and_revision_output(
        valid_split_output(),
        valid_revision_redo_output(),
    )
    .await;

    // Create and confirm a first plan; its work items must survive the later Draft revision.
    let (status, generate_resp) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items:generate",
        json!({
            "title": "登录拆分实现",
            "story_spec_ids": ["story_spec_0001"],
            "design_spec_ids": ["design_spec_0001"],
            "include_integration_tests": true,
            "include_e2e_tests": false,
            "force_frontend_backend_split": true,
            "require_execution_plan_confirm": false
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let confirmed_plan_id = generate_resp["work_item_plan"]["plan_id"]
        .as_str()
        .unwrap()
        .to_string();
    let confirmed_ids: Vec<String> = generate_resp["work_items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["work_item_id"].as_str().unwrap().to_string())
        .collect();

    let (status, _confirm_resp) = request_json(
        app.clone(),
        Method::POST,
        &format!(
            "/api/projects/project_0001/issues/issue_0001/work-item-plans/{confirmed_plan_id}/confirm"
        ),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Prepare a second Draft plan and trigger revision on it.
    let session_id = prepare_and_start_generation(&app).await;
    let mut ws = connect_ws(app.clone(), &session_id).await;

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

    let messages = recv_ws_messages_with_timeout(&mut ws, Duration::from_secs(10), 10).await;
    let initial_artifact = messages
        .iter()
        .find(|m| m["type"] == "artifact_update")
        .expect("initial artifact_update");
    let initial_version = initial_artifact["version"].as_u64().unwrap();
    let initial_count = initial_artifact["candidate"]["work_items"]
        .as_array()
        .unwrap()
        .len();

    // First work item of the new Draft plan follows the confirmed ones (0001-0003).
    let draft_first_id = "work_item_0004";
    ws.send(Message::Text(
        json!({
            "type": "revert_work_item",
            "work_item_id": draft_first_id,
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

    let messages = recv_ws_messages_with_timeout(&mut ws, Duration::from_secs(10), 12).await;
    assert!(
        messages.iter().all(|m| m["type"] != "error"),
        "request_revision should not produce error messages: {:?}",
        messages
            .iter()
            .filter(|m| m["type"] == "error")
            .collect::<Vec<_>>()
    );

    let artifact = messages
        .iter()
        .rfind(|m| m["type"] == "artifact_update")
        .expect("artifact_update after revision");
    assert!(
        artifact["version"].as_u64().unwrap() > initial_version,
        "revision must produce a new artifact version"
    );
    let draft_ids_after: HashSet<String> = artifact["candidate"]["work_items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["id"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        draft_ids_after.len(),
        initial_count,
        "revision must keep the same total work item count"
    );
    assert!(
        !draft_ids_after.contains(draft_first_id),
        "reverted draft work item must be replaced"
    );

    // Confirmed records must still exist and remain confirmed.
    let app_paths =
        cadence_aria::product::app_paths::ProductAppPaths::new(_repo.path().join(".aria"));
    let lifecycle = cadence_aria::product::lifecycle_store::LifecycleStore::new(app_paths);
    let all_work_items = lifecycle
        .list_work_items("project_0001", "issue_0001")
        .expect("list work items");
    for confirmed_id in &confirmed_ids {
        let wi = all_work_items
            .iter()
            .find(|w| w.id == *confirmed_id)
            .expect("confirmed work item must still exist");
        assert_eq!(
            wi.plan_status.as_str(),
            "confirmed",
            "confirmed work item must not be touched"
        );
    }

    let confirmed_plan = lifecycle
        .get_issue_work_item_plan("project_0001", "issue_0001", &confirmed_plan_id)
        .expect("confirmed plan must still exist");
    assert_eq!(confirmed_plan.status.as_str(), "confirmed");

    ws.close(None).await.ok();
}

#[tokio::test]
async fn work_item_plan_validate_errors_auto_revision_uses_generate_revision() {
    let _guard = WS_TEST_LOCK.lock().await;
    // 首次 generate 返回 validate 失败的输出；带 revision_feedback 的 revision 返回合法输出。
    let (app, _repo) = app_with_confirmed_story_and_design_and_revision_output(
        invalid_split_output_missing_e2e(),
        valid_split_output(),
    )
    .await;
    let session_id = prepare_and_start_generation(&app).await;
    let mut ws = connect_ws(app.clone(), &session_id).await;

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

    let messages = recv_ws_messages_with_timeout(&mut ws, Duration::from_secs(15), 16).await;
    assert!(
        messages.iter().all(|m| m["type"] != "error"),
        "auto revision should not produce error messages: {:?}",
        messages
            .iter()
            .filter(|m| m["type"] == "error")
            .collect::<Vec<_>>()
    );

    let stage = messages
        .iter()
        .find(|m| m["type"] == "stage_change" && m["stage"] == "author_confirm")
        .expect("stage_change to author_confirm");
    assert_eq!(stage["stage"], "author_confirm");

    ws.close(None).await.ok();
}
