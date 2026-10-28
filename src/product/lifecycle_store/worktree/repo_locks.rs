use super::{
    RepoWorktreeLockLease, SharedWorktreeOwnerHints, read_repo_worktree, repo_lock_owner_mismatch,
};

use chrono::Utc;

use crate::product::coding_attempt_store::locking::with_exclusive_lock;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
use crate::product::lifecycle_store::{
    LifecycleStore, UpsertRepoSharedWorktreeInput, path_is_regular_file, remove_file_if_exists,
};
use crate::product::logical_codebase::LogicalRepositoryId;
use crate::product::models::{IssueSharedWorktree, IssueSharedWorktreeStatus};

impl LifecycleStore {
    // ───────────────────────────────────────────────────────────────
    // 多仓 shared worktree（REQ-COD-03）：三元键 (project, issue, logical repository)
    // 与单仓老方法并列，不替换。单仓 issue_shared_worktree 系列完全不变（红线）。
    // ───────────────────────────────────────────────────────────────

    /// 多仓 upsert：写 `shared-worktrees/{repository_id}.json`，填 `target_repository_id`。
    pub fn upsert_repo_shared_worktree(
        &self,
        input: UpsertRepoSharedWorktreeInput,
    ) -> Result<IssueSharedWorktree, ProductStoreError> {
        validate_relative_id(&input.project_id)?;
        validate_relative_id(&input.issue_id)?;

        let project_id = input.project_id.clone();
        let issue_id = input.issue_id.clone();
        self.with_shared_worktree_write_admission(
            &project_id,
            &issue_id,
            Some(input.repository_id),
            SharedWorktreeOwnerHints::empty(),
            false,
            None,
            move || {
                let path = self.repo_shared_worktree_path(
                    &input.project_id,
                    &input.issue_id,
                    input.repository_id,
                );
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
                                "repo_shared_worktree_{}_{}_{}",
                                input.project_id, input.issue_id, input.repository_id.0
                            ),
                            project_id: input.project_id,
                            issue_id: input.issue_id,
                            repository_id: input.repository_id.0.to_string(),
                            target_repository_id: Some(input.repository_id),
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

    /// 删除单个逻辑仓库的 shared worktree json 与 lock 文件。NotFound 视为成功。
    pub fn delete_repo_shared_worktree(
        &self,
        project_id: &str,
        issue_id: &str,
        repository_id: LogicalRepositoryId,
    ) -> Result<(), ProductStoreError> {
        let json_path = self.repo_shared_worktree_path(project_id, issue_id, repository_id);
        let lock_path = json_path.with_file_name(format!(".{}.json.lock", repository_id.0));
        remove_file_if_exists(&json_path)?;
        remove_file_if_exists(&lock_path)?;
        Ok(())
    }

    pub fn get_repo_shared_worktree(
        &self,
        project_id: &str,
        issue_id: &str,
        repository_id: LogicalRepositoryId,
    ) -> Result<Option<IssueSharedWorktree>, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;

        let path = self.repo_shared_worktree_path(project_id, issue_id, repository_id);
        if !path_is_regular_file(&path)? {
            return Ok(None);
        }
        read_json(&path).map(Some)
    }

    /// 枚举 `shared-worktrees/` 目录下全部仓维 worktree 的 `LogicalRepositoryId`。
    /// 供 issue 删除调用链（Task 11）枚举清理；目录不存在时返回空。
    pub fn list_repo_shared_worktrees(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> Result<Vec<LogicalRepositoryId>, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;

        let root = self
            .paths
            .issue_lifecycle_root(project_id, issue_id)
            .join("shared-worktrees");
        let entries = match std::fs::read_dir(&root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(ProductStoreError::Io(format!(
                    "read {}: {error}",
                    root.display()
                )));
            }
        };

        let mut ids = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| {
                ProductStoreError::Io(format!("read {} entry: {error}", root.display()))
            })?;
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let file_name = path
                .file_stem()
                .and_then(|value| value.to_str())
                .ok_or_else(|| ProductStoreError::InvalidRecord {
                    kind: "repo_shared_worktree",
                    reason: format!("record file name is not UTF-8: {}", path.display()),
                })?;
            let id = uuid::Uuid::parse_str(file_name).map_err(|error| {
                ProductStoreError::InvalidRecord {
                    kind: "repo_shared_worktree",
                    reason: format!("record file name is not a UUID: {file_name}: {error}"),
                }
            })?;
            if file_name != id.to_string() {
                return Err(ProductStoreError::InvalidRecord {
                    kind: "repo_shared_worktree",
                    reason: format!("record file name is not a canonical UUID: {file_name}"),
                });
            }
            ids.push(LogicalRepositoryId(id));
        }
        ids.sort();
        Ok(ids)
    }

    pub fn try_acquire_repo_worktree_lock(
        &self,
        project_id: &str,
        issue_id: &str,
        repository_id: LogicalRepositoryId,
        work_item_id: &str,
        lease_id: &str,
    ) -> Result<RepoWorktreeLockLease, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(work_item_id)?;
        validate_relative_id(lease_id)?;

        self.with_shared_worktree_write_admission(
            project_id,
            issue_id,
            Some(repository_id),
            SharedWorktreeOwnerHints {
                work_item_ids: vec![work_item_id],
                owner_ids: vec![lease_id],
            },
            true,
            None,
            || {
                let path = self.repo_shared_worktree_path(project_id, issue_id, repository_id);
                with_exclusive_lock(&path, || {
                    let mut record =
                        read_repo_worktree(&path, project_id, issue_id, repository_id)?;
                    if let Some(active_id) = &record.current_active_work_item_id {
                        if active_id != work_item_id {
                            return Err(ProductStoreError::Io(format!(
                                "repo_worktree_active: repo {} locked by {active_id}",
                                repository_id.0
                            )));
                        }
                        if record.current_lock_owner_id.is_none() {
                            return Err(repo_lock_owner_mismatch(repository_id, work_item_id));
                        }
                        return Ok(RepoWorktreeLockLease {
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
                    Ok(RepoWorktreeLockLease {
                        worktree: record,
                        lease_id: lease_id.to_string(),
                        acquired: true,
                    })
                })
            },
        )
    }

    pub fn bind_repo_worktree_lock_to_attempt(
        &self,
        project_id: &str,
        issue_id: &str,
        repository_id: LogicalRepositoryId,
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
            Some(repository_id),
            SharedWorktreeOwnerHints {
                work_item_ids: vec![work_item_id],
                owner_ids: vec![attempt_id],
            },
            false,
            None,
            || {
                let path = self.repo_shared_worktree_path(project_id, issue_id, repository_id);
                with_exclusive_lock(&path, || {
                    let mut record =
                        read_repo_worktree(&path, project_id, issue_id, repository_id)?;
                    if record.current_active_work_item_id.as_deref() != Some(work_item_id) {
                        return Err(repo_lock_owner_mismatch(repository_id, work_item_id));
                    }
                    match record.current_lock_owner_id.as_deref() {
                        Some(owner) if owner == attempt_id => return Ok(record),
                        // 租约前缀仅命名约定：仓维锁历史上可由组 journal 派生的
                        // issue_worktree_lease_（缺陷 #13 层2 分流修复前的存量）或
                        // repo_worktree_lease_ 持有；实质契约是「owner 为未绑定租约
                        // 方可 bind 到 attempt」，两类前缀均放行。
                        Some(owner)
                            if owner.starts_with("repo_worktree_lease_")
                                || owner.starts_with("issue_worktree_lease_") => {}
                        _ => return Err(repo_lock_owner_mismatch(repository_id, work_item_id)),
                    }
                    record.current_lock_owner_id = Some(attempt_id.to_string());
                    record.updated_at = Utc::now().to_rfc3339();
                    write_json(&path, &record)?;
                    Ok(record)
                })
            },
        )
    }

    pub fn validate_repo_worktree_lock_owner(
        &self,
        project_id: &str,
        issue_id: &str,
        repository_id: LogicalRepositoryId,
        work_item_id: &str,
        owner_id: &str,
    ) -> Result<IssueSharedWorktree, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(work_item_id)?;
        validate_relative_id(owner_id)?;
        let path = self.repo_shared_worktree_path(project_id, issue_id, repository_id);
        with_exclusive_lock(&path, || {
            let record = read_repo_worktree(&path, project_id, issue_id, repository_id)?;
            if record.current_active_work_item_id.as_deref() == Some(work_item_id)
                && record.current_lock_owner_id.as_deref() == Some(owner_id)
            {
                Ok(record)
            } else {
                Err(repo_lock_owner_mismatch(repository_id, work_item_id))
            }
        })
    }

    pub fn transfer_repo_worktree_lock(
        &self,
        project_id: &str,
        issue_id: &str,
        repository_id: LogicalRepositoryId,
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
            Some(repository_id),
            SharedWorktreeOwnerHints {
                work_item_ids: vec![current_work_item_id, next_work_item_id],
                owner_ids: vec![owner_id],
            },
            false,
            None,
            || {
                let path = self.repo_shared_worktree_path(project_id, issue_id, repository_id);
                with_exclusive_lock(&path, || {
                    let mut record =
                        read_repo_worktree(&path, project_id, issue_id, repository_id)?;
                    match record.current_active_work_item_id.as_deref() {
                        Some(active_id)
                            if active_id == current_work_item_id
                                || active_id == next_work_item_id => {}
                        Some(active_id) => {
                            return Err(ProductStoreError::Io(format!(
                                "repo_worktree_active: repo {} locked by {active_id}",
                                repository_id.0
                            )));
                        }
                        None => {
                            return Err(repo_lock_owner_mismatch(
                                repository_id,
                                current_work_item_id,
                            ));
                        }
                    }
                    if record.current_lock_owner_id.as_deref() != Some(owner_id) {
                        return Err(repo_lock_owner_mismatch(
                            repository_id,
                            current_work_item_id,
                        ));
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

    pub fn release_repo_worktree_lock(
        &self,
        project_id: &str,
        issue_id: &str,
        repository_id: LogicalRepositoryId,
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
            Some(repository_id),
            SharedWorktreeOwnerHints {
                work_item_ids: vec![work_item_id],
                owner_ids: vec![owner_id],
            },
            false,
            None,
            || {
                let path = self.repo_shared_worktree_path(project_id, issue_id, repository_id);
                with_exclusive_lock(&path, || {
                    let mut record =
                        read_repo_worktree(&path, project_id, issue_id, repository_id)?;
                    match record.current_active_work_item_id.as_deref() {
                        None if record.current_lock_owner_id.is_none() => return Ok(record),
                        Some(active_id) if active_id == work_item_id => {}
                        _ => return Err(repo_lock_owner_mismatch(repository_id, work_item_id)),
                    }
                    if record.current_lock_owner_id.as_deref() != Some(owner_id) {
                        return Err(repo_lock_owner_mismatch(repository_id, work_item_id));
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

    /// 按 owner 释放锁：只认 owner，幂等。删除 attempt 时使用。
    pub fn release_repo_worktree_lock_by_owner(
        &self,
        project_id: &str,
        issue_id: &str,
        repository_id: LogicalRepositoryId,
        owner_id: &str,
    ) -> Result<IssueSharedWorktree, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(owner_id)?;

        self.with_shared_worktree_write_admission(
            project_id,
            issue_id,
            Some(repository_id),
            SharedWorktreeOwnerHints {
                work_item_ids: Vec::new(),
                owner_ids: vec![owner_id],
            },
            false,
            None,
            || {
                let path = self.repo_shared_worktree_path(project_id, issue_id, repository_id);
                with_exclusive_lock(&path, || {
                    let mut record =
                        read_repo_worktree(&path, project_id, issue_id, repository_id)?;
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

    pub fn mark_repo_worktree_completed_item(
        &self,
        project_id: &str,
        issue_id: &str,
        repository_id: LogicalRepositoryId,
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
            Some(repository_id),
            SharedWorktreeOwnerHints {
                work_item_ids: vec![work_item_id],
                owner_ids: vec![owner_id],
            },
            false,
            None,
            || {
                let path = self.repo_shared_worktree_path(project_id, issue_id, repository_id);
                with_exclusive_lock(&path, || {
                    let mut record =
                        read_repo_worktree(&path, project_id, issue_id, repository_id)?;
                    if record.current_active_work_item_id.is_some()
                        && record.current_lock_owner_id.as_deref() != Some(owner_id)
                    {
                        return Err(repo_lock_owner_mismatch(repository_id, work_item_id));
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

    /// 仓维（多仓三元键）同款死亡租约接管 CAS；语义与 issue 维
    /// [`Self::takeover_issue_worktree_lock_after_dead_owner`] 一致。
    pub fn takeover_repo_worktree_lock_after_dead_owner(
        &self,
        project_id: &str,
        issue_id: &str,
        repository_id: LogicalRepositoryId,
        expected_owner_id: &str,
    ) -> Result<IssueSharedWorktree, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(expected_owner_id)?;
        self.with_shared_worktree_write_admission(
            project_id,
            issue_id,
            Some(repository_id),
            SharedWorktreeOwnerHints {
                work_item_ids: Vec::new(),
                owner_ids: vec![expected_owner_id],
            },
            false,
            None,
            || {
                let path = self.repo_shared_worktree_path(project_id, issue_id, repository_id);
                with_exclusive_lock(&path, || {
                    let mut record = read_repo_worktree(&path, project_id, issue_id, repository_id)?;
                    if record.current_lock_owner_id.as_deref() != Some(expected_owner_id) {
                        return Err(ProductStoreError::IdentityMismatch {
                            kind: "repo_worktree_lock_takeover",
                            id: format!(
                                "{project_id}/{issue_id}/{}: expected owner {expected_owner_id}, got {:?}",
                                repository_id.0, record.current_lock_owner_id
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
