#[tokio::test]
async fn plan_repair_awaiting_rejects_persisted_validation_from_old_revision() {
    let (tmp, lifecycle, revision_store, mut parent) = plan_repair_parent_engine();
    let child = parent
        .start_plan_repair(plan_repair_fixture(
            "plan_repair_request_0001",
            "fingerprint_old_validation_provenance",
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
    let mut package = plan_repair_awaiting_package(&request.id, &amendment_id);
    let mut child_engine = plan_repair_restarted_child_engine(&tmp, &lifecycle, child);
    plan_repair_persist_awaiting_provenance(
        &revision_store,
        &plan,
        &request.id,
        &mut package,
    );
    child_engine
        .plan_repair_snapshot
        .as_mut()
        .unwrap()
        .candidate_package_artifact_id = Some(
        package
            .package_identity
            .candidate_package_artifact_id
            .clone(),
    );
    let before = child_engine.plan_repair_session_state().unwrap().clone();
    package.validation.plan_revision_id = request.base_plan_revision_id.clone();

    let error = child_engine
        .enter_plan_repair_awaiting_confirmation(package)
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        crate::product::plan_repair::PlanRepairError::InvalidRepairTarget(_)
    ));
    assert_eq!(child_engine.plan_repair_session_state(), Some(&before));
}

#[tokio::test]
async fn plan_repair_awaiting_rejects_persisted_outline_review_for_old_revision() {
    let (tmp, lifecycle, revision_store, mut parent) = plan_repair_parent_engine();
    let child = parent
        .start_plan_repair(plan_repair_fixture(
            "plan_repair_request_0001",
            "fingerprint_old_review_provenance",
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
    let mut package = plan_repair_awaiting_package(&request.id, &amendment_id);
    let candidate = plan_repair_persist_candidate_package(
        &revision_store,
        &plan,
        &request,
        &package.amendment,
        &package.projection,
        &package.validation,
        &package.impact,
    );
    package.package_identity.candidate_package_artifact_id = candidate.id;
    package.package_identity.candidate_package_fingerprint =
        candidate.candidate_package_fingerprint;
    let mut old_attestation = plan_repair_review_attestation(&package);
    old_attestation.reviewed_plan_revision_id = request.base_plan_revision_id.clone();
    revision_store
        .put_plan_repair_review_attestation(&plan, &old_attestation)
        .unwrap();
    let mut child_engine = plan_repair_restarted_child_engine(&tmp, &lifecycle, child);
    let before = child_engine.plan_repair_session_state().unwrap().clone();

    let error = child_engine
        .enter_plan_repair_awaiting_confirmation(package)
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        crate::product::plan_repair::PlanRepairError::InvalidRepairTarget(_)
    ));
    assert_eq!(child_engine.plan_repair_session_state(), Some(&before));
}

struct PlanRepairCancelFixture {
    _tmp: TempDir,
    lifecycle: LifecycleStore,
    revision_store: crate::product::work_item_revision_store::WorkItemRevisionStore,
    plan: crate::product::models::WorkItemPlanLineage,
    request: crate::product::models::PlanRepairRequest,
    amendment_id: String,
    engine: WorkspaceEngine,
}

async fn plan_repair_cancel_ready(fingerprint: &str) -> PlanRepairCancelFixture {
    let (tmp, lifecycle, revision_store, mut parent) = plan_repair_parent_engine();
    let child = parent
        .start_plan_repair(plan_repair_fixture("plan_repair_request_0001", fingerprint))
        .await
        .unwrap();
    let plan = revision_store
        .get_plan_lineage("project_0001", "issue_0001", "work_item_plan_0001")
        .unwrap();
    let request = revision_store
        .get_repair_request(&plan, "plan_repair_request_0001")
        .unwrap();
    let amendment_id = request.amendment_id.clone().unwrap();
    let mut engine = plan_repair_restarted_child_engine(&tmp, &lifecycle, child);
    plan_repair_enter_awaiting(
        &mut engine,
        &revision_store,
        &plan,
        plan_repair_awaiting_package(&request.id, &amendment_id),
    )
    .await
    .unwrap();
    PlanRepairCancelFixture {
        _tmp: tmp,
        lifecycle,
        revision_store,
        plan,
        request,
        amendment_id,
        engine,
    }
}

fn plan_repair_assert_cancel_rejected(error: crate::product::plan_repair::PlanRepairError) {
    assert!(matches!(
        error,
        crate::product::plan_repair::PlanRepairError::InvalidRepairTarget(_)
            | crate::product::plan_repair::PlanRepairError::AmendmentConflict { .. }
    ));
}

#[tokio::test]
async fn plan_repair_cancel_direct_rejects_non_awaiting_snapshot_stages() {
    use crate::product::models::PlanRepairSessionStage;

    for stage in [
        PlanRepairSessionStage::Failed,
        PlanRepairSessionStage::AmendmentConflict,
        PlanRepairSessionStage::Published,
        PlanRepairSessionStage::ApplyingAmendment,
        PlanRepairSessionStage::AmendmentApplyFailed,
        PlanRepairSessionStage::Completed,
    ] {
        let mut fixture = plan_repair_cancel_ready(&format!("fingerprint_cancel_{stage:?}")).await;
        fixture.engine.plan_repair_snapshot.as_mut().unwrap().stage = stage;
        let before = fixture.engine.plan_repair_session_state().unwrap().clone();

        let error = fixture
            .engine
            .cancel_plan_amendment(&fixture.amendment_id, None)
            .await
            .unwrap_err();

        plan_repair_assert_cancel_rejected(error);
        assert_eq!(fixture.engine.plan_repair_session_state(), Some(&before));
        assert_eq!(
            fixture
                .revision_store
                .get_repair_request(&fixture.plan, &fixture.request.id)
                .unwrap()
                .status,
            crate::product::models::PlanRepairRequestStatus::AwaitingConfirmation
        );
    }
}

#[tokio::test]
async fn plan_repair_cancel_direct_rejects_non_awaiting_authoritative_request_statuses() {
    use crate::product::models::PlanRepairRequestStatus;

    for status in [
        PlanRepairRequestStatus::Failed,
        PlanRepairRequestStatus::Published,
        PlanRepairRequestStatus::Applied,
        PlanRepairRequestStatus::InProgress,
    ] {
        let mut fixture =
            plan_repair_cancel_ready(&format!("fingerprint_cancel_status_{status:?}")).await;
        fixture
            .revision_store
            .update_repair_request_status(&fixture.plan, &fixture.request.id, status.clone())
            .unwrap();
        let before = fixture.engine.plan_repair_session_state().unwrap().clone();

        let error = fixture
            .engine
            .cancel_plan_amendment(&fixture.amendment_id, None)
            .await
            .unwrap_err();

        plan_repair_assert_cancel_rejected(error);
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
async fn plan_repair_cancel_direct_revalidates_package_base_and_active_lock() {
    for case in ["missing_package", "stale_base", "missing_lock"] {
        let mut fixture = plan_repair_cancel_ready(&format!("fingerprint_cancel_{case}")).await;
        match case {
            "missing_package" => {
                fixture
                    .engine
                    .plan_repair_snapshot
                    .as_mut()
                    .unwrap()
                    .package_identity = None;
            }
            "stale_base" => {
                fixture
                    .revision_store
                    .put_plan_revision(
                        &fixture.plan,
                        &crate::product::models::WorkItemPlanRevision { id: "plan_revision_external".to_string(),
                        plan_id: fixture.plan.id.clone(),
                        revision_no: 2,
                        supersedes: Some(fixture.request.base_plan_revision_id.clone()),
                        reason: crate::product::models::PlanRevisionReason::SubgraphReplan,
                        work_item_bindings: std::collections::BTreeMap::new(),
                        dependency_graph_revision_id: "dependency_graph_external".to_string(),
                        validation_report_ref: "validation_external".to_string(), plan_projection_bundle_id: "projection_external".to_string(), publication_provenance_ref: None, created_at: "2026-07-18T00:00:03Z".to_string(),  },
                    )
                    .unwrap();
                fixture
                    .revision_store
                    .compare_and_set_active_plan_revision(
                        &fixture.plan,
                        &fixture.request.base_plan_revision_id,
                        "plan_revision_external",
                    )
                    .unwrap();
            }
            "missing_lock" => {
                fixture
                    .revision_store
                    .release_active_amendment(&fixture.plan, &fixture.amendment_id)
                    .unwrap();
            }
            _ => unreachable!(),
        }
        let before = fixture.engine.plan_repair_session_state().unwrap().clone();

        let error = fixture
            .engine
            .cancel_plan_amendment(&fixture.amendment_id, None)
            .await
            .unwrap_err();

        plan_repair_assert_cancel_rejected(error);
        assert_eq!(fixture.engine.plan_repair_session_state(), Some(&before));
        assert_eq!(
            fixture
                .revision_store
                .get_repair_request(&fixture.plan, &fixture.request.id)
                .unwrap()
                .status,
            crate::product::models::PlanRepairRequestStatus::AwaitingConfirmation
        );
    }
}

#[tokio::test]
async fn plan_repair_cancel_replay_requires_authoritative_terminal_state() {
    for case in [
        "authoritative_request",
        "authoritative_session",
        "snapshot_stage",
        "engine_stage",
    ] {
        let mut fixture = plan_repair_cancel_ready(&format!("fingerprint_replay_{case}")).await;
        fixture
            .engine
            .cancel_plan_amendment(&fixture.amendment_id, None)
            .await
            .unwrap();
        match case {
            "authoritative_request" => {
                fixture
                    .revision_store
                    .update_repair_request_status(
                        &fixture.plan,
                        &fixture.request.id,
                        crate::product::models::PlanRepairRequestStatus::AwaitingConfirmation,
                    )
                    .unwrap();
            }
            "authoritative_session" => {
                fixture
                    .lifecycle
                    .update_workspace_session_status(
                        &fixture.engine.session.session_id,
                        crate::product::models::WorkspaceSessionStatus::Running,
                    )
                    .unwrap();
            }
            "snapshot_stage" => {
                fixture.engine.plan_repair_snapshot.as_mut().unwrap().stage =
                    crate::product::models::PlanRepairSessionStage::Failed;
            }
            "engine_stage" => {
                fixture.engine.session.stage = WorkspaceStage::HumanConfirm;
            }
            _ => unreachable!(),
        }
        let before = fixture.engine.plan_repair_session_state().unwrap().clone();

        let error = fixture
            .engine
            .cancel_plan_amendment(&fixture.amendment_id, None)
            .await
            .unwrap_err();

        plan_repair_assert_cancel_rejected(error);
        assert_eq!(fixture.engine.plan_repair_session_state(), Some(&before));
    }
}
