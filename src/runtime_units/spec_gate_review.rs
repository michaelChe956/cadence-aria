pub use crate::runtime_units::clarification::PlanningStartChainInput;

use crate::cross_cutting::artifact_validate::{canonical_validator, ArtifactContent};
use crate::cross_cutting::provider_adapter::ProviderAdapter;
use crate::protocol::artifacts::{ArtifactKind, ArtifactRef};
use crate::protocol::constraints::{BundleStatus, OpenSpecConstraintBundle};
use crate::protocol::contracts::ProviderRunRecord;
use crate::protocol::projections::ArtifactProjectionRecord;
use crate::runtime_units::clarification::{
    proposal_constraint_summary, record_protocol_step, run_clarification, run_provider_node,
    structured_output, write_checkpoint, write_json_artifact, write_spec_to_openspec_and_recompile,
    PlanningChainState, PlanningNodeTrace, PlanningUnitError,
};
use crate::runtime_units::design_authoring::run_design_authoring;
use crate::runtime_units::spec_authoring::run_spec_authoring;
use crate::runtime_units::{
    CanonicalNodeInput, DaemonContext, RuntimeProtocolStep, RuntimeUnit, RuntimeUnitError,
    RuntimeUnitResult,
};
use serde_json::{json, Value};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpecGateReviewUnit;

impl RuntimeUnit for SpecGateReviewUnit {
    fn unit_id(&self) -> &'static str {
        "spec_gate_review"
    }

    fn covered_protocol_nodes(&self) -> Vec<&'static str> {
        vec!["N06"]
    }

    async fn execute(
        &self,
        _input: CanonicalNodeInput,
        _ctx: &DaemonContext,
    ) -> Result<RuntimeUnitResult, RuntimeUnitError> {
        Err(RuntimeUnitError {
            code: "provider_adapter_required".to_string(),
            message: "N06 requires ProviderAdapter injection via run_spec_gate_review".to_string(),
        })
    }
}

#[derive(Debug, Clone)]
pub struct PlanningStartChainResult {
    pub protocol_steps: Vec<RuntimeProtocolStep>,
    pub provider_run_records: Vec<ProviderRunRecord>,
    pub checkpoint_paths: Vec<PathBuf>,
    pub node_traces: Vec<PlanningNodeTrace>,
    pub clarification_record: Value,
    pub spec_markdown: String,
    pub spec_ref: ArtifactRef,
    pub spec_projection: ArtifactProjectionRecord,
    pub spec_gate_decision: Value,
    pub spec_gate_decision_ref: ArtifactRef,
    pub spec_writeback_stale_status: BundleStatus,
    pub openspec_bundle_after_spec: OpenSpecConstraintBundle,
    pub design_markdown: String,
    pub design_ref: ArtifactRef,
    pub design_projection: ArtifactProjectionRecord,
}

pub fn run_planning_start_chain(
    input: PlanningStartChainInput,
    provider: &dyn ProviderAdapter,
) -> Result<PlanningStartChainResult, PlanningUnitError> {
    let mut state = PlanningChainState::new(input);
    let (clarification_record, _clarification_ref) = run_clarification(&mut state, provider)?;
    let (spec_markdown, spec_ref, spec_projection) =
        run_spec_authoring(&mut state, provider, &clarification_record)?;
    let (spec_gate_decision, spec_gate_decision_ref, stale_status, bundle_after_spec) =
        run_spec_gate_review(
            &mut state,
            provider,
            &clarification_record,
            &spec_markdown,
            &spec_projection,
        )?;
    let (design_markdown, design_ref, design_projection) = run_design_authoring(
        &mut state,
        provider,
        &spec_markdown,
        &spec_gate_decision,
        spec_projection.projection_id.clone(),
    )?;

    Ok(PlanningStartChainResult {
        protocol_steps: state.protocol_steps,
        provider_run_records: state.provider_run_records,
        checkpoint_paths: state.checkpoint_paths,
        node_traces: state.node_traces,
        clarification_record,
        spec_markdown,
        spec_ref,
        spec_projection,
        spec_gate_decision,
        spec_gate_decision_ref,
        spec_writeback_stale_status: stale_status,
        openspec_bundle_after_spec: bundle_after_spec,
        design_markdown,
        design_ref,
        design_projection,
    })
}

pub fn run_spec_gate_review(
    state: &mut PlanningChainState,
    provider: &dyn ProviderAdapter,
    clarification_record: &Value,
    spec_markdown: &str,
    spec_projection: &ArtifactProjectionRecord,
) -> Result<(Value, ArtifactRef, BundleStatus, OpenSpecConstraintBundle), PlanningUnitError> {
    let output = run_provider_node(
        state,
        provider,
        "N06",
        json!({
            "spec": spec_markdown,
            "clarification_record": clarification_record,
            "constraint_bundle_ref": state.current_bundle.constraint_bundle_id,
        }),
        "spec, clarification record, and proposal constraints",
        vec![spec_projection.projection_id.clone()],
        proposal_constraint_summary(&state.current_bundle),
        Vec::new(),
    )?;
    let advisory = structured_output("N06", &output)?;
    let decision = daemon_spec_gate_decision(&state.current_bundle, spec_projection, &advisory)?;
    canonical_validator(
        ArtifactKind::SpecGateDecision,
        &ArtifactContent::Json(decision.clone()),
    )
    .map_err(PlanningUnitError::ArtifactValidate)?;
    let artifact_ref =
        write_json_artifact(state, ArtifactKind::SpecGateDecision, "N06", &decision)?;
    let (stale_status, bundle_after_spec) = write_spec_to_openspec_and_recompile(
        state,
        spec_markdown,
        vec![spec_projection.projection_id.clone()],
    )?;
    let checkpoint_path = write_checkpoint(
        state,
        "N06",
        "design",
        vec![artifact_ref.artifact_ref_id.clone()],
        vec![spec_projection.projection_id.clone()],
        json!({
            "spec_gate_decision_ref": artifact_ref.artifact_ref_id,
            "openspec_bundle_ref": bundle_after_spec.constraint_bundle_id,
            "stale_status_after_spec_write": stale_status,
        }),
    )?;
    record_protocol_step(
        state,
        "N06",
        vec!["proposal_constraints".to_string()],
        vec![
            "spec_gate_decision".to_string(),
            "openspec_spec_writeback".to_string(),
            "openspec_constraint_bundle".to_string(),
        ],
        checkpoint_path,
    );
    Ok((decision, artifact_ref, stale_status, bundle_after_spec))
}

fn daemon_spec_gate_decision(
    bundle: &OpenSpecConstraintBundle,
    spec_projection: &ArtifactProjectionRecord,
    advisory: &Value,
) -> Result<Value, PlanningUnitError> {
    if bundle.proposal_constraints.business_intent.is_empty()
        || bundle.proposal_constraints.scope.is_empty()
    {
        return Ok(json!({
            "artifact_kind": "spec_gate_decision",
            "decision": "backtrack",
            "review_notes": ["proposal_constraints_empty"],
            "advisory": advisory,
        }));
    }
    if spec_projection.payload.is_empty() {
        return Ok(json!({
            "artifact_kind": "spec_gate_decision",
            "decision": "backtrack",
            "review_notes": ["spec_projection_empty"],
            "advisory": advisory,
        }));
    }
    Ok(json!({
        "artifact_kind": "spec_gate_decision",
        "decision": "pass",
        "review_notes": [],
        "advisory": advisory,
    }))
}
