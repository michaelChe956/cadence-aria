// F3 Task 4.1（restrict-role-write-tools，GC12）：kimi 零变化回归。
// ①argv 冻结：仅 `acp`，不注入任何 tool-policy 物理片段（pi/claude/codex 的
//   denylist 片段不得串入 kimi）；
// ②kimi 不读 `tool_policy`/`audit_sink`：即使 input 误挂策略与 run-bound
//   durable sink，会话行为零变化、不向 sink 追加任何 tool-policy canonical
//   事件（tool-policy-run-audit/ 分区文件因此永不因 kimi 产生）；
// ③出站 request id 保持既有数字命名空间（initialize=1/session/new=2/
//   session/prompt=3，Task 2.2 只升级 codex peer 到 aria-<seq>）。

use std::collections::BTreeMap;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::json_rpc_peer::JsonRpcPeer;
use crate::cross_cutting::kimi_code_provider::session::run_kimi_session;
use crate::cross_cutting::kimi_code_provider::{
    KimiCodeProvider, tests::session_tests::fixture_command,
};
use crate::cross_cutting::streaming_provider::{
    ProviderEvent, ProviderPermissionMode, ProviderToolPolicy, StreamingProviderAdapter,
    StreamingProviderInput,
};
use crate::cross_cutting::tool_policy_audit::test_support::RecordingToolPolicyAuditSink;
use crate::protocol::contracts::{AdapterRole, ProviderType};

fn kimi_input(role: AdapterRole) -> StreamingProviderInput {
    StreamingProviderInput {
        tool_policy: None,
        audit_sink: None,
        provider_type: ProviderType::KimiCode,
        role,
        prompt: "kimi zero change prompt".to_string(),
        working_dir: std::env::current_dir().expect("working directory"),
        workspace_session_id: None,
        resume_provider_session_id: None,
        permission_mode: ProviderPermissionMode::Auto,
        structured_output_contract: None,
        env_vars: BTreeMap::new(),
        timeout_secs: 10,
    }
}

async fn read_wire_request(
    reader: &mut BufReader<tokio::io::ReadHalf<tokio::io::DuplexStream>>,
) -> Value {
    let mut line = String::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        reader.read_line(&mut line),
    )
    .await
    .expect("wire line timeout")
    .expect("wire line");
    assert!(!line.trim().is_empty(), "unexpected empty wire line");
    serde_json::from_str(line.trim()).expect("wire json")
}

async fn write_wire_line(
    writer: &mut tokio::io::WriteHalf<tokio::io::DuplexStream>,
    value: &Value,
) {
    writer
        .write_all(value.to_string().as_bytes())
        .await
        .expect("write wire line");
    writer.write_all(b"\n").await.expect("write wire newline");
}

/// ①argv 冻结：kimi build_args 是完整向量等值 `["acp"]`——不因本 change 混入
/// `--exclude-tools`/`--disallowedTools` 等策略片段。
#[test]
fn kimi_build_args_stay_policy_free() {
    let provider = KimiCodeProvider::new("kimi".into());
    let args = provider.build_args();
    assert_eq!(args, vec!["acp".to_string()]);
    assert!(
        !args.iter().any(|arg| arg.contains("exclude-tools")
            || arg.contains("disallowedTools")
            || arg.contains("sandbox")),
        "kimi argv must not carry any provider tool-policy fragment: {args:?}"
    );
}

/// ③出站 request id 保持数字：真实 `run_kimi_session` wire 级断言
/// initialize=1、session/new=2、session/prompt=3（JSON 数字，非 aria-<seq>）。
#[tokio::test]
async fn kimi_outbound_request_ids_stay_numeric() {
    let (client, server) = tokio::io::duplex(16 * 1024);
    let (reader, writer) = tokio::io::split(client);
    let peer = JsonRpcPeer::new(reader, writer);
    let (event_tx, mut event_rx) = mpsc::channel(32);
    let (_command_tx, command_rx) = mpsc::channel(8);

    let server_task = tokio::spawn(async move {
        let (server_reader, mut server_writer) = tokio::io::split(server);
        let mut reader = BufReader::new(server_reader);

        let initialize = read_wire_request(&mut reader).await;
        assert_eq!(initialize["method"], "initialize");
        assert_eq!(
            initialize["id"],
            json!(1),
            "kimi initialize id stays numeric 1"
        );
        write_wire_line(
            &mut server_writer,
            &json!({
                "jsonrpc":"2.0", "id": initialize["id"].clone(),
                "result": {"protocolVersion": 1, "agentCapabilities": {"loadSession": true}}
            }),
        )
        .await;

        // notifications/initialized：通知无 id。
        let initialized = read_wire_request(&mut reader).await;
        assert_eq!(initialized["method"], "notifications/initialized");
        assert!(initialized.get("id").is_none());

        let session_new = read_wire_request(&mut reader).await;
        assert_eq!(session_new["method"], "session/new");
        assert_eq!(
            session_new["id"],
            json!(2),
            "kimi session/new id stays numeric 2"
        );
        write_wire_line(
            &mut server_writer,
            &json!({
                "jsonrpc":"2.0", "id": session_new["id"].clone(),
                "result": {"sessionId": "kimi-numeric-ids"}
            }),
        )
        .await;

        let prompt = read_wire_request(&mut reader).await;
        assert_eq!(prompt["method"], "session/prompt");
        assert_eq!(
            prompt["id"],
            json!(3),
            "kimi session/prompt id stays numeric 3"
        );
        assert!(
            !prompt["id"].is_string(),
            "kimi outbound ids must never become aria-<seq> strings"
        );
        write_wire_line(
            &mut server_writer,
            &json!({
                "jsonrpc":"2.0", "method":"session/update",
                "params": {"sessionId":"kimi-numeric-ids", "update": {
                    "sessionUpdate":"agent_message_chunk",
                    "content": {"type":"text","text":"kimi numeric ids"}
                }}
            }),
        )
        .await;
        write_wire_line(
            &mut server_writer,
            &json!({
                "jsonrpc":"2.0", "id": prompt["id"].clone(),
                "result": {"stopReason": "end_turn"}
            }),
        )
        .await;
    });

    run_kimi_session(
        peer,
        command_rx,
        event_tx,
        kimi_input(AdapterRole::Orchestrator),
        CancellationToken::new(),
    )
    .await
    .expect("kimi session completes");
    server_task.await.expect("server task");

    loop {
        match tokio::time::timeout(std::time::Duration::from_secs(2), event_rx.recv())
            .await
            .expect("kimi event timeout")
            .expect("kimi event channel open")
        {
            ProviderEvent::Completed(completion) => {
                assert_eq!(completion.full_output, "kimi numeric ids");
                return;
            }
            ProviderEvent::Failed { message } => panic!("kimi session failed: {message}"),
            _ => continue,
        }
    }
}

/// ②kimi 忽略 tool_policy/audit_sink：Orchestrator（策略角色档）input 误挂
/// deny 策略与 run-bound durable sink 时，会话行为零变化、sink 零追加。
#[tokio::test]
async fn kimi_session_ignores_tool_policy_and_never_appends_tool_policy_audit() {
    let sink = RecordingToolPolicyAuditSink::new();
    let provider = KimiCodeProvider::new(fixture_command("kimi_acp_text_fixture.sh"));
    let mut input = kimi_input(AdapterRole::Orchestrator);
    input.tool_policy = Some(ProviderToolPolicy::deny_file_write_builtins());
    input.audit_sink = Some(sink.clone().bound());

    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .expect("kimi starts even with a mistakenly attached policy");

    #[allow(unused_assignments)]
    let mut completed = String::new();
    loop {
        match tokio::time::timeout(std::time::Duration::from_secs(2), session.events.recv())
            .await
            .expect("kimi policy-attached event timeout")
            .expect("kimi policy-attached channel open")
        {
            ProviderEvent::Completed(completion) => {
                completed = completion.full_output;
                break;
            }
            ProviderEvent::Failed { message } => panic!("kimi session failed: {message}"),
            _ => continue,
        }
    }
    assert_eq!(
        completed, "Kimi fixture output",
        "attached policy must not change kimi behavior"
    );
    assert!(
        sink.events().is_empty(),
        "kimi must never append tool-policy canonical events (no provider_start, \
         no approval_decision, no protocol_warning, no session_terminated): {:?}",
        sink.events()
    );
}
