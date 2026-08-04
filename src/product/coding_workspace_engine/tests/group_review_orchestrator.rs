use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::{CodingAttemptStore, CreateCodingAttemptInput};
use crate::product::coding_models::{
    CasOutcome, GroupReviewReductionReport, GroupReviewShardReport, ReviewProvenance, ReviewVerdict,
};
use crate::product::coding_workspace_engine::group_review_orchestrator::{
    FakeGroupReviewExecutor, GroupReviewExecutionError, GroupReviewExecutionResult,
    GroupReviewExecutor, GroupReviewOrchestrationError, GroupReviewOrchestrator,
};
use crate::product::coding_workspace_engine::group_review_types::{
    CompactContractInterface, CompactRoutingTarget, ContractEdge, DeterministicGroupFinding,
    DiffFileEntry, DiffHunk, GroupDiffIndex, GroupPartitionResult, GroupReviewGraph,
    GroupReviewMaterialSnapshot, GroupShardSpec, PromptBudgetBreakdown, ReductionDiffSelection,
    RequirementCoverage, ScopeOverlap, ShardDiffSelection, UnitCrossReviewRecord,
    UnitEvidenceSummary, UnitScopeSummary,
};
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
fn group_review_lease_claim_is_single_flight_and_records_completion() {
    let (_root, store, attempt_id) = setup();

    let lease_id = store
        .claim_group_review_lease(&attempt_id, "snapshot_a", "shard", "shard_0001")
        .expect("claim lease")
        .expect("first caller owns lease");
    assert_eq!(
        store
            .claim_group_review_lease(&attempt_id, "snapshot_a", "shard", "shard_0001")
            .expect("second claim"),
        None
    );
    assert_eq!(
        store
            .get_completed_group_review_result(&attempt_id, "snapshot_a", "shard", "shard_0001")
            .expect("no completed result"),
        None
    );
    store
        .release_group_review_lease(&attempt_id, &lease_id, "group_review_shard_shard_0001")
        .expect("release lease");
    assert_eq!(
        store
            .get_completed_group_review_result(&attempt_id, "snapshot_a", "shard", "shard_0001")
            .expect("completed result"),
        Some("group_review_shard_shard_0001".to_string())
    );
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

fn material_snapshot(
    unit_count: usize,
    shard_count: usize,
    diff_body: &str,
) -> GroupReviewMaterialSnapshot {
    let unit_records = (0..unit_count)
        .map(|index| UnitCrossReviewRecord {
            unit_id: format!("unit_{index:04}"),
            unit_run_id: format!("run_{index:04}"),
            logical_work_item_id: format!("work_item_{index:04}"),
            work_item_revision_id: format!("revision_{index:04}"),
            completion_commit: format!("commit_{index:04}"),
            dependency_ids: Vec::new(),
            scope_summary: UnitScopeSummary {
                exclusive_scopes: Vec::new(),
                forbidden_scopes: Vec::new(),
            },
            contract_interfaces: Vec::<CompactContractInterface>::new(),
            evidence_summary: UnitEvidenceSummary {
                required_command_count: 0,
                executed_command_count: 0,
                manual_check_count: 0,
                missing_refs: Vec::new(),
            },
            routing_targets: Vec::<CompactRoutingTarget>::new(),
        })
        .collect::<Vec<_>>();
    let shards = (0..shard_count)
        .map(|index| GroupShardSpec {
            shard_id: format!("shard_{index:04}"),
            ordered_unit_run_ids: vec![format!("run_{index:04}")],
            partition_rationale: vec!["stable_order".to_string()],
        })
        .collect::<Vec<_>>();
    let shard_selections = shards
        .iter()
        .map(|shard| ShardDiffSelection {
            shard_id: shard.shard_id.clone(),
            fragments: vec![
                crate::product::coding_workspace_engine::group_review_types::SelectedDiffFragment {
                    path: "src/lib.rs".to_string(),
                    level: 'E',
                    body: diff_body.to_string(),
                    hunk_content_hash: "hunk_hash".to_string(),
                    redacted: false,
                    truncated: false,
                    not_shown_count: 0,
                },
            ],
            total_hunks_in_shard: 1,
        })
        .collect();
    GroupReviewMaterialSnapshot {
        schema_version: 1,
        compiler_version: "test".to_string(),
        attempt_id: "placeholder".to_string(),
        review_request_id: "review_request_0001".to_string(),
        base_branch: "main".to_string(),
        final_commit: "final_commit".to_string(),
        authoritative_binding_digest: "binding_digest".to_string(),
        unit_records,
        global_graph: GroupReviewGraph {
            contract_edges: Vec::<ContractEdge>::new(),
            scope_overlaps: Vec::<ScopeOverlap>::new(),
            commit_reachability:
                crate::product::coding_workspace_engine::group_review_types::CommitReachability {
                    reachable_completion_commits: Vec::new(),
                    unreachable_completion_commits: Vec::new(),
                },
            requirement_coverage: RequirementCoverage {
                covered: Vec::new(),
                missing: Vec::new(),
                conflicting: Vec::new(),
            },
        },
        diff_index: GroupDiffIndex {
            files: vec![DiffFileEntry {
                path: "src/lib.rs".to_string(),
                insertions: 1,
                deletions: 0,
                owner_unit_run_ids: Vec::new(),
                shared: false,
                ambiguous: false,
                forbidden_scope_hit: false,
            }],
            hunks: vec![DiffHunk {
                hunk_index: 0,
                path: "src/lib.rs".to_string(),
                owner_unit_run_ids: Vec::new(),
                header: "@@".to_string(),
                body: diff_body.to_string(),
                redacted: false,
                content_hash: "hunk_hash".to_string(),
            }],
            shard_selections,
            reduction_selection: ReductionDiffSelection {
                fragments: Vec::new(),
                total_cross_shard_hunks: 0,
            },
        },
        deterministic_findings: Vec::<DeterministicGroupFinding>::new(),
        partition_result: GroupPartitionResult {
            shards,
            cross_shard_edges: Vec::new(),
        },
        content_hash: "snapshot_hash".to_string(),
    }
}

fn valid_review_output(finding_count: usize) -> String {
    let findings = (0..finding_count)
        .map(|index| serde_json::json!({"message": format!("finding {index}")}))
        .collect::<Vec<_>>();
    format!(
        "GROUP_REVIEW_VERDICT\n{}",
        serde_json::json!({
            "verdict": "approve",
            "summary": "approved",
            "findings": findings,
        })
    )
}

#[tokio::test]
async fn execute_shards_rejects_capacity_before_executor_call() {
    let (_root, store, attempt_id) = setup();
    let mut snapshot = material_snapshot(21, 1, "diff");
    snapshot.attempt_id = attempt_id;
    let executor = FakeGroupReviewExecutor::new(Vec::new());

    let error = GroupReviewOrchestrator::new(&executor, &store)
        .execute_shards(&snapshot)
        .await
        .expect_err("capacity must fail closed");

    assert!(matches!(
        error,
        GroupReviewOrchestrationError::CapacityExceeded
    ));
    assert!(executor.prompts().is_empty());
}

#[tokio::test]
async fn execute_shards_rejects_overflow_before_executor_call() {
    let (_root, store, attempt_id) = setup();
    let mut snapshot = material_snapshot(1, 1, &"x".repeat(31 * 1024));
    snapshot.attempt_id = attempt_id;
    let executor = FakeGroupReviewExecutor::new(Vec::new());

    let error = GroupReviewOrchestrator::new(&executor, &store)
        .execute_shards(&snapshot)
        .await
        .expect_err("overflow must fail closed");

    assert!(matches!(
        error,
        GroupReviewOrchestrationError::MaterialOverflow { .. }
    ));
    assert!(executor.prompts().is_empty());
}

#[tokio::test]
async fn execute_shards_parses_persists_and_returns_reports() {
    let (_root, store, attempt_id) = setup();
    let mut snapshot = material_snapshot(1, 1, "diff");
    snapshot.attempt_id = attempt_id.clone();
    store
        .activate_group_review_snapshot(&attempt_id, &snapshot.content_hash)
        .expect("activate snapshot");
    let executor = FakeGroupReviewExecutor::new(vec![Ok(GroupReviewExecutionResult {
        full_output: valid_review_output(0),
        provider_session_id: Some("session_0001".to_string()),
    })]);

    let reports = GroupReviewOrchestrator::new(&executor, &store)
        .execute_shards(&snapshot)
        .await
        .expect("execute shard");

    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].verdict, ReviewVerdict::Approve);
    assert_eq!(reports[0].run_failure_code, None);
    assert_eq!(reports[0].raw_provider_output_refs.len(), 1);
    assert_eq!(
        store
            .list_group_review_shard_reports(&attempt_id)
            .expect("stored reports")
            .len(),
        1
    );
}

#[tokio::test]
async fn execute_shards_reuses_completed_snapshot_reports_without_provider_calls() {
    let (_root, store, attempt_id) = setup();
    let mut snapshot = material_snapshot(1, 1, "diff");
    snapshot.attempt_id = attempt_id.clone();
    store
        .activate_group_review_snapshot(&attempt_id, &snapshot.content_hash)
        .expect("activate snapshot");
    let first_executor = FakeGroupReviewExecutor::new(vec![Ok(GroupReviewExecutionResult {
        full_output: valid_review_output(0),
        provider_session_id: None,
    })]);
    GroupReviewOrchestrator::new(&first_executor, &store)
        .execute_shards(&snapshot)
        .await
        .expect("initial execution");

    let retry_executor = FakeGroupReviewExecutor::new(Vec::new());
    let reports = GroupReviewOrchestrator::new(&retry_executor, &store)
        .execute_shards(&snapshot)
        .await
        .expect("reuse completed reports");
    assert_eq!(reports.len(), 1);
    assert!(retry_executor.prompts().is_empty());
}

#[tokio::test]
async fn execute_shards_returns_in_progress_without_duplicate_provider_call() {
    let (_root, store, attempt_id) = setup();
    let mut snapshot = material_snapshot(1, 1, "diff");
    snapshot.attempt_id = attempt_id.clone();
    store
        .activate_group_review_snapshot(&attempt_id, &snapshot.content_hash)
        .expect("activate snapshot");
    let shard_id = &snapshot.partition_result.shards[0].shard_id;
    store
        .claim_group_review_lease(&attempt_id, &snapshot.content_hash, "shard", shard_id)
        .expect("claim lease")
        .expect("lease owner");
    let executor = FakeGroupReviewExecutor::new(Vec::new());

    let error = GroupReviewOrchestrator::new(&executor, &store)
        .execute_shards(&snapshot)
        .await
        .expect_err("duplicate invocation must not execute");
    assert!(matches!(
        error,
        GroupReviewOrchestrationError::ShardInProgress { .. }
    ));
    assert!(executor.prompts().is_empty());
}

#[tokio::test]
async fn execute_shards_supersedes_old_snapshot_and_stores_late_result_as_stale() {
    let (_root, store, attempt_id) = setup();
    let mut old_snapshot = material_snapshot(1, 1, "old diff");
    old_snapshot.attempt_id = attempt_id.clone();
    let mut new_snapshot = material_snapshot(1, 1, "new diff");
    new_snapshot.attempt_id = attempt_id.clone();
    new_snapshot.content_hash = "snapshot_new".to_string();
    store
        .activate_group_review_snapshot(&attempt_id, &new_snapshot.content_hash)
        .expect("supersede snapshot");

    let executor = FakeGroupReviewExecutor::new(vec![Ok(GroupReviewExecutionResult {
        full_output: valid_review_output(0),
        provider_session_id: None,
    })]);
    let reports = GroupReviewOrchestrator::new(&executor, &store)
        .execute_shards(&old_snapshot)
        .await
        .expect("late old snapshot is audit-only");
    assert!(reports.is_empty());
    assert_eq!(
        store
            .get_active_group_review_snapshot_hash(&attempt_id)
            .expect("active snapshot"),
        Some(new_snapshot.content_hash)
    );
    assert!(
        store
            .list_group_review_shard_reports(&attempt_id)
            .expect("live reports")
            .is_empty()
    );
}

#[tokio::test]
async fn failed_shard_result_releases_every_preclaimed_lease() {
    let (_root, store, attempt_id) = setup();
    let mut snapshot = material_snapshot(2, 2, "diff");
    snapshot.attempt_id = attempt_id.clone();
    store
        .activate_group_review_snapshot(&attempt_id, &snapshot.content_hash)
        .expect("activate snapshot");
    let executor = FakeGroupReviewExecutor::new(vec![
        Ok(GroupReviewExecutionResult {
            full_output: valid_review_output(9),
            provider_session_id: None,
        }),
        Ok(GroupReviewExecutionResult {
            full_output: valid_review_output(0),
            provider_session_id: None,
        }),
    ]);

    let error = GroupReviewOrchestrator::new(&executor, &store)
        .execute_shards(&snapshot)
        .await
        .expect_err("invalid first shard result");
    assert!(matches!(
        error,
        GroupReviewOrchestrationError::ShardOutputInvalid { .. }
    ));
    for shard in &snapshot.partition_result.shards {
        assert!(
            store
                .claim_group_review_lease(
                    &attempt_id,
                    &snapshot.content_hash,
                    "shard",
                    &shard.shard_id
                )
                .expect("claim after failure")
                .is_some()
        );
    }
}

#[tokio::test]
async fn execute_shards_rejects_more_than_eight_findings() {
    let (_root, store, attempt_id) = setup();
    let mut snapshot = material_snapshot(1, 1, "diff");
    snapshot.attempt_id = attempt_id;
    let executor = FakeGroupReviewExecutor::new(vec![Ok(GroupReviewExecutionResult {
        full_output: valid_review_output(9),
        provider_session_id: None,
    })]);

    let error = GroupReviewOrchestrator::new(&executor, &store)
        .execute_shards(&snapshot)
        .await
        .expect_err("finding overflow must be invalid output");

    assert!(matches!(
        error,
        GroupReviewOrchestrationError::ShardOutputInvalid { .. }
    ));
}

struct ConcurrentExecutor {
    active: AtomicUsize,
    peak: AtomicUsize,
}

#[async_trait::async_trait]
impl GroupReviewExecutor for ConcurrentExecutor {
    async fn execute(
        &self,
        _prompt: &str,
    ) -> Result<GroupReviewExecutionResult, GroupReviewExecutionError> {
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(active, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(30)).await;
        self.active.fetch_sub(1, Ordering::SeqCst);
        Ok(GroupReviewExecutionResult {
            full_output: valid_review_output(0),
            provider_session_id: None,
        })
    }
}

#[tokio::test]
async fn execute_shards_limits_concurrency_to_two() {
    let (_root, store, attempt_id) = setup();
    let mut snapshot = material_snapshot(3, 3, "diff");
    snapshot.attempt_id = attempt_id.clone();
    store
        .activate_group_review_snapshot(&attempt_id, &snapshot.content_hash)
        .expect("activate snapshot");
    let executor = Arc::new(ConcurrentExecutor {
        active: AtomicUsize::new(0),
        peak: AtomicUsize::new(0),
    });

    let reports = GroupReviewOrchestrator::new(executor.as_ref(), &store)
        .execute_shards(&snapshot)
        .await
        .expect("execute shards");

    assert_eq!(reports.len(), 3);
    assert_eq!(executor.peak.load(Ordering::SeqCst), 2);
}
