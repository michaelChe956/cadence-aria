use super::*;
use crate::cross_cutting::streaming_provider::{ProviderCompletion, ProviderSession};
use crate::product::coding_models::{
    CodingAttemptStatus, CodingRoleRunStatus, CodingTimelineNodeStatus, CompactFindingDigest,
    UnitReviewConclusionSnapshot,
};
use crate::product::work_item_projection::renderer_for;
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
struct TransportFailureGroupReviewProvider {
    prompts: Arc<Mutex<Vec<String>>>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for TransportFailureGroupReviewProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.prompts.lock().expect("prompts").push(input.prompt);
        Err(ProviderAdapterError::execution_failed(
            None,
            String::new(),
            "connection refused",
            0,
        ))
    }
}

impl TransportFailureGroupReviewProvider {
    fn prompts(&self) -> Vec<String> {
        self.prompts.lock().expect("prompts").clone()
    }
}

#[derive(Clone, Default)]
struct ShardSuccessThenTransportFailureProvider {
    prompts: Arc<Mutex<Vec<String>>>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for ShardSuccessThenTransportFailureProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let is_shard = self.prompts.lock().expect("prompts").is_empty();
        self.prompts.lock().expect("prompts").push(input.prompt);
        if !is_shard {
            return Err(ProviderAdapterError::execution_failed(
                None,
                String::new(),
                "connection refused",
                0,
            ));
        }
        let (event_tx, event_rx) = mpsc::channel(1);
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

impl ShardSuccessThenTransportFailureProvider {
    fn prompts(&self) -> Vec<String> {
        self.prompts.lock().expect("prompts").clone()
    }
}

#[derive(Clone, Default)]
struct ProtocolFailureGroupReviewProvider {
    prompts: Arc<Mutex<Vec<String>>>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for ProtocolFailureGroupReviewProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.prompts.lock().expect("prompts").push(input.prompt);
        let (event_tx, event_rx) = mpsc::channel(1);
        let (command_tx, _command_rx) = mpsc::channel(1);
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::ProtocolError {
                    code: "provider_protocol_error".to_string(),
                    message: "unexpected provider event".to_string(),
                    context: None,
                })
                .await;
        });
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }
}

impl ProtocolFailureGroupReviewProvider {
    fn prompts(&self) -> Vec<String> {
        self.prompts.lock().expect("prompts").clone()
    }
}

#[derive(Clone, Default)]
struct GroupReviewRunnerProvider {
    prompts: Arc<Mutex<Vec<String>>>,
}

struct CancelledGroupReviewProvider;

#[async_trait::async_trait]
impl StreamingProviderAdapter for CancelledGroupReviewProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (event_tx, event_rx) = mpsc::channel(1);
        let (command_tx, _command_rx) = mpsc::channel(1);
        tokio::spawn(async move {
            cancel.cancelled().await;
            drop(event_tx);
        });
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }
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
    seed_group_attempt_fixture_with_compact_routing(&store, &attempt, true, false);
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
async fn group_final_review_transport_exhaustion_persists_shard_report_and_blocks_attempt() {
    let (_root, store, attempt, engine, _provider) = group_review_runner_fixture(true).await;
    let provider = TransportFailureGroupReviewProvider::default();
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    let error = engine
        .execute_group_final_review_with_commands(&attempt, &provider, &mut command_rx)
        .await
        .expect_err("transport exhaustion must create a retryable group-review gate");

    assert!(matches!(
        error,
        CodingWorkspaceEngineError::GroupReviewBlocked {
            ref reason_code,
            gate_id: Some(_),
        } if reason_code == "shard_transport_exhausted"
    ));
    assert_eq!(
        provider.prompts().len(),
        3,
        "transport retries stay in shard"
    );
    let reports = store
        .list_group_review_shard_reports(&attempt.id)
        .expect("shard reports");
    assert_eq!(reports.len(), 1);
    assert_eq!(
        reports[0].run_failure_code.as_deref(),
        Some("shard_transport_exhausted")
    );
    assert!(reports[0].raw_provider_output_refs.is_empty());
    let role_run = store
        .list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("role runs")
        .into_iter()
        .last()
        .expect("role run");
    assert!(!reports[0].role_run_ids.is_empty());
    assert_eq!(reports[0].role_run_ids, vec![role_run.id.clone()]);
    assert_eq!(role_run.status, CodingRoleRunStatus::Blocked);
    let node = store
        .get_timeline_nodes(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("timeline")
        .into_iter()
        .last()
        .expect("node");
    assert_eq!(node.status, CodingTimelineNodeStatus::Blocked);
    assert!(node.completed_at.is_some());
    assert_eq!(
        store
            .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("attempt")
            .status,
        CodingAttemptStatus::Blocked
    );
    let gates = store
        .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("gates");
    assert_eq!(gates.len(), 1);
    assert_eq!(
        gates[0].reason_code.as_deref(),
        Some("shard_transport_exhausted")
    );
}

#[tokio::test]
async fn group_final_review_reduction_transport_exhaustion_persists_report_and_blocks_attempt() {
    let (_root, store, attempt, engine, _provider) = group_review_runner_fixture(true).await;
    let provider = ShardSuccessThenTransportFailureProvider::default();
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    let error = engine
        .execute_group_final_review_with_commands(&attempt, &provider, &mut command_rx)
        .await
        .expect_err("reduction transport exhaustion must create a retryable group-review gate");

    assert!(matches!(
        error,
        CodingWorkspaceEngineError::GroupReviewBlocked {
            ref reason_code,
            gate_id: Some(_),
        } if reason_code == "reduction_transport_exhausted"
    ));
    assert_eq!(
        provider.prompts().len(),
        4,
        "only reduction retries after the successful shard"
    );
    let reports = store
        .list_group_review_reduction_reports(&attempt.id)
        .expect("reduction reports");
    assert_eq!(reports.len(), 1);
    assert_eq!(
        reports[0].run_failure_code.as_deref(),
        Some("reduction_transport_exhausted")
    );
    assert!(reports[0].raw_provider_output_refs.is_empty());
    let role_run = store
        .list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("role runs")
        .into_iter()
        .last()
        .expect("role run");
    assert!(!reports[0].role_run_ids.is_empty());
    assert_eq!(reports[0].role_run_ids, vec![role_run.id.clone()]);
    assert_eq!(role_run.status, CodingRoleRunStatus::Blocked);
    let node = store
        .get_timeline_nodes(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("timeline")
        .into_iter()
        .last()
        .expect("node");
    assert_eq!(node.status, CodingTimelineNodeStatus::Blocked);
    assert!(node.completed_at.is_some());
    assert_eq!(
        store
            .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("attempt")
            .status,
        CodingAttemptStatus::Blocked
    );
    let gates = store
        .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("gates");
    assert_eq!(gates.len(), 1);
    assert_eq!(
        gates[0].reason_code.as_deref(),
        Some("reduction_transport_exhausted")
    );
}

#[tokio::test]
async fn group_final_review_provider_protocol_error_does_not_retry_and_is_output_invalid() {
    let (_root, store, attempt, engine, _provider) = group_review_runner_fixture(true).await;
    let provider = ProtocolFailureGroupReviewProvider::default();
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    let error = engine
        .execute_group_final_review_with_commands(&attempt, &provider, &mut command_rx)
        .await
        .expect_err("protocol error must fail closed without transport retry");

    assert!(matches!(
        error,
        CodingWorkspaceEngineError::GroupReviewBlocked {
            ref reason_code,
            gate_id: Some(_),
        } if reason_code == "shard_output_invalid"
    ));
    assert_eq!(
        provider.prompts().len(),
        1,
        "protocol errors are not retried"
    );
    let reports = store
        .list_group_review_shard_reports(&attempt.id)
        .expect("shard reports");
    assert_eq!(reports.len(), 1);
    assert_eq!(
        reports[0].run_failure_code.as_deref(),
        Some("shard_output_invalid")
    );
    assert!(reports[0].raw_provider_output_refs.is_empty());
    let role_run = store
        .list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("role runs")
        .into_iter()
        .last()
        .expect("role run");
    assert!(!reports[0].role_run_ids.is_empty());
    assert_eq!(reports[0].role_run_ids, vec![role_run.id]);
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
async fn group_final_review_cancellation_closes_attempt_role_run_and_timeline() {
    let (_root, store, attempt, engine, _provider) = group_review_runner_fixture(true).await;
    let (_command_tx, mut command_rx) = mpsc::channel(1);
    engine.cancellation.cancel();

    let error = engine
        .execute_group_final_review_with_commands(
            &attempt,
            &CancelledGroupReviewProvider,
            &mut command_rx,
        )
        .await
        .expect_err("cancelled group final review must abort");

    assert!(matches!(error, CodingWorkspaceEngineError::Aborted));
    let stored_attempt = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("attempt");
    assert_eq!(stored_attempt.status, CodingAttemptStatus::Aborted);
    let role_run = store
        .list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("role runs")
        .into_iter()
        .last()
        .expect("role run");
    assert_eq!(role_run.status, CodingRoleRunStatus::Aborted);
    let node = store
        .get_timeline_nodes(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("timeline")
        .into_iter()
        .last()
        .expect("node");
    assert_eq!(node.status, CodingTimelineNodeStatus::Failed);
    assert!(node.completed_at.is_some());
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
