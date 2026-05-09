use crate::cross_cutting::artifact_validate::{ArtifactContent, canonical_validator};
use crate::cross_cutting::document_ops::{compute_sha256, read_document_model};
use crate::cross_cutting::openspec_constraints::{
    build_openspec_source_manifest, check_bundle_stale, compile_constraint_bundle,
};
use crate::cross_cutting::provider_adapter::{ProviderAdapter, ProviderAdapterError};
use crate::cross_cutting::provider_context_builder::{
    ProviderContextBuildError, ProviderContextBuildResult, ProviderContextBuilderInput,
    build_provider_context,
};
use crate::cross_cutting::provider_run::{
    ProviderRunPersistError, failed_provider_run_record_from_error,
    provider_run_record_from_output, write_provider_run_record,
};
use crate::cross_cutting::runtime_event_log::append_node_event;
use crate::daemon::checkpoint::{RiskRegistrySnapshot, RuntimeSnapshot};
use crate::protocol::artifacts::{ArtifactKind, ArtifactRef, ArtifactStatus};
use crate::protocol::constraints::{BundleStatus, OpenSpecConstraintBundle};
use crate::protocol::contracts::{
    AdapterInput, AdapterOutput, ApprovalPolicy, ProviderRunRecord, SandboxMode,
};
use crate::protocol::enums::{ChangeId, SessionId, TaskId};
use crate::protocol::loop_counters::LoopCounterName;
use crate::protocol::policies::PolicyMode;
use crate::protocol::projections::{ArtifactProjectionRecord, ProjectionPayload};
use crate::protocol::provider_errors::{ProviderErrorRoute, route_provider_error};
use crate::runtime_units::{
    CanonicalNodeInput, DaemonContext, RuntimeProtocolStep, RuntimeStepStatus, RuntimeUnit,
    RuntimeUnitError, RuntimeUnitResult,
};
use chrono::{SecondsFormat, Utc};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const PLANNING_EXECUTION_CHAIN: &[&str] = &[
    "canonical_node_input",
    "projection_or_bundle",
    "provider_context_package",
    "adapter_input",
    "provider_call",
    "provider_run_record",
    "normalize_output",
    "artifact_validate",
    "checkpoint",
];

#[derive(Debug, Clone)]
pub struct PlanningStartChainInput {
    pub session_id: SessionId,
    pub task_id: TaskId,
    pub change_id: ChangeId,
    pub workspace_root: PathBuf,
    pub worktree_path: Option<String>,
    pub intake_brief: Value,
    pub initial_constraint_bundle: OpenSpecConstraintBundle,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlanningNodeTrace {
    pub node_id: String,
    pub execution_chain: Vec<String>,
    pub consumed_constraint_kinds: Vec<String>,
    pub produced_artifact_kinds: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum PlanningUnitError {
    #[error("provider context build failed: {0}")]
    ProviderContext(#[from] ProviderContextBuildError),
    #[error("provider adapter failed: {0}")]
    ProviderAdapter(#[from] ProviderAdapterError),
    #[error("provider run persist failed: {0}")]
    ProviderRunPersist(#[from] ProviderRunPersistError),
    #[error("artifact validation failed: {0:?}")]
    ArtifactValidate(crate::cross_cutting::artifact_validate::ArtifactValidateError),
    #[error("projection compile failed: {0}")]
    ProjectionCompile(crate::cross_cutting::artifact_projection::ProjectionCompileError),
    #[error("OpenSpec operation failed: {0}")]
    OpenSpec(crate::cross_cutting::openspec_constraints::OpenSpecError),
    #[error("io error: {0}")]
    Io(String),
    #[error("serialization failed: {0}")]
    Serialization(String),
    #[error("provider structured output missing for {0}")]
    StructuredOutputMissing(String),
    #[error("provider output is incompatible for {node_id}: expected {expected}, got {got}")]
    IncompatibleOutput {
        node_id: String,
        expected: String,
        got: String,
    },
    #[error("openspec_requirement_constraints_empty")]
    OpenspecRequirementConstraintsEmpty,
    #[error("design_revision_limit_exceeded: current={current} threshold={threshold}")]
    DesignRevisionLimitExceeded { current: u32, threshold: u32 },
}

impl PlanningUnitError {
    pub fn runtime_code(&self) -> String {
        match self {
            PlanningUnitError::OpenspecRequirementConstraintsEmpty => {
                "openspec_requirement_constraints_empty".to_string()
            }
            PlanningUnitError::ProviderAdapter(error) => error.code.as_str().to_string(),
            PlanningUnitError::DesignRevisionLimitExceeded { .. } => {
                "design_revision_limit_exceeded".to_string()
            }
            _ => "planning_unit_error".to_string(),
        }
    }
}

impl From<PlanningUnitError> for RuntimeUnitError {
    fn from(error: PlanningUnitError) -> Self {
        RuntimeUnitError {
            code: error.runtime_code(),
            message: error.to_string(),
        }
    }
}

pub struct PlanningChainState {
    pub input: PlanningStartChainInput,
    pub current_bundle: OpenSpecConstraintBundle,
    pub provider_run_records: Vec<ProviderRunRecord>,
    pub protocol_steps: Vec<RuntimeProtocolStep>,
    pub checkpoint_paths: Vec<PathBuf>,
    pub node_traces: Vec<PlanningNodeTrace>,
    pub superseded_artifact_refs: Vec<String>,
}

impl PlanningChainState {
    pub fn new(input: PlanningStartChainInput) -> Self {
        Self {
            current_bundle: input.initial_constraint_bundle.clone(),
            input,
            provider_run_records: Vec::new(),
            protocol_steps: Vec::new(),
            checkpoint_paths: Vec::new(),
            node_traces: Vec::new(),
            superseded_artifact_refs: Vec::new(),
        }
    }

    pub fn task_root(&self) -> PathBuf {
        self.input
            .workspace_root
            .join(".aria/runtime/tasks")
            .join(&self.input.task_id)
    }

    pub fn openspec_change_dir(&self) -> PathBuf {
        self.input
            .workspace_root
            .join("openspec/changes")
            .join(&self.input.change_id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClarificationUnit;

impl RuntimeUnit for ClarificationUnit {
    fn unit_id(&self) -> &'static str {
        "clarification"
    }

    fn covered_protocol_nodes(&self) -> Vec<&'static str> {
        vec!["N04"]
    }

    async fn execute(
        &self,
        _input: CanonicalNodeInput,
        _ctx: &DaemonContext,
    ) -> Result<RuntimeUnitResult, RuntimeUnitError> {
        Err(RuntimeUnitError {
            code: "provider_adapter_required".to_string(),
            message: "N04 requires ProviderAdapter injection via run_clarification".to_string(),
        })
    }
}

pub fn run_clarification(
    state: &mut PlanningChainState,
    provider: &dyn ProviderAdapter,
) -> Result<(Value, ArtifactRef), PlanningUnitError> {
    let output = run_provider_node(
        state,
        provider,
        "N04",
        json!({
            "intake_brief": state.input.intake_brief,
            "constraint_bundle_ref": state.current_bundle.constraint_bundle_id,
        }),
        "intake brief and proposal constraints",
        vec![],
        proposal_constraint_summary(&state.current_bundle),
        Vec::new(),
    )?;
    let mut record = structured_output("N04", &output)?;
    normalize_clarification_record_candidate(&mut record);
    canonical_validator(
        ArtifactKind::ClarificationRecord,
        &ArtifactContent::Json(record.clone()),
    )
    .map_err(PlanningUnitError::ArtifactValidate)?;
    let artifact_ref =
        write_json_artifact(state, ArtifactKind::ClarificationRecord, "N04", &record)?;
    let checkpoint_path = write_checkpoint(
        state,
        "N04",
        "spec",
        vec![artifact_ref.artifact_ref_id.clone()],
        vec![],
        json!({
            "clarification_ref": artifact_ref.artifact_ref_id,
        }),
    )?;
    record_protocol_step(
        state,
        "N04",
        vec!["proposal_constraints".to_string()],
        vec!["clarification_record".to_string()],
        checkpoint_path,
    );
    Ok((record, artifact_ref))
}

pub fn run_provider_node(
    state: &mut PlanningChainState,
    provider: &dyn ProviderAdapter,
    node_id: &str,
    mut canonical_inputs: Value,
    canonical_input_summary: impl Into<String>,
    projection_refs: Vec<String>,
    constraint_summary: impl Into<String>,
    context_files: Vec<String>,
) -> Result<AdapterOutput, PlanningUnitError> {
    attach_risk_registry_ref(&mut canonical_inputs, &state.input.task_id);
    let build_result = build_provider_context(ProviderContextBuilderInput {
        session_id: state.input.session_id.clone(),
        task_id: state.input.task_id.clone(),
        node_id: node_id.to_string(),
        canonical_inputs,
        canonical_input_summary: canonical_input_summary.into(),
        projection_refs,
        projection_summary: "planning chain projection summary".to_string(),
        constraint_bundle_ref: state.current_bundle.constraint_bundle_id.clone(),
        constraint_summary: constraint_summary.into(),
        context_files,
        worktree_path: state.input.worktree_path.clone(),
    })?;
    let adapter_input = planning_adapter_input_for_node(&build_result)?;
    let request = crate::cross_cutting::provider_router::ProviderRunRequest {
        provider_run_id: provider_run_id(&state.input.task_id, node_id),
        node_id: node_id.to_string(),
        runtime_role: build_result.context_package.runtime_role.clone(),
        provider_capability_ref: "cap_fake_planning_provider_v1".to_string(),
        adapter_compatibility_ref: "compat_fake_planning_provider_v1".to_string(),
        context_package_ref: build_result.context_package.context_package_id.clone(),
        adapter_input_ref: format!("adapter_input_{}_{}", state.input.task_id, node_id),
        adapter_output_ref: format!("adapter_output_{}_{}", state.input.task_id, node_id),
        approval_policy: ApprovalPolicy::OnRequest,
        sandbox_mode: SandboxMode::WorkspaceWrite,
        constraint_check_ref: Some(state.current_bundle.constraint_bundle_id.clone()),
        traceability_binding_refs: Vec::new(),
    };
    append_node_event(
        &state.task_root(),
        &state.input.task_id,
        node_id,
        "node_enter",
        "started",
        json!({
            "provider_run_id": request.provider_run_id.clone(),
            "context_package_ref": build_result.context_package.context_package_id.clone(),
            "output_schema": adapter_input.output_schema.clone(),
        }),
    );
    let protected_openspec_snapshot = snapshot_directory(&state.openspec_change_dir())?;
    let mut retry_count = 0;
    loop {
        match provider.run(&adapter_input) {
            Ok(output) => {
                restore_directory_snapshot(
                    &state.openspec_change_dir(),
                    &protected_openspec_snapshot,
                )?;
                let mut record = provider_run_record_from_output(&request, &adapter_input, &output);
                record.retry_count = retry_count;
                persist_provider_run(state, &record)?;
                append_node_event(
                    &state.task_root(),
                    &state.input.task_id,
                    node_id,
                    "node_exit",
                    "completed",
                    json!({
                        "provider_run_id": record.provider_run_id,
                        "duration_ms": record.duration_ms,
                        "retry_count": retry_count,
                    }),
                );
                return Ok(output);
            }
            Err(error) => {
                restore_directory_snapshot(
                    &state.openspec_change_dir(),
                    &protected_openspec_snapshot,
                )?;
                let route =
                    route_provider_error(&error.code, retry_count, adapter_input.max_retries);
                if route == ProviderErrorRoute::Retry {
                    retry_count += 1;
                    continue;
                }
                let mut record =
                    failed_provider_run_record_from_error(&request, &adapter_input, &error);
                record.retry_count = retry_count;
                persist_provider_run(state, &record)?;
                append_node_event(
                    &state.task_root(),
                    &state.input.task_id,
                    node_id,
                    "node_exit",
                    "failed",
                    json!({
                        "provider_run_id": record.provider_run_id,
                        "error_code": record.error_code,
                        "error_details": record.error_details,
                        "retry_count": retry_count,
                    }),
                );
                return Err(PlanningUnitError::ProviderAdapter(error));
            }
        }
    }
}

pub(crate) fn planning_adapter_input_for_node(
    build_result: &ProviderContextBuildResult,
) -> Result<AdapterInput, PlanningUnitError> {
    Ok(build_result.adapter_input.clone())
}

#[derive(Debug, Clone)]
struct DirectorySnapshot {
    existed: bool,
    files: BTreeMap<PathBuf, Vec<u8>>,
}

fn snapshot_directory(path: &Path) -> Result<DirectorySnapshot, PlanningUnitError> {
    let mut files = BTreeMap::new();
    if !path.exists() {
        return Ok(DirectorySnapshot {
            existed: false,
            files,
        });
    }
    collect_snapshot_files(path, path, &mut files)?;
    Ok(DirectorySnapshot {
        existed: true,
        files,
    })
}

fn collect_snapshot_files(
    root: &Path,
    path: &Path,
    files: &mut BTreeMap<PathBuf, Vec<u8>>,
) -> Result<(), PlanningUnitError> {
    for entry in std::fs::read_dir(path)
        .map_err(|error| PlanningUnitError::Io(format!("read {}: {error}", path.display())))?
    {
        let entry =
            entry.map_err(|error| PlanningUnitError::Io(format!("read dir entry: {error}")))?;
        let path = entry.path();
        if path.is_dir() {
            collect_snapshot_files(root, &path, files)?;
        } else if path.is_file() {
            let relative = path
                .strip_prefix(root)
                .map_err(|error| PlanningUnitError::Io(error.to_string()))?
                .to_path_buf();
            let content = std::fs::read(&path).map_err(|error| {
                PlanningUnitError::Io(format!("read {}: {error}", path.display()))
            })?;
            files.insert(relative, content);
        }
    }
    Ok(())
}

fn restore_directory_snapshot(
    path: &Path,
    snapshot: &DirectorySnapshot,
) -> Result<(), PlanningUnitError> {
    if path.exists() {
        std::fs::remove_dir_all(path).map_err(|error| {
            PlanningUnitError::Io(format!("remove {}: {error}", path.display()))
        })?;
    }
    if !snapshot.existed {
        return Ok(());
    }
    std::fs::create_dir_all(path)
        .map_err(|error| PlanningUnitError::Io(format!("create {}: {error}", path.display())))?;
    for (relative, content) in &snapshot.files {
        let target = path.join(relative);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                PlanningUnitError::Io(format!("create {}: {error}", parent.display()))
            })?;
        }
        std::fs::write(&target, content).map_err(|error| {
            PlanningUnitError::Io(format!("write {}: {error}", target.display()))
        })?;
    }
    Ok(())
}

fn attach_risk_registry_ref(value: &mut Value, task_id: &str) {
    if !value.is_object() {
        *value = json!({ "payload": value.clone() });
    }
    if value.get("risk_registry_ref").is_none() {
        value["risk_registry_ref"] = json!(format!("riskreg_{task_id}_v0001"));
    }
}

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

pub fn write_spec_to_openspec_and_recompile(
    state: &mut PlanningChainState,
    spec_markdown: &str,
    projection_refs: Vec<String>,
) -> Result<(BundleStatus, OpenSpecConstraintBundle), PlanningUnitError> {
    let change_dir = state.openspec_change_dir();
    let spec_path = change_dir.join("specs/main/spec.md");
    let old_content = std::fs::read(&spec_path)
        .map_err(|error| PlanningUnitError::Io(format!("read {}: {error}", spec_path.display())))?;
    let openspec_markdown = canonical_spec_to_openspec(spec_markdown);
    if let Some(parent) = spec_path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            PlanningUnitError::Io(format!("create {}: {error}", parent.display()))
        })?;
    }
    std::fs::write(&spec_path, openspec_markdown.as_bytes()).map_err(|error| {
        PlanningUnitError::Io(format!("write {}: {error}", spec_path.display()))
    })?;
    if let Err(error) = read_document_model(&spec_path) {
        let _ = std::fs::write(&spec_path, &old_content);
        return Err(PlanningUnitError::Io(format!(
            "openspec_patch_invalid_markdown: {error}"
        )));
    }
    let current_manifest = match build_openspec_source_manifest(&change_dir) {
        Ok(manifest) => manifest,
        Err(error) => {
            let _ = std::fs::write(&spec_path, &old_content);
            return Err(PlanningUnitError::OpenSpec(error));
        }
    };
    let stale_status = check_bundle_stale(&state.current_bundle, &current_manifest);
    let bundle = match compile_constraint_bundle(
        &state.input.change_id,
        &current_manifest,
        projection_refs,
        "N06".to_string(),
    ) {
        Ok(bundle) => bundle,
        Err(error) => {
            let _ = std::fs::write(&spec_path, &old_content);
            return Err(PlanningUnitError::OpenSpec(error));
        }
    };
    if bundle.requirement_constraints.requirement_ids.is_empty() {
        let _ = std::fs::write(&spec_path, &old_content);
        return Err(PlanningUnitError::OpenspecRequirementConstraintsEmpty);
    }
    state.current_bundle = bundle.clone();
    Ok((stale_status, bundle))
}

pub fn write_design_to_openspec_and_recompile(
    state: &mut PlanningChainState,
    design_markdown: &str,
    design_projection: &ArtifactProjectionRecord,
    projection_refs: Vec<String>,
) -> Result<(BundleStatus, OpenSpecConstraintBundle), PlanningUnitError> {
    let openspec_markdown = canonical_design_to_openspec(design_markdown, design_projection);
    let bundle = write_openspec_file_and_recompile(
        state,
        "design.md",
        &openspec_markdown,
        projection_refs,
        "N08",
    )?;
    if bundle.1.design_constraints.design_decision_ids.is_empty()
        && bundle.1.design_constraints.component_ids.is_empty()
    {
        return Err(PlanningUnitError::OpenSpec(
            crate::cross_cutting::openspec_constraints::OpenSpecError::DesignConstraintsEmpty,
        ));
    }
    Ok(bundle)
}

pub fn write_tasks_to_openspec_and_recompile(
    state: &mut PlanningChainState,
    tasks_markdown: &str,
    projection_refs: Vec<String>,
) -> Result<(BundleStatus, OpenSpecConstraintBundle), PlanningUnitError> {
    let bundle = write_openspec_file_and_recompile(
        state,
        "tasks.md",
        tasks_markdown,
        projection_refs,
        "N11",
    )?;
    if bundle.1.task_constraints.task_ids.is_empty() {
        return Err(PlanningUnitError::OpenSpec(
            crate::cross_cutting::openspec_constraints::OpenSpecError::TaskConstraintsEmpty,
        ));
    }
    Ok(bundle)
}

pub fn provider_run_id(task_id: &str, node_id: &str) -> String {
    format!("prun_{}_{}", task_id, node_id.to_lowercase())
}

pub fn proposal_constraint_summary(bundle: &OpenSpecConstraintBundle) -> String {
    format!(
        "proposal business_intent={} scope={}",
        bundle.proposal_constraints.business_intent.join(" | "),
        bundle.proposal_constraints.scope.join(" | ")
    )
}

pub fn requirement_constraint_summary(bundle: &OpenSpecConstraintBundle) -> String {
    format!(
        "requirement_ids={}",
        bundle.requirement_constraints.requirement_ids.join(",")
    )
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

fn persist_provider_run(
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

fn write_openspec_file_and_recompile(
    state: &mut PlanningChainState,
    relative_path: &str,
    content: &str,
    projection_refs: Vec<String>,
    compiled_by_node: &str,
) -> Result<(BundleStatus, OpenSpecConstraintBundle), PlanningUnitError> {
    let change_dir = state.openspec_change_dir();
    let target_path = change_dir.join(relative_path);
    let old_content = std::fs::read(&target_path).map_err(|error| {
        PlanningUnitError::Io(format!("read {}: {error}", target_path.display()))
    })?;
    if let Some(parent) = target_path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            PlanningUnitError::Io(format!("create {}: {error}", parent.display()))
        })?;
    }
    std::fs::write(&target_path, content.as_bytes()).map_err(|error| {
        PlanningUnitError::Io(format!("write {}: {error}", target_path.display()))
    })?;
    if let Err(error) = read_document_model(&target_path) {
        let _ = std::fs::write(&target_path, &old_content);
        return Err(PlanningUnitError::Io(format!(
            "openspec_patch_invalid_markdown: {error}"
        )));
    }
    let current_manifest = match build_openspec_source_manifest(&change_dir) {
        Ok(manifest) => manifest,
        Err(error) => {
            let _ = std::fs::write(&target_path, &old_content);
            return Err(PlanningUnitError::OpenSpec(error));
        }
    };
    let stale_status = check_bundle_stale(&state.current_bundle, &current_manifest);
    let bundle = match compile_constraint_bundle(
        &state.input.change_id,
        &current_manifest,
        projection_refs,
        compiled_by_node.to_string(),
    ) {
        Ok(bundle) => bundle,
        Err(error) => {
            let _ = std::fs::write(&target_path, &old_content);
            return Err(PlanningUnitError::OpenSpec(error));
        }
    };
    state.current_bundle = bundle.clone();
    Ok((stale_status, bundle))
}

fn canonical_spec_to_openspec(spec_markdown: &str) -> String {
    let requirement_ids = ids_from_markdown(spec_markdown, "REQ-");
    let acceptance_ids = ids_from_markdown(spec_markdown, "AC-");
    let requirement_id = requirement_ids
        .first()
        .cloned()
        .unwrap_or_else(|| "REQ-001".to_string());
    let acceptance_id = acceptance_ids
        .first()
        .cloned()
        .unwrap_or_else(|| "AC-001".to_string());
    format!(
        "# Main Spec\n\n### ADDED Requirements\n\n#### Requirement: {requirement_id} Generated requirement\n\n##### Scenario: SCN-001 Generated scenario\n\n- WHEN the accepted planning input is processed\n- THEN the runtime satisfies the canonical spec [{acceptance_id}]\n"
    )
}

fn canonical_design_to_openspec(
    design_markdown: &str,
    design_projection: &ArtifactProjectionRecord,
) -> String {
    let ProjectionPayload::DesignProjection(design) = &design_projection.payload else {
        return design_markdown.to_string();
    };
    let mut output = String::from("# Design\n\n## 设计决策\n\n");
    if design.design_decisions.is_empty() {
        output.push_str("- [DEC-001] Generated design decision.\n");
    } else {
        for decision in &design.design_decisions {
            output.push_str("- [");
            output.push_str(&openspec_id(&decision.design_decision_id));
            output.push_str("] ");
            output.push_str(&single_line(&decision.text));
            output.push('\n');
        }
    }

    output.push_str("\n## 公共组件\n\n");
    if design.shared_components.is_empty() && design.shared_modules.is_empty() {
        output.push_str("- [CMP-001] Generated component.\n");
    } else {
        for component in design
            .shared_components
            .iter()
            .chain(design.shared_modules.iter())
        {
            output.push_str("- [");
            output.push_str(&openspec_id(&component.component_id));
            output.push_str("] ");
            output.push_str(&single_line(&component.name));
            if !component.responsibility.trim().is_empty() {
                output.push_str(": ");
                output.push_str(&single_line(&component.responsibility));
            }
            output.push('\n');
        }
    }

    output.push_str("\n## 风险\n\n");
    if design.risk_refs.is_empty() {
        output.push_str("- [RISK-001] Generated risk.\n");
    } else {
        for risk in &design.risk_refs {
            output.push_str("- [");
            output.push_str(&openspec_id(&risk.risk_id));
            output.push_str("] ");
            output.push_str(&single_line(&risk.text));
            output.push('\n');
        }
    }

    output
}

fn openspec_id(value: &str) -> String {
    value.to_ascii_uppercase()
}

fn single_line(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn ids_from_markdown(markdown: &str, prefix: &str) -> Vec<String> {
    let normalized = markdown
        .replace(['[', ']', '(', ')', ',', ';', ':', '.', '`'], " ")
        .replace('\t', " ");
    let mut ids = Vec::new();
    for token in normalized.split_whitespace() {
        let trimmed = token
            .trim_matches(|character: char| !character.is_ascii_alphanumeric() && character != '-');
        if trimmed.starts_with(prefix) && !ids.iter().any(|existing| existing == trimmed) {
            ids.push(trimmed.to_string());
        }
    }
    ids
}

fn now_iso8601() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}
