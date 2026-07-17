use std::collections::BTreeSet;

use super::{compiled_fixture, compiled_plan_fixture, contract_fixture};
use crate::product::models::HumanPresentationRevision;
use crate::product::work_item_projection::{
    HumanPresentationBase, PlanProjectionCompileInput, PlanProjectionCompiler,
    ProjectionCompileError, WorkItemProjectionCompiler, projection_hashes,
    validate_human_presentation_revision,
};

#[test]
fn work_item_projection_human_is_informative_and_uses_only_explicit_source_refs() {
    let contract = contract_fixture();
    let compiled = WorkItemProjectionCompiler
        .compile(&contract, "work_item_revision_0001")
        .unwrap();

    assert_eq!(
        compiled.human.logical_work_item_id,
        contract.identity.logical_work_item_id
    );
    assert_eq!(compiled.human.title, contract.identity.title);
    assert_eq!(compiled.human.goal, contract.goal.summary);
    assert_eq!(compiled.human.non_goals, contract.non_goals);
    assert_eq!(compiled.human.dependencies, vec!["wi_upstream"]);
    assert_eq!(
        compiled.human.inputs[0].source_refs,
        vec!["contract.source"]
    );
    assert_eq!(
        compiled.human.outputs[0].source_refs,
        vec!["contract.canonical"]
    );
    assert_eq!(
        compiled.human.completion_summary,
        contract
            .acceptance_criteria
            .iter()
            .map(|criterion| criterion.statement.clone())
            .collect::<Vec<_>>()
    );
    assert!(!compiled.human.normative);
    assert!(!compiled.human.used_by_provider);

    let expected_refs = contract
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
        .collect::<Vec<_>>();
    assert_eq!(compiled.human.source_refs, expected_refs);
}

#[test]
fn work_item_projection_human_presentation_does_not_change_provider_hashes() {
    let compiled = compiled_fixture();
    let before = projection_hashes(&compiled).unwrap();
    let presentation = work_item_presentation(&compiled.human.logical_work_item_id, vec![]);

    validate_human_presentation_revision(
        HumanPresentationBase::WorkItem(&compiled.human),
        &presentation,
    )
    .unwrap();

    assert_eq!(projection_hashes(&compiled).unwrap(), before);
    assert!(!presentation.normative);
    assert!(!presentation.used_by_provider);
}

#[test]
fn work_item_projection_human_presentation_validates_flags_bindings_and_source_refs() {
    let compiled = compiled_fixture();
    let known_ref = compiled.human.source_refs[0].clone();
    let valid = work_item_presentation(
        &compiled.human.logical_work_item_id,
        vec![known_ref.clone()],
    );
    assert!(
        validate_human_presentation_revision(
            HumanPresentationBase::WorkItem(&compiled.human),
            &valid,
        )
        .is_ok()
    );

    let mut invalid = valid.clone();
    invalid.normative = true;
    assert_invalid_presentation(&compiled.human, &invalid);
    invalid = valid.clone();
    invalid.used_by_provider = true;
    assert_invalid_presentation(&compiled.human, &invalid);
    invalid = valid.clone();
    invalid.source_plan_projection_bundle_id = Some("plan_0001".to_string());
    assert_invalid_presentation(&compiled.human, &invalid);
    invalid = valid.clone();
    invalid.source_work_item_projection_bundle_id = None;
    assert_invalid_presentation(&compiled.human, &invalid);
    invalid = valid.clone();
    invalid.source_work_item_projection_bundle_id = Some("wrong_base".to_string());
    assert_invalid_presentation(&compiled.human, &invalid);
    invalid = valid.clone();
    invalid.source_refs.push(known_ref);
    assert_invalid_presentation(&compiled.human, &invalid);
    invalid = valid;
    invalid.source_refs.push("invented_ref".to_string());
    assert_invalid_presentation(&compiled.human, &invalid);
}

#[test]
fn work_item_projection_plan_human_presentation_requires_matching_plan_binding() {
    let (graph, work_items) = compiled_plan_fixture();
    let plan = PlanProjectionCompiler
        .compile(PlanProjectionCompileInput {
            plan_id: "plan_0001",
            goal: "Compile the plan",
            split_reason: "Explicit dependency boundary",
            source_refs: &["design_0001".to_string()],
            dependency_graph: &graph,
            work_item_projections: &work_items,
        })
        .unwrap();
    let revision = HumanPresentationRevision {
        id: "presentation_0001".to_string(),
        source_plan_projection_bundle_id: Some(plan.human.plan_id.clone()),
        source_work_item_projection_bundle_id: None,
        supersedes: None,
        human_summary: "Plan summary".to_string(),
        why_split: Some("Readable explanation".to_string()),
        dependency_explanation: vec![],
        risk_explanation: vec![],
        source_refs: plan.human.source_refs.clone(),
        normative: false,
        used_by_provider: false,
        created_at: "2026-07-17T00:00:00Z".to_string(),
    };

    assert!(
        validate_human_presentation_revision(HumanPresentationBase::Plan(&plan.human), &revision,)
            .is_ok()
    );
}

fn work_item_presentation(
    logical_work_item_id: &str,
    source_refs: Vec<String>,
) -> HumanPresentationRevision {
    HumanPresentationRevision {
        id: "presentation_0001".to_string(),
        source_plan_projection_bundle_id: None,
        source_work_item_projection_bundle_id: Some(logical_work_item_id.to_string()),
        supersedes: None,
        human_summary: "Readable summary".to_string(),
        why_split: None,
        dependency_explanation: vec![],
        risk_explanation: vec![],
        source_refs,
        normative: false,
        used_by_provider: false,
        created_at: "2026-07-17T00:00:00Z".to_string(),
    }
}

fn assert_invalid_presentation(
    base: &crate::product::work_item_projection::HumanWorkItemProjection,
    revision: &HumanPresentationRevision,
) {
    assert!(matches!(
        validate_human_presentation_revision(HumanPresentationBase::WorkItem(base), revision),
        Err(ProjectionCompileError::InvalidHumanPresentation(_))
    ));
}
