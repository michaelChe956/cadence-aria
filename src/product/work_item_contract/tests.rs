use std::fmt::Debug;

use serde::{Serialize, de::DeserializeOwned};

use crate::product::models::{
    HumanPresentationRevision, PlanRevisionReason, VerificationPlanRevision, WorkItemDraftRevision,
    WorkItemRevision,
};

use super::{
    AcceptanceCriterion, BlockerRoute, BlockerRule, CanonicalWorkItemContract,
    ContractCompatibilityPolicy, DesignTraceabilityRef, EvidenceKind, HandoffContract,
    PromisedOutputContract, RequiredInputContract, VerificationCheck, WorkItemContractIdentity,
    WorkItemGoal, WorkItemTask, WorkItemWritePolicy, canonical_contract_hash,
};

pub(crate) fn canonical_contract_fixture(logical_work_item_id: &str) -> CanonicalWorkItemContract {
    CanonicalWorkItemContract {
        schema_version: 1,
        identity: WorkItemContractIdentity {
            logical_work_item_id: logical_work_item_id.to_string(),
            title: "Compile canonical contract".to_string(),
            kind: "implementation".to_string(),
        },
        goal: WorkItemGoal {
            summary: "Provide the canonical work item contract".to_string(),
        },
        non_goals: vec!["Compile provider projections".to_string()],
        depends_on: Vec::new(),
        input_contracts: vec![RequiredInputContract {
            contract_id: "contract.source".to_string(),
            provider_logical_work_item_id: "wi_upstream".to_string(),
            required_capabilities: vec!["source_ready".to_string()],
            compatibility_policy: ContractCompatibilityPolicy::RequireAll,
        }],
        output_contracts: vec![PromisedOutputContract {
            contract_id: "contract.canonical".to_string(),
            capabilities: vec!["stable_hash".to_string()],
        }],
        tasks: vec![WorkItemTask {
            task_id: "task_1".to_string(),
            statement: "Define the canonical contract model".to_string(),
            requirement_refs: vec!["REQ-CANONICAL-001".to_string()],
            done_when_refs: vec!["AC-001".to_string()],
        }],
        write_policy: WorkItemWritePolicy {
            exclusive_scopes: vec!["src/product/work_item_contract".to_string()],
            forbidden_scopes: vec!["src/product/projection_compiler".to_string()],
        },
        acceptance_criteria: vec![AcceptanceCriterion {
            criterion_id: "AC-001".to_string(),
            statement: "Contract survives serde roundtrip".to_string(),
            required_evidence: vec![EvidenceKind::SourceDiff, EvidenceKind::NonZeroTestExecution],
        }],
        verification_checks: vec![VerificationCheck {
            check_id: "check_canonical".to_string(),
            command: Some("cargo test --locked --lib canonical_work_item_".to_string()),
            manual_instruction: None,
            required: true,
            non_zero_test_execution_required: true,
        }],
        handoff_contract: HandoffContract {
            required_fields: vec!["commit_sha".to_string()],
            provided_contract_refs: vec!["contract.canonical".to_string()],
            reviewer_check_refs: vec!["AC-001".to_string()],
        },
        blocker_rules: vec![BlockerRule {
            reason_code: "contract_invalid".to_string(),
            route: BlockerRoute::PlanRepairCurrent,
            target_contract_refs: vec!["contract.canonical".to_string()],
        }],
        design_traceability: vec![DesignTraceabilityRef {
            source_type: "design_spec".to_string(),
            source_id: "design_spec_0001".to_string(),
            requirement_id: "REQ-CANONICAL-001".to_string(),
        }],
    }
}

mod dependency;
mod model;
mod validation;

fn assert_enum_cases<T>(cases: impl IntoIterator<Item = (T, &'static str)>)
where
    T: Serialize + DeserializeOwned + PartialEq + Debug,
{
    for (variant, expected) in cases {
        assert_eq!(serde_json::to_value(&variant).unwrap(), expected);
        assert_eq!(
            serde_json::from_str::<T>(&format!("\"{expected}\"")).unwrap(),
            variant
        );
    }
}

fn assert_missing_field_rejected<T>(value: &serde_json::Value, field: &str)
where
    T: DeserializeOwned,
{
    let mut missing = value.clone();
    missing.as_object_mut().unwrap().remove(field);
    assert!(
        serde_json::from_value::<T>(missing).is_err(),
        "missing required field {field} should be rejected"
    );
}

fn contains_object_key(value: &serde_json::Value, key: &str) -> bool {
    match value {
        serde_json::Value::Object(object) => {
            object.contains_key(key) || object.values().any(|value| contains_object_key(value, key))
        }
        serde_json::Value::Array(values) => {
            values.iter().any(|value| contains_object_key(value, key))
        }
        _ => false,
    }
}

#[test]
fn canonical_work_item_contract_roundtrips_without_human_presentation_fields() {
    let contract = canonical_contract_fixture("wi_core");
    let value = serde_json::to_value(&contract).unwrap();

    for human_only_field in [
        "human_summary",
        "why_split",
        "dependency_explanation",
        "risk_explanation",
    ] {
        assert!(!contains_object_key(&value, human_only_field));
    }
    assert_eq!(
        serde_json::from_value::<CanonicalWorkItemContract>(value).unwrap(),
        contract
    );
}

#[test]
fn canonical_work_item_contract_legacy_json_defaults_depends_on() {
    let mut value = serde_json::to_value(canonical_contract_fixture("wi_core")).unwrap();
    value.as_object_mut().unwrap().remove("depends_on");
    let contract = serde_json::from_value::<CanonicalWorkItemContract>(value).unwrap();
    assert!(contract.depends_on.is_empty());
    assert!(
        !serde_json::to_value(&contract)
            .unwrap()
            .as_object()
            .unwrap()
            .contains_key("depends_on")
    );
}

#[test]
fn canonical_work_item_contract_enums_use_snake_case() {
    assert_enum_cases([
        (EvidenceKind::SourceDiff, "source_diff"),
        (
            EvidenceKind::NonZeroTestExecution,
            "non_zero_test_execution",
        ),
        (EvidenceKind::ManualCheck, "manual_check"),
        (EvidenceKind::HandoffField, "handoff_field"),
    ]);
    assert_enum_cases([
        (ContractCompatibilityPolicy::RequireAll, "require_all"),
        (ContractCompatibilityPolicy::RequireAny, "require_any"),
    ]);
    assert_enum_cases([
        (BlockerRoute::CoderRework, "coder_rework"),
        (BlockerRoute::VerificationRetry, "verification_retry"),
        (BlockerRoute::PlanRepairCurrent, "plan_repair_current"),
        (BlockerRoute::PlanRepairUpstream, "plan_repair_upstream"),
        (BlockerRoute::SubgraphReplan, "subgraph_replan"),
        (BlockerRoute::StoryAmendment, "story_amendment"),
        (BlockerRoute::DesignAmendment, "design_amendment"),
        (BlockerRoute::OperationalGate, "operational_gate"),
    ]);
}

#[test]
fn canonical_work_item_hash_is_stable_after_serde_rebuild() {
    let left = canonical_contract_fixture("wi_core");
    let bytes = serde_json::to_vec(&left).unwrap();
    let right = serde_json::from_slice::<CanonicalWorkItemContract>(&bytes).unwrap();

    assert_eq!(left, right);
    assert_eq!(
        canonical_contract_hash(&left).unwrap(),
        canonical_contract_hash(&right).unwrap()
    );
}

#[test]
fn canonical_work_item_hash_changes_when_normative_fields_change() {
    let original = canonical_contract_fixture("wi_core");
    let original_hash = canonical_contract_hash(&original).unwrap();
    let mut changed_contracts = Vec::new();

    let mut changed = original.clone();
    changed.schema_version += 1;
    changed_contracts.push(("schema_version", changed));

    let mut changed = original.clone();
    changed.identity.title = "Changed title".to_string();
    changed_contracts.push(("identity", changed));

    let mut changed = original.clone();
    changed.goal.summary = "Changed goal".to_string();
    changed_contracts.push(("goal", changed));

    let mut changed = original.clone();
    changed.non_goals.push("Changed non-goal".to_string());
    changed_contracts.push(("non_goals", changed));

    let mut changed = original.clone();
    changed.input_contracts[0]
        .required_capabilities
        .push("changed_input".to_string());
    changed_contracts.push(("input_contracts", changed));

    let mut changed = original.clone();
    changed.output_contracts[0]
        .capabilities
        .push("changed_output".to_string());
    changed_contracts.push(("output_contracts", changed));

    let mut changed = original.clone();
    changed.tasks[0].statement = "Changed task".to_string();
    changed_contracts.push(("tasks", changed));

    let mut changed = original.clone();
    changed
        .write_policy
        .exclusive_scopes
        .push("changed/scope".to_string());
    changed_contracts.push(("write_policy", changed));

    let mut changed = original.clone();
    changed.acceptance_criteria[0].statement = "Changed criterion".to_string();
    changed_contracts.push(("acceptance_criteria", changed));

    let mut changed = original.clone();
    changed.verification_checks[0].required = false;
    changed_contracts.push(("verification_checks", changed));

    let mut changed = original.clone();
    changed
        .handoff_contract
        .required_fields
        .push("changed_field".to_string());
    changed_contracts.push(("handoff_contract", changed));

    let mut changed = original.clone();
    changed.blocker_rules[0].reason_code = "changed_reason".to_string();
    changed_contracts.push(("blocker_rules", changed));

    let mut changed = original.clone();
    changed.design_traceability[0].requirement_id = "REQ-CHANGED".to_string();
    changed_contracts.push(("design_traceability", changed));

    for (field, changed) in changed_contracts {
        assert_ne!(
            canonical_contract_hash(&changed).unwrap(),
            original_hash,
            "changing {field} must change the canonical hash"
        );
    }
}

#[test]
fn canonical_work_item_hash_is_64_character_lowercase_hex() {
    let hash = canonical_contract_hash(&canonical_contract_fixture("wi_core")).unwrap();

    assert_eq!(hash.len(), 64);
    assert!(
        hash.bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    );
}

#[test]
fn canonical_work_item_human_presentation_changes_do_not_affect_hash() {
    let contract = canonical_contract_fixture("wi_core");
    let first_presentation = HumanPresentationRevision {
        id: "human_presentation_revision_0001".to_string(),
        source_plan_projection_bundle_id: None,
        source_work_item_projection_bundle_id: Some("projection_0001".to_string()),
        supersedes: None,
        human_summary: "First summary".to_string(),
        why_split: Some("First explanation".to_string()),
        dependency_explanation: vec![],
        risk_explanation: vec![],
        source_refs: vec![],
        normative: false,
        used_by_provider: false,
        created_at: "2026-07-17T00:00:00Z".to_string(),
    };
    let mut second_presentation = first_presentation.clone();
    second_presentation.human_summary = "Changed summary".to_string();
    second_presentation.why_split = Some("Changed explanation".to_string());

    let before = canonical_contract_hash(&contract).unwrap();
    assert_ne!(first_presentation, second_presentation);
    assert_eq!(canonical_contract_hash(&contract).unwrap(), before);
}

#[test]
fn canonical_work_item_draft_revision_roundtrips_with_required_typed_contract() {
    let draft = WorkItemDraftRevision {
        id: "draft_revision_0001".to_string(),
        logical_work_item_id: "wi_core".to_string(),
        revision_no: 1,
        supersedes: None,
        revision_reason: PlanRevisionReason::InitialCompile,
        canonical_contract_candidate: canonical_contract_fixture("wi_core"),
        trigger_repair_request_id: None,
        created_at: "2026-07-17T00:00:00Z".to_string(),
    };
    let value = serde_json::to_value(&draft).unwrap();

    assert_eq!(value["canonical_contract_candidate"]["schema_version"], 1);
    assert_eq!(
        serde_json::from_value::<WorkItemDraftRevision>(value.clone()).unwrap(),
        draft
    );
    for field in [
        "id",
        "logical_work_item_id",
        "revision_no",
        "revision_reason",
        "canonical_contract_candidate",
        "created_at",
    ] {
        assert_missing_field_rejected::<WorkItemDraftRevision>(&value, field);
    }
    let mut invalid = value;
    invalid["canonical_contract_candidate"] = serde_json::json!({"summary": "draft"});
    assert!(serde_json::from_value::<WorkItemDraftRevision>(invalid).is_err());
}

#[test]
fn canonical_work_item_revision_roundtrips_with_required_typed_contract() {
    let contract = canonical_contract_fixture("wi_core");
    let revision = WorkItemRevision {
        id: "work_item_revision_0001".to_string(),
        logical_work_item_id: "wi_core".to_string(),
        source_draft_revision_id: "draft_revision_0001".to_string(),
        canonical_contract_hash: canonical_contract_hash(&contract).unwrap(),
        canonical_contract: contract,
        work_item_projection_bundle_id: "projection_0001".to_string(),
        verification_plan_revision_id: "verification_0001".to_string(),
        created_at: "2026-07-17T00:00:00Z".to_string(),
    };
    let value = serde_json::to_value(&revision).unwrap();

    assert_eq!(value["canonical_contract"]["schema_version"], 1);
    assert_eq!(
        serde_json::from_value::<WorkItemRevision>(value.clone()).unwrap(),
        revision
    );
    for field in [
        "id",
        "logical_work_item_id",
        "source_draft_revision_id",
        "canonical_contract",
        "canonical_contract_hash",
        "work_item_projection_bundle_id",
        "verification_plan_revision_id",
        "created_at",
    ] {
        assert_missing_field_rejected::<WorkItemRevision>(&value, field);
    }
    let mut invalid = value;
    invalid["canonical_contract"] = serde_json::json!({"summary": "compiled"});
    assert!(serde_json::from_value::<WorkItemRevision>(invalid).is_err());
}

#[test]
fn canonical_work_item_verification_plan_roundtrips_with_required_typed_checks() {
    let verification = VerificationPlanRevision {
        id: "verification_0001".to_string(),
        logical_work_item_id: "wi_core".to_string(),
        source_draft_revision_id: "draft_revision_0001".to_string(),
        verification_checks: canonical_contract_fixture("wi_core").verification_checks,
        created_at: "2026-07-17T00:00:00Z".to_string(),
    };
    let value = serde_json::to_value(&verification).unwrap();

    assert_eq!(value["verification_checks"][0]["required"], true);
    assert_eq!(
        serde_json::from_value::<VerificationPlanRevision>(value.clone()).unwrap(),
        verification
    );
    for field in [
        "id",
        "logical_work_item_id",
        "source_draft_revision_id",
        "verification_checks",
        "created_at",
    ] {
        assert_missing_field_rejected::<VerificationPlanRevision>(&value, field);
    }
    let mut invalid = value;
    invalid["verification_checks"] = serde_json::json!([{"command": "cargo test --locked"}]);
    assert!(serde_json::from_value::<VerificationPlanRevision>(invalid).is_err());
}
