use super::*;
use crate::product::coding_models::{
    CodingRoleRun, CodingRoleRunRetryMetadata, CodingRoleRunStatus, CodingRoleRunTrigger,
};

#[test]
fn retry_run_keeps_failed_predecessor_and_records_cycle_metadata() {
    let (_tmp, store, attempt) = setup();
    let initial = store
        .create_role_run(
            &attempt,
            CodingExecutionStage::Coding,
            CodingProviderRole::Coder,
            CodingRoleRunTrigger::Initial,
            Some("coding_node_0001".to_string()),
        )
        .expect("create initial role run");
    let initial_retry = initial
        .retry_metadata
        .clone()
        .expect("initial retry metadata");
    assert_eq!(initial_retry.cycle_id, initial.id);
    assert_eq!(initial_retry.attempt_no, 1);
    assert_eq!(initial_retry.prior_run_id, None);

    store
        .update_role_run_refs(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &initial.id,
            vec!["provider-raw/coding/initial.txt".to_string()],
            vec!["artifacts/coding/initial.json".to_string()],
        )
        .expect("persist initial refs");
    store
        .update_role_run_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &initial.id,
            CodingRoleRunStatus::Failed,
            Some("provider_interrupted".to_string()),
        )
        .expect("fail initial run");

    let second = store
        .create_retry_role_run(
            &attempt,
            CodingExecutionStage::Coding,
            CodingProviderRole::Coder,
            CodingRoleRunTrigger::AutomaticRetry,
            Some("coding_node_0001".to_string()),
            CodingRoleRunRetryMetadata {
                cycle_id: initial_retry.cycle_id.clone(),
                attempt_no: 2,
                prior_run_id: Some(initial.id.clone()),
            },
        )
        .expect("create second retry run");
    assert_eq!(second.status, CodingRoleRunStatus::Running);
    assert_eq!(
        second
            .retry_metadata
            .as_ref()
            .map(|retry| retry.prior_run_id.as_deref()),
        Some(Some(initial.id.as_str()))
    );
    assert_eq!(
        second.retry_metadata.as_ref().map(|retry| retry.attempt_no),
        Some(2)
    );

    let persisted_initial = store
        .get_role_run(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &initial.id,
        )
        .expect("reload failed predecessor");
    assert_eq!(persisted_initial.status, CodingRoleRunStatus::Failed);
    assert_eq!(
        persisted_initial.raw_provider_output_refs,
        vec!["provider-raw/coding/initial.txt".to_string()]
    );
    assert_eq!(
        persisted_initial.artifact_refs,
        vec!["artifacts/coding/initial.json".to_string()]
    );
    assert_eq!(persisted_initial.superseded_by_run_id, None);

    let duplicate_second = store.create_retry_role_run(
        &attempt,
        CodingExecutionStage::Coding,
        CodingProviderRole::Coder,
        CodingRoleRunTrigger::AutomaticRetry,
        Some("coding_node_0001".to_string()),
        CodingRoleRunRetryMetadata {
            cycle_id: initial_retry.cycle_id.clone(),
            attempt_no: 2,
            prior_run_id: Some(initial.id.clone()),
        },
    );
    assert!(
        duplicate_second.is_err(),
        "a cycle cannot contain two runs for attempt 2"
    );

    store
        .update_role_run_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &second.id,
            CodingRoleRunStatus::Failed,
            Some("provider_interrupted".to_string()),
        )
        .expect("fail second run");
    let third = store
        .create_retry_role_run(
            &attempt,
            CodingExecutionStage::Coding,
            CodingProviderRole::Coder,
            CodingRoleRunTrigger::AutomaticRetry,
            Some("coding_node_0001".to_string()),
            CodingRoleRunRetryMetadata {
                cycle_id: initial_retry.cycle_id,
                attempt_no: 3,
                prior_run_id: Some(second.id.clone()),
            },
        )
        .expect("create third retry run");
    let third_retry = third.retry_metadata.expect("third retry metadata");
    assert_eq!(third_retry.attempt_no, 3);
    assert_eq!(
        third_retry.prior_run_id.as_deref(),
        Some(second.id.as_str())
    );
    assert_eq!(third.status, CodingRoleRunStatus::Running);

    store
        .update_role_run_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &third.id,
            CodingRoleRunStatus::Failed,
            Some("provider_interrupted".to_string()),
        )
        .expect("fail third run");
    let fourth = store.create_retry_role_run(
        &attempt,
        CodingExecutionStage::Coding,
        CodingProviderRole::Coder,
        CodingRoleRunTrigger::AutomaticRetry,
        Some("coding_node_0001".to_string()),
        CodingRoleRunRetryMetadata {
            cycle_id: third_retry.cycle_id,
            attempt_no: 4,
            prior_run_id: Some(third.id.clone()),
        },
    );
    assert!(fourth.is_err(), "a retry cycle must stop after attempt 3");
}

#[test]
fn historical_role_run_without_retry_metadata_still_deserializes() {
    let historical = serde_json::json!({
        "id": "coding_role_run_0001",
        "attempt_id": "coding_attempt_0001",
        "stage": "coding",
        "role": "coder",
        "run_no": 1,
        "status": "failed",
        "trigger": "initial",
        "node_id": null,
        "started_at": "2026-08-07T00:00:00Z",
        "completed_at": "2026-08-07T00:01:00Z"
    });

    let role_run: CodingRoleRun =
        serde_json::from_value(historical).expect("legacy role run deserializes");

    assert_eq!(role_run.retry_metadata, None);
}
