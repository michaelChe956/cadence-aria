use axum::http::Method;
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::lifecycle_store::LifecycleStore;
use cadence_aria::product::models::{
    WorkItemDraftStatus, WorkItemDraftSupersedeReason, WorkspaceType,
};
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

async fn enable_test_controls() -> crate::TestControlsEnvGuard {
    crate::enable_test_controls().await
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

async fn prepare_plan_accept_outline_and_select_serial(
    app: &axum::Router,
) -> (String, String, WsStream) {
    prepare_plan_accept_outline_and_select_serial_with_reviewer(app, false).await
}

async fn prepare_plan_accept_outline_and_select_serial_with_reviewer(
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
    let _messages = recv_ws_until(&mut ws, Duration::from_secs(15), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_plan_outline_confirm"
        })
    })
    .await;

    if reviewer_enabled {
        enable_work_item_plan_review_fixture(&app.clone(), &session_id, outline_review_pass())
            .await;
    }

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
        json!({ "type": "select_work_item_generation_mode", "mode": "serial" })
            .to_string()
            .into(),
    ))
    .await
    .expect("send serial mode");

    (session_id, plan_id, ws)
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
        axum::http::StatusCode::OK,
        "enable review fixture failed: {response}"
    );
}

fn outline_review_pass() -> Value {
    json!({
        "verdict": "pass",
        "summary": "Outline review 通过",
        "generation_round_id": "round_001",
        "affects_items": [],
        "findings": []
    })
}

fn item_review_pass(outline_id: &str, draft_id: &str) -> Value {
    json!({
        "verdict": "pass",
        "summary": "Item review 通过",
        "target_outline_id": outline_id,
        "generation_round_id": "round_001",
        "draft_id": draft_id,
        "affects_items": [{ "target_outline_id": outline_id }],
        "findings": []
    })
}

fn item_review_revise(outline_id: &str, draft_id: &str) -> Value {
    json!({
        "verdict": "revise",
        "summary": "需要重写当前 item",
        "target_outline_id": outline_id,
        "generation_round_id": "round_001",
        "draft_id": draft_id,
        "affects_items": [{ "target_outline_id": outline_id }],
        "findings": [{
            "severity": "blocking",
            "message": "当前 draft 缺少关键异常路径",
            "evidence": "未说明会话刷新失败处理",
            "impact": "后续 item 无法可靠消费 handoff",
            "required_action": "补充刷新失败处理"
        }]
    })
}

fn item_review_plan_reopen(outline_id: &str, draft_id: &str) -> Value {
    json!({
        "verdict": "plan_reopen_required",
        "summary": "需要重开 Outline",
        "target_outline_id": outline_id,
        "generation_round_id": "round_001",
        "draft_id": draft_id,
        "affects_items": [{ "target_outline_id": outline_id }],
        "findings": [{
            "severity": "blocking",
            "message": "当前问题需要调整拆分边界",
            "evidence": "当前 item 需要前序 item 未规划的 API",
            "impact": "必须回到 Outline 重新规划依赖",
            "required_action": "重开 Outline"
        }]
    })
}

#[tokio::test]
async fn serial_mode_starts_first_outline_by_topological_order() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls().await;
    let (app, root, _prompts) =
        app_with_confirmed_story_and_design_and_streaming_outputs(vec![valid_outline_output()])
            .await;
    let (_session_id, plan_id, mut ws) = prepare_plan_accept_outline_and_select_serial(&app).await;

    let messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
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
    assert_eq!(node["node"]["title"], "Draft · 实现后端登录会话 API");
    assert_eq!(node["node"]["summary"], "outline_backend_session · pending");

    let index = WorkItemPlanStore::new(ProductAppPaths::new(root.path().join(".aria")))
        .load_active_index("project_0001", "issue_0001", &plan_id)
        .expect("load active index")
        .expect("active index");
    assert_eq!(
        index.active_outline_id.as_deref(),
        Some("outline_backend_session")
    );

    ws.close(None).await.ok();
}

#[tokio::test]
async fn serial_draft_run_emits_provider_prompt_event() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls().await;
    let (app, _root, _prompts) = app_with_confirmed_story_and_design_and_streaming_outputs(vec![
        valid_outline_output(),
        valid_draft_output("outline_backend_session"),
    ])
    .await;
    let (_session_id, _plan_id, mut ws) = prepare_plan_accept_outline_and_select_serial(&app).await;

    let messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "execution_event"
                && message["event"]["title"] == "Provider Prompt"
                && message["event"]["node_id"].as_str().is_some_and(|node_id| {
                    messages.iter().any(|created| {
                        created["type"] == "timeline_node_created"
                            && created["node"]["node_id"] == node_id
                            && created["node"]["node_type"] == "work_item_draft_run"
                    })
                })
        })
    })
    .await;

    let prompt_event = messages
        .iter()
        .find(|message| {
            message["type"] == "execution_event" && message["event"]["title"] == "Provider Prompt"
        })
        .expect("draft Provider Prompt event");

    assert_eq!(prompt_event["event"]["kind"], "output");
    assert_eq!(prompt_event["event"]["status"], "started");
    assert!(
        prompt_event["event"]["output"]
            .as_str()
            .expect("prompt output")
            .contains("Work Item Draft author")
    );

    ws.close(None).await.ok();
}

#[tokio::test]
async fn serial_item_run_writes_draft_record_not_real_work_item() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls().await;
    let (app, root, _prompts) = app_with_confirmed_story_and_design_and_streaming_outputs(vec![
        valid_outline_output(),
        valid_draft_output("outline_backend_session"),
        valid_frontend_draft_output(),
    ])
    .await;
    let (_session_id, plan_id, mut ws) = prepare_plan_accept_outline_and_select_serial(&app).await;

    let messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_draft_confirm"
        })
    })
    .await;
    let node = messages
        .iter()
        .find(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_draft_confirm"
        })
        .expect("draft confirm node");
    assert_eq!(node["node"]["stage"], "author_confirm");
    let draft_run_node_id = messages
        .iter()
        .find(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_draft_run"
        })
        .and_then(|message| message["node"]["node_id"].as_str())
        .expect("draft run node id");
    let draft_run_update = messages
        .iter()
        .find(|message| {
            message["type"] == "timeline_node_updated" && message["node_id"] == draft_run_node_id
        })
        .expect("draft run update");
    assert_eq!(
        draft_run_update["summary"],
        "outline_backend_session · draft_001 · draft"
    );

    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let draft_store = WorkItemPlanStore::new(paths.clone());
    let drafts = draft_store
        .list_draft_records("project_0001", "issue_0001", &plan_id)
        .expect("list drafts");
    assert_eq!(drafts.len(), 1);
    assert_eq!(drafts[0].outline_id, "outline_backend_session");
    assert_eq!(
        drafts[0].status,
        cadence_aria::product::models::WorkItemDraftStatus::Draft
    );

    let lifecycle = LifecycleStore::new(paths);
    assert!(
        lifecycle
            .list_work_items("project_0001", "issue_0001")
            .expect("list work items")
            .is_empty()
    );
    assert!(
        lifecycle
            .list_verification_plans("project_0001", "issue_0001")
            .expect("list verification plans")
            .is_empty()
    );
    assert!(
        lifecycle
            .list_workspace_sessions("project_0001", "issue_0001")
            .expect("list sessions")
            .into_iter()
            .all(|session| session.workspace_type != WorkspaceType::WorkItem)
    );

    ws.close(None).await.ok();
}

#[tokio::test]
async fn local_validation_success_enters_draft_confirm_with_accept() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls().await;
    let (app, _root, _prompts) = app_with_confirmed_story_and_design_and_streaming_outputs(vec![
        valid_outline_output(),
        valid_draft_output("outline_backend_session"),
    ])
    .await;
    let (_session_id, _plan_id, mut ws) = prepare_plan_accept_outline_and_select_serial(&app).await;

    let messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_draft_confirm"
        })
    })
    .await;
    let artifact = messages
        .iter()
        .rfind(|message| {
            message["type"] == "artifact_update" && message.get("draft_candidate").is_some()
        })
        .expect("draft candidate artifact");
    assert_eq!(artifact["draft_candidate"]["can_accept"], true);
    assert!(
        artifact["draft_candidate"]["validator_findings"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    ws.close(None).await.ok();
}

#[tokio::test]
async fn local_validation_failure_enters_draft_confirm_without_accept() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls().await;
    let (app, root, _prompts) = app_with_confirmed_story_and_design_and_streaming_outputs(vec![
        valid_outline_output(),
        invalid_draft_output_missing_scope("outline_backend_session"),
    ])
    .await;
    let (_session_id, plan_id, mut ws) = prepare_plan_accept_outline_and_select_serial(&app).await;

    let messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_draft_confirm"
        })
    })
    .await;
    let artifact = messages
        .iter()
        .rfind(|message| {
            message["type"] == "artifact_update" && message.get("draft_candidate").is_some()
        })
        .expect("draft candidate artifact");
    assert_eq!(artifact["draft_candidate"]["can_accept"], false);
    assert_eq!(
        artifact["draft_candidate"]["draft_record"]["status"],
        "validation_failed"
    );
    assert!(
        !artifact["draft_candidate"]["validator_findings"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    ws.send(Message::Text(
        json!({
            "type": "work_item_draft_decision",
            "outline_id": "outline_backend_session",
            "decision": "accept",
            "feedback": null
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send invalid draft accept");

    let messages = recv_ws_until(&mut ws, Duration::from_secs(5), |messages| {
        messages
            .iter()
            .any(|message| message["type"] == "protocol_error")
    })
    .await;
    assert!(
        messages
            .iter()
            .any(|message| message["type"] == "protocol_error"),
        "accepting invalid draft should return protocol_error, got {messages:?}"
    );

    let index = WorkItemPlanStore::new(ProductAppPaths::new(root.path().join(".aria")))
        .load_active_index("project_0001", "issue_0001", &plan_id)
        .expect("load active index")
        .expect("active index");
    let draft_id = index
        .outline_to_current_draft_id
        .get("outline_backend_session")
        .expect("active backend draft");
    assert_eq!(
        index.draft_statuses.get(draft_id),
        Some(&WorkItemDraftStatus::ValidationFailed)
    );

    ws.close(None).await.ok();
}

#[tokio::test]
async fn accepted_draft_enters_item_review_when_reviewer_enabled() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls().await;
    let (app, root, _prompts) = app_with_confirmed_story_and_design_and_streaming_outputs(vec![
        valid_outline_output(),
        valid_draft_output("outline_backend_session"),
        valid_frontend_draft_output(),
    ])
    .await;
    let (_session_id, plan_id, mut ws) =
        prepare_plan_accept_outline_and_select_serial_with_reviewer(&app, true).await;

    let messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_draft_confirm"
        })
    })
    .await;
    assert!(
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_draft_confirm"
        }),
        "serial draft should reach confirm before item review test continues, got {messages:?}"
    );
    let index = WorkItemPlanStore::new(ProductAppPaths::new(root.path().join(".aria")))
        .load_active_index("project_0001", "issue_0001", &plan_id)
        .expect("load active index")
        .expect("active index");
    let draft_id = index
        .outline_to_current_draft_id
        .get("outline_backend_session")
        .expect("active backend draft")
        .clone();
    enable_work_item_plan_review_fixture(
        &app,
        &_session_id,
        item_review_pass("outline_backend_session", &draft_id),
    )
    .await;

    ws.send(Message::Text(
        json!({
            "type": "work_item_draft_decision",
            "outline_id": "outline_backend_session",
            "decision": "accept",
            "feedback": null
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send draft accept");

    let messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_draft_review"
        })
    })
    .await;
    assert!(
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_draft_review"
        }),
        "accept should enter item review, got {messages:?}"
    );

    ws.close(None).await.ok();
}

#[tokio::test]
async fn author_decision_is_rejected_on_draft_confirm_node() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls().await;
    let (app, _root, _prompts) = app_with_confirmed_story_and_design_and_streaming_outputs(vec![
        valid_outline_output(),
        valid_draft_output("outline_backend_session"),
    ])
    .await;
    let (_session_id, _plan_id, mut ws) =
        prepare_plan_accept_outline_and_select_serial_with_reviewer(&app, true).await;

    let _messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_draft_confirm"
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
    assert!(
        !messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "reviewer_run"
        }),
        "generic reviewer_run must not be created from draft confirm: {messages:?}"
    );

    ws.close(None).await.ok();
}

