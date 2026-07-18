#[tokio::test]
async fn plan_repair_awaiting_journal_recovers_every_persistence_boundary() {
    for crash_point in [
        PlanRepairCrashPoint::TimelinePersisted,
        PlanRepairCrashPoint::SnapshotPersisted,
        PlanRepairCrashPoint::SessionPersisted,
        PlanRepairCrashPoint::RequestPersisted,
    ] {
        let (tmp, lifecycle, revision_store, mut parent) = plan_repair_parent_engine();
        let child = parent
            .start_plan_repair(plan_repair_fixture(
                "plan_repair_request_0001",
                &format!("fingerprint_awaiting_crash_{crash_point:?}"),
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
        let mut child_engine =
            plan_repair_restarted_child_engine(&tmp, &lifecycle, child.clone());
        child_engine.plan_repair_crash_after = Some(crash_point);

        assert!(
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
                .is_err()
        );

        let restored = plan_repair_restarted_child_engine(
            &tmp,
            &lifecycle,
            lifecycle.get_workspace_session(&child.id).unwrap(),
        );
        let snapshot = restored.plan_repair_session_state().unwrap();
        assert_eq!(
            snapshot.stage,
            crate::product::models::PlanRepairSessionStage::AwaitingConfirmation
        );
        assert_eq!(restored.current_stage(), WorkspaceStage::HumanConfirm);
        assert_eq!(
            revision_store
                .get_repair_request(&plan, &request.id)
                .unwrap()
                .status,
            crate::product::models::PlanRepairRequestStatus::AwaitingConfirmation
        );
        assert_eq!(
            snapshot
                .timeline_nodes
                .iter()
                .filter(|node| node.node_type == TimelineNodeType::PlanAmendmentConfirmation)
                .count(),
            1
        );
    }
}

#[tokio::test]
async fn plan_repair_confirm_journal_recovers_completed_confirmation_exactly_once() {
    let (tmp, lifecycle, revision_store, mut parent) = plan_repair_parent_engine();
    let child = parent
        .start_plan_repair(plan_repair_fixture(
            "plan_repair_request_0001",
            "fingerprint_confirm_crash",
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
    let mut child_engine = plan_repair_restarted_child_engine(&tmp, &lifecycle, child.clone());
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
    child_engine.plan_repair_crash_after = Some(PlanRepairCrashPoint::TimelinePersisted);

    assert!(
        child_engine
            .confirm_plan_amendment(&amendment_id)
            .await
            .is_err()
    );

    let restored = plan_repair_restarted_child_engine(
        &tmp,
        &lifecycle,
        lifecycle.get_workspace_session(&child.id).unwrap(),
    );
    let confirmation_nodes = restored
        .plan_repair_session_state()
        .unwrap()
        .timeline_nodes
        .iter()
        .filter(|node| node.node_type == TimelineNodeType::PlanAmendmentConfirmation)
        .collect::<Vec<_>>();
    assert_eq!(confirmation_nodes.len(), 1);
    assert_eq!(confirmation_nodes[0].status, TimelineNodeStatus::Completed);
}

#[tokio::test]
async fn plan_repair_cancel_journal_recovers_after_lock_release() {
    let (tmp, lifecycle, revision_store, mut parent) = plan_repair_parent_engine();
    let child = parent
        .start_plan_repair(plan_repair_fixture(
            "plan_repair_request_0001",
            "fingerprint_cancel_crash",
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
    let mut child_engine = plan_repair_restarted_child_engine(&tmp, &lifecycle, child.clone());
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
    child_engine.plan_repair_crash_after = Some(PlanRepairCrashPoint::LockReleased);

    assert!(
        child_engine
            .cancel_plan_amendment(&amendment_id, Some("cancel crash".to_string()))
            .await
            .is_err()
    );

    let restored = plan_repair_restarted_child_engine(
        &tmp,
        &lifecycle,
        lifecycle.get_workspace_session(&child.id).unwrap(),
    );
    let snapshot = restored.plan_repair_session_state().unwrap();
    assert_eq!(
        snapshot.request.status,
        crate::product::models::PlanRepairRequestStatus::Cancelled
    );
    assert_eq!(
        revision_store
            .get_plan_lineage("project_0001", "issue_0001", "work_item_plan_0001")
            .unwrap()
            .active_amendment_id,
        None
    );
    assert_eq!(
        snapshot
            .timeline_nodes
            .iter()
            .filter(|node| node.node_type == TimelineNodeType::PlanAmendmentCancelled)
            .count(),
        1
    );
}

#[tokio::test]
async fn plan_repair_cancel_journal_fails_closed_when_publication_wins_recovery_race() {
    let (tmp, lifecycle, revision_store, mut parent) = plan_repair_parent_engine();
    let child = parent
        .start_plan_repair(plan_repair_fixture(
            "plan_repair_request_0001",
            "fingerprint_cancel_publish_race",
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
    let mut child_engine = plan_repair_restarted_child_engine(&tmp, &lifecycle, child.clone());
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
    child_engine.plan_repair_crash_after = Some(PlanRepairCrashPoint::SessionPersisted);
    assert!(
        child_engine
            .cancel_plan_amendment(&amendment_id, Some("racing cancel".to_string()))
            .await
            .is_err()
    );
    revision_store
        .put_plan_amendment_publication_journal(
            &plan,
            &crate::product::models::PlanAmendmentPublicationJournal {
                id: format!("{amendment_id}_publication_journal"),
                plan_id: plan.id.clone(),
                amendment_id: amendment_id.clone(),
                phase: crate::product::models::PlanAmendmentPublicationPhase::PlanPublished,
                error: None,
                created_at: "2026-07-18T00:00:03Z".to_string(),
                updated_at: "2026-07-18T00:00:03Z".to_string(),
            },
        )
        .unwrap();

    let restored = plan_repair_restarted_child_engine(
        &tmp,
        &lifecycle,
        lifecycle.get_workspace_session(&child.id).unwrap(),
    );

    assert_eq!(restored.current_stage(), WorkspaceStage::Completed);
    assert!(
        restored
            .plan_repair_session_state()
            .unwrap()
            .error
            .as_deref()
            .is_some_and(|error| error.contains("journal recovery failed"))
    );
    assert_eq!(
        revision_store
            .get_repair_request(&plan, &request.id)
            .unwrap()
            .status,
        crate::product::models::PlanRepairRequestStatus::AwaitingConfirmation
    );
    assert_eq!(
        revision_store
            .get_plan_lineage("project_0001", "issue_0001", "work_item_plan_0001")
            .unwrap()
            .active_amendment_id
            .as_deref(),
        Some(amendment_id.as_str())
    );
}

#[tokio::test]
async fn plan_repair_cancel_rejects_active_revision_already_published_without_journal() {
    let (tmp, lifecycle, revision_store, mut parent) = plan_repair_parent_engine();
    let child = parent
        .start_plan_repair(plan_repair_fixture(
            "plan_repair_request_0001",
            "fingerprint_cancel_active_revision_race",
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
    let package = plan_repair_awaiting_package(&request.id, &amendment_id);
    plan_repair_enter_awaiting(
        &mut child_engine,
        &revision_store,
        &plan,
        package.clone(),
    )
        .await
        .unwrap();
    revision_store
        .put_plan_revision(
            &plan,
            &crate::product::models::WorkItemPlanRevision {
                id: package.amendment.new_plan_revision_id.clone(),
                plan_id: plan.id.clone(),
                revision_no: 2,
                supersedes: Some(request.base_plan_revision_id.clone()),
                reason: crate::product::models::PlanRevisionReason::SubgraphReplan,
                work_item_bindings: std::collections::BTreeMap::new(),
                dependency_graph_revision_id: package
                    .projection
                    .dependency_graph_revision_id
                    .clone(),
                validation_report_ref: package.validation.id.clone(),
                plan_projection_bundle_id: package.projection.id.clone(),
                created_at: "2026-07-18T00:00:03Z".to_string(),
            },
        )
        .unwrap();
    revision_store
        .compare_and_set_active_plan_revision(
            &plan,
            &request.base_plan_revision_id,
            &package.amendment.new_plan_revision_id,
        )
        .unwrap();

    let error = child_engine
        .cancel_plan_amendment(&amendment_id, Some("publication won".to_string()))
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        crate::product::plan_repair::PlanRepairError::AmendmentConflict { .. }
    ));
    assert_eq!(
        revision_store
            .get_repair_request(&plan, &request.id)
            .unwrap()
            .status,
        crate::product::models::PlanRepairRequestStatus::AwaitingConfirmation
    );
    assert_eq!(
        revision_store
            .get_plan_lineage("project_0001", "issue_0001", "work_item_plan_0001")
            .unwrap()
            .active_amendment_id
            .as_deref(),
        Some(amendment_id.as_str())
    );
}

#[tokio::test]
async fn plan_repair_cancel_journal_restores_request_when_publication_wins_after_request_persisted()
{
    let (tmp, lifecycle, revision_store, mut parent) = plan_repair_parent_engine();
    let child = parent
        .start_plan_repair(plan_repair_fixture(
            "plan_repair_request_0001",
            "fingerprint_cancel_request_persisted_race",
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
    let package = plan_repair_awaiting_package(&request.id, &amendment_id);
    revision_store
        .put_plan_revision(
            &plan,
            &crate::product::models::WorkItemPlanRevision {
                id: package.amendment.new_plan_revision_id.clone(),
                plan_id: plan.id.clone(),
                revision_no: 2,
                supersedes: Some(request.base_plan_revision_id.clone()),
                reason: crate::product::models::PlanRevisionReason::SubgraphReplan,
                work_item_bindings: std::collections::BTreeMap::new(),
                dependency_graph_revision_id: package
                    .projection
                    .dependency_graph_revision_id
                    .clone(),
                validation_report_ref: package.validation.id.clone(),
                plan_projection_bundle_id: package.projection.id.clone(),
                created_at: "2026-07-18T00:00:03Z".to_string(),
            },
        )
        .unwrap();
    let mut child_engine = plan_repair_restarted_child_engine(&tmp, &lifecycle, child.clone());
    plan_repair_enter_awaiting(
        &mut child_engine,
        &revision_store,
        &plan,
        package.clone(),
    )
        .await
        .unwrap();
    child_engine.plan_repair_crash_after = Some(PlanRepairCrashPoint::RequestPersisted);
    assert!(
        child_engine
            .cancel_plan_amendment(&amendment_id, Some("request persisted race".to_string()))
            .await
            .is_err()
    );
    assert_eq!(
        revision_store
            .get_repair_request(&plan, &request.id)
            .unwrap()
            .status,
        crate::product::models::PlanRepairRequestStatus::Cancelled
    );
    revision_store
        .compare_and_set_active_plan_revision(
            &plan,
            &request.base_plan_revision_id,
            &package.amendment.new_plan_revision_id,
        )
        .unwrap();

    let restored = plan_repair_restarted_child_engine(
        &tmp,
        &lifecycle,
        lifecycle.get_workspace_session(&child.id).unwrap(),
    );

    assert_eq!(
        revision_store
            .get_repair_request(&plan, &request.id)
            .unwrap()
            .status,
        crate::product::models::PlanRepairRequestStatus::AwaitingConfirmation
    );
    assert_eq!(
        restored
            .plan_repair_session_state()
            .unwrap()
            .request
            .status,
        crate::product::models::PlanRepairRequestStatus::AwaitingConfirmation
    );
    assert_eq!(
        revision_store
            .get_plan_lineage("project_0001", "issue_0001", "work_item_plan_0001")
            .unwrap()
            .active_amendment_id
            .as_deref(),
        Some(amendment_id.as_str())
    );
}
