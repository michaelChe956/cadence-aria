use cadence_aria::daemon::recovery::{
    compute_retention_index, EventLogEntry, EventLogIndex, ReplayDecision, ReplayWindow,
    TaskEventRetention,
};
use cadence_aria::daemon::state_machine::DaemonState;
use cadence_aria::protocol::repl_wire::{Command, HelloRequest, RequestEnvelope};
use std::collections::BTreeMap;
use tempfile::tempdir;

#[test]
fn replay_window_allows_last_seen_at_edge_of_retained_window() {
    let window = ReplayWindow::from_index(EventLogIndex {
        daemon_session_id: "sess_001".to_string(),
        latest_event_id: 1200,
        first_retained_event_id: 201,
        first_retained_event_id_by_task: BTreeMap::from([("task_001".to_string(), 301)]),
    });

    assert_eq!(window.decide(Some(200)), ReplayDecision::ReplayFrom(201));
    assert_eq!(window.decide(Some(1200)), ReplayDecision::ReplayFrom(1201));
}

#[test]
fn replay_window_reports_window_lost_when_last_seen_is_too_old() {
    let window = ReplayWindow::from_index(EventLogIndex {
        daemon_session_id: "sess_001".to_string(),
        latest_event_id: 1200,
        first_retained_event_id: 201,
        first_retained_event_id_by_task: BTreeMap::new(),
    });

    assert_eq!(window.decide(Some(199)), ReplayDecision::WindowLost);
}

#[test]
fn hello_reports_replay_window_lost_when_last_seen_is_too_old() {
    let workspace = tempdir().expect("temp workspace");
    let mut state = DaemonState::bootstrap(workspace.path()).expect("bootstrap daemon state");
    state.set_replay_floor_for_test(10);

    let response = state
        .handle_request(
            RequestEnvelope::new(
                "req_hello_replay_lost",
                Command::Hello,
                HelloRequest {
                    last_seen_event_id: Some(8),
                },
            )
            .expect("hello envelope"),
        )
        .expect("hello response");

    assert!(!response.ok);
    assert_eq!(
        response.error.expect("replay error").code,
        "replay_window_lost"
    );
}

#[test]
fn hello_accepts_last_seen_at_retained_edge() {
    let workspace = tempdir().expect("temp workspace");
    let mut state = DaemonState::bootstrap(workspace.path()).expect("bootstrap daemon state");
    state.set_replay_floor_for_test(10);

    let response = state
        .handle_request(
            RequestEnvelope::new(
                "req_hello_replay_ok",
                Command::Hello,
                HelloRequest {
                    last_seen_event_id: Some(9),
                },
            )
            .expect("hello envelope"),
        )
        .expect("hello response");

    assert!(response.ok);
    assert!(response.error.is_none());
}

#[test]
fn retention_policy_keeps_global_and_per_task_minimums() {
    let retention = TaskEventRetention::phase1_default();

    assert_eq!(retention.global_recent_events, 10_000);
    assert_eq!(retention.per_task_recent_events, 1_000);
    assert_eq!(retention.effective_minimum_for_tasks(3), 10_000);
    assert_eq!(retention.effective_minimum_for_tasks(12), 12_000);
}

#[test]
fn retention_index_preserves_each_task_recent_window_even_beyond_global_window() {
    let mut events = Vec::new();
    for id in 1..=12_000 {
        let task_id = if id <= 6_000 { "task_a" } else { "task_b" };
        events.push(EventLogEntry {
            event_id: id,
            task_id: Some(task_id.to_string()),
        });
    }

    let index = compute_retention_index(&events, TaskEventRetention::phase1_default());

    assert_eq!(index.latest_event_id, 12_000);
    assert_eq!(index.first_retained_event_id, 2_001);
    assert_eq!(index.first_retained_event_id_by_task["task_a"], 5_001);
    assert_eq!(index.first_retained_event_id_by_task["task_b"], 11_001);
}

#[test]
fn retention_index_expands_when_task_windows_exceed_global_window() {
    let mut events = Vec::new();
    for task_number in 0..12 {
        for offset in 0..1_001 {
            events.push(EventLogEntry {
                event_id: task_number * 1_001 + offset + 1,
                task_id: Some(format!("task_{task_number:02}")),
            });
        }
    }

    let index = compute_retention_index(&events, TaskEventRetention::phase1_default());

    assert_eq!(index.latest_event_id, 12_012);
    assert_eq!(index.first_retained_event_id, 2);
    assert_eq!(index.first_retained_event_id_by_task["task_00"], 2);
    assert_eq!(index.first_retained_event_id_by_task["task_11"], 11_013);
}
