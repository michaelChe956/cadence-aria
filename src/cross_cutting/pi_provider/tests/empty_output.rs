//! 空输出防护测试：agent 空轮次输出必须触发一次有界重试，
//! 重试仍为空则上抛 `provider_empty_output` 分类错误。

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::json_rpc_peer::JsonRpcPeer;
use crate::cross_cutting::streaming_provider::ProviderEvent;

use super::super::session::run_pi_session;
use super::{drain_events, read_outbound, streaming_input_for_test, write_inbound};

#[tokio::test]
async fn session_empty_output_retries_once_and_completes() {
    let (client_io, server_io) = tokio::io::duplex(8192);
    let (reader, writer) = tokio::io::split(client_io);
    let peer = JsonRpcPeer::new(reader, writer);
    let (event_tx, mut event_rx) = mpsc::channel(16);
    let (_command_tx, command_rx) = mpsc::channel(8);

    let server = tokio::spawn(async move {
        let (server_reader, mut server_writer) = tokio::io::split(server_io);
        let mut reader = tokio::io::BufReader::new(server_reader);
        let get_state = read_outbound(&mut reader).await;
        write_inbound(
            &mut server_writer,
            serde_json::json!({
                "type": "response", "id": get_state["id"], "command": "get_state",
                "success": true, "data": {"sessionId": "sess-empty-retry"}
            }),
        )
        .await;
        let prompt = read_outbound(&mut reader).await;
        write_inbound(
            &mut server_writer,
            serde_json::json!({
                "type": "response", "id": prompt["id"], "command": "prompt", "success": true
            }),
        )
        .await;
        // 首轮 agent_settled：无任何 text_delta，full_output 为空。
        write_inbound(
            &mut server_writer,
            serde_json::json!({ "type": "agent_settled" }),
        )
        .await;
        // 空输出触发的唯一一次重试 prompt 必须在会话内合法发送。
        let retry_prompt = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            read_outbound(&mut reader),
        )
        .await
        .expect("provider should send one retry prompt after empty output");
        write_inbound(
            &mut server_writer,
            serde_json::json!({
                "type": "response", "id": retry_prompt["id"], "command": "prompt", "success": true
            }),
        )
        .await;
        write_inbound(
            &mut server_writer,
            serde_json::json!({
                "type": "message_update",
                "assistantMessageEvent": {"type": "text_delta", "contentIndex": 0, "delta": "recovered"}
            }),
        )
        .await;
        write_inbound(
            &mut server_writer,
            serde_json::json!({ "type": "agent_settled" }),
        )
        .await;
        retry_prompt
    });

    let run = tokio::spawn(run_pi_session(
        peer,
        command_rx,
        event_tx,
        streaming_input_for_test(None),
        CancellationToken::new(),
    ));
    let events = drain_events(&mut event_rx).await;
    assert!(
        events.iter().any(|event| matches!(
            event,
            ProviderEvent::Execution(execution)
                if execution.event_id == "pi_empty_output_retry"
                    && execution.title == "Turn empty output retry"
        )),
        "empty-output retry must leave a provider-layer audit event"
    );
    run.await
        .expect("session task")
        .expect("session complete after retry");
    let retry_prompt = server.await.expect("server");

    assert_eq!(retry_prompt["type"], "prompt");
    let retry_message = retry_prompt["message"]
        .as_str()
        .expect("retry prompt message");
    assert!(!retry_message.trim().is_empty());
    assert_ne!(retry_message, "fixture prompt");
    let completed = events
        .iter()
        .find_map(|event| match event {
            ProviderEvent::Completed(completion) => Some(completion),
            _ => None,
        })
        .expect("provider should complete after retry");
    assert_eq!(completed.full_output, "recovered");
}

#[tokio::test]
async fn session_empty_output_fails_with_provider_empty_output_after_single_retry() {
    let (client_io, server_io) = tokio::io::duplex(8192);
    let (reader, writer) = tokio::io::split(client_io);
    let peer = JsonRpcPeer::new(reader, writer);
    let (event_tx, mut event_rx) = mpsc::channel(16);
    let (_command_tx, command_rx) = mpsc::channel(8);

    let server = tokio::spawn(async move {
        let (server_reader, mut server_writer) = tokio::io::split(server_io);
        let mut reader = tokio::io::BufReader::new(server_reader);
        let get_state = read_outbound(&mut reader).await;
        write_inbound(
            &mut server_writer,
            serde_json::json!({
                "type": "response", "id": get_state["id"], "command": "get_state",
                "success": true, "data": {"sessionId": "sess-empty-fail"}
            }),
        )
        .await;
        let prompt = read_outbound(&mut reader).await;
        write_inbound(
            &mut server_writer,
            serde_json::json!({
                "type": "response", "id": prompt["id"], "command": "prompt", "success": true
            }),
        )
        .await;
        // 两轮 agent_settled 均无输出。
        write_inbound(
            &mut server_writer,
            serde_json::json!({ "type": "agent_settled" }),
        )
        .await;
        let retry_prompt = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            read_outbound(&mut reader),
        )
        .await
        .expect("provider should send one retry prompt after empty output");
        write_inbound(
            &mut server_writer,
            serde_json::json!({
                "type": "response", "id": retry_prompt["id"], "command": "prompt", "success": true
            }),
        )
        .await;
        write_inbound(
            &mut server_writer,
            serde_json::json!({ "type": "agent_settled" }),
        )
        .await;
        // 重试必须有界：不再期待第三次 prompt。
        tokio::time::timeout(
            std::time::Duration::from_millis(300),
            read_outbound(&mut reader),
        )
        .await
        .ok()
    });

    let run = tokio::spawn(run_pi_session(
        peer,
        command_rx,
        event_tx,
        streaming_input_for_test(None),
        CancellationToken::new(),
    ));
    let events = drain_events(&mut event_rx).await;
    assert!(
        events.iter().any(|event| matches!(
            event,
            ProviderEvent::Execution(execution)
                if execution.event_id == "pi_empty_output_retry"
                    && execution.title == "Turn empty output retry"
        )),
        "empty-output retry must leave a provider-layer audit event"
    );
    let error = run
        .await
        .expect("session task")
        .expect_err("session must fail after empty retry");
    assert!(
        error.details.contains("provider_empty_output"),
        "unexpected error details: {}",
        error.details
    );
    assert!(
        events.iter().any(|event| matches!(
            event,
            ProviderEvent::Failed { message } if message.contains("provider_empty_output")
        )),
        "terminal Failed event must carry provider_empty_output"
    );
    let unexpected_third_prompt = server.await.expect("server");
    assert!(
        unexpected_third_prompt.is_none(),
        "retry must be bounded to one, saw {:?}",
        unexpected_third_prompt
    );
}

#[tokio::test]
async fn session_settled_while_waiting_empty_output_fails_after_single_retry() {
    let (client_io, server_io) = tokio::io::duplex(8192);
    let (reader, writer) = tokio::io::split(client_io);
    let peer = JsonRpcPeer::new(reader, writer);
    let (event_tx, mut event_rx) = mpsc::channel(16);
    let (_command_tx, command_rx) = mpsc::channel(8);

    let server = tokio::spawn(async move {
        let (server_reader, mut server_writer) = tokio::io::split(server_io);
        let mut reader = tokio::io::BufReader::new(server_reader);
        let get_state = read_outbound(&mut reader).await;
        write_inbound(
            &mut server_writer,
            serde_json::json!({
                "type": "response", "id": get_state["id"], "command": "get_state",
                "success": true, "data": {"sessionId": "sess-settled-waiting"}
            }),
        )
        .await;
        let prompt = read_outbound(&mut reader).await;
        // settle 先于 prompt 响应到达：迫使 settled_while_waiting 分支。两轮均无输出。
        write_inbound(
            &mut server_writer,
            serde_json::json!({ "type": "agent_settled" }),
        )
        .await;
        write_inbound(
            &mut server_writer,
            serde_json::json!({
                "type": "response", "id": prompt["id"], "command": "prompt", "success": true
            }),
        )
        .await;
        let retry_prompt = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            read_outbound(&mut reader),
        )
        .await
        .expect("provider should send one retry prompt after empty settled_while_waiting");
        write_inbound(
            &mut server_writer,
            serde_json::json!({ "type": "agent_settled" }),
        )
        .await;
        write_inbound(
            &mut server_writer,
            serde_json::json!({
                "type": "response", "id": retry_prompt["id"], "command": "prompt", "success": true
            }),
        )
        .await;
        tokio::time::timeout(
            std::time::Duration::from_millis(300),
            read_outbound(&mut reader),
        )
        .await
        .ok()
    });

    let run = tokio::spawn(run_pi_session(
        peer,
        command_rx,
        event_tx,
        streaming_input_for_test(None),
        CancellationToken::new(),
    ));
    let events = drain_events(&mut event_rx).await;
    assert!(
        events.iter().any(|event| matches!(
            event,
            ProviderEvent::Execution(execution)
                if execution.event_id == "pi_empty_output_retry"
                    && execution.title == "Turn empty output retry"
        )),
        "empty-output retry must leave a provider-layer audit event"
    );
    let error = run
        .await
        .expect("session task")
        .expect_err("session must fail after empty settled_while_waiting retry");
    assert!(
        error.details.contains("provider_empty_output"),
        "unexpected error details: {}",
        error.details
    );
    assert!(
        events.iter().any(|event| matches!(
            event,
            ProviderEvent::Failed { message } if message.contains("provider_empty_output")
        )),
        "terminal Failed event must carry provider_empty_output"
    );
    let unexpected_third_prompt = server.await.expect("server");
    assert!(
        unexpected_third_prompt.is_none(),
        "retry must be bounded to one, saw {:?}",
        unexpected_third_prompt
    );
}
