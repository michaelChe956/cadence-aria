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
    let _test_guard = enable_test_controls();
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
async fn serial_item_run_writes_draft_record_not_real_work_item() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls();
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
    let _test_guard = enable_test_controls();
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
    let _test_guard = enable_test_controls();
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
    let _test_guard = enable_test_controls();
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
async fn item_review_pass_starts_next_outline() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls();
    let (app, root, _prompts) = app_with_confirmed_story_and_design_and_streaming_outputs(vec![
        valid_outline_output(),
        valid_draft_output("outline_backend_session"),
        valid_frontend_draft_output(),
    ])
    .await;
    let (session_id, plan_id, mut ws) =
        prepare_plan_accept_outline_and_select_serial_with_reviewer(&app, true).await;

    let _messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_draft_confirm"
        })
    })
    .await;
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
        &session_id,
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
            message["type"] == "artifact_update"
                && message["draft_candidate"]["draft_record"]["outline_id"]
                    == "outline_frontend_expiry"
        })
    })
    .await;
    assert!(
        messages.iter().any(|message| {
            message["type"] == "artifact_update"
                && message["draft_candidate"]["draft_record"]["outline_id"]
                    == "outline_frontend_expiry"
        }),
        "item review pass should generate next outline draft, got {messages:?}"
    );

    ws.close(None).await.ok();
}

#[tokio::test]
async fn item_review_revise_rewrites_only_current_item() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls();
    let (app, root, _prompts) = app_with_confirmed_story_and_design_and_streaming_outputs(vec![
        valid_outline_output(),
        valid_draft_output("outline_backend_session"),
        valid_draft_output_with_title(
            "outline_backend_session",
            "Reviewer 返修后的后端登录会话 API",
        ),
    ])
    .await;
    let (session_id, plan_id, mut ws) =
        prepare_plan_accept_outline_and_select_serial_with_reviewer(&app, true).await;

    let _messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_draft_confirm"
        })
    })
    .await;
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
        &session_id,
        item_review_revise("outline_backend_session", &draft_id),
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
            message["type"] == "artifact_update"
                && message["draft_candidate"]["draft_record"]["candidate"]["title"]
                    == "Reviewer 返修后的后端登录会话 API"
        })
    })
    .await;
    assert!(
        messages.iter().any(|message| {
            message["type"] == "artifact_update"
                && message["draft_candidate"]["draft_record"]["candidate"]["title"]
                    == "Reviewer 返修后的后端登录会话 API"
        }),
        "item review revise should regenerate current outline, got {messages:?}"
    );

    ws.close(None).await.ok();
}

#[tokio::test]
async fn item_review_plan_reopen_marks_outline_revising() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls();
    let (app, root, _prompts) = app_with_confirmed_story_and_design_and_streaming_outputs(vec![
        valid_outline_output(),
        valid_draft_output("outline_backend_session"),
    ])
    .await;
    let (session_id, plan_id, mut ws) =
        prepare_plan_accept_outline_and_select_serial_with_reviewer(&app, true).await;

    let _messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_draft_confirm"
        })
    })
    .await;
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
        &session_id,
        item_review_plan_reopen("outline_backend_session", &draft_id),
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
        messages
            .iter()
            .any(|message| message["type"] == "stage_change" && message["stage"] == "human_confirm")
    })
    .await;
    assert!(
        messages
            .iter()
            .any(|message| message["type"] == "stage_change" && message["stage"] == "human_confirm"),
        "plan reopen should enter human confirm, got {messages:?}"
    );

    let index = WorkItemPlanStore::new(ProductAppPaths::new(root.path().join(".aria")))
        .load_active_index("project_0001", "issue_0001", &plan_id)
        .expect("load active index")
        .expect("active index");
    assert_eq!(index.outline_state, "revising");
    assert_eq!(index.active_outline_id, None);

    ws.close(None).await.ok();
}

#[tokio::test]
async fn item_review_revise_affecting_previous_item_downgrades_to_needs_human() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls();
    let (app, root, _prompts) = app_with_confirmed_story_and_design_and_streaming_outputs(vec![
        valid_outline_output(),
        valid_draft_output("outline_backend_session"),
        valid_frontend_draft_output(),
    ])
    .await;
    let (session_id, plan_id, mut ws) =
        prepare_plan_accept_outline_and_select_serial_with_reviewer(&app, true).await;

    let _messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_draft_confirm"
        })
    })
    .await;
    let store = WorkItemPlanStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let index = store
        .load_active_index("project_0001", "issue_0001", &plan_id)
        .expect("load active index")
        .expect("active index");
    let backend_draft_id = index
        .outline_to_current_draft_id
        .get("outline_backend_session")
        .expect("active backend draft")
        .clone();
    enable_work_item_plan_review_fixture(
        &app,
        &session_id,
        item_review_pass("outline_backend_session", &backend_draft_id),
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
    .expect("send backend draft accept");

    let _messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "artifact_update"
                && message["draft_candidate"]["draft_record"]["outline_id"]
                    == "outline_frontend_expiry"
        }) && messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_draft_confirm"
        })
    })
    .await;

    let index = store
        .load_active_index("project_0001", "issue_0001", &plan_id)
        .expect("load active index")
        .expect("active index");
    let frontend_draft_id = index
        .outline_to_current_draft_id
        .get("outline_frontend_expiry")
        .expect("active frontend draft")
        .clone();
    enable_work_item_plan_review_fixture(
        &app,
        &session_id,
        item_review_revise("outline_backend_session", &frontend_draft_id),
    )
    .await;

    ws.send(Message::Text(
        json!({
            "type": "work_item_draft_decision",
            "outline_id": "outline_frontend_expiry",
            "decision": "accept",
            "feedback": null
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send frontend draft accept");

    let messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages
            .iter()
            .any(|message| message["type"] == "stage_change" && message["stage"] == "human_confirm")
    })
    .await;
    assert!(
        messages
            .iter()
            .any(|message| message["type"] == "stage_change" && message["stage"] == "human_confirm"),
        "revise targeting previous item should require human confirm, got {messages:?}"
    );

    ws.close(None).await.ok();
}

#[tokio::test]
async fn draft_accept_marks_record_accepted() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls();
    let (app, root, _prompts) = app_with_confirmed_story_and_design_and_streaming_outputs(vec![
        valid_outline_output(),
        valid_draft_output("outline_backend_session"),
    ])
    .await;
    let (_session_id, plan_id, mut ws) = prepare_plan_accept_outline_and_select_serial(&app).await;

    let _messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_draft_confirm"
        })
    })
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

    let _messages = recv_ws_until(&mut ws, Duration::from_secs(5), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_updated" && message["status"] == "completed"
        })
    })
    .await;

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
        Some(&cadence_aria::product::models::WorkItemDraftStatus::Accepted)
    );

    ws.close(None).await.ok();
}

#[tokio::test]
async fn draft_rewrite_supersedes_old_draft_and_regenerates_current_outline() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls();
    let (app, root, prompts) = app_with_confirmed_story_and_design_and_streaming_outputs(vec![
        valid_outline_output(),
        valid_draft_output("outline_backend_session"),
        valid_draft_output_with_title("outline_backend_session", "重写后的后端登录会话 API"),
    ])
    .await;
    let (_session_id, plan_id, mut ws) = prepare_plan_accept_outline_and_select_serial(&app).await;

    let _messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_draft_confirm"
        })
    })
    .await;

    ws.send(Message::Text(
        json!({
            "type": "work_item_draft_decision",
            "outline_id": "outline_backend_session",
            "decision": "rewrite",
            "feedback": "请收窄后端会话 API 的范围"
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send draft rewrite");

    let _messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "artifact_update"
                && message["draft_candidate"]["draft_record"]["candidate"]["title"]
                    == "重写后的后端登录会话 API"
        })
    })
    .await;

    let draft_store = WorkItemPlanStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let mut drafts = draft_store
        .list_draft_records("project_0001", "issue_0001", &plan_id)
        .expect("list drafts");
    drafts.sort_by(|left, right| left.draft_id.cmp(&right.draft_id));
    assert_eq!(drafts.len(), 2);
    let old = &drafts[0];
    let new = &drafts[1];
    assert_eq!(old.status, WorkItemDraftStatus::Superseded);
    assert!(!old.active);
    assert_eq!(
        old.superseded_by_draft_id.as_deref(),
        Some(new.draft_id.as_str())
    );
    assert_eq!(
        old.supersede_reason,
        Some(WorkItemDraftSupersedeReason::DirectRewrite)
    );
    assert!(old.superseded_at.is_some());
    assert_eq!(new.status, WorkItemDraftStatus::Draft);
    assert!(new.active);
    assert_eq!(new.attempt_index, old.attempt_index + 1);
    let (has_feedback_prompt, captured_prompts) = {
        let captured_prompts = prompts.lock().expect("captured prompts lock");
        (
            captured_prompts.iter().any(|prompt| {
                prompt.contains("[user_or_reviewer_feedback]")
                    && prompt.contains("请收窄后端会话 API 的范围")
            }),
            captured_prompts.clone(),
        )
    };
    assert!(
        has_feedback_prompt,
        "draft rewrite prompt should include user feedback, got {captured_prompts:?}"
    );

    let index = draft_store
        .load_active_index("project_0001", "issue_0001", &plan_id)
        .expect("load active index")
        .expect("active index");
    assert_eq!(
        index
            .outline_to_current_draft_id
            .get("outline_backend_session"),
        Some(&new.draft_id)
    );
    assert_eq!(
        index.draft_statuses.get(&old.draft_id),
        Some(&WorkItemDraftStatus::Superseded)
    );

    ws.close(None).await.ok();
}

#[tokio::test]
async fn draft_pause_enters_human_confirm_without_regenerating() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls();
    let (app, root, _prompts) = app_with_confirmed_story_and_design_and_streaming_outputs(vec![
        valid_outline_output(),
        valid_draft_output("outline_backend_session"),
    ])
    .await;
    let (_session_id, plan_id, mut ws) = prepare_plan_accept_outline_and_select_serial(&app).await;

    let _messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_draft_confirm"
        })
    })
    .await;

    ws.send(Message::Text(
        json!({
            "type": "work_item_draft_decision",
            "outline_id": "outline_backend_session",
            "decision": "pause",
            "feedback": null
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send draft pause");

    let messages = recv_ws_until(&mut ws, Duration::from_secs(5), |messages| {
        messages
            .iter()
            .any(|message| message["type"] == "stage_change" && message["stage"] == "human_confirm")
    })
    .await;
    assert!(
        messages
            .iter()
            .any(|message| message["type"] == "stage_change" && message["stage"] == "human_confirm"),
        "pause should enter human_confirm, got {messages:?}"
    );

    let drafts = WorkItemPlanStore::new(ProductAppPaths::new(root.path().join(".aria")))
        .list_draft_records("project_0001", "issue_0001", &plan_id)
        .expect("list drafts");
    assert_eq!(drafts.len(), 1);

    ws.close(None).await.ok();
}

fn valid_draft_output(outline_id: &str) -> Value {
    valid_draft_output_with_title(outline_id, "实现后端登录会话 API")
}

fn valid_draft_output_with_title(outline_id: &str, title: &str) -> Value {
    json!({
        "draft": {
            "outline_id": outline_id,
            "title": title,
            "kind": "backend",
            "goal": "提供登录会话过期检测与刷新相关 API。",
            "implementation_context": "实现 product service 与 web handler，返回稳定 DTO。",
            "exclusive_write_scopes": ["src/product/session.rs", "src/web/session_handlers.rs"],
            "forbidden_write_scopes": ["web/**"],
            "depends_on_outline_ids": [],
            "required_handoff_from_outline_ids": [],
            "handoff_summary": "输出 SessionStatusDto 与错误语义。",
            "verification_plan": {
                "commands": [
                    {
                        "id": "cmd_backend_session",
                        "label": "cargo test session",
                        "command": "cargo test --locked --lib session",
                        "cwd": "",
                        "purpose": "验证后端 session 逻辑",
                        "required": true,
                        "timeout_seconds": 120,
                        "safety": "approved",
                        "source": "provider"
                    }
                ],
                "manual_checks": [],
                "required_gates": ["cmd_backend_session"]
            }
        }
    })
}

fn valid_frontend_draft_output() -> Value {
    json!({
        "draft": {
            "outline_id": "outline_frontend_expiry",
            "title": "实现前端会话过期提示",
            "kind": "frontend",
            "goal": "在前端展示会话过期提示并触发重新登录入口。",
            "implementation_context": "消费后端会话状态 DTO，展示稳定 UI 状态。",
            "exclusive_write_scopes": ["web/src/session/expiry.ts"],
            "forbidden_write_scopes": ["src/product/**"],
            "depends_on_outline_ids": ["outline_backend_session"],
            "required_handoff_from_outline_ids": ["outline_backend_session"],
            "handoff_summary": "输出前端会话过期提示组件。",
            "verification_plan": {
                "commands": [
                    {
                        "id": "cmd_frontend_session",
                        "label": "pnpm web test",
                        "command": "pnpm -C web test",
                        "cwd": "",
                        "purpose": "验证前端 session UI",
                        "required": true,
                        "timeout_seconds": 120,
                        "safety": "approved",
                        "source": "provider"
                    }
                ],
                "manual_checks": [],
                "required_gates": ["cmd_frontend_session"]
            }
        }
    })
}

fn invalid_draft_output_missing_scope(outline_id: &str) -> Value {
    let mut output = valid_draft_output(outline_id);
    output["draft"]["exclusive_write_scopes"] = json!([]);
    output
}
