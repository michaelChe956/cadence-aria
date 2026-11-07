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
            excluded_finding_ids: vec![],
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

    // L0 typed 重承载（REQ-RET-02）：abandon 的 wire 名 `abandon_human_gate`
    // 是 T4 前端 union 与 T5 protocol error 负向测试的契约依赖，此处钉死。
    let abandon_json = serde_json::json!({
        "type": "abandon_human_gate",
        "command_id": "cmd-003"
    });
    let abandon: WsInMessage = serde_json::from_value(abandon_json).unwrap();
    assert_eq!(
        abandon,
        WsInMessage::AbandonHumanGate {
            command_id: "cmd-003".to_string(),
        }
    );
    let abandon_wire = serde_json::to_value(&abandon).unwrap();
    assert_eq!(abandon_wire["type"], "abandon_human_gate");
    assert_eq!(abandon_wire["command_id"], "cmd-003");
    assert!(abandon_wire.get("AbandonHumanGate").is_none());

    assert_eq!(message_type(&feedback), "human_gate_feedback");
    assert_eq!(message_type(&advance), "advance");
    assert_eq!(message_type(&abandon), "abandon_human_gate");
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

/// L0 typed 重承载（双审修订锚）：SC HumanConfirm stage 白名单必须显式放行
/// typed 门命令族（HumanGateFeedback/Confirm/AbandonHumanGate）与双轨期
/// legacy Terminate 桥接——漏加 AbandonHumanGate 时 typed abandon 在真实
/// L2 重钉（T5/REQ-RET-02）：SC 门白名单=typed 三命令——HumanConfirm 双轨
/// 桥接臂已删（legacy 消息 parse 面拒收 LEGACY_MESSAGE_RETIRED，红测见
/// it_core part_07），白名单结构性收窄为 feedback/confirm/abandon。
#[test]
fn single_candidate_human_gate_stage_whitelist_accepts_typed_close_commands() {
    use crate::product::work_item_plan_policy::WorkItemPlanFlowKind;

    let accepted = [
        WsInMessage::HumanGateFeedback {
            command_id: "cmd-gate".to_string(),
            feedback: "反馈".to_string(),
            excluded_finding_ids: vec![],
        },
        WsInMessage::Confirm,
        WsInMessage::AbandonHumanGate {
            command_id: "cmd-abandon".to_string(),
        },
    ];
    for message in accepted {
        assert!(
            is_message_valid_for_stage_with_flow(
                WorkItemPlanFlowKind::SingleCandidate,
                &message,
                &WorkspaceStage::HumanConfirm
            ),
            "SC human gate must accept {message:?}"
        );
    }
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
            reviewer_provider: Some(ProviderName::Fake),

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
        candidate_recovery: None,
        findings: Vec::new(),
        repeated_fingerprints: Vec::new(),
        attempts_used: 0,
        manual_repairs_remaining: 2,
        trigger: HumanReason::NativeHumanRequired,
        resumable: false,
        accepted_feedback_turns: None,
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
    let workspace_runs = WorkspaceRunRegistry::default();
    let context = WorkspaceInboundContext {
        app_state: WebAppState::new(
            root.path().to_path_buf(),
            crate::web::runtime::WebRuntime::new_fake(root.path().to_path_buf()),
        ),
        engine: engine.clone(),
        run_context: ProviderRunContext::test_fixture(
            Arc::new(ProviderRegistry::new()),
            engine.clone(),
            workspace_runs.clone(),
            record.id.clone(),
            app_paths,
            record,
        ),
        outbound_tx,
        session_id: "socket_gate_session".to_string(),
    };

    handle_workspace_inbound_message(
        context,
        WsInMessage::HumanGateFeedback {
            command_id: "cmd_socket_gate".to_string(),
            feedback: "只修正这个字段".to_string(),
            excluded_finding_ids: vec![],
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
            reviewer_provider: Some(ProviderName::Fake),

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
        candidate_recovery: None,
        findings: Vec::new(),
        repeated_fingerprints: Vec::new(),
        attempts_used: 0,
        manual_repairs_remaining: 0,
        trigger: HumanReason::NativeHumanRequired,
        resumable: false,
        accepted_feedback_turns: None,
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
    let workspace_runs = WorkspaceRunRegistry::default();
    let context = WorkspaceInboundContext {
        app_state: WebAppState::new(
            root.path().to_path_buf(),
            crate::web::runtime::WebRuntime::new_fake(root.path().to_path_buf()),
        ),
        run_context: ProviderRunContext::test_fixture(
            Arc::new(ProviderRegistry::new()),
            engine.clone(),
            workspace_runs.clone(),
            record.id.clone(),
            app_paths,
            record.clone(),
        ),
        engine: engine.clone(),
        outbound_tx,
        session_id: record.id.clone(),
    };

    handle_workspace_inbound_message(
        context,
        WsInMessage::HumanGateFeedback {
            command_id: "cmd_socket_budget_exhausted".to_string(),
            feedback: "只修正这个字段".to_string(),
            excluded_finding_ids: vec![],
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
            excluded_finding_ids: vec![],
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
            reviewer_provider: Some(ProviderName::Fake),

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
        candidate_recovery: None,
        findings: Vec::new(),
        repeated_fingerprints: Vec::new(),
        attempts_used: 0,
        manual_repairs_remaining: 1,
        trigger: HumanReason::NativeHumanRequired,
        resumable: false,
        accepted_feedback_turns: None,
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
    let workspace_runs = WorkspaceRunRegistry::default();
    let context = WorkspaceInboundContext {
        app_state: WebAppState::new(
            root.path().to_path_buf(),
            crate::web::runtime::WebRuntime::new_fake(root.path().to_path_buf()),
        ),
        run_context: ProviderRunContext::test_fixture(
            Arc::new(ProviderRegistry::new()),
            engine.clone(),
            workspace_runs.clone(),
            record.id.clone(),
            app_paths.clone(),
            record.clone(),
        ),
        engine: engine.clone(),
        outbound_tx,
        session_id: record.id.clone(),
    };

    handle_workspace_inbound_message(
        context,
        WsInMessage::HumanGateFeedback {
            command_id: "cmd_post_approve_feedback".to_string(),
            feedback: "只修正这个字段".to_string(),
            excluded_finding_ids: vec![],
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
async fn sg_03b_confirm_compile_failure_typed_context_as_protocol_error() {
    // F7 项 1 → SG3b：SC Approval 门 confirm 后 compile 失败的 typed
    // PlanningCompileFailure 上下文（failure_id/approval/source refs）经
    // ProtocolError.context 上抛（WS 客户端可见）；本测试同场景种入的旧
    // Failed tx（不同 failure）不得被捞为本次失败事实。
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
            reviewer_provider: Some(ProviderName::Fake),

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
        candidate_recovery: None,
        findings: Vec::new(),
        repeated_fingerprints: Vec::new(),
        attempts_used: 0,
        manual_repairs_remaining: 1,
        trigger: HumanReason::NativeHumanRequired,
        resumable: false,
        accepted_feedback_turns: None,
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
            // F1.2：fixture 沿旧 record 形态（kind=None）。
            kind: None,
        },
        WorkItemSplitFinding {
            severity: WorkItemSplitFindingSeverity::Warning,
            code: "WI_MISSING_VERIFICATION".to_string(),
            message: "work item wi_c has no verification plan".to_string(),
            work_item_ids: vec!["wi_c".to_string()],
            // F1.2：fixture 沿旧 record 形态（kind=None）。
            kind: None,
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
                ever_confirmed: None,
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
    let workspace_runs = WorkspaceRunRegistry::default();
    let context = WorkspaceInboundContext {
        app_state: WebAppState::new(
            root.path().to_path_buf(),
            crate::web::runtime::WebRuntime::new_fake(root.path().to_path_buf()),
        ),
        engine: engine.clone(),
        run_context: ProviderRunContext::test_fixture(
            Arc::new(ProviderRegistry::new()),
            engine.clone(),
            workspace_runs.clone(),
            record.id.clone(),
            app_paths,
            record,
        ),
        outbound_tx,
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
    // SG3b：typed PlanningCompileFailure 上下文直达 WS 客户端——
    // failure_id/kind/stable_code 与 approval/source context 字段在场；本
    // fixture 无候选 refs（refs 缺失失败），字段以 null 序列化、合同不缺。
    assert!(message.contains("SOURCE_STORE_MALFORMED_REF"), "{message}");
    let context = context.expect("typed failure context");
    assert!(
        context["failure_id"]
            .as_str()
            .is_some_and(|id| !id.is_empty()),
        "{context}"
    );
    assert_eq!(context["kind"], serde_json::json!("operational"));
    assert_eq!(
        context["stable_code"],
        serde_json::json!("SOURCE_STORE_MALFORMED_REF")
    );
    for field in [
        "approval_attempt_id",
        "source_revision_ref",
        "plan_candidate_ir_ref",
        "mechanical_report_ref",
    ] {
        assert!(context.get(field).is_some(), "{field} missing: {context}");
    }
    // 不同 failure 的旧 Failed tx（本测试种入的 strict-validator 失败）不得
    // 被 latest-scan 捞为本次失败事实。
    assert!(!message.contains("WI_DEP_CYCLE"), "{message}");
    assert!(!message.contains("WI_MISSING_VERIFICATION"), "{message}");
    assert!(
        context
            .get("validator_findings")
            .map(serde_json::Value::is_null)
            .unwrap_or(true),
        "{context}"
    );
}

#[tokio::test]
async fn late_confirm_after_gate_closed_is_silent_idempotent_noop_at_ws_boundary() {
    // F7 项 2（地雷 2，F3 run1d）：先到者已把 durable 推进到 Confirmed，迟到
    // Confirm 在 WS 边界必须是幂等 no-op——不产生第二条出站关闭/错误消息，
    // 可见提示仅由 engine 的 HUMAN_GATE_ALREADY_CLOSED 事件发出，会话不 abort。
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
            entity_id: "plan_late_confirm".to_string(),
            workspace_type: WorkspaceType::WorkItemPlan,
            author_provider: ProviderName::Fake,
            reviewer_provider: Some(ProviderName::Fake),

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
    record.status = WorkspaceSessionStatus::Confirmed;
    record.flow_kind = WorkItemPlanFlowKind::SingleCandidate;
    record.single_candidate_phase = Some(SingleCandidatePhase::Completed);
    record.human_gate_snapshot = None;
    crate::product::json_store::write_json(
        &app_paths
            .issue_lifecycle_root(&record.project_id, &record.issue_id)
            .join("workspace-sessions")
            .join(format!("{}.json", record.id)),
        &record,
    )
    .expect("persist closed gate session");

    let (event_tx, mut event_rx) = mpsc::channel(8);
    // 迟到者视角：in-memory 会话仍是开门态（HumanConfirm/WaitingForHuman）
    let mut session = WorkspaceSession::from_record(record.clone());
    session.stage = crate::product::workspace_engine::WorkspaceStage::HumanConfirm;
    session.session_status = WorkspaceSessionStatus::WaitingForHuman;
    session.single_candidate_phase = Some(SingleCandidatePhase::Approval);
    session.human_gate_snapshot = Some(HumanGateSnapshot {
        candidate_recovery: None,
        findings: Vec::new(),
        repeated_fingerprints: Vec::new(),
        attempts_used: 0,
        manual_repairs_remaining: 1,
        trigger: HumanReason::NativeHumanRequired,
        resumable: false,
        accepted_feedback_turns: None,
    });
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
    let workspace_runs = WorkspaceRunRegistry::default();
    let context = WorkspaceInboundContext {
        app_state: WebAppState::new(
            root.path().to_path_buf(),
            crate::web::runtime::WebRuntime::new_fake(root.path().to_path_buf()),
        ),
        engine: engine.clone(),
        run_context: ProviderRunContext::test_fixture(
            Arc::new(ProviderRegistry::new()),
            engine.clone(),
            workspace_runs.clone(),
            record.id.clone(),
            app_paths,
            record.clone(),
        ),
        outbound_tx,
        session_id: "late_confirm_session".to_string(),
    };

    handle_workspace_inbound_message(context, WsInMessage::Confirm).await;

    assert!(
        outbound_rx.try_recv().is_err(),
        "idempotent no-op must not emit a second outbound close/error message"
    );
    let mut already_closed_notices = 0;
    while let Ok(event) = event_rx.try_recv() {
        if let crate::product::workspace_engine::EngineEvent::ProtocolError { code, .. } = event
            && code == "HUMAN_GATE_ALREADY_CLOSED"
        {
            already_closed_notices += 1;
        }
    }
    assert_eq!(already_closed_notices, 1, "visible notice event required");
    assert_eq!(
        lifecycle
            .get_workspace_session(&record.id)
            .expect("durable session"),
        record,
        "idempotent no-op must keep durable state untouched"
    );
}

#[tokio::test]
async fn story_author_gate_abandon_terminates_session_through_socket_dispatch() {
    // F-18（w2c 实测矩阵）：story 会话（legacy 流）停 author_confirm 门，
    // typed abandon 经真实 inbound 分发链关门—— durable 置 Terminated、
    // 引擎发 terminate close 事件，且不回 Error/ProtocolError 出站帧。
    use crate::product::lifecycle_store::{CreateWorkspaceSessionInput, LifecycleStore};
    use crate::product::models::{ProviderName, WorkspaceSessionStatus, WorkspaceType};
    use tempfile::tempdir;

    let root = tempdir().expect("tempdir");
    let app_paths = crate::product::app_paths::ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let mut record = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: "story_spec_0001".to_string(),
            workspace_type: WorkspaceType::Story,
            author_provider: ProviderName::Fake,
            reviewer_provider: Some(ProviderName::Fake),

            review_rounds: 0,
            superpowers_enabled: false,
            openspec_enabled: false,
            work_item_plan_options: None,
        })
        .expect("create story session");
    record.status = WorkspaceSessionStatus::WaitingForHuman;
    crate::product::json_store::write_json(
        &app_paths
            .issue_lifecycle_root(&record.project_id, &record.issue_id)
            .join("workspace-sessions")
            .join(format!("{}.json", record.id)),
        &record,
    )
    .expect("persist story gate session");

    let (event_tx, mut event_rx) = mpsc::channel(16);
    let mut session = WorkspaceSession::from_record(record.clone());
    session.artifact = Some(crate::web::workspace_ws_types::ArtifactPayload::Markdown {
        markdown: "# Story Spec\n".to_string(),
        diff: None,
    });
    let engine = Arc::new(Mutex::new(WorkspaceEngine::new_persistent(
        Arc::new(CheckpointStore::new(root.path().join("checkpoints"))),
        lifecycle.clone(),
        event_tx,
        session,
    )));
    engine
        .lock()
        .await
        .enter_author_confirm(Some("Author 结果等待确认".to_string()))
        .await;
    let (outbound_tx, mut outbound_rx) = mpsc::channel(16);
    let workspace_runs = WorkspaceRunRegistry::default();
    let context = WorkspaceInboundContext {
        app_state: WebAppState::new(
            root.path().to_path_buf(),
            crate::web::runtime::WebRuntime::new_fake(root.path().to_path_buf()),
        ),
        engine: engine.clone(),
        run_context: ProviderRunContext::test_fixture(
            Arc::new(ProviderRegistry::new()),
            engine.clone(),
            workspace_runs.clone(),
            record.id.clone(),
            app_paths,
            record.clone(),
        ),
        outbound_tx,
        session_id: record.id.clone(),
    };

    handle_workspace_inbound_message(
        context,
        WsInMessage::AbandonHumanGate {
            command_id: "cmd-story-abandon".to_string(),
        },
    )
    .await;

    assert!(
        outbound_rx.try_recv().is_err(),
        "关门成功不得回 Error/ProtocolError 出站帧（close 通知走引擎事件）"
    );
    let mut close_events = 0;
    while let Ok(event) = event_rx.try_recv() {
        if let crate::product::workspace_engine::EngineEvent::HumanGateClosed { decision, stage } =
            event
            && decision == "terminate"
            && stage == "completed"
        {
            close_events += 1;
        }
    }
    assert_eq!(close_events, 1, "恰好一条 terminate close 事件");
    let durable = lifecycle
        .get_workspace_session(&record.id)
        .expect("durable story session");
    assert_eq!(durable.status, WorkspaceSessionStatus::Terminated);
}

// —— T5.1（REQ-REG-15/D4）：确认 trust typed 停等的 WS 边界投影 ——
// 共用 effect（apply_human_gate_termination / handle_confirm_from_handler）
// 必须把 TrustWaiting 投影为稳定 code=workspace_confirmation_trust_waiting
// 的 ProtocolError（不 abort、不 compile failure、不 generic EngineError），
// message/context 同源生成；门保持开态、零 durable 副作用（引擎面 4.1 已
// 锁定，此处锁 WS 序列化边界）。

static T5_TRUST_WAITING_SEQUENCE: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(1);

/// 剧本 precondition：默认 Ready，`script_waiting` 排队一条 HomeWaiting；
/// 记录 operation_id 调用（恰一次/零调用断言，与引擎 4.1 测试同款纪律）。
#[derive(Default)]
struct ScriptedTrustPrecondition {
    calls: std::sync::Mutex<Vec<String>>,
    scripted: std::sync::Mutex<
        std::collections::VecDeque<
            crate::product::logical_codebase::ProviderTrustPreparationResult,
        >,
    >,
}

impl ScriptedTrustPrecondition {
    fn script_waiting(&self, reason_code: &str) {
        self.scripted.lock().unwrap().push_back(
            crate::product::logical_codebase::ProviderTrustPreparationResult::Waiting {
                waiting: crate::product::logical_codebase::ProviderTrustWaiting {
                    provider: ProviderName::Codex,
                    canonical_root: std::path::PathBuf::from("/fixture-trust-root"),
                    trust_key: "fixture-trust-key".to_string(),
                    reason_code: reason_code.to_string(),
                    message: "fixture 等待原因（home 读取失败）".to_string(),
                    retry_action: "修复后重新确认".to_string(),
                    manual_guidance: "fixture 人工指导：修复 Codex home 配置".to_string(),
                    command_markdown: None,
                    recorded_at: "2026-10-11T00:00:00Z".to_string(),
                },
            },
        );
    }

    fn call_count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }

    fn operation_ids(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

impl crate::product::logical_codebase::ProviderTrustPrecondition for ScriptedTrustPrecondition {
    fn ensure_ready(
        &self,
        _project_id: &str,
        operation_id: &str,
        _lc_id: &str,
        _canonical_root: &std::path::Path,
        _providers: &[ProviderName],
        _scenario: crate::product::logical_codebase::provider_trust::ProviderTrustScenario,
    ) -> crate::product::logical_codebase::ProviderTrustPreparationResult {
        self.calls.lock().unwrap().push(operation_id.to_string());
        self.scripted.lock().unwrap().pop_front().unwrap_or(
            crate::product::logical_codebase::ProviderTrustPreparationResult::Ready {
                registrations: Vec::new(),
            },
        )
    }
}

/// presence-only gateway：确认链只把 gateway 用作「LC 会话」谓词（不调用
/// validate/spawn），能力/目标解析全部 fail-closed 不可达（与引擎 4.1 测试
/// 同款最薄实现）。
struct T5PresenceCapabilitySource;

impl crate::product::logical_codebase::ProviderCapabilitySource for T5PresenceCapabilitySource {
    fn require_supported(
        &self,
        _provider: &crate::product::logical_codebase::ProviderRef,
        _action: crate::product::logical_codebase::SessionPolicyAction,
    ) -> Result<
        crate::product::logical_codebase::ProviderCapability,
        crate::product::logical_codebase::ProviderGatewayError,
    > {
        Err(
            crate::product::logical_codebase::ProviderGatewayError::UnsupportedCapability(
                "presence-only fixture gateway".to_string(),
            ),
        )
    }

    fn require_resume_supported(
        &self,
        _provider: &crate::product::logical_codebase::ProviderRef,
        _action: crate::product::logical_codebase::SessionPolicyAction,
    ) -> Result<
        crate::product::logical_codebase::ProviderCapability,
        crate::product::logical_codebase::ProviderGatewayError,
    > {
        Err(
            crate::product::logical_codebase::ProviderGatewayError::UnsupportedCapability(
                "presence-only fixture gateway".to_string(),
            ),
        )
    }

    fn require_write_boundary(
        &self,
        _provider: &crate::product::logical_codebase::ProviderRef,
        _action: crate::product::logical_codebase::SessionPolicyAction,
    ) -> Result<
        crate::product::logical_codebase::ProviderCapability,
        crate::product::logical_codebase::ProviderGatewayError,
    > {
        Err(
            crate::product::logical_codebase::ProviderGatewayError::UnsupportedCapability(
                "presence-only fixture gateway".to_string(),
            ),
        )
    }

    fn require_root_recipe_supported(
        &self,
        _provider: &crate::product::logical_codebase::ProviderRef,
        _credential: &crate::product::logical_codebase::provider_admission_preflight::BootstrapPhaseCredential,
    ) -> Result<
        crate::product::logical_codebase::ProviderCapability,
        crate::product::logical_codebase::ProviderGatewayError,
    > {
        Err(
            crate::product::logical_codebase::ProviderGatewayError::UnsupportedCapability(
                "presence-only fixture gateway".to_string(),
            ),
        )
    }
}

struct T5PresenceTargetResolver;

impl crate::product::logical_codebase::PolicyTargetResolver for T5PresenceTargetResolver {
    fn resolve_and_revalidate(
        &self,
        _request: &crate::product::logical_codebase::SessionLaunchRequest,
    ) -> Result<
        crate::product::logical_codebase::PolicyTarget,
        crate::product::logical_codebase::ProviderGatewayError,
    > {
        Err(
            crate::product::logical_codebase::ProviderGatewayError::UnsupportedCapability(
                "presence-only fixture gateway".to_string(),
            ),
        )
    }
}

struct T5PresenceSyncAdapter;

impl crate::cross_cutting::provider_adapter::ProviderAdapter for T5PresenceSyncAdapter {
    fn run(
        &self,
        _input: &crate::protocol::contracts::AdapterInput,
    ) -> Result<
        crate::protocol::contracts::AdapterOutput,
        crate::cross_cutting::provider_adapter::ProviderAdapterError,
    > {
        Ok(crate::protocol::contracts::AdapterOutput {
            exit_code: Some(0),
            stdout: "ok".to_string(),
            stderr: String::new(),
            structured_output: None,
            files_modified: Vec::new(),
            duration_ms: 0,
            timeout_status: crate::protocol::contracts::TimeoutStatus::NotTimedOut,
        })
    }
}

fn t5_presence_gateway(
    authority_root: std::path::PathBuf,
) -> Arc<crate::product::logical_codebase::LogicalCodebaseProviderGateway> {
    use crate::cross_cutting::provider_availability_gate::ProviderAvailabilityGate;
    use crate::cross_cutting::provider_availability_gate::ProviderHealthSource;
    use crate::cross_cutting::provider_health::{ProviderHealthEntry, ProviderHealthSnapshot};

    struct AlwaysHealthy(Arc<ProviderHealthSnapshot>);

    impl ProviderHealthSource for AlwaysHealthy {
        fn snapshot(&self) -> Arc<ProviderHealthSnapshot> {
            self.0.clone()
        }

        fn degraded(&self) -> bool {
            false
        }
    }

    let checked_at = chrono::Utc::now();
    let snapshot = Arc::new(ProviderHealthSnapshot {
        schema_version: 1,
        generation: 1,
        checked_at,
        providers: [ProviderName::ClaudeCode, ProviderName::Codex]
            .into_iter()
            .map(|provider| ProviderHealthEntry {
                provider,
                command: "stub".to_string(),
                available: true,
                version: Some("1.0".to_string()),
                reason_code: None,
                reason: None,
                checked_at,
            })
            .collect(),
    });
    let paths = ProductAppPaths::new(std::env::temp_dir().join("aria-t5-presence-gateway"));
    Arc::new(
        crate::product::logical_codebase::LogicalCodebaseProviderGateway::new(
            crate::product::logical_codebase::AggregatePolicyArtifactStore::new(paths),
            Arc::new(T5PresenceCapabilitySource),
            Arc::new(T5PresenceTargetResolver),
            Arc::new(crate::cross_cutting::provider_registry::ProviderRegistry::new()),
            Arc::new(T5PresenceSyncAdapter),
            Arc::new(ProviderAvailabilityGate::new(Arc::new(AlwaysHealthy(
                snapshot,
            )))),
            authority_root,
        ),
    )
}

struct T5TrustWaitingFacts {
    session_id: String,
    lc_id: String,
    /// 冻结根本身由 manifest 承载;此处保留供断言对照。
    #[allow(dead_code)]
    canonical_root: std::path::PathBuf,
}

/// T5.1 LC 会话 SC/非 SC 共用夹具：真实 issue（原始 LC 归属）+ LC scope
/// manifest（root=冻结根）+ 开态门 durable 记录（SC 带相位/快照静态资格）
/// + presence gateway；`trust=None` 时不注入确认上下文（缺上下文
///   ContextWaiting，与生产 manager 缺 trust 依赖形态同构）。
#[allow(clippy::type_complexity)]
fn t5_trust_waiting_ws_fixture(
    single_candidate: bool,
    trust: Option<Arc<ScriptedTrustPrecondition>>,
) -> (
    tempfile::TempDir,
    LifecycleStore,
    Arc<Mutex<WorkspaceEngine>>,
    mpsc::Receiver<crate::product::workspace_engine::EngineEvent>,
    T5TrustWaitingFacts,
) {
    use crate::product::issue_store::{CreateProductIssueInput, IssueStore};
    use crate::product::lifecycle_store::{
        CreateWorkspaceSessionInput, WorkItemPlanSessionOptions,
    };
    use crate::product::models::{SingleCandidatePhase, WorkspaceSessionStatus, WorkspaceType};
    use crate::product::work_item_plan_policy::{HumanGateSnapshot, HumanReason, RunPolicy};

    let seq = T5_TRUST_WAITING_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let root = tempfile::tempdir().expect("tempdir");
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let worktree = root.path().join("t5-aggregate-root");
    std::fs::create_dir_all(&worktree).expect("create aggregate root");
    let canonical_root = std::fs::canonicalize(&worktree).expect("canonical aggregate root");
    let lc_id = format!("lc_t5_ws_{seq}");
    let issue = IssueStore::new(app_paths.clone())
        .create(CreateProductIssueInput {
            project_id: "project_0001".to_string(),
            repo_id: None,
            logical_codebase_id: lc_id.clone(),
            base_branch: None,
            title: "t5 ws trust waiting fixture".to_string(),
            description: None,
            change_id: None,
        })
        .expect("create LC issue");
    crate::product::logical_codebase::LogicalCodebaseStore::for_lc(
        app_paths.clone(),
        lc_id.clone(),
    )
    .save_manifest(
        "project_0001",
        &crate::product::logical_codebase::LogicalCodebaseManifest::new(
            "project_0001",
            canonical_root.clone(),
            Vec::new(),
        ),
    )
    .expect("save LC manifest");
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let created = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: issue.id.clone(),
            entity_id: format!("plan_t5_ws_{seq}"),
            workspace_type: WorkspaceType::WorkItemPlan,
            author_provider: ProviderName::Codex,
            reviewer_provider: None,
            review_rounds: 0,
            superpowers_enabled: false,
            openspec_enabled: false,
            work_item_plan_options: single_candidate.then_some(WorkItemPlanSessionOptions {
                flow_kind: WorkItemPlanFlowKind::SingleCandidate,
                run_policy: RunPolicy::Interactive,
                rollout_snapshot: true,
            }),
        })
        .expect("create workspace session");
    // 进程内唯一 session id（close CAS 计数不互窜）+ 开态门 durable 事实。
    let session_id = format!("workspace_session_t5_ws_{seq}");
    let mut record = lifecycle
        .get_workspace_session(&created.id)
        .expect("load created session");
    let old_id = record.id.clone();
    record.id = session_id.clone();
    record.status = WorkspaceSessionStatus::WaitingForHuman;
    if single_candidate {
        record.single_candidate_phase = Some(SingleCandidatePhase::Approval);
        record.human_gate_snapshot = Some(HumanGateSnapshot {
            candidate_recovery: None,
            findings: Vec::new(),
            repeated_fingerprints: Vec::new(),
            attempts_used: 0,
            manual_repairs_remaining: 1,
            trigger: HumanReason::NativeHumanRequired,
            resumable: false,
            accepted_feedback_turns: None,
        });
    }
    let session_dir = app_paths
        .issue_root(&record.project_id, &record.issue_id)
        .join("workspace-sessions");
    crate::product::json_store::write_json(
        &session_dir.join(format!("{}.json", record.id)),
        &record,
    )
    .expect("persist t5 fixture session");
    std::fs::remove_file(session_dir.join(format!("{old_id}.json"))).ok();

    let (event_tx, event_rx) = mpsc::channel(8);
    let mut session = WorkspaceSession::from_record(record);
    session.artifact = Some(crate::web::workspace_ws_types::ArtifactPayload::Markdown {
        markdown: "# Work Item Plan\n".to_string(),
        diff: None,
    });
    let mut engine = WorkspaceEngine::new_persistent(
        Arc::new(CheckpointStore::new(root.path().join("checkpoints"))),
        lifecycle.clone(),
        event_tx,
        session,
    )
    .with_logical_provider_gateway(t5_presence_gateway(canonical_root.clone()));
    if let Some(trust) = trust {
        engine = engine.with_confirmation_trust_context(
            crate::product::workspace_engine::conversational_gate::ConfirmationTrustContext {
                precondition: trust,
                issue_lc_id: Some(lc_id.clone()),
                frozen_root: canonical_root.clone(),
            },
        );
    }
    let engine = Arc::new(Mutex::new(engine));
    let facts = T5TrustWaitingFacts {
        session_id,
        lc_id,
        canonical_root,
    };
    (root, lifecycle, engine, event_rx, facts)
}

fn t5_inbound_context(
    root: &tempfile::TempDir,
    lifecycle: &LifecycleStore,
    engine: Arc<Mutex<WorkspaceEngine>>,
    facts: &T5TrustWaitingFacts,
) -> (WorkspaceInboundContext, mpsc::Receiver<OutboundControl>) {
    let record = lifecycle
        .get_workspace_session(&facts.session_id)
        .expect("fixture durable record");
    let (outbound_tx, outbound_rx) = mpsc::channel(8);
    let context = WorkspaceInboundContext {
        app_state: WebAppState::new(
            root.path().to_path_buf(),
            crate::web::runtime::WebRuntime::new_fake(root.path().to_path_buf()),
        ),
        engine: engine.clone(),
        run_context: ProviderRunContext::test_fixture(
            Arc::new(ProviderRegistry::new()),
            engine,
            WorkspaceRunRegistry::default(),
            facts.session_id.clone(),
            ProductAppPaths::new(root.path().join(".aria")),
            record,
        ),
        outbound_tx,
        session_id: facts.session_id.clone(),
    };
    (context, outbound_rx)
}

/// SC confirm 的 HomeWaiting：稳定 code 的 ProtocolError（非 generic
/// EngineError/Error 帧），message 同源含原因/人工处置/retry_action/门未
/// 关闭/重新确认短语，context 携带全部已知身份与 retryable:true；恰一帧
/// （无 close 广播/session_state）、durable 全不变、close CAS 零推进、
/// ensure 恰一次且 operation_id 关联本会话。
#[tokio::test]
async fn ws_sc_confirm_trust_waiting_is_stable_protocol_error_with_guidance() {
    let trust = Arc::new(ScriptedTrustPrecondition::default());
    trust.script_waiting("fixture_home_failure");
    let (root, lifecycle, engine, mut event_rx, facts) =
        t5_trust_waiting_ws_fixture(true, Some(trust.clone()));
    let before = serde_json::to_vec(
        &lifecycle
            .get_workspace_session(&facts.session_id)
            .expect("durable before"),
    )
    .expect("serialize before");
    let cas_before = crate::product::workspace_engine::conversational_gate::close_cas_attempts(
        &facts.session_id,
    );
    let (context, mut outbound_rx) = t5_inbound_context(&root, &lifecycle, engine, &facts);

    handle_workspace_inbound_message(context, WsInMessage::Confirm).await;

    let outbound = tokio::time::timeout(std::time::Duration::from_secs(2), outbound_rx.recv())
        .await
        .expect("trust waiting must reach the client as a protocol frame")
        .expect("outbound frame");
    let OutboundControl::Text(json) = outbound else {
        panic!("expected protocol error text frame");
    };
    let message: WsOutMessage = serde_json::from_str(&json).expect("protocol error json");
    let WsOutMessage::ProtocolError {
        code,
        message,
        context,
    } = message
    else {
        panic!("SC trust waiting must be a ProtocolError, got {message:?}")
    };
    assert_eq!(code, "workspace_confirmation_trust_waiting");
    assert!(
        message.contains("fixture 等待原因（home 读取失败）"),
        "{message}"
    );
    assert!(
        message.contains("fixture 人工指导：修复 Codex home 配置"),
        "{message}"
    );
    assert!(message.contains("修复后重新确认"), "{message}");
    assert!(message.contains("门保持未关闭"), "{message}");
    assert!(
        message.contains("完成后请重新选择该门并确认；失败不自动重试"),
        "{message}"
    );
    let context = context.expect("waiting context");
    assert_eq!(context["session_id"], serde_json::json!(facts.session_id));
    assert_eq!(context["project_id"], serde_json::json!("project_0001"));
    assert_eq!(context["lc_id"], serde_json::json!(facts.lc_id));
    assert_eq!(context["provider"], serde_json::json!("codex"));
    assert_eq!(
        context["canonical_root"],
        serde_json::json!("/fixture-trust-root")
    );
    assert_eq!(context["trust_key"], serde_json::json!("fixture-trust-key"));
    assert_eq!(
        context["reason_code"],
        serde_json::json!("fixture_home_failure")
    );
    assert_eq!(context["retry_action"], serde_json::json!("修复后重新确认"));
    assert_eq!(
        context["manual_guidance"],
        serde_json::json!("fixture 人工指导：修复 Codex home 配置")
    );
    assert_eq!(context["retryable"], serde_json::json!(true));

    // 恰一帧：不广播 close/session_state（门未关、零推进副作用）；分发
    // 结束后 sender 随 context drop，通道关闭（None）同样算无后续帧。
    assert!(
        matches!(
            tokio::time::timeout(std::time::Duration::from_millis(200), outbound_rx.recv()).await,
            Err(_) | Ok(None)
        ),
        "waiting must not emit any further outbound frame"
    );
    assert!(
        event_rx.try_recv().is_err(),
        "waiting 不产生任何引擎事件（含 HumanGateClosed/abort）"
    );
    assert_eq!(
        serde_json::to_vec(
            &lifecycle
                .get_workspace_session(&facts.session_id)
                .expect("durable after")
        )
        .expect("serialize after"),
        before,
        "waiting 前后 durable 会话记录完全不变"
    );
    assert_eq!(
        crate::product::workspace_engine::conversational_gate::close_cas_attempts(
            &facts.session_id
        ),
        cas_before,
        "准备失败不推进原 close CAS"
    );
    assert_eq!(trust.call_count(), 1, "missing 才进入 ensure（恰一次）");
    assert_eq!(
        trust.operation_ids(),
        vec![format!("workspace_confirm:{}", facts.session_id)],
        "operation_id 仅关联本会话确认"
    );
}

/// 非 SC confirm 的 TrustWaiting：`handle_confirm_from_handler` 此前对
/// Ok(TrustWaiting) 过渡期静默（客户端零回执）——必须与 SC 同源投影为
/// 稳定 code 的 ProtocolError，durable 全不变。
#[tokio::test]
async fn ws_non_sc_confirm_trust_waiting_returns_visible_protocol_error() {
    let trust = Arc::new(ScriptedTrustPrecondition::default());
    trust.script_waiting("fixture_home_failure");
    let (root, lifecycle, engine, mut event_rx, facts) =
        t5_trust_waiting_ws_fixture(false, Some(trust.clone()));
    let before = serde_json::to_vec(
        &lifecycle
            .get_workspace_session(&facts.session_id)
            .expect("durable before"),
    )
    .expect("serialize before");
    let (context, mut outbound_rx) = t5_inbound_context(&root, &lifecycle, engine, &facts);

    handle_workspace_inbound_message(context, WsInMessage::Confirm).await;

    let outbound = tokio::time::timeout(std::time::Duration::from_secs(2), outbound_rx.recv())
        .await
        .expect("non-SC trust waiting must not be swallowed（过渡期静默缺口）")
        .expect("outbound frame");
    let OutboundControl::Text(json) = outbound else {
        panic!("expected protocol error text frame");
    };
    let message: WsOutMessage = serde_json::from_str(&json).expect("protocol error json");
    let WsOutMessage::ProtocolError {
        code,
        message,
        context,
    } = message
    else {
        panic!("non-SC trust waiting must be a ProtocolError, got {message:?}")
    };
    assert_eq!(code, "workspace_confirmation_trust_waiting");
    assert!(
        message.contains("fixture 等待原因（home 读取失败）"),
        "{message}"
    );
    assert!(
        message.contains("完成后请重新选择该门并确认；失败不自动重试"),
        "{message}"
    );
    let context = context.expect("waiting context");
    assert_eq!(context["session_id"], serde_json::json!(facts.session_id));
    assert_eq!(
        context["reason_code"],
        serde_json::json!("fixture_home_failure")
    );
    assert!(
        matches!(
            tokio::time::timeout(std::time::Duration::from_millis(200), outbound_rx.recv()).await,
            Err(_) | Ok(None)
        ),
        "waiting 不产生后续出站帧（分发后 sender 随 context drop，None 亦无后续帧）"
    );
    assert!(event_rx.try_recv().is_err(), "waiting 零引擎事件");
    assert_eq!(
        serde_json::to_vec(
            &lifecycle
                .get_workspace_session(&facts.session_id)
                .expect("durable after")
        )
        .expect("serialize after"),
        before,
        "waiting 前后 durable 会话记录完全不变"
    );
    assert_eq!(trust.call_count(), 1, "非 SC mutex 内锁定事实恰一次 ensure");
}

/// SC confirm 缺确认上下文的 ContextWaiting：只返回已知身份——context 带
/// session/project/issue/reason_code，issue_lc_id/frozen_root 缺席为 null，
/// 不编造 provider/trust_key/manual_guidance；message 含原因码与重新确认
/// 短语。
#[tokio::test]
async fn ws_sc_confirm_trust_context_waiting_returns_known_identity_only() {
    let (root, lifecycle, engine, _event_rx, facts) = t5_trust_waiting_ws_fixture(true, None);
    let before = serde_json::to_vec(
        &lifecycle
            .get_workspace_session(&facts.session_id)
            .expect("durable before"),
    )
    .expect("serialize before");
    let cas_before = crate::product::workspace_engine::conversational_gate::close_cas_attempts(
        &facts.session_id,
    );
    let (context, mut outbound_rx) = t5_inbound_context(&root, &lifecycle, engine, &facts);

    handle_workspace_inbound_message(context, WsInMessage::Confirm).await;

    let outbound = tokio::time::timeout(std::time::Duration::from_secs(2), outbound_rx.recv())
        .await
        .expect("context waiting must reach the client")
        .expect("outbound frame");
    let OutboundControl::Text(json) = outbound else {
        panic!("expected protocol error text frame");
    };
    let message: WsOutMessage = serde_json::from_str(&json).expect("protocol error json");
    let WsOutMessage::ProtocolError {
        code,
        message,
        context,
    } = message
    else {
        panic!("context waiting must be a ProtocolError, got {message:?}")
    };
    assert_eq!(code, "workspace_confirmation_trust_waiting");
    assert!(
        message.contains("confirmation_trust_context_unavailable"),
        "{message}"
    );
    assert!(
        message.contains("完成后请重新选择该门并确认；失败不自动重试"),
        "{message}"
    );
    let context = context.expect("waiting context");
    assert_eq!(context["session_id"], serde_json::json!(facts.session_id));
    assert_eq!(context["project_id"], serde_json::json!("project_0001"));
    assert!(
        context["issue_id"].is_string(),
        "context waiting 携带 issue 身份"
    );
    assert_eq!(
        context["reason_code"],
        serde_json::json!("confirmation_trust_context_unavailable")
    );
    assert_eq!(context["issue_lc_id"], serde_json::json!(null));
    assert_eq!(context["frozen_root"], serde_json::json!(null));
    assert!(
        context.get("provider").is_none()
            && context.get("trust_key").is_none()
            && context.get("manual_guidance").is_none(),
        "ContextWaiting 只返回已知身份，不编造 provider/key/指导"
    );
    assert!(
        matches!(
            tokio::time::timeout(std::time::Duration::from_millis(200), outbound_rx.recv()).await,
            Err(_) | Ok(None)
        ),
        "waiting 不产生后续出站帧（分发后 sender 随 context drop，None 亦无后续帧）"
    );
    assert_eq!(
        serde_json::to_vec(
            &lifecycle
                .get_workspace_session(&facts.session_id)
                .expect("durable after")
        )
        .expect("serialize after"),
        before,
        "context waiting 前后 durable 会话记录完全不变"
    );
    assert_eq!(
        crate::product::workspace_engine::conversational_gate::close_cas_attempts(
            &facts.session_id
        ),
        cas_before,
        "上下文停等零 home 调用、不推进 close CAS"
    );
}

// —— Task 7.2（REQ-REG-15）：SC 真实 manager 全链（create→仲裁→SC Confirm
// 分流→真实 registry→关门 CAS 前停等）——
// 与 T5.1 的剧本 precondition 用例不同：本用例经真实 WebAppState/manager
// create 装配（fake runtime registry home=workspace_root；factory 从 HOME
// 装配的 readonly source 指到同一隔离 home），SC Confirm 帧按 flow_kind
// 分流进 handle_human_gate_termination，Approve 在静态资格预检后接真实
// registry——冲突态 fail-closed：typed TrustWaiting 协议帧、close CAS 零
// 推进、无 HumanGateClosed/close 广播、用户冲突条目字节保持。

/// 与 part_01/gateway_factory 的 T7 HOME 守卫共用同一 sidecar 锁目标
///（同一测试二进制内互斥）。
fn t7_ws_home_env_lock_target() -> std::path::PathBuf {
    std::env::temp_dir().join("aria-t7-home-env")
}

struct T7WsHomeEnvGuard {
    _lock: crate::product::coding_attempt_store::locking::ExclusiveFileLock,
    previous: Option<std::ffi::OsString>,
}

impl T7WsHomeEnvGuard {
    fn isolate(home: &std::path::Path) -> Self {
        let _lock = crate::product::coding_attempt_store::locking::ExclusiveFileLock::acquire(
            &t7_ws_home_env_lock_target(),
        )
        .expect("serialize HOME mutation across the test binary");
        let previous = std::env::var_os("HOME");
        // SAFETY: 与其它改写 HOME 的 T7 用例经同一文件锁串行；本二进制内
        // 并行用例对 HOME 的消费只有 factory 装配的 adapter home 路径拼接
        //（无文件 IO），读写交错不产生可观察行为差异。
        unsafe { std::env::set_var("HOME", home) };
        Self { _lock, previous }
    }
}

impl Drop for T7WsHomeEnvGuard {
    fn drop(&mut self) {
        // SAFETY: 同上，文件锁串行下的恢复写。
        unsafe {
            if let Some(previous) = self.previous.clone() {
                std::env::set_var("HOME", previous);
            } else {
                std::env::remove_var("HOME");
            }
        }
    }
}

/// 真实 SC 会话夹具：复用 enrollment seed 拓扑（project+2 成员 legacy LC
/// +已确认 story/design——manager create 的 workspace context/repository
/// routing/aggregate index 全链可过，part_01 同款实证拓扑），其上建
/// WorkItemPlan/SC/Approval 开态门 durable 记录（plan 实体真实落库）；
/// 返回真实 manager 装配所需原料（不建 engine——create 全权负责）。
fn t7_sc_confirm_chain_fixture() -> (
    crate::web::handlers::automation_enrollment_test_support::Fixture,
    crate::product::models::WorkspaceSessionRecord,
) {
    use crate::product::lifecycle_store::{
        CreateIssueWorkItemPlanInput, CreateWorkspaceSessionInput, WorkItemPlanSessionOptions,
    };
    use crate::product::models::{SingleCandidatePhase, WorkspaceType};
    use crate::product::work_item_plan_policy::{HumanGateSnapshot, HumanReason, RunPolicy};

    let fixture = crate::web::handlers::automation_enrollment_test_support::seed_fixture(2, true);
    let lifecycle = fixture.lifecycle.clone();
    lifecycle
        .create_issue_work_item_plan(CreateIssueWorkItemPlanInput {
            id: Some("plan_t7_sc".to_string()),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            source_story_spec_ids: vec![fixture.story_id.clone()],
            source_design_spec_ids: vec![fixture.design_id.clone()],
            options: crate::product::models::IssueWorkItemPlanOptions {
                include_integration_tests: false,
                include_e2e_tests: false,
                force_frontend_backend_split: false,
                require_execution_plan_confirm: false,
            },
            status: crate::product::models::IssueWorkItemPlanStatus::Draft,
            work_item_ids: vec!["work_item_t7_sc_0001".to_string()],
            repository_profile_ref: None,
            verification_plan_ids: Vec::new(),
            dependency_graph: Vec::new(),
            created_from_provider_run: None,
            validator_findings: Vec::new(),
        })
        .expect("create t7 SC plan entity");
    let created = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: "plan_t7_sc".to_string(),
            workspace_type: WorkspaceType::WorkItemPlan,
            author_provider: ProviderName::Codex,
            reviewer_provider: None,
            review_rounds: 0,
            superpowers_enabled: false,
            openspec_enabled: false,
            work_item_plan_options: Some(WorkItemPlanSessionOptions {
                flow_kind: WorkItemPlanFlowKind::SingleCandidate,
                run_policy: RunPolicy::Interactive,
                rollout_snapshot: true,
            }),
        })
        .expect("create SC session");
    let mut record = lifecycle
        .get_workspace_session(&created.id)
        .expect("load created session");
    record.status = WorkspaceSessionStatus::WaitingForHuman;
    record.single_candidate_phase = Some(SingleCandidatePhase::Approval);
    record.human_gate_snapshot = Some(HumanGateSnapshot {
        candidate_recovery: None,
        findings: Vec::new(),
        repeated_fingerprints: Vec::new(),
        attempts_used: 0,
        manual_repairs_remaining: 1,
        trigger: HumanReason::NativeHumanRequired,
        resumable: false,
        accepted_feedback_turns: None,
    });
    crate::product::json_store::write_json(
        &fixture
            .paths
            .issue_lifecycle_root(&record.project_id, &record.issue_id)
            .join("workspace-sessions")
            .join(format!("{}.json", record.id)),
        &record,
    )
    .expect("persist SC approval session");
    (fixture, record)
}

include!("conversational_gate_protocol/sc_confirm_chain.rs");

async fn t7_sc_drain_outbound(
    rx: &mut mpsc::Receiver<OutboundControl>,
    quiet: std::time::Duration,
) -> Vec<serde_json::Value> {
    let mut frames = Vec::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let wait = quiet.min(deadline.saturating_duration_since(std::time::Instant::now()));
        match tokio::time::timeout(wait, rx.recv()).await {
            Ok(Some(OutboundControl::Text(json))) => {
                if let Ok(value) = serde_json::from_str(&json) {
                    frames.push(value);
                }
            }
            Ok(Some(_)) => continue,
            Ok(None) | Err(_) => break,
        }
        if std::time::Instant::now() >= deadline {
            break;
        }
    }
    frames
}
