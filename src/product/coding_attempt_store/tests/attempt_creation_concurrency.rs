use std::collections::BTreeSet;
use std::sync::{Arc, Barrier};
use std::time::Duration;

use super::*;
use crate::product::coding_attempt_store::locking::{
    ExclusiveFileLock, register_lock_attempt_hook,
};
use crate::product::json_store::ProductStoreError;

#[test]
fn single_work_item_attempt_creation_is_serialized_across_store_instances() {
    let (tmp, first_store) = setup_store();
    let second_store = CodingAttemptStore::new(ProductAppPaths::new(tmp.path().join(".aria")));
    let lock_target = first_store
        .coding_attempts_root(PROJECT_ID, ISSUE_ID)
        .join("work-item-attempt-locks")
        .join(WORK_ITEM_ID);
    let held_lock = ExclusiveFileLock::acquire(&lock_target).expect("hold attempt creation lock");
    let (_hook, lock_attempts) = register_lock_attempt_hook(&lock_target);
    let start = Arc::new(Barrier::new(3));

    let spawn_create = |store: CodingAttemptStore, branch_name: &'static str| {
        let start = Arc::clone(&start);
        std::thread::spawn(move || {
            start.wait();
            store.create_attempt(CreateCodingAttemptInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                work_item_id: WORK_ITEM_ID.to_string(),
                base_branch: "main".to_string(),
                branch_name: branch_name.to_string(),
                worktree_path: None,
                provider_config_snapshot: provider_snapshot(),
                max_auto_rework: 2,
            })
        })
    };
    let first = spawn_create(
        first_store.clone(),
        "aria/work-items/work_item_0001/attempt-a",
    );
    let second = spawn_create(
        second_store.clone(),
        "aria/work-items/work_item_0001/attempt-b",
    );
    start.wait();

    lock_attempts
        .recv_timeout(Duration::from_secs(2))
        .expect("first store must contend on the cross-process lock");
    lock_attempts
        .recv_timeout(Duration::from_secs(2))
        .expect("second store must contend on the same cross-process lock");
    drop(held_lock);

    let results = [
        first.join().expect("first create thread"),
        second.join().expect("second create thread"),
    ];
    let attempts = results
        .iter()
        .filter_map(|result| result.as_ref().ok())
        .collect::<Vec<_>>();
    let conflicts = results
        .iter()
        .filter_map(|result| result.as_ref().err())
        .collect::<Vec<_>>();
    assert_eq!(
        attempts.len(),
        1,
        "at most one active attempt may be created"
    );
    assert_eq!(
        conflicts.len(),
        1,
        "the losing request must return a typed conflict"
    );
    assert!(matches!(
        conflicts[0],
        ProductStoreError::Conflict {
            kind: "active_coding_attempt",
            id,
        } if id == &attempts[0].id
    ));

    let persisted = first_store
        .list_attempts_for_work_item(PROJECT_ID, ISSUE_ID, WORK_ITEM_ID)
        .expect("persisted attempts");
    assert_eq!(persisted.len(), 1);
    assert_eq!(persisted[0].attempt_no, 1);
    assert_eq!(
        persisted
            .iter()
            .map(|attempt| attempt.attempt_no)
            .collect::<BTreeSet<_>>()
            .len(),
        persisted.len()
    );
    assert!(
        first_store
            .role_provider_config_path(PROJECT_ID, ISSUE_ID, &persisted[0].id)
            .is_file()
    );
    let provider_configs =
        std::fs::read_dir(first_store.coding_attempts_root(PROJECT_ID, ISSUE_ID))
            .expect("coding attempts root")
            .filter_map(Result::ok)
            .filter(|entry| entry.path().join("role-provider-config.json").is_file())
            .count();
    assert_eq!(
        provider_configs, 1,
        "losing request must not leave provider config"
    );
}
