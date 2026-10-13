use super::conversational_gate::gate_fixture;
use super::*;
use crate::product::models::{
    IssueWorkItemPlanOptions, IssueWorkItemPlanStatus, SingleCandidatePhase, WorkspaceSessionStatus,
};
use crate::product::work_item_plan_policy::RunPolicy;
use crate::product::workspace_engine::compile::SingleCandidateCompileCheckpoint;
use crate::product::workspace_engine::conversational_gate::close_cas_attempts;

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
        candidate_recovery: None,
        findings: Vec::new(),
        repeated_fingerprints: Vec::new(),
        attempts_used: 0,
        manual_repairs_remaining: 1,
        trigger: crate::product::work_item_plan_policy::HumanReason::NativeHumanRequired,
        resumable: false,
        accepted_feedback_turns: None,
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
        .handle_human_gate_termination(HumanGateCloseDecision::Approve)
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
        .handle_human_gate_termination(HumanGateCloseDecision::Approve)
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
        .handle_human_gate_termination(HumanGateCloseDecision::Approve)
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
        .handle_human_gate_termination(HumanGateCloseDecision::Approve)
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
        candidate_recovery: None,
        findings: Vec::new(),
        repeated_fingerprints: Vec::new(),
        attempts_used: 0,
        manual_repairs_remaining: 1,
        trigger: crate::product::work_item_plan_policy::HumanReason::NativeHumanRequired,
        resumable: false,
        accepted_feedback_turns: None,
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
            .handle_human_gate_termination(HumanGateCloseDecision::Abandon)
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
        HumanGateCloseDecision::Approve,
        HumanGateCloseDecision::Abandon,
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
                ever_confirmed: None,
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
async fn conversational_gate_approve_compile_failure_ignores_stale_failed_transaction() {
    let (_root, lifecycle, mut engine) = super::conversational_gate::gate_fixture(1);
    // SG3b 行为切割留痕：本夹具（无候选 refs）close 失败曾按 latest-scan 捞
    // 旧 Failed tx 的 failure_reason/findings 冒充本次失败；现上下文只认
    // 本次 close 捕获的 typed PlanningCompileFailure。
    seed_failed_compile_with_findings(&lifecycle, "plan_0001");

    let error = engine
        .handle_human_gate_termination(HumanGateCloseDecision::Approve)
        .await
        .expect_err("incomplete fixture must fail closed before confirming");
    assert!(error.contains("human gate remains open"), "{error}");
    // 不同 failure 的旧 Failed tx 不捞：其 findings/failure_reason 不出现。
    assert!(!error.contains(SEEDED_FAILURE_REASON), "{error}");
    assert!(!error.contains("WI_DEP_CYCLE"), "{error}");
    assert!(!error.contains("WI_MISSING_VERIFICATION"), "{error}");

    // typed 上下文（本 close 的 refs 缺失失败）经 wire 投影可见。
    let context = sg3b_wire_context(&engine);
    assert_eq!(context["kind"], serde_json::json!("operational"));
    assert_eq!(
        context["stable_code"],
        serde_json::json!("SOURCE_STORE_MALFORMED_REF")
    );
    assert!(
        context["failure_id"]
            .as_str()
            .is_some_and(|id| !id.is_empty()),
        "{context}"
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
        .handle_human_gate_termination(HumanGateCloseDecision::Approve)
        .await;
    assert_eq!(result, Ok(HumanGateCloseOutcome::Confirmed), "{result:?}");
    assert!(
        engine.last_human_gate_close_compile_failure().is_none(),
        "successful approval must not surface stale compile failure findings"
    );
}

// —— SG3b（REQ-SG-03、REQ-PLN-09、REQ-CG-04/05）：冻结 Final Compile 及
// 同次 typed 传播——结构门先于 prepare/transaction，typed
// failure_id/approval/source context 直达 close→WS/REST，旧 Failed
// transaction 不同 failure 不捞，冻结 refs/context 漂移拒不重 lower。 ——

use crate::product::work_item_contract::RequiredInputContract;
use crate::product::work_item_contract::canonical_contract_fixture;
use crate::product::work_item_plan_compiler::{
    PlanCandidateIr, PlanCandidateItemIr, PlanCandidateMechanicalReport,
    WORK_ITEM_PLAN_COMPILER_VERSION,
};
use crate::product::work_item_plan_source_store::{
    PlanCandidateIrRecord, PlanCandidateMechanicalReportRecord, SourceRevisionRecord,
    SourceStoreScope, WorkItemPlanSourceStore,
};
use sha2::{Digest, Sha256};

/// 把自定义 IR items 持久化为 SC source/IR/report 三 refs 候选（报告沿旧
/// None 形态；Some 冻结由 `sg3b_report_ref_with_frozen_structure` 升级）。
fn sg3b_persist_candidate_with_ir(
    lifecycle: &LifecycleStore,
    engine: &WorkspaceEngine,
    suffix: &str,
    ir_items: Vec<PlanCandidateItemIr>,
) -> (String, String, String, String, String) {
    let project_id = engine.session().project_id.clone();
    let issue_id = engine.session().issue_id.clone();
    let plan_id = engine.session().entity_id.clone();
    let scope = SourceStoreScope {
        project_id: project_id.clone(),
        issue_id: issue_id.clone(),
        plan_id: plan_id.clone(),
    };
    let source_store = WorkItemPlanSourceStore::new(lifecycle.app_paths());
    let source_text = format!("# sg3b single candidate source {suffix}\n");
    let source_hash = hex::encode(Sha256::digest(source_text.as_bytes()));
    let mut source = SourceRevisionRecord {
        id: format!("source-{suffix}"),
        source: source_text,
        source_revision_hash: source_hash.clone(),
        content_hash: String::new(),
    };
    source.content_hash = source.content_hash().expect("source content hash");
    let source_ref = source_store
        .put_source_revision(&scope.project_id, &scope.issue_id, &scope.plan_id, &source)
        .expect("persist sg3b source");
    let repository_id = engine
        .work_item_plan_repository_id(lifecycle, &{
            lifecycle
                .get_issue_work_item_plan(&project_id, &issue_id, &plan_id)
                .expect("sg3b previous plan")
        })
        .expect("sg3b repository id");
    let mut ir_record = PlanCandidateIrRecord {
        id: format!("ir-{suffix}"),
        source_revision_id: source.id.clone(),
        ir: PlanCandidateIr {
            source_revision_hash: source_hash,
            compiler_version: WORK_ITEM_PLAN_COMPILER_VERSION.to_string(),
            items: ir_items,
        },
        content_hash: String::new(),
    };
    ir_record.content_hash = ir_record.content_hash().expect("IR content hash");
    let ir_ref = source_store
        .put_plan_candidate_ir(
            &scope.project_id,
            &scope.issue_id,
            &scope.plan_id,
            &ir_record,
        )
        .expect("persist sg3b IR");
    let report_ref = sg3b_put_mechanical_report(
        &source_store,
        &scope,
        &format!("report-{suffix}"),
        &source,
        &ir_record,
        None,
    );
    (source_ref, ir_ref, report_ref, source.id, ir_record.id)
}

fn sg3b_put_mechanical_report(
    source_store: &WorkItemPlanSourceStore,
    scope: &SourceStoreScope,
    report_id: &str,
    source: &SourceRevisionRecord,
    ir_record: &PlanCandidateIrRecord,
    structure_report: Option<crate::product::planning_failure::PersistedPlanningStructureReport>,
) -> String {
    let mut report = PlanCandidateMechanicalReportRecord {
        id: report_id.to_string(),
        source_revision_id: source.id.clone(),
        ir_id: ir_record.id.clone(),
        report: PlanCandidateMechanicalReport {
            source_revision_hash: ir_record.ir.source_revision_hash.clone(),
            compiler_version: ir_record.ir.compiler_version.clone(),
            findings: Vec::new(),
        },
        structure_report,
        content_hash: String::new(),
    };
    report.content_hash = report.content_hash().expect("report content hash");
    source_store
        .put_mechanical_report(&scope.project_id, &scope.issue_id, &scope.plan_id, &report)
        .expect("persist sg3b mechanical report")
}

/// 用真实 live context + SG1 纯评估冻结 Some 结构报告并落新 report ref
///（候选门同款冻结路径；返回 (report_ref, context_digest)）。
fn sg3b_report_ref_with_frozen_structure(
    engine: &WorkspaceEngine,
    lifecycle: &LifecycleStore,
    scope: &SourceStoreScope,
    suffix: &str,
    source: &SourceRevisionRecord,
    source_ref: &str,
    ir_record: &PlanCandidateIrRecord,
    ir_ref: &str,
) -> (String, String) {
    let source_store = WorkItemPlanSourceStore::new(lifecycle.app_paths());
    let context = engine
        .build_single_candidate_structure_context(&ir_record.ir, source_ref, ir_ref)
        .expect("sg3b live structure context");
    let assessment =
        crate::product::workspace_engine::plan_projection::assess_single_candidate_structure(
            source.source.clone(),
            &ir_record.ir,
            &context,
        );
    let crate::product::workspace_engine::types::StructuralAssessment::Accepted(prepared) =
        assessment
    else {
        panic!("sg3b fixture candidate must pass the structure gate");
    };
    let frozen =
        crate::product::workspace_engine::plan_projection::freeze_persisted_structure_report(
            &prepared.report,
            &source.id,
            &ir_record.id,
        )
        .expect("freeze sg3b structure report");
    let digest = frozen.context_digest.clone();
    let report_ref = sg3b_put_mechanical_report(
        &source_store,
        scope,
        &format!("report-{suffix}-frozen"),
        source,
        ir_record,
        Some(frozen),
    );
    (report_ref, digest)
}

fn sg3b_cycle_ir_items(repository_id: &str) -> Vec<PlanCandidateItemIr> {
    let mut contract_a = canonical_contract_fixture("wi_sg3b_a");
    contract_a.input_contracts = vec![RequiredInputContract {
        contract_id: "contract.sg3b_b".to_string(),
        provider_logical_work_item_id: "wi_sg3b_b".to_string(),
        required_capabilities: vec!["sg3b_ready".to_string()],
        compatibility_policy:
            crate::product::work_item_contract::ContractCompatibilityPolicy::RequireAll,
    }];
    let mut contract_b = canonical_contract_fixture("wi_sg3b_b");
    contract_b.input_contracts = vec![RequiredInputContract {
        contract_id: "contract.sg3b_a".to_string(),
        provider_logical_work_item_id: "wi_sg3b_a".to_string(),
        required_capabilities: vec!["sg3b_ready".to_string()],
        compatibility_policy:
            crate::product::work_item_contract::ContractCompatibilityPolicy::RequireAll,
    }];
    vec![
        PlanCandidateItemIr {
            target_repository_id: repository_id.to_string(),
            contract: contract_a,
            verification_plan: crate::product::models::WorkItemDraftVerificationPlan {
                checks: Vec::new(),
            },
            trusted_commands: Vec::new(),
        },
        PlanCandidateItemIr {
            target_repository_id: repository_id.to_string(),
            contract: contract_b,
            verification_plan: crate::product::models::WorkItemDraftVerificationPlan {
                checks: Vec::new(),
            },
            trusted_commands: Vec::new(),
        },
    ]
}

/// 两个互不依赖但写面重叠的 WI：结构门（有效图）通过，strict validator 在
/// execute 段以 parallel_scope_overlap 落本运行 Failed transaction。
fn sg3b_scope_overlap_ir_items(repository_id: &str) -> Vec<PlanCandidateItemIr> {
    let mut contract_a = canonical_contract_fixture("wi_sg3b_scope_a");
    contract_a.input_contracts.clear();
    contract_a.handoff_contract.provided_contract_refs.clear();
    contract_a.write_policy.exclusive_scopes = vec!["src/sg3b_overlap".to_string()];
    let mut contract_b = canonical_contract_fixture("wi_sg3b_scope_b");
    contract_b.input_contracts.clear();
    contract_b.handoff_contract.provided_contract_refs.clear();
    contract_b.write_policy.exclusive_scopes = vec!["src/sg3b_overlap".to_string()];
    vec![
        PlanCandidateItemIr {
            target_repository_id: repository_id.to_string(),
            contract: contract_a,
            verification_plan: crate::product::models::WorkItemDraftVerificationPlan {
                checks: Vec::new(),
            },
            trusted_commands: Vec::new(),
        },
        PlanCandidateItemIr {
            target_repository_id: repository_id.to_string(),
            contract: contract_b,
            verification_plan: crate::product::models::WorkItemDraftVerificationPlan {
                checks: Vec::new(),
            },
            trusted_commands: Vec::new(),
        },
    ]
}

fn sg3b_wire_context(engine: &WorkspaceEngine) -> serde_json::Value {
    let failure = engine
        .last_human_gate_close_compile_failure()
        .expect("typed close compile failure context");
    serde_json::to_value(failure.wire_presentation()).expect("wire presentation json")
}

#[tokio::test]
async fn sg_03b_prepare_failure_this_close() {
    let _serial = crate::product::workspace_engine::single_candidate_compile_test_lock().await;
    let (_tmp, lifecycle, mut engine) = approval_fixture();
    let repository_id = engine
        .work_item_plan_repository_id(
            &lifecycle,
            &lifecycle
                .get_issue_work_item_plan(
                    &engine.session().project_id,
                    &engine.session().issue_id,
                    &engine.session().entity_id,
                )
                .expect("plan"),
        )
        .expect("repository");
    let (source_ref, ir_ref, report_ref, _source_id, _ir_id) = sg3b_persist_candidate_with_ir(
        &lifecycle,
        &engine,
        "sg3b_cycle",
        sg3b_cycle_ir_items(&repository_id),
    );
    super::single_candidate_recovery::single_candidate_recovery_update_refs(
        &lifecycle,
        &mut engine,
        SingleCandidatePhase::Approval,
        (source_ref.clone(), ir_ref.clone(), report_ref.clone()),
    );
    // 不同 failure 的旧 Failed transaction（strict validator 失败）不得被
    // latest-scan 捞为本次失败事实。
    let plan_id = engine.session().entity_id.clone();
    seed_failed_compile_with_findings(&lifecycle, &plan_id);

    let journal = crate::product::work_item_plan_store::observe_compile_transaction_writes();
    let (event_tx, mut event_rx) = mpsc::channel(32);
    engine.event_tx = event_tx;
    let error = engine
        .handle_human_gate_termination(HumanGateCloseDecision::Approve)
        .await
        .expect_err("cycle candidate must fail the final compile structure gate");

    // prepare 前结构失败：零 compile transaction 写入。
    assert!(
        journal.snapshots().is_empty(),
        "structure gate rejection must precede any transaction put"
    );
    assert!(error.contains("human gate remains open"), "{error}");
    assert!(
        error.contains("final compile structure gate rejected"),
        "{error}"
    );
    assert!(error.contains("dependency_cycle"), "{error}");
    // 旧 Failed tx（不同 failure）不捞：其 findings/failure_reason 不出现。
    assert!(!error.contains("WI_DEP_CYCLE"), "{error}");
    assert!(!error.contains(SEEDED_FAILURE_REASON), "{error}");

    // typed failure_id/approval/source context 同次直达 close 上下文。
    let context = sg3b_wire_context(&engine);
    let failure_id = context["failure_id"]
        .as_str()
        .expect("failure_id")
        .to_string();
    assert!(
        failure_id.starts_with("planning_compile_failure_"),
        "{failure_id}"
    );
    assert_eq!(context["kind"], serde_json::json!("structure"));
    assert_eq!(
        context["stable_code"],
        serde_json::json!("final_compile_structure_rejected")
    );
    let durable = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("durable session");
    assert_eq!(
        context["approval_attempt_id"],
        serde_json::json!(durable.approval_attempt_id),
        "approval identity must be the durable approval tuple of this close"
    );
    assert_eq!(
        context["source_revision_ref"],
        serde_json::json!(source_ref)
    );
    assert_eq!(context["plan_candidate_ir_ref"], serde_json::json!(ir_ref));
    assert_eq!(
        context["mechanical_report_ref"],
        serde_json::json!(report_ref)
    );
    assert!(
        context["context_digest"]
            .as_str()
            .is_some_and(|digest| !digest.is_empty()),
        "live structure context digest must ride the typed failure"
    );
    let findings = context["structure_findings"].as_array().expect("findings");
    assert_eq!(findings.len(), 1, "{context}");
    assert_eq!(findings[0]["code"], serde_json::json!("dependency_cycle"));
    let affected = findings[0]["affected_ids"].as_array().expect("witness");
    assert_eq!(affected.len(), 2, "{context}");
    // 本 close 无 validator findings：旧 tx 不得冒充。
    assert!(
        context
            .get("validator_findings")
            .map(serde_json::Value::is_null)
            .unwrap_or(true),
        "{context}"
    );

    // durable 安全 diagnostic：typed failure_id + 结构稳定 code 落 policy 诊断。
    let durable = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("durable session after failure");
    let diagnostic = durable
        .policy_diagnostics
        .last()
        .expect("structure rejection diagnostic");
    assert_eq!(diagnostic.code, "final_compile_structure_rejected");
    assert!(
        diagnostic.message.contains(&failure_id),
        "{}",
        diagnostic.message
    );

    // 晚发现结构缺口且无活动 grant：零 author（provider 启动）、零确认、零发布。
    assert!(durable.provider_start_ledger.is_empty());
    assert_ne!(durable.status, WorkspaceSessionStatus::Confirmed);
    assert_ne!(
        durable.single_candidate_phase,
        Some(SingleCandidatePhase::Completed)
    );
    assert!(durable.publication_provenance_ref.is_none());
    assert!(
        !event_rx
            .try_recv()
            .is_ok_and(|event| matches!(event, EngineEvent::HumanGateClosed { .. }))
    );
}

#[tokio::test]
async fn sg_03b_frozen_context_drift_refuses_without_relower() {
    let _serial = crate::product::workspace_engine::single_candidate_compile_test_lock().await;
    let (_tmp, lifecycle, mut engine) = approval_fixture();
    let project_id = engine.session().project_id.clone();
    let issue_id = engine.session().issue_id.clone();
    let plan_id = engine.session().entity_id.clone();
    let scope = SourceStoreScope {
        project_id: project_id.clone(),
        issue_id: issue_id.clone(),
        plan_id: plan_id.clone(),
    };
    let previous_plan = lifecycle
        .get_issue_work_item_plan(&project_id, &issue_id, &plan_id)
        .expect("plan");
    let repository_id = engine
        .work_item_plan_repository_id(&lifecycle, &previous_plan)
        .expect("repository");
    let single = |contract: crate::product::work_item_contract::CanonicalWorkItemContract| {
        PlanCandidateItemIr {
            target_repository_id: repository_id.clone(),
            contract,
            verification_plan: crate::product::models::WorkItemDraftVerificationPlan {
                checks: Vec::new(),
            },
            trusted_commands: Vec::new(),
        }
    };
    let mut contract = canonical_contract_fixture("wi_sg3b_drift");
    contract.input_contracts.clear();
    contract.handoff_contract.provided_contract_refs.clear();
    let (source_ref, ir_ref, _report_ref, source_id, ir_id) =
        sg3b_persist_candidate_with_ir(&lifecycle, &engine, "sg3b_drift", vec![single(contract)]);
    let source_store = WorkItemPlanSourceStore::new(lifecycle.app_paths());
    let source = source_store
        .get_source_revision(&scope, &source_ref)
        .expect("source record");
    let ir_record = source_store
        .get_plan_candidate_ir(&scope, &ir_ref)
        .expect("IR record");
    let (frozen_report_ref, _frozen_digest) = sg3b_report_ref_with_frozen_structure(
        &engine,
        &lifecycle,
        &scope,
        "sg3b_drift",
        &source,
        &source_ref,
        &ir_record,
        &ir_ref,
    );
    super::single_candidate_recovery::single_candidate_recovery_update_refs(
        &lifecycle,
        &mut engine,
        SingleCandidatePhase::Approval,
        (
            source_ref.clone(),
            ir_ref.clone(),
            frozen_report_ref.clone(),
        ),
    );

    // 冻结后 live 上下文漂移：plan options 变更 → options_digest →
    // context_digest 变化。
    let mut drifted_plan = previous_plan.clone();
    drifted_plan.options.include_e2e_tests = !previous_plan.options.include_e2e_tests;
    lifecycle
        .restore_issue_work_item_plan_snapshot(&project_id, &issue_id, &plan_id, &drifted_plan)
        .expect("drift plan options");
    let live_context = engine
        .build_single_candidate_structure_context(&ir_record.ir, &source_ref, &ir_ref)
        .expect("live context after drift");

    let journal = crate::product::work_item_plan_store::observe_compile_transaction_writes();
    let error = engine
        .handle_human_gate_termination(HumanGateCloseDecision::Approve)
        .await
        .expect_err("frozen context drift must refuse the final compile");
    assert!(error.contains("human gate remains open"), "{error}");
    assert!(
        error.contains("single_candidate_structure_context_drift"),
        "{error}"
    );
    // 冻结 refs/context 变化拒不重 lower：零 transaction、零发布物替换。
    assert!(journal.snapshots().is_empty(), "{error}");
    let durable = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("durable session");
    assert_eq!(
        durable.work_item_plan_source_revision_ref.as_deref(),
        Some(source_ref.as_str()),
        "current refs must keep pointing at the frozen candidate (no re-lower)"
    );
    assert_eq!(
        durable.mechanical_report_ref.as_deref(),
        Some(frozen_report_ref.as_str())
    );
    assert!(durable.publication_provenance_ref.is_none());
    assert_ne!(durable.status, WorkspaceSessionStatus::Confirmed);
    assert!(durable.provider_start_ledger.is_empty());

    let context = sg3b_wire_context(&engine);
    assert_eq!(context["kind"], serde_json::json!("operational"));
    assert_eq!(
        context["stable_code"],
        serde_json::json!("single_candidate_structure_context_drift")
    );
    assert_eq!(
        context["context_digest"],
        serde_json::json!(live_context.context_digest)
    );
    assert_eq!(
        context["source_revision_ref"],
        serde_json::json!(source_ref)
    );
    assert_eq!(context["plan_candidate_ir_ref"], serde_json::json!(ir_ref));
    assert_eq!(
        context["mechanical_report_ref"],
        serde_json::json!(frozen_report_ref)
    );
    assert!(
        context["approval_attempt_id"].as_str().is_some(),
        "{context}"
    );
}

#[tokio::test]
async fn sg_03b_this_run_validator_findings_still_surface() {
    let _serial = crate::product::workspace_engine::single_candidate_compile_test_lock().await;
    let (_tmp, lifecycle, mut engine) = approval_fixture();
    let plan_id = engine.session().entity_id.clone();
    let previous_plan = lifecycle
        .get_issue_work_item_plan(
            &engine.session().project_id,
            &engine.session().issue_id,
            &plan_id,
        )
        .expect("plan");
    let repository_id = engine
        .work_item_plan_repository_id(&lifecycle, &previous_plan)
        .expect("repository");
    let (source_ref, ir_ref, report_ref, _source_id, _ir_id) = sg3b_persist_candidate_with_ir(
        &lifecycle,
        &engine,
        "sg3b_scope",
        sg3b_scope_overlap_ir_items(&repository_id),
    );
    super::single_candidate_recovery::single_candidate_recovery_update_refs(
        &lifecycle,
        &mut engine,
        SingleCandidatePhase::Approval,
        (source_ref, ir_ref, report_ref),
    );

    let error = engine
        .handle_human_gate_termination(HumanGateCloseDecision::Approve)
        .await
        .expect_err("scope overlap must fail the strict validator in execute");
    assert!(error.contains("human gate remains open"), "{error}");
    assert!(error.contains("parallel_scope_overlap"), "{error}");

    // 本 close 运行（compile_run_id 匹配）的 Failed tx findings 原文可见。
    let context = sg3b_wire_context(&engine);
    let findings = context["validator_findings"].as_array().expect("findings");
    assert!(
        findings
            .iter()
            .any(|finding| finding["code"] == serde_json::json!("parallel_scope_overlap")),
        "{context}"
    );
    assert_eq!(context["kind"], serde_json::json!("operational"));
    let run_id = context["compile_run_id"].as_str().expect("run id");
    assert!(!run_id.is_empty(), "{context}");
    let failed_tx =
        crate::product::work_item_plan_store::WorkItemPlanStore::new(lifecycle.app_paths())
            .list_compile_transactions(
                &engine.session().project_id,
                &engine.session().issue_id,
                &engine.session().entity_id,
            )
            .expect("transactions")
            .into_iter()
            .find(|tx| tx.compile_id == run_id && tx.status == WorkItemPlanCompileStatus::Failed)
            .expect("this-run Failed transaction");
    assert_eq!(
        failed_tx.validator_findings.len(),
        findings.len(),
        "findings must originate from this run's Failed transaction"
    );
}

#[tokio::test]
async fn sg_03b_candidate_and_compile_share_effective_edges() {
    let _serial = crate::product::workspace_engine::single_candidate_compile_test_lock().await;
    let (_tmp, lifecycle, mut engine) = approval_fixture();
    let project_id = engine.session().project_id.clone();
    let issue_id = engine.session().issue_id.clone();
    let plan_id = engine.session().entity_id.clone();
    let scope = SourceStoreScope {
        project_id: project_id.clone(),
        issue_id: issue_id.clone(),
        plan_id: plan_id.clone(),
    };
    let previous_plan = lifecycle
        .get_issue_work_item_plan(&project_id, &issue_id, &plan_id)
        .expect("plan");
    let repository_id = engine
        .work_item_plan_repository_id(&lifecycle, &previous_plan)
        .expect("repository");
    let mut contract = canonical_contract_fixture("wi_sg3b_edges");
    contract.input_contracts.clear();
    contract.handoff_contract.provided_contract_refs.clear();
    let item = PlanCandidateItemIr {
        target_repository_id: repository_id.clone(),
        contract,
        verification_plan: crate::product::models::WorkItemDraftVerificationPlan {
            checks: Vec::new(),
        },
        trusted_commands: Vec::new(),
    };
    let (source_ref, ir_ref, _report_ref, _source_id, _ir_id) =
        sg3b_persist_candidate_with_ir(&lifecycle, &engine, "sg3b_edges", vec![item]);
    let source_store = WorkItemPlanSourceStore::new(lifecycle.app_paths());
    let source = source_store
        .get_source_revision(&scope, &source_ref)
        .expect("source record");
    let ir_record = source_store
        .get_plan_candidate_ir(&scope, &ir_ref)
        .expect("IR record");
    let (frozen_report_ref, _digest) = sg3b_report_ref_with_frozen_structure(
        &engine,
        &lifecycle,
        &scope,
        "sg3b_edges",
        &source,
        &source_ref,
        &ir_record,
        &ir_ref,
    );
    super::single_candidate_recovery::single_candidate_recovery_update_refs(
        &lifecycle,
        &mut engine,
        SingleCandidatePhase::Approval,
        (
            source_ref.clone(),
            ir_ref.clone(),
            frozen_report_ref.clone(),
        ),
    );

    // 同源 candidate/compile：冻结 payload（候选门冻结）与 compile 时重新
    // 投影的边逐条相等（report_digest 单源封口一致）。
    let frozen = source_store
        .get_mechanical_report(&scope, &frozen_report_ref)
        .expect("frozen report record")
        .structure_report
        .expect("Some frozen payload");
    let result = engine
        .handle_human_gate_termination(HumanGateCloseDecision::Approve)
        .await;
    assert_eq!(result, Ok(HumanGateCloseOutcome::Confirmed), "{result:?}");
    let recompile_context = engine
        .build_single_candidate_structure_context(&ir_record.ir, &source_ref, &ir_ref)
        .expect("compile-side context");
    let assessment =
        crate::product::workspace_engine::plan_projection::assess_single_candidate_structure(
            source.source.clone(),
            &ir_record.ir,
            &recompile_context,
        );
    let crate::product::workspace_engine::types::StructuralAssessment::Accepted(prepared) =
        assessment
    else {
        panic!("compile-side re-projection must stay accepted");
    };
    let recompiled =
        crate::product::workspace_engine::plan_projection::freeze_persisted_structure_report(
            &prepared.report,
            &frozen.source_revision_id,
            &frozen.ir_id,
        )
        .expect("recompile freeze");
    assert_eq!(
        recompiled.effective_edges, frozen.effective_edges,
        "candidate-gate and final-compile effective edges must be identical for the same source"
    );
    assert_eq!(recompiled.report_digest, frozen.report_digest);
    assert!(
        engine.last_human_gate_close_compile_failure().is_none(),
        "successful approval must not surface stale compile failure context"
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
        .handle_human_gate_termination(HumanGateCloseDecision::Approve)
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
        .handle_human_gate_termination(HumanGateCloseDecision::Approve)
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
        .handle_human_gate_termination(HumanGateCloseDecision::Approve)
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
        .handle_human_gate_termination(HumanGateCloseDecision::Abandon)
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
            .handle_human_gate_termination(HumanGateCloseDecision::Approve)
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
            .handle_human_gate_termination(HumanGateCloseDecision::Approve)
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
        candidate_recovery: None,
        findings: Vec::new(),
        repeated_fingerprints: Vec::new(),
        attempts_used: 0,
        manual_repairs_remaining: budget,
        trigger: crate::product::work_item_plan_policy::HumanReason::NativeHumanRequired,
        resumable: true,
        accepted_feedback_turns: None,
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
        .handle_human_gate_termination(HumanGateCloseDecision::Approve)
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
            .handle_human_gate_termination(HumanGateCloseDecision::Abandon)
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

/// F-42（复验）：确认链（SC 人门 approve → Final Compile → Confirmed）此前只收口
/// compile 节点，门节点 human_confirm 留在 Active——持久层不自洽。实测
/// workspace_session_0003：node_004 human_confirm 仍 active，而 005 compile /
/// 006 completed 均已 completed；重连投影按残留 Active 节点恢复门态，对话流门卡
/// 仍渲染可点的确认/终止按钮，二次点击被服务端矩阵拒
/// （INVALID_MESSAGE_FOR_STAGE: confirm not allowed in stage completed）。
#[tokio::test]
async fn conversational_gate_approve_completes_human_confirm_node_before_confirmed_terminal() {
    let _serial = crate::product::workspace_engine::single_candidate_compile_test_lock().await;
    let (_tmp, _lifecycle, mut engine) = approval_fixture();
    let (event_tx, _event_rx) = mpsc::channel(32);
    engine.event_tx = event_tx;
    // 门开：review 路由为 SC 计划落 HumanConfirm 门节点（routing.rs
    // ContinueToCompleted + SingleCandidate 非 AutoIfValid → enter_human_confirm），
    // approve 从该门出发。
    engine
        .enter_human_confirm(Some(
            "SingleCandidate 已通过 Evaluate，等待 Approval".to_string(),
        ))
        .await;
    let gate_node_id = engine
        .active_node_id
        .clone()
        .expect("human confirm node is active before approval");
    assert_eq!(
        engine.active_node_type(),
        Some(TimelineNodeType::HumanConfirm)
    );

    let result = engine
        .handle_human_gate_termination(HumanGateCloseDecision::Approve)
        .await;
    assert_eq!(result, Ok(HumanGateCloseOutcome::Confirmed), "{result:?}");

    let gate_node = engine
        .timeline_nodes
        .iter()
        .find(|node| node.node_id == gate_node_id)
        .expect("human confirm node");
    assert_eq!(
        gate_node.status,
        TimelineNodeStatus::Completed,
        "approve chain must close the human_confirm node instead of leaving it active"
    );
    assert!(
        gate_node.completed_at.is_some(),
        "closed gate node carries the chain's completion timestamp"
    );
}

// —— Task 1.1（REQ-REG-15 方案 A/D1）：SC Approve 静态资格预检矩阵 ——
// 覆盖 WaitingForHuman 开门上 phase∉{Approval,Evaluate} 或 snapshot 缺席的
// 全部组合：confirm 必须在带写 close CAS（及 Task 4 接线后的 trust 登记）之前
// 纯拒收——沿原 Conflict 既有翻译（与 store 层判据同文案），零 durable 副作用、
// 零 CAS 尝试；合法资格对照仍走原 CAS 链（store 判据一字不动）。

/// Task 1.1 预检矩阵夹具：在真实 SC 门 fixture 上按参数重写 durable 相位/快照
/// （approval_fixture 同款重写手法），内存会话同步；stage 随 WaitingForHuman→
/// HumanConfirm 映射保持门开态。会话 id 进程内唯一：CAS 尝试计数以 session id
/// 为全局键（同 plan_gate_terminate 纪律），避免并行测试互窜计数。
fn gate_with_static_eligibility_fixture(
    phase: Option<SingleCandidatePhase>,
    human_gate_snapshot: Option<crate::product::work_item_plan_policy::HumanGateSnapshot>,
) -> (
    tempfile::TempDir,
    LifecycleStore,
    WorkspaceEngine,
    mpsc::Receiver<EngineEvent>,
) {
    static STATIC_ELIGIBILITY_SESSION_SEQUENCE: std::sync::atomic::AtomicU64 =
        std::sync::atomic::AtomicU64::new(1);
    let (tmp, lifecycle, mut engine, event_rx) =
        super::conversational_gate::gate_fixture_with_event_rx(1);
    let old_session_id = engine.session().session_id.clone();
    let mut record = lifecycle
        .get_workspace_session(&old_session_id)
        .expect("session");
    record.id = format!(
        "workspace_session_t11_precheck_{}",
        STATIC_ELIGIBILITY_SESSION_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    record.single_candidate_phase = phase;
    record.human_gate_snapshot = human_gate_snapshot;
    let session_dir = lifecycle
        .app_paths()
        .issue_root(&record.project_id, &record.issue_id)
        .join("workspace-sessions");
    crate::product::json_store::write_json(
        &session_dir.join(format!("{}.json", record.id)),
        &record,
    )
    .expect("persist static eligibility session");
    std::fs::remove_file(session_dir.join(format!("{old_session_id}.json"))).ok();
    let artifact = engine.session.artifact.clone();
    engine.session = WorkspaceSession::from_record(record);
    engine.session.artifact = artifact;
    (tmp, lifecycle, engine, event_rx)
}

fn static_eligibility_gate_snapshot() -> crate::product::work_item_plan_policy::HumanGateSnapshot {
    crate::product::work_item_plan_policy::HumanGateSnapshot {
        candidate_recovery: None,
        findings: Vec::new(),
        repeated_fingerprints: Vec::new(),
        attempts_used: 0,
        manual_repairs_remaining: 1,
        trigger: crate::product::work_item_plan_policy::HumanReason::NativeHumanRequired,
        resumable: false,
        accepted_feedback_turns: None,
    }
}

#[tokio::test]
async fn conversational_gate_confirm_static_eligibility_matrix_rejects_before_close_cas() {
    for (phase, snapshot_present) in [
        (None, false),
        (None, true),
        (Some(SingleCandidatePhase::Prepare), false),
        (Some(SingleCandidatePhase::Prepare), true),
        (Some(SingleCandidatePhase::Generate), false),
        (Some(SingleCandidatePhase::Generate), true),
        (Some(SingleCandidatePhase::Failed), false),
        (Some(SingleCandidatePhase::Failed), true),
        (Some(SingleCandidatePhase::Approval), false),
        (Some(SingleCandidatePhase::Evaluate), false),
    ] {
        let label = format!("phase={phase:?}, snapshot={snapshot_present}");
        let snapshot = snapshot_present.then(static_eligibility_gate_snapshot);
        let (_root, lifecycle, mut engine, mut event_rx) =
            gate_with_static_eligibility_fixture(phase, snapshot);
        let session_id = engine.session().session_id.clone();
        let before = lifecycle
            .get_workspace_session(&session_id)
            .expect("durable session before confirm");
        assert_eq!(
            before.status,
            WorkspaceSessionStatus::WaitingForHuman,
            "夹具必须停在开态门（{label}）"
        );
        let cas_attempts_before = close_cas_attempts(&session_id);

        let error = engine
            .handle_human_gate_termination(HumanGateCloseDecision::Approve)
            .await
            .expect_err(&format!(
                "无效静态资格（{label}）confirm 必须在 close CAS 前拒收"
            ));
        assert!(error.contains("product_store_conflict"), "{label}: {error}");
        assert!(error.contains("human_gate_close"), "{label}: {error}");

        let after = lifecycle
            .get_workspace_session(&session_id)
            .expect("durable session after rejected confirm");
        assert_eq!(after, before, "拒收零副作用：durable 全字段不变（{label}）");
        assert_eq!(
            close_cas_attempts(&session_id),
            cas_attempts_before,
            "静态资格预检拒收必须先于任何带写 close CAS（{label}）"
        );
        assert!(
            event_rx.try_recv().is_err(),
            "拒收不产生任何引擎事件（{label}）"
        );
    }
}

/// 对照（Task 1.1）：合法静态资格（Approval 相位 × 快照在场）不被预检拦截——
/// close CAS 尝试计数恰 +1，仍走原 CAS 关门链。本 fixture 未铺 accepted
/// contract drafts，approve 链在 CAS 胜出后于 compile 段失败（与既有
/// does_not_confirm_before_compile_success 同语义），不重复断言终态。
#[tokio::test]
async fn conversational_gate_confirm_valid_static_eligibility_still_reaches_close_cas() {
    let (_root, _lifecycle, mut engine, _event_rx) = gate_with_static_eligibility_fixture(
        Some(SingleCandidatePhase::Approval),
        Some(static_eligibility_gate_snapshot()),
    );
    let session_id = engine.session().session_id.clone();
    let cas_attempts_before = close_cas_attempts(&session_id);

    let error = engine
        .handle_human_gate_termination(HumanGateCloseDecision::Approve)
        .await
        .expect_err("合法资格 fixture 的 compile 链未铺齐，approve 以 compile 失败收场");
    assert!(error.contains("compile failed"), "{error}");
    assert_eq!(
        close_cas_attempts(&session_id),
        cas_attempts_before + 1,
        "合法静态资格仍走原带写 close CAS"
    );
}

// —— Task 4.1/4.2（REQ-REG-15 方案 A/D1/D2/D4）：确认关门前 trust 按需准备 ——
// 覆盖：missing→Ready 才继续；disabled/provisional/Claude-only/非 LC 零调用；
// 冻结身份漂移零 home 不编造；Running/反馈/Abandon 不登记；部分提交保守
// 复用、原 CAS 竞争唯一胜者、取消 caller 不推进 CAS、Waiting 前后 durable
// 全不变量。

mod confirmation_trust {
    use super::*;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::checkpoint_store::CheckpointStore;
    use crate::product::issue_store::{CreateProductIssueInput, IssueStore};
    use crate::product::lifecycle_store::{
        CreateWorkspaceSessionInput, LifecycleStore, WorkItemPlanSessionOptions,
    };
    use crate::product::logical_codebase::provider_trust::ProviderTrustScenario;
    use crate::product::logical_codebase::{
        AggregatePolicyArtifactStore, CodexTrustAdapter, HomeBackedProviderTrustRegistry,
        KimiTrustAdapter, LogicalCodebaseManifest, LogicalCodebaseProviderGateway,
        LogicalCodebaseStore, PolicyTarget, PolicyTargetResolver, ProviderCapabilitySource,
        ProviderGatewayError, ProviderRef, ProviderTrustHomeAdapter, ProviderTrustPrecondition,
        ProviderTrustPreparationResult, ProviderTrustWaiting, SessionLaunchRequest,
        SessionPolicyAction, requires_workspace_trust,
    };
    use crate::product::models::{
        ProviderName, SingleCandidatePhase, WorkspaceSessionStatus, WorkspaceType,
    };
    use crate::product::work_item_plan_policy::{RunPolicy, WorkItemPlanFlowKind};
    use crate::product::workspace_engine::conversational_gate::{
        ConfirmationTrustContext, ConfirmationTrustWaiting,
    };
    use crate::product::workspace_engine::{
        EngineEvent, HumanGateCloseDecision, HumanGateCloseOutcome, HumanGateCommandOutcome,
        HumanGateFeedbackInput, WorkspaceConfirmOutcome,
    };
    use crate::protocol::contracts::{AdapterInput, AdapterOutput, TimeoutStatus};
    use std::future::Future;
    use std::path::PathBuf;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};

    // —— 最小 gateway 桩：确认链只把 gateway 用作「LC 会话」谓词，
    // 不调用 validate/spawn，桩实现全部 fail-closed（不可达）。 ——

    struct PresenceCapabilitySource;

    impl ProviderCapabilitySource for PresenceCapabilitySource {
        fn require_supported(
            &self,
            _provider: &ProviderRef,
            _action: SessionPolicyAction,
        ) -> Result<crate::product::logical_codebase::ProviderCapability, ProviderGatewayError>
        {
            Err(ProviderGatewayError::UnsupportedCapability(
                "presence-only fixture gateway".to_string(),
            ))
        }

        fn require_resume_supported(
            &self,
            _provider: &ProviderRef,
            _action: SessionPolicyAction,
        ) -> Result<crate::product::logical_codebase::ProviderCapability, ProviderGatewayError>
        {
            Err(ProviderGatewayError::UnsupportedCapability(
                "presence-only fixture gateway".to_string(),
            ))
        }

        fn require_write_boundary(
            &self,
            _provider: &ProviderRef,
            _action: SessionPolicyAction,
        ) -> Result<crate::product::logical_codebase::ProviderCapability, ProviderGatewayError>
        {
            Err(ProviderGatewayError::UnsupportedCapability(
                "presence-only fixture gateway".to_string(),
            ))
        }

        fn require_root_recipe_supported(
            &self,
            _provider: &ProviderRef,
            _credential: &crate::product::logical_codebase::provider_admission_preflight::BootstrapPhaseCredential,
        ) -> Result<crate::product::logical_codebase::ProviderCapability, ProviderGatewayError>
        {
            Err(ProviderGatewayError::UnsupportedCapability(
                "presence-only fixture gateway".to_string(),
            ))
        }
    }

    struct PresenceTargetResolver;

    impl PolicyTargetResolver for PresenceTargetResolver {
        fn resolve_and_revalidate(
            &self,
            _request: &SessionLaunchRequest,
        ) -> Result<PolicyTarget, ProviderGatewayError> {
            Err(ProviderGatewayError::UnsupportedCapability(
                "presence-only fixture gateway".to_string(),
            ))
        }
    }

    struct PresenceSyncAdapter;

    impl crate::cross_cutting::provider_adapter::ProviderAdapter for PresenceSyncAdapter {
        fn run(
            &self,
            _input: &AdapterInput,
        ) -> Result<AdapterOutput, crate::cross_cutting::provider_adapter::ProviderAdapterError>
        {
            Ok(AdapterOutput {
                exit_code: Some(0),
                stdout: "ok".to_string(),
                stderr: String::new(),
                structured_output: None,
                files_modified: Vec::new(),
                duration_ms: 0,
                timeout_status: TimeoutStatus::NotTimedOut,
            })
        }
    }

    fn presence_availability_gate()
    -> Arc<crate::cross_cutting::provider_availability_gate::ProviderAvailabilityGate> {
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
        Arc::new(
            crate::cross_cutting::provider_availability_gate::ProviderAvailabilityGate::new(
                Arc::new(AlwaysHealthy(snapshot)),
            ),
        )
    }

    fn presence_gateway(authority_root: PathBuf) -> Arc<LogicalCodebaseProviderGateway> {
        let paths = ProductAppPaths::new(std::env::temp_dir().join("aria-t4-presence-gateway"));
        Arc::new(LogicalCodebaseProviderGateway::new(
            AggregatePolicyArtifactStore::new(paths),
            Arc::new(PresenceCapabilitySource),
            Arc::new(PresenceTargetResolver),
            Arc::new(crate::cross_cutting::provider_registry::ProviderRegistry::new()),
            Arc::new(PresenceSyncAdapter),
            presence_availability_gate(),
            authority_root,
        ))
    }

    // —— 记数/剧本 fake precondition ——

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct RecordedTrustCall {
        project_id: String,
        operation_id: String,
        lc_id: String,
        canonical_root: PathBuf,
        providers: Vec<ProviderName>,
        scenario: ProviderTrustScenario,
    }

    #[derive(Clone, Default)]
    struct RecordingTrustGate {
        calls: Arc<Mutex<Vec<RecordedTrustCall>>>,
        scripted: Arc<Mutex<std::collections::VecDeque<ProviderTrustPreparationResult>>>,
        hook: Arc<Mutex<Option<Arc<dyn Fn() + Send + Sync>>>>,
    }

    impl RecordingTrustGate {
        fn calls(&self) -> Vec<RecordedTrustCall> {
            self.calls.lock().unwrap().clone()
        }

        fn script(&self, result: ProviderTrustPreparationResult) {
            self.scripted.lock().unwrap().push_back(result);
        }

        fn script_waiting(&self, reason_code: &str) {
            self.script(waiting_result(reason_code, ProviderName::Codex));
        }

        fn set_hook(&self, hook: Arc<dyn Fn() + Send + Sync>) {
            *self.hook.lock().unwrap() = Some(hook);
        }
    }

    fn waiting_result(reason_code: &str, provider: ProviderName) -> ProviderTrustPreparationResult {
        ProviderTrustPreparationResult::Waiting {
            waiting: ProviderTrustWaiting {
                provider,
                canonical_root: PathBuf::from("/fixture-root"),
                trust_key: "fixture-trust-key".to_string(),
                reason_code: reason_code.to_string(),
                message: "fixture 等待".to_string(),
                retry_action: "修复后重新确认".to_string(),
                manual_guidance: "fixture 指导".to_string(),
                command_markdown: None,
                recorded_at: "2026-10-11T00:00:00Z".to_string(),
            },
        }
    }

    impl ProviderTrustPrecondition for RecordingTrustGate {
        fn ensure_ready(
            &self,
            project_id: &str,
            operation_id: &str,
            lc_id: &str,
            canonical_root: &std::path::Path,
            providers: &[ProviderName],
            scenario: ProviderTrustScenario,
        ) -> ProviderTrustPreparationResult {
            if let Some(hook) = self.hook.lock().unwrap().clone() {
                hook();
            }
            self.calls.lock().unwrap().push(RecordedTrustCall {
                project_id: project_id.to_string(),
                operation_id: operation_id.to_string(),
                lc_id: lc_id.to_string(),
                canonical_root: canonical_root.to_path_buf(),
                providers: providers.to_vec(),
                scenario,
            });
            self.scripted.lock().unwrap().pop_front().unwrap_or(
                ProviderTrustPreparationResult::Ready {
                    registrations: Vec::new(),
                },
            )
        }
    }

    /// T4.2 取消边界 fake：`ensure_ready_cancellable` 模拟 3.2 的提交边界
    /// ——spawn_blocking 内「临界区开始→收尾」，caller 可在结果前被取消；
    /// detached 段独立完成。
    #[derive(Default)]
    struct DetachingTrustGate {
        section_started: Arc<AtomicBool>,
        section_done: Arc<AtomicBool>,
    }

    impl ProviderTrustPrecondition for DetachingTrustGate {
        fn ensure_ready(
            &self,
            _project_id: &str,
            _operation_id: &str,
            _lc_id: &str,
            _canonical_root: &std::path::Path,
            _providers: &[ProviderName],
            _scenario: ProviderTrustScenario,
        ) -> ProviderTrustPreparationResult {
            ProviderTrustPreparationResult::Ready {
                registrations: Vec::new(),
            }
        }

        fn ensure_ready_cancellable(
            self: Arc<Self>,
            _project_id: String,
            _operation_id: String,
            _lc_id: String,
            _canonical_root: PathBuf,
            _providers: Vec<ProviderName>,
            _scenario: ProviderTrustScenario,
        ) -> Pin<Box<dyn Future<Output = ProviderTrustPreparationResult> + Send>> {
            let started = self.section_started.clone();
            let done = self.section_done.clone();
            let (tx, rx) = tokio::sync::oneshot::channel::<()>();
            tokio::task::spawn_blocking(move || {
                started.store(true, Ordering::SeqCst); // home 锁已获取=提交边界
                let _ = tx.send(());
                std::thread::sleep(std::time::Duration::from_millis(50)); // 临界区收尾
                done.store(true, Ordering::SeqCst); // detached 安全收尾、不误删
            });
            Box::pin(async move {
                let _ = rx.await; // caller 取消点：drop 即停止等待
                ProviderTrustPreparationResult::Ready {
                    registrations: Vec::new(),
                }
            })
        }
    }

    // —— 夹具 ——

    static CONFIRM_TRUST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

    struct ConfirmationGateFacts {
        app_paths: ProductAppPaths,
        lc_id: String,
        canonical_root: PathBuf,
        issue_id: String,
        session_id: String,
    }

    /// LC 会话 SC/非 SC 共用夹具：真实 issue（原始 LC 归属）+ LC scope manifest
    /// （root=冻结根）+ 开态门 durable 记录 + presence gateway + 确认上下文。
    /// `single_candidate=false` 时构造 Legacy 流非 SC 门（无相位/快照要求）。
    #[allow(clippy::too_many_arguments)]
    fn confirmation_gate_fixture(
        single_candidate: bool,
        author: ProviderName,
        reviewer: Option<ProviderName>,
        provisional_reviewer: Option<ProviderName>,
        trust: Arc<dyn ProviderTrustPrecondition>,
    ) -> (
        tempfile::TempDir,
        LifecycleStore,
        WorkspaceEngine,
        mpsc::Receiver<EngineEvent>,
        ConfirmationGateFacts,
    ) {
        let seq = CONFIRM_TRUST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let root = tempfile::tempdir().expect("tempdir");
        let app_paths = ProductAppPaths::new(root.path().join(".aria"));
        let worktree = root.path().join("aggregate-root");
        std::fs::create_dir_all(&worktree).expect("create aggregate root");
        let canonical_root = std::fs::canonicalize(&worktree).expect("canonical aggregate root");
        let lc_id = format!("lc_confirm_{seq}");
        let issue = IssueStore::new(app_paths.clone())
            .create(CreateProductIssueInput {
                project_id: "project_0001".to_string(),
                repo_id: None,
                logical_codebase_id: Some(lc_id.clone()),
                base_branch: None,
                title: "confirm trust fixture".to_string(),
                description: None,
                change_id: None,
            })
            .expect("create LC issue");
        LogicalCodebaseStore::for_lc(app_paths.clone(), lc_id.clone())
            .save_manifest(
                "project_0001",
                &LogicalCodebaseManifest::new("project_0001", canonical_root.clone(), Vec::new()),
            )
            .expect("save LC manifest");
        let lifecycle = LifecycleStore::new(app_paths.clone());
        let created = lifecycle
            .create_workspace_session(CreateWorkspaceSessionInput {
                project_id: "project_0001".to_string(),
                issue_id: issue.id.clone(),
                entity_id: format!("plan_{seq}"),
                workspace_type: WorkspaceType::WorkItemPlan,
                author_provider: author,
                reviewer_provider: reviewer,
                review_rounds: 0,
                superpowers_enabled: false,
                openspec_enabled: false,
                work_item_plan_options: (single_candidate).then(|| WorkItemPlanSessionOptions {
                    flow_kind: WorkItemPlanFlowKind::SingleCandidate,
                    run_policy: RunPolicy::Interactive,
                    rollout_snapshot: true,
                }),
            })
            .expect("create workspace session");
        // T1 fixture 同款重写：进程内唯一 session id（CAS 计数不互窜）+ 开态门。
        let session_id = format!("workspace_session_t4_confirm_{seq}");
        let mut record = lifecycle
            .get_workspace_session(&created.id)
            .expect("load created session");
        let old_id = record.id.clone();
        record.id = session_id.clone();
        record.status = WorkspaceSessionStatus::WaitingForHuman;
        record.provisional_reviewer_provider = provisional_reviewer;
        if single_candidate {
            record.single_candidate_phase = Some(SingleCandidatePhase::Approval);
            record.human_gate_snapshot = Some(super::static_eligibility_gate_snapshot());
        }
        let session_dir = app_paths
            .issue_root(&record.project_id, &record.issue_id)
            .join("workspace-sessions");
        crate::product::json_store::write_json(
            &session_dir.join(format!("{}.json", record.id)),
            &record,
        )
        .expect("persist confirmation fixture session");
        std::fs::remove_file(session_dir.join(format!("{old_id}.json"))).ok();
        let (event_tx, event_rx) = mpsc::channel(8);
        let mut session = WorkspaceSession::from_record(record);
        session.stage = WorkspaceStage::HumanConfirm;
        session.session_status = WorkspaceSessionStatus::WaitingForHuman;
        session.artifact = Some(crate::web::workspace_ws_types::ArtifactPayload::Markdown {
            markdown: "# Work Item Plan\n".to_string(),
            diff: None,
        });
        let engine = WorkspaceEngine::new_persistent(
            Arc::new(CheckpointStore::new(root.path().join("checkpoints"))),
            lifecycle.clone(),
            event_tx,
            session,
        )
        .with_logical_provider_gateway(presence_gateway(canonical_root.clone()))
        .with_confirmation_trust_context(ConfirmationTrustContext {
            precondition: trust,
            issue_lc_id: Some(lc_id.clone()),
            frozen_root: canonical_root.clone(),
        });
        let facts = ConfirmationGateFacts {
            app_paths,
            lc_id,
            canonical_root,
            issue_id: issue.id,
            session_id,
        };
        (root, lifecycle, engine, event_rx, facts)
    }

    fn sc_confirm_fixture(
        author: ProviderName,
        reviewer: Option<ProviderName>,
        trust: Arc<dyn ProviderTrustPrecondition>,
    ) -> (
        tempfile::TempDir,
        LifecycleStore,
        WorkspaceEngine,
        mpsc::Receiver<EngineEvent>,
        ConfirmationGateFacts,
    ) {
        confirmation_gate_fixture(true, author, reviewer, None, trust)
    }

    fn durable_record(
        lifecycle: &LifecycleStore,
        facts: &ConfirmationGateFacts,
    ) -> crate::product::models::WorkspaceSessionRecord {
        lifecycle
            .get_workspace_session(&facts.session_id)
            .expect("durable session")
    }

    fn assert_gate_untouched_by_waiting(
        lifecycle: &LifecycleStore,
        facts: &ConfirmationGateFacts,
        before: &crate::product::models::WorkspaceSessionRecord,
        event_rx: &mut mpsc::Receiver<EngineEvent>,
    ) {
        let after = durable_record(lifecycle, facts);
        assert_eq!(
            after, *before,
            "Waiting 前后 durable 全不变量：session/status/phase/snapshot、节点、预算、\
             turn（human_gate_reservation）、compile reservation、provider-start ledger 完全不变"
        );
        let timeline_after = lifecycle
            .load_timeline_nodes_for_issue_session(
                "project_0001",
                &facts.issue_id,
                &facts.session_id,
            )
            .expect("timeline after");
        assert!(
            timeline_after.is_empty(),
            "Waiting 不产生任何时间线节点（零 compile/child/provider/close/abort）"
        );
        assert!(
            event_rx.try_recv().is_err(),
            "Waiting 不产生任何引擎事件（含 HumanGateClosed/abort）"
        );
    }

    // —— 4.1：missing→Ready 才继续；未 Ready 零推进 ——

    #[tokio::test]
    async fn sc_confirm_waiting_trust_keeps_gate_open_and_skips_close_cas() {
        let trust = Arc::new(RecordingTrustGate::default());
        trust.script_waiting("fixture_home_failure");
        let (_root, lifecycle, mut engine, mut event_rx, facts) =
            sc_confirm_fixture(ProviderName::Codex, None, trust.clone());
        let before = durable_record(&lifecycle, &facts);
        let cas_attempts_before = close_cas_attempts(&facts.session_id);

        let outcome = engine
            .handle_human_gate_termination(HumanGateCloseDecision::Approve)
            .await
            .expect("approve resolves into a typed waiting");
        match outcome {
            HumanGateCloseOutcome::TrustWaiting { waiting } => match waiting {
                ConfirmationTrustWaiting::Home {
                    session_id,
                    project_id,
                    lc_id,
                    waiting: inner,
                } => {
                    assert_eq!(session_id, facts.session_id);
                    assert_eq!(project_id, "project_0001");
                    assert_eq!(lc_id, facts.lc_id);
                    assert_eq!(inner.reason_code, "fixture_home_failure");
                    assert!(inner.retry_action.contains("重新确认"));
                }
                other => panic!("expected Home waiting, got {other:?}"),
            },
            other => panic!("expected TrustWaiting, got {other:?}"),
        }

        assert_eq!(trust.calls().len(), 1, "missing 才进入 ensure（恰一次）");
        assert_eq!(
            close_cas_attempts(&facts.session_id),
            cas_attempts_before,
            "准备失败不推进原 close CAS"
        );
        assert_gate_untouched_by_waiting(&lifecycle, &facts, &before, &mut event_rx);
    }

    #[tokio::test]
    async fn sc_confirm_ready_trust_proceeds_to_original_close_cas() {
        let trust = Arc::new(RecordingTrustGate::default());
        let (_root, _lifecycle, mut engine, _event_rx, facts) = sc_confirm_fixture(
            ProviderName::Codex,
            Some(ProviderName::KimiCode),
            trust.clone(),
        );
        let cas_attempts_before = close_cas_attempts(&facts.session_id);

        // fixture 未铺 accepted contract drafts：Ready 后 approve 沿原链在
        // compile 段失败（与 T1 合法资格对照同语义），只断言原 CAS 被推进。
        let error = engine
            .handle_human_gate_termination(HumanGateCloseDecision::Approve)
            .await
            .expect_err("Ready 后沿原链推进，compile 段按 fixture 缺口失败");
        assert!(error.contains("compile failed"), "{error}");
        assert_eq!(
            close_cas_attempts(&facts.session_id),
            cas_attempts_before + 1,
            "Ready 才继续：原带写 close CAS 恰推进一次"
        );
        let calls = trust.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0],
            RecordedTrustCall {
                project_id: "project_0001".to_string(),
                operation_id: format!("workspace_confirm:{}", facts.session_id),
                lc_id: facts.lc_id.clone(),
                canonical_root: facts.canonical_root.clone(),
                providers: vec![ProviderName::Codex, ProviderName::KimiCode],
                scenario: ProviderTrustScenario::Confirmation,
            },
            "调用参数：SC durable expected 的 author/effective reviewer、冻结根、\
             registry LC 标签与 workspace_confirm 关联 operation_id、Confirmation 场景"
        );
    }

    // —— 4.1：有效 provider 集与免登记会话 ——

    #[tokio::test]
    async fn sc_confirm_effective_provider_matrix_dedup_and_zero_call_exemptions() {
        struct Case {
            label: &'static str,
            author: ProviderName,
            reviewer: Option<ProviderName>,
            provisional: Option<ProviderName>,
            expected_providers: Vec<ProviderName>,
        }
        let cases = [
            Case {
                label: "disabled reviewer（reviewer=None + provisional=Codex）→ 仅 author",
                author: ProviderName::Codex,
                reviewer: None,
                provisional: Some(ProviderName::Codex),
                expected_providers: vec![ProviderName::Codex],
            },
            Case {
                label: "同 provider 去重",
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::Codex),
                provisional: None,
                expected_providers: vec![ProviderName::Codex],
            },
            Case {
                label: "author+reviewer 双 trust provider",
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::KimiCode),
                provisional: None,
                expected_providers: vec![ProviderName::Codex, ProviderName::KimiCode],
            },
        ];
        for case in cases {
            let trust = Arc::new(RecordingTrustGate::default());
            let (_root, _lifecycle, mut engine, _event_rx, facts) = confirmation_gate_fixture(
                true,
                case.author,
                case.reviewer,
                case.provisional,
                trust.clone(),
            );
            let cas_before = close_cas_attempts(&facts.session_id);
            let error = engine
                .handle_human_gate_termination(HumanGateCloseDecision::Approve)
                .await
                .expect_err("矩阵用例均走原链（compile 段失败）");
            assert!(error.contains("compile failed"), "{}: {error}", case.label);
            let calls = trust.calls();
            assert_eq!(calls.len(), 1, "{}：恰一次 ensure 调用", case.label);
            assert_eq!(
                calls[0].providers, case.expected_providers,
                "{}",
                case.label
            );
            assert_eq!(
                close_cas_attempts(&facts.session_id),
                cas_before + 1,
                "{}：Ready 继续原 CAS",
                case.label
            );
        }

        // 零调用豁免：Claude-only / Fake / 非 LC 会话。
        for (label, author, reviewer) in [
            (
                "Claude-only author+reviewer",
                ProviderName::ClaudeCode,
                Some(ProviderName::ClaudeCode),
            ),
            ("Fake provider", ProviderName::Fake, None),
        ] {
            let trust = Arc::new(RecordingTrustGate::default());
            let (_root, _lifecycle, mut engine, _event_rx, facts) =
                sc_confirm_fixture(author, reviewer, trust.clone());
            let cas_before = close_cas_attempts(&facts.session_id);
            let error = engine
                .handle_human_gate_termination(HumanGateCloseDecision::Approve)
                .await
                .expect_err("无需 trust 时沿原链（compile 段失败）");
            assert!(error.contains("compile failed"), "{label}: {error}");
            assert!(
                trust.calls().is_empty(),
                "{label}：零 ensure/home/ownership/audit 调用"
            );
            assert_eq!(close_cas_attempts(&facts.session_id), cas_before + 1);
        }

        // 非 LC 会话（无 gateway）：即使误注入上下文也免登记。
        let trust = Arc::new(RecordingTrustGate::default());
        let (_root, _lifecycle, mut engine, _event_rx, facts) =
            sc_confirm_fixture(ProviderName::Codex, None, trust.clone());
        engine.logical_provider_gateway = None;
        let cas_before = close_cas_attempts(&facts.session_id);
        let error = engine
            .handle_human_gate_termination(HumanGateCloseDecision::Approve)
            .await
            .expect_err("非 LC 会话沿原链（compile 段失败）");
        assert!(error.contains("compile failed"), "{error}");
        assert!(
            trust.calls().is_empty(),
            "非 LC 会话零调用（不扩大到无 LC gateway 的单仓会话）"
        );
        assert_eq!(close_cas_attempts(&facts.session_id), cas_before + 1);
    }

    // —— 4.1：冻结身份缺失/漂移等待（零 home 调用、只返回已知身份） ——

    #[tokio::test]
    async fn sc_confirm_context_drift_matrix_stops_before_home_calls() {
        struct Expectation {
            reason_code: &'static str,
            /// 上下文是否在场（在场=返回已知冻结身份；缺失=身份为 None）。
            context_present: bool,
        }
        // 各漂移用例独立夹具 + 事后破坏。
        for (label, mutate, expectation) in [
            (
                "LC 会话缺确认上下文",
                "context",
                Expectation {
                    reason_code: "confirmation_trust_context_unavailable",
                    context_present: false,
                },
            ),
            (
                "issue 原始 LC 归属已改变",
                "scope",
                Expectation {
                    reason_code: "confirmation_trust_scope_changed",
                    context_present: true,
                },
            ),
            (
                "冻结 root 不能 canonicalize",
                "root-deleted",
                Expectation {
                    reason_code: "confirmation_trust_root_unavailable",
                    context_present: true,
                },
            ),
            (
                "同 scope manifest root 与冻结根不一致",
                "manifest-root",
                Expectation {
                    reason_code: "confirmation_trust_root_changed",
                    context_present: true,
                },
            ),
            (
                "同 scope manifest 缺失",
                "manifest-deleted",
                Expectation {
                    reason_code: "confirmation_trust_root_changed",
                    context_present: true,
                },
            ),
        ] {
            let trust = Arc::new(RecordingTrustGate::default());
            let (root, lifecycle, mut engine, mut event_rx, facts) =
                sc_confirm_fixture(ProviderName::Codex, None, trust.clone());
            match mutate {
                "context" => {
                    engine.confirmation_trust_context = None;
                }
                "scope" => {
                    let mut issue = IssueStore::new(facts.app_paths.clone())
                        .get("project_0001", &facts.issue_id)
                        .expect("issue");
                    issue.logical_codebase_id = Some("lc_drifted_elsewhere".to_string());
                    crate::product::json_store::write_json(
                        &facts
                            .app_paths
                            .issue_root("project_0001", &facts.issue_id)
                            .join("issue.json"),
                        &issue,
                    )
                    .expect("rewrite issue LC ownership");
                }
                "root-deleted" => {
                    std::fs::remove_dir_all(&facts.canonical_root).expect("delete frozen root");
                }
                "manifest-root" => {
                    let other_root = root.path().join("other-root");
                    std::fs::create_dir_all(&other_root).expect("create other root");
                    // 保持 manifest 身份不变（save_manifest 校验 id 稳定），
                    // 仅漂移 provider_context_root。
                    let mut manifest =
                        LogicalCodebaseStore::for_lc(facts.app_paths.clone(), facts.lc_id.clone())
                            .load_manifest("project_0001")
                            .expect("load manifest")
                            .expect("manifest present");
                    manifest.provider_context_root =
                        std::fs::canonicalize(&other_root).expect("canonical other");
                    LogicalCodebaseStore::for_lc(facts.app_paths.clone(), facts.lc_id.clone())
                        .save_manifest("project_0001", &manifest)
                        .expect("rewrite manifest root");
                }
                "manifest-deleted" => {
                    let scope_root = crate::product::logical_codebase::lc_scope_root(
                        &facts.app_paths,
                        "project_0001",
                        &Some(facts.lc_id.clone()),
                    )
                    .expect("scope root");
                    std::fs::remove_file(scope_root.join("manifest.json"))
                        .expect("delete manifest");
                }
                other => unreachable!("unknown drift mutation {other}"),
            }
            let before = durable_record(&lifecycle, &facts);
            let cas_before = close_cas_attempts(&facts.session_id);

            let outcome = engine
                .handle_human_gate_termination(HumanGateCloseDecision::Approve)
                .await
                .expect("漂移确认解析为 typed 停等");
            match outcome {
                HumanGateCloseOutcome::TrustWaiting { waiting } => match waiting {
                    ConfirmationTrustWaiting::Context {
                        session_id,
                        issue_lc_id,
                        frozen_root,
                        reason_code,
                        ..
                    } => {
                        assert_eq!(session_id, facts.session_id, "{label}");
                        assert_eq!(reason_code, expectation.reason_code, "{label}");
                        let expected_lc = if expectation.context_present {
                            Some(facts.lc_id.clone())
                        } else {
                            None
                        };
                        assert_eq!(
                            issue_lc_id, expected_lc,
                            "{label}：只返回已知冻结身份，不编造"
                        );
                        let expected_root = if expectation.context_present {
                            Some(facts.canonical_root.clone())
                        } else {
                            None
                        };
                        assert_eq!(frozen_root, expected_root, "{label}");
                    }
                    other => panic!("{label}：expected Context waiting, got {other:?}"),
                },
                other => panic!("{label}：expected TrustWaiting, got {other:?}"),
            }
            assert!(
                trust.calls().is_empty(),
                "{label}：漂移零 ensure/home/ownership/audit 调用"
            );
            assert_eq!(
                close_cas_attempts(&facts.session_id),
                cas_before,
                "{label}：漂移不推进 close CAS"
            );
            assert_gate_untouched_by_waiting(&lifecycle, &facts, &before, &mut event_rx);
        }
    }

    // —— 4.1：Running/反馈/Abandon 不登记 ——

    #[tokio::test]
    async fn sc_confirm_feedback_abandon_and_running_confirm_do_not_register() {
        let trust = Arc::new(RecordingTrustGate::default());
        let (_root, lifecycle, mut engine, mut event_rx, facts) =
            sc_confirm_fixture(ProviderName::Codex, None, trust.clone());

        // 反馈：开修订 turn，不登记。
        let feedback = engine
            .handle_human_gate_feedback(HumanGateFeedbackInput {
                command_id: "cmd_t4_feedback".to_string(),
                feedback: "只修正这个字段".to_string(),
            })
            .await
            .expect("feedback opens a revision turn");
        assert!(
            matches!(feedback, HumanGateCommandOutcome::TurnOpened { .. }),
            "fixture 反馈须开 turn（{feedback:?}）"
        );
        assert!(trust.calls().is_empty(), "反馈不登记");

        // 该 turn 关闭后（此处直接换新夹具段测 Abandon/Running——同一夹具
        // 上 Busy 会挡 close，故重开一个干净夹具逐段断言）。
        let trust2 = Arc::new(RecordingTrustGate::default());
        let (_root2, _lifecycle2, mut engine2, _event_rx2, _facts2) =
            sc_confirm_fixture(ProviderName::Codex, None, trust2.clone());
        let outcome = engine2
            .handle_human_gate_termination(HumanGateCloseDecision::Abandon)
            .await
            .expect("abandon closes the gate");
        assert_eq!(outcome, HumanGateCloseOutcome::Abandoned);
        assert!(trust2.calls().is_empty(), "Abandon 不登记（脱困不受影响）");

        // Running 普通 confirm：不登记。
        let trust3 = Arc::new(RecordingTrustGate::default());
        let (_root3, _lifecycle3, mut engine3, _event_rx3, _facts3) =
            sc_confirm_fixture(ProviderName::Codex, None, trust3.clone());
        engine3.session.stage = WorkspaceStage::Running;
        let outcome = engine3.handle_confirm().await.expect("running confirm ok");
        assert!(
            matches!(outcome, WorkspaceConfirmOutcome::None),
            "Running 臂保持既有推进语义（{outcome:?}）"
        );
        assert!(trust3.calls().is_empty(), "Running 普通 confirm 不登记");
        let _ = &mut event_rx;
        let _ = (lifecycle, facts);
    }

    // —— 4.1：非 SC（Legacy WorkItemPlan）HumanConfirm 臂共用准备 ——

    #[tokio::test]
    async fn non_sc_confirm_preparation_before_finalize() {
        // Waiting：typed 停等 + 零定稿副作用。
        let trust = Arc::new(RecordingTrustGate::default());
        trust.script_waiting("fixture_home_failure");
        let (_root, lifecycle, mut engine, mut event_rx, facts) =
            confirmation_gate_fixture(false, ProviderName::Codex, None, None, trust.clone());
        let before = durable_record(&lifecycle, &facts);
        let outcome = engine
            .handle_confirm()
            .await
            .expect("non-SC confirm resolves into a typed waiting");
        match outcome {
            WorkspaceConfirmOutcome::TrustWaiting { waiting } => {
                assert_eq!(waiting.reason_code(), "fixture_home_failure");
            }
            other => panic!("expected TrustWaiting, got {other:?}"),
        }
        assert_eq!(
            trust.calls().len(),
            1,
            "非 SC mutex 内锁定事实触发恰一次 ensure"
        );
        assert_eq!(trust.calls()[0].providers, vec![ProviderName::Codex]);
        assert_gate_untouched_by_waiting(&lifecycle, &facts, &before, &mut event_rx);

        // Ready：沿原链推进（fixture 无 plan 记录，confirm_work_item_plan
        // 如实失败——非 TrustWaiting）。
        let trust2 = Arc::new(RecordingTrustGate::default());
        let (_root2, _lifecycle2, mut engine2, _event_rx2, facts2) =
            confirmation_gate_fixture(false, ProviderName::Codex, None, None, trust2.clone());
        let result = engine2.handle_confirm().await;
        assert!(
            result.is_err(),
            "fixture 缺 plan 记录，定稿段如实失败（{:?}）",
            result
        );
        assert_eq!(
            trust2.calls().len(),
            1,
            "Ready 才继续原定稿链（ensure 已发生）"
        );
        let _ = facts2;
    }

    // —— 4.2：真实 registry——部分提交保留 + 重试保守复用 ——

    fn real_registry_fixture(
        app_paths: &ProductAppPaths,
        home: &std::path::Path,
    ) -> Arc<HomeBackedProviderTrustRegistry> {
        Arc::new(HomeBackedProviderTrustRegistry::new(
            app_paths.clone(),
            vec![
                Arc::new(CodexTrustAdapter::for_home(home)),
                Arc::new(KimiTrustAdapter::for_home(home)),
            ],
        ))
    }

    #[tokio::test]
    async fn sc_confirm_partial_commit_keeps_written_provider_and_retry_replays() {
        let trust = Arc::new(RecordingTrustGate::default());
        let (_root, lifecycle, mut engine, _event_rx, facts) =
            sc_confirm_fixture(ProviderName::Codex, Some(ProviderName::KimiCode), trust);
        let home = tempfile::tempdir().expect("trust home");
        let registry = real_registry_fixture(&facts.app_paths, home.path());
        engine.confirmation_trust_context = Some(ConfirmationTrustContext {
            precondition: registry.clone(),
            issue_lc_id: Some(facts.lc_id.clone()),
            frozen_root: facts.canonical_root.clone(),
        });
        // 第二 provider 预置损坏的 Kimi 记录 → 首确认 Codex 已登记、Kimi 失败停等。
        let kimi = KimiTrustAdapter::for_home(home.path());
        let kimi_record = kimi.artifact_path(&facts.canonical_root);
        std::fs::create_dir_all(kimi_record.parent().expect("kimi parent"))
            .expect("create kimi dir");
        std::fs::write(&kimi_record, "{ not valid json").expect("break kimi record");

        let outcome = engine
            .handle_human_gate_termination(HumanGateCloseDecision::Approve)
            .await
            .expect("second provider failure resolves into typed waiting");
        match outcome {
            HumanGateCloseOutcome::TrustWaiting { waiting } => {
                assert_eq!(waiting.reason_code(), "kimi_trust_record_invalid");
            }
            other => panic!("expected TrustWaiting, got {other:?}"),
        }
        let audit = crate::product::logical_codebase::ProviderTrustStore::for_lc(
            facts.app_paths.clone(),
            facts.lc_id.clone(),
        )
        .list_audit("project_0001")
        .expect("audit");
        assert!(
            audit.iter().any(|record| record.action == "ensure"
                && record.outcome == "registered"
                && record.provider == "codex"
                && record.operation_id.as_deref()
                    == Some(format!("workspace_confirm:{}", facts.session_id).as_str())),
            "部分提交：已写 Codex 条目保留并关联 workspace_confirm 审计（audit={audit:?}）"
        );
        assert!(
            audit
                .iter()
                .any(|record| record.action == "gate_waiting" && record.provider == "kimi_code"),
            "失败侧留 gate_waiting 审计（audit={audit:?}）"
        );
        let codex_config =
            CodexTrustAdapter::for_home(home.path()).artifact_path(&facts.canonical_root);
        let codex_bytes_after_first = std::fs::read(&codex_config).expect("codex config written");

        // 修复 Kimi（移除损坏记录）后显式重新确认：Ready、Codex replay、Kimi 注册。
        std::fs::remove_file(&kimi_record).expect("fix kimi record");
        let cas_before = close_cas_attempts(&facts.session_id);
        let error = engine
            .handle_human_gate_termination(HumanGateCloseDecision::Approve)
            .await
            .expect_err("修复后 Ready，沿原链在 compile 段失败");
        assert!(error.contains("compile failed"), "{error}");
        assert_eq!(
            close_cas_attempts(&facts.session_id),
            cas_before + 1,
            "重试 Ready 才推进原 close CAS"
        );
        let audit = crate::product::logical_codebase::ProviderTrustStore::for_lc(
            facts.app_paths.clone(),
            facts.lc_id.clone(),
        )
        .list_audit("project_0001")
        .expect("audit after retry");
        assert!(
            audit.iter().any(|record| record.action == "ensure"
                && record.outcome == "replay"
                && record.provider == "codex"),
            "重试保守复用：Codex 走 ensure/replay（audit={audit:?}）"
        );
        assert!(
            audit.iter().any(|record| record.action == "ensure"
                && record.outcome == "registered"
                && record.provider == "kimi_code"),
            "Kimi 修复后注册（audit={audit:?}）"
        );
        let codex_bytes_after_retry = std::fs::read(&codex_config).expect("codex config kept");
        assert_eq!(
            codex_bytes_after_first, codex_bytes_after_retry,
            "replay 不改写 home 字节"
        );
        let _ = lifecycle;
    }

    #[tokio::test]
    async fn sc_confirm_trusted_replay_and_other_lc_reuse_zero_home_writes() {
        // 本 LC replay：同 scope 直接 seed 一次登记（owned），随后单次引擎
        // 确认复用同一 registry → ensure/replay、home 字节不变。
        let trust = Arc::new(RecordingTrustGate::default());
        let (_root, _lifecycle, mut engine, _event_rx, facts) =
            sc_confirm_fixture(ProviderName::Codex, None, trust);
        let home = tempfile::tempdir().expect("trust home");
        let registry = real_registry_fixture(&facts.app_paths, home.path());
        match registry.ensure_ready(
            "project_0001",
            "op_seed_trusted",
            &facts.lc_id,
            &facts.canonical_root,
            &[ProviderName::Codex],
            ProviderTrustScenario::Recipe,
        ) {
            ProviderTrustPreparationResult::Ready { .. } => {}
            other => panic!("seed must be Ready, got {other:?}"),
        }
        engine.confirmation_trust_context = Some(ConfirmationTrustContext {
            precondition: registry.clone(),
            issue_lc_id: Some(facts.lc_id.clone()),
            frozen_root: facts.canonical_root.clone(),
        });
        let codex_config =
            CodexTrustAdapter::for_home(home.path()).artifact_path(&facts.canonical_root);
        let bytes_after_seed = std::fs::read(&codex_config).expect("codex config");
        let error = engine
            .handle_human_gate_termination(HumanGateCloseDecision::Approve)
            .await
            .expect_err("复用确认沿原链在 compile 段失败（replay 已完成）");
        assert!(error.contains("compile failed"), "{error}");
        let audit = crate::product::logical_codebase::ProviderTrustStore::for_lc(
            facts.app_paths.clone(),
            facts.lc_id.clone(),
        )
        .list_audit("project_0001")
        .expect("audit");
        assert!(
            audit.iter().any(|record| record.action == "ensure"
                && record.outcome == "replay"
                && record.provider == "codex"
                && record.operation_id.as_deref()
                    == Some(format!("workspace_confirm:{}", facts.session_id).as_str())),
            "本 LC replay 记 ensure/replay 并关联确认 operation（audit={audit:?}）"
        );
        assert_eq!(
            std::fs::read(&codex_config).expect("codex config"),
            bytes_after_seed,
            "Trusted replay：home 写次数为 0（字节不变）"
        );

        // 其它 LC 已创建条目：首次复用走 reuse_user_entry、owned=false、字节不变。
        let trust2 = Arc::new(RecordingTrustGate::default());
        let (_root2, _lifecycle2, mut engine2, _event_rx2, facts2) =
            sc_confirm_fixture(ProviderName::Codex, None, trust2);
        let home2 = tempfile::tempdir().expect("trust home 2");
        let registry2 = real_registry_fixture(&facts2.app_paths, home2.path());
        let other_lc = format!("{}_other", facts2.lc_id);
        match registry2.ensure_ready(
            "project_0001",
            "op_other_lc_seed",
            &other_lc,
            &facts2.canonical_root,
            &[ProviderName::Codex],
            ProviderTrustScenario::Recipe,
        ) {
            ProviderTrustPreparationResult::Ready { .. } => {}
            other => panic!("other-LC seed must be Ready, got {other:?}"),
        }
        engine2.confirmation_trust_context = Some(ConfirmationTrustContext {
            precondition: registry2.clone(),
            issue_lc_id: Some(facts2.lc_id.clone()),
            frozen_root: facts2.canonical_root.clone(),
        });
        let codex_config2 =
            CodexTrustAdapter::for_home(home2.path()).artifact_path(&facts2.canonical_root);
        let bytes_before_reuse = std::fs::read(&codex_config2).expect("codex config 2");
        let error = engine2
            .handle_human_gate_termination(HumanGateCloseDecision::Approve)
            .await
            .expect_err("复用确认沿原链在 compile 段失败");
        assert!(error.contains("compile failed"), "{error}");
        let audit2 = crate::product::logical_codebase::ProviderTrustStore::for_lc(
            facts2.app_paths.clone(),
            facts2.lc_id.clone(),
        )
        .list_audit("project_0001")
        .expect("audit 2");
        assert!(
            audit2.iter().any(|record| record.action == "ensure"
                && record.outcome == "reuse_user_entry"
                && record.provider == "codex"),
            "首次复用其它 LC 条目记 ensure/reuse_user_entry（audit={audit2:?}）"
        );
        assert_eq!(
            std::fs::read(&codex_config2).expect("codex config 2"),
            bytes_before_reuse,
            "复用不写 home（字节不变）"
        );
    }

    // —— 4.2：合法 preparation 中途 expected 被改写 → 原 CAS 胜者唯一 ——

    #[tokio::test]
    async fn sc_confirm_ready_then_expected_rewritten_loses_cas_uniquely() {
        let trust = Arc::new(RecordingTrustGate::default());
        let (_root, lifecycle, mut engine, mut event_rx, facts) =
            sc_confirm_fixture(ProviderName::Codex, None, trust.clone());
        let cas_before = close_cas_attempts(&facts.session_id);
        // hook 在 ensure 内同步触发（=准备窗口内）：改写 durable expected，
        // 模拟另一 worker 已在准备窗口内赢得关门（status=Running）。
        let session_path = facts
            .app_paths
            .issue_root("project_0001", &facts.issue_id)
            .join("workspace-sessions")
            .join(format!("{}.json", facts.session_id));
        trust.set_hook({
            let lifecycle = lifecycle.clone();
            let session_id = facts.session_id.clone();
            Arc::new(move || {
                let mut record = lifecycle
                    .get_workspace_session(&session_id)
                    .expect("durable");
                record.status = WorkspaceSessionStatus::Running;
                crate::product::json_store::write_json(&session_path, &record)
                    .expect("rewrite durable expected mid-preparation");
            }) as Arc<dyn Fn() + Send + Sync>
        });

        let outcome = engine
            .handle_human_gate_termination(HumanGateCloseDecision::Approve)
            .await
            .expect("race resolves through existing translation");
        match &outcome {
            HumanGateCloseOutcome::AlreadyClosed { status } => {
                assert_eq!(*status, WorkspaceSessionStatus::Running);
            }
            other => panic!("expected AlreadyClosed via original CAS race, got {other:?}"),
        }
        assert_eq!(
            close_cas_attempts(&facts.session_id),
            cas_before + 1,
            "Ready 后仍以原 expected 走原 CAS（败者被 Conflict 翻译拒收）"
        );
        let mut closed_events = 0;
        while let Ok(event) = event_rx.try_recv() {
            if matches!(event, EngineEvent::HumanGateClosed { .. }) {
                closed_events += 1;
            }
        }
        assert_eq!(
            closed_events, 0,
            "CAS 败者不 compile、不发第二次 close、不 abort"
        );
        let durable = durable_record(&lifecycle, &facts);
        assert_ne!(durable.status, WorkspaceSessionStatus::Confirmed);
        assert!(
            durable.provider_start_ledger.is_empty(),
            "败者零 provider 启动"
        );
        let compile_txs = engine
            .work_item_plan_store()
            .map(|store| {
                store
                    .list_compile_transactions("project_0001", &facts.issue_id, &durable.entity_id)
                    .unwrap_or_default()
            })
            .unwrap_or_default();
        assert!(
            compile_txs.is_empty(),
            "败者不进入 compile（txs={compile_txs:?}）"
        );
    }

    // —— 4.2：取消 caller 不推进 CAS；detached 提交安全收尾 ——

    #[tokio::test]
    async fn sc_confirm_cancelled_caller_does_not_advance_cas_and_detached_section_finalizes() {
        let gate = Arc::new(DetachingTrustGate::default());
        let started = gate.section_started.clone();
        let done = gate.section_done.clone();
        let (_root, lifecycle, mut engine, _event_rx, facts) =
            sc_confirm_fixture(ProviderName::Codex, None, gate);
        let before = durable_record(&lifecycle, &facts);
        let cas_before = close_cas_attempts(&facts.session_id);

        let mut confirm =
            Box::pin(engine.handle_human_gate_termination(HumanGateCloseDecision::Approve));
        // 并发轮询 confirm future 与「临界区开始」信号：confirm 被执行到
        // ensure 的提交边界（spawn_blocking 已启动）后，caller 取消。
        let started_wait = async {
            while !started.load(Ordering::SeqCst) {
                tokio::task::yield_now().await;
            }
        };
        tokio::select! {
            result = &mut confirm => {
                panic!("confirm must not finish before cancellation (got {result:?})");
            }
            _ = started_wait => {}
        }
        // caller 取消（drop）：确认 future 终止。
        drop(confirm);
        assert_eq!(
            close_cas_attempts(&facts.session_id),
            cas_before,
            "取消的 caller 不推进原 close CAS"
        );
        assert_eq!(
            durable_record(&lifecycle, &facts),
            before,
            "取消零 durable 推进（门保持开态）"
        );
        // detached 临界区独立安全收尾（不误删、不复活业务）。
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while !done.load(Ordering::SeqCst) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("detached critical section must finalize safely");
        assert_eq!(
            close_cas_attempts(&facts.session_id),
            cas_before,
            "detached 收尾不推进另一门/CAS"
        );
        assert_eq!(
            durable_record(&lifecycle, &facts),
            before,
            "detached 收尾不改写 durable 门状态"
        );
    }
}
