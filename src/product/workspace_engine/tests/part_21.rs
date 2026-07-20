fn plan_repair_fixture(id: &str, fingerprint: &str) -> crate::product::models::PlanRepairRequest {
    crate::product::models::PlanRepairRequest {
        id: id.to_string(),
        plan_id: "work_item_plan_0001".to_string(),
        base_plan_revision_id: "plan_revision_0001".to_string(),
        trigger_attempt_id: "coding_attempt_0001".to_string(),
        trigger_unit_run_id: "coding_unit_run_0002".to_string(),
        trigger_review_id: Some("code_review_0001".to_string()),
        trigger_finding_id: format!("finding_{id}"),
        amendment_id: None,
        defect_class: crate::product::models::PlanDefectClass::CurrentWorkItemInvalid,
        reason_code: "current_contract_invalid".to_string(),
        repair_target: crate::product::models::RepairTarget {
            kind: crate::product::models::RepairTargetKind::CurrentWorkItem,
            logical_work_item_ids: vec!["logical_work_item_0001".to_string()],
            work_item_revision_ids: vec!["work_item_revision_0001".to_string()],
        },
        contract_refs: vec!["contract_0001".to_string()],
        capability_refs: vec!["capability_0001".to_string()],
        evidence: vec![crate::product::models::PlanDefectEvidence {
            kind: "review_finding".to_string(),
            source_ref: format!("code_review_0001#finding_{id}"),
            message: "当前 Work Item contract 无法继续执行".to_string(),
        }],
        fingerprint: fingerprint.to_string(),
        status: crate::product::models::PlanRepairRequestStatus::Open,
        created_at: "2026-07-18T00:00:00Z".to_string(),
        updated_at: "2026-07-18T00:00:00Z".to_string(),
    }
}

fn plan_repair_parent_engine(
) -> (
    TempDir,
    LifecycleStore,
    crate::product::work_item_revision_store::WorkItemRevisionStore,
    WorkspaceEngine,
) {
    let (tmp, checkpoint_store) = setup();
    let app_paths = ProductAppPaths::new(tmp.path().join(".aria"));
    let lifecycle_store = LifecycleStore::new(app_paths.clone());
    let revision_store =
        crate::product::work_item_revision_store::WorkItemRevisionStore::new(app_paths);
    let plan = crate::product::models::WorkItemPlanLineage {
        id: "work_item_plan_0001".to_string(),
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        story_spec_refs: vec!["story_spec_0001".to_string()],
        design_spec_refs: vec!["design_spec_0001".to_string()],
        active_revision_id: None,
        active_amendment_id: None,
        created_at: "2026-07-18T00:00:00Z".to_string(),
        updated_at: "2026-07-18T00:00:00Z".to_string(),
    };
    revision_store.put_plan_lineage(&plan).unwrap();
    revision_store
        .put_plan_revision(
            &plan,
            &crate::product::models::WorkItemPlanRevision {
                id: "plan_revision_0001".to_string(),
                plan_id: plan.id.clone(),
                revision_no: 1,
                supersedes: None,
                reason: crate::product::models::PlanRevisionReason::InitialCompile,
                work_item_bindings: std::collections::BTreeMap::from([(
                    "logical_work_item_0001".to_string(),
                    "work_item_revision_0001".to_string(),
                )]),
                dependency_graph_revision_id: "dependency_graph_revision_0001".to_string(),
                validation_report_ref: "plan_validation_report_0001".to_string(),
                plan_projection_bundle_id: "plan_projection_bundle_0001".to_string(),
                created_at: "2026-07-18T00:00:01Z".to_string(),
            },
        )
        .unwrap();
    revision_store
        .set_active_plan_revision(&plan, "plan_revision_0001")
        .unwrap();
    let session_record = lifecycle_store
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: plan.id,
            workspace_type: WorkspaceType::WorkItemPlan,
            author_provider: ProviderName::ClaudeCode,
            reviewer_provider: ProviderName::Codex,
            review_rounds: 2,
            superpowers_enabled: true,
            openspec_enabled: true,
        })
        .unwrap();
    let (tx, _) = mpsc::channel(64);
    let engine = WorkspaceEngine::new_persistent(
        checkpoint_store,
        lifecycle_store.clone(),
        tx,
        WorkspaceSession::from_record(session_record),
    );
    (tmp, lifecycle_store, revision_store, engine)
}

fn plan_repair_manifest(
    request_id: &str,
    amendment_id: &str,
) -> crate::product::models::PlanAmendmentManifest {
    crate::product::models::PlanAmendmentManifest {
        id: amendment_id.to_string(),
        repair_request_id: request_id.to_string(),
        previous_plan_revision_id: "plan_revision_0001".to_string(),
        new_plan_revision_id: "plan_revision_0002".to_string(),
        revised_work_items: std::collections::BTreeMap::new(),
        superseded_revisions: Vec::new(),
        dependency_graph_changes: Vec::new(),
        contract_deltas: Vec::new(),
        unaffected_units: Vec::new(),
        revalidation_required_units: vec!["logical_work_item_0001".to_string()],
        stale_units: Vec::new(),
        replacement_units: std::collections::BTreeMap::new(),
        resume_target: crate::product::models::AmendmentResumeTarget {
            logical_work_item_id: "logical_work_item_0001".to_string(),
            mode: crate::product::models::AmendmentResumeMode::Reexecute,
        },
        created_at: "2026-07-18T00:00:02Z".to_string(),
    }
}

fn plan_repair_restarted_child_engine(
    tmp: &TempDir,
    lifecycle: &LifecycleStore,
    child: crate::product::models::WorkspaceSessionRecord,
) -> WorkspaceEngine {
    let (tx, _) = mpsc::channel(64);
    WorkspaceEngine::new_persistent(
        Arc::new(CheckpointStore::new(tmp.path().join("repair-checkpoints"))),
        lifecycle.clone(),
        tx,
        WorkspaceSession::from_record(child),
    )
}

#[tokio::test]
async fn plan_repair_child_session_creates_link_without_changing_parent_engine() {
    let (_tmp, lifecycle, _revision_store, mut engine) = plan_repair_parent_engine();
    let parent_session_id = engine.session().session_id.clone();

    let child = engine
        .start_plan_repair(plan_repair_fixture("plan_repair_request_0001", "fingerprint_same"))
        .await
        .unwrap();

    assert_eq!(child.workspace_type, WorkspaceType::WorkItemPlan);
    assert_ne!(child.id, parent_session_id);
    assert_eq!(engine.session().session_id, parent_session_id);
    let link = lifecycle.get_session_link(&child.id).unwrap();
    assert_eq!(link.parent_session_id, "coding_attempt_0001");
    assert_eq!(
        link.relation,
        crate::product::models::WorkspaceSessionRelation::PlanRepair
    );
    assert_eq!(
        link.return_context.original_unit_run_id,
        "coding_unit_run_0002"
    );
    assert_eq!(
        link.return_context.original_route,
        "/workbench/projects/project_0001/issues/issue_0001/coding/coding_attempt_0001"
    );
}

#[tokio::test]
async fn plan_repair_child_session_reuses_open_fingerprint_and_active_amendment() {
    let (_tmp, lifecycle, revision_store, mut engine) = plan_repair_parent_engine();
    let first_request = plan_repair_fixture("plan_repair_request_0001", "fingerprint_same");
    let mut duplicate_request = first_request.clone();
    duplicate_request.id = "plan_repair_request_0002".to_string();
    duplicate_request.trigger_attempt_id = "coding_attempt_0002".to_string();
    duplicate_request.trigger_unit_run_id = "coding_unit_run_0003".to_string();
    duplicate_request.trigger_review_id = Some("code_review_0002".to_string());
    duplicate_request.trigger_finding_id = "finding_duplicate".to_string();
    duplicate_request.evidence[0].source_ref =
        "code_review_0002#finding_duplicate".to_string();
    let first = engine
        .start_plan_repair(first_request)
        .await
        .unwrap();
    let duplicate = engine.start_plan_repair(duplicate_request).await.unwrap();
    assert_eq!(duplicate.id, first.id);
    assert_eq!(
        lifecycle
            .list_workspace_sessions("project_0001", "issue_0001")
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        lifecycle
            .list_session_links("project_0001", "issue_0001")
            .unwrap()
            .len(),
        1
    );
    let plan = revision_store
        .get_plan_lineage("project_0001", "issue_0001", "work_item_plan_0001")
        .unwrap();
    assert!(plan.active_amendment_id.is_some());
    let requests = revision_store.list_open_repair_requests(&plan).unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].evidence.len(), 2);
}

#[tokio::test]
async fn plan_repair_child_session_recovers_orphan_without_creating_second_session() {
    let (_tmp, lifecycle, revision_store, mut engine) = plan_repair_parent_engine();
    let mut request = plan_repair_fixture(
        "plan_repair_request_0001",
        "fingerprint_orphan_recovery",
    );
    let amendment_id = format!("plan_amendment_{}", request.fingerprint);
    request.amendment_id = Some(amendment_id.clone());
    request.status = crate::product::models::PlanRepairRequestStatus::InProgress;
    let plan = revision_store
        .get_plan_lineage("project_0001", "issue_0001", "work_item_plan_0001")
        .unwrap();
    revision_store.put_repair_request(&plan, &request).unwrap();
    revision_store
        .acquire_active_amendment(&plan, &amendment_id)
        .unwrap();
    let orphan = lifecycle
        .create_workspace_session_with_id(
            CreateWorkspaceSessionInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                entity_id: plan.id.clone(),
                workspace_type: WorkspaceType::WorkItemPlan,
                author_provider: ProviderName::ClaudeCode,
                reviewer_provider: ProviderName::Codex,
                review_rounds: 2,
                superpowers_enabled: true,
                openspec_enabled: true,
            },
            format!("workspace_session_{amendment_id}"),
        )
        .unwrap();

    let recovered = engine.start_plan_repair(request).await.unwrap();

    assert_eq!(recovered.id, orphan.id);
    assert_eq!(
        lifecycle
            .list_workspace_sessions("project_0001", "issue_0001")
            .unwrap()
            .len(),
        2
    );
    assert_eq!(lifecycle.get_session_link(&orphan.id).unwrap().child_session_id, orphan.id);
    assert!(
        lifecycle
            .load_plan_repair_session_state("project_0001", "issue_0001", &recovered.id)
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn plan_repair_child_session_recovers_lock_before_request_write() {
    let (_tmp, lifecycle, revision_store, mut engine) = plan_repair_parent_engine();
    let request = plan_repair_fixture(
        "plan_repair_request_0001",
        "fingerprint_lock_recovery",
    );
    let amendment_id = "plan_amendment_fingerprint_lock_recovery";
    let plan = revision_store
        .get_plan_lineage("project_0001", "issue_0001", "work_item_plan_0001")
        .unwrap();
    revision_store
        .acquire_active_amendment(&plan, amendment_id)
        .unwrap();

    let recovered = engine.start_plan_repair(request).await.unwrap();

    assert_eq!(recovered.id, format!("workspace_session_{amendment_id}"));
    assert_eq!(
        lifecycle
            .list_workspace_sessions("project_0001", "issue_0001")
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        revision_store
            .list_open_repair_requests(
                &revision_store
                    .get_plan_lineage("project_0001", "issue_0001", "work_item_plan_0001")
                    .unwrap(),
            )
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn plan_repair_workspace_session_link_is_absent_for_ordinary_workspace_types() {
    for workspace_type in [
        WorkspaceType::Story,
        WorkspaceType::Design,
        WorkspaceType::WorkItem,
        WorkspaceType::WorkItemPlan,
    ] {
        let tmp = TempDir::new().unwrap();
        let lifecycle = LifecycleStore::new(ProductAppPaths::new(tmp.path().join(".aria")));
        let session = lifecycle
            .create_workspace_session(CreateWorkspaceSessionInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                entity_id: "entity_0001".to_string(),
                workspace_type,
                author_provider: ProviderName::ClaudeCode,
                reviewer_provider: ProviderName::Codex,
                review_rounds: 2,
                superpowers_enabled: true,
                openspec_enabled: true,
            })
            .unwrap();

        assert!(lifecycle.get_session_link(&session.id).is_err());
        assert!(
            lifecycle
                .load_plan_repair_session_state(
                    "project_0001",
                    "issue_0001",
                    &session.id,
                )
                .unwrap()
                .is_none()
        );
    }
}

#[tokio::test]
async fn plan_repair_refresh_restores_awaiting_confirmation_state() {
    let (tmp, lifecycle, revision_store, mut parent) = plan_repair_parent_engine();
    let child = parent
        .start_plan_repair(plan_repair_fixture("plan_repair_request_0001", "fingerprint_refresh"))
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

    lifecycle.save_artifact_versions(&child.id, &[]).unwrap();
    let (recovery_tx, mut recovery_rx) = mpsc::channel(16);
    let mut restored = WorkspaceEngine::new_persistent(
        Arc::new(CheckpointStore::new(tmp.path().join("repair-checkpoints"))),
        lifecycle.clone(),
        recovery_tx,
        WorkspaceSession::from_record(lifecycle.get_workspace_session(&child.id).unwrap()),
    );
    restored
        .ensure_plan_repair_manifest_artifact()
        .await
        .unwrap();
    assert!(matches!(
        recovery_rx.try_recv().unwrap(),
        EngineEvent::ArtifactUpdate {
            payload: ArtifactPayload::PlanAmendmentManifest { .. },
            ..
        }
    ));
    let restored = plan_repair_restarted_child_engine(
        &tmp,
        &lifecycle,
        lifecycle.get_workspace_session(&child.id).unwrap(),
    );
    let snapshot = restored.plan_repair_session_state().unwrap();

    assert_eq!(restored.current_stage(), WorkspaceStage::HumanConfirm);
    assert_eq!(
        snapshot.stage,
        crate::product::models::PlanRepairSessionStage::AwaitingConfirmation
    );
    assert!(snapshot.amendment.is_some());
    assert_eq!(
        snapshot
            .timeline_nodes
            .iter()
            .filter(|node| node.node_type == TimelineNodeType::PlanAmendmentConfirmation)
            .count(),
        1
    );
    match restored.build_session_state() {
        WsOutMessage::SessionState {
            plan_repair: Some(state),
            ..
        } => assert_eq!(state.request.id, "plan_repair_request_0001"),
        message => panic!("expected linked plan repair session state, got {message:?}"),
    }
}

#[tokio::test]
async fn plan_repair_confirmation_is_recorded_exactly_once() {
    let (tmp, lifecycle, revision_store, mut parent) = plan_repair_parent_engine();
    let child = parent
        .start_plan_repair(plan_repair_fixture("plan_repair_request_0001", "fingerprint_confirm"))
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
        .confirm_plan_amendment(&amendment_id)
        .await
        .unwrap();
    child_engine
        .confirm_plan_amendment(&amendment_id)
        .await
        .unwrap();

    let snapshot = child_engine.plan_repair_session_state().unwrap();
    let confirmation_nodes = snapshot
        .timeline_nodes
        .iter()
        .filter(|node| node.node_type == TimelineNodeType::PlanAmendmentConfirmation)
        .collect::<Vec<_>>();
    assert_eq!(confirmation_nodes.len(), 1);
    assert_eq!(confirmation_nodes[0].status, TimelineNodeStatus::Completed);
    assert_eq!(
        snapshot.request.status,
        crate::product::models::PlanRepairRequestStatus::AwaitingConfirmation
    );
}

#[tokio::test]
async fn plan_repair_cancel_keeps_request_timeline_and_lock_consistent() {
    let (tmp, lifecycle, revision_store, mut parent) = plan_repair_parent_engine();
    let child = parent
        .start_plan_repair(plan_repair_fixture("plan_repair_request_0001", "fingerprint_cancel"))
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

    child_engine
        .cancel_plan_amendment(&amendment_id, Some("用户取消".to_string()))
        .await
        .unwrap();

    let stored_request = revision_store
        .get_repair_request(&plan, &request.id)
        .unwrap();
    let stored_plan = revision_store
        .get_plan_lineage("project_0001", "issue_0001", "work_item_plan_0001")
        .unwrap();
    let restored = plan_repair_restarted_child_engine(
        &tmp,
        &lifecycle,
        lifecycle.get_workspace_session(&child.id).unwrap(),
    );
    let snapshot = restored.plan_repair_session_state().unwrap();
    assert_eq!(
        stored_request.status,
        crate::product::models::PlanRepairRequestStatus::Cancelled
    );
    assert_eq!(stored_plan.active_amendment_id, None);
    assert_eq!(
        snapshot.stage,
        crate::product::models::PlanRepairSessionStage::Completed
    );
    assert!(snapshot.timeline_nodes.iter().any(|node| {
        node.node_type == TimelineNodeType::PlanAmendmentCancelled
            && node.status == TimelineNodeStatus::Completed
    }));
}

#[tokio::test]
async fn plan_repair_cancel_fails_closed_after_plan_published() {
    let (tmp, lifecycle, revision_store, mut parent) = plan_repair_parent_engine();
    let child = parent
        .start_plan_repair(plan_repair_fixture("plan_repair_request_0001", "fingerprint_published"))
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
    revision_store
        .put_plan_amendment_publication_journal(
            &plan,
            &crate::product::models::PlanAmendmentPublicationJournal {
                id: format!("{amendment_id}_publication_journal"),
                project_id: plan.project_id.clone(),
                issue_id: plan.issue_id.clone(),
                plan_id: plan.id.clone(),
                amendment_id: amendment_id.clone(),
                request_id: request.id.clone(),
                base_plan_revision_id: request.base_plan_revision_id.clone(),
                new_plan_revision_id: "plan_revision_0002".to_string(),
                confirmation: None,
                artifact_fingerprint: "fingerprint_plan_published".to_string(),
                snapshot: None,
                phase: crate::product::models::PlanAmendmentPublicationPhase::PlanPublished,
                error: None,
                recovery: None,
                created_at: "2026-07-18T00:00:03Z".to_string(),
                updated_at: "2026-07-18T00:00:03Z".to_string(),
            },
        )
        .unwrap();

    let error = child_engine
        .cancel_plan_amendment(&amendment_id, None)
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
    assert!(!child_engine
        .plan_repair_session_state()
        .unwrap()
        .timeline_nodes
        .iter()
        .any(|node| node.node_type == TimelineNodeType::PlanAmendmentCancelled));
}

#[tokio::test]
async fn plan_repair_awaiting_confirmation_persists_broadcasts_and_recovers_manifest_artifact() {
    let (tmp, lifecycle, revision_store, mut parent) = plan_repair_parent_engine();
    let child = parent
        .start_plan_repair(plan_repair_fixture(
            "plan_repair_request_manifest",
            "fingerprint_manifest_artifact",
        ))
        .await
        .unwrap();
    let plan = revision_store
        .get_plan_lineage("project_0001", "issue_0001", "work_item_plan_0001")
        .unwrap();
    let request = revision_store
        .get_repair_request(&plan, "plan_repair_request_manifest")
        .unwrap();
    let amendment_id = request.amendment_id.clone().unwrap();
    let package = plan_repair_awaiting_package(&request.id, &amendment_id);
    let expected_manifest = package.amendment.clone();
    let (event_tx, mut event_rx) = mpsc::channel(64);
    let mut child_engine = WorkspaceEngine::new_persistent(
        Arc::new(CheckpointStore::new(tmp.path().join("repair-checkpoints"))),
        lifecycle.clone(),
        event_tx,
        WorkspaceSession::from_record(child.clone()),
    );

    plan_repair_enter_awaiting(&mut child_engine, &revision_store, &plan, package)
        .await
        .unwrap();

    let mut broadcast_manifest = None;
    while let Ok(event) = event_rx.try_recv() {
        if let EngineEvent::ArtifactUpdate {
            payload: ArtifactPayload::PlanAmendmentManifest { manifest },
            ..
        } = event
        {
            broadcast_manifest = Some(*manifest);
        }
    }
    assert_eq!(broadcast_manifest.as_ref(), Some(&expected_manifest));
    assert!(lifecycle
        .list_artifact_versions(&child.id)
        .unwrap()
        .iter()
        .any(|version| matches!(
            &version.payload,
            ArtifactPayload::PlanAmendmentManifest { manifest }
                if manifest.as_ref() == &expected_manifest
        )));

    let restored = plan_repair_restarted_child_engine(
        &tmp,
        &lifecycle,
        lifecycle.get_workspace_session(&child.id).unwrap(),
    );
    let WsOutMessage::SessionState {
        artifact_versions, ..
    } = restored.build_session_state()
    else {
        panic!("expected session state");
    };
    assert!(artifact_versions.iter().any(|version| matches!(
        &version.payload,
        ArtifactPayload::PlanAmendmentManifest { manifest }
            if manifest.as_ref() == &expected_manifest
    )));
}
