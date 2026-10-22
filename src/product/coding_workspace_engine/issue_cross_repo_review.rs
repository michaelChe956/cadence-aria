//! issue 级跨仓最终评审执行链路（REQ-GFR-CR-04「跨仓评审材料与 provider 选择」，
//! 设计 B §四 B2–B5）。
//!
//! 一次性 advisory 旁路：材料装配（逐仓 diff 分段）→ prompt → 单次 provider
//! 执行 → 解析 → 落盘。**不**新建 reviewer 窗口、不产出 `acceptance` /
//! `review_window_id`、不改任何 attempt 的 status / stage、不跑 plan-defect
//! 路由（REQ-GFR-CR-05 advisory 红线）。
//!
//! provider 取值顺序（设计 B §四 B5，回退不是跳过）：最新「开发完成」attempt 快照的
//! `.internal_reviewer` → 缺失回退同一快照的 `.code_reviewer` → 两者皆缺 →
//! 不执行、`status=Skipped`。
//!
//! 触发接线（REQ-GFR-CR-03「issue 级跨仓最终评审恰好一次」，设计 B §七 E）：
//! `maybe_trigger_issue_cross_repo_final_review` 是**触发层**入口——幂等 +
//! 「全部开发完成」门禁 + provider adapter 由调用边界注入；触发**只登记并
//! 异步执行**（REQ-GFR-CR-05「MUST NOT 阻断」）：评审在独立 task 按
//! advisory 短时限驱动，终态记录经 store 落盘、可轮询，调用方立即返回。

use super::*;
use crate::product::coding_models::{
    CodingAttemptScope, CodingExecutionUnitStatus, CodingRoleProviderConfigSnapshot,
    CrossRepoRepositorySection, IssueCrossRepoReview, IssueCrossRepoReviewStatus,
};
use crate::product::id::next_sequential_id;
use crate::product::repository_store::RepositoryStore;
use std::sync::Arc;

use super::cross_repo_review_prompts::{
    CROSS_REPO_DIFF_SECTION_MAX_CHARS, CROSS_REPO_MATERIAL_MAX_BYTES,
    ISSUE_CROSS_REPO_REVIEW_OUTPUT_SCHEMA, build_issue_cross_repo_review_prompt,
};


/// advisory 跨仓评审的自带短时限（REQ-GFR-CR-05「MUST NOT 阻断最终确认/
/// 交付」）：触发只登记并异步执行，provider 楔死时按此时限收敛为 `Failed`
/// 落盘，不与整 run 总上限 `DEFAULT_PROVIDER_TIMEOUT_SECS`（3h）同界。
/// 测试态缩短（`provider_drive/watchdog.rs` 的 `#[cfg]` 对偶先例）。
#[cfg(not(test))]
const ISSUE_CROSS_REPO_REVIEW_TIMEOUT_SECS: u64 = 120;
#[cfg(test)]
const ISSUE_CROSS_REPO_REVIEW_TIMEOUT_SECS: u64 = 1;

/// 进行中的 issue 级跨仓评审登记（进程内）：触发层「check → spawn → write」
/// 窗口内同一 store 的同一 issue 至多一条在飞评审（防止收口钩子重入导致
/// provider 双跑）。登记在评审任务退出时释放；进程重启后登记清空、记录
/// 缺失 → 下一次收口钩子仍可触发（advisory，与 REQ-GFR-CR-03 的落盘幂等
/// 互补）。
static INFLIGHT_CROSS_REPO_REVIEWS: std::sync::Mutex<std::collections::BTreeSet<String>> =
    std::sync::Mutex::new(std::collections::BTreeSet::new());

/// 在飞登记键：store 根 + project + issue（同一 issue 的唯一身份；含 store
/// 根使同 id 的不同 store 互不干扰）。
fn cross_repo_review_inflight_key(store_root: &Path, project_id: &str, issue_id: &str) -> String {
    format!("{}/{project_id}/{issue_id}", store_root.display())
}

/// 登记在飞评审；已登记（同 issue 有在飞评审）返回 `false`。
fn register_cross_repo_review_inflight(
    store_root: &Path,
    project_id: &str,
    issue_id: &str,
) -> bool {
    INFLIGHT_CROSS_REPO_REVIEWS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(cross_repo_review_inflight_key(store_root, project_id, issue_id))
}

/// 释放在飞登记（评审任务退出路径统一经 guard 触发）。
fn release_cross_repo_review_inflight(store_root: &Path, project_id: &str, issue_id: &str) {
    INFLIGHT_CROSS_REPO_REVIEWS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(&cross_repo_review_inflight_key(store_root, project_id, issue_id));
}

/// 在飞登记的 panic 安全释放守卫：后台评审任务任何退出路径（完成/失败/
/// panic/取消）都释放登记。
struct CrossRepoReviewInflightGuard {
    store_root: PathBuf,
    project_id: String,
    issue_id: String,
}

impl Drop for CrossRepoReviewInflightGuard {
    fn drop(&mut self) {
        release_cross_repo_review_inflight(&self.store_root, &self.project_id, &self.issue_id);
    }
}

/// 跨仓评审的 provider adapter 查询闭包（按 provider 名称取 adapter）。
type CrossRepoReviewProviderLookup<'a> =
    dyn Fn(&ProviderName) -> Option<Arc<dyn StreamingProviderAdapter>> + Send + Sync + 'a;

/// 跨仓最终评审的 provider adapter 解析器（**调用边界注入**）。
///
/// 引擎只决定 provider **名称**（角色快照 `.internal_reviewer` → 回退
/// `.code_reviewer`）；adapter 由调用边界（web 层）按该名称解析后注入——引擎层
/// **不自建 adapter**（设计 B §七 E）。
#[derive(Clone)]
pub(crate) struct IssueCrossRepoReviewProviderResolver<'a> {
    inner: Arc<CrossRepoReviewProviderLookup<'a>>,
}

impl<'a> IssueCrossRepoReviewProviderResolver<'a> {
    pub(crate) fn new<F>(resolver: F) -> Self
    where
        F: Fn(&ProviderName) -> Option<Arc<dyn StreamingProviderAdapter>> + Send + Sync + 'a,
    {
        Self {
            inner: Arc::new(resolver),
        }
    }

    /// 无 provider 边界（单仓 issue / 测试夹具）：跨仓评审不执行、不落任何工件。
    pub(crate) fn unavailable() -> Self {
        Self::new(|_| None)
    }

    fn resolve(&self, provider_name: &ProviderName) -> Option<Arc<dyn StreamingProviderAdapter>> {
        (self.inner)(provider_name)
    }
}

impl<'a> Default for IssueCrossRepoReviewProviderResolver<'a> {
    fn default() -> Self {
        Self::unavailable()
    }
}

/// attempt 的「开发完成」判定（设计 B §七 E 的触发门禁口径）：`Completed`，或
/// `WaitingForHuman && stage == FinalConfirm`——后者表示开发已收尾、只等人工最终
/// 确认（group 形态的收口钩子正落在此刻）。
///
/// 材料装配与触发门禁共用同一口径：否则 group 形态在收口钩子触发时，该仓的
/// attempt 尚未 `Completed`，跨仓材料会缺该仓（甚至整份材料为空 → 评审不执行）。
fn attempt_development_finished(attempt: &CodingExecutionAttempt) -> bool {
    match attempt.status {
        CodingAttemptStatus::Completed => true,
        CodingAttemptStatus::WaitingForHuman => attempt.stage == CodingExecutionStage::FinalConfirm,
        _ => false,
    }
}

/// 最新「开发完成」的 attempt（材料装配的 provider / raw 输出锚点与触发层同一口径）。
fn latest_development_finished_attempt(
    attempts: &[CodingExecutionAttempt],
) -> Option<CodingExecutionAttempt> {
    attempts
        .iter()
        .rfind(|attempt| attempt_development_finished(attempt))
        .cloned()
}

/// 分段排序键：`target_snapshot.logical_repository_id` 升序；快照缺失（旧单仓
/// 记录）时退回 `attempt.id` 参与稳定排序（设计 B §四 B4）。
fn cross_repo_section_sort_key(attempt: &CodingExecutionAttempt) -> (String, String) {
    (
        attempt
            .target_snapshot
            .as_ref()
            .map(|snapshot| snapshot.logical_repository_id.0.to_string())
            .unwrap_or_default(),
        attempt.id.clone(),
    )
}

/// 仓名解析：物理仓库记录名，缺失时回退物理仓库 id（不虚构名称）。
fn cross_repo_repository_name(
    store: &CodingAttemptStore,
    project_id: &str,
    physical_repository_id: &str,
) -> String {
    RepositoryStore::new(store.paths())
        .list(project_id)
        .ok()
        .and_then(|records| {
            records
                .into_iter()
                .find(|record| record.id == physical_repository_id)
        })
        .map(|record| record.name)
        .unwrap_or_else(|| physical_repository_id.to_string())
}

/// 分段覆盖的 work item 列表：group attempt 取其全部 coding units 的 logical
/// work item；单 WI attempt 取自身 item；units 缺失时回退 active item。
fn cross_repo_section_work_item_ids(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
) -> Vec<String> {
    if attempt.scope == CodingAttemptScope::WorkItemGroup {
        let unit_items = store
            .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .map(|units| {
                units
                    .into_iter()
                    .map(|unit| unit.logical_work_item_id)
                    .collect::<Vec<String>>()
            })
            .unwrap_or_default();
        if !unit_items.is_empty() {
            return unit_items;
        }
    }
    vec![active_work_item_id_for_prompt(attempt).to_string()]
}

/// 材料装配结果。
struct CrossRepoReviewMaterial {
    sections: Vec<CrossRepoRepositorySection>,
    diffs: Vec<String>,
    /// 快照在场的最新「开发完成」attempt（provider 配置与 raw 输出锚点）。
    latest_attempt: Option<CodingExecutionAttempt>,
    plan_id: String,
    /// 材料装配失败原因：非空时评审记录落 `Failed`（不静默省略任何仓）。
    failure: Option<String>,
}

enum CrossRepoProviderRun {
    Completed(String),
    Failed(String),
}

impl CodingWorkspaceEngine {
    /// 执行（或跳过）issue 级跨仓最终评审并落盘。
    ///
    /// - 幂等（REQ-GFR-CR-03）：`review.json` 按 issue 唯一，已存在直接返回，
    ///   绝不重跑、不重试。
    /// - 单仓 issue（快照在场的逻辑仓 < 2）不评审、不落任何工件（REQ-GFR-CR-02
    ///   「单仓不产生跨仓评审」的引擎侧兜底）。
    /// - provider 缺失（`internal_reviewer` 与 `code_reviewer` 皆空）→ 落
    ///   `Skipped` 记录；provider 失败 / 超时 / 输出不可解析 → 落 `Failed`
    ///   记录并保留原因。两者都是 advisory，不向调用方抛错、不阻断交付。
    pub async fn maybe_run_issue_cross_repo_final_review(
        &self,
        project_id: &str,
        issue_id: &str,
        provider: &dyn StreamingProviderAdapter,
    ) -> Result<Option<IssueCrossRepoReview>, ProductStoreError> {
        self.run_issue_cross_repo_final_review(project_id, issue_id, Some(provider))
            .await
    }

    /// 触发 issue 级跨仓最终评审（REQ-GFR-CR-03「issue 级跨仓最终评审恰好一次」，
    /// 设计 B §七 E）。
    ///
    /// 调用点：两个既有收口钩子的尾部（`handoffs.rs` 的
    /// `maybe_complete_issue_delivery` 与 `prepare_group_final_confirm_from_readiness`）。
    ///
    /// - **只登记并异步执行**（REQ-GFR-CR-05「MUST NOT 阻断最终确认/交付」）：
    ///   本方法完成幂等/门禁/adapter 解析判定后 `tokio::spawn` 独立任务驱动
    ///   评审，**立即返回**、不等待 provider 终态；终态记录（Completed /
    ///   Failed / Skipped）经 store 落盘、可轮询，provider 楔死由 advisory
    ///   短时限收敛为 `Failed` 落盘。
    /// - **幂等**：issue 级评审记录已存在（或本进程内已有在飞评审）→ 直接
    ///   返回，不重复触发、不重试。
    /// - **门禁**：单仓 issue（无 `logical_codebase_id`）或「全部开发完成」不
    ///   满足 → 不触发、**不落任何工件**（不得让半成品 issue 提前消费掉唯一
    ///   一次评审机会）。
    /// - **provider 注入**：引擎只定 provider **名称**，adapter 由调用边界按该
    ///   名称解析后注入；调用边界解析不出 adapter → 评审不执行、**不落工件**
    ///   （既不 `Completed` 也不 `Skipped`），下一次进入收口钩子仍可触发。
    pub(crate) async fn maybe_trigger_issue_cross_repo_final_review(
        &self,
        project_id: &str,
        issue_id: &str,
        review_provider: &IssueCrossRepoReviewProviderResolver<'_>,
    ) -> Result<(), ProductStoreError> {
        if self
            .store
            .load_issue_cross_repo_review(project_id, issue_id)?
            .is_some()
        {
            return Ok(());
        }
        // 形态门禁（REQ-GFR-CR-03 触发口径）：按 issue 的 logical_codebase_id
        // 判多仓形态——单仓 issue 不评审、不落任何工件。
        if !self.issue_is_cross_repo_shape(project_id, issue_id)? {
            return Ok(());
        }
        if !self.issue_development_finished(project_id, issue_id)? {
            return Ok(());
        }
        let provider_name =
            self.resolve_issue_cross_repo_review_provider_name(project_id, issue_id)?;
        let adapter = match provider_name.as_ref() {
            Some(name) => match review_provider.resolve(name) {
                Some(adapter) => Some(adapter),
                None => {
                    tracing::warn!(
                        project_id = %project_id,
                        issue_id = %issue_id,
                        provider = ?name,
                        "跨仓最终评审的 provider adapter 无法由调用边界解析：评审不执行、不落工件"
                    );
                    return Ok(());
                }
            },
            // 两个 reviewer 都未配置 → 后台落 `Skipped` 记录（与显式执行入口
            // 同语义）。
            None => None,
        };
        // 在飞登记：同一 issue 至多一条在飞评审（重入的收口钩子不双跑）。
        if !register_cross_repo_review_inflight(
            self.store.paths().root(),
            project_id,
            issue_id,
        ) {
            return Ok(());
        }
        let engine = self.clone();
        let guard = CrossRepoReviewInflightGuard {
            store_root: self.store.paths().root().to_path_buf(),
            project_id: project_id.to_string(),
            issue_id: issue_id.to_string(),
        };
        let project_id = guard.project_id.clone();
        let issue_id = guard.issue_id.clone();
        tokio::spawn(async move {
            let _guard = guard;
            if let Err(error) = engine
                .run_issue_cross_repo_final_review(&project_id, &issue_id, adapter.as_deref())
                .await
            {
                tracing::warn!(
                    project_id = %project_id,
                    issue_id = %issue_id,
                    error = %error,
                    "issue 级跨仓最终评审执行失败（advisory，不阻断收口）"
                );
            }
        });
        Ok(())
    }

    /// 跨仓最终评审触发的**旁路**分发（REQ-GFR-CR-03）：advisory 语义——任何
    /// 失败都不得阻断 per-attempt 收口（不抛错、不改 attempt / issue 状态）；
    /// 触发本身只登记并异步执行（见 `maybe_trigger_issue_cross_repo_final_review`），
    /// 调用方（收口钩子 / 最终确认迁移）立即返回。
    pub(crate) async fn dispatch_issue_cross_repo_final_review_trigger(
        &self,
        attempt: &CodingExecutionAttempt,
        review_provider: &IssueCrossRepoReviewProviderResolver<'_>,
    ) {
        if let Err(error) = self
            .maybe_trigger_issue_cross_repo_final_review(
                &attempt.project_id,
                &attempt.issue_id,
                review_provider,
            )
            .await
        {
            tracing::warn!(
                project_id = %attempt.project_id,
                issue_id = %attempt.issue_id,
                error = %error,
                "issue 级跨仓最终评审触发失败（advisory，不阻断收口）"
            );
        }
    }

    /// 评审执行核心。`provider == None` 表示两个 reviewer 都未配置 → 落
    /// `Skipped` 记录（不启动 provider）。
    async fn run_issue_cross_repo_final_review(
        &self,
        project_id: &str,
        issue_id: &str,
        provider: Option<&dyn StreamingProviderAdapter>,
    ) -> Result<Option<IssueCrossRepoReview>, ProductStoreError> {
        if let Some(existing) = self
            .store
            .load_issue_cross_repo_review(project_id, issue_id)?
        {
            return Ok(Some(existing));
        }
        let material = self
            .assemble_issue_cross_repo_review_material(project_id, issue_id)
            .await?;
        let Some(latest_attempt) = material.latest_attempt.clone() else {
            return Ok(None);
        };

        // 本 issue 级评审记录的稳定身份：在 provider 启动前铸造，既作为落盘
        // 记录的 id，也作为 Reviewer wire 会话的工具策略合同引用（P1-1：与
        // 评审窗口解绑——本评审不开窗，REQ-GFR-CR-05）。
        let review_id = next_sequential_id("cross_repo_review", 0);
        let mut review = if let Some(reason) = material.failure.as_deref() {
            self.build_failed_cross_repo_review(
                project_id,
                issue_id,
                &review_id,
                &material,
                &format!("材料装配失败: {reason}"),
            )
        } else {
            match self.resolve_issue_cross_repo_review_provider(
                project_id,
                issue_id,
                &latest_attempt,
            )? {
                Some(provider_name) => match provider {
                    Some(provider) => {
                        self.run_issue_cross_repo_review_provider(
                            project_id,
                            issue_id,
                            &review_id,
                            &material,
                            &latest_attempt,
                            &provider_name,
                            provider,
                        )
                        .await
                    }
                    None => self.build_skipped_cross_repo_review(
                        project_id,
                        issue_id,
                        &review_id,
                        &material,
                        "未执行：provider adapter 不可用",
                        "跨仓最终评审所需的 provider adapter 不可用（调用边界未解析出该 \
                         provider），评审未执行（advisory，不阻断交付）",
                    ),
                },
                None => self.build_skipped_cross_repo_review(
                    project_id,
                    issue_id,
                    &review_id,
                    &material,
                    "未执行：internal_reviewer 与 code_reviewer 均未配置",
                    "internal_reviewer 与 code_reviewer provider 配置均缺失，\
                     跨仓最终评审跳过（advisory，不阻断交付）",
                ),
            }
        };
        if review.summary.trim().is_empty() {
            review.summary = default_review_summary(&review.verdict);
        }
        self.store.save_issue_cross_repo_review(&review)?;
        Ok(Some(review))
    }

    /// 跨仓评审将使用的 provider 名称（最新已完成 attempt 的角色快照：
    /// `.internal_reviewer` → 回退 `.code_reviewer` → 皆缺 `None`）。
    fn resolve_issue_cross_repo_review_provider_name(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> Result<Option<ProviderName>, ProductStoreError> {
        let attempts = self.store.list_attempts_for_issue(project_id, issue_id)?;
        let Some(latest_attempt) = latest_development_finished_attempt(&attempts) else {
            return Ok(None);
        };
        self.resolve_issue_cross_repo_review_provider(project_id, issue_id, &latest_attempt)
    }

    /// issue 形态判定（REQ-GFR-CR-03 触发口径）：`logical_codebase_id.is_some()`
    /// 即多仓逻辑代码库 issue——多仓 issue 的全部 work item 落在同一仓、或
    /// attempt 快照缺失（旧记录）时仍必须评审；仅单仓 issue（`Ok(None)`，含
    /// issue 记录缺失）不评审。**不**以 attempt 快照的去重仓数判形态。
    fn issue_is_cross_repo_shape(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> Result<bool, ProductStoreError> {
        Ok(crate::product::logical_codebase::resolve_issue_logical_codebase_id(
            &self.store.paths(),
            project_id,
            issue_id,
        )?
        .is_some())
    }

    /// 「全部开发完成」判定（触发前置，设计 B §七 E + controller 裁定）。
    ///
    /// - work item 集合 = issue 下**当前有效**的 work item（已删除的不计入）；
    /// - 每个 work item 取 `list_attempts_covering_work_item` 的**最新** attempt；
    /// - 最新 attempt 满足 `Completed`，或 `WaitingForHuman && stage == FinalConfirm`
    ///   → 该 work item 已开发完成；
    /// - **跳过 / 取消按已终结处理**（attempt `Aborted`、group unit `Skipped`）——
    ///   已终结的 work item 不阻塞触发；
    /// - issue 下无 work item → 不触发（无开发可评审）。
    fn issue_development_finished(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> Result<bool, ProductStoreError> {
        let work_item_ids = LifecycleStore::new(self.store.paths())
            .list_work_items(project_id, issue_id)?
            .into_iter()
            .map(|work_item| work_item.id)
            .collect::<Vec<String>>();
        if work_item_ids.is_empty() {
            return Ok(false);
        }
        for work_item_id in work_item_ids {
            let attempts =
                self.store
                    .list_attempts_covering_work_item(project_id, issue_id, &work_item_id)?;
            let Some(latest) = attempts.last() else {
                // 无 attempt → 该 work item 尚未开发完成。
                return Ok(false);
            };
            if !self.work_item_development_finished(latest, &work_item_id)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// 单个 work item 的开发完成判定（跳过 / 取消按已终结处理）。
    fn work_item_development_finished(
        &self,
        attempt: &CodingExecutionAttempt,
        work_item_id: &str,
    ) -> Result<bool, ProductStoreError> {
        // 取消：attempt 终态 `Aborted` → 按已终结处理。
        if attempt.status == CodingAttemptStatus::Aborted {
            return Ok(true);
        }
        // 跳过：group attempt 的对应 unit 被归一为 `Skipped` → 按已终结处理。
        if attempt.scope == CodingAttemptScope::WorkItemGroup {
            let skipped = self
                .store
                .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)?
                .iter()
                .any(|unit| {
                    unit.logical_work_item_id == work_item_id
                        && unit.status == CodingExecutionUnitStatus::Skipped
                });
            if skipped {
                return Ok(true);
            }
        }
        Ok(attempt_development_finished(attempt))
    }

    /// 材料装配：该 issue 下每个「开发完成」的 attempt 一段（`Completed`，或
    /// `WaitingForHuman && stage == FinalConfirm`——group 形态的收口钩子落在后
    /// 一者），按逻辑仓升序；diff 区间与 code reviewer 同源同口径
    /// （`git_diff(worktree, code_review_diff_base)`）。
    async fn assemble_issue_cross_repo_review_material(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> Result<CrossRepoReviewMaterial, ProductStoreError> {
        let attempts = self.store.list_attempts_for_issue(project_id, issue_id)?;
        let latest_attempt = latest_development_finished_attempt(&attempts);
        let mut completed: Vec<CodingExecutionAttempt> = attempts
            .into_iter()
            .filter(attempt_development_finished)
            .collect();
        if completed.is_empty() {
            return Ok(CrossRepoReviewMaterial {
                sections: Vec::new(),
                diffs: Vec::new(),
                latest_attempt: None,
                plan_id: String::new(),
                failure: None,
            });
        }
        // 形态判定（REQ-GFR-CR-03 触发口径）：按 issue 的 `logical_codebase_id`
        // 判多仓形态，而非 attempt 快照的去重仓数——多仓 issue 的全部 work
        // item 落在同一仓（材料只有 1 段）或快照缺失（旧记录）时仍必须评审；
        // 仅单仓 issue 不评审、不落任何工件（REQ-GFR-CR-02 的引擎侧兜底）。
        if !self.issue_is_cross_repo_shape(project_id, issue_id)? {
            return Ok(CrossRepoReviewMaterial {
                sections: Vec::new(),
                diffs: Vec::new(),
                latest_attempt: None,
                plan_id: String::new(),
                failure: None,
            });
        }
        // 排序键每次比较都要分配两个 String → 用 `sort_by_cached_key`（每元素算一次
        // 键），与 `sort_by` 同序且无重复分配。
        completed.sort_by_cached_key(cross_repo_section_sort_key);
        let plan_id = completed
            .iter()
            .filter_map(|attempt| attempt.work_item_group_id.clone())
            .next_back()
            .unwrap_or_default();

        let mut sections = Vec::new();
        let mut diffs = Vec::new();
        let mut used_bytes = 0usize;
        let mut failure = None;
        for attempt in &completed {
            let snapshot = attempt.target_snapshot.as_ref();
            let logical_repository_id = snapshot
                .map(|snapshot| snapshot.logical_repository_id.0.to_string())
                .unwrap_or_default();
            let repository_name = snapshot
                .map(|snapshot| {
                    cross_repo_repository_name(
                        &self.store,
                        project_id,
                        &snapshot.physical_repository_id,
                    )
                })
                .unwrap_or_else(|| attempt.id.clone());
            let Some(worktree_path) = attempt.worktree_path.as_ref() else {
                failure = Some(format!(
                    "attempt {} 缺少 worktree，无法取得 diff（不静默省略该仓）",
                    attempt.id
                ));
                break;
            };
            let diff_base = match code_review_diff_base(attempt) {
                Ok(base) => base.to_string(),
                Err(error) => {
                    failure = Some(format!("attempt {} diff 基线缺失: {error}", attempt.id));
                    break;
                }
            };
            let raw_diff = match self._git_service.git_diff(worktree_path, &diff_base).await {
                Ok(diff) => diff,
                Err(error) => {
                    failure = Some(format!(
                        "attempt {} 仓 {} diff 获取失败: {error}",
                        attempt.id, repository_name
                    ));
                    break;
                }
            };
            let mut diff_truncated = raw_diff.chars().count() > CROSS_REPO_DIFF_SECTION_MAX_CHARS;
            let mut diff = truncate_prompt_section(&raw_diff, CROSS_REPO_DIFF_SECTION_MAX_CHARS);
            if used_bytes + diff.len() > CROSS_REPO_MATERIAL_MAX_BYTES {
                diff_truncated = true;
                diff = format!(
                    "[diff truncated: 跨仓材料全包预算 {} 字节已用尽，本仓 diff 未注入，\
                     详见材料末尾 truncated_repositories 标注]",
                    CROSS_REPO_MATERIAL_MAX_BYTES
                );
            } else {
                used_bytes += diff.len();
            }
            sections.push(CrossRepoRepositorySection {
                repository_name,
                logical_repository_id,
                attempt_id: attempt.id.clone(),
                branch_name: attempt.branch_name.clone(),
                base_commit: diff_base,
                head_commit: attempt.head_commit.clone().unwrap_or_default(),
                work_item_ids: cross_repo_section_work_item_ids(&self.store, attempt),
                diff_truncated,
            });
            diffs.push(diff);
        }
        Ok(CrossRepoReviewMaterial {
            sections,
            diffs,
            latest_attempt,
            plan_id,
            failure,
        })
    }

    /// provider 取值顺序（REQ-GFR-CR-04）：`.internal_reviewer` → 回退同一快照
    /// `.code_reviewer` → 两者皆缺 `None`（调用方落 `Skipped`）。
    fn resolve_issue_cross_repo_review_provider(
        &self,
        project_id: &str,
        issue_id: &str,
        latest_attempt: &CodingExecutionAttempt,
    ) -> Result<Option<ProviderName>, ProductStoreError> {
        let snapshot: CodingRoleProviderConfigSnapshot = self
            .store
            .get_role_provider_config_snapshot(project_id, issue_id, &latest_attempt.id)?;
        Ok(snapshot.internal_reviewer.or(snapshot.code_reviewer))
    }

    /// 单次 provider 执行：不开窗、不写 role run、不产 acceptance；失败 / 超时 /
    /// 输出不可解析 → `Failed` 记录（不重试，REQ-GFR-CR-05）。
#[allow(clippy::too_many_arguments)] // 标识三元组/材料/attempt/provider 名与句柄各自来源不同，拆结构体只会多一层转抄。
    async fn run_issue_cross_repo_review_provider(
        &self,
        project_id: &str,
        issue_id: &str,
        review_id: &str,
        material: &CrossRepoReviewMaterial,
        latest_attempt: &CodingExecutionAttempt,
        provider_name: &ProviderName,
        provider: &dyn StreamingProviderAdapter,
    ) -> IssueCrossRepoReview {
        let snapshot = self
            .store
            .get_role_provider_config_snapshot(project_id, issue_id, &latest_attempt.id)
            .ok();
        // 快照读取失败（advisory 不阻断）：按角色缺省矩阵（Auto）解析权限模式。
        let permission_mode = coding_permission_mode_for_optional_provider(
            Some(provider_name),
            snapshot
                .as_ref()
                .map(|snapshot| {
                    snapshot.permission_mode_for_role(&CodingProviderRole::InternalReviewer)
                })
                .unwrap_or(crate::product::coding_models::CodingProviderPermissionMode::Auto),
        );
        let working_dir = latest_attempt
            .worktree_path
            .clone()
            .unwrap_or_else(|| PathBuf::from("."));
        let prompt = format!(
            "Project: {}\nIssue: {}\nPlan: {}\n{}",
            project_id,
            issue_id,
            material.plan_id,
            build_issue_cross_repo_review_prompt(&material.sections, &material.diffs)
        );
        let legacy_input = AdapterInput {
            env_vars: Default::default(),
            working_directory: None,
            provider_type: provider_type_for_name(provider_name),
            role: AdapterRole::Reviewer,
            worktree_path: Some(working_dir.display().to_string()),
            provider_stream_log_dir: None,
            prompt: prompt.clone(),
            context_files: Vec::new(),
            output_schema: ISSUE_CROSS_REPO_REVIEW_OUTPUT_SCHEMA.to_string(),
            timeout: ISSUE_CROSS_REPO_REVIEW_TIMEOUT_SECS,
            max_retries: 0,
        };
        let mut provider_input =
            streaming_input_from_adapter(&legacy_input, working_dir, permission_mode);
        provider_input.workspace_session_id = Some(format!("issue-cross-repo-{issue_id}"));
        // P1-1：Reviewer wire 角色在真实 adapter spawn 前经
        // `validate_tool_policy_for_role` 复验——`Reviewer + None` 直接
        // `PolicyRequired` 拒启（生产恒 `Failed("provider 启动失败…")`）。本
        // 评审不开评审窗（REQ-GFR-CR-05），为此铸造**与窗口解绑**的稳定合同
        // 引用：本 issue 级评审记录的 id（durable、按 issue 唯一），不复用
        // 任何评审窗 id。
        provider_input.tool_policy =
            Some(ProviderToolPolicy::reviewer_verify(review_id.to_string()));

        let failure_reason = match self
            .drive_issue_cross_repo_provider(provider, provider_input)
            .await
        {
            CrossRepoProviderRun::Completed(full_output) => {
                let raw_provider_output_ref = self
                    .store
                    .save_provider_raw_output(
                        latest_attempt,
                        CodingExecutionStage::IssueCrossRepoReview,
                        "issue_cross_repo_review",
                        &full_output,
                    )
                    .ok();
                // 严格解析（输出不可解析 → Failed，不合成 blocked 结论）：
                // default_source_stage = IssueCrossRepoReview（B6-8）。
                return match parse_group_review_payload(
                    &full_output,
                    CodingExecutionStage::IssueCrossRepoReview,
                ) {
                    Ok(payload) => IssueCrossRepoReview {
                        id: review_id.to_string(),
                        project_id: project_id.to_string(),
                        issue_id: issue_id.to_string(),
                        plan_id: material.plan_id.clone(),
                        verdict: payload.verdict,
                        summary: payload.summary,
                        findings: payload.findings,
                        repository_sections: material.sections.clone(),
                        status: IssueCrossRepoReviewStatus::Completed,
                        failure_reason: None,
                        raw_provider_output_ref,
                        created_at: Utc::now().to_rfc3339(),
                    },
                    Err(error) => {
                        let mut record = self.build_failed_cross_repo_review(
                            project_id,
                            issue_id,
                            review_id,
                            material,
                            &format!("provider 输出不可解析: {error}"),
                        );
                        record.raw_provider_output_ref = raw_provider_output_ref;
                        record
                    }
                };
            }
            CrossRepoProviderRun::Failed(reason) => reason,
        };
        self.build_failed_cross_repo_review(
            project_id,
            issue_id,
            review_id,
            material,
            &failure_reason,
        )
    }

    /// 一次性驱动 provider 会话直到终态（Completed / Failed / 通道关闭）或总超时
    /// （advisory 短时限 `ISSUE_CROSS_REPO_REVIEW_TIMEOUT_SECS`）。无交互面：
    /// 权限/选择请求不响应，由总超时收敛为 `Failed`。
    ///
    /// 本方法运行在触发层 `tokio::spawn` 的独立任务内（P1-2），不阻塞任何
    /// 收口/最终确认调用方。
    async fn drive_issue_cross_repo_provider(
        &self,
        provider: &dyn StreamingProviderAdapter,
        input: StreamingProviderInput,
    ) -> CrossRepoProviderRun {
        let session = match provider
            .start(input, self.cancellation_token().clone())
            .await
        {
            Ok(session) => session,
            Err(error) => {
                return CrossRepoProviderRun::Failed(format!("provider 启动失败: {error}"));
            }
        };
        let mut events = session.events;
        let mut accumulated = String::new();
        let terminal = tokio::time::timeout(
            Duration::from_secs(ISSUE_CROSS_REPO_REVIEW_TIMEOUT_SECS),
            async move {
                loop {
                    match events.recv().await {
                        Some(ProviderEvent::TextDelta { content }) => {
                            accumulated.push_str(&content)
                        }
                        Some(ProviderEvent::Completed(completion)) => {
                            return CrossRepoProviderRun::Completed(completion.full_output);
                        }
                        Some(ProviderEvent::Failed { message }) => {
                            return CrossRepoProviderRun::Failed(format!(
                                "provider 会话失败: {message}"
                            ));
                        }
                        Some(ProviderEvent::ProtocolError { code, message, .. }) => {
                            return CrossRepoProviderRun::Failed(format!(
                                "provider 协议错误 {code}: {message}"
                            ));
                        }
                        // advisory 一次性行：无权限/选择/工具交互面，忽略过程事件。
                        Some(_) => {}
                        None => {
                            if accumulated.trim().is_empty() {
                                return CrossRepoProviderRun::Failed(
                                    "provider 会话在完成前关闭且无任何输出".to_string(),
                                );
                            }
                            return CrossRepoProviderRun::Completed(accumulated);
                        }
                    }
                }
            },
        )
        .await;
        match terminal {
            Ok(outcome) => outcome,
            Err(_) => CrossRepoProviderRun::Failed(format!(
                "provider 超时（{} 秒）",
                ISSUE_CROSS_REPO_REVIEW_TIMEOUT_SECS
            )),
        }
    }

    fn build_failed_cross_repo_review(
        &self,
        project_id: &str,
        issue_id: &str,
        review_id: &str,
        material: &CrossRepoReviewMaterial,
        reason: &str,
    ) -> IssueCrossRepoReview {
        IssueCrossRepoReview {
            id: review_id.to_string(),
            project_id: project_id.to_string(),
            issue_id: issue_id.to_string(),
            plan_id: material.plan_id.clone(),
            verdict: ReviewVerdict::Approve,
            summary: "跨仓最终评审未完成（advisory，不阻断交付）".to_string(),
            findings: Vec::new(),
            repository_sections: material.sections.clone(),
            status: IssueCrossRepoReviewStatus::Failed,
            failure_reason: Some(reason.to_string()),
            raw_provider_output_ref: None,
            created_at: Utc::now().to_rfc3339(),
        }
    }

    /// `Skipped` 记录：评审未执行（provider 未配置 / adapter 不可用）。
    /// advisory 语义——不阻断交付、不重试。
    fn build_skipped_cross_repo_review(
        &self,
        project_id: &str,
        issue_id: &str,
        review_id: &str,
        material: &CrossRepoReviewMaterial,
        summary: &str,
        reason: &str,
    ) -> IssueCrossRepoReview {
        IssueCrossRepoReview {
            id: review_id.to_string(),
            project_id: project_id.to_string(),
            issue_id: issue_id.to_string(),
            plan_id: material.plan_id.clone(),
            verdict: ReviewVerdict::Approve,
            summary: summary.to_string(),
            findings: Vec::new(),
            repository_sections: material.sections.clone(),
            status: IssueCrossRepoReviewStatus::Skipped,
            failure_reason: Some(reason.to_string()),
            raw_provider_output_ref: None,
            created_at: Utc::now().to_rfc3339(),
        }
    }
}
