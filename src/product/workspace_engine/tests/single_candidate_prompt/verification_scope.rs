// single_candidate_prompt 拆分（large_file_guard 1200 行上限）：verification
// scope fail-closed 校验测试从 mod.rs 移入；测试逻辑与断言零改动。

use super::*;

#[test]
fn verification_scope_missing_mechanical_report_fails_protocol_and_durably_marks_failed() {
    let (_tmp, _checkpoint_store, lifecycle, plan_id, mut engine) =
        super::super::make_work_item_plan_engine_with_draft_candidate(
            "verification_scope_missing_report",
        );
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
        super::super::make_work_item_plan_engine_with_draft_candidate(
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
        super::super::make_work_item_plan_engine_with_draft_candidate(
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
        super::super::make_work_item_plan_engine_with_draft_candidate(
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
        super::super::make_work_item_plan_engine_with_draft_candidate(
            "verification_scope_parser_error",
        );
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
        super::super::make_work_item_plan_engine_with_draft_candidate("scope_roundtrip_reconnect");
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
        super::super::make_work_item_plan_engine_with_draft_candidate(
            "initial_scope_in_verification",
        );
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
        super::super::make_work_item_plan_engine_with_draft_candidate(
            "verification_scope_in_initial",
        );
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
        super::super::make_work_item_plan_engine_with_draft_candidate(
            "verification_scope_invalid_digest",
        );
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
        super::super::make_work_item_plan_engine_with_draft_candidate(
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
        super::super::make_work_item_plan_engine_with_draft_candidate(
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
