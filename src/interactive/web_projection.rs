use std::fs;
use std::path::Path;
use std::process::Command;

use serde_json::json;

use crate::interactive::models::{
    GitSummary, PendingProviderStepProjection, SelectedNodeContext, WebWorkspaceProjection,
    WorkspaceProjection,
};
use crate::task_run::types::TaskRunError;

pub fn build_web_projection(
    workspace_root: &Path,
    base: WorkspaceProjection,
    selected_node_id: Option<&str>,
) -> Result<WebWorkspaceProjection, TaskRunError> {
    let task_id = base.active_task_id.clone();
    let pending_provider_step = task_id
        .as_deref()
        .and_then(|task_id| read_pending_step(workspace_root, task_id).transpose())
        .transpose()?;
    let mut available_actions = base.available_actions.clone();
    if pending_provider_step.is_some() {
        available_actions.push("confirm_provider_step".to_string());
        available_actions.push("rollback_pending_checkpoint".to_string());
    }
    Ok(WebWorkspaceProjection {
        workspace_root: base.workspace_root,
        active_task_id: base.active_task_id,
        active_session_id: base.active_session_id,
        overview: base.overview,
        sessions: base.sessions,
        timeline: base.timeline,
        artifact_index: base.artifact_index,
        diagnostics: base.diagnostics,
        available_actions,
        pending_provider_step,
        selected_node_context: selected_node_context(selected_node_id),
        git_summary: git_summary(workspace_root),
        event_cursor: 0,
    })
}

fn read_pending_step(
    workspace_root: &Path,
    task_id: &str,
) -> Result<Option<PendingProviderStepProjection>, TaskRunError> {
    let path = workspace_root
        .join(".aria/runtime/tasks")
        .join(task_id)
        .join("pending/provider-step.json");
    match fs::File::open(&path) {
        Ok(file) => serde_json::from_reader(file).map(Some).map_err(|error| {
            TaskRunError::new(
                "interactive_projection_json",
                format!("parse {}: {error}", path.display()),
            )
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(TaskRunError::new(
            "interactive_projection_io",
            format!("open {}: {error}", path.display()),
        )),
    }
}

fn selected_node_context(selected_node_id: Option<&str>) -> SelectedNodeContext {
    SelectedNodeContext {
        node_id: selected_node_id.map(str::to_string),
        overview: json!({}),
        inputs: Vec::new(),
        run: Vec::new(),
        outputs: Vec::new(),
        diffs: Vec::new(),
    }
}

fn git_summary(workspace_root: &Path) -> GitSummary {
    GitSummary {
        workspace_path: workspace_root.to_string_lossy().to_string(),
        branch: git_stdout(workspace_root, &["branch", "--show-current"]),
        head: git_stdout(workspace_root, &["rev-parse", "--short", "HEAD"]),
        dirty: git_stdout(workspace_root, &["status", "--porcelain"])
            .is_some_and(|text| !text.trim().is_empty()),
        dirty_files: Vec::new(),
    }
}

fn git_stdout(workspace_root: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(workspace_root)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}
