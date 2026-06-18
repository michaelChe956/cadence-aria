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
    app_with_confirmed_story_and_design, request_json, valid_split_output,
};

static WS_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

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
    let (app, repo) = app_with_confirmed_story_and_design(valid_split_output()).await;

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
            ..
        } => {
            assert_eq!(workspace_type, WorkspaceType::WorkItemPlan);
            assert_eq!(stage, "author_confirm");
            let candidate = match artifact {
                Some(ArtifactPayload::WorkItemPlanCandidate { candidate }) => candidate,
                other => panic!("expected WorkItemPlanCandidate artifact, got {other:?}"),
            };
            assert!(!candidate.work_items.is_empty());
            assert!(
                timeline_nodes
                    .iter()
                    .any(|n| n.node_type == TimelineNodeType::AuthorConfirm),
                "work_item_plan timeline should contain author_confirm node"
            );
        }
        other => panic!("expected SessionState, got {other:?}"),
    }
}

#[tokio::test]
async fn work_item_workspace_recovery_unaffected_or_covered() {
    let _guard = WS_TEST_LOCK.lock().await;
    let (app, repo) = app_with_confirmed_story_and_design(valid_split_output()).await;

    // 准备并确认 WorkItemPlan，产生子 WorkItem sessions
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
        msgs.iter()
            .any(|m| m["type"] == "stage_change" && m["stage"] == "author_confirm")
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

#[test]
fn session_state_serde_roundtrip_preserves_work_item_plan_candidate() {
    let candidate = WorkItemPlanCandidateDto {
        plan: WorkItemPlanDto {
            id: "plan_001".to_string(),
            status: "draft".to_string(),
            options: WorkItemSplitOptionsDto {
                include_integration_tests: true,
                include_e2e_tests: false,
                force_frontend_backend_split: true,
                require_execution_plan_confirm: false,
            },
            dependency_graph: vec![WorkItemDependencyEdgeDto {
                from_work_item_id: "wi_001".to_string(),
                to_work_item_id: "wi_002".to_string(),
            }],
        },
        work_items: vec![
            WorkItemCandidateDto {
                id: "wi_001".to_string(),
                kind: "backend".to_string(),
                title: "后端 API".to_string(),
                depends_on: vec![],
                exclusive_write_scopes: vec!["src/api".to_string()],
                verification_plan_ref: Some("vp_001".to_string()),
                meta: WorkItemCandidateMetaDto {
                    reverted: true,
                    revert_feedback: Some("拆得太粗".to_string()),
                },
            },
            WorkItemCandidateDto {
                id: "wi_002".to_string(),
                kind: "frontend".to_string(),
                title: "前端组件".to_string(),
                depends_on: vec!["wi_001".to_string()],
                exclusive_write_scopes: vec!["web/src".to_string()],
                verification_plan_ref: None,
                meta: WorkItemCandidateMetaDto {
                    reverted: false,
                    revert_feedback: None,
                },
            },
        ],
        verification_plans: vec![VerificationPlanDto {
            plan_ref: "vp_001".to_string(),
            scope: "unit".to_string(),
            commands: vec![VerificationCommandDto {
                label: "cargo test".to_string(),
                command: "cargo test".to_string(),
                cwd: "".to_string(),
                purpose: "unit tests".to_string(),
                required: true,
                timeout_seconds: 120,
                safety: "approved".to_string(),
            }],
            manual_checks: vec![VerificationManualCheckDto {
                label: "手工验证".to_string(),
                instructions: "运行并观察".to_string(),
                required: false,
            }],
            required_gates: vec![],
            risk_notes: vec![],
            confidence: "high".to_string(),
            fallback_policy: "manual_gate".to_string(),
        }],
        repository_profile: None,
        validator_findings: vec![ValidatorFindingDto {
            severity: "warning".to_string(),
            code: "SCOPE_OVERLAP".to_string(),
            message: "范围可能重叠".to_string(),
            work_item_ids: vec!["wi_001".to_string()],
        }],
    };

    let state = WsOutMessage::SessionState {
        session_id: "workspace_session_001".to_string(),
        workspace_type: WorkspaceType::WorkItemPlan,
        stage: "author_confirm".to_string(),
        superpowers_enabled: true,
        openspec_enabled: true,
        messages: vec![WsMessageDto {
            id: "msg_001".to_string(),
            role: "system".to_string(),
            content: "候选 work item plan 生成器".to_string(),
            checkpoint_id: None,
            created_at: "2026-06-17T00:00:00Z".to_string(),
        }],
        checkpoints: vec![WsCheckpointDto {
            id: "ckpt_001".to_string(),
            message_index: 1,
            stage: "author_confirm".to_string(),
            created_at: "2026-06-17T00:00:00Z".to_string(),
        }],
        artifact: Some(ArtifactPayload::WorkItemPlanCandidate {
            candidate: Box::new(candidate.clone()),
        }),
        providers: WsProviderConfig {
            author: cadence_aria::product::models::ProviderName::Fake,
            reviewer: Some(cadence_aria::product::models::ProviderName::Codex),
        },
        timeline_nodes: vec![TimelineNode {
            node_id: "node_001".to_string(),
            node_type: TimelineNodeType::AuthorConfirm,
            agent: None,
            stage: WorkspaceStage::AuthorConfirm,
            round: None,
            status: TimelineNodeStatus::Paused,
            title: "Author 结果确认".to_string(),
            summary: None,
            started_at: "2026-06-17T00:00:00Z".to_string(),
            completed_at: None,
            duration_ms: None,
            artifact_ref: Some("artifact_current".to_string()),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: cadence_aria::product::models::ProviderName::Fake,
                reviewer: Some(cadence_aria::product::models::ProviderName::Codex),
                review_rounds: 1,
            },
        }],
        active_node_id: Some("node_001".to_string()),
        artifact_versions: vec![],
        artifact_version_summaries: vec![],
        timeline_node_details: HashMap::new(),
        timeline_node_summaries: HashMap::new(),
        active_run_id: None,
    };

    let value = serde_json::to_value(&state).expect("serialize SessionState");
    let roundtrip: WsOutMessage = serde_json::from_value(value).expect("deserialize SessionState");

    match roundtrip {
        WsOutMessage::SessionState {
            artifact: Some(ArtifactPayload::WorkItemPlanCandidate { candidate: rt }),
            ..
        } => {
            assert_eq!(rt.work_items.len(), 2);
            let wi_001 = rt.work_items.iter().find(|w| w.id == "wi_001").unwrap();
            assert!(wi_001.meta.reverted);
            assert_eq!(wi_001.meta.revert_feedback, Some("拆得太粗".to_string()));
            assert_eq!(rt.verification_plans.len(), 1);
            assert_eq!(rt.validator_findings.len(), 1);
        }
        other => panic!("expected SessionState with WorkItemPlanCandidate, got {other:?}"),
    }
}

#[tokio::test]
async fn reconnect_preserves_revert_marks_from_current_artifact_version() {
    let _guard = WS_TEST_LOCK.lock().await;
    let (app, _repo) = app_with_confirmed_story_and_design(valid_split_output()).await;

    let (_status, prepare_resp) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-item-plans:prepare",
        json!({
            "title": "重连恢复 revert 标记测试",
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
    let session_id = prepare_resp["workspace_session"]["workspace_session_id"]
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

    let messages = recv_ws_until(&mut ws, Duration::from_secs(15), |msgs| {
        msgs.iter().any(|m| m["type"] == "artifact_update")
            && msgs
                .iter()
                .any(|m| m["type"] == "stage_change" && m["stage"] == "author_confirm")
    })
    .await;
    let first_work_item_id = messages
        .iter()
        .find(|m| m["type"] == "artifact_update")
        .unwrap()["candidate"]["work_items"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();

    ws.send(Message::Text(
        json!({
            "type": "revert_work_item",
            "work_item_id": first_work_item_id,
            "feedback": "拆得太粗",
            "clear": false
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send revert_work_item");

    let _messages = recv_ws_until(&mut ws, Duration::from_secs(5), |msgs| {
        msgs.iter().any(|m| m["type"] == "artifact_update")
    })
    .await;
    ws.close(None).await.ok();

    // 重连：服务端应发送 SessionState，其中当前 artifact version 保留 revert 标记
    let mut ws2 = connect_ws(app, &session_id).await;
    let state_messages = recv_ws_until(&mut ws2, Duration::from_secs(5), |msgs| {
        msgs.iter().any(|m| m["type"] == "session_state")
    })
    .await;
    let session_state = state_messages
        .iter()
        .find(|m| m["type"] == "session_state")
        .expect("session_state after reconnect");
    let reverted_item = session_state["artifact"]["candidate"]["work_items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["id"] == first_work_item_id)
        .expect("work item still in recovered candidate");
    assert_eq!(reverted_item["meta"]["reverted"], true);
    assert_eq!(reverted_item["meta"]["revert_feedback"], "拆得太粗");

    ws2.close(None).await.ok();
}
