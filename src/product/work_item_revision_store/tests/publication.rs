use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::Duration;

use crate::product::json_store::{read_json, write_json};

use super::super::{ExclusiveFileLock, lock_path_for, register_lock_attempt_hook};
use super::*;

const LOCKED_TIMEOUT: Duration = Duration::from_millis(200);
const COMPLETION_TIMEOUT: Duration = Duration::from_secs(5);

fn publication_journal(
    amendment_id: &str,
    phase: PlanAmendmentPublicationPhase,
) -> PlanAmendmentPublicationJournal {
    PlanAmendmentPublicationJournal {
        id: format!("{amendment_id}_publication_journal"),
        project_id: PROJECT_ID.to_string(),
        issue_id: ISSUE_ID.to_string(),
        plan_id: PLAN_ID.to_string(),
        amendment_id: amendment_id.to_string(),
        request_id: "plan_repair_request_0001".to_string(),
        base_plan_revision_id: "plan_revision_0001".to_string(),
        new_plan_revision_id: "plan_revision_0002".to_string(),
        confirmation: None,
        artifact_fingerprint: "fingerprint_0001".to_string(),
        snapshot: None,
        phase,
        error: None,
        recovery: None,
        created_at: "2026-07-17T00:00:13Z".to_string(),
        updated_at: "2026-07-17T00:00:13Z".to_string(),
    }
}

#[test]
fn work_item_revision_publication_journal_allows_only_forward_idempotent_transitions() {
    let (_temp, store, plan) = test_store_and_plan();
    let journal = publication_journal("amendment_0001", PlanAmendmentPublicationPhase::Prepared);
    store
        .put_plan_amendment_publication_journal(&plan, &journal)
        .unwrap();

    let published = store
        .advance_plan_amendment_publication(
            &plan,
            &journal.id,
            PlanAmendmentPublicationPhase::PlanPublished,
        )
        .unwrap();
    assert_eq!(
        published.phase,
        PlanAmendmentPublicationPhase::PlanPublished
    );
    assert_eq!(published.error, None);

    let replayed = store
        .advance_plan_amendment_publication(
            &plan,
            &journal.id,
            PlanAmendmentPublicationPhase::PlanPublished,
        )
        .unwrap();
    assert_eq!(replayed, published);

    let error = store
        .advance_plan_amendment_publication(
            &plan,
            &journal.id,
            PlanAmendmentPublicationPhase::Prepared,
        )
        .unwrap_err();
    assert!(error.to_string().contains("amendment_phase_regression"));
}

#[test]
fn plan_repair_publication_journal_preparing_phase_is_idempotent_before_artifacts() {
    let (_temp, store, plan) = test_store_and_plan();
    let journal = publication_journal(
        "amendment_preparing_0001",
        PlanAmendmentPublicationPhase::Preparing,
    );
    store
        .put_plan_amendment_publication_journal(&plan, &journal)
        .unwrap();

    let prepared = store
        .advance_plan_amendment_publication(
            &plan,
            &journal.id,
            PlanAmendmentPublicationPhase::Prepared,
        )
        .unwrap();
    assert_eq!(prepared.phase, PlanAmendmentPublicationPhase::Prepared);
    assert_eq!(
        store
            .advance_plan_amendment_publication(
                &plan,
                &journal.id,
                PlanAmendmentPublicationPhase::Prepared,
            )
            .unwrap(),
        prepared
    );
}

#[test]
fn work_item_revision_publication_journal_failure_preserves_last_successful_phase() {
    let (_temp, store, plan) = test_store_and_plan();
    let journal = publication_journal("amendment_0001", PlanAmendmentPublicationPhase::Prepared);
    store
        .put_plan_amendment_publication_journal(&plan, &journal)
        .unwrap();

    let failed = store
        .mark_plan_amendment_publication_failed(
            &plan,
            &journal.id,
            "plan publish failed".to_string(),
        )
        .unwrap();

    assert_eq!(failed.phase, PlanAmendmentPublicationPhase::Prepared);
    assert_eq!(failed.error.as_deref(), Some("plan publish failed"));
    assert_eq!(failed.created_at, journal.created_at);
    assert_ne!(failed.updated_at, journal.updated_at);
}

#[test]
fn work_item_revision_publication_journal_success_clears_persisted_error() {
    let (_temp, store, plan) = test_store_and_plan();
    let journal = publication_journal("amendment_0001", PlanAmendmentPublicationPhase::Prepared);
    store
        .put_plan_amendment_publication_journal(&plan, &journal)
        .unwrap();
    store
        .mark_plan_amendment_publication_failed(
            &plan,
            &journal.id,
            "plan publish failed".to_string(),
        )
        .unwrap();

    let recovered = store
        .advance_plan_amendment_publication(
            &plan,
            &journal.id,
            PlanAmendmentPublicationPhase::PlanPublished,
        )
        .unwrap();

    assert_eq!(
        recovered.phase,
        PlanAmendmentPublicationPhase::PlanPublished
    );
    assert_eq!(recovered.error, None);
}

#[test]
fn work_item_revision_publication_journal_rejects_corrupted_path_identity() {
    let (_temp, store, plan) = test_store_and_plan();
    let journal = publication_journal("amendment_0001", PlanAmendmentPublicationPhase::Prepared);
    store
        .put_plan_amendment_publication_journal(&plan, &journal)
        .unwrap();
    let path = store.amendment_publication_journal_path(
        &plan.project_id,
        &plan.issue_id,
        &plan.id,
        &journal.id,
    );
    let mut corrupted = journal.clone();
    corrupted.id = "amendment_0002_publication_journal".to_string();
    write_json(&path, &corrupted).unwrap();

    let error = store
        .advance_plan_amendment_publication(
            &plan,
            &journal.id,
            PlanAmendmentPublicationPhase::PlanPublished,
        )
        .unwrap_err();

    assert!(matches!(error, ProductStoreError::IdentityMismatch { .. }));
    assert_eq!(
        read_json::<PlanAmendmentPublicationJournal>(&path).unwrap(),
        corrupted
    );

    let mut corrupted = journal.clone();
    corrupted.plan_id = "work_item_plan_other".to_string();
    write_json(&path, &corrupted).unwrap();
    let error = store
        .mark_plan_amendment_publication_failed(
            &plan,
            &journal.id,
            "plan publish failed".to_string(),
        )
        .unwrap_err();
    assert!(matches!(error, ProductStoreError::IdentityMismatch { .. }));
    assert_eq!(
        read_json::<PlanAmendmentPublicationJournal>(&path).unwrap(),
        corrupted
    );
}

#[test]
fn work_item_revision_publication_journal_validates_id_before_lock_side_effects() {
    let (_temp, store, plan) = test_store_and_plan();
    let invalid_id = "../escaped-journal";
    let target_path = store.amendment_publication_journal_path(
        &plan.project_id,
        &plan.issue_id,
        &plan.id,
        invalid_id,
    );
    let lock_path = lock_path_for(&target_path);
    let journal_root = store
        .plan_root(&plan.project_id, &plan.issue_id, &plan.id)
        .join("amendment-publication-journals");

    let advance_error = store
        .advance_plan_amendment_publication(
            &plan,
            invalid_id,
            PlanAmendmentPublicationPhase::PlanPublished,
        )
        .unwrap_err();
    let failed_error = store
        .mark_plan_amendment_publication_failed(
            &plan,
            invalid_id,
            "plan publish failed".to_string(),
        )
        .unwrap_err();

    assert!(matches!(advance_error, ProductStoreError::PathEscape(_)));
    assert!(matches!(failed_error, ProductStoreError::PathEscape(_)));
    assert!(!journal_root.exists());
    assert!(!lock_path.exists());
}

#[test]
fn work_item_revision_publication_journal_serializes_cross_store_advance_and_failure() {
    let temp = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(temp.path().join(".aria"));
    let store = WorkItemRevisionStore::new(paths.clone());
    let plan = plan_lineage();
    let journal = publication_journal("amendment_0001", PlanAmendmentPublicationPhase::Prepared);
    store.put_plan_lineage(&plan).unwrap();
    store
        .put_plan_amendment_publication_journal(&plan, &journal)
        .unwrap();
    let target_path = store.amendment_publication_journal_path(
        &plan.project_id,
        &plan.issue_id,
        &plan.id,
        &journal.id,
    );
    let guard = ExclusiveFileLock::acquire(&target_path).unwrap();
    let (_hook_guard, lock_attempt_receiver) = register_lock_attempt_hook(&target_path);
    let (result_sender, result_receiver) = mpsc::channel();

    {
        let store = WorkItemRevisionStore::new(paths.clone());
        let plan = plan.clone();
        let journal_id = journal.id.clone();
        let sender = result_sender.clone();
        thread::spawn(move || {
            let result = store.advance_plan_amendment_publication(
                &plan,
                &journal_id,
                PlanAmendmentPublicationPhase::PlanPublished,
            );
            let _ = sender.send(("advance", result));
        });
    }
    {
        let store = WorkItemRevisionStore::new(paths);
        let plan = plan.clone();
        let journal_id = journal.id.clone();
        let sender = result_sender.clone();
        thread::spawn(move || {
            let result = store.mark_plan_amendment_publication_failed(
                &plan,
                &journal_id,
                "plan publish failed".to_string(),
            );
            let _ = sender.send(("failed", result));
        });
    }
    drop(result_sender);

    for _ in 0..2 {
        lock_attempt_receiver
            .recv_timeout(COMPLETION_TIMEOUT)
            .expect("worker did not reach the journal target lock");
    }
    assert!(matches!(
        result_receiver.recv_timeout(LOCKED_TIMEOUT),
        Err(RecvTimeoutError::Timeout)
    ));
    drop(guard);

    let mut advanced = None;
    let mut failed = None;
    for _ in 0..2 {
        let (operation, result) = result_receiver
            .recv_timeout(COMPLETION_TIMEOUT)
            .expect("journal operation did not finish after lock release");
        match operation {
            "advance" => advanced = Some(result.unwrap()),
            "failed" => failed = Some(result.unwrap()),
            _ => unreachable!(),
        }
    }
    let advanced = advanced.unwrap();
    let failed = failed.unwrap();
    let stored: PlanAmendmentPublicationJournal = read_json(&target_path).unwrap();

    assert_eq!(advanced.phase, PlanAmendmentPublicationPhase::PlanPublished);
    assert_eq!(advanced.error, None);
    assert_eq!(failed.error.as_deref(), Some("plan publish failed"));
    if failed.phase == PlanAmendmentPublicationPhase::Prepared {
        assert_eq!(stored, advanced);
    } else {
        assert_eq!(failed.phase, PlanAmendmentPublicationPhase::PlanPublished);
        assert_eq!(stored, failed);
    }
}
