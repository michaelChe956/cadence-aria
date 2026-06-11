use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::protocol::contracts::{AdapterInput, AdapterRole, ProviderType};

#[derive(Debug, Clone)]
pub enum StreamChunk {
    Text(String),
    Done { full_output: String },
    Error(String),
}

pub struct StreamingRunHandle {
    pub receiver: mpsc::Receiver<StreamChunk>,
    pub cancel: CancellationToken,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderPermissionMode {
    Auto,
    Supervised,
}

#[derive(Debug, Clone)]
pub struct StreamingProviderInput {
    pub provider_type: ProviderType,
    pub role: AdapterRole,
    pub prompt: String,
    pub working_dir: PathBuf,
    /// 产品/工作区 session ID，用于日志追踪和关联，不用于 provider 续接。
    pub workspace_session_id: Option<String>,
    /// Provider 原生 session ID，用于续接 Claude Code / Codex 会话。
    pub resume_provider_session_id: Option<String>,
    pub permission_mode: ProviderPermissionMode,
    pub env_vars: BTreeMap<String, String>,
    pub timeout_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RiskLevel {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionRequestData {
    pub id: String,
    pub tool_name: String,
    pub description: String,
    pub risk_level: RiskLevel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChoiceOptionData {
    pub id: String,
    pub label: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChoiceRequestData {
    pub id: String,
    pub prompt: String,
    pub options: Vec<ChoiceOptionData>,
    pub allow_multiple: bool,
    pub allow_free_text: bool,
    pub source: ChoiceRequestSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChoiceRequestSource {
    AskUserQuestion,
    RequestUserInput,
    TextFallback,
    ProviderChoice,
}

impl ChoiceRequestSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AskUserQuestion => "ask_user_question",
            Self::RequestUserInput => "request_user_input",
            Self::TextFallback => "text_fallback",
            Self::ProviderChoice => "provider_choice",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderStatus {
    Starting,
    Running,
    WaitingApproval,
    Completed,
    Failed,
    Aborted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderExecutionEventKind {
    Provider,
    Turn,
    Command,
    Output,
    Artifact,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderExecutionEventStatus {
    Started,
    Running,
    WaitingApproval,
    Completed,
    Failed,
    Aborted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderExecutionEvent {
    pub event_id: String,
    pub kind: ProviderExecutionEventKind,
    pub status: ProviderExecutionEventStatus,
    pub title: String,
    pub detail: Option<String>,
    pub command: Option<String>,
    pub cwd: Option<String>,
    pub output: Option<String>,
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderToolCall {
    pub id: String,
    pub tool_name: String,
    pub input: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderToolResult {
    pub tool_use_id: String,
    pub output: String,
    pub is_error: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderEvent {
    TextDelta {
        content: String,
    },
    PermissionRequest(PermissionRequestData),
    ChoiceRequest(ChoiceRequestData),
    StatusChanged(ProviderStatus),
    Execution(ProviderExecutionEvent),
    ToolCall(ProviderToolCall),
    ToolResult(ProviderToolResult),
    Completed {
        full_output: String,
        provider_session_id: Option<String>,
    },
    Failed {
        message: String,
    },
    ProtocolError {
        code: String,
        message: String,
        context: Option<serde_json::Value>,
    },
    PermissionTimeout {
        permission_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderCommand {
    PermissionResponse {
        id: String,
        approved: bool,
        reason: Option<String>,
    },
    ChoiceResponse {
        id: String,
        selected_option_ids: Vec<String>,
        free_text: Option<String>,
    },
    ToolResult(ProviderToolResult),
    Abort,
}

pub struct ProviderSession {
    pub events: mpsc::Receiver<ProviderEvent>,
    pub commands: mpsc::Sender<ProviderCommand>,
}

const FAKE_STREAMING_STEP_DELAY: Duration = Duration::from_millis(10);

async fn fake_streaming_should_stop(
    cancel: &CancellationToken,
    command_rx: &mut mpsc::Receiver<ProviderCommand>,
    commands_open: &mut bool,
) -> bool {
    let delay = tokio::time::sleep(FAKE_STREAMING_STEP_DELAY);
    tokio::pin!(delay);

    loop {
        if *commands_open {
            tokio::select! {
                _ = cancel.cancelled() => return true,
                command = command_rx.recv() => {
                    match command {
                        Some(ProviderCommand::Abort) => return true,
                        Some(ProviderCommand::PermissionResponse { .. })
                        | Some(ProviderCommand::ChoiceResponse { .. })
                        | Some(ProviderCommand::ToolResult(_)) => {}
                        None => *commands_open = false,
                    }
                }
                _ = &mut delay => return false,
            }
        } else {
            tokio::select! {
                _ = cancel.cancelled() => return true,
                _ = &mut delay => return false,
            }
        }
    }
}

async fn fake_streaming_send_event(
    event_tx: &mpsc::Sender<ProviderEvent>,
    event: ProviderEvent,
    cancel: &CancellationToken,
    command_rx: &mut mpsc::Receiver<ProviderCommand>,
    commands_open: &mut bool,
) -> bool {
    loop {
        if *commands_open {
            tokio::select! {
                _ = cancel.cancelled() => return false,
                permit = event_tx.reserve() => {
                    match permit {
                        Ok(permit) => {
                            permit.send(event);
                            return true;
                        }
                        Err(_) => return false,
                    }
                }
                command = command_rx.recv() => {
                    match command {
                        Some(ProviderCommand::Abort) => return false,
                        Some(ProviderCommand::PermissionResponse { .. })
                        | Some(ProviderCommand::ChoiceResponse { .. })
                        | Some(ProviderCommand::ToolResult(_)) => {}
                        None => *commands_open = false,
                    }
                }
            }
        } else {
            tokio::select! {
                _ = cancel.cancelled() => return false,
                permit = event_tx.reserve() => {
                    match permit {
                        Ok(permit) => {
                            permit.send(event);
                            return true;
                        }
                        Err(_) => return false,
                    }
                }
            }
        }
    }
}

#[async_trait::async_trait]
pub trait StreamingProviderAdapter: Send + Sync {
    fn supports_tool_calls(&self) -> bool {
        false
    }

    fn supports_provider_driven_testing(&self) -> bool {
        false
    }

    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        Err(ProviderAdapterError::execution_failed(
            None,
            String::new(),
            "streaming provider start is not implemented",
            0,
        ))
    }

    async fn run_streaming(
        &self,
        input: &AdapterInput,
        cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        let working_dir = input.worktree_path.as_ref().map(PathBuf::from).unwrap_or(
            std::env::current_dir().map_err(|error| {
                ProviderAdapterError::execution_failed(None, String::new(), error.to_string(), 0)
            })?,
        );
        let provider_input = StreamingProviderInput {
            provider_type: input.provider_type.clone(),
            role: input.role.clone(),
            prompt: input.prompt.clone(),
            working_dir,
            workspace_session_id: None,
            resume_provider_session_id: None,
            permission_mode: ProviderPermissionMode::Auto,
            env_vars: BTreeMap::new(),
            timeout_secs: input.timeout,
        };
        let bridge_cancel = cancel.clone();
        let mut session = self.start(provider_input, cancel).await?;
        let (tx, rx) = mpsc::channel(32);

        tokio::spawn(async move {
            let _commands = session.commands;
            loop {
                let event = tokio::select! {
                    _ = bridge_cancel.cancelled() => return,
                    event = session.events.recv() => {
                        match event {
                            Some(event) => event,
                            None => return,
                        }
                    }
                };
                let chunk = match event {
                    ProviderEvent::TextDelta { content } => StreamChunk::Text(content),
                    ProviderEvent::Completed { full_output, .. } => {
                        StreamChunk::Done { full_output }
                    }
                    ProviderEvent::Failed { message } => StreamChunk::Error(message),
                    ProviderEvent::ProtocolError { message, .. } => StreamChunk::Error(message),
                    ProviderEvent::PermissionTimeout { permission_id } => {
                        StreamChunk::Error(format!("Permission request {permission_id} timed out"))
                    }
                    ProviderEvent::PermissionRequest(_)
                    | ProviderEvent::ChoiceRequest(_)
                    | ProviderEvent::StatusChanged(_)
                    | ProviderEvent::Execution(_)
                    | ProviderEvent::ToolCall(_)
                    | ProviderEvent::ToolResult(_) => {
                        continue;
                    }
                };
                tokio::select! {
                    _ = bridge_cancel.cancelled() => return,
                    send_result = tx.send(chunk) => {
                        if send_result.is_err() {
                            return;
                        }
                    }
                }
            }
        });

        Ok(rx)
    }
}

pub struct FakeStreamingProvider;

#[async_trait::async_trait]
impl StreamingProviderAdapter for FakeStreamingProvider {
    fn supports_provider_driven_testing(&self) -> bool {
        true
    }

    async fn start(
        &self,
        input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (event_tx, event_rx) = mpsc::channel(32);
        let (command_tx, mut command_rx) = mpsc::channel(8);
        let output = fake_workspace_markdown(&input.prompt);

        tokio::spawn(async move {
            let chunks = fake_stream_chunks(&output);
            let mut commands_open = true;

            for content in chunks {
                if fake_streaming_should_stop(&cancel, &mut command_rx, &mut commands_open).await {
                    return;
                }

                if !fake_streaming_send_event(
                    &event_tx,
                    ProviderEvent::TextDelta { content },
                    &cancel,
                    &mut command_rx,
                    &mut commands_open,
                )
                .await
                {
                    return;
                }
            }

            if fake_streaming_should_stop(&cancel, &mut command_rx, &mut commands_open).await {
                return;
            }
            let _ = fake_streaming_send_event(
                &event_tx,
                ProviderEvent::Completed {
                    full_output: output,
                    provider_session_id: None,
                },
                &cancel,
                &mut command_rx,
                &mut commands_open,
            )
            .await;
        });

        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }

    async fn run_streaming(
        &self,
        input: &AdapterInput,
        cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        let working_dir = input.worktree_path.as_ref().map(PathBuf::from).unwrap_or(
            std::env::current_dir().map_err(|error| {
                ProviderAdapterError::execution_failed(None, String::new(), error.to_string(), 0)
            })?,
        );
        let provider_input = StreamingProviderInput {
            provider_type: input.provider_type.clone(),
            role: input.role.clone(),
            prompt: input.prompt.clone(),
            working_dir,
            workspace_session_id: None,
            resume_provider_session_id: None,
            permission_mode: ProviderPermissionMode::Auto,
            env_vars: BTreeMap::new(),
            timeout_secs: input.timeout,
        };
        let bridge_cancel = cancel.clone();
        let mut session = self.start(provider_input, cancel).await?;
        let (tx, rx) = mpsc::channel(32);

        tokio::spawn(async move {
            let _commands = session.commands;
            loop {
                let event = tokio::select! {
                    _ = bridge_cancel.cancelled() => return,
                    event = session.events.recv() => {
                        match event {
                            Some(event) => event,
                            None => return,
                        }
                    }
                };
                let chunk = match event {
                    ProviderEvent::TextDelta { content } => StreamChunk::Text(content),
                    ProviderEvent::Completed { full_output, .. } => {
                        StreamChunk::Done { full_output }
                    }
                    ProviderEvent::Failed { message } => StreamChunk::Error(message),
                    ProviderEvent::ProtocolError { message, .. } => StreamChunk::Error(message),
                    ProviderEvent::PermissionTimeout { permission_id } => {
                        StreamChunk::Error(format!("Permission request {permission_id} timed out"))
                    }
                    ProviderEvent::PermissionRequest(_)
                    | ProviderEvent::ChoiceRequest(_)
                    | ProviderEvent::StatusChanged(_)
                    | ProviderEvent::Execution(_)
                    | ProviderEvent::ToolCall(_)
                    | ProviderEvent::ToolResult(_) => {
                        continue;
                    }
                };
                tokio::select! {
                    _ = bridge_cancel.cancelled() => return,
                    send_result = tx.send(chunk) => {
                        if send_result.is_err() {
                            return;
                        }
                    }
                }
            }
        });

        Ok(rx)
    }
}

fn fake_workspace_markdown(prompt: &str) -> String {
    if prompt.contains("Tester Provider Runtime") && prompt.contains("Phase: plan_tests") {
        return serde_json::json!({
            "summary": "fake provider smoke test plan",
            "steps": [{
                "id": "fake_smoke",
                "title": "Fake provider smoke",
                "intent": "prove fake provider can satisfy provider-driven testing",
                "required": true,
                "tool": "provider_managed",
                "risk_level": "low",
                "command_or_tool_input": {},
                "evidence_expectation": "fake provider emits deterministic step evidence"
            }]
        })
        .to_string();
    }
    if prompt.contains("Tester Provider Runtime") && prompt.contains("Phase: execute_test_plan") {
        return serde_json::json!({
            "step_results": [{
                "step_id": "fake_smoke",
                "status": "passed",
                "evidence_refs": ["fake-provider-smoke.log"],
                "provider_analysis": "fake provider deterministic testing passed"
            }]
        })
        .to_string();
    }

    let issue = extract_prompt_field(prompt, "Issue")
        .or_else(|| extract_prompt_field(prompt, "Issue 描述"))
        .unwrap_or_else(|| "当前 Issue".to_string());
    let user_intent = latest_user_message(prompt)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "开始生成".to_string());

    if prompt.contains("Workspace 类型: Design Spec") {
        return format!(
            "# Design Spec\n\n\
             ## 设计范围\n\n\
             面向 {issue} 生成候选设计，响应用户指令：{user_intent}。\n\n\
             ## 设计决策\n\n\
             [DEC-001] 采用最小可验证实现，保持实现与测试边界清晰。\n\n\
             ## 公共组件\n\n\
             [CMP-001] 在现有代码结构中增加必要模块，不引入无关依赖。\n\n\
             ## API 契约\n\n\
             [API-001] 复用现有 Workspace 工作流入口，不新增外部 API。\n\n\
             ## 数据模型\n\n\
             [DATA-001] 不新增持久化实体；状态变更沿用现有生命周期记录。\n\n\
             ## 风险\n\n\
             [RISK-001] 需求边界不完整时，在待确认项中保留人工确认入口。\n\n\
             ## 追踪关系\n\n\
             - 覆盖关联 Story Spec 与 Issue 约束。"
        );
    }

    if prompt.contains("Workspace 类型: Work Item") {
        return format!(
            "# Work Item\n\n\
             ## 目标\n\n\
             为 {issue} 拆分可执行任务，响应用户指令：{user_intent}。\n\n\
             ## 范围\n\n\
             覆盖实现、测试与验证命令。\n\n\
             ## 任务拆分\n\n\
             [TASK-001] 实现核心逻辑。\n\
             [TASK-002] 补充自动化测试。\n\n\
             ## 依赖\n\n\
             依赖已确认 Story Spec 与 Design Spec。\n\n\
             ## 验证命令\n\n\
             - 运行项目现有测试命令。\n\n\
             ## 风险\n\n\
             输入约束变化时需重新确认计划。\n\n\
             ## 追踪关系\n\n\
             - 绑定来源 Story/Design。"
        );
    }

    format!(
        "# Story Spec\n\n\
         ## 范围\n\n\
         覆盖 {issue} 的候选 Story Spec，响应用户指令：{user_intent}。\n\n\
         ## 用户故事\n\n\
         作为使用者，我希望系统能清晰解决该问题并提供可运行验证。\n\n\
         ## 功能需求\n\n\
         [REQ-001] 程序必须计算爬到第 n 步的走法数量，每次可走 1 或 2 步。\n\
         [REQ-002] 实现必须保持 O(n) 时间复杂度，并包含自动化测试用例。\n\n\
         ## 成功标准\n\n\
         [AC-001] n=1、n=2、n=3 等基础输入返回正确走法数量。\n\
         [AC-002] 测试覆盖边界输入和常规输入。\n\n\
         ## 待确认项\n\n\
         无。\n\n\
         ## 非功能需求\n\n\
         [NFR-001] 代码应保持可读、无额外运行时依赖。\n\n\
         ## 输入摘要\n\n\
         {user_intent}"
    )
}

fn extract_prompt_field(prompt: &str, field: &str) -> Option<String> {
    let prefix = format!("{field}:");
    prompt
        .lines()
        .find_map(|line| line.trim().strip_prefix(&prefix).map(str::trim))
        .filter(|value| !value.is_empty())
        .map(|value| value.split(" (").next().unwrap_or(value).trim().to_string())
}

fn latest_user_message(prompt: &str) -> Option<String> {
    prompt
        .lines()
        .rev()
        .find_map(|line| line.trim().strip_prefix("[user]:").map(str::trim))
        .map(ToString::to_string)
}

fn fake_stream_chunks(output: &str) -> Vec<String> {
    const MAX_PARTS_PER_CHUNK: usize = 16;
    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut part_count = 0usize;
    let mut current_is_whitespace = None;

    for ch in output.chars() {
        let is_whitespace = ch.is_whitespace();
        if current_is_whitespace.is_some_and(|previous| previous != is_whitespace)
            && !current.is_empty()
        {
            part_count += 1;
            if part_count >= MAX_PARTS_PER_CHUNK && !is_whitespace {
                chunks.push(std::mem::take(&mut current));
                part_count = 0;
            }
        }
        current_is_whitespace = Some(is_whitespace);
        current.push(ch);
    }

    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::contracts::AdapterInput;
    use serde_json::json;

    const TEST_TIMEOUT: Duration = Duration::from_secs(1);

    fn make_input(prompt: &str) -> AdapterInput {
        AdapterInput {
            prompt: prompt.to_string(),
            provider_type: crate::protocol::contracts::ProviderType::Fake,
            role: crate::protocol::contracts::AdapterRole::Orchestrator,
            worktree_path: None,
            context_files: Vec::new(),
            output_schema: String::new(),
            timeout: 60,
            max_retries: 0,
        }
    }

    fn make_provider_input(prompt: &str) -> StreamingProviderInput {
        StreamingProviderInput {
            provider_type: crate::protocol::contracts::ProviderType::Fake,
            role: crate::protocol::contracts::AdapterRole::Orchestrator,
            prompt: prompt.to_string(),
            working_dir: std::env::current_dir().unwrap(),
            workspace_session_id: None,
            resume_provider_session_id: None,
            permission_mode: ProviderPermissionMode::Auto,
            env_vars: std::collections::BTreeMap::new(),
            timeout_secs: 60,
        }
    }

    fn prompt_with_word_count(word_count: usize) -> String {
        (0..word_count)
            .map(|index| format!("word{index}"))
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn streaming_provider_input_distinguishes_workspace_and_resume_sessions() {
        let input = StreamingProviderInput {
            provider_type: crate::protocol::contracts::ProviderType::Fake,
            role: crate::protocol::contracts::AdapterRole::Orchestrator,
            prompt: "prompt".to_string(),
            working_dir: std::env::current_dir().unwrap(),
            workspace_session_id: Some("workspace_session_0001".to_string()),
            resume_provider_session_id: Some("provider_session_0001".to_string()),
            permission_mode: ProviderPermissionMode::Auto,
            env_vars: std::collections::BTreeMap::new(),
            timeout_secs: 60,
        };

        assert_eq!(
            input.workspace_session_id.as_deref(),
            Some("workspace_session_0001")
        );
        assert_eq!(
            input.resume_provider_session_id.as_deref(),
            Some("provider_session_0001")
        );
    }

    #[test]
    fn provider_tool_call_and_result_have_stable_json_shape() {
        let call = ProviderToolCall {
            id: "tool_call_0001".to_string(),
            tool_name: "run_command".to_string(),
            input: json!({"command": ["cargo", "test"]}),
        };
        let result = ProviderToolResult {
            tool_use_id: "tool_call_0001".to_string(),
            output: "{\"status\":\"passed\"}".to_string(),
            is_error: false,
        };

        assert_eq!(
            serde_json::to_value(&call).expect("serialize tool call"),
            json!({
                "id": "tool_call_0001",
                "tool_name": "run_command",
                "input": {"command": ["cargo", "test"]}
            })
        );
        assert_eq!(
            serde_json::from_value::<ProviderToolCall>(
                serde_json::to_value(&call).expect("serialize tool call")
            )
            .expect("deserialize tool call"),
            call
        );
        assert_eq!(
            serde_json::to_value(&result).expect("serialize tool result"),
            json!({
                "tool_use_id": "tool_call_0001",
                "output": "{\"status\":\"passed\"}",
                "is_error": false
            })
        );
        assert_eq!(
            serde_json::from_value::<ProviderToolResult>(
                serde_json::to_value(&result).expect("serialize tool result")
            )
            .expect("deserialize tool result"),
            result
        );
    }

    async fn wait_for_buffer_len<T>(rx: &mpsc::Receiver<T>, expected_len: usize) {
        for _ in 0..200 {
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

    async fn wait_for_receiver_closed<T>(rx: &mpsc::Receiver<T>) {
        for _ in 0..200 {
            if rx.is_closed() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("receiver was not closed after cancellation");
    }

    #[tokio::test]
    async fn fake_streaming_provider_emits_chunks_then_done() {
        let provider = FakeStreamingProvider;
        let cancel = CancellationToken::new();
        let input = make_input("Workspace 类型: Story Spec\nIssue: 爬楼梯问题\n[user]: 开始生成");

        let mut rx = provider.run_streaming(&input, cancel).await.unwrap();

        let mut output = String::new();
        let mut done_output = None;

        while let Some(chunk) = rx.recv().await {
            match chunk {
                StreamChunk::Text(t) => output.push_str(&t),
                StreamChunk::Done { full_output } => {
                    done_output = Some(full_output);
                    break;
                }
                StreamChunk::Error(_) => panic!("unexpected error"),
            }
        }

        let done_output = done_output.unwrap();
        assert_eq!(output, done_output);
        assert!(done_output.contains("## 范围"));
        assert!(done_output.contains("## 用户故事"));
        assert!(done_output.contains("## 功能需求"));
        assert!(done_output.contains("[REQ-001]"));
        assert!(done_output.contains("## 成功标准"));
        assert!(done_output.contains("[AC-001]"));
        assert!(done_output.contains("## 待确认项"));
        assert!(done_output.contains("## 非功能需求"));
        assert!(
            !done_output.contains("[system]"),
            "fake provider should generate a candidate artifact instead of echoing full prompt"
        );
    }

    #[tokio::test]
    async fn fake_streaming_provider_session_emits_text_and_completed() {
        let provider = FakeStreamingProvider;
        let cancel = CancellationToken::new();
        let input = make_provider_input(
            "[system]\nWorkspace 类型: Story Spec\nIssue: 爬楼梯问题\n[user]: 开始生成",
        );

        let mut session = provider.start(input, cancel).await.unwrap();
        let mut output = String::new();
        while let Some(event) = session.events.recv().await {
            match event {
                ProviderEvent::TextDelta { content } => output.push_str(&content),
                ProviderEvent::Completed { full_output, .. } => {
                    assert_eq!(full_output, output);
                    break;
                }
                other => panic!("unexpected provider event: {other:?}"),
            }
        }
        assert!(output.contains("## 范围"));
        assert!(output.contains("[REQ-001]"));
        assert!(output.contains("[AC-001]"));
        assert!(!output.contains("[system]"));
    }

    #[tokio::test]
    async fn fake_streaming_provider_abort_after_final_text_suppresses_completed() {
        let provider = FakeStreamingProvider;
        let cancel = CancellationToken::new();
        let input = make_provider_input("Issue: final");

        let mut session = provider.start(input, cancel).await.unwrap();
        let first = session.events.recv().await.unwrap();
        assert!(matches!(first, ProviderEvent::TextDelta { .. }));

        let _ = session.commands.send(ProviderCommand::Abort).await;

        while let Some(event) = tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
            .await
            .expect("provider should close after abort")
        {
            assert!(
                !matches!(event, ProviderEvent::Completed { .. }),
                "abort after the final text delta should suppress completion"
            );
        }
    }

    #[tokio::test]
    async fn fake_streaming_provider_cancel_closes_commands_when_completed_is_backpressured() {
        let provider = FakeStreamingProvider;
        let cancel = CancellationToken::new();
        let prompt = prompt_with_word_count(32);
        let session = provider
            .start(make_provider_input(&prompt), cancel.clone())
            .await
            .unwrap();

        wait_for_buffer_len(&session.events, 6).await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        cancel.cancel();

        tokio::time::timeout(TEST_TIMEOUT, session.commands.closed())
            .await
            .expect(
                "cancel should close the provider command receiver under completed backpressure",
            );
    }

    #[tokio::test]
    async fn fake_streaming_provider_run_streaming_cancel_closes_bridge_when_output_is_backpressured()
     {
        let provider = FakeStreamingProvider;
        let cancel = CancellationToken::new();
        let prompt = prompt_with_word_count(80);
        let input = make_input(&prompt);
        let rx = provider
            .run_streaming(&input, cancel.clone())
            .await
            .unwrap();

        wait_for_buffer_len(&rx, 6).await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        cancel.cancel();

        wait_for_receiver_closed(&rx).await;
    }

    #[tokio::test]
    async fn fake_streaming_provider_cancel_stops_output() {
        let provider = FakeStreamingProvider;
        let cancel = CancellationToken::new();
        let input = make_input("a b c d e f g h i j");

        let mut rx = provider
            .run_streaming(&input, cancel.clone())
            .await
            .unwrap();

        let first = rx.recv().await.unwrap();
        assert!(matches!(first, StreamChunk::Text(_)));
        cancel.cancel();

        for _ in 0..9 {
            let Some(chunk) = tokio::time::timeout(TEST_TIMEOUT, rx.recv())
                .await
                .expect("provider should close after cancel")
            else {
                return;
            };
            assert!(
                !matches!(chunk, StreamChunk::Done { .. }),
                "cancelled provider should not emit a completion marker"
            );
        }

        panic!("cancelled provider should close before emitting the full stream");
    }
}
