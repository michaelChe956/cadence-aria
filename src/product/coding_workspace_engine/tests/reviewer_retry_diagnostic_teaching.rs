// B-④（3.6 矩阵族④根因修复）：reviewer 输出缺 verdict/非法 JSON 被阻塞后，
// 人工点 retry_review 重跑时，retry 诊断通道（retry_diagnostic_for_previous_run
// → 「上一轮 role run 诊断摘要」注入新 review prompt）必须携带 serde 错误
// 原文与结构化输出教学句——族④现场（issue_0151）reviewer 输出中文长叙述无
// JSON，重跑 prompt 只有泛化诊断摘要，弱模型无错误原文可学。
use super::*;

fn blocked_code_review_report(
    attempt: &CodingExecutionAttempt,
    role_run_id: &str,
    summary: String,
) -> CodeReviewReport {
    CodeReviewReport {
        id: "code_review_0001".to_string(),
        attempt_id: attempt.id.clone(),
        round: 1,
        verdict: ReviewVerdict::Blocked,
        findings: Vec::new(),
        tested_evidence_refs: Vec::new(),
        diff_refs: Vec::new(),
        summary,
        created_at: "2026-09-04T00:00:00Z".to_string(),
        raw_provider_output_ref: None,
        role_run_id: Some(role_run_id.to_string()),
        run_no: Some(1),
        unit_run_id: None,
    }
}

/// 造出与真实链路一致的前置状态：上一轮 reviewer run Blocked + 挂着含
/// blocked_review_payload 解析错误原文的 review 报告，再经门 retry_review
/// 语义 supersede 出新 run；返回新 run 供 retry_diagnostic_for_previous_run。
fn previous_blocked_run_with_report(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    summary: String,
) -> CodingRoleRun {
    let previous = store
        .create_role_run(
            attempt,
            CodingExecutionStage::CodeReview,
            CodingProviderRole::CodeReviewer,
            CodingRoleRunTrigger::Initial,
            None,
        )
        .expect("previous reviewer run");
    store
        .update_role_run_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &previous.id,
            CodingRoleRunStatus::Blocked,
            Some("code_review_blocked".to_string()),
        )
        .expect("block previous run");
    store
        .save_code_review_report(
            attempt,
            &blocked_code_review_report(attempt, &previous.id, summary),
        )
        .expect("save blocked review report");
    store
        .supersede_latest_role_run_and_create(
            attempt,
            CodingExecutionStage::CodeReview,
            CodingProviderRole::CodeReviewer,
            CodingRoleRunTrigger::RetryReview,
            None,
            None,
        )
        .expect("retry review run")
}

#[test]
fn reviewer_retry_diagnostic_injects_structured_output_teaching_for_blocked_parse_failure() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    // 族④同款：JSON 对象存在但缺 verdict 字段（serde missing field `verdict`）。
    let summary = "review JSON Schema 校验失败，已阻塞并等待人工确认: missing field `verdict` at line 1 column 2; 原始输出: {\"summary\":\"中文长叙述\"}";
    let retry_run = previous_blocked_run_with_report(&store, &attempt, summary.to_string());

    let diagnostic = engine
        .retry_diagnostic_for_previous_run(&attempt, &retry_run)
        .expect("retry diagnostic")
        .expect("diagnostic summary present");

    // 既有诊断摘要保留，且追加教学段：serde 错误原文 + 结构化输出教学句。
    assert!(diagnostic.contains("[previous_role_run_diagnostic]"));
    assert!(diagnostic.contains("[previous_run_structured_output_teaching]"));
    assert!(
        diagnostic.contains("missing field `verdict`"),
        "诊断必须回灌 serde 错误原文: {diagnostic}"
    );
    assert!(
        diagnostic
            .contains("最终结论必须是只含一个 JSON 对象、以 { 开头 } 结尾、verdict∈approve|request_changes|blocked"),
        "诊断必须携带结构化输出教学句: {diagnostic}"
    );
    // 原始输出全文不进诊断（有界），避免 retry prompt 爆炸。
    assert!(!diagnostic.contains("原始输出"));
}

#[test]
fn reviewer_retry_diagnostic_teaching_only_for_parse_failure_blockeds() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    // 非 parse 失败的普通 blocked（如真实 findings 触发的 blocked）不注入教学。
    let ordinary_blocked = "阻塞 finding 需要人工确认".to_string();
    let retry_run = previous_blocked_run_with_report(&store, &attempt, ordinary_blocked);
    let diagnostic = engine
        .retry_diagnostic_for_previous_run(&attempt, &retry_run)
        .expect("retry diagnostic")
        .expect("diagnostic summary present");
    assert!(diagnostic.contains("[previous_role_run_diagnostic]"));
    assert!(!diagnostic.contains("[previous_run_structured_output_teaching]"));
}

#[test]
fn internal_reviewer_retry_diagnostic_injects_teaching_for_missing_verdict() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    let previous = store
        .create_role_run(
            &attempt,
            CodingExecutionStage::InternalPrReview,
            CodingProviderRole::InternalReviewer,
            CodingRoleRunTrigger::Initial,
            None,
        )
        .expect("previous internal reviewer run");
    store
        .update_role_run_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &previous.id,
            CodingRoleRunStatus::Blocked,
            Some("internal_review_blocked".to_string()),
        )
        .expect("block previous run");
    let review = InternalPrReview {
        id: "internal_review_0001".to_string(),
        attempt_id: attempt.id.clone(),
        review_request_id: "review_request_0001".to_string(),
        verdict: ReviewVerdict::Blocked,
        findings: Vec::new(),
        impact_scope: Vec::new(),
        pr_description: String::new(),
        commit_message_suggestion: String::new(),
        tested_evidence_refs: Vec::new(),
        diff_refs: Vec::new(),
        summary: "review 输出不是有效 JSON，已阻塞并等待人工确认: expected value at line 1 column 1; 原始输出: 本次审查结论如下……（中文长叙述无 JSON）"
            .to_string(),
        created_at: "2026-09-04T00:00:00Z".to_string(),
        raw_provider_output_ref: None,
        role_run_id: Some(previous.id.clone()),
        run_no: Some(1),
    };
    store
        .save_internal_pr_review(&attempt, &review)
        .expect("save internal review");
    let retry_run = store
        .supersede_latest_role_run_and_create(
            &attempt,
            CodingExecutionStage::InternalPrReview,
            CodingProviderRole::InternalReviewer,
            CodingRoleRunTrigger::RetryInternalReview,
            None,
            None,
        )
        .expect("retry internal review run");

    let diagnostic = engine
        .retry_diagnostic_for_previous_run(&attempt, &retry_run)
        .expect("retry diagnostic")
        .expect("diagnostic summary present");
    assert!(diagnostic.contains("[previous_run_structured_output_teaching]"));
    assert!(diagnostic.contains("expected value at line 1 column 1"));
}
