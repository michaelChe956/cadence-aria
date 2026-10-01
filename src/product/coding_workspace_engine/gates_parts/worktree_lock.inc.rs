impl CodingWorkspaceEngine {
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

    /// G5（终局关闸缺口）：显式恢复／重开（RecoverCoding／RestartCoding）
    /// 路径的 WI 级工作树锁复位。
    ///
    /// 现场（issue_0004/WI-001）：runner 死亡 → 确认接管显式释放共享工作树
    /// 锁 → 恢复通道只把 attempt CAS 回 Running 并重启 runner → 编码段
    /// 入口校验（`validate_attempt_issue_shared_worktree_lock_if_present`）
    /// 以 `issue_worktree_lock_owner` 冲突死亡 → 回 AwaitingManualRecovery，
    /// 形成「恢复即失败」死循环，且 restart 对 AMR 恒 409——无产品清理面。
    ///
    /// 判定复用 C1/C2 租约三态（`classify_worktree_lease`，不新建体系）：
    /// - 自持活跃租约 → 原样继续（合法续跑）；
    /// - 锁已释放（空 owner）或 owner 为本 attempt 的死亡残留（AMR／终态
    ///   均非活跃）→ 按死锁判别清理后重新获取并绑定到该 attempt；
    /// - 瞬态 lease 残留（owner 未绑 attempt 且 active item 归本 attempt 的
    ///   恢复目标）→ 沿用 `bind_*_worktree_lock_to_attempt` 既有收编语义；
    /// - 活跃他人 → 停等（`coding_run_already_running`，不抢占）；
    /// - 死亡他人仍占锁 → 停等并指向既有确认接管面（`confirm_takeover`）；
    /// - 证据未知 → fail-closed 停等（无租约事实的 legacy attempt 除外，
    ///   与 C2 `lease_exclusion_option` 同语义）。
    pub fn ensure_issue_worktree_lock_for_resumed_attempt(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<(), CodingWorkspaceEngineError> {
        use crate::product::models::automation::LeaseDisposition;

        let lifecycle = LifecycleStore::new(self.store.paths());
        let route = self.route_issue_shared_worktree(attempt)?;
        let lease = self.store.classify_worktree_lease(&attempt.project_id, &attempt.issue_id);
        match lease.disposition {
            LeaseDisposition::ActiveWait => {
                if lease.lease_id == attempt.id {
                    return Ok(());
                }
                Err(CodingWorkspaceEngineError::Store(ProductStoreError::Io(format!(
                    "coding_run_already_running: lease {} 已在运行，请等待",
                    lease.lease_id
                ))))
            }
            LeaseDisposition::DeadNeedsTakeover => {
                if !lease.lease_id.is_empty() && lease.lease_id != attempt.id {
                    // 死亡他人仍占锁：不自动抢占，等待项投影走既有
                    // lease_takeover 面，人工经 `confirm_takeover` 清出后再恢复。
                    return Err(CodingWorkspaceEngineError::Store(ProductStoreError::Io(
                        format!(
                            "coding_lease_takeover_required: lease {} 已死，请先人工确认接管后再恢复",
                            lease.lease_id
                        ),
                    )));
                }
                // 锁已释放（空 owner）或自持死亡残留：清理后重新获取并绑定。
                if !lease.lease_id.is_empty() {
                    self.release_resumed_route_lock_by_owner(attempt, &route, &lifecycle)?;
                }
                self.reacquire_and_bind_resumed_route_lock(attempt, &route, &lifecycle)
            }
            LeaseDisposition::UnknownNeedsHuman => {
                if lease
                    .evidence
                    .iter()
                    .any(|fact| fact.contains("worktree record not found"))
                {
                    // 无租约事实（未启锁的 legacy attempt）：不拦截。
                    return Ok(());
                }
                if self.resume_target_holds_transient_lease(attempt, &route, &lifecycle)? {
                    // 瞬态 lease 残留且 active item 归本 attempt 的恢复目标：
                    // 沿用 bind 的既有收编语义重绑，不新建清理体系。
                    return self.reacquire_and_bind_resumed_route_lock(
                        attempt,
                        &route,
                        &lifecycle,
                    );
                }
                Err(CodingWorkspaceEngineError::Store(ProductStoreError::Io(format!(
                    "coding_lease_state_unknown: {}",
                    lease.evidence.join("; ")
                ))))
            }
        }
    }

    /// 恢复路径按路由释放本 attempt 名下的残留锁（幂等，只认 owner）。
    fn release_resumed_route_lock_by_owner(
        &self,
        attempt: &CodingExecutionAttempt,
        route: &IssueSharedWorktreeRoute,
        lifecycle: &LifecycleStore,
    ) -> Result<(), CodingWorkspaceEngineError> {
        match route {
            IssueSharedWorktreeRoute::Legacy => {
                if lifecycle
                    .get_issue_shared_worktree(&attempt.project_id, &attempt.issue_id)?
                    .is_some()
                {
                    lifecycle.release_issue_worktree_lock_by_owner(
                        &attempt.project_id,
                        &attempt.issue_id,
                        &attempt.id,
                    )?;
                }
            }
            IssueSharedWorktreeRoute::Repository { repository_id } => {
                if lifecycle
                    .get_repo_shared_worktree(
                        &attempt.project_id,
                        &attempt.issue_id,
                        *repository_id,
                    )?
                    .is_some()
                {
                    lifecycle.release_repo_worktree_lock_by_owner(
                        &attempt.project_id,
                        &attempt.issue_id,
                        *repository_id,
                        &attempt.id,
                    )?;
                }
            }
        }
        Ok(())
    }

    /// 恢复路径重新获取共享工作树锁并绑定到该 attempt（bind 既有语义可
    /// 收编 `*_worktree_lease_*` 瞬态 owner）。
    fn reacquire_and_bind_resumed_route_lock(
        &self,
        attempt: &CodingExecutionAttempt,
        route: &IssueSharedWorktreeRoute,
        lifecycle: &LifecycleStore,
    ) -> Result<(), CodingWorkspaceEngineError> {
        let work_item_id = self.resume_work_item_id_for_attempt(attempt, route, lifecycle)?;
        match route {
            IssueSharedWorktreeRoute::Legacy => {
                let lease_id = format!(
                    "issue_worktree_lease_resume_{}",
                    uuid::Uuid::new_v4().simple()
                );
                lifecycle.try_acquire_issue_worktree_lock(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &work_item_id,
                    &lease_id,
                )?;
                lifecycle.bind_issue_worktree_lock_to_attempt(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &work_item_id,
                    &attempt.id,
                )?;
            }
            IssueSharedWorktreeRoute::Repository { repository_id } => {
                let lease_id = format!(
                    "repo_worktree_lease_resume_{}",
                    uuid::Uuid::new_v4().simple()
                );
                lifecycle.try_acquire_repo_worktree_lock(
                    &attempt.project_id,
                    &attempt.issue_id,
                    *repository_id,
                    &work_item_id,
                    &lease_id,
                )?;
                lifecycle.bind_repo_worktree_lock_to_attempt(
                    &attempt.project_id,
                    &attempt.issue_id,
                    *repository_id,
                    &work_item_id,
                    &attempt.id,
                )?;
            }
        }
        Ok(())
    }

    /// 恢复目标 work item 解析（与
    /// `validate_attempt_issue_shared_worktree_lock_if_present` 同链）：
    /// `current_work_item_id` 优先，group 作用域回退锁记录 active item，
    /// 最后回退 attempt 落盘的 `work_item_id`。
    fn resume_work_item_id_for_attempt(
        &self,
        attempt: &CodingExecutionAttempt,
        route: &IssueSharedWorktreeRoute,
        lifecycle: &LifecycleStore,
    ) -> Result<String, CodingWorkspaceEngineError> {
        if let Some(current) = attempt.current_work_item_id.as_deref() {
            return Ok(current.to_string());
        }
        if attempt.scope == crate::product::coding_models::CodingAttemptScope::WorkItemGroup {
            let shared = match route {
                IssueSharedWorktreeRoute::Legacy => {
                    lifecycle.get_issue_shared_worktree(&attempt.project_id, &attempt.issue_id)?
                }
                IssueSharedWorktreeRoute::Repository { repository_id } => lifecycle
                    .get_repo_shared_worktree(
                        &attempt.project_id,
                        &attempt.issue_id,
                        *repository_id,
                    )?,
            };
            if let Some(active) = shared.and_then(|record| record.current_active_work_item_id) {
                return Ok(active);
            }
        }
        Ok(attempt.work_item_id.clone())
    }

    /// Unknown 证据下的瞬态残留识别：owner 未绑 attempt 且 active item
    /// 恰为本 attempt 的恢复目标（此时不存在并发 create 竞争——同 WI 的
    /// 活跃 attempt 唯一，REST create 已被 `coding_attempt_active` 拒绝）。
    fn resume_target_holds_transient_lease(
        &self,
        attempt: &CodingExecutionAttempt,
        route: &IssueSharedWorktreeRoute,
        lifecycle: &LifecycleStore,
    ) -> Result<bool, CodingWorkspaceEngineError> {
        let record = match route {
            IssueSharedWorktreeRoute::Legacy => {
                lifecycle.get_issue_shared_worktree(&attempt.project_id, &attempt.issue_id)?
            }
            IssueSharedWorktreeRoute::Repository { repository_id } => lifecycle
                .get_repo_shared_worktree(&attempt.project_id, &attempt.issue_id, *repository_id)?,
        };
        let Some(record) = record else {
            return Ok(false);
        };
        let owner_transient = record
            .current_lock_owner_id
            .as_deref()
            .is_some_and(|owner| {
                owner.starts_with("issue_worktree_lease_")
                    || owner.starts_with("repo_worktree_lease_")
            });
        if !owner_transient {
            return Ok(false);
        }
        let work_item_id = self.resume_work_item_id_for_attempt(attempt, route, lifecycle)?;
        Ok(record.current_active_work_item_id.as_deref() == Some(work_item_id.as_str()))
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
}
