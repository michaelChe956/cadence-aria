use super::*;
use crate::product::coding_attempt_store::admission::{
    TARGET_SNAPSHOT_IDENTITY_DRIFTED, TARGET_SNAPSHOT_MISSING_FOR_LOGICAL,
};
use crate::product::coding_models::{CodingTriageDid, CodingTriageFinding, CodingTriageSummary};
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
        let gate = self.store.create_blocked_gate(
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
        // P2-c（REQ-CRO-09，C1 裁定）：分诊卡大白话摘要经 store 独立入口
        // 回写（F-7：摘要构造/落盘绝不阻塞开门，任何失败仅静默降级）。
        self.attach_coder_triage_summary(&updated, &gate, report, parse_error);
        // r51(#1 残点,coding fresh 现场):门必须同帧可见——只落盘不发帧时,
        // 客户端(WS 泵)无门可应答,r51 现场 blocked 后 35min 零 gate 帧
        // 空转到阶段超时。与 rework 上限门/审查中断门同构:落盘后发射
        // CodingGateRequired(durable-first,断连不回滚业务事实)。
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingGateRequired {
                gate: self
                    .store
                    .list_open_blocked_gates(&updated.project_id, &updated.issue_id, &updated.id)?
                    .into_iter()
                    .find(|gate| {
                        gate.reason_code.as_deref() == Some(CODING_OUTPUT_HUMAN_TRIAGE_REASON_CODE)
                    })
                    .expect("triage gate just created"),
            })
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
        let gate = self.store.create_blocked_gate(
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
        // P2-c：与人工分诊门同构——摘要经 store 独立入口回写，绝不阻塞开门。
        self.attach_coder_triage_summary(&updated, &gate, report, None);
        // 与 open_coding_output_human_triage_gate 同构：门必须同帧可见——
        // 落盘后立即发射 CodingGateRequired（durable-first，断连不回滚业务事实）。
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingGateRequired {
                gate: self
                    .store
                    .list_open_blocked_gates(&updated.project_id, &updated.issue_id, &updated.id)?
                    .into_iter()
                    .find(|gate| {
                        gate.reason_code.as_deref() == Some(CODING_OPERATIONAL_BLOCKER_REASON_CODE)
                    })
                    .expect("operational blocker gate just created"),
            })
            .await;
        Ok(updated)
    }

    /// P2-c（REQ-CRO-09）：分诊门大白话摘要的引擎侧接线——门已落盘后，从
    /// `raw_provider_output_ref`（durable 源）读回 coder 原始输出，构造
    /// `CodingTriageSummary` 并经 store 独立入口回写。F-7：摘要构造/落盘
    /// 绝不阻塞开门——引用缺失、文件丢失、解析降级、回写失败都静默跳过，
    /// 门保持原样（卡片回落 `description`）。
    fn attach_coder_triage_summary(
        &self,
        attempt: &CodingExecutionAttempt,
        gate: &CodingGateRequired,
        report: Option<&ExecutionPlanDefectReport>,
        parse_error: Option<&str>,
    ) {
        let Some(reference) = gate.raw_provider_output_ref.as_deref() else {
            return;
        };
        let Ok(Some(full_output)) = self.store.read_provider_raw_output_by_ref(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            reference,
        ) else {
            return;
        };
        let summary = build_coder_triage_summary(report, parse_error, &full_output);
        let _ = self.store.attach_blocked_gate_triage_summary(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &gate.gate_id,
            summary,
        );
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

/// P2-c（A-6 经验值，防 gate JSON 膨胀）：triage_summary 文本截断与条数上限。
const TRIAGE_SUMMARY_TEXT_LIMIT: usize = 500;
const TRIAGE_SUMMARY_MAX_FINDINGS: usize = 5;
const TRIAGE_SUMMARY_MAX_UNVERIFIED: usize = 5;
const TRIAGE_SUMMARY_MAX_CHANGED_FILES: usize = 20;

/// P2-c（REQ-CRO-09）：分诊卡「大白话」投影——固定模板 + 结构化字段，
/// 不引入 LLM。problem/findings/unverified 取自在手的 finding；
/// `did` 对 `extract_plan_defect_payload` 的同一候选对象（最后一个声明
/// `plan_defect_findings` 的顶层 JSON）宽容解析 `delivery_report`，字段
/// 缺失或类型不符一律降级（did=None），绝不报错。
fn build_coder_triage_summary(
    report: Option<&ExecutionPlanDefectReport>,
    parse_error: Option<&str>,
    full_output: &str,
) -> CodingTriageSummary {
    let findings = report
        .map(|report| &report.findings)
        .map(|findings| {
            findings
                .iter()
                .take(TRIAGE_SUMMARY_MAX_FINDINGS)
                .map(|finding| CodingTriageFinding {
                    reason_code: truncate_triage_text(&finding.reason_code),
                    message: truncate_triage_text(&finding.message),
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let problem = report
        .and_then(|report| report.findings.first())
        .map(|finding| truncate_triage_text(&finding.message))
        .unwrap_or_else(|| {
            if parse_error.is_some() {
                "Coder 输出未通过契约校验，需要人工分诊"
            } else {
                "Coder 输出需要人工分诊"
            }
            .to_string()
        });
    let mut unverified: Vec<String> = Vec::new();
    if let Some(report) = report {
        for finding in &report.findings {
            for evidence in &finding.evidence {
                let message = truncate_triage_text(&evidence.message);
                if !message.is_empty() && !unverified.contains(&message) {
                    unverified.push(message);
                }
            }
        }
    }
    unverified.truncate(TRIAGE_SUMMARY_MAX_UNVERIFIED);
    let did = extract_triage_delivery_report(full_output)
        .and_then(|value| serde_json::from_value::<TriageDeliveryReport>(value).ok())
        .map(|delivery| CodingTriageDid {
            changed_files: delivery
                .changed_files
                .into_iter()
                .filter_map(|file| file.path)
                .take(TRIAGE_SUMMARY_MAX_CHANGED_FILES)
                .map(|path| truncate_triage_text(&path))
                .collect(),
            commit_sha: delivery
                .commit
                .and_then(|commit| commit.sha)
                .map(|sha| truncate_triage_text(&sha)),
            verification_exit_code: delivery.verification.and_then(|v| v.exit_code),
            environment_note: delivery
                .environment_notes
                .map(|note| truncate_triage_text(&note)),
        });
    CodingTriageSummary {
        problem,
        findings,
        did,
        unverified,
    }
}

/// 从 coder 原始输出提取 `delivery_report`：与 `extract_plan_defect_payload`
/// 同一候选语义——最后一个声明 `plan_defect_findings` 的顶层平衡对象。
fn extract_triage_delivery_report(full_output: &str) -> Option<serde_json::Value> {
    let mut delivery_report = None;
    for candidate in extract_json_object_candidates(full_output) {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(candidate) else {
            continue;
        };
        if value
            .as_object()
            .is_some_and(|object| object.contains_key("plan_defect_findings"))
        {
            delivery_report = value.get("delivery_report").cloned();
        }
    }
    delivery_report
}

/// `delivery_report` 的宽容投影结构：子字段缺失按 None/空处理，类型不符
/// 整体降级（did=None，F-7）。
#[derive(serde::Deserialize)]
struct TriageDeliveryReport {
    #[serde(default)]
    changed_files: Vec<TriageDeliveryChangedFile>,
    #[serde(default)]
    commit: Option<TriageDeliveryCommit>,
    #[serde(default)]
    verification: Option<TriageDeliveryVerification>,
    #[serde(default)]
    environment_notes: Option<String>,
}

#[derive(serde::Deserialize)]
struct TriageDeliveryChangedFile {
    #[serde(default)]
    path: Option<String>,
}

#[derive(serde::Deserialize)]
struct TriageDeliveryCommit {
    #[serde(default)]
    sha: Option<String>,
}

#[derive(serde::Deserialize)]
struct TriageDeliveryVerification {
    #[serde(default)]
    exit_code: Option<i64>,
}

/// 按字符截断（中文安全，不以字节边界切割）。
fn truncate_triage_text(value: &str) -> String {
    if value.chars().count() <= TRIAGE_SUMMARY_TEXT_LIMIT {
        return value.to_string();
    }
    value.chars().take(TRIAGE_SUMMARY_TEXT_LIMIT).collect()
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

#[cfg(test)]
mod triage_summary_tests {
    use super::*;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::coding_attempt_store::CreateCodingAttemptInput;
    use crate::product::coding_models::CodingTriageSummary;
    use crate::product::plan_repair::{
        PlanDefectClass, PlanDefectConfidence, PlanDefectEvidence, PlanDefectFinding,
        PlanDefectRoute, PlanDefectSeverity,
    };
    use crate::web::workspace_ws_types::ProviderConfigSnapshot;

    /// WI-002 同构 finding（与 P2-b `coding_operational_blocker.rs` 同构）。
    fn wi002_isomorphic_finding() -> PlanDefectFinding {
        PlanDefectFinding {
            finding_id: "PDF-CODER-0001".to_string(),
            severity: PlanDefectSeverity::Error,
            defect_class: PlanDefectClass::OperationalBlocker,
            reason_code: "upstream_deliverable_absent_in_runtime".to_string(),
            message: "CT-001 上游交付物缺失：busi 成员仓（WI-001）在运行环境中完全不存在"
                .to_string(),
            evidence: vec![PlanDefectEvidence {
                kind: "filesystem_observation".to_string(),
                source_ref: "/tmp/notify-center".to_string(),
                message: "聚合根仅含 api/.git 骨架；busi 成员仓不存在".to_string(),
            }],
            contract_refs: vec!["CT-001".to_string()],
            capability_refs: Vec::new(),
            repair_target: None,
            recommended_route: PlanDefectRoute::OperationalGate,
            confidence: PlanDefectConfidence::High,
        }
    }

    fn wi002_report() -> ExecutionPlanDefectReport {
        ExecutionPlanDefectReport {
            source: PlanDefectSource::Coder,
            findings: vec![wi002_isomorphic_finding()],
        }
    }

    fn wi002_finding_json() -> serde_json::Value {
        serde_json::json!({
            "finding_id": "PDF-CODER-0001",
            "severity": "error",
            "defect_class": "operational_blocker",
            "reason_code": "upstream_deliverable_absent_in_runtime",
            "message": "CT-001 上游交付物缺失：busi 成员仓（WI-001）在运行环境中完全不存在",
            "evidence": [{
                "kind": "filesystem_observation",
                "source_ref": "/tmp/notify-center",
                "message": "聚合根仅含 api/.git 骨架；busi 成员仓不存在"
            }],
            "contract_refs": ["CT-001"],
            "capability_refs": [],
            "repair_target": null,
            "recommended_route": "operational_gate",
            "confidence": "high"
        })
    }

    /// WI-002 同构 coder 原始输出：结论对象同时声明 `plan_defect_findings` 与
    /// `delivery_report`（changed_files 4 项、commit.sha、verification.exit_code、
    /// environment_notes——现场 raw 实测形状）。
    fn wi002_raw_with_delivery_report() -> String {
        serde_json::json!({
            "plan_defect_findings": [wi002_finding_json()],
            "delivery_report": {
                "changed_files": [
                    {"path": "api/src/routes.ts", "change": "modified", "note": "挂载 busi 代理路由"},
                    {"path": "api/src/busi-proxy.ts", "change": "added", "note": "新增代理实现"},
                    {"path": "api/package.json", "change": "modified", "note": "登记依赖"},
                    {"path": "api/README.md", "change": "modified", "note": "补充说明"}
                ],
                "commit": {"sha": "9f31c2ab7e5d", "branch": "aria/work-items/WI-002/attempt-1"},
                "verification": {"command": "npm test", "exit_code": 0, "output": "ok"},
                "git_status": "clean",
                "environment_notes": "busi 成员仓（WI-001）在运行环境中不存在，聚合根仅含 api/.git 骨架"
            }
        })
        .to_string()
    }

    fn wi002_raw_without_delivery_report() -> String {
        serde_json::json!({
            "plan_defect_findings": [wi002_finding_json()],
        })
        .to_string()
    }

    fn triage_summary_fixture() -> (
        tempfile::TempDir,
        CodingAttemptStore,
        CodingExecutionAttempt,
    ) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = CodingAttemptStore::new(ProductAppPaths::new(tmp.path().join(".aria")));
        let attempt = store
            .create_attempt(CreateCodingAttemptInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                work_item_id: "work_item_0001".to_string(),
                base_branch: "HEAD".to_string(),
                branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
                worktree_path: None,
                provider_config_snapshot: ProviderConfigSnapshot {
                    author: ProviderName::Fake,
                    reviewer: Some(ProviderName::Fake),
                    review_rounds: 1,
                    permission_modes: Default::default(),
                },
                target_snapshot: None,
                max_auto_rework: 1,
            })
            .expect("create attempt");
        let attempt = store
            .seed_running_attempt_for_test(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("running attempt");
        (tmp, store, attempt)
    }

    /// 红测①（REQ-CRO-09）：WI-002 同构 raw → problem 为首条 finding.message、
    /// did.changed_files 恰 4 项、unverified 取自 evidence.message。
    #[test]
    fn build_coder_triage_summary_extracts_wi002_shaped_raw_output() {
        let report = wi002_report();
        let summary =
            build_coder_triage_summary(Some(&report), None, &wi002_raw_with_delivery_report());
        let finding = &report.findings[0];
        assert_eq!(summary.problem, finding.message);
        assert_eq!(summary.findings.len(), 1);
        assert_eq!(summary.findings[0].reason_code, finding.reason_code);
        assert_eq!(summary.findings[0].message, finding.message);
        let did = summary
            .did
            .as_ref()
            .expect("delivery_report 可解析时 did 必须存在");
        assert_eq!(
            did.changed_files,
            vec![
                "api/src/routes.ts",
                "api/src/busi-proxy.ts",
                "api/package.json",
                "api/README.md",
            ]
        );
        assert_eq!(did.commit_sha.as_deref(), Some("9f31c2ab7e5d"));
        assert_eq!(did.verification_exit_code, Some(0));
        assert_eq!(
            did.environment_note.as_deref(),
            Some("busi 成员仓（WI-001）在运行环境中不存在，聚合根仅含 api/.git 骨架")
        );
        assert_eq!(
            summary.unverified,
            vec!["聚合根仅含 api/.git 骨架；busi 成员仓不存在"]
        );
    }

    /// 红测②（单元面）：`delivery_report` 缺失 / 字段类型错 → did=None，
    /// 其余段照常给出，构造绝不报错。
    #[test]
    fn build_coder_triage_summary_degrades_to_did_none_without_or_malformed_delivery_report() {
        let report = wi002_report();
        let summary =
            build_coder_triage_summary(Some(&report), None, &wi002_raw_without_delivery_report());
        assert!(
            summary.did.is_none(),
            "delivery_report 缺失时 did 必须为 None"
        );
        assert_eq!(summary.problem, report.findings[0].message);

        let raw_malformed = serde_json::json!({
            "plan_defect_findings": [wi002_finding_json()],
            "delivery_report": {"changed_files": "api/src/routes.ts"},
        })
        .to_string();
        let summary = build_coder_triage_summary(Some(&report), None, &raw_malformed);
        assert!(
            summary.did.is_none(),
            "delivery_report 字段类型错时 did 必须为 None"
        );
    }

    /// 解析失败（report=None、有 parse_error）→ problem 为固定中文说明，
    /// findings/unverified 为空、did=None。
    #[test]
    fn build_coder_triage_summary_uses_fixed_problem_text_when_parse_failed() {
        let summary = build_coder_triage_summary(
            None,
            Some("plan_defect_output_invalid: boom"),
            "coder 输出中没有任何 JSON 对象",
        );
        assert_eq!(summary.problem, "Coder 输出未通过契约校验，需要人工分诊");
        assert!(summary.findings.is_empty());
        assert!(summary.did.is_none());
        assert!(summary.unverified.is_empty());
    }

    /// 大小上限（A-6）：字符串 500 字符、findings/unverified 各 5 条、
    /// changed_files 20 项。
    #[test]
    fn build_coder_triage_summary_applies_size_caps() {
        let findings = (0..7)
            .map(|index| PlanDefectFinding {
                finding_id: format!("PDF-CODER-{index:04}"),
                severity: PlanDefectSeverity::Error,
                defect_class: PlanDefectClass::OperationalBlocker,
                reason_code: format!("reason_code_{index}"),
                message: format!("{index}号finding：{}", "很长的说明".repeat(120)),
                evidence: vec![PlanDefectEvidence {
                    kind: "filesystem_observation".to_string(),
                    source_ref: format!("/tmp/obs-{index}"),
                    message: format!("未验证事项 {index}"),
                }],
                contract_refs: Vec::new(),
                capability_refs: Vec::new(),
                repair_target: None,
                recommended_route: PlanDefectRoute::OperationalGate,
                confidence: PlanDefectConfidence::High,
            })
            .collect::<Vec<_>>();
        let report = ExecutionPlanDefectReport {
            source: PlanDefectSource::Coder,
            findings,
        };
        let raw = serde_json::json!({
            "plan_defect_findings": [wi002_finding_json()],
            "delivery_report": {
                "changed_files": (0..25)
                    .map(|index| serde_json::json!({"path": format!("src/file_{index:02}.rs")}))
                    .collect::<Vec<_>>(),
            },
        })
        .to_string();
        let summary = build_coder_triage_summary(Some(&report), None, &raw);
        assert_eq!(summary.findings.len(), 5);
        assert_eq!(summary.unverified.len(), 5);
        assert!(summary.problem.chars().count() <= 500);
        let did = summary.did.expect("did");
        assert_eq!(did.changed_files.len(), 20);
        assert_eq!(did.changed_files[0], "src/file_00.rs");
    }

    /// 红测②（行为面，C1 裁定）：delivery_report 缺失 → did=None 且门照常
    /// 创建；摘要经 coding_attempt_store 独立入口落盘，不经过
    /// CreateBlockedGateInput。
    #[tokio::test]
    async fn human_triage_gate_created_with_did_none_summary_when_delivery_report_missing() {
        let (root, store, attempt) = triage_summary_fixture();
        let reference = store
            .save_provider_raw_output(
                &attempt,
                CodingExecutionStage::Coding,
                "coder_output",
                &wi002_raw_without_delivery_report(),
            )
            .expect("save raw");
        let (tx, _rx) = mpsc::channel(64);
        let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
        let report = wi002_report();
        engine
            .open_coding_output_human_triage_gate(
                &attempt,
                "node_coding_0001",
                Some(&report),
                None,
                Some(reference),
            )
            .await
            .expect("delivery_report 缺失绝不阻塞开门");

        let gates = store
            .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("open gates");
        assert_eq!(gates.len(), 1, "门必须照常创建且只有一张");
        let gate = &gates[0];
        assert_eq!(
            gate.reason_code.as_deref(),
            Some("coding_output_human_triage")
        );
        let summary = gate
            .triage_summary
            .as_ref()
            .expect("摘要应经 store 独立入口落盘");
        assert!(summary.did.is_none());
        assert_eq!(summary.problem, report.findings[0].message);
        assert_eq!(
            summary.unverified,
            vec!["聚合根仅含 api/.git 骨架；busi 成员仓不存在"]
        );
        drop(root);
    }

    /// operational 门同卡同摘要：WI-002 同构 raw（含 delivery_report）→
    /// did.changed_files 4 项随门落盘。
    #[tokio::test]
    async fn operational_blocker_gate_carries_triage_summary_with_did() {
        let (root, store, attempt) = triage_summary_fixture();
        let reference = store
            .save_provider_raw_output(
                &attempt,
                CodingExecutionStage::Coding,
                "coder_output",
                &wi002_raw_with_delivery_report(),
            )
            .expect("save raw");
        let (tx, _rx) = mpsc::channel(64);
        let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
        let report = wi002_report();
        engine
            .open_coding_operational_blocker_gate(
                &attempt,
                "node_coding_0001",
                Some(&report),
                Some(reference),
            )
            .await
            .expect("open operational blocker gate");

        let gates = store
            .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("open gates");
        assert_eq!(gates.len(), 1);
        let gate = &gates[0];
        assert_eq!(
            gate.reason_code.as_deref(),
            Some("coding_operational_blocker")
        );
        let did = gate
            .triage_summary
            .as_ref()
            .and_then(|summary| summary.did.as_ref())
            .expect("operational 门同样携带 did 摘要");
        assert_eq!(did.changed_files.len(), 4);
        drop(root);
    }

    /// F-7 极端面：raw 引用指向的文件不存在 → 门照常创建、无摘要（摘要
    /// 任何失败都不影响开门主流程）。
    #[tokio::test]
    async fn human_triage_gate_creation_survives_missing_raw_output() {
        let (root, store, attempt) = triage_summary_fixture();
        let (tx, _rx) = mpsc::channel(64);
        let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
        engine
            .open_coding_output_human_triage_gate(
                &attempt,
                "node_coding_0001",
                None,
                Some("plan_defect_output_invalid: boom"),
                Some("provider-raw/coding/coder_output_0009.txt".to_string()),
            )
            .await
            .expect("raw 缺失绝不阻塞开门");

        let gates = store
            .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("open gates");
        assert_eq!(gates.len(), 1, "门必须照常创建");
        assert!(
            gates[0].triage_summary.is_none(),
            "读不到 raw 时无摘要，卡片回落 description"
        );
        drop(root);
    }

    /// 旧 gate JSON（无 triage_summary 字段）反序列化为 None——现场 WI-002
    /// 老门兼容（serde default）。
    #[test]
    fn legacy_gate_json_deserializes_with_triage_summary_none() {
        let legacy = serde_json::json!({
            "gate_id": "coding_blocked_gate_0001",
            "kind": "blocked",
            "title": "Coder 输出需要人工分诊",
            "description": "Coder 报告的 plan defect 需要人工分诊（1 条 finding），流程已暂停",
            "stage": "coding",
            "role": "coder",
            "expires_at": null,
            "provider_snapshot": null,
            "available_actions": [
                {"action_id": "retry_coding", "label": "重新启动 Coder", "action_type": "retry_coding"},
                {"action_id": "abort", "label": "终止", "action_type": "abort"}
            ],
            "reason_code": "coding_output_human_triage",
            "evidence_refs": [],
            "raw_provider_output_ref": "provider-raw/coding/coder_output_0001.txt"
        });
        let gate: CodingGateRequired = serde_json::from_value(legacy).expect("legacy gate json");
        assert_eq!(gate.gate_id, "coding_blocked_gate_0001");
        assert!(gate.triage_summary.is_none());

        // 序列化对称：None 时不写字段（旧读者不受影响）。
        let value = serde_json::to_value(&gate).expect("serialize gate");
        assert!(value.get("triage_summary").is_none());

        // 有摘要时随门 JSON 落盘并可读回。
        let mut with_summary = gate.clone();
        with_summary.triage_summary = Some(CodingTriageSummary {
            problem: "CT-001 上游交付物缺失".to_string(),
            findings: Vec::new(),
            did: None,
            unverified: vec!["聚合根仅含 api/.git 骨架".to_string()],
        });
        let value = serde_json::to_value(&with_summary).expect("serialize gate");
        assert_eq!(
            value["triage_summary"]["unverified"][0],
            "聚合根仅含 api/.git 骨架"
        );
        let round_trip: CodingGateRequired = serde_json::from_value(value).expect("round trip");
        assert_eq!(round_trip, with_summary);
    }
}
