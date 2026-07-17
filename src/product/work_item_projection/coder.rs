use crate::product::work_item_contract::CanonicalWorkItemContract;

use super::CoderWorkItemProjection;

pub(crate) fn compile_coder_projection(
    contract: &CanonicalWorkItemContract,
    work_item_revision_id: &str,
) -> CoderWorkItemProjection {
    CoderWorkItemProjection {
        work_item_revision_id: work_item_revision_id.to_string(),
        objective: contract.goal.summary.clone(),
        required_input_contracts: contract.input_contracts.clone(),
        task_refs: contract
            .tasks
            .iter()
            .map(|task| task.task_id.clone())
            .collect(),
        tasks: contract.tasks.clone(),
        write_policy: contract.write_policy.clone(),
        acceptance_criteria: contract.acceptance_criteria.clone(),
        verification_checks: contract.verification_checks.clone(),
        blocker_rules: contract.blocker_rules.clone(),
        handoff_contract: contract.handoff_contract.clone(),
    }
}
