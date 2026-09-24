//! REQ-NDR-05 用例：pi usage 兜底链路的失败环节必须以结构化 warn 留痕。
//!
//! 捕获用进程级 `SharedTracingCapture::global()`（见 `cross_cutting/tracing_capture.rs`，
//! 该 fixture 已解决 tracing Interest 进程级缓存导致的线程局部 guard 失效问题）。
//! 断言只用特异字段值（`stage="…"` / session 文件绝对路径），避免并行测试串扰。

use super::*;
use crate::cross_cutting::pi_provider::session::{PiUsageStage, warn_pi_usage_unavailable};
use crate::cross_cutting::streaming_provider::ProviderEvent;
use crate::cross_cutting::tracing_capture::SharedTracingCapture;

/// 会话 settle 后 usage `get_state` 的脚本行为。
enum UsageStateScript {
    /// 正常回应（`data` 由用例给出）。
    Respond(Value),
    /// 关闭 server→client 写端：usage `get_state` 写入成功但响应永不到达。
    CloseWithoutResponse,
}

/// 跑一个会话：握手 / prompt / 文本输出正常，`agent_settled` 后按脚本处理 usage
/// `get_state`。返回会话内除终态外收集到的事件（含 usage 上报）。
async fn run_usage_probe(script: UsageStateScript) -> Vec<ProviderEvent> {
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
                "success": true, "data": {"sessionId": "sess-usage-probe"}
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
                "assistantMessageEvent": {"type": "text_delta", "contentIndex": 0, "delta": "done"}
            }),
        )
        .await;
        write_inbound(
            &mut server_writer,
            serde_json::json!({ "type": "agent_settled" }),
        )
        .await;
        let usage_state = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            read_outbound(&mut reader),
        )
        .await
        .expect("usage get_state must be sent after settle");
        // pi RPC 请求只带 `type`（响应才带 `command`）。
        assert_eq!(usage_state["type"], "get_state");
        match script {
            UsageStateScript::Respond(data) => {
                write_inbound(
                    &mut server_writer,
                    serde_json::json!({
                        "type": "response", "id": usage_state["id"], "command": "get_state",
                        "success": true, "data": data
                    }),
                )
                .await;
            }
            UsageStateScript::CloseWithoutResponse => {
                // 优雅关闭 server→client 写端：客户端 usage get_state 写入仍成功，
                // 随后读到 EOF（`tokio::io::split` 的 WriteHalf 必须走 shutdown，
                // 直接 drop 不会关闭底层 duplex）。
                server_writer
                    .shutdown()
                    .await
                    .expect("close server write side");
                let mut ignored = String::new();
                let _ = tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    reader.read_line(&mut ignored),
                )
                .await;
            }
        }
    });

    let run = tokio::spawn(run_pi_session(
        peer,
        command_rx,
        event_tx,
        streaming_input_for_test(None),
        CancellationToken::new(),
    ));
    let events = drain_events(&mut event_rx).await;
    run.await.expect("session task").expect("session complete");
    server.await.expect("server");
    events
}

/// 环节标签是稳定结构化字段（`stage="…"`），且消息体可检索。
#[test]
fn pi_usage_stage_warns_carry_stable_stage_labels() {
    let capture = SharedTracingCapture::global();
    for stage in [
        PiUsageStage::RpcSend,
        PiUsageStage::RpcResponse,
        PiUsageStage::Parse,
        PiUsageStage::File,
        PiUsageStage::Emit,
    ] {
        warn_pi_usage_unavailable(stage, Some("probe detail"));
    }
    let logs = capture.captured();
    for label in ["rpc_send", "rpc_response", "parse", "file", "emit"] {
        assert!(
            logs.contains(&format!("stage=\"{label}\"")),
            "missing stage={label}: {logs}"
        );
    }
    assert!(logs.contains("pi usage unavailable"), "logs: {logs}");
    assert!(logs.contains("probe detail"), "logs: {logs}");
}

/// 响应既无 `cost` 也无 `sessionFile`：环节=parse（协议层没有可读来源）。
#[tokio::test]
async fn pi_usage_state_without_source_logs_parse_stage() {
    let capture = SharedTracingCapture::global();
    let events = run_usage_probe(UsageStateScript::Respond(
        serde_json::json!({ "sessionId": "sess-usage-parse" }),
    ))
    .await;

    assert!(
        !events
            .iter()
            .any(|event| matches!(event, ProviderEvent::UsageReport(_))),
        "no usage source must not report usage"
    );
    let logs = capture.captured();
    assert!(logs.contains("stage=\"parse\""), "logs: {logs}");
    assert!(logs.contains("pi usage unavailable"), "logs: {logs}");
    assert!(
        logs.contains("no cost and no absolute session_file"),
        "parse-stage warn must name the missing source: {logs}"
    );
}

/// `sessionFile` 可定位但本地记录里没有 usage：环节=file（带路径，事后可定责）。
#[tokio::test]
async fn pi_usage_unreadable_local_record_logs_file_stage() {
    let capture = SharedTracingCapture::global();
    let session_root = tempfile::tempdir().expect("tempdir");
    let session_file = session_root.path().join("pi-usage-session.jsonl");
    std::fs::write(
        &session_file,
        "{\"type\":\"message\",\"content\":\"no usage in this session file\"}\n",
    )
    .expect("write session file");

    let events = run_usage_probe(UsageStateScript::Respond(serde_json::json!({
        "sessionFile": session_file,
    })))
    .await;

    assert!(
        !events
            .iter()
            .any(|event| matches!(event, ProviderEvent::UsageReport(_))),
        "unreadable local record must not report usage"
    );
    let logs = capture.captured();
    assert!(logs.contains("stage=\"file\""), "logs: {logs}");
    assert!(
        logs.contains(&session_file.to_string_lossy().to_string()),
        "file-stage warn must name the session file: {logs}"
    );
}

/// usage `get_state` 无响应（流提前结束）：环节=rpc_response。
#[tokio::test]
async fn pi_usage_response_lost_logs_rpc_stage() {
    let capture = SharedTracingCapture::global();
    let events = run_usage_probe(UsageStateScript::CloseWithoutResponse).await;

    assert!(
        !events
            .iter()
            .any(|event| matches!(event, ProviderEvent::UsageReport(_))),
        "lost response must not report usage"
    );
    let logs = capture.captured();
    assert!(logs.contains("stage=\"rpc_response\""), "logs: {logs}");
}
