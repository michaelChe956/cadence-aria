use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::{CodingAttemptStore, CreateCodingAttemptInput};
use crate::product::coding_workspace_engine::group_review_orchestrator::{
    FakeGroupReviewExecutor, GroupReviewExecutionError, GroupReviewExecutionResult,
    GroupReviewOrchestrator, RepairError, RepairFidelityError, validate_repair_fidelity,
};
use crate::product::models::ProviderName;
use crate::web::workspace_ws_types::ProviderConfigSnapshot;
use tempfile::TempDir;

fn execution_store() -> (TempDir, CodingAttemptStore, String) {
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
                reviewer: None,
                review_rounds: 1,
                permission_modes: Default::default(),
            },
            max_auto_rework: 1,
        })
        .expect("attempt");
    (root, store, attempt.id)
}

#[test]
fn repair_fidelity_rejects_approve_even_when_raw_marker_is_blocked() {
    let raw = "GROUP_REVIEW_VERDICT: blocked\nprovider diagnostic";
    let repaired = "GROUP_REVIEW_VERDICT: approve\n{\"verdict\":\"approve\"}";
    assert!(matches!(
        validate_repair_fidelity(raw, repaired, 8),
        Err(RepairFidelityError::ForbiddenApprove)
    ));
}

#[test]
fn repair_fidelity_requires_matching_raw_marker_and_subtraceable_findings() {
    let raw = "GROUP_REVIEW_VERDICT: request_changes\nknown defect in src/lib.rs";
    let mismatched = "GROUP_REVIEW_VERDICT: blocked\n{\"verdict\":\"blocked\"}";
    assert!(matches!(
        validate_repair_fidelity(raw, mismatched, 8),
        Err(RepairFidelityError::VerdictMismatch)
    ));
    let invented = "GROUP_REVIEW_VERDICT: request_changes\n{\"verdict\":\"request_changes\",\"findings\":[{\"message\":\"invented defect\"}]}";
    assert!(matches!(
        validate_repair_fidelity(raw, invented, 8),
        Err(RepairFidelityError::FindingNotSubtraceable)
    ));
}

#[test]
fn repair_fidelity_rejects_missing_marker_and_excess_findings() {
    let raw = "no marker\nknown issue";
    let repaired = "GROUP_REVIEW_VERDICT: blocked\n{\"verdict\":\"blocked\"}";
    assert!(matches!(
        validate_repair_fidelity(raw, repaired, 8),
        Err(RepairFidelityError::MissingMarker)
    ));
    let raw = "GROUP_REVIEW_VERDICT: blocked\nknown issue";
    let findings = (0..9)
        .map(|_| serde_json::json!({"message":"known issue"}))
        .collect::<Vec<_>>();
    let repaired = format!(
        "GROUP_REVIEW_VERDICT: blocked\n{}",
        serde_json::json!({"verdict":"blocked", "findings":findings})
    );
    assert!(matches!(
        validate_repair_fidelity(raw, &repaired, 8),
        Err(RepairFidelityError::TooManyFindings)
    ));
}

#[test]
fn repair_fidelity_rejects_forged_canonical_evidence_message() {
    let raw = "GROUP_REVIEW_VERDICT: blocked\nknown issue\nhunk_hash\nknown evidence message";
    let repaired = "GROUP_REVIEW_VERDICT: blocked\n{\"verdict\":\"blocked\",\"findings\":[{\"message\":\"known issue\",\"evidence\":[{\"kind\":\"hunk\",\"source_ref\":\"hunk_hash\",\"message\":\"forged evidence message\"}]}]}";
    assert!(matches!(
        validate_repair_fidelity(raw, repaired, 8),
        Err(RepairFidelityError::EvidenceNotSubtraceable)
    ));
}

#[test]
fn repair_fidelity_rejects_forged_repair_target_kind() {
    let raw = "GROUP_REVIEW_VERDICT: blocked\nknown issue\ncurrent_work_item\nwork_item_0001\nrevision_0001";
    let repaired = "GROUP_REVIEW_VERDICT: blocked\n{\"verdict\":\"blocked\",\"findings\":[{\"message\":\"known issue\",\"repair_target\":{\"kind\":\"upstream_work_item\",\"logical_work_item_ids\":[\"work_item_0001\"],\"work_item_revision_ids\":[\"revision_0001\"]}}]}";
    assert!(matches!(
        validate_repair_fidelity(raw, repaired, 8),
        Err(RepairFidelityError::TargetNotSubtraceable)
    ));
}

#[test]
fn repair_fidelity_rejects_non_subset_evidence_and_target() {
    let raw =
        "GROUP_REVIEW_VERDICT: blocked\nknown issue\nevidence_a\nwork_item_0001\nrevision_0001";
    let invented_evidence = "GROUP_REVIEW_VERDICT: blocked\n{\"verdict\":\"blocked\",\"findings\":[{\"message\":\"known issue\",\"evidence\":[\"invented_evidence\"]}]}";
    assert!(matches!(
        validate_repair_fidelity(raw, invented_evidence, 8),
        Err(RepairFidelityError::EvidenceNotSubtraceable)
    ));
    let invented_target = "GROUP_REVIEW_VERDICT: blocked\n{\"verdict\":\"blocked\",\"findings\":[{\"message\":\"known issue\",\"repair_target\":{\"kind\":\"current_work_item\",\"logical_work_item_ids\":[\"other_work_item\"],\"work_item_revision_ids\":[\"other_revision\"]}}]}";
    assert!(matches!(
        validate_repair_fidelity(raw, invented_target, 8),
        Err(RepairFidelityError::TargetNotSubtraceable)
    ));
}

#[tokio::test]
async fn transport_failure_retries_without_repair_and_cancellation_does_not_retry() {
    let executor = FakeGroupReviewExecutor::new(vec![
        Err(GroupReviewExecutionError::Transport("timeout".to_string())),
        Ok(GroupReviewExecutionResult {
            full_output: "success".to_string(),
            provider_session_id: None,
        }),
    ]);
    let (_root, store, _attempt_id) = execution_store();
    let orchestrator = GroupReviewOrchestrator::new(&executor, &store);
    assert_eq!(
        orchestrator
            .execute_with_retry("normal prompt", 2)
            .await
            .expect("second execution succeeds"),
        "success"
    );
    assert_eq!(executor.prompts(), vec!["normal prompt", "normal prompt"]);

    let cancelled = FakeGroupReviewExecutor::new(vec![
        Err(GroupReviewExecutionError::UserCancelled),
        Ok(GroupReviewExecutionResult {
            full_output: "must not run".to_string(),
            provider_session_id: None,
        }),
    ]);
    let orchestrator = GroupReviewOrchestrator::new(&cancelled, &store);
    assert!(matches!(
        orchestrator.execute_with_retry("prompt", 3).await,
        Err(GroupReviewExecutionError::UserCancelled)
    ));
    assert_eq!(cancelled.prompts(), vec!["prompt"]);
}

#[tokio::test]
async fn internal_error_does_not_retry() {
    let executor = FakeGroupReviewExecutor::new(vec![
        Err(GroupReviewExecutionError::Internal(
            "adapter error".to_string(),
        )),
        Ok(GroupReviewExecutionResult {
            full_output: "must not run".to_string(),
            provider_session_id: None,
        }),
    ]);
    let (_root, store, _attempt_id) = execution_store();
    let orchestrator = GroupReviewOrchestrator::new(&executor, &store);

    assert!(matches!(
        orchestrator.execute_with_retry("prompt", 3).await,
        Err(GroupReviewExecutionError::Internal(message)) if message == "adapter error"
    ));
    assert_eq!(executor.prompts(), vec!["prompt"]);
}

#[tokio::test]
async fn repair_returns_output_that_can_be_persisted_as_raw_and_repaired_audit_refs() {
    let (_root, store, attempt_id) = execution_store();
    let attempt = store.find_attempt_by_id(&attempt_id).expect("attempt");
    let raw = "GROUP_REVIEW_VERDICT: blocked\nknown issue";
    let repaired = "GROUP_REVIEW_VERDICT: blocked\n{\"verdict\":\"blocked\",\"findings\":[{\"message\":\"known issue\"}]}";
    let executor = FakeGroupReviewExecutor::new(vec![Ok(GroupReviewExecutionResult {
        full_output: repaired.to_string(),
        provider_session_id: None,
    })]);
    let orchestrator = GroupReviewOrchestrator::new(&executor, &store);
    let repair = orchestrator.execute_repair(raw, 8).await.expect("repair");
    let refs = orchestrator
        .persist_repair_outputs(&attempt, raw, &repair)
        .expect("persist both audit artifacts");
    assert_eq!(refs.len(), 2);
    assert_ne!(refs[0], refs[1]);
}

#[tokio::test]
async fn retry_limit_returns_last_transport_error_and_repair_checks_input_cap() {
    let executor = FakeGroupReviewExecutor::new(vec![
        Err(GroupReviewExecutionError::Transport("one".to_string())),
        Err(GroupReviewExecutionError::Transport("two".to_string())),
    ]);
    let (_root, store, _attempt_id) = execution_store();
    let orchestrator = GroupReviewOrchestrator::new(&executor, &store);
    assert!(matches!(
        orchestrator.execute_with_retry("prompt", 2).await,
        Err(GroupReviewExecutionError::Transport(message)) if message == "two"
    ));
    let repair_executor = FakeGroupReviewExecutor::new(Vec::new());
    let repair = GroupReviewOrchestrator::new(&repair_executor, &store);
    assert!(matches!(
        repair.execute_repair(&"x".repeat(17 * 1024), 8).await,
        Err(RepairError::InputTooLarge)
    ));
    assert!(repair_executor.prompts().is_empty());
}
