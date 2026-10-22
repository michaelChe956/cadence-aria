use super::*;
use crate::product::coding_attempt_store::IssueDeliveryOverall;
use crate::product::coding_models::{
    CodingAttemptScope, CodingStageGateStatus, GroupFinalReadinessStatus,
    IssueCrossRepoRiskAcknowledgement,
};
use crate::product::id::next_sequential_id_from_existing;
use crate::product::issue_store::IssueStore;
use crate::product::json_store::ProductStoreError;
use crate::product::models::IssueStatus;

impl CodingWorkspaceEngine {
    pub(crate) fn active_work_item_id_for_attempt<'a>(
        &self,
        attempt: &'a CodingExecutionAttempt,
    ) -> &'a str {
        attempt
            .current_work_item_id
            .as_deref()
            .unwrap_or(&attempt.work_item_id)
    }

    pub async fn handle_final_confirm(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        let current = self.store.get_attempt(project_id, issue_id, attempt_id)?;
        if current.status != CodingAttemptStatus::WaitingForHuman
            || current.stage != CodingExecutionStage::FinalConfirm
        {
            return Err(CodingWorkspaceEngineError::FinalConfirmNotReady(
                attempt_id.to_string(),
            ));
        }
        if current.scope == CodingAttemptScope::WorkItemGroup
            && !self.group_attempt_ready_for_final_review(&current)?
        {
            return Err(CodingWorkspaceEngineError::FinalConfirmNotReady(
                attempt_id.to_string(),
            ));
        }
        if current.scope == CodingAttemptScope::WorkItemGroup {
            // Snapshot reads validate the stored attempt identity. A mismatch is intentionally
            // collapsed to the public FinalConfirmNotReady diagnostic instead of leaking a
            // malformed record through the socket protocol.
            let readiness = match self.store.get_group_final_readiness_snapshot(&current) {
                Ok(Some(snapshot)) => snapshot,
                Ok(None) => {
                    return Err(CodingWorkspaceEngineError::FinalConfirmNotReady(
                        attempt_id.to_string(),
                    ));
                }
                Err(ProductStoreError::IdentityMismatch {
                    kind: "group_final_readiness_snapshot",
                    ..
                })
                | Err(ProductStoreError::InvalidRecord {
                    kind: "group_final_readiness_snapshot",
                    ..
                }) => {
                    return Err(CodingWorkspaceEngineError::FinalConfirmNotReady(
                        attempt_id.to_string(),
                    ));
                }
                Err(error) => return Err(error.into()),
            };
            if readiness.attempt_id != current.id
                || readiness.status != GroupFinalReadinessStatus::Complete
                || !readiness.diagnostics.is_empty()
            {
                return Err(CodingWorkspaceEngineError::FinalConfirmNotReady(
                    attempt_id.to_string(),
                ));
            }
        }
        match current.scope {
            CodingAttemptScope::WorkItemGroup => {
                self.validate_attempt_issue_shared_worktree_owner_if_present(&current)?;
            }
            CodingAttemptScope::WorkItem => {
                self.validate_attempt_issue_shared_worktree_lock_if_present(&current)?;
            }
        }
        match current.scope {
            CodingAttemptScope::WorkItemGroup => {
                self.run_group_completion_gates(&current).await?;
            }
            CodingAttemptScope::WorkItem => {
                self.record_work_item_completion_commit_if_present(&current)?;
                self.run_completion_gates(&current).await?;
            }
        }

        let updated = self.store.update_attempt_status(
            project_id,
            issue_id,
            attempt_id,
            CodingAttemptStatus::Completed,
        )?;
        if updated.scope == CodingAttemptScope::WorkItemGroup {
            self.mark_completed_group_work_items_if_present(&updated)?;
            self.release_issue_shared_worktree_lock_for_attempt(project_id, issue_id, &updated.id)?;
        } else {
            let current_work_item_id = self.active_work_item_id_for_attempt(&updated);
            LifecycleStore::new(self.store.paths()).update_work_item_execution_status(
                &updated.project_id,
                &updated.issue_id,
                current_work_item_id,
                WorkItemStatus::Completed,
            )?;
            self.mark_issue_shared_worktree_completed_if_present(
                project_id,
                issue_id,
                current_work_item_id,
                &updated.id,
            )?;
        }

        // REQ-GFR-CR-06（已知风险确认留痕，advisory 旁路）：跨仓评审结论为
        // request_changes/blocked 而用户仍然确认 → 追加一条「已知风险，仍然
        // 确认」留痕。写失败仅告警，不得让已成功的确认回退失败
        //（REQ-GFR-CR-05：确认必须成功）。
        if let Err(error) = self.record_cross_repo_risk_acknowledgement_if_needed(&updated) {
            tracing::warn!(
                project_id = %updated.project_id,
                issue_id = %updated.issue_id,
                attempt_id = %updated.id,
                error = %error,
                "跨仓评审已知风险留痕写入失败（advisory，不影响已完成的确认）"
            );
        }
        if let Some(node_id) =
            self.active_final_confirm_node_id(project_id, issue_id, attempt_id)?
        {
            let completed_at = Utc::now().to_rfc3339();
            self.store.update_timeline_node_status(
                project_id,
                issue_id,
                attempt_id,
                &node_id,
                CodingTimelineNodeStatus::Completed,
                Some("用户已确认完成".to_string()),
                Some(completed_at.clone()),
            )?;
            let _ = self
                .event_tx
                .send(CodingWsOutMessage::CodingTimelineNodeUpdated {
                    node_id,
                    status: CodingTimelineNodeStatus::Completed,
                    summary: Some("用户已确认完成".to_string()),
                    completed_at: Some(completed_at),
                })
                .await;
        }
        Ok(updated)
    }

    /// REQ-GFR-CR-06：「已知风险，仍然确认」留痕（advisory 旁路）。
    ///
    /// 该 issue 存在跨仓最终评审且结论为 `request_changes`/`blocked`、且尚无
    /// 同一 `(cross_repo_review_id, attempt_id)` 的留痕时追加一条。内容只有
    /// 「确认时间 + 评审记录 id + 结论 + findings 摘要」——不记确认人、不记
    /// 通道（owner 裁定）。结论为通过（含 Failed/Skipped 记录，verdict 恒为
    /// approve）不留痕。同一 `(cross_repo_review_id, attempt_id)` 至多一条：
    /// 引擎前置判定 + store 幂等双保险。
    fn record_cross_repo_risk_acknowledgement_if_needed(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<(), ProductStoreError> {
        let Some(review) = self
            .store
            .load_issue_cross_repo_review(&attempt.project_id, &attempt.issue_id)?
        else {
            return Ok(());
        };
        if !matches!(
            review.verdict,
            ReviewVerdict::RequestChanges | ReviewVerdict::Blocked
        ) {
            return Ok(());
        }
        if self.store.has_issue_cross_repo_acknowledgement(
            &attempt.project_id,
            &attempt.issue_id,
            &review.id,
            &attempt.id,
        )? {
            return Ok(());
        }
        let existing = self
            .store
            .list_issue_cross_repo_acknowledgements(&attempt.project_id, &attempt.issue_id)?;
        let acknowledgement = IssueCrossRepoRiskAcknowledgement {
            id: next_sequential_id_from_existing(
                "cross_repo_review_ack",
                existing.iter().map(|record| record.id.as_str()),
            ),
            project_id: attempt.project_id.clone(),
            issue_id: attempt.issue_id.clone(),
            plan_id: review.plan_id.clone(),
            attempt_id: attempt.id.clone(),
            cross_repo_review_id: review.id.clone(),
            verdict: review.verdict.clone(),
            findings_summary: cross_repo_ack_findings_summary(&review.findings),
            acknowledged_at: Utc::now().to_rfc3339(),
        };
        self.store
            .append_issue_cross_repo_acknowledgement(&acknowledgement)?;
        Ok(())
    }

    pub async fn handle_abort(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        let current = self.store.get_attempt(project_id, issue_id, attempt_id)?;
        let current = self
            .reconcile_coding_git_operation_for_termination(&current)
            .await?;
        if current.status == CodingAttemptStatus::Aborted {
            return Ok(current);
        }
        let active_work_item_id = match current.scope {
            CodingAttemptScope::WorkItemGroup => {
                self.validate_attempt_issue_shared_worktree_owner_if_present(&current)?;
                let lifecycle = LifecycleStore::new(self.store.paths());
                match self.route_issue_shared_worktree(&current)? {
                    IssueSharedWorktreeRoute::Legacy => lifecycle
                        .get_issue_shared_worktree(project_id, issue_id)?
                        .and_then(|shared| shared.current_active_work_item_id)
                        .unwrap_or_else(|| {
                            self.active_work_item_id_for_attempt(&current).to_string()
                        }),
                    IssueSharedWorktreeRoute::Repository { repository_id } => lifecycle
                        .get_repo_shared_worktree(project_id, issue_id, repository_id)?
                        .and_then(|shared| shared.current_active_work_item_id)
                        .unwrap_or_else(|| {
                            self.active_work_item_id_for_attempt(&current).to_string()
                        }),
                }
            }
            CodingAttemptScope::WorkItem => {
                self.validate_attempt_issue_shared_worktree_lock_if_present(&current)?;
                self.active_work_item_id_for_attempt(&current).to_string()
            }
        };
        self.ensure_issue_shared_worktree_clean(&current, &active_work_item_id)
            .await?;

        for gate in self
            .store
            .list_open_stage_gates(project_id, issue_id, attempt_id)?
        {
            self.store.update_stage_gate_status(
                project_id,
                issue_id,
                attempt_id,
                &gate.gate_id,
                CodingStageGateStatus::Cancelled,
            )?;
        }

        let updated = self.store.update_attempt_status(
            project_id,
            issue_id,
            attempt_id,
            CodingAttemptStatus::Aborted,
        )?;
        match updated.scope {
            CodingAttemptScope::WorkItemGroup => {
                self.release_issue_shared_worktree_lock_for_attempt(
                    project_id, issue_id, attempt_id,
                )?;
            }
            CodingAttemptScope::WorkItem => {
                self.release_issue_shared_worktree_lock_if_holder(
                    project_id,
                    issue_id,
                    &active_work_item_id,
                    attempt_id,
                )?;
            }
        }
        if let Some(node_id) = self.active_timeline_node_id(project_id, issue_id, attempt_id)? {
            let completed_at = Utc::now().to_rfc3339();
            self.store.update_timeline_node_status(
                project_id,
                issue_id,
                attempt_id,
                &node_id,
                CodingTimelineNodeStatus::Failed,
                Some("用户已中止".to_string()),
                Some(completed_at.clone()),
            )?;
            let _ = self
                .event_tx
                .send(CodingWsOutMessage::CodingTimelineNodeUpdated {
                    node_id,
                    status: CodingTimelineNodeStatus::Failed,
                    summary: Some("用户已中止".to_string()),
                    completed_at: Some(completed_at),
                })
                .await;
        }
        Ok(updated)
    }

    /// 批 2 T2（REQ-WIGA-09，C2/C5，§4 步骤⑤）：plan 终止的专用收口——
    /// 终止编排（`terminate_plan_engine_side`）在 runner 全停后逐 attempt
    /// 调用（幂等；单仓异常由调用方继续后仓）。与手动 `handle_abort` 的
    /// 差异：跳过 worktree clean（脏文件保留，脏树不作「不能停止」的
    /// 理由）、不 reconcile git 操作、不伪造答案；Completed/Failed/Aborted
    /// 终态保持，其余非终态置 Aborted；关闭全部 open blocked/choice/stage
    /// 门；共享 worktree 锁尽力释放、时间线节点尽力收口（失败不阻断终止）。
    pub async fn finalize_attempt_for_plan_termination(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        let current = self.store.get_attempt(project_id, issue_id, attempt_id)?;
        self.store
            .close_all_open_gates_for_termination(project_id, issue_id, attempt_id)?;
        let updated = match current.status {
            CodingAttemptStatus::Completed
            | CodingAttemptStatus::Failed
            | CodingAttemptStatus::Aborted => current,
            _ => self.store.update_attempt_status(
                project_id,
                issue_id,
                attempt_id,
                CodingAttemptStatus::Aborted,
            )?,
        };
        // T14(终止入口 1 行接线,共享集成人角色):回收 attempt 的全部
        // code_delivery 登记资源——先静止证明再清理 run-owned 目录;
        // 未知 writer → Err 停等(终止结果可见 cleanup_failed,可重入)。
        self.recover_coder_delivery_scopes_for_attempt(&updated)
            .await?;
        // T21:attempt 终止→reviewer 窗口与租约收口(先恢复 baseline 保
        // Coder dirty 再弃置关闭;恢复失败 fail-closed 保持锁,人工恢复
        // 续接/重试终止收敛;取消补偿随释放续接)。
        self.close_reviewer_windows_for_attempt(
            &updated,
            super::code_review::AttemptWindowCloseMode::Terminate,
        )
        .await?;
        match updated.scope {
            CodingAttemptScope::WorkItemGroup => {
                if let Err(error) = self.release_issue_shared_worktree_lock_for_attempt(
                    project_id, issue_id, attempt_id,
                ) {
                    tracing::warn!(
                        attempt_id,
                        error = %error,
                        "termination finalization: shared worktree lock release failed"
                    );
                }
            }
            CodingAttemptScope::WorkItem => {
                let active_work_item_id =
                    self.active_work_item_id_for_attempt(&updated).to_string();
                if let Err(error) = self.release_issue_shared_worktree_lock_if_holder(
                    project_id,
                    issue_id,
                    &active_work_item_id,
                    attempt_id,
                ) {
                    tracing::warn!(
                        attempt_id,
                        error = %error,
                        "termination finalization: shared worktree lock release failed"
                    );
                }
            }
        }
        if let Ok(Some(node_id)) = self.active_timeline_node_id(project_id, issue_id, attempt_id) {
            let completed_at = Utc::now().to_rfc3339();
            let _ = self.store.update_timeline_node_status(
                project_id,
                issue_id,
                attempt_id,
                &node_id,
                CodingTimelineNodeStatus::Failed,
                Some("计划已终止".to_string()),
                Some(completed_at.clone()),
            );
            let _ = self
                .event_tx
                .send(CodingWsOutMessage::CodingTimelineNodeUpdated {
                    node_id,
                    status: CodingTimelineNodeStatus::Failed,
                    summary: Some("计划已终止".to_string()),
                    completed_at: Some(completed_at),
                })
                .await;
        }
        Ok(updated)
    }

    pub async fn handle_attempt_failed(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        let current = self.store.get_attempt(project_id, issue_id, attempt_id)?;
        let scope = current.scope.clone();
        let attempt_work_item_id = self.active_work_item_id_for_attempt(&current).to_string();
        match scope {
            CodingAttemptScope::WorkItemGroup => {
                self.validate_attempt_issue_shared_worktree_owner_if_present(&current)?;
            }
            CodingAttemptScope::WorkItem => {
                self.validate_attempt_issue_shared_worktree_lock_if_present(&current)?;
            }
        }
        let updated = if current.status != CodingAttemptStatus::Failed {
            self.store.update_attempt_status(
                project_id,
                issue_id,
                attempt_id,
                CodingAttemptStatus::Failed,
            )?
        } else {
            current
        };

        let lifecycle = LifecycleStore::new(self.store.paths());
        let active_work_item_id = match scope {
            CodingAttemptScope::WorkItemGroup => lifecycle
                .get_issue_shared_worktree(project_id, issue_id)?
                .and_then(|shared| shared.current_active_work_item_id)
                .unwrap_or(attempt_work_item_id),
            CodingAttemptScope::WorkItem => attempt_work_item_id,
        };
        if let Err(error) = self
            .ensure_issue_shared_worktree_clean(&updated, &active_work_item_id)
            .await
        {
            tracing::warn!(
                attempt_id = %updated.id,
                error = %error,
                "failed attempt shared worktree cleanup skipped"
            );
            return Ok(updated);
        }
        let release_result = match scope {
            CodingAttemptScope::WorkItemGroup => self
                .release_issue_shared_worktree_lock_for_attempt(project_id, issue_id, &updated.id),
            CodingAttemptScope::WorkItem => self.release_issue_shared_worktree_lock_if_holder(
                project_id,
                issue_id,
                &active_work_item_id,
                &updated.id,
            ),
        };
        if let Err(error) = release_result {
            tracing::warn!(
                attempt_id = %updated.id,
                error = %error,
                "failed attempt shared worktree lock release failed"
            );
        }
        Ok(updated)
    }

    pub async fn handle_delete_attempt(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<(), CodingWorkspaceEngineError> {
        let current = self.store.get_attempt(project_id, issue_id, attempt_id)?;
        let active_work_item_id = self.active_work_item_id_for_attempt(&current).to_string();
        match current.scope {
            CodingAttemptScope::WorkItemGroup => {
                let lifecycle = LifecycleStore::new(self.store.paths());
                // 缺陷 #13 层2：Logical 路由 group create（分流修复前）误写的
                // 自持 legacy 布局会让 delete 链的每次 route 分流都
                // fail-closed（legacy_shared_worktree_present），删除被拦死
                //（与恢复/重启同为死锁出口）。删除是错误布局的产品面清理
                // 出口：矛盾形态（target_snapshot 存在 + issue 维 legacy record
                // 自持，owner == 本 attempt）下三步（owner 校验 / active item
                // 读取 / clean 检查）整体跳过——归属已由自持判别证明，
                // worktree 的 git 清理由删除链 cleanup_coding_attempt_workspace
                // 负责。Legacy 路由正常形态与他人/无主残留均不受影响
                //（迁移契约 §4.2.6 语义不变）。
                if !Self::delete_tolerates_self_owned_legacy_layout(&lifecycle, &current) {
                    self.validate_attempt_issue_shared_worktree_owner_if_present(&current)?;
                    let shared_active_work_item_id =
                        match self.route_issue_shared_worktree(&current)? {
                            IssueSharedWorktreeRoute::Legacy => lifecycle
                                .get_issue_shared_worktree(project_id, issue_id)?
                                .and_then(|shared| shared.current_active_work_item_id),
                            IssueSharedWorktreeRoute::Repository { repository_id } => lifecycle
                                .get_repo_shared_worktree(project_id, issue_id, repository_id)?
                                .and_then(|shared| shared.current_active_work_item_id),
                        }
                        .unwrap_or_else(|| active_work_item_id.clone());
                    self.ensure_issue_shared_worktree_clean(&current, &shared_active_work_item_id)
                        .await?;
                }
            }
            CodingAttemptScope::WorkItem => {
                self.validate_attempt_issue_shared_worktree_lock_if_present(&current)?;
            }
        }
        if current.status.is_active() {
            self.store.update_attempt_status(
                project_id,
                issue_id,
                attempt_id,
                CodingAttemptStatus::Aborted,
            )?;
        }
        match current.scope {
            CodingAttemptScope::WorkItemGroup => {
                // 同一容忍面（缺陷 #13 层2）：自持矛盾布局下 release 内部的
                // route 分流同样会撞 preflight——按 owner 直接释放 issue 维
                // legacy 锁（正是该布局的锁所在），不绕路 route。
                let lifecycle = LifecycleStore::new(self.store.paths());
                if Self::delete_tolerates_self_owned_legacy_layout(&lifecycle, &current) {
                    lifecycle
                        .release_issue_worktree_lock_by_owner(project_id, issue_id, attempt_id)?;
                } else {
                    self.release_issue_shared_worktree_lock_for_attempt(
                        project_id, issue_id, attempt_id,
                    )?;
                }
            }
            CodingAttemptScope::WorkItem => {
                self.release_issue_shared_worktree_lock_if_holder(
                    project_id,
                    issue_id,
                    &active_work_item_id,
                    attempt_id,
                )?;
            }
        }
        Ok(())
    }

    /// 删除链的「自持矛盾布局」判别：`target_snapshot` 存在（Logical 路由）且
    /// issue 维 legacy record 的锁 owner 正是本 attempt。仅此形态（缺陷 #13 层2
    /// 分流修复前 group create 误写）在删除时整体跳过 owner 校验 / active item
    /// 读取 / clean 检查；Legacy 路由正常形态（snapshot 缺席）与他人/无主残留
    /// 一律返回 false 走原校验链。
    fn delete_tolerates_self_owned_legacy_layout(
        lifecycle: &LifecycleStore,
        attempt: &CodingExecutionAttempt,
    ) -> bool {
        attempt.target_snapshot.is_some()
            && lifecycle
                .get_issue_shared_worktree(&attempt.project_id, &attempt.issue_id)
                .ok()
                .flatten()
                .and_then(|shared| shared.current_lock_owner_id)
                .is_some_and(|owner| owner == attempt.id)
    }

    /// 记录 work item 的完成 commit。
    ///
    /// 交接摘要移除后，`completion_commit` 仍需写入：它由依赖交接引用展示消费，
    /// 与被移除的摘要无关。
    pub(crate) fn record_work_item_completion_commit_if_present(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<(), CodingWorkspaceEngineError> {
        let lifecycle = LifecycleStore::new(self.store.paths());
        let current_work_item_id = self.active_work_item_id_for_attempt(attempt);
        if lifecycle
            .list_work_items(&attempt.project_id, &attempt.issue_id)?
            .iter()
            .any(|item| item.id == current_work_item_id)
        {
            lifecycle.update_work_item_completion_commit(
                &attempt.project_id,
                &attempt.issue_id,
                current_work_item_id,
                attempt.head_commit.clone(),
            )?;
        }
        Ok(())
    }

    pub(crate) async fn complete_attempt_after_final_rework(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        self.validate_attempt_issue_shared_worktree_lock_if_present(attempt)?;
        self.record_work_item_completion_commit_if_present(attempt)?;
        self.run_completion_gates(attempt).await?;
        let staged = self.store.update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::FinalConfirm,
        )?;
        let completed = self.store.update_attempt_status(
            &staged.project_id,
            &staged.issue_id,
            &staged.id,
            CodingAttemptStatus::Completed,
        )?;
        self.mark_work_item_completed_if_present(&completed)?;
        self.mark_issue_shared_worktree_completed_if_present(
            &attempt.project_id,
            &attempt.issue_id,
            self.active_work_item_id_for_attempt(attempt),
            &attempt.id,
        )?;
        let node = self.create_completed_final_confirm_timeline_node(&completed)?;
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingTimelineNodeCreated { node })
            .await;
        Ok(completed)
    }

    pub(crate) async fn complete_attempt_after_review_request(
        &self,
        attempt: &CodingExecutionAttempt,
        review_provider: &IssueCrossRepoReviewProviderResolver<'_>,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        self.validate_attempt_issue_shared_worktree_lock_if_present(attempt)?;
        self.record_work_item_completion_commit_if_present(attempt)?;
        self.run_completion_gates(attempt).await?;
        let completed = self.store.update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Completed,
        )?;
        self.mark_work_item_completed_if_present(&completed)?;
        self.mark_issue_shared_worktree_completed_if_present(
            &completed.project_id,
            &completed.issue_id,
            self.active_work_item_id_for_attempt(&completed),
            &completed.id,
        )?;
        self.maybe_complete_issue_delivery(&completed, review_provider)
            .await?;
        Ok(completed)
    }

    /// 落盘判定接线：issue 下所有 Work Item 均已推送（`AllPushed`）时把 issue 标记
    /// 为 `Completed`；否则不动。单仓/多仓共用，Legacy 单仓走同一路径（纯增量追加
    /// 一次 Completed 落盘）。幂等由 `IssueStore::update_status` 保证。
    ///
    /// REQ-GFR-CR-03（跨仓最终评审触发接线）：收口尾部触发 issue 级跨仓最终
    /// 评审——**旁路**，不参与交付判定、不改 issue `Completed` 判定；触发只
    /// 登记并异步执行（REQ-GFR-CR-05「MUST NOT 阻断」），本调用立即返回。
    pub(crate) async fn maybe_complete_issue_delivery(
        &self,
        attempt: &CodingExecutionAttempt,
        review_provider: &IssueCrossRepoReviewProviderResolver<'_>,
    ) -> Result<(), CodingWorkspaceEngineError> {
        let summary = self
            .store
            .compute_issue_delivery_summary(&attempt.project_id, &attempt.issue_id)?;
        if summary.overall == IssueDeliveryOverall::AllPushed {
            IssueStore::new(self.store.paths()).update_status(
                &attempt.project_id,
                &attempt.issue_id,
                IssueStatus::Completed,
            )?;
        }
        self.dispatch_issue_cross_repo_final_review_trigger(attempt, review_provider)
            .await;
        Ok(())
    }

    /// 将已完成独立审查的 group attempt 交给人工最终确认。
    ///
    /// Readiness 快照仅汇总并持久化权威证据，不会创建 finding、返工或 Plan Repair。
    /// 即使快照不完整，也保留在人工确认界面以展示诊断；`handle_final_confirm`
    /// 会拒绝不完整、缺失或身份不匹配的快照。
    pub(crate) async fn prepare_group_final_confirm_from_readiness(
        &self,
        attempt: &CodingExecutionAttempt,
        review_provider: &IssueCrossRepoReviewProviderResolver<'_>,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        if attempt.scope != CodingAttemptScope::WorkItemGroup
            || !self.group_attempt_ready_for_final_review(attempt)?
        {
            return Err(CodingWorkspaceEngineError::FinalConfirmNotReady(
                attempt.id.clone(),
            ));
        }

        self.build_group_final_readiness_snapshot(attempt).await?;
        let staged = self.store.update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::FinalConfirm,
        )?;
        let updated = self.store.update_attempt_status(
            &staged.project_id,
            &staged.issue_id,
            &staged.id,
            CodingAttemptStatus::WaitingForHuman,
        )?;
        let node = self.create_pending_final_confirm_timeline_node(&updated)?;
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingTimelineNodeCreated { node })
            .await;
        // REQ-GFR-CR-03（跨仓最终评审触发接线）：group 形态的收口尾部触发
        // issue 级跨仓最终评审——**旁路**，不参与交付判定、不阻断人工确认；
        // 触发只登记并异步执行（REQ-GFR-CR-05），本调用立即返回。
        self.dispatch_issue_cross_repo_final_review_trigger(&updated, review_provider)
            .await;
        Ok(updated)
    }

    #[allow(dead_code)]
    /// Historical recovery compatibility only. It preserves former attempts that
    /// had already completed an InternalPrReview; fresh group attempts are routed
    /// through `prepare_group_final_confirm_from_readiness` and user-driven
    /// `handle_final_confirm`.
    pub(crate) async fn complete_group_attempt_after_final_review(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        if attempt.scope != CodingAttemptScope::WorkItemGroup {
            return Err(CodingWorkspaceEngineError::FinalConfirmNotReady(
                attempt.id.clone(),
            ));
        }
        self.validate_attempt_issue_shared_worktree_owner_if_present(attempt)?;
        self.run_group_completion_gates(attempt).await?;
        let completed = self.store.update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Completed,
        )?;
        self.mark_completed_group_work_items_if_present(&completed)?;
        self.release_issue_shared_worktree_lock_for_attempt(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
        )?;
        Ok(completed)
    }

    fn mark_completed_group_work_items_if_present(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<(), CodingWorkspaceEngineError> {
        let completed_units = self
            .store
            .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)?
            .into_iter()
            .filter(|unit| {
                unit.status == crate::product::coding_models::CodingExecutionUnitStatus::Completed
            })
            .collect::<Vec<_>>();
        if self.schema_v2_group_plan_lineage(attempt)?.is_some() {
            if let Some(last_completed) = completed_units.iter().max_by_key(|unit| unit.order_index)
            {
                self.mark_issue_shared_worktree_completed_if_present(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &last_completed.logical_work_item_id,
                    &attempt.id,
                )?;
            }
            return Ok(());
        }
        let lifecycle = LifecycleStore::new(self.store.paths());
        let existing_work_item_ids = lifecycle
            .list_work_items(&attempt.project_id, &attempt.issue_id)?
            .into_iter()
            .map(|work_item| work_item.id)
            .collect::<std::collections::HashSet<_>>();
        for unit in completed_units {
            if existing_work_item_ids.contains(&unit.logical_work_item_id) {
                lifecycle.update_work_item_execution_status(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &unit.logical_work_item_id,
                    WorkItemStatus::Completed,
                )?;
                self.mark_issue_shared_worktree_completed_if_present(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &unit.logical_work_item_id,
                    &attempt.id,
                )?;
            }
        }
        Ok(())
    }

    pub(crate) fn mark_work_item_completed_if_present(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<(), ProductStoreError> {
        let lifecycle = LifecycleStore::new(self.store.paths());
        let exists = lifecycle
            .list_work_items(&attempt.project_id, &attempt.issue_id)?
            .iter()
            .any(|work_item| work_item.id == attempt.work_item_id);
        if exists {
            lifecycle.update_work_item_execution_status(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.work_item_id,
                WorkItemStatus::Completed,
            )?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// T18（REQ-GFR-CR-06）：跨仓评审已知风险留痕的 findings 摘要。
// ---------------------------------------------------------------------------

/// 留痕 findings 摘要最多摘录的 message 条数。
const CROSS_REPO_ACK_FINDINGS_MAX_MESSAGES: usize = 5;
/// 留痕 findings 摘要整体截断上限（防超长 findings 撑爆留痕 JSON）。
const CROSS_REPO_ACK_FINDINGS_SUMMARY_LIMIT: usize = 500;

/// 留痕 findings 摘要：severity 计数 + 前 N 条 finding 的 `message`，整体
/// 截断到固定上限（REQ-GFR-CR-06：内容 = 时间 + 评审 id + 结论 + 摘要）。
fn cross_repo_ack_findings_summary(findings: &[ReviewFinding]) -> String {
    let count_severity = |severity: FindingSeverity| {
        findings
            .iter()
            .filter(|finding| finding.severity == severity)
            .count()
    };
    let mut summary = format!(
        "error={}, warning={}, info={}",
        count_severity(FindingSeverity::Error),
        count_severity(FindingSeverity::Warning),
        count_severity(FindingSeverity::Info),
    );
    let messages = findings
        .iter()
        .take(CROSS_REPO_ACK_FINDINGS_MAX_MESSAGES)
        .map(|finding| finding.message.as_str())
        .collect::<Vec<_>>()
        .join("；");
    if !messages.is_empty() {
        summary.push('；');
        summary.push_str(&messages);
    }
    summary
        .chars()
        .take(CROSS_REPO_ACK_FINDINGS_SUMMARY_LIMIT)
        .collect()
}

// ---------------------------------------------------------------------------
// 批 2 T2（REQ-WIGA-09）：plan 终止专用收口（跳过 clean、保留文件、
// 关全部门、终态保留/其余 Aborted、幂等）。
// ---------------------------------------------------------------------------

#[cfg(test)]
mod termination_finalize_tests {
    use super::*;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::coding_attempt_store::{
        CodingAttemptStore, CreateBlockedGateInput, CreateChoiceGateInput, CreateCodingAttemptInput,
    };
    use crate::product::lifecycle_store::{LifecycleStore, UpsertIssueSharedWorktreeInput};
    use crate::web::workspace_ws_types::ProviderConfigSnapshot;

    const PROJECT_ID: &str = "project_0001";
    const ISSUE_ID: &str = "issue_0001";

    fn run_git(cwd: &std::path::Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .args(args)
            .current_dir(cwd)
            .status()
            .expect("start git");
        assert!(status.success(), "git {} failed", args.join(" "));
    }

    fn provider_snapshot() -> ProviderConfigSnapshot {
        ProviderConfigSnapshot {
            author: ProviderName::Fake,
            reviewer: None,
            review_rounds: 0,
            permission_modes: Default::default(),
        }
    }

    fn engine(store: &CodingAttemptStore) -> CodingWorkspaceEngine {
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx)
    }

    /// 红测现场：WorkItem scope attempt + issue 级 shared worktree（Legacy
    /// 路由，`target_snapshot=None`），与生产单仓终止现场同构。刻意不用
    /// group attempt：group 非终态收口会触发 plan projection bundle 深链
    /// （`coding_group_attempt_incomplete`），与本两例要验证的 clean 门/
    /// 关门语义无关；WorkItem + issue 级 worktree 走同一
    /// `ensure_issue_shared_worktree_clean` 门，复现同一对照红锚。
    fn terminated_work_item_attempt_with_dirty_issue_worktree() -> (
        tempfile::TempDir,
        ProductAppPaths,
        CodingAttemptStore,
        CodingExecutionAttempt,
        std::path::PathBuf,
    ) {
        let root = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        let store = CodingAttemptStore::new(paths.clone());
        let attempt = store
            .create_attempt(CreateCodingAttemptInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                work_item_id: "work_item_0001".to_string(),
                base_branch: "HEAD".to_string(),
                branch_name: "aria/issues/issue_0001".to_string(),
                worktree_path: None,
                provider_config_snapshot: provider_snapshot(),
                target_snapshot: None,
                max_auto_rework: 1,
            })
            .unwrap();
        // 脏工作树：真实 git worktree＋未提交文件（旧 handle_abort 在此被
        // clean 检查阻断——tests/handoffs_worktree_routing 的对照红测）。
        let worktree_root = root.path().join("worktree");
        std::fs::create_dir_all(&worktree_root).unwrap();
        run_git(&worktree_root, &["init", "--quiet"]);
        run_git(&worktree_root, &["config", "user.email", "t@example.test"]);
        run_git(&worktree_root, &["config", "user.name", "T2 Termination"]);
        std::fs::write(worktree_root.join("README.md"), "initial\n").unwrap();
        run_git(&worktree_root, &["add", "."]);
        run_git(&worktree_root, &["commit", "--quiet", "-m", "initial"]);
        let dirty_file = worktree_root.join("dirty.txt");
        std::fs::write(&dirty_file, "dirty\n").unwrap();
        let lifecycle = LifecycleStore::new(paths.clone());
        lifecycle
            .upsert_issue_shared_worktree(UpsertIssueSharedWorktreeInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                repository_id: "repository_0001".to_string(),
                branch_name: attempt.branch_name.clone(),
                worktree_path: worktree_root,
                base_branch: attempt.base_branch.clone(),
            })
            .unwrap();
        // 一步获取锁并直接以 attempt 为 owner（与生产 acquire+bind 语义
        // 等价，少一次 lease 中转）。
        lifecycle
            .try_acquire_issue_worktree_lock(PROJECT_ID, ISSUE_ID, "work_item_0001", &attempt.id)
            .unwrap();
        (root, paths, store, attempt, dirty_file)
    }

    /// 红测②：脏工作树终止——专用收口跳过 clean 检查（对照：handle_abort
    /// 被 shared_worktree_dirty_manual_gate 阻断），脏文件留存，attempt 置
    /// Aborted；重复收口幂等。
    #[tokio::test]
    async fn plan_termination_finalization_keeps_dirty_files_and_aborts() {
        let (_root, _paths, store, attempt, dirty_file) =
            terminated_work_item_attempt_with_dirty_issue_worktree();

        // 对照红锚：旧 handle_abort 在同一现场被 clean 检查阻断。
        let abort_error = engine(&store)
            .handle_abort(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .await
            .unwrap_err();
        assert!(
            abort_error
                .to_string()
                .contains("shared_worktree_dirty_manual_gate"),
            "control: manual abort must be blocked by the dirty gate, got {abort_error}"
        );

        // 专用收口：成功、Aborted、脏文件留存。
        let finalized = engine(&store)
            .finalize_attempt_for_plan_termination(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
            )
            .await
            .expect("termination finalization must not require a clean worktree");
        assert_eq!(finalized.status, CodingAttemptStatus::Aborted);
        assert!(dirty_file.exists(), "termination must preserve dirty files");

        // 幂等：重复收口不再改写（保持 Aborted），仍不触发 clean。
        let again = engine(&store)
            .finalize_attempt_for_plan_termination(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
            )
            .await
            .expect("repeated finalization is idempotent");
        assert_eq!(again.status, CodingAttemptStatus::Aborted);
        assert!(dirty_file.exists());
    }

    /// 红测④⑤：全部 open blocked/choice/stage 门关闭、choice 不伪造答案、
    /// Completed 终态保留；manual work item attempt 回归可用。
    #[tokio::test]
    async fn plan_termination_finalization_closes_all_open_gates_and_keeps_terminal() {
        let (_root, _paths, store, attempt, _dirty) =
            terminated_work_item_attempt_with_dirty_issue_worktree();
        // 开齐三类门。
        store
            .create_stage_gate(
                &attempt,
                crate::product::coding_models::CodingExecutionStage::Coding,
                crate::product::coding_models::CodingProviderRole::Coder,
                "2026-10-16T00:00:05Z".to_string(),
                store
                    .get_role_provider_config_snapshot(
                        &attempt.project_id,
                        &attempt.issue_id,
                        &attempt.id,
                    )
                    .unwrap(),
            )
            .unwrap();
        store
            .create_choice_gate(
                &attempt,
                CreateChoiceGateInput {
                    attempt_id: attempt.id.clone(),
                    choice_id: "choice-1".to_string(),
                    stage: crate::product::coding_models::CodingExecutionStage::Coding,
                    node_id: None,
                    role: crate::product::coding_models::CodingProviderRole::Coder,
                    provider: ProviderName::Fake,
                    source: "provider".to_string(),
                    prompt: "选择".to_string(),
                    options: Vec::new(),
                    allow_multiple: false,
                    allow_free_text: false,
                    questions: Vec::new(),
                },
            )
            .unwrap();
        store
            .create_blocked_gate(
                &attempt,
                CreateBlockedGateInput {
                    attempt_id: attempt.id.clone(),
                    stage: crate::product::coding_models::CodingExecutionStage::CodeReview,
                    node_id: None,
                    role: None,
                    title: "评审阻塞".to_string(),
                    description: "评审阻塞".to_string(),
                    reason_code: Some("code_review_blocked".to_string()),
                    evidence_refs: Vec::new(),
                    raw_provider_output_ref: None,
                    available_actions: Vec::new(),
                },
            )
            .unwrap();
        assert_eq!(
            store
                .list_open_stage_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
                .unwrap()
                .len(),
            1
        );

        let finalized = engine(&store)
            .finalize_attempt_for_plan_termination(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
            )
            .await
            .unwrap();
        assert_eq!(finalized.status, CodingAttemptStatus::Aborted);
        // C5：全部 open 门关闭。
        assert!(
            store
                .list_open_stage_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
                .unwrap()
                .is_empty(),
            "stage gates must be closed"
        );
        assert!(
            store
                .list_open_choice_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
                .unwrap()
                .is_empty(),
            "choice gates must be closed"
        );
        assert!(
            store
                .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
                .unwrap()
                .is_empty(),
            "blocked gates must be closed"
        );
        // choice 关门不伪造答案：门记录 Cancelled 且无 response。
        let choice = store
            .list_choice_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .unwrap()
            .into_iter()
            .find(|gate| gate.choice_id == "choice-1")
            .expect("choice gate record survives");
        assert_eq!(
            choice.status,
            crate::product::coding_models::CodingChoiceGateStatus::Cancelled
        );
        assert!(choice.response.is_none(), "no fabricated answer");

        // Completed 终态保留：Completed 的 attempt 不被改写为 Aborted。
        let (_root2, _paths2, store2, attempt2, _dirty2) =
            terminated_work_item_attempt_with_dirty_issue_worktree();
        let running = store2
            .admit_and_transition_attempt_to_executable(
                &attempt2.project_id,
                &attempt2.issue_id,
                &attempt2.id,
            )
            .unwrap();
        let completed = store2
            .update_attempt_status(
                &running.project_id,
                &running.issue_id,
                &running.id,
                CodingAttemptStatus::Completed,
            )
            .unwrap();
        assert_eq!(completed.status, CodingAttemptStatus::Completed);
        let preserved = engine(&store2)
            .finalize_attempt_for_plan_termination(
                &attempt2.project_id,
                &attempt2.issue_id,
                &attempt2.id,
            )
            .await
            .unwrap();
        assert_eq!(
            preserved.status,
            CodingAttemptStatus::Completed,
            "completed attempts must stay completed"
        );
    }

    /// 手动无绑定路径回归：专用收口对 manual work item attempt 同样可用
    ///（跳过 clean——终止语义由编排层把关，收口本身不做绑定互证）。
    #[tokio::test]
    async fn plan_termination_finalization_works_for_manual_work_item_attempt() {
        let root = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        let store = CodingAttemptStore::new(paths.clone());
        let attempt = store
            .create_attempt(CreateCodingAttemptInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                work_item_id: "work_item_0001".to_string(),
                base_branch: "HEAD".to_string(),
                branch_name: "aria/attempt".to_string(),
                worktree_path: None,
                provider_config_snapshot: provider_snapshot(),
                target_snapshot: None,
                max_auto_rework: 0,
            })
            .unwrap();
        let finalized = engine(&store)
            .finalize_attempt_for_plan_termination(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
            )
            .await
            .unwrap();
        assert_eq!(finalized.status, CodingAttemptStatus::Aborted);
    }
}
