use axum::http::Method;
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::lifecycle_store::LifecycleStore;
use cadence_aria::product::models::{IssueWorkItemPlanStatus, WorkspaceType};
use cadence_aria::product::work_item_plan_store::WorkItemPlanStore;
use cadence_aria::product::work_item_revision_store::WorkItemRevisionStore;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::time::{Duration, timeout};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

use crate::web_work_item_generation::{
    app_with_confirmed_story_and_design_and_streaming_outputs, request_json,
    valid_canonical_draft_output, valid_outline_output,
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

async fn prepare_plan_accept_outline_and_select_mode(
    app: &axum::Router,
    mode: &str,
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
        json!({ "type": "select_work_item_generation_mode", "mode": mode })
            .to_string()
            .into(),
    ))
    .await
    .expect("send generation mode");

    (session_id, plan_id, ws)
}

async fn accept_serial_draft(ws: &mut WsStream, outline_id: &str) {
    let _messages = recv_ws_until(ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_draft_confirm"
        })
    })
    .await;
    ws.send(Message::Text(
        json!({
            "type": "work_item_draft_decision",
            "outline_id": outline_id,
            "decision": "accept",
            "feedback": null
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send draft accept");
}

// 退役留档（T5/REQ-RET-02）：`work_item_plan_serial_flow_outline_to_compile` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// 退役留档（T5/REQ-RET-02）：`work_item_plan_batch_flow_with_validation_failed_then_rewrite` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

#[tokio::test]
async fn session_state_restores_work_item_plan_staged_artifacts() {
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
        prepare_plan_accept_outline_and_select_mode(&app, "batch").await;

    let _messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_batch_confirm"
        })
    })
    .await;
    ws.close(None).await.ok();

    let mut recovered = connect_ws(app.clone(), &session_id).await;
    let state_messages = recv_ws_until(&mut recovered, Duration::from_secs(10), |messages| {
        messages
            .iter()
            .any(|message| message["type"] == "session_state")
    })
    .await;
    let state = state_messages
        .iter()
        .find(|message| message["type"] == "session_state")
        .expect("session_state after reconnect");
    assert_eq!(state["stage"], "author_confirm");
    let active_node_id = state["active_node_id"].as_str().expect("active node id");
    let active_node = state["timeline_nodes"]
        .as_array()
        .expect("timeline nodes")
        .iter()
        .find(|node| node["node_id"] == active_node_id)
        .expect("active node exists in timeline");
    assert_eq!(active_node["node_type"], "work_item_batch_confirm");
    assert_eq!(
        state["artifact"]["batch_state"]["batch_status"],
        "completed"
    );
    assert_eq!(
        state["artifact"]["batch_state"]["queue"],
        json!([
            "outline_backend_session",
            "outline_frontend_expiry",
            "outline_integration_session"
        ])
    );
    assert_eq!(
        state["artifact"]["batch_state"]["active_outline_id"],
        Value::Null
    );
    let artifact_summaries = state["artifact_version_summaries"]
        .as_array()
        .expect("artifact version summaries");
    let current_batch_summary = artifact_summaries
        .iter()
        .find(|summary| {
            summary["is_current"] == true
                && summary["markdown_preview"]
                    .as_str()
                    .is_some_and(|preview| preview.contains("(3 drafts)"))
        })
        .expect("current batch_state artifact summary");
    let batch_source_node_id = current_batch_summary["source_node_id"]
        .as_str()
        .expect("batch artifact source node id");
    let batch_source_node = state["timeline_nodes"]
        .as_array()
        .expect("timeline nodes")
        .iter()
        .find(|node| node["node_id"] == batch_source_node_id)
        .expect("batch artifact source node exists in timeline");
    assert_eq!(batch_source_node["node_type"], "work_item_batch_run");
    assert_eq!(
        artifact_summaries.len(),
        5,
        "artifact_version_summaries should preserve outline, draft, and batch indexes without duplicating mode metadata"
    );
    for expected_preview in [
        "先实现后端会话 API",
        "outline_backend_session",
        "outline_frontend_expiry",
        "outline_integration_session",
        "(3 drafts)",
    ] {
        assert!(
            artifact_summaries.iter().any(|summary| {
                summary["markdown_preview"]
                    .as_str()
                    .is_some_and(|preview| preview.contains(expected_preview))
            }),
            "missing artifact summary preview containing {expected_preview}"
        );
    }

    recovered.close(None).await.ok();
}

fn valid_draft_output(outline_id: &str) -> Value {
    valid_canonical_draft_output(outline_id, "实现后端登录会话 API")
}

fn valid_frontend_draft_output() -> Value {
    valid_canonical_draft_output("outline_frontend_expiry", "实现前端会话过期提示")
}

fn valid_integration_draft_output() -> Value {
    let mut output =
        valid_canonical_draft_output("outline_integration_session", "集成测试：会话过期端到端");
    output["draft"]["canonical_contract"]["handoff_contract"]["provided_contract_refs"] = json!([]);
    output
}

fn assert_initial_plan_revision_published(
    paths: &ProductAppPaths,
    plan_id: &str,
    expected_work_item_count: usize,
) {
    let revision_store = WorkItemRevisionStore::new(paths.clone());
    let lineage = revision_store
        .get_plan_lineage("project_0001", "issue_0001", plan_id)
        .expect("load active plan lineage");
    let active_revision_id = lineage
        .active_revision_id
        .as_deref()
        .expect("active plan revision id");
    let revision = revision_store
        .get_plan_revision("project_0001", "issue_0001", plan_id, active_revision_id)
        .expect("load active plan revision");
    assert_eq!(revision.work_item_bindings.len(), expected_work_item_count);
    let plan_projection = revision_store
        .get_plan_projection_bundle(&lineage, &revision.plan_projection_bundle_id)
        .expect("load active plan projection");
    assert_eq!(
        plan_projection
            .coder_group_context
            .ordered_logical_work_item_ids
            .len(),
        expected_work_item_count
    );
    for (logical_work_item_id, revision_id) in &revision.work_item_bindings {
        let work_item_revision = revision_store
            .get_work_item_revision(&lineage, logical_work_item_id, revision_id)
            .expect("load active work item revision");
        revision_store
            .get_verification_plan_revision(
                &lineage,
                &work_item_revision.verification_plan_revision_id,
            )
            .expect("load active verification plan revision");
        let projection = revision_store
            .get_work_item_projection_bundle(
                &lineage,
                &work_item_revision.work_item_projection_bundle_id,
            )
            .expect("load active work item projection");
        assert_eq!(projection.work_item_revision_id, work_item_revision.id);
    }
}

fn invalid_draft_output_missing_scope(outline_id: &str) -> Value {
    let mut output = valid_draft_output(outline_id);
    output["draft"]["canonical_contract"]["write_policy"]["exclusive_scopes"] = json!([]);
    output
}
