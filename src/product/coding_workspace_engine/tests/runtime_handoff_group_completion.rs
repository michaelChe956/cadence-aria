#[tokio::test]
async fn coding_runtime_handoff_group_completion_resumes_and_starts_waiting_consumer_run() {
    let fixture = group_completion_fixture(true, true);
    create_authoritative_active_run(
        &fixture,
        "coding_unit_run_0001",
        1,
        CodingUnitRunStatus::Running,
        None,
        None,
    );
    let revision_store = WorkItemRevisionStore::new(fixture.store.paths());
    let lineage = revision_store
        .get_plan_lineage(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            "work_item_plan_0001",
        )
        .expect("lineage");
    revision_store
        .put_handoff_revision(
            &lineage,
            &HandoffRevision {
                id: "handoff_revision_previous".to_string(),
                logical_work_item_id: "work_item_0001".to_string(),
                work_item_revision_id: "work_item_revision_0001".to_string(),
                coding_unit_run_id: "coding_unit_run_previous".to_string(),
                provided_contracts: vec!["contract_work_item_0001".to_string()],
                provided_capabilities: std::collections::BTreeMap::from([(
                    "contract_work_item_0001".to_string(),
                    vec!["capability_work_item_0001".to_string()],
                )]),
                contract_hash:
                    "5d1465e86ea2fbad8df040b5eac6ab52130ce6b06d2bd3b6403305c3b3e83b23"
                        .to_string(),
                commit_sha: fixture.original_head.clone(),
                tests: vec!["old test evidence".to_string()],
                artifacts: vec!["old artifact".to_string()],
                created_at: "2026-07-18T00:00:00Z".to_string(),
            },
        )
        .expect("previous handoff");
    let mut amended_plan = revision_store
        .get_plan_revision(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &lineage.id,
            "plan_revision_0001",
        )
        .expect("initial plan revision");
    amended_plan.id = "plan_revision_0002".to_string();
    amended_plan.revision_no = 2;
    amended_plan.supersedes = Some("plan_revision_0001".to_string());
    amended_plan.reason = crate::product::models::PlanRevisionReason::RepairUpstreamContract;
    amended_plan.created_at = "2026-07-18T00:00:01Z".to_string();
    revision_store
        .put_plan_revision(&lineage, &amended_plan)
        .expect("amended plan revision");
    revision_store
        .set_active_plan_revision(&lineage, &amended_plan.id)
        .expect("active amended plan");
    let amendment = crate::product::models::PlanAmendmentManifest {
        id: "plan_amendment_0001".to_string(),
        repair_request_id: "plan_repair_request_0001".to_string(),
        previous_plan_revision_id: "plan_revision_0001".to_string(),
        new_plan_revision_id: amended_plan.id.clone(),
        revised_work_items: std::collections::BTreeMap::new(),
        superseded_revisions: Vec::new(),
        dependency_graph_changes: Vec::new(),
        contract_deltas: Vec::new(),
        unaffected_units: vec!["work_item_0003".to_string()],
        revalidation_required_units: Vec::new(),
        stale_units: Vec::new(),
        replacement_units: std::collections::BTreeMap::new(),
        resume_target: crate::product::models::AmendmentResumeTarget {
            logical_work_item_id: "work_item_0002".to_string(),
            mode: crate::product::models::AmendmentResumeMode::AwaitHandoff,
        },
        created_at: "2026-07-18T00:00:01Z".to_string(),
    };
    revision_store
        .put_amendment_manifest(&lineage, &amendment)
        .expect("amendment");
    let mut binding = fixture
        .store
        .get_plan_binding(&fixture.attempt)
        .expect("plan binding");
    binding.bound_plan_revision_id = amended_plan.id;
    binding.applied_amendment_ids = vec![amendment.id.clone()];
    fixture
        .store
        .save_plan_binding(&fixture.attempt, &binding)
        .expect("applied amendment binding");

    let units = fixture
        .store
        .list_coding_units(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .expect("units");
    let consumer = units
        .iter()
        .find(|unit| unit.logical_work_item_id == "work_item_0002")
        .expect("consumer")
        .clone();
    let revision = revision_store
        .get_work_item_revision(
            &lineage,
            &consumer.logical_work_item_id,
            &consumer.work_item_revision_id,
        )
        .expect("consumer revision");
    let bundle = revision_store
        .get_work_item_projection_bundle(&lineage, &revision.work_item_projection_bundle_id)
        .expect("consumer bundle");
    let providers = fixture
        .store
        .get_role_provider_config_snapshot(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .expect("providers");
    fixture
        .store
        .create_coding_unit_run(
            &fixture.attempt,
            &CodingUnitRun {
                id: format!("coding_unit_run_{}_{}", consumer.id, amendment.id),
                unit_id: consumer.id.clone(),
                execution_no: 1,
                work_item_revision_id: revision.id.clone(),
                resolved_handoff_revision_ids: Vec::new(),
                canonical_contract_hash: revision.canonical_contract_hash.clone(),
                projection_bundle_id: bundle.id.clone(),
                projection_compiler_version: bundle.compiler_version.clone(),
                coder_provider_renderer_version: renderer_for(&providers.coder)
                    .renderer_version()
                    .to_string(),
                reviewer_provider_renderer_version: renderer_for(&providers.code_reviewer)
                    .renderer_version()
                    .to_string(),
                internal_reviewer_provider_renderer_version: None,
                coder_projection_hash: bundle.coder_projection_hash.clone(),
                reviewer_projection_hash: bundle.reviewer_projection_hash.clone(),
                coder_execution_context_hash: None,
                reviewer_execution_context_hash: None,
                internal_reviewer_execution_context_hash: None,
                status: CodingUnitRunStatus::AwaitingAmendment,
                unit_rework_count: 0,
                verification_retry_count: 0,
                operational_retry_count: 0,
                plan_repair_count: 1,
                start_commit: Some(fixture.original_head.clone()),
                completion_commit: None,
                created_at: "2026-07-18T00:00:01Z".to_string(),
                updated_at: "2026-07-18T00:00:01Z".to_string(),
            },
        )
        .expect("waiting consumer run");

    let updated = fixture
        .engine
        .complete_group_unit_after_code_review(&fixture.attempt)
        .await
        .expect("complete repaired source");

    assert_eq!(
        updated.current_work_item_id.as_deref(),
        Some("work_item_0002")
    );
    let consumer_run = fixture
        .store
        .list_unit_runs_by_logical_id(&updated, "work_item_0002")
        .expect("consumer runs")
        .pop()
        .expect("consumer run");
    assert_eq!(consumer_run.status, CodingUnitRunStatus::Running);
    assert_eq!(
        consumer_run.resolved_handoff_revision_ids,
        vec!["handoff_revision_coding_unit_run_0001"]
    );
}
