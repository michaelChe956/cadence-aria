use std::fs;
use std::process::Command;

use serde_json::json;

use crate::cross_cutting::cli_adapter::CliOutputChunk;
use crate::protocol::contracts::ProviderType;
use crate::task_run::types::{TaskRunError, TaskRunStatus};

pub(super) fn task_status_text(status: &TaskRunStatus) -> &'static str {
    match status {
        TaskRunStatus::Completed => "completed",
        TaskRunStatus::Failed => "failed",
        TaskRunStatus::BlockedByGate => "blocked_by_gate",
    }
}

pub(super) fn provider_node_id_for_schema(output_schema: &str) -> &'static str {
    if output_schema.contains("clarification_record") {
        "N04"
    } else if output_schema.contains("spec_gate_review") {
        "N06"
    } else if output_schema.contains("spec/v1") {
        "N05"
    } else if output_schema.contains("design_review") {
        "N08"
    } else if output_schema.contains("design/v1") {
        "N07"
    } else if output_schema.contains("readiness_check") {
        "N10"
    } else if output_schema.contains("plan/v1") {
        "N11"
    } else if output_schema.contains("dispatch_package") {
        "N12"
    } else if output_schema.contains("coding_report") {
        "N16"
    } else if output_schema.contains("testing_report") {
        "N17"
    } else if output_schema.contains("code_review_report") {
        "N18"
    } else if output_schema.contains("final_review") {
        "N25"
    } else if output_schema.contains("patch_task_delta") {
        "N26"
    } else if output_schema.contains("final_summary") {
        "N27"
    } else {
        "provider"
    }
}

pub(super) fn provider_run_id_for_chunk(chunk: &CliOutputChunk) -> String {
    format!(
        "stream_{}_{}",
        provider_type_slug(&chunk.provider_type),
        provider_node_id_for_schema(&chunk.output_schema).to_ascii_lowercase()
    )
}

fn provider_type_slug(provider_type: &ProviderType) -> &'static str {
    match provider_type {
        ProviderType::ClaudeCode => "claude",
        ProviderType::Codex => "codex",
        ProviderType::Fake => "fake",
    }
}

pub(super) fn parse_confirm_provider_type(value: &str) -> Result<ProviderType, TaskRunError> {
    match value {
        "claude_code" => Ok(ProviderType::ClaudeCode),
        "codex" => Ok(ProviderType::Codex),
        other => Err(TaskRunError::new(
            "web_runtime_provider_type",
            format!("unsupported provider_type: {other}"),
        )),
    }
}

pub(super) fn provider_input_ref_for_node(node_id: &str) -> String {
    let normalized = node_id
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '_' || *ch == '-')
        .map(|ch| ch.to_ascii_lowercase())
        .collect::<String>();
    format!("run_{normalized}_0001")
}

pub(super) fn io_error(error: std::io::Error) -> TaskRunError {
    TaskRunError::new("web_runtime_io", error.to_string())
}

pub(super) fn json_error(error: serde_json::Error) -> TaskRunError {
    TaskRunError::new("web_runtime_json", error.to_string())
}

pub(super) fn read_optional_json(
    path: &std::path::Path,
) -> Result<serde_json::Value, TaskRunError> {
    match fs::File::open(path) {
        Ok(file) => serde_json::from_reader(file)
            .map_err(|error| TaskRunError::new("web_runtime_json", error.to_string())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(json!({})),
        Err(error) => Err(io_error(error)),
    }
}

pub(super) fn safe_workspace_path(
    root: &std::path::Path,
    path: &str,
) -> Result<std::path::PathBuf, TaskRunError> {
    if path.contains("..") || path.starts_with('/') || path.starts_with('\\') {
        return Err(TaskRunError::new(
            "invalid_file_path",
            format!("unsafe path: {path}"),
        ));
    }
    Ok(root.join(path))
}

pub(super) fn content_type_for_path(path: &str) -> String {
    if path.ends_with(".md") {
        "markdown".to_string()
    } else if path.ends_with(".json") {
        "json".to_string()
    } else if path.contains("/tests/") || path.contains(".test.") || path.contains(".spec.") {
        "test".to_string()
    } else if path.ends_with(".log") || path.ends_with(".jsonl") {
        "log".to_string()
    } else {
        "source".to_string()
    }
}

pub(super) fn git_head(workspace_root: &std::path::Path) -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(workspace_root)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|head| !head.is_empty())
}
