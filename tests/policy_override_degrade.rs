use cadence_aria::daemon::state_machine::DaemonState;
use cadence_aria::protocol::policies::PolicyMode;
use tempfile::tempdir;

#[test]
fn non_conservative_policy_requests_are_degraded_and_audited() {
    let workspace = tempdir().expect("temp workspace");
    let mut state = DaemonState::bootstrap(workspace.path()).expect("bootstrap daemon state");

    let response = state
        .new_task_with_policy("需要高自主策略", None, PolicyMode::Auto)
        .expect("new task with degraded policy");

    let task = state.task(&response.task_id).expect("task state");
    assert_eq!(task.effective_policy, PolicyMode::Conservative);

    let degraded_event = state
        .events()
        .iter()
        .find(|event| event.event_type == "policy_mode.degraded")
        .expect("policy degrade event");

    assert_eq!(degraded_event.payload["task_id"], response.task_id);
    assert_eq!(degraded_event.payload["requested_mode"], "auto");
    assert_eq!(degraded_event.payload["effective_mode"], "conservative");
}
