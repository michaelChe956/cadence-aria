use std::fs;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::gate_store::{CreateGateInput, GateStore};
use cadence_aria::product::models::{GateStatus, GateType};
use cadence_aria::web::app::build_web_router;
use cadence_aria::web::events::EventHub;
use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::state::WebAppState;
use serde_json::{Value, json};
use tempfile::tempdir;
use tower::ServiceExt;

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

#[test]
fn create_gate_uses_first_available_id_without_overwriting_existing_files() {
    let app = tempdir().expect("app");
    let paths = ProductAppPaths::new(app.path().join(".aria"));
    let store = GateStore::new(paths.clone());

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

    let mut sparse_gate = gate.clone();
    sparse_gate.id = "gate_0003".to_string();
    sparse_gate.binding_id = "binding_0003".to_string();
    sparse_gate.node_id = "N11".to_string();
    sparse_gate.status = GateStatus::Terminated;
    let gate_0003_path = paths
        .issue_root("project_0001", "issue_0001")
        .join("gates")
        .join("gate_0003.json");
    fs::write(
        &gate_0003_path,
        serde_json::to_string_pretty(&sparse_gate).expect("serialize sparse gate"),
    )
    .expect("write sparse gate");

    let new_gate = store
        .create(CreateGateInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            binding_id: "binding_0002".to_string(),
            node_id: "N10".to_string(),
            gate_type: GateType::HardGate,
            artifact_refs: vec!["design-spec-frontend.v1".to_string()],
        })
        .expect("new gate");

    assert_eq!(new_gate.id, "gate_0002");
    let preserved = store
        .get("project_0001", "issue_0001", "gate_0003")
        .expect("preserved sparse gate");
    assert_eq!(preserved.node_id, "N11");
    assert_eq!(preserved.status, GateStatus::Terminated);
}

#[tokio::test]
async fn resolve_gate_with_project_id_only_updates_matching_project_gate() {
    let root = tempdir().expect("root");
    let store = GateStore::new(ProductAppPaths::new(root.path().join(".aria")));

    store
        .create(CreateGateInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            binding_id: "binding_0001".to_string(),
            node_id: "N09".to_string(),
            gate_type: GateType::HardGate,
            artifact_refs: vec![],
        })
        .expect("project 1 gate");
    store
        .create(CreateGateInput {
            project_id: "project_0002".to_string(),
            issue_id: "issue_0001".to_string(),
            binding_id: "binding_0001".to_string(),
            node_id: "N09".to_string(),
            gate_type: GateType::HardGate,
            artifact_refs: vec![],
        })
        .expect("project 2 gate");

    let events = EventHub::new();
    let state = WebAppState::with_events(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
        events,
    );
    let app = build_web_router(state);

    let (status, response) = request_json(
        app,
        Method::POST,
        "/api/issues/issue_0001/gates/gate_0001/confirm?project_id=project_0002",
        json!({"comment":"approved","requested_change":null}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(response["decision"], "confirmed");

    let project_1_gate = store
        .get("project_0001", "issue_0001", "gate_0001")
        .expect("project 1 gate");
    let project_2_gate = store
        .get("project_0002", "issue_0001", "gate_0001")
        .expect("project 2 gate");
    assert_eq!(project_1_gate.status, GateStatus::Open);
    assert_eq!(project_2_gate.status, GateStatus::Confirmed);
}

#[tokio::test]
async fn resolve_gate_without_project_id_rejects_ambiguous_gate_match() {
    let root = tempdir().expect("root");
    let store = GateStore::new(ProductAppPaths::new(root.path().join(".aria")));

    store
        .create(CreateGateInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            binding_id: "binding_0001".to_string(),
            node_id: "N09".to_string(),
            gate_type: GateType::HardGate,
            artifact_refs: vec![],
        })
        .expect("project 1 gate");
    store
        .create(CreateGateInput {
            project_id: "project_0002".to_string(),
            issue_id: "issue_0001".to_string(),
            binding_id: "binding_0001".to_string(),
            node_id: "N09".to_string(),
            gate_type: GateType::HardGate,
            artifact_refs: vec![],
        })
        .expect("project 2 gate");

    let events = EventHub::new();
    let state = WebAppState::with_events(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
        events,
    );
    let app = build_web_router(state);

    let (status, response) = request_json(
        app,
        Method::POST,
        "/api/issues/issue_0001/gates/gate_0001/confirm",
        json!({"comment":"approved","requested_change":null}),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(response["code"], "gate_ambiguous");
}

async fn request_json(
    app: axum::Router,
    method: Method,
    uri: &str,
    body: Value,
) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("request");
    let response = app.oneshot(request).await.expect("response");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let value = serde_json::from_slice(&bytes).expect("json");
    (status, value)
}
