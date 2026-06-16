use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cadence_aria::cross_cutting::provider_adapter::ProviderAdapterError;
use cadence_aria::cross_cutting::streaming_provider::{
    ChoiceOptionData, ChoiceRequestData, ChoiceRequestSource, PermissionRequestData, ProviderEvent,
    ProviderExecutionEvent, ProviderExecutionEventKind, ProviderExecutionEventStatus,
    ProviderPermissionMode, ProviderSession, ProviderStatus, ProviderToolCall, ProviderToolResult,
    RiskLevel, StreamChunk, StreamingProviderAdapter, StreamingProviderInput,
};
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::coding_attempt_store::{
    CodingAttemptStore, CreateBlockedGateInput, CreateCodingAttemptInput,
};
use cadence_aria::product::coding_models::{
    AnalystDecisionNextStage, AnalystDecisionVerdict, AnalystVerdict, CodingAgentRole,
    CodingAttemptStatus, CodingChoiceGateStatus, CodingEntryType, CodingExecutionAttempt,
    CodingExecutionStage, CodingGateAction, CodingGateActionType, CodingProviderPermissionMode,
    CodingProviderRole, CodingReworkInstruction, CodingRolePermissionModes,
    CodingRoleProviderConfigSnapshot, CodingRoleRunEventType, CodingRoleRunStatus,
    CodingRoleRunTrigger, CodingTimelineNode, CodingTimelineNodeStatus, FindingSeverity,
    PushStatus, RemoteKind, ReviewRequest, ReviewRequestKind, ReviewVerdict, TestCommandStatus,
    TestingOverallStatus, TestingReport, TestingStepResult,
};
use cadence_aria::product::coding_workspace_engine::{
    CodingExecutionContext, CodingWorkspaceEngine, testing_report_should_enter_analyst,
};
use cadence_aria::product::git_workspace_service::GitWorkspaceService;
use cadence_aria::product::lifecycle_store::{
    CreateWorkItemInput, CreateWorkspaceSessionInput, LifecycleStore,
    UpsertIssueSharedWorktreeInput,
};
use cadence_aria::product::models::{
    ProviderConversationRef, ProviderConversationRole, ProviderName, WorkItemStatus, WorkspaceType,
};
use cadence_aria::product::test_executor::TestCommandSpec;
use cadence_aria::product::tester_agent_loop::TesterAgentOptions;
use cadence_aria::protocol::contracts::{AdapterInput, AdapterRole, ProviderType};
use cadence_aria::web::coding_ws_handler::CodingWsOutMessage;
use cadence_aria::web::workspace_ws_types::{
    ArtifactVersion, ProviderConfigSnapshot, WsExecutionEventKind, WsExecutionEventStatus,
};
use tempfile::tempdir;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn start_attempt_moves_to_worktree_prepare_and_creates_timeline_node() {
    let root = tempdir().expect("root");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(create_input())
        .expect("create attempt");
    let (tx, mut rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    let updated = engine
        .start_attempt("project_0001", "issue_0001", &attempt.id)
        .await
        .expect("start attempt");

    assert_eq!(updated.status, CodingAttemptStatus::Running);
    assert_eq!(updated.stage, CodingExecutionStage::WorktreePrepare);
    let nodes = store
        .get_timeline_nodes("project_0001", "issue_0001", &attempt.id)
        .expect("timeline nodes");
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].id, "coding_node_0001");
    assert_eq!(nodes[0].stage, CodingExecutionStage::WorktreePrepare);
    assert_eq!(nodes[0].status, CodingTimelineNodeStatus::Running);
    assert_eq!(nodes[0].agent_role, Some(CodingAgentRole::Git));

    assert_eq!(
        rx.recv().await.expect("stage event"),
        CodingWsOutMessage::CodingStageChange {
            stage: CodingExecutionStage::WorktreePrepare
        }
    );
    match rx.recv().await.expect("timeline event") {
        CodingWsOutMessage::CodingTimelineNodeCreated { node } => {
            assert_eq!(node.id, "coding_node_0001");
            assert_eq!(node.status, CodingTimelineNodeStatus::Running);
        }
        other => panic!("expected timeline node event, got {other:?}"),
    }
}

#[test]
fn role_permission_modes_are_persisted_with_role_provider_config() {
    let root = tempfile::tempdir().expect("root");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: "work_item_0001".to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
            },
            max_auto_rework: 2,
        })
        .expect("create attempt");

    let mut snapshot = store
        .get_role_provider_config_snapshot("project_0001", "issue_0001", &attempt.id)
        .expect("default role config");
    snapshot.set_permission_mode_for_role(
        &CodingProviderRole::CodeReviewer,
        CodingProviderPermissionMode::Auto,
    );
    store
        .update_role_provider_config_snapshot("project_0001", "issue_0001", &attempt.id, snapshot)
        .expect("save role config");

    let saved = store
        .get_role_provider_config_snapshot("project_0001", "issue_0001", &attempt.id)
        .expect("saved role config");
    assert_eq!(
        saved.permission_mode_for_role(&CodingProviderRole::CodeReviewer),
        CodingProviderPermissionMode::Auto
    );
}

#[test]
fn testing_report_routes_terminal_statuses_to_analyst_rework() {
    let blocked = TestingReport {
        id: "testing_report_0001".to_string(),
        attempt_id: "coding_attempt_0001".to_string(),
        role_run_id: None,
        run_no: None,
        commands: Vec::new(),
        overall_status: TestingOverallStatus::Blocked,
        provider_claim: None,
        backend_verified: true,
        started_at: "2026-06-11T00:00:00Z".to_string(),
        completed_at: Some("2026-06-11T00:00:01Z".to_string()),
        plan_id: None,
        plan_summary: None,
        steps: Vec::new(),
        unplanned_commands: Vec::new(),
        unplanned_evidence: Vec::new(),
        missing_required_steps: Vec::new(),
        skipped_required_steps: Vec::new(),
        context_warnings: vec!["test_plan_parse_error".to_string()],
        raw_provider_output_ref: Some("provider-raw/testing/plan_tests_0001.txt".to_string()),
    };
    assert!(testing_report_should_enter_analyst(&blocked));

    let mut failed_without_evidence = blocked.clone();
    failed_without_evidence.overall_status = TestingOverallStatus::Failed;
    assert!(testing_report_should_enter_analyst(
        &failed_without_evidence
    ));

    let mut failed_with_evidence = blocked.clone();
    failed_with_evidence.overall_status = TestingOverallStatus::Failed;
    failed_with_evidence.plan_id = Some("test_plan_0001".to_string());
    failed_with_evidence.steps = vec![TestingStepResult {
        step_id: "unit".to_string(),
        status: TestCommandStatus::Failed,
        evidence_refs: vec!["unit.stderr.log".to_string()],
        command: Some(vec![
            "cargo".to_string(),
            "test".to_string(),
            "--locked".to_string(),
        ]),
        provider_analysis: Some("unit failed".to_string()),
    }];
    assert!(testing_report_should_enter_analyst(&failed_with_evidence));

    let mut passed = blocked.clone();
    passed.overall_status = TestingOverallStatus::Passed;
    assert!(testing_report_should_enter_analyst(&passed));
}

#[tokio::test]
async fn execute_worktree_prepare_creates_git_worktree_and_completes_timeline_node() {
    let root = tempdir().expect("root");
    let repo = root.path().join("repo");
    init_repo(&repo);
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(create_input())
        .expect("create attempt");
    let (tx, mut rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let started = engine
        .start_attempt("project_0001", "issue_0001", &attempt.id)
        .await
        .expect("start attempt");
    let _stage = rx.recv().await.expect("stage event");
    let _node = rx.recv().await.expect("node event");

    let prepared = engine
        .execute_worktree_prepare(&started, &repo)
        .await
        .expect("prepare worktree");

    let expected_worktree = repo
        .join(".worktrees")
        .join("aria-work-items")
        .join("work_item_0001")
        .join("attempt-1");
    assert_eq!(
        prepared.worktree_path.as_deref(),
        Some(expected_worktree.as_path())
    );
    assert!(expected_worktree.join(".git").exists());
    let nodes = store
        .get_timeline_nodes("project_0001", "issue_0001", &attempt.id)
        .expect("timeline nodes");
    assert_eq!(nodes[0].status, CodingTimelineNodeStatus::Completed);
    assert_eq!(nodes[0].summary.as_deref(), Some("worktree 已准备"));

    match rx.recv().await.expect("timeline update") {
        CodingWsOutMessage::CodingTimelineNodeUpdated {
            node_id,
            status,
            summary,
            completed_at,
        } => {
            assert_eq!(node_id, "coding_node_0001");
            assert_eq!(status, CodingTimelineNodeStatus::Completed);
            assert_eq!(summary.as_deref(), Some("worktree 已准备"));
            assert!(completed_at.is_some());
        }
        other => panic!("expected timeline update, got {other:?}"),
    }
}

#[tokio::test]
async fn worktree_prepare_uses_issue_shared_worktree_path_for_issue_branch() {
    let root = tempdir().expect("root");
    let repo = git_repo_in(root.path().join("repo").as_path());
    let (store, attempt) =
        coding_store_with_attempt(root.path(), "work_item_0001", "aria/issues/issue_0001");
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    let updated = engine
        .execute_worktree_prepare(&attempt, &repo)
        .await
        .expect("prepare shared worktree");

    assert_eq!(
        updated.worktree_path.as_deref(),
        Some(
            repo.join(".worktrees")
                .join("aria-issues")
                .join("issue_0001")
                .as_path()
        )
    );
}

#[tokio::test]
async fn final_confirm_releases_issue_shared_worktree_lock() {
    let root = tempdir().expect("root");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(paths.clone());
    lifecycle
        .upsert_issue_shared_worktree(UpsertIssueSharedWorktreeInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: root.path().join("repo/.worktrees/aria-issues/issue_0001"),
            base_branch: "main".to_string(),
        })
        .expect("shared worktree");
    lifecycle
        .try_acquire_issue_worktree_lock("project_0001", "issue_0001", "work_item_0001")
        .expect("lock");
    let (store, attempt) = final_confirm_attempt(paths.clone(), "work_item_0001");
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);

    engine
        .handle_final_confirm("project_0001", "issue_0001", &attempt.id)
        .await
        .expect("final confirm");

    let shared = lifecycle
        .get_issue_shared_worktree("project_0001", "issue_0001")
        .expect("load shared")
        .expect("shared exists");
    assert_eq!(shared.current_active_work_item_id, None);
    assert_eq!(
        shared.last_completed_work_item_id.as_deref(),
        Some("work_item_0001")
    );
}

#[tokio::test]
async fn failed_attempt_releases_issue_shared_worktree_lock() {
    let root = tempdir().expect("root");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(paths.clone());
    lifecycle
        .upsert_issue_shared_worktree(UpsertIssueSharedWorktreeInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: root.path().join("repo/.worktrees/aria-issues/issue_0001"),
            base_branch: "main".to_string(),
        })
        .expect("shared worktree");
    lifecycle
        .try_acquire_issue_worktree_lock("project_0001", "issue_0001", "work_item_0001")
        .expect("lock");
    let (store, attempt) = failed_attempt(paths.clone(), "work_item_0001");
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);

    engine
        .handle_attempt_failed("project_0001", "issue_0001", &attempt.id)
        .await
        .expect("handle failed");

    let shared = lifecycle
        .get_issue_shared_worktree("project_0001", "issue_0001")
        .expect("load shared")
        .expect("shared exists");
    assert_eq!(shared.current_active_work_item_id, None);
}

#[tokio::test]
async fn dirty_shared_worktree_blocks_lock_release_and_next_work_item() {
    let root = tempdir().expect("root");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(paths.clone());
    let shared_path = root.path().join("repo/.worktrees/aria-issues/issue_0001");
    git_repo_in(&shared_path);
    std::fs::write(shared_path.join("dirty.txt"), "uncommitted").expect("dirty file");
    lifecycle
        .upsert_issue_shared_worktree(UpsertIssueSharedWorktreeInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: shared_path,
            base_branch: "main".to_string(),
        })
        .expect("shared worktree");
    lifecycle
        .try_acquire_issue_worktree_lock("project_0001", "issue_0001", "work_item_0001")
        .expect("lock");
    let (store, attempt) = dirty_failed_attempt(paths.clone(), "work_item_0001");
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);

    let error = engine
        .handle_attempt_failed("project_0001", "issue_0001", &attempt.id)
        .await
        .expect_err("dirty worktree keeps lock");

    assert!(format!("{error}").contains("shared_worktree_dirty_manual_gate"));
    let shared = lifecycle
        .get_issue_shared_worktree("project_0001", "issue_0001")
        .expect("load shared")
        .expect("shared exists");
    assert_eq!(
        shared.current_active_work_item_id.as_deref(),
        Some("work_item_0001")
    );
}

#[tokio::test]
async fn execute_coding_runs_provider_in_worktree_and_streams_timeline_events() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree.clone()),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let (tx, mut rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = FileWritingStreamingProvider;

    let updated = engine
        .execute_coding(&attempt, &provider, &CodingExecutionContext::default())
        .await
        .expect("execute coding");

    assert_eq!(updated.status, CodingAttemptStatus::Running);
    assert_eq!(updated.stage, CodingExecutionStage::Coding);
    assert_eq!(
        fs::read_to_string(worktree.join("generated.txt")).expect("generated file"),
        "generated by provider\n"
    );
    let nodes = store
        .get_timeline_nodes("project_0001", "issue_0001", &attempt.id)
        .expect("timeline nodes");
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].stage, CodingExecutionStage::Coding);
    assert_eq!(nodes[0].status, CodingTimelineNodeStatus::Completed);
    assert_eq!(nodes[0].summary.as_deref(), Some("代码编写完成"));

    match rx.recv().await.expect("coding node created") {
        CodingWsOutMessage::CodingTimelineNodeCreated { node } => {
            assert_eq!(node.id, "coding_node_0001");
            assert_eq!(node.stage, CodingExecutionStage::Coding);
            assert_eq!(node.status, CodingTimelineNodeStatus::Running);
        }
        other => panic!("expected coding node created, got {other:?}"),
    }
    match rx.recv().await.expect("provider prompt event") {
        CodingWsOutMessage::CodingExecutionEvent { event } => {
            assert_eq!(event.event_id, "coding_node_0001_prompt");
            assert_eq!(event.node_id.as_deref(), Some("coding_node_0001"));
            assert_eq!(event.title, "Provider Prompt");
            assert!(
                event
                    .output
                    .expect("prompt output")
                    .contains("Coding Workspace")
            );
        }
        other => panic!("expected provider prompt event, got {other:?}"),
    }
    assert_eq!(
        rx.recv().await.expect("coding stream chunk"),
        CodingWsOutMessage::CodingStreamChunk {
            content: "created generated.txt".to_string(),
            node_id: Some("coding_node_0001".to_string()),
        }
    );
    assert_eq!(
        rx.recv().await.expect("coding message complete"),
        CodingWsOutMessage::CodingMessageComplete {
            node_id: Some("coding_node_0001".to_string()),
        }
    );
    match rx.recv().await.expect("coding node completed") {
        CodingWsOutMessage::CodingTimelineNodeUpdated {
            node_id,
            status,
            summary,
            completed_at,
        } => {
            assert_eq!(node_id, "coding_node_0001");
            assert_eq!(status, CodingTimelineNodeStatus::Completed);
            assert_eq!(summary.as_deref(), Some("代码编写完成"));
            assert!(completed_at.is_some());
        }
        other => panic!("expected coding node completed, got {other:?}"),
    }
}

#[tokio::test]
async fn coding_coder_run_resumes_previous_coder_provider_session() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree.clone()),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::ClaudeCode,
                reviewer: Some(ProviderName::Codex),
                review_rounds: 1,
            },
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let (tx, mut rx) = mpsc::channel(64);
    tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);
    let provider = SessionInputCapturingProvider::default();

    let first = engine
        .execute_coding(&attempt, &provider, &CodingExecutionContext::default())
        .await
        .expect("first coding run");
    let second = engine
        .execute_coding(&first, &provider, &CodingExecutionContext::default())
        .await
        .expect("second coding run");

    assert_eq!(second.stage, CodingExecutionStage::Coding);
    let inputs = provider.inputs.lock().expect("inputs lock");
    assert_eq!(inputs.len(), 2);
    assert_eq!(
        inputs[0].permission_mode,
        ProviderPermissionMode::Supervised
    );
    assert_eq!(
        inputs[1].permission_mode,
        ProviderPermissionMode::Supervised
    );
    assert_eq!(inputs[0].timeout_secs, 10_800);
    assert_eq!(inputs[1].timeout_secs, 10_800);
    assert_eq!(inputs[0].resume_provider_session_id, None);
    assert_eq!(
        inputs[1].resume_provider_session_id.as_deref(),
        Some("coder-session-1")
    );
}

#[tokio::test]
async fn coding_coder_rework_with_resume_uses_delta_prompt() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::Fake),
                review_rounds: 1,
            },
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let context = CodingExecutionContext {
        work_item_markdown: Some(
            "# 爬楼梯问题 Work Item\n\n\
             ## 实现要求\n\
             这里是一段很长的已确认 Work Item，返修续接时不应重复发送。\n"
                .to_string(),
        ),
        verification_commands: vec!["uv run python -m unittest".to_string()],
    };
    let (tx, mut rx) = mpsc::channel(64);
    tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = SessionInputCapturingProvider::with_outputs(
        ["first coding done", "second coding done"],
        [
            Some("coder-session-1".to_string()),
            Some("coder-session-1".to_string()),
        ],
    );

    let first = engine
        .execute_coding(&attempt, &provider, &context)
        .await
        .expect("first coding run");
    store
        .save_rework_instruction(&CodingReworkInstruction {
            id: "coding_rework_instruction_0001".to_string(),
            attempt_id: attempt.id.clone(),
            source_stage: CodingExecutionStage::CodeReview,
            rework_round: 1,
            summary: "reviewer 要求补充边界测试".to_string(),
            fix_hints: vec!["补充 n=0 的输入处理".to_string()],
            questions: Vec::new(),
            created_at: "2026-06-07T00:00:00Z".to_string(),
            consumed_by_node_id: None,
            consumed_at: None,
        })
        .expect("save rework instruction");

    engine
        .execute_coding(&first, &provider, &context)
        .await
        .expect("second coding run");

    let inputs = provider.inputs.lock().expect("inputs lock");
    assert_eq!(inputs.len(), 2);
    let second_input = &inputs[1];
    assert_eq!(
        second_input.resume_provider_session_id.as_deref(),
        Some("coder-session-1")
    );
    assert!(second_input.prompt.contains("增量代码编写指令"));
    assert!(second_input.prompt.contains("reviewer 要求补充边界测试"));
    assert!(second_input.prompt.contains("补充 n=0 的输入处理"));
    assert!(second_input.prompt.contains("uv run python -m unittest"));
    assert!(!second_input.prompt.contains("# 爬楼梯问题 Work Item"));
    assert!(!second_input.prompt.contains("已确认 Work Item"));
    assert!(
        !second_input
            .prompt
            .contains("不要只输出计划或 Story/Design/Work Item 文档")
    );
}

#[tokio::test]
async fn coding_tester_does_not_resume_coder_provider_session() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::ClaudeCode,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
            },
            ..create_input()
        })
        .expect("create attempt");
    let attempt = store
        .replace_attempt_provider_conversations(
            &attempt.id,
            vec![
                ProviderConversationRef {
                    role: ProviderConversationRole::Coder,
                    provider: ProviderName::ClaudeCode,
                    provider_session_id: "coder-session-1".to_string(),
                    updated_at: "2026-06-01T00:00:00Z".to_string(),
                    last_node_id: Some("coding-node-1".to_string()),
                },
                ProviderConversationRef {
                    role: ProviderConversationRole::Tester,
                    provider: ProviderName::ClaudeCode,
                    provider_session_id: "tester-session-1".to_string(),
                    updated_at: "2026-06-01T00:01:00Z".to_string(),
                    last_node_id: Some("testing-node-1".to_string()),
                },
            ],
        )
        .expect("persist provider conversations");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let (tx, mut rx) = mpsc::channel(64);
    tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = SessionInputCapturingProvider::with_outputs(
        [
            r#"{"summary":"testing plan","steps":[{"id":"provider_check","title":"Provider check","intent":"verify provider session isolation","required":true,"tool":"provider_managed","risk_level":"low","command_or_tool_input":{},"evidence_expectation":"provider evidence","related_requirements":["REQ-TEST"],"related_design_constraints":["DEC-TEST"],"related_work_item_tasks":["TASK-TEST"]}]}"#,
            r#"{"step_results":[{"step_id":"provider_check","status":"passed","evidence_refs":["provider-session.log"],"provider_analysis":"session isolated"}]}"#,
        ],
        [None, Some("tester-session-2".to_string())],
    );

    let _report = engine
        .execute_testing_with_provider(
            &attempt,
            &provider,
            &CodingExecutionContext::default(),
            &[],
            TesterAgentOptions::default(),
        )
        .await
        .expect("testing provider run");

    let inputs = provider.inputs.lock().expect("inputs lock");
    assert_eq!(inputs.len(), 2);
    assert!(inputs[0].prompt.contains("Phase: plan_tests"));
    assert!(inputs[1].prompt.contains("Phase: execute_test_plan"));
    for input in inputs.iter() {
        assert_eq!(input.permission_mode, ProviderPermissionMode::Auto);
        assert_eq!(input.timeout_secs, 10_800);
        assert_eq!(input.resume_provider_session_id, None);
    }
    let updated = store
        .get_attempt("project_0001", "issue_0001", &attempt.id)
        .expect("updated attempt");
    assert!(updated.provider_conversations.iter().any(|conversation| {
        conversation.role == ProviderConversationRole::Tester
            && conversation.provider == ProviderName::ClaudeCode
            && conversation.provider_session_id == "tester-session-2"
    }));
}

#[tokio::test]
async fn coding_tester_uses_role_permission_mode_auto() {
    let root = tempfile::tempdir().expect("root");
    let worktree = root.path().join("worktree");
    init_repo(&worktree);
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            ..create_input()
        })
        .expect("create attempt");
    let mut role_config = store
        .get_role_provider_config_snapshot("project_0001", "issue_0001", &attempt.id)
        .expect("role config");
    role_config.set_permission_mode_for_role(
        &CodingProviderRole::Tester,
        CodingProviderPermissionMode::Auto,
    );
    store
        .update_role_provider_config_snapshot(
            "project_0001",
            "issue_0001",
            &attempt.id,
            role_config,
        )
        .expect("save role config");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");

    let (tx, mut rx) = mpsc::channel(64);
    tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);
    let provider = SessionInputCapturingProvider::with_outputs(
        [
            r#"{"summary":"unit","steps":[{"id":"unit","title":"Unit","intent":"verify unit","required":true,"tool":"provider_managed","risk_level":"low","command_or_tool_input":{},"evidence_expectation":"provider evidence","related_requirements":["REQ-UNIT"],"related_design_constraints":["DEC-UNIT"],"related_work_item_tasks":["TASK-UNIT"]}]}"#,
            r#"{"step_results":[{"step_id":"unit","status":"passed","evidence_refs":["unit.log"],"provider_analysis":"ok"}]}"#,
        ],
        [None, None],
    );

    engine
        .execute_testing_with_provider(
            &attempt,
            &provider,
            &CodingExecutionContext {
                work_item_markdown: Some("Work Item".to_string()),
                verification_commands: Vec::new(),
            },
            &[],
            TesterAgentOptions::default(),
        )
        .await
        .expect("testing");

    let inputs = provider.inputs.lock().expect("inputs");
    assert_eq!(inputs[0].permission_mode, ProviderPermissionMode::Auto);
    assert_eq!(inputs[1].permission_mode, ProviderPermissionMode::Auto);
}

#[tokio::test]
async fn execute_testing_binds_plan_report_and_chat_entries_to_tester_role_run() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let (tx, mut rx) = mpsc::channel(64);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = ExecutePlanToolCallTesterProvider::new();

    let report = engine
        .execute_testing_with_provider(
            &attempt,
            &provider,
            &CodingExecutionContext::default(),
            &[],
            TesterAgentOptions::default(),
        )
        .await
        .expect("testing");

    let runs = store
        .list_role_runs("project_0001", "issue_0001", &attempt.id)
        .expect("role runs");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].role, CodingProviderRole::Tester);
    assert_eq!(runs[0].status, CodingRoleRunStatus::Completed);
    assert_eq!(report.role_run_id.as_deref(), Some(runs[0].id.as_str()));
    assert_eq!(report.run_no, Some(1));
    let events = store
        .list_role_run_events("project_0001", "issue_0001", &attempt.id, &runs[0].id)
        .expect("role run events");
    let provider_prompts = events
        .iter()
        .filter(|event| event.event_type == CodingRoleRunEventType::ProviderPrompt)
        .collect::<Vec<_>>();
    assert_eq!(provider_prompts.len(), 2);
    assert!(provider_prompts.iter().any(|event| {
        event.payload["output_schema"] == "coding_workspace_test_plan_json"
            && event.payload["prompt"]
                .as_str()
                .is_some_and(|prompt| prompt.contains("Phase: plan_tests"))
    }));
    assert!(provider_prompts.iter().any(|event| {
        event.payload["output_schema"] == "coding_workspace_execute_test_plan_json"
            && event.payload["prompt"]
                .as_str()
                .is_some_and(|prompt| prompt.contains("Phase: execute_test_plan"))
    }));
    assert!(events.iter().any(|event| {
        event.event_type == CodingRoleRunEventType::ToolCall
            && event.payload["id"] == "execute_tool_0001"
            && event.payload["tool_name"] == "run_command"
    }));
    assert!(events.iter().any(|event| {
        event.event_type == CodingRoleRunEventType::ToolResult
            && event.payload["tool_use_id"] == "execute_tool_0001"
            && event.payload["is_error"] == false
    }));
    assert!(
        events
            .iter()
            .any(|event| event.event_type == CodingRoleRunEventType::MessageComplete)
    );

    let plans = store
        .list_test_plans("project_0001", "issue_0001", &attempt.id)
        .expect("plans");
    assert_eq!(plans[0].role_run_id.as_deref(), Some(runs[0].id.as_str()));
    assert_eq!(plans[0].run_no, Some(1));

    let mut saw_plan_entry = false;
    let mut saw_result_entry = false;
    while let Ok(message) = rx.try_recv() {
        if let CodingWsOutMessage::CodingChatEntryCreated { entry } = message {
            let metadata = entry.metadata.unwrap_or_default();
            let content = entry.content.unwrap_or_default();
            if metadata.get("role_run_id").and_then(|value| value.as_str())
                == Some(runs[0].id.as_str())
                && metadata.get("phase").and_then(|value| value.as_str()) == Some("test_plan")
            {
                saw_plan_entry = true;
                assert!(content.contains("unit plan"));
            }
            if metadata.get("role_run_id").and_then(|value| value.as_str())
                == Some(runs[0].id.as_str())
                && metadata.get("phase").and_then(|value| value.as_str()) == Some("testing_result")
            {
                saw_result_entry = true;
                assert!(content.contains("passed"));
            }
        }
    }
    assert!(saw_plan_entry);
    assert!(saw_result_entry);
}

#[tokio::test]
async fn execute_testing_blocks_when_provider_completes_before_choice_response() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let (tx, mut rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = ExecutePlanChoiceThenCompletedTesterProvider::default();

    let error = engine
        .execute_testing_with_provider(
            &attempt,
            &provider,
            &CodingExecutionContext::default(),
            &[],
            TesterAgentOptions::default(),
        )
        .await
        .expect_err("provider cannot complete execute_test_plan with unresolved choice");

    assert_eq!(
        error.to_string(),
        "coding_provider_stream_failed: provider_choice_unresolved"
    );
    assert_eq!(
        store
            .get_attempt("project_0001", "issue_0001", &attempt.id)
            .expect("attempt")
            .status,
        CodingAttemptStatus::WaitingForHuman
    );
    assert_eq!(
        store
            .list_open_choice_gates("project_0001", "issue_0001", &attempt.id)
            .expect("open choice gates")
            .len(),
        1
    );
    let events = drain_events(&mut rx);
    assert!(events.iter().any(|event| {
        matches!(
            event,
            CodingWsOutMessage::CodingChoiceRequest {
                id,
                source,
                ..
            } if id == "choice_0001" && source == "ask_user_question"
        )
    }));
}

#[tokio::test]
async fn tester_plan_timeout_blocks_with_retry_test_plan_gate() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let (tx, _rx) = mpsc::channel(64);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    let result = tokio::time::timeout(
        Duration::from_secs(2),
        engine.execute_testing_with_provider(
            &attempt,
            &HangingPlanTesterProvider,
            &CodingExecutionContext::default(),
            &[],
            TesterAgentOptions {
                timeout: Duration::from_millis(20),
                failure_limit: 3,
            },
        ),
    )
    .await
    .expect("engine should return before outer timeout");
    let report = result.expect("timeout becomes blocked report");

    assert_eq!(report.overall_status, TestingOverallStatus::Blocked);
    assert!(
        report
            .context_warnings
            .contains(&"plan_tests_timeout".to_string())
    );
    let gates = store
        .list_open_blocked_gates("project_0001", "issue_0001", &attempt.id)
        .expect("gates");
    assert_eq!(gates.len(), 1);
    assert_eq!(gates[0].reason_code.as_deref(), Some("plan_tests_timeout"));
    assert!(
        gates[0]
            .available_actions
            .iter()
            .any(|action| action.action_id == "retry_test_plan")
    );
}

#[tokio::test]
async fn tester_plan_start_timeout_blocks_with_retry_test_plan_gate() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let (tx, _rx) = mpsc::channel(64);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    let result = tokio::time::timeout(
        Duration::from_secs(2),
        engine.execute_testing_with_provider(
            &attempt,
            &NeverStartingTesterProvider,
            &CodingExecutionContext::default(),
            &[],
            TesterAgentOptions {
                timeout: Duration::from_millis(20),
                failure_limit: 3,
            },
        ),
    )
    .await
    .expect("engine should return before outer timeout");
    let report = result.expect("start timeout becomes blocked report");

    assert_eq!(report.overall_status, TestingOverallStatus::Blocked);
    assert!(
        report
            .context_warnings
            .contains(&"plan_tests_timeout".to_string())
    );
    let runs = store
        .list_role_runs("project_0001", "issue_0001", &attempt.id)
        .expect("role runs");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].status, CodingRoleRunStatus::Blocked);
    assert_eq!(runs[0].reason_code.as_deref(), Some("plan_tests_timeout"));
    let events = store
        .list_role_run_events("project_0001", "issue_0001", &attempt.id, &runs[0].id)
        .expect("role run events");
    assert!(
        events
            .iter()
            .any(|event| event.event_type == CodingRoleRunEventType::ProviderPrompt)
    );
    assert!(
        events
            .iter()
            .any(|event| event.event_type == CodingRoleRunEventType::Timeout)
    );
    let gates = store
        .list_open_blocked_gates("project_0001", "issue_0001", &attempt.id)
        .expect("gates");
    assert_eq!(gates.len(), 1);
    assert_eq!(gates[0].reason_code.as_deref(), Some("plan_tests_timeout"));
    assert!(
        gates[0]
            .available_actions
            .iter()
            .any(|action| action.action_id == "retry_test_plan")
    );
}

#[tokio::test]
async fn tester_execute_plan_start_timeout_blocks_with_retry_gate() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let (tx, _rx) = mpsc::channel(64);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    let result = tokio::time::timeout(
        Duration::from_secs(2),
        engine.execute_testing_with_provider(
            &attempt,
            &HangingExecutePlanStartTesterProvider::default(),
            &CodingExecutionContext::default(),
            &[],
            TesterAgentOptions {
                timeout: Duration::from_millis(20),
                failure_limit: 3,
            },
        ),
    )
    .await
    .expect("engine should return before outer timeout");
    let report = result.expect("execute start timeout becomes blocked report");

    assert_eq!(report.overall_status, TestingOverallStatus::Blocked);
    assert!(
        report
            .context_warnings
            .contains(&"execute_test_plan_timeout".to_string())
    );
    let runs = store
        .list_role_runs("project_0001", "issue_0001", &attempt.id)
        .expect("role runs");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].status, CodingRoleRunStatus::Blocked);
    assert_eq!(
        runs[0].reason_code.as_deref(),
        Some("execute_test_plan_timeout")
    );
    let events = store
        .list_role_run_events("project_0001", "issue_0001", &attempt.id, &runs[0].id)
        .expect("role run events");
    assert!(events.iter().any(|event| {
        event.event_type == CodingRoleRunEventType::ProviderPrompt
            && event.payload["output_schema"] == "coding_workspace_execute_test_plan_json"
            && event.payload["prompt"]
                .as_str()
                .is_some_and(|prompt| prompt.contains("Phase: execute_test_plan"))
    }));
    assert!(events.iter().any(|event| {
        event.event_type == CodingRoleRunEventType::Timeout
            && event.payload["phase"] == "execute_test_plan_start"
            && event.payload["reason_code"] == "execute_test_plan_timeout"
    }));
    let gates = store
        .list_open_blocked_gates("project_0001", "issue_0001", &attempt.id)
        .expect("gates");
    assert_eq!(gates.len(), 1);
    assert_eq!(
        gates[0].reason_code.as_deref(),
        Some("execute_test_plan_timeout")
    );
    assert!(
        gates[0]
            .available_actions
            .iter()
            .any(|action| action.action_id == "retry_test_plan")
    );
}

#[tokio::test]
async fn blocked_testing_gate_reason_overrides_report_warning_for_role_run() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let (tx, _rx) = mpsc::channel(64);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    let report = tokio::time::timeout(
        Duration::from_secs(2),
        engine.execute_testing_with_provider(
            &attempt,
            &HangingExecutePlanStartTesterProvider::with_plan_warning("timeout budget risk"),
            &CodingExecutionContext::default(),
            &[],
            TesterAgentOptions {
                timeout: Duration::from_millis(20),
                failure_limit: 3,
            },
        ),
    )
    .await
    .expect("engine should return before outer timeout")
    .expect("execute start timeout becomes blocked report");

    assert_eq!(report.overall_status, TestingOverallStatus::Blocked);
    assert!(
        report
            .context_warnings
            .contains(&"timeout budget risk".to_string())
    );
    assert!(
        report
            .context_warnings
            .contains(&"execute_test_plan_timeout".to_string())
    );
    let gates = store
        .list_open_blocked_gates("project_0001", "issue_0001", &attempt.id)
        .expect("gates");
    assert_eq!(gates.len(), 1);
    assert_eq!(
        gates[0].reason_code.as_deref(),
        Some("execute_test_plan_timeout")
    );
    let runs = store
        .list_role_runs("project_0001", "issue_0001", &attempt.id)
        .expect("role runs");
    assert_eq!(runs.len(), 1);
    assert_eq!(
        runs[0].reason_code.as_deref(),
        Some("execute_test_plan_timeout")
    );
}

#[tokio::test]
async fn retry_test_plan_supersedes_latest_testing_role_run_and_resumes_testing() {
    let root = tempdir().expect("root");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let worktree = root.path().join("worktree");
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            ..create_input()
        })
        .expect("create attempt");
    fs::create_dir_all(attempt.worktree_path.as_ref().expect("worktree")).expect("worktree dir");
    let attempt = store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let attempt = store
        .update_attempt_stage(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingExecutionStage::Testing,
        )
        .expect("testing");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Blocked,
        )
        .expect("blocked");
    let first_run = store
        .create_role_run(
            &attempt,
            CodingExecutionStage::Testing,
            CodingProviderRole::Tester,
            CodingRoleRunTrigger::Initial,
            Some("coding_node_0003".to_string()),
        )
        .expect("first run");
    store
        .update_role_run_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            &first_run.id,
            CodingRoleRunStatus::Blocked,
            Some("plan_tests_timeout".to_string()),
        )
        .expect("blocked run");
    let gate = store
        .create_blocked_gate(CreateBlockedGateInput {
            attempt_id: attempt.id.clone(),
            stage: CodingExecutionStage::Testing,
            node_id: Some("coding_node_0003".to_string()),
            role: Some(CodingProviderRole::Tester),
            title: "Testing blocked".to_string(),
            description: "Tester plan timeout".to_string(),
            reason_code: Some("plan_tests_timeout".to_string()),
            evidence_refs: vec![],
            raw_provider_output_ref: None,
            available_actions: vec![
                CodingGateAction {
                    action_id: "retry_test_plan".to_string(),
                    label: "重新执行 Tester".to_string(),
                    action_type: CodingGateActionType::RetryTestPlan,
                },
                CodingGateAction {
                    action_id: "send_raw_output_to_analyst".to_string(),
                    label: "发送给 Analyst 决策".to_string(),
                    action_type: CodingGateActionType::SendRawOutputToAnalyst,
                },
                CodingGateAction {
                    action_id: "abort".to_string(),
                    label: "终止".to_string(),
                    action_type: CodingGateActionType::Abort,
                },
            ],
        })
        .expect("gate");
    let (tx, mut rx) = mpsc::channel(64);
    tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    let updated = engine
        .handle_blocked_gate_response(
            "project_0001",
            "issue_0001",
            &attempt.id,
            &gate.gate_id,
            "retry_test_plan",
            None,
        )
        .await
        .expect("gate response");

    assert_eq!(updated.status, CodingAttemptStatus::Running);
    assert_eq!(updated.stage, CodingExecutionStage::Testing);
    let runs = store
        .list_role_runs("project_0001", "issue_0001", &attempt.id)
        .expect("runs");
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].status, CodingRoleRunStatus::Superseded);
    assert_eq!(runs[1].trigger, CodingRoleRunTrigger::RetryTestPlan);
    assert_eq!(runs[1].run_no, 2);
    assert_eq!(runs[1].node_id, None);

    let provider = SessionInputCapturingProvider::with_outputs(
        [
            r#"{"summary":"retry plan","steps":[{"id":"unit","title":"Unit","intent":"run unit checks","required":true,"tool":"provider_managed","risk_level":"low","command_or_tool_input":{},"evidence_expectation":"unit evidence","related_requirements":["REQ-UNIT"],"related_design_constraints":["DEC-UNIT"],"related_work_item_tasks":["TASK-UNIT"]}]}"#,
            r#"{"step_results":[{"step_id":"unit","status":"passed","evidence_refs":["unit.log"],"provider_analysis":"ok"}]}"#,
        ],
        [None, None],
    );
    let report = engine
        .execute_testing_with_provider(
            &updated,
            &provider,
            &CodingExecutionContext::default(),
            &[],
            TesterAgentOptions::default(),
        )
        .await
        .expect("rerun testing");

    let runs = store
        .list_role_runs("project_0001", "issue_0001", &attempt.id)
        .expect("runs after rerun");
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[1].status, CodingRoleRunStatus::Completed);
    assert_eq!(report.role_run_id.as_deref(), Some(runs[1].id.as_str()));
    assert!(runs[1].node_id.is_some());
}

#[tokio::test]
async fn retry_test_plan_prompt_includes_previous_role_run_diagnostic() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Blocked,
        )
        .expect("blocked");
    store
        .update_attempt_stage(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingExecutionStage::Testing,
        )
        .expect("testing");

    let first_run = store
        .create_role_run(
            &attempt,
            CodingExecutionStage::Testing,
            CodingProviderRole::Tester,
            CodingRoleRunTrigger::Initial,
            Some("coding_node_0001".to_string()),
        )
        .expect("first run");
    store
        .append_role_run_event(
            &attempt,
            &first_run,
            CodingRoleRunEventType::ExecutionEvent,
            serde_json::json!({
                "title": "Task update",
                "status": "running",
                "detail": "No tasks found"
            }),
        )
        .expect("event");
    store
        .append_role_run_event(
            &attempt,
            &first_run,
            CodingRoleRunEventType::Timeout,
            serde_json::json!({
                "reason_code": "plan_tests_timeout",
                "message": "timed out"
            }),
        )
        .expect("timeout");
    store
        .update_role_run_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            &first_run.id,
            CodingRoleRunStatus::Blocked,
            Some("plan_tests_timeout".to_string()),
        )
        .expect("block first run");
    let resumed = store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("resume status");
    let retry_run = store
        .supersede_latest_role_run_and_create(
            &resumed,
            CodingExecutionStage::Testing,
            CodingProviderRole::Tester,
            CodingRoleRunTrigger::RetryTestPlan,
            None,
            Some("plan_tests_timeout".to_string()),
        )
        .expect("retry run");
    assert_eq!(
        retry_run.supersedes_run_id.as_deref(),
        Some(first_run.id.as_str())
    );

    let prompts = Arc::new(Mutex::new(Vec::new()));
    let provider = TesterRetryPromptCaptureProvider {
        prompts: prompts.clone(),
    };
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    engine
        .execute_testing_with_provider(
            &resumed,
            &provider,
            &CodingExecutionContext::default(),
            &[],
            TesterAgentOptions {
                timeout: Duration::from_secs(5),
                failure_limit: 3,
            },
        )
        .await
        .expect("execute retry tester");

    let captured = prompts.lock().expect("prompts");
    let prompt = captured.first().expect("first prompt");
    assert!(prompt.contains("[previous_role_run_diagnostic]"));
    assert!(prompt.contains("reason_code: plan_tests_timeout"));
    assert!(prompt.contains("No tasks found"));
    assert!(prompt.contains("CRITICAL: Return ONLY a single JSON object"));
    let diagnostic_index = prompt
        .find("[previous_role_run_diagnostic]")
        .expect("diagnostic marker");
    let final_critical_index = prompt
        .find("CRITICAL: Return ONLY a single JSON object. Do not summarize validation.")
        .expect("final critical instruction");
    assert!(diagnostic_index < final_critical_index);
}

#[tokio::test]
async fn retry_code_review_prompt_includes_previous_role_run_diagnostic() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    init_repo(&worktree);
    fs::write(worktree.join("src.txt"), "hello\nreviewed\n").expect("modify file");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            base_branch: "HEAD".to_string(),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    store
        .update_attempt_stage(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingExecutionStage::CodeReview,
        )
        .expect("code review stage");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Blocked,
        )
        .expect("blocked");

    let first_run = store
        .create_role_run(
            &attempt,
            CodingExecutionStage::CodeReview,
            CodingProviderRole::CodeReviewer,
            CodingRoleRunTrigger::Initial,
            Some("coding_node_0003".to_string()),
        )
        .expect("first run");
    store
        .append_role_run_event(
            &attempt,
            &first_run,
            CodingRoleRunEventType::ExecutionEvent,
            serde_json::json!({
                "title": "Code reviewer task update",
                "status": "blocked",
                "detail": "Review context was missing"
            }),
        )
        .expect("event");
    store
        .update_role_run_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            &first_run.id,
            CodingRoleRunStatus::Blocked,
            Some("code_review_blocked".to_string()),
        )
        .expect("block first run");
    let resumed = store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("resume status");
    let retry_run = store
        .supersede_latest_role_run_and_create(
            &resumed,
            CodingExecutionStage::CodeReview,
            CodingProviderRole::CodeReviewer,
            CodingRoleRunTrigger::RetryReview,
            None,
            Some("code_review_blocked".to_string()),
        )
        .expect("retry run");
    assert_eq!(
        retry_run.supersedes_run_id.as_deref(),
        Some(first_run.id.as_str())
    );

    let provider = SessionInputCapturingProvider::with_outputs(
        [r#"{"verdict":"approve","summary":"code review ok","findings":[]}"#],
        [None],
    );
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    engine
        .execute_code_review(&resumed, &provider)
        .await
        .expect("execute retry code review");

    let inputs = provider.inputs.lock().expect("inputs");
    let prompt = &inputs.first().expect("first input").prompt;
    assert!(prompt.contains("[previous_role_run_diagnostic]"));
    assert!(prompt.contains("reason_code: code_review_blocked"));
    assert!(prompt.contains("Code reviewer task update"));
    assert!(prompt.contains("Review context was missing"));
    assert!(prompt.contains("只输出 JSON"));
    assert!(!prompt.contains(&format!("role_run_id: {}", retry_run.id)));
}

#[tokio::test]
async fn coding_code_reviewer_run_uses_fresh_provider_session() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    init_repo(&worktree);
    fs::write(worktree.join("src.txt"), "hello\nreviewed\n").expect("modify file");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            base_branch: "HEAD".to_string(),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::ClaudeCode,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
            },
            ..create_input()
        })
        .expect("create attempt");
    let attempt = store
        .replace_attempt_provider_conversations(
            &attempt.id,
            vec![
                ProviderConversationRef {
                    role: ProviderConversationRole::Coder,
                    provider: ProviderName::ClaudeCode,
                    provider_session_id: "coder-session-1".to_string(),
                    updated_at: "2026-06-01T00:00:00Z".to_string(),
                    last_node_id: Some("coding-node-1".to_string()),
                },
                ProviderConversationRef {
                    role: ProviderConversationRole::Tester,
                    provider: ProviderName::ClaudeCode,
                    provider_session_id: "tester-session-1".to_string(),
                    updated_at: "2026-06-01T00:01:00Z".to_string(),
                    last_node_id: Some("testing-node-1".to_string()),
                },
                ProviderConversationRef {
                    role: ProviderConversationRole::CodeReviewer,
                    provider: ProviderName::ClaudeCode,
                    provider_session_id: "code-reviewer-session-0".to_string(),
                    updated_at: "2026-06-01T00:02:00Z".to_string(),
                    last_node_id: Some("code-review-node-0".to_string()),
                },
            ],
        )
        .expect("persist conversations");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = SessionInputCapturingProvider::with_outputs(
        [r#"{"verdict":"approve","summary":"code review ok","findings":[]}"#],
        [Some("code-reviewer-session-1".to_string())],
    );

    let report = engine
        .execute_code_review(&attempt, &provider)
        .await
        .expect("code review provider run");

    assert_eq!(report.verdict, ReviewVerdict::Approve);
    let inputs = provider.inputs.lock().expect("inputs lock");
    assert_eq!(inputs.len(), 1);
    assert_eq!(
        inputs[0].permission_mode,
        ProviderPermissionMode::Supervised
    );
    assert_eq!(inputs[0].timeout_secs, 10_800);
    assert_eq!(inputs[0].resume_provider_session_id, None);
    let updated = store
        .get_attempt("project_0001", "issue_0001", &attempt.id)
        .expect("updated attempt");
    assert!(updated.provider_conversations.iter().any(|conversation| {
        conversation.role == ProviderConversationRole::CodeReviewer
            && conversation.provider == ProviderName::ClaudeCode
            && conversation.provider_session_id == "code-reviewer-session-1"
    }));
}

#[tokio::test]
async fn coding_analyst_rework_uses_fresh_provider_session() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::ClaudeCode,
                reviewer: Some(ProviderName::Codex),
                review_rounds: 1,
            },
            ..create_input()
        })
        .expect("create attempt");
    let attempt = store
        .replace_attempt_provider_conversations(
            &attempt.id,
            vec![ProviderConversationRef {
                role: ProviderConversationRole::Analyst,
                provider: ProviderName::ClaudeCode,
                provider_session_id: "analyst-session-1".to_string(),
                updated_at: "2026-06-01T00:00:00Z".to_string(),
                last_node_id: Some("rework-node-1".to_string()),
            }],
        )
        .expect("persist analyst conversation");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    store
        .update_attempt_stage(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingExecutionStage::Testing,
        )
        .expect("testing stage");
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);
    let provider = SessionInputCapturingProvider::with_outputs(
        [r#"{"verdict":"no_issue","summary":"testing ok"}"#],
        [Some("analyst-session-2".to_string())],
    );

    engine
        .execute_rework(&attempt, "testing evidence", &provider)
        .await
        .expect("analyst rework provider run");

    let inputs = provider.inputs.lock().expect("inputs lock");
    assert_eq!(inputs.len(), 1);
    assert_eq!(inputs[0].permission_mode, ProviderPermissionMode::Auto);
    assert_eq!(inputs[0].timeout_secs, 10_800);
    assert_eq!(inputs[0].resume_provider_session_id, None);
}

#[tokio::test]
async fn coding_internal_reviewer_uses_fresh_provider_session() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    init_repo(&worktree);
    fs::write(worktree.join("src.txt"), "hello\ninternal review\n").expect("modify file");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            base_branch: "HEAD".to_string(),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::ClaudeCode,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
            },
            ..create_input()
        })
        .expect("create attempt");
    let attempt = store
        .replace_attempt_provider_conversations(
            &attempt.id,
            vec![ProviderConversationRef {
                role: ProviderConversationRole::InternalReviewer,
                provider: ProviderName::ClaudeCode,
                provider_session_id: "internal-reviewer-session-1".to_string(),
                updated_at: "2026-06-01T00:00:00Z".to_string(),
                last_node_id: Some("internal-review-node-1".to_string()),
            }],
        )
        .expect("persist internal reviewer conversation");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    store
        .update_attempt_stage(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingExecutionStage::ReviewRequest,
        )
        .expect("review request stage");
    store
        .save_review_request(&sample_review_request(&attempt.id))
        .expect("save review request");
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);
    let provider = SessionInputCapturingProvider::with_outputs(
        [
            r#"{"verdict":"approve","summary":"internal review ok","findings":[],"impact_scope":["src"],"pr_description":"实现 work item","commit_message_suggestion":"feat: implement work item"}"#,
        ],
        [Some("internal-reviewer-session-2".to_string())],
    );

    let review = engine
        .execute_internal_pr_review(&attempt, &provider)
        .await
        .expect("internal reviewer provider run");

    assert_eq!(review.verdict, ReviewVerdict::Approve);
    let inputs = provider.inputs.lock().expect("inputs lock");
    assert_eq!(inputs.len(), 1);
    assert_eq!(
        inputs[0].permission_mode,
        ProviderPermissionMode::Supervised
    );
    assert_eq!(inputs[0].timeout_secs, 10_800);
    assert_eq!(inputs[0].resume_provider_session_id, None);
}

#[tokio::test]
async fn execute_coding_includes_work_item_context_in_provider_prompt() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let captured_prompt = Arc::new(Mutex::new(None));
    let provider = PromptCapturingProvider {
        prompt: Arc::clone(&captured_prompt),
    };
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);
    let context = CodingExecutionContext {
        work_item_markdown: Some(
            "# 爬楼梯问题 Work Item\n\n## 验证命令\n\n- `uv run python -m unittest -v tests.test_climbing_stairs`"
                .to_string(),
        ),
        verification_commands: vec![
            "uv run python -m unittest -v tests.test_climbing_stairs".to_string(),
        ],
    };

    engine
        .execute_coding(&attempt, &provider, &context)
        .await
        .expect("execute coding");

    let prompt = captured_prompt
        .lock()
        .expect("prompt lock")
        .clone()
        .expect("captured prompt");
    assert!(prompt.contains("不要只输出计划或 Story/Design/Work Item 文档"));
    assert!(prompt.contains("# 爬楼梯问题 Work Item"));
    assert!(prompt.contains("uv run python -m unittest -v tests.test_climbing_stairs"));
}

#[tokio::test]
async fn coding_prompt_includes_rework_fix_hints() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    store
        .save_rework_instruction(&CodingReworkInstruction {
            id: "coding_rework_instruction_0001".to_string(),
            attempt_id: attempt.id.clone(),
            source_stage: CodingExecutionStage::CodeReview,
            rework_round: 1,
            summary: "reviewer 要求移除运行产物".to_string(),
            fix_hints: vec!["移除 __pycache__ 和 .pyc 文件".to_string()],
            questions: vec!["确认 git diff 只包含业务文件".to_string()],
            created_at: "2026-05-29T00:00:00Z".to_string(),
            consumed_by_node_id: None,
            consumed_at: None,
        })
        .expect("save rework instruction");
    let captured_prompt = Arc::new(Mutex::new(None));
    let provider = PromptCapturingProvider {
        prompt: Arc::clone(&captured_prompt),
    };
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    engine
        .execute_coding(&attempt, &provider, &CodingExecutionContext::default())
        .await
        .expect("execute coding");

    let prompt = captured_prompt
        .lock()
        .expect("prompt lock")
        .clone()
        .expect("captured prompt");
    assert!(prompt.contains("上一轮返修要求"));
    assert!(prompt.contains("来源阶段: CodeReview"));
    assert!(prompt.contains("reviewer 要求移除运行产物"));
    assert!(prompt.contains("移除 __pycache__ 和 .pyc 文件"));
    assert!(prompt.contains("确认 git diff 只包含业务文件"));
    let consumed = store
        .latest_unconsumed_rework_instruction("project_0001", "issue_0001", &attempt.id)
        .expect("latest instruction");
    assert_eq!(consumed, None);
}

#[tokio::test]
async fn execute_coding_includes_unconsumed_context_notes_and_consumes_them() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let mut attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            ..create_input()
        })
        .expect("create attempt");
    attempt = store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    attempt = store
        .increment_attempt_rework_count("project_0001", "issue_0001", &attempt.id)
        .expect("first rework");
    let old_note = store
        .create_context_note(&attempt.id, "不要带入本轮 Coder prompt".to_string())
        .expect("old context note");
    store
        .mark_context_notes_consumed("project_0001", "issue_0001", &attempt.id, &[old_note.id], 1)
        .expect("consume old note");
    let new_note = store
        .create_context_note(
            &attempt.id,
            "请优先修复 provider_install SSE 订阅".to_string(),
        )
        .expect("new context note");
    let captured_prompt = Arc::new(Mutex::new(None));
    let provider = PromptCapturingProvider {
        prompt: Arc::clone(&captured_prompt),
    };
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    engine
        .execute_coding(&attempt, &provider, &CodingExecutionContext::default())
        .await
        .expect("execute coding");

    let prompt = captured_prompt
        .lock()
        .expect("prompt lock")
        .clone()
        .expect("captured prompt");
    assert!(prompt.contains("本轮补充上下文"));
    assert!(prompt.contains("请优先修复 provider_install SSE 订阅"));
    assert!(!prompt.contains("不要带入本轮 Coder prompt"));
    let notes = store
        .list_context_notes("project_0001", "issue_0001", &attempt.id)
        .expect("context notes");
    let consumed_note = notes
        .iter()
        .find(|note| note.id == new_note.id)
        .expect("new note persisted");
    assert_eq!(consumed_note.consumed_by_rework_round, Some(1));
}

#[tokio::test]
async fn execute_coding_emits_prompt_for_coder_provider() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_role_provider_config_snapshot(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingRoleProviderConfigSnapshot {
                coder: ProviderName::Codex,
                tester: ProviderName::Fake,
                analyst: ProviderName::Fake,
                code_reviewer: ProviderName::Fake,
                internal_reviewer: ProviderName::Fake,
                review_rounds: 1,
                permission_modes: CodingRolePermissionModes::default(),
            },
        )
        .expect("set role provider snapshot");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let captured_input = Arc::new(Mutex::new(None));
    let provider = InputCapturingProvider {
        input: Arc::clone(&captured_input),
        output: "done".to_string(),
    };
    let (tx, mut rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);

    engine
        .execute_coding(&attempt, &provider, &CodingExecutionContext::default())
        .await
        .expect("execute coding");

    let _node = rx.recv().await.expect("coding node created");
    match rx.recv().await.expect("provider prompt event") {
        CodingWsOutMessage::CodingExecutionEvent { event } => {
            assert_eq!(event.agent, Some(ProviderName::Codex));
        }
        other => panic!("expected provider prompt event, got {other:?}"),
    }
    let input = captured_input
        .lock()
        .expect("input lock")
        .clone()
        .expect("captured input");
    assert_eq!(input.provider_type, ProviderType::Codex);
}

#[tokio::test]
async fn execute_coding_forwards_provider_execution_and_tool_events() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_role_provider_config_snapshot(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingRoleProviderConfigSnapshot {
                coder: ProviderName::Codex,
                tester: ProviderName::Fake,
                analyst: ProviderName::Fake,
                code_reviewer: ProviderName::Fake,
                internal_reviewer: ProviderName::Fake,
                review_rounds: 1,
                permission_modes: CodingRolePermissionModes::default(),
            },
        )
        .expect("set role provider snapshot");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let provider = EventEmittingCodingProvider;
    let (tx, mut rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);

    engine
        .execute_coding(&attempt, &provider, &CodingExecutionContext::default())
        .await
        .expect("execute coding");

    let events = drain_events(&mut rx);
    let execution_events = events
        .iter()
        .filter_map(|event| match event {
            CodingWsOutMessage::CodingExecutionEvent { event } => Some(event),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert!(
        execution_events
            .iter()
            .any(|event| event.event_id == "command_0001"
                && event.agent == Some(ProviderName::Codex)
                && event.kind == WsExecutionEventKind::Command
                && event.status == WsExecutionEventStatus::Completed
                && event.title == "Run tests"
                && event.command.as_deref() == Some("uv run pytest")
                && event.output.as_deref() == Some("1 passed")),
        "expected command execution event, got {events:?}"
    );
    assert!(
        execution_events
            .iter()
            .any(|event| event.event_id == "tool_0001"
                && event.kind == WsExecutionEventKind::Command
                && event.status == WsExecutionEventStatus::Started
                && event.title == "run_command"
                && event
                    .detail
                    .as_deref()
                    .is_some_and(|detail| detail.contains("uv run pytest"))),
        "expected tool call execution event, got {events:?}"
    );
    assert!(
        execution_events
            .iter()
            .any(|event| event.event_id == "tool_0001"
                && event.kind == WsExecutionEventKind::Command
                && event.status == WsExecutionEventStatus::Completed
                && event.title == "run_command"
                && event.command.as_deref() == Some("uv run pytest")
                && event.output.as_deref() == Some("1 passed")),
        "expected tool result execution event, got {events:?}"
    );
}

#[tokio::test]
async fn execute_coding_forwards_provider_permission_choice_and_status_events() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let provider = ControlEventCodingProvider;
    let (tx, mut rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);

    let error = engine
        .execute_coding(&attempt, &provider, &CodingExecutionContext::default())
        .await
        .expect_err("unresolved provider choice should block completion");
    assert_eq!(
        error.to_string(),
        "coding_provider_stream_failed: provider_choice_unresolved"
    );

    let events = drain_events(&mut rx);
    assert!(
        events.iter().any(|event| {
            matches!(
                event,
                CodingWsOutMessage::CodingPermissionRequest {
                    id,
                    tool_name,
                    description,
                    ..
                } if id == "permission_0001"
                    && tool_name == "shell"
                    && description == "Run uv test command"
            )
        }),
        "expected permission request event, got {events:?}"
    );
    assert!(
        events.iter().any(|event| {
            matches!(
                event,
                CodingWsOutMessage::CodingChoiceRequest {
                    id,
                    prompt,
                    source,
                    options,
                    allow_multiple,
                    allow_free_text,
                } if id == "choice_0001"
                    && prompt == "Select implementation strategy"
                    && source == "provider_choice"
                    && options.len() == 1
                    && !allow_multiple
                    && *allow_free_text
            )
        }),
        "expected choice request event, got {events:?}"
    );
    assert!(
        events.iter().any(|event| {
            matches!(
                event,
                CodingWsOutMessage::CodingExecutionEvent { event }
                    if event.event_id == "coding_node_0001_provider_status_running"
                        && event.status == WsExecutionEventStatus::Running
                        && event.title == "Provider running"
            )
        }),
        "expected visible provider status event, got {events:?}"
    );
}

#[tokio::test]
async fn execute_coding_forwards_permission_responses_to_provider_session() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let provider = PermissionAwaitingProvider;
    let (event_tx, mut event_rx) = mpsc::channel(16);
    let (command_tx, mut command_rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), event_tx);
    let context = CodingExecutionContext::default();

    let execute =
        engine.execute_coding_with_commands(&attempt, &provider, &context, &mut command_rx);
    tokio::pin!(execute);
    let mut saw_permission_request = false;

    let updated = loop {
        tokio::select! {
            result = &mut execute => break result.expect("execute coding"),
            event = event_rx.recv() => {
                if matches!(
                    event,
                    Some(CodingWsOutMessage::CodingPermissionRequest { ref id, .. })
                        if id == "permission_0001"
                ) {
                    saw_permission_request = true;
                    command_tx
                        .send(cadence_aria::product::coding_workspace_runner::CodingRunnerCommand::PermissionResponse {
                            id: "permission_0001".to_string(),
                            approved: true,
                            reason: Some("approved by test".to_string()),
                        })
                        .await
                        .expect("send permission response");
                }
            }
        }
    };

    assert!(saw_permission_request);
    assert_eq!(updated.stage, CodingExecutionStage::Coding);
}

#[tokio::test]
async fn execute_coding_persists_provider_choice_and_resumes_after_response() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let provider = ChoiceAwaitingProvider;
    let (event_tx, mut event_rx) = mpsc::channel(16);
    let (command_tx, mut command_rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), event_tx);
    let context = CodingExecutionContext::default();

    let execute =
        engine.execute_coding_with_commands(&attempt, &provider, &context, &mut command_rx);
    tokio::pin!(execute);
    let mut saw_choice_request = false;

    let updated = loop {
        tokio::select! {
            result = &mut execute => break result.expect("execute coding"),
            event = event_rx.recv() => {
                if let Some(CodingWsOutMessage::CodingChoiceRequest {
                    id,
                    prompt,
                    source,
                    ..
                }) = event
                    && id == "choice_0001"
                {
                    saw_choice_request = true;
                    assert_eq!(prompt, "Select implementation strategy");
                    assert_eq!(source, "request_user_input");
                    let open = store
                        .list_open_choice_gates("project_0001", "issue_0001", &attempt.id)
                        .expect("open choice gates");
                    assert_eq!(open.len(), 1);
                    assert_eq!(open[0].choice_id, "choice_0001");
                    assert_eq!(open[0].status, CodingChoiceGateStatus::Open);
                    assert_eq!(
                        store
                            .get_attempt("project_0001", "issue_0001", &attempt.id)
                            .expect("attempt")
                            .status,
                        CodingAttemptStatus::WaitingForHuman
                    );
                    command_tx
                        .send(cadence_aria::product::coding_workspace_runner::CodingRunnerCommand::ChoiceResponse {
                            id: "choice_0001".to_string(),
                            selected_option_ids: vec!["backend_first".to_string()],
                            free_text: Some("先控制范围".to_string()),
                        })
                        .await
                        .expect("send choice response");
                }
            }
        }
    };

    assert!(saw_choice_request);
    assert_eq!(updated.stage, CodingExecutionStage::Coding);
    assert_eq!(
        store
            .list_open_choice_gates("project_0001", "issue_0001", &attempt.id)
            .expect("open choice gates")
            .len(),
        0
    );
}

#[tokio::test]
async fn execute_coding_blocks_when_provider_completes_before_choice_response() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let provider = ControlEventCodingProvider;
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    let error = engine
        .execute_coding(&attempt, &provider, &CodingExecutionContext::default())
        .await
        .expect_err("provider cannot complete with unresolved choice");

    assert_eq!(
        error.to_string(),
        "coding_provider_stream_failed: provider_choice_unresolved"
    );
    assert_eq!(
        store
            .get_attempt("project_0001", "issue_0001", &attempt.id)
            .expect("attempt")
            .status,
        CodingAttemptStatus::WaitingForHuman
    );
    assert_eq!(
        store
            .list_open_choice_gates("project_0001", "issue_0001", &attempt.id)
            .expect("open choice gates")
            .len(),
        1
    );
}

#[tokio::test]
async fn execute_coding_blocks_later_permission_before_choice_response() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let provider = ChoiceThenPermissionProvider;
    let (tx, mut rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    let error = engine
        .execute_coding(&attempt, &provider, &CodingExecutionContext::default())
        .await
        .expect_err("provider cannot request permission with unresolved choice");

    assert_eq!(
        error.to_string(),
        "coding_provider_stream_failed: provider_choice_unresolved"
    );
    assert_eq!(
        store
            .get_attempt("project_0001", "issue_0001", &attempt.id)
            .expect("attempt")
            .status,
        CodingAttemptStatus::WaitingForHuman
    );
    let events = drain_events(&mut rx);
    assert!(events.iter().any(|event| {
        matches!(
            event,
            CodingWsOutMessage::CodingChoiceRequest { id, .. } if id == "choice_0001"
        )
    }));
    assert!(
        !events.iter().any(|event| {
            matches!(
                event,
                CodingWsOutMessage::CodingPermissionRequest { id, .. } if id == "permission_0001"
            )
        }),
        "pending choice must block later permission requests"
    );
}

#[tokio::test]
async fn execute_coding_stops_forwarding_provider_events_after_abort_command() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let provider = PermissionAwaitingProvider;
    let (event_tx, mut event_rx) = mpsc::channel(16);
    let (command_tx, mut command_rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), event_tx);
    let context = CodingExecutionContext::default();

    let execute =
        engine.execute_coding_with_commands(&attempt, &provider, &context, &mut command_rx);
    tokio::pin!(execute);
    let mut abort_sent = false;

    let error = loop {
        tokio::select! {
            result = &mut execute => break result.expect_err("abort should stop coding execution"),
            event = event_rx.recv() => {
                if !abort_sent
                    && matches!(
                        event,
                        Some(CodingWsOutMessage::CodingPermissionRequest { ref id, .. })
                            if id == "permission_0001"
                    )
                {
                    abort_sent = true;
                    command_tx
                        .send(cadence_aria::product::coding_workspace_runner::CodingRunnerCommand::AbortAttempt)
                        .await
                        .expect("send abort");
                }
            }
        }
    };

    assert_eq!(error.to_string(), "coding_aborted");
    assert!(abort_sent);
}

#[tokio::test]
async fn execute_code_review_forwards_provider_execution_events() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    init_repo(&worktree);
    fs::write(worktree.join("src.txt"), "hello\nreviewed\n").expect("modify file");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            base_branch: "HEAD".to_string(),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let provider = EventThenCompletedProvider {
        output: r#"{"verdict":"approve","summary":"review ok","findings":[]}"#.to_string(),
    };
    let (tx, mut rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);

    engine
        .execute_code_review(&attempt, &provider)
        .await
        .expect("execute code review");

    assert_provider_command_event(&drain_events(&mut rx));
}

#[tokio::test]
async fn execute_code_review_persists_role_run_events_while_forwarding_realtime_events() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    init_repo(&worktree);
    fs::write(worktree.join("src.txt"), "hello\nreviewed\n").expect("modify file");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            base_branch: "HEAD".to_string(),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let provider = EventThenCompletedProvider {
        output: r#"{"verdict":"approve","summary":"review ok","findings":[]}"#.to_string(),
    };
    let (tx, mut rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    engine
        .execute_code_review(&attempt, &provider)
        .await
        .expect("execute code review");

    assert_provider_command_event(&drain_events(&mut rx));
    let runs = store
        .list_role_runs("project_0001", "issue_0001", &attempt.id)
        .expect("role runs");
    assert_eq!(runs.len(), 1);
    let events = store
        .list_role_run_events("project_0001", "issue_0001", &attempt.id, &runs[0].id)
        .expect("role run events");
    let event_types = events
        .iter()
        .map(|event| event.event_type)
        .collect::<Vec<_>>();
    assert_eq!(
        event_types,
        vec![
            CodingRoleRunEventType::ProviderPrompt,
            CodingRoleRunEventType::ProviderStart,
            CodingRoleRunEventType::ExecutionEvent,
            CodingRoleRunEventType::MessageComplete,
        ]
    );
    assert_eq!(events[2].payload["title"], "Provider command");
    assert_eq!(events[2].payload["output"], "changed files");
}

#[tokio::test]
async fn execute_code_review_persists_provider_control_and_tool_event_payloads() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    init_repo(&worktree);
    fs::write(worktree.join("src.txt"), "hello\nreviewed\n").expect("modify file");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            base_branch: "HEAD".to_string(),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let provider = ReviewControlEventProvider;
    let (tx, mut rx) = mpsc::channel(32);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    engine
        .execute_code_review(&attempt, &provider)
        .await
        .expect("execute code review");

    let realtime_events = drain_events(&mut rx);
    assert!(
        realtime_events.iter().any(|event| {
            matches!(
                event,
                CodingWsOutMessage::CodingPermissionRequest { id, .. }
                    if id == "permission_review_0001"
            )
        }),
        "expected realtime permission request, got {realtime_events:?}"
    );
    let runs = store
        .list_role_runs("project_0001", "issue_0001", &attempt.id)
        .expect("role runs");
    assert_eq!(runs.len(), 1);
    let events = store
        .list_role_run_events("project_0001", "issue_0001", &attempt.id, &runs[0].id)
        .expect("role run events");
    let event_types = events
        .iter()
        .map(|event| event.event_type)
        .collect::<Vec<_>>();
    assert_eq!(
        event_types,
        vec![
            CodingRoleRunEventType::ProviderPrompt,
            CodingRoleRunEventType::ProviderStart,
            CodingRoleRunEventType::TextDelta,
            CodingRoleRunEventType::ExecutionEvent,
            CodingRoleRunEventType::ToolCall,
            CodingRoleRunEventType::ToolResult,
            CodingRoleRunEventType::StatusChanged,
            CodingRoleRunEventType::PermissionRequest,
            CodingRoleRunEventType::MessageComplete,
        ]
    );
    assert_eq!(events[2].payload["content"], "reviewing");
    assert_eq!(events[3].payload["event_id"], "review_command_0001");
    assert_eq!(events[3].payload["kind"], "Command");
    assert_eq!(events[3].payload["status"], "Completed");
    assert_eq!(events[3].payload["title"], "Review command");
    assert_eq!(events[3].payload["output"], "review ok");
    assert_eq!(events[4].payload["id"], "review_tool_0001");
    assert_eq!(events[4].payload["tool_name"], "run_command");
    assert_eq!(events[4].payload["input"]["command"], "cargo test --locked");
    assert_eq!(events[5].payload["tool_use_id"], "review_tool_0001");
    assert_eq!(events[5].payload["output"], "tool ok");
    assert_eq!(events[5].payload["is_error"], false);
    assert_eq!(events[6].payload["status"], "Running");
    assert_eq!(events[7].payload["id"], "permission_review_0001");
    assert_eq!(events[7].payload["tool_name"], "shell");
    assert_eq!(events[7].payload["risk_level"], "High");
    assert_eq!(
        events[8].payload["provider_session_id"],
        "review-session-0001"
    );
}

#[tokio::test]
async fn execute_code_review_records_permission_timeout_as_timeout_event() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    init_repo(&worktree);
    fs::write(worktree.join("src.txt"), "hello\nreviewed\n").expect("modify file");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            base_branch: "HEAD".to_string(),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let provider = ReviewPermissionTimeoutProvider;
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    let error = engine
        .execute_code_review(&attempt, &provider)
        .await
        .expect_err("permission timeout should fail code review");

    assert!(
        error
            .to_string()
            .contains("Permission request permission_review_timeout timed out")
    );
    let runs = store
        .list_role_runs("project_0001", "issue_0001", &attempt.id)
        .expect("role runs");
    assert_eq!(runs.len(), 1);
    let events = store
        .list_role_run_events("project_0001", "issue_0001", &attempt.id, &runs[0].id)
        .expect("role run events");
    let timeout = events
        .iter()
        .find(|event| event.payload["permission_id"] == "permission_review_timeout")
        .expect("permission timeout event");
    assert_eq!(timeout.event_type, CodingRoleRunEventType::Timeout);
    assert_eq!(timeout.payload["reason"], "permission_timeout");
    assert_eq!(
        timeout.payload["message"],
        "Permission request permission_review_timeout timed out"
    );
}

#[tokio::test]
async fn execute_rework_forwards_provider_execution_events() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    store
        .update_attempt_stage(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingExecutionStage::Testing,
        )
        .expect("testing stage");
    let provider = EventThenCompletedProvider {
        output: r#"{"verdict":"no_issue","summary":"testing ok"}"#.to_string(),
    };
    let (tx, mut rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);

    engine
        .execute_rework(&attempt, "testing evidence", &provider)
        .await
        .expect("execute rework");

    assert_provider_command_event(&drain_events(&mut rx));
}

#[tokio::test]
async fn execute_internal_pr_review_forwards_provider_execution_events() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    init_repo(&worktree);
    fs::write(worktree.join("src.txt"), "hello\nreviewed\n").expect("modify file");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            base_branch: "HEAD".to_string(),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    store
        .save_review_request(&sample_review_request(&attempt.id))
        .expect("save review request");
    let provider = EventThenCompletedProvider {
        output: r#"{"verdict":"approve","summary":"internal review ok","findings":[],"impact_scope":["src"],"pr_description":"实现 work item","commit_message_suggestion":"feat: implement work item"}"#.to_string(),
    };
    let (tx, mut rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);

    engine
        .execute_internal_pr_review(&attempt, &provider)
        .await
        .expect("execute internal review");

    assert_provider_command_event(&drain_events(&mut rx));
}

#[tokio::test]
async fn execute_testing_runs_commands_persists_report_and_emits_update() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree.clone()),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let (tx, mut rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let specs = vec![TestCommandSpec {
        id: "unit".to_string(),
        command: vec!["sh".to_string(), "-c".to_string(), "printf ok".to_string()],
    }];

    let report = engine
        .execute_testing(&attempt, &specs)
        .await
        .expect("execute testing");

    assert_eq!(report.overall_status, TestingOverallStatus::Passed);
    assert_eq!(report.commands.len(), 1);
    assert_eq!(
        fs::read_to_string(
            store
                .attempt_test_output_root("project_0001", "issue_0001", &attempt.id)
                .join(&report.commands[0].stdout_ref)
        )
        .expect("stdout"),
        "ok"
    );
    let reports = store
        .list_testing_reports("project_0001", "issue_0001", &attempt.id)
        .expect("testing reports");
    assert_eq!(reports, vec![report.clone()]);
    let updated = store
        .get_attempt("project_0001", "issue_0001", &attempt.id)
        .expect("updated attempt");
    assert_eq!(updated.stage, CodingExecutionStage::Testing);

    match rx.recv().await.expect("node created") {
        CodingWsOutMessage::CodingTimelineNodeCreated { node } => {
            assert_eq!(node.stage, CodingExecutionStage::Testing);
            assert_eq!(node.status, CodingTimelineNodeStatus::Running);
        }
        other => panic!("expected testing node created, got {other:?}"),
    }
    match rx.recv().await.expect("testing update") {
        CodingWsOutMessage::TestingReportUpdate {
            report: event_report,
        } => {
            assert_eq!(event_report.id, report.id);
            assert_eq!(event_report.overall_status, TestingOverallStatus::Passed);
        }
        other => panic!("expected testing report update, got {other:?}"),
    }
}

#[tokio::test]
async fn execute_testing_keeps_attempt_running_when_no_commands_are_available_for_analyst() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    let report = engine
        .execute_testing(&attempt, &[])
        .await
        .expect("execute testing");

    assert_eq!(report.overall_status, TestingOverallStatus::Blocked);
    let updated = store
        .get_attempt("project_0001", "issue_0001", &attempt.id)
        .expect("updated attempt");
    assert_eq!(updated.status, CodingAttemptStatus::Running);
    assert_eq!(updated.stage, CodingExecutionStage::Testing);
}

#[tokio::test]
async fn execute_code_review_persists_report_and_emits_review_events() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    init_repo(&worktree);
    fs::write(worktree.join("src.txt"), "hello\nreviewed\n").expect("modify file");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            base_branch: "HEAD".to_string(),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let (tx, mut rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = ReviewStreamingProvider;

    let report = engine
        .execute_code_review(&attempt, &provider)
        .await
        .expect("execute code review");

    assert_eq!(report.id, "code_review_0001");
    assert_eq!(report.attempt_id, attempt.id);
    assert_eq!(report.round, 1);
    assert_eq!(report.verdict, ReviewVerdict::Approve);
    assert_eq!(report.summary, "review ok");
    let persisted = store
        .list_code_review_reports("project_0001", "issue_0001", &attempt.id)
        .expect("code review reports");
    assert_eq!(persisted, vec![report.clone()]);
    let updated = store
        .get_attempt("project_0001", "issue_0001", &attempt.id)
        .expect("updated attempt");
    assert_eq!(updated.stage, CodingExecutionStage::CodeReview);

    match rx.recv().await.expect("code review node created") {
        CodingWsOutMessage::CodingTimelineNodeCreated { node } => {
            assert_eq!(node.id, "coding_node_0001");
            assert_eq!(node.stage, CodingExecutionStage::CodeReview);
            assert_eq!(node.status, CodingTimelineNodeStatus::Running);
        }
        other => panic!("expected code review node created, got {other:?}"),
    }
    match rx.recv().await.expect("code review provider prompt") {
        CodingWsOutMessage::CodingExecutionEvent { event } => {
            assert_eq!(event.event_id, "coding_node_0001_prompt");
            assert_eq!(event.node_id.as_deref(), Some("coding_node_0001"));
            assert_eq!(event.title, "Provider Prompt");
            assert!(
                event
                    .output
                    .as_deref()
                    .is_some_and(|output| output.contains("CodeReviewer"))
            );
        }
        other => panic!("expected code review provider prompt, got {other:?}"),
    }
    assert_eq!(
        rx.recv().await.expect("code review stream chunk"),
        CodingWsOutMessage::CodingStreamChunk {
            content: "reviewing diff".to_string(),
            node_id: Some("coding_node_0001".to_string()),
        }
    );
    assert_eq!(
        rx.recv().await.expect("code review message complete"),
        CodingWsOutMessage::CodingMessageComplete {
            node_id: Some("coding_node_0001".to_string()),
        }
    );
    match rx.recv().await.expect("code review chat entry") {
        CodingWsOutMessage::CodingChatEntryCreated { entry } => {
            assert_eq!(entry.node_id.as_deref(), Some("coding_node_0001"));
            assert_eq!(entry.role, CodingAgentRole::Reviewer);
            assert_eq!(entry.entry_type, CodingEntryType::AssistantMessage);
            assert_eq!(entry.content.as_deref(), Some("review ok"));
            assert_eq!(
                entry
                    .metadata
                    .as_ref()
                    .and_then(|value| value.get("review_id"))
                    .and_then(|value| value.as_str()),
                Some("code_review_0001")
            );
        }
        other => panic!("expected code review chat entry, got {other:?}"),
    }
    match rx.recv().await.expect("code review complete") {
        CodingWsOutMessage::CodeReviewComplete {
            report: event_report,
        } => {
            assert_eq!(event_report.id, "code_review_0001");
            assert_eq!(event_report.verdict, ReviewVerdict::Approve);
        }
        other => panic!("expected code review complete, got {other:?}"),
    }
    match rx.recv().await.expect("code review node completed") {
        CodingWsOutMessage::CodingTimelineNodeUpdated {
            node_id,
            status,
            summary,
            completed_at,
        } => {
            assert_eq!(node_id, "coding_node_0001");
            assert_eq!(status, CodingTimelineNodeStatus::Completed);
            assert_eq!(summary.as_deref(), Some("code review 通过"));
            assert!(completed_at.is_some());
        }
        other => panic!("expected code review node completed, got {other:?}"),
    }
}

#[tokio::test]
async fn parses_real_provider_review_finding_aliases() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    init_repo(&worktree);
    fs::write(worktree.join("src.txt"), "hello\nreviewed\n").expect("modify file");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            base_branch: "HEAD".to_string(),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);
    let provider = InputCapturingProvider {
        input: Arc::new(Mutex::new(None)),
        output: r#"{
            "verdict": "request_changes",
            "summary": "范围污染",
            "findings": [
                {
                    "severity": "blocking",
                    "file": "__pycache__/x.pyc",
                    "description": "不应提交运行产物",
                    "recommendation": "从提交中移除 pyc 文件",
                    "title": "运行产物进入提交"
                }
            ]
        }"#
        .to_string(),
    };

    let report = engine
        .execute_code_review(&attempt, &provider)
        .await
        .expect("execute code review");

    assert_eq!(report.verdict, ReviewVerdict::RequestChanges);
    assert_eq!(report.findings.len(), 1);
    let finding = &report.findings[0];
    assert_eq!(finding.severity, FindingSeverity::Error);
    assert_eq!(finding.file_path.as_deref(), Some("__pycache__/x.pyc"));
    assert_eq!(finding.message, "不应提交运行产物");
    assert_eq!(
        finding.required_action.as_deref(),
        Some("从提交中移除 pyc 文件")
    );
    assert_eq!(finding.source_stage, CodingExecutionStage::CodeReview);
}

#[tokio::test]
async fn review_payload_parse_failure_records_blocked_evidence_for_analyst() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    init_repo(&worktree);
    fs::write(worktree.join("src.txt"), "hello\nreviewed\n").expect("modify file");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            base_branch: "HEAD".to_string(),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = InputCapturingProvider {
        input: Arc::new(Mutex::new(None)),
        output: "review output without valid json".to_string(),
    };

    let report = engine
        .execute_code_review(&attempt, &provider)
        .await
        .expect("execute code review");

    assert_eq!(report.verdict, ReviewVerdict::Blocked);
    assert!(report.summary.contains("review 输出不是有效 JSON"));
    let updated = store
        .get_attempt("project_0001", "issue_0001", &attempt.id)
        .expect("updated attempt");
    assert_eq!(updated.status, CodingAttemptStatus::Running);
    assert_eq!(updated.stage, CodingExecutionStage::CodeReview);
    let gates = store
        .list_open_blocked_gates("project_0001", "issue_0001", &attempt.id)
        .expect("open blocked gates");
    assert!(gates.is_empty());
}

#[tokio::test]
async fn execute_code_review_blocked_keeps_attempt_running_for_analyst() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    init_repo(&worktree);
    fs::write(worktree.join("src.txt"), "hello\nreviewed\n").expect("modify file");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            base_branch: "HEAD".to_string(),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = InputCapturingProvider {
        input: Arc::new(Mutex::new(None)),
        output: r#"{
            "verdict": "blocked",
            "summary": "缺少人工测试账号，无法完成 review",
            "findings": []
        }"#
        .to_string(),
    };

    let report = engine
        .execute_code_review(&attempt, &provider)
        .await
        .expect("execute code review");

    assert_eq!(report.verdict, ReviewVerdict::Blocked);
    assert_eq!(report.summary, "缺少人工测试账号，无法完成 review");
    let updated = store
        .get_attempt("project_0001", "issue_0001", &attempt.id)
        .expect("updated attempt");
    assert_eq!(updated.status, CodingAttemptStatus::Running);
    assert_eq!(updated.stage, CodingExecutionStage::CodeReview);
    let gates = store
        .list_open_blocked_gates("project_0001", "issue_0001", &attempt.id)
        .expect("open blocked gates");
    assert!(gates.is_empty());
}

#[tokio::test]
async fn code_review_provider_start_failure_marks_attempt_blocked_and_node_failed() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    init_repo(&worktree);
    fs::write(worktree.join("src.txt"), "hello\nreviewed\n").expect("modify file");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            base_branch: "HEAD".to_string(),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = StartFailingProvider;

    let error = engine
        .execute_code_review(&attempt, &provider)
        .await
        .expect_err("provider start should fail");

    assert!(error.to_string().contains("provider failed to start"));
    let updated = store
        .get_attempt("project_0001", "issue_0001", &attempt.id)
        .expect("updated attempt");
    assert_eq!(updated.status, CodingAttemptStatus::Failed);
    assert_eq!(updated.stage, CodingExecutionStage::CodeReview);
    let nodes = store
        .get_timeline_nodes("project_0001", "issue_0001", &attempt.id)
        .expect("timeline nodes");
    let node = nodes.last().expect("code review node");
    assert_eq!(node.stage, CodingExecutionStage::CodeReview);
    assert_eq!(node.status, CodingTimelineNodeStatus::Failed);
    assert_eq!(node.summary.as_deref(), Some("provider failed to start"));
    assert!(node.completed_at.is_some());
}

#[tokio::test]
async fn execute_code_review_prompt_includes_diff_work_item_rules_and_role_provider() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    init_repo(&worktree);
    fs::write(worktree.join("src.txt"), "hello\nstairs implementation\n").expect("modify file");
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    seed_work_item_markdown(
        &app_paths,
        "实现爬楼梯问题：给定 n 阶楼梯，每次可以爬 1 或 2 阶。",
    );
    let store = CodingAttemptStore::new(app_paths);
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            base_branch: "HEAD".to_string(),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_role_provider_config_snapshot(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingRoleProviderConfigSnapshot {
                coder: ProviderName::Fake,
                tester: ProviderName::Fake,
                analyst: ProviderName::Fake,
                code_reviewer: ProviderName::Codex,
                internal_reviewer: ProviderName::Fake,
                review_rounds: 1,
                permission_modes: CodingRolePermissionModes::default(),
            },
        )
        .expect("set role provider snapshot");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);
    let captured_input = Arc::new(Mutex::new(None));
    let provider = InputCapturingProvider {
        input: captured_input.clone(),
        output: r#"{"verdict":"approve","summary":"review ok","findings":[]}"#.to_string(),
    };

    engine
        .execute_code_review(&attempt, &provider)
        .await
        .expect("execute code review");

    let input = captured_input
        .lock()
        .expect("input lock")
        .clone()
        .expect("captured input");
    assert_eq!(input.provider_type, ProviderType::Codex);
    assert_eq!(input.role, AdapterRole::Reviewer);
    assert_eq!(input.output_schema, "coding_workspace_code_review_json");
    assert!(input.prompt.contains("CodeReviewer"));
    assert!(input.prompt.contains("git diff"));
    assert!(input.prompt.contains("+stairs implementation"));
    assert!(input.prompt.contains("实现爬楼梯问题"));
    assert!(input.prompt.contains("代码规范"));
}

#[tokio::test]
async fn execute_rework_needs_fix_uses_analyst_prompt_consumes_notes_and_routes_to_coding() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree.clone()),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    store
        .update_attempt_stage(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingExecutionStage::Testing,
        )
        .expect("testing stage");
    let old_note = store
        .create_context_note(&attempt.id, "不要出现在 prompt".to_string())
        .expect("old context note");
    store
        .mark_context_notes_consumed("project_0001", "issue_0001", &attempt.id, &[old_note.id], 1)
        .expect("consume old note");
    let new_note = store
        .create_context_note(&attempt.id, "请补充 n=10 的测试".to_string())
        .expect("new context note");
    let (tx, mut rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let captured_prompt = Arc::new(Mutex::new(None));
    let provider = AnalystStreamingProvider {
        prompt: captured_prompt.clone(),
        output: r#"{"verdict":"needs_fix","summary":"测试仍失败","fix_hints":["补充 climb_stairs 动态规划实现"]}"#.to_string(),
    };

    let updated = engine
        .execute_rework(&attempt, "测试失败: unit failed", &provider)
        .await
        .expect("execute rework");

    assert_eq!(updated.status, CodingAttemptStatus::Running);
    assert_eq!(updated.stage, CodingExecutionStage::Coding);
    assert_eq!(updated.rework_count, 1);
    assert!(!worktree.join("reworked.txt").exists());
    let prompt = captured_prompt
        .lock()
        .expect("prompt lock")
        .clone()
        .expect("captured prompt");
    assert!(prompt.contains("Rework 分析官"));
    assert!(prompt.contains("只做分析和路由决策"));
    assert!(prompt.contains("不要修改代码"));
    assert!(prompt.contains("测试失败: unit failed"));
    assert!(prompt.contains("请补充 n=10 的测试"));
    assert!(!prompt.contains("不要出现在 prompt"));
    let notes = store
        .list_context_notes("project_0001", "issue_0001", &attempt.id)
        .expect("notes");
    assert_eq!(notes[1].id, new_note.id);
    assert_eq!(notes[1].consumed_by_rework_round, Some(1));
    let nodes = store
        .get_timeline_nodes("project_0001", "issue_0001", &attempt.id)
        .expect("timeline nodes");
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].stage, CodingExecutionStage::Rework);
    assert_eq!(nodes[0].title, "分析官判定 #1");
    assert_eq!(nodes[0].status, CodingTimelineNodeStatus::Completed);
    assert_eq!(nodes[0].summary.as_deref(), Some("NeedsFix: 测试仍失败"));
    let instruction = store
        .latest_unconsumed_rework_instruction("project_0001", "issue_0001", &attempt.id)
        .expect("latest rework instruction")
        .expect("rework instruction");
    assert_eq!(instruction.source_stage, CodingExecutionStage::Testing);
    assert_eq!(instruction.rework_round, 1);
    assert_eq!(instruction.summary, "测试仍失败");
    assert_eq!(
        instruction.fix_hints,
        vec!["补充 climb_stairs 动态规划实现".to_string()]
    );

    let events = drain_events(&mut rx);
    assert!(events.iter().any(|event| {
        matches!(
            event,
            CodingWsOutMessage::CodingTimelineNodeCreated { node }
                if node.id == "coding_node_0001"
                    && node.stage == CodingExecutionStage::Rework
                    && node.title == "分析官判定 #1"
                    && node.status == CodingTimelineNodeStatus::Running
        )
    }));
    assert!(events.iter().any(|event| {
        matches!(
            event,
            CodingWsOutMessage::CodingMessageComplete {
                node_id: Some(node_id)
            } if node_id == "coding_node_0001"
        )
    }));
    assert!(events.iter().any(|event| {
        matches!(
            event,
            CodingWsOutMessage::CodingChatEntryCreated { entry }
                if matches!(
                    &entry.entry_type,
                    CodingEntryType::AnalystVerdict {
                        verdict: AnalystVerdict::NeedsFix
                    }
                ) && entry.content.as_deref() == Some("测试仍失败")
                    && entry
                        .metadata
                        .as_ref()
                        .and_then(|metadata| metadata.get("fix_hints"))
                        .and_then(|value| value.as_array())
                        .and_then(|items| items.first())
                        .and_then(|value| value.as_str())
                        == Some("补充 climb_stairs 动态规划实现")
        )
    }));
    assert!(events.iter().any(|event| {
        matches!(
            event,
            CodingWsOutMessage::CodingTimelineNodeUpdated {
                node_id,
                status: CodingTimelineNodeStatus::Completed,
                summary: Some(summary),
                completed_at: Some(_),
            } if node_id == "coding_node_0001" && summary == "NeedsFix: 测试仍失败"
        )
    }));
}

#[tokio::test]
async fn execute_rework_persists_structured_analyst_decision() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    store
        .update_attempt_stage(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingExecutionStage::Testing,
        )
        .expect("testing stage");
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = AnalystStreamingProvider {
        prompt: Arc::new(Mutex::new(None)),
        output: r#"{
            "verdict":"needs_fix",
            "next_stage":"coding",
            "reason":"required 测试步骤被跳过",
            "evidence_refs":["testing_report_0001.json"],
            "raw_provider_output_refs":["provider-raw/testing/execute_test_plan_0001.txt"],
            "rework_instructions":{
                "summary":"补齐 required 测试覆盖",
                "required_changes":["补充 B6 浏览器测试"],
                "verification_expectations":["B6 不再出现在 skipped_required_steps"]
            },
            "human_gate":null
        }"#
        .to_string(),
    };

    let updated = engine
        .execute_rework(&attempt, "testing blocked", &provider)
        .await
        .expect("execute rework");

    assert_eq!(updated.stage, CodingExecutionStage::Coding);
    let decision = store
        .latest_analyst_decision("project_0001", "issue_0001", &attempt.id)
        .expect("latest decision")
        .expect("persisted decision");
    assert_eq!(decision.id, "analyst_decision_0001");
    assert_eq!(decision.source_stage, CodingExecutionStage::Testing);
    assert_eq!(decision.rework_round, 1);
    assert_eq!(decision.verdict, AnalystDecisionVerdict::NeedsFix);
    assert_eq!(decision.next_stage, AnalystDecisionNextStage::Coding);
    assert_eq!(decision.reason, "required 测试步骤被跳过");
    assert_eq!(
        decision.evidence_refs,
        vec!["testing_report_0001.json".to_string()]
    );
    assert_eq!(
        decision.raw_provider_output_refs,
        vec!["provider-raw/testing/execute_test_plan_0001.txt".to_string()]
    );
    let rework = decision.rework_instructions.expect("rework instructions");
    assert_eq!(rework.summary, "补齐 required 测试覆盖");
    assert_eq!(
        rework.required_changes,
        vec!["补充 B6 浏览器测试".to_string()]
    );
    assert_eq!(
        rework.verification_expectations,
        vec!["B6 不再出现在 skipped_required_steps".to_string()]
    );
    assert_eq!(decision.parse_error, None);
}

#[tokio::test]
async fn execute_rework_normalizes_string_rework_instructions() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    store
        .update_attempt_stage(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingExecutionStage::Testing,
        )
        .expect("testing stage");
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = AnalystStreamingProvider {
        prompt: Arc::new(Mutex::new(None)),
        output: r#"{
            "verdict":"needs_fix",
            "next_stage":"coding",
            "reason":"仍有静默 Codex 默认",
            "evidence_refs":["testing_report_0001.steps.step_003_search_anchors"],
            "raw_provider_output_refs":["provider-raw/testing/execute_test_plan_0001.txt"],
            "rework_instructions":"修复 src/web/handlers.rs 和 src/web/runtime.rs 中残留的 codex 默认，并补充回归测试。",
            "human_gate":null
        }"#
        .to_string(),
    };

    let updated = engine
        .execute_rework(&attempt, "testing blocked", &provider)
        .await
        .expect("execute rework");

    assert_eq!(updated.stage, CodingExecutionStage::Coding);
    let decision = store
        .latest_analyst_decision("project_0001", "issue_0001", &attempt.id)
        .expect("latest decision")
        .expect("persisted decision");
    assert_eq!(decision.verdict, AnalystDecisionVerdict::NeedsFix);
    assert_eq!(decision.next_stage, AnalystDecisionNextStage::Coding);
    assert_eq!(decision.parse_error, None);
    let rework = decision.rework_instructions.expect("rework instructions");
    assert_eq!(
        rework.summary,
        "修复 src/web/handlers.rs 和 src/web/runtime.rs 中残留的 codex 默认，并补充回归测试。"
    );
    assert_eq!(
        rework.required_changes,
        vec![
            "修复 src/web/handlers.rs 和 src/web/runtime.rs 中残留的 codex 默认，并补充回归测试。"
                .to_string()
        ]
    );
    assert!(rework.verification_expectations.is_empty());
}

#[tokio::test]
async fn execute_rework_persists_legacy_analyst_verdict_as_decision() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    store
        .update_attempt_stage(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingExecutionStage::CodeReview,
        )
        .expect("code review stage");
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = AnalystStreamingProvider {
        prompt: Arc::new(Mutex::new(None)),
        output: r#"{"verdict":"no_issue","summary":"审查通过"}"#.to_string(),
    };

    let updated = engine
        .execute_rework(&attempt, "code review approve", &provider)
        .await
        .expect("execute rework");

    assert_eq!(updated.stage, CodingExecutionStage::ReviewRequest);
    let decision = store
        .latest_analyst_decision("project_0001", "issue_0001", &attempt.id)
        .expect("latest decision")
        .expect("persisted decision");
    assert_eq!(decision.verdict, AnalystDecisionVerdict::Proceed);
    assert_eq!(decision.next_stage, AnalystDecisionNextStage::ReviewRequest);
    assert_eq!(decision.reason, "审查通过");
    assert_eq!(decision.rework_instructions, None);
    assert_eq!(decision.human_gate, None);
}

#[tokio::test]
async fn execute_rework_consumes_next_stage_testing_without_coding_rework() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    store
        .update_attempt_stage(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingExecutionStage::Testing,
        )
        .expect("testing stage");
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = AnalystStreamingProvider {
        prompt: Arc::new(Mutex::new(None)),
        output: r#"{
            "verdict":"rerun_testing",
            "next_stage":"testing",
            "reason":"Tester evidence is incomplete; rerun required browser steps",
            "evidence_refs":["testing_report_0001.json"]
        }"#
        .to_string(),
    };

    let updated = engine
        .execute_rework(&attempt, "testing evidence incomplete", &provider)
        .await
        .expect("execute rework");

    assert_eq!(updated.status, CodingAttemptStatus::Running);
    assert_eq!(updated.stage, CodingExecutionStage::Testing);
    assert_eq!(updated.rework_count, 0);
    assert!(
        store
            .latest_unconsumed_rework_instruction("project_0001", "issue_0001", &attempt.id)
            .expect("latest rework instruction")
            .is_none()
    );
    let decision = store
        .latest_analyst_decision("project_0001", "issue_0001", &attempt.id)
        .expect("latest decision")
        .expect("persisted decision");
    assert_eq!(decision.verdict, AnalystDecisionVerdict::RerunTesting);
    assert_eq!(decision.next_stage, AnalystDecisionNextStage::Testing);
}

#[tokio::test]
async fn execute_rework_consumes_next_stage_code_review() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    store
        .update_attempt_stage(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingExecutionStage::CodeReview,
        )
        .expect("code review stage");
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = AnalystStreamingProvider {
        prompt: Arc::new(Mutex::new(None)),
        output: r#"{
            "verdict":"proceed",
            "next_stage":"code_review",
            "reason":"Run CodeReviewer again after context-only clarification"
        }"#
        .to_string(),
    };

    let updated = engine
        .execute_rework(&attempt, "review clarification", &provider)
        .await
        .expect("execute rework");

    assert_eq!(updated.status, CodingAttemptStatus::Running);
    assert_eq!(updated.stage, CodingExecutionStage::CodeReview);
    let decision = store
        .latest_analyst_decision("project_0001", "issue_0001", &attempt.id)
        .expect("latest decision")
        .expect("persisted decision");
    assert_eq!(decision.verdict, AnalystDecisionVerdict::Proceed);
    assert_eq!(decision.next_stage, AnalystDecisionNextStage::CodeReview);
}

#[tokio::test]
async fn execute_rework_consumes_next_stage_human_gate() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    store
        .update_attempt_stage(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingExecutionStage::Testing,
        )
        .expect("testing stage");
    let (tx, mut rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = AnalystStreamingProvider {
        prompt: Arc::new(Mutex::new(None)),
        output: r#"{
            "verdict":"human_required",
            "next_stage":"human_gate",
            "reason":"External browser credentials are required",
            "evidence_refs":["testing_report_0001.json"],
            "human_gate":{
                "reason_code":"external_browser_required",
                "available_actions":["provide_context","manual_continue"]
            }
        }"#
        .to_string(),
    };

    let updated = engine
        .execute_rework(&attempt, "testing blocked", &provider)
        .await
        .expect("execute rework");

    assert_eq!(updated.status, CodingAttemptStatus::Blocked);
    assert_eq!(updated.stage, CodingExecutionStage::Rework);
    let gates = store
        .list_open_blocked_gates("project_0001", "issue_0001", &attempt.id)
        .expect("open blocked gates");
    assert_eq!(gates.len(), 1);
    assert_eq!(gates[0].stage, Some(CodingExecutionStage::Rework));
    assert_eq!(gates[0].role, Some(CodingProviderRole::Analyst));
    assert_eq!(
        gates[0].reason_code.as_deref(),
        Some("external_browser_required")
    );
    assert_eq!(gates[0].evidence_refs, vec!["testing_report_0001.json"]);
    assert_eq!(
        gates[0]
            .available_actions
            .iter()
            .map(|action| action.action_id.as_str())
            .collect::<Vec<_>>(),
        vec!["provide_context", "manual_continue"]
    );
    let events = drain_events(&mut rx);
    assert!(events.iter().any(|event| {
        matches!(
            event,
            CodingWsOutMessage::CodingGateRequired { gate }
                if gate.stage.as_ref() == Some(&CodingExecutionStage::Rework)
                    && gate.role == Some(CodingProviderRole::Analyst)
        )
    }));
}

#[tokio::test]
async fn execute_rework_needs_fix_at_limit_opens_human_gate_with_warning() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    store
        .update_attempt_stage(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingExecutionStage::Testing,
        )
        .expect("testing stage");
    store
        .increment_attempt_rework_count("project_0001", "issue_0001", &attempt.id)
        .expect("first rewrite");
    store
        .increment_attempt_rework_count("project_0001", "issue_0001", &attempt.id)
        .expect("second rewrite");
    let (tx, mut rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = AnalystStreamingProvider {
        prompt: Arc::new(Mutex::new(None)),
        output: r#"{"verdict":"needs_fix","summary":"仍有失败","fix_hints":["需要人工承担风险"]}"#
            .to_string(),
    };

    let updated = engine
        .execute_rework(&attempt, "测试失败", &provider)
        .await
        .expect("execute rework");

    assert_eq!(updated.status, CodingAttemptStatus::Blocked);
    assert_eq!(updated.stage, CodingExecutionStage::Rework);
    assert_eq!(updated.rework_count, 2);
    let gates = store
        .list_open_blocked_gates("project_0001", "issue_0001", &attempt.id)
        .expect("open blocked gates");
    assert_eq!(gates.len(), 1);
    assert_eq!(gates[0].stage, Some(CodingExecutionStage::Rework));
    assert_eq!(gates[0].role, Some(CodingProviderRole::Analyst));
    assert_eq!(
        gates[0].reason_code.as_deref(),
        Some("max_auto_rework_exceeded")
    );
    assert_eq!(
        gates[0]
            .available_actions
            .iter()
            .map(|action| action.action_id.as_str())
            .collect::<Vec<_>>(),
        vec![
            "continue_rework",
            "provide_context",
            "manual_continue",
            "abort",
        ]
    );
    let events = drain_events(&mut rx);
    assert!(events.iter().any(|event| {
        matches!(
            event,
            CodingWsOutMessage::CodingChatEntryCreated { entry }
                if matches!(
                    &entry.entry_type,
                    CodingEntryType::SystemEvent { event_type, .. }
                    if event_type == "exceeded_rewrite_limit"
                )
        )
    }));
    assert!(events.iter().any(|event| {
        matches!(
            event,
            CodingWsOutMessage::CodingGateRequired { gate }
                if gate.stage.as_ref() == Some(&CodingExecutionStage::Rework)
                    && gate.role == Some(CodingProviderRole::Analyst)
                    && gate.reason_code.as_deref() == Some("max_auto_rework_exceeded")
        )
    }));
}

#[tokio::test]
async fn execute_rework_needs_human_input_opens_human_gate() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    store
        .update_attempt_stage(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingExecutionStage::Testing,
        )
        .expect("testing stage");
    let (tx, mut rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = AnalystStreamingProvider {
        prompt: Arc::new(Mutex::new(None)),
        output: r#"{"verdict":"needs_human_input","questions":["n 的范围是多少？"]}"#.to_string(),
    };

    let updated = engine
        .execute_rework(&attempt, "需求不明确", &provider)
        .await
        .expect("execute rework");

    assert_eq!(updated.status, CodingAttemptStatus::Blocked);
    assert_eq!(updated.stage, CodingExecutionStage::Rework);
    let gates = store
        .list_open_blocked_gates("project_0001", "issue_0001", &attempt.id)
        .expect("open blocked gates");
    assert_eq!(gates.len(), 1);
    assert_eq!(gates[0].stage, Some(CodingExecutionStage::Rework));
    assert_eq!(gates[0].role, Some(CodingProviderRole::Analyst));
    assert_eq!(gates[0].reason_code.as_deref(), Some("analyst_human_gate"));
    let events = drain_events(&mut rx);
    assert!(events.iter().any(|event| {
        matches!(
            event,
            CodingWsOutMessage::CodingChatEntryCreated { entry }
                if matches!(
                    &entry.entry_type,
                    CodingEntryType::AnalystVerdict {
                        verdict: AnalystVerdict::NeedsHumanInput
                    }
                ) && entry
                    .metadata
                    .as_ref()
                    .and_then(|metadata| metadata.get("questions"))
                    .and_then(|value| value.as_array())
                    .and_then(|items| items.first())
                    .and_then(|value| value.as_str())
                    == Some("n 的范围是多少？")
        )
    }));
}

#[tokio::test]
async fn execute_rework_no_issue_routes_by_previous_stage() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let testing_attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree.clone()),
            ..create_input()
        })
        .expect("create testing attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &testing_attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("testing running");
    store
        .update_attempt_stage(
            "project_0001",
            "issue_0001",
            &testing_attempt.id,
            CodingExecutionStage::Testing,
        )
        .expect("testing stage");
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = AnalystStreamingProvider {
        prompt: Arc::new(Mutex::new(None)),
        output: r#"{"verdict":"no_issue","summary":"测试通过"}"#.to_string(),
    };

    let updated = engine
        .execute_rework(&testing_attempt, "测试通过", &provider)
        .await
        .expect("testing rework");
    assert_eq!(updated.stage, CodingExecutionStage::CodeReview);

    let review_attempt = store
        .create_attempt(CreateCodingAttemptInput {
            work_item_id: "work_item_0002".to_string(),
            branch_name: "aria/work-items/work_item_0002/attempt-1".to_string(),
            worktree_path: Some(worktree),
            ..create_input()
        })
        .expect("create review attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &review_attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("review running");
    store
        .update_attempt_stage(
            "project_0001",
            "issue_0001",
            &review_attempt.id,
            CodingExecutionStage::CodeReview,
        )
        .expect("review stage");
    let provider = AnalystStreamingProvider {
        prompt: Arc::new(Mutex::new(None)),
        output: r#"{"verdict":"no_issue","summary":"审查通过"}"#.to_string(),
    };

    let updated = engine
        .execute_rework(&review_attempt, "审查通过", &provider)
        .await
        .expect("review rework");
    assert_eq!(updated.stage, CodingExecutionStage::ReviewRequest);
}

#[tokio::test]
async fn execute_rework_no_issue_after_internal_review_completes_attempt() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    seed_work_item_markdown(&app_paths, "最终检查后可以完成。");
    let store = CodingAttemptStore::new(app_paths);
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    store
        .update_attempt_stage(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingExecutionStage::InternalPrReview,
        )
        .expect("internal review stage");
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = AnalystStreamingProvider {
        prompt: Arc::new(Mutex::new(None)),
        output: r#"{"verdict":"no_issue","summary":"最终审查通过"}"#.to_string(),
    };

    let updated = engine
        .execute_rework(&attempt, "internal review ok", &provider)
        .await
        .expect("final rework");

    assert_eq!(updated.status, CodingAttemptStatus::Completed);
    assert_eq!(updated.stage, CodingExecutionStage::FinalConfirm);
    assert!(updated.completed_at.is_some());
    let nodes = store
        .get_timeline_nodes("project_0001", "issue_0001", &attempt.id)
        .expect("timeline nodes");
    assert_eq!(nodes.len(), 2);
    assert_eq!(nodes[0].stage, CodingExecutionStage::Rework);
    assert_eq!(nodes[0].status, CodingTimelineNodeStatus::Completed);
    assert_eq!(nodes[1].stage, CodingExecutionStage::FinalConfirm);
    assert_eq!(nodes[1].status, CodingTimelineNodeStatus::Completed);
    assert_eq!(
        nodes[1].summary.as_deref(),
        Some("Analyst 最终判定通过，attempt 已完成")
    );
}

#[tokio::test]
async fn execute_rework_invalid_json_falls_back_to_human_gate() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    store
        .update_attempt_stage(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingExecutionStage::Testing,
        )
        .expect("testing stage");
    let (tx, mut rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = AnalystStreamingProvider {
        prompt: Arc::new(Mutex::new(None)),
        output: "不是 JSON".to_string(),
    };

    let updated = engine
        .execute_rework(&attempt, "分析失败", &provider)
        .await
        .expect("execute rework");

    assert_eq!(updated.status, CodingAttemptStatus::Blocked);
    assert_eq!(updated.stage, CodingExecutionStage::Rework);
    let gates = store
        .list_open_blocked_gates("project_0001", "issue_0001", &attempt.id)
        .expect("open blocked gates");
    assert_eq!(gates.len(), 1);
    assert_eq!(gates[0].stage, Some(CodingExecutionStage::Rework));
    assert_eq!(gates[0].role, Some(CodingProviderRole::Analyst));
    assert_eq!(gates[0].reason_code.as_deref(), Some("analyst_human_gate"));
    assert_eq!(
        gates[0].raw_provider_output_ref.as_deref(),
        Some("provider-raw/rework/analyst_decision_0001.txt")
    );
    let decision = store
        .latest_analyst_decision("project_0001", "issue_0001", &attempt.id)
        .expect("latest decision")
        .expect("persisted decision");
    assert_eq!(
        decision.raw_provider_output_refs,
        vec!["provider-raw/rework/analyst_decision_0001.txt".to_string()]
    );
    let raw_path = store
        .paths()
        .root()
        .join("projects/project_0001/issues/issue_0001/coding-attempts")
        .join(&attempt.id)
        .join("provider-raw/rework/analyst_decision_0001.txt");
    assert_eq!(
        std::fs::read_to_string(raw_path).expect("raw output"),
        "不是 JSON"
    );
    let events = drain_events(&mut rx);
    assert!(events.iter().any(|event| {
        matches!(
            event,
            CodingWsOutMessage::CodingChatEntryCreated { entry }
                if matches!(
                    &entry.entry_type,
                    CodingEntryType::AnalystVerdict {
                        verdict: AnalystVerdict::NeedsHumanInput
                    }
                ) && entry.content.as_deref() == Some("Analyst 输出不是有效 JSON，已转人工确认。")
        )
    }));
}

#[tokio::test]
async fn execute_review_request_commits_pushes_persists_request_and_emits_update() {
    let root = tempdir().expect("root");
    let repo = root.path().join("repo");
    let remote = root.path().join("remote.git");
    init_repo(&repo);
    run_git(root.path(), &["init", "--bare", remote.to_str().unwrap()]);
    run_git(
        &repo,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(create_input())
        .expect("create attempt");
    let (tx, mut rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let started = engine
        .start_attempt("project_0001", "issue_0001", &attempt.id)
        .await
        .expect("start attempt");
    let _stage = rx.recv().await.expect("stage event");
    let _node = rx.recv().await.expect("node event");
    let prepared = engine
        .execute_worktree_prepare(&started, &repo)
        .await
        .expect("prepare worktree");
    let _worktree_complete = rx.recv().await.expect("worktree complete");
    let worktree = prepared.worktree_path.as_ref().expect("worktree path");
    fs::write(worktree.join("src.txt"), "hello\nreview request\n").expect("modify file");

    let review_request = engine
        .execute_review_request(&prepared, "origin", "feat: implement work item")
        .await
        .expect("execute review request");

    assert_eq!(review_request.id, "review_request_0001");
    assert_eq!(review_request.attempt_id, attempt.id);
    assert_eq!(review_request.push_status, PushStatus::Pushed);
    assert_eq!(review_request.remote, "origin");
    assert_eq!(
        review_request.branch_name,
        "aria/work-items/work_item_0001/attempt-1"
    );
    assert_eq!(review_request.commit_sha.len(), 40);
    let persisted = store
        .list_review_requests("project_0001", "issue_0001", &attempt.id)
        .expect("review requests");
    assert_eq!(persisted, vec![review_request.clone()]);
    let updated = store
        .get_attempt("project_0001", "issue_0001", &attempt.id)
        .expect("updated attempt");
    assert_eq!(updated.stage, CodingExecutionStage::ReviewRequest);
    assert_eq!(
        updated.head_commit.as_deref(),
        Some(review_request.commit_sha.as_str())
    );
    assert_eq!(updated.pushed_remote.as_deref(), Some("origin"));
    assert_eq!(
        updated.review_request_id.as_deref(),
        Some("review_request_0001")
    );

    match rx.recv().await.expect("review request node") {
        CodingWsOutMessage::CodingTimelineNodeCreated { node } => {
            assert_eq!(node.stage, CodingExecutionStage::ReviewRequest);
            assert_eq!(node.status, CodingTimelineNodeStatus::Running);
        }
        other => panic!("expected review request node, got {other:?}"),
    }
    match rx.recv().await.expect("review request update") {
        CodingWsOutMessage::ReviewRequestUpdate {
            review_request: event_request,
        } => {
            assert_eq!(event_request.id, review_request.id);
            assert_eq!(event_request.push_status, PushStatus::Pushed);
        }
        other => panic!("expected review request update, got {other:?}"),
    }
    match rx.recv().await.expect("review request node update") {
        CodingWsOutMessage::CodingTimelineNodeUpdated {
            status, summary, ..
        } => {
            assert_eq!(status, CodingTimelineNodeStatus::Completed);
            assert_eq!(summary.as_deref(), Some("review request 已创建"));
        }
        other => panic!("expected review request node update, got {other:?}"),
    }
}

#[tokio::test]
async fn review_request_does_not_commit_runtime_artifacts() {
    let root = tempdir().expect("root");
    let repo = root.path().join("repo");
    let remote = root.path().join("remote.git");
    init_repo(&repo);
    run_git(root.path(), &["init", "--bare", remote.to_str().unwrap()]);
    run_git(
        &repo,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(create_input())
        .expect("create attempt");
    let (tx, mut rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);
    let started = engine
        .start_attempt("project_0001", "issue_0001", &attempt.id)
        .await
        .expect("start attempt");
    let _stage = rx.recv().await.expect("stage event");
    let _node = rx.recv().await.expect("node event");
    let prepared = engine
        .execute_worktree_prepare(&started, &repo)
        .await
        .expect("prepare worktree");
    let _worktree_complete = rx.recv().await.expect("worktree complete");
    let worktree = prepared.worktree_path.as_ref().expect("worktree path");
    fs::create_dir_all(worktree.join("tests/__pycache__")).expect("tests pycache");
    fs::create_dir_all(worktree.join("__pycache__")).expect("root pycache");
    fs::create_dir_all(worktree.join(".aria/coding-artifacts/test-output")).expect("artifacts");
    fs::write(
        worktree.join("climbing_stairs.py"),
        "def climb_stairs(n): return n\n",
    )
    .expect("source");
    fs::write(
        worktree.join("tests/test_climbing_stairs.py"),
        "def test_climb_stairs(): pass\n",
    )
    .expect("test");
    fs::write(
        worktree.join("__pycache__/climbing_stairs.cpython-310.pyc"),
        b"pyc",
    )
    .expect("pyc");
    fs::write(
        worktree.join("tests/__pycache__/test_climbing_stairs.cpython-310.pyc"),
        b"pyc",
    )
    .expect("test pyc");
    fs::write(
        worktree.join(".aria/coding-artifacts/test-output/planned_001.stdout.log"),
        "stdout",
    )
    .expect("stdout");

    let review_request = engine
        .execute_review_request(&prepared, "origin", "feat: implement work item")
        .await
        .expect("execute review request");

    let mut committed = git_stdout(
        worktree,
        &[
            "show",
            "--name-only",
            "--format=",
            &review_request.commit_sha,
        ],
    )
    .lines()
    .filter(|line| !line.trim().is_empty())
    .map(str::to_string)
    .collect::<Vec<_>>();
    committed.sort();
    assert_eq!(
        committed,
        vec![
            "climbing_stairs.py".to_string(),
            "tests/test_climbing_stairs.py".to_string(),
        ]
    );
}

#[tokio::test]
async fn review_request_blocks_when_only_runtime_artifacts_changed() {
    let root = tempdir().expect("root");
    let repo = root.path().join("repo");
    let remote = root.path().join("remote.git");
    init_repo(&repo);
    run_git(root.path(), &["init", "--bare", remote.to_str().unwrap()]);
    run_git(
        &repo,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(create_input())
        .expect("create attempt");
    let (tx, mut rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let started = engine
        .start_attempt("project_0001", "issue_0001", &attempt.id)
        .await
        .expect("start attempt");
    let _stage = rx.recv().await.expect("stage event");
    let _node = rx.recv().await.expect("node event");
    let prepared = engine
        .execute_worktree_prepare(&started, &repo)
        .await
        .expect("prepare worktree");
    let _worktree_complete = rx.recv().await.expect("worktree complete");
    let worktree = prepared.worktree_path.as_ref().expect("worktree path");
    fs::create_dir_all(worktree.join("__pycache__")).expect("pycache");
    fs::create_dir_all(worktree.join(".aria/coding-artifacts/test-output")).expect("artifacts");
    fs::write(
        worktree.join("__pycache__/climbing_stairs.cpython-310.pyc"),
        b"pyc",
    )
    .expect("pyc");
    fs::write(
        worktree.join(".aria/coding-artifacts/test-output/planned_001.stdout.log"),
        "stdout",
    )
    .expect("stdout");

    let error = engine
        .execute_review_request(&prepared, "origin", "feat: implement work item")
        .await
        .expect_err("runtime artifacts only should not create review request");

    assert!(
        error
            .to_string()
            .contains("过滤运行产物后没有可提交的业务变更")
    );
    let updated = store
        .get_attempt("project_0001", "issue_0001", &attempt.id)
        .expect("updated attempt");
    assert_eq!(updated.status, CodingAttemptStatus::Blocked);
    assert_eq!(updated.stage, CodingExecutionStage::ReviewRequest);
    assert!(
        store
            .list_review_requests("project_0001", "issue_0001", &attempt.id)
            .expect("review requests")
            .is_empty()
    );
}

#[tokio::test]
async fn execute_review_request_blocks_attempt_when_push_fails() {
    let root = tempdir().expect("root");
    let repo = root.path().join("repo");
    init_repo(&repo);
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(create_input())
        .expect("create attempt");
    let (tx, mut rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let started = engine
        .start_attempt("project_0001", "issue_0001", &attempt.id)
        .await
        .expect("start attempt");
    let _stage = rx.recv().await.expect("stage event");
    let _node = rx.recv().await.expect("node event");
    let prepared = engine
        .execute_worktree_prepare(&started, &repo)
        .await
        .expect("prepare worktree");
    let _worktree_complete = rx.recv().await.expect("worktree complete");
    let worktree = prepared.worktree_path.as_ref().expect("worktree path");
    fs::write(worktree.join("src.txt"), "hello\npush failure\n").expect("modify file");

    let review_request = engine
        .execute_review_request(&prepared, "missing", "feat: implement work item")
        .await
        .expect("execute review request");

    assert_eq!(review_request.push_status, PushStatus::Failed);
    let updated = store
        .get_attempt("project_0001", "issue_0001", &attempt.id)
        .expect("updated attempt");
    assert_eq!(updated.status, CodingAttemptStatus::Blocked);
    assert_eq!(updated.stage, CodingExecutionStage::ReviewRequest);

    let _node = rx.recv().await.expect("review request node");
    let _request = rx.recv().await.expect("review request update");
    match rx.recv().await.expect("review request node update") {
        CodingWsOutMessage::CodingTimelineNodeUpdated {
            status, summary, ..
        } => {
            assert_eq!(status, CodingTimelineNodeStatus::Failed);
            assert_eq!(summary.as_deref(), Some("review request 推送失败"));
        }
        other => panic!("expected review request node update, got {other:?}"),
    }
}

#[tokio::test]
async fn execute_internal_pr_review_persists_review_and_waits_for_final_rework() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    init_repo(&worktree);
    fs::write(worktree.join("src.txt"), "hello\ninternal review\n").expect("modify file");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            base_branch: "HEAD".to_string(),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    store
        .update_attempt_stage(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingExecutionStage::ReviewRequest,
        )
        .expect("review request stage");
    let request = sample_review_request(&attempt.id);
    store
        .save_review_request(&request)
        .expect("save review request");
    let (tx, mut rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = InternalReviewStreamingProvider;

    let review = engine
        .execute_internal_pr_review(&attempt, &provider)
        .await
        .expect("execute internal pr review");

    assert_eq!(review.id, "internal_review_0001");
    assert_eq!(review.attempt_id, attempt.id);
    assert_eq!(review.review_request_id, request.id);
    assert_eq!(review.verdict, ReviewVerdict::Approve);
    assert_eq!(review.summary, "internal review ok");
    assert_eq!(review.impact_scope, vec!["src"]);
    assert_eq!(review.pr_description, "实现 work item");
    assert_eq!(
        review.commit_message_suggestion,
        "feat: implement work item"
    );
    let persisted = store
        .list_internal_pr_reviews("project_0001", "issue_0001", &attempt.id)
        .expect("internal reviews");
    assert_eq!(persisted, vec![review.clone()]);
    let updated = store
        .get_attempt("project_0001", "issue_0001", &attempt.id)
        .expect("updated attempt");
    assert_eq!(updated.status, CodingAttemptStatus::Running);
    assert_eq!(updated.stage, CodingExecutionStage::InternalPrReview);

    let nodes = store
        .get_timeline_nodes("project_0001", "issue_0001", &attempt.id)
        .expect("timeline nodes");
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].stage, CodingExecutionStage::InternalPrReview);
    assert_eq!(nodes[0].status, CodingTimelineNodeStatus::Completed);

    match rx.recv().await.expect("internal review node created") {
        CodingWsOutMessage::CodingTimelineNodeCreated { node } => {
            assert_eq!(node.id, "coding_node_0001");
            assert_eq!(node.stage, CodingExecutionStage::InternalPrReview);
            assert_eq!(node.status, CodingTimelineNodeStatus::Running);
        }
        other => panic!("expected internal review node created, got {other:?}"),
    }
    match rx.recv().await.expect("internal review provider prompt") {
        CodingWsOutMessage::CodingExecutionEvent { event } => {
            assert_eq!(event.event_id, "coding_node_0001_prompt");
            assert_eq!(event.node_id.as_deref(), Some("coding_node_0001"));
            assert_eq!(event.title, "Provider Prompt");
            assert!(
                event
                    .output
                    .as_deref()
                    .is_some_and(|output| output.contains("InternalReviewer"))
            );
        }
        other => panic!("expected internal review provider prompt, got {other:?}"),
    }
    assert_eq!(
        rx.recv().await.expect("internal review stream chunk"),
        CodingWsOutMessage::CodingStreamChunk {
            content: "reviewing pushed branch".to_string(),
            node_id: Some("coding_node_0001".to_string()),
        }
    );
    assert_eq!(
        rx.recv().await.expect("internal review message complete"),
        CodingWsOutMessage::CodingMessageComplete {
            node_id: Some("coding_node_0001".to_string()),
        }
    );
    match rx.recv().await.expect("internal review chat entry") {
        CodingWsOutMessage::CodingChatEntryCreated { entry } => {
            assert_eq!(entry.node_id.as_deref(), Some("coding_node_0001"));
            assert_eq!(entry.role, CodingAgentRole::Reviewer);
            assert_eq!(entry.entry_type, CodingEntryType::AssistantMessage);
            assert_eq!(entry.content.as_deref(), Some("internal review ok"));
            assert_eq!(
                entry
                    .metadata
                    .as_ref()
                    .and_then(|value| value.get("review_request_id"))
                    .and_then(|value| value.as_str()),
                Some("review_request_0001")
            );
        }
        other => panic!("expected internal review chat entry, got {other:?}"),
    }
    match rx.recv().await.expect("internal review complete") {
        CodingWsOutMessage::InternalPrReviewComplete {
            review: event_review,
        } => {
            assert_eq!(event_review.id, "internal_review_0001");
            assert_eq!(event_review.verdict, ReviewVerdict::Approve);
        }
        other => panic!("expected internal review complete, got {other:?}"),
    }
    match rx.recv().await.expect("internal review node completed") {
        CodingWsOutMessage::CodingTimelineNodeUpdated {
            node_id,
            status,
            summary,
            completed_at,
        } => {
            assert_eq!(node_id, "coding_node_0001");
            assert_eq!(status, CodingTimelineNodeStatus::Completed);
            assert_eq!(summary.as_deref(), Some("internal PR review 通过"));
            assert!(completed_at.is_some());
        }
        other => panic!("expected internal review node completed, got {other:?}"),
    }
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn execute_internal_pr_review_blocked_keeps_attempt_running_for_analyst() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    init_repo(&worktree);
    fs::write(worktree.join("src.txt"), "hello\ninternal review\n").expect("modify file");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            base_branch: "HEAD".to_string(),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    store
        .update_attempt_stage(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingExecutionStage::ReviewRequest,
        )
        .expect("review request stage");
    let request = sample_review_request(&attempt.id);
    store
        .save_review_request(&request)
        .expect("save review request");
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = InputCapturingProvider {
        input: Arc::new(Mutex::new(None)),
        output: r#"{
            "verdict": "blocked",
            "summary": "内部 review 需要人工确认发布窗口",
            "findings": [],
            "impact_scope": ["release"],
            "pr_description": "实现 work item",
            "commit_message_suggestion": "feat: implement work item"
        }"#
        .to_string(),
    };

    let review = engine
        .execute_internal_pr_review(&attempt, &provider)
        .await
        .expect("execute internal pr review");

    assert_eq!(review.verdict, ReviewVerdict::Blocked);
    assert_eq!(review.summary, "内部 review 需要人工确认发布窗口");
    let updated = store
        .get_attempt("project_0001", "issue_0001", &attempt.id)
        .expect("updated attempt");
    assert_eq!(updated.status, CodingAttemptStatus::Running);
    assert_eq!(updated.stage, CodingExecutionStage::InternalPrReview);
    let gates = store
        .list_open_blocked_gates("project_0001", "issue_0001", &attempt.id)
        .expect("open blocked gates");
    assert!(gates.is_empty());
}

#[tokio::test]
async fn execute_internal_pr_review_prompt_includes_request_commit_diff_and_function_context() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    init_repo(&worktree);
    fs::write(worktree.join("src.txt"), "hello\ninternal prompt diff\n").expect("modify file");
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    seed_work_item_markdown(
        &app_paths,
        "函数 climb_stairs(n: i32) -> i32 需要测试 n=10。",
    );
    let store = CodingAttemptStore::new(app_paths);
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            base_branch: "HEAD".to_string(),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_role_provider_config_snapshot(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingRoleProviderConfigSnapshot {
                coder: ProviderName::Fake,
                tester: ProviderName::Fake,
                analyst: ProviderName::Fake,
                code_reviewer: ProviderName::Fake,
                internal_reviewer: ProviderName::Codex,
                review_rounds: 1,
                permission_modes: CodingRolePermissionModes::default(),
            },
        )
        .expect("set role provider snapshot");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    store
        .update_attempt_stage(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingExecutionStage::ReviewRequest,
        )
        .expect("review request stage");
    let request = sample_review_request(&attempt.id);
    store
        .save_review_request(&request)
        .expect("save review request");
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);
    let captured_input = Arc::new(Mutex::new(None));
    let provider = InputCapturingProvider {
        input: captured_input.clone(),
        output: r#"{"verdict":"approve","summary":"internal ok","findings":[],"impact_scope":["src"],"pr_description":"实现 work item","commit_message_suggestion":"feat: implement work item"}"#.to_string(),
    };

    engine
        .execute_internal_pr_review(&attempt, &provider)
        .await
        .expect("execute internal review");

    let input = captured_input
        .lock()
        .expect("input lock")
        .clone()
        .expect("captured input");
    assert_eq!(input.provider_type, ProviderType::Codex);
    assert_eq!(input.role, AdapterRole::Reviewer);
    assert_eq!(
        input.output_schema,
        "coding_workspace_internal_pr_review_json"
    );
    assert!(input.prompt.contains("InternalReviewer"));
    assert!(input.prompt.contains("Review Request: review_request_0001"));
    assert!(
        input
            .prompt
            .contains("Commit: 0123456789012345678901234567890123456789")
    );
    assert!(input.prompt.contains("+internal prompt diff"));
    assert!(input.prompt.contains("climb_stairs"));
    assert!(input.prompt.contains("影响范围"));
    assert!(input.prompt.contains("PR description"));
    assert!(input.prompt.contains("commit message"));
}

#[tokio::test]
async fn handle_final_confirm_completes_waiting_attempt_and_timeline_node() {
    let root = tempdir().expect("root");
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths.clone());
    lifecycle
        .create_work_item(CreateWorkItemInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            story_spec_ids: Vec::new(),
            design_spec_ids: Vec::new(),
            title: "Coding work item".to_string(),
            ..Default::default()
        })
        .expect("create work item");
    lifecycle
        .update_work_item_execution_status(
            "project_0001",
            "issue_0001",
            "work_item_0001",
            WorkItemStatus::Coding,
        )
        .expect("coding work item");
    let store = CodingAttemptStore::new(app_paths);
    let attempt = store
        .create_attempt(create_input())
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    store
        .update_attempt_stage(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingExecutionStage::FinalConfirm,
        )
        .expect("final confirm stage");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::WaitingForHuman,
        )
        .expect("waiting for human");
    store
        .save_timeline_node(CodingTimelineNode {
            id: "coding_node_0001".to_string(),
            attempt_id: attempt.id.clone(),
            stage: CodingExecutionStage::FinalConfirm,
            title: "最终确认".to_string(),
            status: CodingTimelineNodeStatus::Running,
            agent_role: Some(CodingAgentRole::System),
            summary: None,
            started_at: "2026-05-23T00:00:00Z".to_string(),
            completed_at: None,
            artifact_refs: Vec::new(),
        })
        .expect("save final confirm node");
    let (tx, mut rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    let updated = engine
        .handle_final_confirm("project_0001", "issue_0001", &attempt.id)
        .await
        .expect("handle final confirm");

    assert_eq!(updated.status, CodingAttemptStatus::Completed);
    assert_eq!(updated.stage, CodingExecutionStage::FinalConfirm);
    assert!(updated.completed_at.is_some());
    let nodes = store
        .get_timeline_nodes("project_0001", "issue_0001", &attempt.id)
        .expect("timeline nodes");
    assert_eq!(nodes[0].status, CodingTimelineNodeStatus::Completed);
    assert_eq!(nodes[0].summary.as_deref(), Some("用户已确认完成"));
    assert!(nodes[0].completed_at.is_some());
    let work_items = lifecycle
        .list_work_items("project_0001", "issue_0001")
        .expect("work items");
    assert_eq!(work_items[0].execution_status, WorkItemStatus::Completed);

    match rx.recv().await.expect("final confirm timeline update") {
        CodingWsOutMessage::CodingTimelineNodeUpdated {
            node_id,
            status,
            summary,
            completed_at,
        } => {
            assert_eq!(node_id, "coding_node_0001");
            assert_eq!(status, CodingTimelineNodeStatus::Completed);
            assert_eq!(summary.as_deref(), Some("用户已确认完成"));
            assert!(completed_at.is_some());
        }
        other => panic!("expected final confirm timeline update, got {other:?}"),
    }
}

#[tokio::test]
async fn handle_abort_marks_attempt_aborted_and_closes_active_timeline_node() {
    let root = tempdir().expect("root");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(create_input())
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    store
        .update_attempt_stage(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingExecutionStage::Testing,
        )
        .expect("testing stage");
    store
        .save_timeline_node(CodingTimelineNode {
            id: "coding_node_0001".to_string(),
            attempt_id: attempt.id.clone(),
            stage: CodingExecutionStage::Testing,
            title: "执行测试".to_string(),
            status: CodingTimelineNodeStatus::Running,
            agent_role: Some(CodingAgentRole::Tester),
            summary: None,
            started_at: "2026-05-23T00:00:00Z".to_string(),
            completed_at: None,
            artifact_refs: Vec::new(),
        })
        .expect("save testing node");
    let (tx, mut rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    let updated = engine
        .handle_abort("project_0001", "issue_0001", &attempt.id)
        .await
        .expect("handle abort");

    assert_eq!(updated.status, CodingAttemptStatus::Aborted);
    assert_eq!(updated.stage, CodingExecutionStage::Testing);
    assert!(updated.completed_at.is_some());
    let nodes = store
        .get_timeline_nodes("project_0001", "issue_0001", &attempt.id)
        .expect("timeline nodes");
    assert_eq!(nodes[0].status, CodingTimelineNodeStatus::Failed);
    assert_eq!(nodes[0].summary.as_deref(), Some("用户已中止"));
    assert!(nodes[0].completed_at.is_some());

    match rx.recv().await.expect("abort timeline update") {
        CodingWsOutMessage::CodingTimelineNodeUpdated {
            node_id,
            status,
            summary,
            completed_at,
        } => {
            assert_eq!(node_id, "coding_node_0001");
            assert_eq!(status, CodingTimelineNodeStatus::Failed);
            assert_eq!(summary.as_deref(), Some("用户已中止"));
            assert!(completed_at.is_some());
        }
        other => panic!("expected abort timeline update, got {other:?}"),
    }
}

fn create_input() -> CreateCodingAttemptInput {
    CreateCodingAttemptInput {
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        work_item_id: "work_item_0001".to_string(),
        base_branch: "HEAD".to_string(),
        branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
        worktree_path: None,
        provider_config_snapshot: ProviderConfigSnapshot {
            author: ProviderName::Fake,
            reviewer: Some(ProviderName::Fake),
            review_rounds: 1,
        },
        max_auto_rework: 2,
    }
}

fn seed_work_item_markdown(app_paths: &ProductAppPaths, markdown: &str) {
    let lifecycle = LifecycleStore::new(app_paths.clone());
    lifecycle
        .create_work_item(CreateWorkItemInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            story_spec_ids: Vec::new(),
            design_spec_ids: Vec::new(),
            title: "Coding work item".to_string(),
            ..Default::default()
        })
        .expect("create work item");
    let session = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: "work_item_0001".to_string(),
            workspace_type: WorkspaceType::WorkItem,
            author_provider: ProviderName::Fake,
            reviewer_provider: ProviderName::Fake,
            review_rounds: 1,
            superpowers_enabled: true,
            openspec_enabled: true,
        })
        .expect("create workspace session");
    lifecycle
        .append_artifact_version(
            &session.id,
            ArtifactVersion {
                version: 1,
                markdown: markdown.to_string(),
                generated_by: ProviderName::Fake,
                reviewed_by: Some(ProviderName::Fake),
                review_verdict: None,
                confirmed_by: Some("user".to_string()),
                is_current: true,
                created_at: "2026-05-23T00:00:00Z".to_string(),
                source_node_id: "node_0001".to_string(),
            },
        )
        .expect("append artifact version");
}

fn init_repo(repo: &Path) {
    fs::create_dir_all(repo).expect("create repo");
    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "aria@example.com"]);
    run_git(repo, &["config", "user.name", "Aria Test"]);
    fs::write(repo.join("src.txt"), "hello\n").expect("seed file");
    run_git(repo, &["add", "."]);
    run_git(repo, &["commit", "-m", "initial"]);
}

fn git_repo_in(path: &Path) -> PathBuf {
    fs::create_dir_all(path).expect("create repo dir");
    run_git(path, &["init"]);
    run_git(path, &["config", "user.email", "aria@example.com"]);
    run_git(path, &["config", "user.name", "Aria Test"]);
    fs::write(path.join("README.md"), "# repo\n").expect("seed readme");
    run_git(path, &["add", "."]);
    run_git(path, &["commit", "-m", "initial"]);
    run_git(path, &["branch", "-m", "main"]);
    path.to_path_buf()
}

fn coding_store_with_attempt(
    root: &Path,
    work_item_id: &str,
    branch_name: &str,
) -> (CodingAttemptStore, CodingExecutionAttempt) {
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: work_item_id.to_string(),
            base_branch: "main".to_string(),
            branch_name: branch_name.to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
            },
            max_auto_rework: 2,
        })
        .expect("create attempt");
    (store, attempt)
}

fn final_confirm_attempt(
    paths: ProductAppPaths,
    work_item_id: &str,
) -> (CodingAttemptStore, CodingExecutionAttempt) {
    let lifecycle = LifecycleStore::new(paths.clone());
    lifecycle
        .create_work_item(CreateWorkItemInput {
            id: Some(work_item_id.to_string()),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            story_spec_ids: Vec::new(),
            design_spec_ids: Vec::new(),
            title: "work item".to_string(),
            ..Default::default()
        })
        .expect("create work item");
    let store = CodingAttemptStore::new(paths);
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: work_item_id.to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
            },
            max_auto_rework: 2,
        })
        .expect("create attempt");
    let attempt = store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("set running");
    let attempt = store
        .update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::FinalConfirm,
        )
        .expect("set final confirm stage");
    let attempt = store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::WaitingForHuman,
        )
        .expect("set waiting for human");
    (store, attempt)
}

fn failed_attempt(
    paths: ProductAppPaths,
    work_item_id: &str,
) -> (CodingAttemptStore, CodingExecutionAttempt) {
    let store = CodingAttemptStore::new(paths);
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: work_item_id.to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
            },
            max_auto_rework: 2,
        })
        .expect("create attempt");
    let attempt = store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("set running");
    let attempt = store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Failed,
        )
        .expect("set failed");
    (store, attempt)
}

fn dirty_failed_attempt(
    paths: ProductAppPaths,
    work_item_id: &str,
) -> (CodingAttemptStore, CodingExecutionAttempt) {
    failed_attempt(paths, work_item_id)
}

fn run_git(cwd: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {:?} failed\nstdout:{}\nstderr:{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_stdout(cwd: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {:?} failed\nstdout:{}\nstderr:{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn sample_review_request(attempt_id: &str) -> ReviewRequest {
    ReviewRequest {
        id: "review_request_0001".to_string(),
        attempt_id: attempt_id.to_string(),
        kind: ReviewRequestKind::GitBranchOnly,
        remote_kind: RemoteKind::GenericGit,
        remote: "origin".to_string(),
        base_branch: "main".to_string(),
        branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
        commit_sha: "0123456789012345678901234567890123456789".to_string(),
        push_status: PushStatus::Pushed,
        external_url: None,
        manual_instructions: vec!["create review request".to_string()],
        created_at: "2026-05-23T00:00:00Z".to_string(),
        updated_at: "2026-05-23T00:00:00Z".to_string(),
    }
}

struct FileWritingStreamingProvider;

#[async_trait::async_trait]
impl StreamingProviderAdapter for FileWritingStreamingProvider {
    async fn run_streaming(
        &self,
        input: &AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        let worktree = input
            .worktree_path
            .as_ref()
            .map(PathBuf::from)
            .expect("worktree path");
        fs::write(worktree.join("generated.txt"), "generated by provider\n").map_err(|error| {
            ProviderAdapterError::incompatible_output(error.to_string(), "", "")
        })?;
        let (tx, rx) = mpsc::channel(8);
        tx.try_send(StreamChunk::Text("created generated.txt".to_string()))
            .expect("send text chunk");
        tx.try_send(StreamChunk::Done {
            full_output: "done".to_string(),
        })
        .expect("send done chunk");
        Ok(rx)
    }
}

struct PromptCapturingProvider {
    prompt: Arc<Mutex<Option<String>>>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for PromptCapturingProvider {
    async fn run_streaming(
        &self,
        input: &AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        *self.prompt.lock().expect("prompt lock") = Some(input.prompt.clone());
        let (tx, rx) = mpsc::channel(8);
        tx.try_send(StreamChunk::Done {
            full_output: "done".to_string(),
        })
        .expect("send done chunk");
        Ok(rx)
    }
}

struct TesterRetryPromptCaptureProvider {
    prompts: Arc<Mutex<Vec<String>>>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for TesterRetryPromptCaptureProvider {
    fn supports_provider_driven_testing(&self) -> bool {
        true
    }

    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.prompts
            .lock()
            .expect("prompts")
            .push(input.prompt.clone());
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel(8);
        event_tx
            .try_send(ProviderEvent::Completed {
                full_output: r#"{"summary":"retry plan","context_warnings":[],"assumptions":[],"steps":[{"id":"unit","title":"unit","intent":"run unit tests","required":true,"tool":"provider_managed","risk_level":"low","command_or_tool_input":{},"evidence_expectation":"provider evidence","related_requirements":["REQ-UNIT"],"related_design_constraints":["DEC-UNIT"],"related_work_item_tasks":["TASK-UNIT"]}]}"#.to_string(),
                provider_session_id: None,
            })
            .expect("send completed");
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }
}

struct InputCapturingProvider {
    input: Arc<Mutex<Option<AdapterInput>>>,
    output: String,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for InputCapturingProvider {
    async fn run_streaming(
        &self,
        input: &AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        *self.input.lock().expect("input lock") = Some(input.clone());
        let (tx, rx) = mpsc::channel(8);
        tx.try_send(StreamChunk::Done {
            full_output: self.output.clone(),
        })
        .expect("send done chunk");
        Ok(rx)
    }
}

struct SessionInputCapturingProvider {
    inputs: Arc<Mutex<Vec<StreamingProviderInput>>>,
    outputs: Arc<Mutex<VecDeque<String>>>,
    provider_session_ids: Arc<Mutex<VecDeque<Option<String>>>>,
}

impl Default for SessionInputCapturingProvider {
    fn default() -> Self {
        Self::with_outputs(["coding done"], [Some("coder-session-1".to_string())])
    }
}

impl SessionInputCapturingProvider {
    fn with_outputs<const N: usize, const M: usize>(
        outputs: [&str; N],
        provider_session_ids: [Option<String>; M],
    ) -> Self {
        Self {
            inputs: Arc::new(Mutex::new(Vec::new())),
            outputs: Arc::new(Mutex::new(
                outputs.into_iter().map(ToOwned::to_owned).collect(),
            )),
            provider_session_ids: Arc::new(Mutex::new(provider_session_ids.into_iter().collect())),
        }
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for SessionInputCapturingProvider {
    fn supports_tool_calls(&self) -> bool {
        true
    }

    fn supports_provider_driven_testing(&self) -> bool {
        true
    }

    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.inputs.lock().expect("inputs lock").push(input);
        let output = self
            .outputs
            .lock()
            .expect("outputs lock")
            .pop_front()
            .unwrap_or_else(|| "coding done".to_string());
        let provider_session_id = self
            .provider_session_ids
            .lock()
            .expect("provider session ids lock")
            .pop_front()
            .unwrap_or_else(|| Some("coder-session-1".to_string()));
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel(8);
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::Completed {
                    full_output: output,
                    provider_session_id,
                })
                .await;
        });
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }

    async fn run_streaming(
        &self,
        _input: &AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        Err(ProviderAdapterError::execution_failed(
            None,
            String::new(),
            "run_streaming is not used by this test provider",
            0,
        ))
    }
}

struct ExecutePlanToolCallTesterProvider {
    inputs: Arc<Mutex<Vec<StreamingProviderInput>>>,
    starts: Arc<Mutex<usize>>,
}

impl ExecutePlanToolCallTesterProvider {
    fn new() -> Self {
        Self {
            inputs: Arc::new(Mutex::new(Vec::new())),
            starts: Arc::new(Mutex::new(0)),
        }
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for ExecutePlanToolCallTesterProvider {
    fn supports_tool_calls(&self) -> bool {
        true
    }

    fn supports_provider_driven_testing(&self) -> bool {
        true
    }

    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.inputs.lock().expect("inputs lock").push(input.clone());
        let start_no = {
            let mut starts = self.starts.lock().expect("starts lock");
            *starts += 1;
            *starts
        };

        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, mut command_rx) = mpsc::channel(8);
        if start_no == 1 {
            event_tx
                .try_send(ProviderEvent::Completed {
                    full_output: r#"{"summary":"unit plan","steps":[{"id":"unit","title":"Unit","intent":"run unit checks","required":true,"tool":"run_command","risk_level":"low","command_or_tool_input":{"command":["true"]},"evidence_expectation":"unit evidence","related_requirements":["REQ-UNIT"],"related_design_constraints":["DEC-UNIT"],"related_work_item_tasks":["TASK-UNIT"]}]}"#.to_string(),
                    provider_session_id: None,
                })
                .expect("send plan completed");
            return Ok(ProviderSession {
                events: event_rx,
                commands: command_tx,
            });
        }

        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::ToolCall(ProviderToolCall {
                    id: "execute_tool_0001".to_string(),
                    tool_name: "run_command".to_string(),
                    input: serde_json::json!({
                        "step_id": "unit",
                        "command": ["true"]
                    }),
                }))
                .await;
            while let Some(command) = command_rx.recv().await {
                match command {
                    cadence_aria::cross_cutting::streaming_provider::ProviderCommand::ToolResult(
                        result,
                    ) if result.tool_use_id == "execute_tool_0001" => {
                        let _ = event_tx
                            .send(ProviderEvent::Completed {
                                full_output: r#"{"step_results":[{"step_id":"unit","status":"passed","evidence_refs":["unit.log"],"provider_analysis":"ok"}]}"#.to_string(),
                                provider_session_id: None,
                            })
                            .await;
                        return;
                    }
                    cadence_aria::cross_cutting::streaming_provider::ProviderCommand::Abort => {
                        return;
                    }
                    _ => {}
                }
            }
        });
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }
}

#[derive(Default)]
struct ExecutePlanChoiceThenCompletedTesterProvider {
    starts: Arc<Mutex<usize>>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for ExecutePlanChoiceThenCompletedTesterProvider {
    fn supports_provider_driven_testing(&self) -> bool {
        true
    }

    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let start_no = {
            let mut starts = self.starts.lock().expect("starts lock");
            *starts += 1;
            *starts
        };

        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel(8);
        if start_no == 1 {
            event_tx
                .try_send(ProviderEvent::Completed {
                    full_output: r#"{"summary":"unit plan","steps":[{"id":"unit","title":"Unit","intent":"run unit checks","required":true,"tool":"provider_managed","risk_level":"low","command_or_tool_input":{},"evidence_expectation":"unit evidence","related_requirements":["REQ-UNIT"],"related_design_constraints":["DEC-UNIT"],"related_work_item_tasks":["TASK-UNIT"]}]}"#.to_string(),
                    provider_session_id: None,
                })
                .expect("send plan completed");
            return Ok(ProviderSession {
                events: event_rx,
                commands: command_tx,
            });
        }

        event_tx
            .try_send(ProviderEvent::ChoiceRequest(ChoiceRequestData {
                id: "choice_0001".to_string(),
                prompt: "确认是否继续执行测试".to_string(),
                options: vec![ChoiceOptionData {
                    id: "continue".to_string(),
                    label: "继续".to_string(),
                    description: None,
                }],
                allow_multiple: false,
                allow_free_text: false,
                source: ChoiceRequestSource::AskUserQuestion,
            }))
            .expect("send choice request");
        event_tx
            .try_send(ProviderEvent::Completed {
                full_output: r#"{"step_results":[{"step_id":"unit","status":"passed","evidence_refs":["unit.log"],"provider_analysis":"ok"}]}"#.to_string(),
                provider_session_id: None,
            })
            .expect("send completed");

        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }
}

#[derive(Default)]
struct HangingExecutePlanStartTesterProvider {
    starts: Arc<Mutex<usize>>,
    plan_warning: Option<String>,
}

impl HangingExecutePlanStartTesterProvider {
    fn with_plan_warning(plan_warning: &str) -> Self {
        Self {
            starts: Arc::new(Mutex::new(0)),
            plan_warning: Some(plan_warning.to_string()),
        }
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for HangingExecutePlanStartTesterProvider {
    fn supports_tool_calls(&self) -> bool {
        true
    }

    fn supports_provider_driven_testing(&self) -> bool {
        true
    }

    async fn start(
        &self,
        _input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let start_no = {
            let mut starts = self.starts.lock().expect("starts lock");
            *starts += 1;
            *starts
        };

        if start_no == 1 {
            let (event_tx, event_rx) = mpsc::channel(8);
            let (command_tx, _command_rx) = mpsc::channel(8);
            let context_warnings = self
                .plan_warning
                .as_ref()
                .map(|warning| serde_json::json!([warning]))
                .unwrap_or_else(|| serde_json::json!([]));
            event_tx
                .try_send(ProviderEvent::Completed {
                    full_output: serde_json::json!({
                        "summary": "unit plan",
                        "context_warnings": context_warnings,
                        "steps": [{
                            "id": "unit",
                            "title": "Unit",
                            "intent": "run unit checks",
                            "required": true,
                            "tool": "provider_managed",
                            "risk_level": "low",
                            "command_or_tool_input": {},
                            "evidence_expectation": "unit evidence",
                            "related_requirements": ["REQ-UNIT"],
                            "related_design_constraints": ["DEC-UNIT"],
                            "related_work_item_tasks": ["TASK-UNIT"]
                        }]
                    })
                    .to_string(),
                    provider_session_id: None,
                })
                .expect("send plan completed");
            return Ok(ProviderSession {
                events: event_rx,
                commands: command_tx,
            });
        }

        cancel.cancelled().await;
        Err(ProviderAdapterError::execution_failed(
            None,
            String::new(),
            "provider execute start was cancelled",
            1,
        ))
    }
}

struct HangingPlanTesterProvider;

#[async_trait::async_trait]
impl StreamingProviderAdapter for HangingPlanTesterProvider {
    fn supports_provider_driven_testing(&self) -> bool {
        true
    }

    async fn start(
        &self,
        input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel(8);
        tokio::spawn(async move {
            if input.prompt.contains("Phase: plan_tests") {
                let _ = event_tx
                    .send(ProviderEvent::Execution(ProviderExecutionEvent {
                        event_id: "task_update_0001".to_string(),
                        kind: ProviderExecutionEventKind::Command,
                        status: ProviderExecutionEventStatus::Running,
                        title: "Task update".to_string(),
                        detail: Some("planning tests".to_string()),
                        command: None,
                        cwd: None,
                        output: None,
                        exit_code: None,
                    }))
                    .await;
                cancel.cancelled().await;
            }
        });
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }
}

struct NeverStartingTesterProvider;

#[async_trait::async_trait]
impl StreamingProviderAdapter for NeverStartingTesterProvider {
    fn supports_provider_driven_testing(&self) -> bool {
        true
    }

    async fn start(
        &self,
        _input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        cancel.cancelled().await;
        Err(ProviderAdapterError::execution_failed(
            None,
            String::new(),
            "provider start was cancelled",
            1,
        ))
    }
}

struct EventEmittingCodingProvider;

#[async_trait::async_trait]
impl StreamingProviderAdapter for EventEmittingCodingProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (event_tx, event_rx) = mpsc::channel(16);
        let (command_tx, _command_rx) = mpsc::channel(8);
        event_tx
            .try_send(ProviderEvent::TextDelta {
                content: "working".to_string(),
            })
            .expect("send text");
        event_tx
            .try_send(ProviderEvent::Execution(ProviderExecutionEvent {
                event_id: "command_0001".to_string(),
                kind: ProviderExecutionEventKind::Command,
                status: ProviderExecutionEventStatus::Completed,
                title: "Run tests".to_string(),
                detail: Some("Executed verification command".to_string()),
                command: Some("uv run pytest".to_string()),
                cwd: None,
                output: Some("1 passed".to_string()),
                exit_code: Some(0),
            }))
            .expect("send execution event");
        event_tx
            .try_send(ProviderEvent::ToolCall(ProviderToolCall {
                id: "tool_0001".to_string(),
                tool_name: "run_command".to_string(),
                input: serde_json::json!({ "command": "uv run pytest" }),
            }))
            .expect("send tool call");
        event_tx
            .try_send(ProviderEvent::ToolResult(ProviderToolResult {
                tool_use_id: "tool_0001".to_string(),
                output: "1 passed".to_string(),
                is_error: false,
            }))
            .expect("send tool result");
        event_tx
            .try_send(ProviderEvent::Completed {
                full_output: "done".to_string(),
                provider_session_id: None,
            })
            .expect("send completed");

        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }
}

struct ControlEventCodingProvider;

#[async_trait::async_trait]
impl StreamingProviderAdapter for ControlEventCodingProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (event_tx, event_rx) = mpsc::channel(16);
        let (command_tx, _command_rx) = mpsc::channel(8);
        event_tx
            .try_send(ProviderEvent::StatusChanged(ProviderStatus::Running))
            .expect("send status");
        event_tx
            .try_send(ProviderEvent::PermissionRequest(PermissionRequestData {
                id: "permission_0001".to_string(),
                tool_name: "shell".to_string(),
                description: "Run uv test command".to_string(),
                risk_level: RiskLevel::High,
            }))
            .expect("send permission");
        event_tx
            .try_send(ProviderEvent::ChoiceRequest(ChoiceRequestData {
                id: "choice_0001".to_string(),
                prompt: "Select implementation strategy".to_string(),
                options: vec![ChoiceOptionData {
                    id: "dp".to_string(),
                    label: "Dynamic programming".to_string(),
                    description: Some("Iterative solution".to_string()),
                }],
                allow_multiple: false,
                allow_free_text: true,
                source: ChoiceRequestSource::ProviderChoice,
            }))
            .expect("send choice");
        event_tx
            .try_send(ProviderEvent::Completed {
                full_output: "done".to_string(),
                provider_session_id: None,
            })
            .expect("send completed");

        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }
}

struct PermissionAwaitingProvider;

#[async_trait::async_trait]
impl StreamingProviderAdapter for PermissionAwaitingProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (event_tx, event_rx) = mpsc::channel(16);
        let (command_tx, mut command_rx) = mpsc::channel(8);
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::PermissionRequest(PermissionRequestData {
                    id: "permission_0001".to_string(),
                    tool_name: "shell".to_string(),
                    description: "Run uv test command".to_string(),
                    risk_level: RiskLevel::High,
                }))
                .await;
            while let Some(command) = command_rx.recv().await {
                match command {
                    cadence_aria::cross_cutting::streaming_provider::ProviderCommand::PermissionResponse {
                        id,
                        approved,
                        ..
                    } if id == "permission_0001" && approved => {
                        let _ = event_tx
                            .send(ProviderEvent::Completed {
                                full_output: "approved".to_string(),
                                provider_session_id: None,
                            })
                            .await;
                        return;
                    }
                    cadence_aria::cross_cutting::streaming_provider::ProviderCommand::Abort => {
                        return;
                    }
                    _ => {}
                }
            }
        });
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }
}

struct ChoiceAwaitingProvider;

#[async_trait::async_trait]
impl StreamingProviderAdapter for ChoiceAwaitingProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (event_tx, event_rx) = mpsc::channel(16);
        let (command_tx, mut command_rx) = mpsc::channel(8);
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::ChoiceRequest(ChoiceRequestData {
                    id: "choice_0001".to_string(),
                    prompt: "Select implementation strategy".to_string(),
                    options: vec![ChoiceOptionData {
                        id: "backend_first".to_string(),
                        label: "先做后端".to_string(),
                        description: Some("TASK-001 到 TASK-009".to_string()),
                    }],
                    allow_multiple: false,
                    allow_free_text: true,
                    source: ChoiceRequestSource::RequestUserInput,
                }))
                .await;
            while let Some(command) = command_rx.recv().await {
                match command {
                    cadence_aria::cross_cutting::streaming_provider::ProviderCommand::ChoiceResponse {
                        id,
                        selected_option_ids,
                        ..
                    } if id == "choice_0001"
                        && selected_option_ids == vec!["backend_first".to_string()] =>
                    {
                        let _ = event_tx
                            .send(ProviderEvent::Completed {
                                full_output: "selected backend_first".to_string(),
                                provider_session_id: None,
                            })
                            .await;
                        return;
                    }
                    cadence_aria::cross_cutting::streaming_provider::ProviderCommand::Abort => {
                        return;
                    }
                    _ => {}
                }
            }
        });
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }
}

struct ChoiceThenPermissionProvider;

#[async_trait::async_trait]
impl StreamingProviderAdapter for ChoiceThenPermissionProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (event_tx, event_rx) = mpsc::channel(16);
        let (command_tx, _command_rx) = mpsc::channel(8);
        event_tx
            .try_send(ProviderEvent::ChoiceRequest(ChoiceRequestData {
                id: "choice_0001".to_string(),
                prompt: "Select implementation strategy".to_string(),
                options: vec![ChoiceOptionData {
                    id: "backend_first".to_string(),
                    label: "先做后端".to_string(),
                    description: Some("TASK-001 到 TASK-009".to_string()),
                }],
                allow_multiple: false,
                allow_free_text: true,
                source: ChoiceRequestSource::RequestUserInput,
            }))
            .expect("send choice");
        event_tx
            .try_send(ProviderEvent::PermissionRequest(PermissionRequestData {
                id: "permission_0001".to_string(),
                tool_name: "shell".to_string(),
                description: "Run tests".to_string(),
                risk_level: RiskLevel::High,
            }))
            .expect("send permission");
        event_tx
            .try_send(ProviderEvent::Completed {
                full_output: "done".to_string(),
                provider_session_id: None,
            })
            .expect("send completed");

        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }
}

struct EventThenCompletedProvider {
    output: String,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for EventThenCompletedProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (event_tx, event_rx) = mpsc::channel(16);
        let (command_tx, _command_rx) = mpsc::channel(8);
        event_tx
            .try_send(ProviderEvent::Execution(ProviderExecutionEvent {
                event_id: "provider_command_0001".to_string(),
                kind: ProviderExecutionEventKind::Command,
                status: ProviderExecutionEventStatus::Completed,
                title: "Provider command".to_string(),
                detail: Some("Provider emitted a command event".to_string()),
                command: Some("git diff --stat".to_string()),
                cwd: Some(input.working_dir.display().to_string()),
                output: Some("changed files".to_string()),
                exit_code: Some(0),
            }))
            .expect("send execution event");
        event_tx
            .try_send(ProviderEvent::Completed {
                full_output: self.output.clone(),
                provider_session_id: None,
            })
            .expect("send completed");
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }
}

struct ReviewControlEventProvider;

#[async_trait::async_trait]
impl StreamingProviderAdapter for ReviewControlEventProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (event_tx, event_rx) = mpsc::channel(16);
        let (command_tx, _command_rx) = mpsc::channel(8);
        event_tx
            .try_send(ProviderEvent::TextDelta {
                content: "reviewing".to_string(),
            })
            .expect("send text");
        event_tx
            .try_send(ProviderEvent::Execution(ProviderExecutionEvent {
                event_id: "review_command_0001".to_string(),
                kind: ProviderExecutionEventKind::Command,
                status: ProviderExecutionEventStatus::Completed,
                title: "Review command".to_string(),
                detail: Some("Ran review helper".to_string()),
                command: Some("cargo test --locked".to_string()),
                cwd: Some(input.working_dir.display().to_string()),
                output: Some("review ok".to_string()),
                exit_code: Some(0),
            }))
            .expect("send execution");
        event_tx
            .try_send(ProviderEvent::ToolCall(ProviderToolCall {
                id: "review_tool_0001".to_string(),
                tool_name: "run_command".to_string(),
                input: serde_json::json!({ "command": "cargo test --locked" }),
            }))
            .expect("send tool call");
        event_tx
            .try_send(ProviderEvent::ToolResult(ProviderToolResult {
                tool_use_id: "review_tool_0001".to_string(),
                output: "tool ok".to_string(),
                is_error: false,
            }))
            .expect("send tool result");
        event_tx
            .try_send(ProviderEvent::StatusChanged(ProviderStatus::Running))
            .expect("send status");
        event_tx
            .try_send(ProviderEvent::PermissionRequest(PermissionRequestData {
                id: "permission_review_0001".to_string(),
                tool_name: "shell".to_string(),
                description: "Inspect diff".to_string(),
                risk_level: RiskLevel::High,
            }))
            .expect("send permission");
        event_tx
            .try_send(ProviderEvent::Completed {
                full_output: r#"{"verdict":"approve","summary":"review ok","findings":[]}"#
                    .to_string(),
                provider_session_id: Some("review-session-0001".to_string()),
            })
            .expect("send completed");
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }
}

struct ReviewPermissionTimeoutProvider;

#[async_trait::async_trait]
impl StreamingProviderAdapter for ReviewPermissionTimeoutProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel(8);
        event_tx
            .try_send(ProviderEvent::PermissionTimeout {
                permission_id: "permission_review_timeout".to_string(),
            })
            .expect("send permission timeout");
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }
}

struct StartFailingProvider;

#[async_trait::async_trait]
impl StreamingProviderAdapter for StartFailingProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        Err(ProviderAdapterError::command_missing(
            "provider failed to start",
        ))
    }
}

struct ReviewStreamingProvider;

#[async_trait::async_trait]
impl StreamingProviderAdapter for ReviewStreamingProvider {
    async fn run_streaming(
        &self,
        _input: &AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        let (tx, rx) = mpsc::channel(8);
        tx.try_send(StreamChunk::Text("reviewing diff".to_string()))
            .expect("send review chunk");
        tx.try_send(StreamChunk::Done {
            full_output: r#"{"verdict":"approve","summary":"review ok","findings":[]}"#.to_string(),
        })
        .expect("send review done");
        Ok(rx)
    }
}

fn drain_events(rx: &mut mpsc::Receiver<CodingWsOutMessage>) -> Vec<CodingWsOutMessage> {
    let mut events = Vec::new();
    while let Ok(event) = rx.try_recv() {
        events.push(event);
    }
    events
}

fn assert_provider_command_event(events: &[CodingWsOutMessage]) {
    assert!(
        events.iter().any(|event| {
            matches!(
                event,
                CodingWsOutMessage::CodingExecutionEvent { event }
                    if event.title == "Provider command"
                        && event.kind == WsExecutionEventKind::Command
                        && event.status == WsExecutionEventStatus::Completed
                        && event.command.as_deref() == Some("git diff --stat")
                        && event.output.as_deref() == Some("changed files")
            )
        }),
        "expected provider command execution event, got {events:?}"
    );
}

struct AnalystStreamingProvider {
    prompt: Arc<Mutex<Option<String>>>,
    output: String,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for AnalystStreamingProvider {
    async fn run_streaming(
        &self,
        input: &AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        *self.prompt.lock().expect("prompt lock") = Some(input.prompt.clone());
        let (tx, rx) = mpsc::channel(8);
        tx.try_send(StreamChunk::Done {
            full_output: self.output.clone(),
        })
        .expect("send rework done");
        Ok(rx)
    }
}

struct InternalReviewStreamingProvider;

#[async_trait::async_trait]
impl StreamingProviderAdapter for InternalReviewStreamingProvider {
    async fn run_streaming(
        &self,
        _input: &AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        let (tx, rx) = mpsc::channel(8);
        tx.try_send(StreamChunk::Text("reviewing pushed branch".to_string()))
            .expect("send internal review chunk");
        tx.try_send(StreamChunk::Done {
            full_output: r#"{"verdict":"approve","summary":"internal review ok","findings":[],"impact_scope":["src"],"pr_description":"实现 work item","commit_message_suggestion":"feat: implement work item"}"#.to_string(),
        })
        .expect("send internal review done");
        Ok(rx)
    }
}

#[tokio::test]
async fn analyst_human_gate_offers_retry_analyst_action() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    init_repo(&worktree);
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let mut input = create_input();
    input.worktree_path = Some(worktree);
    let attempt = store.create_attempt(input).expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = InputCapturingProvider {
        input: Arc::new(Mutex::new(None)),
        output: "Analyst prose without JSON".to_string(),
    };

    engine
        .execute_rework(&attempt, "testing blocked evidence", &provider)
        .await
        .expect("execute analyst");

    let gates = store
        .list_open_blocked_gates("project_0001", "issue_0001", &attempt.id)
        .expect("gates");
    assert_eq!(gates.len(), 1);
    assert_eq!(gates[0].role, Some(CodingProviderRole::Analyst));
    assert!(gates[0].available_actions.iter().any(|action| {
        action.action_id == "retry_analyst"
            && action.action_type == CodingGateActionType::RetryAnalyst
    }));
}

#[tokio::test]
async fn provide_context_keeps_analyst_human_gate_open_for_retry() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    init_repo(&worktree);
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let mut input = create_input();
    input.worktree_path = Some(worktree);
    let attempt = store.create_attempt(input).expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Blocked,
        )
        .expect("blocked");
    store
        .update_attempt_stage(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingExecutionStage::Rework,
        )
        .expect("rework");
    store
        .create_blocked_gate(CreateBlockedGateInput {
            attempt_id: attempt.id.clone(),
            stage: CodingExecutionStage::Rework,
            node_id: Some("coding_node_0002".to_string()),
            role: Some(CodingProviderRole::Analyst),
            title: "Analyst human gate".to_string(),
            description: "需要重跑 Analyst".to_string(),
            reason_code: Some("analyst_human_gate".to_string()),
            evidence_refs: Vec::new(),
            raw_provider_output_ref: Some(
                "provider-raw/rework/analyst_decision_0001.txt".to_string(),
            ),
            available_actions: vec![
                CodingGateAction {
                    action_id: "retry_analyst".to_string(),
                    label: "重试 Analyst".to_string(),
                    action_type: CodingGateActionType::RetryAnalyst,
                },
                CodingGateAction {
                    action_id: "provide_context".to_string(),
                    label: "补充上下文".to_string(),
                    action_type: CodingGateActionType::ProvideContext,
                },
                CodingGateAction {
                    action_id: "abort".to_string(),
                    label: "终止".to_string(),
                    action_type: CodingGateActionType::Abort,
                },
            ],
        })
        .expect("create gate");
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    let updated = engine
        .handle_blocked_gate_response(
            "project_0001",
            "issue_0001",
            &attempt.id,
            "coding_blocked_gate_0001",
            "provide_context",
            Some("请按系统支持的 Analyst JSON schema 重试".to_string()),
        )
        .await
        .expect("provide context");

    assert_eq!(updated.status, CodingAttemptStatus::WaitingForHuman);
    assert_eq!(updated.stage, CodingExecutionStage::Rework);
    let notes = store
        .list_context_notes("project_0001", "issue_0001", &attempt.id)
        .expect("context notes");
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].consumed_by_rework_round, None);
    let gates = store
        .list_open_blocked_gates("project_0001", "issue_0001", &attempt.id)
        .expect("open gates");
    assert_eq!(gates.len(), 1);
    assert_eq!(gates[0].gate_id, "coding_blocked_gate_0001");
    assert!(gates[0].available_actions.iter().any(|action| {
        action.action_id == "retry_analyst"
            && action.action_type == CodingGateActionType::RetryAnalyst
    }));
}

#[tokio::test]
async fn execute_rework_binds_analyst_decision_chat_and_gate_to_role_run() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    init_repo(&worktree);
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let mut input = create_input();
    input.worktree_path = Some(worktree);
    let attempt = store.create_attempt(input).expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let (tx, _rx) = mpsc::channel(32);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = InputCapturingProvider {
        input: Arc::new(Mutex::new(None)),
        output: "not json".to_string(),
    };

    engine
        .execute_rework(&attempt, "testing blocked evidence", &provider)
        .await
        .expect("execute analyst");

    let runs = store
        .list_role_runs("project_0001", "issue_0001", &attempt.id)
        .expect("role runs");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].stage, CodingExecutionStage::Rework);
    assert_eq!(runs[0].role, CodingProviderRole::Analyst);
    assert_eq!(runs[0].status, CodingRoleRunStatus::Blocked);
    assert!(
        runs[0]
            .raw_provider_output_refs
            .iter()
            .any(|value| value.contains("analyst_decision"))
    );
    assert!(
        runs[0]
            .artifact_refs
            .iter()
            .any(|value| value.contains("analyst_evidence"))
    );

    let decision = store
        .latest_analyst_decision("project_0001", "issue_0001", &attempt.id)
        .expect("latest decision")
        .expect("decision");
    assert_eq!(decision.role_run_id.as_deref(), Some(runs[0].id.as_str()));
    assert_eq!(decision.run_no, Some(1));

    let entries = store
        .list_chat_entries("project_0001", "issue_0001", &attempt.id)
        .expect("chat entries");
    assert!(entries.iter().any(|entry| {
        entry.metadata.as_ref().is_some_and(|metadata| {
            metadata.get("role_run_id").and_then(|value| value.as_str())
                == Some(runs[0].id.as_str())
                && metadata.get("run_no").and_then(|value| value.as_u64()) == Some(1)
        })
    }));
}

#[tokio::test]
async fn retry_analyst_gate_response_supersedes_latest_analyst_run() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    init_repo(&worktree);
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let mut input = create_input();
    input.worktree_path = Some(worktree);
    let attempt = store.create_attempt(input).expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let (tx, _rx) = mpsc::channel(32);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    let first_run = store
        .create_role_run(
            &attempt,
            CodingExecutionStage::Rework,
            CodingProviderRole::Analyst,
            CodingRoleRunTrigger::Initial,
            None,
        )
        .expect("create first run");
    store
        .update_role_run_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            &first_run.id,
            CodingRoleRunStatus::Blocked,
            Some("analyst_human_gate".to_string()),
        )
        .expect("block first run");
    store
        .update_role_run_refs(
            "project_0001",
            "issue_0001",
            &attempt.id,
            &first_run.id,
            Vec::new(),
            vec!["artifacts/rework/analyst_evidence_0001.txt".to_string()],
        )
        .expect("add evidence ref");

    store
        .create_blocked_gate(CreateBlockedGateInput {
            attempt_id: attempt.id.clone(),
            stage: CodingExecutionStage::Rework,
            node_id: Some("coding_node_0002".to_string()),
            role: Some(CodingProviderRole::Analyst),
            title: "Analyst human gate".to_string(),
            description: "需要重跑 Analyst".to_string(),
            reason_code: Some("analyst_human_gate".to_string()),
            evidence_refs: vec!["artifacts/rework/analyst_evidence_0001.txt".to_string()],
            raw_provider_output_ref: None,
            available_actions: vec![CodingGateAction {
                action_id: "retry_analyst".to_string(),
                label: "重试 Analyst".to_string(),
                action_type: CodingGateActionType::RetryAnalyst,
            }],
        })
        .expect("create gate");

    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::WaitingForHuman,
        )
        .expect("wait for human");

    let updated = engine
        .handle_blocked_gate_response(
            "project_0001",
            "issue_0001",
            &attempt.id,
            "coding_blocked_gate_0001",
            "retry_analyst",
            None,
        )
        .await
        .expect("retry analyst");

    assert_eq!(updated.status, CodingAttemptStatus::Running);
    assert_eq!(updated.stage, CodingExecutionStage::Rework);

    let runs = store
        .list_role_runs("project_0001", "issue_0001", &attempt.id)
        .expect("role runs");
    assert_eq!(runs.len(), 2);
    let first = runs
        .iter()
        .find(|run| run.id == first_run.id)
        .expect("first run");
    assert_eq!(first.status, CodingRoleRunStatus::Superseded);
    let second = runs
        .iter()
        .find(|run| run.id != first_run.id)
        .expect("second run");
    assert_eq!(second.status, CodingRoleRunStatus::Running);
    assert_eq!(second.trigger, CodingRoleRunTrigger::RetryAnalyst);
    assert_eq!(
        second.supersedes_run_id.as_deref(),
        Some(first_run.id.as_str())
    );
    assert_eq!(
        second.artifact_refs,
        vec!["artifacts/rework/analyst_evidence_0001.txt".to_string()]
    );
}

#[tokio::test]
async fn execute_code_review_binds_report_chat_and_status_to_role_run() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    init_repo(&worktree);
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let mut input = create_input();
    input.worktree_path = Some(worktree);
    let attempt = store.create_attempt(input).expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    store
        .update_attempt_stage(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingExecutionStage::CodeReview,
        )
        .expect("set stage");
    let (tx, _rx) = mpsc::channel(32);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = InputCapturingProvider {
        input: Arc::new(Mutex::new(None)),
        output: r#"{"verdict":"approve","summary":"review ok","findings":[]}"#.to_string(),
    };

    engine
        .execute_code_review(&attempt, &provider)
        .await
        .expect("execute code review");

    let runs = store
        .list_role_runs("project_0001", "issue_0001", &attempt.id)
        .expect("role runs");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].stage, CodingExecutionStage::CodeReview);
    assert_eq!(runs[0].role, CodingProviderRole::CodeReviewer);
    assert_eq!(runs[0].status, CodingRoleRunStatus::Completed);
    assert_eq!(runs[0].run_no, 1);
    assert!(
        runs[0]
            .raw_provider_output_refs
            .iter()
            .any(|value| value.contains("code_review"))
    );

    let reports = store
        .list_code_review_reports("project_0001", "issue_0001", &attempt.id)
        .expect("reports");
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].role_run_id.as_deref(), Some(runs[0].id.as_str()));
    assert_eq!(reports[0].run_no, Some(1));

    let entries = store
        .list_chat_entries("project_0001", "issue_0001", &attempt.id)
        .expect("chat entries");
    assert!(entries.iter().any(|entry| {
        entry.metadata.as_ref().is_some_and(|metadata| {
            metadata.get("source").and_then(|value| value.as_str()) == Some("code_review")
                && metadata.get("role_run_id").and_then(|value| value.as_str())
                    == Some(runs[0].id.as_str())
                && metadata.get("run_no").and_then(|value| value.as_u64()) == Some(1)
        })
    }));
}

#[tokio::test]
async fn execute_internal_pr_review_binds_review_chat_and_status_to_role_run() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    init_repo(&worktree);
    fs::write(worktree.join("src.txt"), "hello\ninternal reviewed\n").expect("modify file");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let mut input = create_input();
    input.worktree_path = Some(worktree);
    input.base_branch = "HEAD".to_string();
    let attempt = store.create_attempt(input).expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    store
        .update_attempt_stage(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingExecutionStage::InternalPrReview,
        )
        .expect("set stage");
    store
        .save_review_request(&sample_review_request(&attempt.id))
        .expect("save review request");
    let (tx, _rx) = mpsc::channel(32);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = InputCapturingProvider {
        input: Arc::new(Mutex::new(None)),
        output: r#"{"verdict":"approve","summary":"internal ok","findings":[],"impact_scope":["src/lib.rs"],"pr_description":"PR body","commit_message_suggestion":"feat: work"}"#.to_string(),
    };

    engine
        .execute_internal_pr_review(&attempt, &provider)
        .await
        .expect("execute internal review");

    let runs = store
        .list_role_runs("project_0001", "issue_0001", &attempt.id)
        .expect("role runs");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].stage, CodingExecutionStage::InternalPrReview);
    assert_eq!(runs[0].role, CodingProviderRole::InternalReviewer);
    assert_eq!(runs[0].status, CodingRoleRunStatus::Completed);
    assert_eq!(runs[0].run_no, 1);
    assert!(
        runs[0]
            .raw_provider_output_refs
            .iter()
            .any(|value| value.contains("internal_pr_review"))
    );

    let reviews = store
        .list_internal_pr_reviews("project_0001", "issue_0001", &attempt.id)
        .expect("internal reviews");
    assert_eq!(reviews.len(), 1);
    assert_eq!(reviews[0].role_run_id.as_deref(), Some(runs[0].id.as_str()));
    assert_eq!(reviews[0].run_no, Some(1));

    let entries = store
        .list_chat_entries("project_0001", "issue_0001", &attempt.id)
        .expect("chat entries");
    assert!(entries.iter().any(|entry| {
        entry.metadata.as_ref().is_some_and(|metadata| {
            metadata.get("source").and_then(|value| value.as_str()) == Some("internal_pr_review")
                && metadata.get("role_run_id").and_then(|value| value.as_str())
                    == Some(runs[0].id.as_str())
                && metadata.get("run_no").and_then(|value| value.as_u64()) == Some(1)
                && metadata
                    .get("impact_scope")
                    .and_then(|value| value.as_array())
                    .is_some_and(|scope| scope.iter().any(|value| value == "src/lib.rs"))
        })
    }));
}
