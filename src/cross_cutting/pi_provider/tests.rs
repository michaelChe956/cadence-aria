use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

#[cfg(unix)]
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::json_rpc_peer::JsonRpcPeer;
use crate::cross_cutting::streaming_provider::{
    ChoiceRequestSource, ProviderCommand, ProviderEvent, ProviderPermissionMode, ProviderStatus,
    StreamingProviderInput,
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

#[cfg(unix)]
fn write_executable(dir: &std::path::Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    let mut file = fs::File::create(&path).expect("create executable fixture");
    writeln!(file, "#!/bin/sh\n{body}").expect("write executable fixture");
    let mut permissions = file.metadata().expect("fixture metadata").permissions();
    permissions.set_mode(0o755);
    file.set_permissions(permissions)
        .expect("make fixture executable");
    path
}
#[test]
fn pi_provider_aria_ask_extension_is_structured_and_does_not_intercept_tools() {
    assert!(ARIA_ASK_EXTENSION.contains("ask_user"));
    assert!(ARIA_ASK_EXTENSION.contains("ctx.ui.select"));
    assert!(ARIA_ASK_EXTENSION.contains("promptGuidelines"));
    assert!(!ARIA_ASK_EXTENSION.contains("tool_call"));
}

#[test]
fn ensure_ask_extension_reuses_only_matching_content() {
    let cache = tempfile::tempdir().expect("temporary cache");
    let extension = ensure_ask_extension_in(cache.path()).expect("create extension");
    assert_eq!(
        fs::read_to_string(&extension).expect("read extension"),
        ARIA_ASK_EXTENSION
    );
    assert_eq!(
        ensure_ask_extension_in(cache.path()).expect("reuse matching extension"),
        extension
    );
    fs::write(&extension, "untrusted extension").expect("replace extension");
    assert!(ensure_ask_extension_in(cache.path()).is_err());
}

#[cfg(unix)]
#[test]
fn ensure_ask_extension_rejects_symlink() {
    let cache = tempfile::tempdir().expect("temporary cache");
    let target = cache.path().join("target.ts");
    fs::write(&target, ARIA_ASK_EXTENSION).expect("write target");
    let hash = hex::encode(sha2::Sha256::digest(ARIA_ASK_EXTENSION.as_bytes()));
    std::os::unix::fs::symlink(
        &target,
        cache.path().join(format!("aria-ask-{}.ts", &hash[..8])),
    )
    .expect("create symlink");
    assert!(ensure_ask_extension_in(cache.path()).is_err());
}

#[cfg(unix)]
#[tokio::test]
async fn pi_start_below_minimum_returns_before_spawning() {
    let temp = tempfile::tempdir().expect("temporary directory");
    let marker = temp.path().join("spawned");
    let command = write_executable(
        temp.path(),
        "fake-pi",
        &format!(
            "if [ \"$1\" = \"--version\" ]; then echo 0.82.0; exit 0; fi\ntouch {}",
            marker.display()
        ),
    );

    let provider = PiProvider::new(command);
    let result = provider
        .start(streaming_input_for_test(None), CancellationToken::new())
        .await;
    assert!(result.is_err(), "incompatible Pi must be rejected");
    assert!(
        !marker.exists(),
        "Pi process must not spawn when the version is below minimum"
    );
}

#[test]
fn parse_pi_select_request_preserves_title_and_options() {
    let fixture = fs::read_to_string(fixture("select_request.jsonl")).expect("select fixture");
    let request = parse_pi_select_request(
        &serde_json::from_str::<Value>(fixture.trim()).expect("select fixture json"),
    )
    .expect("select request");
    assert_eq!(request.id, "select-1");
    assert_eq!(request.title, "格式?");
    assert_eq!(request.options, vec!["A", "B", "C"]);
}

#[test]
fn pi_version_below_minimum_blocks() {
    assert!(ensure_pi_version_compatible(&PiVersion::Known((0, 82, 0))).is_err());
}

#[test]
fn pi_version_at_or_above_minimum_passes() {
    assert!(ensure_pi_version_compatible(&PiVersion::Known((0, 83, 0))).is_ok());
    assert!(ensure_pi_version_compatible(&PiVersion::Known((0, 84, 0))).is_ok());
}

#[test]
fn pi_version_unparseable_does_not_block() {
    assert!(ensure_pi_version_compatible(&parse_pi_version("pi version xyz")).is_ok());
}

#[tokio::test]
async fn pi_version_command_missing_does_not_block() {
    let missing = tempfile::tempdir()
        .expect("temporary directory")
        .path()
        .join("missing-pi");
    assert_eq!(
        probe_pi_version_with_timeout(&missing, std::time::Duration::from_secs(1)).await,
        PiVersion::Unknown(ProbeFailure::CommandFailed)
    );
}

#[cfg(unix)]
#[tokio::test]
async fn pi_version_timeout_does_not_block() {
    let temp = tempfile::tempdir().expect("temporary directory");
    let command = write_executable(temp.path(), "hanging-pi", "sleep 30");
    assert_eq!(
        probe_pi_version_with_timeout(&command, std::time::Duration::from_millis(50)).await,
        PiVersion::Unknown(ProbeFailure::TimedOut)
    );
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
    let cache = tempfile::tempdir().expect("temporary cache");
    let provider = PiProvider::new("pi".into());
    let extension = ensure_ask_extension_in(cache.path()).expect("ask extension");
    let args = provider.build_args(None, &extension);
    assert!(args.contains(&"--mode".to_string()));
    assert!(args.contains(&"rpc".to_string()));
    assert!(args.contains(&"-e".to_string()));
    assert!(args.iter().any(|arg| std::path::Path::new(arg).is_file()));
    assert!(!args.contains(&"--session-dir".to_string()));
    assert!(!args.contains(&"--no-extensions".to_string()));
    assert!(!args.contains(&"--session-id".to_string()));
}

#[test]
fn build_args_resume_includes_session_id() {
    let cache = tempfile::tempdir().expect("temporary cache");
    let provider = PiProvider::new("pi".into());
    let extension = ensure_ask_extension_in(cache.path()).expect("ask extension");
    let args = provider.build_args(Some("sess-123"), &extension);
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

async fn start_select_session(
    select_before_get_state_response: bool,
) -> (
    tokio::task::JoinHandle<
        Result<(), crate::cross_cutting::provider_adapter::ProviderAdapterError>,
    >,
    mpsc::Sender<ProviderCommand>,
    mpsc::Receiver<ProviderEvent>,
    tokio::task::JoinHandle<Value>,
) {
    let (client_io, server_io) = tokio::io::duplex(8192);
    let (reader, writer) = tokio::io::split(client_io);
    let peer = JsonRpcPeer::new(reader, writer);
    let (event_tx, event_rx) = mpsc::channel(16);
    let (command_tx, command_rx) = mpsc::channel(8);
    let server = tokio::spawn(async move {
        let (server_reader, mut server_writer) = tokio::io::split(server_io);
        let mut reader = tokio::io::BufReader::new(server_reader);
        let get_state = read_outbound(&mut reader).await;
        if select_before_get_state_response {
            write_inbound(
                &mut server_writer,
                serde_json::json!({
                    "type": "extension_ui_request", "id": "select-1", "method": "select",
                    "title": "格式?", "options": ["A", "B"]
                }),
            )
            .await;
        }
        if !select_before_get_state_response {
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
                serde_json::json!({
                    "type": "extension_ui_request", "id": "select-1", "method": "select",
                    "title": "格式?", "options": ["A", "B"]
                }),
            )
            .await;
        }
        let response = read_outbound(&mut reader).await;
        if select_before_get_state_response {
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
        }
        write_inbound(&mut server_writer, serde_json::json!({
            "type": "message_update",
            "assistantMessageEvent": {"type": "text_delta", "contentIndex": 0, "delta": "continued"}
        })).await;
        write_inbound(
            &mut server_writer,
            serde_json::json!({"type": "agent_settled"}),
        )
        .await;
        response
    });
    let run = tokio::spawn(run_pi_session(
        peer,
        command_rx,
        event_tx,
        streaming_input_for_test(None),
        CancellationToken::new(),
    ));
    (run, command_tx, event_rx, server)
}

async fn recv_choice_request(event_rx: &mut mpsc::Receiver<ProviderEvent>) {
    loop {
        match event_rx.recv().await.expect("choice event") {
            ProviderEvent::ChoiceRequest(request) => {
                assert_eq!(request.id, "select-1");
                assert_eq!(request.source, ChoiceRequestSource::ProviderChoice);
                assert_eq!(request.prompt, "格式?");
                assert_eq!(request.options.len(), 2);
                assert_eq!(request.options[0].id, "A");
                assert!(request.allow_free_text);
                assert!(!request.allow_multiple);
                return;
            }
            _ => continue,
        }
    }
}

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
            serde_json::json!({"type": "agent_settled"}),
        )
        .await;
    });

    let input = streaming_input_for_test(Some("sess-old".to_string()));
    let provider = PiProvider::new("pi".into());
    let extension = ensure_ask_extension().expect("ask extension");
    let args = provider.build_args(input.resume_provider_session_id.as_deref(), &extension);
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

#[test]
fn parse_pi_usage_extracts_cost_snapshot_from_get_state_response() {
    let response = serde_json::json!({
        "type": "response",
        "command": "get_state",
        "success": true,
        "data": {
            "sessionId": "pi_session_1",
            "cost": {
                "input": 100.5,
                "output": 42,
                "cacheRead": 7,
                "cacheWrite": 9
            }
        }
    });
    let report = parse_pi_usage(&response, "author").expect("usage should parse");
    assert_eq!(report.role, "author");
    assert_eq!(report.input_tokens, Some(100));
    assert_eq!(report.output_tokens, Some(42));
    assert_eq!(report.cache_read_tokens, Some(7));
    assert_eq!(report.cache_creation_tokens, Some(9));
}

#[test]
fn parse_pi_usage_returns_none_without_cost() {
    let response =
        serde_json::json!({ "type": "response", "command": "get_state", "success": true });
    assert!(parse_pi_usage(&response, "reviewer").is_none());
}
