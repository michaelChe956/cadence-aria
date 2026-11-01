use axum::http::Method;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::time::{Duration, timeout};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

use crate::web_work_item_generation::{
    app_with_confirmed_story_and_design_and_streaming_revision_output,
    invalid_split_output_missing_e2e, request_json, valid_split_output,
};

static WS_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

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

async fn recv_ws_messages_with_timeout(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    timeout_after: Duration,
    max_messages: usize,
) -> Vec<Value> {
    let mut messages = Vec::new();
    let deadline = tokio::time::Instant::now() + timeout_after;
    while messages.len() < max_messages && tokio::time::Instant::now() < deadline {
        let remaining = deadline - tokio::time::Instant::now();
        match timeout(remaining, ws.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => {
                messages.push(serde_json::from_str(&text).expect("ws json"));
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

// 退役留档（T5/REQ-RET-02）：`revert_work_item_is_valid_in_author_confirm_only` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// 退役留档（T5/REQ-RET-02）：`revert_work_item_clear_removes_mark` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

async fn prepare_and_start_generation(app: &axum::Router) -> String {
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

    prepare_resp["workspace_session"]["workspace_session_id"]
        .as_str()
        .unwrap()
        .to_string()
}

// 退役留档（T5/REQ-RET-02）：`revert_work_item_triggers_local_redo_in_revision` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

#[tokio::test]
#[ignore = "legacy full-candidate auto revision flow is superseded by WP2 outline generation; WP3+ will replace this coverage"]
async fn work_item_plan_validate_errors_auto_revision_uses_generate_revision() {
    let _guard = WS_TEST_LOCK.lock().await;
    // 首次 generate 返回 validate 失败的输出；带 revision_feedback 的 revision 返回合法输出。
    let (app, _repo) = app_with_confirmed_story_and_design_and_streaming_revision_output(
        invalid_split_output_missing_e2e(),
        valid_split_output(),
    )
    .await;
    let session_id = prepare_and_start_generation(&app).await;
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

    let messages = recv_ws_messages_with_timeout(&mut ws, Duration::from_secs(15), 48).await;
    assert!(
        messages.iter().all(|m| m["type"] != "error"),
        "auto revision should not produce error messages: {:?}",
        messages
            .iter()
            .filter(|m| m["type"] == "error")
            .collect::<Vec<_>>()
    );
    let auto_revision_node = messages
        .iter()
        .find(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "revision"
                && message["node"]["title"]
                    .as_str()
                    .unwrap_or("")
                    .contains("Work Item Plan 自动返修")
        })
        .expect("expected AutoRevision to create a dedicated revision node");
    let auto_revision_node_id = auto_revision_node["node"]["node_id"]
        .as_str()
        .expect("auto revision node id");
    assert!(
        messages.iter().any(|message| {
            message["type"] == "stream_chunk"
                && message["node_id"] == auto_revision_node_id
                && message["content"]
                    .as_str()
                    .unwrap_or("")
                    .contains("Fake Work Item Plan streaming draft")
        }),
        "expected AutoRevision provider stream on a revision node"
    );

    let stage = messages
        .iter()
        .find(|m| m["type"] == "stage_change" && m["stage"] == "author_confirm")
        .expect("stage_change to author_confirm");
    assert_eq!(stage["stage"], "author_confirm");

    ws.close(None).await.ok();
}
