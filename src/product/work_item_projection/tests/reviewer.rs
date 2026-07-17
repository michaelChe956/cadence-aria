use super::contract_fixture;
use crate::product::work_item_contract::BlockerRoute;
use crate::product::work_item_projection::WorkItemProjectionCompiler;

#[test]
fn work_item_projection_reviewer_covers_requirements_contracts_scope_and_routing() {
    let contract = contract_fixture();
    let compiled = WorkItemProjectionCompiler
        .compile(&contract, "work_item_revision_0001")
        .unwrap();

    assert_eq!(
        compiled.reviewer.work_item_revision_id,
        "work_item_revision_0001"
    );
    assert_eq!(
        compiled.reviewer.criterion_refs,
        contract
            .acceptance_criteria
            .iter()
            .map(|criterion| criterion.criterion_id.clone())
            .collect::<Vec<_>>()
    );
    assert_eq!(compiled.reviewer.scope_policy, contract.write_policy);
    assert_eq!(
        compiled.reviewer.input_contract_checks,
        contract.input_contracts
    );
    assert_eq!(
        compiled.reviewer.output_contract_checks,
        contract.output_contracts
    );
    assert_eq!(
        compiled.reviewer.verification_evidence_rules,
        contract.verification_checks
    );
    assert_eq!(compiled.reviewer.blocker_routing, contract.blocker_rules);

    let check = &compiled.reviewer.requirement_matrix[0];
    assert_eq!(
        check.criterion_id,
        contract.acceptance_criteria[0].criterion_id
    );
    assert_eq!(check.requirement_refs, contract.tasks[0].requirement_refs);
    assert_eq!(
        check.required_evidence,
        contract.acceptance_criteria[0].required_evidence
    );
    assert_eq!(check.failure_route, BlockerRoute::CoderRework);
}

#[test]
fn work_item_projection_reviewer_model_roundtrips_through_serde() {
    let compiled = WorkItemProjectionCompiler
        .compile(&contract_fixture(), "work_item_revision_0001")
        .unwrap();

    let json = serde_json::to_value(&compiled.reviewer).unwrap();
    let rebuilt = serde_json::from_value(json).unwrap();

    assert_eq!(compiled.reviewer, rebuilt);
}
