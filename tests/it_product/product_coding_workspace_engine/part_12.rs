use cadence_aria::product::coding_models::{
    CodingAttemptPlanBinding, CodingUnitRun, CodingUnitRunStatus,
};
use cadence_aria::product::models::{
    DependencyGraphRevision, HandoffRevision, LogicalWorkItem, PlanRevisionReason,
    WorkItemPlanLineage, WorkItemPlanRevision, WorkItemPlanStatus, WorkItemProjectionBundle,
    WorkItemRevision,
};
use cadence_aria::product::work_item_contract::{
    CanonicalWorkItemContract, HandoffContract, WorkItemContractIdentity, WorkItemGoal,
    WorkItemWritePolicy, canonical_contract_hash,
};
use cadence_aria::product::work_item_revision_store::WorkItemRevisionStore;
use cadence_aria::product::work_item_projection::{
    WorkItemProjectionCompiler, projection_hashes, renderer_for,
};

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
    let worktree = root.path().join("group-worktree");
    init_group_worktree(&worktree);
    let attempt = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_work_item_id: "work_item_0001".to_string(),
            base_branch: "HEAD".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: Some(worktree),
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
            logical_work_item_id: "work_item_0001".to_string(),
            work_item_revision_id: "work_item_revision_0001".to_string(),
            dependency_logical_work_item_ids: Vec::new(),
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
            logical_work_item_id: "work_item_0002".to_string(),
            work_item_revision_id: "work_item_revision_0002".to_string(),
            dependency_logical_work_item_ids: vec!["work_item_0001".to_string()],
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
    let worktree = root.path().join("group-worktree");
    init_group_worktree(&worktree);
    let attempt = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_work_item_id: "work_item_0001".to_string(),
            base_branch: "HEAD".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: Some(worktree),
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
            logical_work_item_id: "work_item_0001".to_string(),
            work_item_revision_id: "work_item_revision_0001".to_string(),
            dependency_logical_work_item_ids: Vec::new(),
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
            logical_work_item_id: "work_item_0002".to_string(),
            work_item_revision_id: "work_item_revision_0002".to_string(),
            dependency_logical_work_item_ids: vec!["work_item_0001".to_string()],
            order_index: 1,
            status: CodingExecutionUnitStatus::Running,
        })
        .expect("create coding unit 2");
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    (root, paths, store, engine, attempt)
}

fn init_group_worktree(worktree: &Path) {
    init_repo(worktree);
    fs::create_dir_all(worktree.join("src")).expect("create group src dir");
    fs::write(worktree.join("src/backend.rs"), "// backend\n").expect("write backend file");
    fs::write(worktree.join("src/frontend.rs"), "// frontend\n").expect("write frontend file");
    run_git(worktree, &["add", "."]);
    run_git(worktree, &["commit", "-m", "seed group files"]);
}

fn seed_authoritative_group_terminal_fixture(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
) {
    let revision_store = WorkItemRevisionStore::new(store.paths());
    let lineage = WorkItemPlanLineage {
        id: "work_item_plan_0001".to_string(),
        project_id: attempt.project_id.clone(),
        issue_id: attempt.issue_id.clone(),
        story_spec_refs: Vec::new(),
        design_spec_refs: Vec::new(),
        active_revision_id: None,
        active_amendment_id: None,
        created_at: "2026-06-27T00:00:00Z".to_string(),
        updated_at: "2026-06-27T00:00:00Z".to_string(),
    };
    revision_store
        .put_plan_lineage(&lineage)
        .expect("put group plan lineage");
    let mut bindings = std::collections::BTreeMap::new();
    for (logical_id, revision_id) in [
        ("work_item_0001", "work_item_revision_0001"),
        ("work_item_0002", "work_item_revision_0002"),
    ] {
        let logical = LogicalWorkItem {
            id: logical_id.to_string(),
            plan_id: lineage.id.clone(),
            title: logical_id.to_string(),
            active_revision_id: None,
            created_at: "2026-06-27T00:00:00Z".to_string(),
            updated_at: "2026-06-27T00:00:00Z".to_string(),
        };
        revision_store
            .put_logical_work_item(&lineage, &logical)
            .expect("put logical work item");
        let contract = CanonicalWorkItemContract {
            schema_version: 1,
            identity: WorkItemContractIdentity {
                logical_work_item_id: logical.id.clone(),
                title: logical.title.clone(),
                kind: "implementation".to_string(),
            },
            goal: WorkItemGoal {
                summary: logical.title.clone(),
            },
            non_goals: Vec::new(),
            input_contracts: Vec::new(),
            output_contracts: Vec::new(),
            tasks: Vec::new(),
            write_policy: WorkItemWritePolicy {
                exclusive_scopes: Vec::new(),
                forbidden_scopes: Vec::new(),
            },
            acceptance_criteria: Vec::new(),
            verification_checks: Vec::new(),
            handoff_contract: HandoffContract {
                required_fields: Vec::new(),
                provided_contract_refs: Vec::new(),
                reviewer_check_refs: Vec::new(),
            },
            blocker_rules: Vec::new(),
            design_traceability: Vec::new(),
        };
        let revision = WorkItemRevision {
            id: revision_id.to_string(),
            logical_work_item_id: logical.id.clone(),
            source_draft_revision_id: format!("draft_{logical_id}"),
            canonical_contract_hash: canonical_contract_hash(&contract).expect("contract hash"),
            canonical_contract: contract,
            work_item_projection_bundle_id: format!("projection_{logical_id}"),
            verification_plan_revision_id: format!("verification_{logical_id}"),
            created_at: "2026-06-27T00:00:00Z".to_string(),
        };
        revision_store
            .put_work_item_revision(&lineage, &revision)
            .expect("put work item revision");
        let projections = WorkItemProjectionCompiler
            .compile(&revision.canonical_contract, &revision.id)
            .expect("compile work item projections");
        let hashes = projection_hashes(&projections).expect("projection hashes");
        revision_store
            .put_work_item_projection_bundle(
                &lineage,
                &WorkItemProjectionBundle {
                    id: revision.work_item_projection_bundle_id.clone(),
                    work_item_revision_id: revision.id.clone(),
                    canonical_contract_hash: revision.canonical_contract_hash.clone(),
                    projection_schema_version: 1,
                    compiler_version: "work-item-projection-compiler-v1".to_string(),
                    human_projection: projections.human,
                    coder_projection: projections.coder,
                    reviewer_projection: projections.reviewer,
                    human_projection_hash: hashes.human,
                    coder_projection_hash: hashes.coder,
                    reviewer_projection_hash: hashes.reviewer,
                    created_at: "2026-06-27T00:00:00Z".to_string(),
                },
            )
            .expect("put work item projection bundle");
        revision_store
            .set_active_work_item_revision(&lineage, &logical, None, &revision.id)
            .expect("activate work item revision");
        bindings.insert(logical.id, revision.id);
    }
    let graph = DependencyGraphRevision {
        id: "dependency_graph_revision_0001".to_string(),
        plan_id: lineage.id.clone(),
        edges: vec![cadence_aria::product::work_item_contract::DependencyContractEdge {
            from: "work_item_0001".to_string(),
            to: "work_item_0002".to_string(),
            required_contracts: Vec::new(),
        }],
        created_at: "2026-06-27T00:00:00Z".to_string(),
    };
    revision_store
        .put_dependency_graph_revision(&lineage, &graph)
        .expect("put dependency graph revision");
    let plan_revision = WorkItemPlanRevision {
        id: "plan_revision_0001".to_string(),
        plan_id: lineage.id.clone(),
        revision_no: 1,
        supersedes: None,
        reason: PlanRevisionReason::InitialCompile,
        work_item_bindings: bindings,
        dependency_graph_revision_id: graph.id,
        validation_report_ref: "validation_report_0001".to_string(),
        plan_projection_bundle_id: "plan_projection_bundle_0001".to_string(),
        created_at: "2026-06-27T00:00:00Z".to_string(),
    };
    revision_store
        .put_plan_revision(&lineage, &plan_revision)
        .expect("put plan revision");
    revision_store
        .set_active_plan_revision(&lineage, &plan_revision.id)
        .expect("activate plan revision");
    store
        .save_plan_binding(
            attempt,
            &CodingAttemptPlanBinding {
                attempt_id: attempt.id.clone(),
                plan_id: lineage.id,
                bound_plan_revision_id: plan_revision.id,
                applied_amendment_ids: Vec::new(),
                updated_at: "2026-06-27T00:00:00Z".to_string(),
            },
        )
        .expect("save group plan binding");
}

fn seed_authoritative_group_coder_fixture(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
) {
    seed_authoritative_group_terminal_fixture(store, attempt);
    let revision_store = WorkItemRevisionStore::new(store.paths());
    let lineage = revision_store
        .get_plan_lineage(&attempt.project_id, &attempt.issue_id, "work_item_plan_0001")
        .expect("group plan lineage");
    let units = store
        .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("group coding units");
    let source_unit = units
        .iter()
        .find(|unit| unit.logical_work_item_id == "work_item_0001")
        .expect("source coding unit");
    let revision = revision_store
        .get_work_item_revision(
            &lineage,
            &source_unit.logical_work_item_id,
            &source_unit.work_item_revision_id,
        )
        .expect("source work item revision");
    let bundle = revision_store
        .get_work_item_projection_bundle(&lineage, &revision.work_item_projection_bundle_id)
        .expect("source projection bundle");
    let renderer_version = renderer_for(&ProviderName::Fake).renderer_version().to_string();
    let run = CodingUnitRun {
        id: "coding_unit_run_0001".to_string(),
        unit_id: source_unit.id.clone(),
        execution_no: 1,
        work_item_revision_id: revision.id.clone(),
        resolved_handoff_revision_ids: Vec::new(),
        canonical_contract_hash: revision.canonical_contract_hash.clone(),
        projection_bundle_id: bundle.id.clone(),
        projection_compiler_version: bundle.compiler_version.clone(),
        coder_provider_renderer_version: renderer_version.clone(),
        reviewer_provider_renderer_version: renderer_version,
        coder_projection_hash: bundle.coder_projection_hash.clone(),
        reviewer_projection_hash: bundle.reviewer_projection_hash.clone(),
        coder_execution_context_hash: None,
        reviewer_execution_context_hash: None,
        status: CodingUnitRunStatus::Completed,
        unit_rework_count: 0,
        verification_retry_count: 0,
        operational_retry_count: 0,
        plan_repair_count: 0,
        start_commit: Some("seed-commit".to_string()),
        completion_commit: Some("seed-commit".to_string()),
        created_at: "2026-06-27T00:00:00Z".to_string(),
        updated_at: "2026-06-27T00:00:00Z".to_string(),
    };
    store
        .create_coding_unit_run(attempt, &run)
        .expect("source coding unit run");
    let handoff = HandoffRevision {
        id: "handoff_revision_0001".to_string(),
        logical_work_item_id: source_unit.logical_work_item_id.clone(),
        work_item_revision_id: source_unit.work_item_revision_id.clone(),
        coding_unit_run_id: run.id,
        provided_contracts: Vec::new(),
        provided_capabilities: std::collections::BTreeMap::new(),
        contract_hash: revision.canonical_contract_hash,
        commit_sha: "seed-commit".to_string(),
        tests: Vec::new(),
        artifacts: Vec::new(),
        created_at: "2026-06-27T00:00:00Z".to_string(),
    };
    revision_store
        .put_handoff_revision(&lineage, &handoff)
        .expect("source handoff revision");
    store
        .update_coding_unit_latest_handoff_revision_id(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &source_unit.id,
            Some(handoff.id),
        )
        .expect("source handoff binding");
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
    for (index, work_item_id) in ["work_item_0001", "work_item_0002"]
        .into_iter()
        .enumerate()
    {
        lifecycle
            .create_work_item(CreateWorkItemInput {
                id: Some(work_item_id.to_string()),
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                repository_id: "repository_0001".to_string(),
                story_spec_ids: Vec::new(),
                design_spec_ids: Vec::new(),
                title: format!("title for {work_item_id}"),
                work_item_set_id: Some("work_item_plan_0001".to_string()),
                sequence_hint: Some(((index + 1) * 10) as u32),
                plan_status: WorkItemPlanStatus::Confirmed,
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
    lifecycle
        .create_issue_work_item_plan(CreateIssueWorkItemPlanInput {
            id: Some("work_item_plan_0001".to_string()),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            source_story_spec_ids: Vec::new(),
            source_design_spec_ids: Vec::new(),
            options: IssueWorkItemPlanOptions {
                include_integration_tests: false,
                include_e2e_tests: false,
                force_frontend_backend_split: false,
                require_execution_plan_confirm: false,
            },
            status: IssueWorkItemPlanStatus::Confirmed,
            work_item_ids: vec!["work_item_0001".to_string(), "work_item_0002".to_string()],
            repository_profile_ref: None,
            verification_plan_ids: Vec::new(),
            dependency_graph: Vec::new(),
            created_from_provider_run: None,
            validator_findings: Vec::new(),
        })
        .expect("create issue work item plan");
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
        .update_coding_unit_latest_handoff_revision_id(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            "coding_unit_0001",
            Some("handoff_revision_0001".to_string()),
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
        .update_coding_unit_latest_handoff_revision_id(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            "coding_unit_0002",
            Some("handoff_revision_0002".to_string()),
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
        .save_review_request(&attempt, &sample_review_request(&attempt.id))
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
    for (index, work_item_id) in ["work_item_0001", "work_item_0002"]
        .into_iter()
        .enumerate()
    {
        lifecycle
            .create_work_item(CreateWorkItemInput {
                id: Some(work_item_id.to_string()),
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                repository_id: "repository_0001".to_string(),
                story_spec_ids: Vec::new(),
                design_spec_ids: Vec::new(),
                title: format!("title for {work_item_id}"),
                work_item_set_id: Some("work_item_plan_0001".to_string()),
                sequence_hint: Some(((index + 1) * 10) as u32),
                plan_status: WorkItemPlanStatus::Confirmed,
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
    lifecycle
        .create_issue_work_item_plan(CreateIssueWorkItemPlanInput {
            id: Some("work_item_plan_0001".to_string()),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            source_story_spec_ids: Vec::new(),
            source_design_spec_ids: Vec::new(),
            options: IssueWorkItemPlanOptions {
                include_integration_tests: false,
                include_e2e_tests: false,
                force_frontend_backend_split: false,
                require_execution_plan_confirm: false,
            },
            status: IssueWorkItemPlanStatus::Confirmed,
            work_item_ids: vec!["work_item_0001".to_string(), "work_item_0002".to_string()],
            repository_profile_ref: None,
            verification_plan_ids: Vec::new(),
            dependency_graph: Vec::new(),
            created_from_provider_run: None,
            validator_findings: Vec::new(),
        })
        .expect("create issue work item plan");
    seed_authoritative_group_terminal_fixture(&store, &attempt);
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
        .update_coding_unit_latest_handoff_revision_id(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            "coding_unit_0001",
            Some("handoff_revision_0001".to_string()),
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
        .update_coding_unit_latest_handoff_revision_id(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            "coding_unit_0002",
            Some("handoff_revision_0002".to_string()),
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
    (root, paths, store, engine, attempt)
}
