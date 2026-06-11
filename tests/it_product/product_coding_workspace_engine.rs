use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

use cadence_aria::cross_cutting::provider_adapter::ProviderAdapterError;
use cadence_aria::cross_cutting::streaming_provider::{
    ChoiceOptionData, ChoiceRequestData, ChoiceRequestSource, PermissionRequestData, ProviderEvent,
    ProviderExecutionEvent, ProviderExecutionEventKind, ProviderExecutionEventStatus,
    ProviderPermissionMode, ProviderSession, ProviderStatus, ProviderToolCall, ProviderToolResult,
    RiskLevel, StreamChunk, StreamingProviderAdapter, StreamingProviderInput,
};
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::coding_attempt_store::{CodingAttemptStore, CreateCodingAttemptInput};
use cadence_aria::product::coding_models::{
    AnalystVerdict, CodingAgentRole, CodingAttemptStatus, CodingEntryType, CodingExecutionStage,
    CodingProviderPermissionMode, CodingProviderRole, CodingReworkInstruction,
    CodingRolePermissionModes, CodingRoleProviderConfigSnapshot, CodingTimelineNode,
    CodingTimelineNodeStatus, FindingSeverity, PushStatus, RemoteKind, ReviewRequest,
    ReviewRequestKind, ReviewVerdict, TestingOverallStatus,
};
use cadence_aria::product::coding_workspace_engine::{
    CodingExecutionContext, CodingWorkspaceEngine,
};
use cadence_aria::product::git_workspace_service::GitWorkspaceService;
use cadence_aria::product::lifecycle_store::{
    CreateWorkItemInput, CreateWorkspaceSessionInput, LifecycleStore,
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
        .update_role_provider_config_snapshot(
            "project_0001",
            "issue_0001",
            &attempt.id,
            snapshot,
        )
        .expect("save role config");

    let saved = store
        .get_role_provider_config_snapshot("project_0001", "issue_0001", &attempt.id)
        .expect("saved role config");
    assert_eq!(
        saved.permission_mode_for_role(&CodingProviderRole::CodeReviewer),
        CodingProviderPermissionMode::Auto
    );
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
            r#"{"summary":"testing plan","steps":[{"id":"provider_check","title":"Provider check","intent":"verify provider session isolation","required":true,"tool":"provider_managed","risk_level":"low","command_or_tool_input":{},"evidence_expectation":"provider evidence"}]}"#,
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
        assert_eq!(input.permission_mode, ProviderPermissionMode::Supervised);
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
            r#"{"summary":"unit","steps":[{"id":"unit","title":"Unit","intent":"verify unit","required":true,"tool":"provider_managed","risk_level":"low","command_or_tool_input":{},"evidence_expectation":"provider evidence"}]}"#,
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
    assert_eq!(
        inputs[0].permission_mode,
        ProviderPermissionMode::Supervised
    );
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

    engine
        .execute_coding(&attempt, &provider, &CodingExecutionContext::default())
        .await
        .expect("execute coding");

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
                    options,
                    allow_multiple,
                    allow_free_text,
                } if id == "choice_0001"
                    && prompt == "Select implementation strategy"
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
async fn execute_testing_marks_attempt_blocked_when_no_commands_are_available() {
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
    assert_eq!(updated.status, CodingAttemptStatus::Blocked);
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
async fn review_payload_parse_failure_blocks_instead_of_approves() {
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
    assert_eq!(updated.status, CodingAttemptStatus::Blocked);
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
    assert_eq!(updated.status, CodingAttemptStatus::Blocked);
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
async fn execute_rework_needs_fix_at_limit_routes_to_code_review_with_warning() {
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

    assert_eq!(updated.status, CodingAttemptStatus::Running);
    assert_eq!(updated.stage, CodingExecutionStage::CodeReview);
    assert_eq!(updated.rework_count, 2);
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
}

#[tokio::test]
async fn execute_rework_needs_human_input_waits_for_human() {
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

    assert_eq!(updated.status, CodingAttemptStatus::WaitingForHuman);
    assert_eq!(updated.stage, CodingExecutionStage::Rework);
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
async fn execute_rework_invalid_json_falls_back_to_human_input() {
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

    assert_eq!(updated.status, CodingAttemptStatus::WaitingForHuman);
    assert_eq!(updated.stage, CodingExecutionStage::Rework);
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
