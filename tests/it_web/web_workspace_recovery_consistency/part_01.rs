use axum::http::Method;
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::checkpoint_store::CheckpointStore;
use cadence_aria::product::lifecycle_store::LifecycleStore;
use cadence_aria::product::models::WorkspaceType;
use cadence_aria::product::workspace_engine::{EngineEvent, WorkspaceEngine, WorkspaceSession};
use cadence_aria::web::workspace_ws_types::{
    ArtifactPayload, ProviderConfigSnapshot, TimelineNode, TimelineNodeStatus, TimelineNodeType,
    ValidatorFindingDto, VerificationCommandDto, VerificationManualCheckDto, VerificationPlanDto,
    WorkItemCandidateDto, WorkItemCandidateMetaDto, WorkItemDependencyEdgeDto,
    WorkItemPlanCandidateDto, WorkItemPlanDto, WorkItemSplitOptionsDto, WorkspaceStage,
    WsCheckpointDto, WsMessageDto, WsOutMessage, WsProviderConfig,
};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::time::{Duration, timeout};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

use crate::web_work_item_generation::{
    app_with_confirmed_story_and_design, app_with_confirmed_story_and_design_and_streaming_outputs,
    request_json, valid_outline_output, valid_split_output,
};

static WS_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn enable_test_controls() -> crate::TestControlsEnvGuard {
    crate::enable_test_controls().await
}

async fn connect_ws(app: axum::Router, session_id: &str) -> WsStream {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/api/ws/workspace/{session_id}");
    let (ws, _) = connect_async(url).await.expect("connect ws");

    tokio::spawn(async move {
        server.await.ok();
    });

    ws
}

async fn recv_ws_until<F>(ws: &mut WsStream, timeout_after: Duration, predicate: F) -> Vec<Value>
where
    F: Fn(&[Value]) -> bool,
{
    let mut messages = Vec::new();
    let deadline = tokio::time::Instant::now() + timeout_after;
    while tokio::time::Instant::now() < deadline {
        let remaining = deadline - tokio::time::Instant::now();
        match timeout(remaining, ws.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => match serde_json::from_str(&text) {
                Ok(value) => {
                    messages.push(value);
                    if predicate(&messages) {
                        break;
                    }
                }
                Err(error) => panic!("ws json parse error: {error}\n{text}"),
            },
            Ok(Some(Ok(Message::Close(_)))) => break,
            Ok(Some(Ok(other))) => {
                eprintln!("ignoring non-text ws message: {other:?}");
                continue;
            }
            Ok(Some(Err(error))) => panic!("ws error: {error}"),
            Ok(None) => break,
            Err(_) => break,
        }
    }
    messages
}

async fn generate_session_to_author_confirm(
    app: &axum::Router,
    endpoint: &str,
    body: Value,
) -> String {
    let (_status, resp) = request_json(app.clone(), Method::POST, endpoint, body).await;
    let session_id = resp["workspace_session"]["workspace_session_id"]
        .as_str()
        .unwrap()
        .to_string();

    let mut ws = connect_ws(app.clone(), &session_id).await;
    ws.send(Message::Text(
        json!({
            "type": "start_generation",
            "provider_config": { "author": "fake", "reviewer": null, "review_rounds": 0 },
            "reviewer_enabled": false
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send start_generation");

    let _messages = recv_ws_until(&mut ws, Duration::from_secs(15), |msgs| {
        msgs.iter().any(|m| m["type"] == "artifact_update")
            && msgs
                .iter()
                .any(|m| m["type"] == "stage_change" && m["stage"] == "author_confirm")
    })
    .await;

    ws.close(None).await.ok();
    session_id
}

async fn enable_review_fixture(app: &axum::Router, session_id: &str, verdict: &str) {
    let (status, response) = request_json(
        app.clone(),
        Method::POST,
        &format!("/api/test/workspace-sessions/{session_id}/review-fixture"),
        json!({
            "verdict": verdict,
            "summary": "审核通过",
            "comments": "覆盖核心路径",
            "findings": []
        }),
    )
    .await;
    assert_eq!(
        status,
        axum::http::StatusCode::OK,
        "enable review fixture failed: {response}"
    );
}

fn recover_engine(repo: &tempfile::TempDir, session_id: &str) -> WorkspaceEngine {
    let app_paths = ProductAppPaths::new(repo.path().join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let checkpoint_store = Arc::new(CheckpointStore::new(
        app_paths.issue_lifecycle_root("project_0001", "issue_0001"),
    ));
    let (event_tx, _event_rx) = mpsc::channel::<EngineEvent>(1);
    let session_record = lifecycle
        .get_workspace_session(session_id)
        .expect("session record exists");
    let mut session = WorkspaceSession::from_record(session_record);
    session.repository_path = Some(repo.path().join("repo"));
    WorkspaceEngine::new_persistent(checkpoint_store, lifecycle, event_tx, session)
}

#[tokio::test]
async fn story_design_work_item_plan_recovery_consistency() {
    let _guard = WS_TEST_LOCK.lock().await;
    let (app, repo, _prompts) =
        app_with_confirmed_story_and_design_and_streaming_outputs(vec![valid_outline_output()])
            .await;

    // 生成新的 Story / Design spec 并运行到 author_confirm（不确认）
    let story_session_id = generate_session_to_author_confirm(
        &app,
        "/api/projects/project_0001/issues/issue_0001/story-specs:generate",
        json!({
            "title": "第二个 Story",
            "author_provider": "fake",
            "reviewer_provider": null,
            "review_rounds": 1,
            "superpowers_enabled": false,
            "openspec_enabled": true
        }),
    )
    .await;

    let design_session_id = generate_session_to_author_confirm(
        &app,
        "/api/projects/project_0001/issues/issue_0001/design-specs:generate",
        json!({
            "title": "第二个 Design",
            "story_spec_ids": ["story_spec_0001"],
            "author_provider": "fake",
            "reviewer_provider": null,
            "review_rounds": 1,
            "superpowers_enabled": false,
            "openspec_enabled": true
        }),
    )
    .await;

    // WorkItemPlan prepare + start_generation
    let (_status, prepare_resp) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-item-plans:prepare",
        json!({
            "title": "恢复一致性测试 Plan",
            "story_spec_ids": ["story_spec_0001"],
            "design_spec_ids": ["design_spec_0001"],
            "author_provider": "fake",
            "reviewer_provider": null,
            "review_rounds": 1,
            "superpowers_enabled": false,
            "openspec_enabled": true,
            "include_integration_tests": true,
            "include_e2e_tests": false,
            "force_frontend_backend_split": true,
            "require_execution_plan_confirm": false
        }),
    )
    .await;
    let plan_session_id = prepare_resp["workspace_session"]["workspace_session_id"]
        .as_str()
        .unwrap()
        .to_string();
    let mut ws = connect_ws(app, &plan_session_id).await;
    ws.send(Message::Text(
        json!({
            "type": "start_generation",
            "provider_config": { "author": "fake", "reviewer": null, "review_rounds": 0 },
            "reviewer_enabled": false
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send start_generation");
    let _messages = recv_ws_until(&mut ws, Duration::from_secs(15), |msgs| {
        msgs.iter().any(|m| m["type"] == "artifact_update")
            && msgs
                .iter()
                .any(|m| m["type"] == "stage_change" && m["stage"] == "author_confirm")
    })
    .await;
    ws.close(None).await.ok();

    // 恢复 Story session
    let story_engine = recover_engine(&repo, &story_session_id);
    let story_state = story_engine.build_session_state();
    match story_state {
        WsOutMessage::SessionState {
            workspace_type,
            stage,
            artifact,
            timeline_nodes,
            timeline_node_details,
            ..
        } => {
            assert_eq!(workspace_type, WorkspaceType::Story);
            assert_eq!(stage, "author_confirm");
            let markdown = artifact
                .as_ref()
                .and_then(|a| a.markdown())
                .expect("story artifact should be markdown");
            assert!(markdown.contains("# Story Spec"));
            assert!(
                timeline_nodes
                    .iter()
                    .any(|n| n.node_type == TimelineNodeType::AuthorConfirm),
                "story timeline should contain author_confirm node"
            );
            assert!(
                timeline_node_details.is_empty(),
                "story session_state should keep details lightweight and use summaries"
            );
        }
        other => panic!("expected SessionState, got {other:?}"),
    }

    // 恢复 Design session
    let design_engine = recover_engine(&repo, &design_session_id);
    let design_state = design_engine.build_session_state();
    match design_state {
        WsOutMessage::SessionState {
            workspace_type,
            stage,
            artifact,
            timeline_nodes,
            timeline_node_details,
            ..
        } => {
            assert_eq!(workspace_type, WorkspaceType::Design);
            assert_eq!(stage, "author_confirm");
            let markdown = artifact
                .as_ref()
                .and_then(|a| a.markdown())
                .expect("design artifact should be markdown");
            assert!(markdown.contains("# Design Spec"));
            assert!(
                timeline_nodes
                    .iter()
                    .any(|n| n.node_type == TimelineNodeType::AuthorConfirm),
                "design timeline should contain author_confirm node"
            );
            assert!(
                timeline_node_details.is_empty(),
                "design session_state should keep details lightweight and use summaries"
            );
        }
        other => panic!("expected SessionState, got {other:?}"),
    }

    // 恢复 WorkItemPlan session
    let plan_engine = recover_engine(&repo, &plan_session_id);
    let plan_state = plan_engine.build_session_state();
    match plan_state {
        WsOutMessage::SessionState {
            workspace_type,
            stage,
            artifact,
            timeline_nodes,
            timeline_node_details,
            ..
        } => {
            assert_eq!(workspace_type, WorkspaceType::WorkItemPlan);
            assert_eq!(stage, "author_confirm");
            let outline_candidate = match artifact {
                Some(ArtifactPayload::WorkItemPlanOutlineCandidate { outline_candidate }) => {
                    outline_candidate
                }
                other => panic!("expected WorkItemPlanOutlineCandidate artifact, got {other:?}"),
            };
            assert!(!outline_candidate.outline.work_item_outlines.is_empty());
            assert!(
                timeline_nodes
                    .iter()
                    .any(|n| n.node_type == TimelineNodeType::WorkItemPlanOutlineConfirm),
                "work_item_plan timeline should contain outline confirm node"
            );
            let progress_detail = timeline_node_details
                .values()
                .find(|detail| {
                    detail.node_type == TimelineNodeType::WorkItemPlanOutlineRun
                        && detail
                            .streaming_content
                            .contains("Fake Work Item Plan streaming draft")
                })
                .expect("work_item_plan outline details should include provider stream");
            assert!(
                timeline_nodes
                    .iter()
                    .any(|node| node.node_id == progress_detail.node_id
                        && node.node_type == TimelineNodeType::WorkItemPlanOutlineRun),
                "provider stream detail should belong to recovered outline_run node"
            );
            assert!(
                timeline_node_details.values().all(|detail| detail.node_type
                    != TimelineNodeType::StartGeneration
                    || detail.streaming_content.is_empty()),
                "start_generation should not restore WorkItemPlan provider stream"
            );
        }
        other => panic!("expected SessionState, got {other:?}"),
    }
}

#[tokio::test]
async fn story_workspace_review_sentinel_fallback_still_passes() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls().await;
    let (app, _repo, _prompts) =
        app_with_confirmed_story_and_design_and_streaming_outputs(vec![valid_outline_output()])
            .await;

    let (status, resp) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/story-specs:generate",
        json!({
            "title": "Reviewer fallback Story",
            "author_provider": "fake",
            "reviewer_provider": "codex",
            "review_rounds": 1,
            "superpowers_enabled": false,
            "openspec_enabled": true
        }),
    )
    .await;
    assert_eq!(
        status,
        axum::http::StatusCode::OK,
        "story generate failed: {resp}"
    );
    let session_id = resp["workspace_session"]["workspace_session_id"]
        .as_str()
        .expect("story session id")
        .to_string();

    let mut ws = connect_ws(app.clone(), &session_id).await;
    ws.send(Message::Text(
        json!({
            "type": "start_generation",
            "provider_config": { "author": "fake", "reviewer": "codex", "review_rounds": 1 },
            "reviewer_enabled": true
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send start_generation");
    let _messages = recv_ws_until(&mut ws, Duration::from_secs(15), |messages| {
        messages.iter().any(|message| {
            message["type"] == "stage_change" && message["stage"] == "author_confirm"
        })
    })
    .await;

    enable_review_fixture(&app, &session_id, "pass").await;
    ws.send(Message::Text(
        json!({ "type": "author_decision", "decision": "accept" })
            .to_string()
            .into(),
    ))
    .await
    .expect("send author accept");

    let messages = recv_ws_until(&mut ws, Duration::from_secs(15), |messages| {
        messages
            .iter()
            .any(|message| message["type"] == "review_complete")
            && messages.iter().any(|message| {
                message["type"] == "stage_change" && message["stage"] == "human_confirm"
            })
    })
    .await;
    let review_complete = messages
        .iter()
        .find(|message| message["type"] == "review_complete")
        .expect("story review complete");
    assert_eq!(review_complete["verdict"], "pass");
    assert!(
        review_complete
            .get("work_item_plan_review")
            .is_none_or(Value::is_null),
        "Story review_complete must not carry WorkItemPlan extension: {review_complete:?}"
    );
    assert!(
        messages
            .iter()
            .any(|message| message["type"] == "stage_change" && message["stage"] == "human_confirm"),
        "Story review fallback should enter human_confirm, got {messages:?}"
    );

    ws.close(None).await.ok();
}

#[tokio::test]
async fn design_workspace_artifact_history_still_loads_markdown() {
    let _guard = WS_TEST_LOCK.lock().await;
    let (app, repo, _prompts) =
        app_with_confirmed_story_and_design_and_streaming_outputs(vec![valid_outline_output()])
            .await;

    let design_session_id = generate_session_to_author_confirm(
        &app,
        "/api/projects/project_0001/issues/issue_0001/design-specs:generate",
        json!({
            "title": "Design artifact history",
            "story_spec_ids": ["story_spec_0001"],
            "author_provider": "fake",
            "reviewer_provider": null,
            "review_rounds": 1,
            "superpowers_enabled": false,
            "openspec_enabled": true
        }),
    )
    .await;

    let app_paths = ProductAppPaths::new(repo.path().join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths);
    let versions = lifecycle
        .list_artifact_versions(&design_session_id)
        .expect("list design artifact versions");
    assert!(
        versions
            .iter()
            .any(|version| version.is_current && version.markdown().contains("# Design Spec")),
        "Design artifact history should keep readable markdown versions: {versions:?}"
    );

    let design_engine = recover_engine(&repo, &design_session_id);
    match design_engine.build_session_state() {
        WsOutMessage::SessionState {
            workspace_type,
            artifact,
            artifact_versions,
            artifact_version_summaries,
            ..
        } => {
            assert_eq!(workspace_type, WorkspaceType::Design);
            assert!(artifact_versions.is_empty());
            assert!(
                artifact_version_summaries
                    .iter()
                    .any(|summary| summary.is_current && summary.markdown_size > 0),
                "SessionState should expose Design artifact history summaries"
            );
            let markdown = artifact
                .as_ref()
                .and_then(|payload| payload.markdown())
                .expect("recovered design artifact markdown");
            assert!(markdown.contains("# Design Spec"));
        }
        other => panic!("expected Design SessionState, got {other:?}"),
    }
}

#[tokio::test]
async fn ordinary_work_item_workspace_review_unaffected() {
    let _guard = WS_TEST_LOCK.lock().await;
    let (app, repo, _prompts) = app_with_confirmed_story_and_design_and_streaming_outputs(vec![
        valid_outline_output(),
        valid_draft_output("outline_backend_session"),
        valid_frontend_draft_output(),
        valid_integration_draft_output(),
    ])
    .await;

    // 准备并通过当前两阶段 batch compile 产生子 WorkItem sessions
    let (_status, prepare_resp) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-item-plans:prepare",
        json!({
            "title": "子 WorkItem 恢复测试 Plan",
            "story_spec_ids": ["story_spec_0001"],
            "design_spec_ids": ["design_spec_0001"],
            "author_provider": "fake",
            "reviewer_provider": null,
            "review_rounds": 1,
            "superpowers_enabled": false,
            "openspec_enabled": true,
            "include_integration_tests": true,
            "include_e2e_tests": false,
            "force_frontend_backend_split": true,
            "require_execution_plan_confirm": false
        }),
    )
    .await;
    let plan_session_id = prepare_resp["workspace_session"]["workspace_session_id"]
        .as_str()
        .unwrap()
        .to_string();

    let mut ws = connect_ws(app.clone(), &plan_session_id).await;
    ws.send(Message::Text(
        json!({
            "type": "start_generation",
            "provider_config": { "author": "fake", "reviewer": null, "review_rounds": 0 },
            "reviewer_enabled": false
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send start_generation");
    let _messages = recv_ws_until(&mut ws, Duration::from_secs(15), |msgs| {
        msgs.iter().any(|m| {
            m["type"] == "timeline_node_created"
                && m["node"]["node_type"] == "work_item_plan_outline_confirm"
        })
    })
    .await;

    ws.send(Message::Text(
        json!({ "type": "author_decision", "decision": "accept" })
            .to_string()
            .into(),
    ))
    .await
    .expect("send author_decision accept");
    let _messages = recv_ws_until(&mut ws, Duration::from_secs(10), |msgs| {
        msgs.iter().any(|m| {
            m["type"] == "timeline_node_created"
                && m["node"]["node_type"] == "work_item_generation_mode"
        })
    })
    .await;

    ws.send(Message::Text(
        json!({ "type": "select_work_item_generation_mode", "mode": "batch" })
            .to_string()
            .into(),
    ))
    .await
    .expect("send batch mode");
    let _messages = recv_ws_until(&mut ws, Duration::from_secs(10), |msgs| {
        msgs.iter().any(|m| {
            m["type"] == "timeline_node_created"
                && m["node"]["node_type"] == "work_item_batch_confirm"
        })
    })
    .await;

    ws.send(Message::Text(
        json!({
            "type": "work_item_batch_decision",
            "decision": "accept_all",
            "feedback": null,
            "first_affected_outline_id": null
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send batch accept_all");
    let _messages = recv_ws_until(&mut ws, Duration::from_secs(10), |msgs| {
        msgs.iter()
            .any(|m| m["type"] == "stage_change" && m["stage"] == "human_confirm")
    })
    .await;

    ws.send(Message::Text(
        json!({ "type": "human_confirm", "decision": "confirm" })
            .to_string()
            .into(),
    ))
    .await
    .expect("send human_confirm confirm");
    let _messages = recv_ws_until(&mut ws, Duration::from_secs(10), |msgs| {
        msgs.iter()
            .any(|m| m["type"] == "stage_change" && m["stage"] == "completed")
    })
    .await;
    ws.close(None).await.ok();

    let lifecycle = LifecycleStore::new(ProductAppPaths::new(repo.path().join(".aria")));
    let sessions = lifecycle
        .list_workspace_sessions("project_0001", "issue_0001")
        .unwrap();
    let work_item_session = sessions
        .into_iter()
        .find(|s| s.workspace_type == WorkspaceType::WorkItem)
        .expect("a child WorkItem session should exist");

    // 恢复 WorkItem session：它不应携带 WorkItemPlan candidate，也不应被 WP2a union 影响
    let engine = recover_engine(&repo, &work_item_session.id);
    let state = engine.build_session_state();
    match state {
        WsOutMessage::SessionState {
            workspace_type,
            stage,
            artifact,
            timeline_nodes,
            artifact_versions,
            ..
        } => {
            assert_eq!(workspace_type, WorkspaceType::WorkItem);
            assert_eq!(stage, "prepare_context");
            assert!(
                artifact.is_none(),
                "WorkItem workspace should not have a SessionState artifact"
            );
            assert!(
                artifact_versions.is_empty(),
                "WorkItem workspace should not have artifact versions"
            );
            assert!(
                timeline_nodes
                    .iter()
                    .all(|n| n.node_type != TimelineNodeType::AuthorConfirm),
                "WorkItem workspace timeline should not contain generic author_confirm node"
            );
        }
        other => panic!("expected SessionState, got {other:?}"),
    }
}

