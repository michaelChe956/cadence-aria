use super::*;
use crate::product::coding_attempt_store::admission::{
    TARGET_SNAPSHOT_IDENTITY_DRIFTED, TARGET_SNAPSHOT_MISSING_FOR_LOGICAL,
};
use crate::product::logical_codebase::{LogicalRepositoryId, RepositoryRouting};

mod schema_v2;

pub(crate) const CODING_OUTPUT_HUMAN_TRIAGE_REASON_CODE: &str = "coding_output_human_triage";

pub(crate) fn coding_gate_action_for_id(action_id: &str) -> Option<CodingGateAction> {
    match action_id {
        "provide_context" => Some(CodingGateAction {
            action_id: "provide_context".to_string(),
            label: "补充上下文".to_string(),
            action_type: CodingGateActionType::ProvideContext,
        }),
        "send_to_coder" => Some(CodingGateAction {
            action_id: "send_to_coder".to_string(),
            label: "提交给 Coder 修复".to_string(),
            action_type: CodingGateActionType::SendToCoder,
        }),
        "manual_continue" => Some(CodingGateAction {
            action_id: "manual_continue".to_string(),
            label: "人工继续".to_string(),
            action_type: CodingGateActionType::ManualContinue,
        }),
        "accept_risk" => Some(CodingGateAction {
            action_id: "accept_risk".to_string(),
            label: "接受风险".to_string(),
            action_type: CodingGateActionType::AcceptRisk,
        }),
        "retry_coding" => Some(CodingGateAction {
            action_id: "retry_coding".to_string(),
            label: "重新启动 Coder".to_string(),
            action_type: CodingGateActionType::RetryCoding,
        }),
        "retry_review" => Some(CodingGateAction {
            action_id: "retry_review".to_string(),
            label: "重试审查".to_string(),
            action_type: CodingGateActionType::RetryReview,
        }),
        "retry_internal_review" => Some(CodingGateAction {
            action_id: "retry_internal_review".to_string(),
            label: "重试 Internal Review".to_string(),
            action_type: CodingGateActionType::RetryInternalReview,
        }),
        "retry_group_review_shard" => Some(CodingGateAction {
            action_id: "retry_group_review_shard".to_string(),
            label: "重试组审查分片".to_string(),
            action_type: CodingGateActionType::RetryGroupReviewShard,
        }),
        "retry_group_reduction" => Some(CodingGateAction {
            action_id: "retry_group_reduction".to_string(),
            label: "重试组审查归约".to_string(),
            action_type: CodingGateActionType::RetryGroupReduction,
        }),
        "abort" => Some(CodingGateAction {
            action_id: "abort".to_string(),
            label: "终止".to_string(),
            action_type: CodingGateActionType::Abort,
        }),
        _ => None,
    }
}

/// 分流结果：该 attempt 的 shared worktree 访问应走哪一族方法。
///
/// REQ-COD-03（§4.2.3）：`Some(snapshot)` 走多仓仓维路径（三元键
/// `(project, issue, logical repository)`）；`None + Legacy` 走单仓老路径（行为不变，红线）。
pub(crate) enum IssueSharedWorktreeRoute {
    /// 单仓老路径：`issue-shared-worktree.json`。
    Legacy,
    /// 多仓仓维路径：`shared-worktrees/{repository_id}.json`。
    Repository { repository_id: LogicalRepositoryId },
}

impl CodingWorkspaceEngine {
    /// 按 `attempt.target_snapshot` 与 issue routing 分流 shared worktree 访问。
    ///
    /// 语义（REQ-COD-03 §4.2.3）：
    /// - `Some(snapshot)` → 多仓仓维路径（repository_id 取自
    ///   `snapshot.logical_repository_id`；admission 已保证此时 routing 一致）。
    /// - `None + Legacy` → 单仓老路径（行为不变）。
    /// - `None + Logical` → 防御性 fail-closed（`target_snapshot_missing_for_logical`，
    ///   正常不可能至此，admission 已拦截）。
    /// - `None + FailClosed` → 防御性 fail-closed（`target_snapshot_identity_drifted`）。
    ///
    /// 多仓分支在分流前执行迁移 preflight 断言（§4.2.6）：同 issue 下存在旧
    /// `issue-shared-worktree.json` → fail-closed `legacy_shared_worktree_present`。
    pub(crate) fn route_issue_shared_worktree(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<IssueSharedWorktreeRoute, CodingWorkspaceEngineError> {
        if let Some(snapshot) = &attempt.target_snapshot {
            self.preflight_repo_shared_worktree_absent(attempt)?;
            return Ok(IssueSharedWorktreeRoute::Repository {
                repository_id: snapshot.logical_repository_id,
            });
        }
        match RepositoryRouting::load_for_issue(
            &self.store.paths(),
            &attempt.project_id,
            &attempt.issue_id,
        )? {
            RepositoryRouting::Legacy { .. } => Ok(IssueSharedWorktreeRoute::Legacy),
            RepositoryRouting::Logical { .. } => Err(CodingWorkspaceEngineError::Store(
                ProductStoreError::Io(TARGET_SNAPSHOT_MISSING_FOR_LOGICAL.to_string()),
            )),
            RepositoryRouting::FailClosed { .. } => Err(CodingWorkspaceEngineError::Store(
                ProductStoreError::Io(TARGET_SNAPSHOT_IDENTITY_DRIFTED.to_string()),
            )),
        }
    }

    /// 迁移契约化 preflight（§4.2.6）：多仓路径首次访问仓维 worktree 前，断言同 issue
    /// 下不存在旧 `issue-shared-worktree.json`。发现旧文件 → fail-closed，稳定码
    /// `legacy_shared_worktree_present`；绝不静默覆盖、绝不从旧文件推导 repository。
    fn preflight_repo_shared_worktree_absent(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<(), CodingWorkspaceEngineError> {
        let lifecycle = LifecycleStore::new(self.store.paths());
        let legacy_path =
            lifecycle.issue_shared_worktree_path(&attempt.project_id, &attempt.issue_id);
        if !legacy_path.exists() {
            return Ok(());
        }
        match crate::product::logical_codebase::LegacySharedWorktreeMigration::load_legacy_shared_worktree(
            &self.store.paths(),
            &attempt.project_id,
            &attempt.issue_id,
        ) {
            Ok(None) => Ok(()),
            Ok(Some(_)) => Err(CodingWorkspaceEngineError::LegacySharedWorktreePresent(
                format!("{}/{}", attempt.project_id, attempt.issue_id),
            )),
            Err(ProductStoreError::InvalidRecord { reason, .. })
                if reason.starts_with("legacy_shared_worktree_inconsistent:") =>
            {
                Err(CodingWorkspaceEngineError::Store(ProductStoreError::Io(
                    "legacy_shared_worktree_inconsistent".to_string(),
                )))
            }
            Err(error) => Err(CodingWorkspaceEngineError::Store(error)),
        }
    }

    /// 当 Coder 输出无法进入任何自动化路由（plan defect 契约校验失败，
    /// 或 finding 校验后仍只能人工分诊）时，落地人工分诊 blocked gate，
    /// 避免流程停在 running/coding 而 UI 没有任何可操作入口。
    pub(crate) fn open_coding_output_human_triage_gate(
        &self,
        attempt: &CodingExecutionAttempt,
        node_id: &str,
        report: Option<&ExecutionPlanDefectReport>,
        parse_error: Option<&str>,
        raw_provider_output_ref: Option<String>,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        let description = match (report, parse_error) {
            (Some(report), _) => format!(
                "Coder 报告的 plan defect 需要人工分诊（{} 条 finding），流程已暂停: {}",
                report.findings.len(),
                report
                    .findings
                    .first()
                    .map(|finding| finding.message.clone())
                    .unwrap_or_default()
            ),
            (None, Some(error)) => format!(
                "Coder 完成报告中的 plan_defect_findings 未通过契约校验，流程已暂停并等待人工分诊: {error}"
            ),
            (None, None) => "Coder 输出需要人工分诊，流程已暂停".to_string(),
        };
        let updated = self.store.update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Blocked,
        )?;
        self.store.create_blocked_gate(
            &updated,
            CreateBlockedGateInput {
                attempt_id: attempt.id.clone(),
                stage: CodingExecutionStage::Coding,
                node_id: Some(node_id.to_string()),
                role: Some(CodingProviderRole::Coder),
                title: "Coder 输出需要人工分诊".to_string(),
                description,
                reason_code: Some(CODING_OUTPUT_HUMAN_TRIAGE_REASON_CODE.to_string()),
                evidence_refs: Vec::new(),
                raw_provider_output_ref,
                available_actions: vec![
                    coding_gate_action_for_id("retry_coding").expect("retry coding action"),
                    coding_gate_action_for_id("abort").expect("abort action"),
                ],
            },
        )?;
        Ok(updated)
    }

    pub(crate) async fn emit_permission_request(
        &self,
        node_id: &str,
        provider: &ProviderName,
        request: PermissionRequestData,
    ) {
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingExecutionEvent {
                event: ws_event_from_permission_request(node_id, provider, &request),
            })
            .await;
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingPermissionRequest {
                id: request.id,
                tool_name: request.tool_name,
                description: request.description,
                risk_level: ws_permission_risk_level(request.risk_level),
            })
            .await;
    }

    pub(crate) async fn emit_choice_request(
        &self,
        attempt: &CodingExecutionAttempt,
        node_id: &str,
        stage: CodingExecutionStage,
        role: CodingProviderRole,
        provider: &ProviderName,
        request: ChoiceRequestData,
    ) -> Result<(), CodingWorkspaceEngineError> {
        let source = request.source.as_str().to_string();
        // F-43：落盘 gate 即「未决 choice 的 durable 源」；实时帧与新连接 attach
        // 补发帧都从这条记录构造（`coding_choice_request_frame`），两路同形。
        let gate = self.store.create_choice_gate(
            attempt,
            CreateChoiceGateInput {
                attempt_id: attempt.id.clone(),
                choice_id: request.id.clone(),
                stage,
                node_id: Some(node_id.to_string()),
                role,
                provider: provider.clone(),
                source,
                prompt: request.prompt.clone(),
                options: request
                    .options
                    .iter()
                    .map(|option| CodingChoiceOption {
                        id: option.id.clone(),
                        label: option.label.clone(),
                        description: option.description.clone(),
                    })
                    .collect(),
                allow_multiple: request.allow_multiple,
                allow_free_text: request.allow_free_text,
                questions: request
                    .questions
                    .iter()
                    .map(|question| CodingChoiceQuestion {
                        id: question.id.clone(),
                        prompt: question.prompt.clone(),
                        options: question
                            .options
                            .iter()
                            .map(|option| CodingChoiceOption {
                                id: option.id.clone(),
                                label: option.label.clone(),
                                description: option.description.clone(),
                            })
                            .collect(),
                        allow_multiple: question.allow_multiple,
                        allow_free_text: question.allow_free_text,
                    })
                    .collect(),
            },
        )?;
        let current =
            self.store
                .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        if current.status == CodingAttemptStatus::Running {
            self.store.update_attempt_status(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                CodingAttemptStatus::WaitingForHuman,
            )?;
        }
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingExecutionEvent {
                event: ws_event_from_choice_request(node_id, provider, &request),
            })
            .await;
        let _ = self.event_tx.send(coding_choice_request_frame(&gate)).await;
        Ok(())
    }

    pub(crate) async fn ensure_issue_shared_worktree_clean(
        &self,
        attempt: &CodingExecutionAttempt,
        work_item_id: &str,
    ) -> Result<(), CodingWorkspaceEngineError> {
        let lifecycle = LifecycleStore::new(self.store.paths());
        let shared = match self.route_issue_shared_worktree(attempt)? {
            IssueSharedWorktreeRoute::Legacy => {
                lifecycle.get_issue_shared_worktree(&attempt.project_id, &attempt.issue_id)?
            }
            IssueSharedWorktreeRoute::Repository { repository_id } => lifecycle
                .get_repo_shared_worktree(&attempt.project_id, &attempt.issue_id, repository_id)?,
        };
        let Some(shared) = shared else {
            return Ok(());
        };
        if shared.current_active_work_item_id.as_deref() != Some(work_item_id) {
            return Ok(());
        }
        let worktree_path = shared.worktree_path;
        if !worktree_path.exists() {
            return Ok(());
        }
        self.ensure_worktree_clean_with_manual_gate(
            attempt,
            &worktree_path,
            CodingExecutionStage::FinalConfirm,
        )
        .await
    }

    pub(crate) async fn ensure_worktree_clean_with_manual_gate(
        &self,
        attempt: &CodingExecutionAttempt,
        worktree_path: &Path,
        stage: CodingExecutionStage,
    ) -> Result<(), CodingWorkspaceEngineError> {
        let status = self._git_service.git_status(worktree_path).await?;
        if !status.is_empty() {
            self.store.create_blocked_gate(attempt, CreateBlockedGateInput {
                attempt_id: attempt.id.clone(),
                stage,
                node_id: None,
                role: None,
                title: "Shared worktree has uncommitted changes".to_string(),
                description: "Issue shared worktree has uncommitted changes and must be cleaned up manually before the active lock can be released".to_string(),
                reason_code: Some("shared_worktree_dirty_manual_gate".to_string()),
                evidence_refs: Vec::new(),
                raw_provider_output_ref: None,
                available_actions: vec![
                    coding_gate_action_for_id("manual_continue").expect("manual continue action"),
                    coding_gate_action_for_id("abort").expect("abort action"),
                ],
            })?;
            return Err(CodingWorkspaceEngineError::SharedWorktreeDirtyManualGate(
                worktree_path.to_string_lossy().to_string(),
            ));
        }
        Ok(())
    }

    pub(crate) async fn run_completion_gates(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<CompletionGateReport, CodingWorkspaceEngineError> {
        if attempt.head_commit.is_none() {
            return Err(CodingWorkspaceEngineError::CompletionCommitMissing(
                attempt.id.clone(),
            ));
        }

        let lifecycle = LifecycleStore::new(self.store.paths());
        let work_item = lifecycle
            .list_work_items(&attempt.project_id, &attempt.issue_id)?
            .into_iter()
            .find(|item| item.id == attempt.work_item_id)
            .ok_or_else(|| CodingWorkspaceEngineError::FinalConfirmNotReady(attempt.id.clone()))?;

        let changed_files = self.changed_files_for_attempt(attempt, &work_item).await?;
        let worktree_path = self.attempt_worktree_path(attempt).await.ok();
        self.validate_changed_files_for_work_item(
            &work_item,
            &changed_files,
            worktree_path.as_ref(),
        )?;
        // 所有保留的完成门禁仍必须通过。

        self.ensure_issue_shared_worktree_clean(attempt, &attempt.work_item_id)
            .await?;

        Ok(CompletionGateReport)
    }

    pub(crate) async fn run_group_completion_gates(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<CompletionGateReport, CodingWorkspaceEngineError> {
        if attempt.head_commit.is_none() {
            return Err(CodingWorkspaceEngineError::CompletionCommitMissing(
                attempt.id.clone(),
            ));
        }
        if !self.group_attempt_ready_for_final_review(attempt)? {
            return Err(CodingWorkspaceEngineError::FinalConfirmNotReady(
                attempt.id.clone(),
            ));
        }

        // 所有保留的 group 完成门禁仍必须通过。
        let worktree_path = self.attempt_worktree_path(attempt).await.ok();
        let completed_work_item_ids = self
            .store
            .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)?
            .into_iter()
            .filter(|unit| {
                unit.status == crate::product::coding_models::CodingExecutionUnitStatus::Completed
            })
            .map(|unit| unit.logical_work_item_id)
            .collect::<Vec<_>>();

        // 写入范围门禁的 changed_files 必须来自 git 事实：每个已完成 unit 的
        // start_commit..completion_commit 决定它实际改了哪些文件，从而保住 per-unit 归属判定。
        // 不得依赖交接摘要字段——那会让门禁随摘要移除而空转、越界写入静默放行。
        if self.schema_v2_group_plan_lineage(attempt)?.is_some() {
            for facts in self.schema_v2_group_completion_gate_facts(attempt)? {
                let changed_files = self
                    .changed_files_for_unit_completion_range(attempt, &facts.run)
                    .await?;
                self.validate_changed_files_for_runtime(
                    &facts.runtime,
                    &changed_files,
                    worktree_path.as_ref(),
                )?;
            }
        } else {
            let lifecycle = LifecycleStore::new(self.store.paths());
            let work_items = lifecycle.list_work_items(&attempt.project_id, &attempt.issue_id)?;
            let completed_units = self
                .store
                .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)?
                .into_iter()
                .filter(|unit| {
                    unit.status
                        == crate::product::coding_models::CodingExecutionUnitStatus::Completed
                })
                .collect::<Vec<_>>();
            for unit in &completed_units {
                let work_item = work_items
                    .iter()
                    .find(|item| item.id == unit.logical_work_item_id)
                    .ok_or_else(|| {
                        CodingWorkspaceEngineError::FinalConfirmNotReady(attempt.id.clone())
                    })?;
                let run = self.completed_unit_run_for_group_completion_gate(attempt, unit)?;
                let changed_files = self
                    .changed_files_for_unit_completion_range(attempt, &run)
                    .await?;
                self.validate_changed_files_for_work_item(
                    work_item,
                    &changed_files,
                    worktree_path.as_ref(),
                )?;
            }
        }

        let lifecycle = LifecycleStore::new(self.store.paths());
        let shared_worktree = match self.route_issue_shared_worktree(attempt)? {
            IssueSharedWorktreeRoute::Legacy => {
                lifecycle.get_issue_shared_worktree(&attempt.project_id, &attempt.issue_id)?
            }
            IssueSharedWorktreeRoute::Repository { repository_id } => lifecycle
                .get_repo_shared_worktree(&attempt.project_id, &attempt.issue_id, repository_id)?,
        };
        let lock_holder_work_item_id = shared_worktree
            .and_then(|shared| shared.current_active_work_item_id)
            .or_else(|| attempt.current_work_item_id.clone())
            .or_else(|| completed_work_item_ids.last().cloned())
            .unwrap_or_else(|| attempt.work_item_id.clone());
        self.ensure_issue_shared_worktree_clean(attempt, &lock_holder_work_item_id)
            .await?;

        Ok(CompletionGateReport)
    }

    fn validate_changed_files_for_work_item(
        &self,
        work_item: &LifecycleWorkItemRecord,
        changed_files: &[String],
        worktree_path: Option<&PathBuf>,
    ) -> Result<(), CodingWorkspaceEngineError> {
        for relative_path in changed_files {
            let candidate = std::path::Path::new(relative_path);
            if work_item
                .forbidden_write_scopes
                .iter()
                .any(|scope| scope_allows_path(scope, relative_path, true))
            {
                return Err(CodingWorkspaceEngineError::WorkItemDiffScopeViolation(
                    relative_path.clone(),
                ));
            }
            if !work_item.exclusive_write_scopes.is_empty()
                && let Some(base) = worktree_path
            {
                let _ =
                    validate_write_path(base, &work_item.exclusive_write_scopes, candidate, true)
                        .map_err(|_| {
                        CodingWorkspaceEngineError::WorkItemDiffScopeViolation(
                            relative_path.clone(),
                        )
                    })?;
            }
        }
        Ok(())
    }

    pub(crate) async fn changed_files_for_attempt(
        &self,
        attempt: &CodingExecutionAttempt,
        _work_item: &LifecycleWorkItemRecord,
    ) -> Result<Vec<String>, CodingWorkspaceEngineError> {
        let worktree_path = match self.attempt_worktree_path(attempt).await {
            Ok(path) => path,
            Err(CodingWorkspaceEngineError::MissingWorktree(_)) => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        if !worktree_path.exists() {
            return Ok(Vec::new());
        }
        match self._git_service.git_status(&worktree_path).await {
            Ok(status) => Ok(status.into_iter().map(|file| file.path).collect()),
            Err(_) => Ok(Vec::new()),
        }
    }

    /// 该 attempt 的 provider 流日志目录（Aria 侧）。
    ///
    /// 所有构造 `AdapterInput` 的执行路径都应使用它，而不是留空：留空虽然在当前
    /// 的 streaming fallback 路由下不写日志，但一旦路由变化就会退化成写入目标
    /// 仓库（change `fix-provider-stream-log-location`）。
    pub(crate) fn attempt_provider_stream_log_dir(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> String {
        self.store
            .provider_stream_log_root(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .to_string_lossy()
            .to_string()
    }

    pub(crate) async fn attempt_worktree_path(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<PathBuf, CodingWorkspaceEngineError> {
        if let Some(path) = attempt.worktree_path.as_ref() {
            return Ok(path.clone());
        }
        let lifecycle = LifecycleStore::new(self.store.paths());
        let shared = match self.route_issue_shared_worktree(attempt)? {
            IssueSharedWorktreeRoute::Legacy => {
                lifecycle.get_issue_shared_worktree(&attempt.project_id, &attempt.issue_id)?
            }
            IssueSharedWorktreeRoute::Repository { repository_id } => lifecycle
                .get_repo_shared_worktree(&attempt.project_id, &attempt.issue_id, repository_id)?,
        };
        match shared {
            Some(shared) if shared.worktree_path.exists() => Ok(shared.worktree_path),
            _ => Err(CodingWorkspaceEngineError::MissingWorktree(
                attempt.id.clone(),
            )),
        }
    }

    pub(crate) fn release_issue_shared_worktree_lock_for_attempt(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<(), CodingWorkspaceEngineError> {
        let attempt = self.store.get_attempt(project_id, issue_id, attempt_id)?;
        let lifecycle = LifecycleStore::new(self.store.paths());
        match self.route_issue_shared_worktree(&attempt)? {
            IssueSharedWorktreeRoute::Legacy => {
                if lifecycle
                    .get_issue_shared_worktree(project_id, issue_id)?
                    .is_some()
                {
                    lifecycle
                        .release_issue_worktree_lock_by_owner(project_id, issue_id, attempt_id)?;
                }
            }
            IssueSharedWorktreeRoute::Repository { repository_id } => {
                if lifecycle
                    .get_repo_shared_worktree(project_id, issue_id, repository_id)?
                    .is_some()
                {
                    lifecycle.release_repo_worktree_lock_by_owner(
                        project_id,
                        issue_id,
                        repository_id,
                        attempt_id,
                    )?;
                }
            }
        }
        Ok(())
    }

    pub(crate) fn release_issue_shared_worktree_lock_if_holder(
        &self,
        project_id: &str,
        issue_id: &str,
        work_item_id: &str,
        owner_id: &str,
    ) -> Result<(), CodingWorkspaceEngineError> {
        // owner_id 语义上即持有该锁的 attempt id（所有调用方均以 attempt_id 传入）。
        let attempt = self.store.get_attempt(project_id, issue_id, owner_id)?;
        let lifecycle = LifecycleStore::new(self.store.paths());
        match self.route_issue_shared_worktree(&attempt)? {
            IssueSharedWorktreeRoute::Legacy => {
                let Some(shared) = lifecycle.get_issue_shared_worktree(project_id, issue_id)?
                else {
                    return Ok(());
                };
                if shared.current_active_work_item_id.is_some()
                    || shared.current_lock_owner_id.is_some()
                {
                    lifecycle.release_issue_worktree_lock(
                        project_id,
                        issue_id,
                        work_item_id,
                        owner_id,
                    )?;
                }
            }
            IssueSharedWorktreeRoute::Repository { repository_id } => {
                let Some(shared) =
                    lifecycle.get_repo_shared_worktree(project_id, issue_id, repository_id)?
                else {
                    return Ok(());
                };
                if shared.current_active_work_item_id.is_some()
                    || shared.current_lock_owner_id.is_some()
                {
                    lifecycle.release_repo_worktree_lock(
                        project_id,
                        issue_id,
                        repository_id,
                        work_item_id,
                        owner_id,
                    )?;
                }
            }
        }
        Ok(())
    }

    pub(crate) fn validate_attempt_issue_shared_worktree_owner_if_present(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<(), CodingWorkspaceEngineError> {
        let lifecycle = LifecycleStore::new(self.store.paths());
        let route = self.route_issue_shared_worktree(attempt)?;
        let shared = match &route {
            IssueSharedWorktreeRoute::Legacy => {
                lifecycle.get_issue_shared_worktree(&attempt.project_id, &attempt.issue_id)?
            }
            IssueSharedWorktreeRoute::Repository { repository_id } => lifecycle
                .get_repo_shared_worktree(&attempt.project_id, &attempt.issue_id, *repository_id)?,
        };
        let Some(shared) = shared else {
            return Ok(());
        };
        let Some(active_work_item_id) = shared.current_active_work_item_id.as_deref() else {
            return Ok(());
        };
        match route {
            IssueSharedWorktreeRoute::Legacy => {
                lifecycle.validate_issue_worktree_lock_owner(
                    &attempt.project_id,
                    &attempt.issue_id,
                    active_work_item_id,
                    &attempt.id,
                )?;
            }
            IssueSharedWorktreeRoute::Repository { repository_id } => {
                lifecycle.validate_repo_worktree_lock_owner(
                    &attempt.project_id,
                    &attempt.issue_id,
                    repository_id,
                    active_work_item_id,
                    &attempt.id,
                )?;
            }
        }
        Ok(())
    }

    pub(crate) fn validate_attempt_issue_shared_worktree_lock_if_present(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<(), CodingWorkspaceEngineError> {
        let lifecycle = LifecycleStore::new(self.store.paths());
        let route = self.route_issue_shared_worktree(attempt)?;
        let shared = match &route {
            IssueSharedWorktreeRoute::Legacy => {
                lifecycle.get_issue_shared_worktree(&attempt.project_id, &attempt.issue_id)?
            }
            IssueSharedWorktreeRoute::Repository { repository_id } => lifecycle
                .get_repo_shared_worktree(&attempt.project_id, &attempt.issue_id, *repository_id)?,
        };
        let Some(shared) = shared else {
            return Ok(());
        };
        let work_item_id = attempt
            .current_work_item_id
            .as_deref()
            .or_else(|| {
                (attempt.scope == crate::product::coding_models::CodingAttemptScope::WorkItemGroup)
                    .then_some(shared.current_active_work_item_id.as_deref())
                    .flatten()
            })
            .unwrap_or(&attempt.work_item_id);
        match route {
            IssueSharedWorktreeRoute::Legacy => {
                lifecycle.validate_issue_worktree_lock_owner(
                    &attempt.project_id,
                    &attempt.issue_id,
                    work_item_id,
                    &attempt.id,
                )?;
            }
            IssueSharedWorktreeRoute::Repository { repository_id } => {
                lifecycle.validate_repo_worktree_lock_owner(
                    &attempt.project_id,
                    &attempt.issue_id,
                    repository_id,
                    work_item_id,
                    &attempt.id,
                )?;
            }
        }
        Ok(())
    }

    pub(crate) fn mark_issue_shared_worktree_completed_if_present(
        &self,
        project_id: &str,
        issue_id: &str,
        work_item_id: &str,
        owner_id: &str,
    ) -> Result<(), CodingWorkspaceEngineError> {
        // owner_id 语义上即持有该锁的 attempt id（所有调用方均以 attempt_id 传入）。
        let attempt = self.store.get_attempt(project_id, issue_id, owner_id)?;
        let lifecycle = LifecycleStore::new(self.store.paths());
        match self.route_issue_shared_worktree(&attempt)? {
            IssueSharedWorktreeRoute::Legacy => {
                if lifecycle
                    .get_issue_shared_worktree(project_id, issue_id)?
                    .is_some()
                {
                    lifecycle.mark_issue_worktree_completed_item(
                        project_id,
                        issue_id,
                        work_item_id,
                        owner_id,
                    )?;
                }
            }
            IssueSharedWorktreeRoute::Repository { repository_id } => {
                if lifecycle
                    .get_repo_shared_worktree(project_id, issue_id, repository_id)?
                    .is_some()
                {
                    lifecycle.mark_repo_worktree_completed_item(
                        project_id,
                        issue_id,
                        repository_id,
                        work_item_id,
                        owner_id,
                    )?;
                }
            }
        }
        Ok(())
    }

    pub(crate) async fn release_active_lock_if_shared_worktree_clean(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        work_item_id: &str,
    ) -> Result<(), CodingWorkspaceEngineError> {
        let attempt = self.store.get_attempt(project_id, issue_id, attempt_id)?;
        match self
            .ensure_issue_shared_worktree_clean(&attempt, work_item_id)
            .await
        {
            Ok(()) => self.release_issue_shared_worktree_lock_if_holder(
                project_id,
                issue_id,
                work_item_id,
                attempt_id,
            ),
            Err(CodingWorkspaceEngineError::SharedWorktreeDirtyManualGate(_)) => Ok(()),
            Err(error) => Err(error),
        }
    }

    pub async fn handle_blocked_gate_response(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        gate_id: &str,
        action_id: &str,
        extra_context: Option<String>,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        let Some(gate) = self
            .store
            .list_open_blocked_gates(project_id, issue_id, attempt_id)?
            .into_iter()
            .find(|gate| gate.gate_id == gate_id)
        else {
            return Ok(self.store.get_attempt(project_id, issue_id, attempt_id)?);
        };
        let code_review_provider_interrupted =
            super::failed_review_recovery::is_code_review_provider_interrupted_gate(&gate);
        if code_review_provider_interrupted && action_id != "retry_review" {
            return Err(CodingWorkspaceEngineError::ProviderStream(
                "coding_failed_review_recovery_action_not_allowed".to_string(),
            ));
        }
        let action = gate
            .available_actions
            .iter()
            .find(|action| action.action_id == action_id)
            .ok_or_else(|| {
                CodingWorkspaceEngineError::ProviderStream(
                    "coding_gate_action_not_allowed".to_string(),
                )
            })?;
        if action.action_type == CodingGateActionType::RetryReview
            && code_review_provider_interrupted
        {
            return Err(CodingWorkspaceEngineError::ProviderStream(
                "coding_failed_review_recovery_requires_reservation".to_string(),
            ));
        }
        let should_resolve_gate =
            !matches!(action.action_type, CodingGateActionType::ProvideContext);

        let current = self.store.get_attempt(project_id, issue_id, attempt_id)?;
        let updated = match action.action_type {
            CodingGateActionType::Abort => {
                self.handle_abort(project_id, issue_id, attempt_id).await?
            }
            CodingGateActionType::RetryCoding => {
                let coder_provider = self
                    .store
                    .get_role_provider_config_snapshot(
                        &current.project_id,
                        &current.issue_id,
                        &current.id,
                    )?
                    .coder;
                let cleared = self.clear_attempt_provider_conversation(
                    &current,
                    &CodingProviderRole::Coder,
                    &coder_provider,
                )?;
                let resumed =
                    self.resume_blocked_attempt_at_stage(&cleared, CodingExecutionStage::Coding)?;
                if let Some(failed) = self.store.latest_role_run(
                    &resumed.project_id,
                    &resumed.issue_id,
                    &resumed.id,
                    CodingExecutionStage::Coding,
                    CodingProviderRole::Coder,
                )? && failed.status == CodingRoleRunStatus::Failed
                {
                    self.store.create_manual_retry_role_run(
                        &resumed,
                        CodingExecutionStage::Coding,
                        CodingProviderRole::Coder,
                        &failed,
                        gate.reason_code.clone(),
                    )?;
                }
                resumed
            }
            CodingGateActionType::RetryReview => {
                if gate.stage == Some(CodingExecutionStage::InternalPrReview)
                    || gate.role == Some(CodingProviderRole::InternalReviewer)
                {
                    let resumed = self.resume_blocked_attempt_at_stage(
                        &current,
                        CodingExecutionStage::InternalPrReview,
                    )?;
                    self.store.supersede_latest_role_run_and_create(
                        &resumed,
                        CodingExecutionStage::InternalPrReview,
                        CodingProviderRole::InternalReviewer,
                        CodingRoleRunTrigger::RetryInternalReview,
                        None,
                        gate.reason_code.clone(),
                    )?;
                    resumed
                } else {
                    let resumed = self.resume_blocked_attempt_at_stage(
                        &current,
                        CodingExecutionStage::CodeReview,
                    )?;
                    self.store.supersede_latest_role_run_and_create(
                        &resumed,
                        CodingExecutionStage::CodeReview,
                        CodingProviderRole::CodeReviewer,
                        CodingRoleRunTrigger::RetryReview,
                        None,
                        gate.reason_code.clone(),
                    )?;
                    resumed
                }
            }
            CodingGateActionType::RetryInternalReview => {
                let resumed = self.resume_blocked_attempt_at_stage(
                    &current,
                    CodingExecutionStage::InternalPrReview,
                )?;
                self.store.supersede_latest_role_run_and_create(
                    &resumed,
                    CodingExecutionStage::InternalPrReview,
                    CodingProviderRole::InternalReviewer,
                    CodingRoleRunTrigger::RetryInternalReview,
                    None,
                    gate.reason_code.clone(),
                )?;
                resumed
            }
            CodingGateActionType::RetryGroupReviewShard
            | CodingGateActionType::RetryGroupReduction => self.resume_blocked_attempt_at_stage(
                &current,
                CodingExecutionStage::InternalPrReview,
            )?,
            CodingGateActionType::SendToCoder => {
                if is_code_review_feedback_gate(&gate) {
                    self.send_code_review_feedback_to_coder(&current, extra_context)?
                } else {
                    self.send_review_limit_feedback_to_coder(&current, extra_context)?
                }
            }
            CodingGateActionType::ProvideContext => {
                if let Some(content) = extra_context
                    && !content.trim().is_empty()
                {
                    self.store.create_context_note(&current, content)?;
                }
                let running = if current.status == CodingAttemptStatus::Blocked {
                    self.store.admit_and_transition_attempt_to_executable(
                        project_id, issue_id, attempt_id,
                    )?
                } else {
                    current
                };
                self.store.update_attempt_status(
                    &running.project_id,
                    &running.issue_id,
                    &running.id,
                    CodingAttemptStatus::WaitingForHuman,
                )?
            }
            CodingGateActionType::ManualContinue | CodingGateActionType::AcceptRisk => {
                let operator_context = extra_context
                    .map(|content| content.trim().to_string())
                    .filter(|content| !content.is_empty())
                    .ok_or_else(|| {
                        CodingWorkspaceEngineError::ProviderStream(
                            "coding_gate_extra_context_required".to_string(),
                        )
                    })?;
                self.store
                    .create_context_note(&current, operator_context.clone())?;
                self.store.create_quality_bypass_audit(
                    &current,
                    CreateQualityBypassAuditInput {
                        attempt_id: current.id.clone(),
                        gate_id: gate.gate_id.clone(),
                        stage: gate.stage.clone().unwrap_or_else(|| current.stage.clone()),
                        reason_code: gate.reason_code.clone(),
                        operator_context,
                    },
                )?;
                if current.status == CodingAttemptStatus::Blocked {
                    self.store.admit_and_transition_attempt_to_executable(
                        project_id, issue_id, attempt_id,
                    )?
                } else {
                    current
                }
            }
            _ => {
                return Err(CodingWorkspaceEngineError::ProviderStream(
                    "coding_gate_action_not_allowed".to_string(),
                ));
            }
        };
        if should_resolve_gate {
            self.store
                .resolve_blocked_gate(project_id, issue_id, attempt_id, gate_id)?;
        }
        Ok(updated)
    }

    pub(crate) fn resume_blocked_attempt_at_stage(
        &self,
        current: &CodingExecutionAttempt,
        stage: CodingExecutionStage,
    ) -> Result<CodingExecutionAttempt, ProductStoreError> {
        let mut updated = if matches!(
            current.status,
            CodingAttemptStatus::Blocked | CodingAttemptStatus::WaitingForHuman
        ) {
            self.store.admit_and_transition_attempt_to_executable(
                &current.project_id,
                &current.issue_id,
                &current.id,
            )?
        } else {
            current.clone()
        };
        if updated.stage != stage {
            updated = self.store.update_attempt_stage(
                &updated.project_id,
                &updated.issue_id,
                &updated.id,
                stage,
            )?;
        }
        Ok(updated)
    }

    // ─── C1 Task 6（REQ-WIGA-03）：租约三态判定与确认接管 ───

    /// C2 Task 2（REQ-CRO-02）：engine 侧统一 admission 入口——把 store 的
    /// `admit_coding_run_exclusive`（命令账本 → 租约三态 → 既有
    /// `ensure_provider_run_allowed`）映射为既有错误面；互斥停等以稳定
    /// reason code 呈现（已在运行／需确认接管／租约未知），不启动 provider。
    pub(crate) fn admit_provider_run(
        &self,
        attempt: &CodingExecutionAttempt,
        stage: &CodingExecutionStage,
        purpose: &str,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        use crate::product::coding_attempt_store::CodingRunExclusionDecision;

        let command_id = format!("admit-{}-{:?}-{}", attempt.id, stage, purpose);
        let payload_digest = format!(
            "{}|{}|{}|{:?}|{}",
            attempt.project_id, attempt.issue_id, attempt.id, stage, purpose
        );
        match self
            .store
            .admit_coding_run_exclusive(attempt, &command_id, &payload_digest)?
        {
            CodingRunExclusionDecision::Allowed(authoritative) => Ok(authoritative),
            CodingRunExclusionDecision::AlreadyRunning { lease } => {
                Err(CodingWorkspaceEngineError::ProviderStream(format!(
                    "coding_run_already_running: lease {} 已在运行，请等待",
                    lease.lease_id
                )))
            }
            CodingRunExclusionDecision::TakeoverRequired { lease } => {
                Err(CodingWorkspaceEngineError::ProviderStream(format!(
                    "coding_run_takeover_required: lease {} 持有者已终止，需确认接管后继续",
                    lease.lease_id
                )))
            }
            CodingRunExclusionDecision::LeaseUnknown { lease } => {
                Err(CodingWorkspaceEngineError::ProviderStream(format!(
                    "coding_run_lease_unknown: 租约状态未知，已停等：{}",
                    lease.evidence.join("; ")
                )))
            }
        }
    }

    /// 租约三态判定（应用服务，只读；实现移至 `CodingAttemptStore`，此处
    /// 委托保持既有公共签名，单一实现见 run_exclusion.rs）。只从现有
    /// durable 证据分类：活跃→`ActiveWait`（等待，不抢占）；owner 终态或
    /// 锁已明确释放→`DeadNeedsTakeover`（需确认才接管）；瞬态 owner、
    /// 证据缺失或读失败→`UnknownNeedsHuman`（绝不抢占）。
    pub fn classify_worktree_lease(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> crate::product::models::automation::LeaseDecision {
        self.store.classify_worktree_lease(project_id, issue_id)
    }


    /// 死亡租约确认接管（应用服务）。前置链：命令账本幂等（同 command
    /// 同 payload Replayed、异 payload fail-closed）→ 当前 enrollment
    /// binding 精确校验（过期/异载体 IdentityMismatch）→ 三态判定（活跃
    /// Rejected、未知 NeedsHuman，均不写任何文件）→ expected attempt 终态
    /// 证明 → 既有 worktree 文件锁内 owner CAS 清出死亡 owner（新 owner
    /// 由当前 binding 下的下一次合法 acquire 写入）。接管事实追加命令
    /// 账本；终态 attempt 记录只读保留，不建第二 attempt、不启动 provider。
    pub fn confirm_takeover(
        &self,
        project_id: &str,
        issue_id: &str,
        request: &crate::product::models::automation::LeaseTakeoverRequest,
    ) -> Result<crate::product::models::automation::LeaseTakeoverResult, ProductStoreError> {
        use crate::product::issue_automation_store::IssueAutomationStore;
        use crate::product::json_store::validate_relative_id;
        use crate::product::lifecycle_store::LifecycleStore;
        use crate::product::models::automation::{
            EnrollmentError, LeaseTakeoverResult, OperationState,
        };

        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        let automation = IssueAutomationStore::new(self.store.paths());

        // 命令账本幂等：同 command 同 payload 重放首次 durable 结果。
        let digest = request.payload_digest();
        if automation
            .find_command_result(project_id, issue_id, &request.command_id, &digest)
            .map_err(lease_takeover_store_error)?
            .is_some()
        {
            return Ok(LeaseTakeoverResult {
                command_id: request.command_id.clone(),
                state: OperationState::Replayed,
                lease: self.classify_worktree_lease(project_id, issue_id),
            });
        }

        // 当前 binding 精确校验：过期/异载体/缺 enrollment 一律拒绝。
        let enrollment = automation.get(project_id, issue_id)?;
        let binding = enrollment
            .and_then(|enrollment| enrollment.binding_history)
            .map(|history| history.current)
            .ok_or_else(|| {
                lease_takeover_identity_mismatch("enrollment_binding", &request.command_id)
            })?;
        if binding != request.expected_binding {
            return Err(lease_takeover_identity_mismatch(
                "enrollment_binding",
                &format!(
                    "expected binding v{}, current binding v{}",
                    request.expected_binding.binding_version, binding.binding_version
                ),
            ));
        }

        // 三态判定：活跃只等待（Rejected）、未知停等（NeedsHuman），
        // 都不写任何 durable 文件。
        let decision = self.classify_worktree_lease(project_id, issue_id);
        match decision.disposition {
            crate::product::models::automation::LeaseDisposition::ActiveWait => {
                return Ok(LeaseTakeoverResult {
                    command_id: request.command_id.clone(),
                    state: OperationState::Rejected,
                    lease: decision,
                });
            }
            crate::product::models::automation::LeaseDisposition::UnknownNeedsHuman => {
                return Ok(LeaseTakeoverResult {
                    command_id: request.command_id.clone(),
                    state: OperationState::NeedsHuman,
                    lease: decision,
                });
            }
            crate::product::models::automation::LeaseDisposition::DeadNeedsTakeover => {}
        }

        // 死亡证明：expected attempt 必须存在于本 issue 且已终态；活跃或
        // 缺失均 fail-closed（不猜、不接管）。
        let dead_attempt = self
            .store
            .get_attempt(project_id, issue_id, &request.expected_attempt_id)
            .map_err(|_| {
                lease_takeover_identity_mismatch(
                    "lease_takeover_attempt",
                    &request.expected_attempt_id,
                )
            })?;
        if dead_attempt.status.is_active() {
            let mut decision = decision;
            decision.evidence.push(format!(
                "attempt {} is still active; takeover refused",
                dead_attempt.id
            ));
            return Ok(LeaseTakeoverResult {
                command_id: request.command_id.clone(),
                state: OperationState::Rejected,
                lease: decision,
            });
        }

        let lifecycle = LifecycleStore::new(self.store.paths());
        // 已释放（owner=None）的 Dead 判定无需 CAS 清出；带 owner 的死亡
        // 租约在既有 worktree 文件锁内按 expected_lease_id CAS 清出（issue
        // 维优先，其次按确定性顺序匹配仓维记录；迟到旧 owner 写入统一
        // IdentityMismatch）。
        let mut evidence = decision.evidence.clone();
        if !decision.lease_id.is_empty() {
            if decision.lease_id != request.expected_lease_id {
                return Err(lease_takeover_identity_mismatch(
                    "lease_takeover_owner",
                    &format!(
                        "expected lease {}, classified owner {}",
                        request.expected_lease_id, decision.lease_id
                    ),
                ));
            }
            let issue_record = lifecycle.get_issue_shared_worktree(project_id, issue_id)?;
            if issue_record
                .as_ref()
                .and_then(|record| record.current_lock_owner_id.as_deref())
                == Some(decision.lease_id.as_str())
            {
                lifecycle.takeover_issue_worktree_lock_after_dead_owner(
                    project_id,
                    issue_id,
                    &decision.lease_id,
                )?;
            } else {
                let mut matched = false;
                for repository_id in lifecycle.list_repo_shared_worktrees(project_id, issue_id)? {
                    let record =
                        lifecycle.get_repo_shared_worktree(project_id, issue_id, repository_id)?;
                    if record
                        .as_ref()
                        .and_then(|record| record.current_lock_owner_id.as_deref())
                        == Some(decision.lease_id.as_str())
                    {
                        lifecycle.takeover_repo_worktree_lock_after_dead_owner(
                            project_id,
                            issue_id,
                            repository_id,
                            &decision.lease_id,
                        )?;
                        matched = true;
                        break;
                    }
                }
                if !matched {
                    return Err(lease_takeover_identity_mismatch(
                        "lease_takeover_owner",
                        &decision.lease_id,
                    ));
                }
            }
            evidence.push(format!(
                "takeover_confirmed: dead owner {} cleared under binding v{}",
                decision.lease_id, binding.binding_version
            ));
        } else {
            evidence.push(
                "takeover_confirmed: lock already released; next acquire becomes the owner"
                    .to_string(),
            );
        }

        // 接管事实入命令账本（durable 结果；重放由前置账本查询承担）。
        automation
            .append_command_result(
                project_id,
                issue_id,
                &request.command_id,
                &digest,
                OperationState::Accepted,
            )
            .map_err(lease_takeover_store_error)?;

        let mut lease = self.classify_worktree_lease(project_id, issue_id);
        lease.evidence.extend(evidence);
        Ok(LeaseTakeoverResult {
            command_id: request.command_id.clone(),
            state: OperationState::Accepted,
            lease,
        })
    }
}

/// C1 Task 6：接管链上的身份不匹配（过期 binding/lease/attempt）统一
/// `IdentityMismatch`——迟到旧 owner 写入不改 current owner。
fn lease_takeover_identity_mismatch(kind: &'static str, id: &str) -> ProductStoreError {
    ProductStoreError::IdentityMismatch {
        kind,
        id: id.to_string(),
    }
}

/// C1 Task 6：enrollment store 错误透传（账本读失败不吞成 Unknown——
/// 判定面 fail-closed，由上层决定停等）。
fn lease_takeover_store_error(
    error: crate::product::models::automation::EnrollmentError,
) -> ProductStoreError {
    use crate::product::models::automation::EnrollmentError;

    match error {
        EnrollmentError::Store(error) => error,
        EnrollmentError::Conflict { current_revision } => ProductStoreError::Conflict {
            kind: "enrollment_command_ledger",
            id: format!("current_revision={current_revision:?}"),
        },
        EnrollmentError::InvalidScope(reason) => ProductStoreError::Io(reason),
        EnrollmentError::NotFound => ProductStoreError::NotFound {
            kind: "automation_enrollment",
            id: "lease_takeover".to_string(),
        },
    }
}

fn is_code_review_feedback_gate(gate: &CodingGateRequired) -> bool {
    gate.stage == Some(CodingExecutionStage::CodeReview)
        && gate.role == Some(CodingProviderRole::CodeReviewer)
        && matches!(
            gate.reason_code.as_deref(),
            Some("code_review_blocked")
                | Some("code_review_output_human_triage")
                | Some("code_review_verification_incomplete")
                | Some("code_review_operational_blocker")
        )
}
