use std::fs;
use std::process::Command;

use crate::interactive::models::WebWorkspaceProjection;
use crate::interactive::projection::build_workspace_projection;
use crate::interactive::web_projection::build_web_projection;
use crate::task_run::types::TaskRunError;
use crate::web::types::{ArtifactContentResponse, FileContentResponse, FileDiffResponse};

use super::WebRuntime;
use super::utils::{content_type_for_path, io_error, safe_workspace_path};

impl WebRuntime {
    pub fn projection(
        &self,
        task_id: Option<&str>,
        selected_node_id: Option<&str>,
    ) -> Result<WebWorkspaceProjection, TaskRunError> {
        Self::projection_for_workspace(&self.workspace_root, task_id, selected_node_id)
    }

    pub fn projection_for_workspace(
        workspace_root: &std::path::Path,
        task_id: Option<&str>,
        selected_node_id: Option<&str>,
    ) -> Result<WebWorkspaceProjection, TaskRunError> {
        let base = build_workspace_projection(workspace_root, task_id)?;
        build_web_projection(workspace_root, base, selected_node_id)
    }

    pub fn artifact_content(
        &self,
        artifact_ref: &str,
    ) -> Result<ArtifactContentResponse, TaskRunError> {
        let projection = self.projection(None, None)?;
        let entry = projection
            .artifact_index
            .iter()
            .find(|entry| entry.artifact_ref == artifact_ref)
            .ok_or_else(|| {
                TaskRunError::new(
                    "artifact_not_found",
                    format!("artifact not found: {artifact_ref}"),
                )
            })?;
        let path = self.workspace_root.join(&entry.path);
        let content = fs::read_to_string(path).map_err(io_error)?;
        Ok(ArtifactContentResponse {
            artifact_ref: entry.artifact_ref.clone(),
            artifact_kind: entry.artifact_kind.clone(),
            producer_node: entry.producer_node.clone(),
            path: entry.path.clone(),
            content_type: format!("{:?}", entry.content_type).to_lowercase(),
            content,
        })
    }

    pub fn file_content(&self, path: &str) -> Result<FileContentResponse, TaskRunError> {
        let safe = safe_workspace_path(&self.workspace_root, path)?;
        Ok(FileContentResponse {
            path: path.to_string(),
            content_type: content_type_for_path(path),
            content: fs::read_to_string(safe).map_err(io_error)?,
        })
    }

    pub fn file_diff(
        &self,
        base_checkpoint: &str,
        path: &str,
    ) -> Result<FileDiffResponse, TaskRunError> {
        let _ = safe_workspace_path(&self.workspace_root, path)?;
        let diff = Command::new("git")
            .args(["diff", base_checkpoint, "--", path])
            .current_dir(&self.workspace_root)
            .output()
            .map_err(|error| TaskRunError::new("git_command_failed", error.to_string()))?;
        Ok(FileDiffResponse {
            base_checkpoint: base_checkpoint.to_string(),
            path: path.to_string(),
            diff: String::from_utf8_lossy(&diff.stdout).to_string(),
        })
    }
}
