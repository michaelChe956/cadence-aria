use super::*;
use crate::product::coding_models::{CodingProviderRole, CodingUnitRunStatus};
use crate::product::work_item_projection::RenderedExecutionContext;
use std::sync::{Arc, Barrier};

#[test]
fn coding_unit_run_execution_context_rebind_rejects_different_identity() {
    let (_tmp, store) = setup_store();
    let attempt = super::plan_repair::coding_plan_repair_attempt(&store);
    let unit = super::plan_repair::coding_unit_run_unit(
        &store,
        &attempt,
        "wi_core",
        "work_item_revision_0001",
        0,
        CodingExecutionUnitStatus::Running,
    );
    let run = super::plan_repair::coding_unit_run_record(
        "coding_unit_run_0001",
        &unit.id,
        1,
        &unit.work_item_revision_id,
        CodingUnitRunStatus::Running,
    );
    store.create_coding_unit_run(&attempt, &run).unwrap();
    let first = RenderedExecutionContext {
        text: "authoritative context".to_string(),
        renderer_version: "codex-provider-projection-renderer-v1".to_string(),
        content_hash: "context_hash_0001".to_string(),
    };

    store
        .bind_unit_run_execution_context(&attempt, &run.id, CodingProviderRole::Coder, &first)
        .expect("initial binding");
    store
        .bind_unit_run_execution_context(&attempt, &run.id, CodingProviderRole::Coder, &first)
        .expect("identical binding is idempotent");

    let different = RenderedExecutionContext {
        text: "different context".to_string(),
        renderer_version: "codex-provider-projection-renderer-v2".to_string(),
        content_hash: "context_hash_0002".to_string(),
    };
    assert!(matches!(
        store.bind_unit_run_execution_context(
            &attempt,
            &run.id,
            CodingProviderRole::Coder,
            &different,
        ),
        Err(ProductStoreError::IdentityMismatch {
            kind: "coding_unit_run_execution_context",
            ..
        })
    ));

    let persisted = store.get_active_unit_run(&attempt).unwrap();
    assert_eq!(
        persisted.coder_provider_renderer_version,
        first.renderer_version
    );
    assert_eq!(
        persisted.coder_execution_context_hash.as_deref(),
        Some(first.content_hash.as_str())
    );
}

#[test]
fn coding_plan_repair_unit_run_materialization_is_concurrent_idempotent() {
    let (_tmp, store) = setup_store();
    let attempt = super::plan_repair::coding_plan_repair_attempt(&store);
    let unit = super::plan_repair::coding_unit_run_unit(
        &store,
        &attempt,
        "wi_core",
        "work_item_revision_0001",
        0,
        CodingExecutionUnitStatus::Running,
    );
    let mut left = super::plan_repair::coding_unit_run_record(
        "coding_unit_run_0001",
        &unit.id,
        1,
        &unit.work_item_revision_id,
        CodingUnitRunStatus::Running,
    );
    left.resolved_handoff_revision_ids.clear();
    left.unit_rework_count = 0;
    left.verification_retry_count = 0;
    left.operational_retry_count = 0;
    left.plan_repair_count = 0;
    left.created_at = "caller-left-must-not-persist".to_string();
    left.updated_at = left.created_at.clone();
    let mut right = left.clone();
    right.created_at = "caller-right-must-not-persist".to_string();
    right.updated_at = right.created_at.clone();
    let barrier = Arc::new(Barrier::new(3));

    let left_task = {
        let store = store.clone();
        let attempt = attempt.clone();
        let barrier = barrier.clone();
        std::thread::spawn(move || {
            barrier.wait();
            store.load_or_create_coding_unit_run(&attempt, &left)
        })
    };
    let right_task = {
        let store = store.clone();
        let attempt = attempt.clone();
        let barrier = barrier.clone();
        std::thread::spawn(move || {
            barrier.wait();
            store.load_or_create_coding_unit_run(&attempt, &right)
        })
    };
    barrier.wait();

    let left = left_task.join().unwrap().expect("left materialization");
    let right = right_task.join().unwrap().expect("right materialization");
    assert_eq!(left, right);
    assert_eq!(
        store.list_coding_unit_runs(&attempt, &unit.id).unwrap(),
        vec![left.clone()]
    );
    chrono::DateTime::parse_from_rfc3339(&left.created_at).expect("real created_at");
    assert_eq!(left.created_at, left.updated_at);

    let mut conflicting = left;
    conflicting.canonical_contract_hash = "different_contract_hash".to_string();
    assert!(matches!(
        store.load_or_create_coding_unit_run(&attempt, &conflicting),
        Err(ProductStoreError::IdentityMismatch {
            kind: "coding_unit_run",
            ..
        })
    ));
}
