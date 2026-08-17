//! REQ-COD-03 历史 shared worktree 迁移协议的单元回归。

use std::time::Duration;

use tempfile::tempdir;
use uuid::Uuid;

use super::{
    LegacySharedWorktreeMigration, LegacySharedWorktreeRecord, legacy_path, repository_path,
};
use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::locking::{
    ExclusiveFileLock, register_lock_attempt_hook,
};
use crate::product::json_store::{ProductStoreError, read_json, write_json};
use crate::product::logical_codebase::{
    IdentityMigrationJournal, IdentityMigrationJournalStore, IdentityMigrationPhase,
    LogicalRepositoryId, RepositoryCheckoutId, RepositoryIdentityMapping,
};
use crate::product::models::{IssueSharedWorktree, IssueSharedWorktreeStatus};

const PROJECT_ID: &str = "project_0001";
const ISSUE_ID: &str = "issue_0001";
const LEGACY_REPOSITORY_ID: &str = "repository_0001";

struct Fixture {
    _temp: tempfile::TempDir,
    paths: ProductAppPaths,
    repository_id: LogicalRepositoryId,
    checkout_id: RepositoryCheckoutId,
}

fn fixture() -> Fixture {
    let temp = tempdir().expect("temporary aria root");
    let paths = ProductAppPaths::new(temp.path().join(".aria"));
    let repository_id = LogicalRepositoryId(Uuid::from_u128(0x10));
    let checkout_id = RepositoryCheckoutId(Uuid::from_u128(0x20));
    IdentityMigrationJournalStore::new(paths.clone())
        .save(
            PROJECT_ID,
            &IdentityMigrationJournal {
                journal_version: 1,
                migration_id: "identity-migration:project_0001:v1".to_string(),
                project_id: PROJECT_ID.to_string(),
                target_schema_version: 1,
                phase: IdentityMigrationPhase::Completed,
                source_repos_digest: "sha256:fixture".to_string(),
                mappings: vec![RepositoryIdentityMapping {
                    legacy_repository_id: LEGACY_REPOSITORY_ID.to_string(),
                    source_identity_digest: "sha256:source".to_string(),
                    logical_repository_id: repository_id,
                    primary_checkout_id: checkout_id,
                    physical_repository_id: LEGACY_REPOSITORY_ID.to_string(),
                    idempotency_key: "fixture:mapping".to_string(),
                    authority_written: true,
                    compatibility_backfilled: true,
                }],
                completed_keys: Vec::new(),
                read_mode: Some("dual".to_string()),
                last_error: None,
                created_at: "2026-08-14T00:00:00Z".to_string(),
                updated_at: "2026-08-14T00:00:00Z".to_string(),
                completed_at: Some("2026-08-14T00:00:00Z".to_string()),
            },
        )
        .expect("identity journal");
    Fixture {
        _temp: temp,
        paths,
        repository_id,
        checkout_id,
    }
}

fn legacy_record() -> IssueSharedWorktree {
    IssueSharedWorktree {
        id: "issue_shared_worktree_project_0001_issue_0001".to_string(),
        project_id: PROJECT_ID.to_string(),
        issue_id: ISSUE_ID.to_string(),
        repository_id: LEGACY_REPOSITORY_ID.to_string(),
        target_repository_id: None,
        checkout_id: None,
        path_schema_version: 0,
        branch_name: "aria/issues/issue_0001".to_string(),
        worktree_path: "/tmp/aria-worktree".into(),
        base_branch: "main".to_string(),
        status: IssueSharedWorktreeStatus::Ready,
        current_active_work_item_id: None,
        current_lock_owner_id: None,
        last_completed_work_item_id: Some("work_item_0001".to_string()),
        created_at: "2026-08-14T00:00:00Z".to_string(),
        updated_at: "2026-08-14T00:00:00Z".to_string(),
    }
}

fn seed_legacy(fixture: &Fixture, record: &IssueSharedWorktree) {
    write_json(&legacy_path(&fixture.paths, PROJECT_ID, ISSUE_ID), record).expect("legacy record");
}

fn load_legacy(fixture: &Fixture) -> LegacySharedWorktreeRecord {
    LegacySharedWorktreeMigration::load_legacy_shared_worktree(&fixture.paths, PROJECT_ID, ISSUE_ID)
        .expect("load legacy record")
        .expect("legacy record is present")
}

#[test]
fn legacy_shared_worktree_migration_loads_a_missing_legacy_record_as_absent() {
    let fixture = fixture();

    let loaded = LegacySharedWorktreeMigration::load_legacy_shared_worktree(
        &fixture.paths,
        PROJECT_ID,
        ISSUE_ID,
    )
    .expect("missing legacy fixture is not an error");

    assert!(loaded.is_none());
}

#[test]
fn legacy_shared_worktree_migration_rejects_malformed_or_identity_inconsistent_records() {
    let fixture = fixture();
    let path = legacy_path(&fixture.paths, PROJECT_ID, ISSUE_ID);
    std::fs::create_dir_all(path.parent().expect("issue root")).expect("issue root");
    std::fs::write(&path, "{not-json").expect("malformed legacy fixture");

    let error = LegacySharedWorktreeMigration::load_legacy_shared_worktree(
        &fixture.paths,
        PROJECT_ID,
        ISSUE_ID,
    )
    .expect_err("bad legacy JSON must fail closed");
    assert!(matches!(
        error,
        ProductStoreError::InvalidRecord {
            kind: "legacy_shared_worktree_migration",
            ref reason,
        } if reason.starts_with("legacy_shared_worktree_inconsistent:")
    ));

    let mut inconsistent = legacy_record();
    inconsistent.project_id = "project_other".to_string();
    write_json(&path, &inconsistent).expect("scope-inconsistent legacy record");
    let error = LegacySharedWorktreeMigration::load_legacy_shared_worktree(
        &fixture.paths,
        PROJECT_ID,
        ISSUE_ID,
    )
    .expect_err("scope drift must fail closed");
    assert!(
        error
            .to_string()
            .contains("legacy_shared_worktree_inconsistent")
    );
}

#[test]
fn legacy_shared_worktree_migration_writes_repository_record_redirect_and_cleans_only_after_redirect()
 {
    let fixture = fixture();
    let legacy = legacy_record();
    seed_legacy(&fixture, &legacy);

    let migrated = LegacySharedWorktreeMigration::migrate_to_repository_keyed(
        &fixture.paths,
        load_legacy(&fixture),
    )
    .expect("manual migration");
    assert_eq!(migrated.repository_id, fixture.repository_id);
    assert!(migrated.redirect_persisted);
    assert!(!migrated.legacy_cleanup_completed);

    let new_path = repository_path(&fixture.paths, PROJECT_ID, ISSUE_ID, fixture.repository_id);
    let new_record: IssueSharedWorktree = read_json(&new_path).expect("repository-keyed record");
    assert_eq!(
        new_record.repository_id,
        fixture.repository_id.0.to_string()
    );
    assert_eq!(new_record.target_repository_id, Some(fixture.repository_id));
    assert_eq!(new_record.checkout_id, Some(fixture.checkout_id));
    assert_eq!(new_record.path_schema_version, 1);

    // The old path has a durable tombstone/redirect rather than silently pointing at a record
    // whose repository identity an old API cannot supply.
    let redirect = LegacySharedWorktreeMigration::load_legacy_shared_worktree_redirect(
        &fixture.paths,
        PROJECT_ID,
        ISSUE_ID,
    )
    .expect("read redirect")
    .expect("redirect exists before cleanup");
    assert_eq!(redirect.repository_id, fixture.repository_id);
    assert!(
        LegacySharedWorktreeMigration::load_legacy_shared_worktree(
            &fixture.paths,
            PROJECT_ID,
            ISSUE_ID,
        )
        .expect("redirect hides legacy record")
        .is_none()
    );

    assert!(
        LegacySharedWorktreeMigration::finalize_if_no_active_references(
            &fixture.paths,
            PROJECT_ID,
            ISSUE_ID,
        )
        .expect("redirect + no active references permits cleanup")
    );
    assert!(!legacy_path(&fixture.paths, PROJECT_ID, ISSUE_ID).exists());
    assert!(
        !legacy_path(&fixture.paths, PROJECT_ID, ISSUE_ID)
            .with_file_name(".issue-shared-worktree.json.lock")
            .exists()
    );
    assert!(
        LegacySharedWorktreeMigration::finalize_if_no_active_references(
            &fixture.paths,
            PROJECT_ID,
            ISSUE_ID,
        )
        .expect("cleanup retry is idempotent")
    );
}

#[test]
fn legacy_shared_worktree_migration_is_idempotent_and_rejects_mapping_or_record_conflicts() {
    let primary_fixture = fixture();
    let legacy = legacy_record();
    seed_legacy(&primary_fixture, &legacy);

    let first = LegacySharedWorktreeMigration::migrate_to_repository_keyed(
        &primary_fixture.paths,
        load_legacy(&primary_fixture),
    )
    .expect("first migration");
    let second = LegacySharedWorktreeMigration::migrate_to_repository_keyed(
        &primary_fixture.paths,
        LegacySharedWorktreeRecord { worktree: legacy },
    )
    .expect("redirect recovery is idempotent");
    assert_eq!(first.repository_id, second.repository_id);
    assert_eq!(first.migrated_record, second.migrated_record);

    let conflicting_path = repository_path(
        &primary_fixture.paths,
        PROJECT_ID,
        ISSUE_ID,
        primary_fixture.repository_id,
    );
    let mut conflicting: IssueSharedWorktree = read_json(&conflicting_path).expect("new record");
    conflicting.target_repository_id = None;
    write_json(&conflicting_path, &conflicting).expect("conflicting new record");
    let error = LegacySharedWorktreeMigration::migrate_to_repository_keyed(
        &primary_fixture.paths,
        LegacySharedWorktreeRecord {
            worktree: legacy_record(),
        },
    )
    .expect_err("redirect recovery must detect conflicting identity record");
    assert!(
        error
            .to_string()
            .contains("legacy_shared_worktree_inconsistent")
    );

    let mapping_fixture = fixture();
    let mut wrong_mapping = legacy_record();
    wrong_mapping.repository_id = "repository_missing".to_string();
    seed_legacy(&mapping_fixture, &wrong_mapping);
    let error = LegacySharedWorktreeMigration::migrate_to_repository_keyed(
        &mapping_fixture.paths,
        load_legacy(&mapping_fixture),
    )
    .expect_err("missing physical-to-logical journal mapping must fail closed");
    assert!(
        error
            .to_string()
            .contains("legacy_shared_worktree_inconsistent")
    );
}

#[test]
fn legacy_shared_worktree_migration_blocks_active_legacy_references_before_any_new_write() {
    let fixture = fixture();
    let mut active = legacy_record();
    active.current_active_work_item_id = Some("work_item_0001".to_string());
    active.current_lock_owner_id = Some("attempt_0001".to_string());
    seed_legacy(&fixture, &active);

    let error = LegacySharedWorktreeMigration::migrate_to_repository_keyed(
        &fixture.paths,
        load_legacy(&fixture),
    )
    .expect_err("active legacy lock blocks migration");
    assert!(matches!(
        error,
        ProductStoreError::Conflict {
            kind: "legacy_shared_worktree_active",
            ..
        }
    ));
    assert!(legacy_path(&fixture.paths, PROJECT_ID, ISSUE_ID).exists());
    assert!(!repository_path(&fixture.paths, PROJECT_ID, ISSUE_ID, fixture.repository_id).exists());
}

#[test]
fn legacy_shared_worktree_migration_obtains_legacy_lock_before_repository_lock() {
    let fixture = fixture();
    let legacy = legacy_record();
    seed_legacy(&fixture, &legacy);
    let record = load_legacy(&fixture);
    let legacy_path = legacy_path(&fixture.paths, PROJECT_ID, ISSUE_ID);
    let repository_path =
        repository_path(&fixture.paths, PROJECT_ID, ISSUE_ID, fixture.repository_id);

    // Ensure both derived lock paths exist so the test hooks can canonicalize them.
    let held_legacy = ExclusiveFileLock::acquire(&legacy_path).expect("hold legacy F-6 lock");
    let (_repository_seed, _) = {
        let lock = ExclusiveFileLock::acquire(&repository_path).expect("seed repository lock");
        (lock, ())
    };
    let (_legacy_hook, legacy_attempts) = register_lock_attempt_hook(&legacy_path);
    let (_repository_hook, repository_attempts) = register_lock_attempt_hook(&repository_path);
    let paths = fixture.paths.clone();
    let migration = std::thread::spawn(move || {
        LegacySharedWorktreeMigration::migrate_to_repository_keyed(&paths, record)
    });

    legacy_attempts
        .recv_timeout(Duration::from_secs(2))
        .expect("migration first attempts legacy lock");
    assert!(
        repository_attempts.try_recv().is_err(),
        "repository lock cannot precede legacy lock"
    );
    drop(held_legacy);
    repository_attempts
        .recv_timeout(Duration::from_secs(2))
        .expect("repository lock is only attempted after legacy lock releases");
    drop(_repository_seed);
    migration
        .join()
        .expect("migration thread")
        .expect("migration succeeds");
}
