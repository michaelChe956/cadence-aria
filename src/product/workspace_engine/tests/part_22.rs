fn plan_repair_awaiting_package(
    request_id: &str,
    amendment_id: &str,
) -> crate::product::models::PlanRepairAwaitingConfirmationPackage {
    crate::product::models::PlanRepairAwaitingConfirmationPackage {
        projection: crate::product::models::PlanProjectionBundle {
            id: "plan_projection_bundle_0002".to_string(),
            plan_revision_id: "plan_revision_0002".to_string(),
            dependency_graph_revision_id: "dependency_graph_revision_0002".to_string(),
            work_item_projection_bundle_refs: Vec::new(),
            human_group_projection:
                crate::product::work_item_projection::HumanGroupProjection {
                    plan_id: "work_item_plan_0001".to_string(),
                    goal: "Repair plan".to_string(),
                    split_reason: "Repair contract".to_string(),
                    work_items: Vec::new(),
                    contract_flow: Vec::new(),
                    risks: Vec::new(),
                    source_refs: Vec::new(),
                    normative: false,
                    used_by_provider: false,
                },
            coder_group_context: crate::product::work_item_projection::CoderGroupContext {
                plan_id: "work_item_plan_0001".to_string(),
                ordered_logical_work_item_ids: Vec::new(),
                dependency_edges: Vec::new(),
                group_write_scopes: std::collections::BTreeMap::new(),
            },
            reviewer_group_matrix: crate::product::work_item_projection::ReviewerGroupMatrix {
                plan_id: "work_item_plan_0001".to_string(),
                work_items: Vec::new(),
                dependency_edges: Vec::new(),
                design_traceability_refs: Vec::new(),
            },
            human_group_projection_hash: "human_hash".to_string(),
            coder_group_context_hash: "coder_hash".to_string(),
            reviewer_group_matrix_hash: "reviewer_hash".to_string(),
            compiler_version: "projection-v1".to_string(),
            created_at: "2026-07-18T00:00:02Z".to_string(),
        },
        amendment: plan_repair_manifest(request_id, amendment_id),
        validation: crate::product::models::PlanValidationReportArtifact {
            id: "plan_validation_report_0002".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            contract_validation:
                crate::product::work_item_contract::ContractValidationReport {
                    findings: Vec::new(),
                },
            projection_validation:
                crate::product::work_item_projection::ProjectionValidationReport {
                    findings: Vec::new(),
                },
            created_at: "2026-07-18T00:00:02Z".to_string(),
        },
        impact: crate::product::plan_repair::ContractImpactReport {
            unaffected: Vec::new(),
            direct_revalidation: vec!["logical_work_item_0001".to_string()],
            direct_stale: Vec::new(),
            conditional_downstream: Vec::new(),
            explanation_paths: Vec::new(),
        },
        plan_review: crate::web::workspace_ws_types::WorkItemPlanReviewComplete {
            verdict: crate::web::workspace_ws_types::WorkItemPlanReviewVerdict::Pass,
            review_scope: crate::web::workspace_ws_types::WorkItemPlanReviewScope::Item,
            target_outline_id: Some("logical_work_item_0001".to_string()),
            generation_round_id: "repair_round_0001".to_string(),
            draft_id: Some("draft_revision_0002".to_string()),
            batch_id: None,
            review_action: crate::web::workspace_ws_types::WorkItemPlanReviewAction::Continue,
            gates: Vec::new(),
            affects_items: Vec::new(),
            warnings: Vec::new(),
        },
    }
}

async fn plan_repair_awaiting_rejection<F>(
    fingerprint: &str,
    mutate: F,
) -> crate::product::plan_repair::PlanRepairError
where
    F: FnOnce(&mut crate::product::models::PlanRepairAwaitingConfirmationPackage),
{
    let (tmp, lifecycle, revision_store, mut parent) = plan_repair_parent_engine();
    let child = parent
        .start_plan_repair(plan_repair_fixture(
            "plan_repair_request_0001",
            fingerprint,
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
    let original_timeline = child_engine
        .plan_repair_session_state()
        .unwrap()
        .timeline_nodes
        .clone();
    let mut package = plan_repair_awaiting_package(&request.id, &amendment_id);
    mutate(&mut package);

    let error = child_engine
        .enter_plan_repair_awaiting_confirmation(package)
        .await
        .unwrap_err();

    assert_eq!(
        child_engine
            .plan_repair_session_state()
            .unwrap()
            .timeline_nodes,
        original_timeline
    );
    assert_eq!(child_engine.current_stage(), WorkspaceStage::Running);
    assert_eq!(
        revision_store
            .get_repair_request(&plan, &request.id)
            .unwrap()
            .status,
        crate::product::models::PlanRepairRequestStatus::InProgress
    );
    error
}

#[tokio::test]
async fn plan_repair_awaiting_rejects_invalid_validation_before_timeline_persistence() {
    let error = plan_repair_awaiting_rejection("fingerprint_invalid_validation", |package| {
        package.validation.contract_validation.findings.push(
            crate::product::work_item_contract::ContractValidationFinding {
                code: "invalid_contract".to_string(),
                severity: crate::product::work_item_contract::ContractFindingSeverity::Error,
                logical_work_item_id: Some("logical_work_item_0001".to_string()),
                contract_ref: None,
                capability_ref: None,
                message: "contract invalid".to_string(),
            },
        );
    })
    .await;

    assert!(matches!(
        error,
        crate::product::plan_repair::PlanRepairError::ContractValidation(_)
    ));
}

#[tokio::test]
async fn plan_repair_awaiting_rejects_invalid_projection_before_timeline_persistence() {
    let error = plan_repair_awaiting_rejection("fingerprint_invalid_projection", |package| {
        package.validation.projection_validation.findings.push(
            crate::product::work_item_projection::ProjectionValidationFinding {
                code: "missing_projection".to_string(),
                projection: "coder".to_string(),
                contract_ref: None,
                message: "projection missing".to_string(),
            },
        );
    })
    .await;

    assert!(matches!(
        error,
        crate::product::plan_repair::PlanRepairError::ProjectionValidation(_)
    ));
}

#[tokio::test]
async fn plan_repair_awaiting_requires_passing_continue_review_without_gates() {
    for (fingerprint, mutate) in [
        (
            "fingerprint_review_verdict",
            (|package: &mut crate::product::models::PlanRepairAwaitingConfirmationPackage| {
                package.plan_review.verdict =
                    crate::web::workspace_ws_types::WorkItemPlanReviewVerdict::Revise;
            }) as fn(&mut crate::product::models::PlanRepairAwaitingConfirmationPackage),
        ),
        (
            "fingerprint_review_action",
            |package| {
                package.plan_review.review_action =
                    crate::web::workspace_ws_types::WorkItemPlanReviewAction::HumanTriage;
            },
        ),
        (
            "fingerprint_review_gate",
            |package| {
                package.plan_review.gates.push(
                    crate::web::workspace_ws_types::WorkItemPlanReviewGate::RequiresPlanReopen,
                );
            },
        ),
    ] {
        let error = plan_repair_awaiting_rejection(fingerprint, mutate).await;
        assert!(matches!(
            error,
            crate::product::plan_repair::PlanRepairError::InvalidRepairTarget(_)
        ));
    }
}

#[tokio::test]
async fn plan_repair_awaiting_requires_manifest_impact_partition_match() {
    let error = plan_repair_awaiting_rejection("fingerprint_impact_mismatch", |package| {
        package.impact.direct_revalidation.clear();
        package
            .impact
            .direct_stale
            .push("logical_work_item_0001".to_string());
    })
    .await;

    assert!(matches!(
        error,
        crate::product::plan_repair::PlanRepairError::InvalidRepairTarget(_)
    ));
}

#[tokio::test]
async fn plan_repair_awaiting_rejects_inconsistent_package_identity() {
    type PackageMutation =
        fn(&mut crate::product::models::PlanRepairAwaitingConfirmationPackage);
    let cases: [(&str, PackageMutation); 5] = [
        ("fingerprint_previous_revision", |package| {
            package.amendment.previous_plan_revision_id = "plan_revision_wrong".to_string();
        }),
        ("fingerprint_same_new_revision", |package| {
            package.amendment.new_plan_revision_id = "plan_revision_0001".to_string();
            package.projection.plan_revision_id = "plan_revision_0001".to_string();
        }),
        ("fingerprint_projection_revision", |package| {
            package.projection.plan_revision_id = "plan_revision_wrong".to_string();
        }),
        ("fingerprint_validation_plan", |package| {
            package.validation.plan_id = "work_item_plan_wrong".to_string();
        }),
        ("fingerprint_request_identity", |package| {
            package.amendment.repair_request_id = "plan_repair_request_wrong".to_string();
        }),
    ];
    for (fingerprint, mutate) in cases {
        let error = plan_repair_awaiting_rejection(fingerprint, mutate).await;
        assert!(matches!(
            error,
            crate::product::plan_repair::PlanRepairError::InvalidRepairTarget(_)
                | crate::product::plan_repair::PlanRepairError::AmendmentConflict { .. }
        ));
    }
}

#[tokio::test]
async fn plan_repair_awaiting_rejects_stale_base_revision_before_timeline_persistence() {
    let (tmp, lifecycle, revision_store, mut parent) = plan_repair_parent_engine();
    let child = parent
        .start_plan_repair(plan_repair_fixture(
            "plan_repair_request_0001",
            "fingerprint_stale_base",
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
    revision_store
        .put_plan_revision(
            &plan,
            &crate::product::models::WorkItemPlanRevision {
                id: "plan_revision_external".to_string(),
                plan_id: plan.id.clone(),
                revision_no: 2,
                supersedes: Some("plan_revision_0001".to_string()),
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
        .set_active_plan_revision(&plan, "plan_revision_external")
        .unwrap();
    let mut child_engine = plan_repair_restarted_child_engine(&tmp, &lifecycle, child);
    let original_timeline = child_engine
        .plan_repair_session_state()
        .unwrap()
        .timeline_nodes
        .clone();

    let error = child_engine
        .enter_plan_repair_awaiting_confirmation(plan_repair_awaiting_package(
            &request.id,
            &amendment_id,
        ))
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        crate::product::plan_repair::PlanRepairError::AmendmentConflict { .. }
    ));
    assert_eq!(
        child_engine
            .plan_repair_session_state()
            .unwrap()
            .timeline_nodes,
        original_timeline
    );
}

fn plan_repair_child_record(
    lifecycle: &LifecycleStore,
    session_id: &str,
) -> crate::product::models::WorkspaceSessionRecord {
    lifecycle
        .create_workspace_session_with_id(
            CreateWorkspaceSessionInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                entity_id: "work_item_plan_0001".to_string(),
                workspace_type: WorkspaceType::WorkItemPlan,
                author_provider: ProviderName::ClaudeCode,
                reviewer_provider: ProviderName::Codex,
                review_rounds: 2,
                superpowers_enabled: true,
                openspec_enabled: true,
            },
            session_id.to_string(),
        )
        .unwrap()
}

fn plan_repair_link(
    request: &crate::product::models::PlanRepairRequest,
    amendment_id: &str,
    child_session_id: &str,
) -> crate::product::models::WorkspaceSessionLink {
    crate::product::models::WorkspaceSessionLink {
        id: format!("workspace_session_link_{amendment_id}"),
        relation: crate::product::models::WorkspaceSessionRelation::PlanRepair,
        parent_session_id: request.trigger_attempt_id.clone(),
        child_session_id: child_session_id.to_string(),
        trigger: crate::product::models::WorkspaceSessionLinkTrigger {
            attempt_id: request.trigger_attempt_id.clone(),
            unit_run_id: request.trigger_unit_run_id.clone(),
            review_id: request.trigger_review_id.clone(),
            finding_id: request.trigger_finding_id.clone(),
            repair_request_id: request.id.clone(),
            amendment_id: amendment_id.to_string(),
            fingerprint: request.fingerprint.clone(),
            base_plan_revision_id: request.base_plan_revision_id.clone(),
        },
        return_context: crate::product::models::WorkspaceReturnContext {
            original_attempt_id: request.trigger_attempt_id.clone(),
            original_unit_run_id: request.trigger_unit_run_id.clone(),
            timeline_anchor_id: request.trigger_finding_id.clone(),
            original_route:
                "/workbench/projects/project_0001/issues/issue_0001/coding/coding_attempt_0001"
                    .to_string(),
        },
        created_at: "2026-07-18T00:00:02Z".to_string(),
    }
}

#[tokio::test]
async fn plan_repair_full_digest_ids_do_not_collide_on_shared_prefix() {
    let prefix = "0123456789abcdef01234567";
    let fingerprint_a = format!("{prefix}{}", "a".repeat(40));
    let fingerprint_b = format!("{prefix}{}", "b".repeat(40));
    let (_tmp_a, _lifecycle_a, _store_a, mut parent_a) = plan_repair_parent_engine();
    let (_tmp_b, _lifecycle_b, _store_b, mut parent_b) = plan_repair_parent_engine();

    let child_a = parent_a
        .start_plan_repair(plan_repair_fixture("plan_repair_request_a", &fingerprint_a))
        .await
        .unwrap();
    let child_b = parent_b
        .start_plan_repair(plan_repair_fixture("plan_repair_request_b", &fingerprint_b))
        .await
        .unwrap();

    assert_ne!(child_a.id, child_b.id);
}

#[tokio::test]
async fn plan_repair_link_identity_includes_trigger_review() {
    let (_tmp, lifecycle, revision_store, mut parent) = plan_repair_parent_engine();
    let fingerprint = "fingerprint_review_identity";
    let amendment_id = format!("plan_amendment_{fingerprint}");
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
    let foreign_child = plan_repair_child_record(&lifecycle, "workspace_session_foreign");
    let mut foreign_link = plan_repair_link(&request, "foreign", &foreign_child.id);
    foreign_link.trigger.review_id = Some("code_review_wrong".to_string());
    lifecycle
        .put_session_link("project_0001", "issue_0001", &foreign_link)
        .unwrap();

    let recovered = parent.start_plan_repair(request).await.unwrap();

    assert_ne!(recovered.id, foreign_child.id);
    assert_eq!(
        recovered.id,
        format!("workspace_session_{amendment_id}")
    );
}

#[tokio::test]
async fn plan_repair_link_identity_includes_request_amendment_fingerprint_and_base() {
    let (_tmp, lifecycle, revision_store, mut parent) = plan_repair_parent_engine();
    let fingerprint = "fingerprint_full_link_identity";
    let amendment_id = format!("plan_amendment_{fingerprint}");
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
    let foreign_child = plan_repair_child_record(&lifecycle, "workspace_session_foreign_identity");
    let mut foreign_request = request.clone();
    foreign_request.id = "plan_repair_request_foreign".to_string();
    foreign_request.amendment_id = Some("plan_amendment_foreign".to_string());
    foreign_request.fingerprint = "fingerprint_foreign".to_string();
    foreign_request.base_plan_revision_id = "plan_revision_foreign".to_string();
    let foreign_link = plan_repair_link(
        &foreign_request,
        "plan_amendment_foreign",
        &foreign_child.id,
    );
    lifecycle
        .put_session_link("project_0001", "issue_0001", &foreign_link)
        .unwrap();

    let recovered = parent.start_plan_repair(request).await.unwrap();

    assert_ne!(recovered.id, foreign_child.id);
    assert_eq!(
        recovered.id,
        format!("workspace_session_{amendment_id}")
    );
}

#[tokio::test]
async fn plan_repair_existing_child_reconciles_session_link_timeline_snapshot_and_evidence() {
    let (_tmp, lifecycle, revision_store, mut parent) = plan_repair_parent_engine();
    let fingerprint = "fingerprint_reconcile";
    let amendment_id = format!("plan_amendment_{fingerprint}");
    let child_session_id = format!("workspace_session_{amendment_id}");
    let mut stored_request = plan_repair_fixture("plan_repair_request_0001", fingerprint);
    stored_request.amendment_id = Some(amendment_id.clone());
    stored_request.status = crate::product::models::PlanRepairRequestStatus::InProgress;
    let plan = revision_store
        .get_plan_lineage("project_0001", "issue_0001", "work_item_plan_0001")
        .unwrap();
    revision_store
        .put_repair_request(&plan, &stored_request)
        .unwrap();
    revision_store
        .acquire_active_amendment(&plan, &amendment_id)
        .unwrap();
    let child = plan_repair_child_record(&lifecycle, &child_session_id);
    lifecycle
        .put_session_link(
            "project_0001",
            "issue_0001",
            &plan_repair_link(&stored_request, &amendment_id, &child.id),
        )
        .unwrap();
    let mut duplicate = plan_repair_fixture("plan_repair_request_0002", fingerprint);
    duplicate.evidence[0].source_ref = "code_review_0002#finding_new".to_string();

    let recovered = parent.start_plan_repair(duplicate).await.unwrap();

    assert_eq!(recovered.id, child.id);
    assert_eq!(
        recovered.status,
        crate::product::models::WorkspaceSessionStatus::Running
    );
    let snapshot = lifecycle
        .load_plan_repair_session_state("project_0001", "issue_0001", &child.id)
        .unwrap()
        .expect("existing child snapshot must be reconciled");
    assert_eq!(snapshot.request.evidence.len(), 2);
    assert_eq!(snapshot.link.child_session_id, child.id);
    assert_eq!(snapshot.timeline_nodes.len(), 1);
    assert_eq!(
        lifecycle
            .load_timeline_nodes_for_issue_session("project_0001", "issue_0001", &child.id)
            .unwrap(),
        snapshot.timeline_nodes
    );
}
