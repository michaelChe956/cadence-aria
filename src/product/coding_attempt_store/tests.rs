use tempfile::TempDir;

use super::*;
use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_models::{
    CodingAttemptScope, CodingAttemptStatus, CodingExecutionStage, CodingExecutionUnitStatus,
    CodingGateAction, CodingGateActionType, CodingProviderRole,
};
use crate::product::models::ProviderName;
use crate::web::workspace_ws_types::ProviderConfigSnapshot;

mod attempt_creation_concurrency;
mod failed_review_recovery;
mod failed_review_recovery_rollback;
mod git_operation;
mod group_uniqueness;
mod plan_repair;
mod provider_stream_log_root;
mod role_run;
mod unit_run_execution_context;

const PROJECT_ID: &str = "project_0001";
const ISSUE_ID: &str = "issue_0001";
const WORK_ITEM_ID: &str = "work_item_0001";

fn setup_store() -> (TempDir, CodingAttemptStore) {
    let tmp = TempDir::new().unwrap();
    let store = CodingAttemptStore::new(ProductAppPaths::new(tmp.path().join(".aria")));
    (tmp, store)
}

fn setup() -> (TempDir, CodingAttemptStore, CodingExecutionAttempt) {
    let (tmp, store) = setup_store();
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            work_item_id: WORK_ITEM_ID.to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
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
    (tmp, store, attempt)
}

fn provider_snapshot() -> ProviderConfigSnapshot {
    ProviderConfigSnapshot {
        author: ProviderName::Codex,
        reviewer: Some(ProviderName::ClaudeCode),
        review_rounds: 1,
        permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
    }
}

#[test]
fn updates_attempt_max_auto_rework_with_range_validation() {
    let (_tmp, store, attempt) = setup();

    let updated = store
        .update_attempt_max_auto_rework(&attempt.project_id, &attempt.issue_id, &attempt.id, 4)
        .expect("update max auto rework");

    assert_eq!(updated.max_auto_rework, 4);

    let rejected = store.update_attempt_max_auto_rework(
        &attempt.project_id,
        &attempt.issue_id,
        &attempt.id,
        6,
    );
    assert!(matches!(
        rejected,
        Err(ProductStoreError::Io(message)) if message == "invalid_max_auto_rework: 6"
    ));

    let unchanged = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("attempt");
    assert_eq!(unchanged.max_auto_rework, 4);
}

#[test]
fn code_review_can_route_directly_back_to_coding_for_reviewer_feedback() {
    let (_tmp, store, attempt) = setup();
    let attempt = store
        .update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::Coding,
        )
        .expect("coding stage");
    let attempt = store
        .update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::CodeReview,
        )
        .expect("code review stage");

    let updated = store
        .update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::Coding,
        )
        .expect("code review can return to coder");

    assert_eq!(updated.stage, CodingExecutionStage::Coding);
}

#[test]
fn failed_code_review_attempt_cannot_be_reopened_as_recoverable() {
    let (_tmp, store, attempt) = setup();
    let attempt = store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("start attempt");
    let attempt = store
        .update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::CodeReview,
        )
        .expect("enter code review");
    let failed = store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Failed,
        )
        .expect("fail attempt");
    assert!(failed.completed_at.is_some());

    let error = store
        .reopen_failed_code_review_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect_err("terminal failed code review cannot become recoverable");
    assert!(
        error
            .to_string()
            .contains("coding_failed_review_not_recoverable")
    );
    assert_eq!(
        store
            .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("terminal attempt remains readable"),
        failed
    );
}

#[test]
fn reopen_failed_code_review_attempt_rejects_other_terminal_states_without_persisting_changes() {
    let cases = [
        (
            "completed code review",
            CodingExecutionStage::CodeReview,
            CodingAttemptStatus::Completed,
        ),
        (
            "aborted code review",
            CodingExecutionStage::CodeReview,
            CodingAttemptStatus::Aborted,
        ),
    ];

    for (case_name, stage, status) in cases {
        let (_tmp, store, attempt) = setup();
        let attempt = store
            .update_attempt_status(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                CodingAttemptStatus::Running,
            )
            .unwrap_or_else(|error| panic!("{case_name}: start attempt: {error}"));
        let attempt = store
            .update_attempt_stage(&attempt.project_id, &attempt.issue_id, &attempt.id, stage)
            .unwrap_or_else(|error| panic!("{case_name}: enter terminal stage: {error}"));
        let terminal = store
            .update_attempt_status(&attempt.project_id, &attempt.issue_id, &attempt.id, status)
            .unwrap_or_else(|error| panic!("{case_name}: enter terminal status: {error}"));

        let rejected = store.reopen_failed_code_review_attempt(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
        );

        assert!(
            matches!(
                rejected,
                Err(ProductStoreError::Io(ref message))
                    if message == "coding_failed_review_not_recoverable"
            ),
            "{case_name}: unexpected result: {rejected:?}"
        );
        let persisted = store
            .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .unwrap_or_else(|error| panic!("{case_name}: reload attempt: {error}"));
        assert_eq!(
            persisted, terminal,
            "{case_name}: persisted attempt changed"
        );
    }
}

#[test]
fn legacy_attempt_without_scope_deserializes_as_work_item_scope() {
    let json = serde_json::json!({
        "id": "coding_attempt_0001",
        "project_id": "project_0001",
        "issue_id": "issue_0001",
        "work_item_id": "work_item_0001",
        "attempt_no": 1,
        "status": "created",
        "stage": "prepare_context",
        "base_branch": "main",
        "branch_name": "aria/issues/issue_0001",
        "worktree_path": null,
        "provider_config_snapshot": { "author": "codex", "reviewer": "codex", "review_rounds": 1 },
        "rework_count": 0,
        "max_auto_rework": 2,
        "head_commit": null,
        "pushed_remote": null,
        "review_request_id": null,
        "provider_conversations": [],
        "created_at": "2026-06-27T00:00:00Z",
        "updated_at": "2026-06-27T00:00:00Z",
        "completed_at": null
    });

    let attempt: CodingExecutionAttempt = serde_json::from_value(json).expect("attempt");

    assert_eq!(attempt.scope, CodingAttemptScope::WorkItem);
    assert_eq!(
        attempt.current_work_item_id.as_deref(),
        Some("work_item_0001")
    );
    assert!(attempt.work_item_group_id.is_none());
}

#[test]
fn saving_group_attempt_preserves_explicit_internal_reviewer_role_config() {
    let (_tmp, store) = setup_store();
    let group_attempt = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_work_item_id: WORK_ITEM_ID.to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::Codex),
                review_rounds: 1,
                permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
            },
            max_auto_rework: 2,
        })
        .expect("group attempt");

    let mut role_config = store
        .get_role_provider_config_snapshot(PROJECT_ID, ISSUE_ID, &group_attempt.id)
        .expect("bootstrap role config");
    assert_eq!(role_config.internal_reviewer, ProviderName::Codex);

    role_config.internal_reviewer = ProviderName::ClaudeCode;
    store
        .update_role_provider_config_snapshot(PROJECT_ID, ISSUE_ID, &group_attempt.id, role_config)
        .expect("select internal reviewer");

    store
        .save_coding_attempt(&group_attempt)
        .expect("save after group unit completion");

    let persisted = store
        .get_role_provider_config_snapshot(PROJECT_ID, ISSUE_ID, &group_attempt.id)
        .expect("persisted role config");
    assert_eq!(persisted.internal_reviewer, ProviderName::ClaudeCode);
}

#[test]
fn creates_group_attempt_and_units_with_single_active_unit() {
    let (_tmp, store) = setup_store();

    let group_attempt = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_work_item_id: "work_item_0001".to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: None,
            provider_config_snapshot: provider_snapshot(),
            max_auto_rework: 2,
        })
        .expect("group attempt");

    store
        .create_coding_unit(CreateCodingExecutionUnitInput {
            attempt_id: group_attempt.id.clone(),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            logical_work_item_id: "work_item_0001".to_string(),
            work_item_revision_id: "work_item_revision_0001".to_string(),
            dependency_logical_work_item_ids: Vec::new(),
            order_index: 0,
            status: CodingExecutionUnitStatus::Running,
        })
        .expect("unit 1");
    store
        .create_coding_unit(CreateCodingExecutionUnitInput {
            attempt_id: group_attempt.id.clone(),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            logical_work_item_id: "work_item_0002".to_string(),
            work_item_revision_id: "work_item_revision_0002".to_string(),
            dependency_logical_work_item_ids: vec!["work_item_0001".to_string()],
            order_index: 1,
            status: CodingExecutionUnitStatus::Pending,
        })
        .expect("unit 2");

    let units = store
        .list_coding_units("project_0001", "issue_0001", &group_attempt.id)
        .expect("units");
    let active = store
        .get_active_coding_unit("project_0001", "issue_0001", &group_attempt.id)
        .expect("active lookup")
        .expect("active");

    assert_eq!(group_attempt.scope, CodingAttemptScope::WorkItemGroup);
    assert_eq!(
        group_attempt.work_item_group_id.as_deref(),
        Some("work_item_plan_0001")
    );
    assert_eq!(units.len(), 2);
    assert_eq!(active.logical_work_item_id, "work_item_0001");
}

#[test]
fn rejects_creating_second_active_unit_for_same_attempt() {
    let (_tmp, store) = setup_store();
    let group_attempt = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_work_item_id: WORK_ITEM_ID.to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: None,
            provider_config_snapshot: provider_snapshot(),
            max_auto_rework: 2,
        })
        .expect("group attempt");

    store
        .create_coding_unit(CreateCodingExecutionUnitInput {
            attempt_id: group_attempt.id.clone(),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            logical_work_item_id: WORK_ITEM_ID.to_string(),
            work_item_revision_id: "work_item_revision_0001".to_string(),
            dependency_logical_work_item_ids: Vec::new(),
            order_index: 0,
            status: CodingExecutionUnitStatus::Running,
        })
        .expect("first running unit");

    let error = store
        .create_coding_unit(CreateCodingExecutionUnitInput {
            attempt_id: group_attempt.id.clone(),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            logical_work_item_id: "work_item_0002".to_string(),
            work_item_revision_id: "work_item_revision_0002".to_string(),
            dependency_logical_work_item_ids: vec![WORK_ITEM_ID.to_string()],
            order_index: 1,
            status: CodingExecutionUnitStatus::Running,
        })
        .expect_err("should reject second active unit");

    assert!(error.to_string().contains("active_coding_unit_exists"));
}

#[test]
fn rejects_updating_pending_unit_to_active_when_another_unit_is_active() {
    let (_tmp, store) = setup_store();
    let group_attempt = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_work_item_id: WORK_ITEM_ID.to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: None,
            provider_config_snapshot: provider_snapshot(),
            max_auto_rework: 2,
        })
        .expect("group attempt");

    let running = store
        .create_coding_unit(CreateCodingExecutionUnitInput {
            attempt_id: group_attempt.id.clone(),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            logical_work_item_id: WORK_ITEM_ID.to_string(),
            work_item_revision_id: "work_item_revision_0001".to_string(),
            dependency_logical_work_item_ids: Vec::new(),
            order_index: 0,
            status: CodingExecutionUnitStatus::Running,
        })
        .expect("running unit");
    let pending = store
        .create_coding_unit(CreateCodingExecutionUnitInput {
            attempt_id: group_attempt.id.clone(),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            logical_work_item_id: "work_item_0002".to_string(),
            work_item_revision_id: "work_item_revision_0002".to_string(),
            dependency_logical_work_item_ids: vec![WORK_ITEM_ID.to_string()],
            order_index: 1,
            status: CodingExecutionUnitStatus::Pending,
        })
        .expect("pending unit");

    let error = store
        .update_coding_unit_status(
            PROJECT_ID,
            ISSUE_ID,
            &group_attempt.id,
            &pending.id,
            CodingExecutionUnitStatus::Running,
            None,
        )
        .expect_err("should reject conflicting active update");

    assert!(error.to_string().contains("active_coding_unit_exists"));
    let reloaded_running = store
        .get_active_coding_unit(PROJECT_ID, ISSUE_ID, &group_attempt.id)
        .expect("active lookup")
        .expect("active unit");
    assert_eq!(reloaded_running.id, running.id);
}

#[test]
fn rejects_group_attempt_when_active_group_attempt_already_exists_for_other_plan() {
    let (_tmp, store) = setup_store();

    let first = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_work_item_id: WORK_ITEM_ID.to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: None,
            provider_config_snapshot: provider_snapshot(),
            max_auto_rework: 2,
        })
        .expect("first group attempt");

    let error = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            plan_id: "work_item_plan_0002".to_string(),
            current_work_item_id: "work_item_0002".to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/issues/issue_0001-b".to_string(),
            worktree_path: None,
            provider_config_snapshot: provider_snapshot(),
            max_auto_rework: 2,
        })
        .expect_err("should reject second active attempt");

    assert_eq!(
        error.to_string(),
        format!(
            "product_store_io: active_coding_attempt_exists: {}",
            first.id
        )
    );
}

#[test]
fn rejects_group_attempt_when_active_work_item_attempt_exists() {
    let (_tmp, store, attempt) = setup();

    let error = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_work_item_id: "work_item_0002".to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: None,
            provider_config_snapshot: provider_snapshot(),
            max_auto_rework: 2,
        })
        .expect_err("should reject when single attempt is active");

    assert_eq!(
        error.to_string(),
        format!(
            "product_store_io: active_coding_attempt_exists: {}",
            attempt.id
        )
    );
}

#[test]
fn clears_current_work_item_when_last_active_unit_completes() {
    let (_tmp, store) = setup_store();
    let group_attempt = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_work_item_id: WORK_ITEM_ID.to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: None,
            provider_config_snapshot: provider_snapshot(),
            max_auto_rework: 2,
        })
        .expect("group attempt");

    let running = store
        .create_coding_unit(CreateCodingExecutionUnitInput {
            attempt_id: group_attempt.id.clone(),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            logical_work_item_id: WORK_ITEM_ID.to_string(),
            work_item_revision_id: "work_item_revision_0001".to_string(),
            dependency_logical_work_item_ids: Vec::new(),
            order_index: 0,
            status: CodingExecutionUnitStatus::Running,
        })
        .expect("running unit");

    store
        .update_coding_unit_status(
            PROJECT_ID,
            ISSUE_ID,
            &group_attempt.id,
            &running.id,
            CodingExecutionUnitStatus::Completed,
            Some("done".to_string()),
        )
        .expect("complete running unit");

    let reloaded_attempt = store
        .get_attempt(PROJECT_ID, ISSUE_ID, &group_attempt.id)
        .expect("reload attempt");
    assert!(reloaded_attempt.active_unit_id.is_none());
    assert!(reloaded_attempt.current_work_item_id.is_none());
}

#[test]
fn blocked_or_waiting_units_do_not_set_started_at() {
    let (_tmp, store) = setup_store();
    let group_attempt = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_work_item_id: WORK_ITEM_ID.to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: None,
            provider_config_snapshot: provider_snapshot(),
            max_auto_rework: 2,
        })
        .expect("group attempt");

    let blocked = store
        .create_coding_unit(CreateCodingExecutionUnitInput {
            attempt_id: group_attempt.id.clone(),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            logical_work_item_id: WORK_ITEM_ID.to_string(),
            work_item_revision_id: "work_item_revision_0001".to_string(),
            dependency_logical_work_item_ids: Vec::new(),
            order_index: 0,
            status: CodingExecutionUnitStatus::Blocked,
        })
        .expect("blocked unit");
    assert!(blocked.started_at.is_none());

    store
        .update_coding_unit_status(
            PROJECT_ID,
            ISSUE_ID,
            &group_attempt.id,
            &blocked.id,
            CodingExecutionUnitStatus::Completed,
            None,
        )
        .expect("complete blocked unit");

    let pending = store
        .create_coding_unit(CreateCodingExecutionUnitInput {
            attempt_id: group_attempt.id.clone(),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            logical_work_item_id: "work_item_0002".to_string(),
            work_item_revision_id: "work_item_revision_0002".to_string(),
            dependency_logical_work_item_ids: vec![WORK_ITEM_ID.to_string()],
            order_index: 1,
            status: CodingExecutionUnitStatus::Pending,
        })
        .expect("pending unit");
    assert!(pending.started_at.is_none());

    let waiting = store
        .update_coding_unit_status(
            PROJECT_ID,
            ISSUE_ID,
            &group_attempt.id,
            &pending.id,
            CodingExecutionUnitStatus::WaitingForHuman,
            None,
        )
        .expect("waiting unit");
    assert!(waiting.started_at.is_none());
}

#[test]
fn blocked_gate_creation_is_idempotent_for_same_node_and_reason() {
    let (_tmp, store, attempt) = setup();
    let first = store
        .create_blocked_gate(
            &attempt,
            CreateBlockedGateInput {
                attempt_id: attempt.id.clone(),
                stage: CodingExecutionStage::CodeReview,
                node_id: Some("coding_node_0001".to_string()),
                role: Some(CodingProviderRole::CodeReviewer),
                title: "Code Review blocked".to_string(),
                description: "review requires retry".to_string(),
                reason_code: Some("review_retry_required".to_string()),
                evidence_refs: vec!["code_review_report_0001.json".to_string()],
                raw_provider_output_ref: None,
                available_actions: vec![CodingGateAction {
                    action_id: "retry_review".to_string(),
                    label: "重试审查".to_string(),
                    action_type: CodingGateActionType::RetryReview,
                }],
            },
        )
        .unwrap();

    let second = store
        .create_blocked_gate(
            &attempt,
            CreateBlockedGateInput {
                attempt_id: attempt.id.clone(),
                stage: CodingExecutionStage::CodeReview,
                node_id: Some("coding_node_0001".to_string()),
                role: Some(CodingProviderRole::CodeReviewer),
                title: "Code Review still blocked".to_string(),
                description: "review still requires retry".to_string(),
                reason_code: Some("review_retry_required".to_string()),
                evidence_refs: vec![
                    "code_review_report_0001.json".to_string(),
                    "code_review_report_0002.json".to_string(),
                ],
                raw_provider_output_ref: None,
                available_actions: vec![CodingGateAction {
                    action_id: "retry_review".to_string(),
                    label: "重试审查".to_string(),
                    action_type: CodingGateActionType::RetryReview,
                }],
            },
        )
        .unwrap();

    assert_eq!(second.gate_id, first.gate_id);
    let open = store
        .list_open_blocked_gates(PROJECT_ID, ISSUE_ID, &attempt.id)
        .unwrap();
    assert_eq!(open.len(), 1);
    assert_eq!(
        open[0].evidence_refs,
        vec![
            "code_review_report_0001.json",
            "code_review_report_0002.json"
        ]
    );
    assert_eq!(
        open[0].available_actions[0].action_type,
        CodingGateActionType::RetryReview
    );
}
