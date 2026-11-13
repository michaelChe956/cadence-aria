// P05 schema slice（TeachingDecision 事件）RED 反例 —— 仅 /tmp 备件，未落仓、
// 未挂载（等 owner GO；不在同窗口产未知 variant 的缺接口 RED）。
// 落位：src/product/lifecycle_store/tests/sc_failure_evidence_teaching_decision.rs
// 挂载：sc_failure_evidence.rs 末尾
//   #[cfg(test)]
//   #[path = "tests/sc_failure_evidence_teaching_decision.rs"] mod teaching_decision_tests;
//
// 契约（owner 已批，fail-closed）：
// - 新增事件 TeachingDecision { failed_round_id: String, decision: ScTeachingDecision }；
//   DTO 不扩（复用既有 ScTeachingDecision）。
// - 决策必须针对真实机械失败轮：该 failed_round_id 必须已 RoundBound 且已有
//   FailureObserved；未绑定/未观察到失败时不得写决策（不在 audit 伪造轮次）。
// - 槽位 `teachingdecision/{failed_round_id}` 单槽：同正文幂等、异 decision 冲突。
// - 坏链上的当前事实由 terminal 的 incomplete prefix / missing 保留，不绕 strict
//   日志在坏链追加。

use super::fixture::{
    begin, bound_round, chain_dir, live_sc_session, observed, setup, RAW1, SOURCE1,
};
use crate::product::lifecycle_store::sc_failure_evidence::ScFailureEvidenceEvent;
use crate::web::workspace_ws_types::ScTeachingDecision;

#[test]
fn sc_failure_contract_teaching_decision_requires_observed_failure_and_is_idempotent() {
    let (_tmp, store) = setup();
    let session_id = live_sc_session(&store);
    let handle = begin(&store, &session_id);
    let (chain, round) = bound_round(&store, &session_id, &handle);
    let log_path = chain_dir(&store, &chain).join("rounds.jsonl");
    // ① 未观察到机械失败的 round：决策不得写（不伪造审计轮）
    let err = store
        .append_sc_failure_evidence(
            &chain,
            &ScFailureEvidenceEvent::TeachingDecision {
                failed_round_id: round.round_id.clone(),
                decision: ScTeachingDecision::Ineligible,
            },
        )
        .expect_err("未观察到机械失败的 round 不得写教学决定");
    assert_eq!(err.code, "sc_failure_round_not_bound");
    assert_eq!(err.step, "append_sc_failure_evidence");
    assert!(!err.publication_incomplete);
    assert!(!log_path.exists(), "被拒写不得落日志");
    // ② 绑定 + 观察到真实失败后：决策可写，单槽 `teachingdecision/{failed_round_id}`
    store
        .append_sc_failure_evidence(
            &chain,
            &ScFailureEvidenceEvent::RoundBound {
                round: round.clone(),
            },
        )
        .expect("bind");
    store
        .append_sc_failure_evidence(&chain, &observed(&round, RAW1, SOURCE1))
        .expect("observed");
    let first = store
        .append_sc_failure_evidence(
            &chain,
            &ScFailureEvidenceEvent::TeachingDecision {
                failed_round_id: round.round_id.clone(),
                decision: ScTeachingDecision::Ineligible,
            },
        )
        .expect("首次教学决定必须可写");
    assert_eq!(
        first.entry_key,
        format!("teachingdecision/{}", round.round_id)
    );
    let after_first = std::fs::read(&log_path).expect("read log");
    // ③ 同正文重放：幂等、零膨胀
    let replay = store
        .append_sc_failure_evidence(
            &chain,
            &ScFailureEvidenceEvent::TeachingDecision {
                failed_round_id: round.round_id.clone(),
                decision: ScTeachingDecision::Ineligible,
            },
        )
        .expect("同正文重放必须幂等");
    assert_eq!(replay, first);
    assert_eq!(
        std::fs::read(&log_path).expect("read log"),
        after_first,
        "同正文重放不得膨胀"
    );
    // ④ 同槽异 decision：必须冲突，日志字节不变
    let err = store
        .append_sc_failure_evidence(
            &chain,
            &ScFailureEvidenceEvent::TeachingDecision {
                failed_round_id: round.round_id.clone(),
                decision: ScTeachingDecision::Denied,
            },
        )
        .expect_err("同槽异 decision 必须拒绝");
    assert_eq!(err.code, "sc_failure_entry_conflict");
    assert_eq!(err.step, "append_sc_failure_evidence");
    assert!(!err.publication_incomplete);
    assert_eq!(
        std::fs::read(&log_path).expect("read log"),
        after_first,
        "被拒写不得改日志"
    );
}
