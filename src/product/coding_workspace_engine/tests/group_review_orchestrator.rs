use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::{CodingAttemptStore, CreateCodingAttemptInput};
use crate::product::coding_models::{
    CasOutcome, GroupReviewReductionReport, GroupReviewShardReport, ReviewProvenance, ReviewVerdict,
};
use crate::product::coding_workspace_engine::group_review_orchestrator::{
    FakeGroupReviewExecutor, GroupReviewExecutionError, GroupReviewExecutionResult,
    GroupReviewExecutor, GroupReviewOrchestrationError,
};
use crate::product::coding_workspace_engine::group_review_types::PromptBudgetBreakdown;
use crate::product::models::ProviderName;
use crate::web::workspace_ws_types::ProviderConfigSnapshot;
use tempfile::TempDir;

fn setup() -> (TempDir, CodingAttemptStore, String) {
    let root = TempDir::new().expect("tempdir");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: "work_item_0001".to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/work-item".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
                permission_modes: Default::default(),
            },
            max_auto_rework: 2,
        })
        .expect("attempt");
    (root, store, attempt.id)
}

fn shard_report(attempt_id: &str, snapshot_hash: &str) -> GroupReviewShardReport {
    GroupReviewShardReport {
        id: "group_review_shard_0001".to_string(),
        attempt_id: attempt_id.to_string(),
        snapshot_hash: snapshot_hash.to_string(),
        shard_id: "shard_0001".to_string(),
        ordered_unit_run_ids: vec!["run_0001".to_string()],
        partition_rationale: vec!["stable_order".to_string()],
        verdict: ReviewVerdict::Approve,
        findings: Vec::new(),
        unresolved_obligations: Vec::new(),
        selected_diff_refs: vec!["diff_0001".to_string()],
        raw_provider_output_refs: vec!["raw_0001".to_string()],
        role_run_ids: vec!["role_run_0001".to_string()],
        run_failure_code: None,
    }
}

fn reduction_report(attempt_id: &str, snapshot_hash: &str) -> GroupReviewReductionReport {
    GroupReviewReductionReport {
        id: "group_review_reduction_0001".to_string(),
        attempt_id: attempt_id.to_string(),
        snapshot_hash: snapshot_hash.to_string(),
        shard_report_ids: vec!["group_review_shard_0001".to_string()],
        verdict: ReviewVerdict::Approve,
        findings: Vec::new(),
        impact_scope: Vec::new(),
        pr_description: String::new(),
        commit_message_suggestion: String::new(),
        provenance: vec![ReviewProvenance {
            source_kind: "shard".to_string(),
            source_id: "group_review_shard_0001".to_string(),
            finding_index: 0,
        }],
        raw_provider_output_refs: vec!["raw_0001".to_string()],
        role_run_ids: vec!["role_run_0001".to_string()],
        run_failure_code: None,
    }
}

#[test]
fn activates_and_reads_group_review_snapshot_hash() {
    let (_root, store, attempt_id) = setup();

    assert_eq!(
        store
            .get_active_group_review_snapshot_hash(&attempt_id)
            .expect("read inactive snapshot"),
        None
    );
    store
        .activate_group_review_snapshot(&attempt_id, "snapshot_a")
        .expect("activate snapshot");
    assert_eq!(
        store
            .get_active_group_review_snapshot_hash(&attempt_id)
            .expect("read active snapshot"),
        Some("snapshot_a".to_string())
    );
}

#[test]
fn writes_group_review_reports_when_report_snapshot_is_active() {
    let (_root, store, attempt_id) = setup();
    store
        .activate_group_review_snapshot(&attempt_id, "snapshot_a")
        .expect("activate snapshot");

    assert_eq!(
        store
            .write_group_review_shard_report_cas(
                &attempt_id,
                shard_report(&attempt_id, "snapshot_a")
            )
            .expect("write shard"),
        CasOutcome::Written
    );
    assert_eq!(
        store
            .write_group_review_reduction_report_cas(
                &attempt_id,
                reduction_report(&attempt_id, "snapshot_a"),
            )
            .expect("write reduction"),
        CasOutcome::Written
    );
    assert_eq!(
        store
            .list_group_review_shard_reports(&attempt_id)
            .expect("list shard reports")
            .len(),
        1
    );
    assert_eq!(
        store
            .list_group_review_reduction_reports(&attempt_id)
            .expect("list reduction reports")
            .len(),
        1
    );
}

#[test]
fn stores_stale_reports_without_changing_active_snapshot() {
    let (_root, store, attempt_id) = setup();
    store
        .activate_group_review_snapshot(&attempt_id, "snapshot_b")
        .expect("activate snapshot");

    assert_eq!(
        store
            .write_group_review_shard_report_cas(
                &attempt_id,
                shard_report(&attempt_id, "snapshot_a")
            )
            .expect("store stale shard"),
        CasOutcome::StoredStale
    );
    assert_eq!(
        store
            .write_group_review_reduction_report_cas(
                &attempt_id,
                reduction_report(&attempt_id, "snapshot_a"),
            )
            .expect("store stale reduction"),
        CasOutcome::StoredStale
    );
    assert_eq!(
        store
            .get_active_group_review_snapshot_hash(&attempt_id)
            .expect("active remains unchanged"),
        Some("snapshot_b".to_string())
    );
    assert!(
        store
            .list_group_review_shard_reports(&attempt_id)
            .expect("list reports")
            .is_empty()
    );
}

#[tokio::test]
async fn fake_executor_and_orchestration_errors_are_constructible() {
    let executor = FakeGroupReviewExecutor::new(vec![Ok(GroupReviewExecutionResult {
        full_output: "output".to_string(),
        provider_session_id: Some("session_0001".to_string()),
    })]);

    assert_eq!(
        executor.execute("prompt").await.expect("fake result"),
        GroupReviewExecutionResult {
            full_output: "output".to_string(),
            provider_session_id: Some("session_0001".to_string()),
        }
    );
    executor.push_result(Err(GroupReviewExecutionError::Transport(
        "offline".to_string(),
    )));
    assert!(matches!(
        executor.execute("second").await,
        Err(GroupReviewExecutionError::Transport(message)) if message == "offline"
    ));
    assert_eq!(
        executor.prompts(),
        vec!["prompt".to_string(), "second".to_string()]
    );

    let error = GroupReviewOrchestrationError::MaterialOverflow {
        breakdown: PromptBudgetBreakdown {
            fixed_protocol: 1,
            identity: 0,
            unit_records: 0,
            evidence_digest: 0,
            graph: 0,
            diff: 0,
            retry_diagnostic_reserve: 0,
            total: 1,
        },
    };
    assert_eq!(error.to_string(), "material_overflow");
}
