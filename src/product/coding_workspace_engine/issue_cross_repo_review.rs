//! issue 级跨仓最终评审执行链路（REQ-GFR-CR-04「跨仓评审材料与 provider 选择」，
//! 设计 B §四 B2–B5）。
//!
//! 一次性 advisory 旁路：材料装配（逐仓 diff 分段）→ prompt → 单次 provider
//! 执行 → 解析 → 落盘。**不**新建 reviewer 窗口、不产出 `acceptance` /
//! `review_window_id`、不改任何 attempt 的 status / stage、不跑 plan-defect
//! 路由（REQ-GFR-CR-05 advisory 红线）。
//!
//! provider 取值顺序（设计 B §四 B5，回退不是跳过）：最新已完成 attempt 快照的
//! `.internal_reviewer` → 缺失回退同一快照的 `.code_reviewer` → 两者皆缺 →
//! 不执行、`status=Skipped`。

use super::*;
use crate::product::coding_models::{
    CodingAttemptScope, CodingRoleProviderConfigSnapshot, CrossRepoRepositorySection,
    IssueCrossRepoReview, IssueCrossRepoReviewStatus,
};
use crate::product::id::next_sequential_id;
use crate::product::repository_store::RepositoryStore;

use super::cross_repo_review_prompts::{
    CROSS_REPO_DIFF_SECTION_MAX_CHARS, CROSS_REPO_MATERIAL_MAX_BYTES,
    ISSUE_CROSS_REPO_REVIEW_OUTPUT_SCHEMA, build_issue_cross_repo_review_prompt,
};

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
    /// 快照在场的最新已完成 attempt（provider 配置与 raw 输出锚点）。
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

        let mut review = if let Some(reason) = material.failure.as_deref() {
            self.build_failed_cross_repo_review(
                project_id,
                issue_id,
                &material,
                &format!("材料装配失败: {reason}"),
            )
        } else {
            match self.resolve_issue_cross_repo_review_provider(
                project_id,
                issue_id,
                &latest_attempt,
            )? {
                Some(provider_name) => {
                    self.run_issue_cross_repo_review_provider(
                        project_id,
                        issue_id,
                        &material,
                        &latest_attempt,
                        &provider_name,
                        provider,
                    )
                    .await
                }
                None => IssueCrossRepoReview {
                    id: next_sequential_id("cross_repo_review", 0),
                    project_id: project_id.to_string(),
                    issue_id: issue_id.to_string(),
                    plan_id: material.plan_id.clone(),
                    verdict: ReviewVerdict::Approve,
                    summary: "未执行：internal_reviewer 与 code_reviewer 均未配置".to_string(),
                    findings: Vec::new(),
                    repository_sections: material.sections.clone(),
                    status: IssueCrossRepoReviewStatus::Skipped,
                    failure_reason: Some(
                        "internal_reviewer 与 code_reviewer provider 配置均缺失，\
                         跨仓最终评审跳过（advisory，不阻断交付）"
                            .to_string(),
                    ),
                    raw_provider_output_ref: None,
                    created_at: Utc::now().to_rfc3339(),
                },
            }
        };
        if review.summary.trim().is_empty() {
            review.summary = default_review_summary(&review.verdict);
        }
        self.store.save_issue_cross_repo_review(&review)?;
        Ok(Some(review))
    }

    /// 材料装配：该 issue 下每个已完成 attempt 一段，按逻辑仓升序；diff 区间与
    /// code reviewer 同源同口径（`git_diff(worktree, code_review_diff_base)`）。
    async fn assemble_issue_cross_repo_review_material(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> Result<CrossRepoReviewMaterial, ProductStoreError> {
        let attempts = self.store.list_attempts_for_issue(project_id, issue_id)?;
        let latest_attempt = attempts
            .iter()
            .filter(|attempt| attempt.status == CodingAttemptStatus::Completed)
            .map(|attempt| attempt.clone())
            .last();
        let mut completed: Vec<CodingExecutionAttempt> = attempts
            .into_iter()
            .filter(|attempt| attempt.status == CodingAttemptStatus::Completed)
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
        // 跨仓判定：快照在场的逻辑仓不足 2 个 → 单仓 issue，不评审、不落工件。
        let distinct_logical_repos = completed
            .iter()
            .filter_map(|attempt| attempt.target_snapshot.as_ref())
            .map(|snapshot| snapshot.logical_repository_id)
            .collect::<std::collections::BTreeSet<_>>()
            .len();
        if distinct_logical_repos < 2 {
            return Ok(CrossRepoReviewMaterial {
                sections: Vec::new(),
                diffs: Vec::new(),
                latest_attempt: None,
                plan_id: String::new(),
                failure: None,
            });
        }
        completed.sort_by(|left, right| {
            cross_repo_section_sort_key(left).cmp(&cross_repo_section_sort_key(right))
        });
        let plan_id = completed
            .iter()
            .filter_map(|attempt| attempt.work_item_group_id.clone())
            .last()
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
    async fn run_issue_cross_repo_review_provider(
        &self,
        project_id: &str,
        issue_id: &str,
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
            timeout: DEFAULT_PROVIDER_TIMEOUT_SECS,
            max_retries: 0,
        };
        let mut provider_input =
            streaming_input_from_adapter(&legacy_input, working_dir, permission_mode);
        provider_input.workspace_session_id = Some(format!("issue-cross-repo-{issue_id}"));

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
                        id: next_sequential_id("cross_repo_review", 0),
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
        self.build_failed_cross_repo_review(project_id, issue_id, material, &failure_reason)
    }

    /// 一次性驱动 provider 会话直到终态（Completed / Failed / 通道关闭）或总超时
    /// （`DEFAULT_PROVIDER_TIMEOUT_SECS`）。无交互面：权限/选择请求不响应，
    /// 由总超时收敛为 `Failed`。
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
            Duration::from_secs(DEFAULT_PROVIDER_TIMEOUT_SECS),
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
                DEFAULT_PROVIDER_TIMEOUT_SECS
            )),
        }
    }

    fn build_failed_cross_repo_review(
        &self,
        project_id: &str,
        issue_id: &str,
        material: &CrossRepoReviewMaterial,
        reason: &str,
    ) -> IssueCrossRepoReview {
        IssueCrossRepoReview {
            id: next_sequential_id("cross_repo_review", 0),
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
}
