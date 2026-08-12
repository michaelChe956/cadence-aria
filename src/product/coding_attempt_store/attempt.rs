use chrono::Utc;
use uuid::Uuid;

#[cfg(test)]
use std::collections::HashMap;
#[cfg(test)]
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(test)]
use std::sync::{Arc, Mutex, OnceLock, mpsc};

use crate::product::coding_attempt_store::CreateCodingAttemptInput;
use crate::product::coding_models::WorkItemExecutionPlan;
use crate::product::coding_models::{
    CodingAttemptScope, CodingAttemptStatus, CodingExecutionAttempt, CodingExecutionStage,
    CodingRoleProviderConfigSnapshot,
};
use crate::product::json_store::{
    ProductStoreError, read_json, validate_relative_artifact_ref, validate_relative_id, write_json,
};
use crate::product::models::{ProviderConversationRef, WorkItemExecutionPlanStatus};
use crate::web::workspace_ws_types::ProviderConfigSnapshot;

use super::WorkItemAttemptCreationGuard;
use super::locking::with_exclusive_lock;

impl super::CodingAttemptStore {
    pub fn create_attempt(
        &self,
        input: CreateCodingAttemptInput,
    ) -> Result<CodingExecutionAttempt, ProductStoreError> {
        let guard = self.acquire_work_item_attempt_creation(
            &input.project_id,
            &input.issue_id,
            &input.work_item_id,
        )?;
        self.create_attempt_with_guard(input, &guard)
    }

    pub(crate) fn create_attempt_with_guard(
        &self,
        input: CreateCodingAttemptInput,
        guard: &WorkItemAttemptCreationGuard,
    ) -> Result<CodingExecutionAttempt, ProductStoreError> {
        validate_relative_id(&input.project_id)?;
        validate_relative_id(&input.issue_id)?;
        validate_relative_id(&input.work_item_id)?;
        super::validate_max_auto_rework(input.max_auto_rework)?;
        guard.validate_identity(
            self,
            &input.project_id,
            &input.issue_id,
            &input.work_item_id,
        )?;

        if let Some(active) =
            self.get_active_attempt(&input.project_id, &input.issue_id, &input.work_item_id)?
        {
            return Err(ProductStoreError::Conflict {
                kind: "active_coding_attempt",
                id: active.id,
            });
        }

        let id = self.allocate_coding_attempt_id();
        let attempt_no = self
            .list_attempts_for_work_item(&input.project_id, &input.issue_id, &input.work_item_id)?
            .iter()
            .map(|attempt| attempt.attempt_no)
            .max()
            .unwrap_or(0)
            + 1;
        let now = Utc::now().to_rfc3339();
        let current_work_item_id = input.work_item_id.clone();
        let attempt = CodingExecutionAttempt {
            id: id.clone(),
            project_id: input.project_id,
            issue_id: input.issue_id,
            work_item_id: input.work_item_id,
            attempt_no,
            scope: CodingAttemptScope::WorkItem,
            status: CodingAttemptStatus::Created,
            version: 0,
            manual_recovery_reason: None,
            admission_ticket_consumed_at: None,
            stage: CodingExecutionStage::PrepareContext,
            base_branch: input.base_branch,
            branch_name: input.branch_name,
            worktree_path: input.worktree_path,
            provider_config_snapshot: input.provider_config_snapshot,
            rework_count: 0,
            max_auto_rework: input.max_auto_rework,
            work_item_group_id: None,
            current_work_item_id: Some(current_work_item_id),
            active_unit_id: None,
            head_commit: None,
            pushed_remote: None,
            review_request_id: None,
            provider_conversations: Vec::new(),
            created_at: now.clone(),
            updated_at: now,
            target_snapshot: input.target_snapshot,
            completed_at: None,
        };

        let attempt_path = self.attempt_path(&attempt.project_id, &attempt.issue_id, &id);
        write_json(&attempt_path, &attempt)?;
        let provider_config_path =
            self.role_provider_config_path(&attempt.project_id, &attempt.issue_id, &id);
        if let Err(error) = write_json(
            &provider_config_path,
            &CodingRoleProviderConfigSnapshot::from(&attempt.provider_config_snapshot),
        ) {
            let _ = std::fs::remove_file(&attempt_path);
            if let Some(parent) = provider_config_path.parent() {
                let _ = std::fs::remove_dir(parent);
            }
            return Err(error);
        }
        Ok(attempt)
    }

    /// 更新可公开修改的 attempt 字段，同时保留状态和 admission 冻结字段。
    ///
    /// 该 API 必须以字段方式构造持久化 record，而非将传入的 `attempt` 整体写盘。
    /// 因而后续为 admission 添加的新冻结字段时，编译器会强制在这里明确从
    /// 已持久化 record 保留它们，避免公开 API 无意中取得覆写权限。
    ///
    /// ```compile_fail
    /// use cadence_aria::product::coding_attempt_store::CodingAttemptStore;
    ///
    /// // 写入 status 的底层 API 只对 coding_attempt_store 模块可见。
    /// let _ = CodingAttemptStore::save_coding_attempt_with_status;
    /// ```
    pub fn update_attempt_non_status_fields(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<(), ProductStoreError> {
        let path = self.attempt_path(&attempt.project_id, &attempt.issue_id, &attempt.id);
        with_exclusive_lock(&path, || {
            let stored = self.get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
            if stored.work_item_id != attempt.work_item_id
                || stored.attempt_no != attempt.attempt_no
                || stored.scope != attempt.scope
                || stored.work_item_group_id != attempt.work_item_group_id
            {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "coding_attempt",
                    id: attempt.id.clone(),
                });
            }
            let updated = CodingExecutionAttempt {
                id: stored.id,
                project_id: stored.project_id,
                issue_id: stored.issue_id,
                work_item_id: stored.work_item_id,
                attempt_no: stored.attempt_no,
                scope: stored.scope,
                status: stored.status,
                version: stored.version,
                manual_recovery_reason: stored.manual_recovery_reason,
                admission_ticket_consumed_at: stored.admission_ticket_consumed_at,
                stage: attempt.stage.clone(),
                base_branch: attempt.base_branch.clone(),
                branch_name: attempt.branch_name.clone(),
                worktree_path: attempt.worktree_path.clone(),
                provider_config_snapshot: attempt.provider_config_snapshot.clone(),
                rework_count: attempt.rework_count,
                max_auto_rework: attempt.max_auto_rework,
                work_item_group_id: stored.work_item_group_id,
                current_work_item_id: attempt.current_work_item_id.clone(),
                active_unit_id: attempt.active_unit_id.clone(),
                head_commit: attempt.head_commit.clone(),
                pushed_remote: attempt.pushed_remote.clone(),
                review_request_id: attempt.review_request_id.clone(),
                provider_conversations: attempt.provider_conversations.clone(),
                created_at: attempt.created_at.clone(),
                updated_at: attempt.updated_at.clone(),
                target_snapshot: stored.target_snapshot,
                completed_at: attempt.completed_at.clone(),
            };
            self.save_coding_attempt_with_status(&updated)
        })
    }

    #[cfg(test)]
    pub(crate) fn write_coding_attempt_for_test(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<(), ProductStoreError> {
        self.save_coding_attempt_with_status(attempt)
    }

    /// 仅供受控状态转换使用的底层全字段持久化方法。
    pub(super) fn save_coding_attempt_with_status(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(&attempt.project_id)?;
        validate_relative_id(&attempt.issue_id)?;
        validate_relative_id(&attempt.id)?;
        write_json(
            &self.attempt_path(&attempt.project_id, &attempt.issue_id, &attempt.id),
            attempt,
        )?;
        Ok(())
    }

    pub(crate) fn allocate_coding_attempt_id(&self) -> String {
        format!("coding_attempt_{}", Uuid::new_v4().simple())
    }

    pub fn get_attempt(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<CodingExecutionAttempt, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(attempt_id)?;
        let path = self.attempt_path(project_id, issue_id, attempt_id);
        if !super::path_is_regular_file(&path)? {
            return Err(ProductStoreError::NotFound {
                kind: "coding_attempt",
                id: attempt_id.to_string(),
            });
        }
        let attempt: CodingExecutionAttempt = read_json(&path)?;
        if attempt.project_id != project_id
            || attempt.issue_id != issue_id
            || attempt.id != attempt_id
        {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "coding_attempt",
                id: attempt_id.to_string(),
            });
        }
        Ok(attempt)
    }

    pub fn save_work_item_execution_plan(
        &self,
        plan: &WorkItemExecutionPlan,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(&plan.project_id)?;
        validate_relative_id(&plan.issue_id)?;
        validate_relative_id(&plan.attempt_id)?;
        write_json(
            &self.work_item_execution_plan_path(&plan.project_id, &plan.issue_id, &plan.attempt_id),
            plan,
        )
    }

    pub fn get_work_item_execution_plan(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<Option<WorkItemExecutionPlan>, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(attempt_id)?;
        let path = self.work_item_execution_plan_path(project_id, issue_id, attempt_id);
        if !super::path_is_regular_file(&path)? {
            return Ok(None);
        }
        read_json(&path).map(Some)
    }

    pub fn update_work_item_execution_plan_status(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        status: WorkItemExecutionPlanStatus,
    ) -> Result<WorkItemExecutionPlan, ProductStoreError> {
        let mut plan = self
            .get_work_item_execution_plan(project_id, issue_id, attempt_id)?
            .ok_or_else(|| ProductStoreError::NotFound {
                kind: "work_item_execution_plan",
                id: attempt_id.to_string(),
            })?;
        plan.status = status;
        plan.updated_at = Utc::now().to_rfc3339();
        self.save_work_item_execution_plan(&plan)?;
        Ok(plan)
    }

    pub fn get_attempt_by_id(
        &self,
        attempt_id: &str,
    ) -> Result<CodingExecutionAttempt, ProductStoreError> {
        self.find_attempt_by_id(attempt_id)
    }

    pub fn list_attempts_for_work_item(
        &self,
        project_id: &str,
        issue_id: &str,
        work_item_id: &str,
    ) -> Result<Vec<CodingExecutionAttempt>, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(work_item_id)?;
        let mut attempts: Vec<CodingExecutionAttempt> =
            super::list_json_records(&self.coding_attempts_root(project_id, issue_id))?
                .into_iter()
                .filter(|attempt: &CodingExecutionAttempt| attempt.work_item_id == work_item_id)
                .collect();
        attempts.sort_by(|left, right| {
            left.attempt_no
                .cmp(&right.attempt_no)
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(attempts)
    }

    /// 列出某 issue 下的全部 coding attempt（不限 work_item / scope）。
    ///
    /// 用于 issue 级清理判定（如 attempt 删除后是否还有其他 attempt 记录）。
    /// 与 `list_attempts_for_work_item` 不同：不做 work_item 过滤，覆盖该 issue 下
    /// 所有 attempt 记录（含 group / single / 不同 work_item）。
    pub fn list_attempts_for_issue(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> Result<Vec<CodingExecutionAttempt>, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        let mut attempts: Vec<CodingExecutionAttempt> =
            super::list_json_records(&self.coding_attempts_root(project_id, issue_id))?;
        attempts.sort_by(|left, right| {
            left.attempt_no
                .cmp(&right.attempt_no)
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(attempts)
    }

    pub fn get_active_attempt(
        &self,
        project_id: &str,
        issue_id: &str,
        work_item_id: &str,
    ) -> Result<Option<CodingExecutionAttempt>, ProductStoreError> {
        let active = self
            .list_attempts_for_work_item(project_id, issue_id, work_item_id)?
            .into_iter()
            .find(|attempt| attempt.status.is_active());
        Ok(active)
    }

    pub fn delete_attempt(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<CodingExecutionAttempt, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(attempt_id)?;
        let attempt = self.get_attempt(project_id, issue_id, attempt_id)?;
        self.delete_group_initialization_for_attempt(&attempt)?;
        super::remove_file_if_exists(&self.attempt_path(project_id, issue_id, attempt_id))?;
        super::remove_dir_all_if_exists(&self.attempt_dir(project_id, issue_id, attempt_id))?;
        Ok(attempt)
    }

    pub fn delete_attempts_for_work_item(
        &self,
        project_id: &str,
        issue_id: &str,
        work_item_id: &str,
    ) -> Result<Vec<CodingExecutionAttempt>, ProductStoreError> {
        let attempts = self.list_attempts_for_work_item(project_id, issue_id, work_item_id)?;
        for attempt in &attempts {
            self.delete_attempt(project_id, issue_id, &attempt.id)?;
        }
        Ok(attempts)
    }

    pub fn update_attempt_status(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        status: CodingAttemptStatus,
    ) -> Result<CodingExecutionAttempt, ProductStoreError> {
        let _recovery_arbitration = if status == CodingAttemptStatus::AwaitingPlanAmendment {
            Some(self.acquire_failed_code_review_recovery_arbitration(
                project_id, issue_id, attempt_id,
            )?)
        } else {
            None
        };
        let path = self.attempt_path(project_id, issue_id, attempt_id);
        with_exclusive_lock(&path, || {
            let mut attempt = self.get_attempt(project_id, issue_id, attempt_id)?;
            if !valid_status_transition(&attempt.status, &status) {
                return Err(ProductStoreError::Io(format!(
                    "invalid_coding_attempt_status_transition: {:?} -> {:?}",
                    attempt.status, status
                )));
            }
            if attempt.scope == CodingAttemptScope::WorkItemGroup
                && matches!(
                    &status,
                    CodingAttemptStatus::Completed
                        | CodingAttemptStatus::Failed
                        | CodingAttemptStatus::Aborted
                )
            {
                return self.update_group_terminal_status_locked(attempt, status);
            }
            if status == CodingAttemptStatus::AwaitingPlanAmendment {
                self.rollback_failed_code_review_recovery_for_plan_amendment_locked(&attempt)?;
            }
            let now = Utc::now().to_rfc3339();
            if matches!(
                status,
                CodingAttemptStatus::Completed
                    | CodingAttemptStatus::Failed
                    | CodingAttemptStatus::Aborted
            ) {
                attempt.completed_at = Some(now.clone());
            }
            attempt.status = status;
            attempt.updated_at = now;
            write_json(&path, &attempt)?;
            Ok(attempt)
        })
    }

    pub fn reopen_failed_code_review_attempt(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<CodingExecutionAttempt, ProductStoreError> {
        self.get_attempt(project_id, issue_id, attempt_id)?;
        Err(ProductStoreError::Io(
            "coding_failed_review_not_recoverable".to_string(),
        ))
    }

    pub fn update_attempt_stage(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        stage: CodingExecutionStage,
    ) -> Result<CodingExecutionAttempt, ProductStoreError> {
        let path = self.attempt_path(project_id, issue_id, attempt_id);
        with_exclusive_lock(&path, || {
            let mut attempt = self.get_attempt(project_id, issue_id, attempt_id)?;
            if !valid_stage_transition(&attempt.stage, &stage) {
                return Err(ProductStoreError::Io(format!(
                    "invalid_coding_attempt_stage_transition: {:?} -> {:?}",
                    attempt.stage, stage
                )));
            }
            attempt.stage = stage;
            attempt.updated_at = Utc::now().to_rfc3339();
            self.save_coding_attempt_with_status(&attempt)?;
            Ok(attempt)
        })
    }

    pub fn update_attempt_worktree_path(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        worktree_path: std::path::PathBuf,
    ) -> Result<CodingExecutionAttempt, ProductStoreError> {
        let path = self.attempt_path(project_id, issue_id, attempt_id);
        with_exclusive_lock(&path, || {
            let mut attempt = self.get_attempt(project_id, issue_id, attempt_id)?;
            attempt.worktree_path = Some(worktree_path);
            attempt.updated_at = Utc::now().to_rfc3339();
            self.save_coding_attempt_with_status(&attempt)?;
            Ok(attempt)
        })
    }

    /// 更新 head commit，只覆盖该 API 负责的字段。
    ///
    /// 读-改-写全程持有 attempt 文件锁，并从锁内最新 record 保留
    /// status/version/manual_recovery_reason/target_snapshot/
    /// admission_ticket_consumed_at 等冻结字段，避免并发覆盖 admission CAS 提交。
    pub fn update_attempt_head_commit(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        head_commit: Option<String>,
    ) -> Result<CodingExecutionAttempt, ProductStoreError> {
        let path = self.attempt_path(project_id, issue_id, attempt_id);
        with_exclusive_lock(&path, || {
            let mut attempt = self.get_attempt(project_id, issue_id, attempt_id)?;
            attempt.head_commit = head_commit;
            attempt.updated_at = Utc::now().to_rfc3339();
            #[cfg(test)]
            notify_attempt_write_gap(&path);
            self.save_coding_attempt_with_status(&attempt)?;
            Ok(attempt)
        })
    }

    /// 更新 review request 相关字段，只覆盖该 API 负责的字段。
    ///
    /// 读-改-写全程持有 attempt 文件锁，并从锁内最新 record 保留
    /// status/version/manual_recovery_reason/target_snapshot/
    /// admission_ticket_consumed_at 等冻结字段，避免并发覆盖 admission CAS 提交。
    pub fn update_attempt_review_request_state(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        head_commit: String,
        pushed_remote: String,
        review_request_id: String,
    ) -> Result<CodingExecutionAttempt, ProductStoreError> {
        let path = self.attempt_path(project_id, issue_id, attempt_id);
        with_exclusive_lock(&path, || {
            let mut attempt = self.get_attempt(project_id, issue_id, attempt_id)?;
            attempt.head_commit = Some(head_commit);
            attempt.pushed_remote = Some(pushed_remote);
            attempt.review_request_id = Some(review_request_id);
            attempt.updated_at = Utc::now().to_rfc3339();
            #[cfg(test)]
            notify_attempt_write_gap(&path);
            self.save_coding_attempt_with_status(&attempt)?;
            Ok(attempt)
        })
    }

    /// 更新 provider 配置快照，只覆盖该 API 负责的字段。
    ///
    /// 读-改-写全程持有 attempt 文件锁，并从锁内最新 record 保留
    /// status/version/manual_recovery_reason/target_snapshot/
    /// admission_ticket_consumed_at 等冻结字段，避免并发覆盖 admission CAS 提交。
    pub fn update_attempt_provider_config_snapshot(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        provider_config_snapshot: ProviderConfigSnapshot,
    ) -> Result<CodingExecutionAttempt, ProductStoreError> {
        let path = self.attempt_path(project_id, issue_id, attempt_id);
        with_exclusive_lock(&path, || {
            let mut attempt = self.get_attempt(project_id, issue_id, attempt_id)?;
            attempt.provider_config_snapshot = provider_config_snapshot;
            attempt.updated_at = Utc::now().to_rfc3339();
            #[cfg(test)]
            notify_attempt_write_gap(&path);
            self.save_coding_attempt_with_status(&attempt)?;
            Ok(attempt)
        })
    }

    pub fn get_role_provider_config_snapshot(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<CodingRoleProviderConfigSnapshot, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(attempt_id)?;
        let path = self.role_provider_config_path(project_id, issue_id, attempt_id);
        if super::path_is_regular_file(&path)? {
            return read_json(&path);
        }
        let attempt = self.get_attempt(project_id, issue_id, attempt_id)?;
        Ok(CodingRoleProviderConfigSnapshot::from(
            &attempt.provider_config_snapshot,
        ))
    }

    pub fn update_role_provider_config_snapshot(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        role_provider_config_snapshot: CodingRoleProviderConfigSnapshot,
    ) -> Result<CodingRoleProviderConfigSnapshot, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(attempt_id)?;
        let attempt = self.get_attempt(project_id, issue_id, attempt_id)?;
        write_json(
            &self.role_provider_config_path(&attempt.project_id, &attempt.issue_id, &attempt.id),
            &role_provider_config_snapshot,
        )?;
        Ok(role_provider_config_snapshot)
    }

    /// 自增 rework 计数，只覆盖该 API 负责的字段。
    ///
    /// 读-改-写全程持有 attempt 文件锁，并从锁内最新 record 保留
    /// status/version/manual_recovery_reason/target_snapshot/
    /// admission_ticket_consumed_at 等冻结字段，避免并发覆盖 admission CAS 提交。
    pub fn increment_attempt_rework_count(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<CodingExecutionAttempt, ProductStoreError> {
        let path = self.attempt_path(project_id, issue_id, attempt_id);
        with_exclusive_lock(&path, || {
            let mut attempt = self.get_attempt(project_id, issue_id, attempt_id)?;
            attempt.rework_count += 1;
            attempt.updated_at = Utc::now().to_rfc3339();
            #[cfg(test)]
            notify_attempt_write_gap(&path);
            self.save_coding_attempt_with_status(&attempt)?;
            Ok(attempt)
        })
    }

    pub fn update_attempt_max_auto_rework(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        max_auto_rework: u32,
    ) -> Result<CodingExecutionAttempt, ProductStoreError> {
        super::validate_max_auto_rework(max_auto_rework)?;
        let path = self.attempt_path(project_id, issue_id, attempt_id);
        let mut attempt = self.get_attempt(project_id, issue_id, attempt_id)?;
        attempt.max_auto_rework = max_auto_rework;
        attempt.updated_at = Utc::now().to_rfc3339();
        write_json(&path, &attempt)?;
        Ok(attempt)
    }

    /// 替换 provider 会话引用，只覆盖该 API 负责的字段。
    ///
    /// 读-改-写全程持有 attempt 文件锁，并从锁内最新 record 保留
    /// status/version/manual_recovery_reason/target_snapshot/
    /// admission_ticket_consumed_at 等冻结字段，避免并发覆盖 admission CAS 提交。
    pub fn replace_attempt_provider_conversations(
        &self,
        attempt: &CodingExecutionAttempt,
        provider_conversations: Vec<ProviderConversationRef>,
    ) -> Result<CodingExecutionAttempt, ProductStoreError> {
        self.validate_scoped_attempt_record(attempt, &attempt.id, "coding_attempt", &attempt.id)?;
        let path = self.attempt_path(&attempt.project_id, &attempt.issue_id, &attempt.id);
        with_exclusive_lock(&path, || {
            let mut updated =
                self.get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
            updated.provider_conversations = provider_conversations;
            updated.updated_at = Utc::now().to_rfc3339();
            #[cfg(test)]
            notify_attempt_write_gap(&path);
            self.save_coding_attempt_with_status(&updated)?;
            Ok(updated)
        })
    }

    pub fn read_attempt_artifact_text(
        &self,
        attempt: &CodingExecutionAttempt,
        artifact_ref: &str,
    ) -> Result<String, ProductStoreError> {
        use std::fs;

        validate_relative_artifact_ref(artifact_ref)?;
        self.validate_scoped_attempt_record(
            attempt,
            &attempt.id,
            "coding_attempt_artifact",
            artifact_ref,
        )?;
        let path = self
            .attempt_dir(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .join(artifact_ref);
        fs::read_to_string(&path)
            .map_err(|error| ProductStoreError::Io(format!("read {}: {error}", path.display())))
    }
}

pub(super) fn valid_status_transition(
    current: &CodingAttemptStatus,
    next: &CodingAttemptStatus,
) -> bool {
    if current == &CodingAttemptStatus::AwaitingManualRecovery {
        return next == &CodingAttemptStatus::Aborted;
    }
    if current == next {
        return true;
    }
    match current {
        CodingAttemptStatus::Created => {
            matches!(
                next,
                CodingAttemptStatus::Running | CodingAttemptStatus::Aborted
            )
        }
        CodingAttemptStatus::Running => matches!(
            next,
            CodingAttemptStatus::WaitingForHuman
                | CodingAttemptStatus::Blocked
                | CodingAttemptStatus::AwaitingPlanAmendment
                | CodingAttemptStatus::Completed
                | CodingAttemptStatus::Failed
                | CodingAttemptStatus::Aborted
        ),
        CodingAttemptStatus::WaitingForHuman => {
            matches!(
                next,
                CodingAttemptStatus::Running
                    | CodingAttemptStatus::Completed
                    | CodingAttemptStatus::Aborted
            )
        }
        CodingAttemptStatus::Blocked => {
            matches!(
                next,
                CodingAttemptStatus::Running
                    | CodingAttemptStatus::AwaitingPlanAmendment
                    | CodingAttemptStatus::Aborted
            )
        }
        CodingAttemptStatus::AwaitingManualRecovery => {
            matches!(next, CodingAttemptStatus::Aborted)
        }
        CodingAttemptStatus::AwaitingPlanAmendment => matches!(
            next,
            CodingAttemptStatus::ApplyingPlanAmendment | CodingAttemptStatus::Aborted
        ),
        CodingAttemptStatus::ApplyingPlanAmendment => matches!(
            next,
            CodingAttemptStatus::Running
                | CodingAttemptStatus::AmendmentApplyFailed
                | CodingAttemptStatus::Aborted
        ),
        CodingAttemptStatus::AmendmentApplyFailed => matches!(
            next,
            CodingAttemptStatus::ApplyingPlanAmendment | CodingAttemptStatus::Aborted
        ),
        CodingAttemptStatus::Completed
        | CodingAttemptStatus::Failed
        | CodingAttemptStatus::Aborted => false,
    }
}

fn valid_stage_transition(current: &CodingExecutionStage, next: &CodingExecutionStage) -> bool {
    if current == next {
        return true;
    }
    if matches!(
        (current, next),
        (
            CodingExecutionStage::CodeReview,
            CodingExecutionStage::Coding
        )
    ) {
        return true;
    }
    next.order() >= current.order()
}

#[cfg(test)]
struct AttemptWriteGapHookEntry {
    registration_id: u64,
    reached_gap: mpsc::Sender<()>,
    proceed: Arc<Mutex<mpsc::Receiver<()>>>,
}

#[cfg(test)]
pub(crate) struct AttemptWriteGapHookGuard {
    attempt_path: PathBuf,
    registration_id: u64,
}

#[cfg(test)]
static ATTEMPT_WRITE_GAP_HOOKS: OnceLock<Mutex<HashMap<PathBuf, AttemptWriteGapHookEntry>>> =
    OnceLock::new();

#[cfg(test)]
static NEXT_ATTEMPT_WRITE_GAP_HOOK_ID: AtomicU64 = AtomicU64::new(1);

#[cfg(test)]
fn attempt_write_gap_hooks() -> &'static Mutex<HashMap<PathBuf, AttemptWriteGapHookEntry>> {
    ATTEMPT_WRITE_GAP_HOOKS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Registers a test hook that pauses a full-record attempt write API after it
/// read the attempt record but before the write-back is persisted. The caller
/// receives a channel that fires when the gap is reached and a sender that
/// releases the paused write.
#[cfg(test)]
pub(crate) fn register_attempt_write_gap_hook(
    attempt_path: &Path,
) -> (
    AttemptWriteGapHookGuard,
    mpsc::Receiver<()>,
    mpsc::Sender<()>,
) {
    let attempt_path = std::fs::canonicalize(attempt_path)
        .expect("registered attempt write gap hook path must be canonicalizable");
    let registration_id = NEXT_ATTEMPT_WRITE_GAP_HOOK_ID.fetch_add(1, Ordering::Relaxed);
    let (reached_gap_tx, reached_gap_rx) = mpsc::channel();
    let (proceed_tx, proceed_rx) = mpsc::channel();
    let mut hooks = attempt_write_gap_hooks()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert!(
        !hooks.contains_key(&attempt_path),
        "attempt write gap hook already registered for {}",
        attempt_path.display()
    );
    hooks.insert(
        attempt_path.clone(),
        AttemptWriteGapHookEntry {
            registration_id,
            reached_gap: reached_gap_tx,
            proceed: Arc::new(Mutex::new(proceed_rx)),
        },
    );
    (
        AttemptWriteGapHookGuard {
            attempt_path,
            registration_id,
        },
        reached_gap_rx,
        proceed_tx,
    )
}

#[cfg(test)]
fn notify_attempt_write_gap(attempt_path: &Path) {
    let Ok(attempt_path) = std::fs::canonicalize(attempt_path) else {
        return;
    };
    let Some((reached_gap, proceed)) = attempt_write_gap_hooks()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(&attempt_path)
        .map(|entry| (entry.reached_gap.clone(), Arc::clone(&entry.proceed)))
    else {
        return;
    };
    let _ = reached_gap.send(());
    let _ = proceed
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .recv();
}

#[cfg(test)]
impl Drop for AttemptWriteGapHookGuard {
    fn drop(&mut self) {
        let mut hooks = attempt_write_gap_hooks()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if hooks
            .get(&self.attempt_path)
            .is_some_and(|entry| entry.registration_id == self.registration_id)
        {
            hooks.remove(&self.attempt_path);
        }
    }
}
