#[tokio::test]
async fn plan_repair_reviewer_orchestration_binds_initial_and_shrink_scope_attestations() {
    let (_tmp, lifecycle, plan_id, mut engine) =
        make_work_item_plan_engine_with_accepted_contract_drafts();
    let outcome = engine.run_work_item_plan_compile().await.unwrap();
    let store = WorkItemRevisionStore::new(lifecycle.app_paths());
    let plan = store
        .get_plan_lineage("project_0001", "issue_0001", &plan_id)
        .unwrap();
    let base_revision_id = plan.active_revision_id.clone().unwrap();
    let logical_id = outcome
        .plan_revision
        .work_item_bindings
        .keys()
        .next()
        .unwrap()
        .clone();
    let previous_work_item_revision_id = outcome.plan_revision.work_item_bindings[&logical_id].clone();
    let minimum_scope_unit = outcome
        .plan_revision
        .work_item_bindings
        .keys()
        .find(|candidate| **candidate != logical_id)
        .unwrap()
        .clone();
    let amendment_id = "plan_amendment_review_candidate_0001";
    let request_id = "plan_repair_request_review_candidate_0001";
    let plan = store
        .acquire_active_amendment(&plan, amendment_id)
        .unwrap();
    let request = crate::product::models::PlanRepairRequest {
        id: request_id.to_string(),
        plan_id: plan.id.clone(),
        base_plan_revision_id: base_revision_id.clone(),
        trigger_attempt_id: "coding_attempt_0001".to_string(),
        trigger_unit_run_id: "coding_unit_run_0001".to_string(),
        trigger_review_id: Some("code_review_0001".to_string()),
        trigger_finding_id: "finding_review_candidate_0001".to_string(),
        amendment_id: Some(amendment_id.to_string()),
        defect_class: crate::product::models::PlanDefectClass::UpstreamContractInvalid,
        reason_code: "upstream_contract_invalid".to_string(),
        repair_target: crate::product::models::RepairTarget {
            kind: crate::product::models::RepairTargetKind::UpstreamWorkItem,
            logical_work_item_ids: vec![logical_id.clone()],
            work_item_revision_ids: vec![previous_work_item_revision_id.clone()],
        },
        contract_refs: vec!["contract.canonical".to_string()],
        capability_refs: vec!["stable_hash".to_string()],
        evidence: vec![],
        fingerprint: "fingerprint_review_candidate_0001".to_string(),
        status: crate::product::models::PlanRepairRequestStatus::InProgress,
        created_at: "2026-07-18T00:00:02Z".to_string(),
        updated_at: "2026-07-18T00:00:02Z".to_string(),
    };
    store.put_repair_request(&plan, &request).unwrap();

    let mut candidate_projection = outcome.plan_projection_bundle.clone();
    candidate_projection.id = "plan_projection_bundle_review_candidate_0002".to_string();
    candidate_projection.plan_revision_id = "plan_revision_review_candidate_0002".to_string();
    candidate_projection.created_at = "2026-07-18T00:00:03Z".to_string();
    store
        .put_plan_projection_bundle(&plan, &candidate_projection)
        .unwrap();
    let mut candidate_validation = outcome.validation_report.clone();
    candidate_validation.id = "plan_validation_report_review_candidate_0002".to_string();
    candidate_validation.plan_revision_id = candidate_projection.plan_revision_id.clone();
    candidate_validation.plan_projection_bundle_id = candidate_projection.id.clone();
    candidate_validation.created_at = "2026-07-18T00:00:03Z".to_string();
    store
        .put_plan_validation_report(&plan, &candidate_validation)
        .unwrap();
    let mut candidate_revision = outcome.plan_revision.clone();
    candidate_revision.id = candidate_projection.plan_revision_id.clone();
    candidate_revision.revision_no += 1;
    candidate_revision.supersedes = Some(base_revision_id.clone());
    candidate_revision.reason = crate::product::models::PlanRevisionReason::RepairUpstreamContract;
    candidate_revision.validation_report_ref = candidate_validation.id.clone();
    candidate_revision.plan_projection_bundle_id = candidate_projection.id.clone();
    candidate_revision.created_at = "2026-07-18T00:00:03Z".to_string();
    store.put_plan_revision(&plan, &candidate_revision).unwrap();
    let manifest = crate::product::models::PlanAmendmentManifest {
        id: amendment_id.to_string(),
        repair_request_id: request.id.clone(),
        previous_plan_revision_id: base_revision_id.clone(),
        new_plan_revision_id: candidate_revision.id.clone(),
        revised_work_items: std::collections::BTreeMap::from([(
            logical_id.clone(),
            crate::product::models::WorkItemRevisionReplacement {
                previous_revision_id: previous_work_item_revision_id.clone(),
                next_revision_id: previous_work_item_revision_id,
                delta_kind: crate::product::models::ContractDeltaKind::InformativeOnly,
            },
        )]),
        superseded_revisions: vec![],
        dependency_graph_changes: vec![],
        contract_deltas: vec![],
        unaffected_units: candidate_revision
            .work_item_bindings
            .keys()
            .filter(|candidate| **candidate != logical_id && **candidate != minimum_scope_unit)
            .cloned()
            .collect(),
        revalidation_required_units: vec![minimum_scope_unit.clone()],
        stale_units: vec![],
        replacement_units: std::collections::BTreeMap::new(),
        resume_target: crate::product::models::AmendmentResumeTarget {
            logical_work_item_id: logical_id.clone(),
            mode: crate::product::models::AmendmentResumeMode::AwaitHandoff,
        },
        created_at: "2026-07-18T00:00:03Z".to_string(),
    };
    let impact = crate::product::plan_repair::ContractImpactReport {
        unaffected: candidate_revision
            .work_item_bindings
            .keys()
            .filter(|candidate| **candidate != logical_id && **candidate != minimum_scope_unit)
            .cloned()
            .collect(),
        direct_revalidation: vec![minimum_scope_unit.clone()],
        direct_stale: vec![],
        conditional_downstream: vec![],
        explanation_paths: vec![],
    };
    let candidate_package = plan_repair_persist_candidate_package(
        &store,
        &plan,
        &request,
        &manifest,
        &candidate_projection,
        &candidate_validation,
        &impact,
    );
    let link = crate::product::models::WorkspaceSessionLink {
        id: "workspace_session_link_review_candidate_0001".to_string(),
        relation: crate::product::models::WorkspaceSessionRelation::PlanRepair,
        parent_session_id: request.trigger_attempt_id.clone(),
        child_session_id: engine.session.session_id.clone(),
        trigger: crate::product::models::WorkspaceSessionLinkTrigger {
            attempt_id: request.trigger_attempt_id.clone(),
            unit_run_id: request.trigger_unit_run_id.clone(),
            review_id: request.trigger_review_id.clone(),
            finding_id: request.trigger_finding_id.clone(),
            repair_request_id: request.id.clone(),
            amendment_id: amendment_id.to_string(),
            fingerprint: request.fingerprint.clone(),
            base_plan_revision_id: base_revision_id.clone(),
        },
        return_context: crate::product::models::WorkspaceReturnContext {
            original_attempt_id: request.trigger_attempt_id.clone(),
            original_unit_run_id: request.trigger_unit_run_id.clone(),
            timeline_anchor_id: request.trigger_finding_id.clone(),
            original_route: "/coding/coding_attempt_0001".to_string(),
        },
        created_at: "2026-07-18T00:00:02Z".to_string(),
    };
    engine.plan_repair_snapshot = Some(crate::product::models::PlanRepairSessionSnapshotDto {
        request,
        link,
        stage: crate::product::models::PlanRepairSessionStage::PlanReview,
        projection: Some(candidate_projection.clone()),
        amendment: Some(manifest),
        validation: Some(candidate_validation),
        impact: Some(impact),
        plan_review: None,
        package_identity: None,
        candidate_package_artifact_id: Some(candidate_package.id),
        impact_scope_review: None,
        timeline_nodes: vec![],
        error: None,
    });
    engine.session.artifact = Some(ArtifactPayload::WorkItemPlanProjection {
        projection: Box::new(candidate_projection),
    });

    let input = engine.build_work_item_plan_review_input().unwrap();

    assert!(input.prompt.contains(request_id));
    assert_eq!(
        store
            .get_plan_lineage("project_0001", "issue_0001", &plan_id)
            .unwrap()
            .active_revision_id
            .as_deref(),
        Some(base_revision_id.as_str())
    );

    engine.begin_work_item_plan_outline_review_run().await;
    engine
        .drive_review_session(
            Arc::new(ReviewVerdictStreamingProvider {
                output: r#"候选修订通过 Plan Review。

```json
{
  "verdict": "pass",
  "review_scope": "outline",
  "generation_round_id": "round_0001",
  "summary": "Plan Repair candidate review passed",
  "findings": []
}
```"#,
                provider_type: Arc::new(Mutex::new(None)),
                prompt: Arc::new(Mutex::new(None)),
            }),
            empty_provider_commands(),
        )
        .await;

    let snapshot = engine.plan_repair_session_state().unwrap();
    assert_eq!(
        snapshot.stage,
        crate::product::models::PlanRepairSessionStage::AwaitingConfirmation
    );
    let identity = snapshot.package_identity.as_ref().unwrap();
    let attestation = store
        .get_plan_repair_review_attestation(&plan, &identity.review_attestation_id)
        .unwrap();
    assert_eq!(attestation.request_id, request_id);
    assert_eq!(attestation.accepted_impact_scope, vec![minimum_scope_unit]);
    assert_eq!(attestation.risk_acceptance_reason, None);

    engine
        .request_plan_repair_impact_scope_review(
            vec![],
            "accept delayed downstream validation risk".to_string(),
        )
        .await
        .unwrap();
    let proposal = engine
        .plan_repair_session_state()
        .unwrap()
        .impact_scope_review
        .as_ref()
        .unwrap();
    assert_eq!(
        proposal.risk_acceptance_reason,
        "accept delayed downstream validation risk"
    );
    assert_eq!(
        proposal.review_generation_round_id,
        "plan_repair_impact_scope_review_0001"
    );
    let re_review_input = engine.build_work_item_plan_review_input().unwrap();
    assert!(re_review_input.prompt.contains("System Minimum Impact Scope"));
    assert!(re_review_input.prompt.contains("Proposed Accepted Impact Scope"));
    assert!(re_review_input.prompt.contains("Risk Acceptance Reason"));
    assert!(
        re_review_input
            .prompt
            .contains(&proposal.candidate_package_fingerprint)
    );

    engine
        .drive_review_session(
            Arc::new(ReviewVerdictStreamingProvider {
                output: r#"缩小影响范围通过 Plan Review。

```json
{
  "verdict": "pass",
  "review_scope": "outline",
  "generation_round_id": "plan_repair_impact_scope_review_0001",
  "summary": "Plan Repair shrink scope review passed",
  "findings": []
}
```"#,
                provider_type: Arc::new(Mutex::new(None)),
                prompt: Arc::new(Mutex::new(None)),
            }),
            empty_provider_commands(),
        )
        .await;

    let snapshot = engine.plan_repair_session_state().unwrap();
    assert_eq!(
        snapshot.stage,
        crate::product::models::PlanRepairSessionStage::AwaitingConfirmation
    );
    let identity = snapshot.package_identity.as_ref().unwrap();
    let attestation = store
        .get_plan_repair_review_attestation(&plan, &identity.review_attestation_id)
        .unwrap();
    assert!(attestation.accepted_impact_scope.is_empty());
    assert_eq!(
        attestation.risk_acceptance_reason.as_deref(),
        Some("accept delayed downstream validation risk")
    );
    assert_eq!(
        attestation.candidate_package_fingerprint,
        identity.candidate_package_fingerprint
    );
}

#[tokio::test]
async fn plan_repair_real_prepare_review_publish_keeps_full_canonical_candidate_package() {
    let (_tmp, lifecycle, plan_id, mut engine) =
        make_work_item_plan_engine_with_accepted_contract_drafts();
    let outcome = engine.run_work_item_plan_compile().await.unwrap();
    let store = WorkItemRevisionStore::new(lifecycle.app_paths());
    let mut plan = store
        .get_plan_lineage("project_0001", "issue_0001", &plan_id)
        .unwrap();
    let base_revision_id = plan.active_revision_id.clone().unwrap();
    let logical_id = outcome
        .plan_revision
        .work_item_bindings
        .keys()
        .next()
        .unwrap()
        .clone();
    let previous_revision_id = outcome.plan_revision.work_item_bindings[&logical_id].clone();
    let previous_revision = store
        .get_work_item_revision(&plan, &logical_id, &previous_revision_id)
        .unwrap();
    let previous_draft = store
        .get_draft_revision(&plan, &previous_revision.source_draft_revision_id)
        .unwrap();
    let amendment_id = "plan_amendment_real_review_publish_0001";
    plan = store
        .acquire_active_amendment(&plan, amendment_id)
        .unwrap();
    let request = crate::product::models::PlanRepairRequest {
        id: "plan_repair_request_real_review_publish_0001".to_string(),
        plan_id: plan.id.clone(),
        base_plan_revision_id: base_revision_id.clone(),
        trigger_attempt_id: "coding_attempt_real_review_publish_0001".to_string(),
        trigger_unit_run_id: "coding_unit_run_real_review_publish_0001".to_string(),
        trigger_review_id: Some("code_review_real_review_publish_0001".to_string()),
        trigger_finding_id: "finding_real_review_publish_0001".to_string(),
        amendment_id: Some(amendment_id.to_string()),
        defect_class: crate::product::models::PlanDefectClass::UpstreamContractInvalid,
        reason_code: "upstream_contract_invalid".to_string(),
        repair_target: crate::product::models::RepairTarget {
            kind: crate::product::models::RepairTargetKind::UpstreamWorkItem,
            logical_work_item_ids: vec![logical_id.clone()],
            work_item_revision_ids: vec![previous_revision_id],
        },
        contract_refs: vec!["contract.real-review-publish".to_string()],
        capability_refs: vec!["canonical_candidate_package".to_string()],
        evidence: vec![crate::product::models::PlanDefectEvidence {
            kind: "review_finding".to_string(),
            source_ref: "review://real-review-publish".to_string(),
            message: "bind the reviewed package to every projection".to_string(),
        }],
        fingerprint: "fingerprint_real_review_publish_0001".to_string(),
        status: crate::product::models::PlanRepairRequestStatus::InProgress,
        created_at: "2026-07-18T00:00:02Z".to_string(),
        updated_at: "2026-07-18T00:00:02Z".to_string(),
    };
    store.put_repair_request(&plan, &request).unwrap();
    let mut candidate_draft = previous_draft.clone();
    candidate_draft.id = "work_item_draft_real_review_publish_0002".to_string();
    candidate_draft.revision_no += 1;
    candidate_draft.supersedes = Some(previous_draft.id);
    candidate_draft.revision_reason =
        crate::product::models::PlanRevisionReason::RepairUpstreamContract;
    candidate_draft.trigger_repair_request_id = Some(request.id.clone());
    candidate_draft.created_at = "2026-07-18T00:00:03Z".to_string();
    let repair_engine = crate::product::plan_repair::PlanRepairEngine::new(
        store.clone(),
        plan.clone(),
    )
    .with_candidate_drafts(vec![candidate_draft])
    .with_created_at("2026-07-18T00:00:03Z");
    let prepared = repair_engine.prepare_amendment(&request).unwrap();
    let unchanged_projection_ids = outcome
        .plan_revision
        .work_item_bindings
        .iter()
        .filter(|(candidate, _)| **candidate != logical_id)
        .map(|(candidate, revision_id)| {
            store
                .get_work_item_revision(&plan, candidate, revision_id)
                .unwrap()
                .work_item_projection_bundle_id
        })
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(prepared.revised_work_items.len(), 1);
    assert_eq!(
        prepared.work_item_projection_bundles.len(),
        outcome.plan_revision.work_item_bindings.len()
    );
    assert!(unchanged_projection_ids.iter().all(|bundle_id| {
        prepared
            .work_item_projection_bundles
            .iter()
            .any(|bundle| &bundle.id == bundle_id)
    }));

    let link = crate::product::models::WorkspaceSessionLink {
        id: "workspace_session_link_real_review_publish_0001".to_string(),
        relation: crate::product::models::WorkspaceSessionRelation::PlanRepair,
        parent_session_id: request.trigger_attempt_id.clone(),
        child_session_id: engine.session.session_id.clone(),
        trigger: crate::product::models::WorkspaceSessionLinkTrigger {
            attempt_id: request.trigger_attempt_id.clone(),
            unit_run_id: request.trigger_unit_run_id.clone(),
            review_id: request.trigger_review_id.clone(),
            finding_id: request.trigger_finding_id.clone(),
            repair_request_id: request.id.clone(),
            amendment_id: amendment_id.to_string(),
            fingerprint: request.fingerprint.clone(),
            base_plan_revision_id: base_revision_id.clone(),
        },
        return_context: crate::product::models::WorkspaceReturnContext {
            original_attempt_id: request.trigger_attempt_id.clone(),
            original_unit_run_id: request.trigger_unit_run_id.clone(),
            timeline_anchor_id: request.trigger_finding_id.clone(),
            original_route: "/coding/coding_attempt_real_review_publish_0001".to_string(),
        },
        created_at: "2026-07-18T00:00:02Z".to_string(),
    };
    engine.plan_repair_snapshot = Some(crate::product::models::PlanRepairSessionSnapshotDto {
        request: request.clone(),
        link,
        stage: crate::product::models::PlanRepairSessionStage::AuthoringRevision,
        projection: None,
        amendment: None,
        validation: None,
        impact: None,
        plan_review: None,
        package_identity: None,
        candidate_package_artifact_id: None,
        impact_scope_review: None,
        timeline_nodes: vec![],
        error: None,
    });
    engine.enter_plan_repair_review(prepared.clone()).await.unwrap();
    assert!(matches!(
        store.get_amendment_manifest(&plan, &prepared.manifest.id),
        Err(crate::product::json_store::ProductStoreError::NotFound { .. })
    ));
    let prompt = Arc::new(Mutex::new(None));
    engine
        .drive_review_session(
            Arc::new(ReviewVerdictStreamingProvider {
                output: r#"Canonical candidate package passed review.

```json
{
  "verdict": "pass",
  "review_scope": "outline",
  "generation_round_id": "real_review_publish_round_0001",
  "summary": "canonical package passed",
  "findings": []
}
```"#,
                provider_type: Arc::new(Mutex::new(None)),
                prompt: prompt.clone(),
            }),
            empty_provider_commands(),
        )
        .await;
    assert!(prompt.lock().unwrap().is_some());
    let snapshot = engine.plan_repair_session_state().unwrap();
    assert_eq!(
        snapshot.stage,
        crate::product::models::PlanRepairSessionStage::AwaitingConfirmation
    );
    let identity = snapshot.package_identity.as_ref().unwrap();
    let attestation = store
        .get_plan_repair_review_attestation(&plan, &identity.review_attestation_id)
        .unwrap();
    assert_eq!(
        attestation.candidate_package_fingerprint,
        prepared.candidate_package.candidate_package_fingerprint
    );
    let published = repair_engine
        .publish_amendment(
            prepared.clone(),
            crate::product::models::PlanAmendmentConfirmation {
                amendment_id: prepared.manifest.id.clone(),
                base_plan_revision_id: prepared.base_plan_revision_id.clone(),
                accepted_impact_scope: attestation.accepted_impact_scope.clone(),
                risk_acceptance_reason: attestation.risk_acceptance_reason.clone(),
                review_attestation_id: Some(attestation.id),
                confirmed_by: "user_real_review_publish_0001".to_string(),
                confirmed_at: "2026-07-18T00:00:05Z".to_string(),
            },
        )
        .unwrap();
    assert_eq!(published.id, prepared.manifest.id);
    let published_plan = store
        .get_plan_lineage("project_0001", "issue_0001", &plan_id)
        .unwrap();
    assert_eq!(
        published_plan.active_revision_id.as_deref(),
        Some(prepared.next_plan_revision.id.as_str())
    );
}
