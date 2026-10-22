use serde::{Deserialize, Serialize};

use super::review::{ReviewFinding, ReviewVerdict};

/// REQ-GFR-CR-03：issue 级跨仓最终评审的**逐仓分段**。
///
/// 一段 = 该 issue 下一个已完成 attempt（group attempt 一仓一段，单 WI attempt
/// 一仓一段）。字段集严格对齐设计 B §四 B1：仓标识 + attempt + 分支 + diff 区间 +
/// 覆盖的 work item + 该段 diff 是否被截断。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrossRepoRepositorySection {
    pub repository_name: String,
    pub logical_repository_id: String,
    pub attempt_id: String,
    pub branch_name: String,
    pub base_commit: String,
    pub head_commit: String,
    #[serde(default)]
    pub work_item_ids: Vec<String>,
    #[serde(default)]
    pub diff_truncated: bool,
}

/// REQ-GFR-CR-03：跨仓最终评审**记录状态**。
///
/// advisory 语义（REQ-GFR-CR-05）：`Failed` / `Skipped` 都**不**阻断交付、
/// 不自动重试。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueCrossRepoReviewStatus {
    Completed,
    Failed,
    Skipped,
}

/// REQ-GFR-CR-03：issue 级跨仓最终评审记录（**按 issue 唯一**）。
///
/// 字段集严格对齐设计 B §四 B1。`findings` 即 REQ-GFR-CR-07 中跨仓 findings 的
/// 载体：每条 finding 的 `source_stage` 恒为
/// `CodingExecutionStage::IssueCrossRepoReview`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IssueCrossRepoReview {
    pub id: String,
    pub project_id: String,
    pub issue_id: String,
    pub plan_id: String,
    pub verdict: ReviewVerdict,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub findings: Vec<ReviewFinding>,
    #[serde(default)]
    pub repository_sections: Vec<CrossRepoRepositorySection>,
    pub status: IssueCrossRepoReviewStatus,
    #[serde(default)]
    pub failure_reason: Option<String>,
    #[serde(default)]
    pub raw_provider_output_ref: Option<String>,
    #[serde(default)]
    pub created_at: String,
}

/// REQ-GFR-CR-06：「已知风险，仍然确认」留痕记录。
///
/// 字段集严格对齐设计 B §六 D：**无** `acknowledged_by`（确认人身份）与
/// `acknowledged_channel`（确认通道）——留痕只回答「我已知道风险，仍然确认」。
/// `id` / `project_id` / `issue_id` / `plan_id` / `attempt_id` 是结构键
/// （落盘路径 + 幂等键），不是确认人/通道信息。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IssueCrossRepoRiskAcknowledgement {
    pub id: String,
    pub project_id: String,
    pub issue_id: String,
    pub plan_id: String,
    pub attempt_id: String,
    pub cross_repo_review_id: String,
    pub verdict: ReviewVerdict,
    #[serde(default)]
    pub findings_summary: String,
    #[serde(default)]
    pub acknowledged_at: String,
}
