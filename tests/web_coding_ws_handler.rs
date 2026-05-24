use cadence_aria::cross_cutting::provider_adapter::ProviderAdapterError;
use cadence_aria::cross_cutting::provider_registry::ProviderRegistry;
use cadence_aria::cross_cutting::streaming_provider::{StreamChunk, StreamingProviderAdapter};
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::coding_attempt_store::{CodingAttemptStore, CreateCodingAttemptInput};
use cadence_aria::product::coding_models::{
    CodingAgentRole, CodingAttemptStatus, CodingExecutionStage, CodingGateAction,
    CodingGateActionType, CodingGateKind, CodingGateRequired, CodingTimelineNode,
    CodingTimelineNodeStatus, PushStatus, TestingOverallStatus,
};
use cadence_aria::product::lifecycle_store::{CreateWorkItemInput, LifecycleStore};
use cadence_aria::product::models::WorkItemStatus;
use cadence_aria::product::models::{ProviderName, WorkItemPlanStatus};
use cadence_aria::product::repository_store::{CreateRepositoryInput, RepositoryStore};
use cadence_aria::protocol::contracts::{AdapterInput, AdapterRole};
use cadence_aria::web::app::build_web_router;
use cadence_aria::web::coding_ws_handler::{
    CodingWsInMessage, CodingWsOutMessage, is_coding_ws_message_allowed,
};
use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::state::WebAppState;
use cadence_aria::web::workspace_ws_types::ProviderConfigSnapshot;
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use tempfile::tempdir;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::time::{Duration, timeout};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use tokio_util::sync::CancellationToken;

static WS_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[test]
fn coding_ws_out_messages_serialize_with_coding_message_type_names() {
    let message = CodingWsOutMessage::CodingStageChange {
        stage: CodingExecutionStage::Testing,
    };

    let value = serde_json::to_value(message).expect("serialize");

    assert_eq!(
        value,
        json!({
            "type": "coding_stage_change",
            "stage": "testing"
        })
    );
}

#[test]
fn coding_ws_in_messages_deserialize_client_commands() {
    let message: CodingWsInMessage = serde_json::from_value(json!({
        "type": "gate_response",
        "gate_id": "gate_0001",
        "action_id": "continue_rework",
        "extra_context": "已补充测试"
    }))
    .expect("deserialize");

    assert_eq!(
        message,
        CodingWsInMessage::GateResponse {
            gate_id: "gate_0001".to_string(),
            action_id: "continue_rework".to_string(),
            extra_context: Some("已补充测试".to_string())
        }
    );
}

#[test]
fn coding_ws_stage_validation_matches_attempt_status_and_stage() {
    assert!(is_coding_ws_message_allowed(
        &CodingAttemptStatus::Running,
        &CodingExecutionStage::PrepareContext,
        &CodingWsInMessage::StartCoding,
    ));
    assert!(!is_coding_ws_message_allowed(
        &CodingAttemptStatus::Running,
        &CodingExecutionStage::Testing,
        &CodingWsInMessage::ContextNote {
            content: "补充背景".to_string()
        },
    ));
    assert!(is_coding_ws_message_allowed(
        &CodingAttemptStatus::WaitingForHuman,
        &CodingExecutionStage::FinalConfirm,
        &CodingWsInMessage::FinalConfirm,
    ));
    assert!(is_coding_ws_message_allowed(
        &CodingAttemptStatus::Blocked,
        &CodingExecutionStage::CodeReview,
        &CodingWsInMessage::GateResponse {
            gate_id: "gate_0001".to_string(),
            action_id: "accept_risk".to_string(),
            extra_context: None
        },
    ));
    assert!(is_coding_ws_message_allowed(
        &CodingAttemptStatus::Completed,
        &CodingExecutionStage::FinalConfirm,
        &CodingWsInMessage::CodingPing,
    ));
    assert!(!is_coding_ws_message_allowed(
        &CodingAttemptStatus::Completed,
        &CodingExecutionStage::FinalConfirm,
        &CodingWsInMessage::AbortAttempt,
    ));
}

#[test]
fn coding_gate_required_out_message_preserves_action_contract() {
    let gate = CodingGateRequired {
        gate_id: "gate_0001".to_string(),
        kind: CodingGateKind::Blocked,
        title: "需要人工决策".to_string(),
        description: "测试失败次数达到上限".to_string(),
        available_actions: vec![CodingGateAction {
            action_id: "accept_risk".to_string(),
            label: "接受风险".to_string(),
            action_type: CodingGateActionType::AcceptRisk,
        }],
    };
    let message = CodingWsOutMessage::CodingGateRequired { gate };

    let value = serde_json::to_value(message).expect("serialize");

    assert_eq!(value["type"], "coding_gate_required");
    assert_eq!(value["gate"]["kind"], "blocked");
    assert_eq!(
        value["gate"]["available_actions"][0]["action_type"],
        "accept_risk"
    );
}

#[tokio::test]
async fn coding_ws_sends_session_state_on_connect_and_responds_to_ping() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let app = app_with_attempt(root.path());
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/ws/coding-attempts/coding_attempt_0001");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");

    match recv_json(&mut ws).await {
        CodingWsOutMessage::CodingSessionState {
            attempt_id,
            status,
            stage,
            branch_name,
            timeline_nodes,
            testing_report,
            ..
        } => {
            assert_eq!(attempt_id, "coding_attempt_0001");
            assert_eq!(status, CodingAttemptStatus::Created);
            assert_eq!(stage, CodingExecutionStage::PrepareContext);
            assert_eq!(branch_name, "aria/work-items/work_item_0001/attempt-1");
            assert!(timeline_nodes.is_empty());
            assert!(testing_report.is_none());
        }
        other => panic!("expected coding session state, got {other:?}"),
    }

    send_json(&mut ws, &CodingWsInMessage::CodingPing).await;
    assert_eq!(recv_json(&mut ws).await, CodingWsOutMessage::CodingPong);

    ws.close(None).await.expect("close ws");
    server.abort();
}

#[tokio::test]
async fn coding_ws_start_coding_pushes_engine_stage_and_timeline_events() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let app = app_with_attempt(root.path());
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/ws/coding-attempts/coding_attempt_0001");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;

    send_json(&mut ws, &CodingWsInMessage::StartCoding).await;

    assert_eq!(
        recv_json(&mut ws).await,
        CodingWsOutMessage::CodingStageChange {
            stage: CodingExecutionStage::WorktreePrepare
        }
    );
    match recv_json(&mut ws).await {
        CodingWsOutMessage::CodingTimelineNodeCreated { node } => {
            assert_eq!(node.id, "coding_node_0001");
            assert_eq!(node.stage, CodingExecutionStage::WorktreePrepare);
            assert_eq!(node.status, CodingTimelineNodeStatus::Running);
        }
        other => panic!("expected coding timeline node event, got {other:?}"),
    }
    let updated = store
        .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("updated attempt");
    assert_eq!(updated.status, CodingAttemptStatus::Running);
    assert_eq!(updated.stage, CodingExecutionStage::WorktreePrepare);

    ws.close(None).await.expect("close ws");
    server.abort();
}

#[tokio::test]
async fn coding_ws_start_coding_drives_full_happy_path_to_final_confirm() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let app = app_with_full_chain_attempt(root.path());
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/ws/coding-attempts/coding_attempt_0001");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;

    send_json(&mut ws, &CodingWsInMessage::StartCoding).await;

    let mut stages = Vec::new();
    let mut final_snapshot_seen = false;
    for _ in 0..40 {
        let message = recv_json(&mut ws).await;
        match message {
            CodingWsOutMessage::CodingTimelineNodeCreated { node } => {
                stages.push(node.stage);
            }
            CodingWsOutMessage::CodingSessionState { status, stage, .. }
                if status == CodingAttemptStatus::WaitingForHuman
                    && stage == CodingExecutionStage::FinalConfirm =>
            {
                final_snapshot_seen = true;
                break;
            }
            CodingWsOutMessage::CodingProtocolError { code, message } => {
                panic!("unexpected coding protocol error {code}: {message}");
            }
            _ => {}
        }
    }

    assert!(
        final_snapshot_seen,
        "expected final_confirm snapshot over websocket"
    );
    for expected in [
        CodingExecutionStage::WorktreePrepare,
        CodingExecutionStage::Coding,
        CodingExecutionStage::Testing,
        CodingExecutionStage::CodeReview,
        CodingExecutionStage::ReviewRequest,
        CodingExecutionStage::InternalPrReview,
        CodingExecutionStage::FinalConfirm,
    ] {
        assert!(
            stages.contains(&expected),
            "missing timeline stage {expected:?}; got {stages:?}"
        );
    }

    let attempt = store
        .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("updated attempt");
    assert_eq!(attempt.status, CodingAttemptStatus::WaitingForHuman);
    assert_eq!(attempt.stage, CodingExecutionStage::FinalConfirm);
    let worktree = attempt.worktree_path.as_ref().expect("worktree path");
    assert_ne!(worktree, &root.path().join("repo"));
    assert!(worktree.join("src/lib.rs").is_file());

    let report = store
        .list_testing_reports("project_0001", "issue_0001", &attempt.id)
        .expect("testing reports")
        .pop()
        .expect("testing report");
    assert_eq!(report.overall_status, TestingOverallStatus::Passed);
    assert!(report.backend_verified);

    let review_request = store
        .list_review_requests("project_0001", "issue_0001", &attempt.id)
        .expect("review requests")
        .pop()
        .expect("review request");
    assert_eq!(review_request.push_status, PushStatus::Pushed);
    assert!(attempt.head_commit.is_some());

    assert_eq!(
        store
            .list_internal_pr_reviews("project_0001", "issue_0001", &attempt.id)
            .expect("internal reviews")
            .len(),
        1
    );

    ws.close(None).await.expect("close ws");
    server.abort();
}

#[tokio::test]
async fn coding_ws_final_confirm_completes_attempt_and_sends_snapshot() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let app = app_with_final_confirm_attempt(root.path());
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/ws/coding-attempts/coding_attempt_0001");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");

    match recv_json(&mut ws).await {
        CodingWsOutMessage::CodingSessionState { status, stage, .. } => {
            assert_eq!(status, CodingAttemptStatus::WaitingForHuman);
            assert_eq!(stage, CodingExecutionStage::FinalConfirm);
        }
        other => panic!("expected coding session state, got {other:?}"),
    }

    send_json(&mut ws, &CodingWsInMessage::FinalConfirm).await;

    match recv_json(&mut ws).await {
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
    match recv_json(&mut ws).await {
        CodingWsOutMessage::CodingSessionState {
            status,
            stage,
            active_node_id,
            ..
        } => {
            assert_eq!(status, CodingAttemptStatus::Completed);
            assert_eq!(stage, CodingExecutionStage::FinalConfirm);
            assert!(active_node_id.is_none());
        }
        other => panic!("expected completed coding session state, got {other:?}"),
    }
    let updated = store
        .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("updated attempt");
    assert_eq!(updated.status, CodingAttemptStatus::Completed);
    assert!(updated.completed_at.is_some());

    ws.close(None).await.expect("close ws");
    server.abort();
}

#[tokio::test]
async fn coding_ws_abort_attempt_closes_active_node_and_sends_snapshot() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let app = app_with_running_testing_attempt(root.path());
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/ws/coding-attempts/coding_attempt_0001");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");

    match recv_json(&mut ws).await {
        CodingWsOutMessage::CodingSessionState {
            status,
            stage,
            active_node_id,
            ..
        } => {
            assert_eq!(status, CodingAttemptStatus::Running);
            assert_eq!(stage, CodingExecutionStage::Testing);
            assert_eq!(active_node_id.as_deref(), Some("coding_node_0001"));
        }
        other => panic!("expected coding session state, got {other:?}"),
    }

    send_json(&mut ws, &CodingWsInMessage::AbortAttempt).await;

    match recv_json(&mut ws).await {
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
    match recv_json(&mut ws).await {
        CodingWsOutMessage::CodingSessionState {
            status,
            stage,
            active_node_id,
            ..
        } => {
            assert_eq!(status, CodingAttemptStatus::Aborted);
            assert_eq!(stage, CodingExecutionStage::Testing);
            assert!(active_node_id.is_none());
        }
        other => panic!("expected aborted coding session state, got {other:?}"),
    }
    let updated = store
        .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("updated attempt");
    assert_eq!(updated.status, CodingAttemptStatus::Aborted);
    assert!(updated.completed_at.is_some());

    ws.close(None).await.expect("close ws");
    server.abort();
}

fn app_with_attempt(root_path: &std::path::Path) -> axum::Router {
    let app_paths = ProductAppPaths::new(root_path.join(".aria"));
    let repo = root_path.join("repo");
    init_simple_git_repo(&repo);
    let repository = RepositoryStore::new(app_paths.clone())
        .create(CreateRepositoryInput {
            project_id: "project_0001".to_string(),
            name: "repo".to_string(),
            path: repo,
            default_policy_preset: Some("manual-write".to_string()),
            default_provider_mode: Some("fake".to_string()),
        })
        .expect("create repository");
    LifecycleStore::new(app_paths.clone())
        .create_work_item(CreateWorkItemInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: repository.id,
            story_spec_ids: Vec::new(),
            design_spec_ids: Vec::new(),
            title: "Coding work item".to_string(),
        })
        .expect("create work item");
    let store = CodingAttemptStore::new(app_paths);
    store
        .create_attempt(CreateCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: "work_item_0001".to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Fake,
                reviewer: Some(ProviderName::Fake),
                review_rounds: 1,
            },
            max_auto_rework: 2,
        })
        .expect("create attempt");
    build_web_router(WebAppState::new(
        root_path.to_path_buf(),
        WebRuntime::new_fake(root_path.to_path_buf()),
    ))
}

fn app_with_full_chain_attempt(root_path: &Path) -> axum::Router {
    let repo = root_path.join("repo");
    let remote = root_path.join("remote.git");
    init_cargo_repo(&repo);
    run_git(root_path, &["init", "--bare", remote.to_str().unwrap()]);
    run_git(
        &repo,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );

    let app_paths = ProductAppPaths::new(root_path.join(".aria"));
    let repository = RepositoryStore::new(app_paths.clone())
        .create(CreateRepositoryInput {
            project_id: "project_0001".to_string(),
            name: "repo".to_string(),
            path: repo,
            default_policy_preset: Some("manual-write".to_string()),
            default_provider_mode: Some("fake".to_string()),
        })
        .expect("create repository");
    let lifecycle = LifecycleStore::new(app_paths.clone());
    lifecycle
        .create_work_item(CreateWorkItemInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: repository.id,
            story_spec_ids: Vec::new(),
            design_spec_ids: Vec::new(),
            title: "实现爬楼梯".to_string(),
        })
        .expect("create work item");
    lifecycle
        .update_work_item_plan_status(
            "project_0001",
            "issue_0001",
            "work_item_0001",
            WorkItemPlanStatus::Confirmed,
        )
        .expect("confirm work item");
    CodingAttemptStore::new(app_paths)
        .create_attempt(CreateCodingAttemptInput {
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
        })
        .expect("create attempt");

    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::Fake, Arc::new(FullChainStreamingProvider));
    build_web_router(WebAppState::with_provider_registry(
        root_path.to_path_buf(),
        WebRuntime::new_fake(root_path.to_path_buf()),
        registry,
    ))
}

fn app_with_running_testing_attempt(root_path: &std::path::Path) -> axum::Router {
    let store = CodingAttemptStore::new(ProductAppPaths::new(root_path.join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: "work_item_0001".to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Fake,
                reviewer: Some(ProviderName::Fake),
                review_rounds: 1,
            },
            max_auto_rework: 2,
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
        .save_timeline_node(CodingTimelineNode {
            id: "coding_node_0001".to_string(),
            attempt_id: attempt.id,
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
    build_web_router(WebAppState::new(
        root_path.to_path_buf(),
        WebRuntime::new_fake(root_path.to_path_buf()),
    ))
}

fn app_with_final_confirm_attempt(root_path: &std::path::Path) -> axum::Router {
    let app_paths = ProductAppPaths::new(root_path.join(".aria"));
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
        .create_attempt(CreateCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: "work_item_0001".to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Fake,
                reviewer: Some(ProviderName::Fake),
                review_rounds: 1,
            },
            max_auto_rework: 2,
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
            attempt_id: attempt.id,
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
    build_web_router(WebAppState::new(
        root_path.to_path_buf(),
        WebRuntime::new_fake(root_path.to_path_buf()),
    ))
}

async fn send_json(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    message: &CodingWsInMessage,
) {
    ws.send(Message::Text(
        serde_json::to_string(message).unwrap().into(),
    ))
    .await
    .expect("send ws message");
}

async fn recv_json(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> CodingWsOutMessage {
    let message = timeout(Duration::from_secs(3), ws.next())
        .await
        .expect("ws message timeout")
        .expect("ws message")
        .expect("valid ws message");
    match message {
        Message::Text(text) => serde_json::from_str(&text).expect("ws json"),
        other => panic!("expected text ws message, got {other:?}"),
    }
}

fn init_cargo_repo(repo: &Path) {
    fs::create_dir_all(repo.join("src")).expect("create src");
    fs::write(
        repo.join("Cargo.toml"),
        "[package]\nname = \"coding-ws-full-chain\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .expect("write cargo manifest");
    fs::write(
        repo.join("src/lib.rs"),
        "pub fn climb_stairs(_n: u32) -> u32 { 0 }\n",
    )
    .expect("write lib");
    run_command(repo, "cargo", &["generate-lockfile"]);
    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "aria@example.com"]);
    run_git(repo, &["config", "user.name", "Aria Test"]);
    run_git(repo, &["add", "."]);
    run_git(repo, &["commit", "-m", "initial"]);
}

fn init_simple_git_repo(repo: &Path) {
    fs::create_dir_all(repo).expect("create repo");
    fs::write(repo.join("README.md"), "coding fixture\n").expect("write readme");
    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "aria@example.com"]);
    run_git(repo, &["config", "user.name", "Aria Test"]);
    run_git(repo, &["add", "."]);
    run_git(repo, &["commit", "-m", "initial"]);
}

fn run_git(cwd: &Path, args: &[&str]) {
    run_command(cwd, "git", args);
}

fn run_command(cwd: &Path, program: &str, args: &[&str]) {
    let output = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("run command");
    assert!(
        output.status.success(),
        "{program} {:?} failed\nstdout:{}\nstderr:{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

struct FullChainStreamingProvider;

#[async_trait::async_trait]
impl StreamingProviderAdapter for FullChainStreamingProvider {
    async fn run_streaming(
        &self,
        input: &AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        let (tx, rx) = mpsc::channel(8);
        match input.role {
            AdapterRole::Executor => {
                let worktree = input
                    .worktree_path
                    .as_ref()
                    .map(PathBuf::from)
                    .expect("worktree path");
                fs::write(worktree.join("src/lib.rs"), CLIMB_STAIRS_LIB).map_err(|error| {
                    ProviderAdapterError::incompatible_output(error.to_string(), "", "")
                })?;
                tx.try_send(StreamChunk::Text("implemented climb_stairs".to_string()))
                    .expect("send coding chunk");
                tx.try_send(StreamChunk::Done {
                    full_output: "implemented climb_stairs".to_string(),
                })
                .expect("send coding done");
            }
            AdapterRole::Reviewer => {
                tx.try_send(StreamChunk::Text("review approved".to_string()))
                    .expect("send review chunk");
                tx.try_send(StreamChunk::Done {
                    full_output: r#"{"verdict":"approve","summary":"review ok","findings":[]}"#
                        .to_string(),
                })
                .expect("send review done");
            }
            _ => {
                tx.try_send(StreamChunk::Done {
                    full_output: "ok".to_string(),
                })
                .expect("send done");
            }
        }
        Ok(rx)
    }
}

const CLIMB_STAIRS_LIB: &str = r#"pub fn climb_stairs(n: u32) -> u32 {
    if n <= 2 {
        return n;
    }
    let mut prev = 1;
    let mut curr = 2;
    for _ in 3..=n {
        let next = prev + curr;
        prev = curr;
        curr = next;
    }
    curr
}

#[cfg(test)]
mod tests {
    use super::climb_stairs;

    #[test]
    fn computes_climb_stairs_examples() {
        assert_eq!(climb_stairs(1), 1);
        assert_eq!(climb_stairs(2), 2);
        assert_eq!(climb_stairs(3), 3);
        assert_eq!(climb_stairs(5), 8);
        assert_eq!(climb_stairs(10), 89);
    }
}
"#;
