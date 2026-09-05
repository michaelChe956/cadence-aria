use crate::cross_cutting::streaming_provider::ProviderCompletion;
use crate::product::json_store::write_json;
use crate::product::models::{WorkItemSplitFinding, WorkspaceSessionStatus};
use crate::product::work_item_plan_compiler::{
    PlanCandidateIr, PlanCandidateItemIr, PlanCandidateMechanicalReport,
    WORK_ITEM_PLAN_COMPILER_VERSION,
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
use crate::web::workspace_ws_types::{
    ProviderConfigSnapshot, TimelineNode, TimelineNodeStatus, TimelineNodeType,
    WorkspaceStage as WsWorkspaceStage,
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

#[tokio::test]
async fn single_candidate_scope_uses_one_reviewer_cycle_key_across_ensure_and_policy() {
    let (_tmp, _checkpoint_store, lifecycle, plan_id, mut engine) =
        super::make_work_item_plan_engine_with_draft_candidate("single_reviewer_cycle_key");
    let (ir_ref, report_ref) = persist_verification_artifacts(&lifecycle, &plan_id);
    persist_single_candidate_scope(
        &lifecycle,
        &mut engine,
        ReviewInvocationScope::initial(ir_ref.clone()),
        RunHistory {
            review_cycles: std::collections::BTreeMap::from([(
                "review:reviewer-node".to_string(),
                ReviewCycleState {
                    initial_count: 1,
                    ..ReviewCycleState::default()
                },
            )]),
            ..RunHistory::default()
        },
    );
    let mut record = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("load session");
    record.plan_candidate_ir_ref = Some(ir_ref.clone());
    record.mechanical_report_ref = Some(report_ref.clone());
    write_json(
        &lifecycle
            .app_paths()
            .issue_root(&record.project_id, &record.issue_id)
            .join("workspace-sessions")
            .join(format!("{}.json", record.id)),
        &record,
    )
    .expect("persist durable verification refs");
    engine.session.plan_candidate_ir_ref = Some(ir_ref);
    engine.session.mechanical_report_ref = Some(report_ref);
    engine.active_node_id = Some("reviewer-node".to_string());
    engine.timeline_nodes.push(TimelineNode {
        node_id: "reviewer-node".to_string(),
        node_type: TimelineNodeType::WorkItemBatchReview,
        agent: None,
        stage: WsWorkspaceStage::Running,
        round: Some(2),
        status: TimelineNodeStatus::Active,
        title: "reviewer run".to_string(),
        summary: None,
        started_at: "2026-08-08T00:00:00Z".to_string(),
        completed_at: None,
        duration_ms: None,
        artifact_ref: None,
        provider_config_snapshot: ProviderConfigSnapshot {
            author: crate::product::models::ProviderName::ClaudeCode,
            reviewer: Some(crate::product::models::ProviderName::KimiCode),
            review_rounds: 2,
            permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
        },
        retry: None,
    });

    engine
        .ensure_review_invocation_scope()
        .await
        .expect("ensure must construct verification scope");
    let action = engine
        .work_item_policy_action("reviewer-node", &pass_verdict())
        .expect("policy action");
    assert!(
        !matches!(
            action,
            RoutingAction::AbortFatal {
                reason: FatalReason::ProtocolViolation,
                ..
            }
        ),
        "ensure and policy must share the reviewer cycle key, got {action:?}"
    );
}

#[tokio::test]
async fn ensure_materializes_verification_scope_from_durable_same_node_cycle() {
    let (_tmp, _checkpoint_store, lifecycle, plan_id, mut engine) =
        super::make_work_item_plan_engine_with_draft_candidate("durable_verification_scope");
    let (repaired_ir_ref, report_ref) = persist_verification_artifacts(&lifecycle, &plan_id);
    let original_fingerprint = fingerprint("original repair finding");
    persist_single_candidate_scope(
        &lifecycle,
        &mut engine,
        ReviewInvocationScope::initial("first-review-ir"),
        RunHistory {
            seen_fingerprints: BTreeSet::from([original_fingerprint.clone()]),
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
    let mut durable = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("load durable session");
    durable.plan_candidate_ir_ref = Some(repaired_ir_ref.clone());
    durable.mechanical_report_ref = Some(report_ref.clone());
    write_json(
        &lifecycle
            .app_paths()
            .issue_root(&durable.project_id, &durable.issue_id)
            .join("workspace-sessions")
            .join(format!("{}.json", durable.id)),
        &durable,
    )
    .expect("persist repaired durable artifacts");

    // Simulate the stale worker that performed the first review: it has neither the
    // repaired refs nor this node's durable initial count in memory.
    engine.session.run_history = RunHistory::default();
    engine.session.plan_candidate_ir_ref = Some("first-review-ir".to_string());
    engine.session.mechanical_report_ref = None;
    engine.active_node_id = Some("verification-node".to_string());

    engine
        .ensure_review_invocation_scope()
        .await
        .expect("ensure must derive Verification from the durable node cycle");

    let expected_scope = ReviewInvocationScope::verification(
        BTreeSet::from([original_fingerprint]),
        repaired_ir_ref,
        report_ref,
    );
    assert_eq!(
        engine.session().review_invocation_scope,
        Some(expected_scope.clone())
    );
    expected_scope
        .validate_digest()
        .expect("verification scope digest must be valid");
    let action = engine
        .work_item_policy_action("verification-node", &pass_verdict())
        .expect("policy action");
    assert!(
        !matches!(action, RoutingAction::AbortFatal { .. }),
        "durable Verification materialization must prevent a phase mismatch: {action:?}"
    );
}

#[tokio::test]
async fn single_candidate_verification_scope_requires_durable_mechanical_report_at_ensure() {
    let (_tmp, _checkpoint_store, lifecycle, _plan_id, mut engine) =
        super::make_work_item_plan_engine_with_draft_candidate(
            "verification_scope_ensure_missing_report",
        );
    persist_single_candidate_scope(
        &lifecycle,
        &mut engine,
        ReviewInvocationScope::initial("initial-ir"),
        RunHistory {
            review_cycles: std::collections::BTreeMap::from([(
                "review:reviewer-node".to_string(),
                ReviewCycleState {
                    initial_count: 1,
                    ..ReviewCycleState::default()
                },
            )]),
            ..RunHistory::default()
        },
    );
    let mut record = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("load session");
    record.plan_candidate_ir_ref = Some("ir-001".to_string());
    record.mechanical_report_ref = None;
    write_json(
        &lifecycle
            .app_paths()
            .issue_root(&record.project_id, &record.issue_id)
            .join("workspace-sessions")
            .join(format!("{}.json", record.id)),
        &record,
    )
    .expect("clear durable report ref");
    engine.session.mechanical_report_ref = None;
    engine.active_node_id = Some("reviewer-node".to_string());

    let error = engine
        .ensure_review_invocation_scope()
        .await
        .expect_err("Verification scope must not be constructed without a durable report");
    assert!(error.contains("verification review requires a durable mechanical report"));
    assert!(matches!(
        engine.session().review_invocation_scope,
        Some(ReviewInvocationScope::Initial { .. })
    ));
}

#[tokio::test]
async fn repaired_review_upgrades_initial_scope_from_durable_session_report_ref() {
    let (_tmp, _checkpoint_store, lifecycle, plan_id, mut engine) =
        super::make_work_item_plan_engine_with_draft_candidate("verification_scope_upgrade");
    let (ir_ref, report_ref) = persist_verification_artifacts(&lifecycle, &plan_id);
    persist_single_candidate_scope(
        &lifecycle,
        &mut engine,
        ReviewInvocationScope::initial(ir_ref.clone()),
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
    let mut record = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("load session");
    record.plan_candidate_ir_ref = Some(ir_ref);
    record.mechanical_report_ref = Some(report_ref.clone());
    write_json(
        &lifecycle
            .app_paths()
            .issue_root(&record.project_id, &record.issue_id)
            .join("workspace-sessions")
            .join(format!("{}.json", record.id)),
        &record,
    )
    .expect("persist durable mechanical report ref");
    engine.session.plan_candidate_ir_ref = record.plan_candidate_ir_ref.clone();
    engine.session.mechanical_report_ref = Some(report_ref.clone());
    engine.active_node_id = Some("verification-node".to_string());

    engine
        .ensure_review_invocation_scope()
        .await
        .expect("verification scope upgrades from session report ref");

    assert!(matches!(
        engine.session().review_invocation_scope,
        Some(ReviewInvocationScope::Verification {
            ref mechanical_report_ref,
            ..
        }) if mechanical_report_ref == &report_ref
    ));
    let action = engine
        .work_item_policy_action("verification-node", &pass_verdict())
        .expect("verification policy invocation matches upgraded scope");
    assert!(!matches!(
        action,
        RoutingAction::AbortFatal {
            reason: FatalReason::ProtocolViolation,
            ..
        }
    ));
}

mod review_prompt;
mod verification_scope;
