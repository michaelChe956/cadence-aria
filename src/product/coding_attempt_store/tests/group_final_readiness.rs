use super::*;

#[test]
fn group_final_readiness_rejects_non_empty_observation_without_commit_range_facts() {
    let (_tmp, store, attempt) = setup();
    let mut unit = ready_unit("unit_0001", "work_item_0001", "unit_run_0001");
    unit.commit_shas.clear();
    unit.start_commit = Some("start_commit_unit_0001".to_string());
    unit.completion_commit = Some("completion_commit_unit_0001".to_string());
    let snapshot = GroupFinalReadinessSnapshot {
        attempt_id: attempt.id.clone(),
        status: GroupFinalReadinessStatus::Complete,
        units: vec![unit],
        diagnostics: Vec::new(),
        created_at: String::new(),
    };

    assert_invalid_group_final_readiness_snapshot(
        store
            .write_group_final_readiness_snapshot(&attempt, &snapshot)
            .expect_err("non-empty observation must include commit range facts"),
        "non-empty observation unit unit_0001 must include commit range facts",
    );
}

#[test]
fn group_final_readiness_rejects_complete_snapshot_with_diagnostics() {
    let (_tmp, store, attempt) = setup();
    let snapshot = GroupFinalReadinessSnapshot {
        attempt_id: attempt.id.clone(),
        status: GroupFinalReadinessStatus::Complete,
        units: vec![ready_unit("unit_0001", "work_item_0001", "unit_run_0001")],
        diagnostics: vec![GroupFinalReadinessDiagnostic {
            kind: GroupFinalReadinessDiagnosticKind::CodeReviewMissing,
            unit_id: Some("unit_0001".to_string()),
            message: "a complete snapshot cannot carry this diagnostic".to_string(),
        }],
        created_at: String::new(),
    };

    assert_invalid_group_final_readiness_snapshot(
        store
            .write_group_final_readiness_snapshot(&attempt, &snapshot)
            .expect_err("complete snapshots must not carry diagnostics"),
        "complete snapshot must not carry diagnostics",
    );
}
