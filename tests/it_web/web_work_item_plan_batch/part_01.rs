use axum::http::Method;
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::models::{WorkItemBatchStatus, WorkItemGenerationMode};
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

async fn prepare_plan_accept_outline_and_select_batch(
    app: &axum::Router,
) -> (String, String, WsStream) {
    prepare_plan_accept_outline_and_select_batch_with_reviewer(app, false).await
}

async fn prepare_plan_accept_outline_and_select_batch_with_reviewer(
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
        enable_work_item_plan_review_fixture(app, &session_id, outline_review_pass()).await;
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
        json!({ "type": "select_work_item_generation_mode", "mode": "batch" })
            .to_string()
            .into(),
    ))
    .await
    .expect("send batch mode");

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

fn batch_review_pass() -> Value {
    json!({
        "verdict": "pass",
        "review_scope": "batch",
        "summary": "Batch review 通过",
        "generation_round_id": "round_001",
        "affects_items": [
            { "target_outline_id": "outline_backend_session" },
            { "target_outline_id": "outline_frontend_expiry" },
            { "target_outline_id": "outline_integration_session" }
        ],
        "findings": []
    })
}

fn batch_review_revise() -> Value {
    json!({
        "verdict": "revise_batch",
        "review_scope": "batch",
        "summary": "Batch review 要求整组返修",
        "generation_round_id": "round_001",
        "affects_items": [
            { "target_outline_id": "outline_backend_session" },
            { "target_outline_id": "outline_frontend_expiry" }
        ],
        "findings": []
    })
}

fn batch_review_plan_reopen() -> Value {
    json!({
        "verdict": "plan_reopen_required",
        "review_scope": "batch",
        "summary": "Batch review 要求重开 Outline",
        "generation_round_id": "round_001",
        "affects_items": [
            { "target_outline_id": "outline_backend_session" },
            { "target_outline_id": "outline_frontend_expiry" },
            { "target_outline_id": "outline_integration_session" }
        ],
        "findings": []
    })
}

#[tokio::test]
async fn batch_mode_creates_batch_record_for_current_round() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls().await;
    let (app, root, _prompts) =
        app_with_confirmed_story_and_design_and_streaming_outputs(vec![valid_outline_output()])
            .await;
    let (_session_id, plan_id, mut ws) = prepare_plan_accept_outline_and_select_batch(&app).await;

    let messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_batch_run"
        })
    })
    .await;
    assert!(
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_batch_run"
        }),
        "batch mode should enter batch run, got {messages:?}"
    );

    let index = WorkItemPlanStore::new(ProductAppPaths::new(root.path().join(".aria")))
        .load_active_index("project_0001", "issue_0001", &plan_id)
        .expect("load active index")
        .expect("active index");
    assert_eq!(index.batches.len(), 1);
    let batch = &index.batches[0];
    assert_eq!(batch.generation_round_id, "round_001");
    assert_eq!(batch.mode, WorkItemGenerationMode::Batch);
    assert_eq!(batch.status, WorkItemBatchStatus::Generating);
    assert!(batch.item_draft_ids.is_empty());
    assert!(batch.validation_failed_ids.is_empty());

    ws.close(None).await.ok();
}

#[tokio::test]
async fn batch_generation_invokes_one_provider_run_per_outline() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls().await;
    let (app, root, prompts) = app_with_confirmed_story_and_design_and_streaming_outputs(vec![
        valid_outline_output(),
        valid_draft_output("outline_backend_session"),
        valid_frontend_draft_output(),
        valid_integration_draft_output(),
    ])
    .await;
    let (_session_id, plan_id, mut ws) = prepare_plan_accept_outline_and_select_batch(&app).await;

    let messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_batch_confirm"
        })
    })
    .await;
    assert!(
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_batch_confirm"
        }),
        "batch generation should enter batch confirm, got {messages:?}"
    );
    let batch_state = messages
        .iter()
        .find_map(|message| {
            if message["type"] == "artifact_update" && message.get("batch_state").is_some() {
                Some(&message["batch_state"])
            } else {
                None
            }
        })
        .expect("batch confirm should publish batch_state artifact");
    assert_eq!(batch_state["batch_status"], "completed");
    assert_eq!(
        batch_state["queue"],
        json!([
            "outline_backend_session",
            "outline_frontend_expiry",
            "outline_integration_session"
        ])
    );
    assert_eq!(batch_state["draft_records"].as_array().unwrap().len(), 3);
    assert!(
        batch_state["failure_summary"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        !messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_draft_confirm"
        }),
        "batch generation must not enter per-item draft confirm"
    );

    let store = WorkItemPlanStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let drafts = store
        .list_draft_records("project_0001", "issue_0001", &plan_id)
        .expect("list drafts");
    let outline_ids: Vec<&str> = drafts
        .iter()
        .map(|draft| draft.outline_id.as_str())
        .collect();
    assert_eq!(
        outline_ids,
        vec![
            "outline_backend_session",
            "outline_frontend_expiry",
            "outline_integration_session"
        ]
    );

    let index = store
        .load_active_index("project_0001", "issue_0001", &plan_id)
        .expect("load active index")
        .expect("active index");
    let batch = index.batches.last().expect("batch record");
    assert_eq!(batch.status, WorkItemBatchStatus::Completed);
    assert_eq!(batch.item_draft_ids.len(), 3);
    assert!(batch.validation_failed_ids.is_empty());

    let (
        prompt_count,
        frontend_prompt_has_backend_context,
        integration_prompt_has_frontend_context,
    ) = {
        let captured_prompts = prompts.lock().unwrap();
        (
            captured_prompts.len(),
            captured_prompts[2].contains("outline_backend_session")
                && captured_prompts[2].contains("输出 SessionStatusDto"),
            captured_prompts[3].contains("outline_frontend_expiry")
                && captured_prompts[3].contains("输出前端会话过期提示组件"),
        )
    };
    assert_eq!(prompt_count, 4, "outline author + 3 item drafts");
    assert!(
        frontend_prompt_has_backend_context,
        "frontend batch prompt should include previous batch backend draft context"
    );
    assert!(
        integration_prompt_has_frontend_context,
        "integration batch prompt should include previous batch frontend draft context"
    );

    ws.close(None).await.ok();
}

#[tokio::test]
async fn batch_local_validation_failure_retries_once() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls().await;
    let (app, root, prompts) = app_with_confirmed_story_and_design_and_streaming_outputs(vec![
        valid_outline_output(),
        invalid_draft_output_missing_scope("outline_backend_session"),
        valid_draft_output("outline_backend_session"),
        valid_frontend_draft_output(),
        valid_integration_draft_output(),
    ])
    .await;
    let (_session_id, plan_id, mut ws) = prepare_plan_accept_outline_and_select_batch(&app).await;

    let messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_batch_confirm"
        })
    })
    .await;
    assert!(
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_batch_confirm"
        }),
        "batch retry should still reach batch confirm, got {messages:?}"
    );

    let store = WorkItemPlanStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let drafts = store
        .list_draft_records("project_0001", "issue_0001", &plan_id)
        .expect("list drafts");
    assert_eq!(drafts.len(), 3);
    assert!(
        drafts
            .iter()
            .any(|draft| draft.outline_id == "outline_backend_session"
                && draft.status == cadence_aria::product::models::WorkItemDraftStatus::Draft)
    );

    let index = store
        .load_active_index("project_0001", "issue_0001", &plan_id)
        .expect("load active index")
        .expect("active index");
    let batch = index.batches.last().expect("batch record");
    assert_eq!(batch.item_draft_ids.len(), 3);
    assert!(batch.validation_failed_ids.is_empty());

    let prompt_count = prompts.lock().unwrap().len();
    assert_eq!(
        prompt_count, 5,
        "outline author + failed draft + retry + 2 drafts"
    );

    ws.close(None).await.ok();
}

#[tokio::test]
async fn batch_local_validation_second_failure_marks_validation_failed_and_continues() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls().await;
    let (app, root, prompts) = app_with_confirmed_story_and_design_and_streaming_outputs(vec![
        valid_outline_output(),
        invalid_draft_output_missing_scope("outline_backend_session"),
        invalid_draft_output_missing_scope("outline_backend_session"),
        valid_frontend_draft_output(),
        valid_integration_draft_output(),
    ])
    .await;
    let (_session_id, plan_id, mut ws) = prepare_plan_accept_outline_and_select_batch(&app).await;

    let messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_batch_confirm"
        })
    })
    .await;
    assert!(
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_batch_confirm"
        }),
        "batch should continue after second validation failure, got {messages:?}"
    );
    let batch_state = messages
        .iter()
        .find_map(|message| {
            if message["type"] == "artifact_update" && message.get("batch_state").is_some() {
                Some(&message["batch_state"])
            } else {
                None
            }
        })
        .expect("batch confirm should publish batch_state artifact");
    assert_eq!(batch_state["batch_status"], "completed");
    assert_eq!(batch_state["failure_summary"].as_array().unwrap().len(), 1);
    assert_eq!(
        batch_state["failure_summary"][0]["outline_id"],
        "outline_backend_session"
    );
    assert_eq!(
        batch_state["failure_summary"][0]["status"],
        "validation_failed"
    );

    let store = WorkItemPlanStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let drafts = store
        .list_draft_records("project_0001", "issue_0001", &plan_id)
        .expect("list drafts");
    assert_eq!(drafts.len(), 3);
    let failed = drafts
        .iter()
        .find(|draft| draft.outline_id == "outline_backend_session")
        .expect("failed backend draft");
    assert_eq!(
        failed.status,
        cadence_aria::product::models::WorkItemDraftStatus::ValidationFailed
    );

    let index = store
        .load_active_index("project_0001", "issue_0001", &plan_id)
        .expect("load active index")
        .expect("active index");
    let batch = index.batches.last().expect("batch record");
    assert_eq!(batch.status, WorkItemBatchStatus::Completed);
    assert_eq!(batch.item_draft_ids.len(), 2);
    assert_eq!(batch.validation_failed_ids, vec![failed.draft_id.clone()]);

    let prompt_count = prompts.lock().unwrap().len();
    assert_eq!(
        prompt_count, 5,
        "outline author + failed draft + retry + 2 drafts"
    );

    ws.close(None).await.ok();
}

#[tokio::test]
async fn batch_confirm_accept_all_marks_all_valid_drafts_accepted() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls().await;
    let (app, root, _prompts) = app_with_confirmed_story_and_design_and_streaming_outputs(vec![
        valid_outline_output(),
        valid_draft_output("outline_backend_session"),
        valid_frontend_draft_output(),
        valid_integration_draft_output(),
    ])
    .await;
    let (_session_id, plan_id, mut ws) = prepare_plan_accept_outline_and_select_batch(&app).await;

    let _messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_batch_confirm"
        })
    })
    .await;
    ws.send(Message::Text(
        json!({
            "type": "work_item_batch_decision",
            "decision": "accept_all",
            "feedback": null,
            "first_affected_outline_id": null
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send batch accept all");

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
        "accept_all should enter final compile placeholder, got {messages:?}"
    );

    let store = WorkItemPlanStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let drafts = store
        .list_draft_records("project_0001", "issue_0001", &plan_id)
        .expect("list drafts");
    assert_eq!(drafts.len(), 3);
    assert!(drafts.iter().all(|draft| {
        draft.status == cadence_aria::product::models::WorkItemDraftStatus::Accepted
    }));

    let index = store
        .load_active_index("project_0001", "issue_0001", &plan_id)
        .expect("load active index")
        .expect("active index");
    assert!(
        index.draft_statuses.values().all(|status| {
            status == &cadence_aria::product::models::WorkItemDraftStatus::Accepted
        })
    );

    ws.close(None).await.ok();
}

#[tokio::test]
async fn batch_accept_enters_batch_review_when_reviewer_enabled() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls().await;
    let (app, _root, _prompts) = app_with_confirmed_story_and_design_and_streaming_outputs(vec![
        valid_outline_output(),
        valid_draft_output("outline_backend_session"),
        valid_frontend_draft_output(),
        valid_integration_draft_output(),
    ])
    .await;
    let (session_id, _plan_id, mut ws) =
        prepare_plan_accept_outline_and_select_batch_with_reviewer(&app, true).await;

    let _messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_batch_confirm"
        })
    })
    .await;
    enable_work_item_plan_review_fixture(&app, &session_id, batch_review_pass()).await;

    ws.send(Message::Text(
        json!({
            "type": "work_item_batch_decision",
            "decision": "accept_all",
            "feedback": null,
            "first_affected_outline_id": null
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send batch accept all");

    let messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_batch_review"
        })
    })
    .await;
    assert!(
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_batch_review"
        }),
        "accept_all should enter batch review when reviewer is enabled, got {messages:?}"
    );

    ws.close(None).await.ok();
}

