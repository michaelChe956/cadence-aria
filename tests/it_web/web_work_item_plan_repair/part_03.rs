#[tokio::test]
async fn web_work_item_plan_repair_deduplicates_duplicate_review_finding() {
    let root = tempdir().expect("fixture root");
    let runtime = PlanRepairFixtureRuntime::seed(root.path(), PlanRepairFixtureControl::default())
        .await
        .expect("seed plan repair fixture");

    runtime
        .drive_until_review_finds_upstream_contract_invalid()
        .await
        .expect("route first finding");
    runtime
        .replay_duplicate_plan_defect_finding()
        .await
        .expect("replay duplicate finding");

    assert_eq!(
        runtime.plan_repair_request_count().expect("request count"),
        1
    );
}

#[tokio::test]
async fn web_work_item_plan_repair_concurrent_finding_reuses_active_amendment() {
    let root = tempdir().expect("fixture root");
    let runtime = PlanRepairFixtureRuntime::seed(root.path(), PlanRepairFixtureControl::default())
        .await
        .expect("seed plan repair fixture");
    let [first, second] = runtime
        .start_overlapping_plan_defect_findings()
        .await
        .expect("overlapping findings must converge");

    assert_eq!(second, first);
    assert_eq!(runtime.plan_repair_identity().expect("active identity"), first);
    assert_eq!(
        runtime.plan_repair_request_count().expect("request count"),
        1
    );
}

#[tokio::test]
async fn web_work_item_plan_repair_stale_base_returns_amendment_conflict() {
    let root = tempdir().expect("fixture root");
    let runtime = PlanRepairFixtureRuntime::seed(root.path(), PlanRepairFixtureControl::default())
        .await
        .expect("seed plan repair fixture");
    runtime
        .drive_until_review_finds_upstream_contract_invalid()
        .await
        .expect("route first finding");

    let error = runtime
        .start_stale_base_plan_repair()
        .await
        .expect_err("stale base revision must conflict");

    assert!(matches!(
        &error,
        cadence_aria::product::plan_repair::PlanRepairError::AmendmentConflict {
            expected,
            actual,
        } if expected == "plan_revision_0000" && actual == "plan_revision_0001"
    ), "unexpected stale-base error: {error:?}");
}

#[tokio::test]
async fn web_work_item_plan_repair_dirty_worktree_opens_one_manual_gate() {
    let root = tempdir().expect("fixture root");
    let runtime = PlanRepairFixtureRuntime::seed(root.path(), PlanRepairFixtureControl::default())
        .await
        .expect("seed plan repair fixture");
    runtime
        .drive_until_review_finds_upstream_contract_invalid()
        .await
        .expect("route first finding");

    let blocked = runtime
        .publish_then_attempt_dirty_worktree_apply()
        .await
        .expect("dirty worktree gate");

    assert_eq!(blocked.open_gate_count, 1);
    assert_eq!(
        blocked.open_gate_reason_codes,
        vec!["worktree_dirty_before_plan_amendment"]
    );
    assert_eq!(blocked.application_journal_count, 0);
    assert_eq!(blocked.bound_plan_revision_id, "plan_revision_0001");
    assert_eq!(blocked.applied_amendment_count, 0);
}

#[tokio::test]
async fn web_work_item_plan_repair_restores_story_design_and_work_item_children() {
    use cadence_aria::product::models::{WorkspaceSessionStatus, WorkspaceType};

    let root = tempdir().expect("fixture root");
    let runtime = PlanRepairFixtureRuntime::seed(root.path(), PlanRepairFixtureControl::default())
        .await
        .expect("seed plan repair fixture");
    runtime
        .drive_until_review_finds_upstream_contract_invalid()
        .await
        .expect("route plan repair");

    let restored = runtime
        .restore_linked_workspace_matrix()
        .await
        .expect("restore linked workspace matrix");

    for (workspace_type, entity_id) in [
        (WorkspaceType::Story, "story_spec_0001"),
        (WorkspaceType::Design, "design_spec_0001"),
        (WorkspaceType::WorkItem, "wi_core"),
    ] {
        let snapshot = restored
            .iter()
            .find(|snapshot| snapshot.workspace_type == workspace_type)
            .expect("workspace type snapshot");
        assert_eq!(snapshot.entity_id, entity_id);
        assert_eq!(snapshot.artifact_version_id, Some(7));
        assert_eq!(snapshot.timeline_nodes.len(), 1);
        assert_eq!(
            snapshot.selected_timeline_node_id,
            Some(format!("timeline_node_linked_{entity_id}"))
        );
        assert_eq!(
            snapshot.human_confirm_state,
            WorkspaceSessionStatus::WaitingForHuman
        );
    }
}
