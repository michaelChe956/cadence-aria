use crate::cross_cutting::artifact_projection::compile_design_projection;
use crate::cross_cutting::artifact_validate::{
    ArtifactContent, ArtifactIndex, canonical_validator, projection_validator,
};
use crate::cross_cutting::document_ops::read_document_model;
use crate::cross_cutting::provider_adapter::ProviderAdapter;
use crate::protocol::artifacts::{ArtifactKind, ArtifactRef};
use crate::protocol::projections::ArtifactProjectionRecord;
use crate::runtime_units::clarification::{
    PlanningChainState, PlanningUnitError, record_protocol_step, requirement_constraint_summary,
    run_provider_node, structured_output, write_checkpoint, write_json_artifact,
    write_markdown_artifact,
};
use crate::runtime_units::{
    CanonicalNodeInput, DaemonContext, RuntimeUnit, RuntimeUnitError, RuntimeUnitResult,
};
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DesignRevisionUnit;

impl RuntimeUnit for DesignRevisionUnit {
    fn unit_id(&self) -> &'static str {
        "design_revision"
    }

    fn covered_protocol_nodes(&self) -> Vec<&'static str> {
        vec!["N09"]
    }

    async fn execute(
        &self,
        _input: CanonicalNodeInput,
        _ctx: &DaemonContext,
    ) -> Result<RuntimeUnitResult, RuntimeUnitError> {
        Err(RuntimeUnitError {
            code: "provider_adapter_required".to_string(),
            message: "N09 requires ProviderAdapter injection via run_design_revision".to_string(),
        })
    }
}

#[derive(Debug, Clone)]
pub struct DesignRevisionOutcome {
    pub revision_record: Value,
    pub revision_ref: ArtifactRef,
    pub revised_design_markdown: String,
    pub revised_design_ref: ArtifactRef,
    pub revised_design_projection: ArtifactProjectionRecord,
}

pub fn run_design_revision(
    state: &mut PlanningChainState,
    provider: &dyn ProviderAdapter,
    design_review: &Value,
    previous_design_ref: &ArtifactRef,
    previous_design_projection: &ArtifactProjectionRecord,
) -> Result<DesignRevisionOutcome, PlanningUnitError> {
    let output = run_provider_node(
        state,
        provider,
        "N09",
        json!({
            "design_ref": previous_design_ref.artifact_ref_id.clone(),
            "design_projection_ref": previous_design_projection.projection_id,
            "design_review": design_review,
            "constraint_bundle_ref": state.current_bundle.constraint_bundle_id,
        }),
        "design, design review findings, and requirement constraints",
        vec![previous_design_projection.projection_id.clone()],
        requirement_constraint_summary(&state.current_bundle),
        Vec::new(),
    )?;
    let revision_record = structured_output("N09", &output)?;
    canonical_validator(
        ArtifactKind::DesignRevisionRecord,
        &ArtifactContent::Json(revision_record.clone()),
    )
    .map_err(PlanningUnitError::ArtifactValidate)?;
    let revised_design_markdown = revision_record
        .get("revised_design_markdown")
        .and_then(Value::as_str)
        .ok_or_else(|| PlanningUnitError::IncompatibleOutput {
            node_id: "N09".to_string(),
            expected: "revised_design_markdown".to_string(),
            got: "missing".to_string(),
        })?
        .to_string();
    canonical_validator(
        ArtifactKind::Design,
        &ArtifactContent::Markdown(revised_design_markdown.clone()),
    )
    .map_err(PlanningUnitError::ArtifactValidate)?;
    let revision_ref = write_json_artifact(
        state,
        ArtifactKind::DesignRevisionRecord,
        "N09",
        &revision_record,
    )?;
    let revised_design_ref =
        write_markdown_artifact(state, ArtifactKind::Design, "N09", &revised_design_markdown)?;
    let source = read_document_model(std::path::Path::new(&revised_design_ref.path))
        .map_err(|error| PlanningUnitError::Io(error.to_string()))?;
    let revised_design_projection =
        compile_design_projection(&source, &revised_design_ref, "N09".to_string())
            .map_err(PlanningUnitError::ProjectionCompile)?;
    projection_validator(
        &revised_design_projection,
        &ArtifactIndex::from_active_refs(vec![revised_design_ref.clone()]),
        None,
    )
    .map_err(PlanningUnitError::ArtifactValidate)?;
    write_projection(state, &revised_design_projection)?;
    if !state
        .superseded_artifact_refs
        .contains(&previous_design_ref.artifact_ref_id)
    {
        state
            .superseded_artifact_refs
            .push(previous_design_ref.artifact_ref_id.clone());
    }
    let checkpoint_path = write_checkpoint(
        state,
        "N09",
        "design_review",
        vec![
            revision_ref.artifact_ref_id.clone(),
            revised_design_ref.artifact_ref_id.clone(),
        ],
        vec![revised_design_projection.projection_id.clone()],
        json!({
            "design_revision_record_ref": revision_ref.artifact_ref_id,
            "revised_design_ref": revised_design_ref.artifact_ref_id,
            "revised_design_projection_ref": revised_design_projection.projection_id,
            "superseded_artifact_refs": state.superseded_artifact_refs.clone(),
            "next_node": "N08",
        }),
    )?;
    record_protocol_step(
        state,
        "N09",
        vec!["requirement_constraints".to_string()],
        vec![
            "design_revision_record".to_string(),
            "design".to_string(),
            "design_projection".to_string(),
        ],
        checkpoint_path,
    );
    Ok(DesignRevisionOutcome {
        revision_record,
        revision_ref,
        revised_design_markdown,
        revised_design_ref,
        revised_design_projection,
    })
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
