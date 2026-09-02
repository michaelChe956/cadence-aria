use super::*;
use crate::product::coding_models::{
    CodingAdmissionKind, CodingAttemptScope, CodingExecutionUnitStatus, CodingUnitRun,
    CodingUnitRunStatus,
};
use crate::product::work_item_projection::renderer_for;
use crate::product::work_item_revision_store::WorkItemRevisionStore;
use std::sync::atomic::{AtomicUsize, Ordering};

fn fixture() -> (
    tempfile::TempDir,
    CodingAttemptStore,
    CodingExecutionAttempt,
) {
    let root = tempdir().expect("root");
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
        .seed_running_attempt_for_test(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("running group attempt");
    let attempt = store
        .update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::Coding,
        )
        .expect("coding group attempt");
    (root, store, attempt)
}

fn sc_advance_fixture() -> (
    tempfile::TempDir,
    CodingAttemptStore,
    CodingExecutionAttempt,
) {
    let (root, store, attempt) = fixture();
    let worktree = root.path().join("worktree");
    std::fs::create_dir_all(&worktree).expect("worktree directory");
    let mut attempt = store
        .update_attempt_worktree_path(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            worktree,
        )
        .expect("worktree");
    attempt.admission_kind = CodingAdmissionKind::ScAdvance;
    store
        .write_coding_attempt_for_test(&attempt)
        .expect("SC Advance admission");
    let attempt = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("reload SC Advance attempt");
    (root, store, attempt)
}

fn seed_run_for_unit(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    unit_id: &str,
    retry_count: u32,
) {
    seed_run_for_unit_with_id(
        store,
        attempt,
        unit_id,
        retry_count,
        &format!("{}_run_0001", unit_id),
    );
}

fn seed_run_for_unit_with_id(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    unit_id: &str,
    retry_count: u32,
    run_id: &str,
) {
    let unit = store
        .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("units")
        .into_iter()
        .find(|unit| unit.id == unit_id)
        .expect("unit");
    let revisions = WorkItemRevisionStore::new(store.paths());
    let lineage = revisions
        .get_plan_lineage(&attempt.project_id, &attempt.issue_id, &unit.plan_id)
        .expect("lineage");
    let revision = revisions
        .get_work_item_revision(
            &lineage,
            &unit.logical_work_item_id,
            &unit.work_item_revision_id,
        )
        .expect("revision");
    let bundle = revisions
        .get_work_item_projection_bundle(&lineage, &revision.work_item_projection_bundle_id)
        .expect("bundle");
    let providers = store
        .get_role_provider_config_snapshot(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("providers");
    let run = CodingUnitRun {
        id: run_id.to_string(),
        unit_id: unit.id,
        execution_no: retry_count + 1,
        work_item_revision_id: revision.id,
        resolved_handoff_revision_ids: Vec::new(),
        canonical_contract_hash: bundle.canonical_contract_hash,
        projection_bundle_id: bundle.id,
        projection_compiler_version: bundle.compiler_version,
        coder_provider_renderer_version: renderer_for(&providers.coder)
            .renderer_version()
            .to_string(),
        reviewer_provider_renderer_version: renderer_for(&providers.code_reviewer)
            .renderer_version()
            .to_string(),
        internal_reviewer_provider_renderer_version: None,
        coder_projection_hash: bundle.coder_projection_hash,
        reviewer_projection_hash: bundle.reviewer_projection_hash,
        coder_execution_context_hash: None,
        reviewer_execution_context_hash: None,
        internal_reviewer_execution_context_hash: None,
        status: CodingUnitRunStatus::Running,
        unit_rework_count: 0,
        verification_retry_count: 0,
        operational_retry_count: retry_count,
        plan_repair_count: 0,
        start_commit: None,
        completion_commit: None,
        created_at: "2026-08-31T00:00:00Z".to_string(),
        updated_at: "2026-08-31T00:00:00Z".to_string(),
    };
    store
        .create_coding_unit_run(attempt, &run)
        .expect("unit run");
}

fn seed_run(store: &CodingAttemptStore, attempt: &CodingExecutionAttempt, retry_count: u32) {
    let unit_id = attempt.active_unit_id.clone().expect("active unit");
    seed_run_for_unit(store, attempt, &unit_id, retry_count);
}

fn retryable() -> ProviderFailureClassification {
    ProviderFailureClassification::Retryable {
        failure: RetryableProviderFailure::ConnectionInterrupted,
        reason_code: "provider_connection_interrupted".to_string(),
        message: "connection reset by peer".to_string(),
    }
}

struct ScAdvanceFailureProvider {
    starts: AtomicUsize,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for ScAdvanceFailureProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        let (event_tx, event_rx) = mpsc::channel(2);
        let (command_tx, _command_rx) = mpsc::channel(2);
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::Failed {
                    message: "connection reset by peer".to_string(),
                })
                .await;
        });
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }
}

#[tokio::test]
async fn sc_advance_provider_retry_exhaustion_enters_awaiting_manual_recovery() {
    let (_root, store, attempt) = sc_advance_fixture();
    let unit_id = attempt.active_unit_id.clone().expect("active unit");
    seed_run(&store, &attempt, 0);
    let provider = ScAdvanceFailureProvider {
        starts: AtomicUsize::new(0),
    };
    let (tx, _rx) = mpsc::channel(32);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    let error = engine
        .execute_coding_with_commands(
            &attempt,
            &provider,
            &CodingExecutionContext::default(),
            &mut command_rx,
        )
        .await
        .expect_err("provider retry exhaustion should require recovery");
    assert!(matches!(
        error,
        CodingWorkspaceEngineError::ProviderStream(_)
    ));
    assert_eq!(provider.starts.load(Ordering::SeqCst), 3);
    let persisted = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("attempt");
    assert_eq!(persisted.id, attempt.id);
    assert_eq!(
        persisted.status,
        CodingAttemptStatus::AwaitingManualRecovery
    );
    let runs = store
        .list_coding_unit_runs(&persisted, &unit_id)
        .expect("unit runs");
    assert_eq!(runs.len(), 3);
    assert!(
        runs.iter()
            .all(|run| run.status == CodingUnitRunStatus::Failed)
    );
    assert!(
        store
            .list_open_blocked_gates(&persisted.project_id, &persisted.issue_id, &persisted.id)
            .expect("blocked gates")
            .is_empty()
    );
}

#[tokio::test]
async fn legacy_provider_retry_exhaustion_remains_blocked_and_gated() {
    let (_root, store, mut attempt) = sc_advance_fixture();
    attempt.admission_kind = CodingAdmissionKind::LegacyGroup;
    store
        .write_coding_attempt_for_test(&attempt)
        .expect("legacy admission");
    let provider = ScAdvanceFailureProvider {
        starts: AtomicUsize::new(0),
    };
    let (tx, _rx) = mpsc::channel(32);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    let error = engine
        .execute_coding_with_commands(
            &attempt,
            &provider,
            &CodingExecutionContext::default(),
            &mut command_rx,
        )
        .await
        .expect_err("legacy provider failure should remain blocked");
    assert!(matches!(
        error,
        CodingWorkspaceEngineError::ProviderStream(_)
    ));
    assert_eq!(provider.starts.load(Ordering::SeqCst), 3);
    let persisted = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("attempt");
    assert_eq!(persisted.status, CodingAttemptStatus::Blocked);
    assert_eq!(persisted.scope, CodingAttemptScope::WorkItemGroup);
    assert_eq!(persisted.admission_kind, CodingAdmissionKind::LegacyGroup);
    let gates = store
        .list_open_blocked_gates(&persisted.project_id, &persisted.issue_id, &persisted.id)
        .expect("gates");
    assert_eq!(gates.len(), 1);
    assert_eq!(
        gates[0].reason_code.as_deref(),
        Some("coder_provider_interrupted")
    );
    let unit_id = persisted.active_unit_id.as_deref().expect("active unit");
    let unit = store
        .list_coding_units(&persisted.project_id, &persisted.issue_id, &persisted.id)
        .expect("units")
        .into_iter()
        .find(|unit| unit.id == unit_id)
        .expect("unit");
    assert_eq!(unit.status, CodingExecutionUnitStatus::Running);
}

#[tokio::test]
async fn group_failure_retry_stays_on_same_attempt_and_unit() {
    let (_root, store, attempt) = sc_advance_fixture();
    seed_run(&store, &attempt, 0);
    let (tx, mut rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let unit_id = attempt.active_unit_id.clone().expect("active unit");
    let outcome = engine
        .handle_group_unit_failure(&attempt, &unit_id, retryable())
        .await
        .expect("retry outcome");
    assert!(matches!(
        outcome,
        GroupUnitFailureOutcome::RetrySameUnit { run_no: 2, .. }
    ));
    let persisted = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("attempt");
    assert_eq!(persisted.id, attempt.id);
    assert_eq!(persisted.active_unit_id.as_deref(), Some(unit_id.as_str()));
    let runs = store
        .list_coding_unit_runs(&persisted, &unit_id)
        .expect("runs");
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].status, CodingUnitRunStatus::Failed);
    assert_eq!(runs[1].status, CodingUnitRunStatus::Running);
    assert_eq!(runs[1].operational_retry_count, 1);
    let role_runs = store
        .list_role_runs(&persisted.project_id, &persisted.issue_id, &persisted.id)
        .expect("role runs");
    assert!(role_runs.is_empty());
    let timelines = store
        .get_timeline_nodes(&persisted.project_id, &persisted.issue_id, &persisted.id)
        .expect("timeline nodes");
    assert!(timelines.is_empty());
    assert!(rx.try_recv().is_err());
}

#[test]
fn coding_unit_run_ids_are_attempt_wide_across_multiple_units() {
    let (_root, store, attempt) = fixture();
    let units = store
        .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("units");
    assert!(units.len() >= 2);
    seed_run_for_unit_with_id(&store, &attempt, &units[0].id, 0, "coding_unit_run_0001");
    seed_run_for_unit_with_id(&store, &attempt, &units[1].id, 0, "coding_unit_run_0002");
    let retry = store
        .create_retry_coding_unit_run(&attempt, &units[0].id, "coding_unit_run_0001")
        .expect("retry run");
    assert_eq!(retry.id, "coding_unit_run_0003");
    let all_runs = store
        .list_all_coding_unit_runs(&attempt)
        .expect("all attempt runs");
    assert_eq!(all_runs.len(), 3);
    assert_eq!(
        all_runs
            .iter()
            .map(|run| run.id.as_str())
            .collect::<std::collections::HashSet<_>>()
            .len(),
        3
    );
}

#[tokio::test]
async fn group_failure_exhausted_enters_manual_recovery_without_new_attempt() {
    let (_root, store, attempt) = sc_advance_fixture();
    seed_run(&store, &attempt, 2);
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let unit_id = attempt.active_unit_id.clone().expect("active unit");
    let outcome = engine
        .handle_group_unit_failure(&attempt, &unit_id, retryable())
        .await
        .expect("manual recovery");
    assert!(matches!(
        outcome,
        GroupUnitFailureOutcome::AwaitingManualRecovery { .. }
    ));
    let persisted = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("attempt");
    assert_eq!(
        persisted.status,
        CodingAttemptStatus::AwaitingManualRecovery
    );
    assert_eq!(persisted.scope, CodingAttemptScope::WorkItemGroup);
    assert_eq!(persisted.admission_kind, CodingAdmissionKind::ScAdvance);
    assert_eq!(
        store
            .get_attempt_for_work_item_group(
                &attempt.project_id,
                &attempt.issue_id,
                "work_item_plan_0001"
            )
            .expect("group attempts")
            .expect("attempt")
            .id,
        attempt.id
    );
    assert_eq!(
        store
            .list_coding_unit_runs(&persisted, &unit_id)
            .expect("runs")
            .len(),
        1
    );
    let role_runs = store
        .list_role_runs(&persisted.project_id, &persisted.issue_id, &persisted.id)
        .expect("role runs");
    assert!(role_runs.is_empty());
    let timelines = store
        .get_timeline_nodes(&persisted.project_id, &persisted.issue_id, &persisted.id)
        .expect("timeline nodes");
    assert!(timelines.is_empty());
}

#[tokio::test]
async fn group_failure_non_retryable_surfaces_blocked_gate_in_group_projection() {
    let (_root, store, attempt) = sc_advance_fixture();
    seed_run(&store, &attempt, 0);
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let unit_id = attempt.active_unit_id.clone().expect("active unit");
    let outcome = engine
        .handle_group_unit_failure(
            &attempt,
            &unit_id,
            ProviderFailureClassification::NonRetryable {
                reason_code: "provider_parse_error".to_string(),
                interaction_wait: false,
            },
        )
        .await
        .expect("blocked outcome");
    assert!(matches!(
        outcome,
        GroupUnitFailureOutcome::AwaitingManualRecovery { .. }
    ));
    let persisted = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("attempt");
    assert_eq!(persisted.status, CodingAttemptStatus::Blocked);
    assert_eq!(persisted.scope, CodingAttemptScope::WorkItemGroup);
    assert_eq!(persisted.admission_kind, CodingAdmissionKind::ScAdvance);
    let unit = store
        .list_coding_units(&persisted.project_id, &persisted.issue_id, &persisted.id)
        .expect("units")
        .into_iter()
        .find(|unit| unit.id == unit_id)
        .expect("unit");
    assert_eq!(unit.status, CodingExecutionUnitStatus::Blocked);
    let gates = store
        .list_open_blocked_gates(&persisted.project_id, &persisted.issue_id, &persisted.id)
        .expect("gates");
    assert_eq!(gates.len(), 1);
    assert_eq!(
        gates[0].reason_code.as_deref(),
        Some("provider_parse_error")
    );
    let role_runs = store
        .list_role_runs(&persisted.project_id, &persisted.issue_id, &persisted.id)
        .expect("role runs");
    assert!(role_runs.is_empty());
}

#[tokio::test]
async fn group_abort_preserves_units_runs_commits_and_durable_events() {
    let (_root, store, attempt) = fixture();
    seed_run(&store, &attempt, 0);
    let unit_id = attempt.active_unit_id.clone().expect("active unit");
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let aborted = engine
        .handle_abort(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .await
        .expect("abort");
    assert_eq!(aborted.status, CodingAttemptStatus::Aborted);
    assert_eq!(
        store
            .list_coding_unit_runs(&aborted, &unit_id)
            .expect("runs")
            .len(),
        1
    );
    assert_eq!(
        store
            .list_coding_units(&aborted.project_id, &aborted.issue_id, &aborted.id)
            .expect("units")
            .len(),
        3
    );
    let role_runs = store
        .list_role_runs(&aborted.project_id, &aborted.issue_id, &aborted.id)
        .expect("role runs");
    assert!(role_runs.is_empty());
    let timelines = store
        .get_timeline_nodes(&aborted.project_id, &aborted.issue_id, &aborted.id)
        .expect("timeline nodes");
    assert!(timelines.is_empty());
    let replay = engine
        .handle_abort(&aborted.project_id, &aborted.issue_id, &aborted.id)
        .await
        .expect("abort replay");
    assert_eq!(replay, aborted);
}

#[tokio::test]
async fn group_failure_replay_returns_existing_terminal_attempt() {
    let (_root, store, attempt) = fixture();
    seed_run(&store, &attempt, 0);
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let aborted = engine
        .handle_abort(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .await
        .expect("abort");
    let unit_id = attempt
        .active_unit_id
        .clone()
        .expect("original active unit");
    let replay = engine
        .handle_group_unit_failure(&aborted, &unit_id, retryable())
        .await
        .expect("terminal replay");
    assert!(
        matches!(replay, GroupUnitFailureOutcome::Aborted { attempt_id, .. } if attempt_id == attempt.id)
    );
    assert_eq!(
        store
            .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("attempt"),
        aborted
    );
}
