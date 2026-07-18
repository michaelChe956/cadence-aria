fn plan_repair_review_attestation(
    package: &crate::product::models::PlanRepairAwaitingConfirmationPackage,
) -> crate::product::models::PlanRepairReviewAttestation {
    crate::product::models::PlanRepairReviewAttestation {
        id: package.package_identity.review_attestation_id.clone(),
        request_id: package.package_identity.request_id.clone(),
        amendment_id: package.package_identity.amendment_id.clone(),
        plan_id: package.package_identity.plan_id.clone(),
        base_plan_revision_id: package.package_identity.base_plan_revision_id.clone(),
        reviewed_plan_revision_id: package
            .package_identity
            .reviewed_plan_revision_id
            .clone(),
        plan_projection_bundle_id: package.package_identity.projection_bundle_id.clone(),
        generation_round_id: package.plan_review.generation_round_id.clone(),
        accepted_impact_scope: package
            .amendment
            .revalidation_required_units
            .iter()
            .chain(package.amendment.stale_units.iter())
            .cloned()
            .collect(),
        risk_acceptance_reason: None,
        review: package.plan_review.clone(),
        created_at: "2026-07-18T00:00:02Z".to_string(),
    }
}

fn plan_repair_persist_awaiting_provenance(
    revision_store: &crate::product::work_item_revision_store::WorkItemRevisionStore,
    plan: &crate::product::models::WorkItemPlanLineage,
    package: &crate::product::models::PlanRepairAwaitingConfirmationPackage,
) {
    let mut persisted_projection = package.projection.clone();
    persisted_projection.id = package.package_identity.projection_bundle_id.clone();
    let mut persisted_validation = package.validation.clone();
    persisted_validation.id = package.package_identity.validation_report_id.clone();
    persisted_validation.plan_id = plan.id.clone();
    let mut persisted_review = plan_repair_review_attestation(package);
    persisted_review.plan_id = plan.id.clone();
    revision_store
        .put_plan_projection_bundle(plan, &persisted_projection)
        .unwrap();
    revision_store
        .put_plan_validation_report(plan, &persisted_validation)
        .unwrap();
    revision_store
        .put_plan_repair_review_attestation(plan, &persisted_review)
        .unwrap();
}

async fn plan_repair_enter_awaiting(
    engine: &mut WorkspaceEngine,
    revision_store: &crate::product::work_item_revision_store::WorkItemRevisionStore,
    plan: &crate::product::models::WorkItemPlanLineage,
    package: crate::product::models::PlanRepairAwaitingConfirmationPackage,
) -> Result<(), crate::product::plan_repair::PlanRepairError> {
    plan_repair_persist_awaiting_provenance(revision_store, plan, &package);
    engine.enter_plan_repair_awaiting_confirmation(package).await
}

async fn plan_repair_assert_status_wins_journal_recovery(
    operation: &str,
    crash_point: PlanRepairCrashPoint,
    status: crate::product::models::PlanRepairRequestStatus,
) {
    let (tmp, lifecycle, revision_store, mut parent) = plan_repair_parent_engine();
    let fingerprint = format!("fingerprint_{operation}_{crash_point:?}_{status:?}");
    let child = parent
        .start_plan_repair(plan_repair_fixture(
            "plan_repair_request_0001",
            &fingerprint,
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
    if operation == "confirm" {
        plan_repair_enter_awaiting(
            &mut child_engine,
            &revision_store,
            &plan,
            plan_repair_awaiting_package(&request.id, &amendment_id),
        )
        .await
        .unwrap();
    }
    child_engine.plan_repair_crash_after = Some(crash_point);

    let crashed = if operation == "awaiting" {
        plan_repair_enter_awaiting(
            &mut child_engine,
            &revision_store,
            &plan,
            plan_repair_awaiting_package(&request.id, &amendment_id),
        )
        .await
        .is_err()
    } else {
        child_engine
            .confirm_plan_amendment(&amendment_id)
            .await
            .is_err()
    };
    assert!(crashed);
    revision_store
        .update_repair_request_status(&plan, &request.id, status.clone())
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
        status
    );
    assert_eq!(restored.current_stage(), WorkspaceStage::Completed);
    assert_ne!(restored.current_stage(), WorkspaceStage::HumanConfirm);
}

#[tokio::test]
async fn plan_repair_awaiting_journal_does_not_downgrade_published_or_applied_request() {
    for crash_point in [
        PlanRepairCrashPoint::TimelinePersisted,
        PlanRepairCrashPoint::SnapshotPersisted,
        PlanRepairCrashPoint::SessionPersisted,
    ] {
        for status in [
            crate::product::models::PlanRepairRequestStatus::Published,
            crate::product::models::PlanRepairRequestStatus::Applied,
        ] {
            plan_repair_assert_status_wins_journal_recovery("awaiting", crash_point, status).await;
        }
    }
}

#[tokio::test]
async fn plan_repair_confirm_journal_does_not_downgrade_published_or_applied_request() {
    for crash_point in [
        PlanRepairCrashPoint::TimelinePersisted,
        PlanRepairCrashPoint::SnapshotPersisted,
        PlanRepairCrashPoint::SessionPersisted,
    ] {
        for status in [
            crate::product::models::PlanRepairRequestStatus::Published,
            crate::product::models::PlanRepairRequestStatus::Applied,
        ] {
            plan_repair_assert_status_wins_journal_recovery("confirm", crash_point, status).await;
        }
    }
}

#[tokio::test]
async fn plan_repair_awaiting_idempotent_rejects_authoritative_successor_status() {
    for status in [
        crate::product::models::PlanRepairRequestStatus::Published,
        crate::product::models::PlanRepairRequestStatus::Applied,
    ] {
        let mut fixture =
            plan_repair_cancel_ready(&format!("fingerprint_awaiting_idempotent_{status:?}"))
                .await;
        let package = awaiting_confirmation_package_from_snapshot(
            fixture.engine.plan_repair_session_state().unwrap(),
        )
        .unwrap();
        fixture
            .revision_store
            .update_repair_request_status(&fixture.plan, &fixture.request.id, status.clone())
            .unwrap();
        let before = fixture.engine.plan_repair_session_state().unwrap().clone();

        let error = fixture
            .engine
            .enter_plan_repair_awaiting_confirmation(package)
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            crate::product::plan_repair::PlanRepairError::InvalidRepairTarget(_)
                | crate::product::plan_repair::PlanRepairError::Store(_)
        ));
        assert_eq!(fixture.engine.plan_repair_session_state(), Some(&before));
        assert_eq!(
            fixture
                .revision_store
                .get_repair_request(&fixture.plan, &fixture.request.id)
                .unwrap()
                .status,
            status
        );
    }
}

#[tokio::test]
async fn plan_repair_orphan_recovery_rejects_awaiting_request_before_writes() {
    let (_tmp, lifecycle, revision_store, mut parent) = plan_repair_parent_engine();
    let fingerprint = "fingerprint_awaiting_orphan";
    let child = parent
        .start_plan_repair(plan_repair_fixture(
            "plan_repair_request_persisted",
            fingerprint,
        ))
        .await
        .unwrap();
    let plan = revision_store
        .get_plan_lineage("project_0001", "issue_0001", "work_item_plan_0001")
        .unwrap();
    let request = revision_store
        .get_repair_request(&plan, "plan_repair_request_persisted")
        .unwrap();
    let amendment_id = request.amendment_id.clone().unwrap();
    let mut child_engine =
        plan_repair_restarted_child_engine(&_tmp, &lifecycle, child.clone());
    plan_repair_enter_awaiting(
        &mut child_engine,
        &revision_store,
        &plan,
        plan_repair_awaiting_package(&request.id, &amendment_id),
    )
    .await
    .unwrap();
    let before_request = revision_store
        .get_repair_request(&plan, &request.id)
        .unwrap();
    let before_snapshot = lifecycle
        .load_plan_repair_session_state("project_0001", "issue_0001", &child.id)
        .unwrap();
    let before_session = lifecycle.get_workspace_session(&child.id).unwrap();
    let before_sessions = lifecycle
        .list_workspace_sessions("project_0001", "issue_0001")
        .unwrap();
    let link = lifecycle.get_session_link(&child.id).unwrap();
    let link_path = lifecycle
        .app_paths()
        .issue_lifecycle_root("project_0001", "issue_0001")
        .join("workspace-session-links")
        .join(format!("{}.json", link.id));
    std::fs::remove_file(link_path).unwrap();
    let mut retry = plan_repair_fixture("plan_repair_request_incoming", fingerprint);
    retry.trigger_attempt_id = "coding_attempt_0002".to_string();
    retry.trigger_unit_run_id = "coding_unit_run_0003".to_string();
    retry.trigger_review_id = Some("code_review_0002".to_string());
    retry.trigger_finding_id = "finding_incoming".to_string();
    retry.evidence[0].source_ref = "code_review_0002#finding_incoming".to_string();

    let error = parent.start_plan_repair(retry).await.unwrap_err();

    assert!(matches!(
        error,
        crate::product::plan_repair::PlanRepairError::InvalidRepairTarget(_)
            | crate::product::plan_repair::PlanRepairError::Store(
                crate::product::json_store::ProductStoreError::IdentityMismatch { .. }
            )
    ));
    assert_eq!(
        revision_store
            .get_repair_request(&plan, &request.id)
            .unwrap(),
        before_request
    );
    assert_eq!(
        lifecycle
            .load_plan_repair_session_state("project_0001", "issue_0001", &child.id)
            .unwrap(),
        before_snapshot
    );
    assert_eq!(lifecycle.get_workspace_session(&child.id).unwrap(), before_session);
    assert_eq!(
        lifecycle
            .list_workspace_sessions("project_0001", "issue_0001")
            .unwrap(),
        before_sessions
    );
    assert!(
        lifecycle
            .list_session_links("project_0001", "issue_0001")
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn plan_repair_reuse_linked_awaiting_request_preserves_status_and_syncs_snapshot() {
    let (_tmp, lifecycle, revision_store, mut parent) = plan_repair_parent_engine();
    let fingerprint = "fingerprint_linked_awaiting_reuse";
    let child = parent
        .start_plan_repair(plan_repair_fixture(
            "plan_repair_request_persisted",
            fingerprint,
        ))
        .await
        .unwrap();
    let plan = revision_store
        .get_plan_lineage("project_0001", "issue_0001", "work_item_plan_0001")
        .unwrap();
    let request = revision_store
        .get_repair_request(&plan, "plan_repair_request_persisted")
        .unwrap();
    let amendment_id = request.amendment_id.clone().unwrap();
    let mut child_engine =
        plan_repair_restarted_child_engine(&_tmp, &lifecycle, child.clone());
    plan_repair_enter_awaiting(
        &mut child_engine,
        &revision_store,
        &plan,
        plan_repair_awaiting_package(&request.id, &amendment_id),
    )
    .await
    .unwrap();
    let mut retry = plan_repair_fixture("plan_repair_request_incoming", fingerprint);
    retry.trigger_attempt_id = "coding_attempt_0002".to_string();
    retry.trigger_unit_run_id = "coding_unit_run_0003".to_string();
    retry.trigger_review_id = Some("code_review_0002".to_string());
    retry.trigger_finding_id = "finding_incoming".to_string();
    retry.evidence[0].source_ref = "code_review_0002#finding_incoming".to_string();

    let reused = parent.start_plan_repair(retry).await.unwrap();

    assert_eq!(reused.id, child.id);
    assert_eq!(
        reused.status,
        crate::product::models::WorkspaceSessionStatus::WaitingForHuman
    );
    let stored_request = revision_store
        .get_repair_request(&plan, &request.id)
        .unwrap();
    assert_eq!(
        stored_request.status,
        crate::product::models::PlanRepairRequestStatus::AwaitingConfirmation
    );
    assert_eq!(stored_request.evidence.len(), 2);
    let snapshot = lifecycle
        .load_plan_repair_session_state("project_0001", "issue_0001", &child.id)
        .unwrap()
        .unwrap();
    assert_eq!(snapshot.request, stored_request);
    assert_eq!(
        snapshot.stage,
        crate::product::models::PlanRepairSessionStage::AwaitingConfirmation
    );
    assert_eq!(
        lifecycle
            .list_session_links("project_0001", "issue_0001")
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn plan_repair_active_amendment_arbitration_precedes_selected_request_reuse() {
    let (_tmp, lifecycle, revision_store, mut parent) = plan_repair_parent_engine();
    let active_fingerprint = "fingerprint_active_g";
    let active_child = parent
        .start_plan_repair(plan_repair_fixture(
            "plan_repair_request_active_g",
            active_fingerprint,
        ))
        .await
        .unwrap();
    let plan = revision_store
        .get_plan_lineage("project_0001", "issue_0001", "work_item_plan_0001")
        .unwrap();
    let active_request = revision_store
        .get_repair_request(&plan, "plan_repair_request_active_g")
        .unwrap();
    let selected_fingerprint = "fingerprint_selected_f";
    let selected_request = plan_repair_fixture(
        "plan_repair_request_selected_f",
        selected_fingerprint,
    );
    revision_store
        .put_repair_request(&plan, &selected_request)
        .unwrap();
    let before_selected = revision_store
        .get_repair_request(&plan, &selected_request.id)
        .unwrap();
    let before_active = revision_store
        .get_repair_request(&plan, &active_request.id)
        .unwrap();
    let before_sessions = lifecycle
        .list_workspace_sessions("project_0001", "issue_0001")
        .unwrap()
        .len();
    let before_links = lifecycle
        .list_session_links("project_0001", "issue_0001")
        .unwrap()
        .len();
    let before_requests = revision_store.list_open_repair_requests(&plan).unwrap().len();
    let before_snapshot = lifecycle
        .load_plan_repair_session_state("project_0001", "issue_0001", &active_child.id)
        .unwrap();
    let selected_amendment_id = format!("plan_amendment_{selected_fingerprint}");
    let selected_child_id = format!("workspace_session_{selected_amendment_id}");
    let selected_snapshot_path = lifecycle
        .workspace_timeline_root_for_issue_session(
            "project_0001",
            "issue_0001",
            &selected_child_id,
        )
        .unwrap()
        .join("plan_repair_session_state.json");
    assert!(!selected_snapshot_path.exists());
    let mut incoming = plan_repair_fixture(
        "plan_repair_request_incoming_f",
        selected_fingerprint,
    );
    incoming.trigger_attempt_id = "coding_attempt_0002".to_string();
    incoming.trigger_unit_run_id = "coding_unit_run_0003".to_string();
    incoming.trigger_review_id = Some("code_review_0002".to_string());
    incoming.trigger_finding_id = "finding_incoming_f".to_string();
    incoming.evidence[0].source_ref = "code_review_0002#finding_incoming_f".to_string();

    let result = parent.start_plan_repair(incoming).await;

    assert_eq!(
        revision_store
            .get_repair_request(&plan, &selected_request.id)
            .unwrap(),
        before_selected
    );
    assert_eq!(
        revision_store
            .get_repair_request(&plan, &active_request.id)
            .unwrap(),
        before_active
    );
    let returned = result.unwrap();
    assert_eq!(returned.id, active_child.id);
    assert_eq!(
        revision_store
            .get_plan_lineage("project_0001", "issue_0001", "work_item_plan_0001")
            .unwrap()
            .active_amendment_id,
        active_request.amendment_id
    );
    assert_eq!(
        revision_store.list_open_repair_requests(&plan).unwrap().len(),
        before_requests
    );
    assert_eq!(
        lifecycle
            .list_workspace_sessions("project_0001", "issue_0001")
            .unwrap()
            .len(),
        before_sessions
    );
    assert_eq!(
        lifecycle
            .list_session_links("project_0001", "issue_0001")
            .unwrap()
            .len(),
        before_links
    );
    assert_eq!(
        lifecycle
            .load_plan_repair_session_state("project_0001", "issue_0001", &active_child.id)
            .unwrap(),
        before_snapshot
    );
    assert!(!selected_snapshot_path.exists());
    assert!(lifecycle.get_workspace_session(&selected_child_id).is_err());
}
