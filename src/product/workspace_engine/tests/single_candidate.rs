use super::*;
use crate::cross_cutting::streaming_provider::ProviderCompletion;
use crate::product::json_store::write_json;
use crate::product::models::{SingleCandidatePhase, WorkItemSplitFinding, WorkspaceSessionStatus};
use crate::product::work_item_plan_compiler::{
    PlanCandidateIr, PlanCandidateItemIr, PlanCandidateMechanicalReport,
    WORK_ITEM_PLAN_COMPILER_VERSION,
};
use crate::product::work_item_plan_policy::{
    FindingClassHint, HumanReason, ReviewCycleState, ReviewFindingCategory, ReviewInvocationScope,
    RunBudgets, RunHistory, RunPolicy, WorkItemPlanFlowKind,
};
use crate::product::work_item_plan_source_store::{
    PlanCandidateIrRecord, PlanCandidateMechanicalReportRecord, SourceRevisionRecord,
    WorkItemPlanSourceStore,
};
use crate::web::workspace_ws_types::{
    ReviewFinding, ReviewFindingSeverity, ReviewGate, ReviewVerdict, ReviewVerdictType,
};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

use crate::product::workspace_engine::review::policy_routing::RoutingAction;

pub(super) fn single_candidate_record(
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
    .expect("persist candidate session");
    let artifact = engine.session.artifact.clone();
    engine.session = WorkspaceSession::from_record(record);
    engine.session.artifact = artifact;
    let refs = persist_candidate_artifacts(lifecycle, engine, "initial");
    update_durable_candidate_refs(lifecycle, engine, phase, refs);
}

fn pass_verdict() -> ReviewVerdict {
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

fn repairable_verdict(message: &str) -> ReviewVerdict {
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

fn mechanical_report_ref(engine: &WorkspaceEngine) -> &str {
    engine
        .session()
        .mechanical_report_ref
        .as_deref()
        .expect("durable mechanical report ref")
}

fn persist_candidate_artifacts(
    lifecycle: &LifecycleStore,
    engine: &WorkspaceEngine,
    suffix: &str,
) -> (String, String, String) {
    let project_id = &engine.session().project_id;
    let issue_id = &engine.session().issue_id;
    let plan_id = &engine.session().entity_id;
    let source_store = WorkItemPlanSourceStore::new(lifecycle.app_paths());
    let source_text = format!("# immutable single candidate source {suffix}\\n");
    let source_hash = hex::encode(Sha256::digest(source_text.as_bytes()));
    let mut source = SourceRevisionRecord {
        id: format!("source-{suffix}"),
        source: source_text,
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

fn update_durable_candidate_refs(
    lifecycle: &LifecycleStore,
    engine: &mut WorkspaceEngine,
    phase: SingleCandidatePhase,
    refs: (String, String, String),
) {
    let mut record = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("load candidate session");
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
    .expect("persist candidate refs");
    let artifact = engine.session.artifact.clone();
    engine.session = WorkspaceSession::from_record(record);
    engine.session.artifact = artifact;
}

fn complete_repair_generation(
    lifecycle: &LifecycleStore,
    engine: &mut WorkspaceEngine,
    suffix: &str,
) {
    let refs = persist_candidate_artifacts(lifecycle, engine, suffix);
    let expected = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("reload repairing session");
    let generated = lifecycle
        .compare_and_save_single_candidate_generation(&expected, &refs.0, &refs.1)
        .expect("persist repaired source and IR refs");
    let saved = lifecycle
        .compare_and_save_single_candidate_evaluation(&generated, &refs.2)
        .expect("persist repaired mechanical report ref");
    let artifact = engine.session.artifact.clone();
    engine.session = WorkspaceSession::from_record(saved);
    engine.session.artifact = artifact;
}

async fn complete_single_candidate_review(engine: &mut WorkspaceEngine, verdict: ReviewVerdict) {
    engine
        .complete_review(
            ProviderCompletion::plain("review".to_string(), None),
            verdict,
        )
        .await;
}

mod internal_generation_mode {
    use super::*;
    use crate::product::models::ProviderName;
    use crate::web::workspace_ws_types::WorkItemGenerationModeDto;

    fn generation_input(
        provider: ProviderName,
        candidate_item_count: usize,
    ) -> SingleCandidateGenerationDecisionInput {
        SingleCandidateGenerationDecisionInput {
            provider,
            candidate_item_count,
        }
    }

    #[test]
    fn internal_generation_mode_uses_compiled_ir_item_count_and_provider_profile() {
        let source = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/product/work_item_plan_compiler/fixtures/work-item-plan-rep4.md"
        ));
        let ir = crate::product::work_item_plan_compiler::compile_work_item_plan(
            source,
            &crate::product::work_item_plan_compiler::WorkItemPlanSourceContext {
                target_repository_id: "repo-levels".to_string(),
            },
        )
        .expect("rep4 source must compile to IR");
        assert_eq!(ir.items.len(), 3);
        assert_eq!(
            select_internal_generation_mode(&generation_input(ProviderName::Codex, ir.items.len())),
            WorkItemGenerationModeDto::Batch,
            "three compiled items with the Codex profile remain batch-diagnostic eligible"
        );
        assert_eq!(
            select_internal_generation_mode(&generation_input(ProviderName::Pi, ir.items.len())),
            WorkItemGenerationModeDto::Serial,
            "the Pi provider profile remains conservatively serial"
        );
        assert_eq!(
            select_internal_generation_mode(&generation_input(ProviderName::Codex, 4)),
            WorkItemGenerationModeDto::Serial,
            "four compiled items must be serial regardless of profile"
        );
    }

    #[test]
    fn internal_generation_mode_is_deterministic_for_identical_input() {
        let input = generation_input(ProviderName::Codex, 3);
        assert_eq!(
            select_internal_generation_mode(&input),
            select_internal_generation_mode(&input)
        );
    }
}

mod phase_machine {
    use super::*;

    #[tokio::test]
    async fn auto_valid_advances_evaluate_approval_completed_without_legacy_route() {
        let (_tmp, lifecycle, _plan_id, mut engine) =
            make_work_item_plan_engine_with_accepted_contract_drafts();
        single_candidate_record(
            &lifecycle,
            &mut engine,
            SingleCandidatePhase::Evaluate,
            RunPolicy::AutoIfValid,
        );

        complete_single_candidate_review(&mut engine, pass_verdict()).await;

        let persisted = lifecycle
            .get_workspace_session(&engine.session().session_id)
            .expect("load persisted session");
        assert_eq!(
            persisted.single_candidate_phase,
            Some(SingleCandidatePhase::Completed),
            "status={:?}, diagnostics={:#?}, messages={:#?}",
            persisted.status,
            persisted.policy_diagnostics,
            engine.session().messages,
        );
        assert_eq!(persisted.status, WorkspaceSessionStatus::Confirmed);
        assert!(persisted.approval_attempt_id.is_some());
        assert!(persisted.approved_at.is_some());
        assert!(persisted.compile_reservation.is_some());
    }

    #[tokio::test]
    async fn interactive_approval_confirmation_compiles_and_reaches_completed() {
        let (_tmp, lifecycle, plan_id, mut engine) =
            make_work_item_plan_engine_with_accepted_contract_drafts();
        single_candidate_record(
            &lifecycle,
            &mut engine,
            SingleCandidatePhase::Evaluate,
            RunPolicy::Interactive,
        );

        complete_single_candidate_review(&mut engine, pass_verdict()).await;
        assert_eq!(engine.session().stage, WorkspaceStage::HumanConfirm);
        assert_eq!(
            engine.session().single_candidate_phase,
            Some(SingleCandidatePhase::Approval)
        );

        let outcome = engine
            .handle_confirm()
            .await
            .expect("interactive Approval confirmation compiles and confirms");
        assert_eq!(outcome, WorkspaceConfirmOutcome::None);
        let persisted = lifecycle
            .get_workspace_session(&engine.session().session_id)
            .expect("load completed session");
        assert_eq!(engine.session().stage, WorkspaceStage::Completed);
        assert_eq!(
            persisted.single_candidate_phase,
            Some(SingleCandidatePhase::Completed)
        );
        assert_eq!(persisted.status, WorkspaceSessionStatus::Confirmed);
        assert_eq!(
            lifecycle
                .get_issue_work_item_plan("project_0001", "issue_0001", &plan_id)
                .expect("confirmed plan")
                .status,
            crate::product::models::IssueWorkItemPlanStatus::Confirmed
        );
        let transaction_count = engine
            .work_item_plan_store()
            .expect("plan store")
            .list_compile_transactions("project_0001", "issue_0001", &engine.session().entity_id)
            .expect("compile transactions")
            .len();
        assert_eq!(transaction_count, 1, "confirmation compiles exactly once");

        let replay = engine
            .handle_confirm()
            .await
            .expect("Completed confirmation is absorbing");
        assert_eq!(replay, WorkspaceConfirmOutcome::None);
        assert_eq!(
            engine
                .work_item_plan_store()
                .expect("plan store")
                .list_compile_transactions(
                    "project_0001",
                    "issue_0001",
                    &engine.session().entity_id
                )
                .expect("compile transactions")
                .len(),
            transaction_count,
            "terminal confirmation must not run compile again"
        );
    }

    #[tokio::test]
    async fn ensure_reconciles_stale_memory_scope_from_durable_candidate_and_cycle() {
        let (_tmp, lifecycle, _plan_id, mut engine) =
            make_work_item_plan_engine_with_accepted_contract_drafts();
        single_candidate_record(
            &lifecycle,
            &mut engine,
            SingleCandidatePhase::Evaluate,
            RunPolicy::Interactive,
        );
        let repaired_refs = persist_candidate_artifacts(&lifecycle, &engine, "r23-repaired");
        update_durable_candidate_refs(
            &lifecycle,
            &mut engine,
            SingleCandidatePhase::Evaluate,
            repaired_refs,
        );
        let durable_candidate = lifecycle
            .get_workspace_session(&engine.session().session_id)
            .expect("load durable candidate");
        let current_ir_ref = durable_candidate
            .plan_candidate_ir_ref
            .clone()
            .expect("durable candidate IR");
        let report_ref = durable_candidate
            .mechanical_report_ref
            .clone()
            .expect("durable mechanical report");
        let mut durable = durable_candidate;
        durable.review_invocation_scope = Some(ReviewInvocationScope::verification(
            BTreeSet::new(),
            current_ir_ref.clone(),
            report_ref,
        ));
        durable.run_history = RunHistory {
            review_cycles: std::collections::BTreeMap::from([(
                "review:new-reviewer-node".to_string(),
                ReviewCycleState::default(),
            )]),
            ..RunHistory::default()
        };
        write_json(
            &lifecycle
                .app_paths()
                .issue_root(&durable.project_id, &durable.issue_id)
                .join("workspace-sessions")
                .join(format!("{}.json", durable.id)),
            &durable,
        )
        .expect("persist durable current candidate and new-node cycle");

        // Simulate r23: the worker retained a round-one scope and candidate while the
        // durable session has the current candidate/report and a different reviewer node.
        engine.session.review_invocation_scope = Some(ReviewInvocationScope::initial("stale-ir"));
        engine.session.plan_candidate_ir_ref = Some("stale-ir".to_string());
        engine.active_node_id = Some("new-reviewer-node".to_string());

        engine
            .ensure_review_invocation_scope()
            .await
            .expect("ensure must reconcile from durable state");

        let expected_scope = ReviewInvocationScope::initial(current_ir_ref.clone());
        assert_eq!(
            engine.session().review_invocation_scope,
            Some(expected_scope.clone())
        );
        expected_scope
            .validate_digest()
            .expect("reconciled Initial scope digest must be valid");
        let durable = lifecycle
            .get_workspace_session(&engine.session().session_id)
            .expect("load reconciled durable session");
        assert_eq!(durable.review_invocation_scope, Some(expected_scope));

        let action = engine
            .work_item_policy_action("new-reviewer-node", &pass_verdict())
            .expect("policy action");
        assert!(
            !matches!(action, RoutingAction::AbortFatal { .. }),
            "durable-first reconciliation must not produce a policy fatal: {action:?}"
        );
        let durable = lifecycle
            .get_workspace_session(&engine.session().session_id)
            .expect("load verdict-updated durable session");
        assert_eq!(
            durable
                .run_history
                .review_cycles
                .get("review:new-reviewer-node")
                .expect("new reviewer node cycle")
                .initial_count,
            1,
            "the merged verdict must consume the new node's Initial review budget"
        );
    }

    #[tokio::test]
    async fn repairable_runs_once_then_same_node_recovery_uses_real_mechanical_report_ref() {
        let (_tmp, lifecycle, _plan_id, mut engine) =
            make_work_item_plan_engine_with_accepted_contract_drafts();
        single_candidate_record(
            &lifecycle,
            &mut engine,
            SingleCandidatePhase::Evaluate,
            RunPolicy::Interactive,
        );
        engine.start_review().await;
        let initial_node_id = engine
            .active_node_id
            .clone()
            .expect("start_review creates the initial ReviewerRun node");

        complete_single_candidate_review(&mut engine, repairable_verdict("missing contract")).await;

        let after_repair = lifecycle
            .get_workspace_session(&engine.session().session_id)
            .expect("repair persisted");
        assert_eq!(after_repair.run_history.repairs_used, 1);
        assert_eq!(
            after_repair.single_candidate_phase,
            Some(SingleCandidatePhase::Generate),
            "repair provider must be reserved after durable Generate transition"
        );
        complete_repair_generation(&lifecycle, &mut engine, "verification");
        let evaluated = lifecycle
            .get_workspace_session(&engine.session().session_id)
            .expect("verification refs persisted");
        assert_eq!(
            evaluated.single_candidate_phase,
            Some(SingleCandidatePhase::Evaluate)
        );
        assert!(matches!(
            evaluated.review_invocation_scope,
            Some(ReviewInvocationScope::Initial { .. })
        ));

        engine
            .ensure_review_invocation_scope()
            .await
            .expect("same ReviewerRun node must materialize Verification from durable refs");
        let verified = lifecycle
            .get_workspace_session(&engine.session().session_id)
            .expect("verification scope persisted");
        let scope = verified
            .review_invocation_scope
            .expect("verification scope persisted");
        match scope {
            crate::product::work_item_plan_policy::ReviewInvocationScope::Verification {
                repaired_revision_id,
                mechanical_report_ref: persisted_report_ref,
                ..
            } => {
                assert_eq!(
                    repaired_revision_id,
                    verified.plan_candidate_ir_ref.unwrap()
                );
                assert_eq!(persisted_report_ref, mechanical_report_ref(&engine));
            }
            other => panic!("expected verification scope for {initial_node_id}, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn repair_then_second_review_passes_with_one_durable_reviewer_cycle() {
        let (_tmp, lifecycle, _plan_id, mut engine) =
            make_work_item_plan_engine_with_accepted_contract_drafts();
        single_candidate_record(
            &lifecycle,
            &mut engine,
            SingleCandidatePhase::Evaluate,
            RunPolicy::Interactive,
        );
        engine.start_review().await;
        let node_a = engine
            .active_node_id
            .clone()
            .expect("start_review creates round-one ReviewerRun node");
        complete_single_candidate_review(&mut engine, repairable_verdict("missing contract")).await;
        let after_repair = lifecycle
            .get_workspace_session(&engine.session().session_id)
            .expect("repair route persisted");
        assert_eq!(
            after_repair.single_candidate_phase,
            Some(SingleCandidatePhase::Generate)
        );
        assert_eq!(after_repair.run_history.repairs_used, 1);

        complete_repair_generation(&lifecycle, &mut engine, "second-review-pass");
        let evaluated = lifecycle
            .get_workspace_session(&engine.session().session_id)
            .expect("repaired evaluation persisted");
        assert!(matches!(
            evaluated.review_invocation_scope,
            Some(ReviewInvocationScope::Initial { .. })
        ));

        engine.start_review().await;
        let node_b = engine
            .active_node_id
            .clone()
            .expect("start_review creates a new ReviewerRun node for round two");
        assert_ne!(
            node_a, node_b,
            "each review start must create a distinct node"
        );
        engine
            .ensure_review_invocation_scope()
            .await
            .expect("new node must materialize Initial scope");
        let second_scope = engine
            .session()
            .review_invocation_scope
            .clone()
            .expect("scope materialized for node B");
        assert!(matches!(
            second_scope,
            ReviewInvocationScope::Initial { .. }
        ));
        second_scope
            .validate_digest()
            .expect("node B initial scope digest is valid");

        complete_single_candidate_review(&mut engine, pass_verdict()).await;

        let completed = lifecycle
            .get_workspace_session(&engine.session().session_id)
            .expect("second review persisted");
        assert_eq!(
            completed.single_candidate_phase,
            Some(SingleCandidatePhase::Approval)
        );
        assert_eq!(completed.status, WorkspaceSessionStatus::WaitingForHuman);
        assert!(
            !completed
                .policy_diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "verification_scope_violation"),
            "node rotation must not terminate from an invocation/cycle phase mismatch"
        );
        let reviewer_cycles = completed
            .run_history
            .review_cycles
            .iter()
            .filter(|(key, _)| key.starts_with("review:"))
            .collect::<Vec<_>>();
        assert_eq!(reviewer_cycles.len(), 2, "each ReviewerRun owns one cycle");
        for (_, cycle) in reviewer_cycles {
            assert!(cycle.initial_count <= 1);
            assert!(cycle.verification_count <= 1);
            assert!(cycle.repairs_used <= 1);
        }
    }

    #[tokio::test]
    async fn repeated_fingerprint_enters_the_policy_terminal_for_each_run_policy() {
        for (policy, expected_status) in [
            (
                RunPolicy::Interactive,
                WorkspaceSessionStatus::WaitingForHuman,
            ),
            (
                RunPolicy::AutoIfValid,
                WorkspaceSessionStatus::StoppedNeedsHuman,
            ),
        ] {
            let (_tmp, lifecycle, _plan_id, mut engine) =
                make_work_item_plan_engine_with_accepted_contract_drafts();
            single_candidate_record(
                &lifecycle,
                &mut engine,
                SingleCandidatePhase::Evaluate,
                policy,
            );
            engine.start_review().await;
            let finding = repairable_verdict("duplicate contract finding");
            complete_single_candidate_review(&mut engine, finding.clone()).await;
            complete_repair_generation(&lifecycle, &mut engine, "repeated");
            engine
                .ensure_review_invocation_scope()
                .await
                .expect("same ReviewerRun must materialize Verification after repair");
            complete_single_candidate_review(&mut engine, finding).await;

            let persisted = lifecycle
                .get_workspace_session(&engine.session().session_id)
                .expect("terminal persisted");
            assert_eq!(persisted.status, expected_status);
            assert_eq!(persisted.run_history.repairs_used, 1);
            assert!(matches!(
                persisted.human_gate_snapshot,
                Some(ref snapshot) if snapshot.trigger == HumanReason::RepeatedFingerprint
            ));
            for cycle in persisted.run_history.review_cycles.values() {
                assert!(cycle.initial_count <= 1);
                assert!(cycle.verification_count <= 1);
                assert!(cycle.repairs_used <= 1);
            }
        }
    }

    #[tokio::test]
    async fn repair_budget_exhaustion_with_new_fingerprint_stops_for_human_not_failed() {
        for (policy, expected_status) in [
            (
                RunPolicy::Interactive,
                WorkspaceSessionStatus::WaitingForHuman,
            ),
            (
                RunPolicy::AutoIfValid,
                WorkspaceSessionStatus::StoppedNeedsHuman,
            ),
        ] {
            let (_tmp, lifecycle, _plan_id, mut engine) =
                make_work_item_plan_engine_with_accepted_contract_drafts();
            single_candidate_record(
                &lifecycle,
                &mut engine,
                SingleCandidatePhase::Evaluate,
                policy,
            );
            engine.start_review().await;
            complete_single_candidate_review(&mut engine, repairable_verdict("first finding"))
                .await;
            complete_repair_generation(&lifecycle, &mut engine, "budget");
            engine
                .ensure_review_invocation_scope()
                .await
                .expect("same ReviewerRun must materialize Verification after repair");
            complete_single_candidate_review(&mut engine, repairable_verdict("new finding")).await;

            let persisted = lifecycle
                .get_workspace_session(&engine.session().session_id)
                .expect("terminal persisted");
            assert_eq!(persisted.status, expected_status);
            assert_ne!(persisted.status, WorkspaceSessionStatus::Failed);
            assert!(persisted.human_gate_snapshot.is_some());
            assert!(
                !persisted
                    .policy_diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.code == "verification_scope_violation"),
                "a Verification-scope external finding must retain its human route"
            );
            for cycle in persisted.run_history.review_cycles.values() {
                assert!(cycle.initial_count <= 1);
                assert!(cycle.verification_count <= 1);
                assert!(cycle.repairs_used <= 1);
            }
        }
    }

    #[tokio::test]
    async fn exhausted_transition_budget_is_failed_and_failed_is_absorbing() {
        let (_tmp, lifecycle, _plan_id, mut engine) =
            make_work_item_plan_engine_with_accepted_contract_drafts();
        single_candidate_record(
            &lifecycle,
            &mut engine,
            SingleCandidatePhase::Evaluate,
            RunPolicy::AutoIfValid,
        );
        engine.session.run_history = RunHistory {
            transitions_used: RunBudgets::default().max_transitions,
            ..RunHistory::default()
        };
        let mut record = lifecycle
            .get_workspace_session(&engine.session().session_id)
            .expect("load record");
        record.run_history = engine.session.run_history.clone();
        write_json(
            &lifecycle
                .app_paths()
                .issue_root(&record.project_id, &record.issue_id)
                .join("workspace-sessions")
                .join(format!("{}.json", record.id)),
            &record,
        )
        .expect("persist exhausted budget");

        complete_single_candidate_review(&mut engine, pass_verdict()).await;
        let failed = lifecycle
            .get_workspace_session(&engine.session().session_id)
            .expect("failed session");
        assert_eq!(
            failed.single_candidate_phase,
            Some(SingleCandidatePhase::Failed)
        );
        assert_eq!(failed.status, WorkspaceSessionStatus::Failed);

        complete_single_candidate_review(&mut engine, pass_verdict()).await;
        let replayed = lifecycle
            .get_workspace_session(&engine.session().session_id)
            .expect("failed session after replay");
        assert_eq!(replayed, failed, "failed phase must be absorbing");
    }
}
