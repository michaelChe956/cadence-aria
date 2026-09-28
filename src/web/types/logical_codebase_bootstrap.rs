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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct BootstrapFailureDto {
    pub reason_code: String,
    pub detail: String,
    pub retryable: bool,
    pub external_side_effect: String,
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
                    }),
                    allowed_actions: step
                        .allowed_actions
                        .iter()
                        .map(|action| action_str(*action))
                        .collect(),
                })
                .collect(),
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
