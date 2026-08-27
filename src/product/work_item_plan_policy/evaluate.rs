use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};

use super::{
    ClassifiedFinding, FindingClass, FindingFingerprint, ReviewCycleState, ReviewInvocationScope,
    ReviewPhase, RunBudgets, RunHistory,
};

/// 已分类 finding 的纯策略裁决输入。
///
/// `invocation` 由服务端生成；本模块只验证其完整性，不读写 durable store。
#[derive(Debug, Clone, Copy)]
pub struct ReviewEvaluationInput<'a> {
    pub mechanical: &'a [ClassifiedFinding],
    pub review: &'a [ClassifiedFinding],
    pub cycle_key: &'a str,
    pub phase: ReviewPhase,
    pub invocation: &'a ReviewInvocationScope,
}

/// 纯策略裁决结果。reservation 仅能由持久化路由层在已知 owner 后物化。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvaluationDecision {
    pub outcome: PlanOutcome,
    pub history_delta: RunHistoryDelta,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanOutcome {
    Valid,
    Repairable {
        findings: Vec<ClassifiedFinding>,
    },
    HumanRequired {
        findings: Vec<ClassifiedFinding>,
        repeated_fingerprints: Vec<FindingFingerprint>,
        reason: HumanReason,
    },
    Fatal {
        reason: FatalReason,
        diagnostics: Vec<PolicyDiagnostic>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FatalReason {
    TransitionBudgetExhausted,
    UnknownCategory,
    UnknownClassHint,
    InvalidStructuredOutput,
    StateCorruption,
    ProtocolViolation,
    PersistenceFailure,
    SafetyInvariantViolation,
}

impl FatalReason {
    pub const fn as_code(self) -> &'static str {
        match self {
            Self::TransitionBudgetExhausted => "transition_budget_exhausted",
            Self::UnknownCategory => "unknown_category",
            Self::UnknownClassHint => "unknown_class_hint",
            Self::InvalidStructuredOutput => "invalid_structured_output",
            Self::StateCorruption => "state_corruption",
            Self::ProtocolViolation => "protocol_violation",
            Self::PersistenceFailure => "persistence_failure",
            Self::SafetyInvariantViolation => "safety_invariant_violation",
        }
    }
}

impl fmt::Display for FatalReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_code())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HumanReason {
    NativeHumanRequired,
    RepeatedFingerprint,
    VerificationNewFindings,
    RepairBudgetExhausted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyDiagnostic {
    pub code: String,
    pub message: String,
    pub field: Option<String>,
}

/// A CAS-safe addition to durable run history.
///
/// The evaluator emits no repair or transition consumption. The durable route
/// handler increments those counters only after the relevant durable protocol
/// step succeeds.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RunHistoryDelta {
    pub seen_fingerprints_to_add: BTreeSet<FindingFingerprint>,
    pub repairs_used_delta: u32,
    pub manual_repairs_used_delta: u32,
    pub transitions_used_delta: u32,
    /// Session-level review totals are incremented for metrics only.
    pub initial_review_count_delta: u32,
    pub verification_review_count_delta: u32,
    /// Per-cycle counter deltas. These are the only review-count budget gate.
    pub review_cycles_to_add: BTreeMap<String, ReviewCycleState>,
}

impl RunHistoryDelta {
    /// Applies this delta atomically to `history`.
    ///
    /// The input history is not modified when an existing invariant is broken,
    /// an addition overflows, or the resulting counters exceed their limits.
    pub fn merge_into(
        &self,
        history: &mut RunHistory,
        budgets: &RunBudgets,
    ) -> Result<(), PlanOutcome> {
        if !history_is_within_limits(history, budgets) {
            return Err(state_corruption());
        }

        let Some(repairs_used) = history.repairs_used.checked_add(self.repairs_used_delta) else {
            return Err(state_corruption());
        };
        let Some(manual_repairs_used) = history
            .manual_repairs_used
            .checked_add(self.manual_repairs_used_delta)
        else {
            return Err(state_corruption());
        };
        let Some(transitions_used) = history
            .transitions_used
            .checked_add(self.transitions_used_delta)
        else {
            return Err(state_corruption());
        };
        let Some(initial_review_count) = history
            .initial_review_count
            .checked_add(self.initial_review_count_delta)
        else {
            return Err(state_corruption());
        };
        let Some(verification_review_count) = history
            .verification_review_count
            .checked_add(self.verification_review_count_delta)
        else {
            return Err(state_corruption());
        };

        let mut seen_fingerprints = history.seen_fingerprints.clone();
        seen_fingerprints.extend(self.seen_fingerprints_to_add.iter().cloned());
        let mut review_cycles = history.review_cycles.clone();
        for (cycle_key, delta) in &self.review_cycles_to_add {
            let cycle = review_cycles.entry(cycle_key.clone()).or_default();
            let Some(repairs_used) = cycle.repairs_used.checked_add(delta.repairs_used) else {
                return Err(state_corruption());
            };
            let Some(initial_count) = cycle.initial_count.checked_add(delta.initial_count) else {
                return Err(state_corruption());
            };
            let Some(verification_count) = cycle
                .verification_count
                .checked_add(delta.verification_count)
            else {
                return Err(state_corruption());
            };
            cycle.repairs_used = repairs_used;
            cycle.initial_count = initial_count;
            cycle.verification_count = verification_count;
        }
        let next = RunHistory {
            seen_fingerprints,
            repairs_used,
            manual_repairs_used,
            transitions_used,
            initial_review_count,
            verification_review_count,
            review_cycles,
        };

        if !history_is_within_limits(&next, budgets) {
            return Err(state_corruption());
        }

        *history = next;
        Ok(())
    }
}

/// Reservation state written only by the durable repair route handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepairReservationState {
    Reserved,
    ProviderStarted,
    Committed,
    Released,
}

/// Durable identity and lifecycle record for one automatic repair attempt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepairReservation {
    pub token: String,
    pub owner_session_id: String,
    pub owner_run_id: String,
    pub provider_start_idempotency_key: String,
    pub state: RepairReservationState,
    pub commit_id: Option<String>,
}

/// Evaluates already-classified findings without performing I/O or allocating
/// durable repair resources.
///
/// Decision priority after invariant/protocol validation is fixed as:
/// transition budget, repeated non-advisory fingerprint, aggregate repair,
/// native human requirement, then valid.
pub fn evaluate(
    input: &ReviewEvaluationInput<'_>,
    history: &RunHistory,
    budgets: &RunBudgets,
) -> EvaluationDecision {
    if !history_is_within_limits(history, budgets) {
        return fatal_decision(FatalReason::StateCorruption, state_corruption_diagnostics());
    }

    if !invocation_matches_phase(input) || input.invocation.validate_digest().is_err() {
        return fatal_decision(
            FatalReason::ProtocolViolation,
            vec![PolicyDiagnostic {
                code: FatalReason::ProtocolViolation.as_code().to_owned(),
                message: "review evaluation phase and invocation scope are invalid".to_owned(),
                field: Some("review_invocation_scope".to_owned()),
            }],
        );
    }

    let mut history_delta = review_count_delta(input.cycle_key, input.phase);
    if !delta_can_merge(history, &history_delta, budgets) {
        return fatal_decision(FatalReason::StateCorruption, state_corruption_diagnostics());
    }

    if budgets.max_transitions == 0 || history.transitions_used >= budgets.max_transitions {
        return EvaluationDecision {
            outcome: PlanOutcome::Fatal {
                reason: FatalReason::TransitionBudgetExhausted,
                diagnostics: vec![PolicyDiagnostic {
                    code: FatalReason::TransitionBudgetExhausted.as_code().to_owned(),
                    message: "stage transition budget is exhausted".to_owned(),
                    field: Some("transitions_used".to_owned()),
                }],
            },
            history_delta,
        };
    }

    let findings = actionable_findings(input);
    history_delta.seen_fingerprints_to_add = findings
        .iter()
        .map(|finding| finding.fingerprint.clone())
        .collect();

    // A verification invocation is intentionally closed over the original
    // finding identities.  A finding outside that scope is not a protocol
    // failure: the reviewer may have found a related issue while rephrasing
    // its report, and that must stop automatic progress at the human gate.
    let verification_new_findings = if input.phase == ReviewPhase::Verification {
        input
            .review
            .iter()
            .filter(|finding| {
                !matches!(finding.class, FindingClass::Advisory)
                    && !matches!(
                        input.invocation,
                        ReviewInvocationScope::Verification {
                            original_fingerprints,
                            ..
                        } if original_fingerprints.contains(&finding.fingerprint)
                    )
            })
            .cloned()
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    if !verification_new_findings.is_empty() {
        return EvaluationDecision {
            outcome: PlanOutcome::HumanRequired {
                findings: verification_new_findings,
                repeated_fingerprints: Vec::new(),
                reason: HumanReason::VerificationNewFindings,
            },
            history_delta,
        };
    }

    let repeated_fingerprints = findings
        .iter()
        .filter(|finding| history.seen_fingerprints.contains(&finding.fingerprint))
        .map(|finding| finding.fingerprint.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if !repeated_fingerprints.is_empty() {
        return EvaluationDecision {
            outcome: PlanOutcome::HumanRequired {
                findings,
                repeated_fingerprints,
                reason: HumanReason::RepeatedFingerprint,
            },
            history_delta,
        };
    }

    let repairable_findings = findings
        .iter()
        .filter(|finding| is_automatic_repair_candidate(finding))
        .cloned()
        .collect::<Vec<_>>();
    if !repairable_findings.is_empty() {
        let cycle_repairs_used = history
            .review_cycles
            .get(input.cycle_key)
            .map(|cycle| cycle.repairs_used)
            .unwrap_or(0);
        if input.phase == ReviewPhase::Initial && cycle_repairs_used < budgets.max_repairs {
            return EvaluationDecision {
                outcome: PlanOutcome::Repairable {
                    findings: repairable_findings,
                },
                history_delta,
            };
        }

        return EvaluationDecision {
            outcome: PlanOutcome::HumanRequired {
                findings,
                repeated_fingerprints: Vec::new(),
                reason: HumanReason::RepairBudgetExhausted,
            },
            history_delta,
        };
    }

    if findings
        .iter()
        .any(|finding| finding.class == FindingClass::HumanRequired)
    {
        return EvaluationDecision {
            outcome: PlanOutcome::HumanRequired {
                findings,
                repeated_fingerprints: Vec::new(),
                reason: HumanReason::NativeHumanRequired,
            },
            history_delta,
        };
    }

    EvaluationDecision {
        outcome: PlanOutcome::Valid,
        history_delta,
    }
}

fn actionable_findings(input: &ReviewEvaluationInput<'_>) -> Vec<ClassifiedFinding> {
    input
        .mechanical
        .iter()
        .chain(input.review)
        .filter(|finding| finding.class != FindingClass::Advisory)
        .cloned()
        .collect()
}

fn is_automatic_repair_candidate(finding: &ClassifiedFinding) -> bool {
    matches!(
        finding.class,
        FindingClass::MechanicalError | FindingClass::Repairable
    )
}

fn invocation_matches_phase(input: &ReviewEvaluationInput<'_>) -> bool {
    matches!(
        (input.phase, input.invocation),
        (ReviewPhase::Initial, ReviewInvocationScope::Initial { .. })
            | (
                ReviewPhase::Verification,
                ReviewInvocationScope::Verification { .. }
            )
    )
}

fn review_count_delta(cycle_key: &str, phase: ReviewPhase) -> RunHistoryDelta {
    let cycle_delta = match phase {
        ReviewPhase::Initial => ReviewCycleState {
            repairs_used: 0,
            initial_count: 1,
            verification_count: 0,
        },
        ReviewPhase::Verification => ReviewCycleState {
            repairs_used: 0,
            initial_count: 0,
            verification_count: 1,
        },
    };
    let mut delta = RunHistoryDelta {
        review_cycles_to_add: BTreeMap::from([(cycle_key.to_owned(), cycle_delta)]),
        ..RunHistoryDelta::default()
    };
    match phase {
        ReviewPhase::Initial => delta.initial_review_count_delta = 1,
        ReviewPhase::Verification => delta.verification_review_count_delta = 1,
    }
    delta
}

fn delta_can_merge(history: &RunHistory, delta: &RunHistoryDelta, budgets: &RunBudgets) -> bool {
    let mut history = history.clone();
    delta.merge_into(&mut history, budgets).is_ok()
}

fn history_is_within_limits(history: &RunHistory, budgets: &RunBudgets) -> bool {
    history.manual_repairs_used <= budgets.max_manual_repairs
        && history.transitions_used <= budgets.max_transitions
        && history.review_cycles.values().all(|cycle| {
            cycle.repairs_used <= budgets.max_repairs
                && cycle.initial_count <= 1
                && cycle.verification_count <= 1
        })
}

fn fatal_decision(reason: FatalReason, diagnostics: Vec<PolicyDiagnostic>) -> EvaluationDecision {
    EvaluationDecision {
        outcome: PlanOutcome::Fatal {
            reason,
            diagnostics,
        },
        history_delta: RunHistoryDelta::default(),
    }
}

fn state_corruption() -> PlanOutcome {
    PlanOutcome::Fatal {
        reason: FatalReason::StateCorruption,
        diagnostics: state_corruption_diagnostics(),
    }
}

fn state_corruption_diagnostics() -> Vec<PolicyDiagnostic> {
    vec![PolicyDiagnostic {
        code: FatalReason::StateCorruption.as_code().to_owned(),
        message: "run history delta cannot be merged within policy limits".to_owned(),
        field: None,
    }]
}

#[cfg(test)]
#[path = "tests_evaluate.rs"]
mod tests_evaluate;
