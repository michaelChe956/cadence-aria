use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::json_rpc_peer::JsonRpcPeer;
use crate::cross_cutting::streaming_provider::{
    ProviderCommand, ProviderEvent, ProviderPermissionMode, StreamingProviderInput,
};
use crate::protocol::contracts::{AdapterRole, ProviderType};

use super::session::run_pi_session;
use super::*;

#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
struct FixtureEnvelope {
    direction: String,
    payload: Value,
}

fn fixture(relative_path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src/cross_cutting/pi_provider/tests/fixtures")
        .join(relative_path)
}

fn load_fixture(relative_path: &str) -> Vec<FixtureEnvelope> {
    fs::read_to_string(fixture(relative_path))
        .expect("fixture file")
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("fixture envelope json"))
        .collect()
}

fn inbound(envelopes: &[FixtureEnvelope]) -> Vec<Value> {
    envelopes
        .iter()
        .filter(|envelope| envelope.direction == "pi_to_client")
        .map(|envelope| envelope.payload.clone())
        .collect()
}

fn outbound(envelopes: &[FixtureEnvelope]) -> Vec<Value> {
    envelopes
        .iter()
        .filter(|envelope| envelope.direction == "client_to_pi")
        .map(|envelope| envelope.payload.clone())
        .collect()
}

#[test]
fn recorded_fixtures_freeze_pi_protocol_envelopes() {
    let text = load_fixture("auto_text.jsonl");
    let cancel = load_fixture("auto_cancel.jsonl");
    let resume = load_fixture("resume.jsonl");

    assert!(
        outbound(&text)
            .iter()
            .any(|value| value["type"] == "prompt")
    );
    assert!(
        inbound(&text)
            .iter()
            .any(|value| value["type"] == "response")
    );
    assert!(
        inbound(&text)
            .iter()
            .any(|value| value["type"] == "message_update")
    );
    assert!(
        inbound(&text)
            .iter()
            .any(|value| value["type"] == "tool_execution_start")
    );
    assert!(
        inbound(&text)
            .iter()
            .any(|value| value["type"] == "tool_execution_end")
    );
    assert!(
        inbound(&text)
            .iter()
            .any(|value| value["type"] == "agent_settled")
    );
    assert!(
        outbound(&cancel)
            .iter()
            .any(|value| value["type"] == "abort")
    );
    assert!(
        outbound(&resume)
            .iter()
            .any(|value| value["type"] == "get_state")
    );
    assert!(inbound(&resume).iter().any(|value| {
        value["type"] == "response"
            && value["command"] == "get_state"
            && value.pointer("/data/sessionId").is_some()
    }));
}

#[test]
fn parse_text_delta_from_message_update() {
    let event = serde_json::json!({
        "type": "message_update",
        "assistantMessageEvent": { "type": "text_delta", "contentIndex": 0, "delta": "Hello" }
    });
    assert_eq!(parse_pi_text_delta(&event).as_deref(), Some("Hello"));
}

#[test]
fn parse_tool_execution_events() {
    let start =
        serde_json::json!({"type": "tool_execution_start", "toolCallId": "c1", "toolName": "bash"});
    assert!(parse_pi_tool_start(&start).is_some());
    let end = serde_json::json!({"type": "tool_execution_end", "toolCallId": "c1", "toolName": "bash", "isError": false});
    assert!(parse_pi_tool_end(&end).is_some());
}

#[test]
fn parse_agent_settled_as_terminal() {
    assert!(is_pi_terminal(
        &serde_json::json!({"type": "agent_settled"})
    ));
}

#[test]
fn parse_session_id_from_get_state_response() {
    let resp = serde_json::json!({"type": "response", "command": "get_state", "success": true, "data": {"sessionId": "sess-1"}});
    assert_eq!(parse_pi_session_id(&resp).as_deref(), Some("sess-1"));
}

#[test]
fn build_args_rpc_mode_auto_only() {
    let provider = PiProvider::new("pi".into());
    let args = provider.build_args(None);
    assert!(args.contains(&"--mode".to_string()));
    assert!(args.contains(&"rpc".to_string()));
    assert!(!args.contains(&"-e".to_string()));
    assert!(!args.contains(&"--session-dir".to_string()));
    assert!(!args.contains(&"--no-extensions".to_string()));
    assert!(!args.contains(&"--session-id".to_string()));
}

#[test]
fn build_args_resume_includes_session_id() {
    let provider = PiProvider::new("pi".into());
    let args = provider.build_args(Some("sess-123"));
    assert!(args.contains(&"--session-id".to_string()));
    assert!(args.contains(&"sess-123".to_string()));
}

fn streaming_input_for_test(resume_id: Option<String>) -> StreamingProviderInput {
    StreamingProviderInput {
        provider_type: ProviderType::Pi,
        role: AdapterRole::Orchestrator,
        prompt: "fixture prompt".to_string(),
        working_dir: tempfile::tempdir().expect("temporary working dir").keep(),
        workspace_session_id: None,
        resume_provider_session_id: resume_id,
        permission_mode: ProviderPermissionMode::Auto,
        structured_output_contract: None,
        env_vars: BTreeMap::new(),
        timeout_secs: 60,
    }
}

async fn read_outbound(
    reader: &mut tokio::io::BufReader<impl tokio::io::AsyncRead + Unpin>,
) -> Value {
    let mut line = String::new();
    reader.read_line(&mut line).await.expect("read outbound");
    serde_json::from_str(&line).expect("outbound is json")
}

async fn write_inbound(writer: &mut (impl tokio::io::AsyncWrite + Unpin), value: Value) {
    writer
        .write_all(value.to_string().as_bytes())
        .await
        .expect("write inbound");
    writer.write_all(b"\n").await.expect("write newline");
}

async fn drain_events(rx: &mut mpsc::Receiver<ProviderEvent>) -> Vec<ProviderEvent> {
    let mut out = Vec::new();
    while let Some(event) = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
        .await
        .expect("provider should emit terminal event")
    {
        let terminal = matches!(
            event,
            ProviderEvent::Completed(_) | ProviderEvent::Failed { .. }
        );
        out.push(event);
        if terminal {
            break;
        }
    }
    out
}

#[tokio::test]
async fn session_sends_prompt_and_emits_text_until_settled() {
    let (client_io, server_io) = tokio::io::duplex(8192);
    let (reader, writer) = tokio::io::split(client_io);
    let peer = JsonRpcPeer::new(reader, writer);
    let (event_tx, mut event_rx) = mpsc::channel(16);
    let (_command_tx, command_rx) = mpsc::channel(8);

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
    .expect("session ok");

    let events = drain_events(&mut event_rx).await;
    assert!(
        events.iter().any(
            |event| matches!(event, ProviderEvent::TextDelta { content } if content == "Hello")
        )
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
            serde_json::json!({"type": "agent_settled"}),
        )
        .await;
    });

    let input = streaming_input_for_test(Some("sess-old".to_string()));
    let provider = PiProvider::new("pi".into());
    let args = provider.build_args(input.resume_provider_session_id.as_deref());
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
