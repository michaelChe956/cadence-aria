//! C4 Task 3/6：LC 冷启动 bootstrap 投影的 HTTP DTO。
//!
//! 只投影 `LogicalCodebaseBootstrapProjection`/`BootstrapActionResult` 的
//! durable 事实；字段与 product 类型一一对应（snake_case），不另造状态机。

use serde::{Deserialize, Serialize};

use crate::product::logical_codebase::bootstrap::{
    BootstrapActionResult as ServiceResult, LogicalCodebaseBootstrapProjection as Projection,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct BootstrapCheckpointDto {
    pub object_id: String,
    pub input_digest: Option<String>,
    pub output_artifact_ref: Option<String>,
    pub expected_membership_revision: Option<u64>,
    pub completed_at: Option<String>,
}

/// 失败面三段成品（REQ-BOOT-08 / REQ-LAYOUT-02）的 HTTP 形态。
///
/// 五字段与 `product::logical_codebase::failure_copy::FailureCopy` 一一对应，
/// **全部必填非 null**：`reason_zh` 不是 `Option`，前端不得把缺失当降级路径。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct FailureCopyDto {
    pub title_zh: String,
    pub reason_zh: String,
    pub action_zh: String,
    pub triage: String,
    pub machine_line: String,
}

impl From<&crate::product::logical_codebase::failure_copy::FailureCopy> for FailureCopyDto {
    fn from(copy: &crate::product::logical_codebase::failure_copy::FailureCopy) -> Self {
        Self {
            title_zh: copy.title_zh.clone(),
            reason_zh: copy.reason_zh.clone(),
            action_zh: copy.action_zh.clone(),
            triage: copy.triage.as_str().to_string(),
            machine_line: copy.machine_line.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct BootstrapFailureDto {
    pub reason_code: String,
    pub detail: String,
    pub retryable: bool,
    pub external_side_effect: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub triage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub copy: Option<FailureCopyDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct BootstrapStepProjectionDto {
    pub step: String,
    pub status: String,
    pub object_id: String,
    pub checkpoint: Option<BootstrapCheckpointDto>,
    pub failure: Option<BootstrapFailureDto>,
    pub allowed_actions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct LogicalCodebaseBootstrapNoticeDto {
    pub key: String,
    pub step: String,
    pub object_id: String,
    pub reason_code: String,
    pub summary: String,
    pub external_side_effect: String,
    pub allowed_actions: Vec<String>,
    pub next_step: Option<String>,
    pub created_at: String,
}

/// A（REQ-REG-10）：最近一次 advance/手动「准备」的可解释停止。仅当被停
/// 步骤仍 `not_started` 时出现；`reason_code` 与 failure_copy 同码表
/// （provider 选择门/信任门/配方终态等各有稳定码），`detail` 是已脱敏的
/// 原始拒绝上下文。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct BootstrapAdvanceStopDto {
    pub step: String,
    pub reason: String,
    pub reason_code: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub stopped_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct LogicalCodebaseBootstrapProjectionDto {
    pub project_id: String,
    pub logical_codebase_id: String,
    pub authority_root: String,
    pub membership_revision: Option<u64>,
    pub policy: Option<BootstrapPolicyReferenceDto>,
    pub steps: Vec<BootstrapStepProjectionDto>,
    pub planning_ready: bool,
    pub notices: Vec<LogicalCodebaseBootstrapNoticeDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub advance_stop: Option<BootstrapAdvanceStopDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct BootstrapPolicyReferenceDto {
    pub policy_id: String,
    pub policy_revision: u64,
    pub policy_digest: String,
    pub artifact_root: String,
}

impl From<&Projection> for LogicalCodebaseBootstrapProjectionDto {
    fn from(projection: &Projection) -> Self {
        Self {
            project_id: projection.project_id.clone(),
            logical_codebase_id: projection.logical_codebase_id.clone(),
            authority_root: projection.authority_root.to_string_lossy().into_owned(),
            membership_revision: projection.membership_revision,
            policy: projection.policy.as_ref().map(|policy| {
                BootstrapPolicyReferenceDto {
                    policy_id: policy.policy_id.clone(),
                    policy_revision: policy.policy_revision,
                    policy_digest: policy.policy_digest.clone(),
                    artifact_root: policy.artifact_root.to_string_lossy().into_owned(),
                }
            }),
            steps: projection
                .steps
                .iter()
                .map(|step| BootstrapStepProjectionDto {
                    step: step.step.as_str().to_string(),
                    status: match step.status {
                        crate::product::logical_codebase::bootstrap::LogicalCodebaseBootstrapStepStatus::NotStarted => "not_started",
                        crate::product::logical_codebase::bootstrap::LogicalCodebaseBootstrapStepStatus::Running => "running",
                        crate::product::logical_codebase::bootstrap::LogicalCodebaseBootstrapStepStatus::Completed => "completed",
                        crate::product::logical_codebase::bootstrap::LogicalCodebaseBootstrapStepStatus::Failed => "failed",
                        crate::product::logical_codebase::bootstrap::LogicalCodebaseBootstrapStepStatus::WaitingForHuman => "waiting_for_human",
                    }
                    .to_string(),
                    object_id: step.object_id.clone(),
                    checkpoint: step.checkpoint.as_ref().map(|checkpoint| {
                        BootstrapCheckpointDto {
                            object_id: checkpoint.object_id.clone(),
                            input_digest: checkpoint.input_digest.clone(),
                            output_artifact_ref: checkpoint.output_artifact_ref.clone(),
                            expected_membership_revision: checkpoint
                                .expected_membership_revision,
                            completed_at: checkpoint.completed_at.clone(),
                        }
                    }),
                    failure: step.failure.as_ref().map(|failure| BootstrapFailureDto {
                        reason_code: failure.reason_code.clone(),
                        detail: failure.detail.clone(),
                        retryable: failure.retryable,
                        external_side_effect: failure.external_side_effect.clone(),
                        triage: failure.triage.clone(),
                        // 纯搬运：后端已渲染好，前端不再自拼文案。
                        copy: failure.copy.as_ref().map(FailureCopyDto::from),
                    }),
                    allowed_actions: step
                        .allowed_actions
                        .iter()
                        .map(|action| action_str(*action))
                        .collect(),
                })
                .collect(),
            advance_stop: projection.advance_stop.as_ref().map(|stop| {
                BootstrapAdvanceStopDto {
                    step: stop.step.as_str().to_string(),
                    reason: stop.reason.clone(),
                    reason_code: stop.reason_code.clone(),
                    detail: stop.detail.clone(),
                    stopped_at: stop.stopped_at.clone(),
                }
            }),
            planning_ready: projection.planning_ready,
            notices: projection
                .notices
                .iter()
                .map(|notice| LogicalCodebaseBootstrapNoticeDto {
                    key: notice.key.clone(),
                    step: notice.step.as_str().to_string(),
                    object_id: notice.object_id.clone(),
                    reason_code: notice.reason_code.clone(),
                    summary: notice.summary.clone(),
                    external_side_effect: notice.external_side_effect.clone(),
                    allowed_actions: notice
                        .allowed_actions
                        .iter()
                        .map(|action| action_str(*action))
                        .collect(),
                    next_step: notice.next_step.map(|step| step.as_str().to_string()),
                    created_at: notice.created_at.clone(),
                })
                .collect(),
        }
    }
}

fn action_str(action: crate::product::logical_codebase::BootstrapActionKind) -> String {
    match action {
        crate::product::logical_codebase::BootstrapActionKind::Prepare => "prepare",
        crate::product::logical_codebase::BootstrapActionKind::Continue => "continue",
        crate::product::logical_codebase::BootstrapActionKind::Retry => "retry",
        crate::product::logical_codebase::BootstrapActionKind::Revalidate => "revalidate",
        crate::product::logical_codebase::BootstrapActionKind::Repair => "repair",
        crate::product::logical_codebase::BootstrapActionKind::StartGeneration => {
            "start_generation"
        }
        crate::product::logical_codebase::BootstrapActionKind::Republish => "republish",
    }
    .to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct BootstrapActionResultDto {
    pub command_id: String,
    pub outcome: String,
    pub projection: LogicalCodebaseBootstrapProjectionDto,
}

impl From<&ServiceResult> for BootstrapActionResultDto {
    fn from(result: &ServiceResult) -> Self {
        Self {
            command_id: result.command_id.clone(),
            outcome: match result.outcome {
                crate::product::logical_codebase::bootstrap::BootstrapActionOutcome::Accepted => {
                    "accepted"
                }
                crate::product::logical_codebase::bootstrap::BootstrapActionOutcome::Replayed => {
                    "replayed"
                }
                crate::product::logical_codebase::bootstrap::BootstrapActionOutcome::WaitingForHuman => {
                    "waiting_for_human"
                }
                crate::product::logical_codebase::bootstrap::BootstrapActionOutcome::Completed => {
                    "completed"
                }
            }
            .to_string(),
            projection: LogicalCodebaseBootstrapProjectionDto::from(&result.projection),
        }
    }
}

/// A（REQ-REG-10 自动推进）响应：本次真正推进了哪些步骤 + 停在哪里 +
/// 最终投影（与 GET 完全同构——前端不需要再发一次 GET）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct BootstrapAdvanceResultDto {
    pub project_id: String,
    pub logical_codebase_id: String,
    /// 本次真正产生 durable 进展的步骤名（依赖顺序；幂等重放时为空）。
    pub advanced_steps: Vec<String>,
    /// `planning_ready` / `in_progress` / `failed` / `waiting_for_human` /
    /// `no_actionable_step` / `iteration_limit`。
    pub stopped_reason: String,
    pub stopped_step: Option<String>,
    /// 停住步骤的稳定原因码（来自既有投影 failure 或推进器自身稳定码）。
    pub stopped_reason_code: Option<String>,
    /// 停住步骤的可读上下文（已脱敏；等待人工停止时携带，如根配方启动链
    /// 被拒的具体原因——provider 选择门/信任门各自的可区分原因）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stopped_detail: Option<String>,
    pub projection: LogicalCodebaseBootstrapProjectionDto,
}

impl From<&crate::product::logical_codebase::BootstrapAdvanceResult> for BootstrapAdvanceResultDto {
    fn from(result: &crate::product::logical_codebase::BootstrapAdvanceResult) -> Self {
        Self {
            project_id: result.project_id.clone(),
            logical_codebase_id: result.logical_codebase_id.clone(),
            advanced_steps: result
                .advanced_steps
                .iter()
                .map(|attempt| attempt.step.as_str().to_string())
                .collect(),
            stopped_reason: result.stopped.reason().to_string(),
            stopped_step: result.stopped.step().map(|step| step.as_str().to_string()),
            stopped_reason_code: result.stopped.reason_code().map(str::to_string),
            stopped_detail: result.stopped.detail().map(str::to_string),
            projection: LogicalCodebaseBootstrapProjectionDto::from(&result.projection),
        }
    }
}

/// C4 Task 6：显式 bootstrap 动作请求（canonical POST body）。step/action
/// 直接复用 product 枚举的 snake_case 序列化，不引入第二套字符串协议。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BootstrapActionRequestDto {
    pub command_id: String,
    pub step: crate::product::logical_codebase::LogicalCodebaseBootstrapStep,
    pub action: crate::product::logical_codebase::BootstrapActionKind,
    pub expected_revision: Option<u64>,
    pub expected_object_id: String,
}
