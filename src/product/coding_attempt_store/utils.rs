use std::fs;
use std::io::{ErrorKind, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::product::coding_models::{
    CodingExecutionStage, CodingRoleRunEvent, CodingRoleRunEventType,
};
use crate::product::json_store::{ProductStoreError, read_json};

pub(crate) const ROLE_RUN_EVENT_INLINE_STRING_LIMIT: usize = 16_384;
pub(crate) const ROLE_RUN_RETRY_DIAGNOSTIC_LIMIT: usize = 8_000;
pub(crate) const ROLE_RUN_RETRY_DIAGNOSTIC_FIELD_LIMIT: usize = 512;

pub(crate) fn list_json_records<T: for<'de> Deserialize<'de>>(
    path: &Path,
) -> Result<Vec<T>, ProductStoreError> {
    let entries = json_file_paths(path)?;
    let mut records = Vec::with_capacity(entries.len());
    for entry in entries {
        records.push(read_json(&entry)?);
    }
    Ok(records)
}

pub(crate) fn append_jsonl<T: Serialize>(path: &Path, value: &T) -> Result<(), ProductStoreError> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|error| {
            ProductStoreError::Io(format!("create {}: {error}", parent.display()))
        })?;
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| ProductStoreError::Io(format!("open {}: {error}", path.display())))?;
    let mut line =
        serde_json::to_vec(value).map_err(|error| ProductStoreError::Json(error.to_string()))?;
    line.push(b'\n');
    file.write_all(&line)
        .map_err(|error| ProductStoreError::Io(format!("write {}: {error}", path.display())))?;
    file.flush()
        .map_err(|error| ProductStoreError::Io(format!("flush {}: {error}", path.display())))?;
    Ok(())
}

pub(crate) fn read_jsonl_records<T: for<'de> Deserialize<'de>>(
    path: &Path,
) -> Result<Vec<T>, ProductStoreError> {
    if !path_exists(path)? {
        return Ok(Vec::new());
    }
    let content = fs::read_to_string(path)
        .map_err(|error| ProductStoreError::Io(format!("read {}: {error}", path.display())))?;
    let mut records = Vec::new();
    for line in content.lines().filter(|line| !line.trim().is_empty()) {
        records.push(
            serde_json::from_str(line)
                .map_err(|error| ProductStoreError::Json(error.to_string()))?,
        );
    }
    Ok(records)
}

pub(crate) fn next_jsonl_sequence(path: &Path) -> Result<u64, ProductStoreError> {
    Ok(read_jsonl_records::<serde_json::Value>(path)?.len() as u64 + 1)
}

pub(crate) fn truncate_utf8(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_string();
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_string()
}

pub(crate) fn coding_role_run_event_type_name(event_type: CodingRoleRunEventType) -> &'static str {
    match event_type {
        CodingRoleRunEventType::ProviderPrompt => "provider_prompt",
        CodingRoleRunEventType::ProviderStart => "provider_start",
        CodingRoleRunEventType::TextDelta => "text_delta",
        CodingRoleRunEventType::ExecutionEvent => "execution_event",
        CodingRoleRunEventType::ToolCall => "tool_call",
        CodingRoleRunEventType::ToolResult => "tool_result",
        CodingRoleRunEventType::StatusChanged => "status_changed",
        CodingRoleRunEventType::PermissionRequest => "permission_request",
        CodingRoleRunEventType::ChoiceRequest => "choice_request",
        CodingRoleRunEventType::MessageComplete => "message_complete",
        CodingRoleRunEventType::ProviderFailed => "provider_failed",
        CodingRoleRunEventType::Timeout => "timeout",
        CodingRoleRunEventType::Aborted => "aborted",
        CodingRoleRunEventType::PersistenceWarning => "persistence_warning",
    }
}

pub(crate) fn role_run_event_payload_text<'a>(
    event: &'a CodingRoleRunEvent,
    field: &str,
) -> Option<&'a str> {
    event.payload.get(field).and_then(|value| value.as_str())
}

pub(crate) fn role_run_event_payload_summary_text(
    event: &CodingRoleRunEvent,
    field: &str,
) -> String {
    role_run_event_payload_text(event, field)
        .map(|value| truncate_utf8(value, ROLE_RUN_RETRY_DIAGNOSTIC_FIELD_LIMIT))
        .unwrap_or_else(|| "-".to_string())
}

pub(crate) fn role_run_event_payload_reason(event: &CodingRoleRunEvent) -> Option<&str> {
    role_run_event_payload_text(event, "reason_code")
        .or_else(|| role_run_event_payload_text(event, "message"))
}

pub(crate) fn role_run_event_payload_reason_summary(event: &CodingRoleRunEvent) -> Option<String> {
    role_run_event_payload_reason(event)
        .map(|value| truncate_utf8(value, ROLE_RUN_RETRY_DIAGNOSTIC_FIELD_LIMIT))
}

pub(crate) fn role_run_event_artifact_refs(event: &CodingRoleRunEvent) -> Vec<String> {
    let mut artifact_refs = Vec::new();
    if let Some(artifact_ref) = event.artifact_ref.as_deref() {
        push_unique_artifact_ref(&mut artifact_refs, artifact_ref);
    }
    collect_payload_artifact_refs(&event.payload, &mut artifact_refs);
    artifact_refs
}

pub(crate) fn collect_payload_artifact_refs(
    value: &serde_json::Value,
    artifact_refs: &mut Vec<String>,
) {
    match value {
        serde_json::Value::Object(object) => {
            if let Some(artifact_ref) = object.get("artifact_ref").and_then(|value| value.as_str())
            {
                push_unique_artifact_ref(artifact_refs, artifact_ref);
            }
            for nested in object.values() {
                collect_payload_artifact_refs(nested, artifact_refs);
            }
        }
        serde_json::Value::Array(values) => {
            for nested in values {
                collect_payload_artifact_refs(nested, artifact_refs);
            }
        }
        _ => {}
    }
}

pub(crate) fn push_unique_artifact_ref(artifact_refs: &mut Vec<String>, artifact_ref: &str) {
    if !artifact_refs
        .iter()
        .any(|existing| existing == artifact_ref)
    {
        artifact_refs.push(artifact_ref.to_string());
    }
}

pub(crate) fn count_json_files(path: &Path) -> Result<usize, ProductStoreError> {
    Ok(json_file_paths(path)?.len())
}

pub(crate) fn next_text_file_sequence(
    path: &Path,
    purpose: &str,
) -> Result<usize, ProductStoreError> {
    if !path_exists(path)? {
        return Ok(1);
    }
    let prefix = format!("{purpose}_");
    let mut count = 0;
    for entry in fs::read_dir(path)
        .map_err(|error| ProductStoreError::Io(format!("read {}: {error}", path.display())))?
    {
        let entry = entry.map_err(|error| {
            ProductStoreError::Io(format!("read {} entry: {error}", path.display()))
        })?;
        let file_type = entry.file_type().map_err(|error| {
            ProductStoreError::Io(format!(
                "read {} entry type: {error}",
                entry.path().display()
            ))
        })?;
        if !file_type.is_file() {
            continue;
        }
        let Some(file_name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if file_name.starts_with(&prefix) && file_name.ends_with(".txt") {
            count += 1;
        }
    }
    Ok(count + 1)
}

pub(crate) fn coding_stage_dir_name(stage: &CodingExecutionStage) -> &'static str {
    match stage {
        CodingExecutionStage::PrepareContext => "prepare_context",
        CodingExecutionStage::WorktreePrepare => "worktree_prepare",
        CodingExecutionStage::Coding => "coding",
        CodingExecutionStage::Testing => "testing",
        CodingExecutionStage::CodeReview => "code_review",
        CodingExecutionStage::Rework => "rework",
        CodingExecutionStage::ReviewRequest => "review_request",
        CodingExecutionStage::InternalPrReview => "internal_pr_review",
        CodingExecutionStage::FinalConfirm => "final_confirm",
    }
}

pub(crate) fn merge_unique_strings(target: &mut Vec<String>, source: Vec<String>) {
    for value in source {
        if !target.iter().any(|existing| existing == &value) {
            target.push(value);
        }
    }
}

pub(crate) fn json_file_paths(path: &Path) -> Result<Vec<std::path::PathBuf>, ProductStoreError> {
    if !path_exists(path)? {
        return Ok(Vec::new());
    }
    let mut entries = Vec::new();
    for entry in fs::read_dir(path)
        .map_err(|error| ProductStoreError::Io(format!("read {}: {error}", path.display())))?
    {
        let entry = entry.map_err(|error| {
            ProductStoreError::Io(format!("read {} entry: {error}", path.display()))
        })?;
        let file_type = entry.file_type().map_err(|error| {
            ProductStoreError::Io(format!(
                "read {} entry type: {error}",
                entry.path().display()
            ))
        })?;
        let entry_path = entry.path();
        if file_type.is_file()
            && entry_path.extension().and_then(|value| value.to_str()) == Some("json")
        {
            entries.push(entry_path);
        }
    }
    entries.sort();
    Ok(entries)
}

pub(crate) fn child_directories(path: &Path) -> Result<Vec<std::path::PathBuf>, ProductStoreError> {
    if !path_exists(path)? {
        return Ok(Vec::new());
    }
    let mut entries = Vec::new();
    for entry in fs::read_dir(path)
        .map_err(|error| ProductStoreError::Io(format!("read {}: {error}", path.display())))?
    {
        let entry = entry.map_err(|error| {
            ProductStoreError::Io(format!("read {} entry: {error}", path.display()))
        })?;
        let file_type = entry.file_type().map_err(|error| {
            ProductStoreError::Io(format!(
                "read {} entry type: {error}",
                entry.path().display()
            ))
        })?;
        if file_type.is_dir() {
            entries.push(entry.path());
        }
    }
    entries.sort();
    Ok(entries)
}

pub(crate) fn path_exists(path: &Path) -> Result<bool, ProductStoreError> {
    match fs::metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(ProductStoreError::Io(format!(
            "metadata {}: {error}",
            path.display()
        ))),
    }
}

pub(crate) fn path_is_regular_file(path: &Path) -> Result<bool, ProductStoreError> {
    match fs::metadata(path) {
        Ok(metadata) => Ok(metadata.is_file()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(ProductStoreError::Io(format!(
            "metadata {}: {error}",
            path.display()
        ))),
    }
}

pub(crate) fn remove_file_if_exists(path: &Path) -> Result<(), ProductStoreError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(ProductStoreError::Io(format!(
            "remove {}: {error}",
            path.display()
        ))),
    }
}

pub(crate) fn remove_dir_all_if_exists(path: &Path) -> Result<(), ProductStoreError> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(ProductStoreError::Io(format!(
            "remove {}: {error}",
            path.display()
        ))),
    }
}
