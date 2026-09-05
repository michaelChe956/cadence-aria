use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::cross_cutting::structured_output::{
    StructuredOutputContract, StructuredOutputState, parse_structured_output,
    parse_structured_output_first_line_nonce,
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

/// Tool-policy canonical 序列的规范版本前缀（REQ-ENV-09 digest 规范冻结：`"tp-v1"`）。
pub(crate) const TOOL_POLICY_CANONICAL_VERSION: &str = "tp-v1";

/// Codex 审批分类规则的版本后缀（digest 规范冻结：`"ap-v1"`；审批规则变化必须升级）。
pub(crate) const TOOL_POLICY_APPROVAL_POLICY_VERSION: &str = "ap-v1";

/// Tool-policy 语义意图。本期唯一合法意图为 `DenyFileWriteBuiltins`：
/// 保护范围是 built-in 文件写工具（黑名单），不是全工具 allowlist。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolPolicyIntent {
    DenyFileWriteBuiltins,
}

/// Provider 无关的语义工具策略。只表达语义意图，不携带 provider 物理片段；
/// 物理片段由各 provider translator（`translate_tool_policy`）按 provider 名冻结。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderToolPolicy {
    pub intent: ToolPolicyIntent,
}

impl ProviderToolPolicy {
    /// 唯一合法意图构造器：拒绝 built-in 文件写工具。后续 Task 统一用此构造器。
    pub fn deny_file_write_builtins() -> Self {
        Self {
            intent: ToolPolicyIntent::DenyFileWriteBuiltins,
        }
    }
}

/// 双向 spawn 前守卫错误（REQ-ENV-09 Task 3.1）。策略角色缺失策略与
/// 非策略角色误带策略都在创建子进程之前拒绝，不 fallback 到无策略 argv。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolPolicyGuardError {
    /// 策略角色（Orchestrator/WorkItemSplitter/Reviewer）缺失或非法策略。
    PolicyRequired { role: String },
    /// 非策略角色（Executor/Handoff，含 Coder 与聚合初始化 turns）误带策略。
    PolicyForbidden { role: String },
}

impl std::fmt::Display for ToolPolicyGuardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ToolPolicyGuardError::PolicyRequired { role } => write!(
                f,
                "tool policy guard: policy role {role} must carry DenyFileWriteBuiltins"
            ),
            ToolPolicyGuardError::PolicyForbidden { role } => write!(
                f,
                "tool policy guard: non-policy role {role} must not carry a tool policy"
            ),
        }
    }
}

impl std::error::Error for ToolPolicyGuardError {}

/// AdapterRole 的真实序列化文本（durable 审计 provider_start.role 用；与
/// `UsageReportData::role_text` 的 usage 展示层归一严格分离，P1-8）。
pub(crate) fn adapter_role_text(role: &AdapterRole) -> &'static str {
    match role {
        AdapterRole::Orchestrator => "orchestrator",
        AdapterRole::Executor => "executor",
        AdapterRole::Reviewer => "reviewer",
        AdapterRole::WorkItemSplitter => "work_item_splitter",
        AdapterRole::Handoff => "handoff",
    }
}

/// Provider 版本探测统一错误（GC9，Task 3.3 正式落地；3.2 期间作为注入 supplier
/// 的错误语义先行）。策略会话遇任一错误直接 fail-closed，Coder/非策略路径零变化。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionProbeError {
    /// 命令失败或输出为空：版本不可得。
    Unavailable,
    /// 有界超时内未取得版本。
    Timeout,
}

impl std::fmt::Display for VersionProbeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VersionProbeError::Unavailable => {
                write!(f, "provider version unavailable")
            }
            VersionProbeError::Timeout => write!(f, "provider version probe timed out"),
        }
    }
}

impl std::error::Error for VersionProbeError {}

/// 策略会话 provider 版本 supplier（controller Ruling：3.2 以注入 supplier/fixture
/// 字符串实现 `provider_start.version`；3.3 落地真实 CLI `--version` 探测+进程内
/// 缓存后接线替换默认路径，测试 seam 保留）。
pub type ProviderVersionSupplier =
    std::sync::Arc<dyn Fn() -> Result<String, VersionProbeError> + Send + Sync>;

/// CLI `--version` 探测的进程内缓存（GC9：进程内缓存、有界超时）。按命令路径
/// 缓存成功结果；失败不缓存（下次启动重试）。supplier seam（测试）优先于缓存。
pub(crate) async fn cached_cli_version(
    command: &std::path::Path,
    probe: impl std::future::Future<Output = Result<String, VersionProbeError>>,
) -> Result<String, VersionProbeError> {
    static CACHE: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<std::path::PathBuf, String>>,
    > = std::sync::OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Ok(guard) = cache.lock()
        && let Some(version) = guard.get(command)
    {
        return Ok(version.clone());
    }
    let version = probe.await?;
    if let Ok(mut guard) = cache.lock() {
        guard.insert(command.to_path_buf(), version.clone());
    }
    Ok(version)
}

/// 双向角色×策略守卫：Orchestrator/WorkItemSplitter/Reviewer 必须携带
/// `DenyFileWriteBuiltins`；Executor/Handoff 必须不携带策略。非法组合在
/// provider 创建子进程之前拒绝（三 adapter `start` 首步调用）。
pub fn validate_tool_policy_for_role(
    role: &AdapterRole,
    policy: Option<&ProviderToolPolicy>,
) -> Result<(), ToolPolicyGuardError> {
    match role {
        AdapterRole::Orchestrator | AdapterRole::WorkItemSplitter | AdapterRole::Reviewer => {
            // 缺失或未来非法意图均拒绝：`matches!` 对新增 intent 变体默认不命中，
            // fail-closed（本期唯一合法意图为 DenyFileWriteBuiltins）。
            if policy.is_some_and(|policy| {
                matches!(policy.intent, ToolPolicyIntent::DenyFileWriteBuiltins)
            }) {
                Ok(())
            } else {
                Err(ToolPolicyGuardError::PolicyRequired {
                    role: adapter_role_text(role).to_string(),
                })
            }
        }
        AdapterRole::Executor | AdapterRole::Handoff => {
            if policy.is_some() {
                return Err(ToolPolicyGuardError::PolicyForbidden {
                    role: adapter_role_text(role).to_string(),
                });
            }
            Ok(())
        }
    }
}

/// Canonical tool policy 投影：provider 名 + 按 provider 冻结的 canonical token
/// 序列 + 审批规则版本 + 依规范输入实算的 sha256 digest。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalToolPolicy {
    pub provider: String,
    pub tokens: Vec<String>,
    pub approval_policy_version: String,
    pub digest: String,
}

/// Codex 审批分类（GC6 冻结）：`item/commandExecution/requestApproval` 是
/// commandExecution；`item/fileChange/requestApproval` 是 fileChange；
/// `mcpServer/elicitation/request` 仅在 `_meta.codex_approval_kind="mcp_tool_call"`
/// 时是 MCP，否则为未知 elicitation；未知 `item/*/requestApproval` 为未知 item。
/// 自然语言 `reason` 不得作为分类依据；未知形态保留原 method。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodexApprovalCategory {
    McpToolCall,
    CommandExecution,
    FileChange,
    Unknown { method: String },
}

impl CodexApprovalCategory {
    /// 审计载荷中的类别文本（结构化事件用）。
    pub fn audit_text(&self) -> &'static str {
        match self {
            CodexApprovalCategory::McpToolCall => "mcp_tool_call",
            CodexApprovalCategory::CommandExecution => "command_execution",
            CodexApprovalCategory::FileChange => "file_change",
            CodexApprovalCategory::Unknown { .. } => "unknown",
        }
    }
}

/// Codex 审批应答协议结果：`Accept`/`Decline` 序列化为 `{"decision":...}`；
/// `ElicitationError` 序列化为 JSON-RPC error（未知 elicitation = `-32601` + data）。
#[derive(Debug, Clone, PartialEq)]
pub enum CodexApprovalResponse {
    Accept,
    Decline,
    ElicitationError { code: i32, data: serde_json::Value },
}

impl CodexApprovalResponse {
    /// 审计载荷中的决策文本（结构化事件用）。
    pub fn audit_text(&self) -> &'static str {
        match self {
            CodexApprovalResponse::Accept => "accept",
            CodexApprovalResponse::Decline => "decline",
            CodexApprovalResponse::ElicitationError { .. } => "protocol_error",
        }
    }
}

/// 策略会话审批决策的结构化事件（GC11 canonical `approval_decision` 的内存形态；
/// durable 落盘接线在后续 task 的 sink 注入，本类型即类型化决策出口）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexApprovalDecisionEvent {
    pub request_id: String,
    pub category: &'static str,
    pub decision: &'static str,
}

/// 未知审批形态的结构化告警事件（GC11 canonical `protocol_warning` 内存形态）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexProtocolWarningEvent {
    pub reason_code: String,
    pub method: String,
    pub occurrence: u32,
}

/// 未知审批风暴终止的结构化事件（GC11 canonical `session_terminated` 内存形态）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexSessionTerminatedEvent {
    pub reason_code: String,
}

/// Tool-policy 翻译/canonical 化错误。未知 provider 名 fail-closed（kimi 不接策略）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolPolicyError {
    UnsupportedProvider(String),
}

impl std::fmt::Display for ToolPolicyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ToolPolicyError::UnsupportedProvider(provider) => {
                write!(f, "unsupported tool-policy provider: {provider}")
            }
        }
    }
}

impl std::error::Error for ToolPolicyError {}

/// 依 digest 规范实算：`"tp-v1" + \x1f + provider + \x1f + tokens.join(\x1f) + \x1f + "ap-v1"`
/// 的 sha256 hex。argv flag/value 按出现顺序原样、大小写保留。
pub(crate) fn tool_policy_digest(
    provider: &str,
    tokens: &[String],
    approval_policy_version: &str,
) -> String {
    use sha2::{Digest, Sha256};

    let canonical_input = [
        TOOL_POLICY_CANONICAL_VERSION,
        provider,
        &tokens.join("\x1f"),
        approval_policy_version,
    ]
    .join("\x1f");
    let mut hasher = Sha256::new();
    hasher.update(canonical_input.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// 各 provider 冻结的 DenyFileWriteBuiltins canonical token 序列；
/// 未知 provider 返回 `UnsupportedProvider`。
fn deny_file_write_builtins_tokens(provider: &str) -> Result<Vec<String>, ToolPolicyError> {
    match provider {
        crate::cross_cutting::pi_provider::TOOL_POLICY_PROVIDER_NAME => {
            Ok(crate::cross_cutting::pi_provider::deny_file_write_builtins_tokens())
        }
        crate::cross_cutting::claude_code_provider::TOOL_POLICY_PROVIDER_NAME => {
            Ok(crate::cross_cutting::claude_code_provider::deny_file_write_builtins_tokens())
        }
        crate::cross_cutting::codex_provider::session::TOOL_POLICY_PROVIDER_NAME => {
            Ok(crate::cross_cutting::codex_provider::session::deny_file_write_builtins_tokens())
        }
        other => Err(ToolPolicyError::UnsupportedProvider(other.to_string())),
    }
}

/// 把语义策略投影为 canonical 形态（provider + tokens + 审批规则版本 + digest）。
pub fn canonical_tool_policy(
    provider: &str,
    policy: &ProviderToolPolicy,
) -> Result<CanonicalToolPolicy, ToolPolicyError> {
    match policy.intent {
        ToolPolicyIntent::DenyFileWriteBuiltins => {
            let tokens = deny_file_write_builtins_tokens(provider)?;
            Ok(CanonicalToolPolicy {
                provider: provider.to_string(),
                digest: tool_policy_digest(provider, &tokens, TOOL_POLICY_APPROVAL_POLICY_VERSION),
                tokens,
                approval_policy_version: TOOL_POLICY_APPROVAL_POLICY_VERSION.to_string(),
            })
        }
    }
}

/// 把语义策略翻译为 provider 物理片段（argv flag/value 或 codex 启动参数原文，
/// 按出现顺序原样、大小写保留）。
pub fn translate_tool_policy(
    provider: &str,
    policy: &ProviderToolPolicy,
) -> Result<Vec<String>, ToolPolicyError> {
    match policy.intent {
        ToolPolicyIntent::DenyFileWriteBuiltins => deny_file_write_builtins_tokens(provider),
    }
}

#[derive(Clone)]
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
    /// 语义工具策略（REQ-ENV-09）。策略角色（Orchestrator/WorkItemSplitter/Reviewer
    /// 的作者/评审链）携带 `DenyFileWriteBuiltins`；Executor/Coder、聚合初始化与
    /// 非策略路径必须传 `None`（kimi 零改动，不读此字段）。
    pub tool_policy: Option<ProviderToolPolicy>,
    /// durable tool-policy 审计 sink（REQ-ENV-09 Task 3.2/D7）。策略会话由 engine
    /// 按 provider run 绑定 `(workspace_session_id, role_run_seq)` 后注入；缺失时
    /// 策略会话启动 fail-closed。非策略路径恒为 `None`。
    pub audit_sink:
        Option<std::sync::Arc<dyn crate::cross_cutting::tool_policy_audit::ToolPolicyAuditSink>>,
    pub structured_output_contract: Option<StructuredOutputContract>,
    pub env_vars: BTreeMap<String, String>,
    pub timeout_secs: u64,
}

impl std::fmt::Debug for StreamingProviderInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamingProviderInput")
            .field("provider_type", &self.provider_type)
            .field("role", &self.role)
            .field("prompt", &self.prompt)
            .field("working_dir", &self.working_dir)
            .field("workspace_session_id", &self.workspace_session_id)
            .field(
                "resume_provider_session_id",
                &self.resume_provider_session_id,
            )
            .field("permission_mode", &self.permission_mode)
            .field("tool_policy", &self.tool_policy)
            .field(
                "audit_sink",
                &self.audit_sink.as_ref().map(|_| "<tool-policy-audit-sink>"),
            )
            .field(
                "structured_output_contract",
                &self.structured_output_contract,
            )
            .field("env_vars", &self.env_vars)
            .field("timeout_secs", &self.timeout_secs)
            .finish()
    }
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
            tool_policy: None,
            audit_sink: None,
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
    /// Provider 上报的 token 用量事件（best-effort，见 `UsageReportData`）。
    Usage,
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
        let parsed = if contract.schema_name == "single_candidate_work_item_plan_review" {
            parse_structured_output_first_line_nonce(&full_output, contract)
        } else {
            parse_structured_output(&full_output, contract)
        };
        if parsed.preamble_trimmed {
            tracing::info!(
                diagnostic = "preamble_trimmed",
                schema = %contract.schema_name,
                "discarded preamble before single-candidate reviewer nonce sentinel"
            );
        }
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
    /// Provider 完成时上报的 token 用量（best-effort）。
    ///
    /// 数据源因 provider 而异（claude stream-json result.usage、pi get_state cost 等）；
    /// 任一字段不可得时为 `None`，不视为错误。
    UsageReport(UsageReportData),
    /// 策略会话审批决策的结构化出口（GC11 canonical `approval_decision` 的内存形态；
    /// durable 落盘在后续 task 的 LifecycleStore sink 注入接线）。
    ToolPolicyDecision(CodexApprovalDecisionEvent),
    /// 未知审批形态告警的结构化出口（GC11 canonical `protocol_warning` 内存形态）。
    ToolPolicyWarning(CodexProtocolWarningEvent),
    /// 未知审批风暴终止的结构化出口（GC11 canonical `session_terminated` 内存形态）。
    ToolPolicyTerminated(CodexSessionTerminatedEvent),
}

/// 一次 provider 会话的 token 用量快照。
///
/// `role` 为投放侧角色（`author` / `reviewer`，由 adapter 依据 `AdapterRole` 归一化）；
/// 其余字段与 Anthropic 风格 usage 命名对齐，serde 使用 snake_case。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct UsageReportData {
    pub role: String,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cache_read_tokens: Option<u64>,
    pub cache_creation_tokens: Option<u64>,
}

impl UsageReportData {
    /// 将 adapter role 归一化为 usage 角色：Reviewer → `reviewer`，其余（Orchestrator/
    /// Executor/WorkItemSplitter/Handoff）在 story/design 链路上均为产出侧 → `author`。
    pub fn role_text(role: &AdapterRole) -> &'static str {
        match role {
            AdapterRole::Reviewer => "reviewer",
            AdapterRole::Orchestrator
            | AdapterRole::Executor
            | AdapterRole::WorkItemSplitter
            | AdapterRole::Handoff => "author",
        }
    }

    /// 全部 token 字段均缺失时视为无可上报用量。
    pub fn has_any_tokens(&self) -> bool {
        self.input_tokens.is_some()
            || self.output_tokens.is_some()
            || self.cache_read_tokens.is_some()
            || self.cache_creation_tokens.is_some()
    }
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
    /// 原生 provider session id（Task 3.2）：策略会话在 `start` 返回前完成有界
    /// 握手后写入（codex=thread id；claude=init session id/已知 resume id；
    /// pi=预生成 `--session-id`）。非策略/Coder/kimi 路径恒为 `None`（零变化）。
    pub native_session_id: Option<String>,
}

#[async_trait::async_trait]
pub trait StreamingProviderAdapter: Send + Sync {
    fn supports_tool_calls(&self) -> bool {
        false
    }

    /// legacy bridge 按角色矩阵派生策略会话时注入的 durable 审计 sink（engine 侧
    /// 注入点，P2-1）：真实 adapter 的策略会话缺 sink 会 fail-closed，engine 必须
    /// 在 legacy 直连路径同时提供策略与 sink。默认 `None`（非 engine 直调方不
    /// 派生 sink，策略会话对裸直调保持 fail-closed）。
    fn legacy_tool_policy_audit_sink(
        &self,
    ) -> Option<std::sync::Arc<dyn crate::cross_cutting::tool_policy_audit::ToolPolicyAuditSink>>
    {
        None
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
        let audit_sink = self.legacy_tool_policy_audit_sink();
        let this = self;
        run_legacy_bridge_stream(
            audit_sink,
            Box::new(move |provider_input, cancel| {
                Box::pin(async move { this.start(provider_input, cancel).await })
            }),
            input,
            cancel,
        )
        .await
    }
}

/// legacy bridge 的启动闭包（返回装箱 future，供自由函数复用默认桥接体）。
pub(crate) type LegacyBridgeStart<'a> = Box<
    dyn FnOnce(
            StreamingProviderInput,
            CancellationToken,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = Result<ProviderSession, ProviderAdapterError>>
                    + Send
                    + 'a,
            >,
        > + Send
        + 'a,
>;

/// 默认 legacy bridge 的桥接体（P2-1 抽出为自由函数）：`run_streaming` 默认实现
/// 与 engine 侧注入装饰器（`LegacyToolPolicyAuditProvider`）共用，避免装饰器经
/// 虚分发回调自身默认实现导致递归。
pub(crate) async fn run_legacy_bridge_stream<'a>(
    audit_sink: Option<
        std::sync::Arc<dyn crate::cross_cutting::tool_policy_audit::ToolPolicyAuditSink>,
    >,
    start: LegacyBridgeStart<'a>,
    input: &'a AdapterInput,
    cancel: CancellationToken,
) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
    let working_dir = input.worktree_path.as_ref().map(PathBuf::from).unwrap_or(
        std::env::current_dir().map_err(|error| {
            ProviderAdapterError::execution_failed(None, String::new(), error.to_string(), 0)
        })?,
    );
    // REQ-ENV-09（GC13 例外边界 + Task 3.1）：legacy 同步直连不携带策略字段，
    // 但必须接受 adapter 双向守卫——被角色矩阵派生语义策略（策略角色带上
    // DenyFileWriteBuiltins，Executor/Handoff 保持 None），随后统一经
    // `start` 接受守卫与 argv 注入。kimi 不读 `tool_policy`，零物理变化。
    let tool_policy = match input.role {
        AdapterRole::Orchestrator | AdapterRole::WorkItemSplitter | AdapterRole::Reviewer => {
            Some(ProviderToolPolicy::deny_file_write_builtins())
        }
        AdapterRole::Executor | AdapterRole::Handoff => None,
    };
    let provider_input = StreamingProviderInput {
        tool_policy,
        // P2-1：派生策略必须同时注入 sink（engine 侧 hook）。
        audit_sink: audit_sink.clone(),
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
    let mut session = start(provider_input, cancel).await?;
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
                                "run_streaming does not support interactive permission requests"
                                    .to_string(),
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
                | ProviderEvent::ToolResult(_)
                | ProviderEvent::UsageReport(_)
                | ProviderEvent::ToolPolicyDecision(_)
                | ProviderEvent::ToolPolicyWarning(_)
                | ProviderEvent::ToolPolicyTerminated(_) => {
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
