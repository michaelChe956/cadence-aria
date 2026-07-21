use super::super::runtime_impact::stable_handoff_contract_hash;
use super::*;
use crate::product::models::HandoffRevision;
use std::collections::BTreeMap;

fn delta_handoff(id: &str, capabilities: &[&str], contract_hash: &str) -> HandoffRevision {
    HandoffRevision {
        id: id.to_string(),
        logical_work_item_id: "wi_core".to_string(),
        work_item_revision_id: "work_item_revision_wi01_v2".to_string(),
        coding_unit_run_id: format!("coding_unit_run_{id}"),
        provided_contracts: vec!["registration_contract".to_string()],
        provided_capabilities: BTreeMap::from([(
            "registration_contract".to_string(),
            capabilities
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
        )]),
        contract_hash: contract_hash.to_string(),
        commit_sha: format!("commit_{id}"),
        tests: vec![format!("test_{id}")],
        artifacts: vec![format!("artifact_{id}")],
        created_at: "2026-07-20T00:00:00Z".to_string(),
    }
}

#[test]
fn coding_runtime_handoff_ignores_commit_tests_and_artifacts_for_unchanged_contract() {
    let previous = delta_handoff("0001", &["registration_ready"], "same_contract_hash");
    let mut next = previous.clone();
    next.id = "handoff_revision_0002".to_string();
    next.coding_unit_run_id = "coding_unit_run_0002".to_string();
    next.commit_sha = "different_commit".to_string();
    next.tests = vec!["different test explanation".to_string()];
    next.artifacts = vec!["different artifact explanation".to_string()];

    assert_eq!(
        compare_handoff_revisions(Some(&previous), &next),
        HandoffDeltaKind::Unchanged
    );
}

#[test]
fn coding_runtime_handoff_classifies_capability_extension_and_breaking_loss() {
    let previous = delta_handoff("0001", &["registration_ready"], "contract_hash_v1");
    let extended = delta_handoff(
        "0002",
        &["registration_ready", "workflow_explicit_completion"],
        "contract_hash_v2",
    );
    let breaking = delta_handoff(
        "0003",
        &["workflow_explicit_completion"],
        "contract_hash_v3",
    );

    assert_eq!(
        compare_handoff_revisions(Some(&previous), &extended),
        HandoffDeltaKind::CompatibleExtension
    );
    assert_eq!(
        compare_handoff_revisions(Some(&previous), &breaking),
        HandoffDeltaKind::BreakingChange
    );
}

#[test]
fn coding_runtime_handoff_contract_hash_uses_only_sorted_contract_capabilities() {
    let contracts = vec![
        "registration_contract".to_string(),
        "finalization_contract".to_string(),
    ];
    let reordered_contracts = vec![
        "finalization_contract".to_string(),
        "registration_contract".to_string(),
        "registration_contract".to_string(),
    ];
    let capabilities = BTreeMap::from([
        (
            "registration_contract".to_string(),
            vec!["registration_ready".to_string()],
        ),
        (
            "finalization_contract".to_string(),
            vec![
                "failure_message".to_string(),
                "workflow_explicit_completion".to_string(),
            ],
        ),
    ]);
    let reordered_capabilities = BTreeMap::from([
        (
            "finalization_contract".to_string(),
            vec![
                "workflow_explicit_completion".to_string(),
                "failure_message".to_string(),
                "failure_message".to_string(),
            ],
        ),
        (
            "registration_contract".to_string(),
            vec!["registration_ready".to_string()],
        ),
    ]);

    let first = stable_handoff_contract_hash(&contracts, &capabilities).unwrap();
    let reordered =
        stable_handoff_contract_hash(&reordered_contracts, &reordered_capabilities).unwrap();
    assert_eq!(first, reordered);

    let changed = stable_handoff_contract_hash(
        &contracts,
        &BTreeMap::from([
            (
                "registration_contract".to_string(),
                vec!["registration_ready".to_string()],
            ),
            (
                "finalization_contract".to_string(),
                vec!["failure_message".to_string()],
            ),
        ]),
    )
    .unwrap();
    assert_ne!(first, changed);
}
