use crate::cross_cutting::artifact_validate::{ArtifactContent, canonical_validator};
use crate::cross_cutting::provider_adapter::ProviderAdapter;
use crate::protocol::artifacts::{ArtifactKind, ArtifactRef};
use crate::protocol::constraints::{BundleStatus, OpenSpecConstraintBundle};
use crate::protocol::projections::ArtifactProjectionRecord;
use crate::runtime_units::clarification::{
    PlanningChainState, PlanningUnitError, record_protocol_step, requirement_constraint_summary,
    run_provider_node, structured_output, write_checkpoint, write_design_to_openspec_and_recompile,
    write_json_artifact,
};
use crate::runtime_units::{
    CanonicalNodeInput, DaemonContext, RuntimeUnit, RuntimeUnitError, RuntimeUnitResult,
};
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DesignReviewUnit;

impl RuntimeUnit for DesignReviewUnit {
    fn unit_id(&self) -> &'static str {
        "design_review"
    }

    fn covered_protocol_nodes(&self) -> Vec<&'static str> {
        vec!["N08"]
    }

    async fn execute(
        &self,
        _input: CanonicalNodeInput,
        _ctx: &DaemonContext,
    ) -> Result<RuntimeUnitResult, RuntimeUnitError> {
        Err(RuntimeUnitError {
            code: "provider_adapter_required".to_string(),
            message: "N08 requires ProviderAdapter injection via run_design_review".to_string(),
        })
    }
}

#[derive(Debug, Clone)]
pub enum DesignReviewRoute {
    Pass {
        review: Value,
        review_ref: ArtifactRef,
        stale_status: BundleStatus,
        bundle_after_design: OpenSpecConstraintBundle,
    },
    Revise {
        review: Value,
        review_ref: ArtifactRef,
    },
}

pub fn run_design_review(
    state: &mut PlanningChainState,
    provider: &dyn ProviderAdapter,
    spec_projection: &ArtifactProjectionRecord,
    design_markdown: &str,
    design_projection: &ArtifactProjectionRecord,
) -> Result<DesignReviewRoute, PlanningUnitError> {
    let canonical_input_summary =
        design_review_canonical_input_summary(spec_projection, design_markdown, design_projection)?;
    let output = run_provider_node(
        state,
        provider,
        "N08",
        json!({
            "spec_projection_ref": spec_projection.projection_id,
            "spec_projection_payload": spec_projection.payload,
            "design_markdown": design_markdown,
            "design_projection_ref": design_projection.projection_id,
            "design_projection_payload": design_projection.payload,
            "constraint_bundle_ref": state.current_bundle.constraint_bundle_id,
        }),
        canonical_input_summary,
        vec![
            spec_projection.projection_id.clone(),
            design_projection.projection_id.clone(),
        ],
        requirement_constraint_summary(&state.current_bundle),
        Vec::new(),
    )?;
    let mut review = structured_output("N08", &output)?;
    normalize_review_decision_aliases(&mut review);
    canonical_validator(
        ArtifactKind::DesignReview,
        &ArtifactContent::Json(review.clone()),
    )
    .map_err(PlanningUnitError::ArtifactValidate)?;
    let decision = review
        .get("review_decision")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !matches!(decision, "pass" | "conditional_pass" | "revise" | "fail") {
        return Err(PlanningUnitError::IncompatibleOutput {
            node_id: "N08".to_string(),
            expected: "pass|conditional_pass|revise|fail".to_string(),
            got: decision.to_string(),
        });
    }
    let review_ref = write_json_artifact(state, ArtifactKind::DesignReview, "N08", &review)?;
    if matches!(decision, "pass" | "conditional_pass") {
        let (stale_status, bundle_after_design) = write_design_to_openspec_and_recompile(
            state,
            design_markdown,
            design_projection,
            vec![
                spec_projection.projection_id.clone(),
                design_projection.projection_id.clone(),
            ],
        )?;
        let checkpoint_path = write_checkpoint(
            state,
            "N08",
            "planning",
            vec![review_ref.artifact_ref_id.clone()],
            vec![design_projection.projection_id.clone()],
            json!({
                "design_review_ref": review_ref.artifact_ref_id,
                "review_decision": decision,
                "openspec_bundle_ref": bundle_after_design.constraint_bundle_id,
                "stale_status_after_design_write": stale_status,
            }),
        )?;
        record_protocol_step(
            state,
            "N08",
            vec!["requirement_constraints".to_string()],
            vec![
                "design_review".to_string(),
                "openspec_design_writeback".to_string(),
                "openspec_constraint_bundle".to_string(),
            ],
            checkpoint_path,
        );
        Ok(DesignReviewRoute::Pass {
            review,
            review_ref,
            stale_status,
            bundle_after_design,
        })
    } else {
        let checkpoint_path = write_checkpoint(
            state,
            "N08",
            "design",
            vec![review_ref.artifact_ref_id.clone()],
            vec![design_projection.projection_id.clone()],
            json!({
                "design_review_ref": review_ref.artifact_ref_id,
                "review_decision": decision,
                "next_node": "N09",
            }),
        )?;
        record_protocol_step(
            state,
            "N08",
            vec!["requirement_constraints".to_string()],
            vec!["design_review".to_string()],
            checkpoint_path,
        );
        Ok(DesignReviewRoute::Revise { review, review_ref })
    }
}

fn design_review_canonical_input_summary(
    spec_projection: &ArtifactProjectionRecord,
    design_markdown: &str,
    design_projection: &ArtifactProjectionRecord,
) -> Result<String, PlanningUnitError> {
    let spec_projection_payload = serde_json::to_string_pretty(&spec_projection.payload)
        .map_err(|error| PlanningUnitError::Serialization(error.to_string()))?;
    let design_projection_payload = serde_json::to_string_pretty(&design_projection.payload)
        .map_err(|error| PlanningUnitError::Serialization(error.to_string()))?;
    Ok(format!(
        "只评审以下 Aria runtime canonical inputs；OpenSpec design.md 可能尚未 writeback，不得以 worktree 中的占位 design.md 作为评审真相。\n\n[spec_projection_ref]\n{}\n\n[spec_projection_payload]\n{}\n\n[design_markdown]\n{}\n\n[design_projection_ref]\n{}\n\n[design_projection_payload]\n{}",
        spec_projection.projection_id,
        spec_projection_payload,
        design_markdown,
        design_projection.projection_id,
        design_projection_payload
    ))
}

fn normalize_review_decision_aliases(review: &mut Value) {
    let Some(decision) = review
        .get("review_decision")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
    else {
        return;
    };
    let normalized = match decision.as_str() {
        "changes_requested" | "change_requested" | "request_changes" | "needs_changes" => {
            Some("fail")
        }
        _ => None,
    };
    if let Some(normalized) = normalized {
        review["review_decision"] = json!(normalized);
        review["normalized_from_review_decision"] = json!(decision);
    }
}
