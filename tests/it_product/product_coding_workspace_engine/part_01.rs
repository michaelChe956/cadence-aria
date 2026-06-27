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
    ArtifactPayload, ArtifactVersion, ProviderConfigSnapshot, WsExecutionEventKind,
    WsExecutionEventStatus,
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

