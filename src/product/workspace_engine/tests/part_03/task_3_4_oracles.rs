use crate::product::json_store::write_json;
use crate::product::models::WorkspaceSessionStatus;
use crate::product::work_item_plan_compiler::{
    PlanCandidateIr, PlanCandidateItemIr, PlanCandidateMechanicalReport,
    WORK_ITEM_PLAN_COMPILER_VERSION,
};
use crate::product::work_item_plan_source_store::{
    PlanCandidateIrRecord, PlanCandidateMechanicalReportRecord, SourceRevisionRecord,
    WorkItemPlanSourceStore,
};
use crate::product::work_item_revision_store::InitialPlanPublicationPhase;
use crate::product::workspace_engine::compile::{
    CompileStores, InitialPlanCompileDurableContext, SingleCandidateCompileCheckpoint,
    execute_initial_plan_compile, prepare_initial_plan_compile,
};
use crate::product::workspace_engine::plan_projection::prepare_initial_plan_publication;
use crate::product::workspace_engine::types::WorkspaceStage;

#[test]
fn task_3_4_wrapper_and_pure_publication_journals_are_field_equal() {
    let (_tmp, lifecycle, plan_id, mut engine) =
        make_work_item_plan_engine_with_accepted_contract_drafts();
    let (tx, accepted_drafts) = prepare_initial_compile_transaction(
        &engine,
        &lifecycle,
        &plan_id,
        "compile_task_3_4_parity",
        "2026-08-27T00:00:00Z",
    );
    let input = initial_plan_compile_input_from_fixture(
        &engine,
        &lifecycle,
        &plan_id,
        &tx.compile_id,
        &tx.created_at,
    );
    let pure = prepare_initial_plan_compile(input, InitialPlanCompileDurableContext::legacy())
        .expect("pure preparation")
        .publication_input
        .expect("publication input");
    let pure = prepare_initial_plan_publication(pure).expect("pure journal");
    engine
        .work_item_plan_store()
        .expect("store")
        .put_compile_transaction(&tx)
        .expect("seed transaction");
    let outcome = engine
        .compile_initial_plan_revision(&accepted_drafts)
        .expect("wrapper compile");
    let wrapper = engine
        .revision_store()
        .get_initial_plan_publication_journal(
            "project_0001",
            "issue_0001",
            &plan_id,
            &tx.compile_id,
        )
        .expect("wrapper journal");
    assert_eq!(wrapper.project_id, pure.project_id);
    assert_eq!(wrapper.issue_id, pure.issue_id);
    assert_eq!(wrapper.plan_id, pure.plan_id);
    assert_eq!(wrapper.outline_version_ref, pure.outline_version_ref);
    assert_eq!(
        wrapper.active_draft_revision_ids,
        pure.active_draft_revision_ids
    );
    assert_eq!(wrapper.allocated_ids, pure.allocated_ids);
    assert_eq!(wrapper.artifact_fingerprint, pure.artifact_fingerprint);
    assert_eq!(wrapper.artifacts, pure.artifacts);
    assert_eq!(wrapper.phase, InitialPlanPublicationPhase::PlanActivated);
    assert_eq!(wrapper.error, None);
    assert_eq!(wrapper.created_at, pure.created_at);
    assert_eq!(wrapper.updated_at, pure.created_at);
    assert_eq!(wrapper.compile_id, tx.compile_id);
    assert_eq!(outcome.plan_revision.id, pure.artifacts.plan_revision.id);
}

#[test]
fn task_3_4_legacy_transaction_without_flow_kind_uses_legacy_continue_path() {
    let (_tmp, lifecycle, plan_id, engine) =
        make_work_item_plan_engine_with_accepted_contract_drafts();
    let (tx, _) = prepare_initial_compile_transaction(
        &engine,
        &lifecycle,
        &plan_id,
        "compile_legacy_json_without_flow_kind",
        "2026-08-27T00:00:00Z",
    );
    let mut json = serde_json::to_value(tx).expect("serialize transaction");
    json.as_object_mut()
        .expect("transaction object")
        .remove("flow_kind");
    let old: WorkItemPlanCompileTransaction = serde_json::from_value(json).expect("legacy JSON");
    assert_eq!(old.flow_kind, None);
    assert_eq!(old.effective_flow_kind(), WorkItemPlanFlowKind::Legacy);
}

#[test]
fn task_3_4_execute_io_failure_is_reported_before_publication() {
    let (_tmp, lifecycle, plan_id, engine) =
        make_work_item_plan_engine_with_accepted_contract_drafts();
    let input = initial_plan_compile_input_from_fixture(
        &engine,
        &lifecycle,
        &plan_id,
        "compile_execute_io_failure",
        "2026-08-27T00:00:00Z",
    );
    let prepared = prepare_initial_plan_compile(input, InitialPlanCompileDurableContext::legacy())
        .expect("prepare");
    let issue_root = lifecycle
        .app_paths()
        .issue_root("project_0001", "issue_0001");
    std::fs::write(issue_root.join("work_item_plan_compiles"), "blocked")
        .expect("block compile transaction directory");
    let stores = CompileStores {
        plan_store: engine.work_item_plan_store().expect("plan store"),
        revision_store: engine.revision_store(),
    };
    let error =
        execute_initial_plan_compile(&stores, prepared).expect_err("transaction IO failure");
    assert!(error.contains("save compile transaction failed"));
}

#[tokio::test(flavor = "current_thread")]
async fn task_3_4_legacy_transaction_json_without_flow_kind_continues_legacy_path() {
    let (_tmp, lifecycle, plan_id, mut engine) =
        make_work_item_plan_engine_with_accepted_contract_drafts();
    let (mut tx, _) = prepare_initial_compile_transaction(
        &engine,
        &lifecycle,
        &plan_id,
        "compile_legacy_json_continue",
        "2026-08-27T00:00:00Z",
    );
    tx.status = WorkItemPlanCompileStatus::RecoveryRequired;
    tx.failure_reason = Some("legacy JSON recovery fixture".to_string());
    let mut json = serde_json::to_value(&tx).expect("serialize transaction");
    json.as_object_mut()
        .expect("transaction object")
        .remove("flow_kind");
    let path = lifecycle
        .app_paths()
        .issue_root("project_0001", "issue_0001")
        .join("work_item_plan_compiles")
        .join(&plan_id)
        .join(format!("{}.json", tx.compile_id));
    write_json(&path, &json).expect("write legacy transaction JSON");
    engine
        .enter_work_item_plan_compile_recovery(Some("legacy JSON recovery fixture".to_string()))
        .await;
    let outcome = engine
        .handle_work_item_plan_compile_recovery_action(
            crate::web::workspace_ws_types::WorkItemPlanCompileRecoveryActionDto::Continue,
            None,
        )
        .await
        .expect("legacy Continue");
    assert_eq!(
        outcome,
        crate::product::workspace_engine::types::WorkItemPlanCompileRecoveryOutcome::HumanConfirm
    );
    let committed = engine
        .work_item_plan_store()
        .expect("store")
        .get_compile_transaction("project_0001", "issue_0001", &plan_id, &tx.compile_id)
        .expect("committed transaction");
    assert_eq!(committed.flow_kind, None);
    assert_eq!(
        committed.effective_flow_kind(),
        WorkItemPlanFlowKind::Legacy
    );
    assert_eq!(committed.status, WorkItemPlanCompileStatus::Committed);
}

#[tokio::test(flavor = "current_thread")]
async fn task_3_4_normal_and_recovery_artifact_versions_retain_compile_created_at() {
    let (_tmp, _lifecycle, plan_id, mut engine) =
        make_work_item_plan_engine_with_accepted_contract_drafts();
    let outcome = engine
        .run_work_item_plan_compile()
        .await
        .expect("normal compile");
    assert!(!outcome.work_items.is_empty());
    let tx = engine
        .work_item_plan_store()
        .expect("store")
        .list_compile_transactions("project_0001", "issue_0001", &plan_id)
        .expect("transactions")
        .into_iter()
        .next()
        .expect("transaction");
    for version in &engine.artifact_versions {
        assert_eq!(
            version.created_at, tx.created_at,
            "artifact version created_at"
        );
    }
}

#[test]
fn task_3_4_recovery_failure_records_durable_failed_diagnostic() {
    let (_tmp, lifecycle, plan_id, mut engine) =
        make_work_item_plan_engine_with_accepted_contract_drafts();
    let (mut tx, _) = prepare_initial_compile_transaction(
        &engine,
        &lifecycle,
        &plan_id,
        "compile_recovery_failure_diagnostic",
        "2026-08-27T00:00:00Z",
    );
    tx.flow_kind = Some(WorkItemPlanFlowKind::SingleCandidate);
    tx.status = WorkItemPlanCompileStatus::RecoveryRequired;
    tx.source_revision_ref = None;
    let store = engine.work_item_plan_store().expect("store");
    store.put_compile_transaction(&tx).expect("seed tx");
    engine.session.stage = WorkspaceStage::HumanConfirm;
    engine.session.session_status = WorkspaceSessionStatus::WaitingForHuman;
    let error = engine
        .validate_single_candidate_transaction_refs(&tx)
        .expect_err("missing source ref");
    let failure = lifecycle
        .get_workspace_session(&engine.session.session_id)
        .expect("session");
    assert!(error.contains("source ref"));
    assert_eq!(failure.status, WorkspaceSessionStatus::Failed);
    assert!(
        failure
            .policy_diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "single_candidate_recovery_failed")
    );
}

#[tokio::test(flavor = "current_thread")]
async fn task_3_4_single_candidate_crash_boundaries_reuse_durable_reservation() {
    for checkpoint in [
        SingleCandidateCompileCheckpoint::ApprovalPersisted,
        SingleCandidateCompileCheckpoint::ReservationPersisted,
        SingleCandidateCompileCheckpoint::ProvenancePersisted,
    ] {
        task_3_4_single_candidate_continue_uses_only_canonical_refs_and_binds_provenance(
            checkpoint,
        )
        .await;
    }
}

async fn task_3_4_single_candidate_continue_uses_only_canonical_refs_and_binds_provenance(
    checkpoint: SingleCandidateCompileCheckpoint,
) {
    let (_tmp, lifecycle, plan_id, mut engine) =
        make_work_item_plan_engine_with_accepted_contract_drafts();
    let source_store = WorkItemPlanSourceStore::new(lifecycle.app_paths());
    let source_text = "# immutable single candidate source\\n";
    let source_hash = hex::encode(Sha256::digest(source_text.as_bytes()));
    let mut source = SourceRevisionRecord {
        id: "source_single_candidate".to_string(),
        source: source_text.to_string(),
        source_revision_hash: source_hash.clone(),
        content_hash: String::new(),
    };
    source.content_hash = source.content_hash().expect("source content hash");
    let source_ref = source_store
        .put_source_revision("project_0001", "issue_0001", &plan_id, &source)
        .expect("source revision");
    let store = engine.work_item_plan_store().expect("plan store");
    let index = store
        .load_active_index("project_0001", "issue_0001", &plan_id)
        .expect("active index")
        .expect("active index exists");
    let outline = engine
        .latest_work_item_plan_outline_candidate()
        .expect("outline");
    let outline_order = work_item_plan_outline_topological_order(&outline.outline).expect("order");
    let drafts = engine
        .accepted_active_draft_records_for_compile(&store, &index, &outline_order)
        .expect("accepted drafts");
    let previous_plan = lifecycle
        .get_issue_work_item_plan("project_0001", "issue_0001", &plan_id)
        .expect("previous plan");
    let repository_id = engine
        .work_item_plan_repository_id(&lifecycle, &previous_plan)
        .expect("repository id");
    let mut ir = PlanCandidateIrRecord {
        id: "ir_single_candidate".to_string(),
        source_revision_id: source.id.clone(),
        ir: PlanCandidateIr {
            source_revision_hash: source_hash.clone(),
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
        .put_plan_candidate_ir("project_0001", "issue_0001", &plan_id, &ir)
        .expect("candidate IR");
    let mut report = PlanCandidateMechanicalReportRecord {
        id: "report_single_candidate".to_string(),
        source_revision_id: source.id.clone(),
        ir_id: ir.id.clone(),
        report: PlanCandidateMechanicalReport {
            source_revision_hash: source_hash,
            compiler_version: WORK_ITEM_PLAN_COMPILER_VERSION.to_string(),
            findings: Vec::new(),
        },
        content_hash: String::new(),
    };
    report.content_hash = report.content_hash().expect("report content hash");
    let report_ref = source_store
        .put_mechanical_report("project_0001", "issue_0001", &plan_id, &report)
        .expect("mechanical report");

    let mut record = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("session record");
    record.flow_kind = WorkItemPlanFlowKind::SingleCandidate;
    record.single_candidate_phase = Some(crate::product::models::SingleCandidatePhase::Approval);
    record.work_item_plan_source_revision_ref = Some(source_ref.clone());
    record.plan_candidate_ir_ref = Some(ir_ref.clone());
    record.mechanical_report_ref = Some(report_ref.clone());
    record.status = WorkspaceSessionStatus::WaitingForHuman;
    write_json(
        &lifecycle
            .app_paths()
            .issue_root("project_0001", "issue_0001")
            .join("workspace-sessions")
            .join(format!("{}.json", record.id)),
        &record,
    )
    .expect("persist candidate session");
    engine.session.flow_kind = WorkItemPlanFlowKind::SingleCandidate;
    engine.session.single_candidate_phase = record.single_candidate_phase.clone();
    engine.session.work_item_plan_source_revision_ref =
        record.work_item_plan_source_revision_ref.clone();
    engine.session.plan_candidate_ir_ref = record.plan_candidate_ir_ref.clone();
    engine.session.mechanical_report_ref = record.mechanical_report_ref.clone();
    engine.session.session_status = WorkspaceSessionStatus::WaitingForHuman;

    // 删除 legacy active-index/outline/draft 输入；SingleCandidate Continue 只能依赖 canonical refs。
    let _ = std::fs::remove_dir_all(
        lifecycle
            .app_paths()
            .issue_root("project_0001", "issue_0001")
            .join("work_item_plan_drafts")
            .join(&plan_id),
    );
    let _ = std::fs::remove_dir_all(
        lifecycle
            .app_paths()
            .issue_root("project_0001", "issue_0001")
            .join("work_item_plan_outlines")
            .join(&plan_id),
    );

    let approval_id = crate::product::lifecycle_store::single_candidate_approval_attempt_id(
        &record.id,
        &record.entity_id,
        record
            .work_item_plan_source_revision_ref
            .as_deref()
            .expect("source ref"),
        record.plan_candidate_ir_ref.as_deref().expect("IR ref"),
        record.mechanical_report_ref.as_deref().expect("report ref"),
    );
    let approved = lifecycle
        .compare_and_save_single_candidate_approval(&record, &approval_id, "2026-08-27T00:00:00Z")
        .expect("persist approval");
    let compile_id = crate::product::lifecycle_store::single_candidate_compile_id(
        &approved.id,
        &approved.entity_id,
        &approval_id,
        approved.approved_at.as_deref().expect("approved at"),
    );
    // 每个 failpoint 都位于首个 transaction put 之前；重启后应从 durable session 继续。
    let failpoint = engine.register_single_candidate_compile_failpoint(checkpoint);
    let provider_ledger_before = provider_ledger_bytes(&lifecycle);
    let compile_writes = crate::product::work_item_plan_store::observe_compile_transaction_writes();
    let first_error = {
        let _legacy_read_spy =
            crate::product::work_item_plan_store::panic_on_legacy_compile_reads();
        engine
            .run_work_item_plan_compile()
            .await
            .expect_err("single candidate crash failpoint interrupts compile")
    };
    drop(failpoint);
    assert!(first_error.contains("single_candidate_compile_failpoint"));
    assert!(compile_writes.snapshots().is_empty());
    assert_eq!(provider_ledger_bytes(&lifecycle), provider_ledger_before);
    assert!(matches!(
        engine
            .revision_store()
            .get_plan_lineage("project_0001", "issue_0001", &plan_id),
        Err(crate::product::json_store::ProductStoreError::NotFound { .. })
    ));
    assert!(matches!(
        engine
            .revision_store()
            .get_initial_plan_publication_journal(
                "project_0001",
                "issue_0001",
                &plan_id,
                &compile_id,
            ),
        Err(crate::product::json_store::ProductStoreError::NotFound { .. })
    ));
    let interrupted = engine
        .work_item_plan_store()
        .expect("plan store")
        .list_compile_transactions("project_0001", "issue_0001", &plan_id)
        .expect("transactions");
    assert!(interrupted.is_empty());
    let persisted_after_crash = lifecycle
        .get_workspace_session(&record.id)
        .expect("session after crash");
    match checkpoint {
        SingleCandidateCompileCheckpoint::ApprovalPersisted => {
            assert!(persisted_after_crash.approval_attempt_id.is_some());
            assert!(persisted_after_crash.compile_reservation.is_none());
        }
        SingleCandidateCompileCheckpoint::ReservationPersisted
        | SingleCandidateCompileCheckpoint::ProvenancePersisted => {
            assert!(persisted_after_crash.compile_reservation.is_some());
        }
    }
    let scope = crate::product::work_item_plan_source_store::SourceStoreScope {
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        plan_id: plan_id.clone(),
    };
    let expected_provenance_ref = format!(
        "project/project_0001/issue/issue_0001/plan/{plan_id}/publication_provenance/{compile_id}"
    );
    match checkpoint {
        SingleCandidateCompileCheckpoint::ApprovalPersisted
        | SingleCandidateCompileCheckpoint::ReservationPersisted => {
            assert!(matches!(
                source_store.get_publication_provenance(&scope, &expected_provenance_ref),
                Err(crate::product::work_item_plan_source_store::SourceStoreError::DanglingRef)
            ));
        }
        SingleCandidateCompileCheckpoint::ProvenancePersisted => {
            source_store
                .get_publication_provenance(&scope, &expected_provenance_ref)
                .expect("provenance must survive the provenance boundary");
        }
    }
    drop(compile_writes);
    let provider_ledger_before_restart = provider_ledger_before;
    let session_record = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("session for restart");
    let (event_tx, _event_rx) = tokio::sync::mpsc::channel(8);
    let mut restarted = WorkspaceEngine::new_persistent(
        engine.checkpoint_store.clone(),
        lifecycle.clone(),
        event_tx,
        WorkspaceSession::from_record(session_record),
    );
    let resumed_outcome = {
        let _legacy_read_spy =
            crate::product::work_item_plan_store::panic_on_legacy_compile_reads();
        restarted
            .run_work_item_plan_compile()
            .await
            .expect("restart resumes session recovery")
    };
    assert!(!resumed_outcome.work_items.is_empty());
    assert_eq!(
        provider_ledger_bytes(&lifecycle),
        provider_ledger_before_restart,
        "reservation/session recovery must not start a provider"
    );
    let transactions = restarted
        .work_item_plan_store()
        .expect("plan store")
        .list_compile_transactions("project_0001", "issue_0001", &plan_id)
        .expect("transactions");
    assert_eq!(transactions.len(), 1, "restart must not duplicate transaction");
    let tx = transactions.into_iter().next().expect("single transaction");
    assert_eq!(tx.compile_id, compile_id);
    assert_eq!(tx.created_at, "2026-08-27T00:00:00Z");
    let restarted_session = lifecycle
        .get_workspace_session(&restarted.session().session_id)
        .expect("restarted session");
    let reservation = restarted_session
        .compile_reservation
        .as_ref()
        .expect("durable reservation after restart");
    assert_eq!(reservation.compile_id, compile_id);
    assert_eq!(reservation.now, tx.created_at);
    assert_eq!(
        reservation.publication_provenance_ref,
        format!(
            "project/project_0001/issue/issue_0001/plan/{plan_id}/publication_provenance/{compile_id}"
        )
    );
    let journal = restarted
        .revision_store()
        .get_initial_plan_publication_journal(
            "project_0001",
            "issue_0001",
            &plan_id,
            &tx.compile_id,
        )
        .expect("publication journal");
    let provenance_ref = journal
        .artifacts
        .publication_provenance_ref
        .clone()
        .expect("journal provenance ref");
    let provenance = source_store
        .get_publication_provenance(
            &crate::product::work_item_plan_source_store::SourceStoreScope {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                plan_id: plan_id.clone(),
            },
            &provenance_ref,
        )
        .expect("provenance");
    assert_eq!(tx.flow_kind, Some(WorkItemPlanFlowKind::SingleCandidate));
    assert_eq!(
        tx.publication_provenance_ref.as_deref(),
        Some(provenance_ref.as_str())
    );
    assert_eq!(
        tx.publication_provenance_content_hash.as_deref(),
        Some(provenance.content_hash.as_str())
    );
    assert_eq!(
        journal
            .artifacts
            .publication_provenance_content_hash
            .as_deref(),
        Some(provenance.content_hash.as_str())
    );
    assert_eq!(
        journal
            .artifacts
            .plan_revision
            .publication_provenance_ref
            .as_deref(),
        Some(provenance_ref.as_str())
    );
    assert_eq!(
        provenance.plan_revision_id,
        journal.artifacts.plan_revision.id
    );
    let allocated = restarted
        .revision_store()
        .allocate_initial_plan_publication_ids(
            "project_0001",
            "issue_0001",
            &plan_id,
            &compile_id,
            &resumed_outcome
                .work_items
                .iter()
                .map(|item| item.work_item_revision.logical_work_item_id.clone())
                .collect::<Vec<_>>(),
        )
        .expect("allocated publication IDs");
    assert_eq!(journal.allocated_ids, allocated);
    assert_eq!(
        journal
            .artifacts
            .plan_revision
            .publication_provenance_ref
            .as_deref(),
        Some(provenance_ref.as_str())
    );

    // 模拟 publication 已写入但 finalizer 尚未完成的重启：Continue 必须重载
    // canonical refs、复用同一 transaction/provenance，而不能回退 legacy stores。
    let store = engine.work_item_plan_store().expect("recovery store");
    let mut interrupted = store
        .get_compile_transaction("project_0001", "issue_0001", &plan_id, &tx.compile_id)
        .expect("completed transaction");
    interrupted.status = WorkItemPlanCompileStatus::RecoveryRequired;
    interrupted.failure_reason = Some("single candidate publication recovery oracle".to_string());
    interrupted.step_cursor = "committing".to_string();
    store
        .put_compile_transaction(&interrupted)
        .expect("seed publication recovery boundary");
    engine
        .enter_work_item_plan_compile_recovery(Some(
            "single candidate publication recovery oracle".to_string(),
        ))
        .await;
    let session_record = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("recovery session");
    let (event_tx, _event_rx) = tokio::sync::mpsc::channel(8);
    let mut recovered = WorkspaceEngine::new_persistent(
        engine.checkpoint_store.clone(),
        lifecycle.clone(),
        event_tx,
        WorkspaceSession::from_record(session_record),
    );
    let publication_failpoint = recovered
        .revision_store()
        .register_initial_plan_publication_failpoint(
            "project_0001",
            "issue_0001",
            &plan_id,
            &tx.compile_id,
            crate::product::work_item_revision_store::InitialPlanPublicationCheckpoint::PlanArtifactsWritten,
        );
    let first_recovery_error = {
        let _legacy_read_spy =
            crate::product::work_item_plan_store::panic_on_legacy_compile_reads();
        recovered
            .handle_work_item_plan_compile_recovery_action(
                crate::web::workspace_ws_types::WorkItemPlanCompileRecoveryActionDto::Continue,
                None,
            )
            .await
            .expect_err("publication failpoint interrupts recovery")
    };
    assert!(first_recovery_error.contains("PlanArtifactsWritten"));
    drop(publication_failpoint);
    let recovery_session = lifecycle
        .get_workspace_session(&recovered.session().session_id)
        .expect("failed recovery session");
    let (event_tx, _event_rx) = tokio::sync::mpsc::channel(8);
    let mut recovered = WorkspaceEngine::new_persistent(
        engine.checkpoint_store.clone(),
        lifecycle.clone(),
        event_tx,
        WorkspaceSession::from_record(recovery_session),
    );
    let recovery_writes =
        crate::product::work_item_plan_store::observe_compile_transaction_writes();
    let recovery = {
        let _legacy_read_spy =
            crate::product::work_item_plan_store::panic_on_legacy_compile_reads();
        recovered
            .handle_work_item_plan_compile_recovery_action(
                crate::web::workspace_ws_types::WorkItemPlanCompileRecoveryActionDto::Continue,
                None,
            )
            .await
            .expect("single candidate Continue")
    };
    assert_eq!(
        recovery,
        crate::product::workspace_engine::types::WorkItemPlanCompileRecoveryOutcome::HumanConfirm
    );
    let recovered_tx = recovered
        .work_item_plan_store()
        .expect("recovery store")
        .get_compile_transaction("project_0001", "issue_0001", &plan_id, &tx.compile_id)
        .expect("recovered transaction");
    assert_eq!(recovered_tx.compile_id, tx.compile_id);
    assert_eq!(
        recovered_tx.flow_kind,
        Some(WorkItemPlanFlowKind::SingleCandidate)
    );
    assert_eq!(
        recovered_tx.publication_provenance_ref.as_deref(),
        Some(provenance_ref.as_str())
    );
    assert_eq!(
        recovered_tx.publication_provenance_content_hash.as_deref(),
        Some(provenance.content_hash.as_str())
    );
    assert_eq!(recovered_tx.step_cursor, "committed");
    assert!(
        recovery_writes
            .snapshots()
            .iter()
            .any(|snapshot| snapshot.step_cursor == "publication_resumed"),
        "resume path must persist publication_resumed through tx observer"
    );
    assert_eq!(
        provider_ledger_bytes(&lifecycle),
        provider_ledger_before_restart
    );
    let resumed_journal = recovered
        .revision_store()
        .get_initial_plan_publication_journal(
            "project_0001",
            "issue_0001",
            &plan_id,
            &tx.compile_id,
        )
        .expect("resumed journal");
    assert_eq!(
        resumed_journal
            .artifacts
            .publication_provenance_ref
            .as_deref(),
        Some(provenance_ref.as_str())
    );
    for version in &recovered.artifact_versions {
        assert_eq!(
            version.created_at, recovered_tx.created_at,
            "recovery artifact version created_at"
        );
    }
}

#[test]
fn task_3_4_transaction_flow_kind_is_explicit_for_single_candidate_publication() {
    let (_tmp, lifecycle, plan_id, engine) =
        make_work_item_plan_engine_with_accepted_contract_drafts();
    let (mut tx, _) = prepare_initial_compile_transaction(
        &engine,
        &lifecycle,
        &plan_id,
        "compile_explicit_flow_kind",
        "2026-08-27T00:00:00Z",
    );
    tx.flow_kind = Some(WorkItemPlanFlowKind::SingleCandidate);
    assert_eq!(
        tx.effective_flow_kind(),
        WorkItemPlanFlowKind::SingleCandidate
    );
    assert_eq!(tx.plan_commit_state, WorkItemPlanCommitState::NotStarted);
}
