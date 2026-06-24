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

#[tokio::test]
#[ignore = "legacy full-candidate revision flow is superseded by WP2 outline generation; WP3+ will replace this coverage"]
async fn work_item_plan_revision_streams_provider_output_before_candidate_artifact() {
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

    let mut revision_node_id: Option<String> = None;
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
            Some("timeline_node_created") if value["node"]["node_type"] == "revision" => {
                revision_node_id = value["node"]["node_id"].as_str().map(str::to_string);
            }
            Some("stream_chunk")
                if revision_node_id
                    .as_deref()
                    .is_some_and(|node_id| value["node_id"].as_str() == Some(node_id))
                    && value["content"]
                        .as_str()
                        .unwrap_or("")
                        .contains("Fake Work Item Plan streaming draft") =>
            {
                saw_provider_stream = true;
            }
            Some("artifact_update") if value.get("candidate").is_some() => {
                saw_candidate = true;
                break;
            }
            Some("error") => panic!("ws error message: {value}"),
            _ => {}
        }
    }

    assert!(
        revision_node_id.is_some(),
        "expected dedicated revision node"
    );
    assert!(
        saw_provider_stream,
        "expected revision provider text stream before candidate"
    );
    assert!(
        saw_candidate,
        "expected candidate artifact_update after revision"
    );

    ws.close(None).await.ok();
}

#[tokio::test]
async fn work_item_plan_author_persists_outline_without_draft_work_items_or_child_sessions() {
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
            .any(|message| message["type"] == "artifact_update")
    })
    .await;

    let artifact_update = messages
        .iter()
        .find(|m| m["type"] == "artifact_update")
        .expect("artifact_update");
    assert!(
        !artifact_update["outline_candidate"]["outline"]["work_item_outlines"]
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
    assert!(
        work_items.is_empty(),
        "WP2 outline generation must not materialize draft work items"
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

#[tokio::test]
async fn work_item_plan_author_emits_provider_prompt_event() {
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
    let mut saw_provider_prompt = false;
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
                if value["node"]["node_type"] == "work_item_plan_outline_run" =>
            {
                author_node_id = value["node"]["node_id"].as_str().map(str::to_string);
            }
            Some("execution_event")
                if author_node_id
                    .as_deref()
                    .is_some_and(|node_id| value["event"]["node_id"].as_str() == Some(node_id))
                    && value["event"]["title"] == "Provider Prompt" =>
            {
                saw_provider_prompt = true;
                assert!(
                    value["event"]["output"]
                        .as_str()
                        .unwrap_or("")
                        .contains("WorkItemPlan Outline"),
                    "provider prompt should contain the prompt text: {value}"
                );
            }
            Some("artifact_update") if value.get("outline_candidate").is_some() => break,
            Some("error") => panic!("ws error message: {value}"),
            _ => {}
        }
    }

    assert!(
        saw_provider_prompt,
        "expected Provider Prompt execution_event for work_item_plan_outline_run"
    );

    ws.close(None).await.ok();
}

async fn enable_work_item_plan_review_fixture(app: &axum::Router, session_id: &str, review: Value) {
    let (status, response) = request_json(
        app.clone(),
        Method::POST,
        &format!("/api/test/workspace-sessions/{session_id}/review-fixture"),
        json!({
            "verdict": review["verdict"].as_str().unwrap_or("pass"),
            "summary": review["summary"].as_str().unwrap_or("review fixture"),
            "comments": "review fixture",
            "raw_json": review
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "enable review fixture failed: {response}"
    );
}

fn outline_review_revise() -> Value {
    json!({
        "verdict": "revise",
        "summary": "Outline 需要重写",
        "generation_round_id": "round_001",
        "affects_items": [{ "target_outline_id": "outline_backend_session" }],
        "findings": [{
            "severity": "blocking",
            "message": "写入范围存在重叠",
            "evidence": "outline_frontend 与 outline_backend 都声明 web/",
            "impact": "后续 work item 无法安全并行",
            "required_action": "重新拆分 exclusive_write_scopes"
        }]
    })
}

fn outline_review_pass() -> Value {
    json!({
        "verdict": "pass",
        "summary": "Outline 可以进入下一阶段",
        "generation_round_id": "round_002",
        "affects_items": [],
        "findings": []
    })
}
