use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use thiserror::Error;

#[derive(Debug, Error)]
#[error("{code}: {message}")]
pub struct WorkspaceRegistryError {
    code: &'static str,
    message: String,
}

impl WorkspaceRegistryError {
    pub fn code(&self) -> &'static str {
        self.code
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkspaceRecord {
    pub workspace_id: String,
    pub name: String,
    pub path: PathBuf,
    pub default_policy_preset: String,
    pub default_provider_mode: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CreateWorkspaceInput {
    pub name: String,
    pub path: PathBuf,
    pub default_policy_preset: Option<String>,
    pub default_provider_mode: Option<String>,
}

#[derive(Debug, Clone)]
pub struct WorkspaceRegistry {
    app_root: PathBuf,
}

impl WorkspaceRegistry {
    pub fn new(app_root: PathBuf) -> Self {
        Self { app_root }
    }

    pub fn list(&self) -> Result<Vec<WorkspaceRecord>, WorkspaceRegistryError> {
        let path = self.path();
        if !path.exists() {
            return Ok(Vec::new());
        }
        let file = fs::File::open(path).map_err(io_error)?;
        serde_json::from_reader(file).map_err(json_error)
    }

    pub fn create(
        &self,
        input: CreateWorkspaceInput,
    ) -> Result<WorkspaceRecord, WorkspaceRegistryError> {
        let canonical_path = validate_workspace_path(&input.path)?;
        let mut records = self.list()?;
        let now = Utc::now();
        let record = WorkspaceRecord {
            workspace_id: format!("workspace_{:04}", records.len() + 1),
            name: input.name.trim().to_string(),
            path: canonical_path,
            default_policy_preset: input
                .default_policy_preset
                .unwrap_or_else(|| "manual-write".to_string()),
            default_provider_mode: input
                .default_provider_mode
                .unwrap_or_else(|| "fake".to_string()),
            created_at: now,
            updated_at: now,
        };
        records.push(record.clone());
        self.write(&records)?;
        Ok(record)
    }

    pub fn get(&self, workspace_id: &str) -> Result<WorkspaceRecord, WorkspaceRegistryError> {
        self.list()?
            .into_iter()
            .find(|record| record.workspace_id == workspace_id)
            .ok_or_else(|| registry_error("workspace_not_found", workspace_id))
    }

    pub fn ensure_default_workspace(&self) -> Result<Vec<WorkspaceRecord>, WorkspaceRegistryError> {
        let existing = self.list()?;
        if !existing.is_empty() {
            return Ok(existing);
        }
        let name = self
            .app_root
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("workspace")
            .to_string();
        self.create(CreateWorkspaceInput {
            name,
            path: self.app_root.clone(),
            default_policy_preset: None,
            default_provider_mode: None,
        })?;
        self.list()
    }

    fn path(&self) -> PathBuf {
        self.app_root.join(".aria/runtime/web/workspaces.json")
    }

    fn write(&self, records: &[WorkspaceRecord]) -> Result<(), WorkspaceRegistryError> {
        let path = self.path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(io_error)?;
        }
        let file = fs::File::create(path).map_err(io_error)?;
        serde_json::to_writer_pretty(file, records).map_err(json_error)
    }
}

fn validate_workspace_path(path: &Path) -> Result<PathBuf, WorkspaceRegistryError> {
    if !path.exists() {
        return Err(registry_error(
            "workspace_path_missing",
            &path.display().to_string(),
        ));
    }
    if !path.is_dir() {
        return Err(registry_error(
            "workspace_path_not_directory",
            &path.display().to_string(),
        ));
    }
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(path)
        .output()
        .map_err(io_error)?;
    if !output.status.success() {
        return Err(registry_error(
            "workspace_path_not_git_repo",
            &path.display().to_string(),
        ));
    }
    Ok(PathBuf::from(String::from_utf8_lossy(&output.stdout).trim()))
}

fn registry_error(code: &'static str, value: &str) -> WorkspaceRegistryError {
    WorkspaceRegistryError {
        code,
        message: value.to_string(),
    }
}

fn io_error(error: std::io::Error) -> WorkspaceRegistryError {
    WorkspaceRegistryError {
        code: "workspace_registry_io",
        message: error.to_string(),
    }
}

fn json_error(error: serde_json::Error) -> WorkspaceRegistryError {
    WorkspaceRegistryError {
        code: "workspace_registry_json",
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command;
    use tempfile::tempdir;

    fn git_repo() -> tempfile::TempDir {
        let dir = tempdir().expect("tempdir");
        let status = Command::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .status()
            .expect("git init");
        assert!(status.success());
        dir
    }

    #[test]
    fn creates_lists_and_persists_workspaces() {
        let app = tempdir().expect("tempdir");
        let repo = git_repo();
        let registry = WorkspaceRegistry::new(app.path().to_path_buf());

        let created = registry
            .create(CreateWorkspaceInput {
                name: "Aria".to_string(),
                path: repo.path().to_path_buf(),
                default_policy_preset: None,
                default_provider_mode: None,
            })
            .expect("create workspace");

        assert_eq!(created.workspace_id, "workspace_0001");
        assert_eq!(created.name, "Aria");
        assert_eq!(created.default_policy_preset, "manual-write");
        assert_eq!(created.default_provider_mode, "fake");

        let reloaded = WorkspaceRegistry::new(app.path().to_path_buf())
            .list()
            .expect("list workspaces");
        assert_eq!(reloaded, vec![created]);
    }

    #[test]
    fn rejects_non_git_workspace_path() {
        let app = tempdir().expect("tempdir");
        let not_git = tempdir().expect("tempdir");
        fs::write(not_git.path().join("README.md"), "not a repo").expect("write file");
        let registry = WorkspaceRegistry::new(app.path().to_path_buf());

        let error = registry
            .create(CreateWorkspaceInput {
                name: "Not Git".to_string(),
                path: not_git.path().to_path_buf(),
                default_policy_preset: None,
                default_provider_mode: None,
            })
            .expect_err("non git repo should fail");

        assert_eq!(error.code(), "workspace_path_not_git_repo");
    }

    #[test]
    fn bootstraps_app_root_as_default_workspace_when_git_repo() {
        let app = git_repo();
        let registry = WorkspaceRegistry::new(app.path().to_path_buf());

        let workspaces = registry
            .ensure_default_workspace()
            .expect("default workspace");
        let expected_path = app.path().canonicalize().expect("canonical app root");

        assert_eq!(workspaces.len(), 1);
        assert_eq!(workspaces[0].workspace_id, "workspace_0001");
        assert_eq!(workspaces[0].path, expected_path);
    }
}
