use std::fs;
use std::str::FromStr;

use serde_json::json;

use crate::interactive::models::RuntimeCheckpoint;
use crate::interactive::policy::{NodeWriteClass, PolicyPreset};
use crate::task_run::types::TaskRunError;
use crate::web::runtime_store::WebRuntimeStore;
use crate::web::types::PendingProviderStepDto;

use super::utils::{git_head, io_error, read_optional_json};

pub(super) fn preserve_web_task_metadata(
    store: &WebRuntimeStore,
    previous_state: &serde_json::Value,
    task_id: &str,
    change_id: &str,
    timeout_secs: u64,
) -> Result<(), TaskRunError> {
    let mut current = read_optional_json(&store.task_root().join("state.json"))?;
    if !current.is_object() {
        current = json!({});
    }
    let object = current.as_object_mut().expect("state object");
    object.insert("task_id".to_string(), json!(task_id));
    object.insert("change_id".to_string(), json!(change_id));
    object.insert("provider_mode".to_string(), json!("real"));
    object.insert(
        "request_text".to_string(),
        previous_state
            .get("request_text")
            .cloned()
            .unwrap_or_else(|| json!("")),
    );
    object.insert(
        "policy_preset".to_string(),
        previous_state
            .get("policy_preset")
            .cloned()
            .unwrap_or_else(|| json!("manual-write")),
    );
    object.insert("timeout_secs".to_string(), json!(timeout_secs));
    store.write_json("state.json", &current)?;
    Ok(())
}

pub(super) fn policy_for_task(store: &WebRuntimeStore) -> Result<PolicyPreset, TaskRunError> {
    let state = read_optional_json(&store.task_root().join("state.json"))?;
    let policy = state
        .get("policy_preset")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("manual-write");
    PolicyPreset::from_str(policy).map_err(|error| TaskRunError::new("web_runtime_policy", error))
}

pub(super) fn run_internal_n00_if_needed(store: &WebRuntimeStore) -> Result<(), TaskRunError> {
    if node_event_exists(store, "N00")? {
        return Ok(());
    }
    store.append_event("node_started", "N00", json!({"status":"running"}))?;
    store.write_json(
        "node-runs/nrun_n00.json",
        &json!({
            "node_run_id": "nrun_n00",
            "node_id": "N00",
            "turn_id": null,
            "provider_run_id": null,
            "input_refs": [],
            "output_schema": null,
            "artifact_refs": ["internal_n00"],
            "status": "completed",
            "duration_ms": 1,
            "diagnostic_refs": [],
            "dropped": false,
            "created_at": "2026-05-09T00:00:00Z",
            "updated_at": "2026-05-09T00:00:00Z"
        }),
    )?;
    store.write_json(
        "artifacts/internal/n00.json",
        &json!({
            "artifact_ref": "internal_n00",
            "artifact_kind": "internal_step",
            "producer_node": "N00",
            "summary": "runtime bootstrap",
            "dropped": false
        }),
    )?;
    store.append_event(
        "artifact_written",
        "N00",
        json!({"status":"completed","artifact_ref":"internal_n00"}),
    )?;
    store.append_event("node_completed", "N00", json!({"status":"completed"}))
}

fn node_event_exists(store: &WebRuntimeStore, node_id: &str) -> Result<bool, TaskRunError> {
    let path = store.task_root().join("logs/node-events.jsonl");
    match fs::read_to_string(path) {
        Ok(events) => Ok(events.contains(&format!("\"node_id\":\"{node_id}\""))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(io_error(error)),
    }
}

pub(super) fn write_checkpoint(
    workspace_root: &std::path::Path,
    task_id: &str,
    store: &WebRuntimeStore,
    projection_version: u64,
) -> Result<(), TaskRunError> {
    store.write_json(
        "checkpoints/state@ckpt_0001.json",
        &read_optional_json(&store.task_root().join("state.json"))?,
    )?;
    store.write_json(
        "checkpoints/projection@ckpt_0001.json",
        &json!({"projection_version": projection_version}),
    )?;
    let task_root = store.task_root();
    store.write_json(
        "checkpoints/ckpt_0001.json",
        &RuntimeCheckpoint {
            checkpoint_id: "ckpt_0001".to_string(),
            task_id: task_id.to_string(),
            session_id: format!("sess_{task_id}"),
            turn_id: Some("turn_0001".to_string()),
            git_head: git_head(workspace_root),
            dirty_summary: json!({}),
            state_snapshot_ref: "state@ckpt_0001.json".to_string(),
            projection_snapshot_ref: "projection@ckpt_0001.json".to_string(),
            artifact_boundary: count_runtime_artifacts(task_root)?,
            provider_run_boundary: count_json_files_recursive(&task_root.join("provider-runs"))?,
            node_run_boundary: count_json_files(&task_root.join("node-runs"))?,
            created_at: "2026-05-09T00:00:00Z".to_string(),
        },
    )?;
    Ok(())
}

fn count_runtime_artifacts(task_root: &std::path::Path) -> Result<usize, TaskRunError> {
    Ok(count_json_files_recursive(&task_root.join("artifacts"))?
        + count_json_files_recursive(&task_root.join("reports"))?)
}

fn count_json_files(root: &std::path::Path) -> Result<usize, TaskRunError> {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(io_error(error)),
    };
    let mut count = 0;
    for entry in entries {
        let entry = entry.map_err(io_error)?;
        if entry
            .path()
            .extension()
            .and_then(|extension| extension.to_str())
            == Some("json")
        {
            count += 1;
        }
    }
    Ok(count)
}

fn count_json_files_recursive(root: &std::path::Path) -> Result<usize, TaskRunError> {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(io_error(error)),
    };
    let mut count = 0;
    for entry in entries {
        let entry = entry.map_err(io_error)?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(io_error)?;
        if file_type.is_dir() {
            count += count_json_files_recursive(&path)?;
        } else if path.extension().and_then(|extension| extension.to_str()) == Some("json") {
            count += 1;
        }
    }
    Ok(count)
}

pub(super) fn pending_provider_step_for_policy(
    policy: PolicyPreset,
    state: &serde_json::Value,
) -> PendingProviderStepDto {
    let task_id = state
        .get("task_id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("task_0001");
    let request_text = state
        .get("request_text")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("实现 Fibonacci square sum");
    let change_id = state
        .get("change_id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("aria-fibonacci-square");
    match policy {
        PolicyPreset::ManualAll => PendingProviderStepDto {
            node_id: "N04".to_string(),
            provider_type: "claude_code".to_string(),
            runtime_role: "orchestrator".to_string(),
            adapter_role: "orchestrator".to_string(),
            prompt: format!("执行 N04：{request_text}"),
            input_summary: json!({"node_id":"N04"}),
            canonical_input_refs: vec![format!("task:{task_id}")],
            context_files: vec![format!("openspec/changes/{change_id}/proposal.md")],
            output_schema: "schema://aria/artifacts/planning_report/v1".to_string(),
            allowed_write_scope: vec![".aria/runtime/".to_string(), "openspec/".to_string()],
            forbidden_actions: vec!["修改 cadence/project-rules".to_string()],
            verification_commands: vec!["cargo check --locked".to_string()],
            checkpoint_id: "ckpt_0001".to_string(),
        },
        PolicyPreset::ManualWrite | PolicyPreset::AutoReview | PolicyPreset::NonInteractive => {
            PendingProviderStepDto {
                node_id: "N16".to_string(),
                provider_type: "codex".to_string(),
                runtime_role: "executor".to_string(),
                adapter_role: "executor".to_string(),
                prompt: request_text.to_string(),
                input_summary: json!({"worktask_id":"work_wt_001"}),
                canonical_input_refs: vec!["worktask:work_wt_001".to_string()],
                context_files: vec![format!("openspec/changes/{change_id}/tasks.md")],
                output_schema: "schema://aria/artifacts/coding_report/v1".to_string(),
                allowed_write_scope: vec!["src/".to_string(), "tests/".to_string()],
                forbidden_actions: vec!["修改 cadence/project-rules".to_string()],
                verification_commands: vec!["cargo test --locked -j 1".to_string()],
                checkpoint_id: "ckpt_0001".to_string(),
            }
        }
    }
}

pub(super) fn write_class_for_pending(pending: &PendingProviderStepDto) -> NodeWriteClass {
    match pending.node_id.as_str() {
        "N16" | "N19" => NodeWriteClass::WritesWorkspace,
        "N04" | "N05" | "N07" | "N09" | "N10" | "N11" | "N12" | "N25" | "N26" | "N27" => {
            NodeWriteClass::WritesRuntime
        }
        _ => NodeWriteClass::ReadOnly,
    }
}
