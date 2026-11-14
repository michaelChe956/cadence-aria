//! 内容级评审缺口的额外自动返修额度（`RunPolicy::AutoIfValid`）：逐工件、有界。
//!
//! 三道闸门同时成立，`HumanRequired` 才改道 `TriggerAggregateRepair`：
//! - **原因**：仅 `NativeHumanRequired` / `VerificationNewFindings`（内容还需
//!   再改）；`RepeatedFingerprint` / `UnstableFindingIdentity` /
//!   `RepairBudgetExhausted` 是安全类结论 ⇒ 照旧停等人工。
//! - **额度**：逐工件读 `review_cycles[cycle_key].repairs_used`，未达
//!   `max_repairs` 才开放；返修消耗仍由既有 `repairs_used` 记账（不新增持久化
//!   字段）⇒ 同一工件最多再返修一次，另一个工件不共享该额度。
//! - **内容**：无 actionable finding（如结构化 needs_human 的空 findings）⇒
//!   没有可执行的返修内容，照旧停等人工。
//!
//! 额度用尽后 `StopNeedsHuman` 仍是兜底（返修后重新评审仍不合格即交门）。

use super::{classified_finding, gate_context};
use crate::product::work_item_plan_policy::{
    FindingClass, HumanReason, PlanOutcome, ReviewCycleState, RunBudgets, RunPolicy,
};
use crate::product::workspace_engine::review::policy_routing::{
    GateSnapshotContext, RoutingAction, route_outcome,
};

/// 夹具：指定原因 + 指定工件 cycle 的 `repairs_used`。`actionable=false`
/// 模拟无内容可返修（空 findings 的交门臂）。
fn cycle_context(
    reason: HumanReason,
    cycle_key: &str,
    repairs_used: u32,
    actionable: bool,
) -> GateSnapshotContext {
    let mut context = gate_context(reason);
    context.cycle_key = Some(cycle_key.to_owned());
    context.findings = if actionable {
        vec![classified_finding(FindingClass::HumanRequired, "内容缺口")]
    } else {
        Vec::new()
    };
    context.history.review_cycles.insert(
        cycle_key.to_owned(),
        ReviewCycleState {
            repairs_used,
            ..ReviewCycleState::default()
        },
    );
    context
}

fn human_required(context: &GateSnapshotContext, reason: HumanReason) -> PlanOutcome {
    PlanOutcome::HumanRequired {
        findings: context.findings.clone(),
        repeated_fingerprints: context.repeated_fingerprints.clone(),
        reason,
    }
}

#[test]
fn content_level_human_required_triggers_the_extra_aggregate_repair_before_its_cycle_budget_is_used() {
    let max_repairs = RunBudgets::default().max_repairs;
    assert!(max_repairs > 0, "夹具前提：A 池默认额度非零");
    for reason in [
        HumanReason::NativeHumanRequired,
        HumanReason::VerificationNewFindings,
    ] {
        for repairs_used in 0..max_repairs {
            let context = cycle_context(reason, "sc:candidate:artifact_a", repairs_used, true);
            let action = route_outcome(
                human_required(&context, reason),
                RunPolicy::AutoIfValid,
                context,
            );
            assert!(
                matches!(action, RoutingAction::TriggerAggregateRepair { .. }),
                "{reason:?}（repairs_used={repairs_used}）必须再走一次自动返修，实际 {action:?}"
            );
        }
    }
}

#[test]
fn extra_aggregate_repair_stops_for_human_once_the_artifact_cycle_budget_is_used() {
    let context = cycle_context(
        HumanReason::NativeHumanRequired,
        "sc:candidate:artifact_a",
        RunBudgets::default().max_repairs,
        true,
    );
    let expected_findings = context.findings.clone();
    let action = route_outcome(
        human_required(&context, HumanReason::NativeHumanRequired),
        RunPolicy::AutoIfValid,
        context,
    );
    let RoutingAction::StopNeedsHuman { snapshot } = action else {
        panic!("逐工件额度用尽后必须停等人工，实际 {action:?}");
    };
    assert_eq!(snapshot.trigger, HumanReason::NativeHumanRequired);
    assert_eq!(snapshot.findings, expected_findings);
    assert!(snapshot.resumable);
}

#[test]
fn safety_human_reasons_never_open_the_extra_aggregate_repair() {
    for reason in [
        HumanReason::RepeatedFingerprint,
        HumanReason::UnstableFindingIdentity,
        HumanReason::RepairBudgetExhausted,
    ] {
        // 额度 fresh（repairs_used=0）：额度本身不是拦截原因，原因闸门才是。
        let context = cycle_context(reason, "sc:candidate:artifact_a", 0, true);
        let action = route_outcome(human_required(&context, reason), RunPolicy::AutoIfValid, context);
        let RoutingAction::StopNeedsHuman { snapshot } = action else {
            panic!("{reason:?} 属安全类结论，不得额外返修，实际 {action:?}");
        };
        assert_eq!(snapshot.trigger, reason);
    }
}

#[test]
fn extra_aggregate_repair_allowance_is_scoped_per_artifact_cycle() {
    // 会话总计 repairs_used=1（== max_repairs，见 gate_context）刻意保留：
    // 额度判定 MUST 逐工件，不得被会话总计耗尽。
    let mut context = cycle_context(
        HumanReason::NativeHumanRequired,
        "draft:artifact_a",
        RunBudgets::default().max_repairs,
        true,
    );
    context.history.review_cycles.insert(
        "draft:artifact_b".to_owned(),
        ReviewCycleState::default(),
    );

    let mut exhausted = context.clone();
    exhausted.cycle_key = Some("draft:artifact_a".to_owned());
    let action = route_outcome(
        human_required(&exhausted, HumanReason::NativeHumanRequired),
        RunPolicy::AutoIfValid,
        exhausted,
    );
    assert!(
        matches!(action, RoutingAction::StopNeedsHuman { .. }),
        "工件 A 额度已用尽 ⇒ 停等人工，实际 {action:?}"
    );

    let mut fresh = context;
    fresh.cycle_key = Some("draft:artifact_b".to_owned());
    let action = route_outcome(
        human_required(&fresh, HumanReason::NativeHumanRequired),
        RunPolicy::AutoIfValid,
        fresh,
    );
    assert!(
        matches!(action, RoutingAction::TriggerAggregateRepair { .. }),
        "工件 B 有独立额度，不共享 A 的耗尽状态，实际 {action:?}"
    );
}

#[test]
fn content_level_extra_repair_stays_conservative_without_findings_or_cycle_identity() {
    let without_findings = cycle_context(
        HumanReason::NativeHumanRequired,
        "sc:candidate:artifact_a",
        0,
        false,
    );
    let action = route_outcome(
        human_required(&without_findings, HumanReason::NativeHumanRequired),
        RunPolicy::AutoIfValid,
        without_findings,
    );
    assert!(
        matches!(action, RoutingAction::StopNeedsHuman { .. }),
        "无 actionable finding ⇒ 无内容可返修，必须交门，实际 {action:?}"
    );

    let mut unknown_cycle = cycle_context(
        HumanReason::NativeHumanRequired,
        "sc:candidate:artifact_a",
        0,
        true,
    );
    unknown_cycle.cycle_key = None;
    let action = route_outcome(
        human_required(&unknown_cycle, HumanReason::NativeHumanRequired),
        RunPolicy::AutoIfValid,
        unknown_cycle,
    );
    assert!(
        matches!(action, RoutingAction::StopNeedsHuman { .. }),
        "cycle 身份未知 ⇒ 保守交门，实际 {action:?}"
    );
}
