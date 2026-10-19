use std::collections::BTreeMap;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll};

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot};

use crate::cross_cutting::streaming_provider::{
    ProviderCommand, ProviderEvent, ProviderPermissionMode, StreamingProviderInput,
};
use crate::protocol::contracts::{AdapterRole, ProviderType};

use crate::cross_cutting::json_rpc_peer::JsonRpcPeer;
use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use tokio_util::sync::CancellationToken;

use super::super::session::run_kimi_session;
use super::super::{KimiCodeProvider, StreamingProviderAdapter};

pub(crate) fn fixture_command(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/provider")
        .join(name)
}

pub(crate) fn input(resume: Option<&str>, timeout_secs: u64) -> StreamingProviderInput {
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

fn input_with_env(
    resume: Option<&str>,
    timeout_secs: u64,
    env_vars: BTreeMap<String, String>,
) -> StreamingProviderInput {
    StreamingProviderInput {
        working_directory: None,
        env_vars,
        ..input(resume, timeout_secs)
    }
}

async fn terminal_events(
    session: &mut crate::cross_cutting::streaming_provider::ProviderSession,
) -> Vec<ProviderEvent> {
    let mut events = Vec::new();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    while let Ok(Some(event)) = tokio::time::timeout_at(deadline, session.events.recv()).await {
        let terminal = matches!(
            event,
            ProviderEvent::Completed(_) | ProviderEvent::Failed { .. }
        );
        events.push(event);
        if terminal {
            break;
        }
    }
    events
}

pub(crate) async fn read_request(
    reader: &mut tokio::io::BufReader<impl tokio::io::AsyncRead + Unpin>,
) -> Value {
    let mut line = String::new();
    reader.read_line(&mut line).await.expect("read request");
    serde_json::from_str(&line).expect("JSON-RPC request")
}

pub(crate) async fn send_message(writer: &mut (impl tokio::io::AsyncWrite + Unpin), value: Value) {
    writer
        .write_all(value.to_string().as_bytes())
        .await
        .expect("write message");
    writer.write_all(b"\n").await.expect("write newline");
    writer.flush().await.expect("flush message");
}

pub(crate) fn test_peer() -> (
    JsonRpcPeer<tokio::io::WriteHalf<tokio::io::DuplexStream>>,
    tokio::io::DuplexStream,
) {
    let (client, server) = tokio::io::duplex(16 * 1024);
    let (reader, writer) = tokio::io::split(client);
    (JsonRpcPeer::new(reader, writer), server)
}

pub(crate) async fn direct_session_events<W>(
    peer: JsonRpcPeer<W>,
    input: StreamingProviderInput,
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
    let run = tokio::spawn(run_kimi_session(
        peer,
        command_rx,
        event_tx,
        input,
        CancellationToken::new(),
    ));
    (commands, events, run)
}

struct ToggleFailWriter<W> {
    inner: W,
    fail_writes: Arc<AtomicBool>,
}

impl<W> tokio::io::AsyncWrite for ToggleFailWriter<W>
where
    W: tokio::io::AsyncWrite + Unpin,
{
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let this = self.get_mut();
        if this.fail_writes.load(Ordering::SeqCst) {
            return Poll::Ready(Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "simulated closed ACP stdin",
            )));
        }
        Pin::new(&mut this.inner).poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        let this = self.get_mut();
        if this.fail_writes.load(Ordering::SeqCst) {
            return Poll::Ready(Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "simulated closed ACP stdin",
            )));
        }
        Pin::new(&mut this.inner).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

#[test]
fn kimi_version_parser_and_gate_enforce_minimum() {
    assert_eq!(
        super::super::parse_kimi_version("kimi 0.34.0"),
        super::super::KimiVersion(0, 34, 0)
    );
    assert!(
        super::super::ensure_kimi_version_compatible(&super::super::parse_kimi_version(
            "kimi 0.33.9"
        ))
        .is_err()
    );
    assert!(
        super::super::ensure_kimi_version_compatible(&super::super::parse_kimi_version(
            "kimi 0.34.0"
        ))
        .is_ok()
    );
}

#[tokio::test]
async fn text_turn_completes_with_full_output() {
    let provider = KimiCodeProvider::new(fixture_command("kimi_acp_text_fixture.sh"));
    let mut session = provider
        .start(input(None, 10), CancellationToken::new())
        .await
        .expect("start");
    let events = terminal_events(&mut session).await;
    assert!(events.iter().any(|event| matches!(event, ProviderEvent::TextDelta { content } if content == "Kimi fixture output")));
    let completion = events
        .iter()
        .find_map(|event| match event {
            ProviderEvent::Completed(completion) => Some(completion),
            _ => None,
        })
        .expect("completion");
    assert_eq!(completion.full_output, "Kimi fixture output");
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(
                event,
                ProviderEvent::Completed(_) | ProviderEvent::Failed { .. }
            ))
            .count(),
        1,
        "exactly one successful-or-failed terminal event"
    );
    assert_eq!(
        completion.provider_session_id.as_deref(),
        Some("kimi_text_fixture")
    );
}

#[tokio::test]
async fn tool_call_emits_toolcall_then_toolresult_once() {
    let provider = KimiCodeProvider::new(fixture_command("kimi_acp_tool_fixture.sh"));
    let mut session = provider
        .start(input(None, 10), CancellationToken::new())
        .await
        .expect("start");
    let events = terminal_events(&mut session).await;
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, ProviderEvent::ToolCall(_)))
            .count(),
        2
    );
    let results = events
        .iter()
        .filter_map(|event| match event {
            ProviderEvent::ToolResult(result) => Some(result),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(results.len(), 2);
    assert!(results.iter().any(|result| result.tool_use_id == "tool_1"
        && !result.is_error
        && result.output.contains("tmp")));
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(
                event,
                ProviderEvent::Completed(_) | ProviderEvent::Failed { .. }
            ))
            .count(),
        1,
        "tool turn emits exactly one terminal event"
    );
}

#[tokio::test]
async fn protocol_and_resume_capability_rejections_emit_one_failed() {
    for response in [
        serde_json::json!({"protocolVersion":2,"agentCapabilities":{"loadSession":true,"sessionCapabilities":{"resume":{}}}}),
        serde_json::json!({"protocolVersion":1,"agentCapabilities":{"loadSession":false,"sessionCapabilities":{}}}),
    ] {
        let (peer, server) = test_peer();
        let (_commands, _events, run) =
            direct_session_events(peer, input(Some("resume"), 10)).await;
        let server_task = tokio::spawn(async move {
            let (reader, mut writer) = tokio::io::split(server);
            let mut reader = tokio::io::BufReader::new(reader);
            let initialize = read_request(&mut reader).await;
            send_message(
                &mut writer,
                serde_json::json!({"jsonrpc":"2.0","id":initialize["id"],"result":response}),
            )
            .await;
        });
        assert!(run.await.expect("run join").is_err());
        server_task.await.expect("server task");
    }
}

#[tokio::test]
async fn unknown_request_receives_method_not_found_and_notification_is_ignored() {
    let (peer, server) = test_peer();
    let (_commands, mut events, run) = direct_session_events(peer, input(None, 10)).await;
    let server_task = tokio::spawn(async move {
        let (reader, mut writer) = tokio::io::split(server);
        let mut reader = tokio::io::BufReader::new(reader);
        let initialize = read_request(&mut reader).await;
        send_message(&mut writer, serde_json::json!({"jsonrpc":"2.0","id":initialize["id"],"result":{"protocolVersion":1,"agentCapabilities":{"loadSession":true,"sessionCapabilities":{"resume":{}}}}})).await;
        let _initialized = read_request(&mut reader).await;
        let new = read_request(&mut reader).await;
        send_message(&mut writer, serde_json::json!({"jsonrpc":"2.0","id":new["id"],"result":{"sessionId":"unknown_fixture"}})).await;
        let _prompt = read_request(&mut reader).await;
        send_message(
            &mut writer,
            serde_json::json!({"jsonrpc":"2.0","method":"future/notification","params":{}}),
        )
        .await;
        send_message(&mut writer, serde_json::json!({"jsonrpc":"2.0","id":"unknown-request","method":"future/request","params":{}})).await;
        let reply = read_request(&mut reader).await;
        assert_eq!(reply["id"], "unknown-request");
        assert_eq!(reply["error"]["code"], -32601);
    });
    let mut seen_completed = 0;
    while let Some(event) = tokio::time::timeout(std::time::Duration::from_secs(1), events.recv())
        .await
        .expect("event")
    {
        if matches!(event, ProviderEvent::Completed(_)) {
            seen_completed += 1;
            break;
        }
        assert!(!matches!(event, ProviderEvent::Failed { .. }));
    }
    assert_eq!(seen_completed, 0);
    assert!(run.await.expect("run join").is_err());
    server_task.await.expect("server task");
}

#[tokio::test]
async fn unknown_permission_option_replies_cancelled_with_original_rpc_id() {
    let (peer, server) = test_peer();
    let (_commands, _events, run) = direct_session_events(peer, input(None, 10)).await;
    let server_task = tokio::spawn(async move {
        let (reader, mut writer) = tokio::io::split(server);
        let mut reader = tokio::io::BufReader::new(reader);
        let initialize = read_request(&mut reader).await;
        send_message(&mut writer, serde_json::json!({"jsonrpc":"2.0","id":initialize["id"],"result":{"protocolVersion":1,"agentCapabilities":{"loadSession":true,"sessionCapabilities":{"resume":{}}}}})).await;
        let _initialized = read_request(&mut reader).await;
        let new = read_request(&mut reader).await;
        send_message(&mut writer, serde_json::json!({"jsonrpc":"2.0","id":new["id"],"result":{"sessionId":"permission_fixture"}})).await;
        let _prompt = read_request(&mut reader).await;
        send_message(&mut writer, serde_json::json!({"jsonrpc":"2.0","id":"permission-id","method":"session/request_permission","params":{"options":[{"optionId":"future","name":"Future","kind":"future_kind"}],"toolCall":{"toolCallId":"tool","title":"Bash","content":[]}}})).await;
        let reply = read_request(&mut reader).await;
        assert_eq!(reply["id"], "permission-id");
        assert_eq!(
            reply["result"]["outcome"],
            serde_json::json!({"outcome":"cancelled"})
        );
    });
    assert!(run.await.expect("run join").is_err());
    server_task.await.expect("server task");
}

#[tokio::test]
async fn total_timeout_cancels_once_without_failed() {
    let provider = KimiCodeProvider::new(fixture_command("kimi_acp_timeout_fixture.sh"));
    let mut session = provider
        .start(input(None, 1), CancellationToken::new())
        .await
        .expect("start");
    let mut statuses = Vec::new();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
    while tokio::time::Instant::now() < deadline {
        let event = tokio::time::timeout_at(deadline, session.events.recv())
            .await
            .expect("event wait");
        let Some(event) = event else {
            break;
        };
        statuses.push(event.clone());
        if matches!(
            event,
            ProviderEvent::StatusChanged(
                crate::cross_cutting::streaming_provider::ProviderStatus::Aborted
            )
        ) {
            break;
        }
    }
    assert_eq!(
        statuses
            .iter()
            .filter(|event| matches!(
                event,
                ProviderEvent::StatusChanged(
                    crate::cross_cutting::streaming_provider::ProviderStatus::Aborted
                )
            ))
            .count(),
        1
    );
    assert_eq!(
        statuses
            .iter()
            .filter(|event| matches!(event, ProviderEvent::Failed { .. }))
            .count(),
        0
    );
}

#[tokio::test]
async fn resume_stall_aborts_before_total_timeout() {
    let (peer, server) = test_peer();
    let (_commands, mut events, run) = direct_session_events(peer, input(Some("resume"), 10)).await;
    let (ready_tx, ready_rx) = oneshot::channel();
    let (finish_tx, finish_rx) = oneshot::channel();
    let server_task = tokio::spawn(async move {
        let (reader, mut writer) = tokio::io::split(server);
        let mut reader = tokio::io::BufReader::new(reader);
        let initialize = read_request(&mut reader).await;
        send_message(&mut writer, serde_json::json!({
                "jsonrpc":"2.0", "id":initialize["id"],
                "result":{"protocolVersion":1,"agentCapabilities":{"loadSession":true,"sessionCapabilities":{"resume":{}}}}
            })).await;
        let _initialized = read_request(&mut reader).await;
        let load = read_request(&mut reader).await;
        assert_eq!(load["method"], "session/load");
        send_message(
            &mut writer,
            serde_json::json!({
                "jsonrpc":"2.0", "id":load["id"], "result":{"sessionId":"stalled_resume"}
            }),
        )
        .await;
        let prompt = read_request(&mut reader).await;
        assert_eq!(prompt["method"], "session/prompt");
        ready_tx.send(()).expect("resume prompt ready");
        let cancel = read_request(&mut reader).await;
        assert_eq!(cancel["method"], "session/cancel");
        finish_rx.await.expect("finish server");
    });

    ready_rx.await.expect("resume path reached prompt");
    let started = tokio::time::Instant::now();
    let mut saw_aborted = false;
    let mut saw_failed = false;
    while !saw_aborted {
        let event = tokio::time::timeout(std::time::Duration::from_secs(1), events.recv())
            .await
            .expect("resume stall must be shorter than the ten-second total timeout")
            .expect("event value");
        saw_aborted = matches!(
            event,
            ProviderEvent::StatusChanged(
                crate::cross_cutting::streaming_provider::ProviderStatus::Aborted
            )
        );
        saw_failed |= matches!(event, ProviderEvent::Failed { .. });
    }
    assert!(
        started.elapsed() < std::time::Duration::from_secs(1),
        "resume stall must expire before the total timeout"
    );
    assert!(!saw_failed);
    finish_tx.send(()).expect("finish server signal");
    assert!(run.await.expect("run join").is_err());
    server_task.await.expect("server task");
}

fn host_sleep_bin() -> &'static str {
    if std::path::Path::new("/bin/sleep").exists() {
        "/bin/sleep"
    } else {
        "/usr/bin/sleep"
    }
}

/// 批2 Task 7（Bug45）红测：真实静默长 CHECK 跨 ACP stall 界仍完成。
///
/// resume 会话的 stall 界为 `KIMI_RESUME_STALL_TIMEOUT`（测试形态 100ms，
/// 即线上 60s 的等效界）。server 在 prompt 后发起 terminal/create 执行
/// 真实 `sleep 0.6`，期间**零 session/update**（静默 CHECK 形态）；
/// wait_for_exit 返回后才有输出与 end_turn。当前实现把「无 Progress
/// 事件」当无工作 stall → 100ms 即杀；接线后 ACP stall 必须读真实活动
/// registry（同一 terminal_id 的 Started→Running→Exited 状态），在途
/// terminal 暂停 stall 判定——且不延长 10s 总 deadline。
#[tokio::test]
async fn silent_terminal_across_resume_stall_boundary_completes() {
    let (peer, server) = test_peer();
    let (_commands, mut events, run) = direct_session_events(peer, input(Some("resume"), 10)).await;
    let (ready_tx, ready_rx) = oneshot::channel();
    let (finish_tx, finish_rx) = oneshot::channel();
    let server_task = tokio::spawn(async move {
        let (reader, mut writer) = tokio::io::split(server);
        let mut reader = tokio::io::BufReader::new(reader);
        let initialize = read_request(&mut reader).await;
        send_message(&mut writer, serde_json::json!({
                "jsonrpc":"2.0", "id":initialize["id"],
                "result":{"protocolVersion":1,"agentCapabilities":{"loadSession":true,"sessionCapabilities":{"resume":{}}}}
            })).await;
        let _initialized = read_request(&mut reader).await;
        let load = read_request(&mut reader).await;
        assert_eq!(load["method"], "session/load");
        send_message(
            &mut writer,
            serde_json::json!({
                "jsonrpc":"2.0", "id":load["id"], "result":{"sessionId":"terminal_resume"}
            }),
        )
        .await;
        let prompt = read_request(&mut reader).await;
        assert_eq!(prompt["method"], "session/prompt");
        ready_tx.send(()).expect("resume prompt ready");

        // 静默长 CHECK：terminal/create（argv 形态，真实 sleep 0.6s）。
        send_message(&mut writer, serde_json::json!({
            "jsonrpc":"2.0", "id":100, "method":"terminal/create",
            "params":{"sessionId":"terminal_resume", "command":host_sleep_bin(), "args":["0.65"], "cwd":""}
        })).await;
        let create_reply = read_request(&mut reader).await;
        let terminal_id = create_reply["result"]["terminalId"]
            .as_str()
            .expect("terminal create reply carries terminalId")
            .to_string();
        // wait_for_exit 期间会话无任何 session/update（静默）。
        send_message(
            &mut writer,
            serde_json::json!({
                "jsonrpc":"2.0", "id":101, "method":"terminal/wait_for_exit",
                "params":{"sessionId":"terminal_resume", "terminalId":terminal_id}
            }),
        )
        .await;
        let wait_reply = read_request(&mut reader).await;
        assert_eq!(wait_reply["id"], serde_json::json!(101));
        assert_eq!(wait_reply["result"]["exitCode"], serde_json::json!(0));

        // CHECK 结束才有输出与 turn 终止。
        send_message(&mut writer, serde_json::json!({
            "jsonrpc":"2.0", "method":"session/update",
            "params":{"sessionId":"terminal_resume", "update":{"sessionUpdate":"agent_message_chunk", "content":{"type":"text","text":"check done"}}}
        })).await;
        send_message(
            &mut writer,
            serde_json::json!({
                "jsonrpc":"2.0", "id":prompt["id"], "result":{"stopReason":"end_turn"}
            }),
        )
        .await;
        finish_rx.await.expect("finish server");
    });

    ready_rx.await.expect("resume path reached prompt");
    let started = std::time::Instant::now();
    let mut saw_completed = false;
    let mut saw_aborted = false;
    while !saw_completed && !saw_aborted {
        let event = tokio::time::timeout(std::time::Duration::from_secs(5), events.recv())
            .await
            .expect("terminal run must stay inside the 10s total deadline")
            .expect("event value");
        saw_completed = matches!(event, ProviderEvent::Completed(_));
        saw_aborted |= matches!(
            event,
            ProviderEvent::StatusChanged(
                crate::cross_cutting::streaming_provider::ProviderStatus::Aborted
            )
        );
    }
    assert!(
        saw_completed,
        "真实静默 terminal 在途时 ACP stall 必须暂停，run 应正常完成（而非 60s 等效界误杀）"
    );
    assert!(started.elapsed() >= std::time::Duration::from_millis(500));
    finish_tx.send(()).expect("finish server signal");
    assert!(run.await.expect("run join").is_ok());
    server_task.await.expect("server task");
}

/// 批2 Task 7（Bug45）红测：活动 terminal 暂停 stall 判定但不延长总
/// deadline。
///
/// resume 会话（stall 界 100ms 等效），总 deadline 2s；prompt 后发起
/// 真实 `sleep 5` terminal 并保持静默。接线后：100ms 界被真实在途
/// terminal 暂停，但 2s 总 deadline 仍收口。
/// 当前实现 100ms 即按无工作 stall 误杀（elapsed < 1s）→ 红。
/// 资源回收由 dispatcher Drop→cleanup_all 沿既有 kill 链承担（迟到
/// create 的 spawn 前拒绝见 terminal_handlers 测试）。
///
/// 计时口径（确定性）：总 deadline 在 `run_kimi_session_inner` 入口即起算
/// （早于 initialize/load/prompt 握手），因此测点必须锚在会话任务启动之前
/// 而不是 prompt 之后——后者会把「随负载浮动的握手耗时」从窗口里减掉，
/// 使正确实现的机械仍可能测出 < 阈值（全量并行时曾出现 1.41s/1.49s）。
/// 锚点早于或等于 deadline 原点，故下界 `>= 2s` 精确成立：更早的 Aborted
/// 只可能来自 100ms stall 误杀；上界 `< 4s` 排除「terminal 活动延长了
/// deadline」——那要等真实 sleep 5 退出（≥5s）才收口。
#[tokio::test]
async fn active_terminal_pause_does_not_extend_total_deadline() {
    let (peer, server) = test_peer();
    // duration 起点：早于会话任务 spawn，因而早于会话内 deadline 原点。
    let invocation_started = std::time::Instant::now();
    let (_commands, mut events, run) = direct_session_events(peer, input(Some("resume"), 2)).await;
    let (ready_tx, ready_rx) = oneshot::channel();
    let (finish_tx, finish_rx) = oneshot::channel();
    let server_task = tokio::spawn(async move {
        let (reader, mut writer) = tokio::io::split(server);
        let mut reader = tokio::io::BufReader::new(reader);
        let initialize = read_request(&mut reader).await;
        send_message(&mut writer, serde_json::json!({
                "jsonrpc":"2.0", "id":initialize["id"],
                "result":{"protocolVersion":1,"agentCapabilities":{"loadSession":true,"sessionCapabilities":{"resume":{}}}}
            })).await;
        let _initialized = read_request(&mut reader).await;
        let load = read_request(&mut reader).await;
        assert_eq!(load["method"], "session/load");
        send_message(
            &mut writer,
            serde_json::json!({
                "jsonrpc":"2.0", "id":load["id"], "result":{"sessionId":"terminal_deadline"}
            }),
        )
        .await;
        let prompt = read_request(&mut reader).await;
        assert_eq!(prompt["method"], "session/prompt");
        ready_tx.send(()).expect("resume prompt ready");

        // 在途长 CHECK：真实 sleep 5s（超出 2s 总 deadline），会话静默。
        send_message(&mut writer, serde_json::json!({
            "jsonrpc":"2.0", "id":100, "method":"terminal/create",
            "params":{"sessionId":"terminal_deadline", "command":host_sleep_bin(), "args":["5"], "cwd":""}
        })).await;
        let create_reply = read_request(&mut reader).await;
        assert!(
            create_reply["result"]["terminalId"].is_string(),
            "create reply: {create_reply}"
        );

        // 总 deadline 收口：session/cancel 是 abort 的第一个可见信号。
        let cancel = read_request(&mut reader).await;
        assert_eq!(cancel["method"], "session/cancel");
        finish_rx.await.expect("finish server");
    });

    ready_rx.await.expect("resume path reached prompt");
    let mut saw_aborted = false;
    while !saw_aborted {
        let event = tokio::time::timeout(std::time::Duration::from_secs(5), events.recv())
            .await
            .expect("run must stay bounded")
            .expect("event value");
        saw_aborted = matches!(
            event,
            ProviderEvent::StatusChanged(
                crate::cross_cutting::streaming_provider::ProviderStatus::Aborted
            )
        );
    }
    let elapsed = invocation_started.elapsed();
    assert!(
        elapsed >= std::time::Duration::from_secs(2),
        "活动 terminal 只暂停 stall 判定，不得提前于总 deadline 误杀: {elapsed:?}"
    );
    assert!(
        elapsed < std::time::Duration::from_secs(4),
        "总 deadline（2s）必须仍然收口: {elapsed:?}"
    );
    assert!(run.await.expect("run join").is_err());
    finish_tx.send(()).expect("finish server signal");
    server_task.await.expect("server task");
}

#[tokio::test]
async fn cancel_write_failure_still_emits_aborted_without_failed() {
    let (client, server) = tokio::io::duplex(16 * 1024);
    let (reader, writer) = tokio::io::split(client);
    let fail_writes = Arc::new(AtomicBool::new(false));
    let peer = JsonRpcPeer::new(
        reader,
        ToggleFailWriter {
            inner: writer,
            fail_writes: Arc::clone(&fail_writes),
        },
    );
    let (commands, mut events, run) = direct_session_events(peer, input(None, 10)).await;
    let (ready_tx, ready_rx) = oneshot::channel();
    let (finish_tx, finish_rx) = oneshot::channel();
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
                "jsonrpc":"2.0", "id":new["id"], "result":{"sessionId":"cancel_write_failure"}
            }),
        )
        .await;
        let prompt = read_request(&mut reader).await;
        assert_eq!(prompt["method"], "session/prompt");
        ready_tx.send(()).expect("prompt ready");
        finish_rx.await.expect("finish server");
    });

    ready_rx.await.expect("session prompt ready");
    fail_writes.store(true, Ordering::SeqCst);
    commands
        .send(ProviderCommand::Abort)
        .await
        .expect("abort command");
    let mut saw_aborted = false;
    let mut saw_failed = false;
    while !saw_aborted {
        let event = tokio::time::timeout(std::time::Duration::from_secs(1), events.recv())
            .await
            .expect("event wait")
            .expect("event value");
        saw_aborted = matches!(
            event,
            ProviderEvent::StatusChanged(
                crate::cross_cutting::streaming_provider::ProviderStatus::Aborted
            )
        );
        saw_failed |= matches!(event, ProviderEvent::Failed { .. });
    }
    assert!(
        !saw_failed,
        "a best-effort cancel write failure must not emit Failed"
    );
    finish_tx.send(()).expect("finish server signal");
    assert!(run.await.expect("run join").is_err());
    server_task.await.expect("server task");
}

#[tokio::test]
async fn direct_load_error_does_not_send_prompt() {
    let (peer, server) = test_peer();
    let (_commands, _events, run) =
        direct_session_events(peer, input(Some("old-session"), 10)).await;
    let server_task = tokio::spawn(async move {
        let (reader, mut writer) = tokio::io::split(server);
        let mut reader = tokio::io::BufReader::new(reader);
        let initialize = read_request(&mut reader).await;
        send_message(&mut writer, serde_json::json!({
                "jsonrpc":"2.0", "id":initialize["id"],
                "result":{"protocolVersion":1,"agentCapabilities":{"loadSession":true,"sessionCapabilities":{"resume":{}}}}
            })).await;
        let _initialized = read_request(&mut reader).await;
        let load = read_request(&mut reader).await;
        assert_eq!(load["method"], "session/load");
        send_message(
            &mut writer,
            serde_json::json!({
                "jsonrpc":"2.0", "id":load["id"],
                "error":{"code":-32001,"message":"missing session"}
            }),
        )
        .await;
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        assert!(
            tokio::time::timeout(
                std::time::Duration::from_millis(100),
                read_request(&mut reader)
            )
            .await
            .is_err(),
            "load error must not send session/new or session/prompt"
        );
    });
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(1), run)
            .await
            .expect("session task")
            .expect("session join")
            .is_err()
    );
    server_task.await.expect("server task");
}

#[tokio::test]
async fn resume_load_replay_over_queue_capacity_does_not_timeout() {
    let requested_resume_id = "resume_load_replay";
    let (peer, server) = test_peer();
    let (_commands, mut events, run) =
        direct_session_events(peer, input(Some(requested_resume_id), 10)).await;
    let server_task = tokio::spawn(async move {
        let (reader, mut writer) = tokio::io::split(server);
        let mut reader = tokio::io::BufReader::new(reader);
        let initialize = read_request(&mut reader).await;
        send_message(&mut writer, serde_json::json!({
                "jsonrpc":"2.0", "id":initialize["id"],
                "result":{"protocolVersion":1,"agentCapabilities":{"loadSession":true,"sessionCapabilities":{"resume":{}}}}
            })).await;
        let _initialized = read_request(&mut reader).await;
        let load = read_request(&mut reader).await;
        assert_eq!(load["method"], "session/load");
        for index in 0..46 {
            send_message(&mut writer, serde_json::json!({
                    "jsonrpc":"2.0", "method":"session/update",
                    "params":{"sessionId":requested_resume_id,"update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":format!("historical-{index}")}}}
                })).await;
        }
        send_message(
            &mut writer,
            serde_json::json!({
                "jsonrpc":"2.0", "id":load["id"], "result":{}
            }),
        )
        .await;
        let prompt = read_request(&mut reader).await;
        assert_eq!(prompt["method"], "session/prompt");
        assert_eq!(prompt["params"]["sessionId"], requested_resume_id);
        send_message(&mut writer, serde_json::json!({
                "jsonrpc":"2.0", "method":"session/update",
                "params":{"sessionId":requested_resume_id,"update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"current prompt"}}}
            })).await;
        send_message(
            &mut writer,
            serde_json::json!({
                "jsonrpc":"2.0", "id":prompt["id"], "result":{"stopReason":"end_turn"}
            }),
        )
        .await;
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    });

    let events = tokio::time::timeout(std::time::Duration::from_millis(500), async {
        let mut received = Vec::new();
        loop {
            let Some(event) = events.recv().await else {
                return received;
            };
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
    .expect("session/load must complete before the replay fills the incoming queue");
    let run_result = run.await.expect("run join");
    let completion = events
        .iter()
        .find_map(|event| match event {
            ProviderEvent::Completed(completion) => Some(completion),
            _ => None,
        })
        .unwrap_or_else(|| panic!("completion; events: {events:?}; run: {run_result:?}"));
    assert_eq!(completion.full_output, "current prompt");
    assert_eq!(
        completion.provider_session_id.as_deref(),
        Some(requested_resume_id)
    );
    assert!(
        !completion.full_output.contains("historical-"),
        "session/load replay must not be forwarded into the current prompt output"
    );
    assert!(run_result.is_ok());
    server_task.await.expect("server task");
}

#[tokio::test]
async fn terminal_response_drains_prebuffered_updates_without_sleep() {
    let provider = KimiCodeProvider::new(fixture_command("kimi_acp_fast_fixture.sh"));
    let mut session = provider
        .start(input(None, 10), CancellationToken::new())
        .await
        .expect("start");
    let events = terminal_events(&mut session).await;
    let completion = events
        .iter()
        .find_map(|event| match event {
            ProviderEvent::Completed(completion) => Some(completion),
            _ => None,
        })
        .expect("completion");
    assert_eq!(completion.full_output, "first second");
    assert!(
        events
            .iter()
            .any(|event| matches!(event, ProviderEvent::ToolCall(call) if call.id == "fast_tool"))
    );
    assert!(events.iter().any(|event| matches!(event, ProviderEvent::ToolResult(result) if result.tool_use_id == "fast_tool" && result.output == "ok")));
}

#[tokio::test]
async fn prompt_response_wins_when_reader_closes_after_terminal_response() {
    let (peer, server) = test_peer();
    let (_commands, mut events, run) = direct_session_events(peer, input(None, 10)).await;
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
                "jsonrpc":"2.0", "id":new["id"], "result":{"sessionId":"terminal_response"}
            }),
        )
        .await;
        let prompt = read_request(&mut reader).await;
        send_message(&mut writer, serde_json::json!({
                "jsonrpc":"2.0", "method":"session/update",
                "params":{"sessionId":"terminal_response","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"complete"}}}
            })).await;
        send_message(
            &mut writer,
            serde_json::json!({
                "jsonrpc":"2.0", "id":prompt["id"], "result":{"stopReason":"end_turn"}
            }),
        )
        .await;
    });

    let events = tokio::time::timeout(std::time::Duration::from_secs(1), async {
        let mut received = Vec::new();
        loop {
            let Some(event) = events.recv().await else {
                return received;
            };
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
    .expect("terminal response must complete before reader closure");
    let completion = events
        .iter()
        .find_map(|event| match event {
            ProviderEvent::Completed(completion) => Some(completion),
            _ => None,
        })
        .unwrap_or_else(|| panic!("completion after terminal response; events: {events:?}"));
    assert_eq!(completion.full_output, "complete");
    assert!(run.await.expect("run join").is_ok());
    server_task.await.expect("server task");
}

#[tokio::test]
async fn prompt_eof_after_buffered_update_returns_stream_ended_error() {
    let (peer, server) = test_peer();
    let (_commands, mut events, run) = direct_session_events(peer, input(None, 10)).await;
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
                "jsonrpc":"2.0", "id":new["id"], "result":{"sessionId":"eof_update"}
            }),
        )
        .await;
        let _prompt = read_request(&mut reader).await;
        send_message(&mut writer, serde_json::json!({
                "jsonrpc":"2.0", "method":"session/update",
                "params":{"sessionId":"eof_update","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"before eof"}}}
            })).await;
    });

    // 批2 Task 7：会话启动时先投递一次活动快照，跳过后再校验 Running。
    assert!(matches!(
        events.recv().await,
        Some(ProviderEvent::TerminalActivity(_))
    ));
    assert!(matches!(
        events.recv().await,
        Some(ProviderEvent::StatusChanged(
            crate::cross_cutting::streaming_provider::ProviderStatus::Running
        ))
    ));
    assert!(matches!(
        events.recv().await,
        Some(ProviderEvent::TextDelta { content }) if content == "before eof"
    ));
    let error = run
        .await
        .expect("run join")
        .expect_err("prompt stream must fail");
    assert!(
        error.stderr.contains("response channel closed"),
        "unexpected error: {error:?}"
    );
    server_task.await.expect("server task");
}

#[tokio::test]
async fn terminal_response_restarts_prompt_for_buffered_free_text_choice() {
    let (peer, server) = test_peer();
    let (commands, mut events, run) = direct_session_events(peer, input(None, 10)).await;
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
                "jsonrpc":"2.0", "id":new["id"], "result":{"sessionId":"terminal_choice"}
            }),
        )
        .await;
        let first_prompt = read_request(&mut reader).await;
        send_message(&mut writer, serde_json::json!({
                "jsonrpc":"2.0", "id":"choice-1", "method":"session/request_permission",
                "params":{"options":[{"optionId":"selected","name":"Selected","kind":"allow_once"}],"toolCall":{"toolCallId":"choice-tool","title":"AskUserQuestion","content":{"type":"text","text":"Continue?"}}}
            })).await;
        send_message(
            &mut writer,
            serde_json::json!({
                "jsonrpc":"2.0", "id":first_prompt["id"], "result":{"stopReason":"end_turn"}
            }),
        )
        .await;
        let cancel = read_request(&mut reader).await;
        assert_eq!(cancel["id"], "choice-1");
        let second_prompt = tokio::time::timeout(
            std::time::Duration::from_millis(500),
            read_request(&mut reader),
        )
        .await
        .expect("free-text choice must start a replacement prompt");
        assert_eq!(second_prompt["method"], "session/prompt");
        assert_eq!(
            second_prompt["params"]["prompt"][0]["text"],
            "replacement prompt"
        );
        send_message(&mut writer, serde_json::json!({
                "jsonrpc":"2.0", "method":"session/update",
                "params":{"sessionId":"terminal_choice","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"replacement answered"}}}
            })).await;
        send_message(
            &mut writer,
            serde_json::json!({
                "jsonrpc":"2.0", "id":second_prompt["id"], "result":{"stopReason":"end_turn"}
            }),
        )
        .await;
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    });

    let choice = loop {
        match events.recv().await.expect("choice event") {
            ProviderEvent::ChoiceRequest(choice) => break choice,
            ProviderEvent::StatusChanged(_) => {}
            // 批2 Task 7：会话启动时一次投递的活动快照，非 choice 前异常。
            ProviderEvent::TerminalActivity(_) => {}
            other => panic!("unexpected event before choice: {other:?}"),
        }
    };
    commands
        .send(ProviderCommand::ChoiceResponse {
            id: choice.id,
            selected_option_ids: Vec::new(),
            free_text: Some("replacement prompt".to_string()),
            answers: Vec::new(),
            receipt: None,
        })
        .await
        .expect("free-text choice response");
    let events = tokio::time::timeout(std::time::Duration::from_secs(1), async {
        let mut received = Vec::new();
        loop {
            let event = events.recv().await.expect("event");
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
    .expect("replacement prompt completion");
    assert!(
        events
            .iter()
            .any(|event| matches!(event, ProviderEvent::Completed(_)))
    );
    assert!(run.await.expect("run join").is_ok());
    server_task.await.expect("server task");
}

#[tokio::test]
async fn resume_load_failure_never_falls_back_to_new_or_prompt() {
    let provider = KimiCodeProvider::new(fixture_command("kimi_acp_load_failure_fixture.sh"));
    let mut session = provider
        .start(input(Some("missing_session"), 10), CancellationToken::new())
        .await
        .expect("start");
    let events = terminal_events(&mut session).await;
    let failures = events
        .iter()
        .filter(|event| matches!(event, ProviderEvent::Failed { .. }))
        .collect::<Vec<_>>();
    assert_eq!(failures.len(), 1);
    assert!(
        matches!(failures[0], ProviderEvent::Failed { message } if message.contains("session/load failed"))
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, ProviderEvent::Completed(_)))
            .count(),
        0
    );
}

#[tokio::test]
async fn resume_uses_session_load_when_resume_id_present() {
    let provider = KimiCodeProvider::new(fixture_command("kimi_acp_resume_fixture.sh"));
    let mut session = provider
        .start(
            input(Some("existing_session"), 10),
            CancellationToken::new(),
        )
        .await
        .expect("start");
    let events = terminal_events(&mut session).await;
    let completion = events
        .iter()
        .find_map(|event| match event {
            ProviderEvent::Completed(completion) => Some(completion),
            _ => None,
        })
        .expect("completion");
    assert_eq!(
        completion.provider_session_id.as_deref(),
        Some("resumed_kimi_fixture")
    );
}

#[tokio::test]
async fn resume_load_without_session_id_reuses_requested_session_id() {
    let provider = KimiCodeProvider::new(fixture_command(
        "kimi_acp_resume_without_session_id_fixture.sh",
    ));
    let mut session = provider
        .start(
            input(Some("existing_session"), 10),
            CancellationToken::new(),
        )
        .await
        .expect("start");
    let events = terminal_events(&mut session).await;
    let completion = events
        .iter()
        .find_map(|event| match event {
            ProviderEvent::Completed(completion) => Some(completion),
            _ => None,
        })
        .expect("completion");
    assert_eq!(completion.full_output, "resumed");
    assert_eq!(
        completion.provider_session_id.as_deref(),
        Some("existing_session")
    );
}

#[tokio::test]
async fn nonstandard_process_crash_emits_failed_once() {
    let mut env_vars = BTreeMap::new();
    env_vars.insert("KIMI_FIXTURE_EXIT_CODE".to_string(), "42".to_string());
    let provider = KimiCodeProvider::new(fixture_command("kimi_acp_crash_fixture.sh"));
    let mut session = provider
        .start(input_with_env(None, 10, env_vars), CancellationToken::new())
        .await
        .expect("start");
    let events = terminal_events(&mut session).await;
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, ProviderEvent::Failed { .. }))
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, ProviderEvent::Completed(_)))
            .count(),
        0
    );
}

#[tokio::test]
async fn early_exit_codes_have_stable_distinct_messages() {
    for (code, phrase) in [
        ("0", "code 0 before terminal prompt result"),
        ("1", "code 1 (non-retryable failure)"),
        ("75", "code 75 (temporary failure)"),
    ] {
        let mut env_vars = BTreeMap::new();
        env_vars.insert("KIMI_FIXTURE_EXIT_CODE".to_string(), code.to_string());
        let provider = KimiCodeProvider::new(fixture_command("kimi_acp_exit_fixture.sh"));
        let mut session = provider
            .start(input_with_env(None, 10, env_vars), CancellationToken::new())
            .await
            .expect("start");
        let events = terminal_events(&mut session).await;
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, ProviderEvent::Failed { .. }))
                .count(),
            1
        );
        assert!(events.iter().any(|event| matches!(event, ProviderEvent::Failed { message } if message.contains(phrase))), "exit {code} missing {phrase}: {events:?}");
    }
}

#[tokio::test]
async fn authentication_failures_prompt_kimi_login_without_leaking_credentials() {
    for mode in ["acp", "stderr"] {
        let mut env_vars = BTreeMap::new();
        env_vars.insert("KIMI_FIXTURE_AUTH_MODE".to_string(), mode.to_string());
        env_vars.insert("KIMI_API_KEY".to_string(), "ignored-api-key".to_string());
        let provider = KimiCodeProvider::new(fixture_command("kimi_acp_auth_fixture.sh"));
        let mut session = provider
            .start(input_with_env(None, 10, env_vars), CancellationToken::new())
            .await
            .expect("start");
        let events = terminal_events(&mut session).await;
        let failures = events
            .iter()
            .filter_map(|event| match event {
                ProviderEvent::Failed { message } => Some(message),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(
            failures.len(),
            1,
            "{mode} must emit one failure: {events:?}"
        );
        assert!(failures[0].contains("kimi login"));
        assert!(!failures[0].contains("fixture-secret-token"));
        assert!(!failures[0].contains("/tmp/kimi/config.toml"));
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, ProviderEvent::Completed(_)))
        );
    }
}

#[tokio::test]
async fn abort_sends_session_cancel_before_process_termination() {
    let marker = tempfile::NamedTempFile::new().expect("marker");
    let marker_path = marker.path().to_path_buf();
    std::fs::remove_file(&marker_path).expect("remove marker placeholder");
    let mut env_vars = BTreeMap::new();
    env_vars.insert(
        "KIMI_CANCEL_MARKER".to_string(),
        marker_path.display().to_string(),
    );
    let provider = KimiCodeProvider::new(fixture_command("kimi_acp_hanging_fixture.sh"));
    let mut session = provider
        .start(input_with_env(None, 10, env_vars), CancellationToken::new())
        .await
        .expect("start");
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    session
        .commands
        .send(ProviderCommand::Abort)
        .await
        .expect("abort command");
    loop {
        let event = tokio::time::timeout(std::time::Duration::from_secs(3), session.events.recv())
            .await
            .expect("event")
            .expect("event value");
        if matches!(
            event,
            ProviderEvent::StatusChanged(
                crate::cross_cutting::streaming_provider::ProviderStatus::Aborted
            )
        ) {
            break;
        }
    }
    assert!(
        marker_path.exists(),
        "ACP session/cancel must be written before process termination fallback"
    );
}

#[tokio::test]
async fn command_sender_drop_aborts_session_without_failed() {
    let (peer, server) = test_peer();
    let (commands, mut events, run) = direct_session_events(peer, input(None, 10)).await;
    let (prompt_ready_tx, prompt_ready_rx) = oneshot::channel();
    let server_task = tokio::spawn(async move {
        let (reader, mut writer) = tokio::io::split(server);
        let mut reader = tokio::io::BufReader::new(reader);
        let initialize = read_request(&mut reader).await;
        send_message(&mut writer, serde_json::json!({"jsonrpc":"2.0","id":initialize["id"],"result":{"protocolVersion":1,"agentCapabilities":{"loadSession":true,"sessionCapabilities":{"resume":{}}}}})).await;
        let _initialized = read_request(&mut reader).await;
        let new = read_request(&mut reader).await;
        send_message(&mut writer, serde_json::json!({"jsonrpc":"2.0","id":new["id"],"result":{"sessionId":"closed_commands_fixture"}})).await;
        let _prompt = read_request(&mut reader).await;
        prompt_ready_tx.send(()).expect("prompt ready");
        let cancel = read_request(&mut reader).await;
        assert_eq!(cancel["method"], "session/cancel");
    });
    prompt_ready_rx.await.expect("session prompt ready");
    drop(commands);
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(2), run)
            .await
            .expect("session must abort")
            .expect("run join")
            .is_err()
    );
    server_task.await.expect("server task");

    let mut aborted_count = 0;
    let mut failed_count = 0;
    let mut completed_count = 0;
    while let Ok(Some(event)) =
        tokio::time::timeout(std::time::Duration::from_millis(200), events.recv()).await
    {
        aborted_count += usize::from(matches!(
            event,
            ProviderEvent::StatusChanged(
                crate::cross_cutting::streaming_provider::ProviderStatus::Aborted
            )
        ));
        failed_count += usize::from(matches!(event, ProviderEvent::Failed { .. }));
        completed_count += usize::from(matches!(event, ProviderEvent::Completed(_)));
    }
    assert_eq!(aborted_count, 1, "closed command channel must abort once");
    assert_eq!(
        failed_count, 0,
        "closed command channel must not emit Failed"
    );
    assert_eq!(
        completed_count, 0,
        "closed command channel must not emit Completed"
    );
}
#[tokio::test]
async fn abort_emits_aborted_without_failed() {
    let provider = KimiCodeProvider::new(fixture_command("kimi_acp_hanging_fixture.sh"));
    let cancel = CancellationToken::new();
    let mut session = provider
        .start(input(None, 10), cancel.clone())
        .await
        .expect("start");
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    session
        .commands
        .send(ProviderCommand::Abort)
        .await
        .expect("abort command");
    let mut saw_aborted = false;
    while let Some(event) =
        tokio::time::timeout(std::time::Duration::from_secs(3), session.events.recv())
            .await
            .expect("event")
    {
        if matches!(
            event,
            ProviderEvent::StatusChanged(
                crate::cross_cutting::streaming_provider::ProviderStatus::Aborted
            )
        ) {
            saw_aborted = true;
            break;
        }
        assert!(!matches!(event, ProviderEvent::Failed { .. }));
    }
    assert!(saw_aborted);
}

// ==== Task 9c:Kimi LC 原生 resume 的 session/load 同 id 确认(错/缺 id 绝不 fresh)====

use crate::cross_cutting::provider_availability_gate::ProviderAvailabilityGate;
use crate::cross_cutting::provider_registry::ProviderRegistry;
use crate::cross_cutting::session_launch::ValidatedStreamingProviderInput;
use crate::cross_cutting::tool_policy_audit::test_support::RecordingToolPolicyAuditSink;
use crate::cross_cutting::tool_policy_audit::{DurableToolPolicyEvent, ToolPolicyAuditSink};
use crate::product::app_paths::ProductAppPaths;
use crate::product::logical_codebase::policy::AggregatePolicyArtifactStore;
use crate::product::logical_codebase::policy::{PolicyTarget, SessionPolicyAction};
use crate::product::logical_codebase::provider_capability_store::ProviderActionCapability;
use crate::product::logical_codebase::provider_gateway::{
    GatewayRunAudit, LogicalCodebaseProviderGateway, PolicyTargetResolver, ProviderCapability,
    ProviderCapabilitySource, ProviderGatewayError, ProviderRefType, SessionLaunchRequest,
};
use crate::product::logical_codebase::store::LogicalCodebaseManifest;
use crate::product::models::ProviderName;
use crate::protocol::contracts::{AdapterOutput, TimeoutStatus};

/// Kimi 9c LC fixture:与 4c 的 LcKimiLaunchFixture 同构(trimmed)——经真实
/// `LogicalCodebaseProviderGateway::validate` 链产出 validated input,供
/// `start_validated` 的 LC 原生 resume 确认测试消费(pub(crate) 供
/// mcp_bundle_tests 复用)。
pub(crate) struct LcKimiResumeFixture {
    _root: tempfile::TempDir,
    paths: ProductAppPaths,
    audit: std::sync::Arc<GatewayRunAudit>,
}

impl LcKimiResumeFixture {
    pub(crate) fn new() -> Self {
        let root = tempfile::tempdir().expect("lc fixture root");
        let paths = ProductAppPaths::new(root.path());
        let manifest =
            LogicalCodebaseManifest::new("project_0001", root.path().to_path_buf(), vec![]);
        AggregatePolicyArtifactStore::new(paths.clone())
            .ensure_bootstrap(&manifest)
            .expect("install lc bootstrap policy");
        let fixture = Self {
            _root: root,
            paths,
            audit: std::sync::Arc::new(GatewayRunAudit::new()),
        };
        fixture.target_worktree();
        fixture
    }

    /// canonical LC root(= manifest provider_context_root)。
    pub(crate) fn canonical_root(&self) -> PathBuf {
        std::fs::canonicalize(self.paths.root()).expect("lc fixture root exists")
    }

    /// 唯一可写 target(成员 worktree;与 canonical root 分离)。
    pub(crate) fn target_worktree(&self) -> PathBuf {
        let worktree = self.paths.root().join("member-worktree");
        std::fs::create_dir_all(&worktree).expect("create member worktree");
        worktree
    }

    pub(crate) fn gateway(&self) -> LogicalCodebaseProviderGateway {
        let mut registry = ProviderRegistry::new();
        registry.register(
            ProviderName::KimiCode,
            std::sync::Arc::new(LcNoopStreamingAdapter),
        );
        LogicalCodebaseProviderGateway::with_audit(
            AggregatePolicyArtifactStore::new(self.paths.clone()),
            std::sync::Arc::new(LcStaticCapabilitySource),
            std::sync::Arc::new(LcTargetResolver),
            std::sync::Arc::new(registry),
            std::sync::Arc::new(LcNoopSyncAdapter),
            lc_available_gate(),
            self.audit.clone(),
            self.canonical_root(),
        )
    }

    /// LC coding 请求:cwd=canonical root,target=成员 worktree。
    pub(crate) fn coding_request(&self) -> SessionLaunchRequest {
        let manifest =
            LogicalCodebaseManifest::new("project_0001", self.paths.root().to_path_buf(), vec![]);
        let worktree = self.target_worktree();
        SessionLaunchRequest {
            project_id: manifest.project_id,
            provider: crate::product::logical_codebase::provider_gateway::ProviderRef::kimi_code(
                "cap_kimi_lc_fixture",
            ),
            action: SessionPolicyAction::CodingTargetWrite,
            target: PolicyTarget::checkout("logical_repo_0001", "checkout_0001", worktree.clone()),
            working_directory: self.canonical_root(),
            readable_roots: vec![self.canonical_root()],
            writable_roots: vec![worktree],
            config_artifact_ref: "sha256:managed-config-artifact".to_string(),
        }
    }

    /// LC streaming input(Kimi 通用 tool policy 恒 None)。
    pub(crate) fn lc_streaming_input(
        &self,
        audit_sink: Option<std::sync::Arc<dyn ToolPolicyAuditSink>>,
        resume_id: Option<String>,
    ) -> StreamingProviderInput {
        StreamingProviderInput {
            working_directory: Some(self.canonical_root()),
            baseline_tree: None,
            tool_policy: None,
            audit_sink,
            provider_type: ProviderType::KimiCode,
            role: AdapterRole::Executor,
            prompt: "Run the LC fixture provider".to_string(),
            working_dir: self.target_worktree(),
            workspace_session_id: Some("ws-lc-fixture-1".to_string()),
            resume_provider_session_id: resume_id,
            permission_mode: ProviderPermissionMode::Auto,
            structured_output_contract: None,
            env_vars: BTreeMap::new(),
            timeout_secs: 60,
        }
    }

    /// 经真实 gateway validate 产出 coding validated input。
    pub(crate) fn validated_coding_input(
        &self,
        raw: StreamingProviderInput,
    ) -> ValidatedStreamingProviderInput {
        let validated = self
            .gateway()
            .validate(self.coding_request())
            .expect("lc coding launch validates");
        ValidatedStreamingProviderInput::new(raw, validated)
    }
}

/// LC fixture 的 capability 源:仅返回 kimi 的已实测快照(fixture 事实)。
struct LcStaticCapabilitySource;

fn lc_kimi_capability() -> ProviderCapability {
    ProviderCapability {
        provider_type: ProviderRefType::KimiCode,
        version: "kimi 0.34.0-lc-fixture".to_string(),
        adapter_dialect: crate::product::logical_codebase::policy::ProviderDialect::KimiAcpV1,
        wire_dialect: crate::product::logical_codebase::policy::ProviderWireDialect::KimiAcp,
        capability_snapshot_ref: "cap_kimi_lc_fixture".to_string(),
        action_capability: ProviderActionCapability {
            action: SessionPolicyAction::CodingTargetWrite,
            launch:
                crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence::Confirmed,
            resume:
                crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence::Confirmed,
            write_boundary:
                crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence::Confirmed,
            projection_digest: String::new(),
            evidence_ref: String::new(),
        },
        trust: crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence::Confirmed,
    }
}

impl ProviderCapabilitySource for LcStaticCapabilitySource {
    fn require_supported(
        &self,
        _provider: &crate::product::logical_codebase::provider_gateway::ProviderRef,
        _action: SessionPolicyAction,
    ) -> Result<ProviderCapability, ProviderGatewayError> {
        Ok(lc_kimi_capability())
    }

    fn require_resume_supported(
        &self,
        _provider: &crate::product::logical_codebase::provider_gateway::ProviderRef,
        _action: SessionPolicyAction,
    ) -> Result<ProviderCapability, ProviderGatewayError> {
        Ok(lc_kimi_capability())
    }

    fn require_write_boundary(
        &self,
        _provider: &crate::product::logical_codebase::provider_gateway::ProviderRef,
        _action: SessionPolicyAction,
    ) -> Result<ProviderCapability, ProviderGatewayError> {
        Ok(lc_kimi_capability())
    }

    fn require_root_recipe_supported(
        &self,
        _provider: &crate::product::logical_codebase::provider_gateway::ProviderRef,
        _credential: &crate::product::logical_codebase::provider_admission_preflight::BootstrapPhaseCredential,
    ) -> Result<ProviderCapability, ProviderGatewayError> {
        Err(ProviderGatewayError::UnsupportedCapability(
            "lc fixture has no root recipe facts".to_string(),
        ))
    }
}

/// LC fixture 的 target resolver:直接返回请求冻结的 target。
struct LcTargetResolver;

impl PolicyTargetResolver for LcTargetResolver {
    fn resolve_and_revalidate(
        &self,
        request: &SessionLaunchRequest,
    ) -> Result<PolicyTarget, ProviderGatewayError> {
        Ok(request.target.clone())
    }
}

/// 占位 streaming adapter:validate 不触 registry,仅为 gateway 构造提供槽位。
struct LcNoopStreamingAdapter;

#[async_trait::async_trait]
impl crate::cross_cutting::streaming_provider::StreamingProviderAdapter for LcNoopStreamingAdapter {}

/// 占位 sync adapter:gateway 构造参数,LC validated 测试不调用。
struct LcNoopSyncAdapter;

impl crate::cross_cutting::provider_adapter::ProviderAdapter for LcNoopSyncAdapter {
    fn run(
        &self,
        _input: &crate::protocol::contracts::AdapterInput,
    ) -> Result<AdapterOutput, crate::cross_cutting::provider_adapter::ProviderAdapterError> {
        Ok(AdapterOutput {
            exit_code: Some(0),
            stdout: "ok".to_string(),
            stderr: String::new(),
            structured_output: None,
            files_modified: Vec::new(),
            duration_ms: 0,
            timeout_status: TimeoutStatus::NotTimedOut,
        })
    }
}

fn lc_available_gate() -> std::sync::Arc<ProviderAvailabilityGate> {
    use crate::cross_cutting::provider_availability_gate::ProviderHealthSource;
    use crate::cross_cutting::provider_health::{ProviderHealthEntry, ProviderHealthSnapshot};
    use chrono::Utc;

    struct AlwaysHealthy(std::sync::Arc<ProviderHealthSnapshot>);
    impl ProviderHealthSource for AlwaysHealthy {
        fn snapshot(&self) -> std::sync::Arc<ProviderHealthSnapshot> {
            self.0.clone()
        }
        fn degraded(&self) -> bool {
            false
        }
    }

    let checked_at = Utc::now();
    let snapshot = std::sync::Arc::new(ProviderHealthSnapshot {
        schema_version: 1,
        generation: 1,
        checked_at,
        providers: [ProviderName::KimiCode]
            .into_iter()
            .map(|provider| ProviderHealthEntry {
                provider,
                command: "stub".to_string(),
                available: true,
                version: Some("0.34.0".to_string()),
                reason_code: None,
                reason: None,
                checked_at,
            })
            .collect(),
    });
    std::sync::Arc::new(ProviderAvailabilityGate::new(std::sync::Arc::new(
        AlwaysHealthy(snapshot),
    )))
}

/// 写可执行 fixture 脚本(0o755)。
#[cfg(unix)]
pub(crate) fn write_executable(dir: &std::path::Path, name: &str, script: &str) -> PathBuf {
    use std::io::Write as _;
    use std::os::unix::fs::PermissionsExt as _;
    let path = dir.join(name);
    let mut file = std::fs::File::create(&path).expect("create fixture script");
    file.write_all(script.as_bytes())
        .expect("write fixture script");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod fixture script");
    path
}

/// Task 9c fixture(fake kimi ACP):`--version` 打印兼容版本;ACP 循环内
/// initialize 应答完整 resume 能力(loadSession+sessionCapabilities.resume),
/// session/load 按 `KIMI_LOAD_SESSION_ID` 应答(非空 → `result.sessionId`;
/// 空 → success 但无 sessionId),session/prompt 应答文本增量与 end_turn;
/// 收到 session/new 即写 `KIMI_NEW_MARKER`(漂移测试断言其永不出现)并
/// 失败退出;登记 pid 到 `KIMI_PID_MARKER`(kill/reap 断言),其余情况不
/// 自行退出——终止必须来自 adapter 的 kill 链。
#[cfg(unix)]
pub(crate) fn lc_kimi_resume_fixture(dir: &std::path::Path) -> PathBuf {
    write_executable(
        dir,
        "fake-kimi-lc-resume-acp",
        r#"#!/usr/bin/env bash
if [[ "${1:-}" == "--version" ]]; then
  echo "kimi 0.34.0"
  exit 0
fi
echo $$ > "${KIMI_PID_MARKER:-/dev/null}"
while IFS= read -r line; do
  id="$(printf '%s' "$line" | sed -n 's/.*"id"[[:space:]]*:[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
  if [[ "$line" == *'"initialize"'* ]]; then
    echo "{\"jsonrpc\":\"2.0\",\"id\":${id:-1},\"result\":{\"protocolVersion\":1,\"agentCapabilities\":{\"loadSession\":true,\"sessionCapabilities\":{\"resume\":{}}}}}"
  elif [[ "$line" == *'"session/new"'* ]]; then
    echo new > "${KIMI_NEW_MARKER:-/dev/null}"
    echo "{\"jsonrpc\":\"2.0\",\"id\":${id:-2},\"error\":{\"code\":-32000,\"message\":\"session/new must not follow an explicit resume\"}}"
    exit 1
  elif [[ "$line" == *'"session/load"'* ]]; then
    if [[ -n "${KIMI_LOAD_SESSION_ID:-}" ]]; then
      echo "{\"jsonrpc\":\"2.0\",\"id\":${id:-2},\"result\":{\"sessionId\":\"${KIMI_LOAD_SESSION_ID}\"}}"
    else
      echo "{\"jsonrpc\":\"2.0\",\"id\":${id:-2},\"result\":{}}"
    fi
  elif [[ "$line" == *'"session/prompt"'* ]]; then
    sid="${KIMI_LOAD_SESSION_ID:-none}"
    echo "{\"jsonrpc\":\"2.0\",\"method\":\"session/update\",\"params\":{\"sessionId\":\"$sid\",\"update\":{\"sessionUpdate\":\"agent_message_chunk\",\"content\":{\"type\":\"text\",\"text\":\"lc resume done\"}}}}"
    echo "{\"jsonrpc\":\"2.0\",\"id\":${id:-3},\"result\":{\"stopReason\":\"end_turn\"}}"
    exit 0
  fi
done
"#,
    )
}

/// Task 9c kill 链断言辅助:轮询读取 fixture 登记的子进程 pid,再轮询确认
/// 该 pid 已从进程表消失(kill+reap 都发生)。轮询用 tokio sleep(yield):
/// kill 链在 start_validated 的会话任务里执行,current_thread 运行时下
/// 阻塞式 sleep 会饿死该任务。
#[cfg(unix)]
pub(crate) async fn t09c_child_killed_and_reaped(pid_marker: &std::path::Path) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let pid = loop {
        if let Some(pid) = std::fs::read_to_string(pid_marker)
            .ok()
            .and_then(|content| content.trim().parse::<u32>().ok())
        {
            break pid;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "fixture must register its pid at {}",
            pid_marker.display()
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    };
    loop {
        let alive = std::process::Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false);
        if !alive {
            return true;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "kimi child (pid {pid}) must be killed and reaped after an unconfirmed native resume"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

/// Task 9c(Step 1 断言组 438-440 逐字;r62 协议实测修订):Kimi LC 显式
/// resume 的原生确认——`session/load` 应答按 kimi ACP 实际协议形态确认:
/// - 应答同 id → 续接,confirmed==requested,provider_start 落确认 id;
/// - 缺 id(load 应答 success 但无 sessionId)→ **kimi 2.0.2 实测的正常
///   确认形态**(成功不回显 sessionId,仅 configOptions;kimi-6 story
///   resume 现场 + 本地协议 probe 双证)——采纳请求 id 续接,confirmed==
///   requested;真正的恢复失败(Unknown sessionId)走 JSON-RPC error,由
///   ensure_response_success fail-closed 拦截(既有 Err+kill 链不变);
/// - 错 id(应答不同会话 id)→ Err:已启动 child 后的 runtime 失败:child
///   被 kill/reap、错误记录「未恢复」(不伪称零 spawn),不回填请求 id、
///   不清 id 转 fresh、零新 provider_start。
#[cfg(unix)]
#[tokio::test]
async fn lcg_t09_kimi_missing_or_wrong_native_id_never_fresh() {
    let fixture = LcKimiResumeFixture::new();
    let requested_native_id = "kimi-session-resume-t09c".to_string();

    // 1) session/load 应答同 id:真实确认后续接,provider_start 落确认 id,
    //    会话流照常完成。
    {
        let marker_dir = tempfile::tempdir().expect("t09c matching marker dir");
        let sink = RecordingToolPolicyAuditSink::new();
        let mut raw = fixture.lc_streaming_input(
            Some(sink.clone().bound()),
            Some(requested_native_id.clone()),
        );
        raw.env_vars.insert(
            "KIMI_LOAD_SESSION_ID".to_string(),
            requested_native_id.clone(),
        );
        let provider = KimiCodeProvider::new(lc_kimi_resume_fixture(marker_dir.path()));
        let mut session = provider
            .start_validated(
                fixture.validated_coding_input(raw),
                CancellationToken::new(),
            )
            .await
            .expect("a native-confirmed resume must continue the LC session");
        let confirmed_native_id = session.native_session_id.clone().unwrap_or_default();
        assert_eq!(confirmed_native_id, requested_native_id);
        let events = sink.events();
        assert_eq!(
            events.len(),
            1,
            "confirmed resume writes exactly one provider_start"
        );
        assert!(matches!(
            &events[0],
            DurableToolPolicyEvent::ProviderStart(record)
                if record.provider_session_id == requested_native_id
        ));
        let terminal = terminal_events(&mut session).await;
        let completion = terminal
            .iter()
            .find_map(|event| match event {
                ProviderEvent::Completed(completion) => Some(completion.clone()),
                _ => None,
            })
            .expect("confirmed kimi resume must complete");
        assert_eq!(completion.full_output, "lc resume done");
    }

    // 2) 缺 id:session/load success 但应答无 sessionId——kimi ACP 实测的
    //    正常确认形态(2.0.2 成功不回显,仅 configOptions;kimi-6 story
    //    resume 现场撞断言)→ 采纳请求 id 续接,provider_start 落请求 id,
    //    会话流照常完成。
    {
        let marker_dir = tempfile::tempdir().expect("t09c missing-id marker dir");
        let sink = RecordingToolPolicyAuditSink::new();
        let mut raw = fixture.lc_streaming_input(
            Some(sink.clone().bound()),
            Some(requested_native_id.clone()),
        );
        raw.env_vars
            .insert("KIMI_LOAD_SESSION_ID".to_string(), String::new());
        let provider = KimiCodeProvider::new(lc_kimi_resume_fixture(marker_dir.path()));
        let mut session = provider
            .start_validated(
                fixture.validated_coding_input(raw),
                CancellationToken::new(),
            )
            .await
            .expect(
                "kimi session/load success without sessionId echo must resume (kimi ACP dialect)",
            );
        assert_eq!(
            session.native_session_id.as_deref(),
            Some(requested_native_id.as_str()),
            "unechoed kimi load confirmation adopts the requested resume id"
        );
        let events = sink.events();
        assert_eq!(
            events.len(),
            1,
            "the unechoed kimi resume confirmation writes exactly one provider_start"
        );
        assert!(matches!(
            &events[0],
            DurableToolPolicyEvent::ProviderStart(record)
                if record.provider_session_id == requested_native_id
        ));
        let terminal = terminal_events(&mut session).await;
        let completion = terminal
            .iter()
            .find_map(|event| match event {
                ProviderEvent::Completed(completion) => Some(completion.clone()),
                _ => None,
            })
            .expect("kimi resume with unechoed load confirmation must complete");
        assert_eq!(completion.full_output, "lc resume done");
    }

    // 3) 错 id:session/load 应答不同的原生会话 id → 同为 runtime 失败:
    //    Err + kill/reap,绝不采纳陌生 id 续接、绝不清请求 id 转 fresh。
    {
        let marker_dir = tempfile::tempdir().expect("t09c wrong-id marker dir");
        let pid_marker = marker_dir.path().join("kimi-t09c-wrong-id.pid");
        let sink = RecordingToolPolicyAuditSink::new();
        let wrong_native_id = "kimi-session-native-wrong-t09c".to_string();
        let mut raw = fixture.lc_streaming_input(
            Some(sink.clone().bound()),
            Some(requested_native_id.clone()),
        );
        raw.env_vars
            .insert("KIMI_LOAD_SESSION_ID".to_string(), wrong_native_id.clone());
        raw.env_vars.insert(
            "KIMI_PID_MARKER".to_string(),
            pid_marker.display().to_string(),
        );
        let provider = KimiCodeProvider::new(lc_kimi_resume_fixture(marker_dir.path()));
        let wrong_native_id_result = provider
            .start_validated(
                fixture.validated_coding_input(raw),
                CancellationToken::new(),
            )
            .await;
        let Err(rejected) = wrong_native_id_result else {
            panic!("a wrong native id must fail the resume");
        };
        assert!(
            rejected.details.contains("kimi-session-native-wrong-t09c")
                && rejected.details.contains(&requested_native_id)
                && rejected.details.contains("session NOT resumed"),
            "rejection must name both ids and record the not-resumed outcome: {rejected:?}"
        );
        assert!(
            sink.events().is_empty(),
            "no fresh provider_start may be written for a mismatched native resume confirmation"
        );
        let started_child_was_killed_and_reaped = t09c_child_killed_and_reaped(&pid_marker).await;
        assert!(started_child_was_killed_and_reaped);
    }
}

/// r19 续修回归(fix 轮 2,oracle 裁决三家同批收口):CLI 子进程在消费
/// initialize 写入后立即 kill -9,同组 `sleep 300` 持有管道写端使 EOF 无限
/// 推迟——会话泵对 initialize 应答的等待(r19 前形态)只能等 60s RPC 超时;
/// exit-watch 竞速必须使会话秒级以 Failed 终结(ProviderSession 终结保证)。
#[cfg(unix)]
#[tokio::test]
async fn kimi_child_death_after_initialize_write_terminates_session_promptly() {
    let provider = KimiCodeProvider::new(fixture_command("kimi_acp_dead_child_fixture.sh"));
    let mut session = provider
        .start(input(None, 10), CancellationToken::new())
        .await
        .expect("kimi session starts");

    let failure = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if let ProviderEvent::Failed { message } = session
                .events
                .recv()
                .await
                .expect("event channel stays open until terminal event")
            {
                break message;
            }
        }
    })
    .await
    .expect("child exit must terminate the kimi session stream within seconds");
    assert!(
        !failure.is_empty(),
        "terminal failure must carry a cause message"
    );
}

// ==== Task 3.6(B0e):kimi 真实 facts 与 full 投影/audit 同源接线 ====

use crate::cross_cutting::kimi_code_provider::projection::{
    KIMI_LC_APPROVAL_POLICY, KIMI_NATIVE_MCP_SOURCE, KimiPolicyProjector,
};
use crate::product::logical_codebase::RootRecipeReceiptStore;
use crate::product::logical_codebase::policy::{
    ProviderDialect, ProviderWireDialect, SessionPolicyEnvelope,
};
use crate::product::logical_codebase::provider_gateway::{
    GatewaySessionDisposition, ProviderLaunchAuditContext, ProviderRef, ResumeSessionLaunchRequest,
    SessionResumeFingerprint, canonical_target_git_identity,
};
use crate::product::logical_codebase::provider_projection::{
    ProviderPolicyProjector as _, ProviderProjectionInput,
};
use crate::product::logical_codebase::provider_trust::{
    ProviderTrustFingerprintFacts, ProviderTrustSource, ProviderTrustVerification,
};

impl LcKimiResumeFixture {
    /// Task 3.6:注册真实 streaming adapter 的 gateway(prepare_lc_facts
    /// 消费面),并注入只读 trust/receipt 事实源(Kimi 需 workspace trust,
    /// root record digest 由 `AnchorT36KimiTrustSource` 提供)。
    pub(crate) fn gateway_with_streaming_adapter(
        &self,
        adapter: std::sync::Arc<
            dyn crate::cross_cutting::streaming_provider::StreamingProviderAdapter,
        >,
    ) -> LogicalCodebaseProviderGateway {
        let mut registry = ProviderRegistry::new();
        registry.register(ProviderName::KimiCode, adapter);
        LogicalCodebaseProviderGateway::with_audit(
            AggregatePolicyArtifactStore::new(self.paths.clone()),
            std::sync::Arc::new(AnchorT36KimiCapabilitySource),
            std::sync::Arc::new(LcTargetResolver),
            std::sync::Arc::new(registry),
            std::sync::Arc::new(LcNoopSyncAdapter),
            lc_available_gate(),
            self.audit.clone(),
            self.canonical_root(),
        )
        .with_readonly_lc_facts(
            RootRecipeReceiptStore::for_lc(self.paths.clone(), "logical_codebase_0001"),
            std::sync::Arc::new(AnchorT36KimiTrustSource),
            Some("logical_codebase_0001".to_string()),
        )
    }

    /// policy 工件 revision/digest(prepare 零写观测面)。
    pub(crate) fn policy_revision_and_digest(&self) -> (u64, String) {
        let artifact = AggregatePolicyArtifactStore::new(self.paths.clone())
            .get("project_0001")
            .expect("lc fixture policy present")
            .expect("lc fixture policy artifact");
        (artifact.revision, artifact.digest)
    }

    /// gateway 审计 supersede 计数(prepare 零写观测面)。
    pub(crate) fn gateway_supersede_count(&self) -> usize {
        self.audit.supersede_count()
    }
}

/// anchor t36 的 run-bound audit 上下文(prepare 冻结进候选)。
fn anchor_t36_audit_context(
    sink: std::sync::Arc<RecordingToolPolicyAuditSink>,
) -> ProviderLaunchAuditContext {
    ProviderLaunchAuditContext {
        workspace_session_id: "ws-anchor-t36".to_string(),
        role_run_seq: 1,
        audit_sink: sink,
    }
}

/// anchor t36 capability 源:version 与 fixture 脚本一致(`kimi 0.34.0`),
/// action 行 `projection_digest` 用真实 `KimiPolicyProjector` 对同一
/// action+native MCP 来源+version 计算(与 gateway prepare 冻结的 full
/// projection `capability_projection_digest` 同源,admission 第 3 步消费)。
fn anchor_t36_kimi_capability() -> ProviderCapability {
    let action = SessionPolicyAction::CodingTargetWrite;
    let envelope = SessionPolicyEnvelope {
        policy_id: "policy-lc-0001".to_string(),
        policy_revision: 1,
        policy_digest: "sha256:policy-lc-0001".to_string(),
        action,
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
        created_at: "2026-10-11T00:00:00Z".to_string(),
        authority_root: PathBuf::from("/lc/lc-root"),
    };
    let projection_input = ProviderProjectionInput::new(
        envelope,
        ProviderRef::kimi_code("cap_kimi_anchor_t36"),
        action,
        AdapterRole::Executor,
        ProviderPermissionMode::Auto,
        None,
        KIMI_LC_APPROVAL_POLICY.to_string(),
        KIMI_NATIVE_MCP_SOURCE.to_string(),
        "sha256:cfg-a".to_string(),
        String::new(),
        None,
    );
    let projection = KimiPolicyProjector::new("kimi 0.34.0")
        .project(&projection_input)
        .expect("anchor t36 capability-row projection");
    ProviderCapability {
        provider_type: ProviderRefType::KimiCode,
        version: "kimi 0.34.0".to_string(),
        adapter_dialect: ProviderDialect::KimiAcpV1,
        wire_dialect: ProviderWireDialect::KimiAcp,
        capability_snapshot_ref: "cap_kimi_anchor_t36".to_string(),
        action_capability: ProviderActionCapability {
            action,
            launch:
                crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence::Confirmed,
            resume:
                crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence::Confirmed,
            write_boundary:
                crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence::Confirmed,
            projection_digest: projection.capability_projection_digest().to_string(),
            evidence_ref: String::new(),
        },
        trust: crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence::Confirmed,
    }
}

/// anchor t36 capability 源:全部走 `anchor_t36_kimi_capability()`。
struct AnchorT36KimiCapabilitySource;

impl ProviderCapabilitySource for AnchorT36KimiCapabilitySource {
    fn require_supported(
        &self,
        _provider: &crate::product::logical_codebase::provider_gateway::ProviderRef,
        _action: SessionPolicyAction,
    ) -> Result<ProviderCapability, ProviderGatewayError> {
        Ok(anchor_t36_kimi_capability())
    }

    fn require_resume_supported(
        &self,
        _provider: &crate::product::logical_codebase::provider_gateway::ProviderRef,
        _action: SessionPolicyAction,
    ) -> Result<ProviderCapability, ProviderGatewayError> {
        Ok(anchor_t36_kimi_capability())
    }

    fn require_write_boundary(
        &self,
        _provider: &crate::product::logical_codebase::provider_gateway::ProviderRef,
        _action: SessionPolicyAction,
    ) -> Result<ProviderCapability, ProviderGatewayError> {
        Ok(anchor_t36_kimi_capability())
    }

    fn require_root_recipe_supported(
        &self,
        _provider: &crate::product::logical_codebase::provider_gateway::ProviderRef,
        _credential: &crate::product::logical_codebase::provider_admission_preflight::BootstrapPhaseCredential,
    ) -> Result<ProviderCapability, ProviderGatewayError> {
        Err(ProviderGatewayError::UnsupportedCapability(
            "anchor t36 fixture has no root recipe facts".to_string(),
        ))
    }
}

/// anchor t36 只读 trust 源:Kimi 需 workspace trust——root record digest
/// 冻结为固定 fixture 摘要(`fingerprint_facts` 零写;admission 第 4 步
/// `verify_trusted` 同源判定)。
struct AnchorT36KimiTrustSource;

impl ProviderTrustSource for AnchorT36KimiTrustSource {
    fn verify_trusted(
        &self,
        _project_id: &str,
        _lc_id: &str,
        provider: &ProviderName,
        canonical_root: &std::path::Path,
    ) -> Result<ProviderTrustVerification, ProviderGatewayError> {
        Ok(ProviderTrustVerification {
            provider: provider.clone(),
            canonical_root: canonical_root.to_path_buf(),
            trust_key: String::new(),
            trusted: true,
            ownership: None,
            detail: String::new(),
            verified_at: String::new(),
        })
    }

    fn fingerprint_facts(
        &self,
        _project_id: &str,
        _lc_id: &str,
        provider: &ProviderName,
        canonical_root: &std::path::Path,
    ) -> Result<ProviderTrustFingerprintFacts, ProviderGatewayError> {
        Ok(ProviderTrustFingerprintFacts {
            verification: ProviderTrustVerification {
                provider: provider.clone(),
                canonical_root: canonical_root.to_path_buf(),
                trust_key: String::new(),
                trusted: true,
                ownership: None,
                detail: String::new(),
                verified_at: String::new(),
            },
            record_digest: Some("sha256:kimi-root-record-anchor-t36".to_string()),
        })
    }
}

/// anchor t36 fake kimi ACP 脚本:`--version` 打印指定版本(每次探测追加
/// `${0}.probe` 计数);ACP 循环 initialize 应答完整 resume 能力;
/// session/load 按 `KIMI_LOAD_SESSION_ID` 应答(与 9c fixture 同语义);
/// session/new 应答冻结 fresh id 并追加 `${0}.session-new` 计数(显式
/// fresh 观测面);pid 登记到 `KIMI_PID_MARKER`;终止只来自 adapter kill 链。
/// 版本以 `__T36_VERSION__` 占位注入,`__T36_PAD__` 使不同版本脚本长度
/// 确定不同(真实可执行身份漂移:len+mtime 都变)。
#[cfg(unix)]
fn anchor_t36_kimi_acp_script(version: &str) -> String {
    let pad = version
        .rsplit('.')
        .next()
        .and_then(|patch| patch.parse::<usize>().ok())
        .map(|patch| "x".repeat(patch + 1))
        .unwrap_or_else(|| "x".to_string());
    r#"#!/usr/bin/env bash
# anchor t36 fixture version: __T36_VERSION__
# drift pad: __T36_PAD__
if [[ "${1:-}" == "--version" ]]; then
  echo x >> "${0}.probe"
  echo "kimi __T36_VERSION__"
  exit 0
fi
echo $$ > "${KIMI_PID_MARKER:-/dev/null}"
while IFS= read -r line; do
  id="$(printf '%s' "$line" | sed -n 's/.*"id"[[:space:]]*:[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
  if [[ "$line" == *'"initialize"'* ]]; then
    echo "{\"jsonrpc\":\"2.0\",\"id\":${id:-1},\"result\":{\"protocolVersion\":1,\"agentCapabilities\":{\"loadSession\":true,\"sessionCapabilities\":{\"resume\":{}}}}}"
  elif [[ "$line" == *'"session/new"'* ]]; then
    echo x >> "${0}.session-new"
    echo "{\"jsonrpc\":\"2.0\",\"id\":${id:-2},\"result\":{\"sessionId\":\"kimi-session-fresh-anchor-t36\"}}"
  elif [[ "$line" == *'"session/load"'* ]]; then
    if [[ -n "${KIMI_LOAD_SESSION_ID:-}" ]]; then
      echo "{\"jsonrpc\":\"2.0\",\"id\":${id:-2},\"result\":{\"sessionId\":\"${KIMI_LOAD_SESSION_ID}\"}}"
    else
      echo "{\"jsonrpc\":\"2.0\",\"id\":${id:-2},\"result\":{}}"
    fi
  elif [[ "$line" == *'"session/prompt"'* ]]; then
    sid="${KIMI_LOAD_SESSION_ID:-kimi-session-fresh-anchor-t36}"
    echo "{\"jsonrpc\":\"2.0\",\"method\":\"session/update\",\"params\":{\"sessionId\":\"$sid\",\"update\":{\"sessionUpdate\":\"agent_message_chunk\",\"content\":{\"type\":\"text\",\"text\":\"lc anchor t36 done\"}}}}"
    echo "{\"jsonrpc\":\"2.0\",\"id\":${id:-3},\"result\":{\"stopReason\":\"end_turn\"}}"
    exit 0
  fi
done
"#
    .replace("__T36_VERSION__", version)
    .replace("__T36_PAD__", &pad)
}

/// 写 anchor t36 fixture(0o755)到 dir/fake-kimi-anchor-t36-acp。
#[cfg(unix)]
pub(crate) fn anchor_t36_kimi_acp_fixture(dir: &std::path::Path, version: &str) -> PathBuf {
    write_executable(
        dir,
        "fake-kimi-anchor-t36-acp",
        &anchor_t36_kimi_acp_script(version),
    )
}

/// 同路径重写 anchor t36 fixture(真实可执行身份漂移:len+mtime 变化)。
#[cfg(unix)]
pub(crate) fn anchor_t36_rewrite_kimi_acp_fixture(bin: &std::path::Path, version: &str) {
    use std::io::Write as _;
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::write(bin, anchor_t36_kimi_acp_script(version))
        .expect("rewrite anchor t36 fixture script");
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(bin)
        .expect("open anchor t36 fixture for mtime bump");
    // 确认 mtime 漂移不依赖同瞬写入:显式 flush 后再睡 2ms。
    file.flush().expect("flush anchor t36 fixture");
    std::thread::sleep(std::time::Duration::from_millis(2));
    std::fs::set_permissions(bin, std::fs::Permissions::from_mode(0o755))
        .expect("chmod anchor t36 fixture");
}

/// fixture `--version` 探测计数(`${0}.probe` 行数)。
#[cfg(unix)]
pub(crate) fn anchor_t36_probe_invocations(fixture_bin: &std::path::Path) -> usize {
    std::fs::read_to_string(format!("{}.probe", fixture_bin.display()))
        .map(|content| {
            content
                .lines()
                .filter(|line| !line.trim().is_empty())
                .count()
        })
        .unwrap_or(0)
}

/// fixture session/new 计数(`${0}.session-new` 行数;显式 fresh 观测面)。
#[cfg(unix)]
pub(crate) fn anchor_t36_fresh_session_count(fixture_bin: &std::path::Path) -> usize {
    std::fs::read_to_string(format!("{}.session-new", fixture_bin.display()))
        .map(|content| {
            content
                .lines()
                .filter(|line| !line.trim().is_empty())
                .count()
        })
        .unwrap_or(0)
}

/// Task 3.6(B0e Step 1):Kimi prepare facts/full projection/audit 同源
/// 指纹——真实 `--version` TTL 观测恰一次;无 Aria 注入时 MCP 来源为
/// native 标记;root trust record digest 冻结进 full projection;prepare
/// 零 durable 写/零会话 child;full-matching resume 后 spawn 消费 prepared
/// projection,`provider_start.lc_projection` 字面量与候选 full projection
/// 逐字段同源;Kimi 通用策略例外保持(gateway 注入的非空策略以稳定码
/// typed 拒,且拒于 version probe 之前);session/load 确认同 id(零
/// session/new)。
#[cfg(unix)]
#[tokio::test]
async fn anchor_t36_provider_prepare_and_audit_share_full_fingerprint() {
    let fixture = LcKimiResumeFixture::new();
    let marker_dir = tempfile::tempdir().expect("anchor t36 marker dir");
    let sink = RecordingToolPolicyAuditSink::new();
    let requested_native_id = "kimi-session-resume-anchor-t36".to_string();
    let pid_marker = marker_dir.path().join("kimi-t36.pid");

    let fixture_bin = anchor_t36_kimi_acp_fixture(marker_dir.path(), "0.34.0");
    let provider = Arc::new(KimiCodeProvider::new(fixture_bin.clone()));
    let gateway = fixture.gateway_with_streaming_adapter(provider.clone());

    // 1) 批2 Task 11 矩阵:gateway 不再按 AdapterRole 注入 deny(Orchestrator
    //    承载的规划候选 wire 面全开)——prepare 成功且候选 input 的
    //    tool_policy=None(kimi 通用策略例外口径由「零派发」承接;adapter
    //    对到达的非空通用策略仍稳定码 typed 拒,该面由 kimi adapter 自测
    //    与 lcg_t01 映射测试钉住)。成功 prepare 走真实只读 facts 观测
    //(version probe 属合法观测,不再有「拒于探测之前」的旧形态)。
    let mut orchestrator_raw = fixture.lc_streaming_input(None, None);
    orchestrator_raw.role = AdapterRole::Orchestrator;
    let orchestrator_candidate = gateway
        .prepare_lc_streaming_candidate(
            orchestrator_raw,
            fixture.coding_request(),
            anchor_t36_audit_context(sink.clone()),
            CancellationToken::new(),
        )
        .await
        .expect("T11: orchestrator planning candidate prepares with no derived deny");
    assert!(
        orchestrator_candidate.input().tool_policy.is_none(),
        "T11: no deny must be derived from the Orchestrator carrier for kimi"
    );

    // 2) 真实 prepare:facts/full projection/root trust 同源冻结(native 来源)。
    let durable_before = (
        fixture.policy_revision_and_digest(),
        fixture.gateway_supersede_count(),
        sink.events().len(),
    );
    let mut raw = fixture.lc_streaming_input(
        Some(sink.clone().bound()),
        Some(requested_native_id.clone()),
    );
    raw.env_vars.insert(
        "KIMI_LOAD_SESSION_ID".to_string(),
        requested_native_id.clone(),
    );
    raw.env_vars.insert(
        "KIMI_PID_MARKER".to_string(),
        pid_marker.display().to_string(),
    );

    let candidate = gateway
        .prepare_lc_streaming_candidate(
            raw,
            fixture.coding_request(),
            anchor_t36_audit_context(sink.clone()),
            CancellationToken::new(),
        )
        .await
        .expect("kimi prepare freezes real adapter facts into a full candidate");
    assert_eq!(candidate.facts().exact_version(), "kimi 0.34.0");
    assert_eq!(anchor_t36_probe_invocations(&fixture_bin), 1);
    assert!(candidate.facts().version_observed_at() <= std::time::Instant::now());
    assert_eq!(
        candidate.facts().version_probe_key().provider,
        ProviderName::KimiCode
    );
    assert_eq!(
        candidate.facts().version_probe_key().argv,
        vec!["--version".to_string()]
    );
    let native_mcp_source = candidate.full_projection().mcp_bundle_digest().to_string();
    assert_eq!(native_mcp_source, "native-project-config");
    // root trust:readonly root record digest 冻结进 full projection(同源)。
    assert_eq!(
        candidate.full_projection().trust_digest(),
        "lc-trust-v1:KimiCode:sha256:kimi-root-record-anchor-t36"
    );
    // facts 与 full projection 同版本同源(同 projector);approval/MCP 面一致。
    assert_eq!(
        candidate.full_projection().exact_version(),
        candidate.facts().exact_version()
    );
    assert!(!candidate.full_projection().projection_digest().is_empty());
    // prepare 零 durable 写(policy revision/digest/supersede/audit)且零
    // 会话 child(pid 标记未出现;probe 是独立 `--version` 观测,不计 child)。
    assert!(!pid_marker.exists());
    assert_eq!(fixture.policy_revision_and_digest(), durable_before.0);
    assert_eq!(fixture.gateway_supersede_count(), durable_before.1);
    assert_eq!(sink.events().len(), durable_before.2);

    // 3) full-matching resume → Resume(冻结候选投影)→ spawn 消费 prepared
    //    projection:audit 与 full projection 同 fingerprint(含 root trust)。
    let previous = SessionResumeFingerprint::from_envelope(
        candidate.validated().envelope(),
        candidate.full_projection(),
        candidate.validated().action_evidence_digest(),
        &canonical_target_git_identity(&candidate.validated().envelope().target.worktree),
    );
    let disposition = gateway
        .resume_or_start_with_projection(
            ResumeSessionLaunchRequest {
                launch: fixture.coding_request(),
                previous_fingerprint: previous,
                previous_session_id: "sess-anchor-t36".to_string(),
            },
            candidate.full_projection().clone(),
        )
        .expect("full-matching kimi fingerprint resumes");
    let GatewaySessionDisposition::Resume(resume_validated) = disposition else {
        panic!("a full-matching fingerprint must resume, not wait for explicit fresh");
    };
    let spawn_input =
        ValidatedStreamingProviderInput::new(candidate.input().clone(), resume_validated);
    let mut session = provider
        .start_validated(spawn_input, CancellationToken::new())
        .await
        .expect("spawn consumes the prepared full projection");
    assert_eq!(
        session.native_session_id.as_deref(),
        Some(requested_native_id.as_str())
    );
    // TTL:spawn 前复验同 key 命中——总 probe 数仍为 1。
    assert_eq!(anchor_t36_probe_invocations(&fixture_bin), 1);
    // ACP 同 ID:显式 resume 走 session/load(零 session/new)。
    assert_eq!(anchor_t36_fresh_session_count(&fixture_bin), 0);

    let events = sink.events();
    assert_eq!(
        events.len(),
        1,
        "the resumed spawn writes exactly one provider_start"
    );
    let DurableToolPolicyEvent::ProviderStart(record) = &events[0] else {
        panic!("expected provider_start");
    };
    assert_eq!(record.provider, "kimi-code");
    assert_eq!(record.provider_version, candidate.facts().exact_version());
    assert_eq!(record.adapter_dialect, "kimi-acp");
    assert_eq!(record.provider_session_id, requested_native_id);
    assert!(!record.tool_policy_canonical_digest.is_empty());
    let lc_projection = record
        .lc_projection
        .as_ref()
        .expect("lc projection audit is recorded from the prepared projection");
    assert_eq!(lc_projection.action, "coding_target_write");
    assert_eq!(lc_projection.wire_dialect, "kimi-acp");
    assert_eq!(
        lc_projection.projection_digest,
        candidate.full_projection().projection_digest()
    );
    assert_eq!(
        lc_projection.capability_projection_digest,
        candidate.full_projection().capability_projection_digest()
    );
    assert_eq!(
        lc_projection.boundary_evidence_ref,
        candidate.full_projection().boundary_evidence_ref()
    );

    let terminal = terminal_events(&mut session).await;
    let completion = terminal
        .iter()
        .find_map(|event| match event {
            ProviderEvent::Completed(completion) => Some(completion.clone()),
            _ => None,
        })
        .expect("the resumed kimi session completes");
    assert_eq!(completion.full_output, "lc anchor t36 done");
}

/// Task 3.6(B0e Step 1 断言组逐字):真实漂移零 session child——版本在
/// prepare 冻结后真实替换(可执行身份变化→换 key 再观测):
/// - gateway 复验 typed `PolicyDrift(provider_version)` 拒 resume;
/// - adapter spawn 守卫在 child 之前 typed 拒(mismatch_children=0,零
///   provider_start);
/// - 错误握手(wrong native id):child 已启动 → kill/reap,显式 resume
///   后零 fresh(fresh_after_explicit_resume_count=0)。
#[cfg(unix)]
#[tokio::test]
async fn anchor_t36_real_drift_zero_session_child() {
    let fixture = LcKimiResumeFixture::new();
    let drift_dir = tempfile::tempdir().expect("anchor t36 drift dir");
    let wrong_dir = tempfile::tempdir().expect("anchor t36 wrong-id dir");
    let drift_pid_marker = drift_dir.path().join("kimi-t36-drift.pid");
    let wrong_pid_marker = wrong_dir.path().join("kimi-t36-wrong-id.pid");

    // —— 场景 1:真实版本漂移 ——
    let drift_bin = anchor_t36_kimi_acp_fixture(drift_dir.path(), "0.34.0");
    let drift_provider = Arc::new(KimiCodeProvider::new(drift_bin.clone()));
    let drift_gateway = fixture.gateway_with_streaming_adapter(drift_provider.clone());
    let drift_sink = RecordingToolPolicyAuditSink::new();
    let mut drift_raw = fixture.lc_streaming_input(
        Some(drift_sink.clone().bound()),
        Some("kimi-session-drift-anchor-t36".to_string()),
    );
    drift_raw.env_vars.insert(
        "KIMI_PID_MARKER".to_string(),
        drift_pid_marker.display().to_string(),
    );

    let candidate = drift_gateway
        .prepare_lc_streaming_candidate(
            drift_raw,
            fixture.coding_request(),
            anchor_t36_audit_context(drift_sink.clone()),
            CancellationToken::new(),
        )
        .await
        .expect("prepare freezes v1 facts before the drift");
    assert_eq!(candidate.facts().exact_version(), "kimi 0.34.0");

    // full-matching resume 判定先行(冻结 spawn 输入仍持 v1 投影)。
    let previous = SessionResumeFingerprint::from_envelope(
        candidate.validated().envelope(),
        candidate.full_projection(),
        candidate.validated().action_evidence_digest(),
        &canonical_target_git_identity(&candidate.validated().envelope().target.worktree),
    );
    let disposition = drift_gateway
        .resume_or_start_with_projection(
            ResumeSessionLaunchRequest {
                launch: fixture.coding_request(),
                previous_fingerprint: previous,
                previous_session_id: "sess-anchor-t36-drift".to_string(),
            },
            candidate.full_projection().clone(),
        )
        .expect("full-matching disposition is reached before the drift");
    let GatewaySessionDisposition::Resume(resume_validated) = disposition else {
        panic!("the pre-drift disposition must be a full-matching resume");
    };
    let drifted_spawn_input =
        ValidatedStreamingProviderInput::new(candidate.input().clone(), resume_validated);

    // 真实漂移:同路径替换 fixture 二进制(可执行身份变化→新 key 再观测)。
    anchor_t36_rewrite_kimi_acp_fixture(&drift_bin, "0.34.1");

    // gateway 复验:typed PolicyDrift。
    let current = drift_gateway
        .reobserve_lc_prepare_facts(&candidate, CancellationToken::new())
        .await
        .expect("reobservation probes the replaced executable under a new key");
    assert_eq!(current.exact_version(), "kimi 0.34.1");
    let typed_rejection = candidate
        .assert_frozen_version(&current)
        .expect_err("a real version drift must be typed-rejected against the frozen facts");
    assert!(
        matches!(
            &typed_rejection,
            ProviderGatewayError::PolicyDrift { dimension } if dimension == "provider_version"
        ),
        "unexpected rejection shape: {typed_rejection:?}"
    );

    // adapter spawn 守卫:child 之前 typed 拒 → 零 ACP child、零 audit。
    let rejected = drift_provider
        .start_validated(drifted_spawn_input, CancellationToken::new())
        .await;
    let Err(rejection) = rejected else {
        panic!("a drifted provider version must reject the spawn before any child");
    };
    assert!(
        rejection.details.contains("provider version drifted")
            && rejection.details.contains("kimi 0.34.1")
            && rejection.details.contains("kimi 0.34.0"),
        "unexpected rejection: {rejection:?}"
    );
    let mismatch_children = usize::from(drift_pid_marker.exists());
    assert_eq!(mismatch_children, 0);
    assert_eq!(
        drift_sink.events().len(),
        0,
        "no provider_start may be written for a drifted spawn"
    );
    assert_eq!(anchor_t36_fresh_session_count(&drift_bin), 0);

    // —— 场景 2:错误握手 id(wrong native id)——
    let wrong_bin = anchor_t36_kimi_acp_fixture(wrong_dir.path(), "0.34.0");
    let wrong_provider = Arc::new(KimiCodeProvider::new(wrong_bin.clone()));
    let wrong_gateway = fixture.gateway_with_streaming_adapter(wrong_provider.clone());
    let wrong_sink = RecordingToolPolicyAuditSink::new();
    let requested_native_id = "kimi-session-resume-t36-wrong".to_string();
    let wrong_native_id = "kimi-session-native-wrong-t36".to_string();
    let mut wrong_raw = fixture.lc_streaming_input(
        Some(wrong_sink.clone().bound()),
        Some(requested_native_id.clone()),
    );
    wrong_raw
        .env_vars
        .insert("KIMI_LOAD_SESSION_ID".to_string(), wrong_native_id.clone());
    wrong_raw.env_vars.insert(
        "KIMI_PID_MARKER".to_string(),
        wrong_pid_marker.display().to_string(),
    );

    let wrong_candidate = wrong_gateway
        .prepare_lc_streaming_candidate(
            wrong_raw,
            fixture.coding_request(),
            anchor_t36_audit_context(wrong_sink.clone()),
            CancellationToken::new(),
        )
        .await
        .expect("prepare for the wrong-id handshake scenario");
    let wrong_previous = SessionResumeFingerprint::from_envelope(
        wrong_candidate.validated().envelope(),
        wrong_candidate.full_projection(),
        wrong_candidate.validated().action_evidence_digest(),
        &canonical_target_git_identity(&wrong_candidate.validated().envelope().target.worktree),
    );
    let wrong_disposition = wrong_gateway
        .resume_or_start_with_projection(
            ResumeSessionLaunchRequest {
                launch: fixture.coding_request(),
                previous_fingerprint: wrong_previous,
                previous_session_id: "sess-anchor-t36-wrong".to_string(),
            },
            wrong_candidate.full_projection().clone(),
        )
        .expect("full-matching disposition for the wrong-id scenario");
    let GatewaySessionDisposition::Resume(wrong_resume_validated) = wrong_disposition else {
        panic!("the wrong-id scenario must start from a full-matching resume");
    };
    let wrong_spawn_input = ValidatedStreamingProviderInput::new(
        wrong_candidate.input().clone(),
        wrong_resume_validated,
    );
    let wrong_result = wrong_provider
        .start_validated(wrong_spawn_input, CancellationToken::new())
        .await;
    let Err(wrong_rejection) = wrong_result else {
        panic!("a wrong native id must fail the resume");
    };
    assert!(
        wrong_rejection.details.contains(&wrong_native_id)
            && wrong_rejection.details.contains(&requested_native_id)
            && wrong_rejection.details.contains("session NOT resumed"),
        "unexpected rejection: {wrong_rejection:?}"
    );
    assert!(
        wrong_sink.events().is_empty(),
        "no fresh provider_start may be written for a mismatched native resume confirmation"
    );
    let fresh_after_explicit_resume_count = anchor_t36_fresh_session_count(&wrong_bin);
    assert_eq!(fresh_after_explicit_resume_count, 0);
    let wrong_native_child_killed_and_reaped =
        t09c_child_killed_and_reaped(&wrong_pid_marker).await;
    assert!(wrong_native_child_killed_and_reaped);
}
