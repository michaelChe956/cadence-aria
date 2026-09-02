//! Task 5.4：SingleCandidate 持久化恢复测试矩阵。
//!
//! 3.4 已覆盖 reservation 的四个原子边界；本模块只覆盖其后的完整恢复语义，
//! 并复用同一 durable JSON/transaction 夹具验证恢复时不重放已完成前缀。

use super::*;
use std::collections::BTreeSet;

use crate::cross_cutting::streaming_provider::ProviderCompletion;
use crate::product::json_store::write_json;
use crate::product::models::{
    SingleCandidatePhase, WorkItemPlanCommitState, WorkItemPlanCompileStatus, WorkItemSplitFinding,
    WorkspaceSessionStatus, WorkspaceType,
};
use crate::product::work_item_plan_compiler::{
    PlanCandidateIr, PlanCandidateItemIr, PlanCandidateMechanicalReport,
    WORK_ITEM_PLAN_COMPILER_VERSION,
};
use crate::product::work_item_plan_policy::{
    FindingClassHint, ReviewFindingCategory, RunPolicy, WorkItemPlanFlowKind,
};
use crate::product::work_item_plan_source_store::{
    PlanCandidateIrRecord, PlanCandidateMechanicalReportRecord, SourceRevisionRecord,
    SourceStoreScope, WorkItemPlanSourceStore,
};
use crate::product::work_item_revision_store::{
    InitialPlanPublicationCheckpoint, InitialPlanPublicationPhase,
};
use crate::web::workspace_ws_types::{
    ReviewFinding, ReviewFindingSeverity, ReviewGate, ReviewVerdict, ReviewVerdictType,
    WorkItemPlanCompileRecoveryActionDto,
};
use sha2::{Digest, Sha256};

/// failpoint key 由 persistent scope 构成；所有 SingleCandidate compile failpoint
/// 矩阵都必须复用模块级串行锁，避免跨测试文件注册同一 durable scope。
async fn single_candidate_recovery_failpoint_lock() -> tokio::sync::MutexGuard<'static, ()> {
    crate::product::workspace_engine::single_candidate_compile_test_lock().await
}

/// 以同一 persistent store 重建 engine，禁止测试通过内存 session 越过 durable 边界。
fn single_candidate_recovery_restart(
    engine: &WorkspaceEngine,
    lifecycle: &LifecycleStore,
) -> WorkspaceEngine {
    let record = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("restart 必须读取 durable session JSON");
    let (event_tx, _event_rx) = mpsc::channel(64);
    WorkspaceEngine::new_persistent(
        engine.checkpoint_store.clone(),
        lifecycle.clone(),
        event_tx,
        WorkspaceSession::from_record(record),
    )
}

fn single_candidate_recovery_events(
    lifecycle: &LifecycleStore,
    session: &WorkspaceSession,
) -> Vec<TimelineNode> {
    lifecycle
        .load_timeline_nodes_for_issue_session(
            &session.project_id,
            &session.issue_id,
            &session.session_id,
        )
        .expect("读取 durable events")
}

fn single_candidate_recovery_assert_event_prefix(
    initial_events: &[TimelineNode],
    current_events: &[TimelineNode],
) {
    assert!(
        current_events.len() >= initial_events.len(),
        "恢复不得删除 events 前缀"
    );
    assert_eq!(
        &current_events[..initial_events.len()],
        initial_events,
        "恢复不得改写已 durable 的 events 前缀"
    );
}

fn single_candidate_recovery_assert_provider_start_dedup(
    record: &WorkspaceSessionRecord,
    expected_key: &str,
    expected_started: usize,
) {
    let matching = record
        .provider_start_ledger
        .iter()
        .filter(|entry| entry.provider_start_idempotency_key == expected_key)
        .collect::<Vec<_>>();
    assert_eq!(
        matching.len(),
        1,
        "provider start idempotency key 必须只有一条 durable ledger"
    );
    assert_eq!(
        matching.iter().filter(|entry| entry.started).count(),
        expected_started,
        "同一 idempotency key 最多启动一次"
    );
}

fn single_candidate_recovery_assert_transaction_durable_fields(
    lifecycle: &LifecycleStore,
    tx: &WorkItemPlanCompileTransaction,
) {
    assert_eq!(tx.flow_kind, Some(WorkItemPlanFlowKind::SingleCandidate));
    let scope = SourceStoreScope {
        project_id: tx.project_id.clone(),
        issue_id: tx.issue_id.clone(),
        plan_id: tx.plan_id.clone(),
    };
    let source_store = WorkItemPlanSourceStore::new(lifecycle.app_paths());
    let source_ref = tx
        .source_revision_ref
        .as_deref()
        .expect("transaction durable source ref");
    let source = source_store
        .get_source_revision(&scope, source_ref)
        .expect("完整 source 内容必须可从 direct ref 重载");
    assert_eq!(tx.source_revision_id.as_deref(), Some(source.id.as_str()));
    let ir_ref = tx
        .plan_candidate_ir_ref
        .as_deref()
        .expect("transaction durable IR ref");
    source_store
        .get_plan_candidate_ir(&scope, ir_ref)
        .expect("完整 IR 内容必须可从 direct ref 重载");
    let report_ref = tx
        .mechanical_report_ref
        .as_deref()
        .expect("transaction durable report ref");
    source_store
        .get_mechanical_report(&scope, report_ref)
        .expect("完整 mechanical report 内容必须可从 direct ref 重载");
    let provenance_ref = tx
        .publication_provenance_ref
        .as_deref()
        .expect("transaction durable provenance ref");
    let provenance = source_store
        .get_publication_provenance(&scope, provenance_ref)
        .expect("完整 provenance 内容必须可从 direct ref 重载");
    assert_eq!(
        tx.publication_provenance_content_hash.as_deref(),
        Some(provenance.content_hash.as_str()),
        "transaction durable provenance hash"
    );
    assert_eq!(
        provenance.content_hash().expect("重算完整 provenance hash"),
        provenance.content_hash,
        "provenance content hash 必须覆盖完整内容"
    );
}

async fn single_candidate_recovery_mark_transaction_recovery(
    engine: &mut WorkspaceEngine,
    plan_id: &str,
    compile_id: &str,
    reason: &str,
) {
    let store = engine.work_item_plan_store().expect("plan store");
    let mut tx = store
        .get_compile_transaction("project_0001", "issue_0001", plan_id, compile_id)
        .expect("interrupted transaction");
    tx.status = WorkItemPlanCompileStatus::RecoveryRequired;
    tx.failure_reason = Some(reason.to_string());
    store
        .put_compile_transaction(&tx)
        .expect("durably mark compile recovery required");
    engine
        .enter_work_item_plan_compile_recovery(Some(reason.to_string()))
        .await;
}

pub(crate) fn single_candidate_recovery_record(
    lifecycle: &LifecycleStore,
    engine: &mut WorkspaceEngine,
    phase: SingleCandidatePhase,
    policy: RunPolicy,
) {
    let mut record = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("load session");
    record.flow_kind = WorkItemPlanFlowKind::SingleCandidate;
    record.run_policy = policy;
    record.single_candidate_phase = Some(phase.clone());
    write_json(
        &lifecycle
            .app_paths()
            .issue_root(&record.project_id, &record.issue_id)
            .join("workspace-sessions")
            .join(format!("{}.json", record.id)),
        &record,
    )
    .expect("persist single-candidate session JSON");
    let artifact = engine.session.artifact.clone();
    engine.session = WorkspaceSession::from_record(record);
    engine.session.artifact = artifact;
    let refs = single_candidate_recovery_persist_artifacts(lifecycle, engine, "initial");
    single_candidate_recovery_update_refs(lifecycle, engine, phase, refs);
}

fn single_candidate_recovery_persist_artifacts(
    lifecycle: &LifecycleStore,
    engine: &WorkspaceEngine,
    suffix: &str,
) -> (String, String, String) {
    single_candidate_recovery_persist_candidate_artifacts(
        lifecycle,
        engine,
        suffix,
        &format!("# immutable single candidate source {suffix}\\n"),
    )
}

/// 把任意候选文本（如 campaign 门的真实 markdown 候选）持久化为同一套
/// source/IR/mechanical-report durable 三 refs，供 fixture 以真实门候选文本
/// 铺出 SC 流在门开启前的 durable 形态。
pub(crate) fn single_candidate_recovery_persist_candidate_artifacts(
    lifecycle: &LifecycleStore,
    engine: &WorkspaceEngine,
    suffix: &str,
    source_text: &str,
) -> (String, String, String) {
    let project_id = &engine.session().project_id;
    let issue_id = &engine.session().issue_id;
    let plan_id = &engine.session().entity_id;
    let source_store = WorkItemPlanSourceStore::new(lifecycle.app_paths());
    let source_hash = hex::encode(Sha256::digest(source_text.as_bytes()));
    let mut source = SourceRevisionRecord {
        id: format!("source-{suffix}"),
        source: source_text.to_string(),
        source_revision_hash: source_hash.clone(),
        content_hash: String::new(),
    };
    source.content_hash = source.content_hash().expect("source content hash");
    let source_ref = source_store
        .put_source_revision(project_id, issue_id, plan_id, &source)
        .expect("persist source");
    let store = engine.work_item_plan_store().expect("plan store");
    let index = store
        .load_active_index(project_id, issue_id, plan_id)
        .expect("active index")
        .expect("active index exists");
    let outline = engine
        .latest_work_item_plan_outline_candidate()
        .expect("outline candidate");
    let order = work_item_plan_outline_topological_order(&outline.outline).expect("outline order");
    let drafts = engine
        .accepted_active_draft_records_for_compile(&store, &index, &order)
        .expect("accepted drafts");
    let previous_plan = lifecycle
        .get_issue_work_item_plan(project_id, issue_id, plan_id)
        .expect("previous plan");
    let repository_id = engine
        .work_item_plan_repository_id(lifecycle, &previous_plan)
        .expect("repository id");
    let mut ir = PlanCandidateIrRecord {
        id: format!("ir-{suffix}"),
        source_revision_id: source.id.clone(),
        ir: PlanCandidateIr {
            source_revision_hash: source_hash,
            compiler_version: WORK_ITEM_PLAN_COMPILER_VERSION.to_string(),
            items: drafts
                .iter()
                .map(|draft| PlanCandidateItemIr {
                    target_repository_id: repository_id.to_string(),
                    contract: draft.candidate.canonical_contract_candidate.clone(),
                    verification_plan: draft.candidate.verification_plan.clone(),
                    trusted_commands: Vec::new(),
                })
                .collect(),
        },
        content_hash: String::new(),
    };
    ir.content_hash = ir.content_hash().expect("IR content hash");
    let ir_ref = source_store
        .put_plan_candidate_ir(project_id, issue_id, plan_id, &ir)
        .expect("persist IR");
    let mut report = PlanCandidateMechanicalReportRecord {
        id: format!("report-{suffix}"),
        source_revision_id: source.id,
        ir_id: ir.id,
        report: PlanCandidateMechanicalReport {
            source_revision_hash: ir.ir.source_revision_hash.clone(),
            compiler_version: ir.ir.compiler_version.clone(),
            findings: Vec::<WorkItemSplitFinding>::new(),
        },
        content_hash: String::new(),
    };
    report.content_hash = report.content_hash().expect("report content hash");
    let report_ref = source_store
        .put_mechanical_report(project_id, issue_id, plan_id, &report)
        .expect("persist mechanical report");
    (source_ref, ir_ref, report_ref)
}

pub(crate) fn single_candidate_recovery_update_refs(
    lifecycle: &LifecycleStore,
    engine: &mut WorkspaceEngine,
    phase: SingleCandidatePhase,
    refs: (String, String, String),
) {
    let mut record = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("reload candidate session");
    record.single_candidate_phase = Some(phase);
    record.work_item_plan_source_revision_ref = Some(refs.0);
    record.plan_candidate_ir_ref = Some(refs.1);
    record.mechanical_report_ref = Some(refs.2);
    write_json(
        &lifecycle
            .app_paths()
            .issue_root(&record.project_id, &record.issue_id)
            .join("workspace-sessions")
            .join(format!("{}.json", record.id)),
        &record,
    )
    .expect("persist candidate refs JSON");
    let artifact = engine.session.artifact.clone();
    engine.session = WorkspaceSession::from_record(record);
    engine.session.artifact = artifact;
}

fn single_candidate_recovery_pass_verdict() -> ReviewVerdict {
    ReviewVerdict {
        verdict: ReviewVerdictType::Pass,
        comments: "review pass".to_string(),
        summary: "review pass".to_string(),
        findings: Vec::new(),
        review_gate: ReviewGate::UserConfirmAllowed,
        work_item_plan_review: None,
        structured_output_diagnostic: None,
    }
}

fn single_candidate_recovery_repairable_verdict(message: &str) -> ReviewVerdict {
    ReviewVerdict {
        verdict: ReviewVerdictType::Revise,
        comments: "repair required".to_string(),
        summary: "repair required".to_string(),
        findings: vec![ReviewFinding {
            severity: ReviewFindingSeverity::MustFix,
            message: message.to_string(),
            evidence: "evidence".to_string(),
            required_action: "repair".to_string(),
            category: Some(ReviewFindingCategory::ContractGap),
            class_hint: Some(FindingClassHint::Repairable),
            contract_field: Some("contract.field".to_string()),
        }],
        review_gate: ReviewGate::RequiresRevision,
        work_item_plan_review: None,
        structured_output_diagnostic: None,
    }
}

async fn single_candidate_recovery_complete_review(
    engine: &mut WorkspaceEngine,
    verdict: ReviewVerdict,
) {
    engine
        .complete_review(ProviderCompletion::plain("review", None), verdict)
        .await;
}

async fn single_candidate_recovery_prepare_approval()
-> (tempfile::TempDir, LifecycleStore, String, WorkspaceEngine) {
    let (tmp, lifecycle, plan_id, mut engine) =
        make_work_item_plan_engine_with_accepted_contract_drafts();
    single_candidate_recovery_record(
        &lifecycle,
        &mut engine,
        SingleCandidatePhase::Approval,
        RunPolicy::Interactive,
    );
    (tmp, lifecycle, plan_id, engine)
}

/// 在实际 finalizer/publication 故障前，先经过 provenance crash 边界以固定 approval、
/// reservation、compile ID 和 now；这也是 3.4 四边界测试复用的 durable 前缀。
async fn single_candidate_recovery_prepare_after_provenance_boundary(
    engine: &mut WorkspaceEngine,
    lifecycle: &LifecycleStore,
) -> String {
    let failpoint = engine.register_single_candidate_compile_failpoint(
        SingleCandidateCompileCheckpoint::ProvenancePersisted,
    );
    let error = engine
        .run_work_item_plan_compile()
        .await
        .expect_err("provenance 边界必须先中断");
    drop(failpoint);
    assert!(error.contains("ProvenancePersisted"));
    let record = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("provenance 后 durable session");
    let reservation = record
        .compile_reservation
        .as_ref()
        .expect("reservation 必须先于 provenance durable");
    assert_eq!(
        record.single_candidate_phase,
        Some(SingleCandidatePhase::Approval)
    );
    assert!(record.approval_attempt_id.is_some());
    assert!(record.approved_at.is_some());
    reservation.compile_id.clone()
}

#[test]
fn single_candidate_recovery_generate_reserved_without_ledger_replans_once() {
    let (_tmp, lifecycle, _plan_id, mut engine) =
        make_work_item_plan_engine_with_accepted_contract_drafts();
    single_candidate_recovery_record(
        &lifecycle,
        &mut engine,
        SingleCandidatePhase::Generate,
        RunPolicy::Interactive,
    );

    let first_start = engine
        .reserve_single_candidate_author_start()
        .expect("Generate reservation must persist before provider start");
    assert!(first_start, "首次 Generate reservation 可启动 provider");
    let before_restart = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("Generate durable JSON");
    let ledger_bytes_before = serde_json::to_vec(&before_restart.provider_start_ledger)
        .expect("serialize provider ledger");
    let key = format!(
        "single_candidate_author:{}:{}",
        before_restart.id, before_restart.run_history.repairs_used
    );
    single_candidate_recovery_assert_provider_start_dedup(&before_restart, &key, 1);
    let events_before = single_candidate_recovery_events(&lifecycle, engine.session());

    let mut restarted = single_candidate_recovery_restart(&engine, &lifecycle);
    assert!(
        !restarted
            .reserve_single_candidate_author_start()
            .expect("restarted Generate must reuse reservation"),
        "Generate restart must replan/reuse Reserved entry rather than launch twice"
    );
    let after_restart = lifecycle
        .get_workspace_session(&restarted.session().session_id)
        .expect("restarted durable JSON");
    assert_eq!(
        serde_json::to_vec(&after_restart.provider_start_ledger)
            .expect("serialize provider ledger"),
        ledger_bytes_before,
        "Generate dedup restart 不得改写已有 provider ledger 字节"
    );
    single_candidate_recovery_assert_provider_start_dedup(&after_restart, &key, 1);
    assert_eq!(after_restart.run_history.repairs_used, 0);
    single_candidate_recovery_assert_event_prefix(
        &events_before,
        &single_candidate_recovery_events(&lifecycle, restarted.session()),
    );
}

#[tokio::test]
async fn single_candidate_recovery_repair_started_ledger_is_deduplicated() {
    let (_tmp, lifecycle, _plan_id, mut engine) =
        make_work_item_plan_engine_with_accepted_contract_drafts();
    single_candidate_recovery_record(
        &lifecycle,
        &mut engine,
        SingleCandidatePhase::Evaluate,
        RunPolicy::Interactive,
    );
    single_candidate_recovery_complete_review(
        &mut engine,
        single_candidate_recovery_repairable_verdict("repair durable recovery"),
    )
    .await;
    let after_repair = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("repair durable JSON");
    assert_eq!(
        after_repair.single_candidate_phase,
        Some(SingleCandidatePhase::Generate)
    );
    assert_eq!(after_repair.run_history.repairs_used, 1);

    let first_start = engine
        .reserve_single_candidate_author_start()
        .expect("repair provider reservation");
    assert!(first_start, "repair 第一次 reservation 可启动 provider");
    let before_restart = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("repair started ledger");
    let ledger_bytes_before = serde_json::to_vec(&before_restart.provider_start_ledger)
        .expect("serialize provider ledger");
    let key = format!(
        "single_candidate_author:{}:{}",
        before_restart.id, before_restart.run_history.repairs_used
    );
    single_candidate_recovery_assert_provider_start_dedup(&before_restart, &key, 1);
    let events_before = single_candidate_recovery_events(&lifecycle, engine.session());

    let mut restarted = single_candidate_recovery_restart(&engine, &lifecycle);
    assert!(
        !restarted
            .reserve_single_candidate_author_start()
            .expect("repair restart must deduplicate provider reservation")
    );
    let after_restart = lifecycle
        .get_workspace_session(&restarted.session().session_id)
        .expect("repair restarted JSON");
    assert_eq!(
        after_restart.run_history.repairs_used, 1,
        "recovery 不得重复计 repair"
    );
    assert_eq!(
        serde_json::to_vec(&after_restart.provider_start_ledger)
            .expect("serialize provider ledger"),
        ledger_bytes_before,
        "repair recovery provider ledger 前后字节不变"
    );
    single_candidate_recovery_assert_provider_start_dedup(&after_restart, &key, 1);
    single_candidate_recovery_assert_event_prefix(
        &events_before,
        &single_candidate_recovery_events(&lifecycle, restarted.session()),
    );
}

#[tokio::test]
async fn single_candidate_recovery_approval_pending_reconnect_preserves_gate() {
    let (_tmp, lifecycle, _plan_id, mut engine) =
        make_work_item_plan_engine_with_accepted_contract_drafts();
    single_candidate_recovery_record(
        &lifecycle,
        &mut engine,
        SingleCandidatePhase::Evaluate,
        RunPolicy::Interactive,
    );
    single_candidate_recovery_complete_review(
        &mut engine,
        single_candidate_recovery_pass_verdict(),
    )
    .await;
    let before_restart = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("Approval durable JSON");
    assert_eq!(
        before_restart.single_candidate_phase,
        Some(SingleCandidatePhase::Approval)
    );
    assert_eq!(
        before_restart.status,
        WorkspaceSessionStatus::WaitingForHuman
    );
    let ledger_bytes_before = serde_json::to_vec(&before_restart.provider_start_ledger)
        .expect("serialize provider ledger");
    let events_before = single_candidate_recovery_events(&lifecycle, engine.session());

    let restarted = single_candidate_recovery_restart(&engine, &lifecycle);
    match restarted.build_session_state() {
        WsOutMessage::SessionState {
            session_status,
            provider_start_ledger,
            ..
        } => {
            assert_eq!(session_status, WorkspaceSessionStatus::WaitingForHuman);
            assert!(
                provider_start_ledger.is_empty(),
                "Approval pending 不得启动 provider"
            );
        }
        _ => panic!("expected SessionState"),
    }
    let after_restart = lifecycle
        .get_workspace_session(&restarted.session().session_id)
        .expect("Approval reconnect durable JSON");
    assert_eq!(
        after_restart.single_candidate_phase,
        Some(SingleCandidatePhase::Approval)
    );
    assert_eq!(
        after_restart.status,
        WorkspaceSessionStatus::WaitingForHuman
    );
    assert_eq!(
        serde_json::to_vec(&after_restart.provider_start_ledger)
            .expect("serialize provider ledger"),
        ledger_bytes_before,
        "Approval reconnect provider ledger 前后字节不变且新增 started=0"
    );
    single_candidate_recovery_assert_event_prefix(
        &events_before,
        &single_candidate_recovery_events(&lifecycle, restarted.session()),
    );
}

#[test]
fn single_candidate_recovery_completed_replay_is_absorbing() {
    single_candidate_recovery_terminal_replay(
        SingleCandidatePhase::Completed,
        WorkspaceSessionStatus::Confirmed,
    );
}

#[test]
fn single_candidate_recovery_failed_replay_is_absorbing() {
    single_candidate_recovery_terminal_replay(
        SingleCandidatePhase::Failed,
        WorkspaceSessionStatus::Failed,
    );
}

fn single_candidate_recovery_terminal_replay(
    phase: SingleCandidatePhase,
    status: WorkspaceSessionStatus,
) {
    let (_tmp, lifecycle, plan_id, mut engine) =
        make_work_item_plan_engine_with_accepted_contract_drafts();
    single_candidate_recovery_record(
        &lifecycle,
        &mut engine,
        phase.clone(),
        RunPolicy::Interactive,
    );
    let mut record = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("terminal durable JSON");
    record.status = status.clone();
    write_json(
        &lifecycle
            .app_paths()
            .issue_root(&record.project_id, &record.issue_id)
            .join("workspace-sessions")
            .join(format!("{}.json", record.id)),
        &record,
    )
    .expect("persist terminal status");
    let terminal_json = serde_json::to_value(&record).expect("terminal JSON snapshot");
    let events_before = single_candidate_recovery_events(&lifecycle, engine.session());
    let ledger_bytes_before =
        serde_json::to_vec(&record.provider_start_ledger).expect("serialize provider ledger");

    let restarted = single_candidate_recovery_restart(&engine, &lifecycle);
    match restarted.build_session_state() {
        WsOutMessage::SessionState { session_status, .. } => assert_eq!(session_status, status),
        _ => panic!("expected SessionState"),
    }
    let after_restart = lifecycle
        .get_workspace_session(&restarted.session().session_id)
        .expect("terminal replay durable JSON");
    assert_eq!(after_restart.single_candidate_phase, Some(phase));
    assert_eq!(after_restart.status, status);
    assert_eq!(
        serde_json::to_value(&after_restart).expect("terminal JSON"),
        terminal_json
    );
    assert_eq!(
        serde_json::to_vec(&after_restart.provider_start_ledger)
            .expect("serialize provider ledger"),
        ledger_bytes_before,
        "terminal replay provider ledger 前后字节不变且新增 started=0"
    );
    assert!(
        restarted
            .work_item_plan_store()
            .expect("plan store")
            .list_compile_transactions("project_0001", "issue_0001", &plan_id)
            .expect("transactions")
            .is_empty(),
        "terminal replay 不得新建 compile"
    );
    single_candidate_recovery_assert_event_prefix(
        &events_before,
        &single_candidate_recovery_events(&lifecycle, restarted.session()),
    );
}

#[tokio::test(flavor = "current_thread")]
async fn single_candidate_recovery_finalizer_checkpoint_matrix() {
    let _serial = single_candidate_recovery_failpoint_lock().await;
    for checkpoint in [
        WorkItemPlanCompileFinalizerCheckpoint::PlanSummaryPrepared,
        WorkItemPlanCompileFinalizerCheckpoint::FirstChildSessionEnsured,
        WorkItemPlanCompileFinalizerCheckpoint::FirstChildBindingEnsured,
        WorkItemPlanCompileFinalizerCheckpoint::FirstChildContextPrepared,
        WorkItemPlanCompileFinalizerCheckpoint::CompileReportPersisted,
    ] {
        let (_tmp, lifecycle, plan_id, mut engine) =
            single_candidate_recovery_prepare_approval().await;
        let compile_id =
            single_candidate_recovery_prepare_after_provenance_boundary(&mut engine, &lifecycle)
                .await;
        let mut restarted = single_candidate_recovery_restart(&engine, &lifecycle);
        let finalizer_failpoint =
            restarted.register_work_item_plan_compile_finalizer_failpoint(&compile_id, checkpoint);
        let error = restarted
            .run_work_item_plan_compile()
            .await
            .expect_err("每个 finalizer checkpoint 必须中断");
        drop(finalizer_failpoint);
        assert!(error.contains(&format!("{checkpoint:?}")));
        let initial_events = single_candidate_recovery_events(&lifecycle, restarted.session());
        single_candidate_recovery_mark_transaction_recovery(
            &mut restarted,
            &plan_id,
            &compile_id,
            &format!("finalizer crash {checkpoint:?}"),
        )
        .await;
        let ledger_before = provider_ledger_bytes(&lifecycle);
        let session_before = lifecycle
            .get_workspace_session(&restarted.session().session_id)
            .expect("finalizer recovery session");
        let provider_start_ledger_before =
            serde_json::to_vec(&session_before.provider_start_ledger)
                .expect("serialize provider ledger");
        let transaction_journal =
            crate::product::work_item_plan_store::observe_compile_transaction_writes();

        let mut recovered = single_candidate_recovery_restart(&restarted, &lifecycle);
        let outcome = recovered
            .handle_work_item_plan_compile_recovery_action(
                WorkItemPlanCompileRecoveryActionDto::Continue,
                None,
            )
            .await
            .expect("finalizer restart Continue");
        assert_eq!(outcome, WorkItemPlanCompileRecoveryOutcome::HumanConfirm);
        let transactions = recovered
            .work_item_plan_store()
            .expect("plan store")
            .list_compile_transactions("project_0001", "issue_0001", &plan_id)
            .expect("transactions");
        assert_eq!(
            transactions.len(),
            1,
            "finalizer recovery 不得新建 transaction"
        );
        let tx = &transactions[0];
        assert_eq!(tx.compile_id, compile_id);
        assert_eq!(tx.status, WorkItemPlanCompileStatus::Committed);
        assert_eq!(tx.plan_commit_state, WorkItemPlanCommitState::Committed);
        assert_eq!(tx.step_cursor, "committed");
        single_candidate_recovery_assert_transaction_durable_fields(&lifecycle, tx);
        let child_sessions = lifecycle
            .list_workspace_sessions("project_0001", "issue_0001")
            .expect("child sessions")
            .into_iter()
            .filter(|session| session.workspace_type == WorkspaceType::WorkItem)
            .collect::<Vec<_>>();
        assert_eq!(
            tx.child_session_ids
                .iter()
                .cloned()
                .collect::<BTreeSet<_>>(),
            child_sessions
                .iter()
                .map(|session| session.id.clone())
                .collect(),
            "child finalization 必须幂等"
        );
        assert_eq!(provider_ledger_bytes(&lifecycle), ledger_before);
        let after = lifecycle
            .get_workspace_session(&recovered.session().session_id)
            .expect("finalizer recovered session");
        assert_eq!(
            serde_json::to_vec(&after.provider_start_ledger).expect("serialize provider ledger"),
            provider_start_ledger_before,
            "finalizer recovery provider ledger 前后字节不变且新增 started=0"
        );
        single_candidate_recovery_assert_event_prefix(
            &initial_events,
            &single_candidate_recovery_events(&lifecycle, recovered.session()),
        );
        assert!(
            transaction_journal
                .snapshots()
                .iter()
                .any(|snapshot| snapshot.step_cursor == "committed"),
            "恢复 finalizer 必须 durable committed cursor"
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn single_candidate_recovery_publication_checkpoint_matrix() {
    let _serial = single_candidate_recovery_failpoint_lock().await;
    for checkpoint in [
        InitialPlanPublicationCheckpoint::LineageWritten,
        InitialPlanPublicationCheckpoint::FirstWorkItemArtifactsWritten,
        InitialPlanPublicationCheckpoint::PlanArtifactsWritten,
        InitialPlanPublicationCheckpoint::FirstWorkItemActivated,
        InitialPlanPublicationCheckpoint::PlanActivated,
    ] {
        let (_tmp, lifecycle, plan_id, mut engine) =
            single_candidate_recovery_prepare_approval().await;
        let compile_id =
            single_candidate_recovery_prepare_after_provenance_boundary(&mut engine, &lifecycle)
                .await;
        let mut restarted = single_candidate_recovery_restart(&engine, &lifecycle);
        let publication_failpoint = restarted
            .revision_store()
            .register_initial_plan_publication_failpoint(
                "project_0001",
                "issue_0001",
                &plan_id,
                &compile_id,
                checkpoint,
            );
        let error = restarted
            .run_work_item_plan_compile()
            .await
            .expect_err("每个 publication checkpoint 必须中断");
        drop(publication_failpoint);
        assert!(error.contains(&format!("{checkpoint:?}")));
        let initial_events = single_candidate_recovery_events(&lifecycle, restarted.session());
        single_candidate_recovery_mark_transaction_recovery(
            &mut restarted,
            &plan_id,
            &compile_id,
            &format!("publication crash {checkpoint:?}"),
        )
        .await;
        let prepared_journal = restarted
            .revision_store()
            .get_initial_plan_publication_journal(
                "project_0001",
                "issue_0001",
                &plan_id,
                &compile_id,
            )
            .expect("durable prepared publication journal");
        let ledger_before = provider_ledger_bytes(&lifecycle);
        let session_before = lifecycle
            .get_workspace_session(&restarted.session().session_id)
            .expect("publication recovery session");
        let provider_start_ledger_before =
            serde_json::to_vec(&session_before.provider_start_ledger)
                .expect("serialize provider ledger");
        let transaction_journal =
            crate::product::work_item_plan_store::observe_compile_transaction_writes();

        let mut recovered = single_candidate_recovery_restart(&restarted, &lifecycle);
        let outcome = recovered
            .handle_work_item_plan_compile_recovery_action(
                WorkItemPlanCompileRecoveryActionDto::Continue,
                None,
            )
            .await
            .expect("publication restart Continue");
        assert_eq!(outcome, WorkItemPlanCompileRecoveryOutcome::HumanConfirm);
        let snapshots = transaction_journal.snapshots();
        let publication_resumed = snapshots
            .iter()
            .position(|snapshot| snapshot.step_cursor == "publication_resumed")
            .expect("resume path must persist publication_resumed");
        let first_finalizer = snapshots
            .iter()
            .position(|snapshot| snapshot.step_cursor == "plan_summary_prepared")
            .expect("publication resume 必须进入 finalizer");
        assert!(
            publication_resumed < first_finalizer,
            "publication_resumed 必须位于 publication replay 后、首个 finalizer cursor 前"
        );
        let transactions = recovered
            .work_item_plan_store()
            .expect("plan store")
            .list_compile_transactions("project_0001", "issue_0001", &plan_id)
            .expect("transactions");
        assert_eq!(
            transactions.len(),
            1,
            "publication recovery 不得新建 transaction"
        );
        let tx = &transactions[0];
        assert_eq!(tx.compile_id, compile_id);
        assert_eq!(tx.status, WorkItemPlanCompileStatus::Committed);
        assert_eq!(tx.step_cursor, "committed");
        single_candidate_recovery_assert_transaction_durable_fields(&lifecycle, tx);
        let completed_journal = recovered
            .revision_store()
            .get_initial_plan_publication_journal(
                "project_0001",
                "issue_0001",
                &plan_id,
                &compile_id,
            )
            .expect("completed publication journal");
        assert_eq!(
            completed_journal.phase,
            InitialPlanPublicationPhase::PlanActivated
        );
        assert_eq!(completed_journal.id, prepared_journal.id);
        assert_eq!(
            completed_journal.allocated_ids,
            prepared_journal.allocated_ids
        );
        assert_eq!(
            completed_journal.artifact_fingerprint,
            prepared_journal.artifact_fingerprint
        );
        assert_eq!(completed_journal.artifacts, prepared_journal.artifacts);
        let logical_ids = completed_journal
            .artifacts
            .work_items
            .iter()
            .map(|item| item.logical_work_item.id.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            recovered
                .revision_store()
                .allocate_initial_plan_publication_ids(
                    "project_0001",
                    "issue_0001",
                    &plan_id,
                    &compile_id,
                    &logical_ids,
                )
                .expect("deterministically recompute publication IDs"),
            completed_journal.allocated_ids,
            "publication IDs 必须完全由 compile ID 确定"
        );
        assert_eq!(provider_ledger_bytes(&lifecycle), ledger_before);
        let after = lifecycle
            .get_workspace_session(&recovered.session().session_id)
            .expect("publication recovered session");
        assert_eq!(
            serde_json::to_vec(&after.provider_start_ledger).expect("serialize provider ledger"),
            provider_start_ledger_before,
            "publication recovery provider ledger 前后字节不变且新增 started=0"
        );
        single_candidate_recovery_assert_event_prefix(
            &initial_events,
            &single_candidate_recovery_events(&lifecycle, recovered.session()),
        );
    }
}

/// SC 编译轮次草稿的字节级快照(文件名→内容,排序稳定),用于恢复重放幂等断言。
fn single_candidate_recovery_sc_draft_snapshot(
    lifecycle: &LifecycleStore,
    plan_id: &str,
    compile_id: &str,
) -> Vec<(String, Vec<u8>)> {
    let round_dir = lifecycle
        .app_paths()
        .issue_root("project_0001", "issue_0001")
        .join("work_item_plan_drafts")
        .join(plan_id)
        .join(format!("single_candidate_{compile_id}"));
    let mut snapshot = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&round_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
                let name = path.file_name().unwrap().to_string_lossy().to_string();
                snapshot.push((name, std::fs::read(&path).expect("draft bytes")));
            }
        }
    }
    snapshot.sort_by(|left, right| left.0.cmp(&right.0));
    snapshot
}

/// SC 编译 source-draft 落盘的恢复幂等(8.3 review Major-1 修复):
/// publication 中断 crash → Continue 恢复重放(再次中断)→ 再 Continue 完成,
/// 草稿库恰一套 SC draft:每个 `tx.active_draft_ids` 恰好一条、Accepted/active,
/// 且每次重放前后字节完全不变(draft_id/generation_round_id 由 durable
/// reservation 确定性派生,重放为同路径覆盖写,不重复不缺失)。
#[tokio::test(flavor = "current_thread")]
async fn single_candidate_recovery_replays_source_drafts_idempotently() {
    let _serial = single_candidate_recovery_failpoint_lock().await;
    let (_tmp, lifecycle, plan_id, mut engine) = single_candidate_recovery_prepare_approval().await;
    let compile_id =
        single_candidate_recovery_prepare_after_provenance_boundary(&mut engine, &lifecycle).await;
    let mut restarted = single_candidate_recovery_restart(&engine, &lifecycle);
    let publication_failpoint = restarted
        .revision_store()
        .register_initial_plan_publication_failpoint(
            "project_0001",
            "issue_0001",
            &plan_id,
            &compile_id,
            InitialPlanPublicationCheckpoint::PlanArtifactsWritten,
        );
    let error = restarted
        .run_work_item_plan_compile()
        .await
        .expect_err("publication checkpoint 必须中断");
    drop(publication_failpoint);
    assert!(error.contains("PlanArtifactsWritten"));

    // crash 时 drafts 已在编译提交段落盘(先于 publication),不缺失。
    let store = restarted.work_item_plan_store().expect("plan store");
    let interrupted_tx = store
        .get_compile_transaction("project_0001", "issue_0001", &plan_id, &compile_id)
        .expect("interrupted transaction");
    let expected_draft_ids = interrupted_tx
        .active_draft_ids
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    assert_eq!(expected_draft_ids.len(), 2, "fixture 两个 SC 候选");
    let list_sc_drafts = |lifecycle: &LifecycleStore| {
        crate::product::work_item_plan_store::WorkItemPlanStore::new(lifecycle.app_paths())
            .list_draft_records("project_0001", "issue_0001", &plan_id)
            .expect("draft records")
            .into_iter()
            .filter(|record| record.generation_round_id == format!("single_candidate_{compile_id}"))
            .collect::<Vec<_>>()
    };
    let drafts_after_crash = list_sc_drafts(&lifecycle);
    let ids_after_crash = drafts_after_crash
        .iter()
        .map(|record| record.draft_id.clone())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        ids_after_crash, expected_draft_ids,
        "编译提交段必须把全部 active draft 落盘"
    );
    assert!(
        drafts_after_crash.iter().all(|record| {
            record.status == crate::product::models::WorkItemDraftStatus::Accepted && record.active
        }),
        "SC source draft 必须是 accepted+active"
    );
    let bytes_after_crash =
        single_candidate_recovery_sc_draft_snapshot(&lifecycle, &plan_id, &compile_id);
    assert_eq!(bytes_after_crash.len(), 2);
    let total_after_crash =
        crate::product::work_item_plan_store::WorkItemPlanStore::new(lifecycle.app_paths())
            .list_draft_records("project_0001", "issue_0001", &plan_id)
            .expect("draft records")
            .len();
    assert_eq!(
        total_after_crash, 4,
        "fixture 基座 2 条 legacy + SC 恰 2 条"
    );

    // 第一次恢复重放:resume 在 publication 重放前重写同一套 draft,再次 crash。
    single_candidate_recovery_mark_transaction_recovery(
        &mut restarted,
        &plan_id,
        &compile_id,
        "publication crash PlanArtifactsWritten",
    )
    .await;
    let mut resumed_once = single_candidate_recovery_restart(&restarted, &lifecycle);
    let replay_failpoint = resumed_once
        .revision_store()
        .register_initial_plan_publication_failpoint(
            "project_0001",
            "issue_0001",
            &plan_id,
            &compile_id,
            InitialPlanPublicationCheckpoint::PlanActivated,
        );
    let replay_error = resumed_once
        .handle_work_item_plan_compile_recovery_action(
            WorkItemPlanCompileRecoveryActionDto::Continue,
            None,
        )
        .await
        .expect_err("恢复重放必须再次中断");
    drop(replay_failpoint);
    assert!(replay_error.contains("PlanActivated"));
    assert_eq!(
        single_candidate_recovery_sc_draft_snapshot(&lifecycle, &plan_id, &compile_id),
        bytes_after_crash,
        "第一次重放后草稿字节必须不变(同 draft_id 覆盖写,不重复)"
    );

    // 第二次恢复重放:完成到 committed,草稿仍恰一套。
    single_candidate_recovery_mark_transaction_recovery(
        &mut resumed_once,
        &plan_id,
        &compile_id,
        "publication replay crash PlanActivated",
    )
    .await;
    let mut recovered = single_candidate_recovery_restart(&resumed_once, &lifecycle);
    let outcome = recovered
        .handle_work_item_plan_compile_recovery_action(
            WorkItemPlanCompileRecoveryActionDto::Continue,
            None,
        )
        .await
        .expect("第二次恢复重放必须完成");
    assert_eq!(outcome, WorkItemPlanCompileRecoveryOutcome::HumanConfirm);
    assert_eq!(
        single_candidate_recovery_sc_draft_snapshot(&lifecycle, &plan_id, &compile_id),
        bytes_after_crash,
        "最终恢复后草稿字节仍不变(恰一套,不重复不缺失)"
    );
    let final_tx =
        crate::product::work_item_plan_store::WorkItemPlanStore::new(lifecycle.app_paths())
            .get_compile_transaction("project_0001", "issue_0001", &plan_id, &compile_id)
            .expect("final transaction");
    assert_eq!(final_tx.status, WorkItemPlanCompileStatus::Committed);
    assert_eq!(
        list_sc_drafts(&lifecycle)
            .iter()
            .map(|record| record.draft_id.clone())
            .collect::<BTreeSet<_>>(),
        expected_draft_ids,
        "恢复完成后 SC 草稿仍与 active_draft_ids 一一对应"
    );
}

#[tokio::test]
async fn single_candidate_recovery_rejects_malformed_ref() {
    single_candidate_recovery_rejects_invalid_source_ref("malformed", "SOURCE_STORE_MALFORMED_REF")
        .await;
}

#[tokio::test]
async fn single_candidate_recovery_rejects_wrong_kind_ref() {
    single_candidate_recovery_rejects_invalid_source_ref("wrong_kind", "SOURCE_STORE_WRONG_KIND")
        .await;
}

#[tokio::test]
async fn single_candidate_recovery_rejects_scope_mismatch_ref() {
    single_candidate_recovery_rejects_invalid_source_ref(
        "scope_mismatch",
        "SOURCE_STORE_SCOPE_MISMATCH",
    )
    .await;
}

#[tokio::test]
async fn single_candidate_recovery_rejects_dangling_ref() {
    single_candidate_recovery_rejects_invalid_source_ref("dangling", "SOURCE_STORE_DANGLING_REF")
        .await;
}

async fn single_candidate_recovery_rejects_invalid_source_ref(case: &str, expected_code: &str) {
    let _serial = single_candidate_recovery_failpoint_lock().await;
    let (_tmp, lifecycle, plan_id, mut engine) = single_candidate_recovery_prepare_approval().await;
    let invalid_ref = match case {
        "malformed" => "not-a-canonical-ref".to_string(),
        "wrong_kind" => format!(
            "project/project_0001/issue/issue_0001/plan/{plan_id}/plan_candidate_ir/ir-initial"
        ),
        "scope_mismatch" => format!(
            "project/project_0001/issue/other_issue/plan/{plan_id}/source_revision/source-initial"
        ),
        "dangling" => format!(
            "project/project_0001/issue/issue_0001/plan/{plan_id}/source_revision/source-missing"
        ),
        _ => unreachable!("unknown invalid ref case"),
    };
    let _compile_id =
        single_candidate_recovery_prepare_after_provenance_boundary(&mut engine, &lifecycle).await;
    let mut restarted = single_candidate_recovery_restart(&engine, &lifecycle);
    restarted
        .run_work_item_plan_compile()
        .await
        .expect("finish an immutable transaction before ref validation recovery");
    let store = restarted.work_item_plan_store().expect("plan store");
    let mut tx = store
        .list_compile_transactions("project_0001", "issue_0001", &plan_id)
        .expect("transactions")
        .into_iter()
        .next()
        .expect("one transaction");
    tx.source_revision_ref = Some(invalid_ref);
    tx.status = WorkItemPlanCompileStatus::RecoveryRequired;
    tx.plan_commit_state = WorkItemPlanCommitState::NotStarted;
    tx.step_cursor = "committing".to_string();
    tx.failure_reason = Some(format!("{case} injected"));
    store
        .put_compile_transaction(&tx)
        .expect("persist invalid recovery transaction");
    restarted
        .enter_work_item_plan_compile_recovery(Some(format!("{case} injected")))
        .await;
    let initial_events = single_candidate_recovery_events(&lifecycle, restarted.session());
    let ledger_before = provider_ledger_bytes(&lifecycle);

    let mut recovered = single_candidate_recovery_restart(&restarted, &lifecycle);
    let error = recovered
        .handle_work_item_plan_compile_recovery_action(
            WorkItemPlanCompileRecoveryActionDto::Continue,
            None,
        )
        .await
        .expect_err("invalid direct ref 必须拒绝恢复");
    assert!(error.contains(expected_code), "{case}: {error}");
    let failed = lifecycle
        .get_workspace_session(&recovered.session().session_id)
        .expect("durable failure diagnostic");
    assert_eq!(
        failed.single_candidate_phase,
        Some(SingleCandidatePhase::Failed)
    );
    assert_eq!(failed.status, WorkspaceSessionStatus::Failed);
    assert!(failed.policy_diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "single_candidate_recovery_failed"
            && diagnostic.message.contains(expected_code)
    }));
    assert_eq!(provider_ledger_bytes(&lifecycle), ledger_before);
    single_candidate_recovery_assert_event_prefix(
        &initial_events,
        &single_candidate_recovery_events(&lifecycle, recovered.session()),
    );
}
