fn plan_repair_assert_failed_recovery(engine: &WorkspaceEngine, expected_error: &str) {
    assert_eq!(engine.current_stage(), WorkspaceStage::Completed);
    if let Some(amendment_id) = engine
        .plan_repair_session_state()
        .and_then(|snapshot| snapshot.request.amendment_id.as_deref())
    {
        assert!(!engine.is_cancelled_plan_amendment_replay(amendment_id));
    }
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
async fn plan_repair_refresh_fails_closed_on_link_return_identity_mismatch() {
    type LinkMutation = fn(&mut crate::product::models::WorkspaceSessionLink);
    let cases: [(&str, LinkMutation); 5] = [
        ("parent", |link| {
            link.parent_session_id = "coding_attempt_wrong".to_string();
        }),
        ("return_attempt", |link| {
            link.return_context.original_attempt_id = "coding_attempt_wrong".to_string();
        }),
        ("return_unit", |link| {
            link.return_context.original_unit_run_id = "coding_unit_run_wrong".to_string();
        }),
        ("timeline_anchor", |link| {
            link.return_context.timeline_anchor_id = "finding_wrong".to_string();
        }),
        ("return_route", |link| {
            link.return_context.original_route = "/wrong-route".to_string();
        }),
    ];
    for (case, mutate) in cases {
        let (tmp, lifecycle, _revision_store, mut parent) = plan_repair_parent_engine();
        let child = parent
            .start_plan_repair(plan_repair_fixture(
                "plan_repair_request_0001",
                &format!("fingerprint_refresh_link_{case}"),
            ))
            .await
            .unwrap();
        let mut snapshot = lifecycle
            .load_plan_repair_session_state("project_0001", "issue_0001", &child.id)
            .unwrap()
            .unwrap();
        mutate(&mut snapshot.link);
        let link_path = lifecycle
            .app_paths()
            .issue_lifecycle_root("project_0001", "issue_0001")
            .join("workspace-session-links")
            .join(format!("{}.json", snapshot.link.id));
        std::fs::write(
            link_path,
            serde_json::to_vec_pretty(&snapshot.link).unwrap(),
        )
        .unwrap();
        let snapshot_path = lifecycle
            .workspace_timeline_root_for_issue_session(
                "project_0001",
                "issue_0001",
                &child.id,
            )
            .unwrap()
            .join("plan_repair_session_state.json");
        std::fs::write(
            snapshot_path,
            serde_json::to_vec_pretty(&snapshot).unwrap(),
        )
        .unwrap();

        let restored = plan_repair_restarted_child_engine(&tmp, &lifecycle, child);

        plan_repair_assert_failed_recovery(&restored, "identity");
    }
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
    plan_repair_enter_awaiting(
        &mut child_engine,
        &revision_store,
        &plan,
        plan_repair_awaiting_package(
            &request.id,
            &amendment_id,
        ),
    )
    .await
        .unwrap();
    child_engine
        .cancel_plan_amendment(&amendment_id, Some("first cancel".to_string()))
        .await
        .unwrap();
    assert!(child_engine.is_cancelled_plan_amendment_replay(&amendment_id));
    assert!(!child_engine.is_cancelled_plan_amendment_replay("plan_amendment_wrong"));
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
