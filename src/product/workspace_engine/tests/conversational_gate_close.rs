use super::conversational_gate::gate_fixture;
use super::*;
use crate::product::models::{
    IssueWorkItemPlanOptions, IssueWorkItemPlanStatus, SingleCandidatePhase, WorkspaceSessionStatus,
};
use crate::product::work_item_plan_policy::RunPolicy;
use crate::product::workspace_engine::compile::SingleCandidateCompileCheckpoint;

fn approval_fixture() -> (tempfile::TempDir, LifecycleStore, WorkspaceEngine) {
    let (tmp, lifecycle, _plan_id, mut engine) =
        super::make_work_item_plan_engine_with_accepted_contract_drafts();
    super::single_candidate_recovery::single_candidate_recovery_record(
        &lifecycle,
        &mut engine,
        SingleCandidatePhase::Approval,
        RunPolicy::Interactive,
    );
    let mut record = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("session");
    record.flow_kind = WorkItemPlanFlowKind::SingleCandidate;
    record.run_policy = RunPolicy::Interactive;
    record.single_candidate_phase = Some(SingleCandidatePhase::Approval);
    record.status = WorkspaceSessionStatus::WaitingForHuman;
    record.human_gate_snapshot = Some(crate::product::work_item_plan_policy::HumanGateSnapshot {
        findings: Vec::new(),
        repeated_fingerprints: Vec::new(),
        attempts_used: 0,
        manual_repairs_remaining: 1,
        trigger: crate::product::work_item_plan_policy::HumanReason::NativeHumanRequired,
        resumable: false,
    });
    crate::product::json_store::write_json(
        &lifecycle
            .app_paths()
            .issue_root(&record.project_id, &record.issue_id)
            .join("workspace-sessions")
            .join(format!("{}.json", record.id)),
        &record,
    )
    .expect("persist approval session");
    let artifact = engine.session.artifact.clone();
    engine.session = WorkspaceSession::from_record(record);
    engine.session.stage = WorkspaceStage::HumanConfirm;
    engine.session.session_status = WorkspaceSessionStatus::WaitingForHuman;
    engine.session.artifact = artifact;
    (tmp, lifecycle, engine)
}

#[tokio::test]
async fn conversational_gate_approve_does_not_confirm_before_compile_success() {
    let (_root, lifecycle, mut engine) = super::conversational_gate::gate_fixture(1);
    let before = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("durable session before confirm");
    let error = engine
        .handle_human_gate_termination(
            crate::web::workspace_ws_types::HumanConfirmDecision::Confirm,
        )
        .await
        .expect_err("incomplete fixture must fail closed before confirming");
    assert!(error.contains("compile failed"));
    let after = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("durable session after failed confirm");
    assert_ne!(before.status, WorkspaceSessionStatus::Confirmed);
    assert_ne!(after.status, WorkspaceSessionStatus::Confirmed);
    assert_ne!(
        after.single_candidate_phase,
        Some(SingleCandidatePhase::Completed)
    );
    assert!(
        after.human_gate_snapshot.is_none() || after.status != WorkspaceSessionStatus::Confirmed
    );
}

#[tokio::test]
async fn conversational_gate_approve_fails_closed_at_compile_finalizer_failpoint() {
    let _serial = crate::product::workspace_engine::single_candidate_compile_test_lock().await;
    let (_tmp, lifecycle, mut engine) = approval_fixture();
    let (event_tx, mut event_rx) = mpsc::channel(32);
    engine.event_tx = event_tx;
    let _guard = engine.register_single_candidate_compile_failpoint(
        SingleCandidateCompileCheckpoint::ProvenancePersisted,
    );
    let error = engine
        .handle_human_gate_termination(
            crate::web::workspace_ws_types::HumanConfirmDecision::Confirm,
        )
        .await
        .expect_err("compile failpoint must fail closed");
    assert!(error.contains("human gate remains open"), "{error}");
    let durable = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("durable session");
    assert_ne!(durable.status, WorkspaceSessionStatus::Confirmed);
    assert_ne!(
        durable.single_candidate_phase,
        Some(SingleCandidatePhase::Completed)
    );
    assert_eq!(engine.session().stage, WorkspaceStage::HumanConfirm);
    assert!(
        lifecycle
            .get_workspace_session(&engine.session().session_id)
            .expect("durable session")
            .provider_start_ledger
            .is_empty()
    );
    assert!(
        !event_rx
            .try_recv()
            .is_ok_and(|event| matches!(event, EngineEvent::HumanGateClosed { .. }))
    );
}

#[tokio::test]
async fn conversational_gate_approve_confirms_only_after_durable_compile() {
    let _serial = crate::product::workspace_engine::single_candidate_compile_test_lock().await;
    let (_tmp, lifecycle, mut engine) = approval_fixture();
    let (event_tx, mut event_rx) = mpsc::channel(32);
    engine.event_tx = event_tx;
    let result = engine
        .handle_human_gate_termination(
            crate::web::workspace_ws_types::HumanConfirmDecision::Confirm,
        )
        .await;
    assert_eq!(result, Ok(HumanGateCloseOutcome::Confirmed), "{result:?}");
    let durable = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("durable session");
    assert_eq!(durable.status, WorkspaceSessionStatus::Confirmed);
    assert_eq!(
        durable.single_candidate_phase,
        Some(SingleCandidatePhase::Completed)
    );
    let mut close_events = 0;
    while let Ok(event) = event_rx.try_recv() {
        if matches!(event, EngineEvent::HumanGateClosed { .. }) {
            close_events += 1;
        }
    }
    assert_eq!(close_events, 1);
}

#[tokio::test]
async fn conversational_gate_approve_retains_gate_snapshot_in_confirmed_terminal() {
    // 7.2 快照断裂追修：SC 计划批准链（close_human_gate → compile →
    // confirm_work_item_plan）必须在 Confirmed+Completed 终态保留
    // human_gate_snapshot——D11（预算接续原 session 快照，单一预算源）与
    // D16（原 plan session 是唯一门宿主）；REQ-GCE-03 修订链重开同一门依赖
    // 该快照在场。终态由真实引擎批准链产生，禁止手工构造终态快照。
    let _serial = crate::product::workspace_engine::single_candidate_compile_test_lock().await;
    let (_tmp, lifecycle, mut engine) = approval_fixture();
    let (event_tx, _event_rx) = mpsc::channel(32);
    engine.event_tx = event_tx;
    let session_id = engine.session().session_id.clone();
    let result = engine
        .handle_human_gate_termination(
            crate::web::workspace_ws_types::HumanConfirmDecision::Confirm,
        )
        .await;
    assert_eq!(result, Ok(HumanGateCloseOutcome::Confirmed), "{result:?}");
    let durable = lifecycle
        .get_workspace_session(&session_id)
        .expect("durable session");
    assert_eq!(durable.status, WorkspaceSessionStatus::Confirmed);
    assert_eq!(
        durable.single_candidate_phase,
        Some(SingleCandidatePhase::Completed)
    );
    let snapshot = durable.human_gate_snapshot.as_ref().expect(
        "SC plan approval must retain the gate snapshot on the Confirmed terminal (D11/D16)",
    );
    assert_eq!(snapshot.manual_repairs_remaining, 1);
    assert_eq!(durable.human_gate_reservation, None);
}

#[tokio::test]
async fn conversational_gate_post_approve_feedback_keeps_structured_stage_rejection() {
    // D11 单一预算源:首次 approve 成功后的终态是 Confirmed+Completed 且门快照保留
    // 在 session record 上(与 group_amendment_chain 的 amendment_chain_fixture 同构),
    // 此处无任何 Open/Applying PlanAmendmentContext。该状态下收到 human_gate_feedback
    // 必须保持 7.2 之前的结构化 stage 拒绝,而不是上抛泛型 Err(经 ws 层变成
    // WsOutMessage::Error)。
    let (root, lifecycle, _engine) = super::conversational_gate::gate_fixture(1);
    let session_id = lifecycle
        .list_workspace_sessions("project_0001", "issue_0001")
        .expect("list sessions")
        .into_iter()
        .find(|record| record.entity_id == "plan_0001")
        .expect("gate session")
        .id;
    let mut record = lifecycle
        .get_workspace_session(&session_id)
        .expect("gate session");
    record.status = WorkspaceSessionStatus::Confirmed;
    record.single_candidate_phase = Some(SingleCandidatePhase::Completed);
    record.human_gate_snapshot = Some(crate::product::work_item_plan_policy::HumanGateSnapshot {
        findings: Vec::new(),
        repeated_fingerprints: Vec::new(),
        attempts_used: 0,
        manual_repairs_remaining: 1,
        trigger: crate::product::work_item_plan_policy::HumanReason::NativeHumanRequired,
        resumable: false,
    });
    crate::product::json_store::write_json(
        &lifecycle
            .app_paths()
            .issue_lifecycle_root(&record.project_id, &record.issue_id)
            .join("workspace-sessions")
            .join(format!("{}.json", record.id)),
        &record,
    )
    .expect("persist post-approve session");
    let before = record.clone();

    let (event_tx, _event_rx) = mpsc::channel(8);
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
    );

    let outcome = engine
        .handle_human_gate_feedback(super::conversational_gate::feedback(
            "cmd_post_approve_feedback",
        ))
        .await;

    assert_eq!(
        outcome,
        Ok(HumanGateCommandOutcome::Rejected {
            code: "WORK_ITEM_PLAN_HUMAN_GATE_STAGE_INVALID".to_string(),
            reason: "human gate feedback is only available for a single-candidate work-item plan in human_confirm"
                .to_string(),
        }),
        "post-approve feedback without an amendment context must keep the structured stage rejection"
    );
    assert_eq!(
        lifecycle
            .get_workspace_session(&engine.session().session_id)
            .expect("durable session after rejection"),
        before,
        "stage rejection must stay zero side effect"
    );
    assert!(
        lifecycle
            .list_human_gate_turns(&engine.session().session_id)
            .expect("turns")
            .is_empty()
    );
}

#[tokio::test]
async fn conversational_gate_abandon_is_terminal_without_compile() {
    let (_root, lifecycle, mut engine, mut event_rx) =
        super::conversational_gate::gate_fixture_with_event_rx(1);
    let session_id = engine.session().session_id.clone();
    assert_eq!(
        engine
            .handle_human_gate_termination(
                crate::web::workspace_ws_types::HumanConfirmDecision::Terminate
            )
            .await
            .expect("terminate"),
        HumanGateCloseOutcome::Abandoned
    );
    let durable = lifecycle
        .get_workspace_session(&session_id)
        .expect("session");
    assert_eq!(durable.status, WorkspaceSessionStatus::Terminated);
    let plan_store = WorkItemPlanStore::new(lifecycle.app_paths());
    assert!(
        plan_store
            .list_compile_transactions("project_0001", "issue_0001", "plan_0001")
            .expect("compile transactions")
            .is_empty()
    );
    assert!(
        matches!(event_rx.try_recv(), Ok(EngineEvent::HumanGateClosed { decision, stage }) if decision == "terminate" && stage == "completed")
    );
}

#[tokio::test]
async fn conversational_gate_close_is_busy_during_inflight_turn() {
    let (_root, _lifecycle, mut engine) = gate_fixture(1);
    let opened = engine
        .handle_human_gate_feedback(super::conversational_gate::feedback("busy-close"))
        .await
        .expect("feedback");
    let turn_id = match opened {
        HumanGateCommandOutcome::TurnOpened { turn, .. } => turn.turn_id,
        other => panic!("expected opened turn, got {other:?}"),
    };
    for decision in [
        crate::web::workspace_ws_types::HumanConfirmDecision::Confirm,
        crate::web::workspace_ws_types::HumanConfirmDecision::Terminate,
    ] {
        assert_eq!(
            engine
                .handle_human_gate_termination(decision)
                .await
                .expect("busy"),
            HumanGateCloseOutcome::Busy {
                turn_id: turn_id.clone()
            }
        );
    }
}

// —— F7 项 1（历史观察项族 12）：confirm 失败必须上抛 validator findings ——

fn seeded_gate_findings() -> Vec<WorkItemSplitFinding> {
    vec![
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
    ]
}

const SEEDED_FAILURE_REASON: &str =
    "Final Compile strict validator failed（errors: 1, warnings: 1）";

/// 模拟 execute_initial_plan_compile 的 validator 失败落盘结果：一个携带
/// failure_reason + validator_findings 原文的 Failed compile transaction。
fn seed_failed_compile_with_findings(lifecycle: &LifecycleStore, plan_id: &str) {
    use crate::product::models::{
        IssueWorkItemDependencyEdge, IssueWorkItemPlan, WorkItemPlanCommitState,
        WorkItemPlanCompileStatus, WorkItemPlanCompileTransaction,
    };
    WorkItemPlanStore::new(lifecycle.app_paths())
        .put_compile_transaction(&WorkItemPlanCompileTransaction {
            compile_id: "compile_gate_findings".to_string(),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: plan_id.to_string(),
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
            validator_findings: seeded_gate_findings(),
            abort_requested_at: None,
            failure_reason: Some(SEEDED_FAILURE_REASON.to_string()),
            previous_plan_snapshot: IssueWorkItemPlan {
                id: plan_id.to_string(),
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
}

#[tokio::test]
async fn conversational_gate_approve_compile_failure_surfaces_validator_findings() {
    let (_root, lifecycle, mut engine) = super::conversational_gate::gate_fixture(1);
    seed_failed_compile_with_findings(&lifecycle, "plan_0001");

    let error = engine
        .handle_human_gate_termination(HumanConfirmDecision::Confirm)
        .await
        .expect_err("incomplete fixture must fail closed before confirming");
    assert!(error.contains("human gate remains open"), "{error}");
    // failure_reason 原文与每条 finding 的 severity/code/message/work_item_ids 原文可见
    assert!(error.contains(SEEDED_FAILURE_REASON), "{error}");
    assert!(error.contains("[error] WI_DEP_CYCLE"), "{error}");
    assert!(
        error.contains("work item dependency cycle: wi_a -> wi_b -> wi_a"),
        "{error}"
    );
    assert!(
        error.contains("[warning] WI_MISSING_VERIFICATION"),
        "{error}"
    );
    assert!(error.contains("wi_a, wi_b"), "{error}");

    // 结构化副本供 web 层 ProtocolError.context 使用
    let context = engine
        .last_human_gate_close_compile_failure_context()
        .expect("structured findings context");
    assert_eq!(
        context["failure_reason"],
        serde_json::json!(SEEDED_FAILURE_REASON)
    );
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

// 注：compile 失败后 durable 停在 WaitingForHuman+phase=Failed（既有语义，修订后
// 须重走 Evaluate/Approval 链），此时重试 confirm 属项 2 的「真冲突保持」分支，
// 由 late_confirm_with_waiting_phase_drift_keeps_conflict 覆盖，此处不重复断言。

#[tokio::test]
async fn conversational_gate_approve_success_leaves_no_compile_failure_context() {
    let _serial = crate::product::workspace_engine::single_candidate_compile_test_lock().await;
    let (_tmp, _lifecycle, mut engine) = approval_fixture();
    let (event_tx, _event_rx) = mpsc::channel(32);
    engine.event_tx = event_tx;
    let result = engine
        .handle_human_gate_termination(HumanConfirmDecision::Confirm)
        .await;
    assert_eq!(result, Ok(HumanGateCloseOutcome::Confirmed), "{result:?}");
    assert!(
        engine
            .last_human_gate_close_compile_failure_context()
            .is_none(),
        "successful approval must not surface stale compile failure findings"
    );
}

// —— F7 项 2（地雷 2，F3 run1d）：gate close 竞态幂等化（先到者赢、后到者幂等友好） ——

/// 模拟另一个 worker 赢得关门竞态后的 durable 状态漂移；engine 内存会话保持
/// 陈旧（HumanConfirm/WaitingForHuman），复刻迟到者视角。
fn drift_gate_session(
    lifecycle: &LifecycleStore,
    engine: &WorkspaceEngine,
    mutate: impl FnOnce(&mut WorkspaceSessionRecord),
) -> WorkspaceSessionRecord {
    let mut record = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("gate session");
    mutate(&mut record);
    crate::product::json_store::write_json(
        &lifecycle
            .app_paths()
            .issue_lifecycle_root(&record.project_id, &record.issue_id)
            .join("workspace-sessions")
            .join(format!("{}.json", record.id)),
        &record,
    )
    .expect("persist drifted gate session");
    record
}

#[tokio::test]
async fn late_confirm_after_durable_confirmed_is_idempotent_already_closed() {
    let (_root, lifecycle, mut engine, mut event_rx) =
        super::conversational_gate::gate_fixture_with_event_rx(1);
    let drifted = drift_gate_session(&lifecycle, &engine, |record| {
        record.status = WorkspaceSessionStatus::Confirmed;
        record.single_candidate_phase = Some(SingleCandidatePhase::Completed);
        record.human_gate_snapshot = None;
    });

    let outcome = engine
        .handle_human_gate_termination(HumanConfirmDecision::Confirm)
        .await;
    assert_eq!(
        outcome,
        Ok(HumanGateCloseOutcome::AlreadyClosed {
            status: WorkspaceSessionStatus::Confirmed
        }),
        "late confirm after the winner closed must be an idempotent no-op"
    );
    // durable 零改写；会话不 abort
    assert_eq!(
        lifecycle
            .get_workspace_session(engine.session().session_id.as_str())
            .expect("durable session"),
        drifted
    );
    // 事件：无第二个 HumanGateClosed；恰好一条幂等提示
    let mut already_closed_notices = 0;
    let mut closed_events = 0;
    while let Ok(event) = event_rx.try_recv() {
        match event {
            EngineEvent::HumanGateClosed { .. } => closed_events += 1,
            EngineEvent::ProtocolError { code, .. } if code == "HUMAN_GATE_ALREADY_CLOSED" => {
                already_closed_notices += 1;
            }
            _ => {}
        }
    }
    assert_eq!(closed_events, 0, "idempotent no-op must not close again");
    assert_eq!(already_closed_notices, 1, "visible notice event required");
    // in-memory 会话状态同步到 durable
    assert_eq!(
        engine.session().session_status,
        WorkspaceSessionStatus::Confirmed
    );
}

#[tokio::test]
async fn late_confirm_after_durable_running_is_idempotent_already_closed() {
    let (_root, lifecycle, mut engine, _event_rx) =
        super::conversational_gate::gate_fixture_with_event_rx(1);
    let drifted = drift_gate_session(&lifecycle, &engine, |record| {
        record.status = WorkspaceSessionStatus::Running;
    });
    let outcome = engine
        .handle_human_gate_termination(HumanConfirmDecision::Confirm)
        .await;
    assert_eq!(
        outcome,
        Ok(HumanGateCloseOutcome::AlreadyClosed {
            status: WorkspaceSessionStatus::Running
        })
    );
    assert_eq!(
        lifecycle
            .get_workspace_session(engine.session().session_id.as_str())
            .expect("durable session"),
        drifted
    );
}

#[tokio::test]
async fn late_confirm_after_terminate_gets_explicit_terminated_error() {
    let (_root, lifecycle, mut engine, mut event_rx) =
        super::conversational_gate::gate_fixture_with_event_rx(1);
    let drifted = drift_gate_session(&lifecycle, &engine, |record| {
        record.status = WorkspaceSessionStatus::Terminated;
    });
    let error = engine
        .handle_human_gate_termination(HumanConfirmDecision::Confirm)
        .await
        .expect_err("late confirm on a terminated gate must be an explicit error");
    assert!(error.contains("already terminated"), "{error}");
    assert!(!error.contains("conflict"), "{error}");
    assert_eq!(
        lifecycle
            .get_workspace_session(engine.session().session_id.as_str())
            .expect("durable session"),
        drifted
    );
    assert!(
        event_rx.try_recv().is_err(),
        "terminated translation must not emit gate events"
    );
}

#[tokio::test]
async fn late_terminate_after_terminate_gets_explicit_terminated_error() {
    let (_root, _lifecycle, mut engine, _event_rx) =
        super::conversational_gate::gate_fixture_with_event_rx(1);
    drift_gate_session(&_lifecycle, &engine, |record| {
        record.status = WorkspaceSessionStatus::Terminated;
    });
    let error = engine
        .handle_human_gate_termination(HumanConfirmDecision::Terminate)
        .await
        .expect_err("late terminate must be an explicit terminated error");
    assert!(error.contains("already terminated"), "{error}");
    assert!(!error.contains("conflict"), "{error}");
}

#[tokio::test]
async fn late_confirm_with_waiting_phase_drift_keeps_conflict() {
    // 被推翻的旧假设（3.6 矩阵族⑤ / oracle 裁决 A）：本测试曾断言「Evaluate
    // 相位漂移 = 真冲突」，即门只允许 Approval 相位关门——这正是 Evaluate 进门
    // × Approval 关门死锁的根因。修复后 close 前置接受 phase∈{Approval,Evaluate}
    // 并在 confirm 时人工权威升级，Evaluate 漂移不再冲突（改由
    // conversational_gate_confirm_at_evaluate_gate_* 系列守卫）。仍应 conflict 的
    // 漂移形态改为：漂到 Generate（非门相位）或清空门快照（反伪造判据——
    // update_workspace_session_status 只在终态清快照、从不创建快照）。
    {
        let (_root, _lifecycle, mut engine) = super::conversational_gate::gate_fixture(1);
        drift_gate_session(&_lifecycle, &engine, |record| {
            record.single_candidate_phase = Some(SingleCandidatePhase::Generate);
        });
        let error = engine
            .handle_human_gate_termination(HumanConfirmDecision::Confirm)
            .await
            .expect_err("phase drift to a non-gate phase must stay a conflict");
        assert!(error.contains("product_store_conflict"), "{error}");
        assert!(error.contains("human_gate_close"), "{error}");
    }
    {
        let (_root, _lifecycle, mut engine) = super::conversational_gate::gate_fixture(1);
        drift_gate_session(&_lifecycle, &engine, |record| {
            record.human_gate_snapshot = None;
        });
        let error = engine
            .handle_human_gate_termination(HumanConfirmDecision::Confirm)
            .await
            .expect_err("snapshot-less gate drift must stay a conflict (anti-forgery)");
        assert!(error.contains("product_store_conflict"), "{error}");
        assert!(error.contains("human_gate_close"), "{error}");
    }
}

/// Evaluate 相位人工门 fixture（与 conversational_gate_revision 的
/// `evaluate_gate_revision_fixture` 同构种子）：accepted contract drafts 基座
/// （批准链 compile 可真实走通）+ 真实 handoff-clean rep4 候选三 refs +
/// Evaluate 相位 WaitingForHuman 门快照，复刻 request-change 修订后
/// repeated_fingerprint 门保持 Evaluate 进门的 durable 形态
/// （routing_scope 的 EnterHumanGate 不提升相位）。
fn evaluate_gate_fixture(
    session_id: &str,
    budget: u32,
) -> (tempfile::TempDir, LifecycleStore, WorkspaceEngine) {
    let (root, lifecycle, _plan_id, mut engine) =
        super::make_work_item_plan_engine_with_accepted_contract_drafts();
    super::single_candidate_recovery::single_candidate_recovery_record(
        &lifecycle,
        &mut engine,
        SingleCandidatePhase::Evaluate,
        RunPolicy::Interactive,
    );
    let gate_refs =
        super::single_candidate_recovery::single_candidate_recovery_persist_candidate_artifacts(
            &lifecycle,
            &engine,
            "evaluate-gate-close",
            &super::conversational_gate_revision::handoff_clean_rep4(),
        );
    super::single_candidate_recovery::single_candidate_recovery_update_refs(
        &lifecycle,
        &mut engine,
        SingleCandidatePhase::Evaluate,
        gate_refs,
    );
    let mut record = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("evaluate gate session record");
    record.review_rounds = 0;
    record.status = WorkspaceSessionStatus::WaitingForHuman;
    record.human_gate_snapshot = Some(crate::product::work_item_plan_policy::HumanGateSnapshot {
        findings: Vec::new(),
        repeated_fingerprints: Vec::new(),
        attempts_used: 0,
        manual_repairs_remaining: budget,
        trigger: crate::product::work_item_plan_policy::HumanReason::NativeHumanRequired,
        resumable: true,
    });
    crate::product::json_store::write_json(
        &lifecycle
            .app_paths()
            .issue_root(&record.project_id, &record.issue_id)
            .join("workspace-sessions")
            .join(format!("{}.json", record.id)),
        &record,
    )
    .expect("persist evaluate gate session");
    let mut session = WorkspaceSession::from_record(record);
    session.stage = WorkspaceStage::HumanConfirm;
    session.session_status = WorkspaceSessionStatus::WaitingForHuman;
    session.artifact = Some(crate::web::workspace_ws_types::ArtifactPayload::Markdown {
        markdown: super::conversational_gate_revision::handoff_clean_rep4(),
        diff: None,
    });
    let (event_tx, _event_rx) = mpsc::channel(64);
    let engine = WorkspaceEngine::new_persistent(
        Arc::new(CheckpointStore::new(
            root.path().join(format!("{session_id}-checkpoints")),
        )),
        lifecycle.clone(),
        event_tx,
        session,
    );
    (root, lifecycle, engine)
}

#[tokio::test]
async fn conversational_gate_confirm_at_evaluate_gate_completes_approval_chain() {
    // 3.6 矩阵族⑤（oracle 裁决 A）：request-change 修订后 repeated_fingerprint
    // 门保持 Evaluate 进门（EnterHumanGate 不提升相位），confirm 的 close CAS
    // 前置若硬要求 Approval 即死锁。修复 = close 时人工权威升级：confirm 在
    // close CAS 内原子提升 durable phase→Approval 再走既有 compile 链。本测试
    // 同时守卫 CAS 成功后的内存相位同步——缺失时 compile 链以内存 phase 判
    // auto_confirm，compile 会成功但不落 Confirmed 而回 enter_human_confirm。
    let _serial = crate::product::workspace_engine::single_candidate_compile_test_lock().await;
    let (_tmp, lifecycle, mut engine) = evaluate_gate_fixture("evaluate_gate_confirm", 1);
    let session_id = engine.session().session_id.clone();

    let outcome = engine
        .handle_human_gate_termination(HumanConfirmDecision::Confirm)
        .await
        .expect("confirm at an Evaluate gate must unlock the close deadlock");
    assert_eq!(outcome, HumanGateCloseOutcome::Confirmed);

    let durable = lifecycle
        .get_workspace_session(&session_id)
        .expect("closed session");
    assert_eq!(durable.status, WorkspaceSessionStatus::Confirmed);
    assert_eq!(
        durable.single_candidate_phase,
        Some(SingleCandidatePhase::Completed)
    );
}

#[tokio::test]
async fn conversational_gate_terminate_at_evaluate_gate_abandons_durably() {
    // Evaluate 门 terminate：关门成功但不提升相位（terminate 不是批准权威），
    // 终态 Terminated + 快照/reservation 清空，不进 compile。
    let (_tmp, lifecycle, mut engine) = evaluate_gate_fixture("evaluate_gate_terminate", 1);
    let session_id = engine.session().session_id.clone();

    assert_eq!(
        engine
            .handle_human_gate_termination(HumanConfirmDecision::Terminate)
            .await
            .expect("terminate at an Evaluate gate must close without promotion"),
        HumanGateCloseOutcome::Abandoned
    );

    let durable = lifecycle
        .get_workspace_session(&session_id)
        .expect("terminated session");
    assert_eq!(durable.status, WorkspaceSessionStatus::Terminated);
    assert_eq!(
        durable.single_candidate_phase,
        Some(SingleCandidatePhase::Evaluate),
        "terminate 不得提升相位"
    );
    assert_eq!(durable.human_gate_snapshot, None);
    assert_eq!(durable.human_gate_reservation, None);
    assert_eq!(engine.session().stage, WorkspaceStage::Completed);
}
