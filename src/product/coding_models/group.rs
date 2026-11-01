use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingExecutionUnitStatus {
    Pending,
    Running,
    WaitingForHuman,
    Completed,
    Failed,
    Blocked,
    BlockedByPlanDefect,
    AwaitingAmendment,
    NeedsRevalidation,
    Stale,
    Superseded,
    Skipped,
}

impl CodingExecutionUnitStatus {
    pub fn is_active(&self) -> bool {
        matches!(
            self,
            Self::Running
                | Self::WaitingForHuman
                | Self::Blocked
                | Self::BlockedByPlanDefect
                | Self::AwaitingAmendment
                | Self::NeedsRevalidation
                | Self::Stale
        )
    }

    /// F-44 fix1：group 继任 unit 的候选谓词——`Pending` 之外**必须**含中止/失败归一
    /// 出的 `Skipped`/`Failed`。终态重启只复位「首个」remainder；若继任选择器只认
    /// `Pending`，则中止在第 k<n 个 unit 的主流形态下 unit_{k+1..n} 会被永久放弃
    /// （final confirm 永不满足、非终态又拒 RestartCoding → 死胡同）。
    /// 复位 `unit_k` 完成后，选择器按同一谓词接续复活 `unit_{k+1}`（走既有
    /// `start_pending_coding_unit_run` 链路）。`Superseded`（被修订取代）与
    /// `Completed` 一律不是候选；依赖语义与 `Pending` 候选同构（SC 路径仍需依赖就绪）。
    pub fn is_group_remainder_candidate(&self) -> bool {
        matches!(self, Self::Pending | Self::Skipped | Self::Failed)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingExecutionUnit {
    pub id: String,
    pub attempt_id: String,
    pub project_id: String,
    pub issue_id: String,
    pub plan_id: String,
    pub logical_work_item_id: String,
    pub work_item_revision_id: String,
    pub dependency_logical_work_item_ids: Vec<String>,
    pub order_index: u32,
    pub status: CodingExecutionUnitStatus,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub latest_handoff_revision_id: Option<String>,
    pub completion_commit: Option<String>,
    pub summary: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}
