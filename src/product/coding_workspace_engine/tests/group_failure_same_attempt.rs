use super::*;
use crate::product::coding_models::{CodingUnitRun, CodingUnitRunStatus};
use crate::product::work_item_projection::renderer_for;
use crate::product::work_item_revision_store::WorkItemRevisionStore;

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

fn seed_run(store: &CodingAttemptStore, attempt: &CodingExecutionAttempt, retry_count: u32) {
    let unit = store
        .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("units")
        .into_iter()
        .find(|unit| unit.status == CodingExecutionUnitStatus::Running)
        .expect("active unit");
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
        id: format!("{}_run_0001", unit.id),
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

fn retryable() -> ProviderFailureClassification {
    ProviderFailureClassification::Retryable {
        failure: RetryableProviderFailure::ConnectionInterrupted,
        reason_code: "provider_connection_interrupted".to_string(),
        message: "connection reset by peer".to_string(),
    }
}

#[tokio::test]
async fn group_failure_retry_stays_on_same_attempt_and_unit() {
    let (_root, store, attempt) = fixture();
    seed_run(&store, &attempt, 0);
    let (tx, _rx) = mpsc::channel(8);
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
}

#[tokio::test]
async fn group_failure_exhausted_enters_manual_recovery_without_new_attempt() {
    let (_root, store, attempt) = fixture();
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
}

#[tokio::test]
async fn group_failure_non_retryable_surfaces_blocked_gate_in_group_projection() {
    let (_root, store, attempt) = fixture();
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
    let gates = store
        .list_open_blocked_gates(&persisted.project_id, &persisted.issue_id, &persisted.id)
        .expect("gates");
    assert_eq!(gates.len(), 1);
    assert_eq!(
        gates[0].reason_code.as_deref(),
        Some("provider_parse_error")
    );
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
