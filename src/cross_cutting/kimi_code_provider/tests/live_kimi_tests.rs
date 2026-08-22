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
