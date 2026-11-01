// 7.2 快照断裂追修的清理语义回归锁定（从 workspace.rs 内联 tests 拆出到独立
// include 文件，守 large_file_guard 1200 行红线）：通用状态写入
// `update_workspace_session_status` 对 Confirmed/Terminated/Failed 依旧清
// `human_gate_snapshot`，非终态（WaitingForHuman）依旧保留。SC 计划批准链绕开
// 该清理属于快照追修的例外路径，不得反向放宽通用语义。

fn persist_session_with_gate_snapshot(store: &LifecycleStore, session_id: &str) {
    let mut record = store.get_workspace_session(session_id).unwrap();
    record.human_gate_snapshot = Some(HumanGateSnapshot {
        candidate_recovery: None,
        findings: Vec::new(),
        repeated_fingerprints: Vec::new(),
        attempts_used: 0,
        manual_repairs_remaining: 1,
        trigger: HumanReason::NativeHumanRequired,
        resumable: false,
        accepted_feedback_turns: None,
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

    // 非终态(WaitingForHuman)保留快照——置于终态写之前:批 2 T2 的
    // Terminated 粘滞守卫使 from-Terminated 的后续通用写原样返回,
    // 非终态回归段不能以 Terminated 为起点。
    persist_session_with_gate_snapshot(&store, &session.id);
    let saved = store
        .update_workspace_session_status(&session.id, WorkspaceSessionStatus::WaitingForHuman)
        .unwrap();
    assert_eq!(saved.status, WorkspaceSessionStatus::WaitingForHuman);
    assert!(
        saved.human_gate_snapshot.is_some(),
        "non-terminal generic write must keep the gate snapshot"
    );

    // 终态序列 Confirmed→Failed→Terminated:覆盖三个终态、清快照断言
    // 不变,且全程无 from-Terminated 转换(粘滞守卫只挡 from-Terminated)。
    for status in [
        WorkspaceSessionStatus::Confirmed,
        WorkspaceSessionStatus::Failed,
        WorkspaceSessionStatus::Terminated,
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

    // 批 2 T2(REQ-WIGA-09)语义钉住:Terminated 粘滞守卫——run 取消后
    // 迟到的 Failed 通用写不得覆盖终止终态,原样返回 Terminated 记录且
    // 无落盘副作用(不顺带清 gate 快照)。
    persist_session_with_gate_snapshot(&store, &session.id);
    let sticky = store
        .update_workspace_session_status(&session.id, WorkspaceSessionStatus::Failed)
        .unwrap();
    assert_eq!(sticky.status, WorkspaceSessionStatus::Terminated);
    let reloaded = store.get_workspace_session(&session.id).unwrap();
    assert_eq!(reloaded.status, WorkspaceSessionStatus::Terminated);
    assert!(
        reloaded.human_gate_snapshot.is_some(),
        "sticky guard must not write or clear the snapshot"
    );
}
