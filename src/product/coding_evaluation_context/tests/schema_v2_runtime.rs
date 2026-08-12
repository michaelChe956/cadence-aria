use super::*;

use crate::product::coding_evaluation_context::builder::schema_v2_active_unit_runtime;
use crate::product::coding_models::CodingAttemptScope;
use crate::product::json_store::ProductStoreError;
use crate::product::models::WorkItemPlanLineage;
use crate::product::work_item_revision_store::WorkItemRevisionStore;

#[test]
fn schema_v2_attempt_without_active_unit_binding_fails_closed() {
    let temp = TempDir::new().expect("tempdir");
    let paths = ProductAppPaths::new(temp.path().join(".aria"));
    let plan_id = "work_item_plan_schema_v2";
    WorkItemRevisionStore::new(paths.clone())
        .put_plan_lineage(&WorkItemPlanLineage {
            id: plan_id.to_string(),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            story_spec_refs: Vec::new(),
            design_spec_refs: Vec::new(),
            active_revision_id: None,
            active_amendment_id: None,
            created_at: "2026-07-27T00:00:00Z".to_string(),
            updated_at: "2026-07-27T00:00:00Z".to_string(),
        })
        .expect("store schema v2 plan lineage");
    let attempt = CodingExecutionAttempt {
        id: "coding_attempt_schema_v2".to_string(),
        project_id: PROJECT_ID.to_string(),
        issue_id: ISSUE_ID.to_string(),
        work_item_id: "work_item_schema_v2".to_string(),
        attempt_no: 1,
        scope: CodingAttemptScope::WorkItemGroup,
        status: CodingAttemptStatus::Running,
        version: 0,
        manual_recovery_reason: None,
        admission_ticket_consumed_at: None,
        stage: CodingExecutionStage::Coding,
        base_branch: "main".to_string(),
        branch_name: "aria/work-items/schema-v2".to_string(),
        worktree_path: None,
        provider_config_snapshot: ProviderConfigSnapshot {
            author: ProviderName::Codex,
            reviewer: Some(ProviderName::ClaudeCode),
            review_rounds: 1,
            permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
        },
        rework_count: 0,
        max_auto_rework: 2,
        work_item_group_id: Some(plan_id.to_string()),
        current_work_item_id: Some("work_item_schema_v2".to_string()),
        active_unit_id: None,
        head_commit: None,
        pushed_remote: None,
        review_request_id: None,
        provider_conversations: Vec::new(),
        created_at: "2026-07-27T00:00:00Z".to_string(),
        updated_at: "2026-07-27T00:00:00Z".to_string(),
        target_snapshot: None,
        completed_at: None,
    };

    let error = schema_v2_active_unit_runtime(&paths, &attempt)
        .expect_err("schema v2 runtime must not fall back when its binding is missing");

    assert!(matches!(
        error,
        ProductStoreError::IdentityMismatch {
            kind: "runtime_binding_missing",
            ..
        }
    ));
}
