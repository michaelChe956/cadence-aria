use crate::cross_cutting::streaming_provider::ProviderCompletion;
use crate::product::json_store::write_json;
use crate::product::models::{WorkItemSplitFinding, WorkspaceSessionStatus};
use crate::product::work_item_plan_compiler::{
    PlanCandidateIr, PlanCandidateMechanicalReport, WORK_ITEM_PLAN_COMPILER_VERSION,
};
use crate::product::work_item_plan_policy::{
    FatalReason, FindingClass, FindingFingerprint, HumanReason, ProviderStartLedgerEntry,
    ReviewCycleState, ReviewFindingCategory, ReviewInvocationScope, RunHistory,
    WorkItemPlanFlowKind,
};
use crate::product::work_item_plan_source_store::{
    PlanCandidateIrRecord, PlanCandidateMechanicalReportRecord, SourceRevisionRecord,
    WorkItemPlanSourceStore,
};
use crate::product::workspace_engine::review::policy_routing::RoutingAction;
use crate::web::workspace_ws_types::review::{
    ReviewFinding, ReviewFindingSeverity, ReviewGate, ReviewVerdict, ReviewVerdictType,
};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

fn persist_verification_scope(
    lifecycle: &crate::product::lifecycle_store::LifecycleStore,
    engine: &mut crate::product::workspace_engine::WorkspaceEngine,
    scope: ReviewInvocationScope,
) {
    persist_verification_scope_with_cycle(lifecycle, engine, scope, 0);
}

fn persist_single_candidate_scope(
    lifecycle: &crate::product::lifecycle_store::LifecycleStore,
    engine: &mut crate::product::workspace_engine::WorkspaceEngine,
    scope: ReviewInvocationScope,
    run_history: RunHistory,
) {
    let mut record = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("load persistent workspace session");
    record.flow_kind = WorkItemPlanFlowKind::SingleCandidate;
    record.review_invocation_scope = Some(scope.clone());
    if let ReviewInvocationScope::Verification {
        repaired_revision_id,
        mechanical_report_ref,
        ..
    } = &scope
    {
        record.plan_candidate_ir_ref = Some(repaired_revision_id.clone());
        record.mechanical_report_ref = Some(mechanical_report_ref.clone());
    }
    record.run_history = run_history;
    engine.session.flow_kind = WorkItemPlanFlowKind::SingleCandidate;
    engine.session.review_invocation_scope = Some(scope);
    engine.session.plan_candidate_ir_ref = record.plan_candidate_ir_ref.clone();
    engine.session.mechanical_report_ref = record.mechanical_report_ref.clone();
    engine.session.run_history = record.run_history.clone();
    write_json(
        &lifecycle
            .app_paths()
            .issue_root(&record.project_id, &record.issue_id)
            .join("workspace-sessions")
            .join(format!("{}.json", record.id)),
        &record,
    )
    .expect("persist single-candidate scope");
}

fn persist_verification_scope_with_cycle(
    lifecycle: &crate::product::lifecycle_store::LifecycleStore,
    engine: &mut crate::product::workspace_engine::WorkspaceEngine,
    scope: ReviewInvocationScope,
    verification_count: u32,
) {
    persist_single_candidate_scope(
        lifecycle,
        engine,
        scope,
        RunHistory {
            review_cycles: std::collections::BTreeMap::from([(
                "review:verification-node".to_string(),
                ReviewCycleState {
                    initial_count: 1,
                    verification_count,
                    ..ReviewCycleState::default()
                },
            )]),
            ..RunHistory::default()
        },
    );
}

fn source_hash(source: &str) -> String {
    hex::encode(Sha256::digest(source.as_bytes()))
}

fn persist_verification_artifacts(
    lifecycle: &crate::product::lifecycle_store::LifecycleStore,
    plan_id: &str,
) -> (String, String) {
    let source = "# immutable repaired Work Item Plan\n";
    let source_store = WorkItemPlanSourceStore::new(lifecycle.app_paths());
    let mut source_revision = SourceRevisionRecord {
        id: "source-001".to_string(),
        source: source.to_string(),
        source_revision_hash: source_hash(source),
        content_hash: String::new(),
    };
    source_revision.content_hash = source_revision.content_hash().expect("source hash");
    source_store
        .put_source_revision("project_0001", "issue_0001", plan_id, &source_revision)
        .expect("persist source revision");

    let mut ir = PlanCandidateIrRecord {
        id: "ir-001".to_string(),
        source_revision_id: source_revision.id.clone(),
        ir: PlanCandidateIr {
            source_revision_hash: source_revision.source_revision_hash.clone(),
            compiler_version: WORK_ITEM_PLAN_COMPILER_VERSION.to_string(),
            items: Vec::new(),
        },
        content_hash: String::new(),
    };
    ir.content_hash = ir.content_hash().expect("IR content hash");
    let ir_ref = source_store
        .put_plan_candidate_ir("project_0001", "issue_0001", plan_id, &ir)
        .expect("persist repaired IR");

    let mut report = PlanCandidateMechanicalReportRecord {
        id: "report-001".to_string(),
        source_revision_id: source_revision.id,
        ir_id: ir.id,
        report: PlanCandidateMechanicalReport {
            source_revision_hash: ir.ir.source_revision_hash,
            compiler_version: ir.ir.compiler_version,
            findings: Vec::<WorkItemSplitFinding>::new(),
        },
        content_hash: String::new(),
    };
    report.content_hash = report.content_hash().expect("report content hash");
    let report_ref = source_store
        .put_mechanical_report("project_0001", "issue_0001", plan_id, &report)
        .expect("persist mechanical report");

    (ir_ref, report_ref)
}

fn repairable_verdict(message: &str) -> ReviewVerdict {
    repairable_verdict_for_field(message, "contract.field")
}

fn repairable_verdict_for_field(message: &str, contract_field: &str) -> ReviewVerdict {
    ReviewVerdict {
        verdict: ReviewVerdictType::Revise,
        comments: "verification finding".to_string(),
        summary: "verification revise".to_string(),
        findings: vec![ReviewFinding {
            severity: ReviewFindingSeverity::MustFix,
            message: message.to_string(),
            evidence: "evidence".to_string(),
            required_action: "repair".to_string(),
            category: Some(ReviewFindingCategory::ContractGap),
            class_hint: None,
            contract_field: Some(contract_field.to_string()),
        }],
        review_gate: ReviewGate::RequiresRevision,
        work_item_plan_review: None,
        structured_output_diagnostic: None,
    }
}

fn fingerprint(message: &str) -> FindingFingerprint {
    FindingFingerprint::for_finding(
        Some(ReviewFindingCategory::ContractGap),
        FindingClass::Repairable,
        message,
        Some("contract.field"),
    )
}

fn assert_durable_protocol_failure(
    lifecycle: &crate::product::lifecycle_store::LifecycleStore,
    engine: &crate::product::workspace_engine::WorkspaceEngine,
) {
    assert_eq!(
        engine.session().session_status,
        WorkspaceSessionStatus::Failed
    );
    let persisted = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("persisted failed session");
    assert_eq!(persisted.status, WorkspaceSessionStatus::Failed);
    assert_eq!(
        persisted.policy_diagnostics[0].code,
        "verification_scope_violation"
    );
}

fn pass_verdict() -> ReviewVerdict {
    ReviewVerdict {
        verdict: ReviewVerdictType::Pass,
        comments: "verification complete".to_string(),
        summary: "verification pass".to_string(),
        findings: Vec::new(),
        review_gate: ReviewGate::UserConfirmAllowed,
        work_item_plan_review: None,
        structured_output_diagnostic: None,
    }
}

#[test]
fn single_candidate_initial_prompt_is_derived_from_server_scope() {
    let scope = ReviewInvocationScope::initial("revision-001");
    let instructions = crate::product::workspace_engine::review_scope_instructions(&scope)
        .expect("initial scope instructions");

    assert!(instructions.contains("Initial"));
    assert!(instructions.contains("revision-001"));
    assert!(instructions.contains("一次全候选评估"));
    for category in [
        ReviewFindingCategory::ContractGap,
        ReviewFindingCategory::SelfContradiction,
        ReviewFindingCategory::ScopeConflict,
        ReviewFindingCategory::VerificationUnattributable,
        ReviewFindingCategory::Completeness,
        ReviewFindingCategory::Other,
    ] {
        assert!(
            instructions.contains(category.as_str()),
            "initial review prompt must teach category whitelist value {}",
            category.as_str()
        );
    }
    assert!(instructions.contains("category 只能取以上六值之一；无法归类时用 other"));
    for class_hint in ["repairable", "human_required", "advisory"] {
        assert!(
            instructions.contains(class_hint),
            "initial review prompt must teach class_hint value {class_hint}"
        );
    }
    assert!(instructions.contains(
        "class_hint 只能取三值之一：repairable（可自动返修）、human_required（需人工裁决）、advisory（仅建议）"
    ));
    assert!(instructions.contains("must_fix"));
    assert!(instructions.contains("机械漏网硬错误或明确自相矛盾"));
    assert!(instructions.contains("advisory"));
    assert!(instructions.contains(scope.scope_digest()));
    assert!(!instructions.contains("review_invocation_scope"));
}

#[test]
fn single_candidate_verification_prompt_replays_only_original_fingerprints() {
    let fingerprint = FindingFingerprint::for_finding(
        Some(crate::product::work_item_plan_policy::ReviewFindingCategory::ContractGap),
        crate::product::work_item_plan_policy::FindingClass::Repairable,
        "original",
        Some("contract.field"),
    );
    let scope = ReviewInvocationScope::verification(
        BTreeSet::from([fingerprint.clone()]),
        "revision-002",
        "project/issue/plan/mechanical_report/report-002",
    );
    let instructions = crate::product::workspace_engine::review_scope_instructions(&scope)
        .expect("verification scope instructions");

    assert!(instructions.contains("Verification"));
    assert!(instructions.contains("revision-002"));
    assert!(instructions.contains("mechanical_report"));
    assert!(instructions.contains(fingerprint.0.as_str()));
    for category in [
        ReviewFindingCategory::ContractGap,
        ReviewFindingCategory::SelfContradiction,
        ReviewFindingCategory::ScopeConflict,
        ReviewFindingCategory::VerificationUnattributable,
        ReviewFindingCategory::Completeness,
        ReviewFindingCategory::Other,
    ] {
        assert!(
            instructions.contains(category.as_str()),
            "verification review prompt must teach category whitelist value {}",
            category.as_str()
        );
    }
    assert!(instructions.contains("category 只能取以上六值之一；无法归类时用 other"));
    for class_hint in ["repairable", "human_required", "advisory"] {
        assert!(
            instructions.contains(class_hint),
            "verification review prompt must teach class_hint value {class_hint}"
        );
    }
    assert!(instructions.contains(
        "class_hint 只能取三值之一：repairable（可自动返修）、human_required（需人工裁决）、advisory（仅建议）"
    ));
    assert!(instructions.contains("仅复核原 fingerprints"));
    assert!(instructions.contains("机械漏网硬错误或明确自相矛盾"));
    assert!(instructions.contains("advisory"));
    assert!(instructions.contains(scope.scope_digest()));
}

#[test]
fn single_candidate_scope_instructions_reject_invalid_digest() {
    let mut value = serde_json::to_value(ReviewInvocationScope::initial("revision-001")).unwrap();
    value["scope_digest"] = serde_json::Value::String("review_scope_v1:invalid".to_string());
    let scope = serde_json::from_value::<ReviewInvocationScope>(value);
    assert!(scope.is_err());
}

#[test]
fn single_candidate_scope_instructions_reject_empty_verification_report() {
    let scope = ReviewInvocationScope::verification(BTreeSet::new(), "revision-002", "");
    let error = crate::product::workspace_engine::review_scope_instructions(&scope)
        .expect_err("missing mechanical report must be fatal");
    assert!(error.contains("mechanical report"));
}

#[test]
fn verification_scope_missing_mechanical_report_fails_protocol_and_durably_marks_failed() {
    let (_tmp, _checkpoint_store, lifecycle, plan_id, mut engine) =
        super::make_work_item_plan_engine_with_draft_candidate("verification_scope_missing_report");
    let report_ref = format!(
        "project/project_0001/issue/issue_0001/plan/{plan_id}/mechanical_report/report-001"
    );
    persist_verification_scope(
        &lifecycle,
        &mut engine,
        ReviewInvocationScope::verification(BTreeSet::new(), "ir-001", report_ref),
    );

    let action = engine
        .work_item_policy_action("verification-node", &pass_verdict())
        .expect("verification policy action");

    assert!(
        matches!(
            action,
            RoutingAction::AbortFatal {
                reason: FatalReason::ProtocolViolation,
                ..
            }
        ),
        "expected verification protocol fatal, got {action:?}"
    );
    assert_durable_protocol_failure(&lifecycle, &engine);
}

#[test]
fn verification_scope_rejects_mismatched_mechanical_report_ref() {
    let (_tmp, _checkpoint_store, lifecycle, plan_id, mut engine) =
        super::make_work_item_plan_engine_with_draft_candidate(
            "verification_scope_wrong_report_ref",
        );
    let (ir_ref, report_ref) = persist_verification_artifacts(&lifecycle, &plan_id);
    let report_path = lifecycle
        .app_paths()
        .issue_root("project_0001", "issue_0001")
        .join("work-item-plan-sources")
        .join(&plan_id)
        .join("mechanical_report")
        .join("report-001.json");
    let mut report: PlanCandidateMechanicalReportRecord =
        crate::product::json_store::read_json(&report_path).expect("load report");
    report.content_hash = report
        .content_hash()
        .expect("recalculate report content hash");
    let wrong_report_ref = format!(
        "project/project_0001/issue/issue_0001/plan/{plan_id}/mechanical_report/report-002"
    );
    write_json(
        &report_path
            .parent()
            .expect("report parent")
            .join("report-002.json"),
        &report,
    )
    .expect("inject ref-to-record identity mismatch");
    assert_ne!(report_ref, wrong_report_ref);
    persist_verification_scope(
        &lifecycle,
        &mut engine,
        ReviewInvocationScope::verification(BTreeSet::new(), ir_ref, wrong_report_ref),
    );

    let action = engine
        .work_item_policy_action("verification-node", &pass_verdict())
        .expect("verification policy action");

    assert!(matches!(
        action,
        RoutingAction::AbortFatal {
            reason: FatalReason::ProtocolViolation,
            ..
        }
    ));
    assert_durable_protocol_failure(&lifecycle, &engine);
}

#[test]
fn verification_scope_rejects_report_version_mismatched_with_repaired_ir() {
    let (_tmp, _checkpoint_store, lifecycle, plan_id, mut engine) =
        super::make_work_item_plan_engine_with_draft_candidate(
            "verification_scope_report_version_mismatch",
        );
    let (ir_ref, report_ref) = persist_verification_artifacts(&lifecycle, &plan_id);
    let source_store = WorkItemPlanSourceStore::new(lifecycle.app_paths());
    let scope = crate::product::work_item_plan_source_store::SourceStoreScope {
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        plan_id: plan_id.clone(),
    };
    let mut report = source_store
        .get_mechanical_report(&scope, &report_ref)
        .expect("load valid report before mutation");
    report.report.compiler_version = "work-item-plan-compiler@wrong".to_string();
    report.content_hash = report
        .content_hash()
        .expect("recalculate report content hash");
    write_json(
        &lifecycle
            .app_paths()
            .issue_root("project_0001", "issue_0001")
            .join("work-item-plan-sources")
            .join(&plan_id)
            .join("mechanical_report")
            .join("report-001.json"),
        &report,
    )
    .expect("inject report version mismatch");
    persist_verification_scope(
        &lifecycle,
        &mut engine,
        ReviewInvocationScope::verification(BTreeSet::new(), ir_ref, report_ref),
    );

    let action = engine
        .work_item_policy_action("verification-node", &pass_verdict())
        .expect("verification policy action");

    assert!(matches!(
        action,
        RoutingAction::AbortFatal {
            reason: FatalReason::ProtocolViolation,
            ..
        }
    ));
    assert_durable_protocol_failure(&lifecycle, &engine);
}

#[test]
fn verification_scope_rejects_report_hash_mismatched_with_repaired_ir() {
    let (_tmp, _checkpoint_store, lifecycle, plan_id, mut engine) =
        super::make_work_item_plan_engine_with_draft_candidate(
            "verification_scope_report_hash_mismatch",
        );
    let (ir_ref, report_ref) = persist_verification_artifacts(&lifecycle, &plan_id);
    let source_store = WorkItemPlanSourceStore::new(lifecycle.app_paths());
    let scope = crate::product::work_item_plan_source_store::SourceStoreScope {
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        plan_id: plan_id.clone(),
    };
    let mut report = source_store
        .get_mechanical_report(&scope, &report_ref)
        .expect("load valid report before mutation");
    report.report.source_revision_hash = source_hash("different source");
    report.content_hash = report
        .content_hash()
        .expect("recalculate report content hash");
    write_json(
        &lifecycle
            .app_paths()
            .issue_root("project_0001", "issue_0001")
            .join("work-item-plan-sources")
            .join(&plan_id)
            .join("mechanical_report")
            .join("report-001.json"),
        &report,
    )
    .expect("inject report hash mismatch");
    persist_verification_scope(
        &lifecycle,
        &mut engine,
        ReviewInvocationScope::verification(BTreeSet::new(), ir_ref, report_ref),
    );

    let action = engine
        .work_item_policy_action("verification-node", &pass_verdict())
        .expect("verification policy action");

    assert!(matches!(
        action,
        RoutingAction::AbortFatal {
            reason: FatalReason::ProtocolViolation,
            ..
        }
    ));
    assert_durable_protocol_failure(&lifecycle, &engine);
}

#[tokio::test]
async fn verification_scope_parser_error_is_protocol_fatal_instead_of_needs_human_fallback() {
    let (_tmp, _checkpoint_store, lifecycle, plan_id, mut engine) =
        super::make_work_item_plan_engine_with_draft_candidate("verification_scope_parser_error");
    let (ir_ref, report_ref) = persist_verification_artifacts(&lifecycle, &plan_id);
    persist_verification_scope(
        &lifecycle,
        &mut engine,
        ReviewInvocationScope::verification(BTreeSet::new(), ir_ref, report_ref),
    );
    let completion = ProviderCompletion::plain("reviewer omitted structured output", None);

    let error = engine
        .parse_review_completion_for_active_node(&completion)
        .expect_err("verification parser failure must reject the invocation");

    assert_eq!(error.code(), "verification_scope_violation");
    let verdict = ReviewVerdict {
        verdict: ReviewVerdictType::NeedsHuman,
        comments: completion.readable_output.clone(),
        summary: "verification parser error".to_string(),
        findings: Vec::new(),
        review_gate: ReviewGate::UserTriageRequired,
        work_item_plan_review: None,
        structured_output_diagnostic: Some(
            crate::web::workspace_ws_types::review::StructuredOutputDiagnostic {
                code: error.code().to_string(),
                message: error.message(),
                repair_attempted: false,
                repair_succeeded: false,
                raw_output_preview: None,
            },
        ),
    };
    engine.complete_review(completion, verdict).await;

    assert_durable_protocol_failure(&lifecycle, &engine);
}

#[test]
fn single_candidate_scope_json_roundtrip_and_reconnect_preserve_digest() {
    let (_tmp, checkpoint_store, lifecycle, plan_id, mut engine) =
        super::make_work_item_plan_engine_with_draft_candidate("scope_roundtrip_reconnect");
    let (ir_ref, report_ref) = persist_verification_artifacts(&lifecycle, &plan_id);
    let scope = ReviewInvocationScope::verification(
        BTreeSet::from([fingerprint("original finding")]),
        ir_ref,
        report_ref,
    );
    let scope_json = serde_json::to_string(&scope).expect("serialize verification scope");
    let round_tripped_scope = serde_json::from_str::<ReviewInvocationScope>(&scope_json)
        .expect("deserialize verification scope");
    assert_eq!(round_tripped_scope, scope);
    assert_eq!(round_tripped_scope.scope_digest(), scope.scope_digest());
    persist_verification_scope(&lifecycle, &mut engine, scope.clone());

    let durable_record = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("load durable session through JSON store");
    assert_eq!(
        durable_record.review_invocation_scope.as_ref(),
        Some(&scope)
    );
    let (event_tx, _event_rx) = tokio::sync::mpsc::channel(8);
    let reconnected = crate::product::workspace_engine::WorkspaceEngine::new_persistent(
        checkpoint_store,
        lifecycle,
        event_tx,
        crate::product::workspace_engine::WorkspaceSession::from_record(durable_record),
    );

    let restored_scope = reconnected
        .session()
        .review_invocation_scope
        .as_ref()
        .expect("scope must survive engine reconstruction");
    assert_eq!(restored_scope, &scope);
    assert_eq!(restored_scope.scope_digest(), scope.scope_digest());
    restored_scope
        .validate_digest()
        .expect("reconnected scope digest must remain valid");
}

#[test]
fn single_candidate_scope_phase_violations_fail_closed_for_initial_and_verification() {
    let (_tmp, _checkpoint_store, lifecycle, _plan_id, mut engine) =
        super::make_work_item_plan_engine_with_draft_candidate("initial_scope_in_verification");
    persist_single_candidate_scope(
        &lifecycle,
        &mut engine,
        ReviewInvocationScope::initial("revision-001"),
        RunHistory {
            review_cycles: std::collections::BTreeMap::from([(
                "review:verification-node".to_string(),
                ReviewCycleState {
                    initial_count: 1,
                    ..ReviewCycleState::default()
                },
            )]),
            ..RunHistory::default()
        },
    );
    let action = engine
        .work_item_policy_action("verification-node", &pass_verdict())
        .expect("initial scope in verification must be routed");
    assert!(matches!(
        action,
        RoutingAction::AbortFatal {
            reason: FatalReason::ProtocolViolation,
            ..
        }
    ));
    assert_durable_protocol_failure(&lifecycle, &engine);

    let (_tmp, _checkpoint_store, lifecycle, _plan_id, mut engine) =
        super::make_work_item_plan_engine_with_draft_candidate("verification_scope_in_initial");
    persist_single_candidate_scope(
        &lifecycle,
        &mut engine,
        ReviewInvocationScope::verification(BTreeSet::new(), "ir-001", "report-001"),
        RunHistory::default(),
    );
    let action = engine
        .work_item_policy_action("initial-node", &pass_verdict())
        .expect("verification scope in initial must be routed");
    assert!(matches!(
        action,
        RoutingAction::AbortFatal {
            reason: FatalReason::ProtocolViolation,
            ..
        }
    ));
    assert_durable_protocol_failure(&lifecycle, &engine);
}

#[test]
fn verification_scope_rejects_invalid_scope_digest() {
    let (_tmp, _checkpoint_store, lifecycle, plan_id, mut engine) =
        super::make_work_item_plan_engine_with_draft_candidate("verification_scope_invalid_digest");
    let (ir_ref, report_ref) = persist_verification_artifacts(&lifecycle, &plan_id);
    let valid_scope = ReviewInvocationScope::verification(BTreeSet::new(), ir_ref, report_ref);
    let ReviewInvocationScope::Verification {
        repaired_revision_id,
        mechanical_report_ref,
        ..
    } = valid_scope
    else {
        unreachable!("verification scope")
    };
    persist_verification_scope(
        &lifecycle,
        &mut engine,
        ReviewInvocationScope::verification(
            BTreeSet::new(),
            repaired_revision_id.clone(),
            mechanical_report_ref.clone(),
        ),
    );
    engine.session.review_invocation_scope = Some(ReviewInvocationScope::Verification {
        original_fingerprints: BTreeSet::new(),
        repaired_revision_id,
        mechanical_report_ref,
        scope_digest: "review_scope_v1:invalid".to_string(),
    });

    let action = engine
        .work_item_policy_action("verification-node", &pass_verdict())
        .expect("verification policy action");

    assert!(matches!(
        action,
        RoutingAction::AbortFatal {
            reason: FatalReason::ProtocolViolation,
            ..
        }
    ));
    assert_durable_protocol_failure(&lifecycle, &engine);
}

#[test]
fn verification_scope_repeated_original_fingerprint_enters_human_without_second_repair() {
    let (_tmp, _checkpoint_store, lifecycle, plan_id, mut engine) =
        super::make_work_item_plan_engine_with_draft_candidate(
            "verification_scope_repeated_original_fingerprint",
        );
    let (ir_ref, report_ref) = persist_verification_artifacts(&lifecycle, &plan_id);
    let original = fingerprint("original finding");
    persist_verification_scope(
        &lifecycle,
        &mut engine,
        ReviewInvocationScope::verification(BTreeSet::from([original.clone()]), ir_ref, report_ref),
    );
    engine
        .session
        .run_history
        .seen_fingerprints
        .insert(original);
    engine.session.provider_start_ledger = vec![ProviderStartLedgerEntry {
        provider_start_idempotency_key: "repair-001".to_string(),
        started: true,
    }];
    let mut persisted = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("load persisted session");
    persisted.run_history = engine.session.run_history.clone();
    persisted.provider_start_ledger = engine.session.provider_start_ledger.clone();
    write_json(
        &lifecycle
            .app_paths()
            .issue_root(&persisted.project_id, &persisted.issue_id)
            .join("workspace-sessions")
            .join(format!("{}.json", persisted.id)),
        &persisted,
    )
    .expect("persist original fingerprint history");

    let action = engine
        .work_item_policy_action("verification-node", &repairable_verdict("original finding"))
        .expect("verification policy action");

    assert!(
        matches!(action, RoutingAction::EnterHumanGate { ref snapshot }
        if snapshot.trigger == HumanReason::RepeatedFingerprint)
    );
    assert_eq!(engine.session().provider_start_ledger.len(), 1);
    assert_eq!(
        lifecycle
            .get_workspace_session(&engine.session().session_id)
            .expect("persisted session")
            .provider_start_ledger
            .len(),
        1,
        "verification must not create a second automatic repair provider start"
    );
}

#[test]
fn verification_scope_new_fingerprint_requires_human_without_second_repair() {
    let (_tmp, _checkpoint_store, lifecycle, plan_id, mut engine) =
        super::make_work_item_plan_engine_with_draft_candidate(
            "verification_scope_new_fingerprint",
        );
    let (ir_ref, report_ref) = persist_verification_artifacts(&lifecycle, &plan_id);
    persist_verification_scope_with_cycle(
        &lifecycle,
        &mut engine,
        ReviewInvocationScope::verification(
            BTreeSet::from([fingerprint("original finding")]),
            ir_ref,
            report_ref,
        ),
        0,
    );

    let action = engine
        .work_item_policy_action(
            "verification-node",
            &repairable_verdict_for_field("new finding", "new.contract.field"),
        )
        .expect("verification policy action");

    assert!(
        matches!(action, RoutingAction::EnterHumanGate { ref snapshot }
            if snapshot.trigger == HumanReason::VerificationNewFindings),
        "new fingerprint must preserve evaluator reason, got {action:?}"
    );
    assert!(engine.session().provider_start_ledger.is_empty());
}
