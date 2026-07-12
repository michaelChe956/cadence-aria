use super::*;
use crate::product::lifecycle_store::UpsertIssueSharedWorktreeInput;

const PROJECT_ID: &str = "project_0001";
const ISSUE_ID: &str = "issue_0001";
const WORK_ITEM_ID: &str = "work_item_0001";
const NODE_ID: &str = "coding_node_0009";

#[tokio::test]
async fn code_review_provider_failure_blocks_attempt_without_cleaning_shared_worktree() {
    let root = tempdir().expect("tempdir");
    let shared_worktree = root.path().join("shared-worktree");
    fs::create_dir_all(&shared_worktree).expect("shared worktree dir");
    init_test_git_repo(&shared_worktree);
    fs::write(shared_worktree.join("dirty.txt"), "uncommitted\n").expect("dirty worktree");

    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(paths.clone());
    lifecycle
        .upsert_issue_shared_worktree(UpsertIssueSharedWorktreeInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            repository_id: "repository_0001".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: shared_worktree.clone(),
            base_branch: "HEAD".to_string(),
        })
        .expect("shared worktree");
    lifecycle
        .try_acquire_issue_worktree_lock(PROJECT_ID, ISSUE_ID, WORK_ITEM_ID)
        .expect("shared worktree lock");

    let store = CodingAttemptStore::new(paths);
    let attempt = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_work_item_id: WORK_ITEM_ID.to_string(),
            base_branch: "HEAD".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: Some(shared_worktree),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
            },
            max_auto_rework: 2,
        })
        .expect("group attempt");
    let active_unit = store
        .create_coding_unit(CreateCodingExecutionUnitInput {
            attempt_id: attempt.id.clone(),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            work_item_id: WORK_ITEM_ID.to_string(),
            order_index: 0,
            status: CodingExecutionUnitStatus::Running,
        })
        .expect("running active unit");
    let attempt = store
        .update_attempt_status(
            PROJECT_ID,
            ISSUE_ID,
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running attempt");
    let attempt = store
        .update_attempt_stage(
            PROJECT_ID,
            ISSUE_ID,
            &attempt.id,
            CodingExecutionStage::CodeReview,
        )
        .expect("code review stage");
    store
        .save_timeline_node(CodingTimelineNode {
            id: NODE_ID.to_string(),
            attempt_id: attempt.id.clone(),
            stage: CodingExecutionStage::CodeReview,
            title: "代码审查".to_string(),
            status: CodingTimelineNodeStatus::Running,
            agent_role: Some(CodingAgentRole::Reviewer),
            summary: None,
            started_at: "2026-07-12T00:00:00Z".to_string(),
            completed_at: None,
            artifact_refs: Vec::new(),
        })
        .expect("code review timeline node");
    store
        .create_role_run(
            &attempt,
            CodingExecutionStage::CodeReview,
            CodingProviderRole::CodeReviewer,
            CodingRoleRunTrigger::Initial,
            Some(NODE_ID.to_string()),
        )
        .expect("reviewer role run");

    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let message = "Permission request permission_1 timed out".to_string();

    let error = engine
        .fail_provider_stream::<()>(&attempt, NODE_ID, message.clone())
        .await
        .expect_err("provider timeout remains surfaced");

    assert!(matches!(
        error,
        CodingWorkspaceEngineError::ProviderStream(ref persisted_message)
            if persisted_message == &message
    ));
    let persisted = store
        .get_attempt(PROJECT_ID, ISSUE_ID, &attempt.id)
        .expect("persisted attempt");
    assert_eq!(persisted.status, CodingAttemptStatus::Blocked);
    assert_eq!(persisted.stage, CodingExecutionStage::CodeReview);
    assert_eq!(persisted.completed_at, None);

    let timeline_node = store
        .get_timeline_nodes(PROJECT_ID, ISSUE_ID, &attempt.id)
        .expect("timeline nodes")
        .into_iter()
        .find(|node| node.id == NODE_ID)
        .expect("review timeline node");
    assert_eq!(timeline_node.status, CodingTimelineNodeStatus::Failed);
    assert_eq!(timeline_node.summary.as_deref(), Some(message.as_str()));
    assert!(timeline_node.completed_at.is_some());

    let role_run = store
        .latest_role_run(
            PROJECT_ID,
            ISSUE_ID,
            &attempt.id,
            CodingExecutionStage::CodeReview,
            CodingProviderRole::CodeReviewer,
        )
        .expect("latest reviewer role run")
        .expect("reviewer role run");
    assert_eq!(role_run.status, CodingRoleRunStatus::Failed);
    assert_eq!(
        role_run.reason_code.as_deref(),
        Some("code_review_provider_interrupted")
    );

    let open_gates = store
        .list_open_blocked_gates(PROJECT_ID, ISSUE_ID, &attempt.id)
        .expect("open blocked gates");
    let recovery_gate = open_gates
        .iter()
        .find(|gate| gate.reason_code.as_deref() == Some("code_review_provider_interrupted"))
        .expect("code review recovery gate");
    assert_eq!(recovery_gate.title, "代码审查中断");
    assert_eq!(
        recovery_gate
            .available_actions
            .iter()
            .map(|action| action.action_id.as_str())
            .collect::<Vec<_>>(),
        vec!["retry_review", "send_to_coder", "abort"]
    );
    assert_eq!(recovery_gate.available_actions[0].label, "重试代码审查");
    assert!(
        open_gates.iter().all(|gate| {
            gate.reason_code.as_deref() != Some("shared_worktree_dirty_manual_gate")
        })
    );

    let persisted_unit = store
        .get_active_coding_unit(PROJECT_ID, ISSUE_ID, &attempt.id)
        .expect("active coding unit")
        .expect("active coding unit remains");
    assert_eq!(persisted_unit.id, active_unit.id);
    assert_eq!(persisted_unit.status, CodingExecutionUnitStatus::Running);

    let shared = lifecycle
        .get_issue_shared_worktree(PROJECT_ID, ISSUE_ID)
        .expect("load shared worktree")
        .expect("shared worktree exists");
    assert_eq!(
        shared.current_active_work_item_id.as_deref(),
        Some(WORK_ITEM_ID)
    );
}
