#[tokio::test]
async fn completing_group_unit_marks_current_unit_completed_and_next_running() {
    let (_root, paths, store, engine, attempt) = group_engine_with_two_units();

    let updated = engine
        .complete_current_group_unit(&attempt, Some("unit handoff saved".to_string()))
        .await
        .expect("complete unit");

    let units = store
        .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("units");
    assert_eq!(updated.scope, CodingAttemptScope::WorkItemGroup);
    assert_eq!(updated.stage, CodingExecutionStage::PrepareContext);
    assert_eq!(updated.status, CodingAttemptStatus::Running);
    assert_eq!(updated.current_work_item_id.as_deref(), Some("work_item_0002"));
    assert_eq!(updated.active_unit_id.as_deref(), Some("coding_unit_0002"));
    assert_eq!(units[0].status, CodingExecutionUnitStatus::Completed);
    assert_eq!(units[0].summary.as_deref(), Some("unit handoff saved"));
    assert_eq!(units[1].status, CodingExecutionUnitStatus::Running);
    assert_eq!(units[1].summary.as_deref(), Some("进入下一个 Work Item"));
    assert!(paths.root().exists());
}

#[tokio::test]
async fn completing_group_unit_moves_issue_shared_lock_to_next_unit() {
    let (_root, paths, _store, engine, attempt) = group_engine_with_two_units();
    let lifecycle = LifecycleStore::new(paths.clone());
    lifecycle
        .upsert_issue_shared_worktree(UpsertIssueSharedWorktreeInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: paths.root().join("shared-worktree"),
            base_branch: "HEAD".to_string(),
        })
        .expect("upsert shared worktree");
    lifecycle
        .try_acquire_issue_worktree_lock(
            "project_0001",
            "issue_0001",
            "work_item_0001",
            &attempt.id,
        )
        .expect("acquire shared lock for first unit");

    engine
        .complete_current_group_unit(&attempt, Some("unit handoff saved".to_string()))
        .await
        .expect("complete unit");

    let shared = lifecycle
        .get_issue_shared_worktree("project_0001", "issue_0001")
        .expect("shared worktree")
        .expect("existing shared worktree");
    assert_eq!(
        shared.current_active_work_item_id.as_deref(),
        Some("work_item_0002")
    );
}

#[tokio::test]
async fn completing_last_group_unit_enters_review_request_stage() {
    let (_root, _paths, store, engine, attempt) = group_engine_with_last_running_unit();

    let updated = engine
        .complete_current_group_unit(&attempt, Some("last unit done".to_string()))
        .await
        .expect("complete last unit");

    assert_eq!(updated.scope, CodingAttemptScope::WorkItemGroup);
    assert_eq!(updated.stage, CodingExecutionStage::ReviewRequest);
    assert!(engine
        .group_attempt_ready_for_final_review(&updated)
        .expect("ready"));
    assert!(store
        .list_coding_units(&updated.project_id, &updated.issue_id, &updated.id)
        .expect("units")
        .iter()
        .all(|unit| unit.status == CodingExecutionUnitStatus::Completed));
}

#[tokio::test]
async fn completing_group_units_publishes_revisions_without_legacy_handoff_artifacts() {
    let (_root, paths, store, engine, attempt) = group_engine_with_two_units();
    seed_authoritative_group_terminal_fixture(&store, &attempt);
    create_active_coding_unit_run(&store, &attempt);

    let after_first = engine
        .complete_group_unit_after_code_review(&attempt)
        .await
        .expect("complete first unit");

    create_active_coding_unit_run(&store, &after_first);
    let after_second = engine
        .complete_group_unit_after_code_review(&after_first)
        .await
        .expect("complete second unit");
    let units = store
        .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("units");
    let revision_store = WorkItemRevisionStore::new(store.paths());
    let lineage = revision_store
        .get_plan_lineage(
            &attempt.project_id,
            &attempt.issue_id,
            "work_item_plan_0001",
        )
        .expect("plan lineage");
    let unit1_handoff_id = units[0]
        .latest_handoff_revision_id
        .as_deref()
        .expect("unit1 handoff revision id");
    let unit2_handoff_id = units[1]
        .latest_handoff_revision_id
        .as_deref()
        .expect("unit2 handoff revision id");
    let unit1_handoff = revision_store
        .get_handoff_revision(&lineage, "work_item_0001", unit1_handoff_id)
        .expect("unit1 handoff revision");
    let unit2_handoff = revision_store
        .get_handoff_revision(&lineage, "work_item_0002", unit2_handoff_id)
        .expect("unit2 handoff revision");

    assert_eq!(after_second.stage, CodingExecutionStage::ReviewRequest);
    assert_eq!(unit1_handoff.logical_work_item_id, "work_item_0001");
    assert_eq!(unit2_handoff.logical_work_item_id, "work_item_0002");
    assert_eq!(
        Some(unit1_handoff_id),
        Some("handoff_revision_coding_unit_run_coding_unit_0001")
    );
    assert_eq!(
        Some(unit2_handoff_id),
        Some("handoff_revision_coding_unit_run_coding_unit_0002")
    );
    assert_ne!(unit1_handoff.id, unit2_handoff.id);

    let attempt_root = paths
        .issue_lifecycle_root(&attempt.project_id, &attempt.issue_id)
        .join("coding-attempts")
        .join(&attempt.id);
    assert!(
        !attempt_root.join("work-item-handoff.json").exists(),
        "unit 完成不得生成 attempt 级交接摘要；旧 provider/占位路径会写入该文件"
    );
    for unit in &units {
        assert!(
            !attempt_root
                .join("units")
                .join(&unit.id)
                .join("work-item-handoff.json")
                .exists(),
            "unit 完成不得生成 unit 级交接摘要；旧 provider/占位路径会写入该文件"
        );
    }
}

#[tokio::test]
async fn group_final_review_prompt_includes_all_unit_handoffs() {
    let (_root, _paths, _store, engine, attempt) =
        completed_group_attempt_with_handoff_revisions();

    let prompt = engine
        .build_group_internal_pr_review_prompt_for_test(&attempt)
        .await
        .expect("prompt");

    assert!(prompt.contains("Coding Workspace GroupFinalReview"));
    assert!(prompt.contains("WorkItemGroup GroupFinalReview"));
    assert!(prompt.contains("source_stage=group_final_review"));
    assert!(!prompt.contains("Coding Workspace InternalReviewer"));
    assert!(prompt.contains("work_item_0001"));
    assert!(prompt.contains("work_item_0002"));
    assert!(prompt.contains("Handoff Revision"));
    assert!(prompt.contains("Provided Contracts"));
    assert!(prompt.contains("Provided Capabilities"));
    assert!(prompt.contains("HandoffRevision 契约与能力汇总"));
    assert!(!prompt.contains("completed units 的 handoff 汇总"));
    assert!(!prompt.contains("Handoff Summary"));
    assert!(!prompt.contains("Tests Run"));
    assert!(prompt.contains("Reviewer 非 E2E 测试边界"));
    assert!(prompt.contains("Playwright"));
    assert!(prompt.contains("单元测试"));
    assert!(prompt.contains("不受 Verification Plan 已列命令的严格限制"));
    assert!(prompt.contains(
        "上述测试及其所需浏览器环境的安装、配置、缺失、失败或相关证据（包括缺少证据）"
    ));
    assert!(prompt.contains("均不得成为 finding，也不得导致 request_changes 或 blocked"));
    assert!(prompt.contains("不得作为 verdict 或 summary 中的否决理由"));
    assert!(prompt.contains("不得成为 Coder required_action 或任何返修要求"));
    assert!(prompt.contains(
        "即使 Work Item、Design Spec、Verification Plan、handoff 或 EvaluationContextPack 提到上述测试及其所需浏览器环境"
    ));
}

#[tokio::test]
async fn group_final_review_prompt_rejects_dangling_published_handoff_revision() {
    let (_root, _paths, store, engine, attempt) =
        completed_group_attempt_with_handoff_revisions();
    let units = store
        .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("completed units");
    let final_unit = units
        .iter()
        .max_by_key(|unit| unit.order_index)
        .expect("final completed unit");
    store
        .update_coding_unit_latest_handoff_revision_id(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &final_unit.id,
            Some("handoff_revision_missing".to_string()),
        )
        .expect("set dangling published handoff revision id");

    let error = engine
        .build_group_internal_pr_review_prompt_for_test(&attempt)
        .await
        .expect_err("published handoff revision read failures must not be rendered as absent");

    assert!(
        error.to_string().contains("handoff_revision_missing"),
        "已有 revision id 但记录缺失时必须失败关闭，实际: {error:?}"
    );
}

#[tokio::test]
async fn group_final_confirm_completes_attempt_after_all_units_completed() {
    let (_root, _paths, store, engine, attempt) = group_attempt_waiting_for_final_confirm();

    let updated = engine
        .handle_final_confirm(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .await
        .expect("final confirm");

    assert_eq!(updated.status, CodingAttemptStatus::Completed);
    assert_eq!(updated.scope, CodingAttemptScope::WorkItemGroup);
    assert!(
        store
            .list_coding_units(&updated.project_id, &updated.issue_id, &updated.id)
            .expect("units")
            .iter()
            .all(|unit| unit.status == CodingExecutionUnitStatus::Completed)
    );
    let shared = LifecycleStore::new(store.paths())
        .get_issue_shared_worktree(&updated.project_id, &updated.issue_id)
        .expect("shared worktree")
        .expect("existing shared worktree");
    assert_eq!(shared.current_active_work_item_id, None);
    assert_eq!(
        shared.last_completed_work_item_id.as_deref(),
        Some("work_item_0002")
    );
}

#[tokio::test]
async fn group_final_confirm_without_authoritative_plan_binding_fails_closed() {
    let (_root, paths, store, engine, attempt) = group_engine_with_last_running_unit();
    let lifecycle = LifecycleStore::new(paths.clone());
    let worktree = attempt
        .worktree_path
        .as_ref()
        .expect("group attempt worktree path");
    let completion_commit = git_head(worktree);
    for work_item_id in ["work_item_0001", "work_item_0002"] {
        lifecycle
            .create_work_item(CreateWorkItemInput {
                id: Some(work_item_id.to_string()),
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                repository_id: "repository_0001".to_string(),
                story_spec_ids: Vec::new(),
                design_spec_ids: Vec::new(),
                title: format!("title for {work_item_id}"),
                ..Default::default()
            })
            .expect("create work item");
    }
    for unit_id in ["coding_unit_0001", "coding_unit_0002"] {
        store
            .update_coding_unit_latest_handoff_revision_id(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                unit_id,
                Some(format!("handoff_revision_{unit_id}")),
            )
            .expect("set handoff ref");
        store
            .update_coding_unit_completion_commit(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                unit_id,
                Some(completion_commit.clone()),
            )
            .expect("set completion commit");
    }
    store
        .update_coding_unit_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            "coding_unit_0002",
            CodingExecutionUnitStatus::Completed,
            Some("frontend done".to_string()),
        )
        .expect("complete unit2");
    for unit_id in ["coding_unit_0001", "coding_unit_0002"] {
        create_completed_unit_run_for_test(
            &store,
            &attempt,
            unit_id,
            &completion_commit,
            &completion_commit,
        );
    }
    let attempt = store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("set running");
    let attempt = store
        .update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::FinalConfirm,
        )
        .expect("final confirm stage");
    let attempt = store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::WaitingForHuman,
        )
        .expect("waiting for human");
    let attempt = store
        .update_attempt_head_commit(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            Some("deadbeef".to_string()),
        )
        .expect("set head commit");
    lifecycle
        .upsert_issue_shared_worktree(UpsertIssueSharedWorktreeInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: paths.root().join("shared-worktree"),
            base_branch: "HEAD".to_string(),
        })
        .expect("upsert shared worktree");
    lifecycle
        .try_acquire_issue_worktree_lock(
            "project_0001",
            "issue_0001",
            "work_item_0002",
            &attempt.id,
        )
        .expect("acquire shared worktree lock");
    store
        .save_timeline_node(&attempt, CodingTimelineNode {
            id: "coding_node_0001".to_string(),
            attempt_id: attempt.id.clone(),
            stage: CodingExecutionStage::FinalConfirm,
            title: "最终确认".to_string(),
            status: CodingTimelineNodeStatus::Running,
            agent_role: Some(CodingAgentRole::System),
            summary: None,
            started_at: "2026-06-27T00:00:00Z".to_string(),
            completed_at: None,
            artifact_refs: Vec::new(),
        })
        .expect("save final confirm node");
    assert!(matches!(
        store.get_plan_binding(&attempt),
        Err(cadence_aria::product::json_store::ProductStoreError::NotFound {
            kind: "coding_attempt_plan_binding",
            ..
        })
    ));

    let error = engine
        .handle_final_confirm(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .await
        .expect_err("legacy group without authoritative binding must fail closed");
    assert!(matches!(
        error,
        cadence_aria::product::coding_workspace_engine::CodingWorkspaceEngineError::FinalConfirmNotReady(ref id)
            if id == &attempt.id
    ));

    let persisted = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("persisted attempt");
    assert_eq!(persisted.status, CodingAttemptStatus::WaitingForHuman);
    assert!(persisted.completed_at.is_none());
}

#[tokio::test]
async fn group_final_confirm_rejects_when_any_unit_not_completed() {
    let (_root, paths, store, engine, attempt) = group_engine_with_last_running_unit();
    let lifecycle = LifecycleStore::new(paths.clone());
    for work_item_id in ["work_item_0001", "work_item_0002"] {
        lifecycle
            .create_work_item(CreateWorkItemInput {
                id: Some(work_item_id.to_string()),
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                repository_id: "repository_0001".to_string(),
                story_spec_ids: Vec::new(),
                design_spec_ids: Vec::new(),
                title: format!("title for {work_item_id}"),
                ..Default::default()
            })
            .expect("create work item");
    }
    let attempt = store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("set running");
    let attempt = store
        .update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::FinalConfirm,
        )
        .expect("set final confirm");
    let attempt = store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::WaitingForHuman,
        )
        .expect("set waiting");
    let error = engine
        .handle_final_confirm(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .await
        .expect_err("group final confirm should reject incomplete units");

    assert!(matches!(
        error,
        cadence_aria::product::coding_workspace_engine::CodingWorkspaceEngineError::FinalConfirmNotReady(id)
            if id == attempt.id
    ));
}
