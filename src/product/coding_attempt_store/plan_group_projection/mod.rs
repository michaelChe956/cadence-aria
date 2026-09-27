//! REQ-MTG-04（WP3）：plan 级 group 聚合只读投影。
//!
//! 三值终态判据（约束 9 定案，k3 F4 可测化）：
//! - **已达 provider 启动** = attempt 离开初始二元组（`StartCoding` 是唯一
//!   入口，离开必经 `admit_and_transition_attempt_to_executable`）；
//!   `(Aborted, PrepareContext)` **双形态消歧**：pre-start abort（无执行
//!   证据）未达（k3 fix round 1）；多 unit group attempt 在 unit 间推进时
//!   `advance_to_next_group_unit` 把 stage 回退 PrepareContext，其后 abort
//!   落盘同形态但已执行过 provider——取该 attempt 的 role_runs 非空或
//!   head_commit 在场为执行铁证，证据在场即算已达（k3 fix round 2）；
//! - **未启**（判定优先于「部分」）：无 target-attempt，或全部 target-attempt
//!   均未达 provider 启动（`(Created, PrepareContext)` 未触碰形态或无执行
//!   证据的 pre-start abort 形态）；
//! - **全部交付**：存在 target-attempt 且每 target 最新 attempt
//!   `Completed` 且最新 ReviewRequest `Pushed`——对齐
//!   `compute_issue_delivery_summary` 口径（issue_delivery.rs:110-113）；
//! - **部分**：其余一切（MUST NOT 伪装全局成功）。
//!
//! 只读派生（无第二状态机）：每次读取从 attempt/ReviewRequest durable 事实
//! 现算，不落任何持久化聚合记录。无快照存量 attempt 不进 per-target 投影
//! （D2.1 A4——按 issue 级 attempt 列表既有形态呈现，聚合视图不猜测归属）。

use std::collections::BTreeMap;

use crate::product::coding_models::{
    CodingAttemptStatus, CodingExecutionAttempt, CodingExecutionStage, PushStatus, ReviewRequest,
};
use crate::product::issue_store::IssueStore;
use crate::product::json_store::{ProductStoreError, validate_relative_id};
use crate::product::logical_codebase::LogicalRepositoryId;
use crate::product::project_store::ProjectStore;
use crate::product::repository_store::RepositoryStore;

/// plan 级 group 聚合终态三值。
///
/// 与 issue 级 `IssueDeliveryOverall` 的口径差异（k3 §2.3）：plan 级「未启」
/// ≠ issue 级 `None`（无条目）——plan 级未启=无 target-attempt 或全部未达
/// provider 启动（未启优先于「部分」），故 DTO 序列化不复用 `"none"`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanGroupOverall {
    /// 每 target 最新 attempt `Completed` 且最新 ReviewRequest `Pushed`。
    AllDelivered,
    /// 任一 target 已达 provider 启动且非全部交付（含「部分启动+部分未启」）。
    Partial,
    /// 无 target-attempt，或全部 target-attempt 均未离开 `(Created, PrepareContext)`。
    NotStarted,
}

/// 单个 target 的只读投影条目（按最新 target-attempt 派生）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanTargetEntry {
    pub target_repository_id: LogicalRepositoryId,
    /// 展示名（`resolve_repository_name` 同款解析：logical id 经 strict 解析取
    /// checkout 路径末段目录名，解析不出末段回落 id 字符串）。
    pub repository_name: String,
    pub attempt_id: Option<String>,
    pub attempt_status: Option<CodingAttemptStatus>,
    pub stage: Option<CodingExecutionStage>,
    pub branch_name: Option<String>,
    pub head_commit: Option<String>,
    /// 取自最新 attempt 的最新 ReviewRequest；`None` 表示无 ReviewRequest。
    pub push_status: Option<PushStatus>,
    pub review_request_id: Option<String>,
    /// 失败/阻塞原因（只呈现不判定，OQ3 定案派生规则）：
    /// `manual_recovery_reason` 优先 → `push_error` → 失败态 status 文本。
    pub blocked_reason: Option<String>,
}

/// plan 级 group 聚合只读投影结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanGroupProjection {
    pub project_id: String,
    pub issue_id: String,
    pub plan_id: String,
    pub entries: Vec<PlanTargetEntry>,
    pub overall: PlanGroupOverall,
}

/// `blocked_reason` 派生规则（OQ3 定案，只呈现不判定）：
/// `manual_recovery_reason` 优先 → 最新 ReviewRequest 的 `push_error` →
/// 失败态 status 文本（`Failed`/`Aborted`/`AmendmentApplyFailed`，与 DTO
/// status 文本一致的 snake_case）。
fn plan_target_blocked_reason(
    attempt: &CodingExecutionAttempt,
    latest_review: Option<&ReviewRequest>,
) -> Option<String> {
    if let Some(reason) = attempt.manual_recovery_reason.as_ref() {
        return Some(reason.clone());
    }
    if let Some(error) = latest_review.and_then(|review| review.push_error.as_ref()) {
        return Some(error.clone());
    }
    match attempt.status {
        CodingAttemptStatus::Failed => Some("failed".to_string()),
        CodingAttemptStatus::Aborted => Some("aborted".to_string()),
        CodingAttemptStatus::AmendmentApplyFailed => Some("amendment_apply_failed".to_string()),
        _ => None,
    }
}

impl super::CodingAttemptStore {
    /// 计算某个 plan 的 group 级聚合只读投影（REQ-MTG-04，无写入）。
    ///
    /// 判定语义（约束 9 定案）：每 target 取最新 attempt（按 `attempt_no`
    /// 升序末元素，与 `compute_issue_delivery_summary` 的最新口径一致）；
    /// `status == Completed` 且最新 ReviewRequest `push_status == Pushed`
    /// 才算该 target 已交付。三值判定见模块文档。
    pub fn compute_plan_group_projection(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
    ) -> Result<PlanGroupProjection, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(plan_id)?;
        // 同 compute_issue_delivery_summary：先读 issue 确认存在，避免对不存在的 issue 返回空投影。
        IssueStore::new(self.paths()).get(project_id, issue_id)?;

        let attempts = self.list_attempts_for_work_item_group(project_id, issue_id, plan_id)?;

        // D2.1 A4：无快照 attempt 不入任何 target 桶——聚合投影不猜测归属。
        let mut by_target: BTreeMap<LogicalRepositoryId, Vec<&CodingExecutionAttempt>> =
            BTreeMap::new();
        for attempt in &attempts {
            if let Some(snapshot) = attempt.target_snapshot.as_ref() {
                by_target
                    .entry(snapshot.logical_repository_id)
                    .or_default()
                    .push(attempt);
            }
        }

        let mut entries = Vec::with_capacity(by_target.len());
        let mut all_delivered = true;
        let mut any_target_started = false;
        for (target, target_attempts) in &by_target {
            // list_attempts_for_work_item_group 已按 (attempt_no, id) 升序——末元素为最新。
            let latest = target_attempts
                .last()
                .expect("target bucket is never empty");
            // 已达 provider 启动 = 该 target 任一 attempt 已达（历史事实口径，
            // 判据见 attempt_reached_provider——k3 fix round 1/2 消歧版）。
            let mut target_started = false;
            for attempt in target_attempts {
                if self.attempt_reached_provider(project_id, issue_id, attempt)? {
                    target_started = true;
                    break;
                }
            }
            let latest_review = self
                .list_review_requests(project_id, issue_id, &latest.id)?
                .into_iter()
                .last();
            let delivered = latest.status == CodingAttemptStatus::Completed
                && latest_review
                    .as_ref()
                    .is_some_and(|review| review.push_status == PushStatus::Pushed);
            all_delivered = all_delivered && delivered;
            any_target_started = any_target_started || target_started;

            let repository_name = self.resolve_plan_target_repository_name(project_id, *target)?;
            entries.push(PlanTargetEntry {
                target_repository_id: *target,
                repository_name,
                attempt_id: Some(latest.id.clone()),
                attempt_status: Some(latest.status.clone()),
                stage: Some(latest.stage.clone()),
                branch_name: Some(latest.branch_name.clone()),
                head_commit: latest.head_commit.clone(),
                push_status: latest_review
                    .as_ref()
                    .map(|review| review.push_status.clone()),
                review_request_id: latest_review.as_ref().map(|review| review.id.clone()),
                blocked_reason: plan_target_blocked_reason(latest, latest_review.as_ref()),
            });
        }

        let overall = if entries.is_empty() {
            PlanGroupOverall::NotStarted
        } else if all_delivered {
            PlanGroupOverall::AllDelivered
        } else if !any_target_started {
            // 未启优先于部分（约束 9）：全部 target-attempt 均未离开初始二元组。
            PlanGroupOverall::NotStarted
        } else {
            PlanGroupOverall::Partial
        };

        Ok(PlanGroupProjection {
            project_id: project_id.to_string(),
            issue_id: issue_id.to_string(),
            plan_id: plan_id.to_string(),
            entries,
            overall,
        })
    }

    /// 该 attempt 是否已达 provider 启动（k3 fix round 1/2 消歧版判据）。
    ///
    /// - stage 已离开 `PrepareContext` → 已达（含启动后 abort）；
    /// - `(Created, PrepareContext)` → 未达（未触碰，`StartCoding` 唯一入口）；
    /// - 其余 PrepareContext 停留态（非 Created 非 Aborted）→ 已达：非 Created
    ///   状态均经 admission，PrepareContext 停留只可能来自 unit 间 stage 回退；
    /// - `(Aborted, PrepareContext)` **双形态消歧**（fix round 1 排除 pre-start
    ///   abort；fix round 2 修正误排）：pre-start abort（Created→Aborted 白名单
    ///   转换、abort 不改 stage、无执行证据）未达；多 unit group attempt 在
    ///   unit 间推进时 `advance_to_next_group_unit` 会把 stage 回退
    ///   PrepareContext（coding_workspace_engine/group.rs:315），其后 abort
    ///   落盘同形态但已执行过 provider——取 durable 执行证据消歧：该 attempt
    ///   的 role_runs 非空或 head_commit 在场即算已达（只读派生面，两者均在
    ///   attempt record/子目录可读）。
    fn attempt_reached_provider(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt: &CodingExecutionAttempt,
    ) -> Result<bool, ProductStoreError> {
        if attempt.stage != CodingExecutionStage::PrepareContext {
            return Ok(true);
        }
        match attempt.status {
            CodingAttemptStatus::Created => Ok(false),
            CodingAttemptStatus::Aborted => Ok(!self
                .list_role_runs(project_id, issue_id, &attempt.id)?
                .is_empty()
                || attempt.head_commit.is_some()),
            _ => Ok(true),
        }
    }

    /// 解析 plan target 的仓展示名（issue_delivery.rs `resolve_repository_name`
    /// 同款）：logical id 经 `resolve_logical_repository_strict` 取 checkout 路径
    /// 末段目录名；解析不出末段时回落 logical id 字符串本身。
    fn resolve_plan_target_repository_name(
        &self,
        project_id: &str,
        target: LogicalRepositoryId,
    ) -> Result<String, ProductStoreError> {
        let paths = self.paths();
        let project = ProjectStore::new(paths.clone()).get(project_id)?;
        let (_, checkout, _) = RepositoryStore::for_project(paths, &project)
            .resolve_logical_repository_strict(project_id, target)?;
        if let Some(name) = checkout
            .canonical_path
            .file_name()
            .map(|value| value.to_string_lossy().to_string())
            .filter(|value| !value.is_empty())
        {
            return Ok(name);
        }
        Ok(target.0.to_string())
    }
}


#[cfg(test)]
mod tests;
