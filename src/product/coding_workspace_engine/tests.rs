use super::*;
use crate::cross_cutting::streaming_provider::ProviderSession;
use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::{
    CreateCodingAttemptInput, CreateCodingExecutionUnitInput, CreateGroupCodingAttemptInput,
};
use crate::product::coding_models::{
    CodingAttemptScope, CodingExecutionUnitStatus, CodingProviderRole, RemoteKind,
};
use crate::product::lifecycle_store::{CreateWorkItemInput, LifecycleStore};
use crate::product::models::{ProviderConversationRef, ProviderConversationRole};
use crate::web::workspace_ws_types::ProviderConfigSnapshot;
use std::fs;
use std::path::Path;
use std::process::Command as StdCommand;
use tempfile::tempdir;

fn blocked_report_with(missing: Vec<String>, skipped: Vec<String>) -> TestingReport {
    TestingReport {
        id: "testing_report_0001".to_string(),
        attempt_id: "coding_attempt_0001".to_string(),
        role_run_id: None,
        run_no: None,
        commands: Vec::new(),
        overall_status: TestingOverallStatus::Blocked,
        provider_claim: None,
        backend_verified: true,
        started_at: "2026-06-10T00:00:00Z".to_string(),
        completed_at: Some("2026-06-10T00:00:01Z".to_string()),
        plan_id: Some("test_plan_0001".to_string()),
        plan_summary: Some("plan".to_string()),
        steps: Vec::new(),
        unplanned_commands: Vec::new(),
        unplanned_evidence: Vec::new(),
        missing_required_steps: missing,
        skipped_required_steps: skipped,
        context_warnings: Vec::new(),
        raw_provider_output_ref: None,
    }
}

mod gate_coder_feedback;
mod gate_rework;
mod parser_prompt;
mod provider_driven;

#[tokio::test]
async fn group_start_attempt_with_existing_worktree_skips_worktree_prepare_node() {
    let root = tempdir().expect("tempdir");
    let worktree = root.path().join("shared-worktree");
    std::fs::create_dir_all(&worktree).expect("worktree dir");
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
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
            },
            max_auto_rework: 2,
        })
        .expect("group attempt");
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
}

#[tokio::test]
async fn single_attempt_completes_after_review_request_without_internal_review_node() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    LifecycleStore::new(store.paths())
        .create_work_item(CreateWorkItemInput {
            id: Some(attempt.work_item_id.clone()),
            project_id: attempt.project_id.clone(),
            issue_id: attempt.issue_id.clone(),
            repository_id: "repository_0001".to_string(),
            title: "single work item".to_string(),
            ..Default::default()
        })
        .expect("create work item");
    let attempt = store
        .update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::ReviewRequest,
        )
        .expect("review request stage");
    let attempt = store
        .update_attempt_head_commit(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            Some("deadbeef".to_string()),
        )
        .expect("head commit");
    store
        .save_review_request(&ReviewRequest {
            id: "review_request_0001".to_string(),
            attempt_id: attempt.id.clone(),
            kind: ReviewRequestKind::GitBranchOnly,
            remote_kind: RemoteKind::GenericGit,
            remote: "origin".to_string(),
            base_branch: attempt.base_branch.clone(),
            branch_name: attempt.branch_name.clone(),
            commit_sha: "deadbeef".to_string(),
            push_status: PushStatus::Pushed,
            external_url: None,
            manual_instructions: Vec::new(),
            created_at: "2026-07-07T00:00:00Z".to_string(),
            updated_at: "2026-07-07T00:00:00Z".to_string(),
        })
        .expect("review request");
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    let completed = engine
        .complete_attempt_after_review_request(&attempt)
        .await
        .expect("complete after review request");

    assert_eq!(completed.scope, CodingAttemptScope::WorkItem);
    assert_eq!(completed.status, CodingAttemptStatus::Completed);
    assert_eq!(completed.stage, CodingExecutionStage::ReviewRequest);
    assert!(
        store
            .get_work_item_handoff(&completed.project_id, &completed.issue_id, &completed.id)
            .expect("work item handoff")
            .is_some()
    );
    assert!(
        store
            .get_timeline_nodes(&completed.project_id, &completed.issue_id, &completed.id)
            .expect("timeline")
            .iter()
            .all(|node| node.stage != CodingExecutionStage::InternalPrReview)
    );
}

#[tokio::test]
async fn group_unit_completion_commits_changes_and_advances_to_next_unit() {
    let root = tempdir().expect("tempdir");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree dir");
    init_test_git_repo(&worktree);
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let lifecycle = LifecycleStore::new(store.paths());
    for work_item_id in ["work_item_0001", "work_item_0002"] {
        lifecycle
            .create_work_item(CreateWorkItemInput {
                id: Some(work_item_id.to_string()),
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                repository_id: "repository_0001".to_string(),
                title: work_item_id.to_string(),
                ..Default::default()
            })
            .expect("create work item");
    }
    let attempt = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_work_item_id: "work_item_0001".to_string(),
            base_branch: "HEAD~1".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: Some(worktree.clone()),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
            },
            max_auto_rework: 2,
        })
        .expect("group attempt");
    let attempt = store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running attempt");
    let unit1 = store
        .create_coding_unit(CreateCodingExecutionUnitInput {
            attempt_id: attempt.id.clone(),
            project_id: attempt.project_id.clone(),
            issue_id: attempt.issue_id.clone(),
            plan_id: "work_item_plan_0001".to_string(),
            work_item_id: "work_item_0001".to_string(),
            order_index: 1,
            status: CodingExecutionUnitStatus::Running,
        })
        .expect("unit1");
    let unit2 = store
        .create_coding_unit(CreateCodingExecutionUnitInput {
            attempt_id: attempt.id.clone(),
            project_id: attempt.project_id.clone(),
            issue_id: attempt.issue_id.clone(),
            plan_id: "work_item_plan_0001".to_string(),
            work_item_id: "work_item_0002".to_string(),
            order_index: 2,
            status: CodingExecutionUnitStatus::Pending,
        })
        .expect("unit2");
    fs::write(worktree.join("unit1.txt"), "unit 1 change\n").expect("unit change");
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let attempt = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("current attempt");

    let updated = engine
        .complete_group_unit_after_code_review(&attempt)
        .await
        .expect("complete unit");

    assert_eq!(
        updated.current_work_item_id.as_deref(),
        Some("work_item_0002")
    );
    assert_eq!(updated.active_unit_id.as_deref(), Some(unit2.id.as_str()));
    assert_eq!(updated.stage, CodingExecutionStage::PrepareContext);
    assert_eq!(updated.status, CodingAttemptStatus::Running);
    let units = store
        .list_coding_units(&updated.project_id, &updated.issue_id, &updated.id)
        .expect("units");
    let completed_unit = units
        .iter()
        .find(|unit| unit.id == unit1.id)
        .expect("completed unit");
    assert_eq!(completed_unit.status, CodingExecutionUnitStatus::Completed);
    let completion_commit = completed_unit
        .completion_commit
        .as_deref()
        .expect("completion commit");
    assert_eq!(completion_commit.len(), 40);
    assert_eq!(updated.head_commit.as_deref(), Some(completion_commit));
    assert_eq!(
        git_stdout(&worktree, &["status", "--porcelain"]),
        "",
        "unit completion should leave no staged or unstaged source changes"
    );
}

fn running_attempt_with_worktree() -> (
    tempfile::TempDir,
    CodingAttemptStore,
    CodingExecutionAttempt,
) {
    let root = tempdir().expect("tempdir");
    let worktree = root.path().join("worktree");
    std::fs::create_dir_all(&worktree).expect("worktree dir");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: "work_item_0001".to_string(),
            base_branch: "HEAD".to_string(),
            branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
            worktree_path: Some(worktree),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
            },
            max_auto_rework: 2,
        })
        .expect("create attempt");
    let attempt = store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running attempt");
    (root, store, attempt)
}

fn test_attempt(id: &str) -> CodingExecutionAttempt {
    CodingExecutionAttempt {
        id: id.to_string(),
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        work_item_id: "work_item_0001".to_string(),
        attempt_no: 1,
        scope: crate::product::coding_models::CodingAttemptScope::WorkItem,
        status: CodingAttemptStatus::Running,
        stage: CodingExecutionStage::Coding,
        base_branch: "HEAD".to_string(),
        branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
        worktree_path: None,
        provider_config_snapshot: ProviderConfigSnapshot {
            author: ProviderName::ClaudeCode,
            reviewer: Some(ProviderName::Codex),
            review_rounds: 1,
        },
        provider_conversations: Vec::new(),
        rework_count: 0,
        max_auto_rework: 2,
        work_item_group_id: None,
        current_work_item_id: Some("work_item_0001".to_string()),
        active_unit_id: None,
        head_commit: None,
        pushed_remote: None,
        review_request_id: None,
        created_at: "2026-06-01T00:00:00Z".to_string(),
        updated_at: "2026-06-01T00:00:00Z".to_string(),
        completed_at: None,
    }
}

fn init_test_git_repo(repo: &Path) {
    run_test_git(repo, &["init"]);
    run_test_git(repo, &["config", "user.email", "aria@example.com"]);
    run_test_git(repo, &["config", "user.name", "Aria Test"]);
    fs::write(repo.join("README.md"), "initial\n").expect("seed file");
    run_test_git(repo, &["add", "."]);
    run_test_git(repo, &["commit", "-m", "initial"]);
}

fn git_stdout(cwd: &Path, args: &[&str]) -> String {
    let output = run_test_git(cwd, args);
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn run_test_git(cwd: &Path, args: &[&str]) -> std::process::Output {
    let output = StdCommand::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap_or_else(|error| panic!("git {} failed to start: {error}", args.join(" ")));
    if !output.status.success() {
        panic!(
            "git {} failed\nstdout:\n{}\nstderr:\n{}",
            args.join(" "),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    output
}
