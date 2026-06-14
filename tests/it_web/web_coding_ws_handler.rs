use cadence_aria::cross_cutting::provider_adapter::ProviderAdapterError;
use cadence_aria::cross_cutting::provider_registry::ProviderRegistry;
use cadence_aria::cross_cutting::streaming_provider::{
    ProviderEvent, ProviderSession, StreamChunk, StreamingProviderAdapter, StreamingProviderInput,
};
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::coding_attempt_store::{
    CodingAttemptStore, CreateBlockedGateInput, CreateCodingAttemptInput,
};
use cadence_aria::product::coding_models::{
    AnalystDecisionNextStage, AnalystDecisionRecord, AnalystDecisionVerdict, CodingAgentRole,
    CodingAttemptStatus, CodingEntryType, CodingExecutionStage, CodingGateAction,
    CodingGateActionType, CodingGateKind, CodingGateRequired, CodingProviderPermissionMode,
    CodingProviderRole, CodingRoleProviderConfigSnapshot, CodingRoleRunEventType,
    CodingRoleRunStatus, CodingRoleRunTrigger, CodingTimelineNode, CodingTimelineNodeStatus,
    PushStatus, RemoteKind, ReviewRequest, ReviewRequestKind, ReviewVerdict, TestingOverallStatus,
};
use cadence_aria::product::lifecycle_store::{
    CreateWorkItemInput, CreateWorkspaceSessionInput, LifecycleStore,
};
use cadence_aria::product::models::WorkItemStatus;
use cadence_aria::product::models::{
    ProviderName, WorkItemPlanStatus, WorkspaceSessionStatus, WorkspaceType,
};
use cadence_aria::product::repository_store::{CreateRepositoryInput, RepositoryStore};
use cadence_aria::protocol::contracts::{AdapterInput, AdapterRole};
use cadence_aria::web::app::build_web_router;
use cadence_aria::web::coding_ws_handler::{
    CodingWsInMessage, CodingWsOutMessage, is_coding_ws_message_allowed,
};
use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::state::WebAppState;
use cadence_aria::web::workspace_ws_types::{ArtifactVersion, ProviderConfigSnapshot};
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

#[tokio::test]
async fn coding_ws_prepare_context_sends_work_item_context_and_updates_provider_selection() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let app = app_with_confirmed_work_item_context(root.path());
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/ws/coding-attempts/coding_attempt_0001");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");

    match recv_json(&mut ws).await {
        CodingWsOutMessage::CodingSessionState {
            work_item_markdown,
            verification_commands,
            provider_config_snapshot,
            ..
        } => {
            let markdown = work_item_markdown.expect("work item markdown");
            assert!(markdown.contains("实现爬楼梯问题"));
            assert!(markdown.contains("climb_stairs"));
            assert_eq!(
                verification_commands.as_ref(),
                &vec!["uv run python -m unittest discover -s tests -v".to_string()]
            );
            assert_eq!(provider_config_snapshot.author, ProviderName::Fake);
        }
        other => panic!("expected coding session state, got {other:?}"),
    }

    send_json(
        &mut ws,
        &CodingWsInMessage::ProviderSelect {
            role: "author".to_string(),
            provider: ProviderName::Codex,
        },
    )
    .await;

    assert_eq!(
        recv_json(&mut ws).await,
        CodingWsOutMessage::CodingProviderConfigUpdated {
            role: CodingProviderRole::Coder,
            provider: ProviderName::Codex,
        }
    );
    match recv_json(&mut ws).await {
        CodingWsOutMessage::CodingSessionState {
            provider_config_snapshot,
            work_item_markdown,
            ..
        } => {
            assert_eq!(provider_config_snapshot.author, ProviderName::Codex);
            assert_eq!(provider_config_snapshot.reviewer, Some(ProviderName::Fake));
            assert!(
                work_item_markdown
                    .as_deref()
                    .unwrap_or_default()
                    .contains("实现爬楼梯问题")
            );
        }
        other => panic!("expected updated coding session state, got {other:?}"),
    }

    ws.close(None).await.expect("close ws");
    server.abort();
}

#[tokio::test]
async fn coding_ws_context_note_persists_and_echoes_chat_entry() {
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

    send_json(
        &mut ws,
        &CodingWsInMessage::ContextNote {
            content: "请优先使用 unittest".to_string(),
        },
    )
    .await;

    match recv_json(&mut ws).await {
        CodingWsOutMessage::CodingChatEntryCreated { entry } => {
            assert_eq!(entry.id, "coding_chat_entry_0001");
            assert_eq!(entry.attempt_id, "coding_attempt_0001");
            assert_eq!(entry.node_id, None);
            assert_eq!(entry.role, CodingAgentRole::Author);
            assert_eq!(entry.entry_type, CodingEntryType::UserMessage);
            assert_eq!(entry.content.as_deref(), Some("请优先使用 unittest"));
            assert_eq!(
                entry.metadata.as_ref().and_then(|value| {
                    value
                        .get("context_note_id")
                        .and_then(|context_note_id| context_note_id.as_str())
                }),
                Some("coding_context_note_0001")
            );
        }
        other => panic!("expected coding chat entry echo, got {other:?}"),
    }

    let notes = store
        .list_context_notes("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("list context notes");
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].id, "coding_context_note_0001");
    assert_eq!(notes[0].content, "请优先使用 unittest");
    assert!(notes[0].consumed_by_rework_round.is_none());

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
async fn coding_ws_start_coding_waits_at_stage_gate_and_confirm_resumes_runner() {
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

    let gate = wait_for_stage_gate(&mut ws, CodingExecutionStage::Coding).await;
    assert_eq!(gate.kind, CodingGateKind::StageGate);
    assert_eq!(gate.role, Some(CodingProviderRole::Coder));
    assert_eq!(
        gate.provider_snapshot
            .as_ref()
            .map(|snapshot| &snapshot.coder),
        Some(&ProviderName::Fake)
    );
    assert!(
        store
            .list_open_stage_gates("project_0001", "issue_0001", "coding_attempt_0001")
            .expect("open stage gates")
            .iter()
            .any(|gate| gate.stage == CodingExecutionStage::Coding)
    );

    send_json(
        &mut ws,
        &CodingWsInMessage::StageGateConfirm {
            stage: CodingExecutionStage::Coding,
        },
    )
    .await;

    let node = wait_for_timeline_node(&mut ws, CodingExecutionStage::Coding).await;
    assert_eq!(node.status, CodingTimelineNodeStatus::Running);

    ws.close(None).await.expect("close ws");
    server.abort();
}

#[tokio::test]
async fn coding_ws_provider_select_during_stage_gate_updates_roles_and_refreshes_gate() {
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

    let gate = wait_for_stage_gate(&mut ws, CodingExecutionStage::Coding).await;
    let original_expires_at = gate.expires_at.expect("gate expires_at");
    tokio::time::sleep(Duration::from_millis(20)).await;

    send_json(
        &mut ws,
        &CodingWsInMessage::ProviderSelect {
            role: "tester".to_string(),
            provider: ProviderName::Codex,
        },
    )
    .await;

    assert_eq!(
        wait_for_provider_config_update(&mut ws).await,
        CodingWsOutMessage::CodingProviderConfigUpdated {
            role: CodingProviderRole::Tester,
            provider: ProviderName::Codex,
        }
    );
    let refreshed_gate = wait_for_stage_gate(&mut ws, CodingExecutionStage::Coding).await;
    assert_ne!(
        refreshed_gate.expires_at.as_deref(),
        Some(original_expires_at.as_str())
    );
    assert_eq!(
        refreshed_gate
            .provider_snapshot
            .as_ref()
            .map(|snapshot| &snapshot.tester),
        Some(&ProviderName::Codex)
    );
    assert_eq!(
        store
            .get_role_provider_config_snapshot("project_0001", "issue_0001", "coding_attempt_0001")
            .expect("role provider snapshot")
            .tester,
        ProviderName::Codex
    );

    ws.close(None).await.expect("close ws");
    server.abort();
}

#[tokio::test]
async fn coding_ws_permission_mode_select_updates_role_config() {
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

    send_json(
        &mut ws,
        &CodingWsInMessage::PermissionModeSelect {
            role: "tester".to_string(),
            permission_mode: CodingProviderPermissionMode::Supervised,
        },
    )
    .await;

    assert_eq!(
        wait_for_provider_config_update(&mut ws).await,
        CodingWsOutMessage::CodingProviderConfigUpdated {
            role: CodingProviderRole::Tester,
            provider: ProviderName::Fake,
        }
    );
    match recv_json(&mut ws).await {
        CodingWsOutMessage::CodingSessionState {
            role_provider_config_snapshot,
            ..
        } => {
            assert_eq!(
                role_provider_config_snapshot.permission_mode_for_role(&CodingProviderRole::Tester),
                CodingProviderPermissionMode::Supervised
            );
        }
        other => panic!("expected updated coding session state, got {other:?}"),
    }

    let snapshot = store
        .get_role_provider_config_snapshot("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("role config");
    assert_eq!(
        snapshot.permission_mode_for_role(&CodingProviderRole::Tester),
        CodingProviderPermissionMode::Supervised
    );

    ws.close(None).await.expect("close ws");
    server.abort();
}

#[tokio::test]
async fn coding_ws_stage_gate_timeout_auto_starts_stage() {
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

    let gate = wait_for_stage_gate(&mut ws, CodingExecutionStage::Coding).await;
    let node = wait_for_timeline_node(&mut ws, CodingExecutionStage::Coding).await;
    assert_eq!(node.status, CodingTimelineNodeStatus::Running);
    let gates = store
        .list_stage_gates("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("list stage gates");
    let expired = gates
        .iter()
        .find(|candidate| candidate.gate_id == gate.gate_id)
        .expect("expired gate");
    assert_eq!(
        expired.status,
        cadence_aria::product::coding_models::CodingStageGateStatus::Expired
    );

    ws.close(None).await.expect("close ws");
    server.abort();
}

#[tokio::test]
async fn coding_ws_provider_select_rejects_current_running_stage_role_without_gate() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let app = app_with_running_testing_attempt(root.path());
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
        &CodingWsInMessage::ProviderSelect {
            role: "tester".to_string(),
            provider: ProviderName::Codex,
        },
    )
    .await;

    match recv_json(&mut ws).await {
        CodingWsOutMessage::CodingProtocolError { code, .. } => {
            assert_eq!(code, "coding_provider_role_locked");
        }
        other => panic!("expected provider lock error, got {other:?}"),
    }

    ws.close(None).await.expect("close ws");
    server.abort();
}

#[tokio::test]
async fn coding_ws_abort_during_stage_gate_cancels_gate_before_snapshot() {
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
    let gate = wait_for_stage_gate(&mut ws, CodingExecutionStage::Coding).await;
    match recv_json(&mut ws).await {
        CodingWsOutMessage::CodingSessionState { pending_gates, .. } => {
            assert_eq!(pending_gates.len(), 1);
        }
        other => panic!("expected stage gate session state, got {other:?}"),
    }

    send_json(&mut ws, &CodingWsInMessage::AbortAttempt).await;

    match recv_json(&mut ws).await {
        CodingWsOutMessage::CodingSessionState {
            status,
            pending_gates,
            ..
        } => {
            assert_eq!(status, CodingAttemptStatus::Aborted);
            assert!(pending_gates.is_empty());
        }
        other => panic!("expected aborted session state, got {other:?}"),
    }
    let gates = store
        .list_stage_gates("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("list gates");
    let cancelled = gates
        .iter()
        .find(|candidate| candidate.gate_id == gate.gate_id)
        .expect("cancelled gate");
    assert_eq!(
        cancelled.status,
        cadence_aria::product::coding_models::CodingStageGateStatus::Cancelled
    );

    ws.close(None).await.expect("close ws");
    server.abort();
}

#[tokio::test]
async fn coding_ws_start_coding_keeps_socket_responsive_while_runner_is_active() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let app = app_with_hanging_coding_attempt(root.path());
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/ws/coding-attempts/coding_attempt_0001");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;

    send_json(&mut ws, &CodingWsInMessage::StartCoding).await;
    let _gate = wait_for_stage_gate(&mut ws, CodingExecutionStage::Coding).await;
    send_json(
        &mut ws,
        &CodingWsInMessage::StageGateConfirm {
            stage: CodingExecutionStage::Coding,
        },
    )
    .await;

    let mut saw_hanging_chunk = false;
    for _ in 0..8 {
        match recv_json(&mut ws).await {
            CodingWsOutMessage::CodingStreamChunk { content, .. }
                if content == "hanging provider started" =>
            {
                saw_hanging_chunk = true;
                break;
            }
            CodingWsOutMessage::CodingProtocolError { code, message } => {
                panic!("unexpected coding protocol error {code}: {message}");
            }
            _ => {}
        }
    }
    assert!(saw_hanging_chunk, "expected hanging provider to start");

    send_json(&mut ws, &CodingWsInMessage::CodingPing).await;
    assert_eq!(recv_json(&mut ws).await, CodingWsOutMessage::CodingPong);

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
    let mut final_chat_entries = Vec::new();
    let mut confirmed_gates = HashSet::new();
    for _ in 0..80 {
        let message = recv_json(&mut ws).await;
        match message {
            CodingWsOutMessage::CodingTimelineNodeCreated { node } => {
                stages.push(node.stage);
            }
            CodingWsOutMessage::CodingGateRequired { gate } => {
                if respond_to_testing_result_review_gate(&mut ws, &gate).await {
                    continue;
                }
                assert_eq!(
                    gate.kind,
                    CodingGateKind::StageGate,
                    "unexpected non-stage gate: {:?} {:?}",
                    gate.reason_code,
                    gate.description
                );
                if let Some(stage) = gate.stage.clone()
                    && confirmed_gates.insert(gate.gate_id)
                {
                    send_json(&mut ws, &CodingWsInMessage::StageGateConfirm { stage }).await;
                }
            }
            CodingWsOutMessage::CodingSessionState {
                status,
                stage,
                chat_entries,
                ..
            } if status == CodingAttemptStatus::Completed
                && stage == CodingExecutionStage::FinalConfirm =>
            {
                final_chat_entries = *chat_entries;
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
        CodingExecutionStage::Rework,
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
    assert_eq!(
        stages
            .iter()
            .filter(|stage| **stage == CodingExecutionStage::Rework)
            .count(),
        3,
        "expected rework after testing, code review, and internal review; got {stages:?}"
    );
    assert!(
        final_chat_entries
            .iter()
            .any(|entry| matches!(entry.entry_type, CodingEntryType::AnalystVerdict { .. })),
        "expected persisted analyst verdict chat entry"
    );
    assert!(
        final_chat_entries.iter().any(|entry| {
            entry
                .metadata
                .as_ref()
                .and_then(|value| value.get("source"))
                .and_then(|value| value.as_str())
                == Some("code_review")
        }),
        "expected persisted code review chat entry"
    );
    assert!(
        final_chat_entries.iter().any(|entry| {
            entry
                .metadata
                .as_ref()
                .and_then(|value| value.get("source"))
                .and_then(|value| value.as_str())
                == Some("internal_pr_review")
        }),
        "expected persisted internal PR review chat entry"
    );

    let attempt = store
        .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("updated attempt");
    assert_eq!(attempt.status, CodingAttemptStatus::Completed);
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
async fn coding_ws_testing_blocked_waits_for_human_result_review_before_analyst() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let app =
        app_with_full_chain_attempt_and_provider(root.path(), Arc::new(TestingBlockedProvider));
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/ws/coding-attempts/coding_attempt_0001");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;

    send_json(&mut ws, &CodingWsInMessage::StartCoding).await;

    let gate = wait_for_testing_result_review_gate(&mut ws).await;
    assert_eq!(gate.kind, CodingGateKind::Blocked);
    assert_eq!(gate.stage, Some(CodingExecutionStage::Testing));
    assert_eq!(gate.role, Some(CodingProviderRole::Tester));
    assert_eq!(
        gate.reason_code.as_deref(),
        Some("testing_result_review_required")
    );
    assert!(
        gate.description.contains("测试被阻塞"),
        "expected blocked testing summary, got {}",
        gate.description
    );

    send_json(
        &mut ws,
        &CodingWsInMessage::GateResponse {
            gate_id: gate.gate_id,
            action_id: "accept_testing_result".to_string(),
            extra_context: None,
        },
    )
    .await;

    let mut saw_analyst = false;
    for _ in 0..80 {
        match recv_json(&mut ws).await {
            CodingWsOutMessage::CodingTimelineNodeCreated { node }
                if node.stage == CodingExecutionStage::Rework =>
            {
                saw_analyst = true;
                break;
            }
            CodingWsOutMessage::CodingProtocolError { code, message } => {
                panic!("unexpected coding protocol error {code}: {message}");
            }
            _ => {}
        }
    }

    assert!(
        saw_analyst,
        "testing blocked did not enter analyst after accept"
    );

    let attempt = store
        .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("attempt");
    assert_eq!(attempt.status, CodingAttemptStatus::Running);
    assert!(
        matches!(
            attempt.stage,
            CodingExecutionStage::Rework | CodingExecutionStage::CodeReview
        ),
        "expected Rework or CodeReview after accept, got {:?}",
        attempt.stage
    );

    let nodes = store
        .get_timeline_nodes("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("nodes");
    assert!(
        nodes
            .iter()
            .any(|node| node.stage == CodingExecutionStage::Rework)
    );

    ws.close(None).await.expect("close ws");
    server.abort();
}

#[tokio::test]
async fn coding_ws_testing_completion_waits_for_human_result_review() {
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

    let gate = wait_for_testing_result_review_gate(&mut ws).await;
    assert_eq!(gate.kind, CodingGateKind::Blocked);
    assert_eq!(gate.stage, Some(CodingExecutionStage::Testing));
    assert_eq!(gate.role, Some(CodingProviderRole::Tester));
    assert_eq!(
        gate.reason_code.as_deref(),
        Some("testing_result_review_required")
    );
    assert!(
        gate.available_actions
            .iter()
            .any(|action| action.action_id == "accept_testing_result"
                && action.action_type == CodingGateActionType::AcceptTestingResult),
        "expected accept_testing_result action, got {:?}",
        gate.available_actions
    );
    assert!(
        gate.available_actions
            .iter()
            .any(|action| action.action_id == "rerun_testing"
                && action.action_type == CodingGateActionType::RerunTesting),
        "expected rerun_testing action, got {:?}",
        gate.available_actions
    );
    assert!(
        gate.evidence_refs
            .iter()
            .any(|reference| reference == "testing_report_0001.json"),
        "expected testing report evidence ref, got {:?}",
        gate.evidence_refs
    );

    assert!(
        store
            .list_open_blocked_gates("project_0001", "issue_0001", "coding_attempt_0001")
            .expect("open gates")
            .iter()
            .any(|gate| gate.reason_code.as_deref() == Some("testing_result_review_required"))
    );
    let attempt = store
        .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("attempt");
    assert_eq!(attempt.status, CodingAttemptStatus::Blocked);
    assert_eq!(attempt.stage, CodingExecutionStage::Testing);
    assert!(
        store
            .list_role_runs("project_0001", "issue_0001", "coding_attempt_0001")
            .expect("role runs")
            .iter()
            .all(|run| run.role != CodingProviderRole::Analyst),
        "analyst must not start before human accepts tester result"
    );

    ws.close(None).await.expect("close ws");
    server.abort();
}

#[tokio::test]
async fn coding_ws_accept_testing_result_enters_analyst_with_testing_report_evidence() {
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
    let gate = wait_for_testing_result_review_gate(&mut ws).await;

    send_json(
        &mut ws,
        &CodingWsInMessage::GateResponse {
            gate_id: gate.gate_id,
            action_id: "accept_testing_result".to_string(),
            extra_context: None,
        },
    )
    .await;

    let mut saw_analyst = false;
    for _ in 0..80 {
        match recv_json(&mut ws).await {
            CodingWsOutMessage::CodingTimelineNodeCreated { node }
                if node.stage == CodingExecutionStage::Rework =>
            {
                saw_analyst = true;
                break;
            }
            CodingWsOutMessage::CodingProtocolError { code, message } => {
                panic!("unexpected coding protocol error {code}: {message}");
            }
            _ => {}
        }
    }
    assert!(saw_analyst, "accepting tester result did not start analyst");

    let runs = store
        .list_role_runs("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("role runs");
    let analyst_run = runs
        .iter()
        .find(|run| run.role == CodingProviderRole::Analyst)
        .expect("analyst role run");
    let evidence_ref = analyst_run
        .artifact_refs
        .iter()
        .find(|reference| reference.contains("analyst_evidence"))
        .expect("analyst evidence ref");
    let evidence = store
        .read_attempt_artifact_text("coding_attempt_0001", evidence_ref)
        .expect("analyst evidence");
    assert!(
        evidence.contains("\"id\": \"testing_report_0001\""),
        "expected TestingReport JSON evidence, got {evidence}"
    );

    ws.close(None).await.expect("close ws");
    server.abort();
}

#[tokio::test]
async fn coding_ws_rerun_testing_result_review_reexecutes_tester() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let provider = Arc::new(RerunTestingProvider::default());
    let app = app_with_full_chain_attempt_and_provider(root.path(), provider.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/ws/coding-attempts/coding_attempt_0001");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;

    send_json(&mut ws, &CodingWsInMessage::StartCoding).await;
    let first_gate = wait_for_testing_result_review_gate(&mut ws).await;
    assert_eq!(provider.testing_execute_calls(), 1);

    send_json(
        &mut ws,
        &CodingWsInMessage::GateResponse {
            gate_id: first_gate.gate_id,
            action_id: "rerun_testing".to_string(),
            extra_context: None,
        },
    )
    .await;

    let mut confirmed_stage_gates = HashSet::new();
    let mut second_gate = None;
    for _ in 0..120 {
        match recv_json(&mut ws).await {
            CodingWsOutMessage::CodingGateRequired { gate }
                if gate.kind == CodingGateKind::StageGate =>
            {
                if let Some(stage) = gate.stage
                    && confirmed_stage_gates.insert(gate.gate_id)
                {
                    send_json(&mut ws, &CodingWsInMessage::StageGateConfirm { stage }).await;
                }
            }
            CodingWsOutMessage::CodingGateRequired { gate }
                if gate.reason_code.as_deref() == Some("testing_result_review_required")
                    && gate
                        .evidence_refs
                        .iter()
                        .any(|reference| reference == "testing_report_0002.json") =>
            {
                second_gate = Some(gate);
                break;
            }
            CodingWsOutMessage::CodingSessionState { pending_gates, .. } => {
                if let Some(gate) = pending_gates.iter().find(|gate| {
                    gate.reason_code.as_deref() == Some("testing_result_review_required")
                        && gate
                            .evidence_refs
                            .iter()
                            .any(|reference| reference == "testing_report_0002.json")
                }) {
                    second_gate = Some(gate.clone());
                    break;
                }
                for gate in pending_gates
                    .into_iter()
                    .filter(|gate| gate.kind == CodingGateKind::StageGate)
                {
                    if let Some(stage) = gate.stage
                        && confirmed_stage_gates.insert(gate.gate_id)
                    {
                        send_json(&mut ws, &CodingWsInMessage::StageGateConfirm { stage }).await;
                    }
                }
            }
            CodingWsOutMessage::CodingProtocolError { code, message } => {
                panic!("unexpected coding protocol error {code}: {message}");
            }
            _ => {}
        }
    }

    let second_gate = second_gate.expect("second testing result review gate");
    assert_eq!(second_gate.stage, Some(CodingExecutionStage::Testing));
    assert_eq!(provider.testing_execute_calls(), 2);
    let reports = store
        .list_testing_reports("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("testing reports");
    assert_eq!(reports.len(), 2);
    let runs = store
        .list_role_runs("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("role runs");
    let tester_runs = runs
        .iter()
        .filter(|run| run.role == CodingProviderRole::Tester)
        .collect::<Vec<_>>();
    assert_eq!(tester_runs.len(), 2);
    assert_eq!(tester_runs[0].status, CodingRoleRunStatus::Superseded);
    assert_eq!(tester_runs[1].trigger, CodingRoleRunTrigger::ManualRerun);

    ws.close(None).await.expect("close ws");
    server.abort();
}

#[tokio::test]
async fn coding_ws_code_review_blocked_enters_analyst_before_coding() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let provider = Arc::new(ReviewerBlockedProvider::code_review());
    let app = app_with_full_chain_attempt_and_provider(root.path(), provider.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/ws/coding-attempts/coding_attempt_0001");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;

    send_json(&mut ws, &CodingWsInMessage::StartCoding).await;

    let mut confirmed_gates = HashSet::new();
    let mut saw_code_review_blocked = false;
    let mut saw_analyst_after_code_review = false;
    let mut saw_coding_gate_after_analyst = false;
    let mut accepted_testing_result_gates = HashSet::new();
    for _ in 0..160 {
        match recv_json(&mut ws).await {
            CodingWsOutMessage::CodingGateRequired { gate }
                if gate.kind == CodingGateKind::Blocked
                    && gate.stage.as_ref() == Some(&CodingExecutionStage::CodeReview) =>
            {
                panic!("code review blocked should be routed to analyst, got gate {gate:?}");
            }
            CodingWsOutMessage::CodingGateRequired { gate }
                if is_testing_result_review_gate(&gate)
                    && accepted_testing_result_gates.insert(gate.gate_id.clone()) =>
            {
                respond_to_testing_result_review_gate(&mut ws, &gate).await;
            }
            CodingWsOutMessage::CodingSessionState {
                status,
                pending_gates,
                ..
            } => {
                if let Some(gate) = pending_gates.iter().find(|gate| {
                    gate.kind == CodingGateKind::Blocked
                        && gate.stage.as_ref() == Some(&CodingExecutionStage::CodeReview)
                }) {
                    panic!("code review blocked should be routed to analyst, got gate {gate:?}");
                }
                let mut responded_to_testing_result = false;
                if status == CodingAttemptStatus::Blocked {
                    for gate in pending_gates
                        .iter()
                        .filter(|gate| is_testing_result_review_gate(gate))
                    {
                        if accepted_testing_result_gates.insert(gate.gate_id.clone()) {
                            respond_to_testing_result_review_gate(&mut ws, gate).await;
                            responded_to_testing_result = true;
                        }
                    }
                }
                if responded_to_testing_result {
                    continue;
                }
                let stage_gates = pending_gates
                    .iter()
                    .filter(|gate| gate.kind == CodingGateKind::StageGate)
                    .filter_map(|gate| {
                        gate.stage
                            .clone()
                            .map(|stage| (gate.gate_id.clone(), stage))
                    })
                    .collect::<Vec<_>>();
                if saw_analyst_after_code_review
                    && stage_gates
                        .iter()
                        .any(|(_, stage)| *stage == CodingExecutionStage::Coding)
                {
                    saw_coding_gate_after_analyst = true;
                    break;
                }
                for (gate_id, stage) in stage_gates {
                    if confirmed_gates.insert(gate_id) {
                        send_json(&mut ws, &CodingWsInMessage::StageGateConfirm { stage }).await;
                    }
                }
            }
            CodingWsOutMessage::CodingGateRequired { gate }
                if gate.kind == CodingGateKind::StageGate =>
            {
                if saw_analyst_after_code_review
                    && gate.stage.as_ref() == Some(&CodingExecutionStage::Coding)
                {
                    saw_coding_gate_after_analyst = true;
                    break;
                }
                if let Some(stage) = gate.stage.clone()
                    && confirmed_gates.insert(gate.gate_id)
                {
                    send_json(&mut ws, &CodingWsInMessage::StageGateConfirm { stage }).await;
                }
            }
            CodingWsOutMessage::CodeReviewComplete { report }
                if report.verdict == ReviewVerdict::Blocked =>
            {
                saw_code_review_blocked = true;
            }
            CodingWsOutMessage::CodingTimelineNodeCreated { node }
                if saw_code_review_blocked && node.stage == CodingExecutionStage::Rework =>
            {
                saw_analyst_after_code_review = true;
            }
            CodingWsOutMessage::CodingProtocolError { code, message } => {
                panic!("unexpected coding protocol error {code}: {message}");
            }
            _ => {}
        }
    }

    assert!(
        saw_code_review_blocked,
        "code review blocked report missing"
    );
    assert!(
        saw_analyst_after_code_review,
        "code review blocked did not enter analyst"
    );
    assert!(
        saw_coding_gate_after_analyst,
        "analyst next_stage=coding did not route back to coder"
    );
    assert!(
        provider
            .analyst_prompts()
            .iter()
            .any(|prompt| prompt.contains("Previous Stage: CodeReview")),
        "analyst did not receive code review evidence"
    );

    let attempt = store
        .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("attempt");
    assert_eq!(attempt.status, CodingAttemptStatus::Running);
    assert_eq!(attempt.stage, CodingExecutionStage::Coding);

    ws.close(None).await.expect("close ws");
    server.abort();
}

#[tokio::test]
async fn coding_ws_internal_pr_review_blocked_enters_analyst_before_final_confirm() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let provider = Arc::new(ReviewerBlockedProvider::internal_pr_review());
    let app = app_with_full_chain_attempt_and_provider(root.path(), provider.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/ws/coding-attempts/coding_attempt_0001");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;

    send_json(&mut ws, &CodingWsInMessage::StartCoding).await;

    let mut confirmed_gates = HashSet::new();
    let mut saw_internal_review_blocked = false;
    let mut saw_analyst_after_internal_review = false;
    let mut completed = false;
    let mut accepted_testing_result_gates = HashSet::new();
    for _ in 0..220 {
        match recv_json(&mut ws).await {
            CodingWsOutMessage::CodingGateRequired { gate }
                if gate.kind == CodingGateKind::Blocked
                    && gate.stage.as_ref() == Some(&CodingExecutionStage::InternalPrReview) =>
            {
                panic!("internal review blocked should be routed to analyst, got gate {gate:?}");
            }
            CodingWsOutMessage::CodingGateRequired { gate }
                if is_testing_result_review_gate(&gate)
                    && accepted_testing_result_gates.insert(gate.gate_id.clone()) =>
            {
                respond_to_testing_result_review_gate(&mut ws, &gate).await;
            }
            CodingWsOutMessage::CodingSessionState {
                status,
                stage,
                pending_gates,
                ..
            } => {
                if let Some(gate) = pending_gates.iter().find(|gate| {
                    gate.kind == CodingGateKind::Blocked
                        && gate.stage.as_ref() == Some(&CodingExecutionStage::InternalPrReview)
                }) {
                    panic!(
                        "internal review blocked should be routed to analyst, got gate {gate:?}"
                    );
                }
                let mut responded_to_testing_result = false;
                if status == CodingAttemptStatus::Blocked {
                    for gate in pending_gates
                        .iter()
                        .filter(|gate| is_testing_result_review_gate(gate))
                    {
                        if accepted_testing_result_gates.insert(gate.gate_id.clone()) {
                            respond_to_testing_result_review_gate(&mut ws, gate).await;
                            responded_to_testing_result = true;
                        }
                    }
                }
                if responded_to_testing_result {
                    continue;
                }
                let stage_gates = pending_gates
                    .iter()
                    .filter(|gate| gate.kind == CodingGateKind::StageGate)
                    .filter_map(|gate| {
                        gate.stage
                            .clone()
                            .map(|stage| (gate.gate_id.clone(), stage))
                    })
                    .collect::<Vec<_>>();
                for (gate_id, stage) in stage_gates {
                    if confirmed_gates.insert(gate_id) {
                        send_json(&mut ws, &CodingWsInMessage::StageGateConfirm { stage }).await;
                    }
                }
                if saw_analyst_after_internal_review
                    && status == CodingAttemptStatus::Completed
                    && stage == CodingExecutionStage::FinalConfirm
                {
                    completed = true;
                    break;
                }
            }
            CodingWsOutMessage::CodingGateRequired { gate }
                if gate.kind == CodingGateKind::StageGate =>
            {
                if let Some(stage) = gate.stage.clone()
                    && confirmed_gates.insert(gate.gate_id)
                {
                    send_json(&mut ws, &CodingWsInMessage::StageGateConfirm { stage }).await;
                }
            }
            CodingWsOutMessage::InternalPrReviewComplete { review }
                if review.verdict == ReviewVerdict::Blocked =>
            {
                saw_internal_review_blocked = true;
            }
            CodingWsOutMessage::CodingTimelineNodeCreated { node }
                if saw_internal_review_blocked && node.stage == CodingExecutionStage::Rework =>
            {
                saw_analyst_after_internal_review = true;
            }
            CodingWsOutMessage::CodingProtocolError { code, message } => {
                panic!("unexpected coding protocol error {code}: {message}");
            }
            _ => {}
        }
    }

    assert!(
        saw_internal_review_blocked,
        "internal review blocked report missing"
    );
    assert!(
        saw_analyst_after_internal_review,
        "internal review blocked did not enter analyst"
    );
    assert!(
        completed,
        "analyst next_stage=final_confirm did not complete final confirm path"
    );
    assert!(
        provider
            .analyst_prompts()
            .iter()
            .any(|prompt| prompt.contains("Previous Stage: InternalPrReview")),
        "analyst did not receive internal review evidence"
    );

    let attempt = store
        .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("attempt");
    assert_eq!(attempt.status, CodingAttemptStatus::Completed);
    assert_eq!(attempt.stage, CodingExecutionStage::FinalConfirm);

    ws.close(None).await.expect("close ws");
    server.abort();
}

#[tokio::test]
async fn coding_ws_analyst_next_stage_testing_reruns_tester_before_code_review() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let provider = Arc::new(RerunTestingProvider::default());
    let app = app_with_full_chain_attempt_and_provider(root.path(), provider.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/ws/coding-attempts/coding_attempt_0001");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;

    send_json(&mut ws, &CodingWsInMessage::StartCoding).await;

    let mut confirmed_gates = HashSet::new();
    let mut completed = false;
    let mut testing_nodes = 0;
    for _ in 0..180 {
        match recv_json(&mut ws).await {
            CodingWsOutMessage::CodingGateRequired { gate } => {
                if respond_to_testing_result_review_gate(&mut ws, &gate).await {
                    continue;
                }
                assert_eq!(
                    gate.kind,
                    CodingGateKind::StageGate,
                    "unexpected non-stage gate: {:?} {:?}",
                    gate.reason_code,
                    gate.description
                );
                if let Some(stage) = gate.stage.clone()
                    && confirmed_gates.insert(gate.gate_id)
                {
                    send_json(&mut ws, &CodingWsInMessage::StageGateConfirm { stage }).await;
                }
            }
            CodingWsOutMessage::CodingTimelineNodeCreated { node }
                if node.stage == CodingExecutionStage::Testing =>
            {
                testing_nodes += 1;
            }
            CodingWsOutMessage::CodingSessionState { status, stage, .. }
                if status == CodingAttemptStatus::Completed
                    && stage == CodingExecutionStage::FinalConfirm =>
            {
                completed = true;
                break;
            }
            CodingWsOutMessage::CodingProtocolError { code, message } => {
                panic!("unexpected coding protocol error {code}: {message}");
            }
            _ => {}
        }
    }

    assert!(
        completed,
        "expected completed attempt after analyst requested tester rerun"
    );
    assert_eq!(
        provider.testing_execute_calls(),
        2,
        "analyst next_stage=testing should rerun Tester"
    );
    assert_eq!(testing_nodes, 2);
    assert_eq!(
        store
            .list_testing_reports("project_0001", "issue_0001", "coding_attempt_0001")
            .expect("testing reports")
            .len(),
        2
    );

    ws.close(None).await.expect("close ws");
    server.abort();
}

#[tokio::test]
async fn internal_review_rework_creates_new_review_request_commit() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let app = app_with_internal_review_rework_attempt(root.path());
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/ws/coding-attempts/coding_attempt_0001");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;

    send_json(&mut ws, &CodingWsInMessage::StartCoding).await;

    let mut confirmed_gates = HashSet::new();
    let mut completed = false;
    for _ in 0..140 {
        match recv_json(&mut ws).await {
            CodingWsOutMessage::CodingGateRequired { gate } => {
                if respond_to_testing_result_review_gate(&mut ws, &gate).await {
                    continue;
                }
                assert_eq!(
                    gate.kind,
                    CodingGateKind::StageGate,
                    "unexpected non-stage gate: {:?} {:?}",
                    gate.reason_code,
                    gate.description
                );
                if let Some(stage) = gate.stage.clone()
                    && confirmed_gates.insert(gate.gate_id)
                {
                    send_json(&mut ws, &CodingWsInMessage::StageGateConfirm { stage }).await;
                }
            }
            CodingWsOutMessage::CodingSessionState { status, stage, .. }
                if status == CodingAttemptStatus::Completed
                    && stage == CodingExecutionStage::FinalConfirm =>
            {
                completed = true;
                break;
            }
            CodingWsOutMessage::CodingProtocolError { code, message } => {
                panic!("unexpected coding protocol error {code}: {message}");
            }
            _ => {}
        }
    }

    assert!(
        completed,
        "expected completed attempt after internal review rework"
    );
    let requests = store
        .list_review_requests("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("review requests");
    assert_eq!(requests.len(), 2);
    assert_ne!(requests[0].commit_sha, requests[1].commit_sha);
    assert_eq!(
        store
            .list_internal_pr_reviews("project_0001", "issue_0001", "coding_attempt_0001")
            .expect("internal reviews")
            .len(),
        2
    );
    let attempt = store
        .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("attempt");
    assert_eq!(
        attempt.review_request_id.as_deref(),
        Some("review_request_0002")
    );
    let worktree = attempt.worktree_path.expect("worktree path");
    assert!(worktree.join("src/internal_fix.rs").is_file());

    ws.close(None).await.expect("close ws");
    server.abort();
}

#[tokio::test]
async fn code_review_findings_are_injected_into_next_coding_round() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let provider = Arc::new(CodeReviewReworkProvider::default());
    let app = app_with_code_review_rework_attempt(root.path(), provider.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/ws/coding-attempts/coding_attempt_0001");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;

    send_json(&mut ws, &CodingWsInMessage::StartCoding).await;

    let mut confirmed_gates = HashSet::new();
    let mut completed = false;
    for _ in 0..140 {
        match recv_json(&mut ws).await {
            CodingWsOutMessage::CodingGateRequired { gate } => {
                if respond_to_testing_result_review_gate(&mut ws, &gate).await {
                    continue;
                }
                assert_eq!(
                    gate.kind,
                    CodingGateKind::StageGate,
                    "unexpected non-stage gate: {:?} {:?}",
                    gate.reason_code,
                    gate.description
                );
                if let Some(stage) = gate.stage.clone()
                    && confirmed_gates.insert(gate.gate_id)
                {
                    send_json(&mut ws, &CodingWsInMessage::StageGateConfirm { stage }).await;
                }
            }
            CodingWsOutMessage::CodingSessionState { status, stage, .. }
                if status == CodingAttemptStatus::Completed
                    && stage == CodingExecutionStage::FinalConfirm =>
            {
                completed = true;
                break;
            }
            CodingWsOutMessage::CodingProtocolError { code, message } => {
                panic!("unexpected coding protocol error {code}: {message}");
            }
            _ => {}
        }
    }

    assert!(
        completed,
        "expected completed attempt after code review rework"
    );
    let prompts = provider.coding_prompts();
    assert_eq!(prompts.len(), 2);
    assert!(prompts[1].contains("上一轮返修要求"));
    assert!(prompts[1].contains("移除 __pycache__ 和 .pyc 文件"));
    let attempt = store
        .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("attempt");
    let worktree = attempt.worktree_path.expect("worktree path");
    let latest_request = store
        .list_review_requests("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("review requests")
        .pop()
        .expect("review request");
    let committed = git_stdout(
        &worktree,
        &[
            "show",
            "--name-only",
            "--format=",
            &latest_request.commit_sha,
        ],
    );
    assert!(!committed.contains("__pycache__"));
    assert!(!committed.contains(".pyc"));
    assert!(!committed.contains(".aria/coding-artifacts"));

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

fn app_with_confirmed_work_item_context(root_path: &std::path::Path) -> axum::Router {
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
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: repository.id,
            story_spec_ids: Vec::new(),
            design_spec_ids: Vec::new(),
            title: "实现爬楼梯问题".to_string(),
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
                markdown: CLIMB_STAIRS_WORK_ITEM.to_string(),
                generated_by: ProviderName::Fake,
                reviewed_by: Some(ProviderName::Fake),
                review_verdict: None,
                confirmed_by: Some("user".to_string()),
                is_current: true,
                created_at: "2026-05-28T00:00:00Z".to_string(),
                source_node_id: "node_0001".to_string(),
            },
        )
        .expect("append artifact version");
    lifecycle
        .update_workspace_session_status(&session.id, WorkspaceSessionStatus::Confirmed)
        .expect("confirm workspace session");
    CodingAttemptStore::new(app_paths)
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
    app_with_full_chain_attempt_and_provider(root_path, Arc::new(FullChainStreamingProvider))
}

fn app_with_full_chain_attempt_and_provider(
    root_path: &Path,
    provider: Arc<dyn StreamingProviderAdapter>,
) -> axum::Router {
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
    registry.register(ProviderName::Fake, provider);
    build_web_router(WebAppState::with_provider_registry(
        root_path.to_path_buf(),
        WebRuntime::new_fake(root_path.to_path_buf()),
        registry,
    ))
}

fn app_with_internal_review_rework_attempt(root_path: &Path) -> axum::Router {
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
    registry.register(
        ProviderName::Fake,
        Arc::new(InternalReviewReworkProvider::default()),
    );
    build_web_router(WebAppState::with_provider_registry(
        root_path.to_path_buf(),
        WebRuntime::new_fake(root_path.to_path_buf()),
        registry,
    ))
}

fn app_with_code_review_rework_attempt(
    root_path: &Path,
    provider: Arc<CodeReviewReworkProvider>,
) -> axum::Router {
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
    registry.register(ProviderName::Fake, provider);
    build_web_router(WebAppState::with_provider_registry(
        root_path.to_path_buf(),
        WebRuntime::new_fake(root_path.to_path_buf()),
        registry,
    ))
}

fn app_with_hanging_coding_attempt(root_path: &Path) -> axum::Router {
    let repo = root_path.join("repo");
    init_cargo_repo(&repo);

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
    registry.register(ProviderName::Fake, Arc::new(HangingCodingProvider));
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

async fn wait_for_stage_gate(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    stage: CodingExecutionStage,
) -> CodingGateRequired {
    for _ in 0..50 {
        match recv_json(ws).await {
            CodingWsOutMessage::CodingGateRequired { gate }
                if gate.kind == CodingGateKind::StageGate
                    && gate.stage.as_ref() == Some(&stage) =>
            {
                return gate;
            }
            CodingWsOutMessage::CodingSessionState { pending_gates, .. } => {
                if let Some(gate) = pending_gates.into_iter().find(|gate| {
                    gate.kind == CodingGateKind::StageGate && gate.stage.as_ref() == Some(&stage)
                }) {
                    return gate;
                }
            }
            CodingWsOutMessage::CodingTimelineNodeCreated { node } if node.stage == stage => {
                panic!("stage {stage:?} started before stage gate was confirmed");
            }
            CodingWsOutMessage::CodingProtocolError { code, message } => {
                panic!("unexpected coding protocol error {code}: {message}");
            }
            _ => {}
        }
    }
    panic!("expected stage gate for {stage:?}");
}

fn is_testing_result_review_gate(gate: &CodingGateRequired) -> bool {
    gate.reason_code.as_deref() == Some("testing_result_review_required")
}

async fn respond_to_testing_result_review_gate(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    gate: &CodingGateRequired,
) -> bool {
    if !is_testing_result_review_gate(gate) {
        return false;
    }
    send_json(
        ws,
        &CodingWsInMessage::GateResponse {
            gate_id: gate.gate_id.clone(),
            action_id: "accept_testing_result".to_string(),
            extra_context: None,
        },
    )
    .await;
    true
}

async fn wait_for_testing_result_review_gate(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> CodingGateRequired {
    let mut confirmed_stage_gates = HashSet::new();
    for _ in 0..80 {
        match recv_json(ws).await {
            CodingWsOutMessage::CodingGateRequired { gate }
                if gate.reason_code.as_deref() == Some("testing_result_review_required") =>
            {
                return gate;
            }
            CodingWsOutMessage::CodingSessionState { pending_gates, .. } => {
                if let Some(gate) = pending_gates.iter().find(|gate| {
                    gate.reason_code.as_deref() == Some("testing_result_review_required")
                }) {
                    return gate.clone();
                }
                for gate in pending_gates
                    .into_iter()
                    .filter(|gate| gate.kind == CodingGateKind::StageGate)
                {
                    if let Some(stage) = gate.stage
                        && confirmed_stage_gates.insert(gate.gate_id)
                    {
                        send_json(ws, &CodingWsInMessage::StageGateConfirm { stage }).await;
                    }
                }
            }
            CodingWsOutMessage::CodingGateRequired { gate }
                if gate.kind == CodingGateKind::StageGate =>
            {
                if let Some(stage) = gate.stage
                    && confirmed_stage_gates.insert(gate.gate_id)
                {
                    send_json(ws, &CodingWsInMessage::StageGateConfirm { stage }).await;
                }
            }
            CodingWsOutMessage::CodingTimelineNodeCreated { node }
                if node.stage == CodingExecutionStage::Rework =>
            {
                panic!("analyst started before tester result review gate was accepted");
            }
            CodingWsOutMessage::CodingProtocolError { code, message } => {
                panic!("unexpected coding protocol error {code}: {message}");
            }
            _ => {}
        }
    }
    panic!("expected testing result review gate");
}

async fn wait_for_timeline_node(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    stage: CodingExecutionStage,
) -> CodingTimelineNode {
    for _ in 0..50 {
        match recv_json(ws).await {
            CodingWsOutMessage::CodingTimelineNodeCreated { node } if node.stage == stage => {
                return node;
            }
            CodingWsOutMessage::CodingProtocolError { code, message } => {
                panic!("unexpected coding protocol error {code}: {message}");
            }
            _ => {}
        }
    }
    panic!("expected timeline node for {stage:?}");
}

async fn wait_for_provider_config_update(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> CodingWsOutMessage {
    for _ in 0..20 {
        match recv_json(ws).await {
            message @ CodingWsOutMessage::CodingProviderConfigUpdated { .. } => return message,
            CodingWsOutMessage::CodingProtocolError { code, message } => {
                panic!("unexpected coding protocol error {code}: {message}");
            }
            _ => {}
        }
    }
    panic!("expected provider config update");
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
    serde_json::from_value(recv_json_value(ws).await).expect("ws json")
}

async fn recv_json_value(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> serde_json::Value {
    let message = timeout(Duration::from_secs(10), ws.next())
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
    fn supports_provider_driven_testing(&self) -> bool {
        true
    }

    async fn start(
        &self,
        input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        start_web_test_provider_driven_testing_session(&input.prompt, cancel)
    }

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
            AdapterRole::Reviewer
                if input.output_schema == "coding_workspace_analyst_verdict_json" =>
            {
                tx.try_send(StreamChunk::Done {
                    full_output: r#"{"verdict":"no_issue","summary":"testing ok"}"#.to_string(),
                })
                .expect("send analyst done");
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

struct TestingBlockedProvider;

#[async_trait::async_trait]
impl StreamingProviderAdapter for TestingBlockedProvider {
    fn supports_provider_driven_testing(&self) -> bool {
        true
    }

    async fn start(
        &self,
        input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let output = if input.prompt.contains("Phase: plan_tests_repair") {
            "still not json".to_string()
        } else if input.prompt.contains("Phase: plan_tests") {
            "not json at all".to_string()
        } else {
            return Err(ProviderAdapterError::execution_failed(
                None,
                String::new(),
                "streaming provider start is not implemented",
                0,
            ));
        };

        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel(8);
        tokio::spawn(async move {
            if cancel.is_cancelled() {
                return;
            }
            if event_tx
                .send(ProviderEvent::TextDelta {
                    content: output.clone(),
                })
                .await
                .is_err()
            {
                return;
            }
            if cancel.is_cancelled() {
                return;
            }
            let _ = event_tx
                .send(ProviderEvent::Completed {
                    full_output: output,
                    provider_session_id: None,
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
        input: &AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        FullChainStreamingProvider
            .run_streaming(input, CancellationToken::new())
            .await
    }
}

#[derive(Clone, Copy)]
enum BlockedReviewerStage {
    CodeReview,
    InternalPrReview,
}

struct ReviewerBlockedProvider {
    blocked_stage: BlockedReviewerStage,
    analyst_prompts: Mutex<Vec<String>>,
}

impl ReviewerBlockedProvider {
    fn code_review() -> Self {
        Self {
            blocked_stage: BlockedReviewerStage::CodeReview,
            analyst_prompts: Mutex::new(Vec::new()),
        }
    }

    fn internal_pr_review() -> Self {
        Self {
            blocked_stage: BlockedReviewerStage::InternalPrReview,
            analyst_prompts: Mutex::new(Vec::new()),
        }
    }

    fn analyst_prompts(&self) -> Vec<String> {
        self.analyst_prompts
            .lock()
            .expect("analyst prompts lock")
            .clone()
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for ReviewerBlockedProvider {
    fn supports_provider_driven_testing(&self) -> bool {
        true
    }

    async fn start(
        &self,
        input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        start_web_test_provider_driven_testing_session(&input.prompt, cancel)
    }

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
                tx.try_send(StreamChunk::Done {
                    full_output: "implemented climb_stairs".to_string(),
                })
                .expect("send coding done");
            }
            AdapterRole::Reviewer
                if input.output_schema == "coding_workspace_analyst_verdict_json" =>
            {
                self.analyst_prompts
                    .lock()
                    .expect("analyst prompts lock")
                    .push(input.prompt.clone());
                let full_output = if input.prompt.contains("Previous Stage: Testing") {
                    r#"{"verdict":"proceed","next_stage":"code_review","reason":"testing evidence accepted"}"#
                } else if input.prompt.contains("Previous Stage: CodeReview") {
                    match self.blocked_stage {
                        BlockedReviewerStage::CodeReview => {
                            r#"{"verdict":"needs_fix","next_stage":"coding","reason":"code review blocked requires coder follow-up","fix_hints":["补充 review 所需上下文"]}"#
                        }
                        BlockedReviewerStage::InternalPrReview => {
                            r#"{"verdict":"proceed","next_stage":"review_request","reason":"code review accepted"}"#
                        }
                    }
                } else if input.prompt.contains("Previous Stage: InternalPrReview") {
                    r#"{"verdict":"proceed","next_stage":"final_confirm","reason":"internal review blocked is accepted for final confirmation"}"#
                } else {
                    r#"{"verdict":"no_issue","summary":"ok"}"#
                };
                tx.try_send(StreamChunk::Done {
                    full_output: full_output.to_string(),
                })
                .expect("send analyst done");
            }
            AdapterRole::Reviewer if input.output_schema == "coding_workspace_code_review_json" => {
                let full_output = match self.blocked_stage {
                    BlockedReviewerStage::CodeReview => {
                        r#"{"verdict":"blocked","summary":"code review 缺少人工确认信息","findings":[]}"#
                    }
                    BlockedReviewerStage::InternalPrReview => {
                        r#"{"verdict":"approve","summary":"code review ok","findings":[]}"#
                    }
                };
                tx.try_send(StreamChunk::Done {
                    full_output: full_output.to_string(),
                })
                .expect("send code review done");
            }
            AdapterRole::Reviewer
                if input.output_schema == "coding_workspace_internal_pr_review_json" =>
            {
                let full_output = match self.blocked_stage {
                    BlockedReviewerStage::CodeReview => {
                        r#"{"verdict":"approve","summary":"internal review ok","findings":[],"impact_scope":["src"],"pr_description":"实现 work item","commit_message_suggestion":"feat: implement work item"}"#
                    }
                    BlockedReviewerStage::InternalPrReview => {
                        r#"{"verdict":"blocked","summary":"internal review 需要人工确认发布窗口","findings":[],"impact_scope":["release"],"pr_description":"实现 work item","commit_message_suggestion":"feat: implement work item"}"#
                    }
                };
                tx.try_send(StreamChunk::Done {
                    full_output: full_output.to_string(),
                })
                .expect("send internal review done");
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

#[derive(Default)]
struct RerunTestingProvider {
    state: Mutex<RerunTestingProviderState>,
}

#[derive(Default)]
struct RerunTestingProviderState {
    analyst_calls: usize,
    testing_execute_calls: usize,
}

impl RerunTestingProvider {
    fn testing_execute_calls(&self) -> usize {
        self.state.lock().expect("state lock").testing_execute_calls
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for RerunTestingProvider {
    fn supports_provider_driven_testing(&self) -> bool {
        true
    }

    async fn start(
        &self,
        input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        if input.prompt.contains("Phase: execute_test_plan") {
            self.state.lock().expect("state lock").testing_execute_calls += 1;
        }
        start_web_test_provider_driven_testing_session(&input.prompt, cancel)
    }

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
                tx.try_send(StreamChunk::Done {
                    full_output: "implemented climb_stairs".to_string(),
                })
                .expect("send coding done");
            }
            AdapterRole::Reviewer
                if input.output_schema == "coding_workspace_analyst_verdict_json" =>
            {
                let testing_analyst_call =
                    input.prompt.contains("Previous Stage: Testing").then(|| {
                        let mut state = self.state.lock().expect("state lock");
                        state.analyst_calls += 1;
                        state.analyst_calls
                    });
                let full_output = if testing_analyst_call == Some(1) {
                    r#"{"verdict":"rerun_testing","next_stage":"testing","reason":"rerun Tester before review"}"#
                } else if testing_analyst_call.is_some() {
                    r#"{"verdict":"proceed","next_stage":"code_review","reason":"testing evidence accepted"}"#
                } else {
                    r#"{"verdict":"no_issue","summary":"ok"}"#
                };
                tx.try_send(StreamChunk::Done {
                    full_output: full_output.to_string(),
                })
                .expect("send analyst done");
            }
            AdapterRole::Reviewer => {
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

#[derive(Default)]
struct InternalReviewReworkProvider {
    state: Mutex<InternalReviewReworkState>,
}

#[derive(Default)]
struct InternalReviewReworkState {
    coding_calls: usize,
    internal_review_calls: usize,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for InternalReviewReworkProvider {
    fn supports_provider_driven_testing(&self) -> bool {
        true
    }

    async fn start(
        &self,
        input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        start_web_test_provider_driven_testing_session(&input.prompt, cancel)
    }

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
                let coding_call = {
                    let mut state = self.state.lock().expect("state lock");
                    state.coding_calls += 1;
                    state.coding_calls
                };
                fs::write(worktree.join("src/lib.rs"), CLIMB_STAIRS_LIB).map_err(|error| {
                    ProviderAdapterError::incompatible_output(error.to_string(), "", "")
                })?;
                if coding_call >= 2 {
                    fs::write(
                        worktree.join("src/internal_fix.rs"),
                        "pub const FIXED: bool = true;\n",
                    )
                    .map_err(|error| {
                        ProviderAdapterError::incompatible_output(error.to_string(), "", "")
                    })?;
                }
                tx.try_send(StreamChunk::Text(format!("coding round {coding_call}")))
                    .expect("send coding chunk");
                tx.try_send(StreamChunk::Done {
                    full_output: format!("coding round {coding_call} done"),
                })
                .expect("send coding done");
            }
            AdapterRole::Reviewer
                if input.output_schema == "coding_workspace_analyst_verdict_json" =>
            {
                let full_output = if input.prompt.contains("Previous Stage: InternalPrReview")
                    && input.prompt.contains(r#""verdict": "request_changes""#)
                {
                    r#"{"verdict":"needs_fix","summary":"internal review 要求修复","fix_hints":["补充 internal_fix.rs"]}"#
                } else {
                    r#"{"verdict":"no_issue","summary":"ok"}"#
                };
                tx.try_send(StreamChunk::Done {
                    full_output: full_output.to_string(),
                })
                .expect("send analyst done");
            }
            AdapterRole::Reviewer
                if input.output_schema == "coding_workspace_internal_pr_review_json" =>
            {
                let internal_review_call = {
                    let mut state = self.state.lock().expect("state lock");
                    state.internal_review_calls += 1;
                    state.internal_review_calls
                };
                let full_output = if internal_review_call == 1 {
                    r#"{"verdict":"request_changes","summary":"需要 internal fix","findings":[{"severity":"medium","file":"src/internal_fix.rs","description":"缺少 internal fix","recommendation":"补充 internal_fix.rs"}],"impact_scope":["src"],"pr_description":"实现 work item","commit_message_suggestion":"feat: implement work item"}"#
                } else {
                    r#"{"verdict":"approve","summary":"internal review ok","findings":[],"impact_scope":["src"],"pr_description":"实现 work item","commit_message_suggestion":"feat: implement work item"}"#
                };
                tx.try_send(StreamChunk::Done {
                    full_output: full_output.to_string(),
                })
                .expect("send internal review done");
            }
            AdapterRole::Reviewer => {
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

#[derive(Default)]
struct CodeReviewReworkProvider {
    state: Mutex<CodeReviewReworkState>,
}

#[derive(Default)]
struct CodeReviewReworkState {
    coding_calls: usize,
    analyst_calls: usize,
    code_review_calls: usize,
    coding_prompts: Vec<String>,
}

impl CodeReviewReworkProvider {
    fn coding_prompts(&self) -> Vec<String> {
        self.state
            .lock()
            .expect("state lock")
            .coding_prompts
            .clone()
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for CodeReviewReworkProvider {
    fn supports_provider_driven_testing(&self) -> bool {
        true
    }

    async fn start(
        &self,
        input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        start_web_test_provider_driven_testing_session(&input.prompt, cancel)
    }

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
                let coding_call = {
                    let mut state = self.state.lock().expect("state lock");
                    state.coding_calls += 1;
                    state.coding_prompts.push(input.prompt.clone());
                    state.coding_calls
                };
                fs::write(worktree.join("src/lib.rs"), CLIMB_STAIRS_LIB).map_err(|error| {
                    ProviderAdapterError::incompatible_output(error.to_string(), "", "")
                })?;
                if coding_call == 1 {
                    fs::create_dir_all(worktree.join("__pycache__")).map_err(|error| {
                        ProviderAdapterError::incompatible_output(error.to_string(), "", "")
                    })?;
                    fs::write(
                        worktree.join("__pycache__/climbing_stairs.cpython-310.pyc"),
                        b"pyc",
                    )
                    .map_err(|error| {
                        ProviderAdapterError::incompatible_output(error.to_string(), "", "")
                    })?;
                } else {
                    let _ = fs::remove_dir_all(worktree.join("__pycache__"));
                    fs::write(
                        worktree.join("src/review_fix.rs"),
                        "pub const FIXED: bool = true;\n",
                    )
                    .map_err(|error| {
                        ProviderAdapterError::incompatible_output(error.to_string(), "", "")
                    })?;
                }
                tx.try_send(StreamChunk::Done {
                    full_output: format!("coding round {coding_call} done"),
                })
                .expect("send coding done");
            }
            AdapterRole::Reviewer
                if input.output_schema == "coding_workspace_analyst_verdict_json" =>
            {
                let analyst_call = {
                    let mut state = self.state.lock().expect("state lock");
                    state.analyst_calls += 1;
                    state.analyst_calls
                };
                let full_output = if analyst_call == 2 {
                    r#"{"verdict":"needs_fix","summary":"code review 要求移除运行产物","fix_hints":["移除 __pycache__ 和 .pyc 文件"]}"#
                } else {
                    r#"{"verdict":"no_issue","summary":"ok"}"#
                };
                tx.try_send(StreamChunk::Done {
                    full_output: full_output.to_string(),
                })
                .expect("send analyst done");
            }
            AdapterRole::Reviewer if input.output_schema == "coding_workspace_code_review_json" => {
                let code_review_call = {
                    let mut state = self.state.lock().expect("state lock");
                    state.code_review_calls += 1;
                    state.code_review_calls
                };
                let full_output = if code_review_call == 1 {
                    r#"{"verdict":"request_changes","summary":"运行产物进入 diff","findings":[{"severity":"medium","file":"__pycache__/climbing_stairs.cpython-310.pyc","description":"不应提交 pyc","recommendation":"移除 __pycache__ 和 .pyc 文件"}]}"#
                } else {
                    r#"{"verdict":"approve","summary":"review ok","findings":[]}"#
                };
                tx.try_send(StreamChunk::Done {
                    full_output: full_output.to_string(),
                })
                .expect("send code review done");
            }
            AdapterRole::Reviewer => {
                tx.try_send(StreamChunk::Done {
                    full_output: r#"{"verdict":"approve","summary":"internal review ok","findings":[],"impact_scope":["src"],"pr_description":"实现 work item","commit_message_suggestion":"feat: implement work item"}"#.to_string(),
                })
                .expect("send internal review done");
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

fn start_web_test_provider_driven_testing_session(
    prompt: &str,
    cancel: CancellationToken,
) -> Result<ProviderSession, ProviderAdapterError> {
    let Some(output) = web_test_provider_driven_testing_output(prompt) else {
        return Err(ProviderAdapterError::execution_failed(
            None,
            String::new(),
            "streaming provider start is not implemented",
            0,
        ));
    };
    let (event_tx, event_rx) = mpsc::channel(8);
    let (command_tx, _command_rx) = mpsc::channel(8);

    tokio::spawn(async move {
        if cancel.is_cancelled() {
            return;
        }
        if event_tx
            .send(ProviderEvent::TextDelta {
                content: output.clone(),
            })
            .await
            .is_err()
        {
            return;
        }
        if cancel.is_cancelled() {
            return;
        }
        let _ = event_tx
            .send(ProviderEvent::Completed {
                full_output: output,
                provider_session_id: None,
            })
            .await;
    });

    Ok(ProviderSession {
        events: event_rx,
        commands: command_tx,
    })
}

fn web_test_provider_driven_testing_output(prompt: &str) -> Option<String> {
    if prompt.contains("Tester Provider Runtime") && prompt.contains("Phase: plan_tests") {
        return Some(
            json!({
                "summary": "web integration provider-driven test plan",
                "steps": [{
                    "id": "cargo_test",
                    "title": "Cargo test",
                    "intent": "verify the coding worktree with the provider-managed test fixture",
                    "required": true,
                    "tool": "provider_managed",
                    "risk_level": "low",
                    "command_or_tool_input": {
                        "command": "cargo test --locked"
                    },
                    "evidence_expectation": "provider reports deterministic cargo test evidence",
                    "related_requirements": ["REQ-CARGO"],
                    "related_design_constraints": ["DEC-CARGO"],
                    "related_work_item_tasks": ["TASK-CARGO"]
                }]
            })
            .to_string(),
        );
    }
    if prompt.contains("Tester Provider Runtime") && prompt.contains("Phase: execute_test_plan") {
        return Some(
            json!({
                "step_results": [{
                    "step_id": "cargo_test",
                    "status": "passed",
                    "evidence_refs": ["web-it-provider-driven-testing.log"],
                    "provider_analysis": "web integration fixture completed deterministic provider-managed testing"
                }]
            })
            .to_string(),
        );
    }
    None
}

struct HangingCodingProvider;

#[async_trait::async_trait]
impl StreamingProviderAdapter for HangingCodingProvider {
    async fn run_streaming(
        &self,
        input: &AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        let (tx, rx) = mpsc::channel(8);
        if input.role == AdapterRole::Executor {
            tokio::spawn(async move {
                let _ = tx
                    .send(StreamChunk::Text("hanging provider started".to_string()))
                    .await;
                tokio::time::sleep(Duration::from_secs(60)).await;
            });
        } else if input.output_schema == "coding_workspace_analyst_verdict_json" {
            tx.try_send(StreamChunk::Done {
                full_output: r#"{"verdict":"no_issue","summary":"testing ok"}"#.to_string(),
            })
            .expect("send analyst done");
        } else {
            tx.try_send(StreamChunk::Done {
                full_output: r#"{"verdict":"approve","summary":"review ok","findings":[]}"#
                    .to_string(),
            })
            .expect("send done");
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

const CLIMB_STAIRS_WORK_ITEM: &str = r#"# 实现爬楼梯问题 Work Item

请使用 python 实现函数 `climb_stairs(n: i32) -> i32`，覆盖 n=1、n=2、n=3、n=5、n=10。

## 验证命令

```bash
uv run python -m unittest discover -s tests -v
```
"#;

struct RetryAnalystCaptureProvider {
    captured_prompts: Arc<Mutex<Vec<String>>>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for RetryAnalystCaptureProvider {
    async fn run_streaming(
        &self,
        input: &AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        self.captured_prompts
            .lock()
            .expect("lock")
            .push(input.prompt.clone());
        let (tx, rx) = mpsc::channel(8);
        tx.try_send(StreamChunk::Done {
            full_output: r#"{"verdict":"proceed","next_stage":"code_review","reason":"retry analyst accepted from test","evidence_refs":["artifacts/rework/analyst_evidence_0001.txt"],"raw_provider_output_refs":[]}"#.to_string(),
        })
        .expect("send done");
        Ok(rx)
    }
}

struct RetryInternalReviewCaptureProvider {
    captured_prompts: Arc<Mutex<Vec<String>>>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for RetryInternalReviewCaptureProvider {
    async fn run_streaming(
        &self,
        input: &AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        self.captured_prompts
            .lock()
            .expect("lock")
            .push(input.prompt.clone());
        let (tx, rx) = mpsc::channel(8);
        let full_output = if input.output_schema == "coding_workspace_internal_pr_review_json" {
            r#"{"verdict":"approve","summary":"internal reviewer retry accepted","findings":[],"impact_scope":["src"],"pr_description":"PR body","commit_message_suggestion":"feat: work"}"#
        } else if input.output_schema == "coding_workspace_analyst_verdict_json" {
            r#"{"verdict":"proceed","next_stage":"final_confirm","reason":"internal reviewer retry accepted"}"#
        } else {
            r#"{"verdict":"approve","summary":"review ok","findings":[]}"#
        };
        tx.try_send(StreamChunk::Done {
            full_output: full_output.to_string(),
        })
        .expect("send done");
        Ok(rx)
    }
}

fn app_with_blocked_analyst_attempt(
    root_path: &Path,
    provider: Arc<dyn StreamingProviderAdapter>,
) -> (axum::Router, CodingAttemptStore) {
    let repo = root_path.join("repo");
    init_cargo_repo(&repo);

    let app_paths = ProductAppPaths::new(root_path.join(".aria"));
    let repository = RepositoryStore::new(app_paths.clone())
        .create(CreateRepositoryInput {
            project_id: "project_0001".to_string(),
            name: "repo".to_string(),
            path: repo.clone(),
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
    let store = CodingAttemptStore::new(app_paths.clone());
    store
        .create_attempt(CreateCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: "work_item_0001".to_string(),
            base_branch: "HEAD".to_string(),
            branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
            worktree_path: Some(repo),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Fake,
                reviewer: Some(ProviderName::Fake),
                review_rounds: 1,
            },
            max_auto_rework: 2,
        })
        .expect("create attempt");

    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::Fake, provider);
    let router = build_web_router(WebAppState::with_provider_registry(
        root_path.to_path_buf(),
        WebRuntime::new_fake(root_path.to_path_buf()),
        registry,
    ));
    (router, store)
}

#[tokio::test]
async fn coding_ws_retry_analyst_resumes_rework_from_persisted_evidence() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let captured = Arc::new(Mutex::new(Vec::new()));
    let (app, store) = app_with_blocked_analyst_attempt(
        root.path(),
        Arc::new(RetryAnalystCaptureProvider {
            captured_prompts: captured.clone(),
        }),
    );

    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            "coding_attempt_0001",
            CodingAttemptStatus::Running,
        )
        .expect("running");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            "coding_attempt_0001",
            CodingAttemptStatus::Blocked,
        )
        .expect("block attempt");
    store
        .update_attempt_stage(
            "project_0001",
            "issue_0001",
            "coding_attempt_0001",
            CodingExecutionStage::Rework,
        )
        .expect("set rework stage");

    let first_run = store
        .create_role_run(
            &store
                .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
                .expect("get attempt"),
            CodingExecutionStage::Rework,
            CodingProviderRole::Analyst,
            CodingRoleRunTrigger::Initial,
            Some("coding_node_0001".to_string()),
        )
        .expect("create first run");
    store
        .update_role_run_status(
            "project_0001",
            "issue_0001",
            "coding_attempt_0001",
            &first_run.id,
            CodingRoleRunStatus::Blocked,
            Some("analyst_human_gate".to_string()),
        )
        .expect("block first run");

    fs::create_dir_all(
        root.path()
            .join(".aria")
            .join("projects")
            .join("project_0001")
            .join("issues")
            .join("issue_0001")
            .join("coding-attempts")
            .join("coding_attempt_0001")
            .join("artifacts")
            .join("rework"),
    )
    .expect("create evidence dir");
    fs::write(
        root.path()
            .join(".aria")
            .join("projects")
            .join("project_0001")
            .join("issues")
            .join("issue_0001")
            .join("coding-attempts")
            .join("coding_attempt_0001")
            .join("artifacts")
            .join("rework")
            .join("analyst_evidence_0001.txt"),
        "persisted testing evidence",
    )
    .expect("write evidence");
    store
        .update_role_run_refs(
            "project_0001",
            "issue_0001",
            "coding_attempt_0001",
            &first_run.id,
            Vec::new(),
            vec!["artifacts/rework/analyst_evidence_0001.txt".to_string()],
        )
        .expect("add evidence ref");
    store
        .append_role_run_event(
            &store
                .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
                .expect("get attempt"),
            &first_run,
            CodingRoleRunEventType::ExecutionEvent,
            serde_json::json!({
                "title": "Analyst task update",
                "status": "running",
                "detail": "No tasks found"
            }),
        )
        .expect("append analyst event");

    store
        .create_blocked_gate(CreateBlockedGateInput {
            attempt_id: "coding_attempt_0001".to_string(),
            stage: CodingExecutionStage::Rework,
            node_id: Some("coding_node_0001".to_string()),
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
        &CodingWsInMessage::GateResponse {
            gate_id: "coding_blocked_gate_0001".to_string(),
            action_id: "retry_analyst".to_string(),
            extra_context: None,
        },
    )
    .await;

    let mut saw_rework_node = false;
    for _ in 0..240 {
        match timeout(Duration::from_millis(500), recv_json(&mut ws)).await {
            Ok(CodingWsOutMessage::CodingTimelineNodeCreated { node })
                if node.stage == CodingExecutionStage::Rework =>
            {
                saw_rework_node = true;
            }
            Ok(CodingWsOutMessage::CodingSessionState { ref stage, .. })
                if saw_rework_node && stage == &CodingExecutionStage::CodeReview =>
            {
                break;
            }
            Ok(CodingWsOutMessage::CodingProtocolError { code, message }) => {
                panic!("unexpected protocol error {code}: {message}");
            }
            _ => {}
        }
    }
    assert!(saw_rework_node, "expected new rework timeline node");

    let runs = store
        .list_role_runs("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("role runs");
    assert_eq!(runs.len(), 2);
    {
        let prompts = captured.lock().expect("lock");
        let prompt = prompts
            .iter()
            .find(|prompt| prompt.contains("persisted testing evidence"))
            .expect("expected analyst prompt to contain persisted evidence");
        assert!(prompt.contains("[previous_role_run_diagnostic]"));
        assert!(prompt.contains("Analyst task update"));
        assert!(prompt.contains("No tasks found"));
        assert_eq!(prompt.matches("[previous_role_run_diagnostic]").count(), 1);
        assert!(!prompt.contains(&format!("role_run_id: {}", runs[1].id)));
    }

    ws.close(None).await.expect("close ws");
    server.abort();
}

#[tokio::test]
async fn coding_ws_retry_internal_review_resumes_internal_reviewer_run() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let captured = Arc::new(Mutex::new(Vec::new()));
    let (app, store) = app_with_blocked_analyst_attempt(
        root.path(),
        Arc::new(RetryInternalReviewCaptureProvider {
            captured_prompts: captured.clone(),
        }),
    );

    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            "coding_attempt_0001",
            CodingAttemptStatus::Running,
        )
        .expect("running");
    store
        .update_attempt_stage(
            "project_0001",
            "issue_0001",
            "coding_attempt_0001",
            CodingExecutionStage::InternalPrReview,
        )
        .expect("set internal review stage");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            "coding_attempt_0001",
            CodingAttemptStatus::Blocked,
        )
        .expect("block attempt");
    store
        .save_review_request(&ReviewRequest {
            id: "review_request_0001".to_string(),
            attempt_id: "coding_attempt_0001".to_string(),
            kind: ReviewRequestKind::GitBranchOnly,
            remote_kind: RemoteKind::GenericGit,
            remote: "origin".to_string(),
            base_branch: "HEAD".to_string(),
            branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
            commit_sha: "abc123".to_string(),
            push_status: PushStatus::Pushed,
            external_url: None,
            manual_instructions: Vec::new(),
            created_at: "2026-06-13T00:00:00Z".to_string(),
            updated_at: "2026-06-13T00:00:00Z".to_string(),
        })
        .expect("save review request");
    let first_run = store
        .create_role_run(
            &store
                .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
                .expect("get attempt"),
            CodingExecutionStage::InternalPrReview,
            CodingProviderRole::InternalReviewer,
            CodingRoleRunTrigger::Initial,
            Some("coding_node_0001".to_string()),
        )
        .expect("create first run");
    store
        .update_role_run_status(
            "project_0001",
            "issue_0001",
            "coding_attempt_0001",
            &first_run.id,
            CodingRoleRunStatus::Blocked,
            Some("internal_review_blocked".to_string()),
        )
        .expect("block first run");
    store
        .append_role_run_event(
            &store
                .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
                .expect("get attempt"),
            &first_run,
            CodingRoleRunEventType::ExecutionEvent,
            serde_json::json!({
                "title": "Internal reviewer task update",
                "status": "blocked",
                "detail": "internal_review_blocked"
            }),
        )
        .expect("append internal reviewer event");
    store
        .create_blocked_gate(CreateBlockedGateInput {
            attempt_id: "coding_attempt_0001".to_string(),
            stage: CodingExecutionStage::InternalPrReview,
            node_id: Some("coding_node_0001".to_string()),
            role: Some(CodingProviderRole::InternalReviewer),
            title: "Internal Reviewer blocked".to_string(),
            description: "需要重跑 Internal Reviewer".to_string(),
            reason_code: Some("internal_review_blocked".to_string()),
            evidence_refs: Vec::new(),
            raw_provider_output_ref: None,
            available_actions: vec![CodingGateAction {
                action_id: "retry_internal_review".to_string(),
                label: "重试 Internal Reviewer".to_string(),
                action_type: CodingGateActionType::RetryInternalReview,
            }],
        })
        .expect("create gate");

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
        &CodingWsInMessage::GateResponse {
            gate_id: "coding_blocked_gate_0001".to_string(),
            action_id: "retry_internal_review".to_string(),
            extra_context: None,
        },
    )
    .await;

    let mut saw_internal_node = false;
    let mut saw_internal_complete = false;
    let mut saw_internal_chat = false;
    for _ in 0..40 {
        match timeout(Duration::from_millis(250), recv_json(&mut ws)).await {
            Ok(CodingWsOutMessage::CodingTimelineNodeCreated { node })
                if node.stage == CodingExecutionStage::InternalPrReview =>
            {
                saw_internal_node = true;
            }
            Ok(CodingWsOutMessage::CodingTimelineNodeCreated { node })
                if node.stage == CodingExecutionStage::CodeReview =>
            {
                panic!("retry_internal_review should not resume CodeReview first");
            }
            Ok(CodingWsOutMessage::InternalPrReviewComplete { review }) => {
                assert_eq!(review.summary, "internal reviewer retry accepted");
                saw_internal_complete = true;
            }
            Ok(CodingWsOutMessage::CodingChatEntryCreated { entry })
                if entry.content.as_deref().is_some_and(|content| {
                    content.contains("internal reviewer retry accepted")
                }) =>
            {
                let metadata = entry.metadata.unwrap_or_default();
                assert_eq!(
                    metadata.get("source").and_then(|value| value.as_str()),
                    Some("internal_pr_review")
                );
                saw_internal_chat = true;
            }
            Ok(CodingWsOutMessage::CodingProtocolError { code, message }) => {
                panic!("unexpected protocol error {code}: {message}");
            }
            _ => {}
        }
        if saw_internal_node && saw_internal_complete && saw_internal_chat {
            break;
        }
    }
    assert!(saw_internal_node, "expected new internal review node");
    assert!(
        saw_internal_complete,
        "expected internal reviewer completion"
    );
    assert!(
        saw_internal_chat,
        "expected readable internal reviewer chat"
    );

    {
        let prompts = captured.lock().expect("lock");
        assert!(
            prompts
                .iter()
                .any(|prompt| prompt.contains("review_request_0001")
                    && prompt.contains("[previous_role_run_diagnostic]")
                    && prompt.contains("internal_review_blocked")),
            "expected internal reviewer prompt to contain review request and retry diagnostic"
        );
    }

    let runs = store
        .list_role_runs("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("role runs");
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].status, CodingRoleRunStatus::Superseded);
    assert_eq!(runs[1].role, CodingProviderRole::InternalReviewer);
    assert_eq!(runs[1].trigger, CodingRoleRunTrigger::RetryInternalReview);
    assert_eq!(runs[1].status, CodingRoleRunStatus::Completed);
    let reviews = store
        .list_internal_pr_reviews("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("internal reviews");
    assert_eq!(reviews.len(), 1);
    assert_eq!(reviews[0].role_run_id.as_deref(), Some(runs[1].id.as_str()));

    ws.close(None).await.expect("close ws");
    server.abort();
}
