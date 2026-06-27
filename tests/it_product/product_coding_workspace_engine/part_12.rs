fn group_engine_with_two_units() -> (
    tempfile::TempDir,
    ProductAppPaths,
    CodingAttemptStore,
    CodingWorkspaceEngine,
    CodingExecutionAttempt,
) {
    let root = tempdir().expect("root");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let store = CodingAttemptStore::new(paths.clone());
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
            },
            max_auto_rework: 2,
        })
        .expect("create group attempt");
    store
        .create_coding_unit(CreateCodingExecutionUnitInput {
            attempt_id: attempt.id.clone(),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            work_item_id: "work_item_0001".to_string(),
            order_index: 0,
            status: CodingExecutionUnitStatus::Running,
        })
        .expect("create coding unit 1");
    store
        .create_coding_unit(CreateCodingExecutionUnitInput {
            attempt_id: attempt.id.clone(),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            work_item_id: "work_item_0002".to_string(),
            order_index: 1,
            status: CodingExecutionUnitStatus::Pending,
        })
        .expect("create coding unit 2");
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    (root, paths, store, engine, attempt)
}

fn group_engine_with_last_running_unit() -> (
    tempfile::TempDir,
    ProductAppPaths,
    CodingAttemptStore,
    CodingWorkspaceEngine,
    CodingExecutionAttempt,
) {
    let root = tempdir().expect("root");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let store = CodingAttemptStore::new(paths.clone());
    let attempt = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_work_item_id: "work_item_0002".to_string(),
            base_branch: "HEAD".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Fake,
                reviewer: Some(ProviderName::Fake),
                review_rounds: 1,
            },
            max_auto_rework: 2,
        })
        .expect("create group attempt");
    store
        .create_coding_unit(CreateCodingExecutionUnitInput {
            attempt_id: attempt.id.clone(),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            work_item_id: "work_item_0001".to_string(),
            order_index: 0,
            status: CodingExecutionUnitStatus::Completed,
        })
        .expect("create coding unit 1");
    store
        .create_coding_unit(CreateCodingExecutionUnitInput {
            attempt_id: attempt.id.clone(),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            work_item_id: "work_item_0002".to_string(),
            order_index: 1,
            status: CodingExecutionUnitStatus::Running,
        })
        .expect("create coding unit 2");
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    (root, paths, store, engine, attempt)
}

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
async fn completing_group_units_saves_distinct_handoffs_per_unit() {
    let (_root, _paths, store, engine, attempt) = group_engine_with_two_units();

    let after_first = engine
        .complete_group_unit_after_code_review(&attempt)
        .await
        .expect("complete first unit");
    let unit1_handoff = store
        .get_coding_unit_handoff(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            "coding_unit_0001",
        )
        .expect("load unit1 handoff")
        .expect("unit1 handoff exists");
    assert_eq!(unit1_handoff.work_item_id, "work_item_0001");
    assert!(store
        .get_work_item_handoff(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("attempt handoff")
        .is_none());

    let after_second = engine
        .complete_group_unit_after_code_review(&after_first)
        .await
        .expect("complete second unit");
    let unit2_handoff = store
        .get_coding_unit_handoff(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            "coding_unit_0002",
        )
        .expect("load unit2 handoff")
        .expect("unit2 handoff exists");
    let units = store
        .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("units");

    assert_eq!(after_second.stage, CodingExecutionStage::ReviewRequest);
    assert_eq!(unit2_handoff.work_item_id, "work_item_0002");
    assert_eq!(
        units[0].handoff_ref.as_deref(),
        Some("units/coding_unit_0001/work-item-handoff.json")
    );
    assert_eq!(
        units[1].handoff_ref.as_deref(),
        Some("units/coding_unit_0002/work-item-handoff.json")
    );
    assert_ne!(unit1_handoff.work_item_id, unit2_handoff.work_item_id);
}

#[test]
fn group_visible_handoff_returns_last_completed_unit_when_no_active_unit_exists() {
    let (_root, _paths, store, _engine, attempt) = group_engine_with_last_running_unit();
    store
        .save_coding_unit_handoff(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            "coding_unit_0001",
            &WorkItemHandoff {
                id: "work_item_handoff_0001".to_string(),
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                work_item_id: "work_item_0001".to_string(),
                attempt_id: attempt.id.clone(),
                provider_run_ref: None,
                summary: "unit1".to_string(),
                files_changed: Vec::new(),
                commit_sha: None,
                diff_summary: String::new(),
                tests_run: Vec::new(),
                test_result_summary: String::new(),
                review_summary: None,
                api_or_contract_changes: Vec::new(),
                open_risks: Vec::new(),
                next_work_item_notes: Vec::new(),
                created_at: "2026-06-27T00:00:00Z".to_string(),
            },
        )
        .expect("save unit1 handoff");
    store
        .save_coding_unit_handoff(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            "coding_unit_0002",
            &WorkItemHandoff {
                id: "work_item_handoff_0002".to_string(),
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                work_item_id: "work_item_0002".to_string(),
                attempt_id: attempt.id.clone(),
                provider_run_ref: None,
                summary: "unit2".to_string(),
                files_changed: Vec::new(),
                commit_sha: None,
                diff_summary: String::new(),
                tests_run: Vec::new(),
                test_result_summary: String::new(),
                review_summary: None,
                api_or_contract_changes: Vec::new(),
                open_risks: Vec::new(),
                next_work_item_notes: Vec::new(),
                created_at: "2026-06-27T00:00:00Z".to_string(),
            },
        )
        .expect("save unit2 handoff");
    store
        .update_coding_unit_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            "coding_unit_0002",
            CodingExecutionUnitStatus::Completed,
            Some("done".to_string()),
        )
        .expect("complete last unit");
    let updated = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("updated");

    let visible = store
        .get_visible_work_item_handoff(&updated)
        .expect("visible handoff")
        .expect("last completed handoff exists");

    assert!(updated.active_unit_id.is_none());
    assert!(updated.current_work_item_id.is_none());
    assert_eq!(visible.work_item_id, "work_item_0002");
    assert_eq!(visible.summary, "unit2");
}
