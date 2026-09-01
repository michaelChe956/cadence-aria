use std::sync::Arc;

use super::*;
use crate::product::coding_attempt_store::CreateGroupCodingAttemptInput;
use crate::product::coding_models::{
    CodingAdmissionKind, CodingExecutionStage, CodingExecutionUnitStatus,
};
use crate::product::coding_workspace_engine::group_dependency_gate::dependency_gate_applies;
use crate::product::git_workspace_service::GitWorkspaceService;
use crate::product::lifecycle_store::{LifecycleStore, UpsertIssueSharedWorktreeInput};
use crate::product::models::ProviderName;
use crate::web::workspace_ws_types::ProviderConfigSnapshot;
use tokio::sync::{Barrier, mpsc};

fn fixture(
    with_dependency: bool,
) -> (
    tempfile::TempDir,
    CodingAttemptStore,
    CodingExecutionAttempt,
) {
    let root = tempfile::tempdir().expect("root");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_work_item_id: "work_item_0001".to_string(),
            base_branch: "HEAD".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Fake,
                reviewer: Some(ProviderName::Fake),
                review_rounds: 1,
                permission_modes: Default::default(),
            },
            target_snapshot: None,
            max_auto_rework: 2,
        })
        .expect("group attempt");
    super::seed_group_attempt_fixture(&store, &attempt, true, with_dependency);
    let mut attempt = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("persisted attempt");
    attempt.admission_kind = CodingAdmissionKind::ScAdvance;
    store
        .write_coding_attempt_for_test(&attempt)
        .expect("SC admission kind");
    (root, store, attempt)
}

fn unit(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    logical_id: &str,
) -> crate::product::coding_models::CodingExecutionUnit {
    store
        .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("units")
        .into_iter()
        .find(|unit| unit.logical_work_item_id == logical_id)
        .expect("unit")
}

fn set_dependencies(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    logical_id: &str,
    dependencies: &[&str],
) {
    let unit = unit(store, attempt, logical_id);
    let path = store.coding_unit_path(
        &attempt.project_id,
        &attempt.issue_id,
        &attempt.id,
        &unit.id,
    );
    let mut unit: crate::product::coding_models::CodingExecutionUnit =
        serde_json::from_slice(&std::fs::read(&path).expect("coding unit JSON"))
            .expect("coding unit JSON value");
    unit.dependency_logical_work_item_ids = dependencies
        .iter()
        .map(|dependency| (*dependency).to_string())
        .collect();
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&unit).expect("coding unit JSON encoding"),
    )
    .expect("coding unit JSON");
}

#[tokio::test]
async fn sc_group_dependency_gate_allows_only_one_active_unit_under_concurrency() {
    let (root, store, attempt) = fixture(false);
    let first = unit(&store, &attempt, "work_item_0001");
    store
        .update_coding_unit_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &first.id,
            CodingExecutionUnitStatus::Completed,
            Some("completed initial unit".to_string()),
        )
        .expect("complete initial unit");
    let lifecycle = LifecycleStore::new(store.paths());
    let worktree_path = root.path().join("shared-worktree");
    std::fs::create_dir_all(&worktree_path).expect("worktree directory");
    lifecycle
        .upsert_issue_shared_worktree(UpsertIssueSharedWorktreeInput {
            project_id: attempt.project_id.clone(),
            issue_id: attempt.issue_id.clone(),
            repository_id: "repository_0001".to_string(),
            branch_name: attempt.branch_name.clone(),
            worktree_path,
            base_branch: attempt.base_branch.clone(),
        })
        .expect("shared worktree");
    lifecycle
        .try_acquire_issue_worktree_lock(
            &attempt.project_id,
            &attempt.issue_id,
            "work_item_0001",
            &attempt.id,
        )
        .expect("initial worktree owner");
    let (event_tx, _event_rx) = mpsc::channel(8);
    let engines = [
        CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), event_tx.clone()),
        CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), event_tx),
    ];
    let start = Arc::new(Barrier::new(3));
    let handles = engines.map(|engine| {
        let attempt = attempt.clone();
        let start = Arc::clone(&start);
        tokio::spawn(async move {
            start.wait().await;
            engine.advance_to_next_group_unit(&attempt).await
        })
    });
    start.wait().await;
    let [first_handle, second_handle] = handles;
    let (first_result, second_result) = tokio::join!(first_handle, second_handle);
    let results = [
        first_result.expect("first concurrent advance task"),
        second_result.expect("second concurrent advance task"),
    ];
    assert!(results.iter().all(|result| {
        result.as_ref().is_ok_and(|updated| {
            updated.active_unit_id.is_some()
                && updated.current_work_item_id.as_deref() == Some("work_item_0002")
        })
    }));
    let active = store
        .get_active_coding_unit(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("active unit")
        .expect("one active unit");
    assert_eq!(active.logical_work_item_id, "work_item_0002");
    let persisted = store
        .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("persisted units");
    assert_eq!(
        persisted
            .iter()
            .filter(|unit| unit.status == CodingExecutionUnitStatus::Running)
            .count(),
        1
    );
    assert_eq!(
        persisted
            .iter()
            .filter(|unit| unit.status == CodingExecutionUnitStatus::Pending)
            .count(),
        1
    );
    let persisted_attempt = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("attempt");
    assert_eq!(
        persisted_attempt.active_unit_id.as_deref(),
        Some(active.id.as_str())
    );
    // Concurrent selector snapshots are diagnostic artifacts and may reflect the last Ready
    // writer; the load-bearing assertion is durable active_unit_id + shared-worktree ownership.
    let worktree = lifecycle
        .get_issue_shared_worktree(&attempt.project_id, &attempt.issue_id)
        .expect("shared worktree")
        .expect("shared worktree record");
    assert_eq!(
        worktree.current_active_work_item_id.as_deref(),
        Some("work_item_0002")
    );
    assert_eq!(
        worktree.current_lock_owner_id.as_deref(),
        Some(attempt.id.as_str())
    );
}

#[tokio::test]
async fn legacy_group_admission_bypasses_dependency_gate_and_uses_order_index() {
    let (_root, store, original) = fixture(true);
    let mut attempt = store
        .get_attempt(&original.project_id, &original.issue_id, &original.id)
        .expect("attempt");
    attempt.admission_kind = CodingAdmissionKind::LegacyGroup;
    store
        .write_coding_attempt_for_test(&attempt)
        .expect("legacy admission kind");
    let first = unit(&store, &attempt, "work_item_0001");
    set_dependencies(&store, &attempt, "work_item_0002", &["unknown_dependency"]);
    store
        .update_coding_unit_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &first.id,
            CodingExecutionUnitStatus::Completed,
            Some("completed dependency-incomplete unit".to_string()),
        )
        .expect("complete first unit");
    let before_flow_kind = serde_json::to_value(&attempt)
        .expect("legacy attempt JSON before advance")
        .get("flow_kind")
        .cloned();
    let (event_tx, _event_rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), event_tx);
    let updated = engine
        .advance_to_next_group_unit(&attempt)
        .await
        .expect("legacy advance");
    assert_eq!(updated.admission_kind, CodingAdmissionKind::LegacyGroup);
    assert_eq!(updated.stage, CodingExecutionStage::PrepareContext);
    assert_eq!(
        updated.current_work_item_id.as_deref(),
        Some("work_item_0002")
    );
    assert_eq!(
        store
            .get_active_coding_unit(&updated.project_id, &updated.issue_id, &updated.id)
            .expect("active unit")
            .expect("legacy unit")
            .logical_work_item_id,
        "work_item_0002"
    );
    let after_flow_kind = serde_json::to_value(&updated)
        .expect("legacy attempt JSON after advance")
        .get("flow_kind")
        .cloned();
    assert_eq!(after_flow_kind, before_flow_kind);
    assert!(!dependency_gate_applies(&updated));
    assert!(
        store
            .get_group_dependency_gate_snapshot(&updated)
            .expect("legacy gate snapshot lookup")
            .is_none()
    );
}

#[test]
fn legacy_group_old_json_defaults_to_legacy_admission() {
    let (_root, store, original) = fixture(false);
    let path = store.attempt_path(&original.project_id, &original.issue_id, &original.id);
    let mut json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("attempt JSON"))
            .expect("attempt JSON value");
    json.as_object_mut()
        .expect("attempt object")
        .remove("admission_kind");
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&json).expect("legacy JSON encoding"),
    )
    .expect("legacy attempt JSON");
    let restored = store
        .get_attempt(&original.project_id, &original.issue_id, &original.id)
        .expect("legacy attempt");
    assert_eq!(restored.admission_kind, CodingAdmissionKind::LegacyGroup);
    assert!(!dependency_gate_applies(&restored));
}
