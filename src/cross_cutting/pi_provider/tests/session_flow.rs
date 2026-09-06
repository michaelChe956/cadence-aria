// pi session 流程测试（select/abort/resume/demux/failure/suppress）：从
// tests.rs（现 tests/mod.rs）拆出以满足 large_file_guard 的 1200 行上限；
// 仅文件组织变化，测试逻辑与断言零改动。

use super::*;

#[tokio::test]
async fn session_select_request_maps_to_choice_request_and_forwards_response() {
    let (run, command_tx, mut event_rx, server) = start_select_session(false).await;
    recv_choice_request(&mut event_rx).await;
    command_tx
        .send(ProviderCommand::ChoiceResponse {
            id: "select-1".to_string(),
            selected_option_ids: vec!["A".to_string()],
            free_text: None,
            answers: vec![],
        })
        .await
        .expect("choice response");
    let response = server.await.expect("server");
    assert_eq!(
        response,
        serde_json::json!({"type":"extension_ui_response", "id":"select-1", "value":"A"})
    );
    run.await.expect("session task").expect("session complete");
    let events = drain_events(&mut event_rx).await;
    assert!(events.iter().any(
        |event| matches!(event, ProviderEvent::TextDelta { content } if content == "continued")
    ));
}

#[tokio::test]
async fn session_select_with_free_text_maps_to_value() {
    let (run, command_tx, mut event_rx, server) = start_select_session(false).await;
    recv_choice_request(&mut event_rx).await;
    command_tx
        .send(ProviderCommand::ChoiceResponse {
            id: "select-1".to_string(),
            selected_option_ids: vec![],
            free_text: Some("自定义".to_string()),
            answers: vec![],
        })
        .await
        .expect("choice response");
    let response = server.await.expect("server");
    assert_eq!(response["value"], "自定义");
    run.await.expect("session task").expect("session complete");
}

#[tokio::test]
async fn session_select_empty_response_sends_full_cancelled_envelope() {
    let (run, command_tx, mut event_rx, server) = start_select_session(false).await;
    recv_choice_request(&mut event_rx).await;
    command_tx
        .send(ProviderCommand::ChoiceResponse {
            id: "select-1".to_string(),
            selected_option_ids: vec![],
            free_text: None,
            answers: vec![],
        })
        .await
        .expect("choice response");
    let response = server.await.expect("server");
    assert_eq!(
        response,
        serde_json::json!({"type":"extension_ui_response", "id":"select-1", "cancelled":true})
    );
    run.await.expect("session task").expect("session complete");
}

#[tokio::test]
async fn session_select_during_handshake_is_handled() {
    let (run, command_tx, mut event_rx, server) = start_select_session(true).await;
    recv_choice_request(&mut event_rx).await;
    command_tx
        .send(ProviderCommand::ChoiceResponse {
            id: "select-1".to_string(),
            selected_option_ids: vec!["B".to_string()],
            free_text: None,
            answers: vec![],
        })
        .await
        .expect("choice response");
    assert_eq!(server.await.expect("server")["value"], "B");
    run.await.expect("session task").expect("session complete");
}

#[tokio::test]
async fn session_select_abort_during_wait_sends_pi_abort() {
    let (client_io, server_io) = tokio::io::duplex(8192);
    let (reader, writer) = tokio::io::split(client_io);
    let peer = JsonRpcPeer::new(reader, writer);
    let (event_tx, mut event_rx) = mpsc::channel(16);
    let (command_tx, command_rx) = mpsc::channel(8);
    let server = tokio::spawn(async move {
        let (server_reader, mut server_writer) = tokio::io::split(server_io);
        let mut reader = tokio::io::BufReader::new(server_reader);
        let get_state = read_outbound(&mut reader).await;
        write_inbound(&mut server_writer, serde_json::json!({"type":"response","id":get_state["id"],"success":true,"data":{"sessionId":"sess-1"}})).await;
        let prompt = read_outbound(&mut reader).await;
        write_inbound(
            &mut server_writer,
            serde_json::json!({"type":"response","id":prompt["id"],"success":true}),
        )
        .await;
        write_inbound(&mut server_writer, serde_json::json!({"type":"extension_ui_request","id":"select-1","method":"select","title":"格式?","options":["A", "B"]})).await;
        read_outbound(&mut reader).await
    });
    let run = tokio::spawn(run_pi_session(
        peer,
        command_rx,
        event_tx,
        streaming_input_for_test(None),
        CancellationToken::new(),
    ));
    recv_choice_request(&mut event_rx).await;
    command_tx
        .send(ProviderCommand::Abort)
        .await
        .expect("abort");
    assert_eq!(server.await.expect("server")["type"], "abort");
    run.await.expect("session task").expect("session abort");
    assert!(
        drain_events(&mut event_rx)
            .await
            .iter()
            .any(|event| matches!(event, ProviderEvent::StatusChanged(ProviderStatus::Aborted)))
    );
}

#[tokio::test]
async fn session_select_closed_command_channel_sends_pi_abort() {
    let (run, command_tx, mut event_rx, server) = start_select_session(false).await;
    recv_choice_request(&mut event_rx).await;
    drop(command_tx);
    assert_eq!(server.await.expect("server")["type"], "abort");
    run.await.expect("session task").expect("session abort");
    assert!(
        drain_events(&mut event_rx)
            .await
            .iter()
            .any(|event| matches!(event, ProviderEvent::StatusChanged(ProviderStatus::Aborted)))
    );
}

#[tokio::test]
async fn session_select_wrong_id_then_correct_id() {
    let (run, command_tx, mut event_rx, server) = start_select_session(false).await;
    recv_choice_request(&mut event_rx).await;
    command_tx
        .send(ProviderCommand::ChoiceResponse {
            id: "wrong".to_string(),
            selected_option_ids: vec!["A".to_string()],
            free_text: None,
            answers: vec![],
        })
        .await
        .expect("wrong response");
    let protocol_error = event_rx.recv().await.expect("protocol error");
    assert!(
        matches!(protocol_error, ProviderEvent::ProtocolError { ref code, .. } if code == "CHOICE_ID_UNMATCHED")
    );
    command_tx
        .send(ProviderCommand::ChoiceResponse {
            id: "select-1".to_string(),
            selected_option_ids: vec!["A".to_string()],
            free_text: None,
            answers: vec![],
        })
        .await
        .expect("correct response");
    assert_eq!(server.await.expect("server")["value"], "A");
    run.await.expect("session task").expect("session complete");
}

#[tokio::test]
async fn session_tool_call_does_not_produce_permission_or_choice_request() {
    let (client_io, server_io) = tokio::io::duplex(8192);
    let (reader, writer) = tokio::io::split(client_io);
    let peer = JsonRpcPeer::new(reader, writer);
    let (event_tx, mut event_rx) = mpsc::channel(16);
    let (_command_tx, command_rx) = mpsc::channel(8);
    let tool_events = inbound(&load_fixture("auto_text.jsonl"))
        .into_iter()
        .filter(|event| {
            matches!(
                event["type"].as_str(),
                Some("tool_execution_start" | "tool_execution_end")
            )
        })
        .collect::<Vec<_>>();

    tokio::spawn(async move {
        let (server_reader, mut server_writer) = tokio::io::split(server_io);
        let mut reader = tokio::io::BufReader::new(server_reader);
        let get_state = read_outbound(&mut reader).await;
        assert_eq!(get_state["type"], "get_state");
        write_inbound(
            &mut server_writer,
            serde_json::json!({
                "type": "response", "id": get_state["id"], "command": "get_state",
                "success": true, "data": {"sessionId": "sess-1"}
            }),
        )
        .await;
        let prompt = read_outbound(&mut reader).await;
        assert_eq!(prompt["type"], "prompt");
        write_inbound(
            &mut server_writer,
            serde_json::json!({
                "type": "response", "id": prompt["id"], "command": "prompt", "success": true
            }),
        )
        .await;
        for event in tool_events {
            write_inbound(&mut server_writer, event).await;
        }
        write_inbound(
            &mut server_writer,
            serde_json::json!({
                "type": "message_update",
                "assistantMessageEvent": {"type": "text_delta", "contentIndex": 0, "delta": "Hello"}
            }),
        )
        .await;
        write_inbound(
            &mut server_writer,
            serde_json::json!({"type": "agent_settled"}),
        )
        .await;
    });

    run_pi_session(
        peer,
        command_rx,
        event_tx,
        streaming_input_for_test(None),
        CancellationToken::new(),
    )
    .await
    .expect("Auto session completes without an Aria approval round trip");

    let events = drain_events(&mut event_rx).await;
    assert!(
        events.iter().any(
            |event| matches!(event, ProviderEvent::TextDelta { content } if content == "Hello")
        )
    );
    assert!(events.iter().any(|event| matches!(
        event,
        ProviderEvent::ToolCall(call)
            if call.id == "toolu_bdrk_012r11A6JgdXmJY7LeXvA4d3"
                && call.tool_name == "bash"
                && call.input == serde_json::json!({"command": "printf fixture-tool-ok"})
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        ProviderEvent::ToolResult(result)
            if result.tool_use_id == "toolu_bdrk_012r11A6JgdXmJY7LeXvA4d3"
                && result.output == "fixture-tool-ok"
                && !result.is_error
    )));
    assert!(
        !events.iter().any(|event| matches!(
            event,
            ProviderEvent::ChoiceRequest(_) | ProviderEvent::PermissionRequest(_)
        )),
        "ordinary Pi tools must not trigger choice or permission requests"
    );
    let completion = events
        .iter()
        .find_map(|event| match event {
            ProviderEvent::Completed(completion) => Some(completion.clone()),
            _ => None,
        })
        .expect("completed");
    assert_eq!(completion.provider_session_id.as_deref(), Some("sess-1"));
}

#[tokio::test]
async fn session_aborts_on_provider_command_abort() {
    let (client_io, server_io) = tokio::io::duplex(8192);
    let (reader, writer) = tokio::io::split(client_io);
    let peer = JsonRpcPeer::new(reader, writer);
    let (event_tx, mut event_rx) = mpsc::channel(16);
    let (command_tx, command_rx) = mpsc::channel(8);

    tokio::spawn(async move {
        let (server_reader, mut server_writer) = tokio::io::split(server_io);
        let mut reader = tokio::io::BufReader::new(server_reader);
        let get_state = read_outbound(&mut reader).await;
        write_inbound(
            &mut server_writer,
            serde_json::json!({
                "type": "response", "id": get_state["id"], "command": "get_state",
                "success": true, "data": {"sessionId": "sess-1"}
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
        let abort = read_outbound(&mut reader).await;
        assert_eq!(abort["type"], "abort");
    });

    command_tx
        .send(ProviderCommand::Abort)
        .await
        .expect("send abort");
    run_pi_session(
        peer,
        command_rx,
        event_tx,
        streaming_input_for_test(None),
        CancellationToken::new(),
    )
    .await
    .expect("session ends after abort");

    let events = drain_events(&mut event_rx).await;
    assert!(events.iter().any(|event| matches!(
        event,
        ProviderEvent::StatusChanged(
            crate::cross_cutting::streaming_provider::ProviderStatus::Aborted
        )
    )));
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, ProviderEvent::Completed(_)))
    );
}

#[tokio::test]
async fn session_aborts_when_abort_command_and_token_race_during_prompt_handshake() {
    let (client_io, server_io) = tokio::io::duplex(8192);
    let (reader, writer) = tokio::io::split(client_io);
    let peer = JsonRpcPeer::new(reader, writer);
    let (event_tx, mut event_rx) = mpsc::channel(16);
    let (command_tx, command_rx) = mpsc::channel(8);
    let cancel = CancellationToken::new();
    let (prompt_pending_tx, prompt_pending_rx) = tokio::sync::oneshot::channel();

    let server = tokio::spawn(async move {
        let (server_reader, mut server_writer) = tokio::io::split(server_io);
        let mut reader = tokio::io::BufReader::new(server_reader);
        let get_state = read_outbound(&mut reader).await;
        write_inbound(
            &mut server_writer,
            serde_json::json!({
                "type": "response", "id": get_state["id"], "command": "get_state",
                "success": true, "data": {"sessionId": "sess-1"}
            }),
        )
        .await;
        let _prompt = read_outbound(&mut reader).await;
        prompt_pending_tx.send(()).expect("signal pending prompt");

        let abort = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            read_outbound(&mut reader),
        )
        .await
        .expect("Pi should receive abort during prompt handshake");
        assert_eq!(abort["type"], "abort");
    });

    let run = tokio::spawn(run_pi_session(
        peer,
        command_rx,
        event_tx,
        streaming_input_for_test(None),
        cancel.clone(),
    ));
    prompt_pending_rx.await.expect("prompt is pending");
    command_tx
        .send(ProviderCommand::Abort)
        .await
        .expect("send provider abort");
    cancel.cancel();

    let result = tokio::time::timeout(std::time::Duration::from_secs(1), run)
        .await
        .expect("session should stop after cancellation")
        .expect("session task should not panic");
    server.await.expect("fake Pi task should not panic");
    assert!(result.is_ok(), "cancellation must not surface as failure");

    let events = drain_events(&mut event_rx).await;
    assert!(events.iter().any(|event| matches!(
        event,
        ProviderEvent::StatusChanged(
            crate::cross_cutting::streaming_provider::ProviderStatus::Aborted
        )
    )));
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, ProviderEvent::Failed { .. })),
        "cancellation must not emit Failed"
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, ProviderEvent::Completed(_))),
        "cancellation must not emit Completed"
    );
}

#[tokio::test]
async fn session_aborts_when_token_cancels_during_get_state_handshake() {
    let (client_io, server_io) = tokio::io::duplex(8192);
    let (reader, writer) = tokio::io::split(client_io);
    let peer = JsonRpcPeer::new(reader, writer);
    let (event_tx, mut event_rx) = mpsc::channel(16);
    let (_command_tx, command_rx) = mpsc::channel(8);
    let cancel = CancellationToken::new();
    let (get_state_pending_tx, get_state_pending_rx) = tokio::sync::oneshot::channel();

    let server = tokio::spawn(async move {
        let (server_reader, mut server_writer) = tokio::io::split(server_io);
        let mut reader = tokio::io::BufReader::new(server_reader);
        let _get_state = read_outbound(&mut reader).await;
        get_state_pending_tx
            .send(())
            .expect("signal pending get_state");

        let abort = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            read_outbound(&mut reader),
        )
        .await
        .expect("Pi should receive abort during get_state handshake");
        assert_eq!(abort["type"], "abort");
        write_inbound(
            &mut server_writer,
            serde_json::json!({
                "type": "message_update",
                "assistantMessageEvent": {"type": "text_delta", "contentIndex": 0, "delta": "ignored"}
            }),
        )
        .await;
    });

    let run = tokio::spawn(run_pi_session(
        peer,
        command_rx,
        event_tx,
        streaming_input_for_test(None),
        cancel.clone(),
    ));
    get_state_pending_rx.await.expect("get_state is pending");
    cancel.cancel();

    let result = tokio::time::timeout(std::time::Duration::from_secs(1), run)
        .await
        .expect("session should stop after cancellation")
        .expect("session task should not panic");
    server.await.expect("fake Pi task should not panic");
    assert!(result.is_ok(), "cancellation must not surface as failure");

    let events = drain_events(&mut event_rx).await;
    assert!(events.iter().any(|event| matches!(
        event,
        ProviderEvent::StatusChanged(
            crate::cross_cutting::streaming_provider::ProviderStatus::Aborted
        )
    )));
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, ProviderEvent::Failed { .. })),
        "cancellation must not emit Failed"
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, ProviderEvent::Completed(_))),
        "cancellation must not emit Completed"
    );
}

#[tokio::test]
async fn session_suppresses_output_after_abort() {
    let (client_io, server_io) = tokio::io::duplex(8192);
    let (reader, writer) = tokio::io::split(client_io);
    let peer = JsonRpcPeer::new(reader, writer);
    let (event_tx, mut event_rx) = mpsc::channel(16);
    let (command_tx, command_rx) = mpsc::channel(8);
    let (session_running_tx, session_running_rx) = tokio::sync::oneshot::channel();

    let server = tokio::spawn(async move {
        let (server_reader, mut server_writer) = tokio::io::split(server_io);
        let mut reader = tokio::io::BufReader::new(server_reader);
        let get_state = read_outbound(&mut reader).await;
        write_inbound(
            &mut server_writer,
            serde_json::json!({
                "type": "response", "id": get_state["id"], "command": "get_state",
                "success": true, "data": {"sessionId": "sess-1"}
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
        session_running_tx.send(()).expect("signal running session");

        let abort = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            read_outbound(&mut reader),
        )
        .await
        .expect("Pi should receive abort after prompt");
        assert_eq!(abort["type"], "abort");
        write_inbound(
            &mut server_writer,
            serde_json::json!({
                "type": "message_update",
                "assistantMessageEvent": {"type": "text_delta", "contentIndex": 0, "delta": "must not leak"}
            }),
        )
        .await;
        write_inbound(
            &mut server_writer,
            serde_json::json!({"type": "agent_settled"}),
        )
        .await;
    });

    let run = tokio::spawn(run_pi_session(
        peer,
        command_rx,
        event_tx,
        streaming_input_for_test(None),
        CancellationToken::new(),
    ));
    session_running_rx.await.expect("session is running");
    command_tx
        .send(ProviderCommand::Abort)
        .await
        .expect("send provider abort");

    let result = tokio::time::timeout(std::time::Duration::from_secs(1), run)
        .await
        .expect("session should stop after abort")
        .expect("session task should not panic");
    server.await.expect("fake Pi task should not panic");
    result.expect("abort is a successful terminal path");

    let events = drain_events(&mut event_rx).await;
    assert!(events.iter().any(|event| matches!(
        event,
        ProviderEvent::StatusChanged(
            crate::cross_cutting::streaming_provider::ProviderStatus::Aborted
        )
    )));
    assert!(
        !events.iter().any(
            |event| matches!(event, ProviderEvent::TextDelta { content } if content == "must not leak")
        ),
        "post-abort output must not reach the provider event stream"
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, ProviderEvent::Completed(_))),
        "post-abort agent_settled must not complete the session"
    );
}

#[tokio::test]
async fn session_resumes_with_existing_session_id() {
    let (client_io, server_io) = tokio::io::duplex(8192);
    let (reader, writer) = tokio::io::split(client_io);
    let peer = JsonRpcPeer::new(reader, writer);
    let (event_tx, mut event_rx) = mpsc::channel(16);
    let (_command_tx, command_rx) = mpsc::channel(8);

    tokio::spawn(async move {
        let (server_reader, mut server_writer) = tokio::io::split(server_io);
        let mut reader = tokio::io::BufReader::new(server_reader);
        let get_state = read_outbound(&mut reader).await;
        write_inbound(
            &mut server_writer,
            serde_json::json!({
                "type": "response", "id": get_state["id"], "command": "get_state",
                "success": true, "data": {"sessionId": "sess-old"}
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
        write_inbound(
            &mut server_writer,
            serde_json::json!({
                "type": "message_update",
                "assistantMessageEvent": {"type": "text_delta", "contentIndex": 0, "delta": "resumed"}
            }),
        )
        .await;
        write_inbound(
            &mut server_writer,
            serde_json::json!({"type": "agent_settled"}),
        )
        .await;
    });

    let input = streaming_input_for_test(Some("sess-old".to_string()));
    let provider = PiProvider::new("pi".into());
    let extension = ensure_ask_extension().expect("ask extension");
    let args = provider.build_args(
        input.resume_provider_session_id.as_deref(),
        &extension,
        input.tool_policy.as_ref(),
    );
    assert!(args.contains(&"--session-id".to_string()));
    assert!(args.contains(&"sess-old".to_string()));
    run_pi_session(peer, command_rx, event_tx, input, CancellationToken::new())
        .await
        .expect("resume session ok");

    let events = drain_events(&mut event_rx).await;
    let completion = events
        .iter()
        .find_map(|event| match event {
            ProviderEvent::Completed(completion) => Some(completion.clone()),
            _ => None,
        })
        .expect("completed");
    assert_eq!(completion.provider_session_id.as_deref(), Some("sess-old"));
}

#[tokio::test]
async fn session_failure_is_terminal_no_retry() {
    let (client_io, server_io) = tokio::io::duplex(8192);
    let (reader, writer) = tokio::io::split(client_io);
    let peer = JsonRpcPeer::new(reader, writer);
    let (event_tx, mut event_rx) = mpsc::channel(16);
    let (_command_tx, command_rx) = mpsc::channel(8);

    tokio::spawn(async move {
        let (server_reader, server_writer) = tokio::io::split(server_io);
        let mut reader = tokio::io::BufReader::new(server_reader);
        let _ = read_outbound(&mut reader).await;
        drop(server_writer);
    });

    let _ = run_pi_session(
        peer,
        command_rx,
        event_tx,
        streaming_input_for_test(None),
        CancellationToken::new(),
    )
    .await;
    let events = drain_events(&mut event_rx).await;
    assert!(
        events
            .iter()
            .any(|event| matches!(event, ProviderEvent::Failed { .. })),
        "EOF must produce Failed without retry"
    );
}

#[tokio::test]
async fn session_demultiplexes_response_by_id() {
    let (client_io, server_io) = tokio::io::duplex(8192);
    let (reader, writer) = tokio::io::split(client_io);
    let peer = JsonRpcPeer::new(reader, writer);
    let (event_tx, mut event_rx) = mpsc::channel(16);
    let (_command_tx, command_rx) = mpsc::channel(8);

    tokio::spawn(async move {
        let (server_reader, mut server_writer) = tokio::io::split(server_io);
        let mut reader = tokio::io::BufReader::new(server_reader);
        let get_state = read_outbound(&mut reader).await;
        write_inbound(
            &mut server_writer,
            serde_json::json!({
                "type": "message_update",
                "assistantMessageEvent": {"type": "text_delta", "contentIndex": 0, "delta": "early"}
            }),
        )
        .await;
        write_inbound(
            &mut server_writer,
            serde_json::json!({
                "type": "response", "id": get_state["id"], "command": "get_state",
                "success": true, "data": {"sessionId": "sess-1"}
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
        write_inbound(
            &mut server_writer,
            serde_json::json!({"type": "agent_settled"}),
        )
        .await;
    });

    run_pi_session(
        peer,
        command_rx,
        event_tx,
        streaming_input_for_test(None),
        CancellationToken::new(),
    )
    .await
    .expect("session ok");
    let events = drain_events(&mut event_rx).await;
    assert!(
        events.iter().any(
            |event| matches!(event, ProviderEvent::TextDelta { content } if content == "early")
        )
    );
}

/// F3 Task 4.1 零变化回归（restrict-role-write-tools）：
/// ①pi 出站 request id 保持既有 `pi-<N>` 字符串命名空间（Task 2.2 只升级 codex
///   peer 到 aria-<seq>，pi 不得被牵连改写）；
/// ②pi→Auto permission mapping 不变：pi 是 Auto-only，bridge 恒以
///   `ProviderPermissionMode::Auto` 构造且从不经它上抛授权——即使 input 携带
///   Supervised 也不得出现 PermissionRequest 事件或权限参数。
#[tokio::test]
async fn pi_outbound_ids_and_auto_permission_mapping_stay_unchanged() {
    let (client_io, server_io) = tokio::io::duplex(8192);
    let (reader, writer) = tokio::io::split(client_io);
    let peer = JsonRpcPeer::new(reader, writer);
    let (event_tx, mut event_rx) = mpsc::channel(16);
    let (_command_tx, command_rx) = mpsc::channel(8);

    let server = tokio::spawn(async move {
        let (server_reader, mut server_writer) = tokio::io::split(server_io);
        let mut reader = tokio::io::BufReader::new(server_reader);
        let get_state = read_outbound(&mut reader).await;
        // 出站 id 保持 pi-1 起（字符串命名空间，非 aria-<seq>、非裸数字）。
        assert_eq!(get_state["id"], serde_json::json!("pi-1"));
        write_inbound(
            &mut server_writer,
            serde_json::json!({
                "type": "response", "id": get_state["id"], "command": "get_state",
                "success": true, "data": {"sessionId": "sess-ids-locked"}
            }),
        )
        .await;
        let prompt = read_outbound(&mut reader).await;
        assert_eq!(prompt["id"], serde_json::json!("pi-2"));
        write_inbound(
            &mut server_writer,
            serde_json::json!({
                "type": "response", "id": prompt["id"], "command": "prompt", "success": true
            }),
        )
        .await;
        write_inbound(
            &mut server_writer,
            serde_json::json!({
                "type": "message_update",
                "assistantMessageEvent": {"type": "text_delta", "contentIndex": 0, "delta": "ids locked"}
            }),
        )
        .await;
        write_inbound(
            &mut server_writer,
            serde_json::json!({"type":"agent_settled"}),
        )
        .await;
    });

    let mut supervised_input = streaming_input_for_test(None);
    supervised_input.permission_mode = ProviderPermissionMode::Supervised;
    run_pi_session(
        peer,
        command_rx,
        event_tx,
        supervised_input,
        CancellationToken::new(),
    )
    .await
    .expect("supervised input must not change pi behavior");
    server.await.expect("server");

    let events = drain_events(&mut event_rx).await;
    assert!(
        events
            .iter()
            .any(|event| matches!(event, ProviderEvent::Completed(_))),
        "pi session must still complete under a Supervised input"
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, ProviderEvent::PermissionRequest(_))),
        "pi is Auto-only: Supervised input must never surface permission requests"
    );
}
