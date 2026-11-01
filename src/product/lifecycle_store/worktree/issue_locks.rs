use super::{
    IssueWorktreeLockLease, SharedWorktreeOwnerHints, lock_owner_mismatch, read_issue_worktree,
};

use chrono::Utc;

use crate::product::coding_attempt_store::locking::with_exclusive_lock;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
use crate::product::lifecycle_store::{
    LifecycleStore, UpsertIssueSharedWorktreeInput, path_is_regular_file, remove_file_if_exists,
};
use crate::product::models::{IssueSharedWorktree, IssueSharedWorktreeStatus};

impl LifecycleStore {
    pub fn upsert_issue_shared_worktree(
        &self,
        input: UpsertIssueSharedWorktreeInput,
    ) -> Result<IssueSharedWorktree, ProductStoreError> {
        validate_relative_id(&input.project_id)?;
        validate_relative_id(&input.issue_id)?;
        validate_relative_id(&input.repository_id)?;

        let project_id = input.project_id.clone();
        let issue_id = input.issue_id.clone();
        let legacy_repository = input.repository_id.clone();
        self.with_shared_worktree_write_admission(
            &project_id,
            &issue_id,
            None,
            SharedWorktreeOwnerHints::empty(),
            false,
            Some(legacy_repository.as_str()),
            move || {
                let path = self.issue_shared_worktree_path(&input.project_id, &input.issue_id);
                with_exclusive_lock(&path, || {
                    let now = Utc::now().to_rfc3339();
                    let record = if path_is_regular_file(&path)? {
                        let mut existing: IssueSharedWorktree = read_json(&path)?;
                        existing.branch_name = input.branch_name;
                        existing.worktree_path = input.worktree_path;
                        existing.base_branch = input.base_branch;
                        existing.updated_at = now.clone();
                        existing
                    } else {
                        IssueSharedWorktree {
                            id: format!(
                                "issue_shared_worktree_{}_{}",
                                input.project_id, input.issue_id
                            ),
                            project_id: input.project_id,
                            issue_id: input.issue_id,
                            repository_id: input.repository_id,
                            target_repository_id: None,
                            checkout_id: None,
                            path_schema_version: 0,
                            branch_name: input.branch_name,
                            worktree_path: input.worktree_path,
                            base_branch: input.base_branch,
                            status: IssueSharedWorktreeStatus::Ready,
                            current_active_work_item_id: None,
                            current_lock_owner_id: None,
                            last_completed_work_item_id: None,
                            created_at: now.clone(),
                            updated_at: now,
                        }
                    };

                    write_json(&path, &record)?;
                    Ok(record)
                })
            },
        )
    }

    /// 删除 issue 级共享 worktree 的 json 与 lock 文件。
    ///
    /// lock 文件名与 `coding_attempt_store::locking::lock_path_for` 的命名约定一致
    /// （`.` + json 文件名 + `.lock`）。NotFound 视为成功：清理路径不应要求被清理对象预先存在。
    pub fn delete_issue_shared_worktree(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> Result<(), ProductStoreError> {
        let json_path = self.issue_shared_worktree_path(project_id, issue_id);
        let lock_path = json_path.with_file_name(".issue-shared-worktree.json.lock");
        remove_file_if_exists(&json_path)?;
        remove_file_if_exists(&lock_path)?;
        Ok(())
    }

    pub fn get_issue_shared_worktree(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> Result<Option<IssueSharedWorktree>, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;

        let path = self.issue_shared_worktree_path(project_id, issue_id);
        if !path_is_regular_file(&path)? {
            return Ok(None);
        }
        read_json(&path).map(Some)
    }

    pub fn try_acquire_issue_worktree_lock(
        &self,
        project_id: &str,
        issue_id: &str,
        work_item_id: &str,
        lease_id: &str,
    ) -> Result<IssueWorktreeLockLease, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(work_item_id)?;
        validate_relative_id(lease_id)?;

        self.with_shared_worktree_write_admission(
            project_id,
            issue_id,
            None,
            SharedWorktreeOwnerHints {
                work_item_ids: vec![work_item_id],
                owner_ids: vec![lease_id],
            },
            true,
            None,
            || {
                let path = self.issue_shared_worktree_path(project_id, issue_id);
                with_exclusive_lock(&path, || {
                    let mut record = read_issue_worktree(&path, project_id, issue_id)?;
                    if let Some(active_id) = &record.current_active_work_item_id {
                        if active_id != work_item_id {
                            return Err(ProductStoreError::Io(format!(
                                "issue_worktree_active: issue {issue_id} locked by {active_id}"
                            )));
                        }
                        if record.current_lock_owner_id.is_none() {
                            return Err(lock_owner_mismatch(issue_id, work_item_id));
                        }
                        return Ok(IssueWorktreeLockLease {
                            acquired: record.current_lock_owner_id.as_deref() == Some(lease_id),
                            worktree: record,
                            lease_id: lease_id.to_string(),
                        });
                    }

                    record.current_active_work_item_id = Some(work_item_id.to_string());
                    record.current_lock_owner_id = Some(lease_id.to_string());
                    record.status = IssueSharedWorktreeStatus::Running;
                    record.updated_at = Utc::now().to_rfc3339();
                    write_json(&path, &record)?;
                    Ok(IssueWorktreeLockLease {
                        worktree: record,
                        lease_id: lease_id.to_string(),
                        acquired: true,
                    })
                })
            },
        )
    }

    pub fn bind_issue_worktree_lock_to_attempt(
        &self,
        project_id: &str,
        issue_id: &str,
        work_item_id: &str,
        attempt_id: &str,
    ) -> Result<IssueSharedWorktree, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(work_item_id)?;
        validate_relative_id(attempt_id)?;
        self.with_shared_worktree_write_admission(
            project_id,
            issue_id,
            None,
            SharedWorktreeOwnerHints {
                work_item_ids: vec![work_item_id],
                owner_ids: vec![attempt_id],
            },
            false,
            None,
            || {
                let path = self.issue_shared_worktree_path(project_id, issue_id);
                with_exclusive_lock(&path, || {
                    let mut record = read_issue_worktree(&path, project_id, issue_id)?;
                    if record.current_active_work_item_id.as_deref() != Some(work_item_id) {
                        return Err(lock_owner_mismatch(issue_id, work_item_id));
                    }
                    match record.current_lock_owner_id.as_deref() {
                        Some(owner) if owner == attempt_id => return Ok(record),
                        Some(owner) if owner.starts_with("issue_worktree_lease_") => {}
                        _ => return Err(lock_owner_mismatch(issue_id, work_item_id)),
                    }
                    record.current_lock_owner_id = Some(attempt_id.to_string());
                    record.updated_at = Utc::now().to_rfc3339();
                    write_json(&path, &record)?;
                    Ok(record)
                })
            },
        )
    }

    pub fn validate_issue_worktree_lock_owner(
        &self,
        project_id: &str,
        issue_id: &str,
        work_item_id: &str,
        owner_id: &str,
    ) -> Result<IssueSharedWorktree, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(work_item_id)?;
        validate_relative_id(owner_id)?;
        let path = self.issue_shared_worktree_path(project_id, issue_id);
        with_exclusive_lock(&path, || {
            let record = read_issue_worktree(&path, project_id, issue_id)?;
            if record.current_active_work_item_id.as_deref() == Some(work_item_id)
                && record.current_lock_owner_id.as_deref() == Some(owner_id)
            {
                Ok(record)
            } else {
                Err(lock_owner_mismatch(issue_id, work_item_id))
            }
        })
    }

    pub fn transfer_issue_worktree_lock(
        &self,
        project_id: &str,
        issue_id: &str,
        current_work_item_id: &str,
        next_work_item_id: &str,
        owner_id: &str,
    ) -> Result<IssueSharedWorktree, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(current_work_item_id)?;
        validate_relative_id(next_work_item_id)?;
        validate_relative_id(owner_id)?;

        self.with_shared_worktree_write_admission(
            project_id,
            issue_id,
            None,
            SharedWorktreeOwnerHints {
                work_item_ids: vec![current_work_item_id, next_work_item_id],
                owner_ids: vec![owner_id],
            },
            false,
            None,
            || {
                let path = self.issue_shared_worktree_path(project_id, issue_id);
                with_exclusive_lock(&path, || {
                    let mut record = read_issue_worktree(&path, project_id, issue_id)?;
                    match record.current_active_work_item_id.as_deref() {
                        Some(active_id)
                            if active_id == current_work_item_id
                                || active_id == next_work_item_id => {}
                        Some(active_id) => {
                            return Err(ProductStoreError::Io(format!(
                                "issue_worktree_active: issue {issue_id} locked by {active_id}"
                            )));
                        }
                        None => return Err(lock_owner_mismatch(issue_id, current_work_item_id)),
                    }
                    if record.current_lock_owner_id.as_deref() != Some(owner_id) {
                        return Err(lock_owner_mismatch(issue_id, current_work_item_id));
                    }

                    record.current_active_work_item_id = Some(next_work_item_id.to_string());
                    record.status = IssueSharedWorktreeStatus::Running;
                    record.updated_at = Utc::now().to_rfc3339();
                    write_json(&path, &record)?;
                    Ok(record)
                })
            },
        )
    }

    pub fn release_issue_worktree_lock(
        &self,
        project_id: &str,
        issue_id: &str,
        work_item_id: &str,
        owner_id: &str,
    ) -> Result<IssueSharedWorktree, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(work_item_id)?;
        validate_relative_id(owner_id)?;

        self.with_shared_worktree_write_admission(
            project_id,
            issue_id,
            None,
            SharedWorktreeOwnerHints {
                work_item_ids: vec![work_item_id],
                owner_ids: vec![owner_id],
            },
            false,
            None,
            || {
                let path = self.issue_shared_worktree_path(project_id, issue_id);
                with_exclusive_lock(&path, || {
                    let mut record = read_issue_worktree(&path, project_id, issue_id)?;
                    match record.current_active_work_item_id.as_deref() {
                        None if record.current_lock_owner_id.is_none() => return Ok(record),
                        Some(active_id) if active_id == work_item_id => {}
                        _ => return Err(lock_owner_mismatch(issue_id, work_item_id)),
                    }
                    if record.current_lock_owner_id.as_deref() != Some(owner_id) {
                        return Err(lock_owner_mismatch(issue_id, work_item_id));
                    }
                    record.current_active_work_item_id = None;
                    record.current_lock_owner_id = None;
                    record.status = IssueSharedWorktreeStatus::Ready;
                    record.updated_at = Utc::now().to_rfc3339();
                    write_json(&path, &record)?;

                    Ok(record)
                })
            },
        )
    }

    /// 按 owner 释放锁：只要锁的 owner 是给定的 attempt 就释放，
    /// 不要求 active work item 匹配。
    ///
    /// 删除 attempt 时使用：failed attempt 的 `current_work_item_id` 可能已变或
    /// 已清空，导致按 work_item_id 匹配的 `release_issue_worktree_lock` 找不到
    /// 而留下孤儿锁；这里只认 owner，幂等释放。
    pub fn release_issue_worktree_lock_by_owner(
        &self,
        project_id: &str,
        issue_id: &str,
        owner_id: &str,
    ) -> Result<IssueSharedWorktree, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(owner_id)?;

        self.with_shared_worktree_write_admission(
            project_id,
            issue_id,
            None,
            SharedWorktreeOwnerHints {
                work_item_ids: Vec::new(),
                owner_ids: vec![owner_id],
            },
            false,
            None,
            || {
                let path = self.issue_shared_worktree_path(project_id, issue_id);
                with_exclusive_lock(&path, || {
                    let mut record = read_issue_worktree(&path, project_id, issue_id)?;
                    if record.current_lock_owner_id.as_deref() != Some(owner_id) {
                        // 锁不是这个 attempt 持有（可能已释放或属于他人）：幂等返回。
                        return Ok(record);
                    }
                    record.current_active_work_item_id = None;
                    record.current_lock_owner_id = None;
                    record.status = IssueSharedWorktreeStatus::Ready;
                    record.updated_at = Utc::now().to_rfc3339();
                    write_json(&path, &record)?;

                    Ok(record)
                })
            },
        )
    }

    pub fn mark_issue_worktree_completed_item(
        &self,
        project_id: &str,
        issue_id: &str,
        work_item_id: &str,
        owner_id: &str,
    ) -> Result<IssueSharedWorktree, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(work_item_id)?;
        validate_relative_id(owner_id)?;

        self.with_shared_worktree_write_admission(
            project_id,
            issue_id,
            None,
            SharedWorktreeOwnerHints {
                work_item_ids: vec![work_item_id],
                owner_ids: vec![owner_id],
            },
            false,
            None,
            || {
                let path = self.issue_shared_worktree_path(project_id, issue_id);
                with_exclusive_lock(&path, || {
                    let mut record = read_issue_worktree(&path, project_id, issue_id)?;
                    if record.current_active_work_item_id.is_some()
                        && record.current_lock_owner_id.as_deref() != Some(owner_id)
                    {
                        return Err(lock_owner_mismatch(issue_id, work_item_id));
                    }
                    record.last_completed_work_item_id = Some(work_item_id.to_string());
                    if record.current_active_work_item_id.as_deref() == Some(work_item_id) {
                        record.current_active_work_item_id = None;
                        record.current_lock_owner_id = None;
                        record.status = IssueSharedWorktreeStatus::Ready;
                    } else if record.current_active_work_item_id.is_none() {
                        record.status = IssueSharedWorktreeStatus::Ready;
                    }
                    record.updated_at = Utc::now().to_rfc3339();
                    write_json(&path, &record)?;
                    Ok(record)
                })
            },
        )
    }

    // ─── C1 Task 6（REQ-WIGA-03）：死亡租约确认接管 ───

    /// 用户显式确认后接管 issue 维死亡租约：在既有 worktree 文件锁内
    /// CAS（owner 必须仍是 `expected_owner_id`），原子清出死亡 owner 与
    /// active work item，使当前 binding 下的下一次合法 acquire 成为新
    /// owner。死亡证明（终态 attempt）由调用方应用服务先行验证；本方法
    /// 只做 owner CAS 与清出，不写 epoch/fence/incarnation。迟到旧 owner
    /// 写入统一 `IdentityMismatch`，current owner 不变。
    pub fn takeover_issue_worktree_lock_after_dead_owner(
        &self,
        project_id: &str,
        issue_id: &str,
        expected_owner_id: &str,
    ) -> Result<IssueSharedWorktree, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(expected_owner_id)?;

        self.with_shared_worktree_write_admission(
            project_id,
            issue_id,
            None,
            SharedWorktreeOwnerHints {
                work_item_ids: Vec::new(),
                owner_ids: vec![expected_owner_id],
            },
            false,
            None,
            || {
                let path = self.issue_shared_worktree_path(project_id, issue_id);
                with_exclusive_lock(&path, || {
                    let mut record = read_issue_worktree(&path, project_id, issue_id)?;
                    if record.current_lock_owner_id.as_deref() != Some(expected_owner_id) {
                        return Err(ProductStoreError::IdentityMismatch {
                            kind: "issue_worktree_lock_takeover",
                            id: format!(
                                "{project_id}/{issue_id}: expected owner {expected_owner_id}, got {:?}",
                                record.current_lock_owner_id
                            ),
                        });
                    }
                    record.current_active_work_item_id = None;
                    record.current_lock_owner_id = None;
                    record.status = IssueSharedWorktreeStatus::Ready;
                    record.updated_at = Utc::now().to_rfc3339();
                    write_json(&path, &record)?;
                    Ok(record)
                })
            },
        )
    }
}
