use std::collections::BTreeMap;

use crate::product::models::{
    PlanDefectClass, PlanRepairRequest, PlanRepairRequestStatus, PlanRevisionReason, RepairTarget,
    RepairTargetKind, WorkItemDraftRevision,
};
use crate::product::plan_repair::tests::amendment::plan_repair_engine_fixture;
use crate::product::plan_repair::{
    PlanRepairEngine, SubgraphReplanReadiness, SubgraphReplanRequest,
};
use crate::product::work_item_contract::{
    CanonicalWorkItemContract, ContractCompatibilityPolicy, PromisedOutputContract,
    RequiredInputContract, canonical_contract_fixture,
};

fn authoritative_request(replacement: CanonicalWorkItemContract) -> SubgraphReplanRequest {
    SubgraphReplanRequest {
        plan_id: "work_item_plan_0001".to_string(),
        base_plan_revision_id: "plan_revision_0001".to_string(),
        repair_request_id: "plan_repair_request_0001".to_string(),
        changed_logical_work_item_ids: vec!["wi_core".to_string()],
        replacement_contracts: vec![replacement],
        replacement_mapping: BTreeMap::from([("wi_core".to_string(), vec!["wi_core".to_string()])]),
        story_spec_refs_changed: false,
        design_spec_refs_changed: false,
    }
}

#[test]
fn plan_repair_subgraph_engine_loads_active_graph_and_allocates_publication_identity() {
    let fixture = plan_repair_engine_fixture();
    let active = fixture
        .store
        .get_work_item_revision(&fixture.plan, "wi_core", "work_item_revision_wi_core_0001")
        .unwrap();

    let result = fixture
        .engine
        .replan_subgraph(&authoritative_request(ready_core_contract(
            active.canonical_contract,
        )))
        .unwrap();

    assert_eq!(result.readiness, SubgraphReplanReadiness::PublicationReady);
    assert_eq!(
        result.base_dependency_graph_revision_id,
        "dependency_graph_revision_0001"
    );
    let revision = result.dependency_graph_revision.unwrap();
    assert_eq!(revision.id, "dependency_graph_revision_0002");
    assert_eq!(revision.created_at, "2026-07-18T00:00:03Z");
}

#[test]
fn plan_repair_subgraph_engine_rejects_stale_base_and_request_graph_identity_injection() {
    let fixture = plan_repair_engine_fixture();
    let active = fixture
        .store
        .get_work_item_revision(&fixture.plan, "wi_core", "work_item_revision_wi_core_0001")
        .unwrap();
    let mut stale = authoritative_request(ready_core_contract(active.canonical_contract));
    stale.base_plan_revision_id = "plan_revision_stale".to_string();
    assert!(fixture.engine.replan_subgraph(&stale).is_err());

    let forged = serde_json::json!({
        "plan_id": "work_item_plan_0001",
        "base_plan_revision_id": "plan_revision_0001",
        "repair_request_id": "plan_repair_request_0001",
        "dependency_graph_revision_id": "dependency_graph_forged",
        "changed_logical_work_item_ids": ["wi_core"],
        "replacement_contracts": [],
        "replacement_mapping": {},
        "story_spec_refs_changed": false,
        "design_spec_refs_changed": false
    });
    assert!(serde_json::from_value::<SubgraphReplanRequest>(forged).is_err());
}

#[test]
fn plan_repair_subgraph_engine_uses_store_lineage_not_constructor_snapshot() {
    let fixture = plan_repair_engine_fixture();
    let active = fixture
        .store
        .get_work_item_revision(&fixture.plan, "wi_core", "work_item_revision_wi_core_0001")
        .unwrap();
    let mut stale_constructor_plan = fixture.plan.clone();
    stale_constructor_plan.active_revision_id = Some("plan_revision_forged".to_string());
    let engine = PlanRepairEngine::new(fixture.store.clone(), stale_constructor_plan)
        .with_created_at("2026-07-18T00:00:03Z");

    assert!(
        engine
            .replan_subgraph(&authoritative_request(ready_core_contract(
                active.canonical_contract,
            )))
            .is_ok()
    );
}

#[test]
fn plan_repair_subgraph_prepare_replaces_old_bindings_and_persists_new_logical_items() {
    let mut fixture = plan_repair_engine_fixture();
    fixture.plan = fixture
        .store
        .release_active_amendment(&fixture.plan, "plan_amendment_0001")
        .unwrap();
    fixture.plan = fixture
        .store
        .acquire_active_amendment(&fixture.plan, "plan_amendment_subgraph_0001")
        .unwrap();

    let request = PlanRepairRequest {
        id: "plan_repair_request_subgraph_0001".to_string(),
        plan_id: fixture.plan.id.clone(),
        base_plan_revision_id: "plan_revision_0001".to_string(),
        trigger_attempt_id: "coding_attempt_0001".to_string(),
        trigger_unit_run_id: "coding_unit_run_0001".to_string(),
        trigger_review_id: Some("code_review_0001".to_string()),
        trigger_finding_id: "finding_subgraph_0001".to_string(),
        amendment_id: Some("plan_amendment_subgraph_0001".to_string()),
        defect_class: PlanDefectClass::DependencyGraphInvalid,
        reason_code: "dependency_graph_invalid".to_string(),
        repair_target: RepairTarget {
            kind: RepairTargetKind::Subgraph,
            logical_work_item_ids: vec!["wi_core".to_string(), "wi_registration".to_string()],
            work_item_revision_ids: vec![
                "work_item_revision_wi_core_0001".to_string(),
                "work_item_revision_wi_registration_0001".to_string(),
            ],
        },
        contract_refs: vec!["contract.workflow".to_string()],
        capability_refs: vec!["finalization_failure".to_string()],
        evidence: vec![],
        fingerprint: "fingerprint_subgraph_0001".to_string(),
        status: PlanRepairRequestStatus::InProgress,
        created_at: "2026-07-18T00:00:02Z".to_string(),
        updated_at: "2026-07-18T00:00:02Z".to_string(),
    };
    fixture
        .store
        .put_repair_request(&fixture.plan, &request)
        .unwrap();

    let prepare = split_contract(
        "wi_core_prepare",
        None,
        "contract.workflow_prepare",
        "prepare_ready",
    );
    let finalize = split_contract(
        "wi_core_finalize",
        Some((
            "wi_core_prepare",
            "contract.workflow_prepare",
            "prepare_ready",
        )),
        "contract.workflow",
        "finalization_failure",
    );
    let mut registration = split_contract(
        "wi_registration",
        Some((
            "wi_core_finalize",
            "contract.workflow",
            "finalization_failure",
        )),
        "contract.registration",
        "registration_ready",
    );
    registration.handoff_contract.provided_contract_refs.clear();
    let contracts = vec![prepare, finalize, registration];
    let drafts = contracts
        .iter()
        .map(|contract| WorkItemDraftRevision {
            id: format!(
                "work_item_draft_revision_{}_{}",
                contract.identity.logical_work_item_id,
                if contract.identity.logical_work_item_id == "wi_registration" {
                    "0002"
                } else {
                    "0001"
                }
            ),
            logical_work_item_id: contract.identity.logical_work_item_id.clone(),
            revision_no: if contract.identity.logical_work_item_id == "wi_registration" {
                2
            } else {
                1
            },
            supersedes: (contract.identity.logical_work_item_id == "wi_registration")
                .then(|| "work_item_draft_revision_wi_registration_0001".to_string()),
            revision_reason: PlanRevisionReason::SubgraphReplan,
            canonical_contract_candidate: contract.clone(),
            trigger_repair_request_id: Some(request.id.clone()),
            created_at: "2026-07-18T00:00:03Z".to_string(),
        })
        .collect::<Vec<_>>();
    let subgraph_request = SubgraphReplanRequest {
        plan_id: fixture.plan.id.clone(),
        base_plan_revision_id: request.base_plan_revision_id.clone(),
        repair_request_id: request.id.clone(),
        changed_logical_work_item_ids: vec!["wi_core".to_string()],
        replacement_contracts: contracts,
        replacement_mapping: BTreeMap::from([
            (
                "wi_core".to_string(),
                vec![
                    "wi_core_prepare".to_string(),
                    "wi_core_finalize".to_string(),
                ],
            ),
            (
                "wi_registration".to_string(),
                vec!["wi_registration".to_string()],
            ),
        ]),
        story_spec_refs_changed: false,
        design_spec_refs_changed: false,
    };
    let engine = PlanRepairEngine::new(fixture.store.clone(), fixture.plan.clone())
        .with_candidate_drafts(drafts)
        .with_subgraph_replan_request(subgraph_request)
        .with_created_at("2026-07-18T00:00:03Z");

    let prepared = engine.prepare_amendment(&request).unwrap();

    assert_eq!(
        prepared.subgraph_replan.as_ref().unwrap().readiness,
        SubgraphReplanReadiness::PublicationReady
    );
    assert!(
        !prepared
            .next_plan_revision
            .work_item_bindings
            .contains_key("wi_core")
    );
    assert!(
        prepared
            .next_plan_revision
            .work_item_bindings
            .contains_key("wi_core_prepare")
    );
    assert!(
        prepared
            .next_plan_revision
            .work_item_bindings
            .contains_key("wi_core_finalize")
    );
    assert_eq!(
        prepared.manifest.replacement_units["wi_core"],
        vec!["wi_core_finalize", "wi_core_prepare"]
    );
    engine.persist_candidate(&prepared).unwrap();
    assert!(
        fixture
            .store
            .get_logical_work_item(&fixture.plan, "wi_core_prepare")
            .is_ok()
    );
    assert!(
        fixture
            .store
            .get_logical_work_item(&fixture.plan, "wi_core_finalize")
            .is_ok()
    );
}

fn split_contract(
    logical_id: &str,
    input: Option<(&str, &str, &str)>,
    output_contract_id: &str,
    output_capability: &str,
) -> CanonicalWorkItemContract {
    let mut contract = canonical_contract_fixture(logical_id);
    contract.identity.logical_work_item_id = logical_id.to_string();
    contract.input_contracts = input
        .map(|(provider, contract_id, capability)| {
            vec![RequiredInputContract {
                contract_id: contract_id.to_string(),
                provider_logical_work_item_id: provider.to_string(),
                required_capabilities: vec![capability.to_string()],
                compatibility_policy: ContractCompatibilityPolicy::RequireAll,
            }]
        })
        .unwrap_or_default();
    contract.output_contracts = vec![PromisedOutputContract {
        contract_id: output_contract_id.to_string(),
        capabilities: vec![output_capability.to_string()],
    }];
    contract.handoff_contract.provided_contract_refs = vec![output_contract_id.to_string()];
    contract
}

fn ready_core_contract(mut contract: CanonicalWorkItemContract) -> CanonicalWorkItemContract {
    let output = contract
        .output_contracts
        .iter_mut()
        .find(|output| output.contract_id == "contract.workflow")
        .unwrap();
    if !output
        .capabilities
        .contains(&"finalization_failure".to_string())
    {
        output.capabilities.push("finalization_failure".to_string());
    }
    contract
}
