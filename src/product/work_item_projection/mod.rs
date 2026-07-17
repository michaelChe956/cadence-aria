mod coder;
mod human;
mod model;
mod plan;
mod reviewer;
mod validation;

pub use human::validate_human_presentation_revision;
pub use model::*;
pub use plan::PlanProjectionCompiler;
pub use validation::{
    projection_hashes, validate_plan_projection_coverage, validate_projection_coverage,
};

pub use crate::product::models::PlanValidationReportArtifact;

use crate::product::work_item_contract::CanonicalWorkItemContract;

use coder::compile_coder_projection;
use human::compile_human_projection;
use plan::{
    contract_flow, design_traceability_refs, human_work_items, reviewer_work_items,
    risks_from_flow, stable_topological_order,
};
use reviewer::compile_reviewer_projection;

#[derive(Debug, Default)]
pub struct WorkItemProjectionCompiler;

impl WorkItemProjectionCompiler {
    pub fn compile(
        &self,
        contract: &CanonicalWorkItemContract,
        work_item_revision_id: &str,
    ) -> Result<CompiledWorkItemProjections, ProjectionCompileError> {
        let compiled = CompiledWorkItemProjections {
            human: compile_human_projection(contract),
            coder: compile_coder_projection(contract, work_item_revision_id),
            reviewer: compile_reviewer_projection(contract, work_item_revision_id),
        };
        let validation = validate_projection_coverage(contract, work_item_revision_id, &compiled);
        if validation.is_valid() {
            Ok(compiled)
        } else {
            Err(ProjectionCompileError::Validation(validation))
        }
    }
}

#[cfg(test)]
mod tests;
