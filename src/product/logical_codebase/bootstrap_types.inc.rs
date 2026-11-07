use crate::product::logical_codebase::failure_copy::{
    describe, describe_code, render_detail, FailureCopy, FailureInput,
};

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
///
/// `copy` 是同源成品文案（`failure_copy`）的载体：一旦存在，其五个字段全部
/// 必填；`triage` 只从 `copy.triage` 派生，**不另存第二份分诊真值**。
/// 外层 `copy`/`triage` 用 `Option` 只为兼容历史非失败载体（旧响应没有
/// 这两个键），新增的已知失败一律为 `Some`。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BootstrapFailure {
    pub reason_code: String,
    pub detail: String,
    pub retryable: bool,
    pub external_side_effect: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub triage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub copy: Option<FailureCopy>,
}

impl BootstrapFailure {
    /// 唯一构造入口：`copy` 与 `triage` 一律由同一文案来源派生，
    /// 不存在「加了失败却漏了文案」的路径。
    pub fn new(
        reason_code: String,
        detail: String,
        retryable: bool,
        external_side_effect: String,
    ) -> Self {
        let copy = describe_code(&reason_code, Some(detail.as_str()));
        Self::with_copy(reason_code, detail, retryable, external_side_effect, copy)
    }

    /// 已持有成品 copy（例如来自 `AggregateInitializationErrorRecord` 的
    /// 适配输入）时用它，避免二次查表得到与渲染不一致的文案。
    pub fn with_copy(
        reason_code: String,
        detail: String,
        retryable: bool,
        external_side_effect: String,
        copy: FailureCopy,
    ) -> Self {
        let triage = Some(copy.triage.as_str().to_string());
        Self {
            reason_code,
            detail,
            retryable,
            external_side_effect,
            triage,
            copy: Some(copy),
        }
    }
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
/// A（REQ-REG-10）：advance/手动「准备」停止解释的 durable 记录（LC scope
/// `.bootstrap-advance-stop.json`）。首建链停在等待人工/失败/无可动步骤时
/// 由推进收口落盘；PlanningReady/进行中收口时清除。被停步骤本身往往没有
/// failure（那是「未开始」，不是失败），GET 投影靠它解释「为什么停在这一步
/// +下一步做什么」；文案沿用 failure_copy 同一码表。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BootstrapAdvanceStopRecord {
    pub step: LogicalCodebaseBootstrapStep,
    /// 稳定停止名（`BootstrapAdvanceStop::reason()`）。
    pub reason: String,
    /// 停住步骤的稳定原因码（与 failure_copy 同码表）。
    pub reason_code: String,
    /// 可读上下文（如根配方启动链被拒的具体原因），写入前已脱敏。
    #[serde(default)]
    pub detail: Option<String>,
    pub stopped_at: String,
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
    /// 最近一次 advance/手动「准备」的可解释停止；仅当该步骤仍投影
    /// `not_started`（没有任何更新的 durable 事实接管解释权）时透出，
    /// 否则为 `None`。GET 只读它，不写。
    #[serde(default)]
    pub advance_stop: Option<BootstrapAdvanceStopRecord>,
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
