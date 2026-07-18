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
