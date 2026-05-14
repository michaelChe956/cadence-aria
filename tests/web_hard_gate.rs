use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::gate_store::{CreateGateInput, GateStore};
use cadence_aria::product::models::{GateStatus, GateType};
use tempfile::tempdir;

#[test]
fn hard_gate_roundtrip_and_single_open_gate_per_node() {
    let app = tempdir().expect("app");
    let store = GateStore::new(ProductAppPaths::new(app.path().join(".aria")));

    let gate = store
        .create(CreateGateInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            binding_id: "binding_0001".to_string(),
            node_id: "N09".to_string(),
            gate_type: GateType::HardGate,
            artifact_refs: vec!["design-spec-backend.v1".to_string()],
        })
        .expect("gate");

    let duplicate = store
        .create(CreateGateInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            binding_id: "binding_0001".to_string(),
            node_id: "N09".to_string(),
            gate_type: GateType::HardGate,
            artifact_refs: vec!["design-spec-backend.v1".to_string()],
        })
        .expect("duplicate");

    assert_eq!(gate.id, duplicate.id);
    assert_eq!(gate.status, GateStatus::Open);
}
