use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductAppPaths {
    root: PathBuf,
}

impl ProductAppPaths {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn product_data_schema_path(&self) -> PathBuf {
        self.root.join("schema.json")
    }

    pub fn projects_root(&self) -> PathBuf {
        self.root.join("projects")
    }

    pub fn state_root(&self) -> PathBuf {
        self.root.join("state")
    }

    pub fn last_project_path(&self) -> PathBuf {
        self.state_root().join("last-project.json")
    }

    pub fn project_root(&self, project_id: &str) -> PathBuf {
        self.projects_root().join(project_id)
    }

    pub fn issue_root(&self, project_id: &str, issue_id: &str) -> PathBuf {
        self.project_root(project_id).join("issues").join(issue_id)
    }

    pub fn issue_lifecycle_root(&self, project_id: &str, issue_id: &str) -> PathBuf {
        self.issue_root(project_id, issue_id)
    }

    pub fn project_provider_defaults_path(&self, project_id: &str) -> PathBuf {
        self.project_root(project_id).join("provider-defaults.json")
    }

    pub fn repository_initializations_root(&self, project_id: &str) -> PathBuf {
        self.project_root(project_id)
            .join("repository-initializations")
    }

    pub fn logical_codebase_root(&self, project_id: &str) -> PathBuf {
        self.project_root(project_id).join("logical-codebase")
    }

    pub fn registration_batches_root(&self, project_id: &str) -> PathBuf {
        self.logical_codebase_root(project_id)
            .join("registration-batches")
    }

    pub fn registration_batches_lock_path(&self, project_id: &str) -> PathBuf {
        self.logical_codebase_root(project_id)
            .join(".registration-batches.lock")
    }

    pub fn identity_migration_lock_path(&self, project_id: &str) -> PathBuf {
        self.logical_codebase_root(project_id)
            .join(".identity-migration.lock")
    }

    pub fn aggregate_indexes_root(&self, project_id: &str) -> PathBuf {
        self.logical_codebase_root(project_id)
            .join("aggregate-indexes")
    }

    pub fn aggregate_index_lock_path(&self, project_id: &str) -> PathBuf {
        self.logical_codebase_root(project_id)
            .join(".aggregate-index.lock")
    }
}

#[cfg(test)]
mod tests {
    use super::ProductAppPaths;

    #[test]
    fn logical_codebase_paths_are_project_scoped() {
        let paths = ProductAppPaths::new("/tmp/aria");
        assert_eq!(
            paths.logical_codebase_root("project_0001"),
            std::path::PathBuf::from("/tmp/aria/projects/project_0001/logical-codebase")
        );
        assert_eq!(
            paths.registration_batches_root("project_0001"),
            std::path::PathBuf::from(
                "/tmp/aria/projects/project_0001/logical-codebase/registration-batches"
            )
        );
        assert_eq!(
            paths.registration_batches_lock_path("project_0001"),
            std::path::PathBuf::from(
                "/tmp/aria/projects/project_0001/logical-codebase/.registration-batches.lock"
            )
        );
        assert_eq!(
            paths.identity_migration_lock_path("project_0001"),
            std::path::PathBuf::from(
                "/tmp/aria/projects/project_0001/logical-codebase/.identity-migration.lock"
            )
        );
    }
}
