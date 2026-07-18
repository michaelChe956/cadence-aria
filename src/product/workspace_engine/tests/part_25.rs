#[tokio::test]
async fn plan_repair_reuse_rejects_persisted_request_lineage_mismatch_without_writes() {
    type RequestMutation = fn(&mut crate::product::models::PlanRepairRequest);
    let cases: [(&str, RequestMutation); 6] = [
        ("base", |request| {
            request.base_plan_revision_id = "plan_revision_wrong".to_string();
        }),
        ("attempt", |request| {
            request.trigger_attempt_id = "coding_attempt_wrong".to_string();
        }),
        ("unit_run", |request| {
            request.trigger_unit_run_id = "coding_unit_run_wrong".to_string();
        }),
        ("review", |request| {
            request.trigger_review_id = Some("code_review_wrong".to_string());
        }),
        ("finding", |request| {
            request.trigger_finding_id = "finding_wrong".to_string();
        }),
        ("repair_target", |request| {
            request.repair_target.logical_work_item_ids =
                vec!["logical_work_item_wrong".to_string()];
        }),
    ];
    for (case, mutate) in cases {
        let (_tmp, lifecycle, revision_store, mut parent) = plan_repair_parent_engine();
        let fingerprint = format!("fingerprint_persisted_request_{case}");
        let amendment_id = format!("plan_amendment_{fingerprint}");
        let incoming = plan_repair_fixture("plan_repair_request_incoming", &fingerprint);
        let mut persisted = incoming.clone();
        persisted.id = "plan_repair_request_persisted".to_string();
        persisted.amendment_id = Some(amendment_id.clone());
        persisted.status = crate::product::models::PlanRepairRequestStatus::InProgress;
        mutate(&mut persisted);
        let plan = revision_store
            .get_plan_lineage("project_0001", "issue_0001", "work_item_plan_0001")
            .unwrap();
        revision_store.put_repair_request(&plan, &persisted).unwrap();
        revision_store
            .acquire_active_amendment(&plan, &amendment_id)
            .unwrap();

        let error = parent.start_plan_repair(incoming).await.unwrap_err();

        assert!(matches!(
            error,
            crate::product::plan_repair::PlanRepairError::InvalidRepairTarget(_)
                | crate::product::plan_repair::PlanRepairError::Store(
                    crate::product::json_store::ProductStoreError::IdentityMismatch { .. }
                )
        ));
        assert_eq!(
            revision_store
                .get_repair_request(&plan, &persisted.id)
                .unwrap()
                .evidence
                .len(),
            1
        );
        assert_eq!(
            lifecycle
                .list_workspace_sessions("project_0001", "issue_0001")
                .unwrap()
                .len(),
            1
        );
        assert!(
            lifecycle
                .list_session_links("project_0001", "issue_0001")
                .unwrap()
                .is_empty()
        );
    }
}

#[tokio::test]
async fn plan_repair_reuse_accepts_reordered_equivalent_repair_target_identity() {
    let (_tmp, _lifecycle, _revision_store, mut parent) = plan_repair_parent_engine();
    let mut first = plan_repair_fixture(
        "plan_repair_request_0001",
        "fingerprint_reordered_repair_target",
    );
    first.repair_target.logical_work_item_ids = vec![
        "logical_work_item_0001".to_string(),
        "logical_work_item_0002".to_string(),
    ];
    first.repair_target.work_item_revision_ids = vec![
        "work_item_revision_0001".to_string(),
        "work_item_revision_0002".to_string(),
    ];
    let mut duplicate = first.clone();
    duplicate.id = "plan_repair_request_0002".to_string();
    duplicate.repair_target.logical_work_item_ids.reverse();
    duplicate.repair_target.work_item_revision_ids.reverse();
    duplicate.evidence[0].source_ref = "code_review_0001#finding_reordered".to_string();

    let first_child = parent.start_plan_repair(first).await.unwrap();
    let duplicate_child = parent.start_plan_repair(duplicate).await.unwrap();

    assert_eq!(duplicate_child.id, first_child.id);
}

#[tokio::test]
async fn plan_repair_reuse_rejects_link_lineage_mismatch_before_evidence_merge() {
    type LinkMutation = fn(&mut crate::product::models::WorkspaceSessionLink);
    let cases: [(&str, LinkMutation); 6] = [
        ("relation", |link| {
            link.relation = crate::product::models::WorkspaceSessionRelation::StoryAmendment;
        }),
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
        let (_tmp, lifecycle, revision_store, mut parent) = plan_repair_parent_engine();
        let fingerprint = format!("fingerprint_link_lineage_{case}");
        let amendment_id = format!("plan_amendment_{fingerprint}");
        let child_session_id = format!("workspace_session_{amendment_id}");
        let mut incoming = plan_repair_fixture("plan_repair_request_incoming", &fingerprint);
        let mut persisted = incoming.clone();
        persisted.id = "plan_repair_request_persisted".to_string();
        persisted.amendment_id = Some(amendment_id.clone());
        persisted.status = crate::product::models::PlanRepairRequestStatus::InProgress;
        let plan = revision_store
            .get_plan_lineage("project_0001", "issue_0001", "work_item_plan_0001")
            .unwrap();
        revision_store.put_repair_request(&plan, &persisted).unwrap();
        revision_store
            .acquire_active_amendment(&plan, &amendment_id)
            .unwrap();
        let child = plan_repair_child_record(&lifecycle, &child_session_id);
        let mut link = plan_repair_link(&persisted, &amendment_id, &child.id);
        mutate(&mut link);
        lifecycle
            .put_session_link("project_0001", "issue_0001", &link)
            .unwrap();
        incoming.evidence[0].source_ref = format!("code_review_0002#finding_{case}");

        let error = parent.start_plan_repair(incoming).await.unwrap_err();

        assert!(matches!(
            error,
            crate::product::plan_repair::PlanRepairError::Store(
                crate::product::json_store::ProductStoreError::IdentityMismatch { .. }
            ) | crate::product::plan_repair::PlanRepairError::InvalidRepairTarget(_)
        ));
        assert_eq!(
            revision_store
                .get_repair_request(&plan, &persisted.id)
                .unwrap()
                .evidence
                .len(),
            1
        );
    }
}

#[tokio::test]
async fn plan_repair_reuse_rejects_child_session_lineage_mismatch_before_reconcile() {
    type InputMutation = fn(&mut CreateWorkspaceSessionInput);
    let cases: [(&str, bool, InputMutation); 5] = [
        ("project", false, |input| {
            input.project_id = "project_wrong".to_string();
        }),
        ("issue", false, |input| {
            input.issue_id = "issue_wrong".to_string();
        }),
        ("entity", false, |input| {
            input.entity_id = "work_item_plan_wrong".to_string();
        }),
        ("workspace_type", false, |input| {
            input.workspace_type = WorkspaceType::WorkItem;
        }),
        ("status", true, |_| {}),
    ];
    for (case, invalid_status, mutate) in cases {
        let (_tmp, lifecycle, revision_store, mut parent) = plan_repair_parent_engine();
        let fingerprint = format!("fingerprint_child_lineage_{case}");
        let amendment_id = format!("plan_amendment_{fingerprint}");
        let child_session_id = format!("workspace_session_{amendment_id}");
        let incoming = plan_repair_fixture("plan_repair_request_incoming", &fingerprint);
        let mut persisted = incoming.clone();
        persisted.id = "plan_repair_request_persisted".to_string();
        persisted.amendment_id = Some(amendment_id.clone());
        persisted.status = crate::product::models::PlanRepairRequestStatus::InProgress;
        let plan = revision_store
            .get_plan_lineage("project_0001", "issue_0001", "work_item_plan_0001")
            .unwrap();
        revision_store.put_repair_request(&plan, &persisted).unwrap();
        revision_store
            .acquire_active_amendment(&plan, &amendment_id)
            .unwrap();
        let mut input = CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: plan.id.clone(),
            workspace_type: WorkspaceType::WorkItemPlan,
            author_provider: ProviderName::ClaudeCode,
            reviewer_provider: ProviderName::Codex,
            review_rounds: 2,
            superpowers_enabled: true,
            openspec_enabled: true,
        };
        mutate(&mut input);
        let child = lifecycle
            .create_workspace_session_with_id(input, child_session_id)
            .unwrap();
        if invalid_status {
            lifecycle
                .update_workspace_session_status(
                    &child.id,
                    crate::product::models::WorkspaceSessionStatus::Confirmed,
                )
                .unwrap();
        }
        lifecycle
            .put_session_link(
                "project_0001",
                "issue_0001",
                &plan_repair_link(&persisted, &amendment_id, &child.id),
            )
            .unwrap();
        let error = parent.start_plan_repair(incoming).await.unwrap_err();

        assert!(matches!(
            error,
            crate::product::plan_repair::PlanRepairError::Store(
                crate::product::json_store::ProductStoreError::IdentityMismatch { .. }
            ) | crate::product::plan_repair::PlanRepairError::InvalidRepairTarget(_)
        ));
    }
}

#[tokio::test]
async fn plan_repair_refresh_rejects_incomplete_awaiting_package_as_failed() {
    type SnapshotMutation = fn(&mut crate::product::models::PlanRepairSessionSnapshotDto);
    let cases: [(&str, SnapshotMutation); 6] = [
        ("projection", |snapshot| snapshot.projection = None),
        ("amendment", |snapshot| snapshot.amendment = None),
        ("validation", |snapshot| snapshot.validation = None),
        ("impact", |snapshot| snapshot.impact = None),
        ("plan_review", |snapshot| snapshot.plan_review = None),
        ("package_identity", |snapshot| {
            snapshot.package_identity = None;
        }),
    ];
    for (case, mutate) in cases {
        let (tmp, lifecycle, revision_store, mut parent) = plan_repair_parent_engine();
        let child = parent
            .start_plan_repair(plan_repair_fixture(
                "plan_repair_request_0001",
                &format!("fingerprint_refresh_missing_{case}"),
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
        child_engine
            .enter_plan_repair_awaiting_confirmation(plan_repair_awaiting_package(
                &request.id,
                &amendment_id,
            ))
            .await
            .unwrap();
        let mut snapshot = child_engine.plan_repair_session_state().unwrap().clone();
        mutate(&mut snapshot);
        lifecycle
            .save_plan_repair_session_state("project_0001", "issue_0001", &child.id, &snapshot)
            .unwrap();

        let restored = plan_repair_restarted_child_engine(
            &tmp,
            &lifecycle,
            lifecycle.get_workspace_session(&child.id).unwrap(),
        );

        assert_eq!(restored.current_stage(), WorkspaceStage::Completed);
        assert_eq!(
            restored.plan_repair_session_state().unwrap().stage,
            crate::product::models::PlanRepairSessionStage::Failed
        );
        assert!(
            restored
                .plan_repair_session_state()
                .unwrap()
                .error
                .as_deref()
                .is_some_and(|error| error.contains("awaiting"))
        );
    }
}

#[tokio::test]
async fn plan_repair_confirm_revalidates_awaiting_package_and_current_amendment_lock() {
    for missing_package_identity in [true, false] {
        let (tmp, lifecycle, revision_store, mut parent) = plan_repair_parent_engine();
        let case = if missing_package_identity {
            "missing_identity"
        } else {
            "missing_lock"
        };
        let child = parent
            .start_plan_repair(plan_repair_fixture(
                "plan_repair_request_0001",
                &format!("fingerprint_confirm_revalidate_{case}"),
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
        if missing_package_identity {
            child_engine
                .plan_repair_snapshot
                .as_mut()
                .unwrap()
                .package_identity = None;
        } else {
            revision_store
                .release_active_amendment(&plan, &amendment_id)
                .unwrap();
        }

        let error = child_engine
            .confirm_plan_amendment(&amendment_id)
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            crate::product::plan_repair::PlanRepairError::InvalidRepairTarget(_)
                | crate::product::plan_repair::PlanRepairError::AmendmentConflict { .. }
        ));
        assert_eq!(
            child_engine
                .timeline_nodes
                .iter()
                .find(|node| node.node_type == TimelineNodeType::PlanAmendmentConfirmation)
                .unwrap()
                .status,
            TimelineNodeStatus::Active
        );
        assert_eq!(
            revision_store
                .get_repair_request(&plan, &request.id)
                .unwrap()
                .status,
            crate::product::models::PlanRepairRequestStatus::AwaitingConfirmation
        );
    }
}

#[tokio::test]
async fn plan_repair_refresh_rejects_tampered_awaiting_package_as_failed() {
    let (tmp, lifecycle, revision_store, mut parent) = plan_repair_parent_engine();
    let child = parent
        .start_plan_repair(plan_repair_fixture(
            "plan_repair_request_0001",
            "fingerprint_refresh_tampered_package",
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
    child_engine
        .enter_plan_repair_awaiting_confirmation(plan_repair_awaiting_package(
            &request.id,
            &amendment_id,
        ))
        .await
        .unwrap();
    let mut snapshot = child_engine.plan_repair_session_state().unwrap().clone();
    snapshot
        .package_identity
        .as_mut()
        .unwrap()
        .validation_report_id = "plan_validation_report_wrong".to_string();
    lifecycle
        .save_plan_repair_session_state("project_0001", "issue_0001", &child.id, &snapshot)
        .unwrap();

    let restored = plan_repair_restarted_child_engine(
        &tmp,
        &lifecycle,
        lifecycle.get_workspace_session(&child.id).unwrap(),
    );

    assert_eq!(restored.current_stage(), WorkspaceStage::Completed);
    assert_eq!(
        restored.plan_repair_session_state().unwrap().stage,
        crate::product::models::PlanRepairSessionStage::Failed
    );
}

#[tokio::test]
async fn plan_repair_refresh_rejects_stale_base_or_active_amendment_as_failed() {
    for stale_base in [true, false] {
        let (tmp, lifecycle, revision_store, mut parent) = plan_repair_parent_engine();
        let case = if stale_base { "base" } else { "lock" };
        let child = parent
            .start_plan_repair(plan_repair_fixture(
                "plan_repair_request_0001",
                &format!("fingerprint_refresh_stale_{case}"),
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
        child_engine
            .enter_plan_repair_awaiting_confirmation(plan_repair_awaiting_package(
                &request.id,
                &amendment_id,
            ))
            .await
            .unwrap();
        if stale_base {
            revision_store
                .put_plan_revision(
                    &plan,
                    &crate::product::models::WorkItemPlanRevision {
                        id: "plan_revision_external".to_string(),
                        plan_id: plan.id.clone(),
                        revision_no: 2,
                        supersedes: Some(request.base_plan_revision_id.clone()),
                        reason: crate::product::models::PlanRevisionReason::SubgraphReplan,
                        work_item_bindings: std::collections::BTreeMap::new(),
                        dependency_graph_revision_id: "dependency_graph_external".to_string(),
                        validation_report_ref: "validation_external".to_string(),
                        plan_projection_bundle_id: "projection_external".to_string(),
                        created_at: "2026-07-18T00:00:03Z".to_string(),
                    },
                )
                .unwrap();
            revision_store
                .compare_and_set_active_plan_revision(
                    &plan,
                    &request.base_plan_revision_id,
                    "plan_revision_external",
                )
                .unwrap();
        } else {
            revision_store
                .release_active_amendment(&plan, &amendment_id)
                .unwrap();
        }

        let restored = plan_repair_restarted_child_engine(
            &tmp,
            &lifecycle,
            lifecycle.get_workspace_session(&child.id).unwrap(),
        );

        assert_eq!(restored.current_stage(), WorkspaceStage::Completed);
        assert_eq!(
            restored.plan_repair_session_state().unwrap().stage,
            crate::product::models::PlanRepairSessionStage::Failed
        );
    }
}

#[tokio::test]
async fn plan_repair_cancel_replay_predicate_rejects_conflict_and_other_completed_states() {
    let (tmp, lifecycle, revision_store, mut parent) = plan_repair_parent_engine();
    let child = parent
        .start_plan_repair(plan_repair_fixture(
            "plan_repair_request_0001",
            "fingerprint_cancel_replay_stage_gate",
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

    {
        let snapshot = child_engine.plan_repair_snapshot.as_mut().unwrap();
        snapshot.stage = crate::product::models::PlanRepairSessionStage::AmendmentConflict;
        snapshot.request.status =
            crate::product::models::PlanRepairRequestStatus::AwaitingConfirmation;
    }
    assert!(!child_engine.is_cancelled_plan_amendment_replay(&amendment_id));

    {
        let snapshot = child_engine.plan_repair_snapshot.as_mut().unwrap();
        snapshot.stage = crate::product::models::PlanRepairSessionStage::Completed;
        snapshot.request.status = crate::product::models::PlanRepairRequestStatus::Applied;
    }
    assert!(!child_engine.is_cancelled_plan_amendment_replay(&amendment_id));
}
