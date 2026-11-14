use super::routing::routing_platform_rescue::is_platform_compile_failure_round;
pub use crate::product::work_item_plan_policy::HumanGateSnapshot;
use crate::product::work_item_plan_policy::{
    ClassifiedFinding, EvaluationDecision, FatalReason, FindingClass, FindingFingerprint,
    HumanReason, PlanOutcome, PolicyDiagnostic, ReviewInvocationScope, RunBudgets, RunHistory,
    RunPolicy,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateSnapshotContext {
    pub history: RunHistory,
    pub budgets: RunBudgets,
    pub invocation: ReviewInvocationScope,
    pub findings: Vec<ClassifiedFinding>,
    pub repeated_fingerprints: Vec<FindingFingerprint>,
    pub trigger: HumanReason,
    /// 本次评审工件的 review-cycle 身份（`RunHistory::review_cycles` 的键）。
    /// 内容级缺口的额外自动返修额度逐工件判定；缺席 ⇒ 不开放（保守交门）。
    pub cycle_key: Option<String>,
    /// Task 4（工作包 1.5）：统一权威余额（durable 读侧组装）。平台编译
    /// 失败轮 ⇒ 快照净余额 := 该值；其它轮保持「新 logical gate 初值」公式。
    pub author_budget_available: Option<u32>,
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
            findings,
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
                // 内容级缺口（内容还需再改）在 AutoIfValid 下不再直接停等人工：
                // 逐工件额度未用时再走一次自动返修，返修后重新评审；仍不合格
                // （额度用尽）才照旧 StopNeedsHuman。
                RunPolicy::AutoIfValid if content_level_repair_available(&context, reason) => {
                    RoutingAction::TriggerAggregateRepair { findings }
                }
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

/// 内容级缺口（“内容还需再改”）——可在 AutoIfValid 下换取一次额外自动返修
/// 的原因集合。`RepeatedFingerprint` / `UnstableFindingIdentity` /
/// `RepairBudgetExhausted` 属安全类结论（重复指纹、finding 身份不可判定、
/// 预算已耗尽）：额外返修只会制造循环或掩盖不可判定身份，MUST NOT 开放。
fn is_content_level_human_reason(reason: HumanReason) -> bool {
    matches!(
        reason,
        HumanReason::NativeHumanRequired | HumanReason::VerificationNewFindings
    )
}

/// 内容级缺口的额外自动返修额度：**逐工件**、**有界**。
///
/// - 逐工件：额度读 `history.review_cycles[cycle_key].repairs_used`——会话总计
///   （`history.repairs_used`）是多工件共享的观测计数，用它判定会让一个工件
///   的返修吃掉另一个工件的额度（`review_cycles` 是唯一预算闸门）。
/// - 有界：额度未用 ⇔ 该工件 cycle 的 `repairs_used < budgets.max_repairs`；
///   返修事实仍由既有 `repairs_used` 记账（routing 层既有 delta），不新增
///   持久化字段，也不会超过 `max_repairs` 上限 ⇒ 同一工件最多再返修一次。
/// - 内容：没有 actionable finding（如结构化 needs_human 的空 findings）就
///   没有可执行的返修内容 ⇒ 照旧交门。
/// - 身份未知（`cycle_key` 缺席）⇒ 不开放（保守交门）。
fn content_level_repair_available(context: &GateSnapshotContext, reason: HumanReason) -> bool {
    if !is_content_level_human_reason(reason) {
        return false;
    }
    if !context
        .findings
        .iter()
        .any(|finding| finding.class != FindingClass::Advisory)
    {
        return false;
    }
    let Some(cycle_key) = context.cycle_key.as_deref() else {
        return false;
    };
    context
        .history
        .review_cycles
        .get(cycle_key)
        .is_none_or(|cycle| cycle.repairs_used < context.budgets.max_repairs)
}

fn human_gate_snapshot(context: &GateSnapshotContext, resumable: bool) -> HumanGateSnapshot {
    // Task 4（工作包 1.5）：平台编译失败轮（author 重跑链）的门快照净余额
    // := 统一权威余额（余额未知 ⇒ 0，保守交门）；自动段消费事实由持久化
    // 面同 CAS 结转清零（platform_gate_carried_budget）。非平台轮（评审
    // finding 驱动）是全新 logical gate，保持既有重置公式——重置合法：
    // 自动段未沉入该快照、账本继续可读，无双计。
    let manual_repairs_remaining = if is_platform_compile_failure_round(&context.findings) {
        context.author_budget_available.unwrap_or(0)
    } else {
        context
            .budgets
            .max_manual_repairs
            .saturating_sub(context.history.manual_repairs_used)
    };
    HumanGateSnapshot {
        candidate_recovery: None,
        findings: context.findings.clone(),
        repeated_fingerprints: context.repeated_fingerprints.clone(),
        attempts_used: context
            .history
            .repairs_used
            .saturating_add(context.history.manual_repairs_used),
        manual_repairs_remaining,
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

/// 把 evaluator 的纯裁决装配成路由输入（路由层唯一入口）：`HumanRequired`
/// 的 findings/repeated/trigger 取裁决本身；`Repairable` 以
/// `RepairBudgetExhausted` 作门快照 trigger；其余为原生人工需求。
/// `cycle_key` 是本评审工件的 review-cycle 身份——内容级缺口的额外自动返修
/// 额度逐工件判定（不是会话总计）。
pub(super) fn route_outcome_from_decision(
    decision: EvaluationDecision,
    policy: RunPolicy,
    history: &RunHistory,
    invocation: &ReviewInvocationScope,
    cycle_key: &str,
    findings: Vec<ClassifiedFinding>,
    author_budget_available: Option<u32>,
) -> RoutingAction {
    let (outcome, outcome_findings, repeated_fingerprints, trigger) = match &decision.outcome {
        PlanOutcome::HumanRequired {
            findings,
            repeated_fingerprints,
            reason,
        } => (
            decision.outcome.clone(),
            findings.clone(),
            repeated_fingerprints.clone(),
            *reason,
        ),
        PlanOutcome::Repairable { findings } => (
            decision.outcome.clone(),
            findings.clone(),
            Vec::new(),
            HumanReason::RepairBudgetExhausted,
        ),
        _ => (
            decision.outcome.clone(),
            findings,
            Vec::new(),
            HumanReason::NativeHumanRequired,
        ),
    };
    route_outcome(
        outcome,
        policy,
        GateSnapshotContext {
            history: history.clone(),
            budgets: RunBudgets::default(),
            invocation: invocation.clone(),
            findings: outcome_findings,
            repeated_fingerprints,
            trigger,
            cycle_key: Some(cycle_key.to_owned()),
            author_budget_available,
        },
    )
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
