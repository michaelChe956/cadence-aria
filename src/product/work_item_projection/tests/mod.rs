use std::collections::BTreeMap;

use crate::product::work_item_contract::{
    AcceptanceCriterion, BlockerRoute, BlockerRule, CanonicalWorkItemContract,
    ContractCompatibilityPolicy, DependencyContractGraph, EvidenceKind, PromisedOutputContract,
    RequiredInputContract, VerificationCheck, WorkItemTask, build_dependency_contract_graph,
    canonical_contract_fixture,
};
use crate::product::work_item_projection::{
    CoderExecutionEnvelope, CompiledWorkItemProjections, ReviewerExecutionEnvelope,
    WorkItemProjectionCompiler,
};

mod coder;
mod human;
mod plan;
mod render_coder;
mod render_hash;
mod render_reviewer;
mod reviewer;
mod validation;

pub(super) fn contract_fixture() -> CanonicalWorkItemContract {
    canonical_contract_fixture("wi_consumer")
}

pub(super) fn compiled_fixture() -> CompiledWorkItemProjections {
    WorkItemProjectionCompiler
        .compile(&contract_fixture(), "work_item_revision_0001")
        .unwrap()
}

pub(super) fn coder_execution_envelope_fixture() -> CoderExecutionEnvelope {
    CoderExecutionEnvelope {
        repository_state_ref: "repository_state_0001".to_string(),
        resolved_handoff_revision_ids: vec![
            "handoff_revision_0001".to_string(),
            "handoff_revision_0002".to_string(),
        ],
        unit_run_id: "unit_run_0001".to_string(),
        previous_actionable_review: Some("Address finding review_finding_0001".to_string()),
        start_commit: Some("1111111111111111111111111111111111111111".to_string()),
    }
}

pub(super) fn reviewer_execution_envelope_fixture() -> ReviewerExecutionEnvelope {
    ReviewerExecutionEnvelope {
        unit_run_id: "unit_run_0001".to_string(),
        diff_ref: "diff_ref_0001".to_string(),
        test_evidence_refs: vec![
            "test_evidence_0001".to_string(),
            "test_evidence_0002".to_string(),
        ],
        handoff_revision_ids: vec!["handoff_revision_0001".to_string()],
        contract_delta_refs: vec!["contract_delta_0001".to_string()],
        completion_commit: "2222222222222222222222222222222222222222".to_string(),
    }
}

pub(super) fn large_contract_fixture() -> CanonicalWorkItemContract {
    let mut contract = contract_fixture();
    contract.tasks.clear();
    contract.acceptance_criteria.clear();
    contract.verification_checks.clear();
    contract.blocker_rules.clear();
    contract.input_contracts.clear();
    contract.output_contracts.clear();
    contract.handoff_contract.provided_contract_refs.clear();
    contract.handoff_contract.reviewer_check_refs.clear();

    for index in 0..48 {
        let task_id = format!("task_large_{index:03}");
        let criterion_id = format!("AC-LARGE-{index:03}");
        let requirement_id = format!("REQ-LARGE-{index:03}");
        let check_id = format!("check_large_{index:03}");
        let blocker_id = format!("blocker_large_{index:03}");
        let input_contract_id = format!("contract.input.large.{index:03}");
        let output_contract_id = format!("contract.output.large.{index:03}");

        contract.input_contracts.push(RequiredInputContract {
            contract_id: input_contract_id.clone(),
            provider_logical_work_item_id: format!("wi_provider_{index:03}"),
            required_capabilities: vec![format!("input_capability_{index:03}")],
            compatibility_policy: ContractCompatibilityPolicy::RequireAll,
        });
        contract.output_contracts.push(PromisedOutputContract {
            contract_id: output_contract_id.clone(),
            capabilities: vec![format!("output_capability_{index:03}")],
        });
        contract.tasks.push(WorkItemTask {
            task_id,
            statement: format!("Implement large fixture task {index:03}"),
            requirement_refs: vec![requirement_id],
            done_when_refs: vec![criterion_id.clone()],
        });
        contract.acceptance_criteria.push(AcceptanceCriterion {
            criterion_id: criterion_id.clone(),
            statement: format!("Large fixture criterion {index:03} is satisfied"),
            required_evidence: vec![EvidenceKind::SourceDiff],
        });
        contract.verification_checks.push(VerificationCheck {
            check_id,
            command: Some(format!("verify-large-{index:03}")),
            manual_instruction: None,
            required: true,
            non_zero_test_execution_required: true,
        });
        contract.blocker_rules.push(BlockerRule {
            reason_code: blocker_id,
            route: BlockerRoute::PlanRepairCurrent,
            target_contract_refs: vec![input_contract_id, output_contract_id.clone()],
        });
        contract
            .handoff_contract
            .provided_contract_refs
            .push(output_contract_id);
        contract
            .handoff_contract
            .reviewer_check_refs
            .push(criterion_id);
    }

    contract
}

pub(super) fn ordered_section_bodies(text: &str) -> Vec<(String, String)> {
    let mut sections = Vec::new();
    let mut current_title: Option<String> = None;
    let mut current_body = Vec::new();

    for line in text.lines() {
        if let Some(title) = line.strip_prefix("## ") {
            if let Some(previous_title) = current_title.replace(title.to_string()) {
                sections.push((previous_title, current_body.join("\n").trim().to_string()));
                current_body.clear();
            }
        } else if current_title.is_some() {
            current_body.push(line);
        }
    }
    if let Some(title) = current_title {
        sections.push((title, current_body.join("\n").trim().to_string()));
    }

    sections
}

pub(super) fn compiled_plan_fixture() -> (
    DependencyContractGraph,
    BTreeMap<String, CompiledWorkItemProjections>,
) {
    let mut provider = canonical_contract_fixture("wi_provider");
    provider.identity.title = "Provide shared contract".to_string();
    provider.input_contracts.clear();
    provider.output_contracts = vec![PromisedOutputContract {
        contract_id: "contract.shared".to_string(),
        capabilities: vec!["capability.a".to_string(), "capability.b".to_string()],
    }];
    provider.handoff_contract.provided_contract_refs = vec!["contract.shared".to_string()];

    let mut consumer = canonical_contract_fixture("wi_consumer");
    consumer.identity.title = "Consume shared contract".to_string();
    consumer.input_contracts = vec![RequiredInputContract {
        contract_id: "contract.shared".to_string(),
        provider_logical_work_item_id: "wi_provider".to_string(),
        required_capabilities: vec!["capability.a".to_string()],
        compatibility_policy: ContractCompatibilityPolicy::RequireAll,
    }];
    consumer.output_contracts = vec![PromisedOutputContract {
        contract_id: "contract.consumer".to_string(),
        capabilities: vec!["consumer.ready".to_string()],
    }];
    consumer.handoff_contract.provided_contract_refs.clear();

    let mut independent = canonical_contract_fixture("wi_independent");
    independent.identity.title = "Independent work".to_string();
    independent.input_contracts.clear();
    independent.output_contracts = vec![PromisedOutputContract {
        contract_id: "contract.independent".to_string(),
        capabilities: vec!["independent.ready".to_string()],
    }];
    independent.handoff_contract.provided_contract_refs.clear();

    let graph =
        build_dependency_contract_graph(&[consumer.clone(), provider.clone(), independent.clone()])
            .unwrap();
    let projections = [provider, consumer, independent]
        .into_iter()
        .map(|contract| {
            let logical_id = contract.identity.logical_work_item_id.clone();
            let revision_id = format!("revision_{logical_id}");
            let compiled = WorkItemProjectionCompiler
                .compile(&contract, &revision_id)
                .unwrap();
            (logical_id, compiled)
        })
        .collect();

    (graph, projections)
}

pub(super) fn expected_plan_revision_ids() -> BTreeMap<String, String> {
    BTreeMap::from([
        (
            "wi_consumer".to_string(),
            "revision_wi_consumer".to_string(),
        ),
        (
            "wi_independent".to_string(),
            "revision_wi_independent".to_string(),
        ),
        (
            "wi_provider".to_string(),
            "revision_wi_provider".to_string(),
        ),
    ])
}
