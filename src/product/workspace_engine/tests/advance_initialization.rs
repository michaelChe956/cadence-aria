use crate::product::advance_store::{
    AdvanceInitializationPhase, AdvanceInput, AdvanceStatus, AdvanceStore,
};
use crate::product::app_paths::ProductAppPaths;
use crate::product::json_store::ProductStoreError;
use tempfile::TempDir;

const PROJECT_ID: &str = "project_advance_init";
const ISSUE_ID: &str = "issue_advance_init";
const PLAN_ID: &str = "plan_advance_init";
const REVISION_ID: &str = "revision_advance_init";

fn store(root: &TempDir) -> AdvanceStore {
    AdvanceStore::new(ProductAppPaths::new(root.path().join(".aria")))
}

fn input(command_id: &str) -> AdvanceInput {
    AdvanceInput {
        command_id: command_id.to_string(),
        project_id: PROJECT_ID.to_string(),
        issue_id: ISSUE_ID.to_string(),
        plan_id: PLAN_ID.to_string(),
    }
}

#[test]
fn advance_initialization_record_is_persisted_before_followup_steps() {
    let root = TempDir::new().unwrap();
    let record = store(&root)
        .persist_advance_record_if_absent(&input("command_1"), REVISION_ID)
        .unwrap();
    assert_eq!(record.status, AdvanceStatus::Initializing);
    assert_eq!(record.plan_revision_id, REVISION_ID);
    assert!(
        store(&root)
            .get_advance_initialization(&record)
            .unwrap()
            .is_none()
    );
}

#[test]
fn advance_initialization_replay_by_command_keeps_record_identity() {
    let root = TempDir::new().unwrap();
    let advance_store = store(&root);
    let first = advance_store
        .persist_advance_record_if_absent(&input("command_1"), REVISION_ID)
        .unwrap();
    let replay = advance_store
        .persist_advance_record_if_absent(&input("command_1"), "revision_changed")
        .unwrap();
    assert_eq!(first, replay);
}

#[test]
fn advance_initialization_replay_by_plan_keeps_original_command() {
    let root = TempDir::new().unwrap();
    let advance_store = store(&root);
    let first = advance_store
        .persist_advance_record_if_absent(&input("command_1"), REVISION_ID)
        .unwrap();
    let replay = advance_store
        .persist_advance_record_if_absent(&input("command_2"), "revision_changed")
        .unwrap();
    assert_eq!(first, replay);
    assert_eq!(replay.command_id, "command_1");
}

#[test]
fn advance_initialization_journal_reuses_same_attempt_id() {
    let root = TempDir::new().unwrap();
    let advance_store = store(&root);
    let record = advance_store
        .persist_advance_record_if_absent(&input("command_1"), REVISION_ID)
        .unwrap();
    let first = advance_store
        .put_advance_initialization_if_absent(&record, "attempt_1")
        .unwrap();
    let replay = advance_store
        .put_advance_initialization_if_absent(&record, "attempt_1")
        .unwrap();
    assert_eq!(first, replay);
    assert_eq!(replay.phase, AdvanceInitializationPhase::JournalPrepared);
}

#[test]
fn advance_initialization_journal_rejects_attempt_identity_drift() {
    let root = TempDir::new().unwrap();
    let advance_store = store(&root);
    let record = advance_store
        .persist_advance_record_if_absent(&input("command_1"), REVISION_ID)
        .unwrap();
    advance_store
        .put_advance_initialization_if_absent(&record, "attempt_1")
        .unwrap();
    assert!(matches!(
        advance_store.put_advance_initialization_if_absent(&record, "attempt_2"),
        Err(ProductStoreError::IdentityMismatch { .. })
    ));
}

#[test]
fn advance_initialization_phase_only_advances_contiguously() {
    let root = TempDir::new().unwrap();
    let advance_store = store(&root);
    let record = advance_store
        .persist_advance_record_if_absent(&input("command_1"), REVISION_ID)
        .unwrap();
    let journal = advance_store
        .put_advance_initialization_if_absent(&record, "attempt_1")
        .unwrap();
    let progressed = advance_store
        .advance_initialization_phase(
            &record,
            &journal,
            AdvanceInitializationPhase::AttemptPersisted,
        )
        .unwrap();
    assert!(matches!(
        advance_store.advance_initialization_phase(
            &record,
            &progressed,
            AdvanceInitializationPhase::UnitsMaterialized,
        ),
        Err(ProductStoreError::Conflict { .. })
    ));
}

#[test]
fn advance_initialization_failure_is_durable_on_record_and_journal() {
    let root = TempDir::new().unwrap();
    let advance_store = store(&root);
    let record = advance_store
        .persist_advance_record_if_absent(&input("command_1"), REVISION_ID)
        .unwrap();
    let journal = advance_store
        .put_advance_initialization_if_absent(&record, "attempt_1")
        .unwrap();
    let (failed_record, failed_journal) = advance_store
        .mark_advance_initialization_error(&record, &journal, "checkpoint failed")
        .unwrap();
    assert_eq!(failed_record.status, AdvanceStatus::Failed);
    assert_eq!(failed_record.error.as_deref(), Some("checkpoint failed"));
    assert_eq!(failed_journal.error.as_deref(), Some("checkpoint failed"));
}

#[test]
fn advance_initialization_journal_rejects_record_identity_drift() {
    let root = TempDir::new().unwrap();
    let advance_store = store(&root);
    let record = advance_store
        .persist_advance_record_if_absent(&input("command_1"), REVISION_ID)
        .unwrap();
    let journal = advance_store
        .put_advance_initialization_if_absent(&record, "attempt_1")
        .unwrap();
    let mut drifted = journal;
    drifted.advance_id = "advance_other".to_string();
    assert!(matches!(
        advance_store.save_advance_initialization(&record, &drifted),
        Err(ProductStoreError::IdentityMismatch { .. })
    ));
}
