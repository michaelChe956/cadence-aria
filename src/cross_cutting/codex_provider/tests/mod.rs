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
    ProviderPermissionMode, ProviderToolPolicy, ProviderVersionSupplier, StreamingProviderAdapter,
    StreamingProviderInput,
};
use crate::cross_cutting::structured_output::{StructuredOutputContract, StructuredOutputState};
use crate::product::logical_codebase::policy::{
    PolicyTarget, ProviderDialect, SessionPolicyAction, SessionPolicyEnvelope,
};
use crate::product::logical_codebase::provider_gateway::ProviderRef;
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
        working_directory: None,
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

// ── Task 5 LC 受限 sandbox 测试 fixture(REQ-LCG-04)──────────────────────
//
// gateway 路由级 Codex 阻断(默认 danger-full-access)迁移到 LC projection
// 判定归 Task 3/8;此前真实 gateway validate 无法为 Codex 产出 validated
// policy。本 fixture 在 provider 域构造 gateway 同形状的冻结 envelope,直接
// 驱动 `CodexProvider::start_lc_validated`(`start_validated` 的真实执行体)
// 锁定受限 sandbox/wire/审计行为。

/// LC fixture 的冻结 exact version(supplier seam 注入)。
fn lc_version_supplier() -> ProviderVersionSupplier {
    std::sync::Arc::new(|| Ok("codex 0.124.0-lc-fixture".to_string()))
}

/// LC app-server 假体(运行时写入;三家 Task 4 先例同构):
/// - 执行即 `touch $LC_SPAWN_MARKER`(零 child 断言的观测点);
/// - `pwd -P > $LC_CWD_MARKER`(进程 cwd 断言);
/// - thread/start|resume 原始请求行写 `$LC_WIRE_MARKER`(真实 RPC params
///   断言),应答固定 thread id `codex-thread-lc`;
/// - turn/start 应答后送出 agentMessage + turn/completed 并退出。
fn lc_app_server_fixture() -> PathBuf {
    let path = tempfile::tempdir()
        .expect("lc fixture dir")
        .keep()
        .join("codex_lc_app_server_fixture.sh");
    std::fs::write(&path, LC_APP_SERVER_FIXTURE_BODY).expect("write lc fixture");
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

const LC_APP_SERVER_FIXTURE_BODY: &str = r#"#!/usr/bin/env bash
set -uo pipefail
pwd -P > "$LC_CWD_MARKER"
touch "$LC_SPAWN_MARKER"
while IFS= read -r line; do
  if [[ "$line" == *'"method":"initialize"'* ]]; then
    id="$(printf '%s' "$line" | sed -n -e 's/.*"id":[[:space:]]*"\([0-9A-Za-z_-][0-9A-Za-z_-]*\)".*/\1/p' -e 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"jsonrpc\":\"2.0\",\"id\":\"${id:-aria-0}\",\"result\":{\"capabilities\":{}}}"
  elif [[ "$line" == *'"method":"initialized"'* ]]; then
    :
  elif [[ "$line" == *'"method":"thread/start"'* ]] || [[ "$line" == *'"method":"thread/resume"'* ]]; then
    printf '%s' "$line" > "$LC_WIRE_MARKER"
    id="$(printf '%s' "$line" | sed -n -e 's/.*"id":[[:space:]]*"\([0-9A-Za-z_-][0-9A-Za-z_-]*\)".*/\1/p' -e 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"jsonrpc\":\"2.0\",\"id\":\"${id:-aria-1}\",\"result\":{\"thread\":{\"id\":\"codex-thread-lc\"}}}"
  elif [[ "$line" == *'"method":"turn/start"'* ]]; then
    id="$(printf '%s' "$line" | sed -n -e 's/.*"id":[[:space:]]*"\([0-9A-Za-z_-][0-9A-Za-z_-]*\)".*/\1/p' -e 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"jsonrpc\":\"2.0\",\"id\":\"${id:-aria-2}\",\"result\":{\"turn\":{\"id\":\"turn-lc\"}}}"
    echo '{"jsonrpc":"2.0","method":"item/completed","params":{"item":{"id":"msg-1","type":"agentMessage","text":"lc restricted done"}}}'
    echo '{"jsonrpc":"2.0","method":"turn/completed","params":{"turnId":"turn-lc"}}'
    exit 0
  fi
done
"#;

/// LC 启动 fixture:canonical root 与成员 target worktree 分离(双 cwd 合同:
/// 进程 cwd=root,协议 cwd 由 action 投影决定)。
struct LcCodexFixture {
    root: PathBuf,
    target: PathBuf,
}

impl LcCodexFixture {
    fn new() -> Self {
        let root = tempfile::tempdir().expect("lc fixture root").keep();
        let target = root.join("member-worktree");
        std::fs::create_dir_all(&target).expect("create member worktree");
        Self { root, target }
    }

    /// canonical LC root(= manifest `provider_context_root` 的 canonical 形态)。
    fn canonical_root(&self) -> PathBuf {
        std::fs::canonicalize(&self.root).expect("lc fixture root exists")
    }

    /// 唯一成员 target worktree(与 canonical root 分离)。
    fn target_worktree(&self) -> PathBuf {
        self.target.clone()
    }

    /// gateway 同形状的冻结 envelope(正确形态:read-only 空 writable roots;
    /// coding 恰一个等于 target 的可写 root。形状违规变体由调用方传入)。
    fn envelope(
        &self,
        action: SessionPolicyAction,
        writable_roots: Vec<PathBuf>,
    ) -> SessionPolicyEnvelope {
        let canonical_root = self.canonical_root();
        SessionPolicyEnvelope {
            policy_id: "policy_lc_codex_fixture".to_string(),
            policy_revision: 1,
            policy_digest: "sha256:policy-lc-codex-fixture".to_string(),
            action,
            target: PolicyTarget::checkout(
                "logical_repo_0001",
                "checkout_0001",
                self.target.clone(),
            ),
            working_directory: canonical_root.clone(),
            readable_roots: vec![canonical_root.clone()],
            writable_roots,
            provider_dialect: ProviderDialect::CodexCliV1,
            config_artifact_ref: "sha256:managed-config-artifact".to_string(),
            config_digest: "sha256:cfg-digest-lc".to_string(),
            created_at: "2026-10-04T00:00:00Z".to_string(),
            authority_root: canonical_root,
        }
    }

    /// LC streaming input(`raw_working_dir` 故意与投影 protocol cwd 相反,
    /// 证明 wire 参数来自投影而非 raw 输入)。
    #[allow(clippy::too_many_arguments)]
    fn lc_input(
        &self,
        role: AdapterRole,
        tool_policy: Option<ProviderToolPolicy>,
        permission_mode: ProviderPermissionMode,
        resume_id: Option<String>,
        audit_sink: Option<
            std::sync::Arc<dyn crate::cross_cutting::tool_policy_audit::ToolPolicyAuditSink>,
        >,
        raw_working_dir: PathBuf,
    ) -> StreamingProviderInput {
        StreamingProviderInput {
            working_directory: Some(self.canonical_root()),
            baseline_tree: None,
            tool_policy,
            audit_sink,
            provider_type: ProviderType::Codex,
            role,
            prompt: "Run the LC fixture provider".to_string(),
            working_dir: raw_working_dir,
            workspace_session_id: Some("ws-lc-fixture-1".to_string()),
            resume_provider_session_id: resume_id,
            permission_mode,
            structured_output_contract: None,
            env_vars: BTreeMap::new(),
            timeout_secs: 60,
        }
    }
}

/// 注入 LC 观测 marker 环境变量:进程 pwd、thread 请求原文、任何子进程执行。
/// 返回 (cwd_marker, wire_marker, spawn_marker)。
fn lc_markers(input: &mut StreamingProviderInput, dir: &Path) -> (PathBuf, PathBuf, PathBuf) {
    let cwd = dir.join("lc-cwd-marker");
    let wire = dir.join("lc-wire-marker");
    let spawn = dir.join("lc-spawn-marker");
    input
        .env_vars
        .insert("LC_CWD_MARKER".to_string(), cwd.display().to_string());
    input
        .env_vars
        .insert("LC_WIRE_MARKER".to_string(), wire.display().to_string());
    input
        .env_vars
        .insert("LC_SPAWN_MARKER".to_string(), spawn.display().to_string());
    (cwd, wire, spawn)
}

/// 读取 wire marker 中的 thread/start|resume 请求并解析 params(真实 RPC
/// params 断言的单一入口)。
fn lc_wire_params(wire_marker: &Path) -> Value {
    let line = std::fs::read_to_string(wire_marker).expect("wire marker is written");
    let request: Value = serde_json::from_str(&line)
        .unwrap_or_else(|error| panic!("wire marker is valid JSON ({line}): {error}"));
    request["params"].clone()
}
