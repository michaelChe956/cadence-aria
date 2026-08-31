use super::*;
use crate::product::models::{
    WorkspaceRolePermissionModes, WorkspaceSessionRecord, WorkspaceSessionStatus,
};
use crate::product::work_item_plan_policy::{RunHistory, RunPolicy, WorkItemPlanFlowKind};
use crate::web::workspace_ws_types::{WorkItemBatchDecisionDto, WorkItemDraftDecisionDto};
use std::time::Duration;
use tokio::time::timeout;

#[test]
fn single_candidate_scope_rejection_parser_preserves_scope_marker() {
    let envelope = parse_workspace_inbound_text(
        r#"{"type":"ping","scope":{"phase":"initial","initial_revision_id":"client"}}"#,
    )
    .expect("raw envelope should parse");
    assert!(matches!(envelope.message, WsInMessage::Ping));
    assert!(envelope.submitted_fields.contains("scope"));
}

#[test]
fn single_candidate_scope_rejection_parser_preserves_review_invocation_scope_marker() {
    let envelope = parse_workspace_inbound_text(
        r#"{"type":"ping","review_invocation_scope":{"phase":"initial","initial_revision_id":"client"}}"#,
    )
    .expect("raw envelope should parse");
    assert!(matches!(envelope.message, WsInMessage::Ping));
    assert!(
        envelope
            .submitted_fields
            .contains("review_invocation_scope")
    );
}

#[test]
fn single_candidate_scope_rejection_parser_preserves_legacy_review_scope_marker() {
    let envelope = parse_workspace_inbound_text(
        r#"{"type":"ping","review_scope":{"phase":"initial","initial_revision_id":"client"}}"#,
    )
    .expect("raw envelope should parse");
    assert!(matches!(envelope.message, WsInMessage::Ping));
    assert!(envelope.submitted_fields.contains("review_scope"));
}

#[tokio::test]
async fn single_candidate_scope_rejection_rejects_all_forbidden_markers_without_mutation() {
    for field in ["scope", "review_invocation_scope", "review_scope"] {
        let (context, engine, mut outbound_rx, mut events) =
            scope_test_context(WorkItemPlanFlowKind::SingleCandidate);
        let envelope = parse_workspace_inbound_text(&format!(
            r#"{{"type":"ping","{field}":{{"client":"must_not_be_read"}}}}"#
        ))
        .expect("raw envelope should parse");
        let before = scope_test_snapshot(&engine).await;

        handle_workspace_inbound_message(context, envelope).await;

        let outbound = outbound_rx.recv().await.expect("protocol error outbound");
        let OutboundControl::Text(json) = outbound else {
            panic!("expected protocol error text");
        };
        let value: serde_json::Value = serde_json::from_str(&json).expect("protocol error json");
        assert_eq!(value["type"], "protocol_error", "field={field}");
        assert_eq!(
            value["code"], "SINGLE_CANDIDATE_SCOPE_FORBIDDEN",
            "field={field}"
        );
        assert_eq!(scope_test_snapshot(&engine).await, before, "field={field}");
        assert!(events.try_recv().is_err(), "field={field}");
    }
}

#[tokio::test]
async fn single_candidate_scope_rejection_precedes_generation_decision_rejection() {
    let (context, engine, mut outbound_rx, mut events) =
        scope_test_context(WorkItemPlanFlowKind::SingleCandidate);
    let envelope = parse_workspace_inbound_text(
        r#"{"type":"select_work_item_generation_mode","mode":"batch","scope":{"client":"forbidden"}}"#,
    )
    .expect("raw envelope should parse");
    let before = scope_test_snapshot(&engine).await;

    handle_workspace_inbound_message(context, envelope).await;

    let outbound = outbound_rx.recv().await.expect("protocol error outbound");
    let OutboundControl::Text(json) = outbound else {
        panic!("expected protocol error text");
    };
    let value: serde_json::Value = serde_json::from_str(&json).expect("protocol error json");
    assert_eq!(value["code"], "SINGLE_CANDIDATE_SCOPE_FORBIDDEN");
    assert_eq!(scope_test_snapshot(&engine).await, before);
    assert!(events.try_recv().is_err());
}

#[tokio::test]
async fn single_candidate_generation_decision_rejection_preserves_phase_history_and_events() {
    let messages = [
        WsInMessage::SelectWorkItemGenerationMode {
            mode: WorkItemGenerationModeDto::Batch,
        },
        WsInMessage::WorkItemDraftDecision {
            outline_id: "outline_client_supplied".to_string(),
            decision: WorkItemDraftDecisionDto::Accept,
            feedback: None,
        },
        WsInMessage::WorkItemBatchDecision {
            decision: WorkItemBatchDecisionDto::AcceptAll,
            feedback: None,
            first_affected_outline_id: None,
        },
    ];

    for message in messages {
        let (context, engine, mut outbound_rx, mut events) =
            scope_test_context(WorkItemPlanFlowKind::SingleCandidate);
        let before = scope_test_snapshot(&engine).await;

        handle_workspace_inbound_message(context, message).await;

        let outbound = outbound_rx.recv().await.expect("protocol error outbound");
        let OutboundControl::Text(json) = outbound else {
            panic!("expected protocol error text");
        };
        let value: serde_json::Value = serde_json::from_str(&json).expect("protocol error json");
        assert_eq!(value["type"], "protocol_error");
        assert_eq!(
            value["code"],
            "SINGLE_CANDIDATE_GENERATION_DECISION_FORBIDDEN"
        );
        assert_eq!(scope_test_snapshot(&engine).await, before);
        assert!(events.try_recv().is_err());
    }
}

#[tokio::test]
async fn single_candidate_scope_rejection_does_not_wait_for_provider_engine_lock() {
    let (context, engine, mut outbound_rx, _events) =
        scope_test_context(WorkItemPlanFlowKind::SingleCandidate);
    let envelope =
        parse_workspace_inbound_text(r#"{"type":"abort"}"#).expect("raw envelope should parse");
    let (command_tx, mut command_rx) = mpsc::channel(1);
    context
        .current_run
        .lock()
        .await
        .replace(WorkspaceActiveRun {
            id: 1,
            token: 1,
            node_id: None,
            cancel: CancellationToken::new(),
            command_tx,
            pending_choice_ids: Arc::new(Mutex::new(std::collections::HashSet::new())),
        });
    let engine_guard = engine.lock().await;

    timeout(
        Duration::from_millis(100),
        handle_workspace_inbound_message(context, envelope),
    )
    .await
    .expect("running abort must not await provider-held engine lock");
    drop(engine_guard);

    assert!(matches!(
        timeout(Duration::from_millis(100), command_rx.recv())
            .await
            .expect("abort command timeout"),
        Some(ProviderCommand::Abort)
    ));
    let outbound = outbound_rx.recv().await.expect("aborted status outbound");
    let OutboundControl::Text(json) = outbound else {
        panic!("expected provider status text");
    };
    let value: serde_json::Value = serde_json::from_str(&json).expect("provider status json");
    assert_eq!(value["type"], "provider_status");
    assert_eq!(value["status"], "aborted");
}

#[tokio::test]
async fn single_candidate_scope_rejection_keeps_legacy_unknown_scope_compatible() {
    let (context, engine, mut outbound_rx, mut events) =
        scope_test_context(WorkItemPlanFlowKind::Legacy);
    let envelope =
        parse_workspace_inbound_text(r#"{"type":"ping","scope":{"client":"legacy_compatible"}}"#)
            .expect("raw envelope should parse");
    let before = scope_test_snapshot(&engine).await;

    handle_workspace_inbound_message(context, envelope).await;

    let outbound = outbound_rx.recv().await.expect("pong outbound");
    let OutboundControl::Text(json) = outbound else {
        panic!("expected pong text");
    };
    let value: serde_json::Value = serde_json::from_str(&json).expect("pong json");
    assert_eq!(value["type"], "pong");
    assert_eq!(scope_test_snapshot(&engine).await, before);
    assert!(events.try_recv().is_err());
}

async fn scope_test_snapshot(engine: &Arc<Mutex<WorkspaceEngine>>) -> Vec<u8> {
    let engine = engine.lock().await;
    serde_json::to_vec(&(
        engine.session().stage.as_str(),
        &engine.session().session_status,
        engine.session().flow_kind,
        &engine.session().run_history,
        &engine.session().single_candidate_phase,
        &engine.session().review_invocation_scope,
        engine
            .session()
            .messages
            .iter()
            .map(|message| {
                (
                    message.id.as_str(),
                    message.role.as_str(),
                    message.content.as_str(),
                    message.checkpoint_id.as_deref(),
                    message.created_at.as_str(),
                )
            })
            .collect::<Vec<_>>(),
        &engine.timeline_nodes,
    ))
    .expect("session and timeline snapshot")
}

pub(super) fn scope_test_context(
    flow_kind: WorkItemPlanFlowKind,
) -> (
    WorkspaceInboundContext,
    Arc<Mutex<WorkspaceEngine>>,
    mpsc::Receiver<OutboundControl>,
    mpsc::Receiver<EngineEvent>,
) {
    let root = tempfile::tempdir().expect("tempdir");
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let session_record = WorkspaceSessionRecord {
        id: "workspace_scope_test".to_string(),
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        entity_id: "plan_0001".to_string(),
        workspace_type: WorkspaceType::WorkItemPlan,
        status: WorkspaceSessionStatus::Open,
        author_provider: ProviderName::Codex,
        reviewer_provider: ProviderName::Codex,
        review_rounds: 0,
        permission_modes: WorkspaceRolePermissionModes::default(),
        provisional_reviewer_provider: None,
        reviewer_enabled_at_start: None,
        superpowers_enabled: false,
        openspec_enabled: false,
        flow_kind,
        run_policy: RunPolicy::Interactive,
        run_history: RunHistory::default(),
        review_invocation_scope: None,
        human_gate_snapshot: None,
        repair_reservation: None,
        human_gate_reservation: None,
        policy_diagnostics: vec![],
        provider_start_ledger: vec![],
        single_candidate_phase: None,
        work_item_plan_source_revision_ref: None,
        plan_candidate_ir_ref: None,
        mechanical_report_ref: None,
        publication_provenance_ref: None,
        approval_attempt_id: None,
        approved_at: None,
        compile_reservation: None,
        work_item_runtime_binding: None,
        provider_conversations: vec![],
        messages: vec![],
        created_at: "2026-08-27T00:00:00Z".to_string(),
        updated_at: "2026-08-27T00:00:00Z".to_string(),
    };
    let session = WorkspaceSession::from_record(session_record.clone());
    let (event_tx, event_rx) = mpsc::channel(8);
    let engine = Arc::new(Mutex::new(WorkspaceEngine::new(
        Arc::new(CheckpointStore::new(root.path().join("checkpoints"))),
        event_tx,
        session,
    )));
    let current_run = Arc::new(Mutex::new(None));
    let workspace_runs = WorkspaceRunRegistry::default();
    let (outbound_tx, outbound_rx) = mpsc::channel(8);
    let context = WorkspaceInboundContext {
        app_state: WebAppState::new(
            root.path().to_path_buf(),
            crate::web::runtime::WebRuntime::new_fake(root.path().to_path_buf()),
        ),
        engine: engine.clone(),
        run_context: ProviderRunContext {
            provider_registry: Arc::new(ProviderRegistry::new()),
            engine: engine.clone(),
            current_run: current_run.clone(),
            workspace_runs: workspace_runs.clone(),
            session_id: session_record.id.clone(),
            next_run_id: Arc::new(Mutex::new(0)),
            app_paths,
            session_record,
        },
        outbound_tx,
        current_run,
        workspace_runs,
        session_id: "workspace_scope_test".to_string(),
    };
    (context, engine, outbound_rx, event_rx)
}
