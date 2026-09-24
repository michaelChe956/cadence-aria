// large_file_guard（>1200 行）拆分：cycle 锚定派生单测从 routing_scope.rs
// 移入本文件（conversational_gate/story_terminate.rs 先例），逻辑零改动。
use super::*;
use crate::product::work_item_plan_policy::ReviewCycleState;

#[test]
fn single_candidate_cycle_is_anchored_on_candidate_revision_identity() {
    let ir_ref = "project/project_0001/issue/issue_0001/plan/plan_0001/plan_candidate_ir/ir-340fe12ab34c5678";
    let cycle_key = "sc:candidate:340fe12ab34c5678".to_string();
    // ① 无 durable scope：按当前候选 source_revision 身份段开新 cycle（Initial）。
    assert_eq!(
        single_candidate_review_cycle(None, Some(ir_ref), &RunHistory::default()),
        Ok((cycle_key.clone(), ReviewPhase::Initial))
    );
    // ② 无 scope 且无候选 ref：fail-closed。
    assert!(single_candidate_review_cycle(None, None, &RunHistory::default()).is_err());

    let anchor_scope = ReviewInvocationScope::initial(ir_ref);
    let repaired_ref = "project/project_0001/issue/issue_0001/plan/plan_0001/plan_candidate_ir/ir-ffffffffffffffff";
    // ③ 初评已计数 → 自动返修后的复评仍是同一 cycle（Verification，锚定初评候选）。
    let anchored = RunHistory {
        review_cycles: std::collections::BTreeMap::from([(
            cycle_key.clone(),
            ReviewCycleState {
                initial_count: 1,
                ..ReviewCycleState::default()
            },
        )]),
        ..RunHistory::default()
    };
    assert_eq!(
        single_candidate_review_cycle(Some(&anchor_scope), Some(repaired_ref), &anchored),
        Ok((cycle_key.clone(), ReviewPhase::Verification))
    );
    // ④ 初评被中断重跑（计数 0；含 reviewer 节点轮换）→ 同 key、Initial。
    assert_eq!(
        single_candidate_review_cycle(Some(&anchor_scope), Some(ir_ref), &RunHistory::default()),
        Ok((cycle_key.clone(), ReviewPhase::Initial))
    );
    // ⑤ Verification scope 显式锚定 chain 头（cycle_anchor_revision_id）。
    let verification_scope = ReviewInvocationScope::verification(
        std::collections::BTreeSet::new(),
        repaired_ref,
        "report-001",
        Some(ir_ref.to_string()),
    );
    assert_eq!(
        single_candidate_review_cycle(
            Some(&verification_scope),
            Some(repaired_ref),
            &RunHistory::default()
        ),
        Ok((cycle_key.clone(), ReviewPhase::Verification))
    );
    // ⑥ 旧 durable Verification（anchor 缺省 None）fail-safe：退化用 repaired 身份段。
    let legacy_verification = ReviewInvocationScope::verification(
        std::collections::BTreeSet::new(),
        repaired_ref,
        "report-001",
        None,
    );
    assert_eq!(
        single_candidate_review_cycle(
            Some(&legacy_verification),
            Some(repaired_ref),
            &RunHistory::default()
        ),
        Ok((
            "sc:candidate:ffffffffffffffff".to_string(),
            ReviewPhase::Verification
        ))
    );
    // ⑦ 旧 review:<node> cycle 记录不迁移：新身份 key 计数为 0 → fresh Initial。
    let legacy_history = RunHistory {
        review_cycles: std::collections::BTreeMap::from([(
            "review:reviewer-node".to_string(),
            ReviewCycleState {
                initial_count: 1,
                verification_count: 1,
                ..ReviewCycleState::default()
            },
        )]),
        ..RunHistory::default()
    };
    assert_eq!(
        single_candidate_review_cycle(Some(&anchor_scope), Some(ir_ref), &legacy_history),
        Ok((cycle_key, ReviewPhase::Initial))
    );
}

#[test]
fn single_candidate_terminal_actions_close_the_review_cycle() {
    let snapshot = crate::product::work_item_plan_policy::HumanGateSnapshot {
        findings: Vec::new(),
        repeated_fingerprints: Vec::new(),
        attempts_used: 0,
        manual_repairs_remaining: 0,
        trigger: crate::product::work_item_plan_policy::HumanReason::NativeHumanRequired,
        resumable: true,
        accepted_feedback_turns: None,
    };
    for action in [
        RoutingAction::ContinueToCompleted,
        RoutingAction::EnterHumanGate {
            snapshot: snapshot.clone(),
        },
        RoutingAction::StopNeedsHuman { snapshot },
        RoutingAction::AbortFatal {
            reason: FatalReason::ProtocolViolation,
            diagnostics: Vec::new(),
        },
    ] {
        assert!(
            closes_single_candidate_cycle(&action),
            "terminal action {action:?} must close the review cycle"
        );
    }
    assert!(!closes_single_candidate_cycle(
        &RoutingAction::TriggerAggregateRepair {
            findings: Vec::new(),
        }
    ));
}

#[test]
fn single_candidate_scope_rejects_initial_scope_during_verification() {
    let result = validate_single_candidate_scope(
        ReviewInvocationScope::initial("revision-001"),
        ReviewPhase::Verification,
    );
    assert!(matches!(
        result,
        Err(RoutingAction::AbortFatal {
            reason: FatalReason::ProtocolViolation,
            ..
        })
    ));
}

#[test]
fn single_candidate_scope_preserves_existing_scope_on_protocol_fatal() {
    let durable_scope = ReviewInvocationScope::initial("durable-revision");
    let replacement_scope = ReviewInvocationScope::initial("replacement-revision");
    let action = RoutingAction::AbortFatal {
        reason: FatalReason::ProtocolViolation,
        diagnostics: Vec::new(),
    };
    assert_eq!(
        scope_for_action(Some(&durable_scope), &replacement_scope, &action),
        Some(durable_scope)
    );
    assert_eq!(scope_for_action(None, &replacement_scope, &action), None);
}

#[test]
fn single_candidate_scope_rejects_verification_scope_during_initial() {
    let result = validate_single_candidate_scope(
        ReviewInvocationScope::verification(
            std::collections::BTreeSet::new(),
            "revision-001",
            "report-001",
            None,
        ),
        ReviewPhase::Initial,
    );
    assert!(matches!(
        result,
        Err(RoutingAction::AbortFatal {
            reason: FatalReason::ProtocolViolation,
            ..
        })
    ));
}
