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
}
