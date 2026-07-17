use std::collections::BTreeSet;

use crate::product::models::HumanPresentationRevision;
use crate::product::work_item_contract::CanonicalWorkItemContract;

use super::{
    HumanContractSummary, HumanPresentationBase, HumanScopeSummary, HumanWorkItemProjection,
    ProjectionCompileError,
};

pub(crate) fn compile_human_projection(
    contract: &CanonicalWorkItemContract,
) -> HumanWorkItemProjection {
    let inputs = contract
        .input_contracts
        .iter()
        .map(|input| HumanContractSummary {
            contract_id: input.contract_id.clone(),
            capabilities: input.required_capabilities.clone(),
            source_refs: vec![input.contract_id.clone()],
        })
        .collect();
    let outputs = contract
        .output_contracts
        .iter()
        .map(|output| HumanContractSummary {
            contract_id: output.contract_id.clone(),
            capabilities: output.capabilities.clone(),
            source_refs: vec![output.contract_id.clone()],
        })
        .collect();
    let dependencies = contract
        .input_contracts
        .iter()
        .map(|input| input.provider_logical_work_item_id.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let source_refs = contract
        .input_contracts
        .iter()
        .map(|input| input.contract_id.clone())
        .chain(
            contract
                .output_contracts
                .iter()
                .map(|output| output.contract_id.clone()),
        )
        .chain(contract.tasks.iter().map(|task| task.task_id.clone()))
        .chain(
            contract
                .acceptance_criteria
                .iter()
                .map(|criterion| criterion.criterion_id.clone()),
        )
        .chain(
            contract
                .verification_checks
                .iter()
                .map(|check| check.check_id.clone()),
        )
        .chain(
            contract
                .blocker_rules
                .iter()
                .map(|rule| rule.reason_code.clone()),
        )
        .chain(
            contract
                .design_traceability
                .iter()
                .map(|trace| trace.requirement_id.clone()),
        )
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();

    HumanWorkItemProjection {
        logical_work_item_id: contract.identity.logical_work_item_id.clone(),
        title: contract.identity.title.clone(),
        goal: contract.goal.summary.clone(),
        non_goals: contract.non_goals.clone(),
        inputs,
        outputs,
        dependencies,
        scope_summary: HumanScopeSummary {
            owned_scopes: contract.write_policy.exclusive_scopes.clone(),
            forbidden_scopes: contract.write_policy.forbidden_scopes.clone(),
        },
        completion_summary: contract
            .acceptance_criteria
            .iter()
            .map(|criterion| criterion.statement.clone())
            .collect(),
        source_refs,
        normative: false,
        used_by_provider: false,
    }
}

pub fn validate_human_presentation_revision(
    base: HumanPresentationBase<'_>,
    revision: &HumanPresentationRevision,
) -> Result<(), ProjectionCompileError> {
    if revision.normative {
        return invalid("human presentation revision must be non-normative");
    }
    if revision.used_by_provider {
        return invalid("human presentation revision must not be used by providers");
    }

    let (expected_plan_bundle_id, expected_work_item_bundle_id, allowed_source_refs) = match base {
        HumanPresentationBase::Plan {
            projection_bundle_id,
            projection,
        } => (Some(projection_bundle_id), None, &projection.source_refs),
        HumanPresentationBase::WorkItem {
            projection_bundle_id,
            projection,
        } => (None, Some(projection_bundle_id), &projection.source_refs),
    };
    if revision.source_plan_projection_bundle_id.as_deref() != expected_plan_bundle_id
        || revision.source_work_item_projection_bundle_id.as_deref() != expected_work_item_bundle_id
    {
        return invalid("human presentation revision must bind exactly its base projection");
    }

    let allowed = allowed_source_refs
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let mut seen = BTreeSet::new();
    for source_ref in &revision.source_refs {
        if !seen.insert(source_ref.as_str()) {
            return invalid("human presentation revision source_refs must be unique");
        }
        if !allowed.contains(source_ref.as_str()) {
            return invalid("human presentation revision contains an unknown source_ref");
        }
    }

    Ok(())
}

fn invalid<T>(message: &str) -> Result<T, ProjectionCompileError> {
    Err(ProjectionCompileError::InvalidHumanPresentation(
        message.to_string(),
    ))
}
