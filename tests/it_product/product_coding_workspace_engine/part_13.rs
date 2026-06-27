fn group_attempt_waiting_for_final_confirm_with_scoped_work_items() -> (
    tempfile::TempDir,
    ProductAppPaths,
    CodingAttemptStore,
    CodingWorkspaceEngine,
    CodingExecutionAttempt,
) {
    let (root, paths, store, engine, attempt) = group_engine_with_last_running_unit();
    let lifecycle = LifecycleStore::new(paths.clone());
    let shared_worktree = paths.root().join("shared-worktree");
    std::fs::create_dir_all(&shared_worktree).expect("create shared worktree dir");
    std::fs::create_dir_all(shared_worktree.join("src")).expect("create src dir");
    std::fs::create_dir_all(shared_worktree.join("web/src")).expect("create web src dir");
    std::fs::write(shared_worktree.join("src/backend.rs"), "// backend\n")
        .expect("write backend file");
    std::fs::write(shared_worktree.join("src/frontend.rs"), "// frontend\n")
        .expect("write frontend file");
    std::fs::write(shared_worktree.join("web/src/app.tsx"), "// app\n")
        .expect("write invalid app file");
    Command::new("git")
        .arg("init")
        .current_dir(&shared_worktree)
        .output()
        .expect("git init shared worktree");
    for (work_item_id, scope) in [
        ("work_item_0001", "src/backend.rs"),
        ("work_item_0002", "src/frontend.rs"),
    ] {
        lifecycle
            .create_work_item(CreateWorkItemInput {
                id: Some(work_item_id.to_string()),
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                repository_id: "repository_0001".to_string(),
                story_spec_ids: Vec::new(),
                design_spec_ids: Vec::new(),
                title: format!("title for {work_item_id}"),
                exclusive_write_scopes: vec![scope.to_string()],
                forbidden_write_scopes: Vec::new(),
                ..Default::default()
            })
            .expect("create scoped work item");
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
                files_changed: vec!["src/backend.rs".to_string()],
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
                files_changed: vec!["src/frontend.rs".to_string()],
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
            worktree_path: shared_worktree,
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
        .try_acquire_issue_worktree_lock("project_0001", "issue_0001", "work_item_0001")
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

#[tokio::test]
async fn group_internal_review_prompt_includes_all_unit_handoffs() {
    let (_root, _paths, _store, engine, attempt) = completed_group_attempt_with_handoffs();

    let prompt = engine
        .build_group_internal_pr_review_prompt_for_test(&attempt)
        .await
        .expect("prompt");

    assert!(prompt.contains("work_item_0001"));
    assert!(prompt.contains("work_item_0002"));
    assert!(prompt.contains("handoff summary for backend"));
    assert!(prompt.contains("handoff summary for frontend"));
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
async fn group_final_confirm_rejects_unit_handoff_outside_exclusive_scope() {
    let (_root, _paths, store, engine, attempt) =
        group_attempt_waiting_for_final_confirm_with_scoped_work_items();
    store
        .save_coding_unit_handoff(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            "coding_unit_0002",
            &WorkItemHandoff {
                files_changed: vec!["web/src/app.tsx".to_string()],
                ..store
                    .get_coding_unit_handoff(
                        &attempt.project_id,
                        &attempt.issue_id,
                        &attempt.id,
                        "coding_unit_0002",
                    )
                    .expect("get unit2 handoff")
                    .expect("existing unit2 handoff")
            },
        )
        .expect("overwrite unit2 handoff");

    let error = engine
        .handle_final_confirm(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .await
        .expect_err("group final confirm should fail");

    match error {
        cadence_aria::product::coding_workspace_engine::CodingWorkspaceEngineError::WorkItemDiffScopeViolation(path) => {
            assert_eq!(path, "web/src/app.tsx");
        }
        other => panic!("expected diff scope violation, got {other:?}"),
    }
}

#[tokio::test]
async fn group_final_confirm_requires_testing_report_for_each_unit_plan() {
    let (_root, paths, store, engine, attempt) = group_engine_with_last_running_unit();
    let lifecycle = LifecycleStore::new(paths.clone());
    for (work_item_id, plan_id) in [
        ("work_item_0001", "verification_plan_0001"),
        ("work_item_0002", "verification_plan_0002"),
    ] {
        create_required_verification_plan(&lifecycle, work_item_id, plan_id);
        lifecycle
            .create_work_item(CreateWorkItemInput {
                id: Some(work_item_id.to_string()),
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                repository_id: "repository_0001".to_string(),
                story_spec_ids: Vec::new(),
                design_spec_ids: Vec::new(),
                title: format!("title for {work_item_id}"),
                verification_plan_ref: Some(plan_id.to_string()),
                ..Default::default()
            })
            .expect("create work item");
    }
    for (unit_id, work_item_id) in [
        ("coding_unit_0001", "work_item_0001"),
        ("coding_unit_0002", "work_item_0002"),
    ] {
        save_minimal_unit_handoff(&store, &attempt, unit_id, work_item_id);
        store
            .update_coding_unit_handoff_ref(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                unit_id,
                Some(format!("units/{unit_id}/work-item-handoff.json")),
            )
            .expect("set handoff ref");
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
    store
        .save_testing_report(&passed_testing_report_for_plan(
            &attempt.id,
            "testing_report_0001",
            "verification_plan_0001",
        ))
        .expect("save unit1 testing report");
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

    let error = engine
        .handle_final_confirm(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .await
        .expect_err("unit2 has no matching testing report");

    match error {
        cadence_aria::product::coding_workspace_engine::CodingWorkspaceEngineError::VerificationGateResultMissing(attempt_id) => {
            assert_eq!(attempt_id, attempt.id);
        }
        other => panic!("expected missing verification gate result, got {other:?}"),
    }
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

fn create_required_verification_plan(
    lifecycle: &LifecycleStore,
    work_item_id: &str,
    plan_id: &str,
) {
    lifecycle
        .create_verification_plan(CreateVerificationPlanInput {
            id: Some(plan_id.to_string()),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: work_item_id.to_string(),
            repository_profile_ref: None,
            provider_run_ref: None,
            scope: VerificationScope::Unit,
            commands: vec![VerificationCommand {
                id: "unit_tests".to_string(),
                label: "Unit tests".to_string(),
                command: "cargo test --locked --lib unit".to_string(),
                cwd: ".".to_string(),
                purpose: "unit tests".to_string(),
                required: true,
                timeout_seconds: 120,
                source: VerificationCommandSource::Provider,
                safety: VerificationCommandSafety::Approved,
            }],
            manual_checks: Vec::new(),
            required_gates: vec!["unit_tests".to_string()],
            risk_notes: Vec::new(),
            confidence: RepositoryProfileConfidence::High,
            fallback_policy: VerificationFallbackPolicy::ManualGate,
        })
        .expect("create verification plan");
}

fn save_minimal_unit_handoff(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    unit_id: &str,
    work_item_id: &str,
) {
    store
        .save_coding_unit_handoff(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            unit_id,
            &WorkItemHandoff {
                id: format!("work_item_handoff_{unit_id}"),
                project_id: attempt.project_id.clone(),
                issue_id: attempt.issue_id.clone(),
                work_item_id: work_item_id.to_string(),
                attempt_id: attempt.id.clone(),
                provider_run_ref: None,
                summary: format!("handoff summary for {work_item_id}"),
                files_changed: Vec::new(),
                commit_sha: Some(format!("{work_item_id}-sha")),
                diff_summary: String::new(),
                tests_run: vec!["cargo test --locked --lib unit".to_string()],
                test_result_summary: "passed".to_string(),
                review_summary: None,
                api_or_contract_changes: Vec::new(),
                open_risks: Vec::new(),
                next_work_item_notes: Vec::new(),
                created_at: "2026-06-27T00:00:00Z".to_string(),
            },
        )
        .expect("save unit handoff");
}

fn passed_testing_report_for_plan(
    attempt_id: &str,
    report_id: &str,
    plan_id: &str,
) -> TestingReport {
    TestingReport {
        id: report_id.to_string(),
        attempt_id: attempt_id.to_string(),
        role_run_id: None,
        run_no: None,
        commands: vec![TestCommand {
            command: vec!["cargo".to_string(), "test".to_string()],
            cwd: PathBuf::from("/tmp/worktree"),
            exit_code: Some(0),
            duration_ms: 100,
            stdout_ref: "artifacts/stdout.txt".to_string(),
            stderr_ref: "artifacts/stderr.txt".to_string(),
            status: TestCommandStatus::Passed,
        }],
        overall_status: TestingOverallStatus::Passed,
        provider_claim: None,
        backend_verified: true,
        started_at: "2026-06-27T00:00:00Z".to_string(),
        completed_at: Some("2026-06-27T00:01:00Z".to_string()),
        plan_id: Some(plan_id.to_string()),
        plan_summary: None,
        steps: Vec::new(),
        unplanned_commands: Vec::new(),
        unplanned_evidence: Vec::new(),
        missing_required_steps: Vec::new(),
        skipped_required_steps: Vec::new(),
        context_warnings: Vec::new(),
        raw_provider_output_ref: None,
    }
}
