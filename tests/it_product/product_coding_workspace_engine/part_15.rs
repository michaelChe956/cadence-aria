fn seed_authoritative_group_final_review_fixture(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
) {
    seed_authoritative_group_terminal_fixture(store, attempt);
    let revision_store = WorkItemRevisionStore::new(store.paths());
    let lineage = revision_store
        .get_plan_lineage(&attempt.project_id, &attempt.issue_id, "work_item_plan_0001")
        .expect("group plan lineage");
    let renderer_version = renderer_for(&ProviderName::Fake).renderer_version().to_string();
    for (index, logical_id) in ["work_item_0001", "work_item_0002"]
        .into_iter()
        .enumerate()
    {
        let revision_id = format!("work_item_revision_{:04}", index + 1);
        let revision = revision_store
            .get_work_item_revision(&lineage, logical_id, &revision_id)
            .expect("work item revision");
        let bundle = revision_store
            .get_work_item_projection_bundle(&lineage, &revision.work_item_projection_bundle_id)
            .expect("projection bundle");
        let unit = store
            .create_coding_unit(CreateCodingExecutionUnitInput {
                attempt_id: attempt.id.clone(),
                project_id: attempt.project_id.clone(),
                issue_id: attempt.issue_id.clone(),
                plan_id: "work_item_plan_0001".to_string(),
                logical_work_item_id: logical_id.to_string(),
                work_item_revision_id: revision.id.clone(),
                dependency_logical_work_item_ids: Vec::new(),
                order_index: index as u32,
                status: CodingExecutionUnitStatus::Completed,
            })
            .expect("completed coding unit");
        store
            .create_coding_unit_run(
                attempt,
                &CodingUnitRun {
                    id: format!("coding_unit_run_{:04}", index + 1),
                    unit_id: unit.id.clone(),
                    execution_no: 1,
                    work_item_revision_id: revision.id,
                    resolved_handoff_revision_ids: Vec::new(),
                    canonical_contract_hash: bundle.canonical_contract_hash,
                    projection_bundle_id: bundle.id,
                    projection_compiler_version: bundle.compiler_version,
                    coder_provider_renderer_version: renderer_version.clone(),
                    reviewer_provider_renderer_version: renderer_version.clone(),
                    coder_projection_hash: bundle.coder_projection_hash,
                    reviewer_projection_hash: bundle.reviewer_projection_hash,
                    coder_execution_context_hash: None,
                    reviewer_execution_context_hash: None,
                    status: CodingUnitRunStatus::Completed,
                    unit_rework_count: 0,
                    verification_retry_count: 0,
                    operational_retry_count: 0,
                    plan_repair_count: 0,
                    start_commit: Some("seed-commit".to_string()),
                    completion_commit: Some("seed-commit".to_string()),
                    created_at: "2026-07-19T00:00:00Z".to_string(),
                    updated_at: "2026-07-19T00:00:00Z".to_string(),
                },
            )
            .expect("completed coding unit run");
        store
            .save_coding_unit_handoff(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &unit.id,
                &WorkItemHandoff {
                    id: format!("work_item_handoff_{:04}", index + 1),
                    project_id: attempt.project_id.clone(),
                    issue_id: attempt.issue_id.clone(),
                    work_item_id: logical_id.to_string(),
                    attempt_id: attempt.id.clone(),
                    provider_run_ref: None,
                    summary: format!("completed {logical_id}"),
                    files_changed: Vec::new(),
                    commit_sha: Some("seed-commit".to_string()),
                    diff_summary: String::new(),
                    tests_run: Vec::new(),
                    test_result_summary: "passed".to_string(),
                    review_summary: None,
                    api_or_contract_changes: Vec::new(),
                    open_risks: Vec::new(),
                    next_work_item_notes: Vec::new(),
                    created_at: "2026-07-19T00:00:00Z".to_string(),
                },
            )
            .expect("coding unit handoff");
    }
}
