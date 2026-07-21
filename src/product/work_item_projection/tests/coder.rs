use super::contract_fixture;
use crate::product::work_item_projection::WorkItemProjectionCompiler;

#[test]
fn work_item_projection_coder_preserves_every_normative_section_exactly() {
    let contract = contract_fixture();
    let compiled = WorkItemProjectionCompiler
        .compile(&contract, "work_item_revision_0001")
        .unwrap();

    assert_eq!(
        compiled.coder.work_item_revision_id,
        "work_item_revision_0001"
    );
    assert_eq!(compiled.coder.objective, contract.goal.summary);
    assert_eq!(
        compiled.coder.required_input_contracts,
        contract.input_contracts
    );
    assert_eq!(
        compiled.coder.task_refs,
        contract
            .tasks
            .iter()
            .map(|task| task.task_id.clone())
            .collect::<Vec<_>>()
    );
    assert_eq!(compiled.coder.tasks, contract.tasks);
    assert_eq!(compiled.coder.write_policy, contract.write_policy);
    assert_eq!(
        compiled.coder.acceptance_criteria,
        contract.acceptance_criteria
    );
    assert_eq!(
        compiled.coder.verification_checks,
        contract.verification_checks
    );
    assert_eq!(compiled.coder.blocker_rules, contract.blocker_rules);
    assert_eq!(compiled.coder.handoff_contract, contract.handoff_contract);
}

#[test]
fn work_item_projection_coder_model_roundtrips_through_serde() {
    let compiled = WorkItemProjectionCompiler
        .compile(&contract_fixture(), "work_item_revision_0001")
        .unwrap();

    let json = serde_json::to_value(&compiled.coder).unwrap();
    let rebuilt = serde_json::from_value(json).unwrap();

    assert_eq!(compiled.coder, rebuilt);
}
