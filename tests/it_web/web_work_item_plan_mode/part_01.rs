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
    QueuedSplitOutput,
    app_with_confirmed_story_and_design_and_streaming_outputs,
    app_with_confirmed_story_and_design_and_streaming_raw_outputs,
    malformed_outline_structured_stdout,
    request_json, valid_outline_output,
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

async fn wait_for_recorded_prompts(
    prompts: &std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    expected: usize,
) -> Vec<String> {
    timeout(Duration::from_secs(5), async {
        loop {
            let snapshot = prompts.lock().expect("captured prompts lock").clone();
            if snapshot.len() >= expected {
                return snapshot;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("provider prompt capture timeout")
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
                "review_scope": "outline",
                "summary": "outline review fixture",
                "generation_round_id": "round_001",
                "affects_items": [
                    { "target_outline_id": "outline_backend_session" }
                ],
                "findings": if verdict == "revise" {
                    json!([{
                        "severity": "must_fix",
                        "message": "拆分遗漏前端错误状态\n影响：后续 work item 无法覆盖体验缺口",
                        "evidence": "Outline 未覆盖前端错误状态",
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

// 退役留档（T5/REQ-RET-02）：`accept_outline_creates_generation_round_and_active_index` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// 退役留档（T5/REQ-RET-02）：`author_decision_is_rejected_on_generation_mode_node` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

#[tokio::test]
async fn request_revision_on_outline_confirm_returns_to_outline_run_without_round() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls().await;
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

// 退役留档（T5/REQ-RET-02）：`outline_accept_enters_outline_review_when_reviewer_enabled` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// 退役留档（T5/REQ-RET-02）：`outline_accept_skips_review_when_reviewer_disabled` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// 退役留档（T5/REQ-RET-02）：`outline_review_pass_enters_generation_mode` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// 退役留档（T5/REQ-RET-02）：`outline_review_revise_returns_to_outline_revision` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// 退役留档（T5/REQ-RET-02）：`outline_review_revision_parse_failure_auto_retries_then_returns_to_outline_confirm` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// 退役留档（T5/REQ-RET-02）：`select_serial_mode_enters_first_item_run` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// 退役留档（T5/REQ-RET-02）：`select_batch_mode_enters_batch_run` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。
