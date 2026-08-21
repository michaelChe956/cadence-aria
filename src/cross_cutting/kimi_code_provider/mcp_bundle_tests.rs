use std::collections::BTreeMap;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

use crate::cross_cutting::json_rpc_peer::JsonRpcPeer;
use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::cross_cutting::streaming_provider::{
    ProviderCommand, ProviderEvent, ProviderPermissionMode, StreamingProviderInput,
};
use crate::protocol::contracts::{AdapterRole, ProviderType};
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::kimi_code_provider::mcp_bundle::{
    KimiMcpInjection, McpServerConfig, ValidatedMcpServerBundle, validate_bundle,
};
use crate::cross_cutting::kimi_code_provider::session::run_kimi_session_with_mcp;

fn input(resume: Option<&str>, timeout_secs: u64) -> StreamingProviderInput {
    StreamingProviderInput {
        provider_type: ProviderType::KimiCode,
        role: AdapterRole::Orchestrator,
        prompt: "fixture prompt".to_string(),
        working_dir: std::env::current_dir().expect("working directory"),
        workspace_session_id: None,
        resume_provider_session_id: resume.map(ToString::to_string),
        permission_mode: ProviderPermissionMode::Auto,
        structured_output_contract: None,
        env_vars: BTreeMap::new(),
        timeout_secs,
    }
}

async fn read_request(
    reader: &mut tokio::io::BufReader<impl tokio::io::AsyncRead + Unpin>,
) -> Value {
    let mut line = String::new();
    reader.read_line(&mut line).await.expect("read request");
    serde_json::from_str(&line).expect("JSON-RPC request")
}

async fn send_message(writer: &mut (impl tokio::io::AsyncWrite + Unpin), value: Value) {
    writer
        .write_all(value.to_string().as_bytes())
        .await
        .expect("write message");
    writer.write_all(b"\n").await.expect("write newline");
    writer.flush().await.expect("flush message");
}

fn test_peer() -> (
    JsonRpcPeer<tokio::io::WriteHalf<tokio::io::DuplexStream>>,
    tokio::io::DuplexStream,
) {
    let (client, server) = tokio::io::duplex(16 * 1024);
    let (reader, writer) = tokio::io::split(client);
    (JsonRpcPeer::new(reader, writer), server)
}

#[allow(clippy::type_complexity)]
async fn direct_session_events_with_mcp<W>(
    peer: JsonRpcPeer<W>,
    input: StreamingProviderInput,
    mcp_injection: Option<KimiMcpInjection>,
) -> (
    mpsc::Sender<ProviderCommand>,
    mpsc::Receiver<ProviderEvent>,
    tokio::task::JoinHandle<Result<(), ProviderAdapterError>>,
)
where
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (commands, command_rx) = mpsc::channel(8);
    let (event_tx, events) = mpsc::channel(32);
    let run = tokio::spawn(run_kimi_session_with_mcp(
        peer,
        command_rx,
        event_tx,
        input,
        mcp_injection,
        CancellationToken::new(),
    ));
    (commands, events, run)
}

fn codegraph_bundle() -> ValidatedMcpServerBundle {
    let mut server = McpServerConfig::new("codegraph", "/usr/local/bin/codegraph");
    server.args = vec!["mcp".to_string()];
    server.cwd = Some("/repo".to_string());
    validate_bundle(vec![server]).expect("valid codegraph bundle")
}

#[tokio::test]
async fn session_new_injects_bundle_derived_mcp_servers() {
    let (peer, server) = test_peer();
    let bundle = codegraph_bundle();
    let expected_mcp = bundle.mcp_servers_json();
    let (_commands, mut events, run) = direct_session_events_with_mcp(
        peer,
        input(None, 10),
        Some(KimiMcpInjection::for_new_session(bundle)),
    )
    .await;
    let server_task = tokio::spawn(async move {
        let (reader, mut writer) = tokio::io::split(server);
        let mut reader = tokio::io::BufReader::new(reader);
        let initialize = read_request(&mut reader).await;
        send_message(&mut writer, serde_json::json!({"jsonrpc":"2.0","id":initialize["id"],"result":{"protocolVersion":1,"agentCapabilities":{"loadSession":true,"sessionCapabilities":{"resume":{}}}}})).await;
        let _initialized = read_request(&mut reader).await;
        let new = read_request(&mut reader).await;
        assert_eq!(new["method"], "session/new");
        assert_eq!(
            new["params"]["mcpServers"],
            serde_json::Value::Array(expected_mcp)
        );
        send_message(&mut writer, serde_json::json!({"jsonrpc":"2.0","id":new["id"],"result":{"sessionId":"mcp-injected"}})).await;
        let prompt = read_request(&mut reader).await;
        send_message(&mut writer, serde_json::json!({"jsonrpc":"2.0","id":prompt["id"],"result":{"stopReason":"end_turn"}})).await;
    });
    let events = tokio::time::timeout(std::time::Duration::from_secs(2), async {
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
    })
    .await
    .expect("session must terminate");
    assert!(
        events
            .iter()
            .any(|event| matches!(event, ProviderEvent::Completed(_))),
        "events: {events:?}"
    );
    assert!(run.await.expect("run join").is_ok());
    server_task.await.expect("server task");
}

#[tokio::test]
async fn session_load_with_matching_frozen_digest_uses_bundle() {
    let (peer, server) = test_peer();
    let bundle = codegraph_bundle();
    let expected_mcp = bundle.mcp_servers_json();
    let frozen_digest = bundle.digest().to_string();
    let injection = KimiMcpInjection::for_resume(bundle, frozen_digest);
    let (_commands, mut events, run) =
        direct_session_events_with_mcp(peer, input(Some("old-session"), 10), Some(injection)).await;
    let server_task = tokio::spawn(async move {
        let (reader, mut writer) = tokio::io::split(server);
        let mut reader = tokio::io::BufReader::new(reader);
        let initialize = read_request(&mut reader).await;
        send_message(&mut writer, serde_json::json!({"jsonrpc":"2.0","id":initialize["id"],"result":{"protocolVersion":1,"agentCapabilities":{"loadSession":true,"sessionCapabilities":{"resume":{}}}}})).await;
        let _initialized = read_request(&mut reader).await;
        let load = read_request(&mut reader).await;
        assert_eq!(load["method"], "session/load");
        assert_eq!(load["params"]["sessionId"], "old-session");
        assert_eq!(
            load["params"]["mcpServers"],
            serde_json::Value::Array(expected_mcp)
        );
        send_message(&mut writer, serde_json::json!({"jsonrpc":"2.0","id":load["id"],"result":{"sessionId":"old-session"}})).await;
        let prompt = read_request(&mut reader).await;
        send_message(&mut writer, serde_json::json!({"jsonrpc":"2.0","id":prompt["id"],"result":{"stopReason":"end_turn"}})).await;
    });
    let events = tokio::time::timeout(std::time::Duration::from_secs(2), async {
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
    })
    .await
    .expect("session must terminate");
    assert!(
        events
            .iter()
            .any(|event| matches!(event, ProviderEvent::Completed(_))),
        "events: {events:?}"
    );
    assert!(run.await.expect("run join").is_ok());
    server_task.await.expect("server task");
}

#[tokio::test]
async fn resume_digest_drift_rejects_load_and_starts_new_session() {
    let (peer, server) = test_peer();
    let bundle = codegraph_bundle();
    let expected_mcp = bundle.mcp_servers_json();
    let injection =
        KimiMcpInjection::for_resume(bundle, "frozen-digest-that-no-longer-matches".to_string());
    let (_commands, mut events, run) =
        direct_session_events_with_mcp(peer, input(Some("drifted-session"), 10), Some(injection))
            .await;
    let server_task = tokio::spawn(async move {
        let (reader, mut writer) = tokio::io::split(server);
        let mut reader = tokio::io::BufReader::new(reader);
        let initialize = read_request(&mut reader).await;
        send_message(&mut writer, serde_json::json!({"jsonrpc":"2.0","id":initialize["id"],"result":{"protocolVersion":1,"agentCapabilities":{"loadSession":true,"sessionCapabilities":{"resume":{}}}}})).await;
        let _initialized = read_request(&mut reader).await;
        let request = read_request(&mut reader).await;
        assert_eq!(
            request["method"], "session/new",
            "digest drift must reject session/load and start a new session"
        );
        assert!(request["params"].get("sessionId").is_none());
        assert_eq!(
            request["params"]["mcpServers"],
            serde_json::Value::Array(expected_mcp)
        );
        send_message(&mut writer, serde_json::json!({"jsonrpc":"2.0","id":request["id"],"result":{"sessionId":"fresh-session-after-drift"}})).await;
        let prompt = read_request(&mut reader).await;
        send_message(&mut writer, serde_json::json!({"jsonrpc":"2.0","id":prompt["id"],"result":{"stopReason":"end_turn"}})).await;
    });
    let events = tokio::time::timeout(std::time::Duration::from_secs(2), async {
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
    })
    .await
    .expect("session must terminate");
    let completion = events
        .iter()
        .find_map(|event| match event {
            ProviderEvent::Completed(completion) => Some(completion),
            _ => None,
        })
        .unwrap_or_else(|| panic!("completion; events: {events:?}"));
    assert_eq!(
        completion.provider_session_id.as_deref(),
        Some("fresh-session-after-drift")
    );
    // superseded 事件化（REQ-ENV-04）：旧会话必须以可消费的 Execution 事件暴露。
    let superseded_event = events
        .iter()
        .find_map(|event| match event {
            ProviderEvent::Execution(execution)
                if execution.event_id.starts_with("kimi_session_superseded_") =>
            {
                Some(execution)
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("superseded execution event; events: {events:?}"));
    let payload: Value = serde_json::from_str(
        superseded_event
            .output
            .as_deref()
            .expect("superseded payload"),
    )
    .expect("superseded payload must be valid JSON");
    assert_eq!(payload["superseded"], Value::Bool(true));
    assert_eq!(payload["old_session_id"], "drifted-session");
    assert_eq!(payload["new_session_id"], "fresh-session-after-drift");
    assert_eq!(payload["new_session_started"], Value::Bool(true));
    assert_eq!(
        payload["frozen_digest"],
        "frozen-digest-that-no-longer-matches"
    );
    assert!(run.await.expect("run join").is_ok());
    server_task.await.expect("server task");
}

#[tokio::test]
async fn without_bundle_mcp_servers_stay_empty_on_new_and_load() {
    let (peer, server) = test_peer();
    let (_commands, mut events, run) =
        direct_session_events_with_mcp(peer, input(None, 10), None).await;
    let server_task = tokio::spawn(async move {
        let (reader, mut writer) = tokio::io::split(server);
        let mut reader = tokio::io::BufReader::new(reader);
        let initialize = read_request(&mut reader).await;
        send_message(&mut writer, serde_json::json!({"jsonrpc":"2.0","id":initialize["id"],"result":{"protocolVersion":1,"agentCapabilities":{"loadSession":true,"sessionCapabilities":{"resume":{}}}}})).await;
        let _initialized = read_request(&mut reader).await;
        let new = read_request(&mut reader).await;
        assert_eq!(new["params"]["mcpServers"], serde_json::json!([]));
        send_message(
            &mut writer,
            serde_json::json!({"jsonrpc":"2.0","id":new["id"],"result":{"sessionId":"bare"}}),
        )
        .await;
        let prompt = read_request(&mut reader).await;
        send_message(&mut writer, serde_json::json!({"jsonrpc":"2.0","id":prompt["id"],"result":{"stopReason":"end_turn"}})).await;
    });
    let events = tokio::time::timeout(std::time::Duration::from_secs(2), async {
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
    })
    .await
    .expect("session must terminate");
    assert!(
        events
            .iter()
            .any(|event| matches!(event, ProviderEvent::Completed(_))),
        "events: {events:?}"
    );
    assert!(run.await.expect("run join").is_ok());
    server_task.await.expect("server task");
}
