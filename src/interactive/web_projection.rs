use std::fs;
use std::path::Path;
use std::process::Command;

use serde_json::Value;
use serde_json::json;

use crate::interactive::models::{
    ArtifactIndexEntry, GitSummary, PendingProviderStepProjection, SelectedNodeContext,
    WebWorkspaceProjection, WorkspaceProjection,
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
    let task_root = task_id
        .as_deref()
        .map(|task_id| workspace_root.join(".aria/runtime/tasks").join(task_id));
    let selected_node_context = match task_root.as_deref() {
        Some(task_root) => selected_node_context(
            task_root,
            selected_node_id,
            &base.artifact_index,
            &base.timeline,
            pending_provider_step.as_ref(),
        )?,
        None => empty_selected_node_context(selected_node_id),
    };
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
        selected_node_context,
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

fn selected_node_context(
    task_root: &Path,
    selected_node_id: Option<&str>,
    artifact_index: &[ArtifactIndexEntry],
    timeline: &[Value],
    pending: Option<&PendingProviderStepProjection>,
) -> Result<SelectedNodeContext, TaskRunError> {
    let overview = selected_node_id
        .map(|node_id| {
            json!({
                "node_id": node_id,
                "provider_type": pending
                    .filter(|step| step.node_id == node_id)
                    .map(|step| step.provider_type.clone()),
                "status": latest_node_status(timeline, node_id),
                "duration_ms": latest_node_duration(timeline, node_id),
                "failure_route": latest_node_failure_route(timeline, node_id),
                "completion_criteria": latest_node_completion_criteria(timeline, node_id)
            })
        })
        .unwrap_or_else(|| json!({}));
    let inputs = selected_node_id
        .and_then(|node_id| pending.filter(|step| step.node_id == node_id))
        .map(|step| {
            vec![
                json!({"kind":"prompt_snapshot","prompt":step.prompt}),
                json!({"kind":"input_summary","value":step.input_summary}),
                json!({"kind":"canonical_input_refs","value":step.canonical_input_refs}),
                json!({"kind":"context_files","value":step.context_files}),
                json!({"kind":"allowed_write_scope","value":step.allowed_write_scope}),
                json!({"kind":"output_schema","value":step.output_schema}),
            ]
        })
        .unwrap_or_default();
    let run = read_provider_output(task_root, selected_node_id)?;
    let outputs = selected_node_id
        .map(|node_id| {
            artifact_index
                .iter()
                .filter(|entry| entry.producer_node.as_deref() == Some(node_id))
                .map(serde_json::to_value)
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()
        .map_err(|error| TaskRunError::new("interactive_projection_json", error.to_string()))?
        .unwrap_or_default();
    let diffs = changed_file_refs(timeline, selected_node_id);

    Ok(SelectedNodeContext {
        node_id: selected_node_id.map(str::to_string),
        overview,
        inputs,
        run,
        outputs,
        diffs,
    })
}

fn empty_selected_node_context(selected_node_id: Option<&str>) -> SelectedNodeContext {
    SelectedNodeContext {
        node_id: selected_node_id.map(str::to_string),
        overview: json!({}),
        inputs: Vec::new(),
        run: Vec::new(),
        outputs: Vec::new(),
        diffs: Vec::new(),
    }
}

fn read_provider_output(
    task_root: &Path,
    selected_node_id: Option<&str>,
) -> Result<Vec<Value>, TaskRunError> {
    let path = task_root.join("logs/provider-output.jsonl");
    let file = match fs::File::open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(TaskRunError::new(
                "interactive_projection_io",
                format!("open {}: {error}", path.display()),
            ));
        }
    };
    let mut items = Vec::new();
    for (index, line) in std::io::BufRead::lines(std::io::BufReader::new(file)).enumerate() {
        let line = line.map_err(|error| {
            TaskRunError::new(
                "interactive_projection_io",
                format!("read {} line {}: {error}", path.display(), index + 1),
            )
        })?;
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = serde_json::from_str(&line).map_err(|error| {
            TaskRunError::new(
                "interactive_projection_jsonl",
                format!("parse {} line {}: {error}", path.display(), index + 1),
            )
        })?;
        if selected_node_id.is_none()
            || value.get("node_id").and_then(Value::as_str) == selected_node_id
        {
            items.push(value);
        }
    }
    Ok(items)
}

fn latest_node_status(timeline: &[Value], node_id: &str) -> Option<String> {
    timeline
        .iter()
        .rev()
        .find(|item| item.get("node_id").and_then(Value::as_str) == Some(node_id))
        .and_then(|item| item.get("status").and_then(Value::as_str))
        .map(str::to_string)
}

fn latest_node_duration(timeline: &[Value], node_id: &str) -> Option<u64> {
    timeline
        .iter()
        .rev()
        .find(|item| item.get("node_id").and_then(Value::as_str) == Some(node_id))
        .and_then(|item| item.get("duration_ms").and_then(Value::as_u64))
}

fn latest_node_failure_route(timeline: &[Value], node_id: &str) -> Option<String> {
    timeline
        .iter()
        .rev()
        .find(|item| item.get("node_id").and_then(Value::as_str) == Some(node_id))
        .and_then(|item| item.get("failure_route").and_then(Value::as_str))
        .map(str::to_string)
}

fn latest_node_completion_criteria(timeline: &[Value], node_id: &str) -> Option<Value> {
    timeline
        .iter()
        .rev()
        .find(|item| item.get("node_id").and_then(Value::as_str) == Some(node_id))
        .and_then(|item| item.get("completion_criteria").cloned())
}

fn changed_file_refs(timeline: &[Value], selected_node_id: Option<&str>) -> Vec<Value> {
    timeline
        .iter()
        .filter(|item| {
            selected_node_id.is_none()
                || item.get("node_id").and_then(Value::as_str) == selected_node_id
        })
        .flat_map(|item| {
            item.get("changed_files")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        })
        .filter_map(|path| path.as_str().map(|path| json!({"path": path})))
        .collect()
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
