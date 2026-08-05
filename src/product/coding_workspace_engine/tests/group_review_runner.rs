use super::*;
use crate::cross_cutting::streaming_provider::{ProviderCompletion, ProviderSession};
use crate::product::coding_models::{
    CodingRoleRunStatus, CodingTimelineNodeStatus, CompactFindingDigest,
    UnitReviewConclusionSnapshot,
};
use crate::product::work_item_projection::renderer_for;
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
struct GroupReviewRunnerProvider {
    prompts: Arc<Mutex<Vec<String>>>,
}

impl GroupReviewRunnerProvider {
    fn prompts(&self) -> Vec<String> {
        self.prompts.lock().expect("prompts").clone()
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for GroupReviewRunnerProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
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

async fn group_review_runner_fixture(
    completion_commits_present: bool,
) -> (
    tempfile::TempDir,
    CodingAttemptStore,
    CodingExecutionAttempt,
    CodingWorkspaceEngine,
    GroupReviewRunnerProvider,
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
    let mut attempt = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_work_item_id: "work_item_0001".to_string(),
            base_branch: base_commit.clone(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: Some(worktree),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Fake,
                reviewer: Some(ProviderName::Fake),
                review_rounds: 1,
                permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
            },
            max_auto_rework: 2,
        })
        .expect("group attempt");
    seed_group_attempt_fixture(&store, &attempt, true, false);
    let revision_store = WorkItemRevisionStore::new(store.paths());
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
            start_commit: Some(base_commit.clone()),
            completion_commit: completion_commits_present.then(|| completion_commit.clone()),
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
        id: "review_request_group_runner".to_string(),
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
    attempt = store
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
    let provider = GroupReviewRunnerProvider::default();
    (root, store, attempt, engine, provider)
}

#[tokio::test]
async fn group_final_review_delegates_to_orchestrator_and_activates_snapshot() {
    let (_root, store, attempt, engine, provider) = group_review_runner_fixture(true).await;
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    let review = engine
        .execute_group_final_review_with_commands(&attempt, &provider, &mut command_rx)
        .await
        .expect("group final review");

    assert_eq!(review.verdict, ReviewVerdict::Approve);
    assert!(
        store
            .get_active_group_review_snapshot_hash(&attempt.id)
            .expect("active snapshot")
            .is_some()
    );
    assert_eq!(provider.prompts().len(), 2, "one shard plus reduction");
    assert_eq!(
        store
            .list_group_review_shard_reports(&attempt.id)
            .expect("shard reports")
            .len(),
        1
    );
    assert_eq!(
        store
            .list_group_review_reduction_reports(&attempt.id)
            .expect("reduction reports")
            .len(),
        1
    );
}

#[tokio::test]
async fn group_final_review_git_fact_failure_closes_running_state_with_failure_gate() {
    let (_root, store, attempt, engine, provider) = group_review_runner_fixture(false).await;
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    let error = engine
        .execute_group_final_review_with_commands(&attempt, &provider, &mut command_rx)
        .await
        .expect_err("missing completion commit must fail closed");

    assert!(matches!(
        error,
        CodingWorkspaceEngineError::GroupReviewBlocked {
            ref reason_code,
            gate_id: Some(_)
        } if reason_code.contains("completion_commit_missing")
    ));
    let stored_attempt = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("attempt");
    assert_eq!(stored_attempt.status, CodingAttemptStatus::Blocked);
    let role_run = store
        .list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("role runs")
        .into_iter()
        .last()
        .expect("role run");
    assert_eq!(role_run.status, CodingRoleRunStatus::Blocked);
    let node = store
        .get_timeline_nodes(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("timeline")
        .into_iter()
        .last()
        .expect("node");
    assert_eq!(node.status, CodingTimelineNodeStatus::Blocked);
    assert!(node.completed_at.is_some());
    let gates = store
        .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("gates");
    assert_eq!(gates.len(), 1);
    assert_eq!(gates[0].reason_code.as_deref(), Some("identity_missing"));
    assert!(provider.prompts().is_empty());
}
