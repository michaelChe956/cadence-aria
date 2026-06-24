use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Map, Value, json};
use tokio::io::AsyncBufReadExt;
use tokio::sync::{Mutex, mpsc};
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::streaming_provider::{
    ProviderCommand, ProviderEvent, ProviderExecutionEventStatus, ProviderPermissionMode,
    ProviderSession, ProviderStatus, StreamingProviderAdapter, StreamingProviderInput,
};
use crate::protocol::contracts::{AdapterInput, AdapterRole, ProviderType};

use super::ClaudeCodeProvider;

const TEST_TIMEOUT: Duration = Duration::from_secs(5);

#[test]
fn claude_code_provider_supports_provider_driven_testing() {
    let provider = ClaudeCodeProvider::new(PathBuf::from("claude"));

    assert!(provider.supports_provider_driven_testing());
}

#[test]
fn claude_args_include_resume_when_provider_session_is_available() {
    let provider = ClaudeCodeProvider::new(PathBuf::from("claude"));
    let args = provider.build_args(
        ProviderPermissionMode::Supervised,
        Some("claude-session-123"),
    );

    assert!(args.contains(&"--resume".to_string()));
    assert!(args.contains(&"claude-session-123".to_string()));
    assert!(!args.contains(&"--continue".to_string()));
    assert!(!args.contains(&"--fork-session".to_string()));
}

#[test]
fn claude_args_do_not_include_resume_without_provider_session() {
    let provider = ClaudeCodeProvider::new(PathBuf::from("claude"));
    let args = provider.build_args(ProviderPermissionMode::Supervised, None);

    assert!(!args.contains(&"--resume".to_string()));
    assert!(!args.contains(&"--continue".to_string()));
}

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

#[tokio::test]
async fn claude_control_response_uses_sdk_success_envelope_for_approved_tool() {
    let payload = capture_tool_control_response(true, None).await;

    assert_eq!(payload["type"], "control_response");
    assert!(payload.get("request_id").is_none());
    assert_eq!(payload["response"]["subtype"], "success");
    assert_eq!(payload["response"]["request_id"], "perm_req_001");
    assert_eq!(payload["response"]["response"]["behavior"], "allow");
    assert!(payload["response"]["response"]["message"].is_null());
}

#[tokio::test]
async fn claude_control_response_uses_sdk_success_envelope_for_denied_tool() {
    let payload = capture_tool_control_response(false, Some("用户拒绝执行".to_string())).await;

    assert_eq!(payload["type"], "control_response");
    assert!(payload.get("request_id").is_none());
    assert_eq!(payload["response"]["subtype"], "success");
    assert_eq!(payload["response"]["request_id"], "perm_req_001");
    assert_eq!(payload["response"]["response"]["behavior"], "deny");
    assert_eq!(payload["response"]["response"]["message"], "用户拒绝执行");
}

#[tokio::test]
async fn claude_choice_control_response_uses_sdk_success_envelope_with_answers() {
    let original_input = json!({
        "questions": [{
            "question": "Drink?",
            "options": [
                { "label": "Tea" },
                { "label": "Coffee" }
            ]
        }]
    });
    let mut answers = Map::new();
    answers.insert("Drink?".to_string(), Value::String("Tea".to_string()));

    let payload = capture_choice_control_response(original_input, answers).await;

    assert_eq!(payload["type"], "control_response");
    assert!(payload.get("request_id").is_none());
    assert_eq!(payload["response"]["subtype"], "success");
    assert_eq!(payload["response"]["request_id"], "ask_req_001");
    assert_eq!(payload["response"]["response"]["behavior"], "allow");
    assert_eq!(
        payload["response"]["response"]["updatedInput"]["answers"]["Drink?"],
        "Tea"
    );
}

#[test]
fn claude_control_request_reads_top_level_tool_use_id() {
    let value = json!({
        "type": "control_request",
        "request_id": "ask_req_001",
        "tool_use_id": "toolu_question",
        "request": {
            "subtype": "can_use_tool",
            "tool_name": "AskUserQuestion",
            "input": {
                "questions": [{
                    "question": "Drink?",
                    "options": [{ "label": "Tea" }]
                }]
            }
        }
    });

    let request = ClaudeCodeProvider::parse_control_request(&value).expect("control request");

    assert_eq!(request.request_id, "ask_req_001");
    assert_eq!(request.tool_name, "AskUserQuestion");
    assert_eq!(request.tool_use_id.as_deref(), Some("toolu_question"));
}

#[tokio::test]
async fn claude_provider_bridges_permission_and_completes() {
    let fixture = executable_fixture("tests/fixtures/provider/claude_stream_json_fixture.sh");
    let provider = ClaudeCodeProvider::new(fixture);
    let input = streaming_input(ProviderType::ClaudeCode, ProviderPermissionMode::Supervised);

    let mut session: ProviderSession = provider
        .start(input, CancellationToken::new())
        .await
        .unwrap();

    assert!(matches!(
        session.events.recv().await.unwrap(),
        ProviderEvent::StatusChanged(ProviderStatus::Starting)
    ));

    let mut saw_text = false;
    let permission_id = loop {
        match session.events.recv().await.unwrap() {
            ProviderEvent::TextDelta { content } => {
                saw_text = content.contains("# Story Spec")
                    && content.contains("## 功能需求")
                    && content.contains("## 成功标准");
            }
            ProviderEvent::PermissionRequest(data) => break data.id,
            ProviderEvent::StatusChanged(_)
            | ProviderEvent::Execution(_)
            | ProviderEvent::ChoiceRequest(_)
            | ProviderEvent::ToolCall(_)
            | ProviderEvent::ToolResult(_) => {}
            other => panic!("unexpected event before permission: {other:?}"),
        }
    };
    assert!(saw_text);

    session
        .commands
        .send(ProviderCommand::PermissionResponse {
            id: permission_id,
            approved: true,
            reason: None,
        })
        .await
        .unwrap();

    let completed = recv_completed(&mut session.events).await;
    assert!(completed.contains("# Story Spec"));
    assert!(completed.contains("## 功能需求"));
    assert!(completed.contains("## 成功标准"));
}

#[tokio::test]
async fn claude_provider_emits_assistant_final_text_after_stream_delta_when_result_is_empty() {
    let fixture = write_fixture(
        "claude_partial_then_final_assistant_empty_result_fixture.sh",
        r##"#!/usr/bin/env bash
set -euo pipefail

if [[ "${1:-}" == "--version" ]]; then
  echo "claude 2.1.160"
  exit 0
fi

while IFS= read -r line; do
  if [[ "$line" == *'"initialize"'* ]]; then
    continue
  fi
  if [[ "$line" == *'"set_permission_mode"'* ]]; then
    continue
  fi
  if [[ "$line" == *'"user"'* ]]; then
    echo '{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"我先调研。\n"}},"session_id":"claude_fixture_session"}'
    echo '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"```artifact\n# 依赖自检查 Design Spec\n\n## 设计范围\n- [DEC-001] 设计入口。\n\n## 设计决策\n- [DEC-001] 使用轻量探测。\n\n## 公共组件\n- [CMP-001] ProviderDependencyDialog。\n\n## API 契约\n- [API-001] GET /api/provider-dependencies。\n\n## 数据模型\n- repository provider settings。\n\n## 风险\n- npm 缺失。\n\n## 追踪关系\n- REQ-001 -> DEC-001\n```"}]},"session_id":"claude_fixture_session"}'
    echo '{"type":"result","subtype":"success","is_error":false,"result":"","session_id":"claude_fixture_session"}'
    exit 0
  fi
done
"##,
    );
    let provider = ClaudeCodeProvider::new(fixture);
    let input = streaming_input(ProviderType::ClaudeCode, ProviderPermissionMode::Supervised);

    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .expect("start provider");
    let mut streamed = String::new();
    let completed = loop {
        match tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
            .await
            .expect("provider should emit completion")
            .expect("provider event channel should stay open")
        {
            ProviderEvent::TextDelta { content } => streamed.push_str(&content),
            ProviderEvent::Completed { full_output, .. } => break full_output,
            ProviderEvent::StatusChanged(_)
            | ProviderEvent::Execution(_)
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
    };

    assert_eq!(completed, streamed);
    assert!(
        completed.contains("# 依赖自检查 Design Spec"),
        "completed output should fall back to visible assistant text, got: {completed}"
    );
    assert!(
        streamed.contains("# 依赖自检查 Design Spec"),
        "final assistant artifact should be emitted as stream text, got: {streamed}"
    );
}

#[tokio::test]
async fn claude_provider_continues_same_session_after_ask_user_question_choice() {
    let fixture = executable_fixture("tests/fixtures/provider/claude_ask_user_question_fixture.sh");
    let provider = ClaudeCodeProvider::new(fixture);
    let input = streaming_input(ProviderType::ClaudeCode, ProviderPermissionMode::Supervised);

    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .expect("start provider");

    let choice = loop {
        match tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
            .await
            .expect("provider should emit choice")
            .expect("provider event channel should stay open")
        {
            ProviderEvent::ChoiceRequest(choice) => break choice,
            ProviderEvent::Failed { message } => {
                panic!("provider failed before choice: {message}")
            }
            ProviderEvent::ProtocolError { message, .. } => {
                panic!("provider protocol error before choice: {message}")
            }
            ProviderEvent::PermissionTimeout { permission_id } => {
                panic!("provider permission timed out before choice: {permission_id}")
            }
            ProviderEvent::StatusChanged(_)
            | ProviderEvent::Execution(_)
            | ProviderEvent::TextDelta { .. }
            | ProviderEvent::PermissionRequest(_)
            | ProviderEvent::ToolCall(_)
            | ProviderEvent::ToolResult(_)
            | ProviderEvent::Completed { .. } => {}
        }
    };
    assert_eq!(choice.id, "ask_req_001");
    assert_eq!(choice.options[0].id, "opt_0");

    session
        .commands
        .send(ProviderCommand::ChoiceResponse {
            id: choice.id,
            selected_option_ids: vec!["opt_0".to_string()],
            free_text: None,
        })
        .await
        .expect("send choice response");

    let completed = recv_completed(&mut session.events).await;
    assert!(completed.contains("# Story Spec"));
    assert!(completed.contains("## 功能需求"));
    assert!(completed.contains("## 成功标准"));
}

#[tokio::test]
async fn claude_provider_deduplicates_assistant_then_control_ask_user_question() {
    let fixture = write_fixture(
        "claude_assistant_then_control_ask_user_question_fixture.sh",
        r##"#!/usr/bin/env bash
set -euo pipefail

if [[ "${1:-}" == "--version" ]]; then
  echo "claude 2.1.160"
  exit 0
fi

sent_question=0
while IFS= read -r line; do
  if [[ "$line" == *'"initialize"'* ]]; then
    continue
  fi
  if [[ "$line" == *'"set_permission_mode"'* ]]; then
    continue
  fi
  if [[ "$sent_question" == "0" && "$line" == *'"user"'* ]]; then
    sent_question=1
    echo '{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"toolu_question","name":"AskUserQuestion","input":{"questions":[{"question":"Scope?","options":[{"label":"Global"},{"label":"Project"}]}]}}]}}'
    continue
  fi
  if [[ "$line" == *'"tool_result"'* ]]; then
    if [[ "$line" != *'"tool_use_id":"toolu_question"'* || "$line" != *'Scope?'* || "$line" != *'Global'* ]]; then
      echo "missing AskUserQuestion tool_result: $line" >&2
      exit 44
    fi
    echo '{"type":"control_request","request_id":"ask_req_duplicate","request":{"subtype":"can_use_tool","tool_name":"AskUserQuestion","input":{"questions":[{"question":"Scope?","options":[{"label":"Global"},{"label":"Project"}]}]},"tool_use_id":"toolu_question"}}'
    continue
  fi
  if [[ "$line" == *'"control_response"'* ]]; then
    if [[ "$line" != *'"request_id":"ask_req_duplicate"'* || "$line" != *'"updatedInput"'* || "$line" != *'Scope?'* || "$line" != *'Global'* ]]; then
      echo "missing reused AskUserQuestion control_response: $line" >&2
      exit 45
    fi
    echo '{"type":"result","subtype":"success","is_error":false,"result":"# Story Spec\n\n## 功能需求\n- [REQ-001] Duplicate AskUserQuestion events are answered once.\n\n## 成功标准\n- [AC-001] The provider completes without a second frontend choice.","session_id":"claude_fixture_session"}'
    exit 0
  fi
done
"##,
    );
    let provider = ClaudeCodeProvider::new(fixture);
    let input = streaming_input(ProviderType::ClaudeCode, ProviderPermissionMode::Supervised);

    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .expect("start provider");

    let choice = loop {
        match tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
            .await
            .expect("provider should emit choice")
            .expect("provider event channel should stay open")
        {
            ProviderEvent::ChoiceRequest(choice) => break choice,
            ProviderEvent::Failed { message } => {
                panic!("provider failed before choice: {message}")
            }
            ProviderEvent::ProtocolError { message, .. } => {
                panic!("provider protocol error before choice: {message}")
            }
            ProviderEvent::PermissionTimeout { permission_id } => {
                panic!("provider permission timed out before choice: {permission_id}")
            }
            ProviderEvent::StatusChanged(_)
            | ProviderEvent::Execution(_)
            | ProviderEvent::TextDelta { .. }
            | ProviderEvent::PermissionRequest(_)
            | ProviderEvent::ToolCall(_)
            | ProviderEvent::ToolResult(_)
            | ProviderEvent::Completed { .. } => {}
        }
    };
    assert_eq!(choice.id, "toolu_question");

    session
        .commands
        .send(ProviderCommand::ChoiceResponse {
            id: choice.id,
            selected_option_ids: vec!["opt_0".to_string()],
            free_text: None,
        })
        .await
        .expect("send choice response");

    loop {
        match tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
            .await
            .expect("provider should emit completion")
            .expect("provider event channel should stay open")
        {
            ProviderEvent::Completed { full_output, .. } => {
                assert!(full_output.contains("# Story Spec"));
                break;
            }
            ProviderEvent::ChoiceRequest(choice) => {
                panic!(
                    "duplicate AskUserQuestion should not be emitted: {}",
                    choice.id
                )
            }
            ProviderEvent::Failed { message } => panic!("provider failed: {message}"),
            ProviderEvent::ProtocolError { message, .. } => {
                panic!("provider protocol error: {message}")
            }
            ProviderEvent::PermissionTimeout { permission_id } => {
                panic!("provider permission timed out: {permission_id}")
            }
            ProviderEvent::StatusChanged(_)
            | ProviderEvent::Execution(_)
            | ProviderEvent::TextDelta { .. }
            | ProviderEvent::PermissionRequest(_)
            | ProviderEvent::ToolCall(_)
            | ProviderEvent::ToolResult(_) => {}
        }
    }
}

#[tokio::test]
async fn claude_provider_abort_during_ask_user_question_closes_without_completion() {
    let fixture = write_fixture(
        "claude_ask_user_question_abort_fixture.sh",
        r#"#!/usr/bin/env bash
set -euo pipefail

if [[ "${1:-}" == "--version" ]]; then
  echo "claude 2.1.160"
  exit 0
fi

printf '%s\n' "$$" > "$ARIA_FIXTURE_PID"

while IFS= read -r line; do
  if [[ "$line" == *'"initialize"'* ]]; then
    continue
  fi
  if [[ "$line" == *'"set_permission_mode"'* ]]; then
    continue
  fi
  if [[ "$line" == *'"user"'* ]]; then
    echo '{"type":"control_request","request_id":"ask_abort_001","request":{"subtype":"can_use_tool","tool_name":"AskUserQuestion","input":{"questions":[{"question":"Continue?","options":[{"label":"Yes"},{"label":"No"}]}]},"tool_use_id":"toolu_abort_question"}}'
  fi
  if [[ "$line" == *'"control_response"'* ]]; then
    if [[ "$line" != *'"subtype":"success"'* ]]; then
      echo "missing SDK success subtype during abort: $line" >&2
      exit 42
    fi
    if [[ "$line" != *'"updatedInput"'* || "$line" != *'"answers"'* || "$line" != *'"Continue?"'* || "$line" != *'"aborted"'* ]]; then
      echo "missing aborted answer: $line" >&2
      exit 43
    fi
    printf 'aborted-control-response\n' > "$ARIA_ABORT_MARKER"
    sleep 30
  fi
done
"#,
    );
    let marker_dir = tempfile::tempdir().expect("marker dir");
    let marker_path = marker_dir.path().join("abort_marker");
    let pid_path = tempfile::NamedTempFile::new()
        .expect("pid file")
        .into_temp_path();
    let provider = ClaudeCodeProvider::new(fixture);
    let mut input = streaming_input(ProviderType::ClaudeCode, ProviderPermissionMode::Supervised);
    input.env_vars.insert(
        "ARIA_ABORT_MARKER".to_string(),
        marker_path.to_string_lossy().to_string(),
    );
    input.env_vars.insert(
        "ARIA_FIXTURE_PID".to_string(),
        pid_path.to_string_lossy().to_string(),
    );
    let cancel = CancellationToken::new();

    let ProviderSession {
        mut events,
        commands,
    } = provider
        .start(input, cancel.clone())
        .await
        .expect("start provider");

    let choice = loop {
        match tokio::time::timeout(TEST_TIMEOUT, events.recv())
            .await
            .expect("provider should emit choice")
            .expect("provider event channel should stay open")
        {
            ProviderEvent::ChoiceRequest(choice) => break choice,
            ProviderEvent::Failed { message } => {
                panic!("provider failed before choice: {message}")
            }
            ProviderEvent::ProtocolError { message, .. } => {
                panic!("provider protocol error before choice: {message}")
            }
            ProviderEvent::PermissionTimeout { permission_id } => {
                panic!("provider permission timed out before choice: {permission_id}")
            }
            ProviderEvent::StatusChanged(_)
            | ProviderEvent::Execution(_)
            | ProviderEvent::TextDelta { .. }
            | ProviderEvent::PermissionRequest(_)
            | ProviderEvent::ToolCall(_)
            | ProviderEvent::ToolResult(_)
            | ProviderEvent::Completed { .. } => {}
        }
    };
    assert_eq!(choice.id, "ask_abort_001");

    commands
        .send(ProviderCommand::Abort)
        .await
        .expect("send abort command");
    drop(commands);
    wait_for_file(&marker_path).await;

    cancel.cancel();
    let mut saw_aborted = false;
    loop {
        let event = tokio::time::timeout(TEST_TIMEOUT, events.recv())
                .await
                .unwrap_or_else(|_| {
                    panic!(
                        "provider receiver did not close after abort cancellation; saw_aborted={saw_aborted}"
                    )
                });
        let Some(event) = event else {
            break;
        };
        match event {
            ProviderEvent::StatusChanged(ProviderStatus::Aborted) => saw_aborted = true,
            ProviderEvent::Completed { .. } => {
                panic!("aborted AskUserQuestion provider should not complete")
            }
            ProviderEvent::Failed { message } => {
                panic!("provider failed during abort: {message}")
            }
            ProviderEvent::ProtocolError { message, .. } => {
                panic!("provider protocol error during abort: {message}")
            }
            ProviderEvent::PermissionTimeout { permission_id } => {
                panic!("provider permission timed out during abort: {permission_id}")
            }
            ProviderEvent::StatusChanged(_)
            | ProviderEvent::Execution(_)
            | ProviderEvent::TextDelta { .. }
            | ProviderEvent::PermissionRequest(_)
            | ProviderEvent::ChoiceRequest(_)
            | ProviderEvent::ToolCall(_)
            | ProviderEvent::ToolResult(_) => {}
        }
    }
    assert!(saw_aborted);

    #[cfg(target_os = "linux")]
    {
        let pid = std::fs::read_to_string(&pid_path)
            .expect("fixture pid")
            .trim()
            .parse::<u32>()
            .expect("fixture pid number");
        wait_for_process_absent(pid).await;
    }
}

#[tokio::test]
async fn claude_provider_reports_failure_when_process_exits_without_result() {
    let fixture = write_fixture(
        "claude_fail_fixture.sh",
        "#!/usr/bin/env bash\nset -euo pipefail\nexec 0<&-\necho 'not authenticated' >&2\nexit 7\n",
    );
    let provider = ClaudeCodeProvider::new(fixture);
    let input = streaming_input(ProviderType::ClaudeCode, ProviderPermissionMode::Auto);

    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .expect("start provider");

    let mut failed = None;
    while let Some(event) = tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
        .await
        .expect("provider should emit failure")
    {
        if let ProviderEvent::Failed { message } = event {
            failed = Some(message);
            break;
        }
    }

    let failed = failed.expect("provider should emit failed event");
    assert!(
        failed.contains("exited without result") || failed.contains("exit status"),
        "unexpected failure message: {failed}"
    );
    assert!(
        failed.contains("not authenticated"),
        "unexpected failure message: {failed}"
    );
}

#[tokio::test]
async fn claude_provider_truncates_multibyte_tool_result_preview_without_panicking() {
    let long_output = "通".repeat(180);
    let tool_use_line = serde_json::json!({
        "type": "assistant",
        "message": {
            "role": "assistant",
            "content": [{
                "type": "tool_use",
                "id": "toolu_utf8",
                "name": "Bash",
                "input": { "command": "printf unicode output" }
            }]
        },
        "session_id": "claude_fixture_session"
    })
    .to_string();
    let tool_result_line = serde_json::json!({
        "type": "user",
        "message": {
            "role": "user",
            "content": [{
                "type": "tool_result",
                "tool_use_id": "toolu_utf8",
                "content": long_output
            }]
        },
        "session_id": "claude_fixture_session"
    })
    .to_string();
    let result_line = serde_json::json!({
        "type": "result",
        "subtype": "success",
        "is_error": false,
        "result": "done",
        "session_id": "claude_fixture_session"
    })
    .to_string();
    let body = format!(
        "#!/usr/bin/env bash\nset -euo pipefail\nwhile IFS= read -r line; do\n  if [[ \"$line\" == *'\"user\"'* ]]; then\n    printf '%s\\n' '{tool_use_line}'\n    printf '%s\\n' '{tool_result_line}'\n    printf '%s\\n' '{result_line}'\n    exit 0\n  fi\ndone\n"
    );
    let fixture = write_fixture("claude_utf8_tool_result_fixture.sh", &body);
    let provider = ClaudeCodeProvider::new(fixture);
    let input = streaming_input(ProviderType::ClaudeCode, ProviderPermissionMode::Auto);

    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .expect("start provider");
    let mut preview = None;
    let mut completed = None;

    while let Some(event) = tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
        .await
        .expect("provider should not hang while handling utf8 tool result")
    {
        match event {
            ProviderEvent::Execution(event) => {
                if event.event_id == "toolu_utf8"
                    && event.status == ProviderExecutionEventStatus::Completed
                {
                    preview = event.output;
                }
            }
            ProviderEvent::Completed { full_output, .. } => {
                completed = Some(full_output);
                break;
            }
            ProviderEvent::Failed { message } => panic!("provider failed: {message}"),
            ProviderEvent::ProtocolError { message, .. } => {
                panic!("provider protocol error: {message}")
            }
            ProviderEvent::PermissionTimeout { permission_id } => {
                panic!("provider permission timed out: {permission_id}")
            }
            ProviderEvent::StatusChanged(_)
            | ProviderEvent::TextDelta { .. }
            | ProviderEvent::PermissionRequest(_)
            | ProviderEvent::ChoiceRequest(_)
            | ProviderEvent::ToolCall(_)
            | ProviderEvent::ToolResult(_) => {}
        }
    }

    assert_eq!(completed.as_deref(), Some("done"));
    let preview = preview.expect("tool result preview");
    assert!(preview.ends_with("..."), "preview should be truncated");
    assert!(
        preview.len() <= 503,
        "preview should be capped to 500 bytes plus suffix, got {} bytes",
        preview.len()
    );
}

#[tokio::test]
#[cfg(target_os = "linux")]
async fn claude_provider_cancel_kills_and_reaps_hanging_process() {
    let fixture = write_fixture(
        "claude_hanging_after_output_fixture.sh",
        "#!/usr/bin/env bash\nset -euo pipefail\nprintf '%s\\n' \"$$\" > \"$ARIA_FIXTURE_PID\"\nwhile IFS= read -r line; do\n  if [[ \"$line\" == *'\"user\"'* ]]; then\n    echo '{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"started\"}]},\"session_id\":\"claude_fixture_session\"}'\n    sleep 30\n  fi\ndone\n",
    );
    let provider = ClaudeCodeProvider::new(fixture);
    let pid_path = tempfile::NamedTempFile::new()
        .expect("pid file")
        .into_temp_path();
    let mut input = streaming_input(ProviderType::ClaudeCode, ProviderPermissionMode::Auto);
    input.env_vars.insert(
        "ARIA_FIXTURE_PID".to_string(),
        pid_path.to_string_lossy().to_string(),
    );
    let cancel = CancellationToken::new();

    let mut session = provider
        .start(input, cancel.clone())
        .await
        .expect("start provider");
    loop {
        match tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
            .await
            .expect("provider should emit startup event")
            .expect("provider channel should stay open until cancellation")
        {
            ProviderEvent::TextDelta { content } if content == "started" => break,
            ProviderEvent::Failed { message } => panic!("provider failed: {message}"),
            ProviderEvent::ProtocolError { message, .. } => {
                panic!("provider protocol error: {message}")
            }
            ProviderEvent::PermissionTimeout { permission_id } => {
                panic!("provider permission timed out: {permission_id}")
            }
            ProviderEvent::StatusChanged(_)
            | ProviderEvent::Execution(_)
            | ProviderEvent::TextDelta { .. }
            | ProviderEvent::PermissionRequest(_)
            | ProviderEvent::ChoiceRequest(_)
            | ProviderEvent::ToolCall(_)
            | ProviderEvent::ToolResult(_)
            | ProviderEvent::Completed { .. } => {}
        }
    }

    let pid = std::fs::read_to_string(&pid_path)
        .expect("fixture pid")
        .trim()
        .parse::<u32>()
        .expect("fixture pid number");
    cancel.cancel();
    drop(session);
    wait_for_process_absent(pid).await;
}

#[tokio::test]
async fn claude_provider_run_streaming_cancel_closes_backpressured_bridge() {
    let mut body = String::from("#!/usr/bin/env bash\nset -euo pipefail\n");
    body.push_str("while IFS= read -r line; do\n");
    body.push_str("  if [[ \"$line\" == *'\"user\"'* ]]; then\n");
    for index in 0..80 {
        body.push_str(&format!(
                "    echo '{{\"type\":\"assistant\",\"message\":{{\"role\":\"assistant\",\"content\":[{{\"type\":\"text\",\"text\":\"chunk {index}\"}}]}},\"session_id\":\"backpressure\"}}'\n"
            ));
    }
    body.push_str("    sleep 5\n");
    body.push_str("  fi\n");
    body.push_str("done\n");

    let fixture = write_fixture("claude_backpressure_fixture.sh", &body);
    let provider = ClaudeCodeProvider::new(fixture);
    let cancel = CancellationToken::new();
    let rx = provider
        .run_streaming(&adapter_input("trigger backpressure"), cancel.clone())
        .await
        .expect("run streaming");

    wait_for_buffer_len(&rx, 32).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    cancel.cancel();
    tokio::time::timeout(TEST_TIMEOUT, wait_for_receiver_closed(&rx))
        .await
        .expect("stream receiver should close after cancellation");
}

#[tokio::test]
async fn claude_provider_ask_user_question_emits_protocol_error_on_bridge_failure() {
    let fixture = executable_fixture("tests/fixtures/provider/claude_ask_user_question_fixture.sh");
    let provider = ClaudeCodeProvider::new(fixture);
    let input = streaming_input(ProviderType::ClaudeCode, ProviderPermissionMode::Supervised);
    let cancel = CancellationToken::new();

    let mut session = provider
        .start(input, cancel.clone())
        .await
        .expect("start provider");

    let _choice = loop {
        match tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
            .await
            .expect("provider should emit choice")
            .expect("provider event channel should stay open")
        {
            ProviderEvent::ChoiceRequest(choice) => break choice,
            ProviderEvent::Failed { message } => {
                panic!("provider failed before choice: {message}")
            }
            ProviderEvent::ProtocolError { message, .. } => {
                panic!("provider protocol error before choice: {message}")
            }
            _ => {}
        }
    };

    // 取消会话，让 bridge 的 request_choice 返回错误，同时保持 receiver 打开以接收 ProtocolError。
    cancel.cancel();

    // provider 应该抛出 ProtocolError，而不是通用的 Failed。
    let mut saw_protocol_error = false;
    while let Some(event) = tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
        .await
        .unwrap_or(None)
    {
        if let ProviderEvent::ProtocolError {
            code,
            message,
            context,
        } = event
        {
            assert_eq!(code, "ask_user_question_unresolved");
            assert!(
                message.contains("AskUserQuestion"),
                "message should mention AskUserQuestion: {message}"
            );
            assert!(
                message.contains("unresolved"),
                "message should mention unresolved: {message}"
            );
            let ctx = context.expect("context should be present");
            assert_eq!(ctx["request_id"], "ask_req_001");
            assert_eq!(ctx["tool_use_id"], "toolu_question");
            saw_protocol_error = true;
            break;
        }
    }
    assert!(
        saw_protocol_error,
        "expected ask_user_question_unresolved protocol error after bridge failure"
    );
}

#[tokio::test]
async fn claude_provider_ask_user_question_tool_use_emits_protocol_error_on_bridge_failure() {
    let fixture = executable_fixture(
        "tests/fixtures/provider/claude_ask_user_question_tool_use_bridge_failure_fixture.sh",
    );
    let provider = ClaudeCodeProvider::new(fixture);
    let input = streaming_input(ProviderType::ClaudeCode, ProviderPermissionMode::Supervised);
    let cancel = CancellationToken::new();

    let mut session = provider
        .start(input, cancel.clone())
        .await
        .expect("start provider");

    let _choice = loop {
        match tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
            .await
            .expect("provider should emit choice")
            .expect("provider event channel should stay open")
        {
            ProviderEvent::ChoiceRequest(choice) => break choice,
            ProviderEvent::Failed { message } => {
                panic!("provider failed before choice: {message}")
            }
            ProviderEvent::ProtocolError { message, .. } => {
                panic!("provider protocol error before choice: {message}")
            }
            _ => {}
        }
    };

    cancel.cancel();

    let mut saw_protocol_error = false;
    while let Some(event) = tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
        .await
        .unwrap_or(None)
    {
        if let ProviderEvent::ProtocolError {
            code,
            message,
            context,
        } = event
        {
            assert_eq!(code, "ask_user_question_unresolved");
            assert!(
                message.contains("AskUserQuestion"),
                "message should mention AskUserQuestion: {message}"
            );
            assert!(
                message.contains("unresolved"),
                "message should mention unresolved: {message}"
            );
            let ctx = context.expect("context should be present");
            assert_eq!(ctx["tool_use_id"], "toolu_question");
            saw_protocol_error = true;
            break;
        }
    }
    assert!(
        saw_protocol_error,
        "expected ask_user_question_unresolved protocol error after tool_use bridge failure"
    );
}

#[tokio::test]
async fn claude_provider_ask_user_question_emits_protocol_error_on_tool_result_error() {
    let fixture = executable_fixture(
        "tests/fixtures/provider/claude_ask_user_question_tool_error_fixture.sh",
    );
    let provider = ClaudeCodeProvider::new(fixture);
    let input = streaming_input(ProviderType::ClaudeCode, ProviderPermissionMode::Supervised);

    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .expect("start provider");

    let choice = loop {
        match tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
            .await
            .expect("provider should emit choice")
            .expect("provider event channel should stay open")
        {
            ProviderEvent::ChoiceRequest(choice) => break choice,
            ProviderEvent::Failed { message } => {
                panic!("provider failed before choice: {message}")
            }
            _ => {}
        }
    };
    assert_eq!(choice.id, "toolu_question");

    session
        .commands
        .send(ProviderCommand::ChoiceResponse {
            id: choice.id,
            selected_option_ids: vec!["opt_0".to_string()],
            free_text: None,
        })
        .await
        .expect("send choice response");

    let mut saw_protocol_error = false;
    while let Some(event) = tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
        .await
        .expect("provider should emit events")
    {
        match event {
            ProviderEvent::ProtocolError {
                code,
                message,
                context,
            } if code == "ask_user_question_unresolved" => {
                assert!(
                    message.contains("AskUserQuestion"),
                    "message should mention AskUserQuestion: {message}"
                );
                assert!(
                    message.contains("unresolved"),
                    "message should mention unresolved: {message}"
                );
                let ctx = context.expect("context should be present");
                assert_eq!(ctx["tool_use_id"], "toolu_question");
                assert_eq!(ctx["output"], "User refused to answer");
                saw_protocol_error = true;
                break;
            }
            ProviderEvent::Completed { .. } => {
                panic!("provider should not complete after AskUserQuestion tool_result error")
            }
            _ => {}
        }
    }
    assert!(
        saw_protocol_error,
        "expected ask_user_question_unresolved protocol error on tool_result is_error"
    );
}
