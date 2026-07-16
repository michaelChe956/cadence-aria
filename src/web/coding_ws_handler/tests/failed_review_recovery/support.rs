use std::fs;
use std::process::Command;

use tokio::sync::mpsc;

use crate::product::coding_attempt_store::{
    CodingAttemptStore, CreateBlockedGateInput, CreateCodingExecutionUnitInput,
    FailedCodeReviewRecoveryJournal,
};
use crate::product::coding_models::{
    CodingAgentRole, CodingAttemptScope, CodingAttemptStatus, CodingExecutionAttempt,
    CodingExecutionStage, CodingExecutionUnitStatus, CodingGateAction, CodingGateActionType,
    CodingGateRequired, CodingProviderRole, CodingRoleRunStatus, CodingRoleRunTrigger,
    CodingTimelineNode, CodingTimelineNodeStatus,
};
use crate::product::coding_workspace_engine::CodingWorkspaceEngine;
use crate::product::git_workspace_service::GitWorkspaceService;

use super::super::{
    CodingWsOutMessage, build_coding_session_state, seed_compiled_work_item_fixture,
};

pub(super) const FAILED_NODE_ID: &str = "coding_node_0009";

#[derive(Debug, Clone, Copy)]
pub(super) enum FixtureCase {
    Recoverable,
    BlockedProviderInterrupted,
    CompletedAttempt,
    AbortedAttempt,
    TestingStage,
    MissingCompletedAt,
    GroupWithoutActiveUnit,
    GroupActiveUnitIdMismatch,
    GroupUnitNotRunning,
    WorkItemWithActiveUnitId,
    LatestReviewCompleted,
    RoleRunNodeMismatch,
    RoleRunNotRunning,
    MissingDirtyGate,
    MissingWorktreePath,
    MissingWorktree,
}

pub(super) struct FailedReviewFixture {
    pub(super) _tmp: tempfile::TempDir,
    pub(super) store: CodingAttemptStore,
    pub(super) attempt: CodingExecutionAttempt,
    pub(super) dirty_gate: Option<CodingGateRequired>,
    pub(super) stale_role_run_id: String,
}

pub(super) struct RepeatedInterruptedReview {
    pub(super) blocked_attempt: CodingExecutionAttempt,
    pub(super) first_journal: FailedCodeReviewRecoveryJournal,
    pub(super) first_retry_role_run_id: String,
    pub(super) second_gate: CodingGateRequired,
}

pub(super) fn attempt_data_fingerprint(attempt: &CodingExecutionAttempt) -> serde_json::Value {
    serde_json::json!({
        "id": attempt.id,
        "project_id": attempt.project_id,
        "issue_id": attempt.issue_id,
        "work_item_id": attempt.work_item_id,
        "attempt_no": attempt.attempt_no,
        "scope": attempt.scope,
        "base_branch": attempt.base_branch,
        "branch_name": attempt.branch_name,
        "worktree_path": attempt.worktree_path,
        "provider_config_snapshot": attempt.provider_config_snapshot,
        "rework_count": attempt.rework_count,
        "max_auto_rework": attempt.max_auto_rework,
        "work_item_group_id": attempt.work_item_group_id,
        "current_work_item_id": attempt.current_work_item_id,
        "active_unit_id": attempt.active_unit_id,
        "head_commit": attempt.head_commit,
        "pushed_remote": attempt.pushed_remote,
        "review_request_id": attempt.review_request_id,
        "provider_conversations": attempt.provider_conversations,
        "created_at": attempt.created_at,
    })
}

pub(super) fn git_status_porcelain(worktree_path: &std::path::Path) -> String {
    let output = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(worktree_path)
        .output()
        .expect("git status --porcelain");
    assert!(output.status.success(), "git status failed: {output:?}");
    String::from_utf8(output.stdout).expect("git status utf8")
}

pub(super) fn assert_recovery_gate(scope: CodingAttemptScope) {
    let fixture = failed_review_fixture(scope, FixtureCase::Recoverable);
    let dirty_gate = fixture.dirty_gate.clone().expect("dirty gate");
    let attempt_before = fixture
        .store
        .get_attempt(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .expect("attempt before SessionState");
    let nodes_before = fixture
        .store
        .get_timeline_nodes(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .expect("nodes before SessionState");
    let runs_before = fixture
        .store
        .list_role_runs(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .expect("runs before SessionState");
    let gates_before = fixture
        .store
        .list_open_blocked_gates(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .expect("gates before SessionState");
    let units_before = fixture
        .store
        .list_coding_units(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .expect("units before SessionState");

    let state = build_coding_session_state(&fixture.store, fixture.attempt.clone())
        .expect("coding session state");
    let CodingWsOutMessage::CodingSessionState { pending_gates, .. } = state else {
        panic!("expected coding session state");
    };
    let gate = pending_gates
        .iter()
        .find(|gate| gate.reason_code.as_deref() == Some("failed_code_review_recoverable"))
        .expect("failed code review recovery gate");

    assert_eq!(gate.gate_id, dirty_gate.gate_id);
    assert_eq!(gate.title, "代码审查中断");
    assert_eq!(gate.stage, Some(CodingExecutionStage::CodeReview));
    assert_eq!(gate.role, Some(CodingProviderRole::CodeReviewer));
    assert_eq!(gate.available_actions.len(), 1);
    assert_eq!(gate.available_actions[0].action_id, "retry_review");
    assert_eq!(gate.available_actions[0].label, "重试代码审查");
    assert_eq!(
        gate.available_actions[0].action_type,
        CodingGateActionType::RetryReview
    );
    assert_eq!(
        gate.evidence_refs,
        vec![FAILED_NODE_ID.to_string(), fixture.stale_role_run_id]
    );

    assert_eq!(
        fixture
            .store
            .get_attempt(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id,
            )
            .expect("attempt after SessionState"),
        attempt_before
    );
    assert_eq!(
        fixture
            .store
            .get_timeline_nodes(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id,
            )
            .expect("nodes after SessionState"),
        nodes_before
    );
    assert_eq!(
        fixture
            .store
            .list_role_runs(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id,
            )
            .expect("runs after SessionState"),
        runs_before
    );
    assert_eq!(
        fixture
            .store
            .list_open_blocked_gates(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id,
            )
            .expect("gates after SessionState"),
        gates_before
    );
    assert_eq!(
        fixture
            .store
            .list_coding_units(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id,
            )
            .expect("units after SessionState"),
        units_before
    );
}

pub(super) async fn seed_repeated_interrupted_review(
    fixture: &FailedReviewFixture,
) -> RepeatedInterruptedReview {
    let first_gate_id = fixture
        .dirty_gate
        .as_ref()
        .expect("first provider interrupted gate")
        .gate_id
        .clone();
    let (event_tx, _event_rx) = mpsc::channel(8);
    let engine =
        CodingWorkspaceEngine::new(fixture.store.clone(), GitWorkspaceService::new(), event_tx);
    let first_running = engine
        .recover_failed_code_review_for_attempt(&fixture.attempt, &first_gate_id)
        .await
        .expect("first interrupted review recovery");
    let first_journal = fixture
        .store
        .complete_failed_code_review_recovery_journal(&first_running, &first_gate_id)
        .expect("complete first recovery journal");
    let first_retry_role_run_id = first_journal
        .retry_role_run_id
        .clone()
        .expect("first retry reviewer run");
    let second_failed_node_id = "coding_node_0010";
    fixture
        .store
        .save_timeline_node(CodingTimelineNode {
            id: second_failed_node_id.to_string(),
            attempt_id: first_running.id.clone(),
            stage: CodingExecutionStage::CodeReview,
            title: "代码审查".to_string(),
            status: CodingTimelineNodeStatus::Failed,
            agent_role: Some(CodingAgentRole::Reviewer),
            summary: Some("second review provider interrupted".to_string()),
            started_at: "2026-07-12T04:40:00Z".to_string(),
            completed_at: Some("2026-07-12T04:40:59Z".to_string()),
            artifact_refs: Vec::new(),
        })
        .expect("second failed review node");
    fixture
        .store
        .attach_role_run_node(
            &first_running.project_id,
            &first_running.issue_id,
            &first_running.id,
            &first_retry_role_run_id,
            second_failed_node_id.to_string(),
        )
        .expect("bind first retry reviewer node");
    fixture
        .store
        .update_role_run_status(
            &first_running.project_id,
            &first_running.issue_id,
            &first_running.id,
            &first_retry_role_run_id,
            CodingRoleRunStatus::Failed,
            Some("code_review_provider_interrupted".to_string()),
        )
        .expect("fail first retry reviewer");
    let blocked_attempt = fixture
        .store
        .update_attempt_status(
            &first_running.project_id,
            &first_running.issue_id,
            &first_running.id,
            CodingAttemptStatus::Blocked,
        )
        .expect("block attempt for second interruption");
    let second_gate = fixture
        .store
        .create_blocked_gate(CreateBlockedGateInput {
            attempt_id: blocked_attempt.id.clone(),
            stage: CodingExecutionStage::CodeReview,
            node_id: Some(second_failed_node_id.to_string()),
            role: Some(CodingProviderRole::CodeReviewer),
            title: "代码审查中断".to_string(),
            description: "second review provider interrupted".to_string(),
            reason_code: Some("code_review_provider_interrupted".to_string()),
            evidence_refs: Vec::new(),
            raw_provider_output_ref: None,
            available_actions: vec![
                CodingGateAction {
                    action_id: "retry_review".to_string(),
                    label: "重试代码审查".to_string(),
                    action_type: CodingGateActionType::RetryReview,
                },
                CodingGateAction {
                    action_id: "send_to_coder".to_string(),
                    label: "发送给 Coder".to_string(),
                    action_type: CodingGateActionType::SendToCoder,
                },
                CodingGateAction {
                    action_id: "abort".to_string(),
                    label: "终止".to_string(),
                    action_type: CodingGateActionType::Abort,
                },
            ],
        })
        .expect("second provider interrupted gate");

    RepeatedInterruptedReview {
        blocked_attempt,
        first_journal,
        first_retry_role_run_id,
        second_gate,
    }
}

pub(super) fn failed_review_fixture(
    scope: CodingAttemptScope,
    case: FixtureCase,
) -> FailedReviewFixture {
    let (tmp, app_paths, mut attempt) = seed_compiled_work_item_fixture();
    let store = CodingAttemptStore::new(app_paths);
    let worktree_path = attempt.worktree_path.clone().expect("worktree path");
    if !matches!(case, FixtureCase::MissingWorktree) {
        fs::create_dir_all(&worktree_path).expect("shared worktree directory");
        let output = Command::new("git")
            .arg("init")
            .arg("--quiet")
            .current_dir(&worktree_path)
            .output()
            .expect("git init fixture worktree");
        assert!(output.status.success(), "git init failed: {output:?}");
        fs::write(worktree_path.join("dirty-review.txt"), "preserve me\n")
            .expect("dirty worktree fixture");
    }

    attempt.scope = scope.clone();
    attempt.status = match case {
        FixtureCase::BlockedProviderInterrupted => CodingAttemptStatus::Blocked,
        FixtureCase::CompletedAttempt => CodingAttemptStatus::Completed,
        FixtureCase::AbortedAttempt => CodingAttemptStatus::Aborted,
        _ => CodingAttemptStatus::Failed,
    };
    attempt.stage = if matches!(case, FixtureCase::TestingStage) {
        CodingExecutionStage::Testing
    } else {
        CodingExecutionStage::CodeReview
    };
    attempt.completed_at = (!matches!(
        case,
        FixtureCase::MissingCompletedAt | FixtureCase::BlockedProviderInterrupted
    ))
    .then(|| "2026-07-12T04:30:59Z".to_string());
    attempt.work_item_group_id = matches!(scope, CodingAttemptScope::WorkItemGroup)
        .then(|| "work_item_plan_0001".to_string());
    attempt.active_unit_id = None;
    store
        .save_coding_attempt(&attempt)
        .expect("save historical failed attempt");

    if matches!(scope, CodingAttemptScope::WorkItemGroup)
        && !matches!(case, FixtureCase::GroupWithoutActiveUnit)
    {
        let unit_status = if matches!(case, FixtureCase::GroupUnitNotRunning) {
            CodingExecutionUnitStatus::WaitingForHuman
        } else {
            CodingExecutionUnitStatus::Running
        };
        store
            .create_coding_unit(CreateCodingExecutionUnitInput {
                attempt_id: attempt.id.clone(),
                project_id: attempt.project_id.clone(),
                issue_id: attempt.issue_id.clone(),
                plan_id: "work_item_plan_0001".to_string(),
                work_item_id: attempt.work_item_id.clone(),
                order_index: 0,
                status: unit_status,
            })
            .expect("historical active coding unit");
        attempt = store
            .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("attempt with active unit");
    }
    if matches!(scope, CodingAttemptScope::WorkItem)
        && matches!(case, FixtureCase::WorkItemWithActiveUnitId)
    {
        attempt.active_unit_id = Some("coding_unit_0001".to_string());
        store
            .save_coding_attempt(&attempt)
            .expect("save invalid work item active unit id");
    }
    if matches!(scope, CodingAttemptScope::WorkItemGroup)
        && matches!(case, FixtureCase::GroupActiveUnitIdMismatch)
    {
        attempt.active_unit_id = Some("coding_unit_9999".to_string());
        store
            .save_coding_attempt(&attempt)
            .expect("save mismatched group active unit id");
    }
    if matches!(case, FixtureCase::MissingWorktreePath) {
        attempt.worktree_path = None;
        store
            .save_coding_attempt(&attempt)
            .expect("save missing worktree path");
    }

    store
        .save_timeline_node(CodingTimelineNode {
            id: "coding_node_0008".to_string(),
            attempt_id: attempt.id.clone(),
            stage: CodingExecutionStage::CodeReview,
            title: "代码审查".to_string(),
            status: CodingTimelineNodeStatus::Completed,
            agent_role: Some(CodingAgentRole::Reviewer),
            summary: Some("older review".to_string()),
            started_at: "2026-07-12T04:20:00Z".to_string(),
            completed_at: Some("2026-07-12T04:21:00Z".to_string()),
            artifact_refs: Vec::new(),
        })
        .expect("older review node");
    store
        .save_timeline_node(CodingTimelineNode {
            id: FAILED_NODE_ID.to_string(),
            attempt_id: attempt.id.clone(),
            stage: CodingExecutionStage::CodeReview,
            title: "代码审查".to_string(),
            status: CodingTimelineNodeStatus::Failed,
            agent_role: Some(CodingAgentRole::Reviewer),
            summary: Some("review interrupted".to_string()),
            started_at: "2026-07-12T04:30:00Z".to_string(),
            completed_at: Some("2026-07-12T04:30:59Z".to_string()),
            artifact_refs: Vec::new(),
        })
        .expect("failed review node");
    if matches!(case, FixtureCase::LatestReviewCompleted) {
        store
            .save_timeline_node(CodingTimelineNode {
                id: "coding_node_0010".to_string(),
                attempt_id: attempt.id.clone(),
                stage: CodingExecutionStage::CodeReview,
                title: "代码审查".to_string(),
                status: CodingTimelineNodeStatus::Completed,
                agent_role: Some(CodingAgentRole::Reviewer),
                summary: Some("newer review completed".to_string()),
                started_at: "2026-07-12T04:31:00Z".to_string(),
                completed_at: Some("2026-07-12T04:32:00Z".to_string()),
                artifact_refs: Vec::new(),
            })
            .expect("newer completed review node");
    }

    let role_node_id = if matches!(case, FixtureCase::RoleRunNodeMismatch) {
        "coding_node_0008"
    } else {
        FAILED_NODE_ID
    };
    let role_run = store
        .create_role_run(
            &attempt,
            CodingExecutionStage::CodeReview,
            CodingProviderRole::CodeReviewer,
            CodingRoleRunTrigger::Initial,
            Some(role_node_id.to_string()),
        )
        .expect("stale reviewer role run");
    if matches!(
        case,
        FixtureCase::RoleRunNotRunning | FixtureCase::BlockedProviderInterrupted
    ) {
        store
            .update_role_run_status(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &role_run.id,
                CodingRoleRunStatus::Failed,
                Some(
                    if matches!(case, FixtureCase::BlockedProviderInterrupted) {
                        "code_review_provider_interrupted"
                    } else {
                        "provider_failed"
                    }
                    .to_string(),
                ),
            )
            .expect("complete stale role run");
    }

    let dirty_gate = (!matches!(case, FixtureCase::MissingDirtyGate)).then(|| {
        let provider_interrupted = matches!(case, FixtureCase::BlockedProviderInterrupted);
        store
            .create_blocked_gate(CreateBlockedGateInput {
                attempt_id: attempt.id.clone(),
                stage: if provider_interrupted {
                    CodingExecutionStage::CodeReview
                } else {
                    CodingExecutionStage::FinalConfirm
                },
                node_id: provider_interrupted.then(|| FAILED_NODE_ID.to_string()),
                role: provider_interrupted.then_some(CodingProviderRole::CodeReviewer),
                title: if provider_interrupted {
                    "代码审查中断".to_string()
                } else {
                    "Shared worktree has uncommitted changes".to_string()
                },
                description: if provider_interrupted {
                    "review provider interrupted".to_string()
                } else {
                    "Issue shared worktree has uncommitted changes".to_string()
                },
                reason_code: Some(
                    if provider_interrupted {
                        "code_review_provider_interrupted"
                    } else {
                        "shared_worktree_dirty_manual_gate"
                    }
                    .to_string(),
                ),
                evidence_refs: Vec::new(),
                raw_provider_output_ref: None,
                available_actions: if provider_interrupted {
                    vec![
                        CodingGateAction {
                            action_id: "retry_review".to_string(),
                            label: "重试代码审查".to_string(),
                            action_type: CodingGateActionType::RetryReview,
                        },
                        CodingGateAction {
                            action_id: "send_to_coder".to_string(),
                            label: "发送给 Coder".to_string(),
                            action_type: CodingGateActionType::SendToCoder,
                        },
                        CodingGateAction {
                            action_id: "abort".to_string(),
                            label: "终止".to_string(),
                            action_type: CodingGateActionType::Abort,
                        },
                    ]
                } else {
                    vec![CodingGateAction {
                        action_id: "manual_continue".to_string(),
                        label: "人工继续".to_string(),
                        action_type: CodingGateActionType::ManualContinue,
                    }]
                },
            })
            .expect("review recovery gate")
    });

    FailedReviewFixture {
        _tmp: tmp,
        store,
        attempt,
        dirty_gate,
        stale_role_run_id: role_run.id,
    }
}
