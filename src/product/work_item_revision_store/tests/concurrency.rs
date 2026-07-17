use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;

use crate::product::json_store::{read_json, write_json};
use crate::product::models::WorkItemDraftRevisionState;

use super::super::ExclusiveFileLock;
use super::*;

const LOCKED_TIMEOUT: Duration = Duration::from_millis(200);
const COMPLETION_TIMEOUT: Duration = Duration::from_secs(5);

fn spawn_operation<T, F>(started_sender: Sender<()>, result_sender: Sender<T>, operation: F)
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let _ = thread::spawn(move || {
        let _ = started_sender.send(());
        let result = operation();
        let _ = result_sender.send(result);
    });
}

fn assert_operations_blocked<T>(
    started_receiver: &Receiver<()>,
    result_receiver: &Receiver<T>,
    operation_count: usize,
) {
    for _ in 0..operation_count {
        started_receiver
            .recv_timeout(COMPLETION_TIMEOUT)
            .expect("operation thread did not start");
    }
    assert!(matches!(
        result_receiver.recv_timeout(LOCKED_TIMEOUT),
        Err(RecvTimeoutError::Timeout)
    ));
}

fn receive_results<T>(result_receiver: &Receiver<T>, count: usize) -> Vec<T> {
    (0..count)
        .map(|_| {
            result_receiver
                .recv_timeout(COMPLETION_TIMEOUT)
                .expect("operation did not finish after lock release")
        })
        .collect()
}

fn assert_one_success_one_identity_mismatch<T>(results: &[Result<T, ProductStoreError>]) {
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, Err(ProductStoreError::IdentityMismatch { .. })))
            .count(),
        1
    );
}

#[test]
fn work_item_revision_store_plan_compare_and_set_waits_for_lineage_lock() {
    let temp = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(temp.path().join(".aria"));
    let store = WorkItemRevisionStore::new(paths.clone());
    let plan = plan_lineage();
    let first = plan_revision("plan_revision_0001", 1);
    let second = plan_revision("plan_revision_0002", 2);
    let third = plan_revision("plan_revision_0003", 3);
    store.put_plan_lineage(&plan).unwrap();
    store.put_plan_revision(&plan, &first).unwrap();
    store.put_plan_revision(&plan, &second).unwrap();
    store.put_plan_revision(&plan, &third).unwrap();
    store.set_active_plan_revision(&plan, &first.id).unwrap();

    let target_path = store.plan_lineage_path(&plan.project_id, &plan.issue_id, &plan.id);
    let guard = ExclusiveFileLock::acquire(&target_path).unwrap();
    let (started_sender, started_receiver) = mpsc::channel();
    let (result_sender, result_receiver) = mpsc::channel();
    for next_revision_id in [second.id.clone(), third.id.clone()] {
        let store = WorkItemRevisionStore::new(paths.clone());
        let plan = plan.clone();
        let expected_revision_id = first.id.clone();
        spawn_operation(started_sender.clone(), result_sender.clone(), move || {
            store.compare_and_set_active_plan_revision(
                &plan,
                &expected_revision_id,
                &next_revision_id,
            )
        });
    }
    drop(started_sender);
    drop(result_sender);

    assert_operations_blocked(&started_receiver, &result_receiver, 2);
    drop(guard);

    let results = receive_results(&result_receiver, 2);
    assert_one_success_one_identity_mismatch(&results);
}

#[test]
fn work_item_revision_store_work_item_compare_and_set_waits_for_logical_item_lock() {
    let temp = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(temp.path().join(".aria"));
    let store = WorkItemRevisionStore::new(paths.clone());
    let plan = plan_lineage();
    let logical = logical_work_item();
    let first = work_item_revision();
    let mut second = first.clone();
    second.id = "work_item_revision_0002".to_string();
    second.canonical_contract_hash = "contract_hash_0002".to_string();
    store.put_plan_lineage(&plan).unwrap();
    store.put_logical_work_item(&plan, &logical).unwrap();
    store.put_work_item_revision(&plan, &first).unwrap();
    store.put_work_item_revision(&plan, &second).unwrap();

    let target_path =
        store.logical_work_item_path(&plan.project_id, &plan.issue_id, &plan.id, &logical.id);
    let guard = ExclusiveFileLock::acquire(&target_path).unwrap();
    let (started_sender, started_receiver) = mpsc::channel();
    let (result_sender, result_receiver) = mpsc::channel();
    for next_revision_id in [first.id.clone(), second.id.clone()] {
        let store = WorkItemRevisionStore::new(paths.clone());
        let plan = plan.clone();
        let logical = logical.clone();
        spawn_operation(started_sender.clone(), result_sender.clone(), move || {
            store.set_active_work_item_revision(&plan, &logical, None, &next_revision_id)
        });
    }
    drop(started_sender);
    drop(result_sender);

    assert_operations_blocked(&started_receiver, &result_receiver, 2);
    drop(guard);

    let results = receive_results(&result_receiver, 2);
    assert_one_success_one_identity_mismatch(&results);
}

#[test]
fn work_item_revision_store_amendment_acquire_waits_for_lineage_lock() {
    let temp = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(temp.path().join(".aria"));
    let store = WorkItemRevisionStore::new(paths.clone());
    let plan = plan_lineage();
    store.put_plan_lineage(&plan).unwrap();

    let target_path = store.plan_lineage_path(&plan.project_id, &plan.issue_id, &plan.id);
    let guard = ExclusiveFileLock::acquire(&target_path).unwrap();
    let (started_sender, started_receiver) = mpsc::channel();
    let (result_sender, result_receiver) = mpsc::channel();
    for amendment_id in ["amendment_0001", "amendment_0002"] {
        let store = WorkItemRevisionStore::new(paths.clone());
        let plan = plan.clone();
        spawn_operation(started_sender.clone(), result_sender.clone(), move || {
            store.acquire_active_amendment(&plan, amendment_id)
        });
    }
    drop(started_sender);
    drop(result_sender);

    assert_operations_blocked(&started_receiver, &result_receiver, 2);
    drop(guard);

    let results = receive_results(&result_receiver, 2);
    assert_one_success_one_identity_mismatch(&results);
}

#[test]
fn work_item_revision_store_amendment_release_waits_for_lineage_lock() {
    let temp = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(temp.path().join(".aria"));
    let store = WorkItemRevisionStore::new(paths.clone());
    let plan = plan_lineage();
    store.put_plan_lineage(&plan).unwrap();
    store
        .acquire_active_amendment(&plan, "amendment_0001")
        .unwrap();

    let target_path = store.plan_lineage_path(&plan.project_id, &plan.issue_id, &plan.id);
    let guard = ExclusiveFileLock::acquire(&target_path).unwrap();
    let (started_sender, started_receiver) = mpsc::channel();
    let (result_sender, result_receiver) = mpsc::channel();
    for _ in 0..2 {
        let store = WorkItemRevisionStore::new(paths.clone());
        let plan = plan.clone();
        spawn_operation(started_sender.clone(), result_sender.clone(), move || {
            store.release_active_amendment(&plan, "amendment_0001")
        });
    }
    drop(started_sender);
    drop(result_sender);

    assert_operations_blocked(&started_receiver, &result_receiver, 2);
    drop(guard);

    let results = receive_results(&result_receiver, 2);
    assert_one_success_one_identity_mismatch(&results);
}

#[test]
fn work_item_revision_store_repair_updates_wait_for_request_lock() {
    let temp = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(temp.path().join(".aria"));
    let store = WorkItemRevisionStore::new(paths.clone());
    let plan = plan_lineage();
    let request = repair_request("plan_repair_request_0001");
    let extra = json!({"kind": "concurrent", "id": "evidence_0001"});
    store.put_plan_lineage(&plan).unwrap();
    store.put_repair_request(&plan, &request).unwrap();

    let target_path =
        store.repair_request_path(&plan.project_id, &plan.issue_id, &plan.id, &request.id);
    let guard = ExclusiveFileLock::acquire(&target_path).unwrap();
    let (started_sender, started_receiver) = mpsc::channel();
    let (result_sender, result_receiver) = mpsc::channel();
    {
        let store = WorkItemRevisionStore::new(paths.clone());
        let plan = plan.clone();
        let request_id = request.id.clone();
        spawn_operation(started_sender.clone(), result_sender.clone(), move || {
            store.update_repair_request_status(
                &plan,
                &request_id,
                PlanRepairRequestStatus::InProgress,
            )
        });
    }
    {
        let store = WorkItemRevisionStore::new(paths.clone());
        let plan = plan.clone();
        let request_id = request.id.clone();
        let extra = extra.clone();
        spawn_operation(started_sender.clone(), result_sender.clone(), move || {
            store.merge_repair_request_evidence(&plan, &request_id, vec![extra])
        });
    }
    drop(started_sender);
    drop(result_sender);

    assert_operations_blocked(&started_receiver, &result_receiver, 2);
    drop(guard);

    for result in receive_results(&result_receiver, 2) {
        result.unwrap();
    }
    let stored = store.list_open_repair_requests(&plan).unwrap().remove(0);
    assert_eq!(stored.status, PlanRepairRequestStatus::InProgress);
    assert!(stored.evidence.contains(&extra));
}

#[test]
fn work_item_revision_store_immutable_conflict_waits_for_artifact_lock() {
    let temp = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(temp.path().join(".aria"));
    let store = WorkItemRevisionStore::new(paths.clone());
    let plan = plan_lineage();
    let mut first = plan_revision("plan_revision_0002", 2);
    first.reason = PlanRevisionReason::SubgraphReplan;
    let mut second = first.clone();
    second.reason = PlanRevisionReason::StoryAmendment;
    store.put_plan_lineage(&plan).unwrap();

    let target_path =
        store.plan_revision_path(&plan.project_id, &plan.issue_id, &plan.id, &first.id);
    let guard = ExclusiveFileLock::acquire(&target_path).unwrap();
    let (started_sender, started_receiver) = mpsc::channel();
    let (result_sender, result_receiver) = mpsc::channel();
    for revision in [first, second] {
        let store = WorkItemRevisionStore::new(paths.clone());
        let plan = plan.clone();
        spawn_operation(started_sender.clone(), result_sender.clone(), move || {
            store.put_plan_revision(&plan, &revision).map(|()| revision)
        });
    }
    drop(started_sender);
    drop(result_sender);

    assert_operations_blocked(&started_receiver, &result_receiver, 2);
    drop(guard);

    let results = receive_results(&result_receiver, 2);
    assert_one_success_one_identity_mismatch(&results);
    let winner = results.into_iter().find_map(Result::ok).unwrap();
    assert_eq!(
        store
            .get_plan_revision(PROJECT_ID, ISSUE_ID, PLAN_ID, &winner.id)
            .unwrap(),
        winner
    );
    store.put_plan_revision(&plan, &winner).unwrap();
}

#[test]
fn work_item_revision_store_immutable_replay_waits_for_artifact_lock() {
    let temp = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(temp.path().join(".aria"));
    let store = WorkItemRevisionStore::new(paths.clone());
    let plan = plan_lineage();
    let revision = plan_revision("plan_revision_0002", 2);
    store.put_plan_lineage(&plan).unwrap();

    let target_path =
        store.plan_revision_path(&plan.project_id, &plan.issue_id, &plan.id, &revision.id);
    let guard = ExclusiveFileLock::acquire(&target_path).unwrap();
    let (started_sender, started_receiver) = mpsc::channel();
    let (result_sender, result_receiver) = mpsc::channel();
    for _ in 0..2 {
        let store = WorkItemRevisionStore::new(paths.clone());
        let plan = plan.clone();
        let revision = revision.clone();
        spawn_operation(started_sender.clone(), result_sender.clone(), move || {
            store.put_plan_revision(&plan, &revision)
        });
    }
    drop(started_sender);
    drop(result_sender);

    assert_operations_blocked(&started_receiver, &result_receiver, 2);
    drop(guard);

    for result in receive_results(&result_receiver, 2) {
        result.unwrap();
    }
}

#[test]
fn work_item_revision_store_draft_state_init_and_update_share_state_lock() {
    let temp = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(temp.path().join(".aria"));
    let store = WorkItemRevisionStore::new(paths.clone());
    let plan = plan_lineage();
    let logical = logical_work_item();
    let draft = draft_revision();
    store.put_plan_lineage(&plan).unwrap();
    store.put_logical_work_item(&plan, &logical).unwrap();
    write_json(
        &store.draft_revision_path(&plan.project_id, &plan.issue_id, &plan.id, &draft.id),
        &draft,
    )
    .unwrap();

    let state_path =
        store.draft_revision_state_path(&plan.project_id, &plan.issue_id, &plan.id, &draft.id);
    assert!(!state_path.exists());
    let guard = ExclusiveFileLock::acquire(&state_path).unwrap();
    let (started_sender, started_receiver) = mpsc::channel();
    let (result_sender, result_receiver) = mpsc::channel();
    {
        let store = WorkItemRevisionStore::new(paths.clone());
        let plan = plan.clone();
        let draft = draft.clone();
        spawn_operation(started_sender.clone(), result_sender.clone(), move || {
            store.put_draft_revision(&plan, &draft)
        });
    }
    {
        let store = WorkItemRevisionStore::new(paths);
        let plan = plan.clone();
        let draft_id = draft.id.clone();
        spawn_operation(started_sender.clone(), result_sender.clone(), move || {
            store
                .update_draft_revision_state(
                    &plan,
                    &draft_id,
                    WorkItemDraftRevisionStatus::Approved,
                )
                .map(|_| ())
        });
    }
    drop(started_sender);
    drop(result_sender);

    assert_operations_blocked(&started_receiver, &result_receiver, 2);
    drop(guard);

    for result in receive_results(&result_receiver, 2) {
        result.unwrap();
    }
    let state: WorkItemDraftRevisionState = read_json(&state_path).unwrap();
    assert_eq!(state.draft_revision_id, draft.id);
    assert_eq!(state.status, WorkItemDraftRevisionStatus::Approved);
}
