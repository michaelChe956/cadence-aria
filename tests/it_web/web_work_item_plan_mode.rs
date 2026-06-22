use axum::http::Method;
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::work_item_plan_store::WorkItemPlanStore;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::time::{Duration, timeout};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

use crate::web_work_item_generation::{
    app_with_confirmed_story_and_design_and_streaming_outputs, request_json, valid_outline_output,
};

static WS_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

struct TestControlsGuard;

impl Drop for TestControlsGuard {
    fn drop(&mut self) {
        unsafe {
            std::env::remove_var("ARIA_E2E_TEST_CONTROLS");
        }
    }
}

fn enable_test_controls() -> TestControlsGuard {
    unsafe {
        std::env::set_var("ARIA_E2E_TEST_CONTROLS", "1");
    }
    TestControlsGuard
}

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

async fn prepare_plan_and_start(
    app: &axum::Router,
    reviewer_enabled: bool,
) -> (String, String, WsStream) {
    let (status, prepare_resp) = request_json(
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
            "reviewer_enabled": reviewer_enabled
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send start_generation");

    let messages = recv_ws_until(&mut ws, Duration::from_secs(15), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_plan_outline_confirm"
        })
    })
    .await;
    messages
        .iter()
        .find(|message| {
            message["type"] == "artifact_update" && message.get("outline_candidate").is_some()
        })
        .expect("outline artifact");

    (session_id, plan_id, ws)
}

async fn enable_work_item_plan_review_fixture(app: &axum::Router, session_id: &str, verdict: &str) {
    let (status, response) = request_json(
        app.clone(),
        Method::POST,
        &format!("/api/test/workspace-sessions/{session_id}/review-fixture"),
        json!({
            "verdict": verdict,
            "summary": "outline review fixture",
            "comments": "outline review comments",
            "raw_json": {
                "verdict": verdict,
                "summary": "outline review fixture",
                "generation_round_id": "round_001",
                "affects_items": [
                    { "target_outline_id": "outline_backend_session" }
                ],
                "findings": if verdict == "revise" {
                    json!([{
                        "severity": "must_fix",
                        "message": "拆分遗漏前端错误状态",
                        "evidence": "Outline 未覆盖前端错误状态",
                        "impact": "后续 work item 无法覆盖体验缺口",
                        "required_action": "补充前端错误状态 outline"
                    }])
                } else {
                    json!([])
                }
            }
        }),
    )
    .await;
    assert_eq!(
        status,
        axum::http::StatusCode::OK,
        "enable review fixture failed: {response}"
    );
}

fn active_index(
    root: &tempfile::TempDir,
    plan_id: &str,
) -> cadence_aria::product::models::WorkItemPlanDraftActiveIndex {
    WorkItemPlanStore::new(ProductAppPaths::new(root.path().join(".aria")))
        .load_active_index("project_0001", "issue_0001", plan_id)
        .expect("load active index")
        .expect("active index")
}

#[tokio::test]
async fn accept_outline_creates_generation_round_and_active_index() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls();
    let (app, root, _prompts) = app_with_confirmed_story_and_design_and_streaming_outputs(vec![
        valid_outline_output(),
        valid_outline_output(),
    ])
    .await;
    let (_session_id, plan_id, mut ws) = prepare_plan_and_start(&app, false).await;

    ws.send(Message::Text(
        json!({ "type": "author_decision", "decision": "accept" })
            .to_string()
            .into(),
    ))
    .await
    .expect("send accept");

    let _messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_generation_mode"
        })
    })
    .await;

    let index = active_index(&root, &plan_id);
    assert_eq!(index.current_generation_round_id, "round_001");
    assert_eq!(index.outline_state, "confirmed");
    assert!(index.outline_to_current_draft_id.is_empty());

    ws.close(None).await.ok();
}

#[tokio::test]
async fn author_decision_is_rejected_on_generation_mode_node() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls();
    let (app, _root, _prompts) =
        app_with_confirmed_story_and_design_and_streaming_outputs(vec![valid_outline_output()])
            .await;
    let (_session_id, _plan_id, mut ws) = prepare_plan_and_start(&app, false).await;

    ws.send(Message::Text(
        json!({ "type": "author_decision", "decision": "accept" })
            .to_string()
            .into(),
    ))
    .await
    .expect("send outline accept");
    let _messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_generation_mode"
        })
    })
    .await;

    ws.send(Message::Text(
        json!({ "type": "author_decision", "decision": "accept" })
            .to_string()
            .into(),
    ))
    .await
    .expect("send invalid author decision");
    let messages = recv_ws_until(&mut ws, Duration::from_secs(5), |messages| {
        messages
            .iter()
            .any(|message| message["type"] == "protocol_error")
    })
    .await;
    let protocol_error = messages
        .iter()
        .find(|message| message["type"] == "protocol_error")
        .expect("protocol error");
    assert_eq!(protocol_error["code"], "INVALID_AUTHOR_DECISION");

    ws.close(None).await.ok();
}

#[tokio::test]
async fn request_revision_on_outline_confirm_returns_to_outline_run_without_round() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls();
    let (app, root, _prompts) = app_with_confirmed_story_and_design_and_streaming_outputs(vec![
        valid_outline_output(),
        valid_outline_output(),
    ])
    .await;
    let (_session_id, plan_id, mut ws) = prepare_plan_and_start(&app, false).await;

    ws.send(Message::Text(
        json!({
            "type": "request_revision",
            "feedback": {
                "feedback_types": ["scope"],
                "description": "先拆出错误状态处理",
                "target_artifact_version": null
            }
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send outline request_revision");

    let messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_plan_outline_run"
        })
    })
    .await;
    let node = messages
        .iter()
        .find(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_plan_outline_run"
        })
        .expect("outline run node");
    assert_eq!(node["node"]["stage"], "running");

    let store = WorkItemPlanStore::new(ProductAppPaths::new(root.path().join(".aria")));
    assert!(
        store
            .load_active_index("project_0001", "issue_0001", &plan_id)
            .expect("load active index")
            .is_none(),
        "outline revision before accept must not create generation round"
    );

    ws.close(None).await.ok();
}

#[tokio::test]
async fn outline_accept_enters_outline_review_when_reviewer_enabled() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls();
    let (app, _root, _prompts) =
        app_with_confirmed_story_and_design_and_streaming_outputs(vec![valid_outline_output()])
            .await;
    let (session_id, _plan_id, mut ws) = prepare_plan_and_start(&app, true).await;
    enable_work_item_plan_review_fixture(&app, &session_id, "pass").await;

    ws.send(Message::Text(
        json!({ "type": "author_decision", "decision": "accept" })
            .to_string()
            .into(),
    ))
    .await
    .expect("send accept");

    let messages = recv_ws_until(&mut ws, Duration::from_secs(15), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_plan_outline_review"
        })
    })
    .await;
    let review_node = messages
        .iter()
        .find(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_plan_outline_review"
        })
        .expect("outline review node");
    assert_eq!(review_node["node"]["stage"], "cross_review");

    ws.close(None).await.ok();
}

#[tokio::test]
async fn outline_accept_skips_review_when_reviewer_disabled() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls();
    let (app, _root, _prompts) =
        app_with_confirmed_story_and_design_and_streaming_outputs(vec![valid_outline_output()])
            .await;
    let (_session_id, _plan_id, mut ws) = prepare_plan_and_start(&app, false).await;

    ws.send(Message::Text(
        json!({ "type": "author_decision", "decision": "accept" })
            .to_string()
            .into(),
    ))
    .await
    .expect("send accept");

    let messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_generation_mode"
        })
    })
    .await;
    assert!(messages.iter().all(|message| {
        !(message["type"] == "timeline_node_created"
            && message["node"]["node_type"] == "work_item_plan_outline_review")
    }));

    ws.close(None).await.ok();
}

#[tokio::test]
async fn outline_review_pass_enters_generation_mode() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls();
    let (app, _root, _prompts) =
        app_with_confirmed_story_and_design_and_streaming_outputs(vec![valid_outline_output()])
            .await;
    let (session_id, _plan_id, mut ws) = prepare_plan_and_start(&app, true).await;
    enable_work_item_plan_review_fixture(&app, &session_id, "pass").await;

    ws.send(Message::Text(
        json!({ "type": "author_decision", "decision": "accept" })
            .to_string()
            .into(),
    ))
    .await
    .expect("send accept");

    let messages = recv_ws_until(&mut ws, Duration::from_secs(15), |messages| {
        messages.iter().any(|message| {
            message["type"] == "review_complete"
                && message["work_item_plan_review"]["review_scope"] == "outline"
        }) && messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_generation_mode"
        })
    })
    .await;
    let review_complete = messages
        .iter()
        .find(|message| message["type"] == "review_complete")
        .expect("review complete");
    assert_eq!(
        review_complete["work_item_plan_review"]["review_scope"],
        "outline"
    );

    ws.close(None).await.ok();
}

#[tokio::test]
async fn outline_review_revise_returns_to_outline_revision() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls();
    let (app, root, _prompts) =
        app_with_confirmed_story_and_design_and_streaming_outputs(vec![valid_outline_output()])
            .await;
    let (session_id, plan_id, mut ws) = prepare_plan_and_start(&app, true).await;
    enable_work_item_plan_review_fixture(&app, &session_id, "revise").await;

    ws.send(Message::Text(
        json!({ "type": "author_decision", "decision": "accept" })
            .to_string()
            .into(),
    ))
    .await
    .expect("send accept");

    let _messages = recv_ws_until(&mut ws, Duration::from_secs(15), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_plan_outline_run"
        }) && messages
            .iter()
            .any(|message| message["type"] == "stage_change" && message["stage"] == "running")
    })
    .await;

    let index = active_index(&root, &plan_id);
    assert_eq!(index.outline_state, "revising");

    ws.close(None).await.ok();
}

#[tokio::test]
async fn select_serial_mode_enters_first_item_run() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls();
    let (app, _root, _prompts) =
        app_with_confirmed_story_and_design_and_streaming_outputs(vec![valid_outline_output()])
            .await;
    let (_session_id, _plan_id, mut ws) = prepare_plan_and_start(&app, false).await;

    ws.send(Message::Text(
        json!({ "type": "author_decision", "decision": "accept" })
            .to_string()
            .into(),
    ))
    .await
    .expect("send accept");
    let _messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_generation_mode"
        })
    })
    .await;

    ws.send(Message::Text(
        json!({ "type": "select_work_item_generation_mode", "mode": "serial" })
            .to_string()
            .into(),
    ))
    .await
    .expect("send serial mode");
    let messages = recv_ws_until(&mut ws, Duration::from_secs(5), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_draft_run"
        })
    })
    .await;
    let node = messages
        .iter()
        .find(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_draft_run"
        })
        .expect("draft run node");
    assert_eq!(node["node"]["stage"], "running");

    ws.close(None).await.ok();
}

#[tokio::test]
async fn select_batch_mode_enters_batch_run() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls();
    let (app, _root, _prompts) =
        app_with_confirmed_story_and_design_and_streaming_outputs(vec![valid_outline_output()])
            .await;
    let (_session_id, _plan_id, mut ws) = prepare_plan_and_start(&app, false).await;

    ws.send(Message::Text(
        json!({ "type": "author_decision", "decision": "accept" })
            .to_string()
            .into(),
    ))
    .await
    .expect("send accept");
    let _messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_generation_mode"
        })
    })
    .await;

    ws.send(Message::Text(
        json!({ "type": "select_work_item_generation_mode", "mode": "batch" })
            .to_string()
            .into(),
    ))
    .await
    .expect("send batch mode");
    let messages = recv_ws_until(&mut ws, Duration::from_secs(5), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_batch_run"
        })
    })
    .await;
    let node = messages
        .iter()
        .find(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_batch_run"
        })
        .expect("batch run node");
    assert_eq!(node["node"]["stage"], "running");

    ws.close(None).await.ok();
}

#[tokio::test]
async fn request_outline_revision_on_mode_node_sets_outline_revising() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls();
    let (app, root, _prompts) =
        app_with_confirmed_story_and_design_and_streaming_outputs(vec![valid_outline_output()])
            .await;
    let (_session_id, plan_id, mut ws) = prepare_plan_and_start(&app, false).await;

    ws.send(Message::Text(
        json!({ "type": "author_decision", "decision": "accept" })
            .to_string()
            .into(),
    ))
    .await
    .expect("send accept");
    let _messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_generation_mode"
        })
    })
    .await;

    ws.send(Message::Text(
        json!({
            "type": "request_outline_revision",
            "feedback": "先拆出错误状态处理"
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send outline revision");
    let _messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_plan_outline_run"
        })
    })
    .await;

    let index = active_index(&root, &plan_id);
    assert_eq!(index.outline_state, "revising");

    ws.close(None).await.ok();
}

#[tokio::test]
async fn select_mode_rejected_outside_generation_mode_node() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls();
    let (app, _root, _prompts) =
        app_with_confirmed_story_and_design_and_streaming_outputs(vec![valid_outline_output()])
            .await;
    let (_session_id, _plan_id, mut ws) = prepare_plan_and_start(&app, false).await;

    ws.send(Message::Text(
        json!({ "type": "select_work_item_generation_mode", "mode": "serial" })
            .to_string()
            .into(),
    ))
    .await
    .expect("send invalid mode");
    let messages = recv_ws_until(&mut ws, Duration::from_secs(5), |messages| {
        messages
            .iter()
            .any(|message| message["type"] == "protocol_error")
    })
    .await;
    let protocol_error = messages
        .iter()
        .find(|message| message["type"] == "protocol_error")
        .expect("protocol error");
    assert_eq!(
        protocol_error["code"],
        "WORK_ITEM_GENERATION_MODE_NODE_REQUIRED"
    );

    ws.close(None).await.ok();
}

#[tokio::test]
async fn session_state_restores_generation_mode_node_with_outline_payload() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls();
    let (app, _root, _prompts) =
        app_with_confirmed_story_and_design_and_streaming_outputs(vec![valid_outline_output()])
            .await;
    let (session_id, _plan_id, mut ws) = prepare_plan_and_start(&app, false).await;

    ws.send(Message::Text(
        json!({ "type": "author_decision", "decision": "accept" })
            .to_string()
            .into(),
    ))
    .await
    .expect("send accept");
    let _messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_generation_mode"
        })
    })
    .await;
    ws.close(None).await.ok();

    let mut restored = connect_ws(app.clone(), &session_id).await;
    let messages = recv_ws_until(&mut restored, Duration::from_secs(5), |messages| {
        messages
            .iter()
            .any(|message| message["type"] == "session_state")
    })
    .await;
    let state = messages
        .iter()
        .find(|message| message["type"] == "session_state")
        .expect("session state");
    assert_eq!(state["stage"], "author_confirm");
    let active_node_id = state["active_node_id"].as_str().expect("active node id");
    let active_node = state["timeline_nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["node_id"] == active_node_id)
        .expect("active node");
    assert_eq!(active_node["node_type"], "work_item_generation_mode");
    assert!(state["artifact"].get("outline_candidate").is_some());
    assert_eq!(
        state["artifact"]["outline_candidate"]["current_generation_round_id"],
        "round_001"
    );

    restored.close(None).await.ok();
}
