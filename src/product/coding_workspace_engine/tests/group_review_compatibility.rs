//! Task 8c: legacy hash 兼容性测试（Step 5-8）。
//!
//! 覆盖：
//! - Step 5-6：历史 Attempt（有 legacy hash）可重试、不触发 IdentityMismatch、hash 不变
//! - Step 7-8：历史 Attempt（无 legacy hash）可重试、不补写 hash

use crate::cross_cutting::streaming_provider::{
    ProviderCompletion, ProviderEvent, ProviderSession, StreamingProviderAdapter,
    StreamingProviderInput,
};
use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::{CodingAttemptStore, CreateGroupCodingAttemptInput};
use crate::product::coding_models::{
    CodingAttemptStatus, CodingExecutionAttempt, CodingUnitRun, CodingUnitRunStatus,
    CompactFindingDigest, PushStatus, RemoteKind, ReviewRequest, ReviewRequestKind, ReviewVerdict,
    UnitReviewConclusionSnapshot,
};
use crate::product::coding_workspace_engine::CodingWorkspaceEngine;
use crate::product::coding_workspace_engine::GitWorkspaceService;
use crate::product::models::ProviderName;
use crate::product::work_item_projection::renderer_for;
use crate::web::workspace_ws_types::ProviderConfigSnapshot as WsProviderConfigSnapshot;
use std::fs;
use std::path::Path;
use std::process::Command as StdCommand;
use std::sync::{Arc, Mutex};
use tempfile::tempdir;
use tokio::sync::mpsc;

fn init_test_git_repo(repo: &Path) {
    run_test_git(repo, &["init", "--quiet"]);
    run_test_git(repo, &["config", "user.email", "test@example.com"]);
    run_test_git(repo, &["config", "user.name", "Test"]);
    run_test_git(repo, &["add", "."]);
    run_test_git(repo, &["commit", "-m", "init", "--quiet", "--allow-empty"]);
}

fn run_test_git(cwd: &Path, args: &[&str]) -> StdCommand {
    let mut cmd = StdCommand::new("git");
    cmd.current_dir(cwd).args(args);
    let _ = cmd.output().expect("git command");
    cmd
}

fn git_stdout(cwd: &Path, args: &[&str]) -> String {
    String::from_utf8(
        StdCommand::new("git")
            .current_dir(cwd)
            .args(args)
            .output()
            .expect("git command")
            .stdout,
    )
    .expect("utf8")
}

/// A fake streaming provider that returns valid group-review JSON for all calls.
#[derive(Clone, Default)]
struct LegacyHashProvider {
    prompts: Arc<Mutex<Vec<String>>>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for LegacyHashProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: tokio_util::sync::CancellationToken,
    ) -> Result<ProviderSession, crate::cross_cutting::provider_adapter::ProviderAdapterError> {
        self.prompts.lock().expect("prompts").push(input.prompt);
        let (event_tx, event_rx) = mpsc::channel(4);
        let (command_tx, _command_rx) = mpsc::channel(1);
        tokio::spawn(async move {
            let output = serde_json::json!({
                "verdict": "approve",
                "summary": "group review approved",
                "findings": [],
                "impact_scope": ["group"],
                "pr_description": "group description",
                "commit_message_suggestion": "review group"
            })
            .to_string();
            let _ = event_tx
                .send(ProviderEvent::Completed(ProviderCompletion::plain(
                    output, None,
                )))
                .await;
        });
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }
}

/// Builds a group attempt with completed unit runs that optionally carry a
/// legacy `internal_reviewer_execution_context_hash`.
async fn legacy_hash_fixture(
    legacy_hash: Option<String>,
) -> (
    tempfile::TempDir,
    CodingAttemptStore,
    CodingExecutionAttempt,
    CodingWorkspaceEngine,
    LegacyHashProvider,
) {
    let root = tempdir().expect("tempdir");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    init_test_git_repo(&worktree);
    let base_commit = git_stdout(&worktree, &["rev-parse", "HEAD"])
        .trim()
        .to_string();
    fs::write(worktree.join("group.txt"), "group change\n").expect("group change");
    run_test_git(&worktree, &["add", "."]);
    run_test_git(&worktree, &["commit", "-m", "group change"]);
    let completion_commit = git_stdout(&worktree, &["rev-parse", "HEAD"])
        .trim()
        .to_string();

    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_work_item_id: "work_item_0001".to_string(),
            base_branch: base_commit.clone(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: Some(worktree),
            provider_config_snapshot: WsProviderConfigSnapshot {
                author: ProviderName::Fake,
                reviewer: Some(ProviderName::Fake),
                review_rounds: 1,
                permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
            },
            target_snapshot: None,
            max_auto_rework: 2,
        })
        .expect("group attempt");
    // Reuse existing seed helper
    super::seed_group_attempt_fixture_with_compact_routing(&store, &attempt, true, false);

    let revision_store =
        crate::product::work_item_revision_store::WorkItemRevisionStore::new(store.paths());
    let lineage = revision_store
        .get_plan_lineage(
            &attempt.project_id,
            &attempt.issue_id,
            attempt.work_item_group_id.as_deref().expect("plan id"),
        )
        .expect("lineage");
    for unit in store
        .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("units")
    {
        let revision = revision_store
            .get_work_item_revision(
                &lineage,
                &unit.logical_work_item_id,
                &unit.work_item_revision_id,
            )
            .expect("revision");
        let bundle = revision_store
            .get_work_item_projection_bundle(&lineage, &revision.work_item_projection_bundle_id)
            .expect("bundle");
        let run = CodingUnitRun {
            id: format!("{}_run_0001", unit.id),
            unit_id: unit.id.clone(),
            execution_no: 1,
            work_item_revision_id: revision.id,
            resolved_handoff_revision_ids: Vec::new(),
            canonical_contract_hash: bundle.canonical_contract_hash,
            projection_bundle_id: bundle.id,
            projection_compiler_version: bundle.compiler_version,
            coder_provider_renderer_version: renderer_for(&ProviderName::Fake)
                .renderer_version()
                .to_string(),
            reviewer_provider_renderer_version: renderer_for(&ProviderName::Fake)
                .renderer_version()
                .to_string(),
            internal_reviewer_provider_renderer_version: legacy_hash.as_ref().map(|_| {
                renderer_for(&ProviderName::Fake)
                    .renderer_version()
                    .to_string()
            }),
            coder_projection_hash: bundle.coder_projection_hash,
            reviewer_projection_hash: bundle.reviewer_projection_hash,
            coder_execution_context_hash: None,
            reviewer_execution_context_hash: None,
            internal_reviewer_execution_context_hash: legacy_hash.clone(),
            status: CodingUnitRunStatus::Completed,
            unit_rework_count: 0,
            verification_retry_count: 0,
            operational_retry_count: 0,
            plan_repair_count: 0,
            start_commit: Some(base_commit.clone()),
            completion_commit: Some(completion_commit.clone()),
            created_at: "2026-08-04T00:00:00Z".to_string(),
            updated_at: "2026-08-04T00:00:00Z".to_string(),
        };
        store
            .create_coding_unit_run(&attempt, &run)
            .expect("unit run");
        store
            .write_unit_review_conclusion_snapshot(&UnitReviewConclusionSnapshot {
                attempt_id: attempt.id.clone(),
                unit_id: run.unit_id.clone(),
                unit_run_id: run.id.clone(),
                logical_work_item_id: unit.logical_work_item_id,
                work_item_revision_id: run.work_item_revision_id.clone(),
                code_review_report_id: format!("review_{}", run.id),
                verdict: ReviewVerdict::Approve,
                finding_digest: vec![CompactFindingDigest {
                    defect_class: None,
                    reason_code: None,
                    severity: "info".to_string(),
                    message_digest: "approved".to_string(),
                }],
                evidence_refs: vec!["cargo test".to_string()],
                diff_refs: Vec::new(),
                raw_report_hash: format!("raw_{}", run.id),
            })
            .expect("snapshot");
    }
    let review_request = ReviewRequest {
        id: "review_request_legacy".to_string(),
        attempt_id: attempt.id.clone(),
        kind: ReviewRequestKind::GitBranchOnly,
        remote_kind: RemoteKind::GenericGit,
        remote: "origin".to_string(),
        base_branch: base_commit,
        branch_name: attempt.branch_name.clone(),
        commit_sha: completion_commit.clone(),
        push_status: PushStatus::Pushed,
        external_url: None,
        manual_instructions: Vec::new(),
        push_error: None,
        created_at: "2026-08-04T00:00:00Z".to_string(),
        updated_at: "2026-08-04T00:00:00Z".to_string(),
    };
    store
        .save_review_request(&attempt, &review_request)
        .expect("review request");
    let mut attempt = store
        .update_attempt_review_request_state(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            completion_commit,
            "origin".to_string(),
            review_request.id.clone(),
        )
        .expect("review request state");
    attempt = store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let (event_tx, _event_rx) = mpsc::channel(128);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), event_tx);
    let provider = LegacyHashProvider::default();
    (root, store, attempt, engine, provider)
}

#[tokio::test]
async fn step5_6_legacy_attempt_with_hash_can_retry_without_identity_mismatch() {
    let (_root, store, attempt, engine, provider) =
        legacy_hash_fixture(Some("legacy_hash_abc123".to_string())).await;

    // Capture the hash before review
    let units_before = store
        .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("units");
    let runs_before = store
        .list_coding_unit_runs(&attempt, &units_before[0].id)
        .expect("runs");
    let hash_before = runs_before[0]
        .internal_reviewer_execution_context_hash
        .clone();

    let (_command_tx, mut command_rx) = mpsc::channel(1);
    let review = engine
        .execute_group_final_review_with_commands(&attempt, &provider, &mut command_rx)
        .await
        .expect("group final review with legacy hash");

    // Should succeed without IdentityMismatch
    assert_eq!(review.verdict, ReviewVerdict::Approve);

    // Hash should be unchanged (orchestrator does not bind or rewrite hash)
    let runs_after = store
        .list_coding_unit_runs(&attempt, &units_before[0].id)
        .expect("runs");
    assert_eq!(
        runs_after[0].internal_reviewer_execution_context_hash, hash_before,
        "legacy hash must not be modified by group review"
    );

    // No identity_missing gates
    let gates = store
        .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("gates");
    assert!(
        gates
            .iter()
            .all(|g| g.reason_code.as_deref() != Some("identity_missing")),
        "no identity_missing gate should be created"
    );
}

#[tokio::test]
async fn step7_8_legacy_attempt_without_hash_can_retry_without_writing_hash() {
    let (_root, store, attempt, engine, provider) = legacy_hash_fixture(None).await;

    // Capture hash state before review (should be None)
    let units_before = store
        .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("units");
    let runs_before = store
        .list_coding_unit_runs(&attempt, &units_before[0].id)
        .expect("runs");
    assert!(
        runs_before[0]
            .internal_reviewer_execution_context_hash
            .is_none(),
        "hash should be None before review"
    );

    let (_command_tx, mut command_rx) = mpsc::channel(1);
    let review = engine
        .execute_group_final_review_with_commands(&attempt, &provider, &mut command_rx)
        .await
        .expect("group final review without legacy hash");

    assert_eq!(review.verdict, ReviewVerdict::Approve);

    // Hash should still be None (orchestrator does not write hash)
    let runs_after = store
        .list_coding_unit_runs(&attempt, &units_before[0].id)
        .expect("runs");
    assert!(
        runs_after[0]
            .internal_reviewer_execution_context_hash
            .is_none(),
        "hash must remain None (orchestrator does not write hash)"
    );

    // No identity_missing gates
    let gates = store
        .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("gates");
    assert!(
        gates
            .iter()
            .all(|g| g.reason_code.as_deref() != Some("identity_missing")),
        "no identity_missing gate should be created"
    );
}
