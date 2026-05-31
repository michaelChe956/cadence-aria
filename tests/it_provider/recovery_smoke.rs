use cadence_aria::daemon::state_machine::DaemonState;
use cadence_aria::protocol::repl_wire::NewTaskRequest;
use tempfile::tempdir;

#[test]
fn daemon_recovers_session_summary_from_checkpoint() {
    let workspace = tempdir().expect("temp workspace");
    let mut state = DaemonState::bootstrap(workspace.path()).expect("bootstrap daemon state");
    let response = state
        .new_task(NewTaskRequest {
            request_text: "恢复测试任务".to_string(),
            requested_change_id: None,
        })
        .expect("new task");

    state.persist_checkpoint().expect("persist checkpoint");

    let session_path = workspace.path().join(".aria/runtime/session.json");
    let value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(session_path).expect("read session"))
            .expect("session json");
    assert!(value.get("daemon_session_id").is_some());
    assert!(value.get("latest_event_id").is_some());
    assert!(value.get("visible_tasks").is_some());
    assert!(value.get("daemonSessionId").is_none());

    let recovered = DaemonState::recover(workspace.path()).expect("recover daemon state");
    let task = recovered.task(&response.task_id).expect("recovered task");
    assert_eq!(task.change_id, response.change_id);
    assert_eq!(task.intake_ref, response.intake_ref);
}
