/// C4 冷启动五步（依赖顺序固定，见计划 Global Constraints）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogicalCodebaseBootstrapStep {
    Identity,
    ManifestCheckout,
    RulesPolicy,
    MemberIndex,
    AggregateIndexActive,
}

impl LogicalCodebaseBootstrapStep {
    /// Canonical five-step order for the logical codebase cold-start chain.
    pub const V1: [Self; 5] = [
        Self::Identity,
        Self::ManifestCheckout,
        Self::RulesPolicy,
        Self::MemberIndex,
        Self::AggregateIndexActive,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Identity => "identity",
            Self::ManifestCheckout => "manifest_checkout",
            Self::RulesPolicy => "rules_policy",
            Self::MemberIndex => "member_index",
            Self::AggregateIndexActive => "aggregate_index_active",
        }
    }
}

/// 步骤状态投影（来自 durable facts，非独立状态机）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogicalCodebaseBootstrapStepStatus {
    NotStarted,
    Running,
    Completed,
    Failed,
    WaitingForHuman,
}

/// 既有 durable record 上的 checkpoint 证据（input digest / output artifact /
/// expected membership revision）。只读投影，不新写文件。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BootstrapCheckpoint {
    pub object_id: String,
    pub input_digest: Option<String>,
    pub output_artifact_ref: Option<String>,
    pub expected_membership_revision: Option<u64>,
    pub completed_at: Option<String>,
}

/// 失败/等待原因投影：reason_code 稳定，external_side_effect 描述可能的
/// provider/index 外部副作用（"none" 表示纯内部 durable 事实）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BootstrapFailure {
    pub reason_code: String,
    pub detail: String,
    pub retryable: bool,
    pub external_side_effect: String,
}

/// 单个冷启动步骤的只读投影。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BootstrapStepProjection {
    pub step: LogicalCodebaseBootstrapStep,
    pub status: LogicalCodebaseBootstrapStepStatus,
    pub object_id: String,
    pub checkpoint: Option<BootstrapCheckpoint>,
    pub failure: Option<BootstrapFailure>,
    pub allowed_actions: Vec<BootstrapActionKind>,
}

/// 通知 DTO（C4 Task 9 首定义于此，供 projection/inbox 复用）：事实先落盘、
/// 通知后投影；notice key/object/action 稳定，前端据此去重。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LogicalCodebaseBootstrapNotice {
    pub key: String,
    pub step: LogicalCodebaseBootstrapStep,
    pub object_id: String,
    pub reason_code: String,
    pub summary: String,
    pub external_side_effect: String,
    pub allowed_actions: Vec<BootstrapActionKind>,
    pub next_step: Option<LogicalCodebaseBootstrapStep>,
    pub created_at: String,
}

/// LC 冷启动总投影：GET `/logical-codebases/{lc_id}/bootstrap` 的唯一来源。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LogicalCodebaseBootstrapProjection {
    pub project_id: String,
    pub logical_codebase_id: String,
    pub authority_root: PathBuf,
    pub membership_revision: Option<u64>,
    pub policy: Option<AuthorityPolicyReference>,
    pub steps: Vec<BootstrapStepProjection>,
    pub planning_ready: bool,
    pub notices: Vec<LogicalCodebaseBootstrapNotice>,
}

/// 显式 bootstrap 动作请求：command_id + 目标身份 + expected
/// revision/object identity；同一命令键重放必须返回同一 durable 结果。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BootstrapActionRequest {
    pub command_id: String,
    pub project_id: String,
    pub logical_codebase_id: String,
    pub step: LogicalCodebaseBootstrapStep,
    pub action: BootstrapActionKind,
    pub expected_revision: Option<u64>,
    pub expected_object_id: String,
}

/// 动作结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BootstrapActionOutcome {
    Accepted,
    Replayed,
    WaitingForHuman,
    Completed,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BootstrapActionResult {
    pub command_id: String,
    pub outcome: BootstrapActionOutcome,
    pub projection: LogicalCodebaseBootstrapProjection,
}

/// bootstrap 动作错误：结构化 conflict（稳定码）或底层 store 错误。
#[derive(Debug)]
pub enum BootstrapActionError {
    Conflict { code: String, detail: String },
    Store(ProductStoreError),
}

impl std::fmt::Display for BootstrapActionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Conflict { code, detail } => write!(formatter, "{code}: {detail}"),
            Self::Store(error) => write!(formatter, "{error}"),
        }
    }
}

impl From<ProductStoreError> for BootstrapActionError {
    fn from(error: ProductStoreError) -> Self {
        Self::Store(error)
    }
}
