use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::cross_cutting::structured_output::{
    StructuredOutputContract, StructuredOutputState, parse_structured_output,
};
use crate::protocol::contracts::{AdapterInput, AdapterRole, ProviderType};

/// Test-only fake provider launch input wrapper.
///
/// `FakeStreamingProviderInput` 把 `StreamingProviderInput` 包裹起来,使其只能在
/// 测试中通过 `for_test` 构造,且必须声明 `ProviderType::Fake`。这是「Fake 经过
/// registry/编译期隔离」契约在 type 层的体现:逻辑代码库真实 provider 启动路径
/// (`LogicalCodebaseProviderGateway`/`ValidatedStreamingProviderInput`)不接受
/// 裸 `StreamingProviderInput`,而测试侧的 Fake 启动用此 wrapper 明确标记为隔离,
/// 而非通过 `if provider != Fake` 的运行时例外放行真实 provider。
///
/// 生产代码不构造本类型;`for_test` 在 `#[cfg(test)]` 下可用。
#[cfg(test)]
pub(crate) struct FakeStreamingProviderInput(StreamingProviderInput);

#[cfg(test)]
impl FakeStreamingProviderInput {
    /// 构造一个测试用 fake launch input。`provider_type` 必须 `ProviderType::Fake`,
    /// 否则 panic——这是「Fake 不可伪装成真实 provider」的编译/构造期断言。
    #[cfg(test)]
    pub fn for_test(provider_type: ProviderType, prompt: &str) -> Self {
        assert_eq!(
            provider_type,
            ProviderType::Fake,
            "FakeStreamingProviderInput::for_test 仅接受 ProviderType::Fake"
        );
        Self(StreamingProviderInput::fake_for_test(prompt))
    }

    /// 返回内部 `StreamingProviderInput` 的 provider type,供测试断言隔离。
    pub fn provider_type(&self) -> &ProviderType {
        &self.0.provider_type
    }
}

pub mod fake;

#[cfg(test)]
pub mod tests;

pub use fake::FakeStreamingProvider;

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

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
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
    pub structured_output_contract: Option<StructuredOutputContract>,
    pub env_vars: BTreeMap<String, String>,
    pub timeout_secs: u64,
}

impl StreamingProviderInput {
    /// 测试专用构造函数:产出一个 `ProviderType::Fake` 的隔离 input。生产代码不得
    /// 调用本方法;它仅供 `FakeStreamingProviderInput::for_test` 复用,使 Fake
    /// 启动路径与真实 provider 启动路径在构造上完全分离。
    #[cfg(test)]
    fn fake_for_test(prompt: &str) -> Self {
        Self {
            provider_type: ProviderType::Fake,
            role: AdapterRole::Orchestrator,
            prompt: prompt.to_string(),
            working_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            workspace_session_id: None,
            resume_provider_session_id: None,
            permission_mode: ProviderPermissionMode::Auto,
            structured_output_contract: None,
            env_vars: BTreeMap::new(),
            timeout_secs: 60,
        }
    }
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
pub struct ChoiceQuestionData {
    pub id: String,
    pub prompt: String,
    pub options: Vec<ChoiceOptionData>,
    pub allow_multiple: bool,
    pub allow_free_text: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChoiceAnswerData {
    pub question_id: String,
    pub selected_option_ids: Vec<String>,
    pub free_text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChoiceRequestData {
    pub id: String,
    pub prompt: String,
    pub options: Vec<ChoiceOptionData>,
    pub allow_multiple: bool,
    pub allow_free_text: bool,
    pub questions: Vec<ChoiceQuestionData>,
    pub source: ChoiceRequestSource,
}

impl ChoiceRequestData {
    pub fn effective_questions(&self) -> Vec<ChoiceQuestionData> {
        if self.questions.is_empty() {
            return vec![ChoiceQuestionData {
                id: "default".to_string(),
                prompt: self.prompt.clone(),
                options: self.options.clone(),
                allow_multiple: self.allow_multiple,
                allow_free_text: self.allow_free_text,
            }];
        }
        self.questions.clone()
    }
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
pub struct ProviderCompletion {
    pub full_output: String,
    pub readable_output: String,
    pub structured_output: StructuredOutputState,
    pub provider_session_id: Option<String>,
}

impl ProviderCompletion {
    pub fn plain(full_output: impl Into<String>, provider_session_id: Option<String>) -> Self {
        let full_output = full_output.into();
        Self {
            readable_output: full_output.clone(),
            full_output,
            structured_output: StructuredOutputState::NotRequested,
            provider_session_id,
        }
    }

    pub fn from_output(
        full_output: String,
        contract: Option<&StructuredOutputContract>,
        provider_session_id: Option<String>,
    ) -> Self {
        let Some(contract) = contract else {
            return Self::plain(full_output, provider_session_id);
        };
        let parsed = parse_structured_output(&full_output, contract);
        Self {
            full_output,
            readable_output: parsed.readable_output,
            structured_output: parsed.state,
            provider_session_id,
        }
    }
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
    Completed(ProviderCompletion),
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
        answers: Vec<ChoiceAnswerData>,
    },
    ToolResult(ProviderToolResult),
    Abort,
}

pub struct ProviderSession {
    pub events: mpsc::Receiver<ProviderEvent>,
    pub commands: mpsc::Sender<ProviderCommand>,
}

#[async_trait::async_trait]
pub trait StreamingProviderAdapter: Send + Sync {
    fn supports_tool_calls(&self) -> bool {
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

    /// **Legacy bridge**:`run_streaming` 把同步 `AdapterInput` 适配为流式 session,
    /// 仅供未携带逻辑代码库 target(`attempt.target_snapshot.is_none()`)的历史调用
    /// 使用。逻辑代码库真实 provider 启动必须经 `LogicalCodebaseProviderGateway`,
    /// 不得回落到本 bridge——这是 Task 12「关闭逻辑代码库裸输入与 legacy streaming
    /// fallback」的边界。逻辑目标的 `allow_legacy_stream_fallback` 被强制为 `false`,
    /// 因此 `provider_stream` 不会在 `start` 未实现时调用本方法。
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
            structured_output_contract: None,
            env_vars: BTreeMap::new(),
            timeout_secs: input.timeout,
        };
        let bridge_cancel = cancel.clone();
        let mut session = self.start(provider_input, cancel).await?;
        let (tx, rx) = mpsc::channel(32);

        tokio::spawn(async move {
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
                    ProviderEvent::Completed(completion) => StreamChunk::Done {
                        full_output: completion.full_output,
                    },
                    ProviderEvent::Failed { message } => StreamChunk::Error(message),
                    ProviderEvent::ProtocolError { message, .. } => StreamChunk::Error(message),
                    ProviderEvent::PermissionTimeout { permission_id } => {
                        StreamChunk::Error(format!("Permission request {permission_id} timed out"))
                    }
                    ProviderEvent::PermissionRequest(request) => {
                        let _ = session
                            .commands
                            .send(ProviderCommand::PermissionResponse {
                                id: request.id,
                                approved: false,
                                reason: Some(
                                    "run_streaming does not support interactive permission requests".to_string(),
                                ),
                            })
                            .await;
                        let _ = tx
                            .send(StreamChunk::Error(
                                "interactive permission request is not supported in run_streaming"
                                    .to_string(),
                            ))
                            .await;
                        return;
                    }
                    ProviderEvent::ChoiceRequest(request) => {
                        let _ = session
                            .commands
                            .send(ProviderCommand::ChoiceResponse {
                                id: request.id,
                                selected_option_ids: vec![],
                                free_text: Some("aborted".to_string()),
                                answers: vec![],
                            })
                            .await;
                        let _ = tx
                            .send(StreamChunk::Error(
                                "interactive choice request is not supported in run_streaming"
                                    .to_string(),
                            ))
                            .await;
                        return;
                    }
                    ProviderEvent::StatusChanged(_)
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
