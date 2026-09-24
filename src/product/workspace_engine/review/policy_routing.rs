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
        // C2（REQ-HGC-01）：纯路由公式只服务新 logical gate——gate-local 轮次
        // 从零起算；同 logical gate 重建由引擎层 carry-forward 覆盖。
        accepted_feedback_turns: Some(0),
    }
}

/// C2（REQ-HGC-01）：durable 快照在场（同一 logical gate 的门 episode 未关闭，
/// 含自动返修期间保留的快照）时，HumanRequired 路由重建的快照预算字段
/// MUST 接续 durable 真值；快照缺席（新 logical gate）保持纯路由公式的
/// 新预算。只覆盖预算双字段，findings/repeated/trigger 仍由本轮 verdict 重建。
pub(super) fn carry_forward_open_gate_budget(
    action: &mut RoutingAction,
    durable: Option<&HumanGateSnapshot>,
) {
    let Some(durable) = durable else {
        return;
    };
    if let RoutingAction::EnterHumanGate { snapshot } = action {
        snapshot.manual_repairs_remaining = durable.manual_repairs_remaining;
        snapshot.accepted_feedback_turns = durable.accepted_feedback_turns;
    }
}

pub(super) fn policy_route_record_values(
    action: &RoutingAction,
) -> (
    crate::product::models::WorkspaceSessionStatus,
    Option<HumanGateSnapshot>,
    Vec<PolicyDiagnostic>,
) {
    match action {
        RoutingAction::EnterHumanGate { snapshot } => (
            crate::product::models::WorkspaceSessionStatus::WaitingForHuman,
            Some(snapshot.clone()),
            Vec::new(),
        ),
        RoutingAction::StopNeedsHuman { snapshot } => (
            crate::product::models::WorkspaceSessionStatus::StoppedNeedsHuman,
            Some(snapshot.clone()),
            Vec::new(),
        ),
        RoutingAction::AbortFatal { diagnostics, .. } => (
            crate::product::models::WorkspaceSessionStatus::Failed,
            None,
            diagnostics.clone(),
        ),
        RoutingAction::ContinueToCompleted | RoutingAction::TriggerAggregateRepair { .. } => (
            crate::product::models::WorkspaceSessionStatus::Running,
            None,
            Vec::new(),
        ),
    }
}
