#[tokio::test]
async fn plan_repair_confirm_and_publish_recovers_partial_publication_with_same_attestation() {
    let root = tempfile::tempdir().unwrap();
    let runtime = crate::web::test_controls::PlanRepairFixtureRuntime::seed(
        root.path(),
        crate::web::test_controls::PlanRepairFixtureControl::default(),
    )
    .await
    .unwrap();
    let identity = runtime.drive_until_awaiting_confirmation().await.unwrap();
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let child = lifecycle
        .get_workspace_session(&identity.child_session_id)
        .unwrap();
    let (tx, _) = mpsc::channel(64);
    let mut engine = WorkspaceEngine::new_persistent(
        Arc::new(CheckpointStore::new(root.path().join("checkpoints"))),
        lifecycle.clone(),
        tx,
        WorkspaceSession::from_record(child),
    );
    let revision_store = WorkItemRevisionStore::new(app_paths);
    let snapshot = engine.plan_repair_session_state().unwrap().clone();
    let plan = revision_store
        .get_plan_lineage(
            &engine.session().project_id,
            &engine.session().issue_id,
            &snapshot.request.plan_id,
        )
        .unwrap();
    let package_identity = snapshot.package_identity.unwrap();
    let attestation = revision_store
        .get_plan_repair_review_attestation(
            &plan,
            &package_identity.review_attestation_id,
        )
        .unwrap();
    let prepared = crate::product::plan_repair::PlanRepairEngine::new(
        revision_store.clone(),
        plan.clone(),
    )
    .load_prepared_amendment(&package_identity.candidate_package_artifact_id)
    .unwrap();
    let failpoint = crate::product::work_item_revision_store::register_plan_amendment_publication_failpoint(
        &revision_store,
        &plan,
        &prepared.publication_ids.journal_id,
        crate::product::work_item_revision_store::PlanAmendmentPublicationCheckpoint::JournalPlanPublished,
    );

    let error = engine
        .confirm_and_publish_plan_amendment(&identity.amendment_id, "workspace_user")
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        crate::product::plan_repair::PlanRepairError::Store(
            crate::product::json_store::ProductStoreError::Io(message)
        ) if message.contains("amendment_publication_failpoint")
    ));
    drop(failpoint);

    let manifest = engine
        .confirm_and_publish_plan_amendment(&identity.amendment_id, "workspace_user")
        .await
        .unwrap();
    assert_eq!(manifest.id, identity.amendment_id);
    let published = engine.plan_repair_session_state().unwrap();
    assert_eq!(
        published.stage,
        crate::product::models::PlanRepairSessionStage::Published
    );
    assert_eq!(
        revision_store
            .get_repair_request(&plan, &published.request.id)
            .unwrap()
            .status,
        crate::product::models::PlanRepairRequestStatus::Published
    );
    assert_eq!(
        revision_store
            .get_plan_repair_review_attestation(&plan, &attestation.id)
            .unwrap(),
        attestation
    );
}
