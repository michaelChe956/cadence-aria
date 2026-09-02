// 7.2 快照断裂追修的清理语义回归锁定（从 workspace.rs 内联 tests 拆出到独立
// include 文件，守 large_file_guard 1200 行红线）：通用状态写入
// `update_workspace_session_status` 对 Confirmed/Terminated/Failed 依旧清
// `human_gate_snapshot`，非终态（WaitingForHuman）依旧保留。SC 计划批准链绕开
// 该清理属于快照追修的例外路径，不得反向放宽通用语义。

fn persist_session_with_gate_snapshot(store: &LifecycleStore, session_id: &str) {
    let mut record = store.get_workspace_session(session_id).unwrap();
    record.human_gate_snapshot = Some(HumanGateSnapshot {
        findings: Vec::new(),
        repeated_fingerprints: Vec::new(),
        attempts_used: 0,
        manual_repairs_remaining: 1,
        trigger: HumanReason::NativeHumanRequired,
        resumable: false,
    });
    write_json(
        &store
            .app_paths()
            .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
            .join("workspace-sessions")
            .join(format!("{}.json", record.id)),
        &record,
    )
    .unwrap();
}

#[test]
fn update_workspace_session_status_still_clears_gate_snapshot_on_terminal_statuses() {
    use crate::product::models::WorkspaceSessionStatus;

    let (_tmp, store) = setup();
    let session = create_session(&store, "story_0001", WorkspaceType::Story);

    for status in [
        WorkspaceSessionStatus::Confirmed,
        WorkspaceSessionStatus::Terminated,
        WorkspaceSessionStatus::Failed,
    ] {
        persist_session_with_gate_snapshot(&store, &session.id);
        let saved = store
            .update_workspace_session_status(&session.id, status.clone())
            .unwrap();
        assert_eq!(saved.status, status);
        assert_eq!(
            saved.human_gate_snapshot, None,
            "generic terminal write must keep clearing the gate snapshot"
        );
    }

    persist_session_with_gate_snapshot(&store, &session.id);
    let saved = store
        .update_workspace_session_status(&session.id, WorkspaceSessionStatus::WaitingForHuman)
        .unwrap();
    assert_eq!(saved.status, WorkspaceSessionStatus::WaitingForHuman);
    assert!(
        saved.human_gate_snapshot.is_some(),
        "non-terminal generic write must keep the gate snapshot"
    );
}
