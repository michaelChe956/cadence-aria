use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Map, Value};
use tokio::io::AsyncBufReadExt;
use tokio::sync::{Mutex, mpsc};
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::streaming_provider::{
    ProviderEvent, ProviderPermissionMode, StreamingProviderInput,
};
use crate::protocol::contracts::{AdapterInput, AdapterRole, ProviderType};

use super::ClaudeCodeProvider;

mod args;
mod ask_user_question;
mod permissions;
mod process;
mod streaming;

const TEST_TIMEOUT: Duration = Duration::from_secs(5);

fn executable_fixture(relative_path: &str) -> PathBuf {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative_path);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = std::fs::metadata(&path)
            .unwrap_or_else(|error| panic!("fixture metadata {}: {error}", path.display()))
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&path, permissions)
            .unwrap_or_else(|error| panic!("chmod fixture {}: {error}", path.display()));
    }
    path
}
fn streaming_input(
    provider_type: ProviderType,
    permission_mode: ProviderPermissionMode,
) -> StreamingProviderInput {
    StreamingProviderInput {
        provider_type,
        role: AdapterRole::Orchestrator,
        prompt: "Run the fixture provider".to_string(),
        working_dir: std::env::current_dir().unwrap(),
        workspace_session_id: None,
        resume_provider_session_id: None,
        permission_mode,
        env_vars: BTreeMap::new(),
        timeout_secs: 60,
    }
}
async fn recv_completed(events: &mut mpsc::Receiver<ProviderEvent>) -> String {
    loop {
        match tokio::time::timeout(TEST_TIMEOUT, events.recv())
            .await
            .expect("provider should emit completion")
            .expect("provider event channel should stay open")
        {
            ProviderEvent::Completed { full_output, .. } => return full_output,
            ProviderEvent::StatusChanged(_)
            | ProviderEvent::Execution(_)
            | ProviderEvent::TextDelta { .. }
            | ProviderEvent::PermissionRequest(_)
            | ProviderEvent::ChoiceRequest(_)
            | ProviderEvent::ToolCall(_)
            | ProviderEvent::ToolResult(_) => {}
            ProviderEvent::Failed { message } => panic!("provider failed: {message}"),
            ProviderEvent::ProtocolError { message, .. } => {
                panic!("provider protocol error: {message}")
            }
            ProviderEvent::PermissionTimeout { permission_id } => {
                panic!("provider permission timed out: {permission_id}")
            }
        }
    }
}
async fn wait_for_receiver_closed<T>(rx: &mpsc::Receiver<T>) {
    for _ in 0..1000 {
        if rx.is_closed() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("receiver did not close after cancellation");
}
async fn wait_for_buffer_len<T>(rx: &mpsc::Receiver<T>, expected_len: usize) {
    for _ in 0..1000 {
        if rx.len() >= expected_len {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!(
        "receiver buffer did not reach {expected_len} items; actual len is {}",
        rx.len()
    );
}
async fn wait_for_file(path: &Path) {
    for _ in 0..200 {
        if path.exists() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("file did not appear: {}", path.display());
}
#[cfg(target_os = "linux")]
async fn wait_for_process_absent(pid: u32) {
    let proc_path = PathBuf::from(format!("/proc/{pid}"));
    for _ in 0..200 {
        if !proc_path.exists() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("process {pid} was not reaped after cancellation");
}
fn adapter_input(prompt: &str) -> AdapterInput {
    AdapterInput {
        provider_type: ProviderType::ClaudeCode,
        role: AdapterRole::Orchestrator,
        worktree_path: Some(
            std::env::current_dir()
                .unwrap()
                .to_string_lossy()
                .to_string(),
        ),
        prompt: prompt.to_string(),
        context_files: Vec::new(),
        output_schema: String::new(),
        timeout: 60,
        max_retries: 0,
    }
}
fn write_fixture(relative_path: &str, body: &str) -> PathBuf {
    let path = tempfile::tempdir()
        .expect("fixture dir")
        .keep()
        .join(relative_path);
    std::fs::write(&path, body).expect("write fixture");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = std::fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&path, permissions).expect("chmod fixture");
    }
    path
}
async fn capture_tool_control_response(approved: bool, reason: Option<String>) -> Value {
    let mut child = tokio::process::Command::new("cat")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn cat fixture");
    let stdin = Arc::new(Mutex::new(child.stdin.take().expect("child stdin")));
    let stdout = child.stdout.take().expect("child stdout");

    ClaudeCodeProvider::write_control_response(&stdin, "perm_req_001", approved, reason)
        .await
        .expect("write control response");
    drop(stdin);

    let mut lines = tokio::io::BufReader::new(stdout).lines();
    let line = tokio::time::timeout(TEST_TIMEOUT, lines.next_line())
        .await
        .expect("control response line timeout")
        .expect("read control response line")
        .expect("control response line");
    let _ = tokio::time::timeout(TEST_TIMEOUT, child.wait())
        .await
        .expect("cat wait timeout")
        .expect("cat status");
    serde_json::from_str(&line).expect("control response json")
}
async fn capture_choice_control_response(
    original_input: Value,
    answers: Map<String, Value>,
) -> Value {
    let mut child = tokio::process::Command::new("cat")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn cat fixture");
    let stdin = Arc::new(Mutex::new(child.stdin.take().expect("child stdin")));
    let stdout = child.stdout.take().expect("child stdout");

    ClaudeCodeProvider::write_choice_control_response(
        &stdin,
        "ask_req_001",
        &original_input,
        answers,
    )
    .await
    .expect("write choice control response");
    drop(stdin);

    let mut lines = tokio::io::BufReader::new(stdout).lines();
    let line = tokio::time::timeout(TEST_TIMEOUT, lines.next_line())
        .await
        .expect("choice control response line timeout")
        .expect("read choice control response line")
        .expect("choice control response line");
    let _ = tokio::time::timeout(TEST_TIMEOUT, child.wait())
        .await
        .expect("cat wait timeout")
        .expect("cat status");
    serde_json::from_str(&line).expect("choice control response json")
}
