use tempfile::TempDir;

use super::*;
use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_models::{
    AttemptTargetSnapshot, CodingAttemptScope, CodingAttemptStatus, CodingExecutionStage,
    CodingExecutionUnitStatus, CodingGateAction, CodingGateActionType, CodingProviderRole,
    FindingSeverity, GroupFinalReadinessDiagnostic, GroupFinalReadinessDiagnosticKind,
    GroupFinalReadinessSnapshot, GroupFinalReadinessStatus, GroupFinalReadinessUnit, ReviewFinding,
    ReviewVerdict,
};
use crate::product::json_store::write_json;
use crate::product::logical_codebase::{LogicalRepositoryId, RepositoryCheckoutId};
use crate::product::models::{
    PlanDefectClass, PlanDefectRoute, ProviderName, RepairTarget, RepairTargetKind,
};
use crate::web::workspace_ws_types::ProviderConfigSnapshot;

mod attempt_creation_concurrency;
mod failed_review_recovery;
mod failed_review_recovery_rollback;
mod git_operation;
mod group_final_readiness;
mod group_uniqueness;
mod plan_repair;
mod provider_stream_log_root;
mod role_run;
mod unit_run_execution_context;

const PROJECT_ID: &str = "project_0001";
const ISSUE_ID: &str = "issue_0001";
const WORK_ITEM_ID: &str = "work_item_0001";
type MissingEvidenceCase = (&'static str, fn(&mut GroupFinalReadinessUnit));

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
            target_snapshot: None,
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
fn update_attempt_non_status_fields_preserves_status_and_frozen_admission_fields() {
    let (_tmp, store, attempt) = setup();
    let original_status = attempt.status.clone();
    let original_target_snapshot = attempt.target_snapshot.clone();
    let original_version = attempt.version;
    let original_manual_recovery_reason = attempt.manual_recovery_reason.clone();
    let mut replacement = attempt.clone();
    replacement.status = CodingAttemptStatus::Running;
    replacement.version = original_version + 1;
    replacement.manual_recovery_reason = Some("replacement recovery reason".to_string());
    replacement.target_snapshot = Some(AttemptTargetSnapshot {
        logical_repository_id: LogicalRepositoryId(uuid::Uuid::nil()),
        checkout_id: RepositoryCheckoutId(uuid::Uuid::nil()),
        physical_repository_id: "repository_replacement".to_string(),
        canonical_path: std::path::PathBuf::from("/replacement/repository"),
        git_dir_identity: "replacement-git-dir".to_string(),
        revision: Some("replacement-revision".to_string()),
        policy_digest: "replacement-policy".to_string(),
        membership_revision: 99,
        captured_at: "2026-08-11T00:00:00Z".to_string(),
        capture_source: "test".to_string(),
    });
    replacement.head_commit = Some("head_commit_replacement".to_string());
    replacement.pushed_remote = Some("origin/replacement".to_string());

    store
        .update_attempt_non_status_fields(&replacement)
        .expect("update non-status fields");

    let persisted = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("reload attempt");
    assert_eq!(persisted.status, original_status);
    assert_eq!(persisted.version, original_version);
    assert_eq!(
        persisted.manual_recovery_reason,
        original_manual_recovery_reason
    );
    assert_eq!(persisted.target_snapshot, original_target_snapshot);
    assert_eq!(
        persisted.head_commit.as_deref(),
        Some("head_commit_replacement")
    );
    assert_eq!(
        persisted.pushed_remote.as_deref(),
        Some("origin/replacement")
    );
}

#[test]
fn status_machine_allows_blocked_to_running_and_manual_recovery_only_to_abort() {
    let (_tmp, store, attempt) = setup();
    let running = store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("start attempt");
    let blocked = store
        .update_attempt_status(
            &running.project_id,
            &running.issue_id,
            &running.id,
            CodingAttemptStatus::Blocked,
        )
        .expect("block attempt");
    let running = store
        .update_attempt_status(
            &blocked.project_id,
            &blocked.issue_id,
            &blocked.id,
            CodingAttemptStatus::Running,
        )
        .expect("reopen blocked attempt until Task 6/7 controlled transition is available");
    assert_eq!(running.status, CodingAttemptStatus::Running);

    store
        .write_coding_attempt_for_test(&CodingExecutionAttempt {
            status: CodingAttemptStatus::AwaitingManualRecovery,
            version: 0,
            manual_recovery_reason: None,
            ..running
        })
        .expect("seed manual-recovery attempt");

    for status in [
        CodingAttemptStatus::Created,
        CodingAttemptStatus::Running,
        CodingAttemptStatus::WaitingForHuman,
        CodingAttemptStatus::Blocked,
        CodingAttemptStatus::AwaitingPlanAmendment,
        CodingAttemptStatus::ApplyingPlanAmendment,
        CodingAttemptStatus::AmendmentApplyFailed,
        CodingAttemptStatus::Completed,
        CodingAttemptStatus::Failed,
    ] {
        let rejected = store.update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            status.clone(),
        );
        assert!(matches!(
            rejected,
            Err(ProductStoreError::Io(message))
                if message == format!(
                    "invalid_coding_attempt_status_transition: AwaitingManualRecovery -> {status:?}"
                )
        ));
    }

    let aborted = store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Aborted,
        )
        .expect("abort manual-recovery attempt");
    assert_eq!(aborted.status, CodingAttemptStatus::Aborted);
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
    assert_eq!(attempt.version, 0);
    assert!(attempt.manual_recovery_reason.is_none());
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
            target_snapshot: None,
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
        .update_attempt_non_status_fields(&group_attempt)
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
            target_snapshot: None,
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
            target_snapshot: None,
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
            target_snapshot: None,
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
            target_snapshot: None,
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
            target_snapshot: None,
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
            target_snapshot: None,
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
            target_snapshot: None,
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
            target_snapshot: None,
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

// Group final readiness persistence regression coverage.

fn final_readiness_finding() -> ReviewFinding {
    ReviewFinding {
        severity: FindingSeverity::Warning,
        file_path: Some("src/product/coding_attempt_store/group_final_readiness.rs".to_string()),
        line: Some(42),
        message: "review evidence is preserved".to_string(),
        required_action: Some("confirm the finding".to_string()),
        source_stage: CodingExecutionStage::CodeReview,
        evidence: vec!["test-output/review-evidence.txt".to_string()],
        plan_defect_evidence: Vec::new(),
        related_requirements: Vec::new(),
        related_design_constraints: Vec::new(),
        related_work_item_tasks: vec!["work_item_0001".to_string()],
        defect_class: PlanDefectClass::ImplementationDefect,
        reason_code: Some("review_evidence".to_string()),
        contract_refs: Vec::new(),
        capability_refs: Vec::new(),
        repair_target: None,
        recommended_route: PlanDefectRoute::CoderRework,
        confidence: None,
    }
}

fn ready_unit(
    unit_id: &str,
    logical_work_item_id: &str,
    unit_run_id: &str,
) -> GroupFinalReadinessUnit {
    GroupFinalReadinessUnit {
        unit_id: unit_id.to_string(),
        logical_work_item_id: logical_work_item_id.to_string(),
        unit_run_id: Some(unit_run_id.to_string()),
        start_commit: Some(format!("start_commit_{unit_id}")),
        completion_commit: Some(format!("completion_commit_{unit_id}")),
        commit_shas: vec![
            format!("start_commit_{unit_id}"),
            format!("completion_commit_{unit_id}"),
        ],
        diff_ref: format!("diffs/{unit_id}.patch"),
        empty_observation: false,
        code_review_report_id: Some(format!("code_review_report_{unit_id}")),
        review_verdict: Some(ReviewVerdict::Approve),
        review_summary: Some(format!("{unit_id} review approved")),
        review_findings: Some(vec![final_readiness_finding()]),
        review_raw_provider_output_ref: Some(format!("provider-raw/code-review/{unit_id}.txt")),
        handoff_revision_id: Some(format!("handoff_revision_{unit_id}")),
        plan_revision_id: Some(format!("plan_revision_{unit_id}")),
    }
}

#[test]
fn group_final_readiness_round_trips_complete_snapshot() {
    let (_tmp, store, attempt) = setup();
    let snapshot = GroupFinalReadinessSnapshot {
        attempt_id: attempt.id.clone(),
        status: GroupFinalReadinessStatus::Complete,
        units: vec![
            ready_unit("unit_0001", "work_item_0001", "unit_run_0001"),
            ready_unit("unit_0002", "work_item_0002", "unit_run_0002"),
        ],
        diagnostics: Vec::new(),
        created_at: "caller-supplied-value-must-not-be-persisted".to_string(),
    };

    store
        .write_group_final_readiness_snapshot(&attempt, &snapshot)
        .expect("write readiness snapshot");

    let restored = store
        .get_group_final_readiness_snapshot(&attempt)
        .expect("read readiness snapshot")
        .expect("snapshot exists");

    assert_eq!(restored.status, GroupFinalReadinessStatus::Complete);
    assert_eq!(restored.units, snapshot.units);
    assert_eq!(
        restored.units[0].commit_shas,
        vec!["start_commit_unit_0001", "completion_commit_unit_0001"]
    );
    assert_eq!(
        restored.units[0].unit_run_id.as_deref(),
        Some("unit_run_0001")
    );
    assert_eq!(
        restored.units[0].start_commit.as_deref(),
        Some("start_commit_unit_0001")
    );
    assert_eq!(
        restored.units[0].completion_commit.as_deref(),
        Some("completion_commit_unit_0001")
    );
    assert_eq!(
        restored.units[0].review_findings,
        Some(vec![final_readiness_finding()])
    );
    assert_eq!(
        restored.units[0].review_raw_provider_output_ref.as_deref(),
        Some("provider-raw/code-review/unit_0001.txt")
    );
    assert!(
        serde_json::to_value(&restored)
            .expect("serialize restored snapshot")
            .get("units")
            .and_then(|units| units.get(0))
            .and_then(|unit| unit.get("review_raw_ref"))
            .is_none()
    );
    assert!(
        serde_json::to_value(&restored)
            .expect("serialize restored snapshot")
            .get("units")
            .and_then(|units| units.get(0))
            .and_then(|unit| unit.get("review_raw_provider_output_ref"))
            .is_some()
    );
    let created_at = chrono::DateTime::parse_from_rfc3339(&restored.created_at)
        .expect("writer stamps snapshot with RFC3339 time");
    assert_eq!(created_at.offset().local_minus_utc(), 0);
    assert_ne!(restored.created_at, snapshot.created_at);
    assert_eq!(
        restored.units[1].handoff_revision_id.as_deref(),
        Some("handoff_revision_unit_0002")
    );
    assert_eq!(
        restored.units[1].plan_revision_id.as_deref(),
        Some("plan_revision_unit_0002")
    );
    assert!(!restored.units[1].empty_observation);
}

#[test]
fn group_final_readiness_empty_observation_requires_empty_git_facts() {
    let (_tmp, store, attempt) = setup();
    let mut unit = ready_unit("unit_0001", "work_item_0001", "unit_run_0001");
    unit.empty_observation = true;
    unit.commit_shas.clear();
    unit.diff_ref.clear();
    let snapshot = GroupFinalReadinessSnapshot {
        attempt_id: attempt.id.clone(),
        status: GroupFinalReadinessStatus::Complete,
        units: vec![unit.clone()],
        diagnostics: Vec::new(),
        created_at: String::new(),
    };

    store
        .write_group_final_readiness_snapshot(&attempt, &snapshot)
        .expect("empty observation snapshot");
    assert!(
        store
            .get_group_final_readiness_snapshot(&attempt)
            .expect("read snapshot")
            .expect("snapshot")
            .units[0]
            .empty_observation
    );

    unit.commit_shas = vec!["completion_commit_unit_0001".to_string()];
    let inconsistent = GroupFinalReadinessSnapshot {
        units: vec![unit],
        ..snapshot
    };
    assert_invalid_group_final_readiness_snapshot(
        store
            .write_group_final_readiness_snapshot(&attempt, &inconsistent)
            .expect_err("empty observation must not include git facts"),
        "empty observation unit unit_0001 must not include git range facts",
    );
}

#[test]
fn group_final_readiness_rejects_snapshot_for_different_attempt() {
    let (_tmp, store, attempt) = setup();
    let mut snapshot = GroupFinalReadinessSnapshot {
        attempt_id: "coding_attempt_other".to_string(),
        status: GroupFinalReadinessStatus::Complete,
        units: vec![ready_unit("unit_0001", "work_item_0001", "unit_run_0001")],
        diagnostics: Vec::new(),
        created_at: String::new(),
    };

    assert!(matches!(
        store.write_group_final_readiness_snapshot(&attempt, &snapshot),
        Err(ProductStoreError::IdentityMismatch {
            kind: "group_final_readiness_snapshot",
            ..
        })
    ));

    snapshot.attempt_id = attempt.id.clone();
    store
        .write_group_final_readiness_snapshot(&attempt, &snapshot)
        .expect("write valid snapshot");
    snapshot.attempt_id = "../other-attempt".to_string();
    write_json(
        &store.group_final_readiness_snapshot_path(&attempt),
        &snapshot,
    )
    .expect("forge mismatched persisted snapshot");

    assert!(matches!(
        store.get_group_final_readiness_snapshot(&attempt),
        Err(ProductStoreError::IdentityMismatch {
            kind: "group_final_readiness_snapshot",
            ..
        })
    ));
}

#[test]
fn group_final_readiness_incomplete_snapshot_keeps_explicit_diagnostics() {
    let (_tmp, store, attempt) = setup();
    let mut unit = ready_unit("unit_0001", "work_item_0001", "unit_run_0001");
    unit.code_review_report_id = None;
    unit.review_verdict = None;
    unit.review_summary = None;
    unit.review_findings = None;
    unit.review_raw_provider_output_ref = None;
    let snapshot = GroupFinalReadinessSnapshot {
        attempt_id: attempt.id.clone(),
        status: GroupFinalReadinessStatus::Incomplete,
        units: vec![unit],
        diagnostics: vec![GroupFinalReadinessDiagnostic {
            kind: GroupFinalReadinessDiagnosticKind::CodeReviewMissing,
            unit_id: Some("unit_0001".to_string()),
            message: "unit_0001 is missing code-review evidence".to_string(),
        }],
        created_at: String::new(),
    };

    store
        .write_group_final_readiness_snapshot(&attempt, &snapshot)
        .expect("write incomplete snapshot");
    let restored = store
        .get_group_final_readiness_snapshot(&attempt)
        .expect("read incomplete snapshot")
        .expect("snapshot exists");

    assert_eq!(restored.status, GroupFinalReadinessStatus::Incomplete);
    assert_eq!(restored.diagnostics, snapshot.diagnostics);
    assert!(restored.units[0].review_verdict.is_none());
}

#[test]
fn group_final_readiness_returns_none_when_snapshot_does_not_exist() {
    let (_tmp, store, attempt) = setup();

    assert_eq!(
        store
            .get_group_final_readiness_snapshot(&attempt)
            .expect("read missing snapshot"),
        None
    );
}

#[test]
fn group_final_readiness_rejects_complete_snapshot_without_authoritative_evidence() {
    let (_tmp, store, attempt) = setup();
    let missing_evidence: [MissingEvidenceCase; 6] = [
        (
            "code_review_report_id",
            |unit: &mut GroupFinalReadinessUnit| unit.code_review_report_id = None,
        ),
        ("review_verdict", |unit: &mut GroupFinalReadinessUnit| {
            unit.review_verdict = None
        }),
        ("review_summary", |unit: &mut GroupFinalReadinessUnit| {
            unit.review_summary = None
        }),
        ("review_findings", |unit: &mut GroupFinalReadinessUnit| {
            unit.review_findings = None
        }),
        (
            "handoff_revision_id",
            |unit: &mut GroupFinalReadinessUnit| unit.handoff_revision_id = None,
        ),
        ("plan_revision_id", |unit: &mut GroupFinalReadinessUnit| {
            unit.plan_revision_id = None
        }),
    ];

    let empty_units = GroupFinalReadinessSnapshot {
        attempt_id: attempt.id.clone(),
        status: GroupFinalReadinessStatus::Complete,
        units: Vec::new(),
        diagnostics: Vec::new(),
        created_at: String::new(),
    };
    assert_invalid_group_final_readiness_snapshot(
        store
            .write_group_final_readiness_snapshot(&attempt, &empty_units)
            .expect_err("complete snapshot without units must be rejected"),
        "complete snapshot must include at least one unit",
    );

    for (field, clear_field) in missing_evidence {
        let mut unit = ready_unit("unit_0001", "work_item_0001", "unit_run_0001");
        clear_field(&mut unit);
        let snapshot = GroupFinalReadinessSnapshot {
            attempt_id: attempt.id.clone(),
            status: GroupFinalReadinessStatus::Complete,
            units: vec![unit],
            diagnostics: Vec::new(),
            created_at: String::new(),
        };

        assert_invalid_group_final_readiness_snapshot(
            store
                .write_group_final_readiness_snapshot(&attempt, &snapshot)
                .expect_err("complete snapshot without authoritative evidence must be rejected"),
            &format!("complete unit unit_0001 is missing {field}"),
        );
    }
}

#[test]
fn group_final_readiness_rejects_empty_or_blank_diagnostics_as_invalid_records() {
    let (_tmp, store, attempt) = setup();
    let empty_diagnostics = GroupFinalReadinessSnapshot {
        attempt_id: attempt.id.clone(),
        status: GroupFinalReadinessStatus::Incomplete,
        units: Vec::new(),
        diagnostics: Vec::new(),
        created_at: String::new(),
    };
    assert_invalid_group_final_readiness_snapshot(
        store
            .write_group_final_readiness_snapshot(&attempt, &empty_diagnostics)
            .expect_err("empty incomplete diagnostics must be rejected"),
        "incomplete snapshot must include diagnostics",
    );

    let blank_message = GroupFinalReadinessSnapshot {
        attempt_id: attempt.id.clone(),
        status: GroupFinalReadinessStatus::Incomplete,
        units: Vec::new(),
        diagnostics: vec![GroupFinalReadinessDiagnostic {
            kind: GroupFinalReadinessDiagnosticKind::CodeReviewMissing,
            unit_id: None,
            message: " \t ".to_string(),
        }],
        created_at: String::new(),
    };
    assert_invalid_group_final_readiness_snapshot(
        store
            .write_group_final_readiness_snapshot(&attempt, &blank_message)
            .expect_err("blank diagnostic messages must be rejected"),
        "diagnostic message must not be empty",
    );
}

#[test]
fn group_final_readiness_rejects_escaping_repair_target_ids() {
    let (_tmp, store, attempt) = setup();
    for repair_target in [
        RepairTarget {
            kind: RepairTargetKind::CurrentWorkItem,
            logical_work_item_ids: vec!["../work-item".to_string()],
            work_item_revision_ids: vec!["work_item_revision_0001".to_string()],
        },
        RepairTarget {
            kind: RepairTargetKind::CurrentWorkItem,
            logical_work_item_ids: vec!["work_item_0001".to_string()],
            work_item_revision_ids: vec!["../work-item-revision".to_string()],
        },
    ] {
        let mut unit = ready_unit("unit_0001", "work_item_0001", "unit_run_0001");
        let mut finding = final_readiness_finding();
        finding.repair_target = Some(repair_target);
        unit.review_findings = Some(vec![finding]);
        let snapshot = GroupFinalReadinessSnapshot {
            attempt_id: attempt.id.clone(),
            status: GroupFinalReadinessStatus::Complete,
            units: vec![unit],
            diagnostics: Vec::new(),
            created_at: String::new(),
        };

        assert!(matches!(
            store.write_group_final_readiness_snapshot(&attempt, &snapshot),
            Err(ProductStoreError::PathEscape(_))
        ));
    }
}

#[test]
fn group_final_readiness_rejects_escaping_artifact_references() {
    let (_tmp, store, attempt) = setup();
    let mut unit = ready_unit("unit_0001", "work_item_0001", "unit_run_0001");
    unit.review_raw_provider_output_ref = Some("../outside.txt".to_string());
    let snapshot = GroupFinalReadinessSnapshot {
        attempt_id: attempt.id.clone(),
        status: GroupFinalReadinessStatus::Complete,
        units: vec![unit],
        diagnostics: Vec::new(),
        created_at: String::new(),
    };

    assert!(matches!(
        store.write_group_final_readiness_snapshot(&attempt, &snapshot),
        Err(ProductStoreError::PathEscape(value)) if value == "../outside.txt"
    ));
}

#[test]
fn group_final_readiness_model_defaults_missing_future_fields() {
    let snapshot: GroupFinalReadinessSnapshot = read_json_from_str(
        r#"{
            "attempt_id": "coding_attempt_0001",
            "status": "incomplete",
            "units": [{
                "unit_id": "unit_0001",
                "logical_work_item_id": "work_item_0001",
                "unit_run_id": "unit_run_0001",
                "start_commit": "start_commit_0001",
                "completion_commit": "completion_commit_0001"
            }]
        }"#,
    );

    assert!(snapshot.diagnostics.is_empty());
    assert!(snapshot.created_at.is_empty());
    assert_eq!(
        snapshot.units[0].unit_run_id.as_deref(),
        Some("unit_run_0001")
    );
    assert_eq!(
        snapshot.units[0].start_commit.as_deref(),
        Some("start_commit_0001")
    );
    assert_eq!(
        snapshot.units[0].completion_commit.as_deref(),
        Some("completion_commit_0001")
    );
    assert!(!snapshot.units[0].empty_observation);
    assert_eq!(snapshot.units[0].commit_shas, Vec::<String>::new());
    assert!(snapshot.units[0].review_verdict.is_none());
}

fn assert_invalid_group_final_readiness_snapshot(error: ProductStoreError, expected_reason: &str) {
    assert!(matches!(
        error,
        ProductStoreError::InvalidRecord {
            kind: "group_final_readiness_snapshot",
            reason,
        } if reason == expected_reason
    ));
}

fn read_json_from_str(value: &str) -> GroupFinalReadinessSnapshot {
    serde_json::from_str(value).expect("deserialize readiness snapshot")
}
