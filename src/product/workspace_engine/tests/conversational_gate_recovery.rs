use crate::product::models::{HumanGateTurn, HumanGateTurnFailureClass, HumanGateTurnStatus};
use crate::product::work_item_plan_policy::WorkItemPlanFlowKind;
use crate::product::workspace_engine::conversational_gate_recovery::{
    assert_human_gate_event_prefix_immutable, recover_human_gate_turn,
};
use crate::product::workspace_engine::{
    HUMAN_GATE_PROVIDER_MAX_ATTEMPTS, HumanGateRecoveryAction, provider_run_kind_for_human_gate,
};

fn turn(status: HumanGateTurnStatus, attempt_no: u32) -> HumanGateTurn {
    HumanGateTurn {
        turn_id: "turn_recovery_001".to_string(),
        session_id: "session_001".to_string(),
        command_id: "command_001".to_string(),
        feedback_text: "修复字段".to_string(),
        status,
        attempt_no,
        budget_reserved: 1,
        source_hash: String::new(),
        result_artifact_ref: None,
        failure_class: None,
        created_at: "2026-08-31T00:00:00Z".to_string(),
        updated_at: "2026-08-31T00:00:00Z".to_string(),
    }
}

#[test]
fn conversational_gate_recovery_resumes_reserved_attempt_one() {
    let action = recover_human_gate_turn(&turn(HumanGateTurnStatus::Reserved, 1), false)
        .expect("reserved turn should restart its unstarted provider attempt");
    assert_eq!(
        action,
        HumanGateRecoveryAction::ResumeSameTurn { next_attempt_no: 1 }
    );
}

#[test]
fn conversational_gate_recovery_waits_for_running_provider() {
    let action = recover_human_gate_turn(&turn(HumanGateTurnStatus::Running, 1), true)
        .expect("running provider should be waitable");
    assert_eq!(action, HumanGateRecoveryAction::WaitForProvider);
}

#[test]
fn conversational_gate_recovery_retries_dead_provider_on_same_turn() {
    let original = turn(HumanGateTurnStatus::Running, 1);
    let action = recover_human_gate_turn(&original, false).expect("dead provider should retry");
    assert_eq!(
        action,
        HumanGateRecoveryAction::ResumeSameTurn { next_attempt_no: 2 }
    );
    assert_eq!(original.turn_id, "turn_recovery_001");
    assert_eq!(original.attempt_no, 1);
    assert_eq!(original.budget_reserved, 1);
}

#[test]
fn conversational_gate_recovery_fails_after_fixed_attempt_limit() {
    let action = recover_human_gate_turn(
        &turn(
            HumanGateTurnStatus::Running,
            HUMAN_GATE_PROVIDER_MAX_ATTEMPTS,
        ),
        false,
    )
    .expect("attempt limit should produce a terminal action");
    assert_eq!(
        action,
        HumanGateRecoveryAction::MarkFailed {
            failure_class: HumanGateTurnFailureClass::ProviderErr,
        }
    );
}

#[test]
fn conversational_gate_recovery_maps_only_single_candidate_to_provider_run_kind() {
    let run_kind = provider_run_kind_for_human_gate(
        WorkItemPlanFlowKind::SingleCandidate,
        "turn_recovery_001",
    )
    .expect("single-candidate gate should map to a dedicated run kind");
    assert!(matches!(
        run_kind,
        crate::product::workspace_engine::ProviderRunKind::HumanGateScManualRevision {
            turn_id,
            prompt
        } if turn_id == "turn_recovery_001" && prompt.is_empty()
    ));
    assert!(
        provider_run_kind_for_human_gate(WorkItemPlanFlowKind::Legacy, "turn_recovery_001")
            .is_err()
    );
}
#[test]
fn conversational_gate_recovery_preserves_event_prefix_and_budget() {
    let event_prefix = vec!["human_gate_turn_open", "human_gate_turn_completed"];
    let recovered_events = vec![
        "human_gate_turn_open",
        "human_gate_turn_completed",
        "human_gate_turn_open",
    ];
    assert_human_gate_event_prefix_immutable(&event_prefix, &recovered_events)
        .expect("recovery may append only a suffix");

    let original = turn(HumanGateTurnStatus::Running, 1);
    let action = recover_human_gate_turn(&original, false).expect("dead provider should retry");
    assert!(matches!(
        action,
        HumanGateRecoveryAction::ResumeSameTurn { next_attempt_no: 2 }
    ));
    assert_eq!(original.turn_id, "turn_recovery_001");
    assert_eq!(original.budget_reserved, 1);
}

#[test]
fn conversational_gate_recovery_revision_crash_window_with_cross_round_refs_fails_closed() {
    use crate::product::json_store::write_json;
    use crate::product::models::{
        HumanGateReservation, HumanGateTurn, HumanGateTurnStatus, SingleCandidatePhase,
        WorkspaceSessionStatus,
    };
    use crate::product::work_item_plan_compiler::{
        PlanCandidateValidationContext, WorkItemPlanSourceContext, compile_work_item_plan,
        validate_plan_candidate_ir,
    };
    use crate::product::work_item_plan_policy::{
        HumanGateSnapshot, HumanReason, ProviderStartLedgerEntry,
    };
    use crate::product::work_item_plan_source_store::{
        PlanCandidateIrRecord, PlanCandidateMechanicalReportRecord, SourceRevisionRecord,
        WorkItemPlanSourceStore,
    };
    use crate::web::workspace_ws_types::{ArtifactPayload, ArtifactVersion};
    use sha2::{Digest, Sha256};

    let (_tmp, lifecycle, plan_id, mut engine) =
        super::make_work_item_plan_engine_with_accepted_contract_drafts();
    let mut session = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("session");
    session.flow_kind = WorkItemPlanFlowKind::SingleCandidate;
    session.single_candidate_phase = Some(SingleCandidatePhase::Approval);
    session.status = WorkspaceSessionStatus::WaitingForHuman;
    session.human_gate_snapshot = Some(HumanGateSnapshot {
        candidate_recovery: None,
        findings: Vec::new(),
        repeated_fingerprints: Vec::new(),
        attempts_used: 0,
        manual_repairs_remaining: 2,
        trigger: HumanReason::NativeHumanRequired,
        resumable: false,
        accepted_feedback_turns: None,
    });
    let session_path = lifecycle
        .app_paths()
        .issue_root(&session.project_id, &session.issue_id)
        .join("workspace-sessions")
        .join(format!("{}.json", session.id));
    write_json(&session_path, &session).expect("persist human gate session");

    let now = "2026-08-31T00:00:00Z".to_string();
    let reserved_source_hash = "a".repeat(64);
    let reserved_turn = HumanGateTurn {
        turn_id: "turn_recovery_revision_crash".to_string(),
        session_id: session.id.clone(),
        command_id: "command_recovery_revision_crash".to_string(),
        feedback_text: "修正当前候选".to_string(),
        status: HumanGateTurnStatus::Reserved,
        attempt_no: 1,
        budget_reserved: 1,
        source_hash: reserved_source_hash.clone(),
        result_artifact_ref: None,
        failure_class: None,
        created_at: now.clone(),
        updated_at: now.clone(),
    };
    let reservation = HumanGateReservation {
        command_id: reserved_turn.command_id.clone(),
        turn_id: reserved_turn.turn_id.clone(),
        provider_start_idempotency_key: format!("human_gate:{}:attempt:1", reserved_turn.turn_id),
        reserved_at: now,
    };
    let (reserved_session, _) = lifecycle
        .compare_and_reserve_human_gate_turn(&session, reserved_turn.clone(), reservation)
        .expect("reserve durable turn");
    let revised_source = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/product/work_item_plan_compiler/fixtures/work-item-plan-rep4.md"
    ))
    .replace(
        "## Work Item WI-001: Backend levels API",
        "## Work Item WI-001: Recovered backend levels API",
    );
    let revised_source_hash = hex::encode(Sha256::digest(revised_source.as_bytes()));
    let source_hash = revised_source_hash.clone();
    let mut running_turn = reserved_turn;
    running_turn.status = HumanGateTurnStatus::Running;
    running_turn.source_hash = reserved_source_hash.clone();
    let running_session = lifecycle
        .update_human_gate_turn(&reserved_session, running_turn.clone())
        .expect("persist running turn");
    let source_store = WorkItemPlanSourceStore::new(lifecycle.app_paths());
    let mut source = SourceRevisionRecord {
        id: "source-recovery-revision".to_string(),
        source: revised_source.to_string(),
        source_revision_hash: source_hash.clone(),
        content_hash: String::new(),
    };
    source.content_hash = source.content_hash().expect("source content hash");
    let source_ref = source_store
        .put_source_revision("project_0001", "issue_0001", &plan_id, &source)
        .expect("persist source");
    let plan = lifecycle
        .get_issue_work_item_plan("project_0001", "issue_0001", &plan_id)
        .expect("plan");
    let repository_id = engine
        .work_item_plan_repository_id(&lifecycle, &plan)
        .expect("repository id");
    let ir = compile_work_item_plan(
        &revised_source,
        &WorkItemPlanSourceContext {
            target_repository_id: repository_id,
        },
    )
    .expect("compile revised source");
    let repository_profile = plan.repository_profile_ref.as_deref().map(|profile_id| {
        lifecycle
            .get_repository_profile("project_0001", "issue_0001", profile_id)
            .expect("repository profile")
    });
    let report = validate_plan_candidate_ir(
        &ir,
        &PlanCandidateValidationContext {
            project_id: "project_0001",
            issue_id: "issue_0001",
            plan_id: &plan_id,
            source_story_spec_ids: &plan.source_story_spec_ids,
            source_design_spec_ids: &plan.source_design_spec_ids,
            repository_profile: repository_profile.as_ref(),
            plan_options: &plan.options,
            baseline_tree: None,
            now: "2026-08-31T00:00:00Z",
        },
    )
    .expect("validate revised source");
    let mut ir_record = PlanCandidateIrRecord {
        id: "ir-recovery-revision".to_string(),
        source_revision_id: source.id.clone(),
        ir,
        content_hash: String::new(),
    };
    ir_record.content_hash = ir_record.content_hash().expect("IR content hash");
    let ir_ref = source_store
        .put_plan_candidate_ir("project_0001", "issue_0001", &plan_id, &ir_record)
        .expect("persist IR");
    let mut report_record = PlanCandidateMechanicalReportRecord {
        id: "report-recovery-revision".to_string(),
        source_revision_id: source.id,
        ir_id: ir_record.id,
        report,
        content_hash: String::new(),
    };
    report_record.content_hash = report_record.content_hash().expect("report content hash");
    let report_ref = source_store
        .put_mechanical_report("project_0001", "issue_0001", &plan_id, &report_record)
        .expect("persist report");

    let versions = vec![
        ArtifactVersion {
            version: 1,
            payload: ArtifactPayload::Markdown {
                markdown: "# old candidate\n".to_string(),
                diff: None,
            },
            generated_by: crate::product::models::ProviderName::Fake,
            reviewed_by: None,
            review_verdict: None,
            confirmed_by: None,
            is_current: false,
            created_at: "2026-08-30T00:00:00Z".to_string(),
            source_node_id: "node_recovery_old".to_string(),
        },
        ArtifactVersion {
            version: 2,
            payload: ArtifactPayload::Markdown {
                markdown: revised_source.to_string(),
                diff: None,
            },
            generated_by: crate::product::models::ProviderName::Fake,
            reviewed_by: None,
            review_verdict: None,
            confirmed_by: None,
            is_current: true,
            created_at: "2026-08-31T00:00:00Z".to_string(),
            source_node_id: "node_recovery_new".to_string(),
        },
    ];
    lifecycle
        .save_artifact_versions(&running_session.id, &versions)
        .expect("persist artifact versions");

    let mut torn_session = running_session;
    torn_session.work_item_plan_source_revision_ref = Some(source_ref);
    torn_session.plan_candidate_ir_ref = Some(ir_ref);
    torn_session.mechanical_report_ref = Some(report_ref);
    write_json(&session_path, &torn_session).expect("persist refs-before-turn crash fixture");
    let refs_before = (
        torn_session.work_item_plan_source_revision_ref.clone(),
        torn_session.plan_candidate_ir_ref.clone(),
        torn_session.mechanical_report_ref.clone(),
    );
    let budget_before = torn_session.human_gate_snapshot.clone();
    let ledger_before: Vec<ProviderStartLedgerEntry> = torn_session.provider_start_ledger.clone();
    engine.session = super::WorkspaceSession::from_record(torn_session);

    let actions = engine
        .recover_human_gate_turns(false)
        .expect("production human gate recovery entry");
    assert_eq!(
        actions,
        vec![(
            running_turn.turn_id.clone(),
            HumanGateRecoveryAction::MarkFailed {
                failure_class: HumanGateTurnFailureClass::ValidationReject,
            },
        )]
    );
    let recovered_turn = lifecycle
        .get_human_gate_turn(engine.session().session_id.as_str(), &running_turn.turn_id)
        .expect("recovered turn");
    assert_eq!(recovered_turn.status, HumanGateTurnStatus::Failed);
    assert_eq!(
        recovered_turn.failure_class,
        Some(HumanGateTurnFailureClass::ValidationReject)
    );
    assert_eq!(recovered_turn.source_hash, running_turn.source_hash);
    let recovered_session = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("recovered session");
    assert_eq!(
        refs_before,
        (
            recovered_session.work_item_plan_source_revision_ref,
            recovered_session.plan_candidate_ir_ref,
            recovered_session.mechanical_report_ref,
        )
    );
    assert_eq!(recovered_session.human_gate_snapshot, budget_before);
    assert_eq!(recovered_session.provider_start_ledger, ledger_before);
}
#[test]
fn conversational_gate_recovery_forward_evidence_completes_revision() {
    use crate::product::json_store::write_json;
    use crate::product::models::{
        HumanGateReservation, HumanGateTurn, HumanGateTurnStatus, SingleCandidatePhase,
        WorkspaceSessionStatus,
    };
    use crate::product::work_item_plan_compiler::{
        PlanCandidateValidationContext, WorkItemPlanSourceContext, compile_work_item_plan,
        validate_plan_candidate_ir,
    };
    use crate::product::work_item_plan_policy::{
        HumanGateSnapshot, HumanReason, ProviderStartLedgerEntry,
    };
    use crate::product::work_item_plan_source_store::{
        PlanCandidateIrRecord, PlanCandidateMechanicalReportRecord, SourceRevisionRecord,
        WorkItemPlanSourceStore,
    };
    use crate::web::workspace_ws_types::{ArtifactPayload, ArtifactVersion};
    use sha2::{Digest, Sha256};

    let (_tmp, lifecycle, plan_id, mut engine) =
        super::make_work_item_plan_engine_with_accepted_contract_drafts();
    let mut session = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("session");
    session.flow_kind = WorkItemPlanFlowKind::SingleCandidate;
    session.single_candidate_phase = Some(SingleCandidatePhase::Approval);
    session.status = WorkspaceSessionStatus::WaitingForHuman;
    session.human_gate_snapshot = Some(HumanGateSnapshot {
        candidate_recovery: None,
        findings: Vec::new(),
        repeated_fingerprints: Vec::new(),
        attempts_used: 0,
        manual_repairs_remaining: 2,
        trigger: HumanReason::NativeHumanRequired,
        resumable: false,
        accepted_feedback_turns: None,
    });
    let session_path = lifecycle
        .app_paths()
        .issue_root(&session.project_id, &session.issue_id)
        .join("workspace-sessions")
        .join(format!("{}.json", session.id));
    write_json(&session_path, &session).expect("persist human gate session");

    let now = "2026-08-30T00:00:00Z".to_string();
    let revised_source = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/product/work_item_plan_compiler/fixtures/work-item-plan-rep4.md"
    ));
    let reserved_source_hash = hex::encode(Sha256::digest(revised_source.as_bytes()));
    let reserved_turn = HumanGateTurn {
        turn_id: "turn_recovery_revision_crash".to_string(),
        session_id: session.id.clone(),
        command_id: "command_recovery_revision_crash".to_string(),
        feedback_text: "修正当前候选".to_string(),
        status: HumanGateTurnStatus::Reserved,
        attempt_no: 1,
        budget_reserved: 1,
        source_hash: reserved_source_hash.clone(),
        result_artifact_ref: None,
        failure_class: None,
        created_at: now.clone(),
        updated_at: now.clone(),
    };
    let reservation = HumanGateReservation {
        command_id: reserved_turn.command_id.clone(),
        turn_id: reserved_turn.turn_id.clone(),
        provider_start_idempotency_key: format!("human_gate:{}:attempt:1", reserved_turn.turn_id),
        reserved_at: now,
    };
    let (reserved_session, _) = lifecycle
        .compare_and_reserve_human_gate_turn(&session, reserved_turn.clone(), reservation)
        .expect("reserve durable turn");
    let source_hash = hex::encode(Sha256::digest(revised_source.as_bytes()));
    let mut running_turn = reserved_turn;
    running_turn.status = HumanGateTurnStatus::Running;
    running_turn.source_hash = source_hash.clone();
    let running_session = lifecycle
        .update_human_gate_turn(&reserved_session, running_turn.clone())
        .expect("persist running turn");
    let source_store = WorkItemPlanSourceStore::new(lifecycle.app_paths());
    let mut source = SourceRevisionRecord {
        id: "source-recovery-revision".to_string(),
        source: revised_source.to_string(),
        source_revision_hash: source_hash.clone(),
        content_hash: String::new(),
    };
    source.content_hash = source.content_hash().expect("source content hash");
    let source_ref = source_store
        .put_source_revision("project_0001", "issue_0001", &plan_id, &source)
        .expect("persist source");
    let plan = lifecycle
        .get_issue_work_item_plan("project_0001", "issue_0001", &plan_id)
        .expect("plan");
    let repository_id = engine
        .work_item_plan_repository_id(&lifecycle, &plan)
        .expect("repository id");
    let ir = compile_work_item_plan(
        revised_source,
        &WorkItemPlanSourceContext {
            target_repository_id: repository_id,
        },
    )
    .expect("compile revised source");
    let repository_profile = plan.repository_profile_ref.as_deref().map(|profile_id| {
        lifecycle
            .get_repository_profile("project_0001", "issue_0001", profile_id)
            .expect("repository profile")
    });
    let report = validate_plan_candidate_ir(
        &ir,
        &PlanCandidateValidationContext {
            project_id: "project_0001",
            issue_id: "issue_0001",
            plan_id: &plan_id,
            source_story_spec_ids: &plan.source_story_spec_ids,
            source_design_spec_ids: &plan.source_design_spec_ids,
            repository_profile: repository_profile.as_ref(),
            plan_options: &plan.options,
            baseline_tree: None,
            now: "2026-08-31T00:00:00Z",
        },
    )
    .expect("validate revised source");
    let mut ir_record = PlanCandidateIrRecord {
        id: "ir-recovery-revision".to_string(),
        source_revision_id: source.id.clone(),
        ir,
        content_hash: String::new(),
    };
    ir_record.content_hash = ir_record.content_hash().expect("IR content hash");
    let ir_ref = source_store
        .put_plan_candidate_ir("project_0001", "issue_0001", &plan_id, &ir_record)
        .expect("persist IR");
    let mut report_record = PlanCandidateMechanicalReportRecord {
        id: "report-recovery-revision".to_string(),
        source_revision_id: source.id,
        ir_id: ir_record.id,
        report,
        content_hash: String::new(),
    };
    report_record.content_hash = report_record.content_hash().expect("report content hash");
    let report_ref = source_store
        .put_mechanical_report("project_0001", "issue_0001", &plan_id, &report_record)
        .expect("persist report");

    let versions = vec![
        ArtifactVersion {
            version: 1,
            payload: ArtifactPayload::Markdown {
                markdown: "# old candidate\n".to_string(),
                diff: None,
            },
            generated_by: crate::product::models::ProviderName::Fake,
            reviewed_by: None,
            review_verdict: None,
            confirmed_by: None,
            is_current: false,
            created_at: "2026-08-30T00:00:00Z".to_string(),
            source_node_id: "node_recovery_old".to_string(),
        },
        ArtifactVersion {
            version: 2,
            payload: ArtifactPayload::Markdown {
                markdown: revised_source.to_string(),
                diff: None,
            },
            generated_by: crate::product::models::ProviderName::Fake,
            reviewed_by: None,
            review_verdict: None,
            confirmed_by: None,
            is_current: true,
            created_at: "2026-08-31T00:00:00Z".to_string(),
            source_node_id: "node_recovery_new".to_string(),
        },
    ];
    lifecycle
        .save_artifact_versions(&running_session.id, &versions)
        .expect("persist artifact versions");

    let mut torn_session = running_session;
    torn_session.work_item_plan_source_revision_ref = Some(source_ref);
    torn_session.plan_candidate_ir_ref = Some(ir_ref);
    torn_session.mechanical_report_ref = Some(report_ref);
    write_json(&session_path, &torn_session).expect("persist refs-before-turn crash fixture");
    let refs_before = (
        torn_session.work_item_plan_source_revision_ref.clone(),
        torn_session.plan_candidate_ir_ref.clone(),
        torn_session.mechanical_report_ref.clone(),
    );
    let budget_before = torn_session.human_gate_snapshot.clone();
    let ledger_before: Vec<ProviderStartLedgerEntry> = torn_session.provider_start_ledger.clone();
    engine.session = super::WorkspaceSession::from_record(torn_session);

    let actions = engine
        .recover_human_gate_turns(false)
        .expect("production human gate recovery entry");
    assert_eq!(
        actions,
        vec![(
            running_turn.turn_id.clone(),
            HumanGateRecoveryAction::CompletedRevision,
        )]
    );
    let recovered_turn = lifecycle
        .get_human_gate_turn(engine.session().session_id.as_str(), &running_turn.turn_id)
        .expect("completed recovered turn");
    assert_eq!(recovered_turn.status, HumanGateTurnStatus::Completed);
    assert_eq!(
        recovered_turn.result_artifact_ref.as_deref(),
        Some("artifact_version_002")
    );
    assert_eq!(recovered_turn.source_hash, running_turn.source_hash);
    let recovered_session = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("recovered session");
    assert_eq!(
        refs_before,
        (
            recovered_session.work_item_plan_source_revision_ref,
            recovered_session.plan_candidate_ir_ref,
            recovered_session.mechanical_report_ref,
        )
    );
    assert_eq!(recovered_session.human_gate_snapshot, budget_before);
    assert_eq!(recovered_session.provider_start_ledger, ledger_before);
}
#[test]
fn conversational_gate_recovery_reservation_commit_restart_keeps_budget_and_turn() {
    let original = turn(HumanGateTurnStatus::Reserved, 1);
    let action = recover_human_gate_turn(&original, false)
        .expect("reserved turn should restart attempt one after reconnect");
    assert_eq!(
        action,
        HumanGateRecoveryAction::ResumeSameTurn { next_attempt_no: 1 }
    );
    assert_eq!(original.turn_id, "turn_recovery_001");
    assert_eq!(original.attempt_no, 1);
    assert_eq!(original.budget_reserved, 1);
    let attempt_key = format!("human_gate:{}:attempt:1", original.turn_id);
    assert_eq!(attempt_key, "human_gate:turn_recovery_001:attempt:1");
    assert_eq!(
        [attempt_key.clone(), attempt_key]
            .into_iter()
            .collect::<std::collections::HashSet<_>>()
            .len(),
        1
    );
}

/// C1 Task 4（REQ-C1-GATE-01/02）：孤儿候选门恢复矩阵。
/// 场景 A：完整 candidate/source/budget/gate 事实落盘 + relay/WS consumer
/// 缺席——零内存态重开引擎后能读同一 snapshot，恢复请求 accepted，原门
/// approve 仍走完整 deterministic compile→Confirmed；同 command 重放返回
/// 首次 durable 结果，不重复扣 budget/provider/turn。
/// 场景 B：缺 source/IR/report refs 的 durable session——恢复返回
/// needs_human（missing 可见），approve 固定 fail-closed，session
/// phase/budget/provider ledger 全不变；旧 gate id 的恢复命令被拒。
#[tokio::test]
async fn orphaned_candidate_snapshot_requires_recovery_before_approve() {
    use crate::product::checkpoint_store::CheckpointStore;
    use crate::product::json_store::write_json;
    use crate::product::models::{SingleCandidatePhase, WorkspaceSessionStatus};
    use crate::product::work_item_plan_policy::{
        HumanGateSnapshot, HumanReason, RunPolicy, WorkItemPlanFlowKind,
    };
    use crate::product::work_item_plan_policy::CandidateRecoveryAction;
    use crate::product::workspace_engine::{CandidateRecoveryCommand, CandidateRecoveryOutcome};
    use crate::product::workspace_engine::{
        HumanGateCloseDecision, WorkspaceEngine, WorkspaceSession, WorkspaceStage,
    };
    use crate::web::workspace_ws_types::ArtifactPayload;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;
    use tokio::sync::mpsc;

    let rep4_candidate = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/product/work_item_plan_compiler/fixtures/work-item-plan-rep4.md"
    ))
    .replace(
        "- provided_contract_refs: contract.levels-integration",
        "- provided_contract_refs: []",
    );

    // —— 场景 A：完整候选事实 + relay/WS consumer 缺席 ——
    let (root, lifecycle, _plan_id, mut engine) =
        super::make_work_item_plan_engine_with_accepted_contract_drafts();
    let app_paths = lifecycle.app_paths();
    // 会话 scope 唯一化（与 campaign fixture 同款）：failpoint 注册表进程级
    // 全局且以 durable scope 为键，避免与并发 failpoint 家族互扰。
    {
        static C1GATE_SESSION_SEQUENCE: AtomicU64 = AtomicU64::new(1);
        let mut record = lifecycle
            .get_workspace_session(&engine.session().session_id)
            .expect("fixture session record");
        let previous_session_path = app_paths
            .issue_root(&record.project_id, &record.issue_id)
            .join("workspace-sessions")
            .join(format!("{}.json", record.id));
        record.id = format!(
            "{}-c1gate-{}",
            record.id,
            C1GATE_SESSION_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        );
        let artifact = engine.session.artifact.clone();
        write_json(
            &app_paths
                .issue_root(&record.project_id, &record.issue_id)
                .join("workspace-sessions")
                .join(format!("{}.json", record.id)),
            &record,
        )
        .expect("persist c1gate-unique session");
        std::fs::remove_file(&previous_session_path).expect("drop fixture session");
        engine.session = WorkspaceSession::from_record(record);
        engine.session.artifact = artifact;
    }
    super::single_candidate_recovery::single_candidate_recovery_record(
        &lifecycle,
        &mut engine,
        SingleCandidatePhase::Approval,
        RunPolicy::Interactive,
    );
    let gate_refs =
        super::single_candidate_recovery::single_candidate_recovery_persist_candidate_artifacts(
            &lifecycle,
            &engine,
            "c1gate",
            &rep4_candidate,
        );
    super::single_candidate_recovery::single_candidate_recovery_update_refs(
        &lifecycle,
        &mut engine,
        SingleCandidatePhase::Approval,
        gate_refs,
    );
    engine
        .update_artifact(ArtifactPayload::Markdown {
            markdown: rep4_candidate.clone(),
            diff: None,
        })
        .await;
    let mut record = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("session record");
    record.flow_kind = WorkItemPlanFlowKind::SingleCandidate;
    record.run_policy = RunPolicy::Interactive;
    record.review_rounds = 0;
    record.single_candidate_phase = Some(SingleCandidatePhase::Approval);
    record.status = WorkspaceSessionStatus::WaitingForHuman;
    record.human_gate_snapshot = Some(HumanGateSnapshot {
        candidate_recovery: None,
        findings: Vec::new(),
        repeated_fingerprints: Vec::new(),
        attempts_used: 0,
        manual_repairs_remaining: 2,
        trigger: HumanReason::NativeHumanRequired,
        resumable: true,
        accepted_feedback_turns: None,
    });
    let session_path = app_paths
        .issue_root(&record.project_id, &record.issue_id)
        .join("workspace-sessions")
        .join(format!("{}.json", record.id));
    write_json(&session_path, &record).expect("persist c1gate gate session");
    let artifact = engine.session.artifact.clone();
    let mut session = WorkspaceSession::from_record(record);
    session.stage = WorkspaceStage::HumanConfirm;
    session.session_status = WorkspaceSessionStatus::WaitingForHuman;
    session.artifact = artifact;
    let (event_tx, _event_rx) = mpsc::channel(64);
    let mut engine = WorkspaceEngine::new_persistent(
        Arc::new(CheckpointStore::new(root.path().join("c1gate-a-checkpoints"))),
        lifecycle.clone(),
        event_tx,
        session,
    );
    // 门开启（生产路径）：落 durable 门节点（绿色阶段同时于 relay 前持久化
    // 候选快照完整性 label）。
    engine
        .enter_human_confirm(Some("C1 候选确认门".to_string()))
        .await;
    let gate_id = engine
        .active_timeline_node_id()
        .expect("durable active gate node");

    // relay/WS consumer 缺席：零内存态重开。
    let record = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("durable session after gate open");
    let artifact = engine.session.artifact.clone();
    let mut session = WorkspaceSession::from_record(record);
    session.stage = WorkspaceStage::HumanConfirm;
    session.artifact = artifact;
    let (event_tx, _event_rx) = mpsc::channel(64);
    let mut reopened = WorkspaceEngine::new_persistent(
        Arc::new(CheckpointStore::new(root.path().join("c1gate-a-reopened"))),
        lifecycle.clone(),
        event_tx,
        session,
    );

    // 恢复请求：accepted，完整事实可见。
    let recovery = reopened
        .recover_candidate_gate(CandidateRecoveryCommand {
            command_id: "cmd-c1gate-recovery-a".to_string(),
            expected_gate_id: gate_id.clone(),
            action: CandidateRecoveryAction::Recover,
        })
        .await
        .expect("candidate recovery accepted");
    let CandidateRecoveryOutcome::Accepted { facts } = &recovery else {
        panic!("expected accepted recovery, got {recovery:?}");
    };
    assert!(facts.complete, "missing: {:?}", facts.missing);
    assert_eq!(facts.gate_id, gate_id);
    assert_eq!(facts.budget_remaining, Some(2));

    // durable label 已落：重开再读同一 snapshot 事实。
    let labeled = lifecycle
        .get_workspace_session(&reopened.session().session_id)
        .expect("labeled session");
    let label = labeled
        .human_gate_snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.candidate_recovery.as_ref())
        .expect("durable candidate recovery label");
    assert!(label.complete);
    assert_eq!(label.commands.len(), 1);

    // 同 command 重放：返回首次 durable 结果，零预算/turn/ledger 增量。
    let replayed = reopened
        .recover_candidate_gate(CandidateRecoveryCommand {
            command_id: "cmd-c1gate-recovery-a".to_string(),
            expected_gate_id: gate_id.clone(),
            action: CandidateRecoveryAction::Recover,
        })
        .await
        .expect("replay resolves");
    assert!(matches!(
        &replayed,
        CandidateRecoveryOutcome::Replayed { record, .. }
            if record.command_id == "cmd-c1gate-recovery-a"
    ));
    let after_replay = lifecycle
        .get_workspace_session(&reopened.session().session_id)
        .expect("session after replay");
    let replay_snapshot = after_replay.human_gate_snapshot.as_ref().unwrap();
    assert_eq!(replay_snapshot.manual_repairs_remaining, 2);
    assert_eq!(
        replay_snapshot
            .candidate_recovery
            .as_ref()
            .unwrap()
            .commands
            .len(),
        1
    );
    assert!(after_replay.provider_start_ledger.is_empty());
    assert!(
        lifecycle
            .list_human_gate_turns(&after_replay.id)
            .expect("turns")
            .is_empty()
    );

    // 原 gate 仍可用既有 approve：完整 deterministic compile → Confirmed。
    let outcome = reopened
        .handle_human_gate_termination(HumanGateCloseDecision::Approve)
        .await
        .expect("approve after recovery confirms");
    assert!(matches!(
        outcome,
        crate::product::workspace_engine::HumanGateCloseOutcome::Confirmed
    ));
    let confirmed = lifecycle
        .get_workspace_session(&reopened.session().session_id)
        .expect("confirmed session");
    assert_eq!(confirmed.status, WorkspaceSessionStatus::Confirmed);

    // —— 场景 B：缺 source/IR/report refs 的孤儿门 ——
    let (root, lifecycle, _plan_id, mut engine) =
        super::make_work_item_plan_engine_with_accepted_contract_drafts();
    let app_paths = lifecycle.app_paths();
    {
        static C1GATE_MISS_SEQUENCE: AtomicU64 = AtomicU64::new(1);
        let mut record = lifecycle
            .get_workspace_session(&engine.session().session_id)
            .expect("fixture session record");
        let previous_session_path = app_paths
            .issue_root(&record.project_id, &record.issue_id)
            .join("workspace-sessions")
            .join(format!("{}.json", record.id));
        record.id = format!(
            "{}-c1gate-miss-{}",
            record.id,
            C1GATE_MISS_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        );
        write_json(
            &app_paths
                .issue_root(&record.project_id, &record.issue_id)
                .join("workspace-sessions")
                .join(format!("{}.json", record.id)),
            &record,
        )
        .expect("persist c1gate-miss session");
        std::fs::remove_file(&previous_session_path).expect("drop fixture session");
        engine.session = WorkspaceSession::from_record(record);
    }
    // 只落门快照与预算，故意不持久化 candidate source/IR/report refs。
    let mut record = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("session record");
    record.flow_kind = WorkItemPlanFlowKind::SingleCandidate;
    record.run_policy = RunPolicy::Interactive;
    record.review_rounds = 0;
    record.single_candidate_phase = Some(SingleCandidatePhase::Approval);
    record.status = WorkspaceSessionStatus::WaitingForHuman;
    record.human_gate_snapshot = Some(HumanGateSnapshot {
        candidate_recovery: None,
        findings: Vec::new(),
        repeated_fingerprints: Vec::new(),
        attempts_used: 0,
        manual_repairs_remaining: 2,
        trigger: HumanReason::NativeHumanRequired,
        resumable: true,
        accepted_feedback_turns: None,
    });
    let session_path = app_paths
        .issue_root(&record.project_id, &record.issue_id)
        .join("workspace-sessions")
        .join(format!("{}.json", record.id));
    write_json(&session_path, &record).expect("persist c1gate-miss gate session");
    let mut session = WorkspaceSession::from_record(record);
    session.stage = WorkspaceStage::HumanConfirm;
    session.session_status = WorkspaceSessionStatus::WaitingForHuman;
    let (event_tx, _event_rx) = mpsc::channel(64);
    let mut engine = WorkspaceEngine::new_persistent(
        Arc::new(CheckpointStore::new(root.path().join("c1gate-b-checkpoints"))),
        lifecycle.clone(),
        event_tx,
        session,
    );
    engine
        .enter_human_confirm(Some("C1 缺事实候选门".to_string()))
        .await;
    let stale_gate_id = engine
        .active_timeline_node_id()
        .expect("durable active gate node");

    // 旧 gate id 的恢复命令：零副作用拒绝（GATE_MISMATCH）。
    let mismatch = engine
        .recover_candidate_gate(CandidateRecoveryCommand {
            command_id: "cmd-c1gate-recovery-b-mismatch".to_string(),
            expected_gate_id: "timeline_node_stale".to_string(),
            action: CandidateRecoveryAction::Recover,
        })
        .await
        .expect_err("stale gate id must be rejected");
    assert!(mismatch.contains("GATE_MISMATCH"), "got: {mismatch}");

    // 恢复请求：needs_human，缺失事实可见。
    let recovery = engine
        .recover_candidate_gate(CandidateRecoveryCommand {
            command_id: "cmd-c1gate-recovery-b".to_string(),
            expected_gate_id: stale_gate_id.clone(),
            action: CandidateRecoveryAction::Recover,
        })
        .await
        .expect("assessment resolves");
    let CandidateRecoveryOutcome::NeedsHuman { facts } = &recovery else {
        panic!("expected needs_human recovery, got {recovery:?}");
    };
    assert!(!facts.complete);
    assert!(!facts.missing.is_empty());

    // 裸 approve：固定 fail-closed，不扣预算、不启动 compile/provider。
    let rejected = engine
        .handle_human_gate_termination(HumanGateCloseDecision::Approve)
        .await
        .expect_err("approve without complete snapshot must fail closed");
    assert!(
        rejected.contains("CANDIDATE_SNAPSHOT_INCOMPLETE"),
        "got: {rejected}"
    );
    let after = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("session after reject");
    assert_eq!(after.status, WorkspaceSessionStatus::WaitingForHuman);
    assert_eq!(
        after.single_candidate_phase,
        Some(SingleCandidatePhase::Approval)
    );
    assert_eq!(
        after.human_gate_snapshot.as_ref().unwrap().manual_repairs_remaining,
        2,
        "预算不动"
    );
    assert!(after.provider_start_ledger.is_empty());
    assert!(
        lifecycle
            .list_human_gate_turns(&after.id)
            .expect("turns")
            .is_empty()
    );

    // 同 command 重放：同一 durable needs_human 结果，不二次评估改写。
    let replayed = engine
        .recover_candidate_gate(CandidateRecoveryCommand {
            command_id: "cmd-c1gate-recovery-b".to_string(),
            expected_gate_id: stale_gate_id.clone(),
            action: CandidateRecoveryAction::Recover,
        })
        .await
        .expect("replay resolves");
    assert!(matches!(
        &replayed,
        CandidateRecoveryOutcome::Replayed { record, .. } if record.state
            == crate::product::models::OperationState::NeedsHuman
    ));
}
