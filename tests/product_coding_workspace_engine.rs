use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use cadence_aria::cross_cutting::provider_adapter::ProviderAdapterError;
use cadence_aria::cross_cutting::streaming_provider::{StreamChunk, StreamingProviderAdapter};
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::coding_attempt_store::{CodingAttemptStore, CreateCodingAttemptInput};
use cadence_aria::product::coding_models::{
    CodingAgentRole, CodingAttemptStatus, CodingExecutionStage, CodingTimelineNode,
    CodingTimelineNodeStatus, PushStatus, RemoteKind, ReviewRequest, ReviewRequestKind,
    ReviewVerdict, TestingOverallStatus,
};
use cadence_aria::product::coding_workspace_engine::CodingWorkspaceEngine;
use cadence_aria::product::git_workspace_service::GitWorkspaceService;
use cadence_aria::product::lifecycle_store::{CreateWorkItemInput, LifecycleStore};
use cadence_aria::product::models::ProviderName;
use cadence_aria::product::models::WorkItemStatus;
use cadence_aria::product::test_executor::TestCommandSpec;
use cadence_aria::protocol::contracts::AdapterInput;
use cadence_aria::web::coding_ws_handler::CodingWsOutMessage;
use cadence_aria::web::workspace_ws_types::ProviderConfigSnapshot;
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
        .execute_coding(&attempt, &provider)
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
        fs::read_to_string(worktree.join(&report.commands[0].stdout_ref)).expect("stdout"),
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
async fn execute_code_review_persists_report_and_emits_review_events() {
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
async fn execute_rework_runs_provider_with_evidence_and_returns_to_testing() {
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
            CodingExecutionStage::CodeReview,
        )
        .expect("code review stage");
    let (tx, mut rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = ReworkStreamingProvider;

    let updated = engine
        .execute_rework(&attempt, "测试失败: unit failed", &provider)
        .await
        .expect("execute rework");

    assert_eq!(updated.status, CodingAttemptStatus::Running);
    assert_eq!(updated.stage, CodingExecutionStage::Testing);
    assert_eq!(updated.rework_count, 1);
    assert_eq!(
        fs::read_to_string(worktree.join("reworked.txt")).expect("reworked file"),
        "fixed after evidence\n"
    );
    let nodes = store
        .get_timeline_nodes("project_0001", "issue_0001", &attempt.id)
        .expect("timeline nodes");
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].stage, CodingExecutionStage::Rework);
    assert_eq!(nodes[0].title, "返工 #1");
    assert_eq!(nodes[0].status, CodingTimelineNodeStatus::Completed);
    assert_eq!(nodes[0].summary.as_deref(), Some("返工完成"));

    match rx.recv().await.expect("rework node created") {
        CodingWsOutMessage::CodingTimelineNodeCreated { node } => {
            assert_eq!(node.id, "coding_node_0001");
            assert_eq!(node.stage, CodingExecutionStage::Rework);
            assert_eq!(node.title, "返工 #1");
            assert_eq!(node.status, CodingTimelineNodeStatus::Running);
        }
        other => panic!("expected rework node created, got {other:?}"),
    }
    assert_eq!(
        rx.recv().await.expect("rework stream chunk"),
        CodingWsOutMessage::CodingStreamChunk {
            content: "applied rework".to_string(),
            node_id: Some("coding_node_0001".to_string()),
        }
    );
    assert_eq!(
        rx.recv().await.expect("rework message complete"),
        CodingWsOutMessage::CodingMessageComplete {
            node_id: Some("coding_node_0001".to_string()),
        }
    );
    match rx.recv().await.expect("rework node completed") {
        CodingWsOutMessage::CodingTimelineNodeUpdated {
            node_id,
            status,
            summary,
            completed_at,
        } => {
            assert_eq!(node_id, "coding_node_0001");
            assert_eq!(status, CodingTimelineNodeStatus::Completed);
            assert_eq!(summary.as_deref(), Some("返工完成"));
            assert!(completed_at.is_some());
        }
        other => panic!("expected rework node completed, got {other:?}"),
    }
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
async fn execute_internal_pr_review_persists_review_and_enters_final_confirm() {
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
    let persisted = store
        .list_internal_pr_reviews("project_0001", "issue_0001", &attempt.id)
        .expect("internal reviews");
    assert_eq!(persisted, vec![review.clone()]);
    let updated = store
        .get_attempt("project_0001", "issue_0001", &attempt.id)
        .expect("updated attempt");
    assert_eq!(updated.status, CodingAttemptStatus::WaitingForHuman);
    assert_eq!(updated.stage, CodingExecutionStage::FinalConfirm);

    let nodes = store
        .get_timeline_nodes("project_0001", "issue_0001", &attempt.id)
        .expect("timeline nodes");
    assert_eq!(nodes.len(), 2);
    assert_eq!(nodes[0].stage, CodingExecutionStage::InternalPrReview);
    assert_eq!(nodes[0].status, CodingTimelineNodeStatus::Completed);
    assert_eq!(nodes[1].stage, CodingExecutionStage::FinalConfirm);
    assert_eq!(nodes[1].status, CodingTimelineNodeStatus::Running);

    match rx.recv().await.expect("internal review node created") {
        CodingWsOutMessage::CodingTimelineNodeCreated { node } => {
            assert_eq!(node.id, "coding_node_0001");
            assert_eq!(node.stage, CodingExecutionStage::InternalPrReview);
            assert_eq!(node.status, CodingTimelineNodeStatus::Running);
        }
        other => panic!("expected internal review node created, got {other:?}"),
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
    match rx.recv().await.expect("final confirm node created") {
        CodingWsOutMessage::CodingTimelineNodeCreated { node } => {
            assert_eq!(node.id, "coding_node_0002");
            assert_eq!(node.stage, CodingExecutionStage::FinalConfirm);
            assert_eq!(node.status, CodingTimelineNodeStatus::Running);
            assert_eq!(node.summary.as_deref(), Some("等待用户最终确认"));
        }
        other => panic!("expected final confirm node created, got {other:?}"),
    }
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

struct ReworkStreamingProvider;

#[async_trait::async_trait]
impl StreamingProviderAdapter for ReworkStreamingProvider {
    async fn run_streaming(
        &self,
        input: &AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        assert!(input.prompt.contains("测试失败: unit failed"));
        let worktree = input
            .worktree_path
            .as_ref()
            .map(PathBuf::from)
            .expect("worktree path");
        fs::write(worktree.join("reworked.txt"), "fixed after evidence\n").map_err(|error| {
            ProviderAdapterError::incompatible_output(error.to_string(), "", "")
        })?;
        let (tx, rx) = mpsc::channel(8);
        tx.try_send(StreamChunk::Text("applied rework".to_string()))
            .expect("send rework chunk");
        tx.try_send(StreamChunk::Done {
            full_output: "rework done".to_string(),
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
            full_output: r#"{"verdict":"approve","summary":"internal review ok","findings":[]}"#
                .to_string(),
        })
        .expect("send internal review done");
        Ok(rx)
    }
}
