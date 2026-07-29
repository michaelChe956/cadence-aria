fn seed_authoritative_group_final_review_fixture(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
) {
    seed_authoritative_group_terminal_fixture(store, attempt);
    let revision_store = WorkItemRevisionStore::new(store.paths());
    let lineage = revision_store
        .get_plan_lineage(&attempt.project_id, &attempt.issue_id, "work_item_plan_0001")
        .expect("group plan lineage");
    let providers = store
        .get_role_provider_config_snapshot(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("provider snapshot");
    let coder_renderer_version = renderer_for(&providers.coder)
        .renderer_version()
        .to_string();
    let reviewer_renderer_version = renderer_for(&providers.code_reviewer)
        .renderer_version()
        .to_string();
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
                dependency_logical_work_item_ids: if index == 1 {
                    vec!["work_item_0001".to_string()]
                } else {
                    Vec::new()
                },
                order_index: index as u32,
                status: CodingExecutionUnitStatus::Completed,
            })
            .expect("completed coding unit");
        let run = CodingUnitRun {
            id: format!("coding_unit_run_{:04}", index + 1),
            unit_id: unit.id.clone(),
            execution_no: 1,
            work_item_revision_id: revision.id,
            resolved_handoff_revision_ids: if index == 1 {
                vec!["handoff_revision_coding_unit_run_0001".to_string()]
            } else {
                Vec::new()
            },
            canonical_contract_hash: bundle.canonical_contract_hash,
            projection_bundle_id: bundle.id,
            projection_compiler_version: bundle.compiler_version,
            coder_provider_renderer_version: coder_renderer_version.clone(),
            reviewer_provider_renderer_version: reviewer_renderer_version.clone(),
            internal_reviewer_provider_renderer_version: None,
            coder_projection_hash: bundle.coder_projection_hash,
            reviewer_projection_hash: bundle.reviewer_projection_hash,
            coder_execution_context_hash: None,
            reviewer_execution_context_hash: None,
            internal_reviewer_execution_context_hash: None,
            status: CodingUnitRunStatus::Completed,
            unit_rework_count: 0,
            verification_retry_count: 0,
            operational_retry_count: 0,
            plan_repair_count: 0,
            start_commit: Some("seed-commit".to_string()),
            completion_commit: Some("seed-commit".to_string()),
            created_at: "2026-07-19T00:00:00Z".to_string(),
            updated_at: "2026-07-19T00:00:00Z".to_string(),
        };
        store
            .create_coding_unit_run(attempt, &run)
            .expect("completed coding unit run");
        let handoff = HandoffRevision {
            id: format!("handoff_revision_{}", run.id),
            logical_work_item_id: logical_id.to_string(),
            work_item_revision_id: run.work_item_revision_id.clone(),
            coding_unit_run_id: run.id,
            provided_contracts: vec![format!("contract::{logical_id}")],
            provided_capabilities: std::collections::BTreeMap::from([(
                format!("contract::{logical_id}"),
                if index == 0 {
                    vec![
                        format!("capability::{logical_id}"),
                        "fn climb_stairs(n: i32) -> i32".to_string(),
                    ]
                } else {
                    vec![format!("capability::{logical_id}")]
                },
            )]),
            contract_hash: run.canonical_contract_hash,
            commit_sha: "seed-commit".to_string(),
            created_at: "2026-07-19T00:00:00Z".to_string(),
        };
        revision_store
            .put_handoff_revision(&lineage, &handoff)
            .expect("handoff revision");
        store
            .update_coding_unit_completion_commit(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &unit.id,
                Some("seed-commit".to_string()),
            )
            .expect("completion commit");
        store
            .update_coding_unit_latest_handoff_revision_id(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &unit.id,
                Some(handoff.id),
            )
            .expect("handoff binding");
    }
}
