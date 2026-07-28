//! Code review triage gate RED tests (Task 1, TDD RED phase).
//!
//! 这些测试覆盖 OpenSpec 变更 `open-code-review-triage-gate` 的 spec requirements：
//! - requirement 1: StopForHumanTriage 落 blocked gate（reason_code=
//!   `code_review_output_human_triage`）。
//! - requirement 2: RetryVerification 落 blocked gate（reason_code=
//!   `code_review_verification_incomplete`）。【夹具待补，见下】
//! - requirement 3: OpenOperationalGate 落 blocked gate（reason_code=
//!   `code_review_operational_blocker`）。【夹具待补，见下】
//! - requirement 4: 三类 gate 的动作集合均为
//!   `[retry_review, send_to_coder, manual_continue, abort]`。
//! - requirement 6: 互斥——同一 stage 不得 double-gate。
//!
//! 当前生产实现（`code_review.rs`）仅在 `verdict == Blocked &&
//! !has_actionable_findings` 时落 `code_review_blocked` gate；对
//! `code_review_flow_decision` 返回的 StopForHumanTriage / RetryVerification /
//! OpenOperationalGate 三个分诊决策静默退出，既不改 attempt 状态为 Blocked，
//! 也不落任何 gate，attempt 停留 Running。Task 1 在此写入失败测试，Task 2 才
//! 做生产实现。

use super::*;

/// 构造一个「implementation_defect 携带非空 plan_defect_evidence」的 code review
/// provider 输出。
///
/// 依据 `validate_plan_defect_finding`（`plan_defect.rs`），`ImplementationDefect`
/// 一旦携带任何 plan_defect 字段（这里塞了非空 `plan_defect_evidence`，同时给出
/// `recommended_route=coder_rework`）即校验失败，整份 report 因此被
/// `code_review_flow_decision` 判定为 `StopForHumanTriage`。此路径不依赖
/// reviewer_projection 的 blocker_routing，可由 `running_attempt_with_worktree()`
/// （WorkItem scope，projection 为空）直接构造。
fn implementation_finding_with_plan_defect_fields() -> serde_json::Value {
    serde_json::json!({
        "verdict": "request_changes",
        "summary": "需要返修",
        "findings": [{
            "severity": "error",
            "file_path": "src/lib.rs",
            "line": 1,
            "message": "缺少测试执行证据",
            "required_action": "补交红灯执行记录",
            "source_stage": "code_review",
            "defect_class": "implementation_defect",
            "recommended_route": "coder_rework",
            "plan_defect_evidence": [{
                "kind": "manual_check",
                "source_ref": "test_evidence_refs",
                "message": "证据为空"
            }]
        }]
    })
}

/// 用给定 provider 输出跑一次 `execute_code_review_with_commands`，返回 store 与
/// 持久化后的 attempt。
///
/// 复用父级 `tests` 模块的 `running_attempt_with_worktree()` + `init_test_git_repo()`
/// 公共夹具；Provider 用 `CapturingProjectionProvider`（`provider_execution_context.rs`）
/// 直接吐出给定 JSON。
///
/// 注意：返回的 `tempfile::TempDir` 必须由调用方持有到测试结束——它绑定的是
/// `running_attempt_with_worktree()` 的临时根目录，一旦 drop 会清空 store 落盘的
/// 所有记录，导致后续断言读到空数据。
async fn run_code_review_with_provider_output(
    output: serde_json::Value,
) -> (
    tempfile::TempDir,
    crate::product::coding_attempt_store::CodingAttemptStore,
    CodingExecutionAttempt,
) {
    let (root, store, attempt) = running_attempt_with_worktree();
    init_test_git_repo(attempt.worktree_path.as_ref().unwrap());
    let (tx, _rx) = tokio::sync::mpsc::channel(64);
    let engine = CodingWorkspaceEngine::new(
        store.clone(),
        crate::product::git_workspace_service::GitWorkspaceService::new(),
        tx,
    );
    let provider =
        super::provider_execution_context::CapturingProjectionProvider::new(output.to_string());
    let (_cmd_tx, mut cmd_rx) = tokio::sync::mpsc::channel(1);
    engine
        .execute_code_review_with_commands(&attempt, &provider, &mut cmd_rx)
        .await
        .unwrap();
    let persisted = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .unwrap();
    (root, store, persisted)
}

#[tokio::test]
async fn stop_for_human_triage_lands_blocked_gate_with_review_actions() {
    let (_root, store, attempt) =
        run_code_review_with_provider_output(implementation_finding_with_plan_defect_fields())
            .await;

    // 当前实现：三个分诊决策静默退出，attempt 仍为 Running（应转为 Blocked）。
    assert_eq!(
        attempt.status,
        CodingAttemptStatus::Blocked,
        "StopForHumanTriage 必须把 attempt 转为 Blocked"
    );

    let gates = store
        .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .unwrap();
    assert_eq!(gates.len(), 1, "StopForHumanTriage 必须且只能落一个 gate");
    assert_eq!(
        gates[0].reason_code.as_deref(),
        Some("code_review_output_human_triage"),
    );

    let mut action_ids: Vec<&str> = gates[0]
        .available_actions
        .iter()
        .map(|action| action.action_id.as_str())
        .collect();
    action_ids.sort();
    assert_eq!(
        action_ids,
        vec!["abort", "manual_continue", "retry_review", "send_to_coder"],
        "code review triage gate 动作集合必须为四项",
    );
}

/// `RetryVerification`（verification_incomplete）门禁测试。
///
/// 该 finding 必须通过 `validate_plan_defect_finding` 的完整契约校验，包括与
/// `reviewer_projection.blocker_routing` 对齐（reason_code + confidence=high +
/// evidence + recommended_route/repair_target 匹配 blocker rule）。
/// `running_attempt_with_worktree()` 构造的 WorkItem scope attempt 没有 plan
/// lineage / projection bundle，无法直接构造能通过校验的 finding。可行的夹具路径
/// 是复用 `provider_execution_context.rs:90-165` 的完整 coding→projection 链路
/// （先 `execute_coding` 产出 unit run 与 projection bundle，再走 code review）。
///
/// 该夹具成本较高，本 Task 1（RED 阶段）先以 `#[ignore]` + `todo!` 占位，明确记录
/// 为「待补 RED 测试」，**不得用 PASS 状态混淆**。Task 2 完成门禁落地后再回填真实
/// 夹具与断言（reason_code=`code_review_verification_incomplete`，动作集合四项）。
#[tokio::test]
#[ignore = "TODO(task-2): verification_incomplete finding 需要真实 reviewer_projection \
            blocker_routing 夹具（coding→projection 链路），待 Task 2 门禁落地后回填"]
async fn retry_verification_lands_blocked_gate_with_review_actions() {
    // 预期 RED（Task 2 实现后）：
    //   attempt.status == Blocked
    //   gates.len() == 1
    //   gates[0].reason_code == Some("code_review_verification_incomplete")
    //   action_ids == [retry_review, send_to_coder, manual_continue, abort]
    todo!("verification_incomplete reviewer_projection blocker_routing 夹具待补");
}

/// `OpenOperationalGate`（operational_blocker）门禁测试。
///
/// 同 `retry_verification_lands_blocked_gate_with_review_actions` 的夹具约束：
/// operational_blocker finding 必须通过完整契约校验并与 reviewer_projection
/// blocker_routing 对齐，WorkItem scope 的 `running_attempt_with_worktree()` 无法
/// 直接构造。本 Task 1（RED 阶段）先以 `#[ignore]` + `todo!` 占位，明确记录为
/// 「待补 RED 测试」。Task 2 完成门禁落地后回填（reason_code=
/// `code_review_operational_blocker`，动作集合四项）。
#[tokio::test]
#[ignore = "TODO(task-2): operational_blocker finding 需要真实 reviewer_projection \
            blocker_routing 夹具（coding→projection 链路），待 Task 2 门禁落地后回填"]
async fn open_operational_gate_lands_blocked_gate_with_review_actions() {
    // 预期 RED（Task 2 实现后）：
    //   attempt.status == Blocked
    //   gates.len() == 1
    //   gates[0].reason_code == Some("code_review_operational_blocker")
    //   action_ids == [retry_review, send_to_coder, manual_continue, abort]
    todo!("operational_blocker reviewer_projection blocker_routing 夹具待补");
}

/// 互斥回归测试：`verdict=blocked` 且无可执行 finding 时，必须只落既有的
/// `code_review_blocked` gate，不得因为新的分诊 gate 逻辑而 double-gate。
///
/// 当前实现已覆盖此行为（`code_review.rs` 中 `Blocked && !actionable` 落
/// `code_review_blocked`），本测试作为回归保护，确保 Task 2 落地分诊 gate 时
/// 不会与既有 `code_review_blocked` 重复落 gate。
#[tokio::test]
async fn blocked_verdict_without_actionable_findings_lands_only_code_review_blocked_gate() {
    let (_root, store, attempt) = run_code_review_with_provider_output(serde_json::json!({
        "verdict": "blocked",
        "summary": "被阻塞",
        "findings": []
    }))
    .await;

    let gates = store
        .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .unwrap();
    assert_eq!(gates.len(), 1, "不得 double-gate");
    assert_eq!(gates[0].reason_code.as_deref(), Some("code_review_blocked"),);
    assert!(
        gates
            .iter()
            .all(|gate| gate.reason_code.as_deref() != Some("code_review_output_human_triage")),
        "不得与 code_review_output_human_triage 分诊 gate 重复",
    );
}
