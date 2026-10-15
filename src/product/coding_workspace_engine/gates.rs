use super::*;
use crate::product::coding_attempt_store::admission::{
    TARGET_SNAPSHOT_IDENTITY_DRIFTED, TARGET_SNAPSHOT_MISSING_FOR_LOGICAL,
};
use crate::product::logical_codebase::{LogicalRepositoryId, RepositoryRouting};

mod schema_v2;

pub(crate) const CODING_OUTPUT_HUMAN_TRIAGE_REASON_CODE: &str = "coding_output_human_triage";

/// P2-b（REQ-COD-08、REQ-CRO-09）：coder 路径运行环境阻塞门 reason_code。
pub(crate) const CODING_OPERATIONAL_BLOCKER_REASON_CODE: &str = "coding_operational_blocker";

/// C2 Task 8（REQ-CVT-03/04，#19）：转入验证处理请求（REST／页面同一
/// 应用服务，路由 Task 12 统一接线）。plan_revision／scope／expiry 由
/// 应用服务按 attempt 权威绑定派生。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationTriageEntryRequest {
    pub finding_id: String,
    pub check_id: String,
    pub original_command: Option<String>,
    pub alternative_command: Option<String>,
    pub cwd: Option<String>,
    pub outcome: Option<String>,
    pub test_execution_count: Option<u64>,
    pub environment: Option<String>,
}

/// C2 Task 8：验证处理决定请求（三类结论均需用户明确批准）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationTriageDecisionRequest {
    pub triage_id: String,
    pub conclusion: crate::product::coding_attempt_store::VerificationTriageConclusion,
    pub decided_by: String,
    pub reason: String,
    pub exemption_scope: Vec<String>,
}

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
        "retry_coding_with_context" => Some(CodingGateAction {
            action_id: "retry_coding_with_context".to_string(),
            label: "我来说明情况，让 AI 继续".to_string(),
            action_type: CodingGateActionType::RetryCodingWithContext,
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
    pub(crate) async fn open_coding_output_human_triage_gate(
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
        // r51(#1 残点,coding fresh 现场):门必须同帧可见——只落盘不发帧时,
        // 客户端(WS 泵)无门可应答,r51 现场 blocked 后 35min 零 gate 帧
        // 空转到阶段超时。与 rework 上限门/审查中断门同构:落盘后发射
        // CodingGateRequired(durable-first,断连不回滚业务事实)。
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingGateRequired { gate: self
                .store
                .list_open_blocked_gates(&updated.project_id, &updated.issue_id, &updated.id)?
                .into_iter()
                .find(|gate| gate.reason_code.as_deref() == Some(CODING_OUTPUT_HUMAN_TRIAGE_REASON_CODE))
                .expect("triage gate just created") })
            .await;
        Ok(updated)
    }

    /// P2-b（REQ-COD-08、REQ-CRO-09）：coder 路径 `OpenOperationalGate` 决策的
    /// 落门分支——运行环境类阻塞（defect_class=operational_blocker ∧
    /// recommended_route=operational_gate）落「运行环境阻塞」门。与
    /// `open_coding_output_human_triage_gate` 同构（Blocked／Coding／Coder、
    /// create_blocked_gate、落盘后同帧发 CodingGateRequired），差异只在
    /// reason_code／标题／动作多一个「我来说明情况，让 AI 继续」——供统计与
    /// 后续自动化接管区分「环境问题」与「输出格式问题」。
    pub(crate) async fn open_coding_operational_blocker_gate(
        &self,
        attempt: &CodingExecutionAttempt,
        node_id: &str,
        report: Option<&ExecutionPlanDefectReport>,
        raw_provider_output_ref: Option<String>,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        let description = match report {
            Some(report) => format!(
                "Coder 报告运行环境阻塞（{} 条 finding），流程已暂停: {}",
                report.findings.len(),
                report
                    .findings
                    .first()
                    .map(|finding| finding.message.clone())
                    .unwrap_or_default()
            ),
            None => "Coder 报告运行环境阻塞，流程已暂停".to_string(),
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
                title: "运行环境阻塞".to_string(),
                description,
                reason_code: Some(CODING_OPERATIONAL_BLOCKER_REASON_CODE.to_string()),
                evidence_refs: Vec::new(),
                raw_provider_output_ref,
                available_actions: vec![
                    coding_gate_action_for_id("retry_coding").expect("retry coding action"),
                    coding_gate_action_for_id("retry_coding_with_context")
                        .expect("retry coding with context action"),
                    coding_gate_action_for_id("abort").expect("abort action"),
                ],
            },
        )?;
        // 与 open_coding_output_human_triage_gate 同构：门必须同帧可见——
        // 落盘后立即发射 CodingGateRequired（durable-first，断连不回滚业务事实）。
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingGateRequired { gate: self
                .store
                .list_open_blocked_gates(&updated.project_id, &updated.issue_id, &updated.id)?
                .into_iter()
                .find(|gate| gate.reason_code.as_deref() == Some(CODING_OPERATIONAL_BLOCKER_REASON_CODE))
                .expect("operational blocker gate just created") })
            .await;
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
            attempt,
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
                    attempt,
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
                    attempt,
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
        attempt: &CodingExecutionAttempt,
        work_item: &LifecycleWorkItemRecord,
        changed_files: &[String],
        worktree_path: Option<&PathBuf>,
    ) -> Result<(), CodingWorkspaceEngineError> {
        if changed_files.is_empty() {
            return Ok(());
        }
        let member_prefix = self.scope_member_prefix(attempt)?;
        validate_changed_files_in_scope(
            &work_item.exclusive_write_scopes,
            &work_item.forbidden_write_scopes,
            changed_files,
            worktree_path,
            member_prefix.as_deref(),
        )
    }

    /// 多仓 attempt 的 scope 路径域前缀：目标成员 checkout 相对聚合根
    /// （manifest `provider_context_root`）的目录，如 `busi`。
    ///
    /// 多仓 Work Item 的 scope 以聚合根为基准书写（`busi/src/a.ts`），而 git 事实是
    /// 仓内相对路径（`src/a.ts`）。`target_snapshot == None`（单仓）与成员即聚合根
    /// 时返回 `None`（行为不变）；manifest 缺失或 checkout 不在聚合根下一律
    /// fail-closed，绝不退回无前缀比较。
    pub(crate) fn scope_member_prefix(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<Option<String>, CodingWorkspaceEngineError> {
        let Some(snapshot) = attempt.target_snapshot.as_ref() else {
            return Ok(None);
        };
        let unresolved = |reason: String| {
            CodingWorkspaceEngineError::WorkItemScopeDomainUnresolved(format!(
                "{}: {reason}",
                attempt.id
            ))
        };
        let paths = self.store.paths();
        let lc_id = crate::product::logical_codebase::resolve_issue_logical_codebase_id(
            &paths,
            &attempt.project_id,
            &attempt.issue_id,
        )
        .map_err(|error| unresolved(error.to_string()))?;
        let authority = match lc_id.as_deref() {
            Some(lc_id) => {
                crate::product::logical_codebase::LogicalCodebaseStore::for_lc(paths.clone(), lc_id)
            }
            None => crate::product::logical_codebase::LogicalCodebaseStore::new(paths.clone()),
        };
        let manifest = authority
            .load_manifest(&attempt.project_id)
            .map_err(|error| unresolved(error.to_string()))?
            .ok_or_else(|| unresolved("logical codebase manifest missing".to_string()))?;
        let root = &manifest.provider_context_root;
        let member = &snapshot.canonical_path;
        let relative = member
            .strip_prefix(root)
            .ok()
            .map(Path::to_path_buf)
            .or_else(|| {
                let root = root.canonicalize().ok()?;
                let member = member.canonicalize().ok()?;
                member.strip_prefix(&root).ok().map(Path::to_path_buf)
            })
            .ok_or_else(|| {
                unresolved(format!(
                    "member checkout {} is outside provider_context_root {}",
                    member.display(),
                    root.display()
                ))
            })?;
        let prefix = relative.to_string_lossy().replace('\\', "/");
        let prefix = prefix.trim_matches('/');
        if prefix.is_empty() {
            return Ok(None);
        }
        Ok(Some(prefix.to_string()))
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
}

include!("gates_parts/worktree_lock.inc.rs");

include!("gates_parts/blocked_gate.inc.rs");

include!("gates_parts/verification_triage.inc.rs");

/// final_confirm 写入范围门禁（单仓 legacy 与 schema v2 共用）。
///
/// forbidden：仓内相对与「根相对带成员前缀」两种写法任一命中即违规；
/// exclusive：非空且有 worktree 时，任一写法命中才放行。路径卫生
/// （`.git`/`.aria` 禁写、symlink 逃逸）沿用 `validate_write_path`。
/// 报错 payload 保持仓内相对路径。
pub(super) fn validate_changed_files_in_scope(
    exclusive_scopes: &[String],
    forbidden_scopes: &[String],
    changed_files: &[String],
    worktree_path: Option<&PathBuf>,
    member_prefix: Option<&str>,
) -> Result<(), CodingWorkspaceEngineError> {
    let hygiene_scope = ["*".to_string()];
    for relative_path in changed_files {
        let violation =
            || CodingWorkspaceEngineError::WorkItemDiffScopeViolation(relative_path.clone());
        if forbidden_scopes
            .iter()
            .any(|scope| scope_allows_member_path(scope, relative_path, member_prefix, true))
        {
            return Err(violation());
        }
        if !exclusive_scopes.is_empty()
            && let Some(base) = worktree_path
        {
            let normalized = validate_write_path(
                base,
                &hygiene_scope,
                std::path::Path::new(relative_path),
                true,
            )
            .map_err(|_| violation())?;
            if !exclusive_scopes
                .iter()
                .any(|scope| scope_allows_member_path(scope, &normalized, member_prefix, true))
            {
                return Err(violation());
            }
        }
    }
    Ok(())
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
