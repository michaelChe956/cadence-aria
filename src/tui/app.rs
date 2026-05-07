use std::path::Path;

use crate::interactive::projection::build_workspace_projection;
use crate::task_run::types::TaskRunError;

pub fn check_tui_browse(workspace: &Path, task_id: Option<&str>) -> Result<(), TaskRunError> {
    let projection = build_workspace_projection(workspace, task_id)?;
    let Some(active_task_id) = projection.active_task_id.as_deref() else {
        return Err(TaskRunError::new(
            "interactive_task_missing",
            "no active task",
        ));
    };
    let task_root = workspace.join(".aria/runtime/tasks").join(active_task_id);
    if !task_root.exists() {
        return Err(TaskRunError::new(
            "interactive_task_missing",
            format!("task does not exist: {active_task_id}"),
        ));
    }
    Ok(())
}
