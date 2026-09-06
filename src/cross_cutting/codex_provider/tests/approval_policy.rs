// 审批分类（GC6）、request-id 命名空间（GC7）与策略 launch 三联动（GC5）的
// 定向单测与 wire 级验证（restrict-role-write-tools Task 2）。
// 从 tests/mod.rs 拆出以满足 large_file_guard 的 1200 行上限。

use super::*;
use crate::cross_cutting::codex_provider::parse_approval_request;
use crate::cross_cutting::codex_provider::session::{
    decide_for_policy, decide_unknown, unknown_storm_reason_after,
};
use crate::cross_cutting::streaming_provider::{
    CodexProtocolWarningEvent, CodexSessionTerminatedEvent, ProviderVersionSupplier,
};
use crate::cross_cutting::tool_policy_audit::test_support::RecordingToolPolicyAuditSink;

/// Task 3.2：策略会话需要 durable sink + provider version supplier（fixture 注入，
/// controller Ruling：3.2 version 用注入字符串，3.3 接线真实 CLI 探测）。
fn policy_version_supplier() -> ProviderVersionSupplier {
    std::sync::Arc::new(|| Ok("codex 0.124.0-policy-fixture".to_string()))
}

fn policy_codex_provider(fixture: std::path::PathBuf) -> CodexProvider {
    CodexProvider::new(fixture).with_version_supplier(policy_version_supplier())
}

fn codex_streaming_input_with_policy(
    audit_sink: Option<
        std::sync::Arc<dyn crate::cross_cutting::tool_policy_audit::ToolPolicyAuditSink>,
    >,
) -> StreamingProviderInput {
    let mut input = streaming_input(ProviderType::Codex, ProviderPermissionMode::Auto);
    // 策略会话 fixture：守卫（Task 3.1）要求策略会话使用策略角色（Reviewer 侧）。
    input.role = AdapterRole::Orchestrator;
    input.tool_policy = Some(ProviderToolPolicy::deny_file_write_builtins());
    input.audit_sink = audit_sink;
    input
}

fn codex_streaming_input_without_policy() -> StreamingProviderInput {
    streaming_input(ProviderType::Codex, ProviderPermissionMode::Supervised)
}

#[test]
fn codex_policy_start_and_resume_use_read_only_on_request() {
    let policy_input = codex_streaming_input_with_policy(None);
    let params = codex_launch_params(&policy_input);
    assert_eq!(params["sandbox"], "read-only");
    assert_eq!(params["approvalPolicy"], "on-request");
    let coder_input = codex_streaming_input_without_policy();
    let coder = codex_launch_params(&coder_input);
    assert_eq!(coder["sandbox"], "danger-full-access");
}

/// F3 Task 4.1 零变化回归：策略/Coder 两档 launch 参数全键等值锁定。
/// 策略档三联动（sandbox=read-only + approvalPolicy=on-request）；Coder 档维持
/// `danger-full-access` 与既有 permission mode 映射（Auto→never /
/// Supervised→on-request）——档位不因本 change 改写。
#[test]
fn codex_policy_and_coder_launch_params_stay_frozen_zero_change() {
    let policy = codex_launch_params(&codex_streaming_input_with_policy(None));
    assert_eq!(
        policy,
        serde_json::json!({
            "cwd": policy["cwd"].clone(),
            "approvalPolicy": "on-request",
            "sandbox": "read-only",
        })
    );

    let coder_auto = codex_launch_params(&streaming_input(
        ProviderType::Codex,
        ProviderPermissionMode::Auto,
    ));
    assert_eq!(coder_auto["sandbox"], "danger-full-access");
    assert_eq!(
        coder_auto["approvalPolicy"], "never",
        "Auto→never 既有映射不变"
    );

    let coder_supervised = codex_launch_params(&streaming_input(
        ProviderType::Codex,
        ProviderPermissionMode::Supervised,
    ));
    assert_eq!(coder_supervised["sandbox"], "danger-full-access");
    assert_eq!(
        coder_supervised["approvalPolicy"], "on-request",
        "Supervised→on-request 既有映射不变"
    );
}

/// F3 Task 4.1 回归锁定：策略会话审批分类决策表（GC6 冻结）——
/// commandExecution/fileChange 拒绝、MCP accept；未知 elicitation 返回
/// `-32601`+data（不静默），未知 item（任意方法名）返回 decline，连续
/// `>=3` 次未知形态终止；自然语言 reason 不作为分类依据。
#[test]
fn codex_policy_decisions_and_unknown_replies_regression_lock() {
    // 策略决策表：commandExecution/fileChange=decline，MCP=accept。
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

    // 分类器不读自然语言 reason：带 reason 文本的 elicitation 缺
    // `_meta.codex_approval_kind` 仍是未知形态，得到协议应答而非静默。
    let reason_only = parse_approval_request(&serde_json::json!({
        "method":"mcpServer/elicitation/request", "id":7,
        "params":{"serverName":"proj","reason":"please approve this write"}
    }));
    assert!(
        reason_only.is_none()
            || matches!(
                reason_only.unwrap().category,
                CodexApprovalCategory::Unknown { .. }
            ),
        "natural-language reason must not classify an elicitation as MCP"
    );

    // 未知 elicitation：JSON-RPC error -32601 且带 data（不静默不应答）。
    let elicitation_error = decide_unknown("mcpServer/elicitation/request", 1);
    let CodexApprovalResponse::ElicitationError { code, data } = &elicitation_error else {
        panic!("unknown elicitation must receive a protocol error reply");
    };
    assert_eq!(*code, -32601);
    assert_eq!(data["reason"], "unsupported_approval_kind");
    assert_eq!(data["codex_approval_kind"], "unknown");

    // 未知 item（任意未知方法名）：拒绝应答，不静默。
    for method in [
        "item/unrecognizedForm/requestApproval",
        "totally/unknownMethod",
    ] {
        assert_eq!(
            decide_unknown(method, 1),
            CodexApprovalResponse::Decline,
            "unknown item `{method}` must get a decline reply"
        );
    }

    // 风暴阈值：第 1、2 次不终止，>=3 次终止并记录 reason_code。
    assert_eq!(unknown_storm_reason_after(1), None);
    assert_eq!(unknown_storm_reason_after(2), None);
    assert_eq!(
        unknown_storm_reason_after(3),
        Some("unknown_approval_storm")
    );
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
    // 真实 peer 计数器 1 起（JsonRpcPeer::new）：Aria 出站 seq 0 起（首出站
    // aria-0，随后 aria-1），Numeric 维持 1 起（pi/kimi 零变化）。
    let mut aria_peer_out = serde_json::json!({"method":"initialize"});
    let peer_counter = std::sync::atomic::AtomicU64::new(1);
    assert_eq!(
        ensure_request_id(&mut aria_peer_out, &peer_counter, OutboundIdNamespace::Aria).unwrap(),
        "aria-0"
    );
    let mut aria_peer_out2 = serde_json::json!({"method":"thread/start"});
    assert_eq!(
        ensure_request_id(
            &mut aria_peer_out2,
            &peer_counter,
            OutboundIdNamespace::Aria
        )
        .unwrap(),
        "aria-1"
    );
    let mut numeric_peer_out = serde_json::json!({"method":"initialize"});
    let numeric_counter = std::sync::atomic::AtomicU64::new(1);
    assert_eq!(
        ensure_request_id(
            &mut numeric_peer_out,
            &numeric_counter,
            OutboundIdNamespace::Numeric
        )
        .unwrap(),
        "1"
    );
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
    let provider = policy_codex_provider(fixture);
    let sink = RecordingToolPolicyAuditSink::new();
    let input = codex_streaming_input_with_policy(Some(sink.clone().bound()));
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
                // Task 3.2 双路径：durable sink 先写 provider_start，再落盘三条
                // approval_decision（append 失败会使 start/会话 fail-closed）。
                let durable = sink.events();
                assert_eq!(
                    durable
                        .iter()
                        .position(|event| event.event_type() == "provider_start"),
                    Some(0),
                    "provider_start must be the first durable event"
                );
                let durable_decisions: Vec<(
                    &String,
                    &String,
                )> = durable
                    .iter()
                    .filter_map(|event| match event {
                        crate::cross_cutting::tool_policy_audit::DurableToolPolicyEvent::ApprovalDecision(decision) => {
                            Some((&decision.category, &decision.decision))
                        }
                        _ => None,
                    })
                    .collect();
                assert_eq!(
                    durable_decisions,
                    vec![
                        (&"file_change".to_string(), &"decline".to_string()),
                        (&"command_execution".to_string(), &"decline".to_string()),
                        (&"mcp_tool_call".to_string(), &"accept".to_string()),
                    ],
                    "durable approval decisions must mirror the wire order"
                );
                // D7 冻结字段（P1-1）：server_name/tool_name 仅 MCP 形态携带；
                // reason_code/policy_digest 表达决策归属的策略语义。
                let durable_decision_records: Vec<&crate::cross_cutting::tool_policy_audit::ApprovalDecisionAudit> =
                    durable
                        .iter()
                        .filter_map(|event| match event {
                            crate::cross_cutting::tool_policy_audit::DurableToolPolicyEvent::ApprovalDecision(decision) => Some(decision),
                            _ => None,
                        })
                        .collect();
                let mcp = durable_decision_records
                    .iter()
                    .find(|decision| decision.category == "mcp_tool_call")
                    .expect("mcp decision must be durable");
                assert!(
                    mcp.server_name
                        .as_deref()
                        .is_some_and(|name| !name.is_empty()),
                    "mcp approval must carry server_name"
                );
                assert_eq!(mcp.reason_code, "policy_allows_mcp");
                let write_side = durable_decision_records
                    .iter()
                    .find(|decision| decision.category == "file_change")
                    .expect("file_change decision must be durable");
                assert_eq!(write_side.server_name, None);
                assert_eq!(
                    write_side.tool_name.as_deref(),
                    Some("file_change"),
                    "file_change 形态携带冻结 tool_name"
                );
                assert_eq!(write_side.reason_code, "policy_denies_write_side");
                assert!(!write_side.policy_digest.is_empty());
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
async fn codex_unknown_item_gets_decline_and_mixed_unknowns_count_toward_storm() {
    // I2：未知 `item/*/requestApproval` 必须得到 {"decision":"decline"}
    // （fixture 字面断言）；未知 item × 未知 elicitation 混合计入同一会话计数，
    // 第 3 次未知经 ToolPolicyWarning/ToolPolicyTerminated 事件出口可观测。
    let fixture =
        executable_fixture("tests/fixtures/provider/codex_app_server_unknown_item_fixture.sh");
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
            .expect("provider should fail after mixed unknown approval storm")
            .expect("provider event channel should stay open until failure")
        {
            ProviderEvent::ToolPolicyWarning(warning) => warnings.push(warning),
            ProviderEvent::ToolPolicyTerminated(termination) => terminations.push(termination),
            ProviderEvent::Failed { message } => {
                assert!(
                    message.contains("unknown_approval_storm"),
                    "unexpected failure message: {message}"
                );
                // 混合计数：未知 item 与未知 elicitation 共享同一 occurrence 序列。
                let expected_methods = [
                    "item/unrecognizedForm/requestApproval",
                    "mcpServer/elicitation/request",
                    "item/anotherNewKind/requestApproval",
                ];
                for (index, method) in expected_methods.iter().enumerate() {
                    assert_eq!(
                        warnings[index],
                        CodexProtocolWarningEvent {
                            reason_code: "unsupported_approval_kind".to_string(),
                            method: method.to_string(),
                            occurrence: index as u32 + 1,
                        },
                        "mixed unknown forms must share one occurrence sequence"
                    );
                }
                assert_eq!(warnings.len(), expected_methods.len());
                assert_eq!(
                    terminations,
                    vec![CodexSessionTerminatedEvent {
                        reason_code: "unknown_approval_storm".to_string(),
                    }],
                    "the third mixed unknown must surface ToolPolicyTerminated"
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
            | ProviderEvent::UsageReport(_)
            | ProviderEvent::ToolPolicyDecision(_) => {}
            other => panic!("unexpected terminal event before storm failure: {other:?}"),
        }
    }
}

#[tokio::test]
async fn codex_policy_resume_carries_read_only_and_on_request_on_wire() {
    // I1：策略 input 的 thread/resume（与 start 同源）必须在真实 wire 上携带
    // sandbox=read-only + approvalPolicy=on-request（fixture 对缺失任一字面退出）。
    // Task 3.3：resume 前置冻结三元组比对——预置与当前 digest/version/dialect 完全
    // 一致的 provider_start 记录，决策为 Resume（保留 resume id）。
    let fixture =
        executable_fixture("tests/fixtures/provider/codex_app_server_policy_resume_fixture.sh");
    let provider = policy_codex_provider(fixture);
    let sink = RecordingToolPolicyAuditSink::new();
    sink.with_stored_provider_start(matching_resume_record("codex-thread-resume-policy"));
    let mut input = codex_streaming_input_with_policy(Some(sink.clone().bound()));
    input.resume_provider_session_id = Some("codex-thread-resume-policy".to_string());
    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .unwrap();

    let completed = recv_completed(&mut session.events).await;

    assert_eq!(completed, "policy resume done");
    // 一致记录：不产生 superseded 终止审计。
    assert!(
        !sink
            .events()
            .iter()
            .any(|event| event.event_type() == "session_terminated"),
        "matching resume record must not be superseded"
    );
}

/// 与当前策略会话完全一致的 provider_start 存档记录（Task 3.3 fixture）。
fn matching_resume_record(
    provider_session_id: &str,
) -> crate::cross_cutting::tool_policy_audit::ProviderStartAudit {
    let policy = ProviderToolPolicy::deny_file_write_builtins();
    let canonical = crate::cross_cutting::streaming_provider::canonical_tool_policy(
        crate::cross_cutting::codex_provider::session::TOOL_POLICY_PROVIDER_NAME,
        &policy,
    )
    .expect("canonical policy");
    crate::cross_cutting::tool_policy_audit::ProviderStartAudit {
        provider: "codex".to_string(),
        workspace_session_id: "ws-test".to_string(),
        role: "author".to_string(),
        tool_policy_canonical_digest: canonical.digest,
        argv: Vec::new(),
        sandbox: Some("read-only".to_string()),
        approval_policy: Some("on-request".to_string()),
        provider_version: "codex 0.124.0-policy-fixture".to_string(),
        adapter_dialect: "codex-app-server-rpc".to_string(),
        provider_session_id: provider_session_id.to_string(),
    }
}

#[tokio::test]
async fn codex_policy_resume_drift_marks_superseded_and_starts_fresh_thread() {
    // Task 3.3：digest drift（或记录缺失）→ 拒绝 resume：追加
    // session_terminated(superseded_policy_drift) 审计、丢弃 resume id 并新建会话
    // （wire 上是 thread/start 而非 thread/resume）。
    let fixture =
        executable_fixture("tests/fixtures/provider/codex_app_server_policy_approval_fixture.sh");
    let provider = policy_codex_provider(fixture);
    let sink = RecordingToolPolicyAuditSink::new();
    // 预置 digest 不一致的记录（sha256:drift ≠ 当前 canonical digest）。
    sink.with_stored_provider_start(
        matching_resume_record("codex-thread-resume-policy").with_digest("sha256:drift"),
    );
    let mut input = codex_streaming_input_with_policy(Some(sink.clone().bound()));
    input.resume_provider_session_id = Some("codex-thread-resume-policy".to_string());
    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .unwrap();

    // drift 决策：fresh thread/start（该 fixture 走 thread/start 路径）+ native id
    // 来自新 thread。
    assert_eq!(
        session.native_session_id.as_deref(),
        Some("codex-thread-policy")
    );
    let completed = recv_completed(&mut session.events).await;
    assert_eq!(completed, "policy approvals done");

    let events = sink.events();
    let superseded = events
        .iter()
        .find(|event| event.event_type() == "session_terminated")
        .expect("drifted resume must be marked superseded");
    assert!(
        serde_json::to_string(superseded)
            .unwrap()
            .contains("superseded_policy_drift")
    );
    // superseded 终止审计先于新会话的 provider_start。
    let superseded_index = events
        .iter()
        .position(|event| event.event_type() == "session_terminated")
        .unwrap();
    let start_index = events
        .iter()
        .position(|event| event.event_type() == "provider_start")
        .unwrap();
    assert!(superseded_index < start_index);
}

#[tokio::test]
async fn codex_policy_resume_with_missing_record_starts_fresh_and_warns_in_band() {
    // GC9：resume 记录缺失（find_provider_start → None）与 drift 同路径处置——
    // 清除 resume id、以全新会话（thread/start+provider_start）启动；「标记 superseded」
    // 仅带内 ToolPolicyWarning（🔴 无旧文件可写，不得伪造无 provider_start 首行的
    // durable 文件；与 drift 有旧文件可写不同）。
    let fixture =
        executable_fixture("tests/fixtures/provider/codex_app_server_policy_approval_fixture.sh");
    let provider = policy_codex_provider(fixture);
    let sink = RecordingToolPolicyAuditSink::new();
    let mut input = codex_streaming_input_with_policy(Some(sink.clone().bound()));
    input.resume_provider_session_id = Some("codex-thread-resume-policy".to_string());
    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .expect("missing record must start a fresh session");

    // 全新会话：fresh thread/start（native id 来自新 thread，非 resume id）。
    assert_eq!(
        session.native_session_id.as_deref(),
        Some("codex-thread-policy")
    );
    // 🔴 不得伪造 durable 事件：无 session_terminated，仅新会话 provider_start。
    let events = sink.events();
    assert_eq!(events.len(), 1, "only the fresh provider_start is written");
    let crate::cross_cutting::tool_policy_audit::DurableToolPolicyEvent::ProviderStart(record) =
        &events[0]
    else {
        panic!("expected provider_start");
    };
    assert_eq!(record.provider_session_id, "codex-thread-policy");

    // 带内标记：ToolPolicyWarning(superseded_policy_record_missing)，随后会话照常完成。
    let mut warning: Option<CodexProtocolWarningEvent> = None;
    loop {
        match tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
            .await
            .expect("provider should emit completion")
            .expect("provider event channel should stay open")
        {
            ProviderEvent::ToolPolicyWarning(event) => warning = Some(event),
            ProviderEvent::Completed(completion) => {
                assert_eq!(completion.full_output, "policy approvals done");
                break;
            }
            ProviderEvent::StatusChanged(_)
            | ProviderEvent::Execution(_)
            | ProviderEvent::TextDelta { .. }
            | ProviderEvent::PermissionRequest(_)
            | ProviderEvent::ChoiceRequest(_)
            | ProviderEvent::ToolCall(_)
            | ProviderEvent::ToolResult(_)
            | ProviderEvent::UsageReport(_)
            | ProviderEvent::ToolPolicyDecision(_)
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
    let warning = warning.expect("missing record must emit an in-band superseded warning");
    assert_eq!(
        warning.reason_code, "superseded_policy_record_missing",
        "unexpected warning: {warning:?}"
    );
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

// ---- Task 3.2（REQ-ENV-09/D7）：策略会话 start 内有界握手 + provider_start ----

#[tokio::test]
async fn codex_policy_start_handshake_yields_native_thread_id_and_frozen_start_record() {
    // 策略握手（initialize→initialized→thread/start）前置到 start 内完成：
    // native_session_id 即 thread id；provider_start 记录冻结三联动原文与 dialect。
    let fixture =
        executable_fixture("tests/fixtures/provider/codex_app_server_policy_approval_fixture.sh");
    let provider = policy_codex_provider(fixture);
    let sink = RecordingToolPolicyAuditSink::new();
    let mut session = provider
        .start(
            codex_streaming_input_with_policy(Some(sink.clone().bound())),
            CancellationToken::new(),
        )
        .await
        .expect("codex policy session starts");

    assert_eq!(
        session.native_session_id.as_deref(),
        Some("codex-thread-policy"),
        "policy handshake must surface the native thread id before start returns"
    );

    let events = sink.events();
    let start_record = events.first().expect("provider_start written at start");
    let crate::cross_cutting::tool_policy_audit::DurableToolPolicyEvent::ProviderStart(record) =
        start_record
    else {
        panic!("first durable event must be provider_start");
    };
    assert_eq!(record.provider, "codex");
    assert_eq!(record.adapter_dialect, "codex-app-server-rpc");
    assert_eq!(record.provider_version, "codex 0.124.0-policy-fixture");
    assert_eq!(record.provider_session_id, "codex-thread-policy");
    assert_eq!(record.sandbox.as_deref(), Some("read-only"));
    assert_eq!(record.approval_policy.as_deref(), Some("on-request"));
    assert!(record.argv.contains(&"app-server".to_string()));
    assert!(!record.tool_policy_canonical_digest.is_empty());

    // 会话照常跑完（握手前置不破坏后台循环）。
    let completed = recv_completed(&mut session.events).await;
    assert_eq!(completed, "policy approvals done");
}

#[tokio::test]
async fn codex_policy_start_without_sink_fails_closed_before_handshake_events() {
    let fixture =
        executable_fixture("tests/fixtures/provider/codex_app_server_policy_approval_fixture.sh");
    let provider = policy_codex_provider(fixture);
    let Err(error) = provider
        .start(
            codex_streaming_input_with_policy(None),
            CancellationToken::new(),
        )
        .await
    else {
        panic!("policy session without sink must fail closed");
    };
    assert!(
        error.details.contains("audit sink is required"),
        "unexpected error: {}",
        error.details
    );
}

/// F1（最终审）：`decide_for_policy` 对 Unknown 形态必须直接 Decline——
/// 函数级单测防未来重构把 Unknown 误并入 accept 分支（fail-closed 语义）。
#[test]
fn codex_decide_for_policy_declines_unknown_category_fail_closed() {
    assert_eq!(
        decide_for_policy(CodexApprovalCategory::Unknown {
            method: "item/newForm/requestApproval".to_string(),
        }),
        CodexApprovalResponse::Decline,
        "unknown approval category must be declined by decide_for_policy, never accepted"
    );
}

// ---- F1（最终审）：codex resume 握手不得回退请求 id 冒充确认 ----

/// F1 fixture：应答 initialize 与 thread/resume，但 thread/resume 应答不确认
/// 请求的 thread id（`missing`=应答缺 id；`mismatch`=应答给出不同 id），随后
/// 保持存活等待 kill 链终止（marker 登记子进程 pid）。
#[cfg(unix)]
fn resume_without_thread_confirmation_fixture(
    marker: &std::path::Path,
    mode: &str,
) -> std::path::PathBuf {
    let resume_result = if mode == "missing" {
        "{}"
    } else {
        "{\\\"thread\\\":{\\\"id\\\":\\\"codex-other-thread\\\"}}"
    };
    let body = r#"#!/usr/bin/env bash
echo $$ > __MARKER__
while IFS= read -r line; do
  if [[ "$line" == *'"method":"initialize"'* ]]; then
    id="$(printf '%s' "$line" | sed -n -e 's/.*"id":[[:space:]]*"\([0-9A-Za-z_-][0-9A-Za-z_-]*\)".*/\1/p' -e 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"jsonrpc\":\"2.0\",\"id\":\"${id:-aria-0}\",\"result\":{\"capabilities\":{}}}"
  elif [[ "$line" == *'"method":"thread/resume"'* ]]; then
    id="$(printf '%s' "$line" | sed -n -e 's/.*"id":[[:space:]]*"\([0-9A-Za-z_-][0-9A-Za-z_-]*\)".*/\1/p' -e 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"jsonrpc\":\"2.0\",\"id\":\"${id:-aria-1}\",\"result\":__RESUME_RESULT__}"
  fi
done
"#
    .replace("__MARKER__", &marker.display().to_string())
    .replace("__RESUME_RESULT__", resume_result);
    let fixture = marker
        .parent()
        .unwrap_or_else(|| panic!("marker must have a parent dir"))
        .join(format!("codex-resume-no-confirm-{mode}.sh"));
    std::fs::write(&fixture, body).expect("write fixture");
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = std::fs::metadata(&fixture).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&fixture, permissions).expect("chmod fixture");
    fixture
}

/// kill 链断言辅助：轮询读取 fixture 登记的子进程 pid（握手应答前写入）。
#[cfg(unix)]
fn wait_for_child_pid_marker(marker: &std::path::Path) -> Option<u32> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        if let Ok(content) = std::fs::read_to_string(marker)
            && let Ok(pid) = content.trim().parse::<u32>()
        {
            return Some(pid);
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    None
}

/// kill 链断言辅助：marker 登记的子进程必须已被终止。
#[cfg(unix)]
fn assert_child_terminated(pid: u32, what: &str) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
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
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "codex child (pid {pid}) must be terminated after {what}"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

/// F1（最终审，红→绿）：thread/resume 应答缺 thread id 时，旧实现回退请求中
/// 的旧 session_id 并返回 Some——旧 id 冒充握手确认。修复后：策略会话 start
/// fail-closed（握手错误）、返回前同步终止子进程，且不落任何 provider_start
/// （未确认的 id 不得进入 durable 审计）。
#[cfg(unix)]
#[tokio::test]
async fn codex_policy_resume_response_missing_thread_id_fails_closed_and_kills_child() {
    let temp = tempfile::tempdir().expect("tempdir");
    let marker = temp.path().join("codex-resume-missing-id.pid");
    let fixture = resume_without_thread_confirmation_fixture(&marker, "missing");
    let provider = policy_codex_provider(fixture);
    let sink = RecordingToolPolicyAuditSink::new();
    sink.with_stored_provider_start(matching_resume_record("codex-thread-resume-policy"));
    let mut input = codex_streaming_input_with_policy(Some(sink.clone().bound()));
    input.resume_provider_session_id = Some("codex-thread-resume-policy".to_string());
    let Err(error) = provider.start(input, CancellationToken::new()).await else {
        panic!("thread/resume response missing thread id must fail the policy handshake");
    };
    assert!(
        error.details.contains("thread/resume") && error.details.contains("thread id"),
        "unexpected error: {}",
        error.details
    );
    assert!(
        sink.events().is_empty(),
        "no provider_start may be written for a resume id the provider never confirmed"
    );
    if let Some(pid) = wait_for_child_pid_marker(&marker) {
        assert_child_terminated(pid, "unconfirmed thread/resume handshake");
    }
}

/// F1（最终审，红→绿）：resume 应答给出了 thread id 但与请求 id 不一致——
/// 应答 id 与请求 id 关系不一致即无效，同样握手失败 fail-closed（不得采用
/// 应答中的陌生 id 绕过 resume 冻结记录比对链）。
#[cfg(unix)]
#[tokio::test]
async fn codex_policy_resume_response_mismatched_thread_id_fails_closed() {
    let temp = tempfile::tempdir().expect("tempdir");
    let marker = temp.path().join("codex-resume-mismatched-id.pid");
    let fixture = resume_without_thread_confirmation_fixture(&marker, "mismatched");
    let provider = policy_codex_provider(fixture);
    let sink = RecordingToolPolicyAuditSink::new();
    sink.with_stored_provider_start(matching_resume_record("codex-thread-resume-policy"));
    let mut input = codex_streaming_input_with_policy(Some(sink.clone().bound()));
    input.resume_provider_session_id = Some("codex-thread-resume-policy".to_string());
    let Err(error) = provider.start(input, CancellationToken::new()).await else {
        panic!("thread/resume response with a mismatched thread id must fail the handshake");
    };
    assert!(
        error.details.contains("thread/resume") && error.details.contains("thread id"),
        "unexpected error: {}",
        error.details
    );
    assert!(
        sink.events().is_empty(),
        "no durable event may be written for a mismatched resume confirmation"
    );
    if let Some(pid) = wait_for_child_pid_marker(&marker) {
        assert_child_terminated(pid, "mismatched thread/resume handshake");
    }
}

/// F1（最终审，红→绿）：非策略（Coder/legacy）resume 路径同样不得以请求 id
/// 冒充握手确认——thread/resume 应答缺 id 时会话以握手错误失败（既有 provider
/// task kill 链终止子进程），而非携旧 id 继续 turn。
#[tokio::test]
async fn codex_resume_response_missing_thread_id_fails_handshake_not_impersonating() {
    let fixture = executable_fixture(
        "tests/fixtures/provider/codex_app_server_resume_missing_thread_id_fixture.sh",
    );
    let provider = CodexProvider::new(fixture);
    let mut input = streaming_input(ProviderType::Codex, ProviderPermissionMode::Auto);
    input.resume_provider_session_id = Some("codex-thread-123".to_string());
    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .unwrap();

    loop {
        match tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
            .await
            .expect("provider should fail the unconfirmed resume")
            .expect("provider event channel should stay open until failure")
        {
            ProviderEvent::Failed { message } => {
                assert!(
                    message.contains("thread/resume") && message.contains("thread id"),
                    "unexpected failure message: {message}"
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
            | ProviderEvent::UsageReport(_)
            | ProviderEvent::ToolPolicyDecision(_)
            | ProviderEvent::ToolPolicyWarning(_)
            | ProviderEvent::ToolPolicyTerminated(_) => {}
            ProviderEvent::Completed(completion) => {
                let full_output = completion.full_output;
                panic!("unconfirmed resume must not complete: {full_output}")
            }
            ProviderEvent::ProtocolError { message, .. } => {
                panic!("provider protocol error: {message}")
            }
            ProviderEvent::PermissionTimeout { permission_id } => {
                panic!("provider permission timed out: {permission_id}")
            }
        }
    }
}

#[cfg(unix)]
#[tokio::test]
async fn codex_policy_start_append_failure_kills_child_and_fails_closed() {
    // 注入 provider_start append 失败：adapter 必须在返回前终止子进程（kill 链）。
    let temp = tempfile::tempdir().expect("tempdir");
    let marker = temp.path().join("codex-policy.pid");
    // 握手必须成功（initialize/thread/start 有应答），provider_start append 才会
    // 触发注入的失败；此后进程挂起等待 kill 链终止。
    let body = r#"#!/usr/bin/env bash
echo $$ > __MARKER__
while IFS= read -r line; do
  if [[ "$line" == *'"method":"initialize"'* ]]; then
    id="$(printf '%s' "$line" | sed -n -e 's/.*"id":[[:space:]]*"\([0-9A-Za-z_-][0-9A-Za-z_-]*\)".*/\1/p' -e 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"jsonrpc\":\"2.0\",\"id\":\"${id:-aria-0}\",\"result\":{\"capabilities\":{}}}"
  elif [[ "$line" == *'"method":"thread/start"'* ]]; then
    id="$(printf '%s' "$line" | sed -n -e 's/.*"id":[[:space:]]*"\([0-9A-Za-z_-][0-9A-Za-z_-]*\)".*/\1/p' -e 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{\"jsonrpc\":\"2.0\",\"id\":\"${id:-aria-1}\",\"result\":{\"thread\":{\"id\":\"thread-append-failure\"}}}"
  fi
done
"#
    .replace("__MARKER__", &marker.display().to_string());
    let fixture = temp.path().join("codex-policy-append-failure.sh");
    std::fs::write(&fixture, body).expect("write fixture");
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = std::fs::metadata(&fixture).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&fixture, permissions).expect("chmod fixture");

    let provider = policy_codex_provider(fixture);
    let sink = RecordingToolPolicyAuditSink::failing_after(0);
    let Err(error) = provider
        .start(
            codex_streaming_input_with_policy(Some(sink.clone().bound())),
            CancellationToken::new(),
        )
        .await
    else {
        panic!("provider_start append failure must fail the session");
    };
    assert!(
        error.details.contains("provider_start audit append failed"),
        "unexpected error: {}",
        error.details
    );

    // kill 链：子进程在返回前被终止（marker 已写则等待退出；未写则登记前已死）。
    let deadline = std::time::Duration::from_secs(5);
    let start = std::time::Instant::now();
    let mut pid: Option<u32> = None;
    while start.elapsed() < deadline {
        if let Ok(content) = std::fs::read_to_string(&marker)
            && let Ok(value) = content.trim().parse::<u32>()
        {
            pid = Some(value);
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    if let Some(pid) = pid {
        let start = std::time::Instant::now();
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
                break;
            }
            assert!(
                start.elapsed() < deadline,
                "codex policy child (pid {pid}) must be terminated after append failure"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
}
