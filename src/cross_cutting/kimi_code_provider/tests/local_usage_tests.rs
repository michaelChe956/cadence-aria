// 迁移自 session_tests.rs（large_file_guard 1200 行红线拆分）：
// kimi 会话完成与引擎 run token 取消竞态的回归测试（真机 issue_0035）。
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

use crate::cross_cutting::json_rpc_peer::JsonRpcPeer;
use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::cross_cutting::streaming_provider::{
    ProviderCommand, ProviderEvent, StreamingProviderInput,
};
use tokio_util::sync::CancellationToken;

use super::super::session::run_kimi_session;

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

fn input(resume: Option<&str>, timeout_secs: u64) -> StreamingProviderInput {
    use crate::cross_cutting::streaming_provider::ProviderPermissionMode;
    use crate::protocol::contracts::{AdapterRole, ProviderType};
    use std::collections::BTreeMap;
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

fn test_peer() -> (
    JsonRpcPeer<tokio::io::WriteHalf<tokio::io::DuplexStream>>,
    tokio::io::DuplexStream,
) {
    let (client, server) = tokio::io::duplex(16 * 1024);
    let (reader, writer) = tokio::io::split(client);
    (JsonRpcPeer::new(reader, writer), server)
}

async fn direct_session_events_with_cancel<W>(
    peer: JsonRpcPeer<W>,
    input: StreamingProviderInput,
    cancel: CancellationToken,
) -> (
    tokio::sync::mpsc::Sender<crate::cross_cutting::streaming_provider::ProviderCommand>,
    tokio::sync::mpsc::Receiver<crate::cross_cutting::streaming_provider::ProviderEvent>,
    tokio::task::JoinHandle<Result<(), ProviderAdapterError>>,
)
where
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (commands, command_rx) = tokio::sync::mpsc::channel(8);
    let (event_tx, events) = tokio::sync::mpsc::channel(32);
    let run = tokio::spawn(run_kimi_session(peer, command_rx, event_tx, input, cancel));
    (commands, events, run)
}

// 回归（真机 issue_0035）：kimi 会话正常完成（含 AskUserQuestion 选择后续生成 + end_turn）
// 后，KimiClientServiceDispatcher 的 Drop 不得 cancel 引擎的 run token——否则引擎
// biased select 的 cancel 分支会抢先于已入队的 Completed 事件，把 author_run 标为
// 「运行已中止」并丢失 artifact 提取。
#[tokio::test]
async fn successful_choice_completion_does_not_cancel_engine_run_token() {
    let (peer, server) = test_peer();
    let run_cancel = CancellationToken::new();
    let (commands, mut events, run) =
        direct_session_events_with_cancel(peer, input(None, 10), run_cancel.clone()).await;
    let server_task = tokio::spawn(async move {
        let (reader, mut writer) = tokio::io::split(server);
        let mut reader = tokio::io::BufReader::new(reader);
        let initialize = read_request(&mut reader).await;
        send_message(&mut writer, serde_json::json!({
                "jsonrpc":"2.0", "id":initialize["id"],
                "result":{"protocolVersion":1,"agentCapabilities":{"loadSession":true,"sessionCapabilities":{"resume":{}}}}
            })).await;
        let _initialized = read_request(&mut reader).await;
        let new = read_request(&mut reader).await;
        send_message(
            &mut writer,
            serde_json::json!({
                "jsonrpc":"2.0", "id":new["id"], "result":{"sessionId":"choice_complete"}
            }),
        )
        .await;
        let prompt = read_request(&mut reader).await;
        send_message(&mut writer, serde_json::json!({
                "jsonrpc":"2.0", "method":"session/update",
                "params":{"sessionId":"choice_complete","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"before choice; "}}}
            })).await;
        send_message(&mut writer, serde_json::json!({
                "jsonrpc":"2.0", "id":"choice-9", "method":"session/request_permission",
                "params":{"options":[{"optionId":"q0_opt_0","name":"30 天","kind":"allow_once"},{"optionId":"q0_opt_1","name":"7 天","kind":"allow_once"}],"toolCall":{"toolCallId":"choice-tool-9","title":"AskUserQuestion","content":{"type":"text","text":"迁移默认值选 30 天还是 7 天？"}}}
            })).await;
        // 用户选择后 kimi 继续在同一 prompt 内生成
        let choice_reply = read_request(&mut reader).await;
        assert_eq!(choice_reply["id"], "choice-9");
        assert_eq!(choice_reply["result"]["outcome"]["outcome"], "selected");
        send_message(&mut writer, serde_json::json!({
                "jsonrpc":"2.0", "method":"session/update",
                "params":{"sessionId":"choice_complete","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"after choice"}}}
            })).await;
        send_message(
            &mut writer,
            serde_json::json!({
                "jsonrpc":"2.0", "id":prompt["id"], "result":{"stopReason":"end_turn"}
            }),
        )
        .await;
    });

    let choice = loop {
        match events.recv().await.expect("choice event") {
            ProviderEvent::ChoiceRequest(choice) => break choice,
            ProviderEvent::StatusChanged(_)
            | ProviderEvent::TextDelta { .. }
            | ProviderEvent::Execution(_) => {}
            other => panic!("unexpected event before choice: {other:?}"),
        }
    };
    commands
        .send(ProviderCommand::ChoiceResponse {
            id: choice.id,
            selected_option_ids: vec!["q0_opt_0".to_string()],
            free_text: None,
            answers: Vec::new(),
        })
        .await
        .expect("selected-option choice response");

    let received = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        let mut received = Vec::new();
        loop {
            let event = events.recv().await.expect("event after choice");
            let terminal = matches!(
                event,
                ProviderEvent::Completed(_) | ProviderEvent::Failed { .. }
            );
            received.push(event);
            if terminal {
                return received;
            }
        }
    })
    .await
    .expect("completion after selected-option choice");
    let completion = received
        .iter()
        .find_map(|event| match event {
            ProviderEvent::Completed(completion) => Some(completion),
            _ => None,
        })
        .unwrap_or_else(|| panic!("completion expected; events: {received:?}"));
    assert_eq!(completion.full_output, "before choice; after choice");
    assert!(run.await.expect("run join").is_ok());
    server_task.await.expect("server task");
    assert!(
        !run_cancel.is_cancelled(),
        "successful kimi completion must not cancel the engine run token"
    );
}
