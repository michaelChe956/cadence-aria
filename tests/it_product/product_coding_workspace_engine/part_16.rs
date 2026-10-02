fn create_active_coding_unit_run(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
) {
    crate::seed_coding_attempt_running(store, &attempt.project_id, &attempt.issue_id, &attempt.id);
    store
        .update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::ReviewRequest,
        )
        .expect("review request stage before group completion");
    // 805f0d84（C2 Task 6 决策 6/#13）：认领前冻结的 start_commit 是 Running
    // run 的必备事实——完成路径对真缺失停等（unit_run_start_commit_missing），
    // 绝不按 base HEAD 回填。fixture 绕过真实认领路径直接物化 run，故从
    // worktree 真实 HEAD 冻结 attempt.head_commit（对齐 it_web part_08 的
    // materialize_running_unit_run_for_logical），run.start_commit 经
    // attempt.head_commit 携带落盘。
    let start_head = attempt
        .worktree_path
        .as_ref()
        .map(|worktree| git_head(worktree));
    let attempt = &store
        .update_attempt_head_commit(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            start_head,
        )
        .expect("freeze fixture head commit");
    let unit = store
        .get_active_coding_unit(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("active unit lookup")
        .expect("active unit");
    let units = store
        .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("coding units");
    let revision_store = WorkItemRevisionStore::new(store.paths());
    let lineage = revision_store
        .get_plan_lineage(&attempt.project_id, &attempt.issue_id, "work_item_plan_0001")
        .expect("plan lineage");
    let resolved_handoff_revision_ids = unit
        .dependency_logical_work_item_ids
        .iter()
        .map(|dependency_id| {
            let dependency = units
                .iter()
                .find(|candidate| candidate.logical_work_item_id == *dependency_id)
                .expect("dependency unit");
            let handoff_id = dependency
                .latest_handoff_revision_id
                .as_ref()
                .expect("dependency handoff pointer");
            revision_store
                .get_handoff_revision(&lineage, dependency_id, handoff_id)
                .expect("dependency handoff revision");
            handoff_id.clone()
        })
        .collect();
    let revision = revision_store
        .get_work_item_revision(&lineage, &unit.logical_work_item_id, &unit.work_item_revision_id)
        .expect("work item revision");
    let bundle = revision_store
        .get_work_item_projection_bundle(&lineage, &revision.work_item_projection_bundle_id)
        .expect("projection bundle");
    let renderer_version = renderer_for(&ProviderName::Fake).renderer_version().to_string();
    let execution_no = store
        .list_coding_unit_runs(attempt, &unit.id)
        .expect("existing unit runs")
        .len() as u32
        + 1;
    store
        .create_coding_unit_run(
            attempt,
            &CodingUnitRun {
                id: format!("coding_unit_run_{}", unit.id),
                unit_id: unit.id,
                execution_no,
                work_item_revision_id: unit.work_item_revision_id,
                resolved_handoff_revision_ids,
                canonical_contract_hash: revision.canonical_contract_hash,
                projection_bundle_id: bundle.id,
                projection_compiler_version: bundle.compiler_version,
                coder_provider_renderer_version: renderer_version.clone(),
                reviewer_provider_renderer_version: renderer_version.clone(),
                internal_reviewer_provider_renderer_version: None,
                coder_projection_hash: bundle.coder_projection_hash,
                reviewer_projection_hash: bundle.reviewer_projection_hash,
                coder_execution_context_hash: None,
                reviewer_execution_context_hash: None,
                internal_reviewer_execution_context_hash: None,
                status: CodingUnitRunStatus::Running,
                unit_rework_count: 0,
                verification_retry_count: 0,
                operational_retry_count: 0,
                plan_repair_count: 0,
                start_commit: attempt.head_commit.clone(),
                completion_commit: None,
                created_at: "2026-07-19T00:00:00Z".to_string(),
                updated_at: "2026-07-19T00:00:00Z".to_string(),
            },
        )
        .expect("create active unit run");
}
