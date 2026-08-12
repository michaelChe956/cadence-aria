use super::*;
use crate::product::lifecycle_store::UpsertIssueSharedWorktreeInput;

fn running_group_attempt() -> (
    tempfile::TempDir,
    CodingAttemptStore,
    CodingExecutionAttempt,
) {
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
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
                permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
            },
            target_snapshot: None,
            max_auto_rework: 2,
        })
        .expect("group attempt");
    seed_group_attempt_fixture(&store, &attempt, true, false);
    let attempt = store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running group attempt");
    (root, store, attempt)
}

fn unit_statuses(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
) -> Vec<CodingExecutionUnitStatus> {
    store
        .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("coding units")
        .into_iter()
        .map(|unit| unit.status)
        .collect()
}

#[tokio::test]
async fn coding_plan_repair_group_terminal_abort_converges_units_and_clears_resume_pointers() {
    let (_root, store, attempt) = running_group_attempt();
    let units = store
        .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("coding units");
    store
        .update_coding_unit_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &units[2].id,
            CodingExecutionUnitStatus::Completed,
            None,
        )
        .expect("completed unit");
    let (tx, mut rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    let aborted = engine
        .handle_abort(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .await
        .expect("abort group attempt");

    assert_eq!(aborted.status, CodingAttemptStatus::Aborted);
    assert_eq!(aborted.active_unit_id, None);
    assert_eq!(aborted.current_work_item_id, None);
    assert_eq!(
        unit_statuses(&store, &aborted),
        vec![
            CodingExecutionUnitStatus::Skipped,
            CodingExecutionUnitStatus::Skipped,
            CodingExecutionUnitStatus::Completed,
        ]
    );
    store
        .validate_group_attempt_integrity(&aborted)
        .expect("aborted group integrity");
    let error = engine
        .start_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .await
        .expect_err("terminal group must not restart");
    assert!(
        error
            .to_string()
            .contains("invalid_coding_attempt_status_transition")
    );
    assert!(
        rx.try_recv().is_err(),
        "terminal group emitted a coding event"
    );
}

#[tokio::test]
async fn coding_plan_repair_group_terminal_failure_fails_active_unit_and_skips_pending_units() {
    let (_root, store, attempt) = running_group_attempt();
    let attempt = store
        .update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::CodeReview,
        )
        .expect("code review group attempt");
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    let failed = engine
        .handle_attempt_failed(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .await
        .expect("fail group attempt");

    assert_eq!(failed.status, CodingAttemptStatus::Failed);
    assert_eq!(failed.active_unit_id, None);
    assert_eq!(failed.current_work_item_id, None);
    assert_eq!(
        unit_statuses(&store, &failed),
        vec![
            CodingExecutionUnitStatus::Failed,
            CodingExecutionUnitStatus::Skipped,
            CodingExecutionUnitStatus::Skipped,
        ]
    );
    store
        .validate_group_attempt_integrity(&failed)
        .expect("failed group integrity");
    assert!(
        recoverable_failed_code_review(&store, &failed)
            .expect("inspect fatal review failure")
            .is_none()
    );
}

#[tokio::test]
async fn group_review_material_failure_keeps_business_error_when_terminal_lock_was_transferred() {
    let (root, store, attempt) = running_group_attempt();
    let lifecycle = LifecycleStore::new(store.paths());
    let shared_worktree = root.path().join("shared-worktree");
    fs::create_dir_all(&shared_worktree).expect("create shared worktree");
    init_test_git_repo(&shared_worktree);
    fs::write(shared_worktree.join("dirty.txt"), "uncommitted\n").expect("dirty worktree");
    lifecycle
        .upsert_issue_shared_worktree(UpsertIssueSharedWorktreeInput {
            project_id: attempt.project_id.clone(),
            issue_id: attempt.issue_id.clone(),
            repository_id: "repository_0001".to_string(),
            branch_name: attempt.branch_name.clone(),
            worktree_path: shared_worktree,
            base_branch: attempt.base_branch.clone(),
        })
        .expect("upsert shared worktree");
    lifecycle
        .try_acquire_issue_worktree_lock(
            &attempt.project_id,
            &attempt.issue_id,
            "work_item_0001",
            &attempt.id,
        )
        .expect("acquire first unit lock");
    lifecycle
        .transfer_issue_worktree_lock(
            &attempt.project_id,
            &attempt.issue_id,
            "work_item_0001",
            "work_item_0002",
            &attempt.id,
        )
        .expect("transfer lock to the second unit");
    let node_id = "coding_node_0001";
    store
        .save_timeline_node(
            &attempt,
            CodingTimelineNode {
                id: node_id.to_string(),
                attempt_id: attempt.id.clone(),
                stage: CodingExecutionStage::InternalPrReview,
                title: "Group final review".to_string(),
                status: CodingTimelineNodeStatus::Running,
                agent_role: Some(CodingAgentRole::Reviewer),
                summary: None,
                started_at: "2026-08-06T00:00:00Z".to_string(),
                completed_at: None,
                artifact_refs: Vec::new(),
            },
        )
        .expect("save group final review node");
    let role_run = store
        .create_role_run(
            &attempt,
            CodingExecutionStage::InternalPrReview,
            CodingProviderRole::InternalReviewer,
            CodingRoleRunTrigger::Initial,
            Some(node_id.to_string()),
        )
        .expect("create group final review role run");
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);

    let error = engine
        .finalize_group_review_failure(
            &attempt,
            node_id,
            &role_run.id,
            CodingWorkspaceEngineError::GroupReviewMaterial(
                "unit_cross_review_record_exceeds_size_limit".into(),
            ),
        )
        .await
        .expect_err("group review material failure remains surfaced");

    assert!(
        error.to_string().contains("group_review_material_error"),
        "terminal lock cleanup must not replace the business error: {error}"
    );
    let shared = lifecycle
        .get_issue_shared_worktree(&attempt.project_id, &attempt.issue_id)
        .expect("reload shared worktree")
        .expect("shared worktree exists");
    assert_eq!(
        shared.current_active_work_item_id.as_deref(),
        Some("work_item_0002")
    );
    assert_eq!(
        shared.current_lock_owner_id.as_deref(),
        Some(attempt.id.as_str())
    );
    assert_eq!(
        shared.status,
        crate::product::models::IssueSharedWorktreeStatus::Running
    );
}

#[tokio::test]
async fn group_final_review_completion_uses_transferred_lock_owner_before_completion_gates() {
    let (root, store, attempt) = running_group_attempt();
    for unit in store
        .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("coding units")
    {
        store
            .update_coding_unit_status(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &unit.id,
                CodingExecutionUnitStatus::Completed,
                None,
            )
            .expect("complete coding unit");
    }
    let lifecycle = LifecycleStore::new(store.paths());
    lifecycle
        .upsert_issue_shared_worktree(UpsertIssueSharedWorktreeInput {
            project_id: attempt.project_id.clone(),
            issue_id: attempt.issue_id.clone(),
            repository_id: "repository_0001".to_string(),
            branch_name: attempt.branch_name.clone(),
            worktree_path: root.path().join("shared-worktree"),
            base_branch: attempt.base_branch.clone(),
        })
        .expect("upsert shared worktree");
    lifecycle
        .try_acquire_issue_worktree_lock(
            &attempt.project_id,
            &attempt.issue_id,
            "work_item_0001",
            &attempt.id,
        )
        .expect("acquire first unit lock");
    lifecycle
        .transfer_issue_worktree_lock(
            &attempt.project_id,
            &attempt.issue_id,
            "work_item_0001",
            "work_item_0002",
            &attempt.id,
        )
        .expect("transfer lock to the second unit");
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);

    let error = engine
        .complete_group_attempt_after_final_review(&attempt)
        .await
        .expect_err("missing completion commit still fails the completion gate");

    assert!(
        error.to_string().contains("completion_commit_missing"),
        "group completion must pass owner preflight before later gates: {error}"
    );
}

fn trim_group_final_review_fixture_to_two_units(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
) {
    let units = store
        .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("coding units");
    let removed_unit = units
        .iter()
        .find(|unit| unit.logical_work_item_id == "work_item_0003")
        .expect("third coding unit");
    let removed_unit_path = store
        .paths()
        .issue_lifecycle_root(&attempt.project_id, &attempt.issue_id)
        .join("coding-attempts")
        .join(&attempt.id)
        .join("units")
        .join(format!("{}.json", removed_unit.id));
    fs::remove_file(removed_unit_path).expect("remove third coding unit");

    let revision_store = WorkItemRevisionStore::new(store.paths());
    let lineage = revision_store
        .get_plan_lineage(
            &attempt.project_id,
            &attempt.issue_id,
            "work_item_plan_0001",
        )
        .expect("plan lineage");
    let mut binding = store
        .get_plan_binding(attempt)
        .expect("attempt plan binding");
    let mut plan = revision_store
        .get_plan_revision(
            &attempt.project_id,
            &attempt.issue_id,
            &lineage.id,
            &binding.bound_plan_revision_id,
        )
        .expect("bound plan revision");
    let removed_revision_id = plan
        .work_item_bindings
        .remove("work_item_0003")
        .expect("third work item binding");
    let removed_revision = revision_store
        .get_work_item_revision(&lineage, "work_item_0003", &removed_revision_id)
        .expect("third work item revision");
    let previous_plan_revision_id = plan.id.clone();
    let mut projections = revision_store
        .get_plan_projection_bundle(&lineage, &plan.plan_projection_bundle_id)
        .expect("plan projection bundle");
    plan.id = "plan_revision_task1_two_units".to_string();
    plan.revision_no += 1;
    plan.supersedes = Some(previous_plan_revision_id);
    plan.plan_projection_bundle_id = "plan_projection_bundle_task1_two_units".to_string();
    plan.created_at = "2026-08-06T00:00:00Z".to_string();

    projections.id = plan.plan_projection_bundle_id.clone();
    projections.plan_revision_id = plan.id.clone();
    projections
        .work_item_projection_bundle_refs
        .retain(|id| id != &removed_revision.work_item_projection_bundle_id);
    projections
        .human_group_projection
        .work_items
        .retain(|item| item.logical_work_item_id != "work_item_0003");
    projections
        .coder_group_context
        .ordered_logical_work_item_ids
        .retain(|id| id != "work_item_0003");
    projections
        .coder_group_context
        .group_write_scopes
        .remove("work_item_0003");
    projections
        .reviewer_group_matrix
        .work_items
        .retain(|item| item.logical_work_item_id != "work_item_0003");
    let hashes = plan_projection_hashes(&CompiledPlanProjections {
        human: projections.human_group_projection.clone(),
        coder: projections.coder_group_context.clone(),
        reviewer: projections.reviewer_group_matrix.clone(),
    })
    .expect("two-unit projection hashes");
    projections.human_group_projection_hash = hashes.human;
    projections.coder_group_context_hash = hashes.coder;
    projections.reviewer_group_matrix_hash = hashes.reviewer;
    projections.created_at = "2026-08-06T00:00:00Z".to_string();
    revision_store
        .put_plan_projection_bundle(&lineage, &projections)
        .expect("two-unit plan projection bundle");
    revision_store
        .put_plan_revision(&lineage, &plan)
        .expect("two-unit plan revision");
    revision_store
        .set_active_plan_revision(&lineage, &plan.id)
        .expect("activate two-unit plan revision");
    binding.bound_plan_revision_id = plan.id;
    binding
        .applied_amendment_ids
        .push("plan_amendment_task1_two_units".to_string());
    binding.updated_at = "2026-08-06T00:00:00Z".to_string();
    store
        .save_plan_binding(attempt, &binding)
        .expect("bind two-unit plan revision");
}

fn seed_completed_group_final_review_runtime(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    completion_commit: &str,
) {
    let revision_store = WorkItemRevisionStore::new(store.paths());
    let lineage = revision_store
        .get_plan_lineage(
            &attempt.project_id,
            &attempt.issue_id,
            "work_item_plan_0001",
        )
        .expect("plan lineage");
    let providers = store
        .get_role_provider_config_snapshot(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("provider snapshot");
    let coder_renderer_version = renderer_for(&providers.coder)
        .renderer_version()
        .to_string();
    let reviewer_renderer_version = renderer_for(&providers.code_reviewer)
        .renderer_version()
        .to_string();
    for (index, unit) in store
        .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("coding units")
        .into_iter()
        .enumerate()
    {
        let revision = revision_store
            .get_work_item_revision(
                &lineage,
                &unit.logical_work_item_id,
                &unit.work_item_revision_id,
            )
            .expect("work item revision");
        let bundle = revision_store
            .get_work_item_projection_bundle(&lineage, &revision.work_item_projection_bundle_id)
            .expect("work item projection bundle");
        let run = CodingUnitRun {
            id: format!("coding_unit_run_task1_{:04}", index + 1),
            unit_id: unit.id.clone(),
            execution_no: 1,
            work_item_revision_id: revision.id.clone(),
            resolved_handoff_revision_ids: Vec::new(),
            canonical_contract_hash: revision.canonical_contract_hash.clone(),
            projection_bundle_id: bundle.id,
            projection_compiler_version: bundle.compiler_version,
            coder_provider_renderer_version: coder_renderer_version.clone(),
            reviewer_provider_renderer_version: reviewer_renderer_version.clone(),
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
            start_commit: Some(completion_commit.to_string()),
            completion_commit: Some(completion_commit.to_string()),
            created_at: "2026-08-06T00:00:00Z".to_string(),
            updated_at: "2026-08-06T00:00:00Z".to_string(),
        };
        store
            .create_coding_unit_run(attempt, &run)
            .expect("completed coding unit run");
        let handoff = HandoffRevision {
            id: format!("handoff_revision_{}", run.id),
            logical_work_item_id: unit.logical_work_item_id.clone(),
            work_item_revision_id: revision.id,
            coding_unit_run_id: run.id,
            provided_contracts: Vec::new(),
            provided_capabilities: std::collections::BTreeMap::new(),
            contract_hash: revision.canonical_contract_hash,
            commit_sha: completion_commit.to_string(),
            created_at: "2026-08-06T00:00:00Z".to_string(),
        };
        revision_store
            .put_handoff_revision(&lineage, &handoff)
            .expect("completed handoff revision");
        store
            .update_coding_unit_completion_commit(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &unit.id,
                Some(completion_commit.to_string()),
            )
            .expect("unit completion commit");
        store
            .update_coding_unit_latest_handoff_revision_id(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &unit.id,
                Some(handoff.id),
            )
            .expect("unit handoff binding");
        store
            .update_coding_unit_status(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &unit.id,
                CodingExecutionUnitStatus::Completed,
                None,
            )
            .expect("complete coding unit");
    }
}

#[tokio::test]
async fn group_final_review_completion_releases_transferred_shared_worktree_lock() {
    let (root, store, attempt) = running_group_attempt();
    trim_group_final_review_fixture_to_two_units(&store, &attempt);
    let worktree = root.path().join("shared-worktree");
    fs::create_dir_all(&worktree).expect("create shared worktree");
    init_test_git_repo(&worktree);
    let completion_commit = git_stdout(&worktree, &["rev-parse", "HEAD"])
        .trim()
        .to_string();
    let attempt = store
        .update_attempt_worktree_path(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            worktree.clone(),
        )
        .expect("group worktree path");
    let attempt = store
        .update_attempt_head_commit(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            Some(completion_commit.clone()),
        )
        .expect("group head commit");
    let attempt = store
        .update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::ReviewRequest,
        )
        .expect("group final review stage");
    seed_completed_group_final_review_runtime(&store, &attempt, &completion_commit);
    let attempt = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("reload final-review attempt");
    let lifecycle = LifecycleStore::new(store.paths());
    lifecycle
        .upsert_issue_shared_worktree(UpsertIssueSharedWorktreeInput {
            project_id: attempt.project_id.clone(),
            issue_id: attempt.issue_id.clone(),
            repository_id: "repository_0001".to_string(),
            branch_name: attempt.branch_name.clone(),
            worktree_path: worktree,
            base_branch: attempt.base_branch.clone(),
        })
        .expect("upsert shared worktree");
    lifecycle
        .try_acquire_issue_worktree_lock(
            &attempt.project_id,
            &attempt.issue_id,
            "work_item_0001",
            &attempt.id,
        )
        .expect("acquire first unit lock");
    lifecycle
        .transfer_issue_worktree_lock(
            &attempt.project_id,
            &attempt.issue_id,
            "work_item_0001",
            "work_item_0002",
            &attempt.id,
        )
        .expect("transfer lock to the second unit");
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);

    let completed = engine
        .complete_group_attempt_after_final_review(&attempt)
        .await
        .expect("complete group final review");

    assert_eq!(completed.status, CodingAttemptStatus::Completed);
    let shared = lifecycle
        .get_issue_shared_worktree(&attempt.project_id, &attempt.issue_id)
        .expect("reload shared worktree")
        .expect("shared worktree exists");
    assert_eq!(shared.current_active_work_item_id, None);
    assert_eq!(shared.current_lock_owner_id, None);
    assert_eq!(
        shared.status,
        crate::product::models::IssueSharedWorktreeStatus::Ready
    );
}

#[test]
fn coding_plan_repair_group_terminal_validator_accepts_final_review_and_completed_states() {
    let (_root, store, attempt) = running_group_attempt();
    for unit in store
        .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("coding units")
    {
        store
            .update_coding_unit_status(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &unit.id,
                CodingExecutionUnitStatus::Completed,
                None,
            )
            .expect("complete unit");
    }
    let final_review = store
        .update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::ReviewRequest,
        )
        .expect("final review stage");
    store
        .validate_group_attempt_integrity(&final_review)
        .expect("final review integrity");

    let completed = store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Completed,
        )
        .expect("completed group attempt");
    store
        .validate_group_attempt_integrity(&completed)
        .expect("completed group integrity");
}

#[test]
fn coding_plan_repair_group_terminal_completion_fails_closed_until_all_units_completed() {
    let (_root, store, attempt) = running_group_attempt();
    let original_attempt = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("original attempt");
    let original_statuses = unit_statuses(&store, &attempt);

    let error = store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Completed,
        )
        .expect_err("incomplete group cannot complete");

    assert!(
        error
            .to_string()
            .contains("coding_group_attempt_incomplete")
    );
    assert_eq!(
        store
            .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("unchanged attempt"),
        original_attempt
    );
    assert_eq!(unit_statuses(&store, &attempt), original_statuses);
}

#[test]
fn coding_plan_repair_group_terminal_write_failure_does_not_expose_terminal_attempt() {
    let (_root, store, attempt) = running_group_attempt();
    let active_unit_id = attempt.active_unit_id.as_deref().expect("active unit");
    let active_unit_path = store
        .paths()
        .issue_lifecycle_root(&attempt.project_id, &attempt.issue_id)
        .join("coding-attempts")
        .join(&attempt.id)
        .join("units")
        .join(format!("{active_unit_id}.json"));
    fs::write(active_unit_path, b"{ invalid json").expect("corrupt active unit");

    store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Aborted,
        )
        .expect_err("unit read failure must abort terminal transition");

    let unchanged = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("attempt remains readable");
    assert_eq!(unchanged.status, CodingAttemptStatus::Running);
    assert_eq!(unchanged.active_unit_id, attempt.active_unit_id);
    assert_eq!(unchanged.current_work_item_id, attempt.current_work_item_id);
}

#[test]
fn coding_plan_repair_group_terminal_validator_rejects_created_attempt_disguised_as_final_review() {
    let (_root, store, attempt) = running_group_attempt();
    for unit in store
        .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("coding units")
    {
        store
            .update_coding_unit_status(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &unit.id,
                CodingExecutionUnitStatus::Completed,
                None,
            )
            .expect("complete unit");
    }
    let mut corrupted = store
        .update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::ReviewRequest,
        )
        .expect("review request stage");
    corrupted.status = CodingAttemptStatus::Created;
    store
        .save_coding_attempt(&corrupted)
        .expect("corrupt attempt status");

    let error = store
        .validate_group_attempt_integrity(&corrupted)
        .expect_err("created attempt cannot use final review no-target state");

    assert!(
        error
            .to_string()
            .contains("coding_group_attempt_incomplete")
    );
}

#[test]
fn coding_plan_repair_single_work_item_terminal_status_update_is_unchanged() {
    let root = tempdir().expect("tempdir");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: "work_item_0001".to_string(),
            base_branch: "HEAD".to_string(),
            branch_name: "aria/work-items/work_item_0001".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
                permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
            },
            target_snapshot: None,
            max_auto_rework: 2,
        })
        .expect("single attempt");
    let running = store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running single attempt");

    let aborted = store
        .update_attempt_status(
            &running.project_id,
            &running.issue_id,
            &running.id,
            CodingAttemptStatus::Aborted,
        )
        .expect("abort single attempt");

    assert_eq!(aborted.status, CodingAttemptStatus::Aborted);
    assert_eq!(aborted.scope, CodingAttemptScope::WorkItem);
    assert_eq!(aborted.work_item_id, running.work_item_id);
    assert_eq!(aborted.active_unit_id, running.active_unit_id);
    assert_eq!(aborted.current_work_item_id, running.current_work_item_id);
}
