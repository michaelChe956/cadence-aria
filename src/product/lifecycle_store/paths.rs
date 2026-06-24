use std::path::PathBuf;

use crate::product::json_store::{ProductStoreError, validate_relative_id};

use super::LifecycleStore;

impl LifecycleStore {
    pub(crate) fn work_item_path(
        &self,
        project_id: &str,
        issue_id: &str,
        work_item_id: &str,
    ) -> Result<PathBuf, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(work_item_id)?;
        let path = self
            .work_items_root(project_id, issue_id)
            .join(format!("{work_item_id}.json"));
        if !super::path_is_regular_file(&path)? {
            return Err(ProductStoreError::NotFound {
                kind: "work_item",
                id: work_item_id.to_string(),
            });
        }
        Ok(path)
    }

    pub(crate) fn story_specs_root(&self, project_id: &str, issue_id: &str) -> PathBuf {
        self.paths
            .issue_lifecycle_root(project_id, issue_id)
            .join("story-specs")
    }

    pub(crate) fn design_specs_root(&self, project_id: &str, issue_id: &str) -> PathBuf {
        self.paths
            .issue_lifecycle_root(project_id, issue_id)
            .join("design-specs")
    }

    pub(crate) fn work_items_root(&self, project_id: &str, issue_id: &str) -> PathBuf {
        self.paths
            .issue_lifecycle_root(project_id, issue_id)
            .join("work-items")
    }

    pub(crate) fn versions_root(
        &self,
        project_id: &str,
        issue_id: &str,
        entity_id: &str,
    ) -> PathBuf {
        self.paths
            .issue_lifecycle_root(project_id, issue_id)
            .join("versions")
            .join(entity_id)
    }

    pub(crate) fn workspace_sessions_root(&self, project_id: &str, issue_id: &str) -> PathBuf {
        self.paths
            .issue_lifecycle_root(project_id, issue_id)
            .join("workspace-sessions")
    }

    pub(crate) fn provider_review_rounds_root(&self, project_id: &str, issue_id: &str) -> PathBuf {
        self.paths
            .issue_lifecycle_root(project_id, issue_id)
            .join("provider-review-rounds")
    }

    pub(crate) fn issue_shared_worktree_path(&self, project_id: &str, issue_id: &str) -> PathBuf {
        self.paths
            .issue_lifecycle_root(project_id, issue_id)
            .join("issue-shared-worktree.json")
    }

    pub(crate) fn issue_work_item_plans_root(&self, project_id: &str, issue_id: &str) -> PathBuf {
        self.paths
            .issue_lifecycle_root(project_id, issue_id)
            .join("issue-work-item-plans")
    }

    pub(crate) fn repository_profiles_root(&self, project_id: &str, issue_id: &str) -> PathBuf {
        self.paths
            .issue_lifecycle_root(project_id, issue_id)
            .join("repository-profiles")
    }

    pub(crate) fn verification_plans_root(&self, project_id: &str, issue_id: &str) -> PathBuf {
        self.paths
            .issue_lifecycle_root(project_id, issue_id)
            .join("verification-plans")
    }

    pub(crate) fn provider_runs_root(&self, project_id: &str, issue_id: &str) -> PathBuf {
        self.paths
            .issue_lifecycle_root(project_id, issue_id)
            .join("provider-runs")
    }
}
