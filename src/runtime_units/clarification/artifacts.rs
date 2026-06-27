use super::utils::{now_iso8601, provider_run_id};
use super::{PLANNING_EXECUTION_CHAIN, PlanningChainState, PlanningNodeTrace, PlanningUnitError};
use crate::cross_cutting::document_ops::compute_sha256;
use crate::cross_cutting::provider_run::write_provider_run_record;
use crate::daemon::checkpoint::{RiskRegistrySnapshot, RuntimeSnapshot};
use crate::protocol::artifacts::{ArtifactKind, ArtifactRef, ArtifactStatus};
use crate::protocol::contracts::{AdapterOutput, ProviderRunRecord};
use crate::protocol::loop_counters::LoopCounterName;
use crate::protocol::policies::PolicyMode;
use crate::runtime_units::{RuntimeProtocolStep, RuntimeStepStatus};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub fn structured_output(
    node_id: &str,
    output: &AdapterOutput,
) -> Result<Value, PlanningUnitError> {
    output
        .structured_output
        .clone()
        .ok_or_else(|| PlanningUnitError::StructuredOutputMissing(node_id.to_string()))
}

pub fn normalize_clarification_record_candidate(record: &mut Value) {
    let Some(object) = record.as_object_mut() else {
        return;
    };
    for field in ["constraints", "assumptions", "open_questions"] {
        if matches!(object.get(field), None | Some(Value::Null)) {
            object.insert(field.to_string(), json!([]));
        }
    }
}

pub fn markdown_from_output(
    node_id: &str,
    output: &AdapterOutput,
    expected_kind: &str,
) -> Result<String, PlanningUnitError> {
    let value = structured_output(node_id, output)?;
    let got = value
        .get("artifact_kind")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if got != expected_kind {
        return Err(PlanningUnitError::IncompatibleOutput {
            node_id: node_id.to_string(),
            expected: expected_kind.to_string(),
            got: got.to_string(),
        });
    }
    value
        .get("markdown")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| PlanningUnitError::IncompatibleOutput {
            node_id: node_id.to_string(),
            expected: "markdown".to_string(),
            got: "missing".to_string(),
        })
}

pub fn write_json_artifact(
    state: &PlanningChainState,
    kind: ArtifactKind,
    node_id: &str,
    value: &Value,
) -> Result<ArtifactRef, PlanningUnitError> {
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| PlanningUnitError::Serialization(error.to_string()))?;
    write_artifact_bytes(state, kind, node_id, "json", &bytes)
}

pub fn write_markdown_artifact(
    state: &PlanningChainState,
    kind: ArtifactKind,
    node_id: &str,
    markdown: &str,
) -> Result<ArtifactRef, PlanningUnitError> {
    write_artifact_bytes(state, kind, node_id, "md", markdown.as_bytes())
}

pub fn record_protocol_step(
    state: &mut PlanningChainState,
    node_id: &str,
    consumed_constraint_kinds: Vec<String>,
    produced_artifact_kinds: Vec<String>,
    _checkpoint_path: PathBuf,
) {
    state.protocol_steps.push(RuntimeProtocolStep {
        node_id: node_id.to_string(),
        status: RuntimeStepStatus::Completed,
        node_specific_fields: json!({
            "provider_run_ref": provider_run_id(&state.input.task_id, node_id),
        }),
    });
    state.node_traces.push(PlanningNodeTrace {
        node_id: node_id.to_string(),
        execution_chain: PLANNING_EXECUTION_CHAIN
            .iter()
            .map(|step| (*step).to_string())
            .collect(),
        consumed_constraint_kinds,
        produced_artifact_kinds,
    });
}

pub fn write_checkpoint(
    state: &mut PlanningChainState,
    node_id: &str,
    phase: &str,
    artifact_refs: Vec<String>,
    projection_refs: Vec<String>,
    node_specific_fields: Value,
) -> Result<PathBuf, PlanningUnitError> {
    let snapshot_dir = state.task_root().join("snapshots");
    std::fs::create_dir_all(&snapshot_dir)
        .map_err(|error| PlanningUnitError::Io(format!("create snapshots dir: {error}")))?;
    let path = snapshot_dir.join(format!("{node_id}.json"));
    let snapshot = RuntimeSnapshot {
        snapshot_id: format!("snap_{}_{}", state.input.task_id, node_id.to_lowercase()),
        session_id: state.input.session_id.clone(),
        task_id: state.input.task_id.clone(),
        node_id: node_id.to_string(),
        phase: phase.to_string(),
        timestamp: now_iso8601(),
        effective_policy: PolicyMode::Conservative,
        artifact_refs,
        provider_run_refs: vec![provider_run_id(&state.input.task_id, node_id)],
        worktree_ref: state.input.worktree_path.clone(),
        rework_counter: 0,
        risk_registry: RiskRegistrySnapshot {
            risk_registry_ref: format!("riskreg_{}_v0001", state.input.task_id),
            risk_ids: vec![],
            risks: vec![],
        },
        loop_counters: BTreeMap::<LoopCounterName, u32>::new(),
        superseded_artifact_refs: state.superseded_artifact_refs.clone(),
        node_specific_fields,
        projection_refs,
        constraint_bundle_refs: vec![state.current_bundle.constraint_bundle_id.clone()],
    };
    snapshot
        .validate()
        .map_err(|message| PlanningUnitError::Io(format!("invalid snapshot: {message}")))?;
    let bytes = serde_json::to_vec_pretty(&snapshot)
        .map_err(|error| PlanningUnitError::Serialization(error.to_string()))?;
    std::fs::write(&path, bytes)
        .map_err(|error| PlanningUnitError::Io(format!("write {}: {error}", path.display())))?;
    state.checkpoint_paths.push(path.clone());
    Ok(path)
}

pub(crate) fn persist_provider_run(
    state: &mut PlanningChainState,
    record: &ProviderRunRecord,
) -> Result<(), PlanningUnitError> {
    let path = state
        .task_root()
        .join("provider-runs")
        .join(format!("{}.json", record.provider_run_id));
    write_provider_run_record(&path, record)?;
    state.provider_run_records.push(record.clone());
    Ok(())
}

fn write_artifact_bytes(
    state: &PlanningChainState,
    kind: ArtifactKind,
    _node_id: &str,
    extension: &str,
    bytes: &[u8],
) -> Result<ArtifactRef, PlanningUnitError> {
    let kind_name = kind.as_str();
    let artifact_id = format!("art_{}_{}_0001", kind_name, state.input.task_id);
    let artifact_ref_id = format!("ref_{artifact_id}_v0001");
    let artifact_dir = state.task_root().join("artifacts").join(kind_name);
    std::fs::create_dir_all(&artifact_dir)
        .map_err(|error| PlanningUnitError::Io(format!("create artifact dir: {error}")))?;
    let artifact_path = artifact_dir.join(format!("{artifact_id}_v0001.{extension}"));
    std::fs::write(&artifact_path, bytes).map_err(|error| {
        PlanningUnitError::Io(format!("write {}: {error}", artifact_path.display()))
    })?;
    let artifact_ref = ArtifactRef {
        artifact_ref_id: artifact_ref_id.clone(),
        artifact_id,
        artifact_kind: kind,
        version: 1,
        path: artifact_path.to_string_lossy().to_string(),
        sha256: compute_sha256(bytes),
        status: ArtifactStatus::Active,
    };
    write_artifact_index(&artifact_dir, &state.input.task_id, &artifact_ref)?;
    Ok(artifact_ref)
}

fn write_artifact_index(
    artifact_dir: &Path,
    task_id: &str,
    artifact_ref: &ArtifactRef,
) -> Result<(), PlanningUnitError> {
    let index_path = artifact_dir.join("artifact_index.json");
    let mut artifacts = if index_path.exists() {
        std::fs::read(&index_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .and_then(|value| value.get("artifacts").cloned())
            .and_then(|value| value.as_array().cloned())
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    artifacts.retain(|existing| {
        existing.get("artifact_ref_id").and_then(Value::as_str)
            != Some(artifact_ref.artifact_ref_id.as_str())
    });
    artifacts.push(
        serde_json::to_value(artifact_ref)
            .map_err(|error| PlanningUnitError::Serialization(error.to_string()))?,
    );
    std::fs::write(
        &index_path,
        serde_json::to_vec_pretty(&json!({
            "task_id": task_id,
            "artifacts": artifacts,
        }))
        .map_err(|error| PlanningUnitError::Serialization(error.to_string()))?,
    )
    .map_err(|error| PlanningUnitError::Io(format!("write {}: {error}", index_path.display())))?;
    std::fs::write(
        artifact_dir.join("latest.json"),
        serde_json::to_vec_pretty(&json!({
            "active_ref": artifact_ref.artifact_ref_id,
        }))
        .map_err(|error| PlanningUnitError::Serialization(error.to_string()))?,
    )
    .map_err(|error| PlanningUnitError::Io(format!("write latest: {error}")))?;
    Ok(())
}
