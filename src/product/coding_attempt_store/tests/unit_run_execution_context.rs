use super::*;
use crate::product::coding_models::{CodingProviderRole, CodingUnitRunStatus};
use crate::product::work_item_projection::RenderedExecutionContext;

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
