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
            start_run_policy: crate::product::coding_models::CodingStartRunPolicy::Manual,
        })
        .unwrap();
    seed_group_attempt_fixture(&store, &attempt, true, false);
    let mut attempt = store
        .seed_running_attempt_for_test(&attempt.project_id, &attempt.issue_id, &attempt.id)
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
// G7（终局关闸缺口，#18 原族 rerun 路径残留）：rerun_planned_command 落
// 未消费 rework 指令后 runner 续起（REST rerun 自动 spawn 与 WS recover
// 同走 execute_coding）——重渲染不得把指令摘要带进绑定渲染上下文
//（unit run 已冻结 coder_execution_context_hash，恒自撞死循环）；指令
// 经增量段进入实际 prompt，并由 Task 7 认领事务消费。
#[tokio::test]
async fn coding_restart_after_unconsumed_rerun_instruction_keeps_frozen_context_and_consumes_instruction() {
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
            base_branch: "HEAD".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: Some(worktree.clone()),
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
        .unwrap();
    seed_group_attempt_fixture(&store, &attempt, true, false);
    let mut attempt = store
        .seed_running_attempt_for_test(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .unwrap();
    attempt.head_commit = Some(head);
    attempt.stage = CodingExecutionStage::Coding;
    store.write_coding_attempt_for_test(&attempt).unwrap();
    let (tx, _rx) = mpsc::channel(64);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let initial = super::provider_execution_context::CapturingProjectionProvider::new(
        super::provider_execution_context::current_plan_defect_output(),
    );

    let coded = engine
        .execute_coding(&attempt, &initial, &CodingExecutionContext::default())
        .await
        .unwrap();
    let run_before = store.get_active_unit_run(&coded).unwrap();
    let frozen_hash = run_before
        .coder_execution_context_hash
        .clone()
        .expect("first coding binds the coder context hash");

    // 落 rerun 返修指令（对齐 rerun_planned_command 的 durable 落地面：
    // 未消费指令＋返修计数推进＋admission 回 Coding 段）。
    let instruction = crate::product::coding_models::CodingReworkInstruction {
        id: "coding_rework_instruction_0001".to_string(),
        attempt_id: coded.id.clone(),
        source_stage: CodingExecutionStage::CodeReview,
        rework_round: coded.rework_count + 1,
        summary: "重跑原计划命令".to_string(),
        fix_hints: vec!["按计划合同字面命令执行：make verify-status".to_string()],
        questions: Vec::new(),
        created_at: "2026-09-29T23:00:00Z".to_string(),
        consumed_by_node_id: None,
        consumed_at: None,
    };
    store.save_rework_instruction(&coded, &instruction).unwrap();
    let rerun_attempt = store
        .increment_attempt_rework_count(&coded.project_id, &coded.issue_id, &coded.id)
        .unwrap();
    let rerun_attempt = store
        .update_attempt_stage(
            &rerun_attempt.project_id,
            &rerun_attempt.issue_id,
            &rerun_attempt.id,
            CodingExecutionStage::Coding,
        )
        .unwrap();

    let restart = super::provider_execution_context::CapturingProjectionProvider::new(
        super::provider_execution_context::current_plan_defect_output(),
    );
    let updated = engine
        .execute_coding(&rerun_attempt, &restart, &CodingExecutionContext::default())
        .await
        .expect("rerun restart must not hit frozen-context identity mismatch");

    // 冻结哈希不因指令演化改变（绑定渲染上下文稳定）。
    let run_after = store.get_active_unit_run(&updated).unwrap();
    assert_eq!(
        run_after.coder_execution_context_hash.as_deref(),
        Some(frozen_hash.as_str())
    );
    // 指令全文（含字面命令）进入实际 prompt。
    let input = restart.input();
    assert!(
        input.prompt.contains("make verify-status"),
        "rerun fix hints must reach the actual prompt"
    );
    // Task 7 认领事务已消费指令（无残留未消费指令→不再自撞）。
    let instructions = store
        .list_rework_instructions(&updated.project_id, &updated.issue_id, &updated.id)
        .unwrap();
    assert!(
        instructions
            .iter()
            .all(|instruction| instruction.consumed_at.is_some()),
        "rerun instruction must be consumed by the restart coding run"
    );
}
