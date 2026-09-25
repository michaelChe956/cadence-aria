//! Real-machine (真机) verification against the locally installed kimi CLI.
//! Gated behind `KIMI_ACP_E2E=1` so CI never depends on a login session.

use std::collections::BTreeMap;
use std::time::Duration;

use tokio::io::BufReader;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::json_rpc_peer::JsonRpcPeer;
use crate::cross_cutting::kimi_code_provider::session::run_kimi_session;
use crate::cross_cutting::streaming_provider::{
    ProviderCommand, ProviderEvent, ProviderPermissionMode, StreamingProviderInput,
};
use crate::protocol::contracts::{AdapterRole, ProviderType};

#[tokio::test]
#[ignore = "set KIMI_ACP_E2E=1 to run against the real kimi CLI"]
async fn live_kimi_bash_echo_hi_round_trip() {
    if std::env::var("KIMI_ACP_E2E").ok().as_deref() != Some("1") {
        return;
    }
    let dir = tempfile::tempdir().expect("dir");
    let mut child = tokio::process::Command::new("kimi")
        .arg("acp")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn kimi acp");
    let stdout = child.stdout.take().expect("kimi stdout");
    let stdin = child.stdin.take().expect("kimi stdin");
    let peer = JsonRpcPeer::new(BufReader::new(stdout), stdin);

    let (commands, command_rx) = mpsc::channel(8);
    let (event_tx, mut events) = mpsc::channel::<ProviderEvent>(256);
    let input = StreamingProviderInput {
        baseline_tree: None,
        tool_policy: None,
        audit_sink: None,
        provider_type: ProviderType::KimiCode,
        role: AdapterRole::Executor,
        prompt: "Use the bash tool to run exactly: echo hi . Then report the output.".to_string(),
        working_dir: dir.path().to_path_buf(),
        workspace_session_id: None,
        resume_provider_session_id: None,
        permission_mode: ProviderPermissionMode::Auto,
        structured_output_contract: None,
        env_vars: BTreeMap::new(),
        timeout_secs: 240,
    };
    let run = tokio::spawn(run_kimi_session(
        peer,
        command_rx,
        event_tx,
        input,
        CancellationToken::new(),
    ));

    let mut text = String::new();
    let mut completed = false;
    let deadline = Duration::from_secs(220);
    while let Ok(Some(event)) = tokio::time::timeout(deadline, events.recv()).await {
        match event {
            ProviderEvent::TextDelta { content } => text.push_str(&content),
            ProviderEvent::Execution(execution) => {
                if let Some(output) = &execution.output {
                    text.push_str(output);
                }
                assert!(
                    !format!("{execution:?}").contains("unavailable"),
                    "terminal capability reported unavailable: {execution:?}"
                );
            }
            ProviderEvent::Completed(_) => {
                completed = true;
                break;
            }
            ProviderEvent::StatusChanged(
                crate::cross_cutting::streaming_provider::ProviderStatus::Aborted,
            ) => break,
            other => {
                if let ProviderEvent::Failed { message } = other {
                    panic!("kimi session failed: {message}");
                }
            }
        }
    }
    let _ = commands.send(ProviderCommand::Abort).await;
    let _ = run.await;
    let _ = child.kill().await;

    assert!(completed, "session did not complete; captured: {text}");
    assert!(
        text.contains("hi"),
        "expected `hi` in the kimi reply; captured: {text}"
    );
    assert!(
        !text
            .to_lowercase()
            .contains("acp terminal capability is unavailable"),
        "terminal capability failure leaked into the reply: {text}"
    );
}

/// WP2.1 方言核对（REQ-PVR-02/D2）：真实 `kimi acp` wire 捕获——以 aria 生产
/// 同款请求（session.rs:142-151/:210-214/:716-721）驱动
/// initialize → notifications/initialized → session/new → session/prompt →
/// session/load，逐帧打印（DIALECT>> 请求 / DIALECT<< 响应 / DIALECT~ 通知），
/// `--nocapture` 输出 tee 落证据文件供核对矩阵比对冻结 fixtures。
/// 门：KIMI_ACP_E2E=1（CI 永不依赖登录态）。
#[tokio::test]
#[ignore = "set KIMI_ACP_E2E=1 to run against the real kimi CLI"]
async fn live_kimi_acp_dialect_wire_capture() {
    if std::env::var("KIMI_ACP_E2E").ok().as_deref() != Some("1") {
        return;
    }
    let timeout = std::time::Duration::from_secs(120);
    let dir = tempfile::tempdir().expect("dir");
    let mut child = tokio::process::Command::new("kimi")
        .arg("acp")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn kimi acp");
    let stdout = child.stdout.take().expect("kimi stdout");
    let stdin = child.stdin.take().expect("kimi stdin");
    let peer = JsonRpcPeer::new(BufReader::new(stdout), stdin);

    // initialize：与生产 session.rs:142-151 同 shape（clientInfo 标注探测身份）。
    let initialize = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": 1,
            "clientCapabilities": {},
            "clientInfo": {"name": "cadence-aria-dialect-probe", "version": "wp2.1"}
        }
    });
    println!("DIALECT>> initialize {initialize}");
    let initialize_result = peer
        .request_with_timeout(initialize, timeout)
        .await
        .expect("initialize response");
    println!("DIALECT<< initialize {initialize_result}");
    assert!(
        initialize_result.get("error").is_none(),
        "initialize 不得返回错误：{initialize_result}"
    );

    peer.send(serde_json::json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized",
        "params": {}
    }))
    .await
    .expect("send notifications/initialized");

    // session/new：与生产 session.rs:210-214 同 shape（cwd=临时目录）。
    let session_new = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "session/new",
        "params": {
            "cwd": dir.path().display().to_string(),
            "mcpServers": []
        }
    });
    println!("DIALECT>> session/new {session_new}");
    let new_result = peer
        .request_with_timeout(session_new, timeout)
        .await
        .expect("session/new response");
    println!("DIALECT<< session/new {new_result}");
    let session_id = new_result
        .get("sessionId")
        .and_then(serde_json::Value::as_str)
        .expect("session/new result 仍须含 sessionId（生产 :224-234 消费面）")
        .to_string();

    // session/prompt：与生产 :716-721 同 shape（content-block 数组）；结果前到达的
    // session/update 通知由 JsonRpcPeer 缓冲，response 到手后排水逐条打印。
    let prompt = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "session/prompt",
        "params": {
            "sessionId": session_id,
            "prompt": [{"type": "text", "text": "Reply with exactly: dialect-ok"}]
        }
    });
    println!("DIALECT>> session/prompt {prompt}");
    let prompt_result = peer
        .request_with_timeout(prompt, timeout)
        .await
        .expect("session/prompt response");
    while let Some(notification) = peer.try_next_incoming().await {
        println!("DIALECT~ incoming {notification}");
    }
    println!("DIALECT<< session/prompt {prompt_result}");
    assert!(
        prompt_result.get("error").is_none(),
        "session/prompt 不得返回错误：{prompt_result}"
    );

    // session/load：与生产 :188-192 同 shape。注：同进程 load 属探测形态（生产为
    // 跨进程 resume）——错误响应也如实入矩阵（对照 kimi_acp_load_failure_fixture.sh
    // 语义判定是否漂移），不据此单独定漂移结论。
    let session_load = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 4,
        "method": "session/load",
        "params": {
            "sessionId": session_id,
            "cwd": dir.path().display().to_string(),
            "mcpServers": []
        }
    });
    println!("DIALECT>> session/load {session_load}");
    let load_result = peer
        .request_with_timeout(session_load, timeout)
        .await
        .expect("session/load response");
    println!("DIALECT<< session/load {load_result}");

    let _ = child.kill().await;
}
