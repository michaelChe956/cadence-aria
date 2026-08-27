use std::collections::BTreeSet;

use super::{
    ClassifiedFinding, EvaluationDecision, FatalReason, FindingClass, FindingFingerprint,
    HumanReason, PlanOutcome, PolicyDiagnostic, RepairReservation, RepairReservationState,
    ReviewCycleState, ReviewEvaluationInput, ReviewInvocationScope, ReviewPhase, RunBudgets,
    RunHistory, RunHistoryDelta, evaluate,
};

fn finding(class: FindingClass, message: &str) -> ClassifiedFinding {
    ClassifiedFinding {
        class,
        fingerprint: FindingFingerprint::for_finding(None, class, message, None),
        category: None,
        severity: "error".to_owned(),
        message: message.to_owned(),
        evidence: None,
        required_action: None,
        contract_field: None,
    }
}

fn structured_finding(
    class: FindingClass,
    category: super::super::ReviewFindingCategory,
    contract_field: &str,
    message: &str,
) -> ClassifiedFinding {
    ClassifiedFinding {
        class,
        fingerprint: FindingFingerprint::for_finding(
            Some(category),
            class,
            message,
            Some(contract_field),
        ),
        category: Some(category),
        severity: "error".to_owned(),
        message: message.to_owned(),
        evidence: None,
        required_action: None,
        contract_field: Some(contract_field.to_owned()),
    }
}

fn evaluate_findings(
    mechanical: &[ClassifiedFinding],
    review: &[ClassifiedFinding],
    phase: ReviewPhase,
    history: &RunHistory,
    budgets: &RunBudgets,
) -> EvaluationDecision {
    let invocation = match phase {
        ReviewPhase::Initial => ReviewInvocationScope::initial("initial-revision"),
        ReviewPhase::Verification => {
            ReviewInvocationScope::verification(BTreeSet::new(), "repaired-revision", "report")
        }
    };
    evaluate(
        &ReviewEvaluationInput {
            mechanical,
            review,
            cycle_key: "draft:two",
            phase,
            invocation: &invocation,
        },
        history,
        budgets,
    )
}

fn assert_fatal(decision: &EvaluationDecision, expected: FatalReason) {
    match &decision.outcome {
        PlanOutcome::Fatal { reason, .. } => assert_eq!(*reason, expected),
        outcome => panic!("expected fatal {expected:?}, got {outcome:?}"),
    }
}

#[test]
fn verification_new_finding_is_human_required_instead_of_protocol_fatal() {
    let new_finding = structured_finding(
        FindingClass::Repairable,
        super::super::ReviewFindingCategory::ScopeConflict,
        "contract.field",
        "reviewer found a new conflict",
    );
    let original = structured_finding(
        FindingClass::Repairable,
        super::super::ReviewFindingCategory::ContractGap,
        "contract.field",
        "original finding",
    );
    let invocation = ReviewInvocationScope::verification(
        BTreeSet::from([original.fingerprint]),
        "repaired-revision",
        "report",
    );
    let decision = evaluate(
        &ReviewEvaluationInput {
            mechanical: &[],
            review: std::slice::from_ref(&new_finding),
            cycle_key: "draft:two",
            phase: ReviewPhase::Verification,
            invocation: &invocation,
        },
        &RunHistory::default(),
        &RunBudgets::default(),
    );

    assert_eq!(
        decision.outcome,
        PlanOutcome::HumanRequired {
            findings: vec![new_finding],
            repeated_fingerprints: vec![],
            reason: HumanReason::VerificationNewFindings,
        }
    );
}

#[test]
fn transition_budget_exhaustion_is_the_first_policy_priority() {
    let repeated = finding(FindingClass::Repairable, "same finding");
    let history = RunHistory {
        transitions_used: 2,
        seen_fingerprints: BTreeSet::from([repeated.fingerprint.clone()]),
        ..RunHistory::default()
    };
    let budgets = RunBudgets {
        max_transitions: 2,
        ..RunBudgets::default()
    };

    let decision = evaluate_findings(
        &[],
        std::slice::from_ref(&repeated),
        ReviewPhase::Initial,
        &history,
        &budgets,
    );

    assert_fatal(&decision, FatalReason::TransitionBudgetExhausted);
    assert_eq!(decision.history_delta.transitions_used_delta, 0);

    let zero_transition_budget = RunBudgets {
        max_transitions: 0,
        ..RunBudgets::default()
    };
    let decision = evaluate_findings(
        &[],
        &[],
        ReviewPhase::Initial,
        &RunHistory::default(),
        &zero_transition_budget,
    );
    assert_fatal(&decision, FatalReason::TransitionBudgetExhausted);
}

#[test]
fn a_repeated_non_advisory_fingerprint_requires_human_before_repair_or_native_human() {
    let repeated = finding(FindingClass::Repairable, "same finding");
    let another_repair = finding(FindingClass::MechanicalError, "mechanical finding");
    let native_human = finding(FindingClass::HumanRequired, "needs a decision");
    let history = RunHistory {
        seen_fingerprints: BTreeSet::from([repeated.fingerprint.clone()]),
        ..RunHistory::default()
    };

    let decision = evaluate_findings(
        std::slice::from_ref(&another_repair),
        &[repeated.clone(), native_human.clone()],
        ReviewPhase::Initial,
        &history,
        &RunBudgets::default(),
    );

    assert_eq!(
        decision.outcome,
        PlanOutcome::HumanRequired {
            findings: vec![
                another_repair.clone(),
                repeated.clone(),
                native_human.clone()
            ],
            repeated_fingerprints: vec![repeated.fingerprint.clone()],
            reason: HumanReason::RepeatedFingerprint,
        }
    );
    assert_eq!(
        decision.history_delta.seen_fingerprints_to_add,
        BTreeSet::from([
            another_repair.fingerprint,
            repeated.fingerprint,
            native_human.fingerprint,
        ])
    );
}

#[test]
fn advisory_fingerprints_never_trigger_repetition_or_history_tracking() {
    let advisory = finding(FindingClass::Advisory, "optional wording");
    let history = RunHistory {
        seen_fingerprints: BTreeSet::from([advisory.fingerprint.clone()]),
        ..RunHistory::default()
    };

    let decision = evaluate_findings(
        &[],
        std::slice::from_ref(&advisory),
        ReviewPhase::Initial,
        &history,
        &RunBudgets::default(),
    );

    assert_eq!(decision.outcome, PlanOutcome::Valid);
    assert!(decision.history_delta.seen_fingerprints_to_add.is_empty());
}

#[test]
fn initial_mechanical_and_review_repairables_share_one_automatic_repair_budget() {
    let mechanical = finding(FindingClass::MechanicalError, "mechanical failure");
    let review = finding(FindingClass::Repairable, "missing contract field");

    let decision = evaluate_findings(
        std::slice::from_ref(&mechanical),
        std::slice::from_ref(&review),
        ReviewPhase::Initial,
        &RunHistory::default(),
        &RunBudgets::default(),
    );

    assert_eq!(
        decision.outcome,
        PlanOutcome::Repairable {
            findings: vec![mechanical.clone(), review.clone()],
        }
    );
    assert_eq!(decision.history_delta.repairs_used_delta, 0);
    assert_eq!(
        decision.history_delta.seen_fingerprints_to_add,
        BTreeSet::from([mechanical.fingerprint, review.fingerprint])
    );
}

#[test]
fn repair_budget_is_cycle_scoped_and_zero_budget_requires_human_instead_of_fatal() {
    let repairable = finding(FindingClass::Repairable, "repair needed");
    let session_with_prior_repairs = RunHistory {
        repairs_used: RunBudgets::default().max_repairs,
        ..RunHistory::default()
    };

    let fresh_cycle = evaluate_findings(
        &[],
        std::slice::from_ref(&repairable),
        ReviewPhase::Initial,
        &session_with_prior_repairs,
        &RunBudgets::default(),
    );
    assert_eq!(
        fresh_cycle.outcome,
        PlanOutcome::Repairable {
            findings: vec![repairable.clone()],
        }
    );

    let zero_budget = evaluate_findings(
        &[],
        std::slice::from_ref(&repairable),
        ReviewPhase::Initial,
        &RunHistory::default(),
        &RunBudgets {
            max_repairs: 0,
            ..RunBudgets::default()
        },
    );
    assert_eq!(
        zero_budget.outcome,
        PlanOutcome::HumanRequired {
            findings: vec![repairable.clone()],
            repeated_fingerprints: vec![],
            reason: HumanReason::RepairBudgetExhausted,
        }
    );
}

#[test]
fn same_cycle_repair_budget_exhaustion_requires_human() {
    let repairable = finding(FindingClass::Repairable, "repair needed");
    let history = RunHistory {
        review_cycles: std::collections::BTreeMap::from([(
            "draft:two".to_owned(),
            ReviewCycleState {
                repairs_used: RunBudgets::default().max_repairs,
                ..ReviewCycleState::default()
            },
        )]),
        ..RunHistory::default()
    };

    let decision = evaluate_findings(
        &[],
        std::slice::from_ref(&repairable),
        ReviewPhase::Initial,
        &history,
        &RunBudgets::default(),
    );

    assert_eq!(
        decision.outcome,
        PlanOutcome::HumanRequired {
            findings: vec![repairable],
            repeated_fingerprints: vec![],
            reason: HumanReason::RepairBudgetExhausted,
        }
    );
}

#[test]
fn verification_repairable_never_opens_a_second_automatic_repair() {
    let repairable = finding(
        FindingClass::MechanicalError,
        "verification mechanical failure",
    );

    let decision = evaluate_findings(
        std::slice::from_ref(&repairable),
        &[],
        ReviewPhase::Verification,
        &RunHistory::default(),
        &RunBudgets::default(),
    );

    assert_eq!(
        decision.outcome,
        PlanOutcome::HumanRequired {
            findings: vec![repairable],
            repeated_fingerprints: vec![],
            reason: HumanReason::RepairBudgetExhausted,
        }
    );
}

#[test]
fn repairable_priority_precedes_native_human_required() {
    let repairable = finding(FindingClass::Repairable, "repair first");
    let native_human = finding(FindingClass::HumanRequired, "human after repair");

    let decision = evaluate_findings(
        &[],
        &[repairable.clone(), native_human],
        ReviewPhase::Initial,
        &RunHistory::default(),
        &RunBudgets::default(),
    );

    assert_eq!(
        decision.outcome,
        PlanOutcome::Repairable {
            findings: vec![repairable],
        }
    );
}

#[test]
fn native_human_required_is_returned_before_valid() {
    let native_human = finding(FindingClass::HumanRequired, "unresolved product decision");

    let decision = evaluate_findings(
        &[],
        std::slice::from_ref(&native_human),
        ReviewPhase::Initial,
        &RunHistory::default(),
        &RunBudgets::default(),
    );

    assert_eq!(
        decision.outcome,
        PlanOutcome::HumanRequired {
            findings: vec![native_human],
            repeated_fingerprints: vec![],
            reason: HumanReason::NativeHumanRequired,
        }
    );
}

#[test]
fn invalid_scope_or_phase_pair_is_a_protocol_violation() {
    let invocation = ReviewInvocationScope::initial("initial-revision");
    let decision = evaluate(
        &ReviewEvaluationInput {
            mechanical: &[],
            review: &[],
            cycle_key: "draft:two",
            phase: ReviewPhase::Verification,
            invocation: &invocation,
        },
        &RunHistory::default(),
        &RunBudgets::default(),
    );

    assert_fatal(&decision, FatalReason::ProtocolViolation);
    assert_eq!(decision.history_delta, RunHistoryDelta::default());
}

#[test]
fn valid_outcome_has_phase_specific_review_count_delta_and_no_transition_delta() {
    let initial = evaluate_findings(
        &[],
        &[],
        ReviewPhase::Initial,
        &RunHistory::default(),
        &RunBudgets::default(),
    );
    assert_eq!(initial.outcome, PlanOutcome::Valid);
    assert_eq!(initial.history_delta.initial_review_count_delta, 1);
    assert_eq!(initial.history_delta.verification_review_count_delta, 0);
    assert_eq!(initial.history_delta.transitions_used_delta, 0);

    let verification = evaluate_findings(
        &[],
        &[],
        ReviewPhase::Verification,
        &RunHistory::default(),
        &RunBudgets::default(),
    );
    assert_eq!(verification.outcome, PlanOutcome::Valid);
    assert_eq!(verification.history_delta.initial_review_count_delta, 0);
    assert_eq!(
        verification.history_delta.verification_review_count_delta,
        1
    );
    assert_eq!(verification.history_delta.transitions_used_delta, 0);
}

#[test]
fn review_budgets_are_scoped_to_the_review_cycle_not_the_session_total() {
    let budgets = RunBudgets::default();
    let history = RunHistory {
        initial_review_count: 2,
        verification_review_count: 2,
        review_cycles: std::collections::BTreeMap::from([(
            "outline:one".to_owned(),
            ReviewCycleState {
                repairs_used: 0,
                initial_count: 1,
                verification_count: 1,
            },
        )]),
        ..RunHistory::default()
    };

    let decision = evaluate_findings(&[], &[], ReviewPhase::Initial, &history, &budgets);

    assert_eq!(decision.outcome, PlanOutcome::Valid);
    assert_eq!(decision.history_delta.initial_review_count_delta, 1);
    assert_eq!(
        decision.history_delta.review_cycles_to_add,
        std::collections::BTreeMap::from([(
            "draft:two".to_owned(),
            ReviewCycleState {
                repairs_used: 0,
                initial_count: 1,
                verification_count: 0,
            },
        )]),
    );
}

#[test]
fn a_second_verification_in_the_same_cycle_is_state_corruption_but_other_cycles_are_allowed() {
    let budgets = RunBudgets::default();
    let history = RunHistory {
        review_cycles: std::collections::BTreeMap::from([(
            "draft:one".to_owned(),
            ReviewCycleState {
                repairs_used: 0,
                initial_count: 1,
                verification_count: 1,
            },
        )]),
        ..RunHistory::default()
    };

    let decision = evaluate_findings(&[], &[], ReviewPhase::Verification, &history, &budgets);
    assert_eq!(decision.outcome, PlanOutcome::Valid);
    assert_eq!(
        decision.history_delta.review_cycles_to_add,
        std::collections::BTreeMap::from([(
            "draft:two".to_owned(),
            ReviewCycleState {
                repairs_used: 0,
                initial_count: 0,
                verification_count: 1,
            },
        )]),
    );

    let second_verification = RunHistoryDelta {
        review_cycles_to_add: std::collections::BTreeMap::from([(
            "draft:one".to_owned(),
            ReviewCycleState {
                repairs_used: 0,
                initial_count: 0,
                verification_count: 1,
            },
        )]),
        ..RunHistoryDelta::default()
    };
    let mut same_cycle = history;
    assert_fatal(
        &EvaluationDecision {
            outcome: second_verification
                .merge_into(&mut same_cycle, &budgets)
                .unwrap_err(),
            history_delta: RunHistoryDelta::default(),
        },
        FatalReason::StateCorruption,
    );
}

#[test]
fn corrupt_history_or_a_delta_that_exceeds_limits_is_fatal_state_corruption() {
    let repairable = finding(FindingClass::Repairable, "repair needed");
    let corrupt_history = RunHistory {
        review_cycles: std::collections::BTreeMap::from([(
            "draft:one".to_owned(),
            ReviewCycleState {
                repairs_used: 0,
                initial_count: 2,
                verification_count: 0,
            },
        )]),
        ..RunHistory::default()
    };
    let budgets = RunBudgets::default();

    let decision = evaluate_findings(
        &[],
        std::slice::from_ref(&repairable),
        ReviewPhase::Initial,
        &corrupt_history,
        &budgets,
    );
    assert_fatal(&decision, FatalReason::StateCorruption);

    let delta = RunHistoryDelta {
        initial_review_count_delta: 1,
        review_cycles_to_add: std::collections::BTreeMap::from([(
            "draft:one".to_owned(),
            ReviewCycleState {
                repairs_used: 0,
                initial_count: 1,
                verification_count: 0,
            },
        )]),
        ..RunHistoryDelta::default()
    };
    let mut reviewed_once = RunHistory {
        initial_review_count: 1,
        review_cycles: std::collections::BTreeMap::from([(
            "draft:one".to_owned(),
            ReviewCycleState {
                repairs_used: 0,
                initial_count: 1,
                verification_count: 0,
            },
        )]),
        ..RunHistory::default()
    };
    assert_fatal(
        &EvaluationDecision {
            outcome: delta
                .merge_into(&mut reviewed_once, &RunBudgets::default())
                .unwrap_err(),
            history_delta: RunHistoryDelta::default(),
        },
        FatalReason::StateCorruption,
    );
}

#[test]
fn run_history_delta_merges_sets_and_all_counters_with_checked_addition() {
    let seen = FindingFingerprint::for_finding(None, FindingClass::Repairable, "new", None);
    let existing = FindingFingerprint::for_finding(None, FindingClass::Repairable, "old", None);
    let delta = RunHistoryDelta {
        seen_fingerprints_to_add: BTreeSet::from([seen.clone(), existing.clone()]),
        repairs_used_delta: 1,
        manual_repairs_used_delta: 1,
        transitions_used_delta: 1,
        initial_review_count_delta: 1,
        verification_review_count_delta: 1,
        review_cycles_to_add: std::collections::BTreeMap::from([(
            "draft:two".to_owned(),
            ReviewCycleState {
                repairs_used: 1,
                initial_count: 1,
                verification_count: 1,
            },
        )]),
    };
    let mut history = RunHistory {
        seen_fingerprints: BTreeSet::from([existing.clone()]),
        ..RunHistory::default()
    };
    let budgets = RunBudgets {
        max_repairs: 2,
        max_transitions: 2,
        max_manual_repairs: 2,
    };

    delta.merge_into(&mut history, &budgets).unwrap();

    assert_eq!(history.seen_fingerprints, BTreeSet::from([existing, seen]));
    assert_eq!(history.repairs_used, 1);
    assert_eq!(history.manual_repairs_used, 1);
    assert_eq!(history.transitions_used, 1);
    assert_eq!(history.initial_review_count, 1);
    assert_eq!(history.verification_review_count, 1);
    assert_eq!(
        history.review_cycles["draft:two"],
        ReviewCycleState {
            repairs_used: 1,
            initial_count: 1,
            verification_count: 1,
        }
    );
}

#[test]
fn delta_merge_rejects_overflow_and_does_not_partially_mutate_history() {
    let delta = RunHistoryDelta {
        repairs_used_delta: 1,
        ..RunHistoryDelta::default()
    };
    let mut history = RunHistory {
        repairs_used: u32::MAX,
        ..RunHistory::default()
    };
    let original = history.clone();
    let budgets = RunBudgets {
        max_repairs: u32::MAX,
        ..RunBudgets::default()
    };

    let outcome = delta.merge_into(&mut history, &budgets).unwrap_err();

    assert_eq!(history, original);
    assert_eq!(
        outcome,
        PlanOutcome::Fatal {
            reason: FatalReason::StateCorruption,
            diagnostics: vec![PolicyDiagnostic {
                code: "state_corruption".to_owned(),
                message: "run history delta cannot be merged within policy limits".to_owned(),
                field: None,
            }],
        }
    );
}

#[test]
fn zero_manual_repair_budget_never_causes_the_evaluator_to_approve_manual_repair() {
    let native_human = finding(FindingClass::HumanRequired, "manual gate");
    let budgets = RunBudgets {
        max_manual_repairs: 0,
        ..RunBudgets::default()
    };

    let decision = evaluate_findings(
        &[],
        std::slice::from_ref(&native_human),
        ReviewPhase::Initial,
        &RunHistory::default(),
        &budgets,
    );

    assert_eq!(
        decision.outcome,
        PlanOutcome::HumanRequired {
            findings: vec![native_human],
            repeated_fingerprints: vec![],
            reason: HumanReason::NativeHumanRequired,
        }
    );
    assert_eq!(decision.history_delta.manual_repairs_used_delta, 0);
}

#[test]
fn fatal_reason_display_and_reservation_state_have_stable_snake_case_wire_values() {
    assert_eq!(
        FatalReason::TransitionBudgetExhausted.to_string(),
        "transition_budget_exhausted"
    );
    assert_eq!(
        serde_json::to_value(FatalReason::StateCorruption).unwrap(),
        serde_json::json!("state_corruption")
    );
    assert_eq!(
        serde_json::to_value(RepairReservationState::ProviderStarted).unwrap(),
        serde_json::json!("provider_started")
    );
    assert_eq!(
        serde_json::to_value(HumanReason::RepairBudgetExhausted).unwrap(),
        serde_json::json!("repair_budget_exhausted")
    );

    let reservation = RepairReservation {
        token: "token".to_owned(),
        owner_session_id: "session".to_owned(),
        owner_run_id: "run".to_owned(),
        provider_start_idempotency_key: "idempotency".to_owned(),
        state: RepairReservationState::Reserved,
        commit_id: None,
    };
    assert_eq!(
        serde_json::from_value::<RepairReservation>(serde_json::to_value(&reservation).unwrap())
            .unwrap(),
        reservation
    );
}
