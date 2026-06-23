use axum::http::Method;
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::lifecycle_store::LifecycleStore;
use cadence_aria::product::models::{IssueWorkItemPlanStatus, WorkspaceType};
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

#[tokio::test]
async fn work_item_plan_serial_flow_outline_to_compile() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls();
    let (app, root, _prompts) = app_with_confirmed_story_and_design_and_streaming_outputs(vec![
        valid_outline_output(),
        valid_draft_output("outline_backend_session"),
        valid_frontend_draft_output(),
        valid_integration_draft_output(),
    ])
    .await;
    let (_session_id, plan_id, mut ws) =
        prepare_plan_accept_outline_and_select_mode(&app, "serial").await;

    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(paths.clone());
    assert!(
        lifecycle
            .list_work_items("project_0001", "issue_0001")
            .expect("list work items before draft")
            .is_empty()
    );
    assert!(
        lifecycle
            .list_verification_plans("project_0001", "issue_0001")
            .expect("list verification plans before draft")
            .is_empty()
    );

    accept_serial_draft(&mut ws, "outline_backend_session").await;
    accept_serial_draft(&mut ws, "outline_frontend_expiry").await;
    assert!(
        lifecycle
            .list_work_items("project_0001", "issue_0001")
            .expect("list work items before final draft")
            .is_empty(),
        "Draft 阶段不能提前写真实 WorkItem"
    );
    accept_serial_draft(&mut ws, "outline_integration_session").await;

    let messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages
            .iter()
            .any(|message| message["type"] == "stage_change" && message["stage"] == "human_confirm")
    })
    .await;
    assert!(
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_plan_compile"
        }),
        "serial flow should create work_item_plan_compile node, got {messages:?}"
    );

    let work_items = lifecycle
        .list_work_items("project_0001", "issue_0001")
        .expect("list compiled work items");
    let verification_plans = lifecycle
        .list_verification_plans("project_0001", "issue_0001")
        .expect("list compiled verification plans");
    let plan = lifecycle
        .get_issue_work_item_plan("project_0001", "issue_0001", &plan_id)
        .expect("get compiled plan");
    let child_sessions = lifecycle
        .list_workspace_sessions("project_0001", "issue_0001")
        .expect("list sessions")
        .into_iter()
        .filter(|session| session.workspace_type == WorkspaceType::WorkItem)
        .collect::<Vec<_>>();
    assert_eq!(work_items.len(), 3);
    assert_eq!(verification_plans.len(), 3);
    assert_eq!(child_sessions.len(), 3);
    assert_eq!(plan.status, IssueWorkItemPlanStatus::Confirmed);

    ws.send(Message::Text(
        json!({ "type": "human_confirm", "decision": "confirm" })
            .to_string()
            .into(),
    ))
    .await
    .expect("send final confirm");
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
        "serial flow should complete workspace, got {completed_messages:?}"
    );

    ws.close(None).await.ok();
}

#[tokio::test]
async fn work_item_plan_batch_flow_with_validation_failed_then_rewrite() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls();
    let (app, root, _prompts) = app_with_confirmed_story_and_design_and_streaming_outputs(vec![
        valid_outline_output(),
        invalid_draft_output_missing_scope("outline_backend_session"),
        invalid_draft_output_missing_scope("outline_backend_session"),
        valid_frontend_draft_output(),
        valid_integration_draft_output(),
        valid_draft_output("outline_backend_session"),
        valid_frontend_draft_output(),
        valid_integration_draft_output(),
    ])
    .await;
    let (_session_id, plan_id, mut ws) =
        prepare_plan_accept_outline_and_select_mode(&app, "batch").await;

    let first_batch_messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_batch_confirm"
        })
    })
    .await;
    let first_batch_state = first_batch_messages
        .iter()
        .find_map(|message| {
            if message["type"] == "artifact_update" && message.get("batch_state").is_some() {
                Some(&message["batch_state"])
            } else {
                None
            }
        })
        .expect("first batch state");
    assert_eq!(
        first_batch_state["failure_summary"][0]["outline_id"],
        "outline_backend_session"
    );

    ws.send(Message::Text(
        json!({
            "type": "work_item_batch_decision",
            "decision": "rewrite_batch",
            "feedback": "修复校验失败后重写整组",
            "first_affected_outline_id": null
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send rewrite batch");

    let second_batch_messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_batch_confirm"
        })
    })
    .await;
    let second_batch_state = second_batch_messages
        .iter()
        .find_map(|message| {
            if message["type"] == "artifact_update" && message.get("batch_state").is_some() {
                Some(&message["batch_state"])
            } else {
                None
            }
        })
        .expect("second batch state");
    assert!(
        second_batch_state["failure_summary"]
            .as_array()
            .expect("failure summary")
            .is_empty()
    );

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
    .expect("send accept all");
    let _messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages
            .iter()
            .any(|message| message["type"] == "stage_change" && message["stage"] == "human_confirm")
    })
    .await;

    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(paths.clone());
    let plan = lifecycle
        .get_issue_work_item_plan("project_0001", "issue_0001", &plan_id)
        .expect("get compiled plan");
    assert_eq!(plan.status, IssueWorkItemPlanStatus::Confirmed);
    assert_eq!(
        lifecycle
            .list_work_items("project_0001", "issue_0001")
            .expect("list work items")
            .len(),
        3
    );
    let index = WorkItemPlanStore::new(paths)
        .load_active_index("project_0001", "issue_0001", &plan_id)
        .expect("load active index")
        .expect("active index");
    assert!(index.batches.len() >= 2);

    ws.close(None).await.ok();
}

#[tokio::test]
async fn session_state_restores_work_item_plan_staged_artifacts() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls();
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
    let current_batch_summary = state["artifact_version_summaries"]
        .as_array()
        .expect("artifact version summaries")
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
        state["artifact_version_summaries"]
            .as_array()
            .expect("artifact version summaries")
            .len(),
        7,
        "artifact_version_summaries should preserve outline, mode, draft, and batch indexes"
    );

    recovered.close(None).await.ok();
}

fn valid_draft_output(outline_id: &str) -> Value {
    json!({
        "draft": {
            "outline_id": outline_id,
            "title": "实现后端登录会话 API",
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

fn valid_integration_draft_output() -> Value {
    json!({
        "draft": {
            "outline_id": "outline_integration_session",
            "title": "集成测试：会话过期端到端",
            "kind": "integration",
            "goal": "覆盖会话过期到前端提示的贯通路径。",
            "implementation_context": "覆盖后端会话 DTO 到前端提示的集成路径。",
            "exclusive_write_scopes": ["tests/session/expiry.rs"],
            "forbidden_write_scopes": [],
            "depends_on_outline_ids": ["outline_frontend_expiry"],
            "required_handoff_from_outline_ids": ["outline_frontend_expiry"],
            "handoff_summary": "输出端到端验证覆盖。",
            "verification_plan": {
                "commands": [
                    {
                        "id": "cmd_integration_session",
                        "label": "cargo test session integration",
                        "command": "cargo test --locked --test it_web session",
                        "cwd": "",
                        "purpose": "验证会话过期贯通路径",
                        "required": true,
                        "timeout_seconds": 120,
                        "safety": "approved",
                        "source": "provider"
                    }
                ],
                "manual_checks": [],
                "required_gates": ["cmd_integration_session"]
            }
        }
    })
}

fn invalid_draft_output_missing_scope(outline_id: &str) -> Value {
    let mut output = valid_draft_output(outline_id);
    output["draft"]["exclusive_write_scopes"] = json!([]);
    output
}
