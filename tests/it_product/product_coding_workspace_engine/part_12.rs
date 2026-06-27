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

fn completed_group_attempt_with_handoffs() -> (
    tempfile::TempDir,
    ProductAppPaths,
    CodingAttemptStore,
    CodingWorkspaceEngine,
    CodingExecutionAttempt,
) {
    let (root, paths, store, engine, attempt) = group_engine_with_last_running_unit();
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
        lifecycle
            .create_workspace_session(CreateWorkspaceSessionInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                entity_id: work_item_id.to_string(),
                workspace_type: WorkspaceType::WorkItem,
                author_provider: ProviderName::Fake,
                reviewer_provider: ProviderName::Fake,
                review_rounds: 1,
                superpowers_enabled: true,
                openspec_enabled: true,
            })
            .expect("create workspace session");
    }
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
                summary: "handoff summary for backend".to_string(),
                files_changed: vec!["src/backend.rs".to_string()],
                commit_sha: Some("backend-sha".to_string()),
                diff_summary: "backend diff".to_string(),
                tests_run: vec!["cargo test --locked --lib backend".to_string()],
                test_result_summary: "passed".to_string(),
                review_summary: Some("backend review summary".to_string()),
                api_or_contract_changes: vec!["POST /api/backend".to_string()],
                open_risks: vec!["backend risk".to_string()],
                next_work_item_notes: vec!["backend note".to_string()],
                created_at: "2026-06-27T00:00:00Z".to_string(),
            },
        )
        .expect("save unit1 handoff");
    store
        .update_coding_unit_handoff_ref(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            "coding_unit_0001",
            Some("units/coding_unit_0001/work-item-handoff.json".to_string()),
        )
        .expect("set unit1 handoff ref");
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
                summary: "handoff summary for frontend".to_string(),
                files_changed: vec!["src/frontend.rs".to_string()],
                commit_sha: Some("frontend-sha".to_string()),
                diff_summary: "frontend diff".to_string(),
                tests_run: vec!["cargo test --locked --lib frontend".to_string()],
                test_result_summary: "passed".to_string(),
                review_summary: Some("frontend review summary".to_string()),
                api_or_contract_changes: vec!["GET /api/frontend".to_string()],
                open_risks: vec!["frontend risk".to_string()],
                next_work_item_notes: vec!["frontend note".to_string()],
                created_at: "2026-06-27T00:00:00Z".to_string(),
            },
        )
        .expect("save unit2 handoff");
    store
        .update_coding_unit_handoff_ref(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            "coding_unit_0002",
            Some("units/coding_unit_0002/work-item-handoff.json".to_string()),
        )
        .expect("set unit2 handoff ref");
    store
        .update_coding_unit_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            "coding_unit_0002",
            CodingExecutionUnitStatus::Completed,
            Some("frontend done".to_string()),
        )
        .expect("complete last unit");
    store
        .save_review_request(&sample_review_request(&attempt.id))
        .expect("save review request");
    let attempt = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("updated attempt");
    (root, paths, store, engine, attempt)
}

fn group_attempt_waiting_for_final_confirm() -> (
    tempfile::TempDir,
    ProductAppPaths,
    CodingAttemptStore,
    CodingWorkspaceEngine,
    CodingExecutionAttempt,
) {
    let (root, paths, store, engine, attempt) = group_engine_with_last_running_unit();
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
        lifecycle
            .update_work_item_execution_status(
                "project_0001",
                "issue_0001",
                work_item_id,
                WorkItemStatus::Coding,
            )
            .expect("set coding status");
    }
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
                summary: "handoff summary for backend".to_string(),
                files_changed: Vec::new(),
                commit_sha: Some("backend-sha".to_string()),
                diff_summary: String::new(),
                tests_run: vec!["cargo test --locked --lib backend".to_string()],
                test_result_summary: "passed".to_string(),
                review_summary: None,
                api_or_contract_changes: Vec::new(),
                open_risks: vec!["backend risk".to_string()],
                next_work_item_notes: Vec::new(),
                created_at: "2026-06-27T00:00:00Z".to_string(),
            },
        )
        .expect("save unit1 handoff");
    store
        .update_coding_unit_handoff_ref(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            "coding_unit_0001",
            Some("units/coding_unit_0001/work-item-handoff.json".to_string()),
        )
        .expect("set unit1 handoff ref");
    store
        .update_coding_unit_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            "coding_unit_0002",
            CodingExecutionUnitStatus::Completed,
            Some("frontend done".to_string()),
        )
        .expect("complete last unit");
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
                summary: "handoff summary for frontend".to_string(),
                files_changed: Vec::new(),
                commit_sha: Some("frontend-sha".to_string()),
                diff_summary: String::new(),
                tests_run: vec!["cargo test --locked --lib frontend".to_string()],
                test_result_summary: "passed".to_string(),
                review_summary: None,
                api_or_contract_changes: Vec::new(),
                open_risks: vec!["frontend risk".to_string()],
                next_work_item_notes: Vec::new(),
                created_at: "2026-06-27T00:00:00Z".to_string(),
            },
        )
        .expect("save unit2 handoff");
    store
        .update_coding_unit_handoff_ref(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            "coding_unit_0002",
            Some("units/coding_unit_0002/work-item-handoff.json".to_string()),
        )
        .expect("set unit2 handoff ref");
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
        .try_acquire_issue_worktree_lock("project_0001", "issue_0001", "work_item_0002")
        .expect("acquire shared worktree lock");
    store
        .save_timeline_node(CodingTimelineNode {
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
    (root, paths, store, engine, attempt)
}
