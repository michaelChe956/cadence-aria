//! F2-A 空输出防护测试：kimi 会话轮以 end_turn 结束但无任何 assistant 输出时，
//! 必须在**同一会话内**重发一次固定重试指令（对齐 pi 先例
//! `pi_provider/session.rs` 的 `pi_empty_output_retry`），并留下 provider 层审计
//! 事件；重试后仍为空则上抛 `provider_empty_output` 分类错误（kimi 侧首见此
//! 错误码）。这是会话层输出兜底，不触碰 kimi client services 角色策略
//! （REQ-ENV-09 工具策略零改动语义不受影响）。

use serde_json::Value;
use tokio::sync::mpsc;

use crate::cross_cutting::streaming_provider::ProviderEvent;

use super::session_tests::{direct_session_events, input, read_request, send_message, test_peer};

/// 空输出→同会话重试一次→二次输出被采：wire 断言重试 prompt 在会话内合法发送
/// 且 prompt 文本非空、非首轮指令；event 断言重试留下审计 Execution 事件。
#[tokio::test]
async fn kimi_session_empty_output_retries_once_in_session_and_completes() {
    let (peer, server) = test_peer();
    let (_commands, mut events, run) = direct_session_events(peer, input(None, 10)).await;
    let server_task = tokio::spawn(async move {
        let (reader, mut writer) = tokio::io::split(server);
        let mut reader = tokio::io::BufReader::new(reader);
        let initialize = read_request(&mut reader).await;
        send_message(
            &mut writer,
            serde_json::json!({
                "jsonrpc":"2.0", "id":initialize["id"],
                "result":{"protocolVersion":1,"agentCapabilities":{"loadSession":true,"sessionCapabilities":{"resume":{}}}}
            }),
        )
        .await;
        let _initialized = read_request(&mut reader).await;
        let new = read_request(&mut reader).await;
        send_message(
            &mut writer,
            serde_json::json!({
                "jsonrpc":"2.0", "id":new["id"], "result":{"sessionId":"kimi-empty-retry"}
            }),
        )
        .await;
        // 首轮 prompt：无任何 agent_message_chunk，end_turn 时 full_output 为空。
        let prompt = read_request(&mut reader).await;
        assert_eq!(prompt["method"], "session/prompt");
        send_message(
            &mut writer,
            serde_json::json!({
                "jsonrpc":"2.0", "id":prompt["id"], "result":{"stopReason":"end_turn"}
            }),
        )
        .await;
        // 空输出触发的唯一一次重试 prompt 必须在同一 session 内发送。
        let retry_prompt = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let request = read_request(&mut reader).await;
                if request["method"] == "session/prompt" {
                    return request;
                }
            }
        })
        .await
        .expect("provider must send one retry prompt after empty output");
        assert_eq!(
            retry_prompt["params"]["sessionId"], "kimi-empty-retry",
            "retry prompt must stay in the same kimi session"
        );
        send_message(
            &mut writer,
            serde_json::json!({
                "jsonrpc":"2.0", "method":"session/update",
                "params":{"sessionId":"kimi-empty-retry","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"recovered"}}}
            }),
        )
        .await;
        send_message(
            &mut writer,
            serde_json::json!({
                "jsonrpc":"2.0", "id":retry_prompt["id"], "result":{"stopReason":"end_turn"}
            }),
        )
        .await;
        retry_prompt
    });

    let events = drain_until_terminal(&mut events).await;
    assert!(
        events.iter().any(|event| matches!(
            event,
            ProviderEvent::Execution(execution)
                if execution.event_id == "kimi_empty_output_retry"
                    && execution.title == "Turn empty output retry"
        )),
        "empty-output retry must leave a provider-layer audit event: {events:?}"
    );
    run.await
        .expect("session task")
        .expect("session must complete after in-session retry");
    let retry_prompt = server_task.await.expect("server task");
    let retry_message = retry_prompt["params"]["prompt"][0]["text"]
        .as_str()
        .expect("retry prompt text");
    assert!(!retry_message.trim().is_empty());
    assert_ne!(retry_message, "fixture prompt");
    let completed = events
        .iter()
        .find_map(|event| match event {
            ProviderEvent::Completed(completion) => Some(completion),
            _ => None,
        })
        .expect("provider should complete with the retried turn output");
    assert_eq!(completed.full_output, "recovered");
}

/// 两轮均空：重试必须有界——恰一次重试后仍空则返回 `provider_empty_output`
/// 分类错误，且不再发出第三次 prompt。
#[tokio::test]
async fn kimi_session_empty_output_fails_with_provider_empty_output_after_single_retry() {
    let (peer, server) = test_peer();
    let (_commands, mut events, run) = direct_session_events(peer, input(None, 10)).await;
    let server_task = tokio::spawn(async move {
        let (reader, mut writer) = tokio::io::split(server);
        let mut reader = tokio::io::BufReader::new(reader);
        let initialize = read_request(&mut reader).await;
        send_message(
            &mut writer,
            serde_json::json!({
                "jsonrpc":"2.0", "id":initialize["id"],
                "result":{"protocolVersion":1,"agentCapabilities":{"loadSession":true,"sessionCapabilities":{"resume":{}}}}
            }),
        )
        .await;
        let _initialized = read_request(&mut reader).await;
        let new = read_request(&mut reader).await;
        send_message(
            &mut writer,
            serde_json::json!({
                "jsonrpc":"2.0", "id":new["id"], "result":{"sessionId":"kimi-empty-fail"}
            }),
        )
        .await;
        let prompt = read_request(&mut reader).await;
        send_message(
            &mut writer,
            serde_json::json!({
                "jsonrpc":"2.0", "id":prompt["id"], "result":{"stopReason":"end_turn"}
            }),
        )
        .await;
        let retry_prompt = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let request = read_request(&mut reader).await;
                if request["method"] == "session/prompt" {
                    return request;
                }
            }
        })
        .await
        .expect("provider must send one retry prompt after empty output");
        send_message(
            &mut writer,
            serde_json::json!({
                "jsonrpc":"2.0", "id":retry_prompt["id"], "result":{"stopReason":"end_turn"}
            }),
        )
        .await;
        // 重试必须有界：不再期待第三次 prompt。
        tokio::time::timeout(std::time::Duration::from_millis(300), async {
            loop {
                let request: Value = read_request(&mut reader).await;
                if request["method"] == "session/prompt" {
                    return request;
                }
            }
        })
        .await
        .ok()
    });

    let events = drain_until_terminal(&mut events).await;
    assert!(
        events.iter().any(|event| matches!(
            event,
            ProviderEvent::Execution(execution)
                if execution.event_id == "kimi_empty_output_retry"
                    && execution.title == "Turn empty output retry"
        )),
        "empty-output retry must leave a provider-layer audit event: {events:?}"
    );
    let error = run
        .await
        .expect("session task")
        .expect_err("session must fail after the single bounded retry");
    assert!(
        error.details.contains("provider_empty_output"),
        "unexpected error details: {}",
        error.details
    );
    let unexpected_third_prompt = server_task.await.expect("server task");
    assert!(
        unexpected_third_prompt.is_none(),
        "retry must be bounded to exactly one, saw {:?}",
        unexpected_third_prompt
    );
}

async fn drain_until_terminal(events: &mut mpsc::Receiver<ProviderEvent>) -> Vec<ProviderEvent> {
    let mut received = Vec::new();
    while let Some(event) = events.recv().await {
        let terminal = matches!(
            event,
            ProviderEvent::Completed(_) | ProviderEvent::Failed { .. }
        );
        received.push(event);
        if terminal {
            return received;
        }
    }
    received
}
