use super::*;
use crate::product::models::WorkspaceSessionStatus;

#[test]
fn conversational_gate_inbound_roundtrips_and_preserves_command_ids() {
    let feedback_json = serde_json::json!({
        "type": "human_gate_feedback",
        "command_id": "cmd-001",
        "feedback": "保留其余内容，只修正这个字段"
    });
    let feedback: WsInMessage = serde_json::from_value(feedback_json).unwrap();
    assert_eq!(
        feedback,
        WsInMessage::HumanGateFeedback {
            command_id: "cmd-001".to_string(),
            feedback: "保留其余内容，只修正这个字段".to_string(),
        }
    );
    let feedback_wire = serde_json::to_value(&feedback).unwrap();
    assert_eq!(feedback_wire["type"], "human_gate_feedback");
    assert_eq!(feedback_wire["command_id"], "cmd-001");
    assert_eq!(feedback_wire["feedback"], "保留其余内容，只修正这个字段");
    assert!(feedback_wire.get("HumanGateFeedback").is_none());

    let advance_json = serde_json::json!({
        "type": "advance",
        "command_id": "cmd-002"
    });
    let advance: WsInMessage = serde_json::from_value(advance_json).unwrap();
    assert_eq!(
        advance,
        WsInMessage::Advance {
            command_id: "cmd-002".to_string(),
        }
    );
    let advance_wire = serde_json::to_value(&advance).unwrap();
    assert_eq!(advance_wire["type"], "advance");
    assert_eq!(advance_wire["command_id"], "cmd-002");
    assert!(advance_wire.get("Advance").is_none());

    assert_eq!(message_type(&feedback), "human_gate_feedback");
    assert_eq!(message_type(&advance), "advance");
}

#[test]
fn conversational_gate_rejects_blank_command_id_at_handler_boundary() {
    for command_id in ["", "   ", "\t\n"] {
        let error = validate_command_id(command_id).expect_err("blank command ID must fail");
        let WsOutMessage::ProtocolError {
            code,
            message,
            context,
        } = error
        else {
            panic!("blank command ID must produce protocol error");
        };
        assert_eq!(code, "INVALID_COMMAND_ID");
        assert!(!message.is_empty());
        assert_eq!(context, None);
    }

    assert!(validate_command_id("cmd-001").is_ok());
}

#[tokio::test]
async fn conversational_gate_feedback_reaches_service_through_socket_dispatch() {
    use crate::product::lifecycle_store::{
        CreateWorkspaceSessionInput, WorkItemPlanSessionOptions,
    };
    use crate::product::models::WorkspaceSessionStatus;
    use crate::product::work_item_plan_policy::{
        HumanGateSnapshot, HumanReason, RunPolicy, WorkItemPlanFlowKind,
    };
    use tempfile::tempdir;

    let root = tempdir().expect("tempdir");
    let app_paths = crate::product::app_paths::ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = crate::product::lifecycle_store::LifecycleStore::new(app_paths.clone());
    let mut record = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: "plan_socket_gate".to_string(),
            workspace_type: crate::product::models::WorkspaceType::WorkItemPlan,
            author_provider: ProviderName::Fake,
            reviewer_provider: ProviderName::Fake,
            review_rounds: 0,
            superpowers_enabled: false,
            openspec_enabled: false,
            work_item_plan_options: Some(WorkItemPlanSessionOptions {
                flow_kind: WorkItemPlanFlowKind::SingleCandidate,
                run_policy: RunPolicy::Interactive,
                rollout_snapshot: true,
            }),
        })
        .expect("create gate session");
    record.status = WorkspaceSessionStatus::WaitingForHuman;
    record.human_gate_snapshot = Some(HumanGateSnapshot {
        findings: Vec::new(),
        repeated_fingerprints: Vec::new(),
        attempts_used: 0,
        manual_repairs_remaining: 2,
        trigger: HumanReason::NativeHumanRequired,
        resumable: false,
    });
    crate::product::json_store::write_json(
        &app_paths
            .issue_lifecycle_root(&record.project_id, &record.issue_id)
            .join("workspace-sessions")
            .join(format!("{}.json", record.id)),
        &record,
    )
    .expect("persist gate session");

    let (event_tx, mut event_rx) = mpsc::channel(8);
    let mut session = WorkspaceSession::from_record(record.clone());
    session.artifact = Some(crate::web::workspace_ws_types::ArtifactPayload::Markdown {
        markdown: "# Work Item Plan\n".to_string(),
        diff: None,
    });
    let engine = Arc::new(Mutex::new(WorkspaceEngine::new_persistent(
        Arc::new(CheckpointStore::new(root.path().join("checkpoints"))),
        lifecycle,
        event_tx,
        session,
    )));
    let (outbound_tx, mut outbound_rx) = mpsc::channel(8);
    let current_run = Arc::new(Mutex::new(None));
    let workspace_runs = WorkspaceRunRegistry::default();
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
            session_id: record.id.clone(),
            next_run_id: Arc::new(Mutex::new(0)),
            app_paths,
            session_record: record,
        },
        outbound_tx,
        current_run,
        workspace_runs,
        session_id: "socket_gate_session".to_string(),
    };

    handle_workspace_inbound_message(
        context,
        WsInMessage::HumanGateFeedback {
            command_id: "cmd_socket_gate".to_string(),
            feedback: "只修正这个字段".to_string(),
        },
    )
    .await;

    let outbound = outbound_rx.recv().await.expect("turn open outbound");
    let OutboundControl::Text(json) = outbound else {
        panic!("expected text turn open");
    };
    let message: WsOutMessage = serde_json::from_str(&json).expect("turn open json");
    assert!(matches!(
        message,
        WsOutMessage::HumanGateTurnOpen {
            command_id,
            remaining_budget: 1,
            ..
        } if command_id == "cmd_socket_gate"
    ));
    assert!(
        event_rx.try_recv().is_err(),
        "reservation dispatch does not start provider"
    );
}
#[tokio::test]
async fn conversational_gate_budget_exhausted_reaches_handler_as_protocol_error() {
    use crate::product::lifecycle_store::{
        CreateWorkspaceSessionInput, LifecycleStore, WorkItemPlanSessionOptions,
    };
    use crate::product::models::{SingleCandidatePhase, WorkspaceSessionStatus, WorkspaceType};
    use crate::product::work_item_plan_policy::{
        HumanGateSnapshot, HumanReason, RunPolicy, WorkItemPlanFlowKind,
    };
    use tempfile::tempdir;

    let root = tempdir().expect("tempdir");
    let app_paths = crate::product::app_paths::ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let mut record = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: "plan_socket_budget_exhausted".to_string(),
            workspace_type: WorkspaceType::WorkItemPlan,
            author_provider: ProviderName::Fake,
            reviewer_provider: ProviderName::Fake,
            review_rounds: 0,
            superpowers_enabled: false,
            openspec_enabled: false,
            work_item_plan_options: Some(WorkItemPlanSessionOptions {
                flow_kind: WorkItemPlanFlowKind::SingleCandidate,
                run_policy: RunPolicy::Interactive,
                rollout_snapshot: true,
            }),
        })
        .expect("create gate session");
    record.status = WorkspaceSessionStatus::WaitingForHuman;
    record.single_candidate_phase = Some(SingleCandidatePhase::Approval);
    record.human_gate_snapshot = Some(HumanGateSnapshot {
        findings: Vec::new(),
        repeated_fingerprints: Vec::new(),
        attempts_used: 0,
        manual_repairs_remaining: 0,
        trigger: HumanReason::NativeHumanRequired,
        resumable: false,
    });
    let session_path = app_paths
        .issue_lifecycle_root(&record.project_id, &record.issue_id)
        .join("workspace-sessions")
        .join(format!("{}.json", record.id));
    crate::product::json_store::write_json(&session_path, &record).expect("persist gate session");
    let before = serde_json::to_vec(&record).expect("serialize session before");

    let (event_tx, mut event_rx) = mpsc::channel(8);
    let mut session = WorkspaceSession::from_record(record.clone());
    session.artifact = Some(crate::web::workspace_ws_types::ArtifactPayload::Markdown {
        markdown: "# Work Item Plan\n".to_string(),
        diff: None,
    });
    let engine = Arc::new(Mutex::new(WorkspaceEngine::new_persistent(
        Arc::new(CheckpointStore::new(root.path().join("checkpoints"))),
        lifecycle.clone(),
        event_tx,
        session,
    )));
    let (outbound_tx, mut outbound_rx) = mpsc::channel(8);
    let current_run = Arc::new(Mutex::new(None));
    let workspace_runs = WorkspaceRunRegistry::default();
    let context = WorkspaceInboundContext {
        app_state: WebAppState::new(
            root.path().to_path_buf(),
            crate::web::runtime::WebRuntime::new_fake(root.path().to_path_buf()),
        ),
        run_context: ProviderRunContext {
            provider_registry: Arc::new(ProviderRegistry::new()),
            engine: engine.clone(),
            current_run: current_run.clone(),
            workspace_runs: workspace_runs.clone(),
            session_id: record.id.clone(),
            next_run_id: Arc::new(Mutex::new(0)),
            app_paths,
            session_record: record.clone(),
        },
        engine: engine.clone(),
        outbound_tx,
        current_run,
        workspace_runs,
        session_id: record.id.clone(),
    };

    handle_workspace_inbound_message(
        context,
        WsInMessage::HumanGateFeedback {
            command_id: "cmd_socket_budget_exhausted".to_string(),
            feedback: "只修正这个字段".to_string(),
        },
    )
    .await;

    let outbound = outbound_rx.recv().await.expect("protocol error outbound");
    let OutboundControl::Text(json) = outbound else {
        panic!("expected protocol error text");
    };
    let error: WsOutMessage = serde_json::from_str(&json).expect("protocol error json");
    assert!(matches!(
        error,
        WsOutMessage::ProtocolError { code, .. }
            if code == "HUMAN_GATE_BUDGET_EXHAUSTED"
    ));
    assert_eq!(
        serde_json::to_vec(&lifecycle.get_workspace_session(&record.id).unwrap()).unwrap(),
        before
    );
    assert!(
        lifecycle
            .list_human_gate_turns(&record.id)
            .unwrap()
            .is_empty()
    );
    assert!(
        engine
            .lock()
            .await
            .session()
            .provider_start_ledger
            .is_empty()
    );
    assert!(event_rx.try_recv().is_err());
}
#[tokio::test]
async fn conversational_gate_wrong_message_type_has_zero_side_effects() {
    let (context, engine, mut outbound_rx, mut events) =
        super::single_candidate_scope_rejection::scope_test_context(
            WorkItemPlanFlowKind::SingleCandidate,
        );
    {
        let mut engine_guard = engine.lock().await;
        engine_guard.session.stage = WorkspaceStage::HumanConfirm;
        engine_guard.session.session_status = WorkspaceSessionStatus::WaitingForHuman;
    }
    let before = super::single_candidate_scope_rejection::scope_test_snapshot(&engine).await;

    handle_workspace_inbound_message(
        context,
        WsInMessage::UserMessage {
            content: "不应绕过人工门".to_string(),
        },
    )
    .await;

    let outbound = outbound_rx.recv().await.expect("protocol error outbound");
    let OutboundControl::Text(json) = outbound else {
        panic!("expected protocol error text");
    };
    let error: WsOutMessage = serde_json::from_str(&json).expect("protocol error json");
    assert!(matches!(
        error,
        WsOutMessage::ProtocolError { code, .. }
            if code == "WORK_ITEM_PLAN_HUMAN_GATE_STAGE_INVALID"
    ));
    assert_eq!(
        super::single_candidate_scope_rejection::scope_test_snapshot(&engine).await,
        before,
        "wrong message must not mutate session state"
    );
    assert!(
        events.try_recv().is_err(),
        "wrong message must not emit events"
    );
}
#[test]
fn conversational_gate_wrong_message_type_boundary_is_stage_specific() {
    let message = WsInMessage::UserMessage {
        content: "不应绕过人工门".to_string(),
    };
    let error = human_gate_message_boundary_error(
        WorkItemPlanFlowKind::SingleCandidate,
        WorkspaceStage::HumanConfirm,
        &message,
    )
    .expect("ordinary user message must be rejected at the human gate boundary");
    let WsOutMessage::ProtocolError { code, context, .. } = error else {
        panic!("human gate boundary must return a protocol error");
    };
    assert_eq!(code, "WORK_ITEM_PLAN_HUMAN_GATE_STAGE_INVALID");
    assert_eq!(context.expect("stage context")["stage"], "human_confirm");
    assert!(
        human_gate_message_boundary_error(
            WorkItemPlanFlowKind::Legacy,
            WorkspaceStage::HumanConfirm,
            &message,
        )
        .is_none()
    );
}
#[tokio::test]
async fn conversational_gate_blank_command_id_is_rejected_through_dispatch() {
    let (context, _engine, mut outbound_rx, _events) =
        super::single_candidate_scope_rejection::scope_test_context(
            crate::product::work_item_plan_policy::WorkItemPlanFlowKind::SingleCandidate,
        );

    handle_workspace_inbound_message(
        context,
        WsInMessage::HumanGateFeedback {
            command_id: "   ".to_string(),
            feedback: "should not be dispatched".to_string(),
        },
    )
    .await;

    let outbound = outbound_rx.recv().await.expect("protocol error outbound");
    let OutboundControl::Text(json) = outbound else {
        panic!("expected text protocol error");
    };
    let error: WsOutMessage = serde_json::from_str(&json).expect("protocol error json");
    assert!(matches!(
        error,
        WsOutMessage::ProtocolError { code, .. } if code == "INVALID_COMMAND_ID"
    ));
}
