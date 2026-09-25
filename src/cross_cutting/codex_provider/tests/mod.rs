use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::approval_bridge::ApprovalBridge;
use crate::cross_cutting::json_rpc_peer::{JsonRpcPeer, OutboundIdNamespace, ensure_request_id};
use crate::cross_cutting::streaming_provider::{
    ChoiceAnswerData, CodexApprovalCategory, CodexApprovalResponse, ProviderCommand,
    ProviderCompletion, ProviderEvent, ProviderExecutionEventKind, ProviderExecutionEventStatus,
    ProviderPermissionMode, ProviderToolPolicy, StreamingProviderAdapter, StreamingProviderInput,
};
use crate::cross_cutting::structured_output::{StructuredOutputContract, StructuredOutputState};
use crate::protocol::contracts::{AdapterRole, ProviderType};

use super::CodexProvider;
use super::is_turn_completed;
use super::parse_codex_usage;
use super::parse_failure;
use super::session::{CodexSessionHandshake, codex_launch_params, run_codex_session_loop};
mod approval_policy;
mod bridging;
mod retry_timeout;
mod streaming;
mod version_probe;

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
    // 非策略 legacy 路径 fixture：守卫（Task 3.1）要求非策略会话使用非策略角色。
    StreamingProviderInput {
        baseline_tree: None,
        tool_policy: None,
        audit_sink: None,
        provider_type,
        role: AdapterRole::Executor,
        prompt: "fixture prompt".to_string(),
        working_dir: std::env::current_dir().unwrap(),
        workspace_session_id: None,
        resume_provider_session_id: None,
        permission_mode,
        structured_output_contract: None,
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
            ProviderEvent::Completed(completion) => return completion.full_output,
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
            ProviderEvent::UsageReport(_)
            | ProviderEvent::ToolPolicyDecision(_)
            | ProviderEvent::ToolPolicyWarning(_)
            | ProviderEvent::ToolPolicyTerminated(_) => {}
            ProviderEvent::PermissionTimeout { permission_id } => {
                panic!("provider permission timed out: {permission_id}")
            }
        }
    }
}

async fn recv_completion(events: &mut mpsc::Receiver<ProviderEvent>) -> ProviderCompletion {
    loop {
        match tokio::time::timeout(TEST_TIMEOUT, events.recv())
            .await
            .expect("provider should emit completion")
            .expect("provider event channel should stay open")
        {
            ProviderEvent::Completed(completion) => return completion,
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
            ProviderEvent::UsageReport(_)
            | ProviderEvent::ToolPolicyDecision(_)
            | ProviderEvent::ToolPolicyWarning(_)
            | ProviderEvent::ToolPolicyTerminated(_) => {}
            ProviderEvent::PermissionTimeout { permission_id } => {
                panic!("provider permission timed out: {permission_id}")
            }
        }
    }
}

#[test]
fn codex_provider_enables_default_mode_request_user_input_feature() {
    let provider = CodexProvider::new(PathBuf::from("codex"));

    assert_eq!(
        provider.build_args(),
        vec![
            "app-server".to_string(),
            "--enable".to_string(),
            "default_mode_request_user_input".to_string(),
        ]
    );
}

#[tokio::test]
async fn codex_provider_carries_structured_completion() {
    let fixture =
        executable_fixture("tests/fixtures/provider/codex_app_server_structured_output_fixture.sh");
    let provider = CodexProvider::new(fixture);
    let mut input = streaming_input(ProviderType::Codex, ProviderPermissionMode::Auto);
    input.structured_output_contract = Some(StructuredOutputContract {
        nonce: "96aca42f".to_string(),
        schema_name: "workspace_review".to_string(),
    });
    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .expect("start provider");

    let completion = recv_completion(&mut session.events).await;

    assert_eq!(completion.readable_output, "审核说明");
    assert!(matches!(
        completion.structured_output,
        StructuredOutputState::Parsed(ref value) if value["verdict"] == "pass"
    ));
}

#[tokio::test]
async fn codex_resume_uses_existing_thread_without_starting_new_thread() {
    let fixture = executable_fixture("tests/fixtures/provider/codex_app_server_resume_fixture.sh");
    let provider = CodexProvider::new(fixture);
    let mut input = streaming_input(ProviderType::Codex, ProviderPermissionMode::Auto);
    input.resume_provider_session_id = Some("codex-thread-123".to_string());
    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .unwrap();

    let completed = recv_completed(&mut session.events).await;

    assert_eq!(completed, "resumed done");
}

#[tokio::test]
async fn codex_thread_start_creates_persistent_thread_for_later_resume() {
    let fixture =
        executable_fixture("tests/fixtures/provider/codex_app_server_persistent_thread_fixture.sh");
    let provider = CodexProvider::new(fixture);
    let input = streaming_input(ProviderType::Codex, ProviderPermissionMode::Auto);
    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .unwrap();

    let completed = recv_completed(&mut session.events).await;

    assert_eq!(completed, "persistent thread done");
}

#[tokio::test]
async fn codex_thread_start_requests_danger_full_access_sandbox() {
    let fixture = executable_fixture("tests/fixtures/provider/codex_app_server_sandbox_fixture.sh");
    let provider = CodexProvider::new(fixture);
    let input = streaming_input(ProviderType::Codex, ProviderPermissionMode::Supervised);
    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .unwrap();

    let completed = recv_completed(&mut session.events).await;

    assert_eq!(completed, "sandbox disabled done");
}

#[tokio::test]
async fn codex_thread_resume_requests_danger_full_access_sandbox() {
    let fixture =
        executable_fixture("tests/fixtures/provider/codex_app_server_resume_sandbox_fixture.sh");
    let provider = CodexProvider::new(fixture);
    let mut input = streaming_input(ProviderType::Codex, ProviderPermissionMode::Auto);
    input.resume_provider_session_id = Some("codex-thread-123".to_string());
    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .unwrap();

    let completed = recv_completed(&mut session.events).await;

    assert_eq!(completed, "resume sandbox disabled done");
}

// 组 2：turn/completed 状态门控的显式断言（既有 completed/缺省轮不受 429 修复影响）。
#[test]
fn codex_turn_completed_status_gate_keeps_completed_and_default_turns_as_completed() {
    // status="completed"（新协议实测形态）仍是完成轮。
    let completed = json!({
        "method": "turn/completed",
        "params": {
            "threadId": "thread-1",
            "turn": { "id": "turn-1", "items": [], "status": "completed" }
        }
    });
    assert!(is_turn_completed(&completed));
    assert!(parse_failure(&completed).is_none());

    // 缺省 status（既有 resume/sandbox/persistent_thread fixtures 形态）仍是完成轮。
    let no_status = json!({
        "method": "turn/completed",
        "params": { "threadId": "thread-1", "turnId": "turn-1" }
    });
    assert!(is_turn_completed(&no_status));
    assert!(parse_failure(&no_status).is_none());

    // legacy codex/event 的 turn_completed 不受影响。
    let legacy = json!({
        "method": "codex/event",
        "params": { "msg": { "type": "turn_completed" } }
    });
    assert!(is_turn_completed(&legacy));

    // status:"failed" 不再命中完成分支，改走失败解析且文案原样透传。
    let failed = json!({
        "method": "turn/completed",
        "params": {
            "threadId": "thread-1",
            "turn": {
                "id": "turn-1",
                "items": [],
                "status": "failed",
                "error": {
                    "message": "exceeded retry limit, last status: 429 Too Many Requests",
                    "codexErrorInfo": {
                        "responseTooManyFailedAttempts": { "httpStatusCode": 429 }
                    }
                }
            }
        }
    });
    assert!(!is_turn_completed(&failed));
    let failure = parse_failure(&failed).expect("failed turn should parse as failure");
    assert_eq!(
        failure, "exceeded retry limit, last status: 429 Too Many Requests",
        "failure message must be passed through verbatim"
    );
}

#[test]
fn parse_codex_usage_reads_camel_case_turn_completed_usage() {
    let notification = serde_json::json!({
        "method": "turn/completed",
        "params": {
            "usage": {
                "inputTokens": 210,
                "outputTokens": 58,
                "cachedInputTokens": 64
            }
        }
    });
    let report = parse_codex_usage(&notification, "reviewer").expect("usage should parse");
    assert_eq!(report.role, "reviewer");
    assert_eq!(report.input_tokens, Some(210));
    assert_eq!(report.output_tokens, Some(58));
    assert_eq!(report.cache_read_tokens, Some(64));
    assert_eq!(report.cache_creation_tokens, None);
}

#[test]
fn parse_codex_usage_reads_snake_case_legacy_usage() {
    let notification = serde_json::json!({
        "method": "codex/event",
        "params": {
            "msg": {
                "type": "turn_completed",
                "usage": { "input_tokens": 11, "output_tokens": 5 }
            }
        }
    });
    let report = parse_codex_usage(&notification, "author").expect("usage should parse");
    assert_eq!(report.input_tokens, Some(11));
    assert_eq!(report.output_tokens, Some(5));
}

#[test]
fn parse_codex_usage_returns_none_when_usage_missing() {
    let notification = serde_json::json!({ "method": "turn/completed", "params": {} });
    assert!(parse_codex_usage(&notification, "author").is_none());
}

#[test]
fn parse_codex_usage_reads_nested_total_token_usage_object() {
    // 嵌套形状（~/.codex/sessions 实测）：total_token_usage 为对象，非扁平 key
    let notification = serde_json::json!({
        "method": "turn/completed",
        "params": {
            "usage": {
                "total_token_usage": {
                    "input_tokens": 16190,
                    "cached_input_tokens": 2432,
                    "output_tokens": 482
                }
            }
        }
    });
    let report = parse_codex_usage(&notification, "author").expect("usage should parse");
    assert_eq!(report.input_tokens, Some(16190));
    assert_eq!(report.output_tokens, Some(482));
    assert_eq!(report.cache_read_tokens, Some(2432));
}
