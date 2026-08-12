use super::*;
use crate::product::work_item_projection::{CoderExecutionEnvelope, renderer_for};

#[tokio::test]
async fn coding_plan_repair_group_rework_uses_bound_authoritative_coder_context() {
    let root = tempdir().unwrap();
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).unwrap();
    init_test_git_repo(&worktree);
    let head = git_stdout(&worktree, &["rev-parse", "HEAD"]);
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
                permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
            },
            target_snapshot: None,
            max_auto_rework: 2,
        })
        .unwrap();
    seed_group_attempt_fixture(&store, &attempt, true, false);
    let mut attempt = store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .unwrap();
    attempt.head_commit = Some(head.clone());
    attempt.stage = CodingExecutionStage::Coding;
    store.write_coding_attempt_for_test(&attempt).unwrap();
    let (tx, _rx) = mpsc::channel(64);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let initial = super::provider_execution_context::CapturingProjectionProvider::new(
        "initial coding complete",
    );

    let coded = engine
        .execute_coding(&attempt, &initial, &CodingExecutionContext::default())
        .await
        .unwrap();
    let run_before = store.get_active_unit_run(&coded).unwrap();
    let coded = store
        .update_attempt_stage(
            &coded.project_id,
            &coded.issue_id,
            &coded.id,
            CodingExecutionStage::CodeReview,
        )
        .unwrap();
    let rework = super::provider_execution_context::CapturingProjectionProvider::new(
        "coder fixed reviewer findings",
    );
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    let updated = engine
        .execute_coder_fix_from_review(
            &coded,
            &super::provider_driven::review_report_requesting_changes(&coded),
            &CodingExecutionContext::default(),
            &rework,
            &mut command_rx,
        )
        .await
        .unwrap();

    let revision_store = WorkItemRevisionStore::new(store.paths());
    let lineage = revision_store
        .get_plan_lineage("project_0001", "issue_0001", "work_item_plan_0001")
        .unwrap();
    let bundle = revision_store
        .get_work_item_projection_bundle(&lineage, &run_before.projection_bundle_id)
        .unwrap();
    let expected_context = renderer_for(&ProviderName::Codex)
        .render_coder(
            &bundle.coder_projection,
            &CoderExecutionEnvelope {
                repository_state_ref: head.clone(),
                resolved_handoff_revision_ids: Vec::new(),
                unit_run_id: run_before.id.clone(),
                previous_actionable_review: None,
                start_commit: Some(head),
            },
        )
        .unwrap();
    let input = rework.input();
    assert!(input.prompt.starts_with(&expected_context.text));
    assert!(input.prompt.contains("reviewer requested changes"));
    assert!(input.prompt.contains("missing validation"));
    let run_after = store.get_active_unit_run(&updated).unwrap();
    assert_eq!(
        run_after.coder_execution_context_hash.as_deref(),
        Some(expected_context.content_hash.as_str())
    );
    assert_eq!(
        run_after.coder_provider_renderer_version,
        expected_context.renderer_version
    );
}
