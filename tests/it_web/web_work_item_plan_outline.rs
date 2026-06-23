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
    QueuedSplitOutput, app_with_confirmed_story_and_design_and_streaming_outputs,
    app_with_confirmed_story_and_design_and_streaming_raw_outputs, context_blocker_outline_output,
    invalid_outline_output_duplicate_ids, malformed_outline_structured_stdout, request_json,
    valid_outline_output,
};

static WS_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

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

async fn prepare_plan_and_start(app: &axum::Router) -> (String, String, WsStream) {
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
            "provider_config": { "author": "fake", "reviewer": null, "review_rounds": 1 },
            "reviewer_enabled": false
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send start_generation");

    (session_id, plan_id, ws)
}

#[tokio::test]
async fn work_item_plan_start_generation_creates_outline_run_node() {
    let _guard = WS_TEST_LOCK.lock().await;
    let (app, _repo, _prompts) =
        app_with_confirmed_story_and_design_and_streaming_outputs(vec![valid_outline_output()])
            .await;
    let (_session_id, _plan_id, mut ws) = prepare_plan_and_start(&app).await;

    let messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_plan_outline_run"
        })
    })
    .await;

    let outline_run = messages
        .iter()
        .find(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_plan_outline_run"
        })
        .expect("outline run node");
    assert_eq!(outline_run["node"]["stage"], "running");

    ws.close(None).await.ok();
}

#[tokio::test]
async fn valid_outline_enters_outline_confirm() {
    let _guard = WS_TEST_LOCK.lock().await;
    let (app, _repo, _prompts) =
        app_with_confirmed_story_and_design_and_streaming_outputs(vec![valid_outline_output()])
            .await;
    let (_session_id, _plan_id, mut ws) = prepare_plan_and_start(&app).await;

    let messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "artifact_update" && message.get("outline_candidate").is_some()
        }) && messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_plan_outline_confirm"
        }) && messages.iter().any(|message| {
            message["type"] == "stage_change" && message["stage"] == "author_confirm"
        })
    })
    .await;

    let artifact = messages
        .iter()
        .find(|message| {
            message["type"] == "artifact_update" && message.get("outline_candidate").is_some()
        })
        .expect("outline artifact");
    assert_eq!(
        artifact["outline_candidate"]["outline"]["work_item_outlines"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert!(
        artifact["outline_candidate"]["design_context_gaps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|gap| gap == "missing_architecture"),
        "legacy design gaps should be exposed on outline candidate: {artifact}"
    );
    assert!(artifact.get("candidate").is_none());

    ws.close(None).await.ok();
}

#[tokio::test]
async fn context_blockers_enter_context_blocker_node() {
    let _guard = WS_TEST_LOCK.lock().await;
    let (app, _repo, _prompts) = app_with_confirmed_story_and_design_and_streaming_outputs(vec![
        context_blocker_outline_output(),
    ])
    .await;
    let (_session_id, _plan_id, mut ws) = prepare_plan_and_start(&app).await;

    let messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "artifact_update" && message.get("context_blocker").is_some()
        }) && messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_plan_context_blocker"
        }) && messages
            .iter()
            .any(|message| message["type"] == "stage_change" && message["stage"] == "human_confirm")
    })
    .await;

    let artifact = messages
        .iter()
        .find(|message| {
            message["type"] == "artifact_update" && message.get("context_blocker").is_some()
        })
        .expect("context blocker artifact");
    assert_eq!(
        artifact["context_blocker"]["context_blockers"][0]["code"],
        "missing_module_boundary"
    );

    ws.close(None).await.ok();
}

#[tokio::test]
async fn context_blocker_confirm_is_rejected() {
    let _guard = WS_TEST_LOCK.lock().await;
    let (app, _repo, _prompts) = app_with_confirmed_story_and_design_and_streaming_outputs(vec![
        context_blocker_outline_output(),
    ])
    .await;
    let (_session_id, _plan_id, mut ws) = prepare_plan_and_start(&app).await;

    let _blocker_messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_plan_context_blocker"
        })
    })
    .await;

    ws.send(Message::Text(
        json!({
            "type": "human_confirm",
            "decision": "confirm"
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send confirm");

    let messages = recv_ws_until(&mut ws, Duration::from_secs(5), |messages| {
        messages
            .iter()
            .any(|message| message["type"] == "protocol_error")
    })
    .await;
    assert!(messages.iter().any(|message| {
        message["type"] == "protocol_error"
            && message["message"]
                .as_str()
                .unwrap_or("")
                .contains("context blocker")
    }));
    assert!(messages.iter().all(|message| {
        !(message["type"] == "stage_change" && message["stage"] == "completed")
    }));

    ws.close(None).await.ok();
}

#[tokio::test]
async fn outline_validation_failure_auto_retries_then_human_blocker() {
    let _guard = WS_TEST_LOCK.lock().await;
    let (app, _repo, _prompts) = app_with_confirmed_story_and_design_and_streaming_outputs(vec![
        invalid_outline_output_duplicate_ids(),
        invalid_outline_output_duplicate_ids(),
    ])
    .await;
    let (_session_id, _plan_id, mut ws) = prepare_plan_and_start(&app).await;

    let messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        let outline_runs = messages
            .iter()
            .filter(|message| {
                message["type"] == "timeline_node_created"
                    && message["node"]["node_type"] == "work_item_plan_outline_run"
            })
            .count();
        outline_runs >= 2
            && messages.iter().any(|message| {
                message["type"] == "timeline_node_created"
                    && message["node"]["node_type"] == "work_item_plan_context_blocker"
            })
    })
    .await;

    let outline_run_count = messages
        .iter()
        .filter(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_plan_outline_run"
        })
        .count();
    assert_eq!(outline_run_count, 2);
    assert!(messages.iter().any(|message| {
        message["type"] == "artifact_update" && message.get("context_blocker").is_some()
    }));

    ws.close(None).await.ok();
}

#[tokio::test]
async fn outline_structured_json_parse_failure_auto_retries_then_accepts_valid_outline() {
    let _guard = WS_TEST_LOCK.lock().await;
    let (app, _repo, prompts) =
        app_with_confirmed_story_and_design_and_streaming_raw_outputs(vec![
            QueuedSplitOutput::RawStdout(malformed_outline_structured_stdout()),
            QueuedSplitOutput::Json(valid_outline_output()),
        ])
        .await;
    let (_session_id, _plan_id, mut ws) = prepare_plan_and_start(&app).await;

    let messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        let outline_runs = messages
            .iter()
            .filter(|message| {
                message["type"] == "timeline_node_created"
                    && message["node"]["node_type"] == "work_item_plan_outline_run"
            })
            .count();
        outline_runs >= 2
            && messages.iter().any(|message| {
                message["type"] == "artifact_update" && message.get("outline_candidate").is_some()
            })
            && messages.iter().any(|message| {
                message["type"] == "timeline_node_created"
                    && message["node"]["node_type"] == "work_item_plan_outline_confirm"
            })
    })
    .await;

    let outline_run_count = messages
        .iter()
        .filter(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_plan_outline_run"
        })
        .count();
    assert_eq!(outline_run_count, 2);
    assert!(
        messages.iter().all(|message| message["type"] != "error"),
        "parse failure should be handled as auto retry, not terminal websocket error: {messages:?}"
    );
    {
        let prompts = prompts.lock().expect("prompts lock");
        assert_eq!(
            prompts.len(),
            2,
            "invalid structured JSON should trigger one fresh outline provider attempt"
        );
        assert!(
            prompts[1].contains("[revision_feedback]")
                && prompts[1].contains("outline_structured_output_parse_error"),
            "retry prompt should explain the structured JSON parse failure: {}",
            prompts[1]
        );
    }

    ws.close(None).await.ok();
}

#[tokio::test]
async fn context_blocker_human_resolution_appends_index_and_next_prompt() {
    let _guard = WS_TEST_LOCK.lock().await;
    let (app, repo, prompts) = app_with_confirmed_story_and_design_and_streaming_outputs(vec![
        context_blocker_outline_output(),
        valid_outline_output(),
    ])
    .await;
    let (_session_id, plan_id, mut ws) = prepare_plan_and_start(&app).await;

    let _blocker_messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_plan_context_blocker"
        })
    })
    .await;

    ws.send(Message::Text(
        json!({
            "type": "human_confirm",
            "decision": "request-change",
            "payload": {
                "description": "模块边界：会话 API 位于 src/product/session.rs；测试策略使用 cargo test --locked --lib session。"
            }
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send context resolution");

    let messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        let saw_resolution_artifact = messages.iter().any(|message| {
            message["type"] == "artifact_update"
                && message.get("markdown").is_some()
                && message["markdown"]
                    .as_str()
                    .unwrap_or("")
                    .contains("模块边界：会话 API 位于 src/product/session.rs")
        });
        let saw_outline_artifact = messages.iter().any(|message| {
            message["type"] == "artifact_update" && message.get("outline_candidate").is_some()
        });
        saw_resolution_artifact && saw_outline_artifact
    })
    .await;
    assert!(
        messages
            .iter()
            .all(|message| message["type"] != "protocol_error")
    );

    let store = WorkItemPlanStore::new(ProductAppPaths::new(repo.path().join(".aria")));
    let index = store
        .load_outline_context_index("project_0001", "issue_0001", &plan_id)
        .expect("load outline context index")
        .expect("outline context index");
    assert_eq!(index.blocker_resolutions.len(), 1);
    assert!(
        index.blocker_resolutions[0]
            .summary
            .as_deref()
            .unwrap_or("")
            .contains("模块边界：会话 API 位于 src/product/session.rs")
    );
    assert!(
        index.blocker_resolutions[0]
            .resolution_artifact_ref
            .starts_with("artifact_version_"),
        "resolution artifact ref should point to persisted artifact version: {}",
        index.blocker_resolutions[0].resolution_artifact_ref
    );

    {
        let prompts = prompts.lock().expect("captured prompts");
        assert!(prompts.len() >= 2, "expected rerun prompt, got {prompts:?}");
        assert!(
            prompts[1].contains("模块边界：会话 API 位于 src/product/session.rs"),
            "rerun prompt should include human resolution: {}",
            prompts[1]
        );
    }

    ws.close(None).await.ok();
}
