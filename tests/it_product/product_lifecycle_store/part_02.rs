#[test]
fn rejects_lock_when_another_work_item_is_active() {
    let root = tempdir().expect("tempdir");
    let store = LifecycleStore::new(ProductAppPaths::new(root.path().join(".aria")));
    store
        .upsert_issue_shared_worktree(UpsertIssueSharedWorktreeInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: PathBuf::from("/tmp/repo/.worktrees/aria-issues/issue_0001"),
            base_branch: "main".to_string(),
        })
        .expect("shared worktree");
    store
        .try_acquire_issue_worktree_lock("project_0001", "issue_0001", "work_item_0001")
        .expect("first lock");

    let error = store
        .try_acquire_issue_worktree_lock("project_0001", "issue_0001", "work_item_0002")
        .expect_err("second lock should fail");

    assert!(format!("{error}").contains("issue_worktree_active"));
}

#[test]
fn marks_issue_shared_worktree_last_completed_work_item() {
    let root = tempdir().expect("tempdir");
    let store = LifecycleStore::new(ProductAppPaths::new(root.path().join(".aria")));
    store
        .upsert_issue_shared_worktree(UpsertIssueSharedWorktreeInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: PathBuf::from("/tmp/repo/.worktrees/aria-issues/issue_0001"),
            base_branch: "main".to_string(),
        })
        .expect("shared worktree");

    let updated = store
        .mark_issue_worktree_completed_item("project_0001", "issue_0001", "work_item_0001")
        .expect("mark completed");

    assert_eq!(
        updated.last_completed_work_item_id.as_deref(),
        Some("work_item_0001")
    );
}

#[test]
fn create_work_item_persists_split_fields() {
    let root = tempdir().expect("tempdir");
    let store = LifecycleStore::new(ProductAppPaths::new(root.path().join(".aria")));

    let work_item = store
        .create_work_item(CreateWorkItemInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            story_spec_ids: vec!["story_spec_0001".to_string()],
            design_spec_ids: vec!["design_spec_0001".to_string()],
            title: "后端 API".to_string(),
            work_item_set_id: Some("work_item_set_0001".to_string()),
            kind: WorkItemKind::Backend,
            sequence_hint: Some(10),
            depends_on: Vec::new(),
            exclusive_write_scopes: vec!["src/product/**".to_string()],
            forbidden_write_scopes: vec!["web/**".to_string()],
            context_budget: WorkItemContextBudget::default(),
            required_handoff_from: Vec::new(),
            verification_plan_ref: Some("verification_plan_work_item_0001".to_string()),
            require_execution_plan_confirm: false,
            id: None,
            plan_status: WorkItemPlanStatus::NotStarted,
        })
        .expect("work item");

    assert_eq!(
        work_item.work_item_set_id.as_deref(),
        Some("work_item_set_0001")
    );
    assert_eq!(work_item.kind, WorkItemKind::Backend);
    assert_eq!(work_item.exclusive_write_scopes, vec!["src/product/**"]);
}

#[test]
fn confirm_issue_work_item_plan_marks_work_items_confirmed() {
    let root = tempdir().expect("tempdir");
    let store = LifecycleStore::new(ProductAppPaths::new(root.path().join(".aria")));

    let work_item_a = store
        .create_work_item(CreateWorkItemInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            story_spec_ids: vec!["story_spec_0001".to_string()],
            design_spec_ids: vec!["design_spec_0001".to_string()],
            title: "后端 API".to_string(),
            kind: WorkItemKind::Backend,
            plan_status: WorkItemPlanStatus::Draft,
            ..Default::default()
        })
        .expect("work item a");
    let work_item_b = store
        .create_work_item(CreateWorkItemInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            story_spec_ids: vec!["story_spec_0001".to_string()],
            design_spec_ids: vec!["design_spec_0001".to_string()],
            title: "前端组件".to_string(),
            kind: WorkItemKind::Frontend,
            plan_status: WorkItemPlanStatus::Draft,
            ..Default::default()
        })
        .expect("work item b");

    let profile = store
        .create_repository_profile(CreateRepositoryProfileInput {
            id: Some("repository_profile_0001".to_string()),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            provider_run_ref: None,
            languages: vec!["rust".to_string()],
            frameworks: Vec::new(),
            package_managers: vec!["cargo".to_string()],
            test_frameworks: Vec::new(),
            build_systems: vec!["cargo".to_string()],
            verification_capabilities: vec!["cargo test".to_string()],
            detected_layers: vec!["backend".to_string(), "frontend".to_string()],
            split_recommendation: "frontend_backend".to_string(),
            confidence: RepositoryProfileConfidence::High,
            uncertainties: Vec::new(),
        })
        .expect("profile");

    let verification_plan_a = store
        .create_verification_plan(CreateVerificationPlanInput {
            id: Some("verification_plan_0001".to_string()),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: work_item_a.id.clone(),
            repository_profile_ref: Some(profile.id.clone()),
            provider_run_ref: None,
            scope: VerificationScope::Unit,
            commands: vec![VerificationCommand {
                id: "cmd_001".to_string(),
                label: "cargo test".to_string(),
                command: "cargo test --lib".to_string(),
                cwd: String::new(),
                purpose: "unit tests".to_string(),
                required: true,
                timeout_seconds: 120,
                source: VerificationCommandSource::Provider,
                safety: VerificationCommandSafety::Approved,
            }],
            manual_checks: Vec::new(),
            required_gates: Vec::new(),
            risk_notes: Vec::new(),
            confidence: RepositoryProfileConfidence::High,
            fallback_policy: VerificationFallbackPolicy::ManualGate,
        })
        .expect("verification plan a");
    let verification_plan_b = store
        .create_verification_plan(CreateVerificationPlanInput {
            id: Some("verification_plan_0002".to_string()),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: work_item_b.id.clone(),
            repository_profile_ref: Some(profile.id.clone()),
            provider_run_ref: None,
            scope: VerificationScope::Unit,
            commands: vec![VerificationCommand {
                id: "cmd_001".to_string(),
                label: "cargo test".to_string(),
                command: "cargo test --lib".to_string(),
                cwd: String::new(),
                purpose: "unit tests".to_string(),
                required: true,
                timeout_seconds: 120,
                source: VerificationCommandSource::Provider,
                safety: VerificationCommandSafety::Approved,
            }],
            manual_checks: Vec::new(),
            required_gates: Vec::new(),
            risk_notes: Vec::new(),
            confidence: RepositoryProfileConfidence::High,
            fallback_policy: VerificationFallbackPolicy::ManualGate,
        })
        .expect("verification plan b");

    let plan = store
        .create_issue_work_item_plan(CreateIssueWorkItemPlanInput {
            id: Some("issue_work_item_plan_0001".to_string()),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            source_story_spec_ids: vec!["story_spec_0001".to_string()],
            source_design_spec_ids: vec!["design_spec_0001".to_string()],
            options: IssueWorkItemPlanOptions {
                include_integration_tests: false,
                include_e2e_tests: false,
                force_frontend_backend_split: false,
                require_execution_plan_confirm: false,
            },
            status: IssueWorkItemPlanStatus::Draft,
            work_item_ids: vec![work_item_a.id.clone(), work_item_b.id.clone()],
            repository_profile_ref: Some(profile.id.clone()),
            verification_plan_ids: vec![
                verification_plan_a.id.clone(),
                verification_plan_b.id.clone(),
            ],
            dependency_graph: vec![IssueWorkItemDependencyEdge {
                from_work_item_id: work_item_a.id.clone(),
                to_work_item_id: work_item_b.id.clone(),
            }],
            created_from_provider_run: None,
            validator_findings: Vec::new(),
        })
        .expect("plan");

    let (confirmed_plan, confirmed_items) = store
        .confirm_issue_work_item_plan("project_0001", "issue_0001", &plan.id)
        .expect("confirm");

    assert_eq!(confirmed_plan.status, IssueWorkItemPlanStatus::Confirmed);
    assert_eq!(confirmed_items.len(), 2);
    assert!(
        confirmed_items
            .iter()
            .all(|item| item.plan_status == WorkItemPlanStatus::Confirmed)
    );
}

#[test]
fn request_change_keeps_split_work_items_not_codeable() {
    let root = tempdir().expect("tempdir");
    let store = LifecycleStore::new(ProductAppPaths::new(root.path().join(".aria")));

    let work_item = store
        .create_work_item(CreateWorkItemInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            story_spec_ids: vec!["story_spec_0001".to_string()],
            design_spec_ids: vec!["design_spec_0001".to_string()],
            title: "后端 API".to_string(),
            kind: WorkItemKind::Backend,
            plan_status: WorkItemPlanStatus::Draft,
            ..Default::default()
        })
        .expect("work item");

    let profile = store
        .create_repository_profile(CreateRepositoryProfileInput {
            id: Some("repository_profile_0001".to_string()),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            provider_run_ref: None,
            languages: vec!["rust".to_string()],
            frameworks: Vec::new(),
            package_managers: vec!["cargo".to_string()],
            test_frameworks: Vec::new(),
            build_systems: vec!["cargo".to_string()],
            verification_capabilities: vec!["cargo test".to_string()],
            detected_layers: vec!["backend".to_string()],
            split_recommendation: "backend".to_string(),
            confidence: RepositoryProfileConfidence::High,
            uncertainties: Vec::new(),
        })
        .expect("profile");

    let verification_plan = store
        .create_verification_plan(CreateVerificationPlanInput {
            id: Some("verification_plan_0001".to_string()),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: work_item.id.clone(),
            repository_profile_ref: Some(profile.id.clone()),
            provider_run_ref: None,
            scope: VerificationScope::Unit,
            commands: vec![VerificationCommand {
                id: "cmd_001".to_string(),
                label: "cargo test".to_string(),
                command: "cargo test --lib".to_string(),
                cwd: String::new(),
                purpose: "unit tests".to_string(),
                required: true,
                timeout_seconds: 120,
                source: VerificationCommandSource::Provider,
                safety: VerificationCommandSafety::Approved,
            }],
            manual_checks: Vec::new(),
            required_gates: Vec::new(),
            risk_notes: Vec::new(),
            confidence: RepositoryProfileConfidence::High,
            fallback_policy: VerificationFallbackPolicy::ManualGate,
        })
        .expect("verification plan");

    let plan = store
        .create_issue_work_item_plan(CreateIssueWorkItemPlanInput {
            id: Some("issue_work_item_plan_0001".to_string()),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            source_story_spec_ids: vec!["story_spec_0001".to_string()],
            source_design_spec_ids: vec!["design_spec_0001".to_string()],
            options: IssueWorkItemPlanOptions {
                include_integration_tests: false,
                include_e2e_tests: false,
                force_frontend_backend_split: false,
                require_execution_plan_confirm: false,
            },
            status: IssueWorkItemPlanStatus::Draft,
            work_item_ids: vec![work_item.id.clone()],
            repository_profile_ref: Some(profile.id.clone()),
            verification_plan_ids: vec![verification_plan.id.clone()],
            dependency_graph: Vec::new(),
            created_from_provider_run: None,
            validator_findings: Vec::new(),
        })
        .expect("plan");

    let (changed_plan, changed_items) = store
        .request_issue_work_item_plan_change(
            "project_0001",
            "issue_0001",
            &plan.id,
            Some("需要补充细节".to_string()),
        )
        .expect("change request");

    assert_eq!(
        changed_plan.status,
        IssueWorkItemPlanStatus::ChangeRequested
    );
    assert_eq!(changed_items.len(), 1);
    assert_eq!(changed_items[0].plan_status, WorkItemPlanStatus::Draft);
}

fn new_split_output_with_ids(
    plan_id: &str,
    profile_id: &str,
    work_item_ids: &[&str],
    verification_plan_ids: &[&str],
) -> WorkItemSplitProviderOutput {
    WorkItemSplitProviderOutput {
        repository_profile: RepositoryProfile {
            id: profile_id.to_string(),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            provider_run_ref: None,
            languages: vec!["rust".to_string()],
            frameworks: Vec::new(),
            package_managers: vec!["cargo".to_string()],
            test_frameworks: Vec::new(),
            build_systems: vec!["cargo".to_string()],
            verification_capabilities: vec!["cargo test".to_string()],
            detected_layers: vec!["backend".to_string()],
            split_recommendation: "backend".to_string(),
            confidence: RepositoryProfileConfidence::High,
            uncertainties: Vec::new(),
            created_at: "2026-06-17T00:00:00Z".to_string(),
            updated_at: "2026-06-17T00:00:00Z".to_string(),
        },
        plan: cadence_aria::product::models::IssueWorkItemPlan {
            id: plan_id.to_string(),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            source_story_spec_ids: vec!["story_spec_0001".to_string()],
            source_design_spec_ids: vec!["design_spec_0001".to_string()],
            options: IssueWorkItemPlanOptions {
                include_integration_tests: false,
                include_e2e_tests: false,
                force_frontend_backend_split: false,
                require_execution_plan_confirm: false,
            },
            status: IssueWorkItemPlanStatus::Draft,
            work_item_ids: work_item_ids.iter().map(|s| s.to_string()).collect(),
            repository_profile_ref: Some(profile_id.to_string()),
            verification_plan_ids: verification_plan_ids
                .iter()
                .map(|s| s.to_string())
                .collect(),
            dependency_graph: vec![IssueWorkItemDependencyEdge {
                from_work_item_id: work_item_ids[0].to_string(),
                to_work_item_id: work_item_ids[1].to_string(),
            }],
            created_from_provider_run: Some("provider_run_split_0001".to_string()),
            validator_findings: Vec::new(),
            review_summary: None,
            created_at: "2026-06-17T00:00:00Z".to_string(),
            updated_at: "2026-06-17T00:00:00Z".to_string(),
        },
        work_items: work_item_ids
            .iter()
            .enumerate()
            .map(
                |(index, id)| cadence_aria::product::models::LifecycleWorkItemRecord {
                    id: id.to_string(),
                    project_id: "project_0001".to_string(),
                    issue_id: "issue_0001".to_string(),
                    repository_id: "repository_0001".to_string(),
                    story_spec_ids: vec!["story_spec_0001".to_string()],
                    design_spec_ids: vec!["design_spec_0001".to_string()],
                    title: format!("new work item {}", index + 1),
                    plan_status: WorkItemPlanStatus::Draft,
                    execution_status: WorkItemStatus::Pending,
                    worktree_path: None,
                    work_item_set_id: None,
                    kind: WorkItemKind::Backend,
                    sequence_hint: Some((index as u32 + 1) * 10),
                    depends_on: Vec::new(),
                    exclusive_write_scopes: Vec::new(),
                    forbidden_write_scopes: Vec::new(),
                    context_budget: WorkItemContextBudget::default(),
                    required_handoff_from: Vec::new(),
                    verification_plan_ref: verification_plan_ids.get(index).map(|s| s.to_string()),
                    require_execution_plan_confirm: false,
                    execution_plan_status: WorkItemExecutionPlanStatus::NotStarted,
                    handoff_summary_ref: None,
                    completion_commit: None,
                    completion_diff_summary_ref: None,
                    created_at: "2026-06-17T00:00:00Z".to_string(),
                    updated_at: "2026-06-17T00:00:00Z".to_string(),
                },
            )
            .collect(),
        verification_plans: verification_plan_ids
            .iter()
            .map(|id| VerificationPlan {
                id: id.to_string(),
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                work_item_id: work_item_ids[0].to_string(),
                repository_profile_ref: Some(profile_id.to_string()),
                provider_run_ref: None,
                scope: VerificationScope::Unit,
                commands: vec![VerificationCommand {
                    id: "cmd_001".to_string(),
                    label: "cargo test".to_string(),
                    command: "cargo test --lib".to_string(),
                    cwd: String::new(),
                    purpose: "unit tests".to_string(),
                    required: true,
                    timeout_seconds: 120,
                    source: VerificationCommandSource::Provider,
                    safety: VerificationCommandSafety::Approved,
                }],
                manual_checks: vec![VerificationManualCheck {
                    id: "manual_001".to_string(),
                    label: "smoke".to_string(),
                    instructions: "run locally".to_string(),
                    required: true,
                }],
                required_gates: Vec::new(),
                risk_notes: Vec::new(),
                confidence: RepositoryProfileConfidence::High,
                fallback_policy: VerificationFallbackPolicy::ManualGate,
                created_at: "2026-06-17T00:00:00Z".to_string(),
                updated_at: "2026-06-17T00:00:00Z".to_string(),
            })
            .collect(),
    }
}

#[test]
fn replace_issue_work_item_plan_candidate_swaps_draft_work_items_and_updates_plan() {
    let root = tempdir().expect("tempdir");
    let store = LifecycleStore::new(ProductAppPaths::new(root.path().join(".aria")));

    let work_item_a = store
        .create_work_item(CreateWorkItemInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            story_spec_ids: vec!["story_spec_0001".to_string()],
            design_spec_ids: vec!["design_spec_0001".to_string()],
            title: "old work item a".to_string(),
            id: Some("work_item_0001".to_string()),
            plan_status: WorkItemPlanStatus::Draft,
            ..Default::default()
        })
        .expect("work item a");
    let work_item_b = store
        .create_work_item(CreateWorkItemInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            story_spec_ids: vec!["story_spec_0001".to_string()],
            design_spec_ids: vec!["design_spec_0001".to_string()],
            title: "old work item b".to_string(),
            id: Some("work_item_0002".to_string()),
            plan_status: WorkItemPlanStatus::Draft,
            ..Default::default()
        })
        .expect("work item b");

    let profile = store
        .create_repository_profile(CreateRepositoryProfileInput {
            id: Some("repository_profile_0001".to_string()),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            provider_run_ref: None,
            languages: vec!["rust".to_string()],
            frameworks: Vec::new(),
            package_managers: vec!["cargo".to_string()],
            test_frameworks: Vec::new(),
            build_systems: vec!["cargo".to_string()],
            verification_capabilities: vec!["cargo test".to_string()],
            detected_layers: vec!["backend".to_string()],
            split_recommendation: "backend".to_string(),
            confidence: RepositoryProfileConfidence::High,
            uncertainties: Vec::new(),
        })
        .expect("profile");

    let verification_plan = store
        .create_verification_plan(CreateVerificationPlanInput {
            id: Some("verification_plan_0001".to_string()),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: work_item_a.id.clone(),
            repository_profile_ref: Some(profile.id.clone()),
            provider_run_ref: None,
            scope: VerificationScope::Unit,
            commands: vec![VerificationCommand {
                id: "cmd_001".to_string(),
                label: "cargo test".to_string(),
                command: "cargo test --lib".to_string(),
                cwd: String::new(),
                purpose: "unit tests".to_string(),
                required: true,
                timeout_seconds: 120,
                source: VerificationCommandSource::Provider,
                safety: VerificationCommandSafety::Approved,
            }],
            manual_checks: Vec::new(),
            required_gates: Vec::new(),
            risk_notes: Vec::new(),
            confidence: RepositoryProfileConfidence::High,
            fallback_policy: VerificationFallbackPolicy::ManualGate,
        })
        .expect("verification plan");

    let plan = store
        .create_issue_work_item_plan(CreateIssueWorkItemPlanInput {
            id: Some("issue_work_item_plan_0001".to_string()),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            source_story_spec_ids: vec!["story_spec_0001".to_string()],
            source_design_spec_ids: vec!["design_spec_0001".to_string()],
            options: IssueWorkItemPlanOptions {
                include_integration_tests: false,
                include_e2e_tests: false,
                force_frontend_backend_split: false,
                require_execution_plan_confirm: false,
            },
            status: IssueWorkItemPlanStatus::Draft,
            work_item_ids: vec![work_item_a.id.clone(), work_item_b.id.clone()],
            repository_profile_ref: Some(profile.id.clone()),
            verification_plan_ids: vec![verification_plan.id.clone()],
            dependency_graph: vec![IssueWorkItemDependencyEdge {
                from_work_item_id: work_item_a.id.clone(),
                to_work_item_id: work_item_b.id.clone(),
            }],
            created_from_provider_run: None,
            validator_findings: Vec::new(),
        })
        .expect("plan");

    let new_output = new_split_output_with_ids(
        "issue_work_item_plan_9999",
        "repository_profile_0002",
        &["work_item_0003", "work_item_0004"],
        &["verification_plan_0002", "verification_plan_0003"],
    );

    let finding = WorkItemSplitFinding {
        severity: WorkItemSplitFindingSeverity::Warning,
        code: "scope_overlap".to_string(),
        message: "watch overlaps".to_string(),
        work_item_ids: vec!["work_item_0003".to_string()],
    };
    let snapshot: WorkItemPlanCandidateSnapshot = store
        .replace_issue_work_item_plan_candidate(
            "project_0001",
            "issue_0001",
            &plan.id,
            &new_output,
            vec![finding.clone()],
        )
        .expect("replace");

    // old work items are removed
    let work_items = store.list_work_items("project_0001", "issue_0001").unwrap();
    assert!(
        work_items
            .iter()
            .all(|wi| wi.id != "work_item_0001" && wi.id != "work_item_0002")
    );
    assert_eq!(work_items.len(), 2);

    // new work items exist
    assert_eq!(
        snapshot.work_item_ids,
        vec!["work_item_0003".to_string(), "work_item_0004".to_string()]
    );
    assert_eq!(snapshot.plan_id, plan.id);

    // plan references updated, status and created_at preserved
    let plan_after = store
        .get_issue_work_item_plan("project_0001", "issue_0001", &plan.id)
        .unwrap();
    assert_eq!(plan_after.work_item_ids, snapshot.work_item_ids);
    assert_eq!(
        plan_after.verification_plan_ids,
        snapshot.verification_plan_ids
    );
    assert_eq!(
        plan_after.repository_profile_ref.as_deref(),
        Some(snapshot.repository_profile_id.as_str())
    );
    assert_eq!(plan_after.status, IssueWorkItemPlanStatus::Draft);
    assert_eq!(plan_after.id, plan.id);
    assert_eq!(plan_after.created_at, plan.created_at);
    assert_eq!(plan_after.validator_findings, vec![finding]);
    assert_eq!(
        plan_after.created_from_provider_run,
        Some("provider_run_split_0001".to_string())
    );

    // output.plan.id is ignored
    assert!(
        store
            .list_issue_work_item_plans("project_0001", "issue_0001")
            .unwrap()
            .iter()
            .all(|p| p.id != "issue_work_item_plan_9999")
    );
}

