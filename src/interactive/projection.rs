use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::interactive::diagnostics::classify_task_diagnostics;
use crate::interactive::models::{
    ArtifactIndexEntry, ArtifactStatus, ContentType, WorkspaceProjection,
};
use crate::task_run::types::TaskRunError;

pub fn build_workspace_projection(
    workspace_root: &Path,
    task_id: Option<&str>,
) -> Result<WorkspaceProjection, TaskRunError> {
    let task_id = match task_id {
        Some(task_id) => task_id.to_string(),
        None => latest_task_id(workspace_root)?,
    };
    validate_task_id(&task_id)?;
    let task_root = workspace_root.join(".aria/runtime/tasks").join(&task_id);
    let state = read_optional_json(&task_root.join("state.json"))?;
    let final_report = read_optional_json(&task_root.join("reports/final-report.json"))?;
    let mut timeline = read_timeline(&task_root.join("logs/node-events.jsonl"))?;
    timeline.extend(read_dropped_node_runs(&task_root.join("node-runs"))?);
    let diagnostics = classify_task_diagnostics(&task_root, &state)?;
    let change_id =
        string_field(&state, "change_id").or_else(|| string_field(&final_report, "change_id"));
    let artifact_index = build_artifact_index(workspace_root, &task_root, change_id.as_deref())?;

    let status = string_field(&final_report, "status").or_else(|| string_field(&state, "phase"));
    let e2e_overall = string_field(&final_report, "e2e_overall").or_else(|| status.clone());
    let overview = json!({
        "task_id": string_field(&state, "task_id").unwrap_or_else(|| task_id.clone()),
        "change_id": change_id,
        "phase": string_field(&state, "phase"),
        "current_worktask": string_field(&state, "current_worktask"),
        "status": status,
        "e2e_overall": e2e_overall,
        "business_code": string_field(&final_report, "business_code"),
        "unit_tests": string_field(&final_report, "unit_tests"),
        "coverage_gate": string_field(&final_report, "coverage_gate"),
        "archive_worktask": string_field(&final_report, "archive_worktask"),
        "root_cause": string_field(&final_report, "root_cause"),
        "workspace": workspace_root.to_string_lossy(),
    });

    Ok(WorkspaceProjection {
        workspace_root: workspace_root.to_string_lossy().to_string(),
        active_task_id: Some(task_id),
        active_session_id: None,
        overview,
        sessions: Vec::new(),
        timeline,
        artifact_index,
        diagnostics,
        available_actions: vec!["refresh".to_string()],
    })
}

fn latest_task_id(workspace_root: &Path) -> Result<String, TaskRunError> {
    let tasks_root = workspace_root.join(".aria/runtime/tasks");
    let entries = match fs::read_dir(&tasks_root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(TaskRunError::new(
                "interactive_task_missing",
                "no task id available",
            ));
        }
        Err(error) => {
            return Err(TaskRunError::new(
                "interactive_projection_io",
                format!("read {}: {error}", tasks_root.display()),
            ));
        }
    };

    let mut task_ids = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| {
            TaskRunError::new(
                "interactive_projection_io",
                format!("read {} entry: {error}", tasks_root.display()),
            )
        })?;
        let file_type = entry.file_type().map_err(|error| {
            TaskRunError::new(
                "interactive_projection_io",
                format!("stat {}: {error}", entry.path().display()),
            )
        })?;
        if file_type.is_dir() {
            task_ids.push(entry.file_name().to_string_lossy().to_string());
        }
    }
    task_ids.sort();
    task_ids
        .pop()
        .ok_or_else(|| TaskRunError::new("interactive_task_missing", "no task id available"))
}

fn read_json(path: &Path) -> Result<Value, TaskRunError> {
    let file = fs::File::open(path).map_err(|error| {
        TaskRunError::new(
            "interactive_projection_io",
            format!("open {}: {error}", path.display()),
        )
    })?;
    serde_json::from_reader(file).map_err(|error| {
        TaskRunError::new(
            "interactive_projection_json",
            format!("parse {}: {error}", path.display()),
        )
    })
}

fn read_optional_json(path: &Path) -> Result<Value, TaskRunError> {
    match fs::File::open(path) {
        Ok(file) => serde_json::from_reader(file).map_err(|error| {
            TaskRunError::new(
                "interactive_projection_json",
                format!("parse {}: {error}", path.display()),
            )
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(json!({})),
        Err(error) => Err(TaskRunError::new(
            "interactive_projection_io",
            format!("open {}: {error}", path.display()),
        )),
    }
}

fn read_timeline(path: &Path) -> Result<Vec<Value>, TaskRunError> {
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(TaskRunError::new(
                "interactive_projection_io",
                format!("open {}: {error}", path.display()),
            ));
        }
    };

    let mut timeline = Vec::new();
    for (index, line) in BufReader::new(file).lines().enumerate() {
        let line = line.map_err(|error| {
            TaskRunError::new(
                "interactive_projection_io",
                format!("read {} line {}: {error}", path.display(), index + 1),
            )
        })?;
        if line.trim().is_empty() {
            continue;
        }
        let event: Value = serde_json::from_str(&line).map_err(|error| {
            TaskRunError::new(
                "interactive_projection_jsonl",
                format!("parse {} line {}: {error}", path.display(), index + 1),
            )
        })?;
        let details = event.get("details").unwrap_or(&Value::Null);
        timeline.push(json!({
            "kind": "node",
            "event_kind": string_field(&event, "event_kind"),
            "task_id": string_field(&event, "task_id"),
            "node_id": string_field(&event, "node_id"),
            "status": string_field(&event, "status"),
            "provider_run_id": string_field(details, "provider_run_id"),
            "duration_ms": u64_field(details, "duration_ms"),
            "output_schema": string_field(details, "output_schema"),
            "changed_files": details.get("changed_files").cloned().unwrap_or_else(|| json!([])),
            "failure_route": string_field(details, "failure_route"),
            "completion_criteria": details.get("completion_criteria").cloned(),
        }));
    }
    Ok(timeline)
}

fn read_dropped_node_runs(root: &Path) -> Result<Vec<Value>, TaskRunError> {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(TaskRunError::new(
                "interactive_projection_io",
                format!("read {}: {error}", root.display()),
            ));
        }
    };

    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| {
            TaskRunError::new(
                "interactive_projection_io",
                format!("read {} entry: {error}", root.display()),
            )
        })?;
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) == Some("json") {
            paths.push(path);
        }
    }
    paths.sort();

    let mut dropped = Vec::new();
    for path in paths {
        let value = read_json(&path)?;
        if value.get("dropped").and_then(Value::as_bool) != Some(true) {
            continue;
        }
        let artifact_count = value
            .get("artifact_refs")
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        dropped.push(json!({
            "kind": "node",
            "event_kind": "dropped",
            "node_id": string_field(&value, "node_id"),
            "status": "dropped",
            "provider_run_id": string_field(&value, "provider_run_id"),
            "duration_ms": u64_field(&value, "duration_ms"),
            "output_schema": string_field(&value, "output_schema"),
            "changed_files": [],
            "artifact_count": artifact_count,
            "dropped": true,
        }));
    }
    Ok(dropped)
}

fn build_artifact_index(
    workspace_root: &Path,
    task_root: &Path,
    change_id: Option<&str>,
) -> Result<Vec<ArtifactIndexEntry>, TaskRunError> {
    let mut files = Vec::new();
    collect_files(&task_root.join("artifacts"), &mut files)?;
    collect_files(&task_root.join("reports"), &mut files)?;
    files.sort();

    let mut entries = files
        .into_iter()
        .map(|path| artifact_entry(workspace_root, task_root, &path))
        .collect::<Result<Vec<_>, _>>()?;
    if let Some(change_id) = change_id {
        entries.extend(openspec_artifacts(workspace_root, change_id));
    }
    Ok(entries)
}

fn collect_files(root: &Path, files: &mut Vec<PathBuf>) -> Result<(), TaskRunError> {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(TaskRunError::new(
                "interactive_projection_io",
                format!("read {}: {error}", root.display()),
            ));
        }
    };

    for entry in entries {
        let entry = entry.map_err(|error| {
            TaskRunError::new(
                "interactive_projection_io",
                format!("read {} entry: {error}", root.display()),
            )
        })?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|error| {
            TaskRunError::new(
                "interactive_projection_io",
                format!("stat {}: {error}", path.display()),
            )
        })?;
        if file_type.is_dir() {
            collect_files(&path, files)?;
        } else if file_type.is_file() {
            files.push(path);
        }
    }
    Ok(())
}

fn artifact_entry(
    workspace_root: &Path,
    task_root: &Path,
    path: &Path,
) -> Result<ArtifactIndexEntry, TaskRunError> {
    let content_type = content_type(path);
    let json_value = if is_json_file(path) {
        Some(read_json(path)?)
    } else {
        None
    };
    let fallback_ref = infer_artifact_ref(task_root, path);
    let artifact_ref = json_value
        .as_ref()
        .and_then(|value| string_field(value, "artifact_ref"))
        .unwrap_or_else(|| fallback_ref.clone());
    let artifact_kind = json_value
        .as_ref()
        .and_then(|value| string_field(value, "artifact_kind"))
        .unwrap_or_else(|| fallback_ref.replace('-', "_"));
    let traceability_refs = json_value
        .as_ref()
        .and_then(traceability_refs)
        .unwrap_or_default();
    let producer_node = json_value
        .as_ref()
        .and_then(|value| string_field(value, "producer_node"))
        .or_else(|| {
            json_value
                .as_ref()
                .and_then(|value| string_field(value, "node_id"))
        });
    let relative_path = path
        .strip_prefix(workspace_root)
        .unwrap_or(path)
        .to_string_lossy()
        .to_string();

    Ok(ArtifactIndexEntry {
        artifact_ref,
        artifact_kind: artifact_kind.clone(),
        producer_node,
        path: relative_path,
        summary: artifact_kind,
        status: ArtifactStatus::Active,
        content_type,
        traceability_refs,
        dropped: false,
    })
}

fn content_type(path: &Path) -> ContentType {
    let path_text = path.to_string_lossy();
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    if path_text.contains("/tests/") || file_name.contains(".test.") || file_name.contains(".spec.")
    {
        return ContentType::Test;
    }
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("md") => ContentType::Markdown,
        Some("json") => ContentType::Json,
        Some("js" | "ts" | "rs") => ContentType::Source,
        Some("log" | "jsonl") => ContentType::Log,
        _ => ContentType::Unknown,
    }
}

fn openspec_artifacts(workspace_root: &Path, change_id: &str) -> Vec<ArtifactIndexEntry> {
    let candidates = [
        (
            "openspec_proposal",
            format!("openspec/changes/{change_id}/proposal.md"),
        ),
        (
            "openspec_design",
            format!("openspec/changes/{change_id}/design.md"),
        ),
        (
            "openspec_tasks",
            format!("openspec/changes/{change_id}/tasks.md"),
        ),
        (
            "openspec_spec",
            format!("openspec/changes/{change_id}/specs/main/spec.md"),
        ),
    ];
    candidates
        .into_iter()
        .filter_map(|(artifact_kind, relative_path)| {
            let path = workspace_root.join(&relative_path);
            path.exists().then(|| ArtifactIndexEntry {
                artifact_ref: artifact_kind.to_string(),
                artifact_kind: artifact_kind.to_string(),
                producer_node: None,
                path: relative_path,
                summary: artifact_kind.to_string(),
                status: ArtifactStatus::Active,
                content_type: ContentType::Markdown,
                traceability_refs: Vec::new(),
                dropped: false,
            })
        })
        .collect()
}

fn is_json_file(path: &Path) -> bool {
    path.extension().and_then(|extension| extension.to_str()) == Some("json")
}

fn infer_artifact_ref(task_root: &Path, path: &Path) -> String {
    let without_extension = path.with_extension("");
    let relative = without_extension
        .strip_prefix(task_root)
        .unwrap_or(&without_extension);
    let mut ref_id = String::new();
    for character in relative.to_string_lossy().chars() {
        if character.is_ascii_alphanumeric() || character == '_' {
            ref_id.push(character);
        } else {
            ref_id.push('_');
        }
    }
    let ref_id = ref_id.trim_matches('_').to_string();
    if ref_id.is_empty() {
        "artifact".to_string()
    } else {
        ref_id
    }
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(ToString::to_string)
}

fn u64_field(value: &Value, key: &str) -> Option<u64> {
    value.get(key).and_then(Value::as_u64)
}

fn traceability_refs(value: &Value) -> Option<Vec<String>> {
    value
        .get("_aria")?
        .get("traceability_refs")?
        .as_array()
        .map(|refs| {
            refs.iter()
                .filter_map(Value::as_str)
                .map(ToString::to_string)
                .collect()
        })
}

fn validate_task_id(task_id: &str) -> Result<(), TaskRunError> {
    if task_id.is_empty()
        || task_id.contains('/')
        || task_id.contains('\\')
        || task_id.contains("..")
    {
        return Err(TaskRunError::new(
            "interactive_projection_invalid_task_id",
            format!("invalid task id: {task_id}"),
        ));
    }
    Ok(())
}
