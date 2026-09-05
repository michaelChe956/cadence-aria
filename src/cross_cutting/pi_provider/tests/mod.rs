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
    ProviderToolPolicy, StreamingProviderInput,
};
use crate::protocol::contracts::{AdapterRole, ProviderType};

use super::session::run_pi_session;
use super::*;

mod empty_output;
mod policy_session;
mod version_probe;

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
    let args = provider.build_args(None, &extension, None);
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
    let args = provider.build_args(Some("sess-123"), &extension, None);
    assert!(args.contains(&"--session-id".to_string()));
    assert!(args.contains(&"sess-123".to_string()));
}

/// F3 restrict-role-write-tools Task 1.3：pi argv enforcement（--exclude-tools 冻结片段）。
mod pi_policy_args {
    use super::*;

    #[test]
    fn build_args_policy_keeps_session_id_and_excludes_only_file_writes() {
        let cache = tempfile::tempdir().expect("temporary cache");
        let provider = PiProvider::new("pi".into());
        let extension = ensure_ask_extension_in(cache.path()).expect("ask extension");
        let args = provider.build_args(
            Some("aria-17"),
            &extension,
            Some(&ProviderToolPolicy::deny_file_write_builtins()),
        );
        assert!(
            args.windows(2)
                .any(|w| w == ["--exclude-tools", "edit,write"])
        );
        assert!(args.windows(2).any(|w| w == ["--session-id", "aria-17"]));
    }

    #[test]
    fn build_args_policy_with_empty_session_id_still_excludes_file_writes() {
        // 空 session id 复用既有 trim/filter 语义（不注入 --session-id），
        // 但不得降级成无限制 argv：策略片段必须仍在。
        let cache = tempfile::tempdir().expect("temporary cache");
        let provider = PiProvider::new("pi".into());
        let extension = ensure_ask_extension_in(cache.path()).expect("ask extension");
        let args = provider.build_args(
            Some("   "),
            &extension,
            Some(&ProviderToolPolicy::deny_file_write_builtins()),
        );
        assert!(!args.contains(&"--session-id".to_string()));
        assert!(
            args.windows(2)
                .any(|w| w == ["--exclude-tools", "edit,write"])
        );
    }

    #[test]
    fn build_args_without_policy_keeps_legacy_argv_unchanged() {
        // 非策略路径（Executor/Coder/聚合初始化/kimi）argv 与既有完全一致。
        let cache = tempfile::tempdir().expect("temporary cache");
        let provider = PiProvider::new("pi".into());
        let extension = ensure_ask_extension_in(cache.path()).expect("ask extension");
        let args = provider.build_args(None, &extension, None);
        assert!(!args.contains(&"--exclude-tools".to_string()));
        assert!(!args.contains(&"--session-id".to_string()));
        assert!(args.contains(&"--mode".to_string()));
        assert!(args.contains(&"rpc".to_string()));
    }
}

fn streaming_input_for_test(resume_id: Option<String>) -> StreamingProviderInput {
    // 非策略 legacy 路径 fixture：守卫（Task 3.1）要求非策略会话使用非策略角色。
    StreamingProviderInput {
        tool_policy: None,
        audit_sink: None,
        provider_type: ProviderType::Pi,
        role: AdapterRole::Executor,
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

mod session_flow;
