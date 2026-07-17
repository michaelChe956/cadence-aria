use std::collections::BTreeSet;

use crate::product::work_item_contract::{BlockerRoute, CanonicalWorkItemContract};

use super::{ReviewerRequirementCheck, ReviewerWorkItemProjection};

pub(crate) fn compile_reviewer_projection(
    contract: &CanonicalWorkItemContract,
    work_item_revision_id: &str,
) -> ReviewerWorkItemProjection {
    let requirement_matrix = contract
        .acceptance_criteria
        .iter()
        .map(|criterion| ReviewerRequirementCheck {
            criterion_id: criterion.criterion_id.clone(),
            requirement_refs: contract
                .tasks
                .iter()
                .filter(|task| task.done_when_refs.contains(&criterion.criterion_id))
                .flat_map(|task| task.requirement_refs.iter().cloned())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
            required_evidence: criterion.required_evidence.clone(),
            failure_route: BlockerRoute::CoderRework,
        })
        .collect();

    ReviewerWorkItemProjection {
        work_item_revision_id: work_item_revision_id.to_string(),
        criterion_refs: contract
            .acceptance_criteria
            .iter()
            .map(|criterion| criterion.criterion_id.clone())
            .collect(),
        requirement_matrix,
        scope_policy: contract.write_policy.clone(),
        input_contract_checks: contract.input_contracts.clone(),
        output_contract_checks: contract.output_contracts.clone(),
        verification_evidence_rules: contract.verification_checks.clone(),
        blocker_routing: contract.blocker_rules.clone(),
    }
}
