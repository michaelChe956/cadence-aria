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
        working_directory: None,
        baseline_tree: None,
        tool_policy: None,
        audit_sink: None,
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
        send_message(&mut writer, serde_json::json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"mcp-injected","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"bundle injected"}}}})).await;
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
        send_message(&mut writer, serde_json::json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"old-session","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"resumed with bundle"}}}})).await;
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
        send_message(&mut writer, serde_json::json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"fresh-session-after-drift","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"fresh after drift"}}}})).await;
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
        send_message(&mut writer, serde_json::json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"bare","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"bare session output"}}}})).await;
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

// ==== Task 4c:LC 投影的 MCP 来源分层 ====

use std::path::PathBuf;

use crate::cross_cutting::kimi_code_provider::projection::{
    KIMI_LC_APPROVAL_POLICY, KIMI_NATIVE_MCP_SOURCE, KimiPolicyProjector,
};
use crate::product::logical_codebase::policy::{
    PolicyTarget, ProviderDialect, SessionPolicyAction, SessionPolicyEnvelope,
};
use crate::product::logical_codebase::provider_gateway::ProviderRef;
use crate::product::logical_codebase::provider_projection::{
    ProviderPolicyProjector, ProviderProjectionInput,
};

/// Task 4c Step 1:`mcp_bundle_digest` 只对应 Aria 注入 bundle——native 项目
/// 配置以来源标记单独进投影(不冒充 Aria bundle digest);两层 digest 随
/// 来源漂移变化;空串来源 fail-closed。
#[test]
fn lcg_t04_kimi_native_mcp_is_not_aria_bundle() {
    let projector = KimiPolicyProjector::new("kimi 0.34.0-lc-fixture");
    let bundle = codegraph_bundle();
    let envelope = SessionPolicyEnvelope {
        policy_id: "policy-lc-0001".to_string(),
        policy_revision: 1,
        policy_digest: "sha256:policy-lc-0001".to_string(),
        action: SessionPolicyAction::CodingTargetWrite,
        target: PolicyTarget::checkout(
            "logical_repo_0001",
            "checkout_0001",
            PathBuf::from("/lc/member-a"),
        ),
        working_directory: PathBuf::from("/lc/lc-root"),
        readable_roots: vec![PathBuf::from("/lc/lc-root")],
        writable_roots: vec![PathBuf::from("/lc/member-a")],
        provider_dialect: ProviderDialect::KimiAcpV1,
        config_artifact_ref: "sha256:cfg-a".to_string(),
        config_digest: "sha256:cfg-digest-a".to_string(),
        created_at: "2026-10-03T00:00:00Z".to_string(),
        authority_root: PathBuf::from("/lc/lc-root"),
    };
    let input_with_mcp = |mcp_source: &str| {
        ProviderProjectionInput::new(
            envelope.clone(),
            ProviderRef::kimi_code("cap_kimi_lc_fixture"),
            envelope.action,
            AdapterRole::Executor,
            ProviderPermissionMode::Auto,
            None,
            KIMI_LC_APPROVAL_POLICY.to_string(),
            mcp_source.to_string(),
            "sha256:cfg-a".to_string(),
            "sha256:trust-1".to_string(),
            None,
        )
    };

    // native:来源标记非 digest——不冒充 Aria bundle。
    let native = projector
        .project(&input_with_mcp(KIMI_NATIVE_MCP_SOURCE))
        .expect("native-source projection is produced");
    assert_eq!(native.mcp_bundle_digest(), KIMI_NATIVE_MCP_SOURCE);
    assert!(!native.mcp_bundle_digest().starts_with("sha256:"));

    // aria:注入 bundle digest 原样进投影(既有 bundle digest 为裸 64 位
    // hex,与 native 标记可区分)。
    let aria = projector
        .project(&input_with_mcp(bundle.digest()))
        .expect("aria-bundle projection is produced");
    assert_eq!(aria.mcp_bundle_digest(), bundle.digest());
    assert_eq!(aria.mcp_bundle_digest().len(), 64);
    assert_ne!(aria.mcp_bundle_digest(), KIMI_NATIVE_MCP_SOURCE);

    // 来源漂移两层 digest 都变(可检)。
    assert_ne!(native.projection_digest(), aria.projection_digest());
    assert_ne!(
        native.capability_projection_digest(),
        aria.capability_projection_digest()
    );

    // 空串来源 fail-closed:必须显式标记 Aria digest 或 native 来源。
    let empty = projector.project(&input_with_mcp(""));
    assert!(
        empty.is_err(),
        "empty mcp source must not project as either aria or native"
    );
}

// ==== Task 9c:LC 显式 resume 的 MCP bundle 漂移不走 session/new ====

use super::session_tests::{
    LcKimiResumeFixture, lc_kimi_resume_fixture, t09c_child_killed_and_reaped,
};
use crate::cross_cutting::tool_policy_audit::test_support::RecordingToolPolicyAuditSink;
use crate::cross_cutting::kimi_code_provider::KimiCodeProvider;
use crate::cross_cutting::streaming_provider::StreamingProviderAdapter;

/// Task 9c(冻结决策:MCP bundle 漂移不走 session/new):LC 显式 resume 的
/// 冻结 bundle digest 与当前注入 bundle 漂移时,direct 路径的「拒绝 load、
/// 静默 session/new」不适用于 LC——drift 属于已启动 child 后的 runtime 失败:
/// 不发 session/new、kill/reap、错误显式记录「未恢复」(不伪称零 spawn),
/// 零新 provider_start;显式 fresh 只能由用户重新发起。
#[cfg(unix)]
#[tokio::test]
async fn lcg_t09_kimi_bundle_drift_never_session_new() {
    let fixture = LcKimiResumeFixture::new();
    let marker_dir = tempfile::tempdir().expect("t09c drift marker dir");
    let pid_marker = marker_dir.path().join("kimi-t09c-drift.pid");
    let new_marker = marker_dir.path().join("kimi-t09c-drift-session-new.marker");
    let sink = RecordingToolPolicyAuditSink::new();
    let mut raw = fixture.lc_streaming_input(
        Some(sink.clone().bound()),
        Some("kimi-session-drift-t09c".to_string()),
    );
    raw.env_vars.insert(
        "KIMI_PID_MARKER".to_string(),
        pid_marker.display().to_string(),
    );
    raw.env_vars.insert(
        "KIMI_NEW_MARKER".to_string(),
        new_marker.display().to_string(),
    );

    let bundle = codegraph_bundle();
    let frozen_digest = "frozen-digest-drifted-t09c".to_string();
    let provider = KimiCodeProvider::new(lc_kimi_resume_fixture(marker_dir.path()))
        .with_mcp_bundle_for_resume(bundle, frozen_digest.clone());
    let drifted_result = provider
        .start_validated(
            fixture.validated_coding_input(raw),
            CancellationToken::new(),
        )
        .await;
    let Err(rejected) = drifted_result else {
        panic!("bundle drift must not silently start a new session for an explicit LC resume")
    };
    assert!(
        rejected.details.contains("session NOT resumed")
            && rejected.details.contains("kimi-session-drift-t09c")
            && rejected.details.contains(&frozen_digest),
        "rejection must name the old session and frozen digest and record the not-resumed outcome: {rejected:?}"
    );
    assert!(
        sink.events().is_empty(),
        "no fresh provider_start may be written for a drifted explicit LC resume"
    );
    assert!(
        !new_marker.exists(),
        "session/new must never be sent after MCP bundle drift on an explicit LC resume"
    );
    let started_child_was_killed_and_reaped = t09c_child_killed_and_reaped(&pid_marker);
    assert!(started_child_was_killed_and_reaped);
}
