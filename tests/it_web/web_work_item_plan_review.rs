use axum::http::Method;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::time::{Duration, timeout};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

use crate::web_work_item_generation::{
    app_with_confirmed_story_and_design_and_test_providers, request_json, valid_split_output,
};

static WS_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn enable_test_controls() {
    unsafe {
        std::env::set_var("ARIA_E2E_TEST_CONTROLS", "1");
    }
}

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
                messages.push(serde_json::from_str(&text).expect("ws json"));
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

type WsStream = tokio_tungstenite::WebSocketStream<
    tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
>;

async fn prepare_work_item_plan_and_author_to_confirm(
    app: &axum::Router,
) -> (String, WsStream) {
    let (_status, prepare_resp) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-item-plans:prepare",
        json!({
            "title": "登录拆分 Work Item Plan",
            "story_spec_ids": ["story_spec_0001"],
            "design_spec_ids": ["design_spec_0001"],
            "author_provider": "fake",
            "reviewer_provider": "codex",
            "review_rounds": 1,
            "include_integration_tests": true,
            "include_e2e_tests": false,
            "force_frontend_backend_split": true,
            "require_execution_plan_confirm": false
        }),
    )
    .await;

    let session_id = prepare_resp["workspace_session"]["workspace_session_id"]
        .as_str()
        .unwrap()
        .to_string();

    let mut ws = connect_ws(app.clone(), &session_id).await;
    ws.send(Message::Text(
        json!({
            "type": "start_generation",
            "provider_config": { "author": "fake", "reviewer": "codex", "review_rounds": 1 },
            "reviewer_enabled": true
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send start_generation");

    let messages = recv_ws_until(
        &mut ws,
        Duration::from_secs(15),
        |msgs| {
            msgs.iter().any(|m| m["type"] == "artifact_update")
                && msgs.iter().any(|m| {
                    m["type"] == "stage_change" && m["stage"] == "author_confirm"
                })
        },
    )
    .await;

    let _artifact_update = messages
        .iter()
        .find(|m| m["type"] == "artifact_update")
        .expect("artifact_update");
    let _author_confirm_stage = messages
        .iter()
        .filter(|m| m["type"] == "stage_change")
        .find(|m| m["stage"] == "author_confirm")
        .expect("stage_change to author_confirm");

    (session_id, ws)
}

async fn enable_revise_review_fixture(app: &axum::Router, session_id: &str) {
    let (status, _response) = request_json(
        app.clone(),
        Method::POST,
        &format!("/api/test/workspace-sessions/{session_id}/review-fixture"),
        json!({
            "verdict": "revise",
            "summary": "需要返修",
            "comments": "存在阻塞问题",
            "findings": [{
                "severity": "blocking",
                "message": "缺少异常路径处理",
                "evidence": "候选计划未覆盖失败场景",
                "impact": "运行时可能遗漏边界条件",
                "required_action": "补充错误处理与回滚策略"
            }]
        }),
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::OK);
}

#[tokio::test]
async fn review_returns_verdict_for_whole_candidate() {
    let _guard = WS_TEST_LOCK.lock().await;
    enable_test_controls();

    let (app, _repo) = app_with_confirmed_story_and_design_and_test_providers(valid_split_output()).await;
    let (session_id, mut ws) = prepare_work_item_plan_and_author_to_confirm(&app).await;
    enable_revise_review_fixture(&app, &session_id).await;

    ws.send(Message::Text(
        json!({ "type": "author_decision", "decision": "accept" })
            .to_string()
            .into(),
    ))
    .await
    .expect("send author_decision");

    let messages = recv_ws_until(
        &mut ws,
        Duration::from_secs(15),
        |msgs| {
            msgs.iter().any(|m| m["type"] == "review_decision_required")
                || msgs.iter().any(|m| {
                    m["type"] == "stage_change" && m["stage"] == "human_confirm"
                })
        },
    )
    .await;

    let _stage_cross = messages
        .iter()
        .find(|m| m["type"] == "stage_change" && m["stage"] == "cross_review")
        .expect("stage_change cross_review");
    let _stream = messages
        .iter()
        .find(|m| m["type"] == "stream_chunk")
        .expect("stream_chunk");
    let review_complete = messages
        .iter()
        .find(|m| m["type"] == "review_complete")
        .expect("review_complete");
    assert!(review_complete["verdict"].is_string());
    assert!(review_complete["summary"].is_string());

    let verdict = review_complete["verdict"].as_str().unwrap();
    if verdict == "revise" {
        let decision_required = messages
            .iter()
            .find(|m| m["type"] == "review_decision_required")
            .expect("review_decision_required for revise verdict");
        let options = decision_required["options"]
            .as_array()
            .expect("options array");
        assert!(options.contains(&json!("continue")));
        assert!(options.contains(&json!("continue_with_context")));
        assert!(options.contains(&json!("human_intervene")));
    } else {
        let _stage_human = messages
            .iter()
            .find(|m| m["type"] == "stage_change" && m["stage"] == "human_confirm")
            .expect("stage_change human_confirm for pass verdict");
    }

    ws.close(None).await.ok();
}

#[tokio::test]
async fn work_item_plan_review_returns_decision_response() {
    let _guard = WS_TEST_LOCK.lock().await;
    enable_test_controls();

    // human_intervene 路径：进入人工确认
    {
        let (app, _repo) = app_with_confirmed_story_and_design_and_test_providers(valid_split_output()).await;
        let (session_id, mut ws) = prepare_work_item_plan_and_author_to_confirm(&app).await;
        enable_revise_review_fixture(&app, &session_id).await;

        ws.send(Message::Text(
            json!({ "type": "author_decision", "decision": "accept" })
                .to_string()
                .into(),
        ))
        .await
        .expect("send author_decision");

        let _messages = recv_ws_until(
            &mut ws,
            Duration::from_secs(15),
            |msgs| msgs.iter().any(|m| m["type"] == "review_decision_required"),
        )
        .await;

        ws.send(Message::Text(
            json!({
                "type": "review_decision_response",
                "decision": "human_intervene",
                "extra_context": null
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send review_decision_response human_intervene");

        let messages = recv_ws_until(
            &mut ws,
            Duration::from_secs(10),
            |msgs| {
                msgs.iter()
                    .any(|m| m["type"] == "stage_change" && m["stage"] == "human_confirm")
            },
        )
        .await;
        messages
            .iter()
            .find(|m| m["type"] == "stage_change" && m["stage"] == "human_confirm")
            .expect("stage_change human_confirm after human_intervene");

        ws.close(None).await.ok();
    }

    // continue 路径：只验证进入 revision 阶段（revision 执行在 WP4 实现）
    {
        let (app, _repo) = app_with_confirmed_story_and_design_and_test_providers(valid_split_output()).await;
        let (session_id, mut ws) = prepare_work_item_plan_and_author_to_confirm(&app).await;
        enable_revise_review_fixture(&app, &session_id).await;

        ws.send(Message::Text(
            json!({ "type": "author_decision", "decision": "accept" })
                .to_string()
                .into(),
        ))
        .await
        .expect("send author_decision");

        let _messages = recv_ws_until(
            &mut ws,
            Duration::from_secs(15),
            |msgs| msgs.iter().any(|m| m["type"] == "review_decision_required"),
        )
        .await;

        ws.send(Message::Text(
            json!({
                "type": "review_decision_response",
                "decision": "continue",
                "extra_context": null
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send review_decision_response continue");

        let messages = recv_ws_until(
            &mut ws,
            Duration::from_secs(10),
            |msgs| {
                msgs.iter()
                    .any(|m| m["type"] == "stage_change" && m["stage"] == "revision")
            },
        )
        .await;
        messages
            .iter()
            .find(|m| m["type"] == "stage_change" && m["stage"] == "revision")
            .expect("stage_change revision after continue");

        ws.close(None).await.ok();
    }
}
