use super::*;

#[tokio::test]
async fn schema_v2_group_final_confirm_completes_without_removed_test_artifacts() {
    let root = tempdir().expect("tempdir");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    init_test_git_repo(&worktree);
    let head = git_stdout(&worktree, &["rev-parse", "HEAD"])
        .trim()
        .to_string();
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_work_item_id: "work_item_0001".to_string(),
            base_branch: head.clone(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: Some(worktree),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
            },
            max_auto_rework: 2,
        })
        .expect("group attempt");
    let required_checks = [VerificationCheck {
        check_id: "check_unit_tests".to_string(),
        command: Some("node --test".to_string()),
        manual_instruction: None,
        required: true,
        non_zero_test_execution_required: true,
    }];
    seed_schema_v2_group_attempt_fixture(&store, &attempt, true, false, &required_checks);
    assert!(
        LifecycleStore::new(store.paths())
            .list_work_items(&attempt.project_id, &attempt.issue_id)
            .expect("legacy work item list")
            .is_empty()
    );

    let revision_store = WorkItemRevisionStore::new(store.paths());
    let lineage = revision_store
        .get_plan_lineage(
            &attempt.project_id,
            &attempt.issue_id,
            attempt
                .work_item_group_id
                .as_deref()
                .expect("group plan id"),
        )
        .expect("lineage");
    for unit in store
        .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("units")
    {
        let revision = revision_store
            .get_work_item_revision(
                &lineage,
                &unit.logical_work_item_id,
                &unit.work_item_revision_id,
            )
            .expect("work item revision");
        let bundle = revision_store
            .get_work_item_projection_bundle(&lineage, &revision.work_item_projection_bundle_id)
            .expect("projection bundle");
        let run = CodingUnitRun {
            id: format!("coding_unit_run_{}", unit.order_index + 1),
            unit_id: unit.id.clone(),
            execution_no: 1,
            work_item_revision_id: revision.id.clone(),
            resolved_handoff_revision_ids: Vec::new(),
            canonical_contract_hash: revision.canonical_contract_hash.clone(),
            projection_bundle_id: bundle.id.clone(),
            projection_compiler_version: bundle.compiler_version.clone(),
            coder_provider_renderer_version: renderer_for(&ProviderName::Codex)
                .renderer_version()
                .to_string(),
            reviewer_provider_renderer_version: renderer_for(&ProviderName::ClaudeCode)
                .renderer_version()
                .to_string(),
            internal_reviewer_provider_renderer_version: None,
            coder_projection_hash: bundle.coder_projection_hash.clone(),
            reviewer_projection_hash: bundle.reviewer_projection_hash.clone(),
            coder_execution_context_hash: None,
            reviewer_execution_context_hash: None,
            internal_reviewer_execution_context_hash: None,
            status: CodingUnitRunStatus::Completed,
            unit_rework_count: 0,
            verification_retry_count: 0,
            operational_retry_count: 0,
            plan_repair_count: 0,
            start_commit: Some(head.clone()),
            completion_commit: Some(head.clone()),
            created_at: "2026-07-27T00:00:00Z".to_string(),
            updated_at: "2026-07-27T00:00:00Z".to_string(),
        };
        store
            .create_coding_unit_run(&attempt, &run)
            .expect("coding unit run");
        let handoff = HandoffRevision {
            id: format!("handoff_revision_{}", unit.order_index + 1),
            logical_work_item_id: unit.logical_work_item_id.clone(),
            work_item_revision_id: revision.id,
            coding_unit_run_id: run.id,
            provided_contracts: revision
                .canonical_contract
                .output_contracts
                .iter()
                .map(|contract| contract.contract_id.clone())
                .collect(),
            provided_capabilities: revision
                .canonical_contract
                .output_contracts
                .iter()
                .map(|contract| (contract.contract_id.clone(), contract.capabilities.clone()))
                .collect(),
            contract_hash: format!("handoff_contract_hash_{}", unit.order_index + 1),
            commit_sha: head.clone(),
            created_at: "2026-07-27T00:00:00Z".to_string(),
        };
        revision_store
            .put_handoff_revision(&lineage, &handoff)
            .expect("handoff revision");
        store
            .update_coding_unit_latest_handoff_revision_id(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &unit.id,
                Some(handoff.id),
            )
            .expect("handoff revision binding");
        store
            .update_coding_unit_status(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &unit.id,
                CodingExecutionUnitStatus::Completed,
                Some("completed".to_string()),
            )
            .expect("complete unit");
        store
            .update_coding_unit_completion_commit(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &unit.id,
                Some(head.clone()),
            )
            .expect("completion commit");
    }
    let mut attempt = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("persisted attempt");
    attempt.status = CodingAttemptStatus::WaitingForHuman;
    attempt.stage = CodingExecutionStage::FinalConfirm;
    attempt.head_commit = Some(head);
    store
        .save_coding_attempt(&attempt)
        .expect("final review attempt");

    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let updated = engine
        .handle_final_confirm(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .await
        .expect("required checks must not require testing reports");

    assert_eq!(updated.status, CodingAttemptStatus::Completed);
    assert!(updated.completed_at.is_some());
    assert!(
        store
            .list_coding_units(&updated.project_id, &updated.issue_id, &updated.id)
            .expect("units")
            .iter()
            .all(|unit| unit.status == CodingExecutionUnitStatus::Completed)
    );
}
