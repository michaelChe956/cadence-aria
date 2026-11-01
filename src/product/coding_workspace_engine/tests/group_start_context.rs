// 从 tests.rs 拆出以满足 large_file_guard 的 1200 行上限（纯移动，无行为变化）：
// group attempt 启动/评审上下文两个内联测试。

use super::*;

#[test]
fn group_final_review_evaluation_context_omits_projection_body_but_keeps_hash() {
    let root = tempdir().expect("tempdir");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_work_item_id: "work_item_0001".to_string(),
            base_branch: "HEAD".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Fake,
                reviewer: Some(ProviderName::Fake),
                review_rounds: 1,
                permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
            },
            target_snapshot: None,
            max_auto_rework: 2,
            start_run_policy: crate::product::coding_models::CodingStartRunPolicy::Manual,
        })
        .expect("group attempt");
    seed_group_attempt_fixture(&store, &attempt, true, false);

    let revision_store = WorkItemRevisionStore::new(store.paths());
    let lineage = revision_store
        .get_plan_lineage(
            &attempt.project_id,
            &attempt.issue_id,
            "work_item_plan_0001",
        )
        .expect("plan lineage");
    let units = store
        .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("coding units");
    for unit in units {
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
        store
            .create_coding_unit_run(
                &attempt,
                &CodingUnitRun {
                    id: format!("{}_run_0001", unit.id),
                    unit_id: unit.id,
                    execution_no: 1,
                    work_item_revision_id: revision.id,
                    resolved_handoff_revision_ids: Vec::new(),
                    canonical_contract_hash: bundle.canonical_contract_hash,
                    projection_bundle_id: bundle.id,
                    projection_compiler_version: bundle.compiler_version,
                    coder_provider_renderer_version: "test-renderer-v1".to_string(),
                    reviewer_provider_renderer_version: "test-renderer-v1".to_string(),
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
                    start_commit: None,
                    completion_commit: None,
                    created_at: "2026-08-04T00:00:00Z".to_string(),
                    updated_at: "2026-08-04T00:00:00Z".to_string(),
                },
            )
            .expect("completed unit run");
    }

    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);
    let context: serde_json::Value = serde_json::from_str(
        &engine
            .group_final_review_evaluation_context_json(&attempt)
            .expect("group final review evaluation context"),
    )
    .expect("evaluation context json");
    let units = context["units"].as_array().expect("unit contexts");
    assert_eq!(units.len(), 3);
    for unit in units {
        let unit = unit.as_object().expect("unit context object");
        assert!(!unit.contains_key("reviewer_projection"));
        assert!(
            unit.get("reviewer_projection_hash")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|hash| !hash.is_empty())
        );
    }
}

#[tokio::test]
async fn group_attempt_records_base_head_as_first_unit_start_commit() {
    let root = tempdir().expect("tempdir");
    let worktree = root.path().join("shared-worktree");
    std::fs::create_dir_all(&worktree).expect("worktree dir");
    init_test_git_repo(&worktree);
    let base_head = git_stdout(&worktree, &["rev-parse", "HEAD"])
        .trim()
        .to_string();
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_work_item_id: "work_item_0001".to_string(),
            base_branch: base_head.clone(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: Some(worktree.clone()),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
                permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
            },
            target_snapshot: None,
            max_auto_rework: 2,
            start_run_policy: crate::product::coding_models::CodingStartRunPolicy::Manual,
        })
        .expect("group attempt");
    seed_group_attempt_fixture(&store, &attempt, true, false);
    let (tx, mut rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    let updated = engine
        .start_attempt("project_0001", "issue_0001", &attempt.id)
        .await
        .expect("start group attempt");

    assert_eq!(updated.scope, CodingAttemptScope::WorkItemGroup);
    assert_eq!(updated.status, CodingAttemptStatus::Running);
    assert_eq!(updated.stage, CodingExecutionStage::Coding);
    assert_eq!(updated.worktree_path.as_deref(), Some(worktree.as_path()));
    assert_eq!(updated.head_commit.as_deref(), Some(base_head.as_str()));
    assert!(
        store
            .get_timeline_nodes("project_0001", "issue_0001", &attempt.id)
            .expect("timeline")
            .is_empty()
    );
    assert_eq!(
        rx.recv().await.expect("stage event"),
        CodingWsOutMessage::CodingStageChange {
            stage: CodingExecutionStage::Coding,
        }
    );
    assert!(rx.try_recv().is_err());

    engine
        .render_coder_unit_run_context(&updated, &ProviderName::Codex)
        .expect("create first unit run");
    let first_unit_run = store.get_active_unit_run(&updated).expect("first unit run");
    assert_eq!(
        first_unit_run.start_commit.as_deref(),
        Some(base_head.as_str())
    );
    let persisted = store
        .get_attempt("project_0001", "issue_0001", &attempt.id)
        .expect("persisted attempt");
    assert_eq!(persisted.head_commit.as_deref(), Some(base_head.as_str()));
}
