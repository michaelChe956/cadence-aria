use axum::http::{Method, StatusCode};
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::lifecycle_store::{
    CreateIssueWorkItemPlanInput, CreateWorkItemInput, LifecycleStore,
};
use cadence_aria::product::models::{
    IssueWorkItemPlanOptions, IssueWorkItemPlanStatus, WorkItemKind, WorkItemPlanStatus,
    WorkspaceType,
};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::time::{Duration, timeout};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

use crate::web_work_item_generation::{
    app_with_confirmed_story_and_design, app_with_confirmed_story_and_design_and_revision_output,
    request_json, valid_revision_redo_output, valid_split_output,
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

async fn recv_ws_until<F>(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    timeout_after: Duration,
    predicate: F,
) -> Vec<Value>
where
    F: Fn(&[Value]) -> bool,
{
    let mut messages = Vec::new();
    let deadline = tokio::time::Instant::now() + timeout_after;
    while tokio::time::Instant::now() < deadline {
        let remaining = deadline - tokio::time::Instant::now();
        match timeout(remaining, ws.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => {
                let value = serde_json::from_str(&text).expect("ws json");
                messages.push(value);
                if predicate(&messages) {
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

    let messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages
            .iter()
            .any(|m| m["type"] == "artifact_update" && m.get("candidate").is_some())
            && messages
                .iter()
                .any(|m| m["type"] == "stage_change" && m["stage"] == "author_confirm")
    })
    .await;

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
async fn work_item_plan_author_streams_progress_before_candidate_artifact() {
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

    let mut saw_progress = false;
    let mut saw_candidate = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while tokio::time::Instant::now() < deadline {
        let remaining = deadline - tokio::time::Instant::now();
        let value = match timeout(remaining, ws.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => {
                serde_json::from_str::<Value>(&text).expect("ws json")
            }
            Ok(Some(Ok(Message::Close(_)))) => break,
            Ok(Some(Ok(other))) => panic!("expected text ws message, got {other:?}"),
            Ok(Some(Err(error))) => panic!("ws error: {error}"),
            Ok(None) => break,
            Err(_) => break,
        };

        match value["type"].as_str() {
            Some("stream_chunk")
                if value["content"]
                    .as_str()
                    .unwrap_or("")
                    .contains("正在生成 Work Item Plan") =>
            {
                saw_progress = true;
            }
            Some("artifact_update") if value.get("candidate").is_some() => {
                saw_candidate = true;
                break;
            }
            Some("error") => panic!("ws error message: {value}"),
            _ => {}
        }
    }

    assert!(saw_progress, "expected visible progress before candidate");
    assert!(saw_candidate, "expected candidate artifact_update");

    ws.close(None).await.ok();
}

#[tokio::test]
async fn work_item_plan_revision_streams_progress_before_candidate_artifact() {
    let _guard = WS_TEST_LOCK.lock().await;
    let (app, _repo) = app_with_confirmed_story_and_design_and_revision_output(
        valid_split_output(),
        valid_revision_redo_output(),
    )
    .await;

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

    let initial_messages = recv_ws_until(&mut ws, Duration::from_secs(10), |msgs| {
        msgs.iter().any(|m| m["type"] == "artifact_update")
            && msgs
                .iter()
                .any(|m| m["type"] == "stage_change" && m["stage"] == "author_confirm")
    })
    .await;
    let first_work_item_id = initial_messages
        .iter()
        .find(|m| m["type"] == "artifact_update")
        .expect("initial artifact_update")["candidate"]["work_items"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();

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
    let _revert_messages = recv_ws_until(&mut ws, Duration::from_secs(5), |msgs| {
        msgs.iter().any(|m| m["type"] == "artifact_update")
    })
    .await;

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

    let mut saw_progress = false;
    let mut saw_candidate = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while tokio::time::Instant::now() < deadline {
        let remaining = deadline - tokio::time::Instant::now();
        let value = match timeout(remaining, ws.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => {
                serde_json::from_str::<Value>(&text).expect("ws json")
            }
            Ok(Some(Ok(Message::Close(_)))) => break,
            Ok(Some(Ok(other))) => panic!("expected text ws message, got {other:?}"),
            Ok(Some(Err(error))) => panic!("ws error: {error}"),
            Ok(None) => break,
            Err(_) => break,
        };

        match value["type"].as_str() {
            Some("stream_chunk")
                if value["content"]
                    .as_str()
                    .unwrap_or("")
                    .contains("正在返修 Work Item Plan") =>
            {
                saw_progress = true;
            }
            Some("artifact_update") if value.get("candidate").is_some() => {
                saw_candidate = true;
                break;
            }
            Some("error") => panic!("ws error message: {value}"),
            _ => {}
        }
    }

    assert!(saw_progress, "expected revision progress before candidate");
    assert!(
        saw_candidate,
        "expected candidate artifact_update after revision"
    );

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

#[tokio::test]
async fn issue_lifecycle_includes_work_item_plan_groups_without_hiding_child_work_items() {
    let (app, repo) = app_with_confirmed_story_and_design(valid_split_output()).await;

    let lifecycle = LifecycleStore::new(ProductAppPaths::new(repo.path().join(".aria")));
    let backend_item = lifecycle
        .create_work_item(CreateWorkItemInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            story_spec_ids: vec!["story_spec_0001".to_string()],
            design_spec_ids: vec!["design_spec_0001".to_string()],
            title: "实现后端登录会话 API".to_string(),
            kind: WorkItemKind::Backend,
            sequence_hint: Some(10),
            plan_status: WorkItemPlanStatus::Draft,
            ..Default::default()
        })
        .unwrap();
    let frontend_item = lifecycle
        .create_work_item(CreateWorkItemInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            story_spec_ids: vec!["story_spec_0001".to_string()],
            design_spec_ids: vec!["design_spec_0001".to_string()],
            title: "实现前端会话过期提示".to_string(),
            kind: WorkItemKind::Frontend,
            sequence_hint: Some(20),
            depends_on: vec![backend_item.id.clone()],
            plan_status: WorkItemPlanStatus::Draft,
            ..Default::default()
        })
        .unwrap();
    let expected_work_item_ids = vec![backend_item.id.clone(), frontend_item.id.clone()];

    let plan = lifecycle
        .create_issue_work_item_plan(CreateIssueWorkItemPlanInput {
            id: None,
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            source_story_spec_ids: vec!["story_spec_0001".to_string()],
            source_design_spec_ids: vec!["design_spec_0001".to_string()],
            options: IssueWorkItemPlanOptions {
                include_integration_tests: true,
                include_e2e_tests: false,
                force_frontend_backend_split: true,
                require_execution_plan_confirm: false,
            },
            status: IssueWorkItemPlanStatus::Draft,
            work_item_ids: expected_work_item_ids.clone(),
            repository_profile_ref: None,
            verification_plan_ids: Vec::new(),
            dependency_graph: Vec::new(),
            created_from_provider_run: None,
            validator_findings: Vec::new(),
        })
        .unwrap();

    let (status, lifecycle_response) = request_json(
        app,
        Method::GET,
        "/api/issues/issue_0001/lifecycle?project_id=project_0001",
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let groups = lifecycle_response["work_item_plans"]
        .as_array()
        .expect("work_item_plans group list");
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0]["id"], plan.id);
    assert_eq!(groups[0]["status"], "draft");
    assert_eq!(
        groups[0]["source_story_spec_ids"],
        json!(["story_spec_0001"])
    );
    assert_eq!(
        groups[0]["source_design_spec_ids"],
        json!(["design_spec_0001"])
    );
    assert_eq!(groups[0]["work_item_ids"], json!(expected_work_item_ids));

    let work_items = lifecycle_response["work_items"].as_array().unwrap();
    assert_eq!(work_items.len(), 2);
    let returned_work_item_ids = work_items
        .iter()
        .map(|item| item["work_item_id"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(returned_work_item_ids.contains(&backend_item.id.as_str()));
    assert!(returned_work_item_ids.contains(&frontend_item.id.as_str()));
}
