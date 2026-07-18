struct PlanRepairReviewRaceFixture {
    _tmp: TempDir,
    store: WorkItemRevisionStore,
    plan: crate::product::models::WorkItemPlanLineage,
    request: crate::product::models::PlanRepairRequest,
    engine: WorkspaceEngine,
}

async fn plan_repair_review_race_fixture() -> PlanRepairReviewRaceFixture {
    let (tmp, lifecycle, plan_id, mut engine) =
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
    let amendment_id = "plan_amendment_review_race_0001";
    let request_id = "plan_repair_request_review_race_0001";
    let plan = store
        .acquire_active_amendment(&plan, amendment_id)
        .unwrap();
    let request = crate::product::models::PlanRepairRequest {
        id: request_id.to_string(),
        plan_id: plan.id.clone(),
        base_plan_revision_id: base_revision_id.clone(),
        trigger_attempt_id: "coding_attempt_review_race_0001".to_string(),
        trigger_unit_run_id: "coding_unit_run_review_race_0001".to_string(),
        trigger_review_id: Some("code_review_review_race_0001".to_string()),
        trigger_finding_id: "finding_review_race_0001".to_string(),
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
        fingerprint: "fingerprint_review_race_0001".to_string(),
        status: crate::product::models::PlanRepairRequestStatus::InProgress,
        created_at: "2026-07-18T00:00:02Z".to_string(),
        updated_at: "2026-07-18T00:00:02Z".to_string(),
    };
    store.put_repair_request(&plan, &request).unwrap();

    let mut candidate_projection = outcome.plan_projection_bundle.clone();
    candidate_projection.id = "plan_projection_bundle_review_race_0002".to_string();
    candidate_projection.plan_revision_id = "plan_revision_review_race_0002".to_string();
    candidate_projection.created_at = "2026-07-18T00:00:03Z".to_string();
    store
        .put_plan_projection_bundle(&plan, &candidate_projection)
        .unwrap();
    let mut candidate_validation = outcome.validation_report.clone();
    candidate_validation.id = "plan_validation_report_review_race_0002".to_string();
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
    store.put_amendment_manifest(&plan, &manifest).unwrap();
    let link = crate::product::models::WorkspaceSessionLink {
        id: "workspace_session_link_review_race_0001".to_string(),
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
            base_plan_revision_id: base_revision_id,
        },
        return_context: crate::product::models::WorkspaceReturnContext {
            original_attempt_id: request.trigger_attempt_id.clone(),
            original_unit_run_id: request.trigger_unit_run_id.clone(),
            timeline_anchor_id: request.trigger_finding_id.clone(),
            original_route: "/coding/coding_attempt_review_race_0001".to_string(),
        },
        created_at: "2026-07-18T00:00:02Z".to_string(),
    };
    engine.plan_repair_snapshot = Some(crate::product::models::PlanRepairSessionSnapshotDto {
        request: request.clone(),
        link,
        stage: crate::product::models::PlanRepairSessionStage::PlanReview,
        projection: Some(candidate_projection.clone()),
        amendment: Some(manifest),
        validation: Some(candidate_validation),
        impact: Some(crate::product::plan_repair::ContractImpactReport {
            unaffected: candidate_revision
                .work_item_bindings
                .keys()
                .filter(|candidate| **candidate != logical_id && **candidate != minimum_scope_unit)
                .cloned()
                .collect(),
            direct_revalidation: vec![minimum_scope_unit],
            direct_stale: vec![],
            conditional_downstream: vec![],
            explanation_paths: vec![],
        }),
        plan_review: None,
        package_identity: None,
        impact_scope_review: None,
        timeline_nodes: vec![],
        error: None,
    });
    engine.session.artifact = Some(ArtifactPayload::WorkItemPlanProjection {
        projection: Box::new(candidate_projection),
    });
    PlanRepairReviewRaceFixture {
        _tmp: tmp,
        store,
        plan,
        request,
        engine,
    }
}

#[tokio::test]
async fn plan_repair_review_context_rejects_request_field_divergence_from_authoritative_store() {
    for field in ["evidence", "refs", "reason", "status", "updated_at"] {
        let mut fixture = plan_repair_review_race_fixture().await;
        let request = &mut fixture.engine.plan_repair_snapshot.as_mut().unwrap().request;
        match field {
            "evidence" => request.evidence.push(crate::product::models::PlanDefectEvidence {
                kind: "review_note".to_string(),
                source_ref: "review://tampered".to_string(),
                message: "tampered evidence".to_string(),
            }),
            "refs" => {
                request.contract_refs.push("contract.tampered".to_string());
                request.capability_refs.push("tampered_capability".to_string());
            }
            "reason" => request.reason_code = "tampered_reason".to_string(),
            "status" => {
                request.status =
                    crate::product::models::PlanRepairRequestStatus::AwaitingConfirmation;
            }
            "updated_at" => request.updated_at = "2026-07-18T00:00:09Z".to_string(),
            _ => unreachable!(),
        }

        let error = fixture
            .engine
            .build_work_item_plan_review_input()
            .unwrap_err();

        assert!(
            format!("{error:?}").contains("Plan Repair review candidate provenance mismatch"),
            "request field {field}"
        );
    }
}

struct PlanRepairReviewRequestRaceProvider {
    inner: ReviewVerdictStreamingProvider,
    store: WorkItemRevisionStore,
    plan: crate::product::models::WorkItemPlanLineage,
    request_id: String,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for PlanRepairReviewRequestRaceProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.store
            .merge_repair_request_evidence(
                &self.plan,
                &self.request_id,
                vec![crate::product::models::PlanDefectEvidence {
                    kind: "late_evidence".to_string(),
                    source_ref: "review://late".to_string(),
                    message: "arrived after prompt construction".to_string(),
                }],
            )
            .unwrap();
        self.inner.start(input, cancel).await
    }

    async fn run_streaming(
        &self,
        input: &AdapterInput,
        cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        self.inner.run_streaming(input, cancel).await
    }
}

#[tokio::test]
async fn plan_repair_review_completion_rejects_request_evidence_race_without_attestation() {
    let mut fixture = plan_repair_review_race_fixture().await;
    fixture.engine.begin_work_item_plan_outline_review_run().await;
    let prompt = Arc::new(Mutex::new(None));

    fixture
        .engine
        .drive_review_session(
            Arc::new(PlanRepairReviewRequestRaceProvider {
                inner: ReviewVerdictStreamingProvider {
                    output: r#"Plan Repair candidate passed.

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
                    prompt: prompt.clone(),
                },
                store: fixture.store.clone(),
                plan: fixture.plan.clone(),
                request_id: fixture.request.id.clone(),
            }),
            empty_provider_commands(),
        )
        .await;

    assert!(
        prompt
            .lock()
            .unwrap()
            .as_deref()
            .is_some_and(|value| value.contains(&fixture.request.id))
    );
    let verdict = fixture.engine.latest_review_verdict.as_ref().unwrap();
    assert_eq!(verdict.verdict, ReviewVerdictType::Pass);
    assert_eq!(
        verdict.work_item_plan_review.as_ref().unwrap().review_action,
        WorkItemPlanReviewAction::Continue
    );
    assert_eq!(fixture.engine.current_stage(), WorkspaceStage::HumanConfirm);
    let snapshot = fixture.engine.plan_repair_session_state().unwrap();
    assert!(snapshot.package_identity.is_none());
    let attestation_id = format!(
        "plan_repair_review_attestation_{}_round_0001",
        fixture.request.amendment_id.as_deref().unwrap()
    );
    assert!(matches!(
        fixture
            .store
            .get_plan_repair_review_attestation(&fixture.plan, &attestation_id),
        Err(crate::product::json_store::ProductStoreError::NotFound { .. })
    ));
}
