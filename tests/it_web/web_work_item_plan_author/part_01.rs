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
    app_with_confirmed_story_and_design_and_streaming_outputs, request_json, valid_outline_output,
    valid_revision_redo_output, valid_split_output,
};

static WS_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn enable_test_controls() -> crate::TestControlsEnvGuard {
    crate::enable_test_controls().await
}

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
async fn work_item_plan_start_generation_returns_outline_artifact() {
    let _guard = WS_TEST_LOCK.lock().await;
    let (app, _repo, _prompts) =
        app_with_confirmed_story_and_design_and_streaming_outputs(vec![valid_outline_output()])
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

    let messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages
            .iter()
            .any(|m| m["type"] == "artifact_update" && m.get("outline_candidate").is_some())
            && messages
                .iter()
                .any(|m| m["type"] == "stage_change" && m["stage"] == "author_confirm")
    })
    .await;

    let artifact_update = messages
        .iter()
        .find(|m| m["type"] == "artifact_update")
        .expect("artifact_update");
    assert!(artifact_update["outline_candidate"]["outline"]["work_item_outlines"].is_array());
    assert!(
        !artifact_update["outline_candidate"]["outline"]["work_item_outlines"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(artifact_update.get("candidate").is_none());

    let _author_confirm_stage = messages
        .iter()
        .filter(|m| m["type"] == "stage_change")
        .find(|m| m["stage"] == "author_confirm")
        .expect("stage_change to author_confirm");

    ws.close(None).await.ok();
}

#[tokio::test]
async fn work_item_plan_author_streams_provider_output_before_outline_artifact() {
    let _guard = WS_TEST_LOCK.lock().await;
    let (app, _repo, _prompts) =
        app_with_confirmed_story_and_design_and_streaming_outputs(vec![valid_outline_output()])
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

    let mut author_node_id: Option<String> = None;
    let mut saw_provider_stream = false;
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
            Some("timeline_node_created")
                if value["node"]["node_type"] == "work_item_plan_outline_run"
                    && value["node"]["title"] == "WorkItemPlan Outline 生成" =>
            {
                author_node_id = value["node"]["node_id"].as_str().map(str::to_string);
            }
            Some("stream_chunk")
                if author_node_id
                    .as_deref()
                    .is_some_and(|node_id| value["node_id"].as_str() == Some(node_id))
                    && value["content"]
                        .as_str()
                        .unwrap_or("")
                        .contains("Fake Work Item Plan streaming draft") =>
            {
                saw_provider_stream = true;
            }
            Some("artifact_update") if value.get("outline_candidate").is_some() => {
                saw_candidate = true;
                break;
            }
            Some("error") => panic!("ws error message: {value}"),
            _ => {}
        }
    }

    assert!(
        author_node_id.is_some(),
        "expected dedicated work_item_plan_outline_run node"
    );
    assert!(
        saw_provider_stream,
        "expected provider text stream before candidate"
    );
    assert!(saw_candidate, "expected outline artifact_update");

    ws.close(None).await.ok();
}

#[tokio::test]
async fn work_item_plan_author_completes_provider_node_before_author_confirm() {
    let _guard = WS_TEST_LOCK.lock().await;
    let (app, _repo, _prompts) =
        app_with_confirmed_story_and_design_and_streaming_outputs(vec![valid_outline_output()])
            .await;

    let (_status, prepare_resp) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-item-plans:prepare",
        json!({
            "title": "依赖自检查拆分",
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

    let mut author_node_id: Option<String> = None;
    let mut saw_author_completed = false;
    let mut saw_author_confirm = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);

    while tokio::time::Instant::now() < deadline {
        let remaining = deadline - tokio::time::Instant::now();
        let value = match timeout(remaining, ws.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => serde_json::from_str::<Value>(&text).unwrap(),
            Ok(Some(Ok(Message::Close(_)))) => break,
            Ok(Some(Ok(_))) => continue,
            Ok(Some(Err(error))) => panic!("ws error: {error}"),
            Ok(None) | Err(_) => break,
        };

        match value["type"].as_str() {
            Some("timeline_node_created")
                if value["node"]["node_type"] == "work_item_plan_outline_run"
                    && value["node"]["title"] == "WorkItemPlan Outline 生成" =>
            {
                author_node_id = value["node"]["node_id"].as_str().map(str::to_string);
            }
            Some("timeline_node_updated")
                if author_node_id
                    .as_deref()
                    .is_some_and(|node_id| value["node_id"].as_str() == Some(node_id))
                    && value["status"] == "completed" =>
            {
                saw_author_completed = true;
            }
            Some("timeline_node_created")
                if value["node"]["node_type"] == "work_item_plan_outline_confirm" =>
            {
                saw_author_confirm = true;
            }
            Some("error") => panic!("ws error message: {value}"),
            _ => {}
        }

        if saw_author_completed && saw_author_confirm {
            break;
        }
    }

    assert!(
        author_node_id.is_some(),
        "expected WorkItemPlan outline run node"
    );
    assert!(saw_author_completed, "expected author_run to be completed");
    assert!(
        saw_author_confirm,
        "expected WorkItemPlan outline confirm node after provider completion"
    );

    ws.close(None).await.ok();
}

#[tokio::test]
async fn outline_review_revise_requires_decision_before_next_outline_provider_run() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_controls = enable_test_controls().await;
    let (app, _repo, _prompts) = app_with_confirmed_story_and_design_and_streaming_outputs(vec![
        valid_outline_output(),
        valid_outline_output(),
    ])
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

    let _messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_plan_outline_confirm"
        })
    })
    .await;

    enable_work_item_plan_review_fixture(&app, &session_id, outline_review_revise()).await;

    ws.send(Message::Text(
        json!({ "type": "author_decision", "decision": "accept" })
            .to_string()
            .into(),
    ))
    .await
    .expect("send outline accept");

    let messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        let outline_run_nodes = messages
            .iter()
            .filter(|message| {
                message["type"] == "timeline_node_created"
                    && message["node"]["node_type"] == "work_item_plan_outline_run"
            })
            .count();
        let saw_second_outline_stream = messages.iter().any(|message| {
            message["type"] == "stream_chunk"
                && message["content"]
                    .as_str()
                    .unwrap_or("")
                    .contains("Fake Work Item Plan streaming draft")
        });
        messages
            .iter()
            .any(|message| message["type"] == "review_decision_required")
            || (outline_run_nodes >= 1 && saw_second_outline_stream)
    })
    .await;

    assert!(
        messages
            .iter()
            .any(|message| { message["type"] == "review_decision_required" }),
        "expected outline review revise to require a human review decision before changes"
    );
    assert!(
        !messages
            .iter()
            .any(|message| { message["type"] == "stage_change" && message["stage"] == "running" }),
        "outline review revise must not re-enter running before human decision"
    );
    assert!(
        !messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_plan_outline_run"
        }),
        "outline review revise must not create a new outline run before human decision"
    );
    assert!(
        !messages.iter().any(|message| {
            message["type"] == "stream_chunk"
                && message["content"]
                    .as_str()
                    .unwrap_or("")
                    .contains("Fake Work Item Plan streaming draft")
        }),
        "outline review revise must not start provider streaming before human decision"
    );

    ws.send(Message::Text(
        json!({
            "type": "review_decision_response",
            "decision": "continue",
            "extra_context": null
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send review_decision_response continue");

    let messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        let outline_run_nodes = messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_plan_outline_run"
        });
        let saw_outline_stream = messages.iter().any(|message| {
            message["type"] == "stream_chunk"
                && message["content"]
                    .as_str()
                    .unwrap_or("")
                    .contains("Fake Work Item Plan streaming draft")
        });
        outline_run_nodes && saw_outline_stream
    })
    .await;

    assert!(
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_plan_outline_run"
        }),
        "expected human continue to create a new outline run node"
    );
    assert!(
        messages.iter().any(|message| {
            message["type"] == "stream_chunk"
                && message["content"]
                    .as_str()
                    .unwrap_or("")
                    .contains("Fake Work Item Plan streaming draft")
        }),
        "expected human continue to start provider streaming"
    );

    ws.close(None).await.ok();
}

#[tokio::test]
async fn work_item_plan_outline_review_revision_reenters_review_after_revised_outline_confirm() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_controls = enable_test_controls().await;
    let (app, _repo, _prompts) = app_with_confirmed_story_and_design_and_streaming_outputs(vec![
        valid_outline_output(),
        valid_outline_output(),
    ])
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

    let _messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_plan_outline_confirm"
        })
    })
    .await;

    enable_work_item_plan_review_fixture(&app, &session_id, outline_review_revise()).await;

    ws.send(Message::Text(
        json!({ "type": "author_decision", "decision": "accept" })
            .to_string()
            .into(),
    ))
    .await
    .expect("send first outline accept");

    let messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages
            .iter()
            .any(|message| message["type"] == "review_decision_required")
    })
    .await;
    assert!(
        messages
            .iter()
            .any(|message| message["type"] == "review_decision_required"),
        "expected first outline review revise to pause for review decision"
    );

    ws.send(Message::Text(
        json!({
            "type": "review_decision_response",
            "decision": "continue",
            "extra_context": null
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send review_decision_response continue");

    let messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_plan_outline_confirm"
        })
    })
    .await;
    assert!(
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_plan_outline_confirm"
        }),
        "expected revised outline to return to author confirm"
    );

    enable_work_item_plan_review_fixture(&app, &session_id, outline_review_pass()).await;

    ws.send(Message::Text(
        json!({ "type": "author_decision", "decision": "accept" })
            .to_string()
            .into(),
    ))
    .await
    .expect("send revised outline accept");

    let messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_plan_outline_review"
                && message["node"]["round"] == 2
        })
    })
    .await;
    assert!(
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_plan_outline_review"
                && message["node"]["round"] == 2
        }),
        "expected revised outline accept to create WorkItemPlan Outline Review Round 2"
    );

    ws.close(None).await.ok();
}

