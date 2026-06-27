use chrono::Utc;

use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
use crate::product::models::{IssueSharedWorktree, IssueSharedWorktreeStatus};

use super::{LifecycleStore, UpsertIssueSharedWorktreeInput, path_is_regular_file};

impl LifecycleStore {
    pub fn upsert_issue_shared_worktree(
        &self,
        input: UpsertIssueSharedWorktreeInput,
    ) -> Result<IssueSharedWorktree, ProductStoreError> {
        validate_relative_id(&input.project_id)?;
        validate_relative_id(&input.issue_id)?;
        validate_relative_id(&input.repository_id)?;

        let path = self.issue_shared_worktree_path(&input.project_id, &input.issue_id);
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
                branch_name: input.branch_name,
                worktree_path: input.worktree_path,
                base_branch: input.base_branch,
                status: IssueSharedWorktreeStatus::Ready,
                current_active_work_item_id: None,
                last_completed_work_item_id: None,
                created_at: now.clone(),
                updated_at: now,
            }
        };

        write_json(&path, &record)?;
        Ok(record)
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
    ) -> Result<IssueSharedWorktree, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(work_item_id)?;

        let path = self.issue_shared_worktree_path(project_id, issue_id);
        let mut record: IssueSharedWorktree = read_json(&path).map_err(|error| match error {
            ProductStoreError::NotFound { .. } => ProductStoreError::NotFound {
                kind: "issue_shared_worktree",
                id: format!("{project_id}/{issue_id}"),
            },
            other => other,
        })?;

        if let Some(active_id) = &record.current_active_work_item_id {
            if active_id != work_item_id {
                return Err(ProductStoreError::Io(format!(
                    "issue_worktree_active: issue {issue_id} locked by {active_id}"
                )));
            }
            return Ok(record);
        }

        record.current_active_work_item_id = Some(work_item_id.to_string());
        record.status = IssueSharedWorktreeStatus::Running;
        record.updated_at = Utc::now().to_rfc3339();
        write_json(&path, &record)?;
        Ok(record)
    }

    pub fn transfer_issue_worktree_lock(
        &self,
        project_id: &str,
        issue_id: &str,
        current_work_item_id: &str,
        next_work_item_id: &str,
    ) -> Result<IssueSharedWorktree, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(current_work_item_id)?;
        validate_relative_id(next_work_item_id)?;

        let path = self.issue_shared_worktree_path(project_id, issue_id);
        let mut record: IssueSharedWorktree = read_json(&path).map_err(|error| match error {
            ProductStoreError::NotFound { .. } => ProductStoreError::NotFound {
                kind: "issue_shared_worktree",
                id: format!("{project_id}/{issue_id}"),
            },
            other => other,
        })?;

        match record.current_active_work_item_id.as_deref() {
            Some(active_id)
                if active_id == current_work_item_id || active_id == next_work_item_id => {}
            None => {}
            Some(active_id) => {
                return Err(ProductStoreError::Io(format!(
                    "issue_worktree_active: issue {issue_id} locked by {active_id}"
                )));
            }
        }

        record.current_active_work_item_id = Some(next_work_item_id.to_string());
        record.status = IssueSharedWorktreeStatus::Running;
        record.updated_at = Utc::now().to_rfc3339();
        write_json(&path, &record)?;
        Ok(record)
    }

    pub fn release_issue_worktree_lock(
        &self,
        project_id: &str,
        issue_id: &str,
        work_item_id: &str,
    ) -> Result<IssueSharedWorktree, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(work_item_id)?;

        let path = self.issue_shared_worktree_path(project_id, issue_id);
        let mut record: IssueSharedWorktree = read_json(&path).map_err(|error| match error {
            ProductStoreError::NotFound { .. } => ProductStoreError::NotFound {
                kind: "issue_shared_worktree",
                id: format!("{project_id}/{issue_id}"),
            },
            other => other,
        })?;

        if record.current_active_work_item_id.as_deref() == Some(work_item_id) {
            record.current_active_work_item_id = None;
            record.status = IssueSharedWorktreeStatus::Ready;
            record.updated_at = Utc::now().to_rfc3339();
            write_json(&path, &record)?;
        }

        Ok(record)
    }

    pub fn mark_issue_worktree_completed_item(
        &self,
        project_id: &str,
        issue_id: &str,
        work_item_id: &str,
    ) -> Result<IssueSharedWorktree, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(work_item_id)?;

        let path = self.issue_shared_worktree_path(project_id, issue_id);
        let mut record: IssueSharedWorktree = read_json(&path).map_err(|error| match error {
            ProductStoreError::NotFound { .. } => ProductStoreError::NotFound {
                kind: "issue_shared_worktree",
                id: format!("{project_id}/{issue_id}"),
            },
            other => other,
        })?;

        record.last_completed_work_item_id = Some(work_item_id.to_string());
        if record.current_active_work_item_id.as_deref() == Some(work_item_id) {
            record.current_active_work_item_id = None;
        }
        record.status = IssueSharedWorktreeStatus::Ready;
        record.updated_at = Utc::now().to_rfc3339();
        write_json(&path, &record)?;
        Ok(record)
    }
}
