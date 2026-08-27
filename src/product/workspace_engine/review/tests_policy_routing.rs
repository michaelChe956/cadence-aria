use crate::product::work_item_plan_policy::{
    ClassifiedFinding, FatalReason, FindingClass, FindingFingerprint, HumanReason, PlanOutcome,
    PolicyDiagnostic, ReviewCycleState, ReviewFindingCategory, ReviewInvocationScope, RunBudgets,
    RunHistory, RunPolicy,
};
use crate::product::workspace_engine::review::policy_routing::{
    GateSnapshotContext, RoutingAction, route_outcome,
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct NormalizedRoute {
    stage: String,
    nodes: Vec<(String, String)>,
}

fn normalized_route(engine: &WorkspaceEngine, nodes_before: usize) -> NormalizedRoute {
    NormalizedRoute {
        stage: format!("{:?}", engine.session().stage),
        nodes: engine.timeline_nodes[nodes_before..]
            .iter()
            .map(|node| (format!("{:?}", node.node_type), format!("{:?}", node.stage)))
            .collect(),
    }
}

fn artifact_variant(engine: &WorkspaceEngine) -> Option<&'static str> {
    engine
        .session()
        .artifact
        .as_ref()
        .map(|artifact| match artifact {
            ArtifactPayload::Markdown { .. } => "markdown",
            ArtifactPayload::WorkItemPlanCandidate { .. } => "work_item_plan_candidate",
            ArtifactPayload::WorkItemPlanOutlineCandidate { .. } => {
                "work_item_plan_outline_candidate"
            }
            ArtifactPayload::WorkItemPlanContextBlocker { .. } => "work_item_plan_context_blocker",
            ArtifactPayload::WorkItemDraftCandidate { .. } => "work_item_draft_candidate",
            ArtifactPayload::WorkItemBatchState { .. } => "work_item_batch_state",
            ArtifactPayload::WorkItemPlanCompileReport { .. } => "work_item_plan_compile_report",
            ArtifactPayload::WorkItemPlanProjection { .. } => "work_item_plan_projection",
            ArtifactPayload::WorkItemProjection { .. } => "work_item_projection",
            ArtifactPayload::WorkItemRevisionHistory { .. } => "work_item_revision_history",
            ArtifactPayload::ProjectionValidation { .. } => "projection_validation",
            ArtifactPayload::PlanAmendmentManifest { .. } => "plan_amendment_manifest",
        })
}

fn normalized_stage(stage: WorkspaceStage) -> String {
    format!("{stage:?}")
}

fn normalized_node(node_type: TimelineNodeType, stage: WsWorkspaceStage) -> (String, String) {
    (format!("{node_type:?}"), format!("{stage:?}"))
}

fn review_verdict(verdict: ReviewVerdictType) -> ReviewVerdict {
    ReviewVerdict {
        verdict,
        comments: "legacy review comments".to_string(),
        summary: "legacy review summary".to_string(),
        findings: Vec::new(),
        review_gate: ReviewGate::UserConfirmAllowed,
        work_item_plan_review: None,
        structured_output_diagnostic: None,
    }
}

fn revise_batch_review_verdict() -> ReviewVerdict {
    structured_batch_review_verdict(WorkItemPlanReviewVerdict::ReviseBatch)
}

fn structured_batch_review_verdict(verdict: WorkItemPlanReviewVerdict) -> ReviewVerdict {
    let (summary, review_action, gates) = match verdict {
        WorkItemPlanReviewVerdict::ReviseBatch => (
            "legacy batch needs rewrite",
            WorkItemPlanReviewAction::ReviseBatch,
            vec![WorkItemPlanReviewGate::RequiresBatchRevision],
        ),
        WorkItemPlanReviewVerdict::NeedsHuman => (
            "structured batch needs human",
            WorkItemPlanReviewAction::HumanTriage,
            Vec::new(),
        ),
        WorkItemPlanReviewVerdict::PlanReopenRequired => (
            "structured batch requires plan reopen",
            WorkItemPlanReviewAction::ReviseOutline,
            vec![WorkItemPlanReviewGate::RequiresPlanReopen],
        ),
        unexpected => panic!("unexpected structured batch verdict: {unexpected:?}"),
    };
    ReviewVerdict {
        verdict: ReviewVerdictType::NeedsHuman,
        comments: "structured batch review comments".to_string(),
        summary: summary.to_string(),
        findings: Vec::new(),
        review_gate: ReviewGate::UserTriageRequired,
        work_item_plan_review: Some(WorkItemPlanReviewComplete {
            verdict,
            review_scope: WorkItemPlanReviewScope::Batch,
            target_outline_id: None,
            generation_round_id: "round_0001".to_string(),
            draft_id: None,
            batch_id: Some("batch_0001".to_string()),
            review_action,
            gates,
            affects_items: Vec::new(),
            warnings: Vec::new(),
        }),
        structured_output_diagnostic: None,
    }
}

fn legacy_outline_engine(
    checkpoint_store: Arc<CheckpointStore>,
    session_id: &str,
) -> WorkspaceEngine {
    let (event_tx, _) = mpsc::channel(8);
    let mut session = make_session(session_id);
    session.workspace_type = WorkspaceType::WorkItemPlan;
    session.stage = WorkspaceStage::CrossReview;
    session.artifact = Some(ArtifactPayload::WorkItemPlanOutlineCandidate {
        outline_candidate: Box::new(WorkItemPlanOutlineCandidateDto {
            outline: test_work_item_plan_outline(Vec::new()),
            design_context_gaps: Vec::new(),
            validator_findings: Vec::new(),
            context_blockers: Vec::new(),
            current_generation_round_id: Some("round_0001".to_string()),
            selected_generation_mode: None,
        }),
    });
    WorkspaceEngine::new(checkpoint_store, event_tx, session)
}

#[tokio::test]
async fn legacy_outline_review_routes_are_characterized_by_normalized_state_sequence() {
    let (_temporary_directory, checkpoint_store) = setup();
    let cases = [
        (
            "pass",
            ReviewVerdictType::Pass,
            NormalizedRoute {
                stage: normalized_stage(WorkspaceStage::AuthorConfirm),
                nodes: vec![normalized_node(
                    TimelineNodeType::WorkItemGenerationMode,
                    WsWorkspaceStage::AuthorConfirm,
                )],
            },
        ),
        (
            "revise",
            ReviewVerdictType::Revise,
            NormalizedRoute {
                stage: normalized_stage(WorkspaceStage::ReviewDecision),
                nodes: vec![normalized_node(
                    TimelineNodeType::ReviewDecision,
                    WsWorkspaceStage::ReviewDecision,
                )],
            },
        ),
        (
            "needs_human",
            ReviewVerdictType::NeedsHuman,
            NormalizedRoute {
                stage: normalized_stage(WorkspaceStage::HumanConfirm),
                nodes: vec![normalized_node(
                    TimelineNodeType::HumanConfirm,
                    WsWorkspaceStage::HumanConfirm,
                )],
            },
        ),
    ];

    for (case, verdict, expected) in cases {
        let mut engine =
            legacy_outline_engine(checkpoint_store.clone(), &format!("legacy_outline_{case}"));
        let nodes_before = engine.timeline_nodes.len();

        engine
            .route_work_item_plan_outline_review(review_verdict(verdict))
            .await;

        assert_eq!(
            normalized_route(&engine, nodes_before),
            expected,
            "outline review {case} must preserve its legacy route after dynamic node IDs and timestamps are normalized"
        );
        assert_eq!(
            artifact_variant(&engine),
            Some("work_item_plan_outline_candidate"),
            "outline review {case} must preserve artifact payload variant"
        );
        // The review node is completed before routing.  A revise verdict then creates a
        // paused decision node, while pass/needs_human create active continuation/gate
        // nodes; this is the legacy state-machine contract, not a dynamic-ID detail.
        let expected_status = match case {
            "revise" => TimelineNodeStatus::Paused,
            _ => TimelineNodeStatus::Active,
        };
        assert_eq!(
            engine.timeline_nodes.last().map(|node| &node.status),
            Some(&expected_status),
            "outline review {case} must preserve its legacy continuation status"
        );
    }
}

#[tokio::test]
async fn legacy_batch_review_routes_are_characterized_by_normalized_state_sequence() {
    let (_temporary_directory, checkpoint_store) = setup();
    let cases = [
        (
            "revise",
            review_verdict(ReviewVerdictType::Revise),
            NormalizedRoute {
                stage: normalized_stage(WorkspaceStage::AuthorConfirm),
                nodes: vec![normalized_node(
                    TimelineNodeType::WorkItemBatchConfirm,
                    WsWorkspaceStage::AuthorConfirm,
                )],
            },
        ),
        (
            "needs_human",
            review_verdict(ReviewVerdictType::NeedsHuman),
            NormalizedRoute {
                stage: normalized_stage(WorkspaceStage::HumanConfirm),
                nodes: vec![normalized_node(
                    TimelineNodeType::HumanConfirm,
                    WsWorkspaceStage::HumanConfirm,
                )],
            },
        ),
        (
            "revise_batch",
            revise_batch_review_verdict(),
            NormalizedRoute {
                stage: normalized_stage(WorkspaceStage::AuthorConfirm),
                nodes: vec![normalized_node(
                    TimelineNodeType::WorkItemBatchConfirm,
                    WsWorkspaceStage::AuthorConfirm,
                )],
            },
        ),
    ];

    for (case, verdict, expected) in cases {
        let mut engine =
            legacy_outline_engine(checkpoint_store.clone(), &format!("legacy_batch_{case}"));
        let nodes_before = engine.timeline_nodes.len();

        engine.route_work_item_batch_review(verdict).await;

        assert_eq!(
            normalized_route(&engine, nodes_before),
            expected,
            "batch review {case} must preserve its legacy route after dynamic node IDs and timestamps are normalized"
        );
        assert_eq!(
            artifact_variant(&engine),
            Some("work_item_plan_outline_candidate"),
            "batch review {case} must preserve artifact payload variant"
        );
        // Every legacy batch route creates an active continuation/gate node;
        // dynamic node IDs and timestamps are intentionally excluded above.
        assert_eq!(
            engine.timeline_nodes.last().map(|node| &node.status),
            Some(&TimelineNodeStatus::Active),
            "batch review {case} must preserve its legacy continuation status"
        );
    }
}

fn classified_finding(class: FindingClass, message: &str) -> ClassifiedFinding {
    ClassifiedFinding {
        class,
        fingerprint: FindingFingerprint::for_finding(
            Some(ReviewFindingCategory::ContractGap),
            class,
            message,
            Some("contract.field"),
        ),
        category: Some(ReviewFindingCategory::ContractGap),
        severity: "must_fix".to_string(),
        message: message.to_string(),
        evidence: Some("evidence".to_string()),
        required_action: Some("repair".to_string()),
        contract_field: Some("contract.field".to_string()),
    }
}

fn gate_context(trigger: HumanReason) -> GateSnapshotContext {
    GateSnapshotContext {
        history: RunHistory {
            repairs_used: 1,
            manual_repairs_used: 2,
            ..RunHistory::default()
        },
        budgets: RunBudgets {
            max_repairs: 1,
            max_transitions: 12,
            max_manual_repairs: 3,
        },
        invocation: ReviewInvocationScope::initial("revision_001"),
        findings: vec![classified_finding(
            FindingClass::HumanRequired,
            "needs human",
        )],
        repeated_fingerprints: vec![FindingFingerprint::for_finding(
            Some(ReviewFindingCategory::ContractGap),
            FindingClass::Repairable,
            "repeated",
            Some("contract.field"),
        )],
        trigger,
    }
}

#[test]
fn work_item_policy_counts_review_budget_per_cycle_and_stops_only_the_third_same_cycle_review() {
    let (_temporary_directory, checkpoint_store) = setup();
    let mut engine = legacy_outline_engine(checkpoint_store, "cycle_scoped_budget");
    engine.session.run_policy = RunPolicy::AutoIfValid;
    let needs_human = review_verdict(ReviewVerdictType::NeedsHuman);

    let first = engine
        .work_item_policy_action("outline_review", &needs_human)
        .expect("work item plan reviews use policy routing");
    assert!(matches!(first, RoutingAction::StopNeedsHuman { .. }));
    assert_eq!(engine.session.run_history.initial_review_count, 1);
    assert_eq!(
        engine.session.run_history.review_cycles["review:outline_review"].initial_count,
        1
    );

    let second = engine
        .work_item_policy_action("outline_review", &needs_human)
        .expect("same cycle verification is permitted once");
    assert!(matches!(second, RoutingAction::StopNeedsHuman { .. }));
    assert_eq!(engine.session.run_history.verification_review_count, 1);
    assert_eq!(
        engine.session.run_history.review_cycles["review:outline_review"].verification_count,
        1
    );

    let third = engine
        .work_item_policy_action("outline_review", &needs_human)
        .expect("second verification routes to its terminal matrix");
    assert!(matches!(third, RoutingAction::StopNeedsHuman { .. }));
    assert_eq!(engine.session.run_history.initial_review_count, 1);
    assert_eq!(engine.session.run_history.verification_review_count, 1);

    let next_cycle = engine
        .work_item_policy_action("draft_review", &needs_human)
        .expect("a different review artifact starts a separate cycle");
    assert!(matches!(next_cycle, RoutingAction::StopNeedsHuman { .. }));
    assert_eq!(engine.session.run_history.initial_review_count, 2);
    assert_eq!(
        engine.session.run_history.review_cycles["review:draft_review"].initial_count,
        1
    );
}

#[test]
fn policy_route_counts_automatic_and_manual_repairs_in_durable_history() {
    let (_temporary_directory, _checkpoint_store, lifecycle, _plan_id, mut engine) =
        make_work_item_plan_engine_with_draft_candidate("policy_route_repair_counters");
    let repairable = ReviewVerdict {
        verdict: ReviewVerdictType::Revise,
        comments: "repairable contract gap".to_string(),
        summary: "repair outline".to_string(),
        findings: vec![ReviewFinding {
            severity: ReviewFindingSeverity::MustFix,
            message: "repairable contract gap".to_string(),
            evidence: "evidence".to_string(),
            required_action: "repair".to_string(),
            category: Some(ReviewFindingCategory::ContractGap),
            class_hint: None,
            contract_field: Some("contract.field".to_string()),
        }],
        review_gate: ReviewGate::RequiresRevision,
        work_item_plan_review: None,
        structured_output_diagnostic: None,
    };

    let action = engine
        .work_item_policy_action("outline_review", &repairable)
        .expect("repairable review must be evaluated");
    assert!(matches!(
        action,
        RoutingAction::TriggerAggregateRepair { .. }
    ));
    assert_eq!(engine.session.run_history.repairs_used, 1);
    assert_eq!(engine.session.run_history.review_cycles.len(), 1);
    assert_eq!(
        engine
            .session
            .run_history
            .review_cycles
            .values()
            .next()
            .expect("the review must create an artifact cycle")
            .repairs_used,
        1,
        "the automatic repair must be accounted to its artifact review cycle"
    );

    let needs_human = review_verdict(ReviewVerdictType::NeedsHuman);
    let _ = engine
        .work_item_policy_action("manual_repair_gate", &needs_human)
        .expect("native human requirement must enter a gate");
    engine
        .record_manual_policy_repair()
        .expect("human-authorized repair must be durably counted");
    assert_eq!(engine.session.run_history.manual_repairs_used, 1);

    let persisted = lifecycle
        .get_workspace_session(&engine.session.session_id)
        .expect("repair counters must be persisted");
    assert_eq!(persisted.run_history.repairs_used, 1);
    assert_eq!(persisted.run_history.review_cycles.len(), 2);
    assert!(
        persisted
            .run_history
            .review_cycles
            .values()
            .any(|cycle| cycle.repairs_used == 1),
        "the persisted artifact cycle must retain automatic repair consumption"
    );
    assert_eq!(persisted.run_history.manual_repairs_used, 1);
}

#[test]
fn policy_route_reloads_and_reevaluates_once_after_a_cas_conflict() {
    let (_temporary_directory, _checkpoint_store, lifecycle, _plan_id, mut engine) =
        make_work_item_plan_engine_with_draft_candidate("policy_route_cas_conflict");
    engine.session.run_policy = RunPolicy::AutoIfValid;
    engine.policy_route_before_persist = Some(Box::new(|store, session_id| {
        store
            .update_workspace_session_status(session_id, WorkspaceSessionStatus::WaitingForHuman)
            .expect("concurrent route update must be durable");
    }));

    let action = engine
        .work_item_policy_action(
            "outline_review",
            &review_verdict(ReviewVerdictType::NeedsHuman),
        )
        .expect("work item policy route must handle the retry");

    assert!(matches!(action, RoutingAction::StopNeedsHuman { .. }));
    assert_eq!(engine.session.run_history.initial_review_count, 1);
    let persisted = lifecycle
        .get_workspace_session(&engine.session.session_id)
        .expect("retried policy route must persist");
    assert_eq!(persisted.status, WorkspaceSessionStatus::StoppedNeedsHuman);
    assert_eq!(persisted.run_history.initial_review_count, 1);
    assert_eq!(
        persisted.run_history.review_cycles["review:outline_review"].initial_count, 1,
        "retry must re-evaluate the current durable history instead of double-applying the stale delta"
    );
}

#[test]
fn policy_route_outcome_covers_terminal_matrix_for_each_run_policy() {
    let repairable = classified_finding(FindingClass::Repairable, "repair once");
    let cases = [
        (
            "valid",
            PlanOutcome::Valid,
            HumanReason::NativeHumanRequired,
            "completed",
        ),
        (
            "first_repairable",
            PlanOutcome::Repairable {
                findings: vec![repairable.clone()],
            },
            HumanReason::RepairBudgetExhausted,
            "repair",
        ),
        (
            "repeated_fingerprint",
            PlanOutcome::HumanRequired {
                findings: vec![repairable.clone()],
                repeated_fingerprints: vec![repairable.fingerprint.clone()],
                reason: HumanReason::RepeatedFingerprint,
            },
            HumanReason::RepeatedFingerprint,
            "human",
        ),
        (
            "native_human_required",
            PlanOutcome::HumanRequired {
                findings: vec![classified_finding(
                    FindingClass::HumanRequired,
                    "native human",
                )],
                repeated_fingerprints: Vec::new(),
                reason: HumanReason::NativeHumanRequired,
            },
            HumanReason::NativeHumanRequired,
            "human",
        ),
        (
            "repair_budget_exhausted",
            PlanOutcome::HumanRequired {
                findings: vec![classified_finding(
                    FindingClass::Repairable,
                    "budget exhausted",
                )],
                repeated_fingerprints: Vec::new(),
                reason: HumanReason::RepairBudgetExhausted,
            },
            HumanReason::RepairBudgetExhausted,
            "human",
        ),
        (
            "verification_new_findings",
            PlanOutcome::HumanRequired {
                findings: vec![classified_finding(
                    FindingClass::HumanRequired,
                    "verification scope has a new finding",
                )],
                repeated_fingerprints: Vec::new(),
                reason: HumanReason::VerificationNewFindings,
            },
            HumanReason::VerificationNewFindings,
            "human",
        ),
        (
            "state_corruption",
            PlanOutcome::Fatal {
                reason: FatalReason::StateCorruption,
                diagnostics: vec![PolicyDiagnostic {
                    code: "state_corruption".to_string(),
                    message: "durable counters are invalid".to_string(),
                    field: Some("run_history".to_string()),
                }],
            },
            HumanReason::NativeHumanRequired,
            "fatal",
        ),
    ];

    for policy in [RunPolicy::Interactive, RunPolicy::AutoIfValid] {
        for (case, outcome, trigger, expected) in &cases {
            let action = route_outcome(outcome.clone(), policy, gate_context(*trigger));
            match (*expected, policy, action) {
                ("completed", _, RoutingAction::ContinueToCompleted) => {}
                ("repair", _, RoutingAction::TriggerAggregateRepair { findings }) => {
                    assert_eq!(findings, vec![repairable.clone()], "{case}");
                }
                ("human", RunPolicy::Interactive, RoutingAction::EnterHumanGate { snapshot }) => {
                    assert_eq!(snapshot.trigger, *trigger, "{case}");
                    assert!(!snapshot.resumable, "{case}");
                }
                ("human", RunPolicy::AutoIfValid, RoutingAction::StopNeedsHuman { snapshot }) => {
                    assert_eq!(snapshot.trigger, *trigger, "{case}");
                    assert!(snapshot.resumable, "{case}");
                }
                (
                    "fatal",
                    _,
                    RoutingAction::AbortFatal {
                        reason,
                        diagnostics,
                    },
                ) => {
                    assert_eq!(reason, FatalReason::StateCorruption, "{case}");
                    assert_eq!(diagnostics[0].code, "state_corruption", "{case}");
                }
                (_, _, unexpected) => panic!("{case} under {policy:?}: {unexpected:?}"),
            }
        }
    }
}

#[test]
fn human_gate_snapshot_uses_context_findings_budgets_and_resumable_rules() {
    let context = gate_context(HumanReason::RepeatedFingerprint);
    let expected_findings = context.findings.clone();
    let expected_repeated_fingerprints = context.repeated_fingerprints.clone();

    let interactive = route_outcome(
        PlanOutcome::HumanRequired {
            findings: Vec::new(),
            repeated_fingerprints: Vec::new(),
            reason: HumanReason::RepeatedFingerprint,
        },
        RunPolicy::Interactive,
        context.clone(),
    );
    let auto = route_outcome(
        PlanOutcome::HumanRequired {
            findings: Vec::new(),
            repeated_fingerprints: Vec::new(),
            reason: HumanReason::RepeatedFingerprint,
        },
        RunPolicy::AutoIfValid,
        context,
    );

    let RoutingAction::EnterHumanGate {
        snapshot: interactive_snapshot,
    } = interactive
    else {
        panic!("interactive policy must enter the existing human gate");
    };
    assert_eq!(interactive_snapshot.findings, expected_findings);
    assert_eq!(
        interactive_snapshot.repeated_fingerprints,
        expected_repeated_fingerprints
    );
    assert_eq!(interactive_snapshot.attempts_used, 3);
    assert_eq!(interactive_snapshot.manual_repairs_remaining, 1);
    assert!(!interactive_snapshot.resumable);

    let RoutingAction::StopNeedsHuman {
        snapshot: auto_snapshot,
    } = auto
    else {
        panic!("auto policy must stop for explicit human takeover");
    };
    assert_eq!(auto_snapshot.attempts_used, 3);
    assert_eq!(auto_snapshot.manual_repairs_remaining, 1);
    assert!(auto_snapshot.resumable);
}

#[test]
fn human_gate_snapshot_manual_repair_remaining_saturates_at_zero() {
    let mut context = gate_context(HumanReason::NativeHumanRequired);
    context.history.manual_repairs_used = 5;
    context.budgets.max_manual_repairs = 3;

    let action = route_outcome(
        PlanOutcome::HumanRequired {
            findings: Vec::new(),
            repeated_fingerprints: Vec::new(),
            reason: HumanReason::NativeHumanRequired,
        },
        RunPolicy::Interactive,
        context,
    );

    let RoutingAction::EnterHumanGate { snapshot } = action else {
        panic!("interactive policy must enter human gate");
    };
    assert_eq!(snapshot.manual_repairs_remaining, 0);
    assert_eq!(snapshot.attempts_used, 6);
}

#[tokio::test]
async fn rep1_needs_human_replay_uses_durable_interactive_gate_and_auto_terminal() {
    let (_tmp, _checkpoint_store, lifecycle, _plan_id, mut interactive) =
        make_work_item_plan_engine_with_draft_candidate("rep1_interactive_needs_human");
    interactive.session.run_policy = RunPolicy::Interactive;
    interactive.begin_work_item_plan_outline_review_run().await;
    interactive
        .complete_review(
            ProviderCompletion::plain("rep1 needs human", None),
            review_verdict(ReviewVerdictType::NeedsHuman),
        )
        .await;

    assert_eq!(interactive.current_stage(), WorkspaceStage::HumanConfirm);
    assert_eq!(
        interactive.session.session_status,
        WorkspaceSessionStatus::WaitingForHuman
    );
    assert_eq!(interactive.session.run_history.initial_review_count, 1);
    assert_eq!(interactive.session.run_history.transitions_used, 1);
    assert!(interactive.session.human_gate_snapshot.is_some());
    let review_nodes_before_accept = interactive
        .timeline_nodes
        .iter()
        .filter(|node| node.node_type == TimelineNodeType::WorkItemPlanOutlineReview)
        .count();

    let accept = interactive
        .handle_human_confirm(HumanConfirmDecision::Confirm, None)
        .await
        .expect("interactive human gate must accept without another review");
    assert!(matches!(
        accept,
        ReviewDecisionOutcome::ConfirmedWithChildSessions { .. }
    ));
    assert_eq!(
        interactive
            .timeline_nodes
            .iter()
            .filter(|node| node.node_type == TimelineNodeType::WorkItemPlanOutlineReview)
            .count(),
        review_nodes_before_accept,
        "accepting the rep1 human gate must not silently re-review"
    );
    let persisted_interactive = lifecycle
        .get_workspace_session(&interactive.session.session_id)
        .expect("persisted interactive session");
    assert_eq!(
        persisted_interactive.status,
        WorkspaceSessionStatus::Confirmed
    );
    assert_eq!(
        persisted_interactive.run_history.transitions_used, 1,
        "a successfully durable policy stage transition must increment its display counter"
    );

    let (_tmp, _checkpoint_store, lifecycle, _plan_id, mut auto) =
        make_work_item_plan_engine_with_draft_candidate("rep1_auto_needs_human");
    auto.session.run_policy = RunPolicy::AutoIfValid;
    auto.begin_work_item_plan_outline_review_run().await;
    auto.complete_review(
        ProviderCompletion::plain("rep1 needs human", None),
        review_verdict(ReviewVerdictType::NeedsHuman),
    )
    .await;

    assert_eq!(auto.current_stage(), WorkspaceStage::Completed);
    assert_eq!(
        auto.session.session_status,
        WorkspaceSessionStatus::StoppedNeedsHuman
    );
    assert_eq!(auto.session.run_history.initial_review_count, 1);
    assert_eq!(auto.session.run_history.transitions_used, 1);
    assert!(
        auto.session
            .human_gate_snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.resumable)
    );
    let persisted_auto = lifecycle
        .get_workspace_session(&auto.session.session_id)
        .expect("persisted auto session");
    assert_eq!(
        persisted_auto.status,
        WorkspaceSessionStatus::StoppedNeedsHuman
    );
    assert_eq!(
        persisted_auto.run_history.transitions_used, 1,
        "the auto terminal transition must be durably counted once"
    );
}

#[tokio::test]
async fn structured_needs_human_batch_review_in_auto_mode_stops_with_durable_gate_and_counts() {
    let (_tmp, _checkpoint_store, lifecycle, plan_id, mut engine) =
        make_work_item_plan_engine_with_draft_candidate("structured_batch_needs_human_auto");
    prepare_work_item_plan_outline_artifact(&mut engine).await;
    save_batch_work_item_plan_index_with_accepted_drafts(&engine, &plan_id);
    engine.session.run_policy = RunPolicy::AutoIfValid;
    engine.begin_work_item_batch_review_run().await;

    engine
        .complete_review(
            ProviderCompletion::plain("structured batch needs human", None),
            structured_batch_review_verdict(WorkItemPlanReviewVerdict::NeedsHuman),
        )
        .await;

    assert_eq!(engine.current_stage(), WorkspaceStage::Completed);
    assert_eq!(
        engine.session.session_status,
        WorkspaceSessionStatus::StoppedNeedsHuman
    );
    assert_eq!(engine.session.run_history.initial_review_count, 1);
    assert_eq!(engine.session.run_history.transitions_used, 1);
    let gate = engine
        .session
        .human_gate_snapshot
        .as_ref()
        .expect("auto native human verdict must persist a gate snapshot");
    assert_eq!(gate.trigger, HumanReason::NativeHumanRequired);
    assert!(gate.resumable);

    let persisted = lifecycle
        .get_workspace_session(&engine.session.session_id)
        .expect("persisted auto terminal session");
    assert_eq!(
        persisted.status,
        WorkspaceSessionStatus::StoppedNeedsHuman
    );
    assert_eq!(persisted.run_history.initial_review_count, 1);
    assert_eq!(persisted.run_history.transitions_used, 1);
    assert_eq!(persisted.human_gate_snapshot, engine.session.human_gate_snapshot);
}

#[tokio::test]
async fn structured_revise_batch_and_plan_reopen_consume_the_cycle_repair_budget_before_legacy_repair() {
    let (_tmp, _checkpoint_store, lifecycle, plan_id, mut revise_batch) =
        make_work_item_plan_engine_with_draft_candidate("structured_revise_batch_budgeted");
    prepare_work_item_plan_outline_artifact(&mut revise_batch).await;
    save_batch_work_item_plan_index_with_accepted_drafts(&revise_batch, &plan_id);
    let (event_tx, mut event_rx) = mpsc::channel(64);
    revise_batch.event_tx = event_tx;
    revise_batch.begin_work_item_batch_review_run().await;

    revise_batch
        .complete_review(
            ProviderCompletion::plain("structured revise batch", None),
            structured_batch_review_verdict(WorkItemPlanReviewVerdict::ReviseBatch),
        )
        .await;

    assert_eq!(revise_batch.current_stage(), WorkspaceStage::Running);
    assert_eq!(
        revise_batch.timeline_nodes.last().map(|node| &node.node_type),
        Some(&TimelineNodeType::WorkItemBatchRun),
        "the first repairable batch verdict must invoke the legacy batch rewrite mechanism"
    );
    assert_eq!(revise_batch.session.run_history.initial_review_count, 1);
    assert_eq!(revise_batch.session.run_history.repairs_used, 1);
    assert_eq!(
        revise_batch.session.run_history.review_cycles["batch:round_0001"].repairs_used,
        1
    );
    assert!(
        std::iter::from_fn(|| event_rx.try_recv().ok()).any(|event| matches!(
            event,
            EngineEvent::ProviderRunRequested {
                kind: ProviderRunKind::WorkItemPlanBatch,
                ..
            }
        )),
        "the policy-driven revise_batch repair must request a batch provider run"
    );
    let persisted_revise_batch = lifecycle
        .get_workspace_session(&revise_batch.session.session_id)
        .expect("persisted revise-batch budgeted session");
    assert_eq!(persisted_revise_batch.run_history.initial_review_count, 1);
    assert_eq!(persisted_revise_batch.run_history.repairs_used, 1);
    let batch_runs_after_first_repair = revise_batch
        .timeline_nodes
        .iter()
        .filter(|node| node.node_type == TimelineNodeType::WorkItemBatchRun)
        .count();

    revise_batch.begin_work_item_batch_review_run().await;
    revise_batch
        .complete_review(
            ProviderCompletion::plain("structured revise batch again", None),
            structured_batch_review_verdict(WorkItemPlanReviewVerdict::ReviseBatch),
        )
        .await;

    assert_eq!(revise_batch.current_stage(), WorkspaceStage::HumanConfirm);
    assert_eq!(
        revise_batch.timeline_nodes.last().map(|node| &node.node_type),
        Some(&TimelineNodeType::HumanConfirm),
        "a verification repairable verdict must not start a second batch generation"
    );
    assert_eq!(revise_batch.session.run_history.verification_review_count, 1);
    assert_eq!(revise_batch.session.run_history.repairs_used, 1);
    assert_eq!(
        revise_batch
            .timeline_nodes
            .iter()
            .filter(|node| node.node_type == TimelineNodeType::WorkItemBatchRun)
            .count(),
        batch_runs_after_first_repair,
        "the second batch review must not schedule another legacy batch generation"
    );
    let persisted_revise_batch = lifecycle
        .get_workspace_session(&revise_batch.session.session_id)
        .expect("persisted revise-batch human gate");
    assert_eq!(
        persisted_revise_batch.status,
        WorkspaceSessionStatus::WaitingForHuman
    );
    assert_eq!(persisted_revise_batch.run_history.repairs_used, 1);

    let (_tmp, _checkpoint_store, lifecycle, plan_id, mut reopen) =
        make_work_item_plan_engine_with_draft_candidate("structured_plan_reopen_budgeted");
    prepare_work_item_plan_outline_artifact(&mut reopen).await;
    save_batch_work_item_plan_index_with_accepted_drafts(&reopen, &plan_id);
    let (event_tx, mut event_rx) = mpsc::channel(64);
    reopen.event_tx = event_tx;
    reopen.begin_work_item_batch_review_run().await;

    reopen
        .complete_review(
            ProviderCompletion::plain("structured plan reopen", None),
            structured_batch_review_verdict(WorkItemPlanReviewVerdict::PlanReopenRequired),
        )
        .await;

    assert_eq!(reopen.current_stage(), WorkspaceStage::Running);
    assert_eq!(
        reopen.timeline_nodes.last().map(|node| &node.node_type),
        Some(&TimelineNodeType::WorkItemPlanOutlineRun),
        "the first repairable reopen verdict must invoke the legacy outline regeneration mechanism"
    );
    assert_eq!(reopen.session.run_history.initial_review_count, 1);
    assert_eq!(reopen.session.run_history.repairs_used, 1);
    assert!(
        std::iter::from_fn(|| event_rx.try_recv().ok()).any(|event| matches!(
            event,
            EngineEvent::ProviderRunRequested {
                kind: ProviderRunKind::WorkItemPlanOutlineRevision { .. },
                ..
            }
        )),
        "the policy-driven plan_reopen_required repair must request an outline provider run"
    );
    let persisted_reopen = lifecycle
        .get_workspace_session(&reopen.session.session_id)
        .expect("persisted plan-reopen budgeted session");
    assert_eq!(persisted_reopen.run_history.initial_review_count, 1);
    assert_eq!(persisted_reopen.run_history.repairs_used, 1);
    let outline_runs_after_first_repair = reopen
        .timeline_nodes
        .iter()
        .filter(|node| node.node_type == TimelineNodeType::WorkItemPlanOutlineRun)
        .count();

    reopen.begin_work_item_batch_review_run().await;
    reopen
        .complete_review(
            ProviderCompletion::plain("structured plan reopen again", None),
            structured_batch_review_verdict(WorkItemPlanReviewVerdict::PlanReopenRequired),
        )
        .await;

    assert_eq!(reopen.current_stage(), WorkspaceStage::HumanConfirm);
    assert_eq!(
        reopen.timeline_nodes.last().map(|node| &node.node_type),
        Some(&TimelineNodeType::HumanConfirm),
        "a verification reopen verdict must not start a second outline generation"
    );
    assert_eq!(reopen.session.run_history.verification_review_count, 1);
    assert_eq!(reopen.session.run_history.repairs_used, 1);
    assert_eq!(
        reopen
            .timeline_nodes
            .iter()
            .filter(|node| node.node_type == TimelineNodeType::WorkItemPlanOutlineRun)
            .count(),
        outline_runs_after_first_repair,
        "the second reopen review must not schedule another legacy outline generation"
    );
    let persisted_reopen = lifecycle
        .get_workspace_session(&reopen.session.session_id)
        .expect("persisted plan-reopen human gate");
    assert_eq!(persisted_reopen.status, WorkspaceSessionStatus::WaitingForHuman);
    assert_eq!(persisted_reopen.run_history.repairs_used, 1);
}

#[tokio::test]
async fn structured_revise_batch_and_plan_reopen_stop_in_auto_mode_after_the_budgeted_repair() {
    for verdict in [
        WorkItemPlanReviewVerdict::ReviseBatch,
        WorkItemPlanReviewVerdict::PlanReopenRequired,
    ] {
        let session_id = format!("budgeted_{verdict:?}_auto");
        let (_tmp, _checkpoint_store, lifecycle, plan_id, mut engine) =
            make_work_item_plan_engine_with_draft_candidate(&session_id);
        prepare_work_item_plan_outline_artifact(&mut engine).await;
        save_batch_work_item_plan_index_with_accepted_drafts(&engine, &plan_id);
        engine.session.run_policy = RunPolicy::AutoIfValid;
        engine.begin_work_item_batch_review_run().await;

        engine
            .complete_review(
                ProviderCompletion::plain("first repairable structured verdict", None),
                structured_batch_review_verdict(verdict.clone()),
            )
            .await;
        let automatic_generation_nodes = engine
            .timeline_nodes
            .iter()
            .filter(|node| {
                matches!(
                    node.node_type,
                    TimelineNodeType::WorkItemBatchRun | TimelineNodeType::WorkItemPlanOutlineRun
                )
            })
            .count();
        assert_eq!(automatic_generation_nodes, 1);
        assert_eq!(engine.session.run_history.repairs_used, 1);

        engine.begin_work_item_batch_review_run().await;
        engine
            .complete_review(
                ProviderCompletion::plain("verification repairable structured verdict", None),
                structured_batch_review_verdict(verdict),
            )
            .await;

        assert_eq!(engine.current_stage(), WorkspaceStage::Completed);
        assert_eq!(
            engine.session.session_status,
            WorkspaceSessionStatus::StoppedNeedsHuman
        );
        assert_eq!(engine.session.run_history.repairs_used, 1);
        assert_eq!(
            engine
                .timeline_nodes
                .iter()
                .filter(|node| {
                    matches!(
                        node.node_type,
                        TimelineNodeType::WorkItemBatchRun
                            | TimelineNodeType::WorkItemPlanOutlineRun
                    )
                })
                .count(),
            automatic_generation_nodes,
            "auto mode must stop instead of beginning a second legacy regeneration"
        );
        let persisted = lifecycle
            .get_workspace_session(&engine.session.session_id)
            .expect("persisted auto terminal session");
        assert_eq!(persisted.status, WorkspaceSessionStatus::StoppedNeedsHuman);
        assert_eq!(persisted.run_history.repairs_used, 1);
    }
}

#[tokio::test]
async fn unknown_category_diagnostic_marks_work_item_plan_session_durably_failed() {
    let (_tmp, _checkpoint_store, lifecycle, _plan_id, mut engine) =
        make_work_item_plan_engine_with_draft_candidate("unknown_category_durable_failure");
    engine.begin_work_item_plan_outline_review_run().await;
    let mut verdict = review_verdict(ReviewVerdictType::NeedsHuman);
    verdict.structured_output_diagnostic = Some(StructuredOutputDiagnostic {
        code: "unknown_finding_category".to_string(),
        message: "unknown finding category: invented_category".to_string(),
        repair_attempted: false,
        repair_succeeded: false,
        raw_output_preview: None,
    });

    engine
        .complete_review(ProviderCompletion::plain("invalid category", None), verdict)
        .await;

    assert_eq!(engine.current_stage(), WorkspaceStage::Completed);
    assert_eq!(
        engine.session.session_status,
        WorkspaceSessionStatus::Failed
    );
    assert!(engine.session.human_gate_snapshot.is_none());
    assert_eq!(engine.session.policy_diagnostics.len(), 1);
    assert_eq!(
        engine.session.policy_diagnostics[0].code,
        "unknown_finding_category"
    );
    let persisted = lifecycle
        .get_workspace_session(&engine.session.session_id)
        .expect("persisted failed session");
    assert_eq!(persisted.status, WorkspaceSessionStatus::Failed);
    assert_eq!(
        persisted.policy_diagnostics,
        engine.session.policy_diagnostics
    );
}

/// 已知缺口复现：legacy `review_decision -> continue_with_context` 路径可反复进入
/// outline revision 而不消耗 cycle 预算。rep11 的 ws.jsonl 记录了 rounds 2-8 共 7 轮
/// 连续 revise，但 durable cycle 计数始终为 `initial=1`、`verification=1`。在服务端
/// 将 legacy decision 路径纳入同一预算门之前，本测试必须保持 ignore。
#[tokio::test]
#[ignore = "known gap: legacy review_decision revise loop bypasses cycle budget, see rep11"]
async fn legacy_review_decision_continue_with_context_bypasses_cycle_budget_reproduction() {
    let (_tmp, _lifecycle, _source_node_id, mut engine) =
        prepare_outline_review_decision_without_index(WorkItemPlanReviewScope::Outline).await;
    engine.latest_review_verdict = Some(review_verdict(ReviewVerdictType::Revise));
    // 写入 rep11 已观测到的两次 durable 计数；下方 legacy decision handler
    // 不会更新该 cycle，正是本测试要复现的缺口。
    engine.session.run_history.initial_review_count = 1;
    engine.session.run_history.verification_review_count = 1;
    engine.session.run_history.review_cycles.insert(
        "legacy:rep11".to_string(),
        ReviewCycleState {
            initial_count: 1,
            verification_count: 1,
            ..ReviewCycleState::default()
        },
    );

    for round in 2..=8 {
        let outcome = engine
            .handle_review_decision(
                "continue_with_context".to_string(),
                Some(format!("rep11 legacy revise round {round}")),
            )
            .await
            .expect("legacy continue_with_context should start outline revision");
        assert!(matches!(
            outcome,
            ReviewDecisionOutcome::StartWorkItemPlanOutlineRevision { .. }
        ));

        // 模拟 outline revision provider run 完成后 WS handler 重新进入 legacy
        // review-decision 节点的过程。刻意不调用带 policy 的 `complete_review`：
        // rep11 的旁路正发生在新 durable review 被记录前的 inbound decision 边界。
        engine.begin_work_item_plan_outline_review_run().await;
        engine.session.stage = WorkspaceStage::ReviewDecision;
        engine.latest_review_verdict = Some(review_verdict(ReviewVerdictType::Revise));
        engine
            .create_timeline_node(TimelineNodeDraft {
                node_type: TimelineNodeType::ReviewDecision,
                agent: None,
                stage: WorkspaceStage::ReviewDecision,
                round: Some(round),
                title: format!("rep11 legacy Review Decision Round {round}"),
                summary: Some("continue_with_context reproduction".to_string()),
                status: TimelineNodeStatus::Active,
            })
            .await;
    }

    let cycle_counts: Vec<_> = engine
        .session
        .run_history
        .review_cycles
        .values()
        .map(|cycle| (cycle.initial_count, cycle.verification_count))
        .collect();
    assert_eq!(
        cycle_counts,
        vec![(1, 1)],
        "复现应证明 7 轮 legacy revise loop 期间 durable cycle 仅保留两次计数"
    );
}

#[tokio::test]
async fn legacy_batch_pass_routes_through_compile_to_human_confirmation_in_interactive_mode() {
    let (_temporary_directory, _checkpoint_store, _lifecycle_store, plan_id, mut engine) =
        make_work_item_plan_engine_with_draft_candidate("legacy_batch_pass_interactive");
    prepare_work_item_plan_outline_artifact(&mut engine).await;
    save_batch_work_item_plan_index_with_accepted_drafts(&engine, &plan_id);
    engine.session.stage = WorkspaceStage::CrossReview;
    let nodes_before = engine.timeline_nodes.len();
    let artifact_before = artifact_variant(&engine);

    engine
        .route_work_item_batch_review(review_verdict(ReviewVerdictType::Pass))
        .await;

    assert_eq!(
        normalized_route(&engine, nodes_before),
        NormalizedRoute {
            stage: normalized_stage(WorkspaceStage::HumanConfirm),
            nodes: vec![
                normalized_node(
                    TimelineNodeType::WorkItemPlanCompile,
                    WsWorkspaceStage::Running,
                ),
                normalized_node(
                    TimelineNodeType::HumanConfirm,
                    WsWorkspaceStage::HumanConfirm
                ),
            ],
        },
        "interactive batch pass must preserve the legacy compile then human-confirm route"
    );
    assert_eq!(artifact_variant(&engine), Some("work_item_plan_projection"));
    assert_ne!(artifact_variant(&engine), artifact_before);
    assert_eq!(
        engine.timeline_nodes.last().map(|node| &node.status),
        Some(&TimelineNodeStatus::Active)
    );
    assert_eq!(engine.session.session_status, WorkspaceSessionStatus::WaitingForHuman);
}

#[tokio::test]
async fn auto_if_valid_batch_pass_skips_final_human_confirmation_and_completes() {
    let (_temporary_directory, _checkpoint_store, lifecycle, plan_id, mut engine) =
        make_work_item_plan_engine_with_draft_candidate("legacy_batch_pass_auto");
    prepare_work_item_plan_outline_artifact(&mut engine).await;
    save_batch_work_item_plan_index_with_accepted_drafts(&engine, &plan_id);
    engine.session.run_policy = RunPolicy::AutoIfValid;
    engine.begin_work_item_batch_review_run().await;

    engine
        .complete_review(
            ProviderCompletion::plain("all green batch pass", None),
            review_verdict(ReviewVerdictType::Pass),
        )
        .await;

    assert_eq!(engine.current_stage(), WorkspaceStage::Completed);
    assert_eq!(engine.session.session_status, WorkspaceSessionStatus::Confirmed);
    assert!(
        !engine
            .timeline_nodes
            .iter()
            .any(|node| node.node_type == TimelineNodeType::HumanConfirm),
        "a policy-valid auto run must not create a final waiting_for_human node"
    );
    assert_eq!(
        engine.timeline_nodes.last().map(|node| &node.node_type),
        Some(&TimelineNodeType::Completed)
    );
    let persisted = lifecycle
        .get_workspace_session(&engine.session.session_id)
        .expect("persisted auto-completed plan session");
    assert_eq!(persisted.status, WorkspaceSessionStatus::Confirmed);
}
