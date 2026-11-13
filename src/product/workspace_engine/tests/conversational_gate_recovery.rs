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
    let action = recover_human_gate_turn(&turn(HumanGateTurnStatus::Reserved, 1), false, false)
        .expect("reserved turn should restart its unstarted provider attempt");
    assert_eq!(
        action,
        HumanGateRecoveryAction::ResumeSameTurn { next_attempt_no: 1 }
    );
}

#[test]
fn conversational_gate_recovery_waits_for_running_provider() {
    let action = recover_human_gate_turn(&turn(HumanGateTurnStatus::Running, 1), true, false)
        .expect("running provider should be waitable");
    assert_eq!(action, HumanGateRecoveryAction::WaitForProvider);
}

#[test]
fn conversational_gate_recovery_retries_dead_provider_on_same_turn() {
    let original = turn(HumanGateTurnStatus::Running, 1);
    let action =
        recover_human_gate_turn(&original, false, false).expect("dead provider should retry");
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
    let action =
        recover_human_gate_turn(&original, false, false).expect("dead provider should retry");
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
            member_targets: None,
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
            existing_work_item_ids: &[],
            enrollment_target: None,
            member_targets: None,
            member_checkout_prefixes: None,

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
        // SG3a.0：旧 fixture 形态（无结构报告）。
        structure_report: None,
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
            member_targets: None,
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
            existing_work_item_ids: &[],
            enrollment_target: None,
            member_targets: None,
            member_checkout_prefixes: None,
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
        // SG3a.0：旧 fixture 形态（无结构报告）。
        structure_report: None,
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
    let action = recover_human_gate_turn(&original, false, false)
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
    use crate::product::work_item_plan_policy::CandidateRecoveryAction;
    use crate::product::work_item_plan_policy::{
        HumanGateSnapshot, HumanReason, RunPolicy, WorkItemPlanFlowKind,
    };
    use crate::product::workspace_engine::{CandidateRecoveryCommand, CandidateRecoveryOutcome};
    use crate::product::workspace_engine::{
        HumanGateCloseDecision, WorkspaceEngine, WorkspaceSession, WorkspaceStage,
    };
    use crate::web::workspace_ws_types::ArtifactPayload;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};
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
        .await
        .expect("fixture artifact write");
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
        Arc::new(CheckpointStore::new(
            root.path().join("c1gate-a-checkpoints"),
        )),
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
        Arc::new(CheckpointStore::new(
            root.path().join("c1gate-b-checkpoints"),
        )),
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
        after
            .human_gate_snapshot
            .as_ref()
            .unwrap()
            .manual_repairs_remaining,
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

// ---------------------------------------------------------------------------
// SG6a.1 P2 修复轮（Main 审查裁定）：recovery 车道识别结构 claim 不续跑。
// 现场链：结构拒收已同 turn claim attempt2（活动 grant phase=
// Attempt2Claimed，冻结机器指令 prompt 身份在案）→ 会话重开
// recover_human_gate_turns 若按普通 Running+attempt_no<2 续跑，会以空
// prompt spawn 并重跑原反馈 prompt——唯一 attempt2 被错误消费，REQ-CG-03
// 机器指令驱动不成立。修后：AwaitingStructuralClosure（零写零 spawn），
// 终结收口归 SG6a.3。
// ---------------------------------------------------------------------------

#[tokio::test]
async fn sg_06a_1_recovery_lane_honors_structural_claim_without_resume() {
    use crate::product::json_store::write_json;
    use crate::product::models::workspace::{
        StructuralGovernanceState, StructuralGrantPhase, StructuralRevisionGrant,
    };
    use crate::product::models::{
        HumanGateReservation, SingleCandidatePhase, WorkspaceSessionStatus,
    };
    use crate::product::work_item_plan_policy::{HumanGateSnapshot, HumanReason};

    let (_tmp, lifecycle, _plan_id, mut engine) =
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
        resumable: true,
        accepted_feedback_turns: None,
    });
    // 无 current refs/审批残留：确定不进 cross-round refs fail-closed 臂。
    session.work_item_plan_source_revision_ref = None;
    session.plan_candidate_ir_ref = None;
    session.mechanical_report_ref = None;
    session.approval_attempt_id = None;
    session.approved_at = None;
    session.compile_reservation = None;
    session.publication_provenance_ref = None;
    let session_path = lifecycle
        .app_paths()
        .issue_root(&session.project_id, &session.issue_id)
        .join("workspace-sessions")
        .join(format!("{}.json", session.id));
    write_json(&session_path, &session).expect("persist human gate session");

    let now = "2026-10-14T00:00:00Z".to_string();
    let turn_id = "turn_sg6a1_p2_recovery".to_string();
    let command_id = "cmd_sg6a1_p2_recovery".to_string();
    let reserved_turn = HumanGateTurn {
        turn_id: turn_id.clone(),
        session_id: session.id.clone(),
        command_id: command_id.clone(),
        feedback_text: "结构拒收后的原反馈".to_string(),
        status: HumanGateTurnStatus::Reserved,
        attempt_no: 1,
        budget_reserved: 1,
        source_hash: "a".repeat(64),
        result_artifact_ref: None,
        failure_class: None,
        created_at: now.clone(),
        updated_at: now.clone(),
    };
    let reservation = HumanGateReservation {
        command_id: command_id.clone(),
        turn_id: turn_id.clone(),
        provider_start_idempotency_key: format!("human_gate:{turn_id}:attempt:1"),
        reserved_at: now,
    };
    let (reserved_session, _) = lifecycle
        .compare_and_reserve_human_gate_turn(&session, reserved_turn, reservation)
        .expect("reserve durable turn");
    let running_turn = HumanGateTurn {
        status: HumanGateTurnStatus::Running,
        ..lifecycle
            .get_human_gate_turn(&session.id, &turn_id)
            .expect("reserved turn")
    };
    lifecycle
        .update_human_gate_turn(&reserved_session, running_turn)
        .expect("mark running");

    // 注入活动 grant（真实 claim 后、终结收口前的耐久形状）：phase=
    // Attempt2Claimed、attempt 2、prompt/instruction 身份在案。
    let mut seeded = lifecycle
        .get_workspace_session(&session.id)
        .expect("reload for grant seed");
    let before_ledger_len = seeded.provider_start_ledger.len();
    let state = seeded
        .structural_governance_state
        .get_or_insert_with(StructuralGovernanceState::default);
    state.current_grant = Some(StructuralRevisionGrant {
        grant_id: format!("structural_grant_{turn_id}"),
        authorization_hash: "f".repeat(64),
        turn_id: Some(turn_id.clone()),
        command_id: Some(command_id),
        candidate_source_revision_ref: None,
        plan_candidate_ir_ref: None,
        mechanical_report_ref: None,
        phase: Some(StructuralGrantPhase::Attempt2Claimed),
        attempt_no: 2,
        prompt_ref: Some(format!("structural-prompt://{turn_id}")),
        prompt_digest: Some("d".repeat(64)),
        instruction_digest: Some("b".repeat(64)),
        failing_source_hash: Some("a".repeat(64)),
        review_identity: None,
        granted_at: "2026-10-14T00:00:00Z".to_string(),
    });
    write_json(&session_path, &seeded).expect("persist structural claim fixture");

    // 会话重开恢复（provider dead）：不 ResumeSameTurn——结构 claim 的
    // attempt2 属于冻结机器指令 prompt，不是空 prompt 的原反馈重跑。
    let actions = engine
        .recover_human_gate_turns(false)
        .expect("recovery must classify");
    assert_eq!(
        actions,
        vec![(
            turn_id.clone(),
            HumanGateRecoveryAction::AwaitingStructuralClosure
        )],
        "structurally-claimed turn must await SG6a.3 closure, not resume with the original feedback prompt"
    );
    // durable 零推进：turn 仍 Running/attempt 1，grant 原样，ledger 零新增。
    let after_turn = lifecycle
        .get_human_gate_turn(&session.id, &turn_id)
        .expect("turn after recovery");
    assert_eq!(after_turn.status, HumanGateTurnStatus::Running);
    assert_eq!(
        after_turn.attempt_no, 1,
        "recovery must not advance the attempt"
    );
    let after_session = lifecycle
        .get_workspace_session(&session.id)
        .expect("session after recovery");
    assert_eq!(
        after_session.provider_start_ledger.len(),
        before_ledger_len,
        "no provider start is spawned from the recovery lane"
    );
    let grant = after_session
        .structural_governance_state
        .and_then(|state| state.current_grant)
        .expect("grant untouched by recovery");
    assert_eq!(grant.phase, Some(StructuralGrantPhase::Attempt2Claimed));
    assert_eq!(grant.attempt_no, 2);

    // 判定面纯函数：claim 标志直接驱动 AwaitingStructuralClosure；provider
    // 在跑时仍先 WaitForProvider（attempt2 在飞）。
    let claimed_running = turn(HumanGateTurnStatus::Running, 1);
    assert_eq!(
        recover_human_gate_turn(&claimed_running, true, true).expect("classify"),
        HumanGateRecoveryAction::WaitForProvider
    );
    assert_eq!(
        recover_human_gate_turn(&claimed_running, false, true).expect("classify"),
        HumanGateRecoveryAction::AwaitingStructuralClosure
    );
    // 无 claim 的既有语义逐字不变。
    assert_eq!(
        recover_human_gate_turn(&claimed_running, false, false).expect("classify"),
        HumanGateRecoveryAction::ResumeSameTurn { next_attempt_no: 2 }
    );
}

// ---------------------------------------------------------------------------
// SG6b（plan v1.1 Task SG6b / REQ-CG-05 恢复分域 + SG6a.3⑧2/⑧3 移交）：
// recovery 按 phase 及原授权 hash/current 合法 hash 分域——Reviewing 重连
// 不 Completed/不 author（合法中间 candidate 不同原授权 hash 不得误
// Failed/误 Completed）；BlockedByLcRecovery（grant phase）优先；
// cancel 臂 hold 结算（Failed(cancelled) → nonstructural_failure_charge）。
// ---------------------------------------------------------------------------

/// Reviewing/BlockedByLcRecovery 分域的通用基座：真实 reserve（物化 hold）
/// + Running turn + 注入指定 phase 的活动 grant + current refs 轮换到真实
///   落盘的「已交付 candidate」（合法中间/交付 hash ≠ 原授权 hash 的现场
///   形态）。返回 (turn_id, session_path, delivered_source_ref)。
fn sg06b_structural_recovery_fixture(
    lifecycle: &crate::product::lifecycle_store::LifecycleStore,
    engine: &mut crate::product::workspace_engine::WorkspaceEngine,
    plan_id: &str,
    phase: crate::product::models::workspace::StructuralGrantPhase,
    turn_label: &str,
    turn_source_hash: &str,
) -> (String, std::path::PathBuf, String) {
    use crate::product::json_store::write_json;
    use crate::product::models::workspace::{StructuralGovernanceState, StructuralRevisionGrant};
    use crate::product::models::{
        HumanGateReservation, HumanGateTurnStatus, SingleCandidatePhase, WorkspaceSessionStatus,
    };
    use crate::product::work_item_plan_policy::{HumanGateSnapshot, HumanReason};
    use crate::product::work_item_plan_source_store::{
        SourceRevisionRecord, WorkItemPlanSourceStore,
    };
    use sha2::{Digest, Sha256};

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
        manual_repairs_remaining: 3,
        trigger: HumanReason::NativeHumanRequired,
        resumable: true,
        accepted_feedback_turns: None,
    });
    session.work_item_plan_source_revision_ref = None;
    session.plan_candidate_ir_ref = None;
    session.mechanical_report_ref = None;
    session.approval_attempt_id = None;
    session.approved_at = None;
    session.compile_reservation = None;
    session.publication_provenance_ref = None;
    let session_path = lifecycle
        .app_paths()
        .issue_root(&session.project_id, &session.issue_id)
        .join("workspace-sessions")
        .join(format!("{}.json", session.id));
    write_json(&session_path, &session).expect("persist gate session");

    let now = "2026-10-14T00:00:00Z".to_string();
    let turn_id = format!("turn_sg6b_{turn_label}");
    let command_id = format!("cmd_sg6b_{turn_label}");
    let reserved_turn = HumanGateTurn {
        turn_id: turn_id.clone(),
        session_id: session.id.clone(),
        command_id: command_id.clone(),
        feedback_text: "SG6b 恢复分域原反馈".to_string(),
        status: HumanGateTurnStatus::Reserved,
        attempt_no: 1,
        budget_reserved: 1,
        source_hash: turn_source_hash.to_string(),
        result_artifact_ref: None,
        failure_class: None,
        created_at: now.clone(),
        updated_at: now.clone(),
    };
    let reservation = HumanGateReservation {
        command_id,
        turn_id: turn_id.clone(),
        provider_start_idempotency_key: format!("human_gate:{turn_id}:attempt:1"),
        reserved_at: now,
    };
    let (reserved_session, _) = lifecycle
        .compare_and_reserve_human_gate_turn(&session, reserved_turn, reservation)
        .expect("reserve durable turn");
    let running_turn = HumanGateTurn {
        status: HumanGateTurnStatus::Running,
        ..lifecycle
            .get_human_gate_turn(&session.id, &turn_id)
            .expect("reserved turn")
    };
    lifecycle
        .update_human_gate_turn(&reserved_session, running_turn)
        .expect("mark running");

    // 已交付 candidate：真实 source store 落盘 + refs 轮换（合法中间/
    // 交付 hash，允许不同于原授权 turn.source_hash）。
    let delivered_source = format!("# SG6b delivered candidate {turn_label}\n\n## 交付正文\n");
    let delivered_hash = hex::encode(Sha256::digest(delivered_source.as_bytes()));
    let mut source = SourceRevisionRecord {
        id: format!("source-sg6b-{turn_label}"),
        source: delivered_source,
        source_revision_hash: delivered_hash.clone(),
        content_hash: String::new(),
    };
    source.content_hash = source.content_hash().expect("source content hash");
    let source_store = WorkItemPlanSourceStore::new(lifecycle.app_paths());
    let delivered_ref = source_store
        .put_source_revision(&session.project_id, &session.issue_id, plan_id, &source)
        .expect("persist delivered source");

    let mut seeded = lifecycle
        .get_workspace_session(&session.id)
        .expect("reload for grant seed");
    seeded.work_item_plan_source_revision_ref = Some(delivered_ref);
    let state = seeded
        .structural_governance_state
        .get_or_insert_with(StructuralGovernanceState::default);
    state.current_grant = Some(StructuralRevisionGrant {
        grant_id: format!("structural_grant_{turn_id}"),
        authorization_hash: "f".repeat(64),
        turn_id: Some(turn_id.clone()),
        command_id: Some(format!("cmd_sg6b_{turn_label}")),
        candidate_source_revision_ref: None,
        plan_candidate_ir_ref: None,
        mechanical_report_ref: None,
        phase: Some(phase),
        attempt_no: 2,
        prompt_ref: Some(format!("structural-prompt://{turn_id}")),
        prompt_digest: Some("d".repeat(64)),
        instruction_digest: Some("b".repeat(64)),
        failing_source_hash: Some("a".repeat(64)),
        review_identity: None,
        granted_at: "2026-10-14T00:00:00Z".to_string(),
    });
    write_json(&session_path, &seeded).expect("persist structural phase fixture");
    (turn_id, session_path, delivered_hash)
}

#[tokio::test]
async fn sg_06b_reviewing_reconnect_neither_completes_nor_authors() {
    use crate::product::models::workspace::StructuralGrantPhase;
    use crate::product::models::{HumanGateTurnStatus, WorkspaceSessionRecord};
    use crate::web::workspace_ws_types::{ArtifactPayload, ArtifactVersion};
    use sha2::{Digest, Sha256};

    let (_tmp, lifecycle, plan_id, mut engine) =
        super::make_work_item_plan_engine_with_accepted_contract_drafts();
    // 变体①（误 Failed 现场）：turn 原授权 hash ≠ 已交付 candidate hash。
    let (turn_id, _session_path, _delivered_hash) = sg06b_structural_recovery_fixture(
        &lifecycle,
        &mut engine,
        plan_id.as_str(),
        StructuralGrantPhase::Reviewing,
        "reviewing_drift",
        &"a".repeat(64),
    );
    let session_id = engine.session().session_id.clone();
    let before: WorkspaceSessionRecord = lifecycle
        .get_workspace_session(&session_id)
        .expect("session before recovery");
    let ledger_before = before.provider_start_ledger.clone();
    engine.session = super::WorkspaceSession::from_record(before.clone());

    let actions = engine
        .recover_human_gate_turns(false)
        .expect("recovery must classify");
    assert_eq!(
        actions,
        vec![(
            turn_id.clone(),
            HumanGateRecoveryAction::AwaitingStructuralClosure
        )],
        "Reviewing 重连不 Completed/不 author：verdict 面拥有收口，零写零 spawn"
    );
    let turn_after = lifecycle
        .get_human_gate_turn(&session_id, &turn_id)
        .expect("turn after recovery");
    assert_eq!(turn_after.status, HumanGateTurnStatus::Running);
    let after = lifecycle
        .get_workspace_session(&session_id)
        .expect("session after recovery");
    assert_eq!(after, before, "Reviewing 恢复零 durable 写");
    assert_eq!(after.provider_start_ledger, ledger_before);

    // 变体②（误 Completed 现场）：turn 原授权 hash == 已交付 candidate
    // hash + 晚于 turn 的 current artifact 版本——不得从 artifact 误
    // Completed（verdict 面拥有收口）。
    let (_tmp2, lifecycle2, plan2, mut engine2) =
        super::make_work_item_plan_engine_with_accepted_contract_drafts();
    let label = "reviewing_artifact";
    let delivered_source = format!("# SG6b delivered candidate {label}\n\n## 交付正文\n");
    let delivered_hash = hex::encode(Sha256::digest(delivered_source.as_bytes()));
    let (turn2, _session_path2, _delivered2) = sg06b_structural_recovery_fixture(
        &lifecycle2,
        &mut engine2,
        plan2.as_str(),
        StructuralGrantPhase::Reviewing,
        label,
        &delivered_hash,
    );
    let session2 = engine2.session().session_id.clone();
    lifecycle2
        .save_artifact_versions(
            &session2,
            &[
                ArtifactVersion {
                    version: 1,
                    payload: ArtifactPayload::Markdown {
                        markdown: "# old\n".to_string(),
                        diff: None,
                    },
                    generated_by: crate::product::models::ProviderName::Fake,
                    reviewed_by: None,
                    review_verdict: None,
                    confirmed_by: None,
                    is_current: false,
                    created_at: "2026-10-13T00:00:00Z".to_string(),
                    source_node_id: "node_sg6b_old".to_string(),
                },
                ArtifactVersion {
                    version: 2,
                    payload: ArtifactPayload::Markdown {
                        markdown: delivered_source,
                        diff: None,
                    },
                    generated_by: crate::product::models::ProviderName::Fake,
                    reviewed_by: None,
                    review_verdict: None,
                    confirmed_by: None,
                    is_current: true,
                    created_at: "2026-10-14T01:00:00Z".to_string(),
                    source_node_id: "node_sg6b_new".to_string(),
                },
            ],
        )
        .expect("persist artifact versions");
    let before2: WorkspaceSessionRecord = lifecycle2
        .get_workspace_session(&session2)
        .expect("session before artifact recovery");
    engine2.session = super::WorkspaceSession::from_record(before2.clone());
    let actions2 = engine2
        .recover_human_gate_turns(false)
        .expect("recovery must classify");
    assert_eq!(
        actions2,
        vec![(
            turn2.clone(),
            HumanGateRecoveryAction::AwaitingStructuralClosure
        )],
        "见 artifact 不误 Completed：Reviewing 的收口属于 verdict 面"
    );
    let turn2_after = lifecycle2
        .get_human_gate_turn(&session2, &turn2)
        .expect("turn after artifact recovery");
    assert_eq!(turn2_after.status, HumanGateTurnStatus::Running);

    // 判定面纯函数对照：provider 在跑（reviewer in-flight）→ WaitForProvider。
    let reviewing_running = turn(HumanGateTurnStatus::Running, 1);
    assert_eq!(
        recover_human_gate_turn(&reviewing_running, true, false).expect("classify"),
        HumanGateRecoveryAction::WaitForProvider
    );
}

#[tokio::test]
async fn sg_06b_recovery_blocked_by_lc_recovery_grant_phase_takes_priority() {
    use crate::product::models::WorkspaceSessionRecord;
    use crate::product::models::workspace::StructuralGrantPhase;

    let (_tmp, lifecycle, plan_id, mut engine) =
        super::make_work_item_plan_engine_with_accepted_contract_drafts();
    // anchor 漂移现场：current refs 指向已交付 candidate（hash ≠ 原
    // turn.source_hash）——若无优先级，source-drift 臂会误 Failed。
    let (turn_id, _session_path, _delivered_hash) = sg06b_structural_recovery_fixture(
        &lifecycle,
        &mut engine,
        plan_id.as_str(),
        StructuralGrantPhase::BlockedByLcRecovery,
        "lc_blocked_drift",
        &"a".repeat(64),
    );
    let session_id = engine.session().session_id.clone();
    let before: WorkspaceSessionRecord = lifecycle
        .get_workspace_session(&session_id)
        .expect("session before recovery");
    engine.session = super::WorkspaceSession::from_record(before.clone());

    let actions = engine
        .recover_human_gate_turns(false)
        .expect("recovery must classify");
    assert_eq!(
        actions,
        vec![(
            turn_id.clone(),
            HumanGateRecoveryAction::BlockedByLcRecovery {
                recovery_id: format!("structural_grant_{turn_id}")
            }
        )],
        "BlockedByLcRecovery（grant phase）优先：不 resume/不 MarkFailed/零写"
    );
    let after = lifecycle
        .get_workspace_session(&session_id)
        .expect("session after recovery");
    assert_eq!(after, before, "Blocked 优先级零 durable 写");
}

#[tokio::test]
async fn sg_06b_cancelled_turn_hold_settles_nonstructural_failure_charge() {
    use crate::product::models::workspace::StructuralGrantPhase;
    use crate::product::models::{HumanGateTurnStatus, UserBudgetReceiptKind};

    let (_tmp, lifecycle, plan_id, mut engine) =
        super::make_work_item_plan_engine_with_accepted_contract_drafts();
    let (turn_id, _session_path, _delivered_hash) = sg06b_structural_recovery_fixture(
        &lifecycle,
        &mut engine,
        plan_id.as_str(),
        StructuralGrantPhase::AuthorRunning,
        "cancelled_settle",
        &"a".repeat(64),
    );
    let session_id = engine.session().session_id.clone();
    // cancel 臂生产点在 handler（fail_human_gate_turn(Cancelled)）；fixture
    // 直写终态模拟「turn 已 Failed(cancelled) 而 hold 结算在崩溃窗内丢失」。
    let expected = lifecycle
        .get_workspace_session(&session_id)
        .expect("session");
    let mut cancelled = lifecycle
        .get_human_gate_turn(&session_id, &turn_id)
        .expect("turn");
    cancelled.status = HumanGateTurnStatus::Failed;
    cancelled.failure_class = Some(crate::product::models::HumanGateTurnFailureClass::Cancelled);
    cancelled.updated_at = chrono::Utc::now().to_rfc3339();
    lifecycle
        .update_human_gate_turn(&expected, cancelled)
        .expect("persist cancelled turn");
    engine.session = {
        let record = lifecycle
            .get_workspace_session(&session_id)
            .expect("reload");
        super::WorkspaceSession::from_record(record)
    };

    let actions = engine
        .recover_human_gate_turns(false)
        .expect("recovery entry");
    assert!(
        actions.is_empty(),
        "终态 turn 不进恢复分类（结算面单独收口），got {actions:?}"
    );
    let settled = lifecycle
        .get_workspace_session(&session_id)
        .expect("session after settlement");
    let state = settled.structural_governance_state.as_ref().expect("state");
    assert_eq!(
        state
            .user_receipts
            .iter()
            .filter(|receipt| receipt.turn_id == turn_id
                && receipt.kind == UserBudgetReceiptKind::NonstructuralFailureCharge)
            .count(),
        1,
        "REQ-CG-02：cancelled → nonstructural_failure_charge 恰一次"
    );
    assert_eq!(state.active_hold_count(), 0, "hold 已结算清空");
    assert_eq!(
        state.available_user_budget(),
        2,
        "非结构失败扣次不退：3 − 1 charge"
    );
    assert_eq!(
        settled
            .human_gate_snapshot
            .as_ref()
            .expect("snapshot")
            .manual_repairs_remaining,
        2,
        "快照可用预算同 CAS 扣减"
    );

    // 幂等：二次恢复零二次结算。
    let again = engine
        .recover_human_gate_turns(false)
        .expect("second recovery entry");
    assert!(again.is_empty());
    let settled_again = lifecycle
        .get_workspace_session(&session_id)
        .expect("session after second recovery");
    assert_eq!(settled_again, settled, "幂等：零二次结算/零写");
}
