use cadence_aria::product::coding_models::{
    CodingAttemptPlanBinding, CodingUnitRun, CodingUnitRunStatus,
};
use cadence_aria::product::models::{
    DependencyGraphRevision, HandoffRevision, HumanPresentationRevision, LogicalWorkItem,
    PlanProjectionBundle, PlanRevisionReason, VerificationPlanRevision, WorkItemPlanLineage,
    WorkItemPlanRevision, WorkItemProjectionBundle, WorkItemRevision,
};
use cadence_aria::product::work_item_contract::{
    CanonicalWorkItemContract, HandoffContract, WorkItemContractIdentity, WorkItemGoal,
    WorkItemWritePolicy, canonical_contract_hash,
};
use cadence_aria::product::work_item_revision_store::WorkItemRevisionStore;
use cadence_aria::product::work_item_projection::{
    CoderGroupContext, HumanGroupProjection, ReviewerGroupMatrix, WorkItemProjectionCompiler,
    projection_hashes, renderer_for,
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
                permission_modes: cadence_aria::product::models::WorkspaceRolePermissionModes::default(),
            },
            target_snapshot: None,
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
                permission_modes: cadence_aria::product::models::WorkspaceRolePermissionModes::default(),
            },
            target_snapshot: None,
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

fn create_completed_unit_run_for_test(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    unit_id: &str,
    start_commit: &str,
    completion_commit: &str,
) {
    let unit = store
        .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("coding units")
        .into_iter()
        .find(|unit| unit.id == unit_id)
        .expect("coding unit");
    store
        .create_coding_unit_run(
            attempt,
            &CodingUnitRun {
                id: format!("coding_unit_run_fixture_{unit_id}"),
                unit_id: unit.id,
                execution_no: 1,
                work_item_revision_id: unit.work_item_revision_id,
                resolved_handoff_revision_ids: Vec::new(),
                canonical_contract_hash: "fixture_contract_hash".to_string(),
                projection_bundle_id: "fixture_projection_bundle".to_string(),
                projection_compiler_version: "fixture_compiler".to_string(),
                coder_provider_renderer_version: "fixture_renderer".to_string(),
                reviewer_provider_renderer_version: "fixture_renderer".to_string(),
                internal_reviewer_provider_renderer_version: None,
                coder_projection_hash: "fixture_coder_projection".to_string(),
                reviewer_projection_hash: "fixture_reviewer_projection".to_string(),
                coder_execution_context_hash: None,
                reviewer_execution_context_hash: None,
                internal_reviewer_execution_context_hash: None,
                status: CodingUnitRunStatus::Completed,
                unit_rework_count: 0,
                verification_retry_count: 0,
                operational_retry_count: 0,
                plan_repair_count: 0,
                start_commit: Some(start_commit.to_string()),
                completion_commit: Some(completion_commit.to_string()),
                created_at: "2026-08-07T00:00:00Z".to_string(),
                updated_at: "2026-08-07T00:00:00Z".to_string(),
            },
        )
        .expect("completed unit run");
}

fn complete_group_final_readiness_snapshot(
    attempt: &CodingExecutionAttempt,
    units: &[cadence_aria::product::coding_models::CodingExecutionUnit],
) -> cadence_aria::product::coding_models::GroupFinalReadinessSnapshot {
    use cadence_aria::product::coding_models::{
        GroupFinalReadinessSnapshot, GroupFinalReadinessStatus, GroupFinalReadinessUnit,
    };

    GroupFinalReadinessSnapshot {
        attempt_id: attempt.id.clone(),
        status: GroupFinalReadinessStatus::Complete,
        units: units
            .iter()
            .map(|unit| GroupFinalReadinessUnit {
                unit_id: unit.id.clone(),
                logical_work_item_id: unit.logical_work_item_id.clone(),
                unit_run_id: Some(format!("fixture_run_{}", unit.id)),
                start_commit: Some("seed-commit".to_string()),
                completion_commit: Some("seed-commit".to_string()),
                empty_observation: true,
                code_review_report_id: Some(format!("fixture_review_{}", unit.id)),
                review_verdict: Some(ReviewVerdict::Approve),
                review_summary: Some("fixture independent review approved".to_string()),
                review_findings: Some(Vec::new()),
                handoff_revision_id: Some(format!("fixture_handoff_{}", unit.id)),
                plan_revision_id: Some("plan_revision_0001".to_string()),
                ..Default::default()
            })
            .collect(),
        diagnostics: Vec::new(),
        created_at: "2026-08-07T00:00:00Z".to_string(),
    }
}

fn write_complete_group_final_readiness_snapshot(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
) {
    let units = store
        .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("group coding units");
    store
        .write_group_final_readiness_snapshot(
            attempt,
            &complete_group_final_readiness_snapshot(attempt, &units),
        )
        .expect("complete group final readiness snapshot");
}

fn init_group_worktree(worktree: &Path) {
    init_repo(worktree);
    fs::create_dir_all(worktree.join("src")).expect("create group src dir");
    fs::write(worktree.join("src/backend.rs"), "// backend\n").expect("write backend file");
    fs::write(worktree.join("src/frontend.rs"), "// frontend\n").expect("write frontend file");
    run_git(worktree, &["add", "."]);
    run_git(worktree, &["commit", "-m", "seed group files"]);
    run_git(worktree, &["tag", "seed-commit"]);
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
    let mut projection_bundle_refs = Vec::new();
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
            depends_on: Vec::new(),
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
            .put_verification_plan_revision(
                &lineage,
                &VerificationPlanRevision {
                    id: revision.verification_plan_revision_id.clone(),
                    logical_work_item_id: logical.id.clone(),
                    source_draft_revision_id: revision.source_draft_revision_id.clone(),
                    verification_checks: revision.canonical_contract.verification_checks.clone(),
                    created_at: "2026-06-27T00:00:00Z".to_string(),
                },
            )
            .expect("put verification plan revision");
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
        projection_bundle_refs.push(revision.work_item_projection_bundle_id.clone());
        revision_store
            .put_human_presentation_revision(
                &lineage,
                &HumanPresentationRevision {
                    id: format!("human_presentation_{logical_id}"),
                    source_plan_projection_bundle_id: None,
                    source_work_item_projection_bundle_id: Some(
                        revision.work_item_projection_bundle_id.clone(),
                    ),
                    supersedes: None,
                    human_summary: logical.title.clone(),
                    why_split: None,
                    dependency_explanation: Vec::new(),
                    risk_explanation: Vec::new(),
                    source_refs: Vec::new(),
                    normative: false,
                    used_by_provider: false,
                    created_at: "2026-06-27T00:00:00Z".to_string(),
                },
            )
            .expect("put human presentation revision");
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
    let ordered_logical_work_item_ids = bindings.keys().cloned().collect::<Vec<_>>();
    revision_store
        .put_plan_projection_bundle(
            &lineage,
            &PlanProjectionBundle {
                id: "plan_projection_bundle_0001".to_string(),
                plan_revision_id: "plan_revision_0001".to_string(),
                dependency_graph_revision_id: graph.id.clone(),
                work_item_projection_bundle_refs: projection_bundle_refs,
                human_group_projection: HumanGroupProjection {
                    plan_id: lineage.id.clone(),
                    goal: "group terminal fixture".to_string(),
                    split_reason: "authoritative runtime fixture".to_string(),
                    work_items: Vec::new(),
                    contract_flow: Vec::new(),
                    risks: Vec::new(),
                    source_refs: Vec::new(),
                    normative: false,
                    used_by_provider: false,
                },
                coder_group_context: CoderGroupContext {
                    plan_id: lineage.id.clone(),
                    ordered_logical_work_item_ids,
                    dependency_edges: graph.edges.clone(),
                    group_write_scopes: std::collections::BTreeMap::new(),
                },
                reviewer_group_matrix: ReviewerGroupMatrix {
                    plan_id: lineage.id.clone(),
                    work_items: Vec::new(),
                    dependency_edges: graph.edges.clone(),
                    design_traceability_refs: Vec::new(),
                },
                human_group_projection_hash: "fixture-human-group-hash".to_string(),
                coder_group_context_hash: "fixture-coder-group-hash".to_string(),
                reviewer_group_matrix_hash: "fixture-reviewer-group-hash".to_string(),
                compiler_version: "plan-projection-compiler-v1".to_string(),
                created_at: "2026-06-27T00:00:00Z".to_string(),
            },
        )
        .expect("put plan projection bundle");
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
        reviewer_provider_renderer_version: renderer_version.clone(),
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
        start_commit: Some("seed-commit".to_string()),
        completion_commit: Some("seed-commit".to_string()),
        created_at: "2026-06-27T00:00:00Z".to_string(),
        updated_at: "2026-06-27T00:00:00Z".to_string(),
    };
    store
        .create_coding_unit_run(attempt, &run)
        .expect("source coding unit run");
    let handoff = HandoffRevision {
        id: format!("handoff_revision_{}", run.id),
        logical_work_item_id: source_unit.logical_work_item_id.clone(),
        work_item_revision_id: source_unit.work_item_revision_id.clone(),
        coding_unit_run_id: run.id.clone(),
        provided_contracts: Vec::new(),
        provided_capabilities: std::collections::BTreeMap::new(),
        contract_hash: revision.canonical_contract_hash,
        commit_sha: "seed-commit".to_string(),
        created_at: "2026-06-27T00:00:00Z".to_string(),
    };
    revision_store
        .put_handoff_revision(&lineage, &handoff)
        .expect("source handoff revision");
    store
        .update_coding_unit_completion_commit(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &source_unit.id,
            Some("seed-commit".to_string()),
        )
        .expect("source completion commit");
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

fn completed_group_attempt_with_handoff_revisions() -> (
    tempfile::TempDir,
    ProductAppPaths,
    CodingAttemptStore,
    CodingWorkspaceEngine,
    CodingExecutionAttempt,
) {
    let root = tempdir().expect("root");
    let worktree = root.path().join("group-worktree");
    init_group_worktree(&worktree);
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    seed_group_work_items_and_plan(&paths);
    let store = CodingAttemptStore::new(paths.clone());
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
                permission_modes: cadence_aria::product::models::WorkspaceRolePermissionModes::default(),
            },
            target_snapshot: None,
            max_auto_rework: 2,
        })
        .expect("create group attempt");
    seed_authoritative_group_final_review_fixture(&store, &attempt);
    store
        .save_review_request(&attempt, &sample_review_request(&attempt.id))
        .expect("save review request");
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    (root, paths, store, engine, attempt)
}

fn group_attempt_waiting_for_final_confirm() -> (
    tempfile::TempDir,
    ProductAppPaths,
    CodingAttemptStore,
    CodingWorkspaceEngine,
    CodingExecutionAttempt,
) {
    let (root, paths, store, engine, attempt) =
        completed_group_attempt_with_handoff_revisions();
    let lifecycle = LifecycleStore::new(paths.clone());
    for work_item_id in ["work_item_0001", "work_item_0002"] {
        lifecycle
            .update_work_item_execution_status(
                "project_0001",
                "issue_0001",
                work_item_id,
                WorkItemStatus::Coding,
            )
            .expect("set coding status");
    }
    let last_unit = store
        .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("group coding units")
        .into_iter()
        .find(|unit| unit.logical_work_item_id == "work_item_0002")
        .expect("last coding unit");
    store
        .update_coding_unit_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &last_unit.id,
            CodingExecutionUnitStatus::Completed,
            Some("frontend done".to_string()),
        )
        .expect("refresh terminal attempt pointers");
    let attempt = crate::seed_coding_attempt_running(&store, &attempt.project_id, &attempt.issue_id, &attempt.id);
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
            Some("seed-commit".to_string()),
        )
        .expect("set head commit");
    let worktree_path = attempt
        .worktree_path
        .clone()
        .expect("group attempt worktree");
    lifecycle
        .upsert_issue_shared_worktree(UpsertIssueSharedWorktreeInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path,
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
    write_complete_group_final_readiness_snapshot(&store, &attempt);
    (root, paths, store, engine, attempt)
}
