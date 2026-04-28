use crate::protocol::artifacts::{RiskEntry, RiskRegistryRef, RiskRegistrySnapshot, RiskStatus};
use crate::protocol::contracts::{
    AdapterInput, AdapterOutput, ProviderRunRecord, ProviderRunStatus,
};
use crate::protocol::enums::ExternalRefId;
use crate::protocol::projections::RiskSeverity;
use chrono::Utc;
use std::path::Path;

use super::provider_router::ProviderRunRequest;

pub fn provider_run_record_from_output(
    request: &ProviderRunRequest,
    input: &AdapterInput,
    output: &AdapterOutput,
) -> ProviderRunRecord {
    let stdout_ref = external_ref(&request.provider_run_id, "stdout");
    let stderr_ref = external_ref(&request.provider_run_id, "stderr");
    let structured_output_ref = external_ref(&request.provider_run_id, "structured_output");
    let completed = output.exit_code == Some(0)
        && matches!(
            output.timeout_status,
            crate::protocol::contracts::TimeoutStatus::NotTimedOut
        );

    ProviderRunRecord {
        provider_run_id: request.provider_run_id.clone(),
        node_id: request.node_id.clone(),
        provider_type: input.provider_type.clone(),
        runtime_role: request.runtime_role.clone(),
        adapter_role: input.role.clone(),
        provider_capability_ref: request.provider_capability_ref.clone(),
        adapter_compatibility_ref: request.adapter_compatibility_ref.clone(),
        context_package_ref: request.context_package_ref.clone(),
        adapter_input_ref: request.adapter_input_ref.clone(),
        adapter_output_ref: request.adapter_output_ref.clone(),
        raw_artifact_refs: vec![
            stdout_ref.clone(),
            stderr_ref.clone(),
            structured_output_ref.clone(),
        ],
        exit_code: output.exit_code,
        error_code: (!completed).then(|| "provider_execution_failed".to_string()),
        error_details: None,
        stdout_ref: Some(stdout_ref),
        stderr_ref: Some(stderr_ref),
        structured_output_ref: Some(structured_output_ref),
        files_modified: output.files_modified.clone(),
        status: if completed {
            ProviderRunStatus::Completed
        } else {
            ProviderRunStatus::Failed
        },
        started_at: Utc::now().to_rfc3339(),
        completed_at: Some(Utc::now().to_rfc3339()),
        duration_ms: Some(output.duration_ms),
        timeout_status: output.timeout_status.clone(),
        retry_count: 0,
        approval_policy: request.approval_policy.clone(),
        sandbox_mode: request.sandbox_mode.clone(),
        constraint_check_ref: request.constraint_check_ref.clone(),
        traceability_binding_refs: request.traceability_binding_refs.clone(),
    }
}

pub fn failed_provider_run_record_from_error(
    request: &ProviderRunRequest,
    input: &AdapterInput,
    error: &crate::cross_cutting::provider_adapter::ProviderAdapterError,
) -> ProviderRunRecord {
    let stdout_ref = external_ref(&request.provider_run_id, "stdout");
    let stderr_ref = external_ref(&request.provider_run_id, "stderr");
    let structured_output_ref = external_ref(&request.provider_run_id, "structured_output");

    ProviderRunRecord {
        provider_run_id: request.provider_run_id.clone(),
        node_id: request.node_id.clone(),
        provider_type: input.provider_type.clone(),
        runtime_role: request.runtime_role.clone(),
        adapter_role: input.role.clone(),
        provider_capability_ref: request.provider_capability_ref.clone(),
        adapter_compatibility_ref: request.adapter_compatibility_ref.clone(),
        context_package_ref: request.context_package_ref.clone(),
        adapter_input_ref: request.adapter_input_ref.clone(),
        adapter_output_ref: request.adapter_output_ref.clone(),
        raw_artifact_refs: vec![
            stdout_ref.clone(),
            stderr_ref.clone(),
            structured_output_ref.clone(),
        ],
        exit_code: error.exit_code,
        error_code: Some(error.code.as_str().to_string()),
        error_details: Some(error.details.clone()),
        stdout_ref: Some(stdout_ref),
        stderr_ref: Some(stderr_ref),
        structured_output_ref: Some(structured_output_ref),
        files_modified: Vec::new(),
        status: ProviderRunStatus::Failed,
        started_at: Utc::now().to_rfc3339(),
        completed_at: Some(Utc::now().to_rfc3339()),
        duration_ms: Some(error.duration_ms),
        timeout_status: error.timeout_status.clone(),
        retry_count: 0,
        approval_policy: request.approval_policy.clone(),
        sandbox_mode: request.sandbox_mode.clone(),
        constraint_check_ref: request.constraint_check_ref.clone(),
        traceability_binding_refs: request.traceability_binding_refs.clone(),
    }
}

pub fn recover_provider_run_records(records: Vec<ProviderRunRecord>) -> Vec<ProviderRunRecord> {
    records
        .into_iter()
        .map(|mut record| {
            if matches!(
                record.status,
                ProviderRunStatus::Pending | ProviderRunStatus::Running
            ) {
                record.status = ProviderRunStatus::RecoveredPending;
                record.error_code = Some("recovered_pending".to_string());
                record.error_details =
                    Some("provider run was pending or running during daemon recovery".to_string());
            }
            record
        })
        .collect()
}

pub fn write_provider_run_record(
    path: &Path,
    record: &ProviderRunRecord,
) -> Result<(), ProviderRunPersistError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            ProviderRunPersistError::Io(format!("create {}: {error}", parent.display()))
        })?;
    }
    let content = serde_json::to_vec_pretty(record)
        .map_err(|error| ProviderRunPersistError::Serialize(error.to_string()))?;
    std::fs::write(path, content)
        .map_err(|error| ProviderRunPersistError::Io(format!("write {}: {error}", path.display())))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RiskEntryInput {
    pub description: String,
    pub severity: RiskSeverity,
    pub status: RiskStatus,
    pub source_artifact: Option<String>,
    pub source_node: String,
    pub resolution: Option<String>,
}

pub fn load_risk_registry_snapshot(
    task_root: &Path,
) -> Result<RiskRegistrySnapshot, RiskRegistryError> {
    let path = registry_path(task_root);
    let content = std::fs::read(&path)
        .map_err(|error| RiskRegistryError::Io(format!("read {}: {error}", path.display())))?;
    let mut snapshot: RiskRegistrySnapshot = serde_json::from_slice(&content)
        .map_err(|error| RiskRegistryError::Serialize(error.to_string()))?;
    normalize_snapshot(task_root, &mut snapshot);
    Ok(snapshot)
}

pub fn recover_risk_registry_snapshot(
    workspace_root: &Path,
    task_id: &str,
) -> Result<RiskRegistrySnapshot, RiskRegistryError> {
    load_risk_registry_snapshot(&workspace_root.join(".aria/runtime/tasks").join(task_id))
}

pub fn allocate_next_risk_id(snapshot: &RiskRegistrySnapshot) -> String {
    let max_id = snapshot
        .risk_ids
        .iter()
        .chain(snapshot.risks.iter().map(|risk| &risk.risk_id))
        .filter_map(|risk_id| {
            risk_id
                .strip_prefix("risk-")
                .and_then(|number| number.parse::<u32>().ok())
        })
        .max()
        .unwrap_or(0);
    format!("risk-{next:03}", next = max_id + 1)
}

pub fn append_risk_entry(
    task_root: &Path,
    input: RiskEntryInput,
) -> Result<RiskRegistrySnapshot, RiskRegistryError> {
    let mut snapshot = load_risk_registry_snapshot(task_root)?;
    let now = Utc::now().to_rfc3339();
    let risk_id = allocate_next_risk_id(&snapshot);
    let entry = RiskEntry {
        risk_id: risk_id.clone(),
        description: input.description,
        severity: input.severity,
        status: input.status,
        source_artifact: input.source_artifact,
        source_node: input.source_node,
        created_at: now.clone(),
        updated_at: now.clone(),
        resolution: input.resolution,
    };
    if !snapshot.risk_ids.contains(&risk_id) {
        snapshot.risk_ids.push(risk_id);
    }
    snapshot.risks.push(entry);
    snapshot.updated_at = now;
    write_risk_registry_snapshot(task_root, &snapshot)?;
    Ok(snapshot)
}

pub fn sync_risk_registry_to_snapshot(
    snapshot_path: &Path,
    registry: &RiskRegistrySnapshot,
) -> Result<(), RiskRegistryError> {
    let content = std::fs::read(snapshot_path).map_err(|error| {
        RiskRegistryError::Io(format!("read {}: {error}", snapshot_path.display()))
    })?;
    let mut value: serde_json::Value = serde_json::from_slice(&content)
        .map_err(|error| RiskRegistryError::Serialize(error.to_string()))?;
    value["risk_registry"] = serde_json::to_value(registry)
        .map_err(|error| RiskRegistryError::Serialize(error.to_string()))?;
    std::fs::write(
        snapshot_path,
        serde_json::to_vec_pretty(&value)
            .map_err(|error| RiskRegistryError::Serialize(error.to_string()))?,
    )
    .map_err(|error| RiskRegistryError::Io(format!("write {}: {error}", snapshot_path.display())))
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProviderRunPersistError {
    #[error("provider run persistence io error: {0}")]
    Io(String),
    #[error("provider run serialization error: {0}")]
    Serialize(String),
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RiskRegistryError {
    #[error("risk registry io error: {0}")]
    Io(String),
    #[error("risk registry serialization error: {0}")]
    Serialize(String),
}

fn external_ref(provider_run_id: &str, channel: &str) -> ExternalRefId {
    format!("ext_{provider_run_id}_{channel}")
}

fn write_risk_registry_snapshot(
    task_root: &Path,
    snapshot: &RiskRegistrySnapshot,
) -> Result<(), RiskRegistryError> {
    let registry_path = registry_path(task_root);
    let registry_dir = registry_path
        .parent()
        .ok_or_else(|| RiskRegistryError::Io("risk registry path missing parent".to_string()))?;
    std::fs::create_dir_all(registry_dir).map_err(|error| {
        RiskRegistryError::Io(format!("create {}: {error}", registry_dir.display()))
    })?;
    let content = serde_json::to_vec_pretty(snapshot)
        .map_err(|error| RiskRegistryError::Serialize(error.to_string()))?;
    std::fs::write(&registry_path, &content).map_err(|error| {
        RiskRegistryError::Io(format!("write {}: {error}", registry_path.display()))
    })?;

    let registry_ref = RiskRegistryRef {
        risk_registry_ref_id: snapshot.risk_registry_ref.clone(),
        risk_registry_id: snapshot.registry_id.clone(),
        task_id: snapshot.task_id.clone(),
        path: "risk-registry/registry.json".to_string(),
        sha256: crate::cross_cutting::document_ops::compute_sha256(&content),
        version: 1,
        risk_count: snapshot.risks.len(),
    };
    let ref_dir = registry_dir.join("refs");
    std::fs::create_dir_all(&ref_dir)
        .map_err(|error| RiskRegistryError::Io(format!("create {}: {error}", ref_dir.display())))?;
    let ref_path = ref_dir.join(format!("{}.json", snapshot.risk_registry_ref));
    std::fs::write(
        &ref_path,
        serde_json::to_vec_pretty(&registry_ref)
            .map_err(|error| RiskRegistryError::Serialize(error.to_string()))?,
    )
    .map_err(|error| RiskRegistryError::Io(format!("write {}: {error}", ref_path.display())))
}

fn normalize_snapshot(task_root: &Path, snapshot: &mut RiskRegistrySnapshot) {
    let task_id = task_root
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default();
    if snapshot.task_id.is_empty() {
        snapshot.task_id = task_id;
    }
    if snapshot.registry_id.is_empty() {
        snapshot.registry_id = format!("riskreg_{}", snapshot.task_id);
    }
    if snapshot.risk_ids.is_empty() && !snapshot.risks.is_empty() {
        snapshot.risk_ids = snapshot
            .risks
            .iter()
            .map(|risk| risk.risk_id.clone())
            .collect();
    }
    if snapshot.updated_at.is_empty() {
        snapshot.updated_at = Utc::now().to_rfc3339();
    }
}

fn registry_path(task_root: &Path) -> std::path::PathBuf {
    task_root.join("risk-registry/registry.json")
}
