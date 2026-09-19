use crate::product::coding_models::{CodingAttemptStatus, CodingExecutionAttempt, PushStatus};
use crate::product::issue_store::IssueStore;
use crate::product::json_store::{ProductStoreError, validate_relative_id};
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::LifecycleWorkItemRecord;
use crate::product::project_store::ProjectStore;
use crate::product::repository_store::RepositoryStore;

/// Issue 级交付状态的三种整体判定。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueDeliveryOverall {
    /// 每个 Work Item 的最新 attempt 均 `Completed` 且最新 ReviewRequest 均 `Pushed`。
    AllPushed,
    /// 至少一个 Work Item 未满足「已推送」条件（含无 attempt / attempt 未完成 / push 失败）。
    Partial,
    /// Issue 下没有任何 Work Item。
    None,
}

/// 单个 Work Item 的交付状态投影。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryEntry {
    pub repository_name: String,
    pub work_item_id: String,
    /// `None` 表示该 Work Item 没有任何 attempt。
    pub attempt_status: Option<CodingAttemptStatus>,
    /// 取自最新 attempt 的 `branch_name`；无 attempt 时为 `None`。
    pub branch_name: Option<String>,
    /// 取自最新 attempt 的 `head_commit`；无 attempt 或尚未落盘时为 `None`。
    pub commit_sha: Option<String>,
    /// 取自最新 ReviewRequest 的 `push_status`；`None` 表示无 ReviewRequest。
    pub push_status: Option<PushStatus>,
    pub push_error: Option<String>,
}

/// Issue 级交付状态聚合结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueDeliverySummary {
    pub project_id: String,
    pub issue_id: String,
    pub entries: Vec<DeliveryEntry>,
    pub overall: IssueDeliveryOverall,
}

impl super::CodingAttemptStore {
    /// 计算某个 issue 的交付状态聚合。
    ///
    /// 判定语义（每 Work Item 取 [`Self::list_attempts_covering_work_item`]
    /// 末元素为最新 attempt；ReviewRequest 取 `list_review_requests` 末元素的
    /// `push_status`）：`attempt_status == Completed` 且
    /// `push_status == Some(Pushed)` 才算已交付。全部条目满足 → `AllPushed`；
    /// 有条目不满足 → `Partial`；无 Work Item → `None`。
    pub fn compute_issue_delivery_summary(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> Result<IssueDeliverySummary, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        // 读 issue 以确认其存在，避免对不存在的 issue 返回空聚合（与 T4 组装点一致）。
        IssueStore::new(self.paths()).get(project_id, issue_id)?;

        let work_items = LifecycleStore::new(self.paths()).list_work_items(project_id, issue_id)?;

        let mut entries = Vec::with_capacity(work_items.len());
        for work_item in &work_items {
            let latest_attempt = self
                .list_attempts_covering_work_item(project_id, issue_id, &work_item.id)?
                .into_iter()
                .last();

            let attempt_status = latest_attempt
                .as_ref()
                .map(|attempt| attempt.status.clone());
            let branch_name = latest_attempt
                .as_ref()
                .map(|attempt| attempt.branch_name.clone());
            let commit_sha = latest_attempt
                .as_ref()
                .and_then(|attempt| attempt.head_commit.clone());

            let (push_status, push_error) = match latest_attempt.as_ref() {
                Some(attempt) => {
                    let latest_review = self
                        .list_review_requests(project_id, issue_id, &attempt.id)?
                        .into_iter()
                        .last();
                    match latest_review {
                        Some(review) => (Some(review.push_status), review.push_error),
                        None => (None, None),
                    }
                }
                None => (None, None),
            };

            let repository_name = self.resolve_repository_name(project_id, work_item)?;

            entries.push(DeliveryEntry {
                repository_name,
                work_item_id: work_item.id.clone(),
                attempt_status,
                branch_name,
                commit_sha,
                push_status,
                push_error,
            });
        }

        let overall = if entries.is_empty() {
            IssueDeliveryOverall::None
        } else if entries.iter().all(|entry| {
            entry.attempt_status == Some(CodingAttemptStatus::Completed)
                && entry.push_status == Some(PushStatus::Pushed)
        }) {
            IssueDeliveryOverall::AllPushed
        } else {
            IssueDeliveryOverall::Partial
        };

        Ok(IssueDeliverySummary {
            project_id: project_id.to_string(),
            issue_id: issue_id.to_string(),
            entries,
            overall,
        })
    }

    /// 列出覆盖指定 Work Item 的全部 attempt（按 `created_at` 升序，同刻以
    /// `(attempt_no, id)` tie-break）。
    ///
    /// 覆盖关系（REQ-COD-06 适配，multi-repo-group-coding WP4）：
    /// - `attempt.work_item_id` 直接绑定（WorkItem scope 既有语义零变化）；
    /// - WorkItemGroup attempt 经物化 coding units 覆盖
    ///   （`unit.logical_work_item_id`）——group attempt 仅在 `work_item_id`
    ///   承载桶内拓扑序首个 WI，多 target 拆分增殖后其余 WI 的 attempt 归属以
    ///   units 为权威（与 web 投影消费面的 units 索引模式一致）。
    ///
    /// 仅按 `work_item_id` 过滤的 `list_attempts_for_work_item` 在 group 形态下
    /// 会漏掉承载该 WI 的 target-attempt，导致 issue 级交付聚合永远 `Partial`、
    /// 完成门（`maybe_complete_issue_delivery`）永不触发。
    fn list_attempts_covering_work_item(
        &self,
        project_id: &str,
        issue_id: &str,
        work_item_id: &str,
    ) -> Result<Vec<CodingExecutionAttempt>, ProductStoreError> {
        let mut attempts = Vec::new();
        for attempt in self.list_attempts_for_issue(project_id, issue_id)? {
            if attempt.work_item_id == work_item_id {
                attempts.push(attempt);
                continue;
            }
            if attempt.scope != crate::product::coding_models::CodingAttemptScope::WorkItemGroup {
                continue;
            }
            let covered = self
                .list_coding_units(project_id, issue_id, &attempt.id)?
                .iter()
                .any(|unit| unit.logical_work_item_id == work_item_id);
            if covered {
                attempts.push(attempt);
            }
        }
        // 排序主键=created_at（Utc::now().to_rfc3339() 同格式字典序=时间序）：
        // per-WI 与 group attempt 的 attempt_no 来自不同计数空间（group 锚桶
        // 内首个 WI），跨空间混排不可比——陈旧 per-WI 可凭编号压过新 group
        // （完成门永不触发）或反向（k3 fix round 1，P2）；(attempt_no, id)
        // 仅作同刻 tie-break（同计数空间内保持既有确定性顺序）。
        attempts.sort_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| left.attempt_no.cmp(&right.attempt_no))
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(attempts)
    }

    /// 解析 Work Item 的仓库展示名。
    ///
    /// 优先 `target_repository_id`（`Option<LogicalRepositoryId>`）经
    /// `RepositoryStore::resolve_logical_repository_strict` 解析出 checkout 路径末段目录名；
    /// `target_repository_id` 缺省或 checkout 路径无末段时回退 `repository_id` 字符串本身。
    fn resolve_repository_name(
        &self,
        project_id: &str,
        work_item: &LifecycleWorkItemRecord,
    ) -> Result<String, ProductStoreError> {
        if let Some(logical_id) = work_item.target_repository_id {
            let paths = self.paths();
            let project = ProjectStore::new(paths.clone()).get(project_id)?;
            let (_, checkout, _) = RepositoryStore::for_project(paths, &project)
                .resolve_logical_repository_strict(project_id, logical_id)?;
            if let Some(name) = checkout
                .canonical_path
                .file_name()
                .map(|value| value.to_string_lossy().to_string())
                .filter(|value| !value.is_empty())
            {
                return Ok(name);
            }
        }
        Ok(work_item.repository_id.clone())
    }
}
