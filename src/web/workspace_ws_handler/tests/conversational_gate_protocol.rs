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

#[tokio::test]
async fn conversational_gate_post_approve_feedback_is_structured_protocol_error() {
    use crate::product::lifecycle_store::{
        CreateWorkspaceSessionInput, LifecycleStore, WorkItemPlanSessionOptions,
    };
    use crate::product::models::{SingleCandidatePhase, WorkspaceSessionStatus, WorkspaceType};
    use crate::product::work_item_plan_policy::{HumanGateSnapshot, HumanReason, RunPolicy};
    use tempfile::tempdir;

    let root = tempdir().expect("tempdir");
    let app_paths = crate::product::app_paths::ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let mut record = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: "plan_post_approve_feedback".to_string(),
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
    // approve 成功后的持久层状态:close CAS 目标 Running 不清快照,phase CAS
    // Confirmed+Completed 也不清快照(lifecycle_store workspace.rs /
    // workspace_single_candidate.rs)。无任何 Open/Applying PlanAmendmentContext。
    record.status = WorkspaceSessionStatus::Confirmed;
    record.single_candidate_phase = Some(SingleCandidatePhase::Completed);
    record.human_gate_snapshot = Some(HumanGateSnapshot {
        findings: Vec::new(),
        repeated_fingerprints: Vec::new(),
        attempts_used: 0,
        manual_repairs_remaining: 1,
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
            app_paths: app_paths.clone(),
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
            command_id: "cmd_post_approve_feedback".to_string(),
            feedback: "只修正这个字段".to_string(),
        },
    )
    .await;

    let outbound = outbound_rx.recv().await.expect("protocol error outbound");
    let OutboundControl::Text(json) = outbound else {
        panic!("expected protocol error text");
    };
    let message: WsOutMessage = serde_json::from_str(&json).expect("protocol error json");
    let WsOutMessage::ProtocolError { code, .. } = &message else {
        panic!("post-approve feedback must stay a structured protocol error, got {message:?}");
    };
    assert_eq!(code, "WORK_ITEM_PLAN_HUMAN_GATE_STAGE_INVALID");
    assert_eq!(
        serde_json::to_vec(
            &lifecycle
                .get_workspace_session(&record.id)
                .expect("durable session")
        )
        .expect("serialize session after"),
        before,
        "post-approve feedback rejection must not mutate durable state"
    );
    assert!(
        lifecycle
            .list_human_gate_turns(&record.id)
            .expect("turns")
            .is_empty()
    );
    assert!(
        event_rx.try_recv().is_err(),
        "stage rejection must not emit events"
    );
}

#[tokio::test]
async fn confirm_compile_failure_surfaces_findings_as_protocol_error_context() {
    // F7 项 1（历史观察项族 12）：SC Approval 门 confirm 后 compile 失败，
    // validator findings 数组必须经 ProtocolError.context 上抛（WS 客户端可见）。
    use crate::product::lifecycle_store::{
        CreateWorkspaceSessionInput, LifecycleStore, WorkItemPlanSessionOptions,
    };
    use crate::product::models::{
        IssueWorkItemDependencyEdge, IssueWorkItemPlan, IssueWorkItemPlanOptions,
        IssueWorkItemPlanStatus, SingleCandidatePhase, WorkItemPlanCommitState,
        WorkItemPlanCompileStatus, WorkItemPlanCompileTransaction, WorkItemSplitFinding,
        WorkItemSplitFindingSeverity, WorkspaceSessionStatus, WorkspaceType,
    };
    use crate::product::work_item_plan_policy::{
        HumanGateSnapshot, HumanReason, RunPolicy, WorkItemPlanFlowKind,
    };
    use crate::product::work_item_plan_store::WorkItemPlanStore;
    use tempfile::tempdir;

    let root = tempdir().expect("tempdir");
    let app_paths = crate::product::app_paths::ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let mut record = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: "plan_gate_findings".to_string(),
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
    record.flow_kind = WorkItemPlanFlowKind::SingleCandidate;
    record.single_candidate_phase = Some(SingleCandidatePhase::Approval);
    record.human_gate_snapshot = Some(HumanGateSnapshot {
        findings: Vec::new(),
        repeated_fingerprints: Vec::new(),
        attempts_used: 0,
        manual_repairs_remaining: 1,
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

    let findings = vec![
        WorkItemSplitFinding {
            severity: WorkItemSplitFindingSeverity::Error,
            code: "WI_DEP_CYCLE".to_string(),
            message: "work item dependency cycle: wi_a -> wi_b -> wi_a".to_string(),
            work_item_ids: vec!["wi_a".to_string(), "wi_b".to_string()],
        },
        WorkItemSplitFinding {
            severity: WorkItemSplitFindingSeverity::Warning,
            code: "WI_MISSING_VERIFICATION".to_string(),
            message: "work item wi_c has no verification plan".to_string(),
            work_item_ids: vec!["wi_c".to_string()],
        },
    ];
    let failure_reason = "Final Compile strict validator failed（errors: 1, warnings: 1）";
    WorkItemPlanStore::new(app_paths.clone())
        .put_compile_transaction(&WorkItemPlanCompileTransaction {
            compile_id: "compile_gate_findings".to_string(),
            project_id: record.project_id.clone(),
            issue_id: record.issue_id.clone(),
            plan_id: record.entity_id.clone(),
            flow_kind: Some(WorkItemPlanFlowKind::SingleCandidate),
            source_revision_id: None,
            source_revision_ref: None,
            plan_candidate_ir_ref: None,
            mechanical_report_ref: None,
            publication_provenance_ref: None,
            publication_provenance_content_hash: None,
            generation_round_id: "round_gate_findings".to_string(),
            outline_version_ref: "outline_gate_findings".to_string(),
            active_draft_ids: Vec::new(),
            status: WorkItemPlanCompileStatus::Failed,
            plan_commit_state: WorkItemPlanCommitState::NotStarted,
            step_cursor: "validating".to_string(),
            outline_to_work_item_id: Default::default(),
            outline_to_verification_plan_id: Default::default(),
            created_work_item_ids: Vec::new(),
            created_verification_plan_ids: Vec::new(),
            child_session_ids: Vec::new(),
            validator_findings: findings,
            abort_requested_at: None,
            failure_reason: Some(failure_reason.to_string()),
            previous_plan_snapshot: IssueWorkItemPlan {
                id: record.entity_id.clone(),
                project_id: record.project_id.clone(),
                issue_id: record.issue_id.clone(),
                source_story_spec_ids: Vec::new(),
                source_design_spec_ids: Vec::new(),
                options: IssueWorkItemPlanOptions {
                    include_integration_tests: false,
                    include_e2e_tests: false,
                    force_frontend_backend_split: false,
                    require_execution_plan_confirm: false,
                },
                status: IssueWorkItemPlanStatus::Draft,
                work_item_ids: Vec::new(),
                repository_profile_ref: None,
                verification_plan_ids: Vec::new(),
                dependency_graph: vec![IssueWorkItemDependencyEdge {
                    from_work_item_id: "wi_a".to_string(),
                    to_work_item_id: "wi_b".to_string(),
                }],
                created_from_provider_run: None,
                validator_findings: Vec::new(),
                review_summary: None,
                created_at: "2026-09-04T00:00:00Z".to_string(),
                updated_at: "2026-09-04T00:00:00Z".to_string(),
            },
            created_at: "2026-09-04T00:00:01Z".to_string(),
            updated_at: "2026-09-04T00:00:01Z".to_string(),
            committed_at: None,
        })
        .expect("seed failed compile transaction");

    let (event_tx, _event_rx) = mpsc::channel(8);
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
        session_id: "confirm_gate_findings".to_string(),
    };

    handle_workspace_inbound_message(context, WsInMessage::Confirm).await;

    let outbound = outbound_rx
        .recv()
        .await
        .expect("confirm compile failure must reach the client");
    let OutboundControl::Text(json) = outbound else {
        panic!("expected text protocol error");
    };
    let message: WsOutMessage = serde_json::from_str(&json).expect("protocol error json");
    let WsOutMessage::ProtocolError {
        code,
        message,
        context,
    } = message
    else {
        panic!(
            "confirm compile failure must surface as a protocol error with findings, got {message:?}"
        )
    };
    assert_eq!(code, "SINGLE_CANDIDATE_APPROVAL_COMPILE_FAILED");
    assert!(message.contains("human gate remains open"), "{message}");
    assert!(message.contains("WI_DEP_CYCLE"), "{message}");
    let context = context.expect("findings context");
    assert_eq!(context["failure_reason"], serde_json::json!(failure_reason));
    let findings = context["findings"].as_array().expect("findings array");
    assert_eq!(findings.len(), 2);
    assert_eq!(findings[0]["severity"], serde_json::json!("error"));
    assert_eq!(findings[0]["code"], serde_json::json!("WI_DEP_CYCLE"));
    assert_eq!(
        findings[0]["message"],
        serde_json::json!("work item dependency cycle: wi_a -> wi_b -> wi_a")
    );
    assert_eq!(
        findings[0]["work_item_ids"],
        serde_json::json!(["wi_a", "wi_b"])
    );
    assert_eq!(findings[1]["severity"], serde_json::json!("warning"));
    assert_eq!(
        findings[1]["code"],
        serde_json::json!("WI_MISSING_VERIFICATION")
    );
}
