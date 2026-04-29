use crate::cross_cutting::artifact_projection::compile_spec_projection;
use crate::cross_cutting::artifact_validate::{
    ArtifactContent, ArtifactIndex, canonical_validator, projection_validator,
};
use crate::cross_cutting::document_ops::read_document_model;
use crate::cross_cutting::provider_adapter::ProviderAdapter;
use crate::protocol::artifacts::ArtifactKind;
use crate::protocol::projections::ArtifactProjectionRecord;
use crate::runtime_units::clarification::{
    PlanningChainState, PlanningUnitError, markdown_from_output, proposal_constraint_summary,
    record_protocol_step, run_provider_node, write_checkpoint, write_markdown_artifact,
};
use crate::runtime_units::{
    CanonicalNodeInput, DaemonContext, RuntimeUnit, RuntimeUnitError, RuntimeUnitResult,
};
use serde_json::json;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpecAuthoringUnit;

impl RuntimeUnit for SpecAuthoringUnit {
    fn unit_id(&self) -> &'static str {
        "spec_authoring"
    }

    fn covered_protocol_nodes(&self) -> Vec<&'static str> {
        vec!["N05"]
    }

    async fn execute(
        &self,
        _input: CanonicalNodeInput,
        _ctx: &DaemonContext,
    ) -> Result<RuntimeUnitResult, RuntimeUnitError> {
        Err(RuntimeUnitError {
            code: "provider_adapter_required".to_string(),
            message: "N05 requires ProviderAdapter injection via run_spec_authoring".to_string(),
        })
    }
}

pub fn run_spec_authoring(
    state: &mut PlanningChainState,
    provider: &dyn ProviderAdapter,
    clarification_record: &serde_json::Value,
) -> Result<
    (
        String,
        crate::protocol::artifacts::ArtifactRef,
        ArtifactProjectionRecord,
    ),
    PlanningUnitError,
> {
    let output = run_provider_node(
        state,
        provider,
        "N05",
        json!({
            "intake_brief": state.input.intake_brief.clone(),
            "clarification_record": clarification_record,
            "constraint_bundle_ref": state.current_bundle.constraint_bundle_id,
        }),
        "intake brief, clarification record, and proposal constraints",
        vec![],
        proposal_constraint_summary(&state.current_bundle),
        Vec::new(),
    )?;
    let markdown = markdown_from_output("N05", &output, "spec")?;
    canonical_validator(
        ArtifactKind::Spec,
        &ArtifactContent::Markdown(markdown.clone()),
    )
    .map_err(PlanningUnitError::ArtifactValidate)?;
    let artifact_ref = write_markdown_artifact(state, ArtifactKind::Spec, "N05", &markdown)?;
    let source = read_document_model(std::path::Path::new(&artifact_ref.path))
        .map_err(|error| PlanningUnitError::Io(error.to_string()))?;
    let projection = compile_spec_projection(&source, &artifact_ref, "N05".to_string())
        .map_err(PlanningUnitError::ProjectionCompile)?;
    projection_validator(
        &projection,
        &ArtifactIndex::from_active_refs(vec![artifact_ref.clone()]),
        None,
    )
    .map_err(PlanningUnitError::ArtifactValidate)?;
    write_projection(state, &projection)?;
    let checkpoint_path = write_checkpoint(
        state,
        "N05",
        "spec_review",
        vec![artifact_ref.artifact_ref_id.clone()],
        vec![projection.projection_id.clone()],
        json!({
            "spec_ref": artifact_ref.artifact_ref_id,
            "spec_projection_ref": projection.projection_id,
        }),
    )?;
    record_protocol_step(
        state,
        "N05",
        vec!["proposal_constraints".to_string()],
        vec!["spec".to_string(), "spec_projection".to_string()],
        checkpoint_path,
    );
    Ok((markdown, artifact_ref, projection))
}

fn write_projection(
    state: &PlanningChainState,
    projection: &ArtifactProjectionRecord,
) -> Result<(), PlanningUnitError> {
    let dir = state.task_root().join("projections");
    std::fs::create_dir_all(&dir)
        .map_err(|error| PlanningUnitError::Io(format!("create projections dir: {error}")))?;
    let path = dir.join(format!("{}.json", projection.projection_id));
    let bytes = serde_json::to_vec_pretty(projection)
        .map_err(|error| PlanningUnitError::Serialization(error.to_string()))?;
    std::fs::write(&path, bytes)
        .map_err(|error| PlanningUnitError::Io(format!("write {}: {error}", path.display())))
}
