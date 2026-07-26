use std::fs;

use axum::http::Method;
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::issue_store::IssueStore;
use cadence_aria::product::lifecycle_store::LifecycleStore;
use cadence_aria::product::models::{
    HumanPresentationRevision, IssueWorkItemPlanStatus, ProviderName, WorkItemDraftStatus,
    WorkItemGenerationMode, WorkItemPlanCommitState, WorkItemPlanCompileStatus, WorkspaceType,
};
use cadence_aria::product::work_item_plan_store::WorkItemPlanStore;
use cadence_aria::product::work_item_revision_store::WorkItemRevisionStore;
use cadence_aria::product::workspace_repository::workspace_repository_for_session;
use cadence_aria::web::workspace_context::ensure_workspace_context_message;
use cadence_aria::web::workspace_ws_types::{
    ArtifactPayload, ProviderConfigSnapshot, TimelineNode, TimelineNodeStatus, TimelineNodeType,
    WorkspaceStage as WsWorkspaceStage,
};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::time::{Duration, timeout};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

use crate::web_work_item_generation::{
    app_with_confirmed_story_and_design_and_streaming_outputs, request_json, valid_outline_output,
    valid_canonical_draft_output,
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
            Ok(Some(Ok(Message::Text(text)))) => {
                let value = serde_json::from_str(&text).expect("ws json");
                messages.push(value);
                if predicate(&messages) {
                    break;
                }
            }
            Ok(Some(Ok(Message::Close(_)))) => break,
            Ok(Some(Ok(other))) => panic!("expected text ws message, got {other:?}"),
            Ok(Some(Err(error))) => panic!("ws error: {error}"),
            Ok(None) => break,
            Err(_) => break,
        }
    }
    messages
}

async fn prepare_plan_accept_outline_and_select_batch(
    app: &axum::Router,
) -> (String, String, WsStream) {
    let (status, prepare_resp) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-item-plans:prepare",
        json!({
            "title": "登录拆分",
            "story_spec_ids": ["story_spec_0001"],
            "design_spec_ids": ["design_spec_0001"],
            "include_integration_tests": true,
            "include_e2e_tests": false,
            "force_frontend_backend_split": true,
            "require_execution_plan_confirm": false,
            "review_rounds": 1
        }),
    )
    .await;
    assert_eq!(
        status,
        axum::http::StatusCode::OK,
        "prepare failed: {prepare_resp}"
    );

    let session_id = prepare_resp["workspace_session"]["workspace_session_id"]
        .as_str()
        .unwrap()
        .to_string();
    let plan_id = prepare_resp["workspace_session"]["entity_id"]
        .as_str()
        .unwrap()
        .to_string();

    let mut ws = connect_ws(app.clone(), &session_id).await;
    ws.send(Message::Text(
        json!({
            "type": "start_generation",
            "provider_config": { "author": "fake", "reviewer": "codex", "review_rounds": 1 },
            "reviewer_enabled": false
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send start_generation");
    let _messages = recv_ws_until(&mut ws, Duration::from_secs(15), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_plan_outline_confirm"
        })
    })
    .await;

    ws.send(Message::Text(
        json!({ "type": "author_decision", "decision": "accept" })
            .to_string()
            .into(),
    ))
    .await
    .expect("send outline accept");
    let _messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_generation_mode"
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

    (session_id, plan_id, ws)
}

#[tokio::test]
async fn batch_accept_all_runs_final_compile_and_publishes_revision_entities() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls().await;
    let (app, root, _prompts) = app_with_confirmed_story_and_design_and_streaming_outputs(vec![
        valid_outline_output(),
        valid_draft_output("outline_backend_session"),
        valid_frontend_draft_output(),
        valid_integration_draft_output(),
    ])
    .await;
    let (_session_id, plan_id, mut ws) = prepare_plan_accept_outline_and_select_batch(&app).await;

    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths.clone());
    assert!(
        lifecycle
            .list_work_items("project_0001", "issue_0001")
            .expect("list work items before compile")
            .is_empty(),
        "Draft 阶段不能提前写入真实 WorkItem"
    );
    assert!(
        lifecycle
            .list_verification_plans("project_0001", "issue_0001")
            .expect("list verification plans before compile")
            .is_empty(),
        "Draft 阶段不能提前写入真实 VerificationPlan"
    );

    let _messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_batch_confirm"
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
    .expect("send batch accept all");

    let messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_plan_compile"
        }) && messages
            .iter()
            .any(|message| message["type"] == "stage_change" && message["stage"] == "human_confirm")
    })
    .await;
    assert!(
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_plan_compile"
        }),
        "accept_all should enter work_item_plan_compile, got {messages:?}"
    );

    let legacy_work_items = lifecycle
        .list_work_items("project_0001", "issue_0001")
        .expect("list work items after compile");
    let legacy_verification_plans = lifecycle
        .list_verification_plans("project_0001", "issue_0001")
        .expect("list verification plans after compile");
    let plan = lifecycle
        .get_issue_work_item_plan("project_0001", "issue_0001", &plan_id)
        .expect("get compiled plan");
    let child_sessions = lifecycle
        .list_workspace_sessions("project_0001", "issue_0001")
        .expect("list workspace sessions");
    let work_item_sessions: Vec<_> = child_sessions
        .iter()
        .filter(|session| session.workspace_type == WorkspaceType::WorkItem)
        .collect();

    assert!(legacy_work_items.is_empty());
    assert!(legacy_verification_plans.is_empty());
    assert_eq!(work_item_sessions.len(), 3);
    assert!(
        work_item_sessions
            .iter()
            .all(|session| session.work_item_runtime_binding.is_some()),
        "Final Compile 进入 human_confirm 前必须持久化每个 Work Item Session 的 RuntimeBinding"
    );
    assert!(
        work_item_sessions.iter().all(|session| {
            session.messages.first().is_some_and(|message| {
                message.role == "system" && message.content.contains("[work_item_context]")
            })
        }),
        "Final Compile 进入 human_confirm 前必须持久化每个 Work Item Session 的 Revision 上下文"
    );
    assert_eq!(plan.status, IssueWorkItemPlanStatus::Confirmed);
    assert_eq!(plan.work_item_ids.len(), 3);
    assert_eq!(plan.verification_plan_ids.len(), 3);
    assert_eq!(plan.dependency_graph.len(), 2);
    let revision_store = WorkItemRevisionStore::new(app_paths.clone());
    let lineage = revision_store
        .get_plan_lineage("project_0001", "issue_0001", &plan_id)
        .expect("load active plan lineage");
    let active_revision_id = lineage
        .active_revision_id
        .as_deref()
        .expect("active plan revision id");
    let plan_revision = revision_store
        .get_plan_revision(
            "project_0001",
            "issue_0001",
            &plan_id,
            active_revision_id,
        )
        .expect("load active plan revision");
    assert_eq!(plan_revision.revision_no, 1);
    assert_eq!(plan_revision.work_item_bindings.len(), 3);
    assert_eq!(
        revision_store
            .get_dependency_graph_revision(&lineage, &plan_revision.dependency_graph_revision_id)
            .expect("load dependency graph revision")
            .edges
            .len(),
        2
    );
    revision_store
        .get_plan_validation_report(&lineage, &plan_revision.validation_report_ref)
        .expect("load plan validation report");
    let plan_projection = revision_store
        .get_plan_projection_bundle(&lineage, &plan_revision.plan_projection_bundle_id)
        .expect("load plan projection bundle");
    assert_eq!(
        plan.work_item_ids,
        plan_projection
            .coder_group_context
            .ordered_logical_work_item_ids
    );
    for logical_id in &plan.work_item_ids {
        let revision_id = plan_revision
            .work_item_bindings
            .get(logical_id)
            .expect("stable work item binding");
        let work_item_revision = revision_store
            .get_work_item_revision(&lineage, logical_id, revision_id)
            .expect("load work item revision");
        revision_store
            .get_verification_plan_revision(
                &lineage,
                &work_item_revision.verification_plan_revision_id,
            )
            .expect("load verification plan revision");
        revision_store
            .get_work_item_projection_bundle(
                &lineage,
                &work_item_revision.work_item_projection_bundle_id,
            )
            .expect("load work item projection bundle");
    }
    assert_eq!(
        work_item_sessions
            .iter()
            .map(|session| session.entity_id.as_str())
            .collect::<std::collections::BTreeSet<_>>(),
        plan.work_item_ids
            .iter()
            .map(String::as_str)
            .collect::<std::collections::BTreeSet<_>>()
    );
    let issue = IssueStore::new(app_paths.clone())
        .get("project_0001", "issue_0001")
        .expect("load issue for runtime repository");
    let expected_repository_id = issue.repo_id.expect("issue repository id");
    for session in &work_item_sessions {
        let binding = session
            .work_item_runtime_binding
            .as_ref()
            .expect("runtime binding");
        let projection_bundle = revision_store
            .get_work_item_projection_bundle(&lineage, &binding.projection_bundle_id)
            .expect("load human projection bundle");
        let context = &session.messages[0].content;
        assert!(context.contains(&format!("plan_id: {}", binding.plan_id)));
        assert!(context.contains(&format!(
            "plan_revision_id: {}",
            binding.plan_revision_id
        )));
        assert!(context.contains(&format!(
            "work_item_revision_id: {}",
            binding.work_item_revision_id
        )));
        assert!(context.contains(&format!(
            "projection_bundle_id: {}",
            binding.projection_bundle_id
        )));
        assert!(context.contains(&format!(
            "verification_plan_revision_id: {}",
            binding.verification_plan_revision_id
        )));
        assert!(context.contains(&format!(
            "human_projection_hash: {}",
            binding.human_projection_hash
        )));
        assert!(context.contains(&format!(
            "title: {}",
            projection_bundle.human_projection.title
        )));
        assert!(context.contains(&format!(
            "goal: {}",
            projection_bundle.human_projection.goal
        )));
        let verification_plan = revision_store
            .get_verification_plan_revision(&lineage, &binding.verification_plan_revision_id)
            .expect("load revision verification plan");
        assert!(context.contains("[verification_checks]"));
        for check in &verification_plan.verification_checks {
            assert!(context.contains(&check.check_id));
        }
        for forbidden in [
            "[work_item_plan_source]",
            "canonical_contract",
            "coder_projection",
            "reviewer_projection",
            "criterion_refs",
        ] {
            assert!(
                !context.contains(forbidden),
                "Work Item Human Context must not expose {forbidden}: {context}"
            );
        }
        assert_eq!(
            workspace_repository_for_session(&app_paths, &lifecycle, session)
                .expect("Work Item RuntimeBinding must resolve repository without Legacy Work Item")
                .id,
            expected_repository_id
        );
    }
    let presentation_session = (*work_item_sessions[0]).clone();
    let original_message_count = presentation_session.messages.len();
    let presentation_binding = presentation_session
        .work_item_runtime_binding
        .as_ref()
        .expect("presentation runtime binding");
    revision_store
        .put_human_presentation_revision(
            &lineage,
            &HumanPresentationRevision {
                id: "human_presentation_revision_0001".to_string(),
                source_plan_projection_bundle_id: None,
                source_work_item_projection_bundle_id: Some(
                    presentation_binding.projection_bundle_id.clone(),
                ),
                supersedes: None,
                human_summary: "先完成库导出，再由服务与界面消费。".to_string(),
                why_split: Some("降低并行修改冲突。".to_string()),
                dependency_explanation: vec!["服务依赖库导出的稳定接口。".to_string()],
                risk_explanation: vec!["变更公共导出时必须保留兼容性。".to_string()],
                source_refs: vec!["design_spec_0001".to_string()],
                normative: false,
                used_by_provider: false,
                created_at: "2026-07-26T00:00:00Z".to_string(),
            },
        )
        .expect("save human presentation");
    let refreshed_session = ensure_workspace_context_message(
        &app_paths,
        &lifecycle,
        presentation_session,
    )
    .expect("refresh Work Item human context");
    assert_eq!(
        refreshed_session.messages.len(),
        original_message_count,
        "refreshing Revision-backed context must replace the existing system message"
    );
    let refreshed_context = &refreshed_session.messages[0].content;
    for expected in [
        "human_presentation_id: human_presentation_revision_0001",
        "human_summary: 先完成库导出，再由服务与界面消费。",
        "why_split: 降低并行修改冲突。",
        "dependency_explanation: [服务依赖库导出的稳定接口。]",
        "risk_explanation: [变更公共导出时必须保留兼容性。]",
    ] {
        assert!(refreshed_context.contains(expected), "missing {expected}");
    }

    let store = WorkItemPlanStore::new(app_paths);
    let index = store
        .load_active_index("project_0001", "issue_0001", &plan_id)
        .expect("load active index")
        .expect("active index");
    let compile_dir = root
        .path()
        .join(".aria/projects/project_0001/issues/issue_0001/work_item_plan_compiles")
        .join(&plan_id);
    let compile_files: Vec<_> = fs::read_dir(&compile_dir)
        .expect("read compile tx dir")
        .collect::<Result<Vec<_>, _>>()
        .expect("compile dir entries");
    assert_eq!(compile_files.len(), 1);
    let compile_tx: Value =
        serde_json::from_slice(&fs::read(compile_files[0].path()).expect("read compile tx"))
            .expect("compile tx json");
    assert_eq!(compile_tx["status"], "committed");
    assert_eq!(compile_tx["plan_commit_state"], "committed");
    assert_eq!(compile_tx["created_work_item_ids"], json!([]));
    assert_eq!(compile_tx["created_verification_plan_ids"], json!([]));
    assert_eq!(compile_tx["previous_plan_snapshot"]["status"], "draft");
    assert_eq!(
        compile_tx["active_draft_ids"]
            .as_array()
            .expect("draft ids")
            .len(),
        index.outline_to_current_draft_id.len()
    );

    ws.send(Message::Text(
        json!({ "type": "human_confirm", "decision": "confirm" })
            .to_string()
            .into(),
    ))
    .await
    .expect("send final human confirm");
    let completed_messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages
            .iter()
            .any(|message| message["type"] == "stage_change" && message["stage"] == "completed")
    })
    .await;
    assert!(
        completed_messages
            .iter()
            .any(|message| message["type"] == "stage_change" && message["stage"] == "completed"),
        "final human confirm after compile should complete workspace, got {completed_messages:?}"
    );
    let work_item_sessions_after_confirm = lifecycle
        .list_workspace_sessions("project_0001", "issue_0001")
        .expect("list workspace sessions after final confirm")
        .into_iter()
        .filter(|session| session.workspace_type == WorkspaceType::WorkItem)
        .collect::<Vec<_>>();
    assert_eq!(
        work_item_sessions_after_confirm.len(),
        3,
        "final human confirm must not create duplicate WorkItem sessions"
    );

    ws.close(None).await.ok();
}

#[tokio::test]
async fn strict_validator_item_failure_in_batch_returns_batch_confirm_without_real_writes() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls().await;
    let (app, root, _prompts) = app_with_confirmed_story_and_design_and_streaming_outputs(vec![
        valid_outline_output(),
        unsafe_backend_draft_output(),
        valid_frontend_draft_output(),
        valid_integration_draft_output(),
    ])
    .await;
    let (_session_id, plan_id, mut ws) = prepare_plan_accept_outline_and_select_batch(&app).await;

    let _messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_batch_confirm"
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
    .expect("send batch accept all");

    let messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_batch_confirm"
        })
    })
    .await;
    assert!(
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_batch_confirm"
        }),
        "strict validator item failure in batch should return batch_confirm, got {messages:?}"
    );

    let lifecycle = LifecycleStore::new(ProductAppPaths::new(root.path().join(".aria")));
    assert!(
        lifecycle
            .list_work_items("project_0001", "issue_0001")
            .expect("list work items after failed compile")
            .is_empty(),
        "failed strict validation must not write real WorkItem records"
    );
    assert!(
        lifecycle
            .list_verification_plans("project_0001", "issue_0001")
            .expect("list verification plans after failed compile")
            .is_empty(),
        "failed strict validation must not write real VerificationPlan records"
    );

    let compile_dir = root
        .path()
        .join(".aria/projects/project_0001/issues/issue_0001/work_item_plan_compiles")
        .join(&plan_id);
    let compile_files: Vec<_> = fs::read_dir(&compile_dir)
        .expect("read compile tx dir")
        .collect::<Result<Vec<_>, _>>()
        .expect("compile dir entries");
    assert_eq!(compile_files.len(), 1);
    let compile_tx: Value =
        serde_json::from_slice(&fs::read(compile_files[0].path()).expect("read compile tx"))
            .expect("compile tx json");
    assert_eq!(compile_tx["status"], "failed");
    assert_eq!(compile_tx["plan_commit_state"], "not_started");
    assert!(
        compile_tx["validator_findings"]
            .as_array()
            .expect("validator findings")
            .iter()
            .any(|finding| finding["code"] == "verification_command_unsafe"),
        "failed compile tx should record unsafe command finding: {compile_tx:?}"
    );

    ws.send(Message::Text(
        json!({
            "type": "work_item_batch_decision",
            "decision": "downgrade_to_serial",
            "feedback": "逐项修复 unsafe command",
            "first_affected_outline_id": "outline_backend_session"
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send downgrade to serial");
    let downgrade_messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_draft_run"
        })
    })
    .await;
    assert!(
        downgrade_messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_draft_run"
                && message["node"]["summary"]
                    .as_str()
                    .is_some_and(|summary| summary.contains("outline_backend_session"))
        }),
        "downgrade_to_serial after strict validation failure should start serial draft run, got {downgrade_messages:?}"
    );

    ws.close(None).await.ok();
}

#[tokio::test]
async fn downgrade_to_serial_copies_unaffected_batch_drafts_and_revalidates() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls().await;
    let (app, root, _prompts) = app_with_confirmed_story_and_design_and_streaming_outputs(vec![
        valid_outline_output(),
        valid_draft_output("outline_backend_session"),
        unsafe_frontend_draft_output(),
        valid_integration_draft_output(),
    ])
    .await;
    let (_session_id, plan_id, mut ws) = prepare_plan_accept_outline_and_select_batch(&app).await;

    let _messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_batch_confirm"
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
    .expect("send batch accept all");

    let _failed_compile_messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_batch_confirm"
        })
    })
    .await;

    let store = WorkItemPlanStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let index_before = store
        .load_active_index("project_0001", "issue_0001", &plan_id)
        .expect("load active index before downgrade")
        .expect("active index before downgrade");
    let source_frontend_draft_id = index_before
        .outline_to_current_draft_id
        .get("outline_frontend_expiry")
        .expect("frontend batch draft id")
        .clone();
    let source_integration_draft_id = index_before
        .outline_to_current_draft_id
        .get("outline_integration_session")
        .expect("integration batch draft id")
        .clone();

    ws.send(Message::Text(
        json!({
            "type": "work_item_batch_decision",
            "decision": "downgrade_to_serial",
            "feedback": "从前端项开始逐项修复",
            "first_affected_outline_id": "outline_frontend_expiry"
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send downgrade to serial");
    let downgrade_messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_draft_run"
                && message["node"]["summary"]
                    .as_str()
                    .is_some_and(|summary| summary.contains("outline_frontend_expiry"))
        })
    })
    .await;
    assert!(
        downgrade_messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_draft_run"
                && message["node"]["summary"]
                    .as_str()
                    .is_some_and(|summary| summary.contains("outline_frontend_expiry"))
        }),
        "downgrade_to_serial should start from first affected outline, got {downgrade_messages:?}"
    );

    let index_after = store
        .load_active_index("project_0001", "issue_0001", &plan_id)
        .expect("load active index after downgrade")
        .expect("active index after downgrade");
    assert_eq!(
        index_after.active_outline_id.as_deref(),
        Some("outline_frontend_expiry")
    );

    let copied_backend_draft_id = index_after
        .outline_to_current_draft_id
        .get("outline_backend_session")
        .expect("copied backend serial draft id");
    assert_ne!(
        copied_backend_draft_id,
        index_before
            .outline_to_current_draft_id
            .get("outline_backend_session")
            .expect("source backend draft id")
    );
    let copied_backend = store
        .get_draft_record(
            "project_0001",
            "issue_0001",
            &plan_id,
            &index_after.current_generation_round_id,
            copied_backend_draft_id,
        )
        .expect("load copied backend draft");
    assert_eq!(
        copied_backend.generation_mode,
        WorkItemGenerationMode::Serial
    );
    assert_eq!(copied_backend.batch_id, None);
    assert_eq!(copied_backend.status, WorkItemDraftStatus::Accepted);
    assert!(copied_backend.active);
    assert_eq!(
        copied_backend.copied_from_draft_id.as_deref(),
        index_before
            .outline_to_current_draft_id
            .get("outline_backend_session")
            .map(String::as_str)
    );

    assert_eq!(
        index_after
            .outline_to_current_draft_id
            .get("outline_frontend_expiry"),
        Some(&source_frontend_draft_id),
        "affected outline should be regenerated, not copied before serial run"
    );
    assert_eq!(
        index_after
            .outline_to_current_draft_id
            .get("outline_integration_session"),
        Some(&source_integration_draft_id),
        "downstream outline should remain available until its serial turn supersedes it"
    );

    ws.close(None).await.ok();
}
