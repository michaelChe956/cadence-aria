// 审批分类（GC6）、request-id 命名空间（GC7）与策略 launch 三联动（GC5）的
// 定向单测与 wire 级验证（restrict-role-write-tools Task 2）。
// 从 tests/mod.rs 拆出以满足 large_file_guard 的 1200 行上限。

use super::*;
use crate::cross_cutting::codex_provider::parse_approval_request;
use crate::cross_cutting::codex_provider::session::{
    decide_for_policy, decide_unknown, unknown_storm_reason_after,
};
use crate::cross_cutting::streaming_provider::{
    CodexProtocolWarningEvent, CodexSessionTerminatedEvent,
};

fn codex_streaming_input_with_policy() -> StreamingProviderInput {
    let mut input = streaming_input(ProviderType::Codex, ProviderPermissionMode::Auto);
    input.tool_policy = Some(ProviderToolPolicy::deny_file_write_builtins());
    input
}

fn codex_streaming_input_without_policy() -> StreamingProviderInput {
    streaming_input(ProviderType::Codex, ProviderPermissionMode::Supervised)
}

#[test]
fn codex_policy_start_and_resume_use_read_only_on_request() {
    let policy_input = codex_streaming_input_with_policy();
    let params = codex_launch_params(&policy_input);
    assert_eq!(params["sandbox"], "read-only");
    assert_eq!(params["approvalPolicy"], "on-request");
    let coder_input = codex_streaming_input_without_policy();
    let coder = codex_launch_params(&coder_input);
    assert_eq!(coder["sandbox"], "danger-full-access");
}

#[test]
fn codex_outbound_ids_use_aria_namespace_default_peers_keep_numeric() {
    let mut codex_out = serde_json::json!({"method":"item/commandExecution/requestApproval"});
    let next = std::sync::atomic::AtomicU64::new(0);
    let assigned = ensure_request_id(&mut codex_out, &next, OutboundIdNamespace::Aria).unwrap();
    assert_eq!(assigned, "aria-0");
    let mut default_out = serde_json::json!({"method":"session/new"});
    let next2 = std::sync::atomic::AtomicU64::new(0);
    assert_eq!(
        ensure_request_id(&mut default_out, &next2, OutboundIdNamespace::Numeric).unwrap(),
        "0"
    ); // pi/kimi 路径零变化
    let mut with_id = serde_json::json!({"id":0,"method":"mcpServer/elicitation/request"});
    let next3 = std::sync::atomic::AtomicU64::new(0);
    assert_eq!(
        ensure_request_id(&mut with_id, &next3, OutboundIdNamespace::Aria).unwrap(),
        "0"
    ); // 已带 id 原样保留（入站消息不经本函数，由读取分发保持原 id）
}

#[test]
fn codex_id_namespaces_keep_server_zero_and_client_aria_zero_distinct() {
    let mut outbound = serde_json::json!({"method":"item/commandExecution/requestApproval"});
    let next = std::sync::atomic::AtomicU64::new(0);
    let client_id = ensure_request_id(&mut outbound, &next, OutboundIdNamespace::Aria).unwrap();
    let server_request = serde_json::json!({"id":0,"method":"mcpServer/elicitation/request"});
    let server_id = server_request["id"].clone();
    assert_eq!(client_id, "aria-0");
    assert_eq!(server_id, serde_json::json!(0));
    assert_ne!(serde_json::Value::from(client_id.clone()), server_id); // 两类 id 值域隔离（GC7）
    // generic elicitation（无 _meta.codex_approval_kind）的 GC6 应答形态=-32601+data，
    // 应答 id 原样回带 server 数字 id
    let reply = serde_json::json!({
        "id": server_id.clone(),
        "error": {"code": -32601, "data": {"codex_approval_kind": null, "reason": "unsupported_approval_kind"}}
    });
    assert_eq!(reply["id"], server_request["id"]);
    assert_ne!(reply["id"].to_string(), client_id);
}

#[test]
fn codex_parser_distinguishes_mcp_from_generic_elicitation() {
    let mcp = parse_approval_request(&json!({
        "method":"mcpServer/elicitation/request", "id":0,
        "params":{"serverName":"proj_spike","_meta":{"codex_approval_kind":"mcp_tool_call","tool_params":{"text":"hi"}}}
    }))
    .unwrap();
    assert!(matches!(mcp.category, CodexApprovalCategory::McpToolCall));
    let unknown = parse_approval_request(&json!({
        "method":"mcpServer/elicitation/request", "id":1,
        "params":{"serverName":"proj_spike"}
    }));
    assert!(
        unknown.is_none()
            || matches!(
                unknown.unwrap().category,
                CodexApprovalCategory::Unknown { .. }
            )
    );
}

#[test]
fn codex_policy_session_declines_exec_and_file_change_but_accepts_mcp() {
    assert_eq!(
        decide_for_policy(CodexApprovalCategory::CommandExecution),
        CodexApprovalResponse::Decline
    );
    assert_eq!(
        decide_for_policy(CodexApprovalCategory::FileChange),
        CodexApprovalResponse::Decline
    );
    assert_eq!(
        decide_for_policy(CodexApprovalCategory::McpToolCall),
        CodexApprovalResponse::Accept
    );
}

#[test]
fn codex_unknown_approval_has_protocol_reply_and_terminates_on_third() {
    let first = decide_unknown("mcpServer/elicitation/request", 1);
    assert_eq!(
        first,
        CodexApprovalResponse::ElicitationError {
            code: -32601,
            data: serde_json::json!({"codex_approval_kind":"unknown","reason":"unsupported_approval_kind"}),
        }
    );
    assert_eq!(
        unknown_storm_reason_after(3),
        Some("unknown_approval_storm")
    );
}

#[tokio::test]
async fn codex_generic_elicitation_gets_wire_error_reply_and_storm_terminates_session() {
    let fixture = executable_fixture(
        "tests/fixtures/provider/codex_app_server_unknown_elicitation_fixture.sh",
    );
    let provider = CodexProvider::new(fixture);
    let input = streaming_input(ProviderType::Codex, ProviderPermissionMode::Supervised);
    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .unwrap();

    let mut warnings: Vec<CodexProtocolWarningEvent> = Vec::new();
    let mut terminations: Vec<CodexSessionTerminatedEvent> = Vec::new();
    loop {
        match tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
            .await
            .expect("provider should fail after unknown approval storm")
            .expect("provider event channel should stay open until failure")
        {
            ProviderEvent::ToolPolicyWarning(warning) => warnings.push(warning),
            ProviderEvent::ToolPolicyTerminated(termination) => terminations.push(termination),
            ProviderEvent::Failed { message } => {
                assert!(
                    message.contains("unknown_approval_storm"),
                    "unexpected failure message: {message}"
                );
                // C1：每次未知形态的 protocol_warning 必须经事件出口可观测
                // （occurrence 单调递增，reason_code/method 为 GC6 冻结值）。
                assert_eq!(
                    warnings,
                    vec![
                        CodexProtocolWarningEvent {
                            reason_code: "unsupported_approval_kind".to_string(),
                            method: "mcpServer/elicitation/request".to_string(),
                            occurrence: 1,
                        },
                        CodexProtocolWarningEvent {
                            reason_code: "unsupported_approval_kind".to_string(),
                            method: "mcpServer/elicitation/request".to_string(),
                            occurrence: 2,
                        },
                        CodexProtocolWarningEvent {
                            reason_code: "unsupported_approval_kind".to_string(),
                            method: "mcpServer/elicitation/request".to_string(),
                            occurrence: 3,
                        },
                    ],
                    "each unknown approval form must surface as a ToolPolicyWarning event"
                );
                // C1：第 3 次未知的 session_terminated（reason_code 冻结）可观测。
                assert_eq!(
                    terminations,
                    vec![CodexSessionTerminatedEvent {
                        reason_code: "unknown_approval_storm".to_string(),
                    }],
                    "the third consecutive unknown form must surface a ToolPolicyTerminated \
                     event before failure"
                );
                return;
            }
            ProviderEvent::StatusChanged(_)
            | ProviderEvent::Execution(_)
            | ProviderEvent::TextDelta { .. }
            | ProviderEvent::PermissionRequest(_)
            | ProviderEvent::ChoiceRequest(_)
            | ProviderEvent::ToolCall(_)
            | ProviderEvent::ToolResult(_)
            | ProviderEvent::UsageReport(_) => {}
            other => panic!("unexpected terminal event before storm failure: {other:?}"),
        }
    }
}

#[tokio::test]
async fn codex_policy_session_answers_approvals_on_wire_without_bridge() {
    let fixture =
        executable_fixture("tests/fixtures/provider/codex_app_server_policy_approval_fixture.sh");
    let provider = CodexProvider::new(fixture);
    let input = codex_streaming_input_with_policy();
    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .unwrap();

    // C1：策略会话的审批决策必须以 ToolPolicyDecision 事件可观测（wire 顺序：
    // fileChange decline → commandExecution decline → MCP accept）。
    let mut decisions = Vec::new();
    loop {
        match tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
            .await
            .expect("provider should emit completion")
            .expect("provider event channel should stay open")
        {
            ProviderEvent::PermissionRequest(_) => {
                panic!("policy session must answer approvals without bridging")
            }
            ProviderEvent::ToolPolicyDecision(decision) => {
                decisions.push((decision.category, decision.decision))
            }
            ProviderEvent::Completed(completion) => {
                assert_eq!(completion.full_output, "policy approvals done");
                assert_eq!(
                    decisions,
                    vec![
                        ("file_change", "decline"),
                        ("command_execution", "decline"),
                        ("mcp_tool_call", "accept"),
                    ],
                    "policy approval decisions must surface as ToolPolicyDecision events"
                );
                return;
            }
            ProviderEvent::StatusChanged(_)
            | ProviderEvent::Execution(_)
            | ProviderEvent::TextDelta { .. }
            | ProviderEvent::ChoiceRequest(_)
            | ProviderEvent::ToolCall(_)
            | ProviderEvent::ToolResult(_)
            | ProviderEvent::UsageReport(_) => {}
            ProviderEvent::ToolPolicyWarning(warning) => {
                panic!("classified approvals must not raise protocol warnings: {warning:?}")
            }
            ProviderEvent::ToolPolicyTerminated(termination) => {
                panic!("classified approvals must not terminate the session: {termination:?}")
            }
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

#[tokio::test]
async fn codex_coder_session_accepts_mcp_elicitation_with_execution_audit() {
    let fixture = executable_fixture(
        "tests/fixtures/provider/codex_app_server_coder_mcp_approval_fixture.sh",
    );
    let provider = CodexProvider::new(fixture);
    let input = streaming_input(ProviderType::Codex, ProviderPermissionMode::Auto);
    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .unwrap();

    let mut saw_mcp_audit = false;
    loop {
        match tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
            .await
            .expect("provider should emit completion")
            .expect("provider event channel should stay open")
        {
            ProviderEvent::PermissionRequest(_) => {
                panic!("MCP elicitation must be auto-accepted, not bridged")
            }
            ProviderEvent::Execution(event) if event.title == "MCP tool call approved" => {
                saw_mcp_audit = true;
            }
            ProviderEvent::Completed(completion) => {
                assert!(saw_mcp_audit, "MCP accept audit event was not emitted");
                assert_eq!(completion.full_output, "coder mcp approved");
                return;
            }
            ProviderEvent::StatusChanged(_)
            | ProviderEvent::Execution(_)
            | ProviderEvent::TextDelta { .. }
            | ProviderEvent::ChoiceRequest(_)
            | ProviderEvent::ToolCall(_)
            | ProviderEvent::ToolResult(_)
            | ProviderEvent::UsageReport(_)
            | ProviderEvent::ToolPolicyDecision(_)
            | ProviderEvent::ToolPolicyWarning(_)
            | ProviderEvent::ToolPolicyTerminated(_) => {}
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
