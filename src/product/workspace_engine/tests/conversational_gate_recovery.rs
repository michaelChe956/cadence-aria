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
        findings: Vec::new(),
        repeated_fingerprints: Vec::new(),
        attempts_used: 0,
        manual_repairs_remaining: 2,
        trigger: HumanReason::NativeHumanRequired,
        resumable: false,
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
        findings: Vec::new(),
        repeated_fingerprints: Vec::new(),
        attempts_used: 0,
        manual_repairs_remaining: 2,
        trigger: HumanReason::NativeHumanRequired,
        resumable: false,
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
