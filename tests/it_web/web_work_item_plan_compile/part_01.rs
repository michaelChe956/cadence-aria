use std::fs;

use axum::http::Method;
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::lifecycle_store::LifecycleStore;
use cadence_aria::product::models::{
    IssueWorkItemPlanStatus, ProviderName, WorkItemDraftStatus, WorkItemGenerationMode,
    WorkItemPlanCommitState, WorkItemPlanCompileStatus, WorkspaceType,
};
use cadence_aria::product::work_item_plan_store::WorkItemPlanStore;
use cadence_aria::web::workspace_ws_types::{
    ProviderConfigSnapshot, TimelineNode, TimelineNodeStatus, TimelineNodeType,
    WorkspaceStage as WsWorkspaceStage,
};
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
            "reviewer_enabled": false
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

#[tokio::test]
async fn batch_accept_all_runs_final_compile_and_materializes_entities() {
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

    let lifecycle = LifecycleStore::new(ProductAppPaths::new(root.path().join(".aria")));
    assert!(
        lifecycle
            .list_work_items("project_0001", "issue_0001")
            .expect("list work items before compile")
            .is_empty(),
        "Draft 阶段不能提前写入真实 WorkItem"
    );
    assert!(
        lifecycle
            .list_verification_plans("project_0001", "issue_0001")
            .expect("list verification plans before compile")
            .is_empty(),
        "Draft 阶段不能提前写入真实 VerificationPlan"
    );

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
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_plan_compile"
        }) && messages
            .iter()
            .any(|message| message["type"] == "stage_change" && message["stage"] == "human_confirm")
    })
    .await;
    assert!(
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_plan_compile"
        }),
        "accept_all should enter work_item_plan_compile, got {messages:?}"
    );

    let work_items = lifecycle
        .list_work_items("project_0001", "issue_0001")
        .expect("list work items after compile");
    let verification_plans = lifecycle
        .list_verification_plans("project_0001", "issue_0001")
        .expect("list verification plans after compile");
    let plan = lifecycle
        .get_issue_work_item_plan("project_0001", "issue_0001", &plan_id)
        .expect("get compiled plan");
    let child_sessions = lifecycle
        .list_workspace_sessions("project_0001", "issue_0001")
        .expect("list workspace sessions");
    let work_item_sessions: Vec<_> = child_sessions
        .iter()
        .filter(|session| session.workspace_type == WorkspaceType::WorkItem)
        .collect();

    assert_eq!(work_items.len(), 3);
    assert_eq!(verification_plans.len(), 3);
    assert_eq!(work_item_sessions.len(), 3);
    assert_eq!(plan.status, IssueWorkItemPlanStatus::Confirmed);
    assert_eq!(plan.work_item_ids.len(), 3);
    assert_eq!(plan.verification_plan_ids.len(), 3);
    assert_eq!(plan.dependency_graph.len(), 2);

    let store = WorkItemPlanStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let index = store
        .load_active_index("project_0001", "issue_0001", &plan_id)
        .expect("load active index")
        .expect("active index");
    let compile_dir = root
        .path()
        .join(".aria/projects/project_0001/issues/issue_0001/work_item_plan_compiles")
        .join(&plan_id);
    let compile_files: Vec<_> = fs::read_dir(&compile_dir)
        .expect("read compile tx dir")
        .collect::<Result<Vec<_>, _>>()
        .expect("compile dir entries");
    assert_eq!(compile_files.len(), 1);
    let compile_tx: Value =
        serde_json::from_slice(&fs::read(compile_files[0].path()).expect("read compile tx"))
            .expect("compile tx json");
    assert_eq!(compile_tx["status"], "committed");
    assert_eq!(compile_tx["plan_commit_state"], "committed");
    assert_eq!(compile_tx["previous_plan_snapshot"]["status"], "draft");
    assert_eq!(
        compile_tx["active_draft_ids"]
            .as_array()
            .expect("draft ids")
            .len(),
        index.outline_to_current_draft_id.len()
    );

    ws.send(Message::Text(
        json!({ "type": "human_confirm", "decision": "confirm" })
            .to_string()
            .into(),
    ))
    .await
    .expect("send final human confirm");
    let completed_messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages
            .iter()
            .any(|message| message["type"] == "stage_change" && message["stage"] == "completed")
    })
    .await;
    assert!(
        completed_messages
            .iter()
            .any(|message| message["type"] == "stage_change" && message["stage"] == "completed"),
        "final human confirm after compile should complete workspace, got {completed_messages:?}"
    );
    let work_item_sessions_after_confirm = lifecycle
        .list_workspace_sessions("project_0001", "issue_0001")
        .expect("list workspace sessions after final confirm")
        .into_iter()
        .filter(|session| session.workspace_type == WorkspaceType::WorkItem)
        .collect::<Vec<_>>();
    assert_eq!(
        work_item_sessions_after_confirm.len(),
        3,
        "final human confirm must not create duplicate WorkItem sessions"
    );

    ws.close(None).await.ok();
}

#[tokio::test]
async fn strict_validator_item_failure_in_batch_returns_batch_confirm_without_real_writes() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls().await;
    let (app, root, _prompts) = app_with_confirmed_story_and_design_and_streaming_outputs(vec![
        valid_outline_output(),
        unsafe_backend_draft_output(),
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
        "strict validator item failure in batch should return batch_confirm, got {messages:?}"
    );

    let lifecycle = LifecycleStore::new(ProductAppPaths::new(root.path().join(".aria")));
    assert!(
        lifecycle
            .list_work_items("project_0001", "issue_0001")
            .expect("list work items after failed compile")
            .is_empty(),
        "failed strict validation must not write real WorkItem records"
    );
    assert!(
        lifecycle
            .list_verification_plans("project_0001", "issue_0001")
            .expect("list verification plans after failed compile")
            .is_empty(),
        "failed strict validation must not write real VerificationPlan records"
    );

    let compile_dir = root
        .path()
        .join(".aria/projects/project_0001/issues/issue_0001/work_item_plan_compiles")
        .join(&plan_id);
    let compile_files: Vec<_> = fs::read_dir(&compile_dir)
        .expect("read compile tx dir")
        .collect::<Result<Vec<_>, _>>()
        .expect("compile dir entries");
    assert_eq!(compile_files.len(), 1);
    let compile_tx: Value =
        serde_json::from_slice(&fs::read(compile_files[0].path()).expect("read compile tx"))
            .expect("compile tx json");
    assert_eq!(compile_tx["status"], "failed");
    assert_eq!(compile_tx["plan_commit_state"], "not_started");
    assert!(
        compile_tx["validator_findings"]
            .as_array()
            .expect("validator findings")
            .iter()
            .any(|finding| finding["code"] == "verification_command_unsafe"),
        "failed compile tx should record unsafe command finding: {compile_tx:?}"
    );

    ws.send(Message::Text(
        json!({
            "type": "work_item_batch_decision",
            "decision": "downgrade_to_serial",
            "feedback": "逐项修复 unsafe command",
            "first_affected_outline_id": "outline_backend_session"
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send downgrade to serial");
    let downgrade_messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_draft_run"
        })
    })
    .await;
    assert!(
        downgrade_messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_draft_run"
                && message["node"]["summary"]
                    .as_str()
                    .is_some_and(|summary| summary.contains("outline_backend_session"))
        }),
        "downgrade_to_serial after strict validation failure should start serial draft run, got {downgrade_messages:?}"
    );

    ws.close(None).await.ok();
}

#[tokio::test]
async fn downgrade_to_serial_copies_unaffected_batch_drafts_and_revalidates() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls().await;
    let (app, root, _prompts) = app_with_confirmed_story_and_design_and_streaming_outputs(vec![
        valid_outline_output(),
        valid_draft_output("outline_backend_session"),
        unsafe_frontend_draft_output(),
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

    let _failed_compile_messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_batch_confirm"
        })
    })
    .await;

    let store = WorkItemPlanStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let index_before = store
        .load_active_index("project_0001", "issue_0001", &plan_id)
        .expect("load active index before downgrade")
        .expect("active index before downgrade");
    let source_frontend_draft_id = index_before
        .outline_to_current_draft_id
        .get("outline_frontend_expiry")
        .expect("frontend batch draft id")
        .clone();
    let source_integration_draft_id = index_before
        .outline_to_current_draft_id
        .get("outline_integration_session")
        .expect("integration batch draft id")
        .clone();

    ws.send(Message::Text(
        json!({
            "type": "work_item_batch_decision",
            "decision": "downgrade_to_serial",
            "feedback": "从前端项开始逐项修复",
            "first_affected_outline_id": "outline_frontend_expiry"
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send downgrade to serial");
    let downgrade_messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_draft_run"
                && message["node"]["summary"]
                    .as_str()
                    .is_some_and(|summary| summary.contains("outline_frontend_expiry"))
        })
    })
    .await;
    assert!(
        downgrade_messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_draft_run"
                && message["node"]["summary"]
                    .as_str()
                    .is_some_and(|summary| summary.contains("outline_frontend_expiry"))
        }),
        "downgrade_to_serial should start from first affected outline, got {downgrade_messages:?}"
    );

    let index_after = store
        .load_active_index("project_0001", "issue_0001", &plan_id)
        .expect("load active index after downgrade")
        .expect("active index after downgrade");
    assert_eq!(
        index_after.active_outline_id.as_deref(),
        Some("outline_frontend_expiry")
    );

    let copied_backend_draft_id = index_after
        .outline_to_current_draft_id
        .get("outline_backend_session")
        .expect("copied backend serial draft id");
    assert_ne!(
        copied_backend_draft_id,
        index_before
            .outline_to_current_draft_id
            .get("outline_backend_session")
            .expect("source backend draft id")
    );
    let copied_backend = store
        .get_draft_record(
            "project_0001",
            "issue_0001",
            &plan_id,
            &index_after.current_generation_round_id,
            copied_backend_draft_id,
        )
        .expect("load copied backend draft");
    assert_eq!(
        copied_backend.generation_mode,
        WorkItemGenerationMode::Serial
    );
    assert_eq!(copied_backend.batch_id, None);
    assert_eq!(copied_backend.status, WorkItemDraftStatus::Accepted);
    assert!(copied_backend.active);
    assert_eq!(
        copied_backend.copied_from_draft_id.as_deref(),
        index_before
            .outline_to_current_draft_id
            .get("outline_backend_session")
            .map(String::as_str)
    );

    assert_eq!(
        index_after
            .outline_to_current_draft_id
            .get("outline_frontend_expiry"),
        Some(&source_frontend_draft_id),
        "affected outline should be regenerated, not copied before serial run"
    );
    assert_eq!(
        index_after
            .outline_to_current_draft_id
            .get("outline_integration_session"),
        Some(&source_integration_draft_id),
        "downstream outline should remain available until its serial turn supersedes it"
    );

    ws.close(None).await.ok();
}

