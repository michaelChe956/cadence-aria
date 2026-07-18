fn plan_repair_assert_failed_recovery(engine: &WorkspaceEngine, expected_error: &str) {
    assert_eq!(engine.current_stage(), WorkspaceStage::Completed);
    match engine.build_session_state() {
        WsOutMessage::SessionState {
            plan_repair: Some(snapshot),
            ..
        } => {
            assert_eq!(
                snapshot.stage,
                crate::product::models::PlanRepairSessionStage::Failed
            );
            assert!(
                snapshot
                    .error
                    .as_deref()
                    .is_some_and(|error| error.contains(expected_error)),
                "unexpected recovery error: {:?}",
                snapshot.error
            );
        }
        message => panic!("expected failed plan repair recovery state, got {message:?}"),
    }
}

#[test]
fn plan_repair_refresh_fails_closed_when_linked_snapshot_is_missing() {
    let (tmp, lifecycle, revision_store, _parent) = plan_repair_parent_engine();
    let fingerprint = "fingerprint_missing_snapshot";
    let amendment_id = format!("plan_amendment_{fingerprint}");
    let child_session_id = format!("workspace_session_{amendment_id}");
    let mut request = plan_repair_fixture("plan_repair_request_0001", fingerprint);
    request.amendment_id = Some(amendment_id.clone());
    request.status = crate::product::models::PlanRepairRequestStatus::InProgress;
    let plan = revision_store
        .get_plan_lineage("project_0001", "issue_0001", "work_item_plan_0001")
        .unwrap();
    revision_store.put_repair_request(&plan, &request).unwrap();
    revision_store
        .acquire_active_amendment(&plan, &amendment_id)
        .unwrap();
    let child = plan_repair_child_record(&lifecycle, &child_session_id);
    lifecycle
        .put_session_link(
            "project_0001",
            "issue_0001",
            &plan_repair_link(&request, &amendment_id, &child.id),
        )
        .unwrap();

    let restored = plan_repair_restarted_child_engine(&tmp, &lifecycle, child);

    plan_repair_assert_failed_recovery(&restored, "missing");
}

#[tokio::test]
async fn plan_repair_refresh_fails_closed_when_snapshot_json_is_corrupt() {
    let (tmp, lifecycle, _revision_store, mut parent) = plan_repair_parent_engine();
    let child = parent
        .start_plan_repair(plan_repair_fixture(
            "plan_repair_request_0001",
            "fingerprint_corrupt_snapshot",
        ))
        .await
        .unwrap();
    let snapshot_path = lifecycle
        .workspace_timeline_root_for_issue_session("project_0001", "issue_0001", &child.id)
        .unwrap()
        .join("plan_repair_session_state.json");
    std::fs::write(snapshot_path, b"{corrupt-json").unwrap();

    let restored = plan_repair_restarted_child_engine(&tmp, &lifecycle, child);

    plan_repair_assert_failed_recovery(&restored, "load");
}

#[tokio::test]
async fn plan_repair_refresh_fails_closed_on_snapshot_identity_mismatch() {
    let (tmp, lifecycle, _revision_store, mut parent) = plan_repair_parent_engine();
    let child = parent
        .start_plan_repair(plan_repair_fixture(
            "plan_repair_request_0001",
            "fingerprint_snapshot_identity",
        ))
        .await
        .unwrap();
    let mut snapshot = lifecycle
        .load_plan_repair_session_state("project_0001", "issue_0001", &child.id)
        .unwrap()
        .unwrap();
    snapshot.request.fingerprint = "fingerprint_tampered".to_string();
    let snapshot_path = lifecycle
        .workspace_timeline_root_for_issue_session("project_0001", "issue_0001", &child.id)
        .unwrap()
        .join("plan_repair_session_state.json");
    std::fs::write(snapshot_path, serde_json::to_vec_pretty(&snapshot).unwrap()).unwrap();

    let restored = plan_repair_restarted_child_engine(&tmp, &lifecycle, child);

    plan_repair_assert_failed_recovery(&restored, "identity");
}

#[tokio::test]
async fn plan_repair_cancelled_replay_is_idempotent_without_state_writes() {
    let (tmp, lifecycle, revision_store, mut parent) = plan_repair_parent_engine();
    let child = parent
        .start_plan_repair(plan_repair_fixture(
            "plan_repair_request_0001",
            "fingerprint_cancel_replay",
        ))
        .await
        .unwrap();
    let plan = revision_store
        .get_plan_lineage("project_0001", "issue_0001", "work_item_plan_0001")
        .unwrap();
    let request = revision_store
        .get_repair_request(&plan, "plan_repair_request_0001")
        .unwrap();
    let amendment_id = request.amendment_id.clone().unwrap();
    let mut child_engine = plan_repair_restarted_child_engine(&tmp, &lifecycle, child);
    child_engine
        .enter_plan_repair_awaiting_confirmation(plan_repair_awaiting_package(
            &request.id,
            &amendment_id,
        ))
        .await
        .unwrap();
    child_engine
        .cancel_plan_amendment(&amendment_id, Some("first cancel".to_string()))
        .await
        .unwrap();
    let before = child_engine.plan_repair_session_state().unwrap().clone();

    child_engine
        .cancel_plan_amendment(&amendment_id, Some("replayed cancel".to_string()))
        .await
        .unwrap();

    assert_eq!(child_engine.plan_repair_session_state(), Some(&before));
    assert_eq!(
        before
            .timeline_nodes
            .iter()
            .filter(|node| node.node_type == TimelineNodeType::PlanAmendmentCancelled)
            .count(),
        1
    );
    let error = child_engine
        .cancel_plan_amendment("plan_amendment_wrong", None)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        crate::product::plan_repair::PlanRepairError::AmendmentConflict { .. }
    ));
}
