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
fn codex_policy_direct_author_launch_uses_danger_full_access_on_request() {
    let policy_input = codex_streaming_input_with_policy(None);
    let params = codex_launch_params(&policy_input);
    // T13 Direct Author 冻结片段：danger-full-access + on-request（检测型
    // 岗位纪律替换旧 read-only 档；不与 LC 冻结序列复用）。
    assert_eq!(params["sandbox"], "danger-full-access");
    assert_eq!(params["approvalPolicy"], "on-request");
    let coder_input = codex_streaming_input_without_policy();
    let coder = codex_launch_params(&coder_input);
    assert_eq!(coder["sandbox"], "danger-full-access");
}

/// T13 冻结片段回归：策略（Author 岗位纪律）档 danger-full-access +
/// approvalPolicy=on-request（检测型纪律，替换旧 read-only canonical）；
/// Coder/Reviewer 档维持 `danger-full-access` 与既有 permission mode 映射
/// （Auto→never / Supervised→on-request，原生 approval）——档位语义按 T13 改写。
#[test]
fn codex_policy_and_coder_launch_params_t13_frozen_fragments() {
    let policy = codex_launch_params(&codex_streaming_input_with_policy(None));
    assert_eq!(
        policy,
        serde_json::json!({
            "cwd": policy["cwd"].clone(),
            "approvalPolicy": "on-request",
            "sandbox": "danger-full-access",
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

/// T13 回归锁定：Author 岗位纪律决策表（定稿）——commandExecution=Accept
/// （danger 基线下危险命令与 Claude/Pi 对齐，不用 never 自禁）、MCP=Accept、
/// fileChange=Decline（仅 fail-closed 兜底；真实处置是会话循环的检测型
/// 纪律：事件→durable→中断→本轮 Failed，不以 Decline 作强制防写）；未知
/// elicitation 返回 `-32601`+data（不静默），未知 item（任意方法名）返回
/// decline，连续 `>=3` 次未知形态终止；自然语言 reason 不作为分类依据。
#[test]
fn codex_policy_decisions_and_unknown_replies_regression_lock() {
    // Author 决策表（T13 定稿）：commandExecution/MCP=accept，
    // fileChange=decline（兜底；会话循环先行检测中断）。
    assert_eq!(
        decide_for_policy(CodexApprovalCategory::CommandExecution),
        CodexApprovalResponse::Accept
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
fn codex_policy_session_accepts_command_and_mcp_while_file_change_stays_declined() {
    // T13 定稿 Author 策略：危险命令 Accept（与 Claude/Pi 对齐，不因
    // approvalPolicy=never 自禁产生跨 provider 不对称）；MCP 自发现 Accept；
    // fileChange 仅 Decline 兜底——真实纪律是检测型中断（见 wire 检测测试），
    // Decline 不是强制防写依据。
    assert_eq!(
        decide_for_policy(CodexApprovalCategory::CommandExecution),
        CodexApprovalResponse::Accept
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
            ProviderEvent::TerminalActivity(_) => {}
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

    // C1：Author 岗位纪律会话的审批决策必须以 ToolPolicyDecision 事件可观测
    // （wire 顺序：commandExecution accept → MCP accept；fileChange 检测纪律
    // 由独立 fixture 覆盖，本 happy path 不含 fileChange）。
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
                        ("command_execution", "accept"),
                        ("mcp_tool_call", "accept"),
                    ],
                    "author policy approval decisions must surface as ToolPolicyDecision events"
                );
                // 双路径：durable sink 先写 provider_start，再落盘两条
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
                        (&"command_execution".to_string(), &"accept".to_string()),
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
                let command = durable_decision_records
                    .iter()
                    .find(|decision| decision.category == "command_execution")
                    .expect("command decision must be durable");
                assert_eq!(command.server_name, None);
                assert_eq!(
                    command.tool_name.as_deref(),
                    Some("command"),
                    "commandExecution 形态携带冻结 tool_name"
                );
                assert_eq!(command.reason_code, "policy_allows_command");
                assert!(!command.policy_digest.is_empty());
                return;
            }
            ProviderEvent::StatusChanged(_)
            | ProviderEvent::Execution(_)
            | ProviderEvent::TextDelta { .. }
            | ProviderEvent::ChoiceRequest(_)
            | ProviderEvent::ToolCall(_)
            | ProviderEvent::ToolResult(_)
            | ProviderEvent::UsageReport(_)
            // 批2 Task 7：终端活动快照——观测性事件，本消费循环不消费。
            | ProviderEvent::TerminalActivity(_) => {}
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
async fn codex_policy_resume_carries_danger_full_access_and_on_request_on_wire() {
    // I1（T13）：Author 岗位纪律 input 的 thread/resume（与 start 同源）必须在
    // 真实 wire 上携带 sandbox=danger-full-access + approvalPolicy=on-request
    //（fixture 对缺失任一字面退出；旧 read-only 三联动不复现）。
    // Task 3.3：resume 前置冻结三元组比对——预置与当前 digest/version/dialect
    // 完全一致的 provider_start 记录，决策为 Resume（保留 resume id）。

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
        sandbox: Some("danger-full-access".to_string()),
        approval_policy: Some("on-request".to_string()),
        provider_version: "codex 0.124.0-policy-fixture".to_string(),
        adapter_dialect: "codex-app-server-rpc".to_string(),
        provider_session_id: provider_session_id.to_string(),
        // direct 策略会话的存档记录:无 LC 投影(仅 LC validated 启动落盘)。
        lc_projection: None,
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
            | ProviderEvent::ToolPolicyTerminated(_)
            | ProviderEvent::TerminalActivity(_) => {}
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
            | ProviderEvent::ToolPolicyTerminated(_)
            | ProviderEvent::TerminalActivity(_) => {}
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
    // native_session_id 即 thread id；provider_start 记录冻结 T13 冻结片段
    //（danger-full-access + on-request）与 dialect。
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
    assert_eq!(record.sandbox.as_deref(), Some("danger-full-access"));
    assert_eq!(record.approval_policy.as_deref(), Some("on-request"));
    assert!(record.argv.contains(&"app-server".to_string()));
    assert!(!record.tool_policy_canonical_digest.is_empty());

    // 会话照常跑完（握手前置不破坏后台循环）。
    let completed = recv_completed(&mut session.events).await;
    assert_eq!(completed, "policy approvals done");
}

// ---- T13：Author 检测型岗位纪律（fileChange 事实 → 事件 → durable → 中断 → Failed）----

/// T13：无审批请求但出现 fileChange item 通知也必须触发检测纪律——
/// 双路径出口（事件通道 ToolPolicyWarning + durable `file_change_detected`
/// 含处置事实，次序：事件→durable→中断）后中断当前 turn，本轮 Failed、
/// 输出不可采信；🔴 不宣称回滚已发生写入或 terminal/MCP 预防。
#[tokio::test]
async fn codex_policy_file_change_item_without_approval_interrupts_turn_and_fails_run() {
    let fixture = executable_fixture(
        "tests/fixtures/provider/codex_app_server_policy_file_change_detection_fixture.sh",
    );
    let provider = policy_codex_provider(fixture);
    let sink = RecordingToolPolicyAuditSink::new();
    let input = codex_streaming_input_with_policy(Some(sink.clone().bound()));
    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .expect("author discipline session starts");

    let mut warnings = Vec::new();
    let mut saw_completed = false;
    let mut saw_whitewash_delta = false;
    loop {
        match tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
            .await
            .expect("provider should fail after fileChange detection")
            .expect("provider event channel should stay open until failure")
        {
            ProviderEvent::ToolPolicyWarning(warning) => warnings.push(warning),
            ProviderEvent::Completed(_) => saw_completed = true,
            ProviderEvent::TextDelta { content } => {
                if content.contains("whitewash") {
                    saw_whitewash_delta = true;
                }
            }
            ProviderEvent::Failed { message } => {
                assert!(
                    message.contains("author_file_change_detected"),
                    "unexpected failure message: {message}"
                );
                assert_eq!(
                    warnings,
                    vec![CodexProtocolWarningEvent {
                        reason_code: "author_file_change_detected".to_string(),
                        method: "item/started".to_string(),
                        occurrence: 1,
                    }],
                    "fileChange detection must surface exactly one in-band warning"
                );
                // 迟到 Completed 不能洗白：中断后本轮 Failed，永不 Completed，
                // 迟到 agentMessage 不被消费为输出。
                assert!(
                    !saw_completed,
                    "turn interrupted by fileChange detection must never complete"
                );
                assert!(
                    !saw_whitewash_delta,
                    "late agent message after detection must not be consumed as output"
                );
                // durable 次序：provider_start 首行 → file_change_detected（含
                // 处置事实与策略指纹；append 失败沿 kill 链的另一路径由
                // 专项测试覆盖）。
                let durable = sink.events();
                assert_eq!(
                    durable
                        .iter()
                        .position(|event| event.event_type() == "provider_start"),
                    Some(0),
                    "provider_start must be the first durable event"
                );
                let detection_index = durable
                    .iter()
                    .position(|event| event.event_type() == "file_change_detected")
                    .expect("file_change_detected must be durable");
                assert_eq!(detection_index, 1);
                let crate::cross_cutting::tool_policy_audit::DurableToolPolicyEvent::FileChangeDetected(record) =
                    &durable[detection_index]
                else {
                    unreachable!("indexed by event_type");
                };
                assert_eq!(record.item_id, "fc_detect");
                assert_eq!(record.summary, "src/main.rs (modify)");
                assert_eq!(record.source_method, "item/started");
                assert_eq!(
                    record.disposition,
                    crate::cross_cutting::tool_policy_audit::AUTHOR_FILE_CHANGE_DISPOSITION
                );
                assert!(!record.policy_digest.is_empty());
                assert!(
                    !durable
                        .iter()
                        .any(|event| event.event_type() == "approval_decision"),
                    "detection without an approval request must not fabricate decisions"
                );
                return;
            }
            ProviderEvent::StatusChanged(_)
            | ProviderEvent::Execution(_)
            | ProviderEvent::PermissionRequest(_)
            | ProviderEvent::ChoiceRequest(_)
            | ProviderEvent::ToolCall(_)
            | ProviderEvent::ToolResult(_)
            | ProviderEvent::UsageReport(_)
            | ProviderEvent::TerminalActivity(_)
            | ProviderEvent::ToolPolicyDecision(_) => {}
            other => panic!("unexpected terminal event before detection failure: {other:?}"),
        }
    }
}

/// T13：检测事件的 durable append 失败必须立即走既有 kill 链——会话终止、
/// 本轮 Failed（fail-closed，不得只记日志继续）。
#[tokio::test]
async fn codex_policy_file_change_detection_durable_failure_terminates() {
    let fixture = executable_fixture(
        "tests/fixtures/provider/codex_app_server_policy_file_change_detection_fixture.sh",
    );
    let provider = policy_codex_provider(fixture);
    // provider_start（第 1 次 append）成功；file_change_detected（第 2 次）注入失败。
    let sink = RecordingToolPolicyAuditSink::failing_after(1);
    let input = codex_streaming_input_with_policy(Some(sink.clone().bound()));
    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .expect("session starts before the detection append");

    loop {
        match tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
            .await
            .expect("provider should fail on detection append failure")
            .expect("provider event channel should stay open until failure")
        {
            ProviderEvent::Failed { message } => {
                assert!(
                    message.contains("codex policy audit append failed"),
                    "unexpected failure message: {message}"
                );
                return;
            }
            ProviderEvent::Completed(_) => {
                panic!("detection durable failure must fail the run, not complete")
            }
            ProviderEvent::StatusChanged(_)
            | ProviderEvent::Execution(_)
            | ProviderEvent::TextDelta { .. }
            | ProviderEvent::PermissionRequest(_)
            | ProviderEvent::ChoiceRequest(_)
            | ProviderEvent::ToolCall(_)
            | ProviderEvent::ToolResult(_)
            | ProviderEvent::UsageReport(_)
            | ProviderEvent::TerminalActivity(_)
            | ProviderEvent::ToolPolicyDecision(_)
            | ProviderEvent::ToolPolicyWarning(_)
            | ProviderEvent::ToolPolicyTerminated(_) => {}
            other => panic!("unexpected terminal event: {other:?}"),
        }
    }
}

/// T13：fileChange 审批请求（无前置 item 通知，danger 下通常不出现）若出现，
/// 即是 fileChange 事实——先合法应答 decline（不静默无应答；Decline 不是
/// 防写依据），随后走同一检测纪律中断 turn、本轮 Failed。
#[tokio::test]
async fn codex_policy_file_change_approval_request_without_item_notification_fails_turn() {
    let fixture = executable_fixture(
        "tests/fixtures/provider/codex_app_server_policy_file_change_request_fixture.sh",
    );
    let provider = policy_codex_provider(fixture);
    let sink = RecordingToolPolicyAuditSink::new();
    let input = codex_streaming_input_with_policy(Some(sink.clone().bound()));
    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .expect("author discipline session starts");

    let mut decisions = Vec::new();
    let mut warnings = Vec::new();
    loop {
        match tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
            .await
            .expect("provider should fail after fileChange approval detection")
            .expect("provider event channel should stay open until failure")
        {
            ProviderEvent::ToolPolicyDecision(decision) => {
                decisions.push((decision.category, decision.decision))
            }
            ProviderEvent::ToolPolicyWarning(warning) => warnings.push(warning),
            ProviderEvent::Failed { message } => {
                assert!(
                    message.contains("author_file_change_detected"),
                    "unexpected failure message: {message}"
                );
                // 请求不静默无应答：fileChange 请求先得到合法 decline 决策。
                assert_eq!(
                    decisions,
                    vec![("file_change", "decline")],
                    "fileChange approval request must be answered before the turn fails"
                );
                assert_eq!(
                    warnings,
                    vec![CodexProtocolWarningEvent {
                        reason_code: "author_file_change_detected".to_string(),
                        method: "item/fileChange/requestApproval".to_string(),
                        occurrence: 1,
                    }],
                    "fileChange approval detection must surface one in-band warning"
                );
                let durable = sink.events();
                let detection = durable
                    .iter()
                    .find(|event| event.event_type() == "file_change_detected")
                    .expect("file_change_detected must be durable");
                let crate::cross_cutting::tool_policy_audit::DurableToolPolicyEvent::FileChangeDetected(record) =
                    detection
                else {
                    unreachable!("filtered by event_type");
                };
                assert_eq!(record.item_id, "fc_req");
                assert_eq!(record.source_method, "item/fileChange/requestApproval");
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
            | ProviderEvent::TerminalActivity(_)
            | ProviderEvent::ToolPolicyTerminated(_) => {}
            other => panic!("unexpected terminal event: {other:?}"),
        }
    }
}

/// T13：旧 tp-v1（read-only canonical 时代）的存档 provider_start 与新冻结
/// 片段 digest 失配 → 显式 superseded + fresh（不清 id 静默复用旧会话）。
#[tokio::test]
async fn codex_policy_resume_from_tp_v1_read_only_digest_starts_fresh() {
    let legacy_digest = crate::cross_cutting::streaming_provider::tool_policy_digest(
        crate::cross_cutting::codex_provider::session::TOOL_POLICY_PROVIDER_NAME,
        &[
            "sandbox=read-only".to_string(),
            "approvalPolicy=on-request".to_string(),
        ],
        crate::cross_cutting::streaming_provider::TOOL_POLICY_APPROVAL_POLICY_VERSION,
    );
    // 旧 digest ≠ 新冻结片段 digest（danger-full-access 档）。
    let canonical = crate::cross_cutting::streaming_provider::canonical_tool_policy(
        crate::cross_cutting::codex_provider::session::TOOL_POLICY_PROVIDER_NAME,
        &ProviderToolPolicy::deny_file_write_builtins(),
    )
    .expect("canonical policy");
    assert_ne!(legacy_digest, canonical.digest);

    let fixture =
        executable_fixture("tests/fixtures/provider/codex_app_server_policy_approval_fixture.sh");
    let provider = policy_codex_provider(fixture);
    let sink = RecordingToolPolicyAuditSink::new();
    sink.with_stored_provider_start(
        matching_resume_record("codex-thread-resume-policy").with_digest(legacy_digest),
    );
    let mut input = codex_streaming_input_with_policy(Some(sink.clone().bound()));
    input.resume_provider_session_id = Some("codex-thread-resume-policy".to_string());
    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .expect("tp-v1 mismatch must start a fresh session");

    // 显式 fresh：native id 来自新 thread/start，非 resume id。
    assert_eq!(
        session.native_session_id.as_deref(),
        Some("codex-thread-policy")
    );
    let events = sink.events();
    assert!(
        events
            .iter()
            .any(|event| event.event_type() == "session_terminated"
                && serde_json::to_string(event)
                    .unwrap()
                    .contains("superseded_policy_drift")),
        "tp-v1 read-only record must be marked superseded before the fresh start"
    );
    let completed = recv_completed(&mut session.events).await;
    assert_eq!(completed, "policy approvals done");
}

/// T13 合同驱动 durable 分区（B2T11 裁定缺口）：无策略 run 只要携带 run-bound
/// audit sink（engine 已分配，如 Reviewer direct），即统一落 provider_start
/// 首行——digest 为空串（无策略指纹）、sandbox/approval 与实际启动参数同源；
/// 审批行为保持原生（ApprovalBridge），不注入岗位策略。
#[tokio::test]
async fn codex_contract_bound_non_policy_run_writes_provider_start_without_discipline() {
    let fixture =
        executable_fixture("tests/fixtures/provider/codex_app_server_completed_only_fixture.sh");
    let provider = policy_codex_provider(fixture);
    let sink = RecordingToolPolicyAuditSink::new();
    let mut input = streaming_input(ProviderType::Codex, ProviderPermissionMode::Auto);
    input.role = AdapterRole::Reviewer;
    // 批2 Task 20:Reviewer direct 合同 run 必须携带 ReviewerVerify 窗口
    // 合同(受信装配铸造;无合同=0 会话);provider_start 冻结 digest 仍为
    // 空串(非岗位纪律 run 无策略指纹,与实际启动参数同源口径不变)。
    input.tool_policy = Some(ProviderToolPolicy::reviewer_verify("w-t20-contract-run"));
    input.audit_sink = Some(sink.clone().bound());
    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .expect("contract-bound reviewer session starts");

    assert_eq!(
        recv_completed(&mut session.events).await,
        "Codex completed-only chunk"
    );
    let events = sink.events();
    let crate::cross_cutting::tool_policy_audit::DurableToolPolicyEvent::ProviderStart(record) =
        &events[0]
    else {
        panic!("contract-bound run must write provider_start as the first durable event");
    };
    assert_eq!(record.role, "reviewer");
    assert_eq!(record.sandbox.as_deref(), Some("danger-full-access"));
    assert_eq!(record.approval_policy.as_deref(), Some("never"));
    // 决策项②(T20b):ReviewerVerify 合同 run 的 provider_start digest=
    // 窗口域 canonical(与决策审计/LC 同源可 join);纯无策略 sink 合同
    // (policy=None)才保持空串口径。
    let expected_window_digest = crate::cross_cutting::streaming_provider::canonical_tool_policy(
        crate::cross_cutting::codex_provider::session::TOOL_POLICY_PROVIDER_NAME,
        &ProviderToolPolicy::reviewer_verify("w-t20-contract-run"),
    )
    .expect("window-domain canonical digest")
    .digest;
    assert_eq!(
        record.tool_policy_canonical_digest, expected_window_digest,
        "ReviewerVerify contract runs carry the window-domain canonical digest"
    );
    assert_eq!(
        record.provider_session_id, "codex_completed_only_thread",
        "native thread id must be audited from the pre-spawn handshake"
    );
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
            | ProviderEvent::ToolPolicyTerminated(_)
            | ProviderEvent::TerminalActivity(_) => {}
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

/// T12(REQ-LCG-04 改版):旧 danger 拒绝语义退役——合法 danger(Coding
/// +target 写面+Coder 全开)在 fresh/resume × Auto/Supervised 全组合下
/// 正常启动:真实 child、wire danger-full-access、provider_start 审计
/// sandbox/approval 与独立审批映射同源。
#[cfg(unix)]
#[tokio::test]
async fn lcg_t12_valid_danger_launches_start_for_fresh_resume_and_permission_modes() {
    let fixture = LcCodexFixture::new();
    let resume = Some("codex-thread-lc".to_string());
    let combinations = [
        (ProviderPermissionMode::Auto, "never", None),
        (ProviderPermissionMode::Auto, "never", resume.clone()),
        (ProviderPermissionMode::Supervised, "on-request", None),
        (ProviderPermissionMode::Supervised, "on-request", resume),
    ];

    for (permission_mode, expected_approval, resume_id) in combinations {
        let sink = RecordingToolPolicyAuditSink::new();
        let marker_root = tempfile::tempdir().expect("danger marker dir").keep();
        let mut raw = fixture.lc_input(
            AdapterRole::Executor,
            None,
            permission_mode.clone(),
            resume_id,
            Some(sink.clone().bound()),
            fixture.canonical_root(),
        );
        let (_cwd, wire_marker, spawn_marker) = lc_markers(&mut raw, &marker_root);
        let provider = CodexProvider::new(lc_app_server_fixture())
            .with_version_supplier(lc_version_supplier());

        // 合法 danger(target 写面齐备)必须启动成功——旧
        // codex_danger_full_access_unsupported 拒绝语义已废止。
        let mut session = provider
            .start_lc_validated(
                raw,
                &fixture.envelope(
                    SessionPolicyAction::CodingTargetWrite,
                    vec![fixture.target_worktree()],
                ),
                "cap_codex_lc_fixture",
                CancellationToken::new(),
            )
            .await
            .unwrap_or_else(|error| {
                panic!("valid danger launch must start ({permission_mode:?}): {error}")
            });
        assert_eq!(
            recv_completed(&mut session.events).await,
            "lc restricted done"
        );

        // 真实 child + wire danger-full-access + 独立审批映射。
        assert!(
            spawn_marker.exists(),
            "a valid danger launch really spawns ({permission_mode:?})"
        );
        let params = lc_wire_params(&wire_marker);
        assert_eq!(
            params["sandbox"], "danger-full-access",
            "{permission_mode:?}"
        );
        assert_eq!(
            params["approvalPolicy"], expected_approval,
            "{permission_mode:?}"
        );

        // provider_start 审计与 wire 同源。
        let events = sink.events();
        assert_eq!(events.len(), 1, "exactly one provider_start is written");
        let crate::cross_cutting::tool_policy_audit::DurableToolPolicyEvent::ProviderStart(record) =
            &events[0]
        else {
            panic!("expected provider_start");
        };
        assert_eq!(record.sandbox.as_deref(), Some("danger-full-access"));
        assert_eq!(record.approval_policy.as_deref(), Some(expected_approval));
        assert!(record.lc_projection.is_some());
    }
}

// ==== Task 9a:LC 显式 resume 的 child 前全 LC audit 比对 ====

use crate::cross_cutting::codex_provider::projection::{
    CodexPolicyProjector as T09aProjector, action_text as t09a_action_text,
    lc_boundary_plan as t09a_lc_boundary_plan, wire_dialect_text as t09a_wire_dialect_text,
};
use crate::cross_cutting::tool_policy_audit::{LcProjectionAudit, ProviderStartAudit};
use crate::product::logical_codebase::provider_projection::ProviderPolicyProjector as _;
use crate::product::logical_codebase::provider_projection::ProviderProjectionInput;

/// 以与 adapter 同源的材料(fixture envelope + 相同投影输入)预计算 LC
/// read-only 启动会得到的投影摘要,供存档记录构造。
fn t09a_expected_codex_lc_projection(
    fixture: &LcCodexFixture,
) -> crate::product::logical_codebase::provider_projection::ProviderPolicyProjection {
    let envelope = fixture.envelope(SessionPolicyAction::PlanningReadOnly, Vec::new());
    let boundary = t09a_lc_boundary_plan(&envelope);
    let projection_input = ProviderProjectionInput::new(
        envelope.clone(),
        crate::product::logical_codebase::provider_gateway::ProviderRef::codex(
            "cap_codex_lc_fixture",
        ),
        envelope.action,
        AdapterRole::Orchestrator,
        ProviderPermissionMode::Auto,
        Some(ProviderToolPolicy::deny_file_write_builtins()),
        String::new(),
        String::new(),
        envelope.config_artifact_ref.clone(),
        String::new(),
        Some(boundary),
    );
    T09aProjector::new("codex 0.124.0-lc-fixture")
        .project(&projection_input)
        .expect("expected lc projection")
}

/// Task 9a:LC 显式 resume 在 child 前比较全 LC audit(完整字面量含投影)。
/// 匹配的完整 LC 存档照常续接(thread/resume 应答确认同 id);旧 audit 缺
/// projection 或投影摘要漂移都拒绝 resume——拒绝发生在 ProcessManager::spawn
/// 之前(零 child、零新 provider_start),漂移存档追加 superseded 终止审计;
/// 绝不在 adapter 内清 resume id 后静默 fresh。记录缺失路径(依赖 native
/// 握手确认)由 streaming.rs 的 wire 测试保持,不在此重复。
#[cfg(unix)]
#[tokio::test]
async fn lcg_t09a_codex_lc_resume_audit_gate_zero_child_on_legacy_or_drift() {
    let fixture = LcCodexFixture::new();
    let expected = t09a_expected_codex_lc_projection(&fixture);
    let matching_lc_projection = LcProjectionAudit {
        action: t09a_action_text(expected.action()).to_string(),
        wire_dialect: t09a_wire_dialect_text(expected.wire_dialect()).to_string(),
        capability_projection_digest: expected.capability_projection_digest().to_string(),
        projection_digest: expected.projection_digest().to_string(),
        boundary_evidence_ref: expected.boundary_evidence_ref().to_string(),
    };
    let stored_record = |lc_projection: Option<LcProjectionAudit>| ProviderStartAudit {
        provider: "codex".to_string(),
        role: "orchestrator".to_string(),
        workspace_session_id: "ws-test".to_string(),
        provider_session_id: "codex-thread-lc".to_string(),
        tool_policy_canonical_digest: "sha256:seed".to_string(),
        argv: Vec::new(),
        sandbox: Some("danger-full-access".to_string()),
        approval_policy: Some("on-request".to_string()),
        provider_version: "codex 0.124.0-lc-fixture".to_string(),
        adapter_dialect: "codex-app-server-rpc".to_string(),
        lc_projection,
    };

    // 1)匹配的完整 LC 存档 → 续接:thread/resume 应答确认同 id。
    {
        let sink = RecordingToolPolicyAuditSink::new();
        let marker_root = tempfile::tempdir()
            .expect("t09a codex matching marker dir")
            .keep();
        let mut raw = fixture.lc_input(
            AdapterRole::Orchestrator,
            Some(ProviderToolPolicy::deny_file_write_builtins()),
            ProviderPermissionMode::Auto,
            Some("codex-thread-lc".to_string()),
            Some(sink.clone().bound()),
            fixture.target_worktree(),
        );
        let _ = lc_markers(&mut raw, &marker_root);
        let provider = CodexProvider::new(lc_app_server_fixture())
            .with_version_supplier(lc_version_supplier());
        let mut session = provider
            .start_lc_validated(
                raw,
                &fixture.envelope(SessionPolicyAction::PlanningReadOnly, Vec::new()),
                "cap_codex_lc_fixture",
                CancellationToken::new(),
            )
            .await
            .expect("matching full LC audit must resume the native thread");
        assert_eq!(
            session.native_session_id.as_deref(),
            Some("codex-thread-lc")
        );
        assert_eq!(
            recv_completed(&mut session.events).await,
            "lc restricted done"
        );
        let events = sink.events();
        assert_eq!(
            events.len(),
            1,
            "resumed run writes exactly one provider_start"
        );
        assert!(matches!(
            &events[0],
            crate::cross_cutting::tool_policy_audit::DurableToolPolicyEvent::ProviderStart(record)
                if record.provider_session_id == "codex-thread-lc"
                    && record.lc_projection.is_some()
        ));
    }

    // 2)旧 audit 缺 projection → 不能 resume LC;投影摘要漂移同理:
    // 零 child、零新 provider_start,不清 id 转 fresh。
    for (case, record) in [
        ("legacy", stored_record(None)),
        (
            "drifted",
            stored_record(Some(LcProjectionAudit {
                projection_digest: "sha256:session-projection-drifted".to_string(),
                ..matching_lc_projection.clone()
            })),
        ),
    ] {
        let sink = RecordingToolPolicyAuditSink::new();
        sink.with_stored_provider_start(record);
        let marker_root = tempfile::tempdir().expect("t09a codex marker dir").keep();
        let mut raw = fixture.lc_input(
            AdapterRole::Orchestrator,
            Some(ProviderToolPolicy::deny_file_write_builtins()),
            ProviderPermissionMode::Auto,
            Some("codex-thread-lc".to_string()),
            Some(sink.clone().bound()),
            fixture.target_worktree(),
        );
        let (_cwd, _wire, spawn_marker) = lc_markers(&mut raw, &marker_root);
        let provider = CodexProvider::new(lc_app_server_fixture())
            .with_version_supplier(lc_version_supplier());

        let rejected = match provider
            .start_lc_validated(
                raw,
                &fixture.envelope(SessionPolicyAction::PlanningReadOnly, Vec::new()),
                "cap_codex_lc_fixture",
                CancellationToken::new(),
            )
            .await
        {
            Err(error) => error,
            Ok(_) => panic!("legacy or drifted LC audit must refuse resume ({case})"),
        };
        assert!(
            rejected.details.contains("resume audit"),
            "rejection must name the resume audit gate: {rejected:?}"
        );
        // 零 child:拒绝发生在 ProcessManager::spawn 之前。
        assert!(
            !spawn_marker.exists(),
            "no codex child may spawn after a refused LC resume ({case})"
        );
        // 零新 provider_start;legacy 与漂移存档同样被标记 superseded(旧 run
        // 追加终止审计,不启动 fresh provider_start)。
        let events = sink.events();
        assert_eq!(
            events.len(),
            1,
            "{case} record is marked superseded without a fresh provider_start"
        );
        assert!(
            matches!(
                &events[0],
                crate::cross_cutting::tool_policy_audit::DurableToolPolicyEvent::SessionTerminated(terminated)
                    if terminated.reason_code == "superseded_policy_drift"
            ),
            "{case} resume must mark the old run superseded"
        );
    }
}

// ==== Task 9b:LC 原生 resume 的 native 会话确认(错/缺 id 绝不 fresh)====

/// Task 9b(Step 1 断言组 438-440 逐字):LC 显式 resume 的 native 会话
/// 确认——Codex 侧 `thread/resume` 应答必须逐字等于请求 id:
/// - 应答同 id(无存档记录,9a 的「无存档→依赖 native 握手」由本测试补
///   真实确认)→ 续接,confirmed==requested,wire 上是 thread/resume
///   携带请求 threadId;
/// - 缺 id(应答无 thread id)与错 id(应答陌生 thread id)→ 已启动
///   child 后的 runtime 失败:Err、child 被 kill/reap、错误记录「未恢复」
///   (不伪称零 spawn),不回填请求 id、不清 id 转 fresh、零新
///   provider_start。
#[cfg(unix)]
#[tokio::test]
async fn lcg_t09_codex_missing_or_wrong_native_id_never_fresh() {
    let fixture = LcCodexFixture::new();
    let requested_native_id = "codex-thread-lc".to_string();

    // 1) thread/resume 应答同 id(无存档记录):真实确认后续接。
    {
        let sink = RecordingToolPolicyAuditSink::new();
        let marker_root = tempfile::tempdir()
            .expect("t09b codex matching marker dir")
            .keep();
        let mut raw = fixture.lc_input(
            AdapterRole::Orchestrator,
            Some(ProviderToolPolicy::deny_file_write_builtins()),
            ProviderPermissionMode::Auto,
            Some(requested_native_id.clone()),
            Some(sink.clone().bound()),
            fixture.target_worktree(),
        );
        let (_cwd_marker, wire_marker, _spawn_marker) = lc_markers(&mut raw, &marker_root);
        let provider = CodexProvider::new(lc_app_server_fixture())
            .with_version_supplier(lc_version_supplier());
        let mut session = provider
            .start_lc_validated(
                raw,
                &fixture.envelope(SessionPolicyAction::PlanningReadOnly, Vec::new()),
                "cap_codex_lc_fixture",
                CancellationToken::new(),
            )
            .await
            .expect("a native-confirmed resume must continue the LC thread");
        let confirmed_native_id = session.native_session_id.clone().unwrap_or_default();
        assert_eq!(confirmed_native_id, requested_native_id);
        // wire 确为 thread/resume 携带请求 threadId(确认 id 来自 provider
        // 应答,不是回填请求 id)。
        let wire = std::fs::read_to_string(&wire_marker).expect("wire marker is written");
        assert!(
            wire.contains("\"method\":\"thread/resume\"")
                && wire.contains("\"threadId\":\"codex-thread-lc\""),
            "resume wire must carry the requested thread id: {wire}"
        );
        assert_eq!(
            recv_completed(&mut session.events).await,
            "lc restricted done"
        );
        let events = sink.events();
        assert_eq!(
            events.len(),
            1,
            "confirmed resume writes exactly one provider_start"
        );
        assert!(matches!(
            &events[0],
            crate::cross_cutting::tool_policy_audit::DurableToolPolicyEvent::ProviderStart(record)
                if record.provider_session_id == requested_native_id
        ));
    }

    // 2) 缺 id 与 3) 错 id:同为已启动 child 后的 runtime 失败(kill/reap +
    //    「未恢复」记录,绝不 fresh)。
    for mode in ["missing", "mismatched"] {
        let marker_dir = tempfile::tempdir().expect("t09b codex marker dir");
        let pid_marker = marker_dir.path().join(format!("codex-t09b-{mode}.pid"));
        let server = resume_without_thread_confirmation_fixture(&pid_marker, mode);
        let sink = RecordingToolPolicyAuditSink::new();
        let raw = fixture.lc_input(
            AdapterRole::Orchestrator,
            Some(ProviderToolPolicy::deny_file_write_builtins()),
            ProviderPermissionMode::Auto,
            Some(requested_native_id.clone()),
            Some(sink.clone().bound()),
            fixture.target_worktree(),
        );
        let provider = CodexProvider::new(server).with_version_supplier(lc_version_supplier());
        let native_id_missing_result = provider
            .start_lc_validated(
                raw,
                &fixture.envelope(SessionPolicyAction::PlanningReadOnly, Vec::new()),
                "cap_codex_lc_fixture",
                CancellationToken::new(),
            )
            .await;
        assert!(native_id_missing_result.is_err());
        let Err(rejected) = native_id_missing_result else {
            panic!("an unconfirmed thread/resume must fail the LC resume ({mode})");
        };
        assert!(
            rejected.details.contains("session NOT resumed")
                && rejected.details.contains("not a zero-spawn refusal"),
            "rejection must record the not-resumed outcome without claiming zero spawn ({mode}): {rejected:?}"
        );
        assert!(
            sink.events().is_empty(),
            "no fresh provider_start may be written for a resume the native side never confirmed ({mode})"
        );
        let started_child_was_killed_and_reaped = match wait_for_child_pid_marker(&pid_marker) {
            Some(pid) => {
                assert_child_terminated(pid, "an unconfirmed LC thread/resume handshake");
                true
            }
            None => false,
        };
        assert!(started_child_was_killed_and_reaped);
    }
}

// ==== Task 3.4(B0c):Codex 真实 prepare facts 与 full 投影/audit 同源 ====
//
// 本节 fixture 组装真实 gateway(capability/policy/target/registry/gate 全真
// 链,先例同 coding_workspace_engine::tests::provider_gateway_validated_input),
// 驱动 3.2 冻结的 `prepare_lc_streaming_candidate` 公共合同消费 Codex 真实
// adapter facts。共享 fixture 标记 `pub(super)` 供 bridging.rs 的 drift/child
// 测试复用;跨家组合守卫归 3.7,本节只断言 Codex 专属面。

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

use crate::cross_cutting::provider_adapter::{ProviderAdapter, ProviderAdapterError};
use crate::cross_cutting::provider_availability_gate::{
    ProviderAvailabilityGate, ProviderHealthSource,
};
use crate::cross_cutting::provider_health::{ProviderHealthEntry, ProviderHealthSnapshot};
use crate::cross_cutting::provider_registry::ProviderRegistry;
use crate::cross_cutting::session_launch::ValidatedStreamingProviderInput;
use crate::cross_cutting::tool_policy_audit::DurableToolPolicyEvent;
use crate::product::app_paths::ProductAppPaths;
use crate::product::logical_codebase::provider_gateway::{
    GatewaySessionDisposition, ProviderLaunchAuditContext, ResumeSessionLaunchRequest,
    SessionResumeFingerprint, canonical_target_git_identity,
};
use crate::product::logical_codebase::provider_projection::UnprovisionedProviderPolicyProjector;
use crate::product::logical_codebase::{
    AggregatePolicyArtifactStore, GatewayRunAudit, LogicalCodebaseManifest,
    LogicalCodebaseProviderGateway, PolicyTargetResolver, ProviderCapability,
    ProviderCapabilitySource, SessionLaunchRequest,
};
use crate::product::models::ProviderName;
use crate::protocol::contracts::{AdapterInput, AdapterOutput, TimeoutStatus};

/// B0c drift 负例的初始真实 CLI 观测版本(combined fixture `--version` 回显)。
pub(super) const ANCHOR_T34_VERSION_INITIAL: &str = "codex 1.0.0-t34probe";
/// B0c drift 负例:命令 identity 变化(重写 fixture)后重观测到的替换版本。
pub(super) const ANCHOR_T34_VERSION_REPLACED: &str = "codex 1.1.0-replaced";

/// anchor_t34 capability fixture:Codex 行 launch/resume/write_boundary 恒
/// Confirmed;version 与 capability 行一致(admission_verdict 的 version 门)。
struct AnchorT34CapabilitySource {
    version: String,
}

impl AnchorT34CapabilitySource {
    fn capability(
        provider: &ProviderRef,
        action: SessionPolicyAction,
        version: &str,
    ) -> ProviderCapability {
        use crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence;
        use crate::product::logical_codebase::policy::ProviderWireDialect;
        use crate::product::logical_codebase::provider_capability_store::ProviderActionCapability;
        ProviderCapability {
            provider_type: provider.provider_type,
            version: version.to_string(),
            adapter_dialect: ProviderDialect::CodexCliV1,
            wire_dialect: ProviderWireDialect::CodexAppServerRpc,
            capability_snapshot_ref: provider.capability_snapshot_ref.clone(),
            action_capability: ProviderActionCapability {
                action,
                launch: ProviderCapabilityEvidence::Confirmed,
                resume: ProviderCapabilityEvidence::Confirmed,
                write_boundary: ProviderCapabilityEvidence::Confirmed,
                projection_digest: anchor_t34_capability_profile_digest(version),
                evidence_ref: format!("probe://{action:?}"),
            },
            trust: ProviderCapabilityEvidence::Confirmed,
        }
    }
}

/// capability 行的 `projection_digest` 与真实 `CodexPolicyProjector` 的
/// capability 摘要同源计算(admission_verdict 第 3 步对齐门)。profile 摘要
/// 只覆盖 provider/dialect/wire/version/action/冻结映射/sandbox/boundary
/// 模式/MCP(空),与单次 role/cwd/target 无关——同 action+version 即同摘要。
fn anchor_t34_capability_profile_digest(version: &str) -> String {
    let fixture = LcCodexFixture::new();
    let envelope = fixture.envelope(SessionPolicyAction::PlanningReadOnly, Vec::new());
    let boundary = t09a_lc_boundary_plan(&envelope);
    let projection_input = ProviderProjectionInput::new(
        envelope.clone(),
        ProviderRef::codex("cap_anchor_t34_digest_probe"),
        envelope.action,
        AdapterRole::Orchestrator,
        ProviderPermissionMode::Auto,
        Some(ProviderToolPolicy::deny_file_write_builtins()),
        String::new(),
        String::new(),
        envelope.config_artifact_ref.clone(),
        String::new(),
        Some(boundary),
    );
    T09aProjector::new(version)
        .project(&projection_input)
        .expect("anchor t34 capability profile digest")
        .capability_projection_digest()
        .to_string()
}

impl ProviderCapabilitySource for AnchorT34CapabilitySource {
    fn require_supported(
        &self,
        provider: &ProviderRef,
        action: SessionPolicyAction,
    ) -> Result<ProviderCapability, crate::product::logical_codebase::ProviderGatewayError> {
        Ok(Self::capability(provider, action, &self.version))
    }

    fn require_resume_supported(
        &self,
        provider: &ProviderRef,
        action: SessionPolicyAction,
    ) -> Result<ProviderCapability, crate::product::logical_codebase::ProviderGatewayError> {
        Ok(Self::capability(provider, action, &self.version))
    }

    fn require_write_boundary(
        &self,
        provider: &ProviderRef,
        action: SessionPolicyAction,
    ) -> Result<ProviderCapability, crate::product::logical_codebase::ProviderGatewayError> {
        Ok(Self::capability(provider, action, &self.version))
    }

    fn require_root_recipe_supported(
        &self,
        _provider: &ProviderRef,
        _credential: &crate::product::logical_codebase::provider_admission_preflight::BootstrapPhaseCredential,
    ) -> Result<ProviderCapability, crate::product::logical_codebase::ProviderGatewayError> {
        Err(
            crate::product::logical_codebase::ProviderGatewayError::UnsupportedCapability(
                "anchor t34 fixture only serves Codex normal LC rows".to_string(),
            ),
        )
    }
}

/// pass-through target resolver:返回请求冻结的 target(先例同 engine 侧)。
struct AnchorT34PassThroughTargetResolver;

impl PolicyTargetResolver for AnchorT34PassThroughTargetResolver {
    fn resolve_and_revalidate(
        &self,
        request: &SessionLaunchRequest,
    ) -> Result<PolicyTarget, crate::product::logical_codebase::ProviderGatewayError> {
        Ok(request.target.clone())
    }
}

/// 同步 adapter stub:仅满足 gateway 构造签名,LC 流式链不触达。
struct AnchorT34StubSyncAdapter;

impl ProviderAdapter for AnchorT34StubSyncAdapter {
    fn run(&self, _input: &AdapterInput) -> Result<AdapterOutput, ProviderAdapterError> {
        Ok(AdapterOutput {
            exit_code: Some(0),
            stdout: String::new(),
            stderr: String::new(),
            structured_output: None,
            files_modified: Vec::new(),
            duration_ms: 0,
            timeout_status: TimeoutStatus::NotTimedOut,
        })
    }
}

fn anchor_t34_gate() -> Arc<ProviderAvailabilityGate> {
    struct AlwaysHealthy(Arc<ProviderHealthSnapshot>);

    impl ProviderHealthSource for AlwaysHealthy {
        fn snapshot(&self) -> Arc<ProviderHealthSnapshot> {
            self.0.clone()
        }

        fn degraded(&self) -> bool {
            false
        }
    }

    let checked_at = chrono::Utc::now();
    let snapshot = Arc::new(ProviderHealthSnapshot {
        schema_version: 1,
        generation: 1,
        checked_at,
        providers: [ProviderName::Codex]
            .into_iter()
            .map(|provider| ProviderHealthEntry {
                provider,
                command: "codex".to_string(),
                available: true,
                version: Some(ANCHOR_T34_VERSION_INITIAL.to_string()),
                reason_code: None,
                reason: None,
                checked_at,
            })
            .collect(),
    });
    Arc::new(ProviderAvailabilityGate::new(Arc::new(AlwaysHealthy(
        snapshot,
    ))))
}

/// anchor_t34 真实 gateway 装配:canonical root 与成员 target worktree 分离
/// (双 cwd 合同);registry 注册真实 `CodexProvider`(gated wrapper)。
pub(super) struct AnchorT34LcHarness {
    pub(super) gateway: LogicalCodebaseProviderGateway,
    pub(super) request: SessionLaunchRequest,
    pub(super) authority_root: PathBuf,
}

pub(super) fn anchor_t34_lc_harness(
    provider: Arc<CodexProvider>,
    capability_version: &str,
) -> AnchorT34LcHarness {
    let base = tempfile::tempdir().expect("anchor t34 lc base").keep();
    let target = base.join("member-worktree");
    std::fs::create_dir_all(&target).expect("create member worktree");
    let authority_root = std::fs::canonicalize(&base).expect("canonical lc root");
    let manifest = LogicalCodebaseManifest::new("project_anchor_t34", base.clone(), vec![]);
    let policies = AggregatePolicyArtifactStore::new(ProductAppPaths::new(base.join("app-data")));
    policies
        .ensure_bootstrap(&manifest)
        .expect("anchor t34 bootstrap policy");

    let mut registry = ProviderRegistry::new();
    registry
        .register_gated(
            ProviderName::Codex,
            provider,
            Arc::new(UnprovisionedProviderPolicyProjector),
            anchor_t34_gate(),
        )
        .expect("register anchor t34 codex adapter trio");

    let gateway = LogicalCodebaseProviderGateway::with_audit(
        policies,
        Arc::new(AnchorT34CapabilitySource {
            version: capability_version.to_string(),
        }),
        Arc::new(AnchorT34PassThroughTargetResolver),
        Arc::new(registry),
        Arc::new(AnchorT34StubSyncAdapter),
        anchor_t34_gate(),
        Arc::new(GatewayRunAudit::new()),
        authority_root.clone(),
    );

    let request = SessionLaunchRequest {
        project_id: "project_anchor_t34".to_string(),
        provider: ProviderRef::codex("cap_codex_anchor_t34"),
        action: SessionPolicyAction::PlanningReadOnly,
        target: PolicyTarget::checkout("logical_repo_0001", "checkout_0001", target),
        working_directory: authority_root.clone(),
        readable_roots: vec![base],
        writable_roots: Vec::new(),
        config_artifact_ref: "sha256:managed-config-artifact".to_string(),
    };
    AnchorT34LcHarness {
        gateway,
        request,
        authority_root,
    }
}

/// anchor_t34 的 LC streaming input:`raw_working_dir` 故意与投影 protocol
/// cwd 相反(wire 参数必须来自投影而非 raw 输入);resume id 由调用方注入。
pub(super) fn anchor_t34_lc_input(
    authority_root: &Path,
    resume_id: Option<String>,
) -> StreamingProviderInput {
    StreamingProviderInput {
        working_directory: Some(authority_root.to_path_buf()),
        baseline_tree: None,
        tool_policy: Some(ProviderToolPolicy::deny_file_write_builtins()),
        audit_sink: None,
        provider_type: ProviderType::Codex,
        role: AdapterRole::Orchestrator,
        prompt: "anchor t34 codex prepare".to_string(),
        working_dir: authority_root.join("raw-working-dir-ignored"),
        workspace_session_id: None,
        resume_provider_session_id: resume_id,
        permission_mode: ProviderPermissionMode::Auto,
        structured_output_contract: None,
        env_vars: BTreeMap::new(),
        timeout_secs: 60,
    }
}

/// anchor_t34 组合 fixture:同一命令身兼 `--version` 有界观测(回显冻结版本,
/// 真实 CLI ①口径)与 LC app-server(pid/cwd/spawn/wire marker + thread
/// 应答固定 `codex-thread-lc`)。重写本文件即命令 identity 变化 → TTL 换 key。
pub(super) fn anchor_t34_combined_app_server_fixture(version: &str) -> PathBuf {
    let body = format!(
        r#"#!/usr/bin/env bash
if [[ "${{1:-}}" == "--version" ]]; then
  echo "{version}"
  exit 0
fi
echo $$ > "$LC_PID_MARKER"
pwd -P > "$LC_CWD_MARKER"
touch "$LC_SPAWN_MARKER"
while IFS= read -r line; do
  if [[ "$line" == *'"method":"initialize"'* ]]; then
    id="$(printf '%s' "$line" | sed -n -e 's/.*"id":[[:space:]]*"\([0-9A-Za-z_-][0-9A-Za-z_-]*\)".*/\1/p' -e 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{{\"jsonrpc\":\"2.0\",\"id\":\"${{id:-aria-0}}\",\"result\":{{\"capabilities\":{{}}}}}}"
  elif [[ "$line" == *'"method":"initialized"'* ]]; then
    :
  elif [[ "$line" == *'"method":"thread/start"'* ]] || [[ "$line" == *'"method":"thread/resume"'* ]]; then
    printf '%s' "$line" > "$LC_WIRE_MARKER"
    id="$(printf '%s' "$line" | sed -n -e 's/.*"id":[[:space:]]*"\([0-9A-Za-z_-][0-9A-Za-z_-]*\)".*/\1/p' -e 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{{\"jsonrpc\":\"2.0\",\"id\":\"${{id:-aria-1}}\",\"result\":{{\"thread\":{{\"id\":\"codex-thread-lc\"}}}}}}"
  elif [[ "$line" == *'"method":"turn/start"'* ]]; then
    id="$(printf '%s' "$line" | sed -n -e 's/.*"id":[[:space:]]*"\([0-9A-Za-z_-][0-9A-Za-z_-]*\)".*/\1/p' -e 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/p')"
    echo "{{\"jsonrpc\":\"2.0\",\"id\":\"${{id:-aria-2}}\",\"result\":{{\"turn\":{{\"id\":\"turn-lc\"}}}}}}"
    echo '{{"jsonrpc":"2.0","method":"item/completed","params":{{"item":{{"id":"msg-1","type":"agentMessage","text":"lc restricted done"}}}}}}'
    echo '{{"jsonrpc":"2.0","method":"turn/completed","params":{{"turnId":"turn-lc"}}}}'
    exit 0
  fi
done
"#
    );
    let path = tempfile::tempdir()
        .expect("anchor t34 fixture dir")
        .keep()
        .join(format!("codex_anchor_t34_{version}.sh"));
    std::fs::write(&path, body).expect("write anchor t34 fixture");
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

/// anchor_t34 观测 marker 注入(与既有 LC fixture 同源,追加 pid marker)。
pub(super) fn anchor_t34_markers(
    input: &mut StreamingProviderInput,
    dir: &Path,
    label: &str,
) -> (PathBuf, PathBuf, PathBuf, PathBuf) {
    let cwd = dir.join(format!("{label}-cwd-marker"));
    let wire = dir.join(format!("{label}-wire-marker"));
    let spawn = dir.join(format!("{label}-spawn-marker"));
    let pid = dir.join(format!("{label}-pid-marker"));
    input
        .env_vars
        .insert("LC_CWD_MARKER".to_string(), cwd.display().to_string());
    input
        .env_vars
        .insert("LC_WIRE_MARKER".to_string(), wire.display().to_string());
    input
        .env_vars
        .insert("LC_SPAWN_MARKER".to_string(), spawn.display().to_string());
    input
        .env_vars
        .insert("LC_PID_MARKER".to_string(), pid.display().to_string());
    (cwd, wire, spawn, pid)
}

/// sink 事件里 provider_start 的 lc_projection 摘要(同源断言的单一入口)。
fn anchor_t34_provider_start_projection_digests(
    sink: &crate::cross_cutting::tool_policy_audit::test_support::RecordingToolPolicyAuditSink,
) -> Vec<String> {
    sink.events()
        .iter()
        .filter_map(|event| match event {
            DurableToolPolicyEvent::ProviderStart(record) => record
                .lc_projection
                .as_ref()
                .map(|lc| lc.projection_digest.clone()),
            _ => None,
        })
        .collect()
}

/// anchor_t34 共享的 kill 链观测(本文件既有 unix 助手的 `pub(super)` 桥,
/// 供 bridging.rs 的 drift/child 测试复用;逻辑零拷贝新增)。
#[cfg(unix)]
pub(super) fn wait_for_child_pid_marker_public(marker: &Path) -> Option<u32> {
    wait_for_child_pid_marker(marker)
}

/// anchor_t34 共享的 kill 链断言(同上)。
#[cfg(unix)]
pub(super) fn assert_child_terminated_public(pid: u32, what: &str) {
    assert_child_terminated(pid, what);
}

/// Task 3.4(B0c)Step 1 红测试:真实 Codex adapter 的 prepare facts 与
/// prepared full 投影、`start_validated` 审计三方同源——supplier seam 观测
/// 本家 version(①恰一次)、受限 sandbox/approval/protocol cwd 映射冻结进
/// facts/full 投影,adapter 审计字面量消费同一 prepared 投影(prepared
/// digest == audit digest);prepare 全程零 durable 写/零 child。
#[cfg(unix)]
#[tokio::test]
async fn anchor_t34_provider_prepare_and_audit_share_full_fingerprint() {
    let supplier_calls = Arc::new(AtomicUsize::new(0));
    let counted = supplier_calls.clone();
    let supplier: crate::cross_cutting::streaming_provider::ProviderVersionSupplier =
        Arc::new(move || {
            counted.fetch_add(1, AtomicOrdering::SeqCst);
            Ok("codex 0.124.0-t34".to_string())
        });
    let provider =
        Arc::new(CodexProvider::new(lc_app_server_fixture()).with_version_supplier(supplier));
    let recording =
        crate::cross_cutting::tool_policy_audit::test_support::RecordingToolPolicyAuditSink::new();
    let harness = anchor_t34_lc_harness(provider.clone(), "codex 0.124.0-t34");

    let marker_root = tempfile::tempdir().expect("t34 marker dir").keep();
    let mut input =
        anchor_t34_lc_input(&harness.authority_root, Some("codex-thread-lc".to_string()));
    let (cwd_marker, wire_marker, spawn_marker, _pid_marker) =
        anchor_t34_markers(&mut input, &marker_root, "happy");

    // durable/审计快照:prepare 必须零写、零 child。
    let events_before = recording.events().len();
    assert_eq!(events_before, 0);

    let candidate = harness
        .gateway
        .prepare_lc_streaming_candidate(
            input,
            harness.request.clone(),
            ProviderLaunchAuditContext {
                workspace_session_id: "ws-anchor-t34".to_string(),
                role_run_seq: 1,
                audit_sink: recording.clone(),
            },
            CancellationToken::new(),
        )
        .await
        .expect("real codex prepare must freeze an opaque full candidate (B0c)");

    // ① supplier 观测恰一次(TTL single-flight);facts 冻结本家真实面。
    assert_eq!(supplier_calls.load(AtomicOrdering::SeqCst), 1);
    assert_eq!(candidate.facts().exact_version(), "codex 0.124.0-t34");
    assert_eq!(
        candidate.facts().version_probe_key().provider,
        ProviderName::Codex
    );
    // 独立 approval 映射:Author 承载角色(Orchestrator+deny)恒 on-request。
    assert_eq!(candidate.facts().approval_policy(), "on-request");

    // 全相位 danger sandbox/protocol cwd:full 投影与 facts 同版本 projector
    // 产出;read-only action 协议 cwd=canonical root。
    assert_eq!(candidate.full_projection().sandbox(), "danger-full-access");
    assert_eq!(
        candidate.full_projection().protocol_working_directory(),
        harness.authority_root.as_path()
    );
    let prepared_projection_digest = candidate.full_projection().projection_digest().to_string();
    assert!(!prepared_projection_digest.is_empty());

    // prepare 零写/零 child:审计零事件、fixture 从未执行。
    assert_eq!(recording.events().len(), events_before);
    assert!(!spawn_marker.exists());

    // full-matching resume:五参指纹一致 → Resume(validated 冻结候选投影)。
    let previous = SessionResumeFingerprint::from_envelope(
        candidate.validated().envelope(),
        candidate.full_projection(),
        candidate.validated().action_evidence_digest(),
        &canonical_target_git_identity(&candidate.validated().envelope().target.worktree),
    );
    let GatewaySessionDisposition::Resume(resume_validated) = harness
        .gateway
        .resume_or_start_with_projection(
            ResumeSessionLaunchRequest {
                launch: harness.request.clone(),
                previous_fingerprint: previous,
                previous_session_id: "sess-anchor-t34".to_string(),
            },
            candidate.full_projection().clone(),
        )
        .expect("full-matching fingerprint must resume")
    else {
        panic!("expected a Resume disposition for the full-matching candidate");
    };

    // start_validated 消费 prepared projection:真实 child + LC 握手确认同 id。
    let mut session = provider
        .start_validated(
            ValidatedStreamingProviderInput::new(candidate.input().clone(), resume_validated),
            CancellationToken::new(),
        )
        .await
        .expect("prepared full projection must drive a real validated start");
    assert_eq!(
        session.native_session_id.as_deref(),
        Some("codex-thread-lc")
    );
    assert_eq!(
        recv_completed(&mut session.events).await,
        "lc restricted done"
    );

    // audit 同源:adapter provider_start 的 lc_projection 摘要与 prepared
    // full 投影摘要逐字节相等(单一来源消费,非重建)。
    let audit_projection_digest = anchor_t34_provider_start_projection_digests(&recording)
        .first()
        .cloned()
        .expect("validated start writes provider_start with lc projection");
    assert_eq!(prepared_projection_digest, audit_projection_digest);

    // wire 同源:thread/resume 携带请求 threadId;sandbox/approvalPolicy/cwd
    // 来自投影(raw working_dir 不泄漏)。
    let wire = std::fs::read_to_string(&wire_marker).expect("wire marker is written");
    assert!(
        wire.contains("\"method\":\"thread/resume\"")
            && wire.contains("\"threadId\":\"codex-thread-lc\""),
        "resume wire must carry the requested thread id: {wire}"
    );
    let params = lc_wire_params(&wire_marker);
    assert_eq!(params["sandbox"], serde_json::json!("danger-full-access"));
    assert_eq!(params["approvalPolicy"], serde_json::json!("on-request"));
    assert_eq!(
        params["cwd"],
        serde_json::json!(harness.authority_root.display().to_string())
    );
    // 双 cwd 合同:进程 cwd=canonical root(read-only 协议 cwd 同 root)。
    assert_eq!(
        std::fs::read_to_string(&cwd_marker)
            .expect("cwd marker is written")
            .trim(),
        harness.authority_root.display().to_string()
    );

    // 只有一次 provider_start(带 lc_projection);mismatch 面零计数(见
    // bridging.rs 的 anchor_t34_real_drift_zero_session_child)。
    assert_eq!(
        anchor_t34_provider_start_projection_digests(&recording).len(),
        1
    );
}
