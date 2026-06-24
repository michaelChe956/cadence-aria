mod artifacts;
mod openspec;
mod provider;
mod snapshot;
mod utils;

pub use artifacts::{
    markdown_from_output, normalize_clarification_record_candidate, record_protocol_step,
    structured_output, write_checkpoint, write_json_artifact, write_markdown_artifact,
};
pub use openspec::{
    write_design_to_openspec_and_recompile, write_spec_to_openspec_and_recompile,
    write_tasks_to_openspec_and_recompile,
};
pub use provider::run_provider_node;
pub use utils::{proposal_constraint_summary, provider_run_id, requirement_constraint_summary};

use crate::cross_cutting::artifact_validate::{ArtifactContent, canonical_validator};
use crate::cross_cutting::provider_adapter::ProviderAdapter;
use crate::protocol::artifacts::{ArtifactKind, ArtifactRef};
use crate::protocol::constraints::OpenSpecConstraintBundle;
use crate::protocol::contracts::ProviderRunRecord;
use crate::protocol::enums::{ChangeId, SessionId, TaskId};
use crate::runtime_units::{
    CanonicalNodeInput, DaemonContext, RuntimeUnit, RuntimeUnitError, RuntimeUnitResult,
};
use serde_json::{Value, json};
use std::path::PathBuf;

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
    ProviderContext(
        #[from] crate::cross_cutting::provider_context_builder::ProviderContextBuildError,
    ),
    #[error("provider adapter failed: {0}")]
    ProviderAdapter(#[from] crate::cross_cutting::provider_adapter::ProviderAdapterError),
    #[error("provider run persist failed: {0}")]
    ProviderRunPersist(#[from] crate::cross_cutting::provider_run::ProviderRunPersistError),
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
    pub protocol_steps: Vec<crate::runtime_units::RuntimeProtocolStep>,
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
