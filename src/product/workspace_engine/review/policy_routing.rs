pub use crate::product::work_item_plan_policy::HumanGateSnapshot;
use crate::product::work_item_plan_policy::{
    ClassifiedFinding, FatalReason, FindingFingerprint, HumanReason, PlanOutcome, PolicyDiagnostic,
    ReviewInvocationScope, RunBudgets, RunHistory, RunPolicy,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateSnapshotContext {
    pub history: RunHistory,
    pub budgets: RunBudgets,
    pub invocation: ReviewInvocationScope,
    pub findings: Vec<ClassifiedFinding>,
    pub repeated_fingerprints: Vec<FindingFingerprint>,
    pub trigger: HumanReason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoutingAction {
    ContinueToCompleted,
    TriggerAggregateRepair {
        findings: Vec<ClassifiedFinding>,
    },
    EnterHumanGate {
        snapshot: HumanGateSnapshot,
    },
    StopNeedsHuman {
        snapshot: HumanGateSnapshot,
    },
    AbortFatal {
        reason: FatalReason,
        diagnostics: Vec<PolicyDiagnostic>,
    },
}

pub(crate) fn route_repeated_human_gate_fingerprint(
    snapshot: &HumanGateSnapshot,
    fingerprint: &FindingFingerprint,
) -> Result<(), String> {
    if snapshot.repeated_fingerprints.contains(fingerprint) {
        Ok(())
    } else {
        Err(format!(
            "human gate fingerprint {} is not recorded as repeated",
            fingerprint.0
        ))
    }
}

pub fn route_outcome(
    outcome: PlanOutcome,
    policy: RunPolicy,
    context: GateSnapshotContext,
) -> RoutingAction {
    match outcome {
        PlanOutcome::Valid => RoutingAction::ContinueToCompleted,
        PlanOutcome::Repairable { findings } => RoutingAction::TriggerAggregateRepair { findings },
        PlanOutcome::HumanRequired {
            findings: _,
            repeated_fingerprints,
            reason,
        } => {
            let snapshot = human_gate_snapshot(&context, policy == RunPolicy::AutoIfValid);
            if reason == HumanReason::RepeatedFingerprint
                && let Some(fingerprint) = repeated_fingerprints.iter().find(|fingerprint| {
                    route_repeated_human_gate_fingerprint(&snapshot, fingerprint).is_err()
                })
            {
                return RoutingAction::AbortFatal {
                    reason: FatalReason::SafetyInvariantViolation,
                    diagnostics: vec![PolicyDiagnostic {
                        code: FatalReason::SafetyInvariantViolation.as_code().to_owned(),
                        message: format!(
                            "repeated human gate fingerprint {} is absent from the durable gate snapshot",
                            fingerprint.0
                        ),
                        field: Some("repeated_fingerprints".to_owned()),
                    }],
                };
            }
            match policy {
                RunPolicy::Interactive => RoutingAction::EnterHumanGate { snapshot },
                RunPolicy::AutoIfValid => RoutingAction::StopNeedsHuman { snapshot },
            }
        }
        PlanOutcome::Fatal {
            reason,
            diagnostics,
        } => RoutingAction::AbortFatal {
            reason,
            diagnostics,
        },
    }
}

fn human_gate_snapshot(context: &GateSnapshotContext, resumable: bool) -> HumanGateSnapshot {
    HumanGateSnapshot {
        findings: context.findings.clone(),
        repeated_fingerprints: context.repeated_fingerprints.clone(),
        attempts_used: context
            .history
            .repairs_used
            .saturating_add(context.history.manual_repairs_used),
        manual_repairs_remaining: context
            .budgets
            .max_manual_repairs
            .saturating_sub(context.history.manual_repairs_used),
        trigger: context.trigger,
        resumable,
    }
}
