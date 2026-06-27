use cadence_aria::cross_cutting::provider_adapter::ProviderAdapterError;
use cadence_aria::cross_cutting::provider_registry::ProviderRegistry;
use cadence_aria::cross_cutting::streaming_provider::{
    ProviderEvent, ProviderSession, StreamChunk, StreamingProviderAdapter, StreamingProviderInput,
};
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::coding_attempt_store::{
    CodingAttemptStore, CreateBlockedGateInput, CreateCodingAttemptInput,
    CreateCodingExecutionUnitInput, CreateGroupCodingAttemptInput,
};
use cadence_aria::product::coding_models::{
    AnalystDecisionNextStage, AnalystDecisionRecord, AnalystDecisionVerdict, CodingAgentRole,
    CodingAttemptStatus, CodingEntryType, CodingExecutionStage, CodingGateAction,
    CodingGateActionType, CodingGateKind, CodingGateRequired, CodingProviderPermissionMode,
    CodingProviderRole, CodingRoleProviderConfigSnapshot, CodingRoleRunEventType,
    CodingRoleRunStatus, CodingRoleRunTrigger, CodingTimelineNode, CodingTimelineNodeStatus,
    CodingExecutionUnitStatus, PushStatus, RemoteKind, ReviewRequest, ReviewRequestKind,
    ReviewVerdict, TestingOverallStatus, WorkItemExecutionPlan,
};
use cadence_aria::product::lifecycle_store::{
    CreateIssueWorkItemPlanInput, CreateWorkItemInput, CreateWorkspaceSessionInput,
    LifecycleStore,
};
use cadence_aria::product::models::WorkItemExecutionPlanStatus;
use cadence_aria::product::models::WorkItemStatus;
use cadence_aria::product::models::{
    IssueWorkItemPlanOptions, IssueWorkItemPlanStatus, ProviderName, WorkItemPlanStatus,
    WorkspaceSessionStatus, WorkspaceType,
};
use cadence_aria::product::repository_store::{CreateRepositoryInput, RepositoryStore};
use cadence_aria::protocol::contracts::{AdapterInput, AdapterRole};
use cadence_aria::web::app::build_web_router;
use cadence_aria::web::coding_ws_handler::{
    CodingWsInMessage, CodingWsOutMessage, is_coding_ws_message_allowed,
};
use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::state::WebAppState;
use cadence_aria::web::workspace_ws_types::{
    ArtifactPayload, ArtifactVersion, ProviderConfigSnapshot,
};
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
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

    let provider_update = CodingWsOutMessage::CodingProviderConfigUpdated {
        role: CodingProviderRole::Coder,
        provider: ProviderName::Codex,
    };

    assert_eq!(
        serde_json::to_value(provider_update).expect("serialize provider update"),
        json!({
            "type": "coding_provider_config_updated",
            "role": "coder",
            "provider": "codex"
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

    let provider_select: CodingWsInMessage = serde_json::from_value(json!({
        "type": "provider_select",
        "role": "author",
        "provider": "codex"
    }))
    .expect("deserialize provider select");

    assert_eq!(
        provider_select,
        CodingWsInMessage::ProviderSelect {
            role: "author".to_string(),
            provider: ProviderName::Codex
        }
    );

    let stage_gate_confirm: CodingWsInMessage = serde_json::from_value(json!({
        "type": "stage_gate_confirm",
        "stage": "testing"
    }))
    .expect("deserialize stage gate confirm");

    assert_eq!(
        stage_gate_confirm,
        CodingWsInMessage::StageGateConfirm {
            stage: CodingExecutionStage::Testing,
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
    assert!(is_coding_ws_message_allowed(
        &CodingAttemptStatus::Running,
        &CodingExecutionStage::Testing,
        &CodingWsInMessage::ContextNote {
            content: "补充背景".to_string()
        },
    ));
    assert!(is_coding_ws_message_allowed(
        &CodingAttemptStatus::WaitingForHuman,
        &CodingExecutionStage::FinalConfirm,
        &CodingWsInMessage::ContextNote {
            content: "最终确认前补充背景".to_string()
        },
    ));
    assert!(is_coding_ws_message_allowed(
        &CodingAttemptStatus::Blocked,
        &CodingExecutionStage::CodeReview,
        &CodingWsInMessage::ContextNote {
            content: "阻塞时补充背景".to_string()
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
    assert!(is_coding_ws_message_allowed(
        &CodingAttemptStatus::Created,
        &CodingExecutionStage::PrepareContext,
        &CodingWsInMessage::ProviderSelect {
            role: "author".to_string(),
            provider: ProviderName::Codex,
        },
    ));
    assert!(is_coding_ws_message_allowed(
        &CodingAttemptStatus::Running,
        &CodingExecutionStage::Testing,
        &CodingWsInMessage::ProviderSelect {
            role: "author".to_string(),
            provider: ProviderName::Codex,
        },
    ));
    assert!(is_coding_ws_message_allowed(
        &CodingAttemptStatus::Running,
        &CodingExecutionStage::Testing,
        &CodingWsInMessage::StageGateConfirm {
            stage: CodingExecutionStage::Testing,
        },
    ));
}

#[test]
fn blocked_attempt_allows_gate_response_messages() {
    assert!(is_coding_ws_message_allowed(
        &CodingAttemptStatus::Blocked,
        &CodingExecutionStage::Testing,
        &CodingWsInMessage::GateResponse {
            gate_id: "coding_blocked_gate_0001".to_string(),
            action_id: "retry_test_plan".to_string(),
            extra_context: None,
        },
    ));
    assert!(is_coding_ws_message_allowed(
        &CodingAttemptStatus::Blocked,
        &CodingExecutionStage::Testing,
        &CodingWsInMessage::AbortAttempt,
    ));
}

#[test]
fn waiting_for_human_rework_allows_blocked_gate_responses() {
    assert!(is_coding_ws_message_allowed(
        &CodingAttemptStatus::WaitingForHuman,
        &CodingExecutionStage::Rework,
        &CodingWsInMessage::GateResponse {
            gate_id: "coding_blocked_gate_0007".to_string(),
            action_id: "retry_analyst".to_string(),
            extra_context: None,
        },
    ));
    assert!(is_coding_ws_message_allowed(
        &CodingAttemptStatus::WaitingForHuman,
        &CodingExecutionStage::Testing,
        &CodingWsInMessage::GateResponse {
            gate_id: "coding_blocked_gate_0008".to_string(),
            action_id: "accept_testing_result".to_string(),
            extra_context: None,
        },
    ));
}

#[test]
fn coding_gate_required_out_message_preserves_action_contract() {
    let gate = CodingGateRequired {
        gate_id: "gate_0001".to_string(),
        kind: CodingGateKind::Blocked,
        title: "需要人工决策".to_string(),
        description: "测试失败次数达到上限".to_string(),
        stage: None,
        role: None,
        expires_at: None,
        provider_snapshot: None,
        available_actions: vec![CodingGateAction {
            action_id: "accept_risk".to_string(),
            label: "接受风险".to_string(),
            action_type: CodingGateActionType::AcceptRisk,
        }],
        reason_code: None,
        evidence_refs: Vec::new(),
        raw_provider_output_ref: None,
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
            role_provider_config_snapshot,
            timeline_nodes,
            testing_report,
            ..
        } => {
            assert_eq!(attempt_id, "coding_attempt_0001");
            assert_eq!(status, CodingAttemptStatus::Created);
            assert_eq!(stage, CodingExecutionStage::PrepareContext);
            assert_eq!(branch_name, "aria/work-items/work_item_0001/attempt-1");
            assert_eq!(role_provider_config_snapshot.coder, ProviderName::Fake);
            assert_eq!(
                role_provider_config_snapshot.code_reviewer,
                ProviderName::Fake
            );
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
async fn coding_session_snapshot_includes_role_runs() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let app = app_with_attempt(root.path());
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("attempt");
    let run = store
        .create_role_run(
            &attempt,
            CodingExecutionStage::Testing,
            CodingProviderRole::Tester,
            CodingRoleRunTrigger::Initial,
            Some("coding_node_0003".to_string()),
        )
        .expect("role run");
    store
        .append_role_run_event(
            &attempt,
            &run,
            CodingRoleRunEventType::ProviderPrompt,
            serde_json::json!({
                "mode": "plan_tests",
                "prompt": "plan tests"
            }),
        )
        .expect("prompt event");
    store
        .append_role_run_event(
            &attempt,
            &run,
            CodingRoleRunEventType::ExecutionEvent,
            serde_json::json!({
                "title": "Task update",
                "status": "running",
                "detail": "No tasks found"
            }),
        )
        .expect("execution event");
    store
        .append_role_run_event(
            &attempt,
            &run,
            CodingRoleRunEventType::Aborted,
            serde_json::json!({
                "reason": "abort_attempt"
            }),
        )
        .expect("aborted event");
    let provider_failed_detail = format!("provider failed detail: {}", "d".repeat(16_500));
    let provider_failed_detail_preview = provider_failed_detail[..16_384].to_string();
    let provider_failed_message = format!("provider failed: {}", "x".repeat(16_500));
    let provider_failed_preview = provider_failed_message[..16_384].to_string();
    store
        .append_role_run_event(
            &attempt,
            &run,
            CodingRoleRunEventType::ProviderFailed,
            serde_json::json!({
                "detail": provider_failed_detail,
                "message": provider_failed_message
            }),
        )
        .expect("provider failed event");
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/ws/coding-attempts/coding_attempt_0001");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");

    let raw_state = recv_json_value(&mut ws).await;
    assert_eq!(raw_state["role_runs"][0]["role"], "tester");
    assert!(raw_state["role_runs"][0].get("run").is_none());

    match serde_json::from_value(raw_state).expect("coding session state") {
        CodingWsOutMessage::CodingSessionState { role_runs, .. } => {
            assert_eq!(role_runs.len(), 1);
            assert_eq!(role_runs[0].role, CodingProviderRole::Tester);
            assert_eq!(role_runs[0].run_no, 1);
            assert_eq!(role_runs[0].node_id.as_deref(), Some("coding_node_0003"));
            let summary = role_runs[0].event_summary.as_ref().expect("event summary");
            assert_eq!(summary.event_count, 4);
            assert_eq!(
                summary.last_event_type,
                Some(CodingRoleRunEventType::ProviderFailed)
            );
            assert_eq!(summary.last_event_title.as_deref(), Some("ProviderFailed"));
            assert_eq!(summary.last_event_status.as_deref(), None);
            assert_eq!(
                summary.terminal_event_type,
                Some(CodingRoleRunEventType::ProviderFailed)
            );
            assert_eq!(
                summary.terminal_reason.as_deref(),
                Some(provider_failed_preview.as_str())
            );
            assert_eq!(role_runs[0].recent_events.len(), 4);
            assert_eq!(
                role_runs[0].recent_events[1].title.as_deref(),
                Some("Task update")
            );
            assert_eq!(
                role_runs[0].recent_events[3].detail.as_deref(),
                Some(provider_failed_detail_preview.as_str())
            );
            assert!(role_runs[0].recent_events[3].truncated);
        }
        other => panic!("expected coding session state, got {other:?}"),
    }

    ws.close(None).await.expect("close ws");
    server.abort();
}

#[tokio::test]
async fn coding_session_snapshot_ignores_corrupt_role_run_events() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let app = app_with_attempt(root.path());
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("attempt");
    let run = store
        .create_role_run(
            &attempt,
            CodingExecutionStage::Testing,
            CodingProviderRole::Tester,
            CodingRoleRunTrigger::Initial,
            Some("coding_node_0003".to_string()),
        )
        .expect("role run");
    store
        .append_role_run_event(
            &attempt,
            &run,
            CodingRoleRunEventType::ExecutionEvent,
            serde_json::json!({
                "title": "Task update",
                "status": "running",
                "detail": "No tasks found"
            }),
        )
        .expect("execution event");
    let event_log = root
        .path()
        .join(".aria/projects/project_0001/issues/issue_0001/coding-attempts/coding_attempt_0001/role-run-events")
        .join(format!("{}.jsonl", run.id));
    fs::write(&event_log, "{not valid jsonl\n").expect("corrupt role run event log");

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/ws/coding-attempts/coding_attempt_0001");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");

    match recv_json(&mut ws).await {
        CodingWsOutMessage::CodingSessionState { role_runs, .. } => {
            assert_eq!(role_runs.len(), 1);
            assert_eq!(role_runs[0].role, CodingProviderRole::Tester);
            assert!(role_runs[0].event_summary.is_none());
            assert!(role_runs[0].recent_events.is_empty());
        }
        other => panic!("expected coding session state, got {other:?}"),
    }

    ws.close(None).await.expect("close ws");
    server.abort();
}

#[tokio::test]
async fn coding_ws_session_state_includes_persisted_open_stage_gates() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let app = app_with_attempt(root.path());
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    store
        .create_stage_gate(
            "coding_attempt_0001",
            CodingExecutionStage::Testing,
            CodingProviderRole::Tester,
            "2026-05-28T00:00:05Z".to_string(),
            CodingRoleProviderConfigSnapshot::from(ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::Fake),
                review_rounds: 1,
            }),
        )
        .expect("create stage gate");
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/ws/coding-attempts/coding_attempt_0001");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");

    match recv_json(&mut ws).await {
        CodingWsOutMessage::CodingSessionState { pending_gates, .. } => {
            assert_eq!(pending_gates.len(), 1);
            assert_eq!(pending_gates[0].gate_id, "coding_stage_gate_0001");
            assert_eq!(pending_gates[0].kind, CodingGateKind::StageGate);
            assert_eq!(pending_gates[0].stage, Some(CodingExecutionStage::Testing));
            assert_eq!(pending_gates[0].role, Some(CodingProviderRole::Tester));
            assert_eq!(
                pending_gates[0].expires_at.as_deref(),
                Some("2026-05-28T00:00:05Z")
            );
            assert!(pending_gates[0].title.contains("Testing"));
            assert_eq!(
                pending_gates[0].available_actions[0].action_type,
                CodingGateActionType::ConfirmStage
            );
        }
        other => panic!("expected coding session state, got {other:?}"),
    }

    ws.close(None).await.expect("close ws");
    server.abort();
}

#[tokio::test]
async fn coding_ws_session_state_includes_latest_analyst_decision() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let app = app_with_attempt(root.path());
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    store
        .save_analyst_decision(&AnalystDecisionRecord {
            id: "analyst_decision_0001".to_string(),
            attempt_id: "coding_attempt_0001".to_string(),
            source_stage: CodingExecutionStage::Testing,
            rework_round: 1,
            verdict: AnalystDecisionVerdict::NeedsFix,
            next_stage: AnalystDecisionNextStage::Coding,
            reason: "required 测试步骤被跳过，需要回到 Coder".to_string(),
            evidence_refs: vec!["testing_report_0001.json".to_string()],
            raw_provider_output_refs: vec![
                "provider-raw/testing/execute_test_plan_0001.txt".to_string(),
            ],
            rework_instructions: None,
            human_gate: None,
            created_at: "2026-06-12T00:00:00Z".to_string(),
            parse_error: None,
            role_run_id: None,
            run_no: None,
        })
        .expect("save analyst decision");
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/ws/coding-attempts/coding_attempt_0001");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");

    match recv_json(&mut ws).await {
        CodingWsOutMessage::CodingSessionState {
            latest_analyst_decision,
            ..
        } => {
            let decision = latest_analyst_decision.expect("latest analyst decision");
            assert_eq!(decision.id, "analyst_decision_0001");
            assert_eq!(decision.source_stage, CodingExecutionStage::Testing);
            assert_eq!(decision.verdict, AnalystDecisionVerdict::NeedsFix);
            assert_eq!(decision.next_stage, AnalystDecisionNextStage::Coding);
            assert_eq!(decision.evidence_refs, vec!["testing_report_0001.json"]);
        }
        other => panic!("expected coding session state, got {other:?}"),
    }

    ws.close(None).await.expect("close ws");
    server.abort();
}

#[tokio::test]
async fn coding_ws_session_state_includes_group_units() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let app = app_with_group_attempt(root.path());
    let attempt_id = "coding_attempt_0001";
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/ws/coding-attempts/{attempt_id}");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let state = match ws.next().await {
        Some(Ok(Message::Text(text))) => {
            serde_json::from_str::<serde_json::Value>(&text).expect("session state json")
        }
        other => panic!("expected text websocket message, got {other:?}"),
    };

    assert_eq!(state["type"], "coding_session_state");
    assert_eq!(state["attempt_scope"], "work_item_group");
    assert_eq!(state["work_item_group_id"], "work_item_plan_0001");
    assert_eq!(state["current_work_item_id"], "work_item_0001");
    assert_eq!(state["units"].as_array().expect("units").len(), 2);
    assert_eq!(state["units"][0]["status"], "running");
    assert_eq!(state["units"][1]["status"], "pending");

    ws.close(None).await.expect("close ws");
    server.abort();
}

fn app_with_group_attempt(root_path: &std::path::Path) -> axum::Router {
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
    let lifecycle = LifecycleStore::new(app_paths.clone());
    lifecycle
        .create_work_item(CreateWorkItemInput {
            id: Some("work_item_0001".to_string()),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: repository.id.clone(),
            story_spec_ids: Vec::new(),
            design_spec_ids: Vec::new(),
            title: "Coding work item 1".to_string(),
            work_item_set_id: Some("work_item_plan_0001".to_string()),
            sequence_hint: Some(10),
            plan_status: WorkItemPlanStatus::Confirmed,
            ..Default::default()
        })
        .expect("create work item 1");
    lifecycle
        .create_work_item(CreateWorkItemInput {
            id: Some("work_item_0002".to_string()),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: repository.id,
            story_spec_ids: Vec::new(),
            design_spec_ids: Vec::new(),
            title: "Coding work item 2".to_string(),
            work_item_set_id: Some("work_item_plan_0001".to_string()),
            sequence_hint: Some(20),
            plan_status: WorkItemPlanStatus::Confirmed,
            ..Default::default()
        })
        .expect("create work item 2");
    lifecycle
        .create_issue_work_item_plan(CreateIssueWorkItemPlanInput {
            id: Some("work_item_plan_0001".to_string()),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            source_story_spec_ids: Vec::new(),
            source_design_spec_ids: Vec::new(),
            options: IssueWorkItemPlanOptions {
                include_integration_tests: false,
                include_e2e_tests: false,
                force_frontend_backend_split: false,
                require_execution_plan_confirm: false,
            },
            status: IssueWorkItemPlanStatus::Confirmed,
            work_item_ids: vec!["work_item_0001".to_string(), "work_item_0002".to_string()],
            repository_profile_ref: None,
            verification_plan_ids: Vec::new(),
            dependency_graph: vec![cadence_aria::product::models::IssueWorkItemDependencyEdge {
                from_work_item_id: "work_item_0001".to_string(),
                to_work_item_id: "work_item_0002".to_string(),
            }],
            created_from_provider_run: None,
            validator_findings: Vec::new(),
        })
        .expect("create work item plan");

    let store = CodingAttemptStore::new(app_paths);
    let attempt = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_work_item_id: "work_item_0001".to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Fake,
                reviewer: Some(ProviderName::Fake),
                review_rounds: 1,
            },
            max_auto_rework: 2,
        })
        .expect("create group attempt");
    store
        .create_coding_unit(CreateCodingExecutionUnitInput {
            attempt_id: attempt.id.clone(),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            work_item_id: "work_item_0001".to_string(),
            order_index: 0,
            status: CodingExecutionUnitStatus::Running,
        })
        .expect("create coding unit 1");
    store
        .create_coding_unit(CreateCodingExecutionUnitInput {
            attempt_id: attempt.id,
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            work_item_id: "work_item_0002".to_string(),
            order_index: 1,
            status: CodingExecutionUnitStatus::Pending,
        })
        .expect("create coding unit 2");

    build_web_router(WebAppState::new(
        root_path.to_path_buf(),
        WebRuntime::new_fake(root_path.to_path_buf()),
    ))
}

#[tokio::test]
async fn coding_ws_stage_gate_confirm_resolves_persisted_gate() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let app = app_with_attempt(root.path());
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let gate = store
        .create_stage_gate(
            "coding_attempt_0001",
            CodingExecutionStage::Testing,
            CodingProviderRole::Tester,
            "2026-05-28T00:00:05Z".to_string(),
            CodingRoleProviderConfigSnapshot::from(ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::Fake),
                review_rounds: 1,
            }),
        )
        .expect("create stage gate");
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/ws/coding-attempts/coding_attempt_0001");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;

    send_json(
        &mut ws,
        &CodingWsInMessage::StageGateConfirm {
            stage: CodingExecutionStage::Testing,
        },
    )
    .await;

    match recv_json(&mut ws).await {
        CodingWsOutMessage::CodingSessionState { pending_gates, .. } => {
            assert!(pending_gates.is_empty());
        }
        other => panic!("expected coding session state, got {other:?}"),
    }
    let gates = store
        .list_stage_gates("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("list stage gates");
    assert_eq!(gates.len(), 1);
    assert_eq!(gates[0].gate_id, gate.gate_id);
    assert_eq!(
        gates[0].status,
        cadence_aria::product::coding_models::CodingStageGateStatus::Confirmed
    );

    ws.close(None).await.expect("close ws");
    server.abort();
}
