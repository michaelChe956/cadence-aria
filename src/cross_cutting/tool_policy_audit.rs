//! Tool-policy durable 审计契约（REQ-ENV-09 / GC11，Task 3.2）。
//!
//! 本模块是 **中立层**：只定义 durable 事件的 schema（v1）、四类 canonical 事件
//! DTO、行封装与 `ToolPolicyAuditSink` trait。durable 落盘由 product 层的
//! `LifecycleStore`（分区 `tool-policy-run-audit/`）实现该 trait 完成，避免
//! cross-cutting → product 反向依赖。
//!
//! 契约冻结（GC11）：
//! - 仅四类 canonical 事件：`provider_start`、`approval_decision`、
//!   `protocol_warning`、`session_terminated`；
//! - append-only JSONL，文件 key=`(workspace_session_id, role_run_seq)`；
//! - 文件内行 `seq` 单调递增；`provider_start` 恰为首行且每文件仅一条；
//! - 读取坏行跳过并产生读取结果告警，该告警不写回 durable 分区；
//! - 写入失败必须传播错误（调用方沿既有 kill 链处理），不得只记日志。

use std::sync::Arc;

/// durable 事件 schema 版本（GC11 冻结：v1）。
pub const TOOL_POLICY_AUDIT_SCHEMA_VERSION: u32 = 1;

/// durable 审计分区名（GC10 冻结：策略角色专用，与 `execution_event_audit` 分离）。
pub const TOOL_POLICY_RUN_AUDIT_PARTITION: &str = "tool-policy-run-audit";

/// durable 审计写入/读取错误。写入端错误必须传播（GC11），由 adapter/engine 沿
/// 既有 provider kill 链终止会话并将 run 判失败。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolPolicyAuditError {
    /// 底层 IO 失败（分区创建/读写）。
    Io(String),
    /// 序列化失败（canonical 事件必须可序列化）。
    Serde(String),
    /// 同一文件内出现第二条 `provider_start`（或非首行补写）。
    DuplicateProviderStart,
    /// canonical 文件必须以 `provider_start` 首行开启。
    ProviderStartRequired,
    /// 标识符非法（路径逃逸/空串）。
    InvalidIdentifier(String),
    /// 未绑定 (workspace_session_id, role_run_seq) 的 sink 不支持 run-bound 写入。
    UnboundSink,
}

impl std::fmt::Display for ToolPolicyAuditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ToolPolicyAuditError::Io(message) => write!(f, "tool policy audit io: {message}"),
            ToolPolicyAuditError::Serde(message) => {
                write!(f, "tool policy audit serde: {message}")
            }
            ToolPolicyAuditError::DuplicateProviderStart => {
                write!(f, "tool policy audit: duplicate provider_start")
            }
            ToolPolicyAuditError::ProviderStartRequired => {
                write!(
                    f,
                    "tool policy audit: provider_start must be the first event"
                )
            }
            ToolPolicyAuditError::InvalidIdentifier(value) => {
                write!(f, "tool policy audit: invalid identifier {value:?}")
            }
            ToolPolicyAuditError::UnboundSink => {
                write!(f, "tool policy audit: sink is not bound to a provider run")
            }
        }
    }
}

impl std::error::Error for ToolPolicyAuditError {}

/// `provider_start`（D7/GC9）：一次策略 provider run 的启动指纹。
/// D6 冻结持久化字段：`workspace_session_id`、`provider_session_id`、
/// `tool_policy_canonical_digest`、`provider_version`、`adapter_dialect`、
/// provider 名、role、最终片段（argv/沙箱/审批原文）。
/// digest/version/dialect 三元组即 resume 冻结比对输入（Task 3.3）。
/// 🔴 必需字段不以 `#[serde(default)]` 宽容：缺失 = 解析失败（fail-closed）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
pub struct ProviderStartAudit {
    /// tool-policy canonical 序列 provider 名（`pi`/`claude-code`/`codex`）。
    pub provider: String,
    /// 投放侧角色（`AdapterRole` 真实序列化值：`orchestrator`/
    /// `work_item_splitter`/`reviewer`/`executor`/`handoff`）。
    pub role: String,
    /// workspace 会话 id（与文件 key 同源；D6 冻结进入事件 DTO）。
    pub workspace_session_id: String,
    /// 原生 provider session id（握手确认：codex thread id / claude init session id /
    /// pi 预生成 `--session-id`）。
    pub provider_session_id: String,
    /// canonical tool-policy digest（sha256，见 `tool_policy_digest` 规范；独立字段，
    /// 不与 gateway aggregate policy digest 混用）。
    pub tool_policy_canonical_digest: String,
    /// 最终 argv（策略片段按出现顺序原样、大小写保留）。
    pub argv: Vec<String>,
    /// sandbox 原文（codex `read-only`/`danger-full-access`；pi/claude 为 `None`）。
    pub sandbox: Option<String>,
    /// approval policy 原文（codex `on-request`/`never`；pi/claude 为 `None`）。
    pub approval_policy: Option<String>,
    /// provider CLI version（策略会话启动时探测；不可得则启动 fail-closed）。
    pub provider_version: String,
    /// adapter dialect 常量（如 `codex-app-server-rpc`/`claude-stream-json`/`pi-rpc`）。
    pub adapter_dialect: String,
}

impl ProviderStartAudit {
    /// resume 冻结三元组的 digest 变体（审计 fixture / drift 测试用）。
    pub fn with_digest(mut self, digest: impl Into<String>) -> Self {
        self.tool_policy_canonical_digest = digest.into();
        self
    }

    /// resume 冻结三元组的 version 变体（审计 fixture / drift 测试用）。
    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.provider_version = version.into();
        self
    }

    /// resume 冻结三元组的 dialect 变体（审计 fixture / drift 测试用）。
    pub fn with_dialect(mut self, dialect: impl Into<String>) -> Self {
        self.adapter_dialect = dialect.into();
        self
    }

    /// provider session id 变体（审计 fixture / resume 检索测试用）。
    pub fn with_provider_session_id(mut self, provider_session_id: impl Into<String>) -> Self {
        self.provider_session_id = provider_session_id.into();
        self
    }

    /// resume 冻结三元组（digest, version, dialect）。
    pub fn resume_fingerprint(&self) -> (&str, &str, &str) {
        (
            &self.tool_policy_canonical_digest,
            &self.provider_version,
            &self.adapter_dialect,
        )
    }
}

/// `approval_decision`（GC6/D7）：策略会话审批决策审计。D7 冻结字段：
/// category/server_name/tool_name/request_id/decision/reason_code/policy_digest。
/// server_name/tool_name 仅 MCP 形态携带（commandExecution/fileChange 为 `None`）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
pub struct ApprovalDecisionAudit {
    /// 决策对应的 request id（codex 策略会话为 Aria 出站 `aria-<seq>` 或原生 id 原文）。
    pub request_id: String,
    /// 审批分类（`command_execution`/`file_change`/`mcp_tool_call`/`unknown`）。
    pub category: String,
    /// MCP server 名（仅 `mcp_tool_call` 形态；其余为 `None`）。
    pub server_name: Option<String>,
    /// 工具/形态名（MCP 工具名；commandExecution=`command`、fileChange=`file_change`）。
    pub tool_name: Option<String>,
    /// 决策（`accept`/`decline`/`protocol_error`）。
    pub decision: String,
    /// 决策原因码（策略拒绝/允许的语义原因，如 `policy_denies_write_side`）。
    pub reason_code: String,
    /// 会话 tool-policy canonical digest（决策归属的策略指纹）。
    pub policy_digest: String,
}

/// `protocol_warning`（GC6）：未知审批形态告警审计。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
pub struct ProtocolWarningAudit {
    /// 告警原因码（如 `unsupported_approval_kind`、读取端 `invalid_json_line` 内存形态）。
    pub reason_code: String,
    /// 触发告警的原始 method（保留原文，不读 reason 字段）。
    pub method: String,
    /// 同一会话连续出现序号（风暴计数输入）。
    pub occurrence: u32,
}

/// `session_terminated`（GC6/GC9）：会话终止审计（风暴终止 / resume superseded）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
pub struct SessionTerminatedAudit {
    /// 终止原因码（如 `unknown_approval_storm`、`superseded_policy_drift`）。
    pub reason_code: String,
}

/// durable canonical 事件四类枚举（GC11 冻结，serde tag=`event_type` snake_case）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "event_type", rename_all = "snake_case")]
pub enum DurableToolPolicyEvent {
    ProviderStart(ProviderStartAudit),
    ApprovalDecision(ApprovalDecisionAudit),
    ProtocolWarning(ProtocolWarningAudit),
    SessionTerminated(SessionTerminatedAudit),
}

impl DurableToolPolicyEvent {
    /// canonical 事件类型文本（JSONL `event_type` 字段值）。
    pub fn event_type(&self) -> &'static str {
        match self {
            DurableToolPolicyEvent::ProviderStart(_) => "provider_start",
            DurableToolPolicyEvent::ApprovalDecision(_) => "approval_decision",
            DurableToolPolicyEvent::ProtocolWarning(_) => "protocol_warning",
            DurableToolPolicyEvent::SessionTerminated(_) => "session_terminated",
        }
    }
}

/// durable JSONL 行封装：schema 版本 + 单调 `seq` + canonical 事件（flatten）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ToolPolicyAuditLine {
    pub schema_version: u32,
    pub seq: u64,
    #[serde(flatten)]
    pub event: DurableToolPolicyEvent,
}

impl ToolPolicyAuditLine {
    /// 以指定 seq 封装一行 canonical 事件。
    pub fn from_event(seq: u64, event: DurableToolPolicyEvent) -> Self {
        Self {
            schema_version: TOOL_POLICY_AUDIT_SCHEMA_VERSION,
            seq,
            event,
        }
    }

    /// 行的事件类型文本。
    pub fn event_type(&self) -> &'static str {
        self.event.event_type()
    }
}

/// resume 决策（GC9 冻结）：spawn 前比对 (tool-policy digest, version, dialect)
/// 三元组；任一不一致或记录缺失 → 拒绝 resume、标记 superseded、新建会话。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResumeDecision {
    Resume,
    RejectSupersedeAndStartNew,
}

/// 审计 fixture：以冻结三元组构造 provider_start 记录（Task 3.3 resume 测试）。
pub fn provider_start_record(
    tool_policy_canonical_digest: &str,
    provider_version: &str,
    adapter_dialect: &str,
) -> ProviderStartAudit {
    ProviderStartAudit {
        provider: "codex".to_string(),
        role: "orchestrator".to_string(),
        workspace_session_id: "ws-fixture".to_string(),
        provider_session_id: "thread-1".to_string(),
        tool_policy_canonical_digest: tool_policy_canonical_digest.to_string(),
        argv: Vec::new(),
        sandbox: Some("read-only".to_string()),
        approval_policy: Some("on-request".to_string()),
        provider_version: provider_version.to_string(),
        adapter_dialect: adapter_dialect.to_string(),
    }
}

/// resume 冻结三元组精确比较：stored 缺失或任一不一致 → 拒绝并新建会话。
/// 注意比较的是 `tool_policy_canonical_digest`，与 gateway aggregate policy digest
/// 不混用（各自独立计算）。
pub fn resume_with_audit_record(
    stored: Option<ProviderStartAudit>,
    current: &ProviderStartAudit,
) -> ResumeDecision {
    match stored {
        Some(stored) if stored.resume_fingerprint() == current.resume_fingerprint() => {
            ResumeDecision::Resume
        }
        _ => ResumeDecision::RejectSupersedeAndStartNew,
    }
}

/// 读取坏行告警（内存形态；不写回 durable 分区）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolPolicyAuditReadWarning {
    pub reason_code: String,
    pub line_no: u32,
}

/// 读取结果：可解析事件 + 坏行告警。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ToolPolicyAuditReadResult {
    pub events: Vec<ToolPolicyAuditLine>,
    pub warnings: Vec<ToolPolicyAuditReadWarning>,
}

/// durable tool-policy 审计 sink（Task 3.2 接口冻结）。
///
/// 生产实现是 product 层 `LifecycleStore`（分区 `tool-policy-run-audit/`）；
/// engine 按 provider run 分配 `role_run_seq` 后以 [`RoleRunBoundAuditSink`]
/// 绑定标识传给 adapter，adapter 侧统一走 `append_bound`。
pub trait ToolPolicyAuditSink: Send + Sync {
    /// 以 `(workspace_session_id, role_run_seq)` 为文件 key 追加一条 canonical
    /// 事件。`provider_start` 必须是该文件首行且唯一；后续事件只能跟随其后。
    /// 写入失败必须返回错误（调用方沿 kill 链处理）。
    fn append(
        &self,
        workspace_session_id: &str,
        role_run_seq: u64,
        event: DurableToolPolicyEvent,
    ) -> Result<(), ToolPolicyAuditError>;

    /// run-bound 追加（adapter 侧入口）：sink 已绑定
    /// `(workspace_session_id, role_run_seq)`，adapter 不重复传标识。
    /// 默认 fail-closed：未绑定的裸 sink 拒绝写入。
    fn append_bound(&self, _event: DurableToolPolicyEvent) -> Result<(), ToolPolicyAuditError> {
        Err(ToolPolicyAuditError::UnboundSink)
    }

    /// resume 检索（Task 3.3）：按原生 provider session id 检索最近
    /// `provider_start` 审计记录。默认无实现（裸 sink 不支持检索）。
    fn find_provider_start(
        &self,
        _native_provider_session_id: &str,
    ) -> Result<Option<ProviderStartAudit>, ToolPolicyAuditError> {
        Ok(None)
    }

    /// run 维度的 provider_start 检索（按 workspace 分区扫描）。
    /// 默认无实现；生产由 `LifecycleStore` 覆写，bound 装饰器透传绑定 workspace。
    fn find_provider_start_by_session(
        &self,
        _workspace_session_id: &str,
        _native_provider_session_id: &str,
    ) -> Result<Option<ProviderStartAudit>, ToolPolicyAuditError> {
        Ok(None)
    }
}

/// 绑定 `(workspace_session_id, role_run_seq)` 的 sink 装饰器。
///
/// engine 按 provider run 分配并持久化 `role_run_seq`（分配即由 provider_start
/// 首行落盘留档），随后以本装饰器把标识传给 adapter；adapter 调用
/// `append_bound`/`find_provider_start`，不感知标识分配。
pub struct RoleRunBoundAuditSink {
    inner: Arc<dyn ToolPolicyAuditSink>,
    workspace_session_id: String,
    role_run_seq: u64,
}

impl RoleRunBoundAuditSink {
    pub fn new(
        inner: Arc<dyn ToolPolicyAuditSink>,
        workspace_session_id: impl Into<String>,
        role_run_seq: u64,
    ) -> Self {
        Self {
            inner,
            workspace_session_id: workspace_session_id.into(),
            role_run_seq,
        }
    }

    /// 转为 `Arc<dyn ToolPolicyAuditSink>` 注入 `StreamingProviderInput.audit_sink`。
    pub fn into_sink(self) -> Arc<dyn ToolPolicyAuditSink> {
        Arc::new(self)
    }
}

impl ToolPolicyAuditSink for RoleRunBoundAuditSink {
    fn append(
        &self,
        workspace_session_id: &str,
        role_run_seq: u64,
        event: DurableToolPolicyEvent,
    ) -> Result<(), ToolPolicyAuditError> {
        self.inner.append(workspace_session_id, role_run_seq, event)
    }

    fn append_bound(&self, event: DurableToolPolicyEvent) -> Result<(), ToolPolicyAuditError> {
        self.inner
            .append(&self.workspace_session_id, self.role_run_seq, event)
    }

    fn find_provider_start(
        &self,
        native_provider_session_id: &str,
    ) -> Result<Option<ProviderStartAudit>, ToolPolicyAuditError> {
        self.inner
            .find_provider_start_by_session(&self.workspace_session_id, native_provider_session_id)
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    //! 跨模块共享的审计测试 fixture（RecordingSink 与事件构造器）。

    use super::*;
    use std::sync::Mutex;

    /// 记录型 sink：按序记录 append_bound 调用；可注入指定次数后的失败；
    /// 可预置 provider_start 审计记录供 resume 检索（Task 3.3）。
    pub struct RecordingToolPolicyAuditSink {
        pub appends: Mutex<Vec<DurableToolPolicyEvent>>,
        pub fail_after: Option<usize>,
        stored_provider_starts: Mutex<Vec<ProviderStartAudit>>,
    }

    impl RecordingToolPolicyAuditSink {
        pub fn new() -> Arc<Self> {
            Arc::new(Self {
                appends: Mutex::new(Vec::new()),
                fail_after: None,
                stored_provider_starts: Mutex::new(Vec::new()),
            })
        }

        pub fn failing_after(count: usize) -> Arc<Self> {
            Arc::new(Self {
                appends: Mutex::new(Vec::new()),
                fail_after: Some(count),
                stored_provider_starts: Mutex::new(Vec::new()),
            })
        }

        /// 预置一条 provider_start 审计记录（resume 检索输入）。
        pub fn with_stored_provider_start(self: &Arc<Self>, record: ProviderStartAudit) {
            self.stored_provider_starts
                .lock()
                .expect("sink lock")
                .push(record);
        }

        pub fn events(&self) -> Vec<DurableToolPolicyEvent> {
            self.appends.lock().expect("sink lock").clone()
        }

        pub fn bound(self: Arc<Self>) -> Arc<dyn ToolPolicyAuditSink> {
            RoleRunBoundAuditSink::new(self, "ws-test", 0).into_sink()
        }
    }

    impl Default for RecordingToolPolicyAuditSink {
        fn default() -> Self {
            Self {
                appends: Mutex::new(Vec::new()),
                fail_after: None,
                stored_provider_starts: Mutex::new(Vec::new()),
            }
        }
    }

    impl ToolPolicyAuditSink for RecordingToolPolicyAuditSink {
        fn append(
            &self,
            _workspace_session_id: &str,
            _role_run_seq: u64,
            event: DurableToolPolicyEvent,
        ) -> Result<(), ToolPolicyAuditError> {
            self.append_bound(event)
        }

        fn append_bound(&self, event: DurableToolPolicyEvent) -> Result<(), ToolPolicyAuditError> {
            let mut appends = self.appends.lock().expect("sink lock");
            if let Some(limit) = self.fail_after
                && appends.len() >= limit
            {
                return Err(ToolPolicyAuditError::Io(
                    "injected append failure".to_string(),
                ));
            }
            appends.push(event);
            Ok(())
        }

        fn find_provider_start_by_session(
            &self,
            _workspace_session_id: &str,
            native_provider_session_id: &str,
        ) -> Result<Option<ProviderStartAudit>, ToolPolicyAuditError> {
            let stored = self.stored_provider_starts.lock().expect("sink lock");
            Ok(stored
                .iter()
                .rev()
                .find(|record| record.provider_session_id == native_provider_session_id)
                .cloned())
        }
    }
}
