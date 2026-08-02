use super::*;
use crate::product::models::HandoffRevision;
use crate::web::workspace_ws_types::ProviderConfigSnapshot;
use std::collections::BTreeMap;

#[tokio::test]
async fn coding_runtime_handoff_without_amendment_keeps_normal_group_path_unchanged() {
    let root = tempdir().unwrap();
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
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
                permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
            },
            max_auto_rework: 2,
        })
        .unwrap();
    seed_group_attempt_fixture(&store, &attempt, true, true);
    let units_before = store
        .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .unwrap();
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let handoff = HandoffRevision {
        id: "handoff_revision_0001".to_string(),
        logical_work_item_id: "work_item_0001".to_string(),
        work_item_revision_id: "work_item_revision_0001".to_string(),
        coding_unit_run_id: "coding_unit_run_0001".to_string(),
        provided_contracts: vec!["contract_work_item_0001".to_string()],
        provided_capabilities: BTreeMap::from([(
            "contract_work_item_0001".to_string(),
            vec!["capability_work_item_0001".to_string()],
        )]),
        contract_hash: "contract_hash_v1".to_string(),
        commit_sha: "commit_v1".to_string(),
        created_at: "2026-07-20T00:00:00Z".to_string(),
    };

    let result = engine
        .apply_completed_handoff(&attempt, &handoff)
        .await
        .unwrap();

    assert_eq!(result, RuntimeHandoffImpactResult::default());
    assert_eq!(
        store
            .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .unwrap(),
        units_before
    );
}
