use std::sync::{Arc, Barrier};
use std::thread;

use super::*;

fn large_plan_lineage() -> WorkItemPlanLineage {
    let mut plan = plan_lineage();
    plan.story_spec_refs = (0..25_000)
        .map(|index| format!("story_spec_{index:05}"))
        .collect();
    plan
}

fn large_plan_revision(id: &str, reason: PlanRevisionReason) -> WorkItemPlanRevision {
    let mut revision = plan_revision(id, 2);
    revision.reason = reason;
    revision.work_item_bindings = (0..25_000)
        .map(|index| {
            (
                format!("logical_work_item_{index:05}"),
                format!("work_item_revision_{index:05}"),
            )
        })
        .collect();
    revision
}

#[test]
fn work_item_revision_store_plan_compare_and_set_has_one_concurrent_winner() {
    let temp = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(temp.path().join(".aria"));
    let store = WorkItemRevisionStore::new(paths.clone());
    let plan = large_plan_lineage();
    let first = plan_revision("plan_revision_0001", 1);
    let second = plan_revision("plan_revision_0002", 2);
    let third = plan_revision("plan_revision_0003", 3);
    store.put_plan_lineage(&plan).unwrap();
    store.put_plan_revision(&plan, &first).unwrap();
    store.put_plan_revision(&plan, &second).unwrap();
    store.put_plan_revision(&plan, &third).unwrap();
    store.set_active_plan_revision(&plan, &first.id).unwrap();

    let barrier = Arc::new(Barrier::new(3));
    let handles = [second.id.clone(), third.id.clone()].map(|next_revision_id| {
        let barrier = Arc::clone(&barrier);
        let store = WorkItemRevisionStore::new(paths.clone());
        let plan = plan.clone();
        let expected_revision_id = first.id.clone();
        thread::spawn(move || {
            barrier.wait();
            store.compare_and_set_active_plan_revision(
                &plan,
                &expected_revision_id,
                &next_revision_id,
            )
        })
    });
    barrier.wait();
    let results = handles.map(|handle| handle.join().unwrap());

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
fn work_item_revision_store_work_item_compare_and_set_has_one_concurrent_winner() {
    let temp = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(temp.path().join(".aria"));
    let store = WorkItemRevisionStore::new(paths.clone());
    let plan = plan_lineage();
    let mut logical = logical_work_item();
    logical.title = "x".repeat(2_000_000);
    let first = work_item_revision();
    let mut second = first.clone();
    second.id = "work_item_revision_0002".to_string();
    second.canonical_contract_hash = "contract_hash_0002".to_string();
    store.put_plan_lineage(&plan).unwrap();
    store.put_logical_work_item(&plan, &logical).unwrap();
    store.put_work_item_revision(&plan, &first).unwrap();
    store.put_work_item_revision(&plan, &second).unwrap();

    let barrier = Arc::new(Barrier::new(3));
    let handles = [first.id.clone(), second.id.clone()].map(|next_revision_id| {
        let barrier = Arc::clone(&barrier);
        let store = WorkItemRevisionStore::new(paths.clone());
        let plan = plan.clone();
        let logical = logical.clone();
        thread::spawn(move || {
            barrier.wait();
            store.set_active_work_item_revision(&plan, &logical, None, &next_revision_id)
        })
    });
    barrier.wait();
    let results = handles.map(|handle| handle.join().unwrap());

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
fn work_item_revision_store_allows_one_concurrent_active_amendment() {
    let temp = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(temp.path().join(".aria"));
    let store = WorkItemRevisionStore::new(paths.clone());
    let plan = large_plan_lineage();
    store.put_plan_lineage(&plan).unwrap();

    let barrier = Arc::new(Barrier::new(3));
    let handles = ["amendment_0001", "amendment_0002"].map(|amendment_id| {
        let barrier = Arc::clone(&barrier);
        let store = WorkItemRevisionStore::new(paths.clone());
        let plan = plan.clone();
        thread::spawn(move || {
            barrier.wait();
            store.acquire_active_amendment(&plan, amendment_id)
        })
    });
    barrier.wait();
    let results = handles.map(|handle| handle.join().unwrap());

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
fn work_item_revision_store_allows_one_concurrent_amendment_release() {
    let temp = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(temp.path().join(".aria"));
    let store = WorkItemRevisionStore::new(paths.clone());
    let plan = large_plan_lineage();
    store.put_plan_lineage(&plan).unwrap();
    store
        .acquire_active_amendment(&plan, "amendment_0001")
        .unwrap();

    let barrier = Arc::new(Barrier::new(3));
    let handles = [(), ()].map(|()| {
        let barrier = Arc::clone(&barrier);
        let store = WorkItemRevisionStore::new(paths.clone());
        let plan = plan.clone();
        thread::spawn(move || {
            barrier.wait();
            store.release_active_amendment(&plan, "amendment_0001")
        })
    });
    barrier.wait();
    let results = handles.map(|handle| handle.join().unwrap());

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
fn work_item_revision_store_concurrent_repair_updates_do_not_lose_fields() {
    let temp = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(temp.path().join(".aria"));
    let store = WorkItemRevisionStore::new(paths.clone());
    let plan = plan_lineage();
    store.put_plan_lineage(&plan).unwrap();
    let mut request = repair_request("plan_repair_request_0001");
    request.evidence = (0..25_000)
        .map(|index| json!({"kind": "seed", "index": index}))
        .collect();
    store.put_repair_request(&plan, &request).unwrap();

    let extra = json!({"kind": "concurrent", "id": "evidence_0001"});
    let barrier = Arc::new(Barrier::new(3));
    let status_handle = {
        let barrier = Arc::clone(&barrier);
        let store = WorkItemRevisionStore::new(paths.clone());
        let plan = plan.clone();
        let request_id = request.id.clone();
        thread::spawn(move || {
            barrier.wait();
            store.update_repair_request_status(
                &plan,
                &request_id,
                PlanRepairRequestStatus::InProgress,
            )
        })
    };
    let evidence_handle = {
        let barrier = Arc::clone(&barrier);
        let store = WorkItemRevisionStore::new(paths);
        let plan = plan.clone();
        let request_id = request.id.clone();
        let extra = extra.clone();
        thread::spawn(move || {
            barrier.wait();
            store.merge_repair_request_evidence(&plan, &request_id, vec![extra])
        })
    };
    barrier.wait();
    status_handle.join().unwrap().unwrap();
    evidence_handle.join().unwrap().unwrap();

    let stored = store.list_open_repair_requests(&plan).unwrap().remove(0);
    assert_eq!(stored.status, PlanRepairRequestStatus::InProgress);
    assert!(stored.evidence.contains(&extra));
}

#[test]
fn work_item_revision_store_concurrent_immutable_conflict_never_overwrites() {
    let temp = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(temp.path().join(".aria"));
    let store = WorkItemRevisionStore::new(paths.clone());
    let plan = plan_lineage();
    store.put_plan_lineage(&plan).unwrap();
    let first = large_plan_revision("plan_revision_0002", PlanRevisionReason::SubgraphReplan);
    let second = large_plan_revision("plan_revision_0002", PlanRevisionReason::StoryAmendment);

    let barrier = Arc::new(Barrier::new(3));
    let handles = [first.clone(), second.clone()].map(|revision| {
        let barrier = Arc::clone(&barrier);
        let store = WorkItemRevisionStore::new(paths.clone());
        let plan = plan.clone();
        thread::spawn(move || {
            barrier.wait();
            store.put_plan_revision(&plan, &revision).map(|()| revision)
        })
    });
    barrier.wait();
    let results = handles.map(|handle| handle.join().unwrap());

    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, Err(ProductStoreError::IdentityMismatch { .. })))
            .count(),
        1
    );
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
fn work_item_revision_store_concurrent_immutable_replay_is_idempotent() {
    let temp = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(temp.path().join(".aria"));
    let store = WorkItemRevisionStore::new(paths.clone());
    let plan = plan_lineage();
    store.put_plan_lineage(&plan).unwrap();
    let revision = large_plan_revision("plan_revision_0002", PlanRevisionReason::SubgraphReplan);

    let barrier = Arc::new(Barrier::new(3));
    let handles = [(), ()].map(|()| {
        let barrier = Arc::clone(&barrier);
        let store = WorkItemRevisionStore::new(paths.clone());
        let plan = plan.clone();
        let revision = revision.clone();
        thread::spawn(move || {
            barrier.wait();
            store.put_plan_revision(&plan, &revision)
        })
    });
    barrier.wait();

    for handle in handles {
        handle.join().unwrap().unwrap();
    }
}
