use cadence_aria::daemon::checkpoint::{RiskRegistrySnapshot, RuntimeSnapshot};
use cadence_aria::protocol::loop_counters::LoopCounterName;
use cadence_aria::protocol::policies::PolicyMode;
use serde_json::json;
use std::collections::BTreeMap;

#[test]
fn runtime_snapshot_round_trips_all_p1_canonical_fields() {
    let snapshot = RuntimeSnapshot {
        snapshot_id: "snap_001".to_string(),
        session_id: "sess_001".to_string(),
        task_id: "task_001".to_string(),
        node_id: "N02".to_string(),
        phase: "intake".to_string(),
        timestamp: "2026-04-26T00:00:00Z".to_string(),
        effective_policy: PolicyMode::Conservative,
        artifact_refs: vec!["ref_art_intake_brief_task_001_0001_v0001".to_string()],
        provider_run_refs: vec![],
        worktree_ref: None,
        rework_counter: 0,
        risk_registry: RiskRegistrySnapshot {
            risk_registry_ref: "riskreg_task_001_v0001".to_string(),
            risk_ids: vec![],
            risks: vec![],
        },
        loop_counters: BTreeMap::<LoopCounterName, u32>::new(),
        superseded_artifact_refs: vec![],
        node_specific_fields: json!({
            "openspec_bootstrap_status": "bootstrap_pending"
        }),
        projection_refs: vec![],
        constraint_bundle_refs: vec![],
    };

    let value = serde_json::to_value(&snapshot).expect("snapshot to json");
    assert!(value.get("effective_policy").is_some());
    assert!(value.get("projection_refs").is_some());
    assert!(value.get("constraint_bundle_refs").is_some());
    assert!(value.get("createdAt").is_none());

    let round_trip: RuntimeSnapshot =
        serde_json::from_value(value).expect("snapshot should deserialize");
    round_trip.validate().expect("snapshot should validate");
}

#[test]
fn runtime_snapshot_rejects_implementation_node_ids() {
    let mut snapshot = RuntimeSnapshot::minimal_for_test("M20");
    let error = snapshot
        .validate()
        .expect_err("implementation node id must not enter protocol fields");

    assert_eq!(error, "invalid node_id M20");

    snapshot.node_id = "X06".to_string();
    snapshot.validate().expect("cross-cutting node id is valid");
}
