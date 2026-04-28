use cadence_aria::daemon::state_machine::DaemonState;
use cadence_aria::protocol::constraints::OpenSpecBootstrapStatus;
use cadence_aria::protocol::policies::PolicyMode;
use cadence_aria::protocol::repl_wire::NewTaskRequest;
use cadence_aria::runtime_units::{
    task_init::TaskInitUnit, CanonicalNodeInput, DaemonContext, RuntimeUnit,
};
use serde_json::{json, Value};
use tempfile::tempdir;

#[test]
fn new_task_materializes_intake_brief_and_runtime_state() {
    let workspace = tempdir().expect("temp workspace");
    let mut state = DaemonState::bootstrap(workspace.path()).expect("bootstrap daemon state");

    let response = state
        .new_task(NewTaskRequest {
            request_text: "实现 Aria P1 入口链".to_string(),
            requested_change_id: None,
        })
        .expect("new task");

    assert_eq!(response.change_id, format!("chg_{}", response.task_id));

    let task = state.task(&response.task_id).expect("task state");
    assert_eq!(task.change_id, response.change_id);
    assert_eq!(task.effective_policy, PolicyMode::Conservative);
    assert_eq!(
        task.openspec_bootstrap_status,
        OpenSpecBootstrapStatus::BootstrapPending
    );
    assert!(!task.risk_registry_ref.is_empty());
    assert_eq!(
        task.protocol_steps,
        vec![
            "N00".to_string(),
            "N01".to_string(),
            "N02".to_string(),
            "N03".to_string()
        ]
    );

    let task_path = workspace
        .path()
        .join(".aria/runtime/tasks")
        .join(&response.task_id)
        .join("task.json");
    assert!(task_path.exists(), "task runtime state sidecar must exist");
    let task_json: Value =
        serde_json::from_slice(&std::fs::read(&task_path).expect("read task state"))
            .expect("task state json");
    assert_eq!(task_json["task_id"], response.task_id);
    assert_eq!(task_json["change_id"], response.change_id);

    let artifact_path = workspace
        .path()
        .join(".aria/runtime/tasks")
        .join(&response.task_id)
        .join("artifacts/intake_brief")
        .join(format!(
            "art_intake_brief_{}_0001_v0001.json",
            response.task_id
        ));
    assert!(artifact_path.exists(), "intake brief must be materialized");

    let content: Value =
        serde_json::from_slice(&std::fs::read(&artifact_path).expect("read intake"))
            .expect("intake json");
    assert_eq!(content["request_text"], "实现 Aria P1 入口链");
    assert_eq!(content["origin_type"], "user_repl");
    assert_eq!(content["task_id"], response.task_id);

    let index_path = artifact_path
        .parent()
        .expect("artifact dir")
        .join("artifact_index.json");
    let latest_path = artifact_path
        .parent()
        .expect("artifact dir")
        .join("latest.json");
    assert!(index_path.exists(), "artifact index must exist");
    assert!(latest_path.exists(), "latest pointer must exist");
    assert_eq!(response.intake_ref, task.intake_ref);
    assert!(
        !task.risk_registry_ref.contains('/'),
        "risk registry ref must be a logical ref, not a path"
    );

    let risk_registry_path = workspace
        .path()
        .join(".aria/runtime/tasks")
        .join(&response.task_id)
        .join("risk-registry/registry.json");
    assert!(
        risk_registry_path.exists(),
        "empty risk registry sidecar must be materialized"
    );
    let risk_registry: Value =
        serde_json::from_slice(&std::fs::read(&risk_registry_path).expect("read risk registry"))
            .expect("risk registry json");
    assert_eq!(risk_registry["risk_registry_ref"], task.risk_registry_ref);
    assert_eq!(
        risk_registry["risk_ids"]
            .as_array()
            .expect("risk ids array")
            .len(),
        0
    );
    let risk_registry_ref_path = workspace
        .path()
        .join(".aria/runtime/tasks")
        .join(&response.task_id)
        .join("risk-registry/refs")
        .join(format!("{}.json", task.risk_registry_ref));
    assert!(
        risk_registry_ref_path.exists(),
        "risk registry ref record must be materialized"
    );

    let session_path = workspace.path().join(".aria/runtime/session.json");
    assert!(
        session_path.exists(),
        "new_task must auto-persist session checkpoint"
    );
    let session: Value =
        serde_json::from_slice(&std::fs::read(&session_path).expect("read session checkpoint"))
            .expect("session checkpoint json");
    assert_eq!(session["latest_event_id"], 2);
    assert_eq!(
        session["attached_clients"]
            .as_array()
            .expect("attached clients array")
            .len(),
        0
    );
    assert_eq!(
        session["open_gates"]
            .as_array()
            .expect("open gates array")
            .len(),
        0
    );

    for node_id in ["N00", "N01", "N02", "N03"] {
        let snapshot_path = workspace
            .path()
            .join(".aria/runtime/tasks")
            .join(&response.task_id)
            .join("snapshots")
            .join(format!("{node_id}.json"));
        assert!(snapshot_path.exists(), "snapshot {node_id} must exist");
    }

    let event_log_dir = workspace.path().join(".aria/runtime/events");
    let event_log = std::fs::read_dir(event_log_dir)
        .expect("event log dir")
        .find_map(|entry| {
            let path = entry.expect("event log entry").path();
            let is_jsonl = path.extension().is_some_and(|ext| ext == "jsonl");
            is_jsonl.then_some(path)
        })
        .expect("event log file");
    let event_line = std::fs::read_to_string(event_log)
        .expect("event log content")
        .lines()
        .next()
        .expect("first event")
        .to_string();
    let event: Value = serde_json::from_str(&event_line).expect("event json");
    assert!(event.get("created_at").is_some());
    assert!(event.get("occurred_at").is_none());

    let event_index: Value = serde_json::from_slice(
        &std::fs::read(workspace.path().join(".aria/runtime/events/index.json"))
            .expect("event index"),
    )
    .expect("event index json");
    assert_eq!(event_index["latest_event_id"], 2);
    assert_eq!(event_index["first_retained_event_id"], 1);
    assert_eq!(
        event_index["first_retained_event_id_by_task"][&response.task_id],
        2
    );

    for path in [
        task_path,
        session_path,
        index_path,
        latest_path,
        risk_registry_path,
        risk_registry_ref_path,
    ] {
        assert_forbidden_runtime_field_names_absent(&path);
    }
}

#[test]
fn requested_change_id_is_validated_and_frozen() {
    let workspace = tempdir().expect("temp workspace");
    let mut state = DaemonState::bootstrap(workspace.path()).expect("bootstrap daemon state");

    let response = state
        .new_task(NewTaskRequest {
            request_text: "带指定 change id 的任务".to_string(),
            requested_change_id: Some("chg_custom_001".to_string()),
        })
        .expect("new task");

    assert_eq!(response.change_id, "chg_custom_001");

    let error = state
        .new_task(NewTaskRequest {
            request_text: "非法 change id".to_string(),
            requested_change_id: Some("bad change id".to_string()),
        })
        .expect_err("invalid change id should fail");
    assert_eq!(error.code, "invalid_request");
}

#[tokio::test]
async fn task_init_runtime_unit_reports_n02_and_n03_protocol_steps() {
    let workspace = tempdir().expect("temp workspace");
    let result = TaskInitUnit
        .execute(
            CanonicalNodeInput {
                session_id: "sess_001".to_string(),
                task_id: Some("task_0001".to_string()),
                node_id: "N02".to_string(),
                risk_registry_ref: Some("riskreg_task_0001_v0001".to_string()),
                payload: json!({}),
            },
            &DaemonContext {
                workspace_root: workspace.path().to_string_lossy().to_string(),
            },
        )
        .await
        .expect("task init unit");

    let node_ids: Vec<_> = result
        .protocol_steps
        .iter()
        .map(|step| step.node_id.as_str())
        .collect();
    assert_eq!(node_ids, vec!["N02", "N03"]);
}

fn assert_forbidden_runtime_field_names_absent(path: &std::path::Path) {
    let content = std::fs::read_to_string(path).expect("runtime json");
    for forbidden in [
        "createdAt",
        "bundleVersion",
        "constraint_bundleId",
        "normalizedArtifactRef",
        "rejectionReason",
        "registeredWorktask_ids",
        "lastSeenEventId",
        "riskRegistryRef",
    ] {
        assert!(
            !content.contains(forbidden),
            "{} contains forbidden field name {forbidden}",
            path.display()
        );
    }
}
