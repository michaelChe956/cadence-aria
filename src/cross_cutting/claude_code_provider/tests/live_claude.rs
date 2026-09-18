//! 真机（live）claude CLI 验证——WP1.3（吸收提案 4.2，REQ-CCI-06）。
//! 门：CLAUDE_E2E=1（CI 永不依赖真实 claude 登录态）。
use std::collections::BTreeMap;
use std::path::PathBuf;

use tokio_util::sync::CancellationToken;

use crate::cross_cutting::streaming_provider::{
    ProviderCommand, ProviderEvent, ProviderPermissionMode, ProviderSession,
    StreamingProviderAdapter, StreamingProviderInput,
};
use crate::protocol::contracts::{AdapterRole, ProviderType};

use super::ClaudeCodeProvider;

/// 真实 claude code CLI headless smoke：Auto 模式下触发 AskUserQuestion，
/// 断言完整所有权链——ChoiceRequest 到达（=control_request(can_use_tool,
/// AskUserQuestion) 已来）且发送 ChoiceResponse 前无 Completed（Auto 不自动
/// 批准、等待用户证明，REQ-CCI-04）；回 ChoiceResponse（=control_response）
/// 后由 CLI 生成原生 tool_result（aria 只消费缓存）并 Completed。
/// 事件逐条打印（EVIDENCE 前缀）供证据留档；tool_use→control_request→
/// control_response→tool_result 四段映射写入证据矩阵。
#[tokio::test]
#[ignore = "set CLAUDE_E2E=1 to run against the real claude CLI"]
async fn live_claude_ask_user_question_smoke() {
    if std::env::var("CLAUDE_E2E").ok().as_deref() != Some("1") {
        return;
    }
    const LIVE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(240);
    let provider = ClaudeCodeProvider::new(PathBuf::from("claude"));
    let input = StreamingProviderInput {
        tool_policy: None,
        audit_sink: None,
        provider_type: ProviderType::ClaudeCode,
        role: AdapterRole::Executor,
        prompt: "Use the AskUserQuestion tool to ask me which color I prefer, \
                 offering at least two options. Wait for my answer, then reply \
                 with exactly: smoke-ok"
            .to_string(),
        working_dir: std::env::temp_dir(),
        workspace_session_id: None,
        resume_provider_session_id: None,
        permission_mode: ProviderPermissionMode::Auto,
        structured_output_contract: None,
        env_vars: BTreeMap::new(),
        timeout_secs: 240,
    };

    let mut session: ProviderSession = provider
        .start(input, CancellationToken::new())
        .await
        .expect("start real claude provider");

    let choice = loop {
        match tokio::time::timeout(LIVE_TIMEOUT, session.events.recv())
            .await
            .expect("real claude should emit events within 240s")
            .expect("provider event channel should stay open")
        {
            ProviderEvent::ChoiceRequest(choice) => {
                println!(
                    "EVIDENCE choice_request id={} options={:?}",
                    choice.id, choice.options
                );
                break choice;
            }
            ProviderEvent::Completed(completion) => panic!(
                "Auto 模式下 AskUserQuestion 不得在用户回答前 Completed（REQ-CCI-04）：{}",
                completion.full_output
            ),
            ProviderEvent::Failed { message } => panic!("provider failed: {message}"),
            ProviderEvent::ProtocolError { message, .. } => {
                panic!("provider protocol error: {message}")
            }
            ProviderEvent::PermissionTimeout { permission_id } => {
                panic!("permission timed out: {permission_id}")
            }
            ProviderEvent::StatusChanged(_)
            | ProviderEvent::Execution(_)
            | ProviderEvent::TextDelta { .. }
            | ProviderEvent::PermissionRequest(_)
            | ProviderEvent::ToolCall(_)
            | ProviderEvent::ToolResult(_)
            | ProviderEvent::UsageReport(_)
            | ProviderEvent::ToolPolicyDecision(_)
            | ProviderEvent::ToolPolicyWarning(_)
            | ProviderEvent::ToolPolicyTerminated(_) => {}
        }
    };

    session
        .commands
        .send(ProviderCommand::ChoiceResponse {
            id: choice.id,
            selected_option_ids: vec![choice.options[0].id.clone()],
            free_text: None,
            answers: vec![],
        })
        .await
        .expect("send choice response");
    println!("EVIDENCE choice_response_sent");

    let completed_output = loop {
        match tokio::time::timeout(LIVE_TIMEOUT, session.events.recv())
            .await
            .expect("completion within 240s after choice response")
            .expect("provider event channel should stay open")
        {
            ProviderEvent::Completed(completion) => break completion.full_output,
            ProviderEvent::Failed { message } => panic!("provider failed after choice: {message}"),
            ProviderEvent::ProtocolError { message, .. } => {
                panic!("provider protocol error after choice: {message}")
            }
            ProviderEvent::PermissionTimeout { permission_id } => {
                panic!("permission timed out after choice: {permission_id}")
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
        }
    };
    println!("EVIDENCE completed output={completed_output}");
    assert!(
        completed_output.contains("smoke-ok"),
        "完成后输出应含约定标记 smoke-ok：{completed_output}"
    );
}
