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
        .save_timeline_node(
            &attempt,
            CodingTimelineNode {
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
            },
        )
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

    let missing_context_error = engine
        .handle_blocked_gate_response(
            PROJECT_ID,
            ISSUE_ID,
            &attempt.id,
            &recovery_gate.gate_id,
            "send_to_coder",
            Some("   ".to_string()),
        )
        .await
        .expect_err("operator context is required");
    assert!(matches!(
        missing_context_error,
        CodingWorkspaceEngineError::ProviderStream(ref message)
            if message == "coding_gate_extra_context_required"
    ));

    let operator_context = "请 Coder 检查权限请求超时前未完成的改动并继续修复";
    let resumed = engine
        .handle_blocked_gate_response(
            PROJECT_ID,
            ISSUE_ID,
            &attempt.id,
            &recovery_gate.gate_id,
            "send_to_coder",
            Some(operator_context.to_string()),
        )
        .await
        .expect("send interrupted review to coder");
    assert_eq!(resumed.status, CodingAttemptStatus::Running);
    assert_eq!(resumed.stage, CodingExecutionStage::Coding);
    assert_eq!(resumed.rework_count, 1);
    assert!(
        store
            .list_open_blocked_gates(PROJECT_ID, ISSUE_ID, &attempt.id)
            .expect("resolved recovery gate")
            .is_empty()
    );
    let notes = store
        .list_context_notes(PROJECT_ID, ISSUE_ID, &attempt.id)
        .expect("context notes");
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].content, operator_context);
    let instructions = store
        .list_rework_instructions(PROJECT_ID, ISSUE_ID, &attempt.id)
        .expect("rework instructions");
    assert_eq!(instructions.len(), 1);
    assert_eq!(
        instructions[0].source_stage,
        CodingExecutionStage::CodeReview
    );
    assert_eq!(instructions[0].rework_round, 1);
    assert_eq!(instructions[0].summary, operator_context);
}

#[tokio::test]
async fn internal_review_blocked_gate_uses_internal_review_retry_action() {
    let root = tempdir().expect("tempdir");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            work_item_id: WORK_ITEM_ID.to_string(),
            base_branch: "HEAD".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
            },
            max_auto_rework: 2,
        })
        .expect("attempt");
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
            CodingExecutionStage::InternalPrReview,
        )
        .expect("internal review stage");
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    engine
        .create_review_blocked_gate(ReviewBlockedGateInput {
            attempt: &attempt,
            node_id: "coding_node_0010",
            stage: CodingExecutionStage::InternalPrReview,
            role: CodingProviderRole::InternalReviewer,
            title: "Internal PR review blocked".to_string(),
            description: "review interrupted".to_string(),
            reason_code: "internal_review_blocked",
            evidence_refs: Vec::new(),
            raw_provider_output_ref: None,
        })
        .await
        .expect("internal review blocked gate");

    let gate = store
        .list_open_blocked_gates(PROJECT_ID, ISSUE_ID, &attempt.id)
        .expect("open gates")
        .into_iter()
        .next()
        .expect("internal review gate");
    assert_eq!(
        gate.available_actions
            .iter()
            .map(|action| action.action_id.as_str())
            .collect::<Vec<_>>(),
        vec!["retry_internal_review", "send_to_coder", "abort"]
    );
    assert_eq!(gate.available_actions[0].label, "重试 Internal Review");
}

#[tokio::test]
async fn retry_coding_gate_clears_latest_coder_provider_conversation() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    let mut role_provider_config = store
        .get_role_provider_config_snapshot(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("role provider config");
    role_provider_config.coder = ProviderName::ClaudeCode;
    store
        .update_role_provider_config_snapshot(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            role_provider_config,
        )
        .expect("update role provider config");
    let attempt = store
        .replace_attempt_provider_conversations(
            &attempt.id,
            vec![
                ProviderConversationRef {
                    role: ProviderConversationRole::Coder,
                    provider: ProviderName::Codex,
                    provider_session_id: "old-codex-coder-thread".to_string(),
                    updated_at: "2026-07-13T00:00:00Z".to_string(),
                    last_node_id: Some("coding_node_0001".to_string()),
                },
                ProviderConversationRef {
                    role: ProviderConversationRole::Coder,
                    provider: ProviderName::ClaudeCode,
                    provider_session_id: "latest-claude-coder-thread".to_string(),
                    updated_at: "2026-07-13T00:00:01Z".to_string(),
                    last_node_id: Some("coding_node_0002".to_string()),
                },
                ProviderConversationRef {
                    role: ProviderConversationRole::CodeReviewer,
                    provider: ProviderName::ClaudeCode,
                    provider_session_id: "review-thread".to_string(),
                    updated_at: "2026-07-13T00:00:02Z".to_string(),
                    last_node_id: Some("coding_node_0003".to_string()),
                },
            ],
        )
        .expect("seed provider conversations");
    let attempt = store
        .update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::Coding,
        )
        .expect("coding stage");
    let attempt = store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Blocked,
        )
        .expect("blocked attempt");
    let gate = store
        .create_blocked_gate(CreateBlockedGateInput {
            attempt_id: attempt.id.clone(),
            stage: CodingExecutionStage::Coding,
            node_id: Some("coding_node_0002".to_string()),
            role: Some(CodingProviderRole::Coder),
            title: "Coder 执行中断".to_string(),
            description: "provider interrupted".to_string(),
            reason_code: Some("coder_provider_interrupted".to_string()),
            evidence_refs: Vec::new(),
            raw_provider_output_ref: None,
            available_actions: vec![
                coding_gate_action_for_id("retry_coding").expect("retry coding action"),
                coding_gate_action_for_id("abort").expect("abort action"),
            ],
        })
        .expect("coder recovery gate");
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    let resumed = engine
        .handle_blocked_gate_response(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &gate.gate_id,
            "retry_coding",
            None,
        )
        .await
        .expect("retry coding gate response");

    assert_eq!(resumed.status, CodingAttemptStatus::Running);
    assert_eq!(resumed.stage, CodingExecutionStage::Coding);
    assert!(resumed.provider_conversations.iter().any(|conversation| {
        conversation.role == ProviderConversationRole::Coder
            && conversation.provider == ProviderName::Codex
            && conversation.provider_session_id == "old-codex-coder-thread"
    }));
    assert!(resumed.provider_conversations.iter().all(|conversation| {
        !(conversation.role == ProviderConversationRole::Coder
            && conversation.provider == ProviderName::ClaudeCode)
    }));
    assert!(resumed.provider_conversations.iter().any(|conversation| {
        conversation.role == ProviderConversationRole::CodeReviewer
            && conversation.provider_session_id == "review-thread"
    }));
    assert!(
        store
            .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("resolved gate")
            .is_empty()
    );
}
