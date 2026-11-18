// 片8（REQ-COD-05 delta）：跨仓证据「未检索」语义测试。由
// evidence_mediator_tests.inc.rs 的 mod tests 内 `mod not_searched` 引入，
// 共享其夹具（setup / FakeCodeGraphRunner / index_record / read_first_audit 等）。
//
// 红态说明：本段先于实现提交——`EvidenceQueryResponse::not_searched` 字段
// 与跳过支路尚未存在，红态为编译期缺字段错误（TDD 红测试）。
use super::*;
use crate::product::logical_codebase::failure_copy::{
    MEMBER_SOURCE_LESS_CODE, member_source_less_evidence_copy,
};

// ---- 片8（REQ-COD-05 delta）：跨仓证据「未检索」语义 ----

/// 夹具：把钉住世代里某成员记为「经盘检确认无源码」（skip_reason）。
fn skip_marked_index(fx: &Fixture, skipped: LogicalRepositoryId) -> AggregateIndexRecord {
    let mut record = index_record(
        &fx.aggregate_index_id,
        fx.api_id,
        fx.web_id,
        fx.api_checkout_id,
        fx.web_checkout_id,
        "rev-api",
        "rev-web",
        &fx.repo,
    );
    for member in record.member_snapshots.iter_mut() {
        if member.logical_repository_id == skipped {
            member.mark_source_less();
        }
    }
    record
}

/// 覆盖 manifest.active 指向的钉住世代文件。
fn overwrite_index(fx: &Fixture, record: &AggregateIndexRecord) {
    write_json(&index_record_path(&fx.paths, &fx.aggregate_index_id), record)
        .expect("overwrite aggregate index record");
}

fn runner_call_count(runner: &FakeCodeGraphRunner) -> usize {
    runner.requests.lock().expect("fake runner requests").len()
}

/// 片8-1：目标被钉住世代记为跳过 ⇒ 200 + not_searched，零 CodeGraph 调用。
#[test]
fn skipped_target_returns_200_not_searched_without_codegraph_query() {
    let fx = setup();
    overwrite_index(&fx, &skip_marked_index(&fx, fx.api_id));
    let runner = Arc::new(FakeCodeGraphRunner::with_stdout("[]"));

    let response = handle_evidence_query_with_runner(
        &fx.paths,
        &query_input(&fx.token, EvidenceRole::Coder, "anySymbol"),
        runner.clone(),
    )
    .expect("skipped target returns explicit not-searched");

    assert_eq!(
        response.not_searched.as_ref().expect("not_searched").code,
        MEMBER_SOURCE_LESS_CODE
    );
    assert!(!response
        .not_searched
        .expect("not_searched alert")
        .message_zh
        .is_empty());
    assert!(!response.truncated);
    assert_eq!(runner_call_count(&runner), 0);
}

/// 片8-2：未检索的 text 是显式说明（与「检索无命中」的空文本可区分），
/// 且 message_zh 与 text 取同一码表工厂（逐字同源）。
#[test]
fn not_searched_text_is_explicit_and_not_empty_hits() {
    let fx = setup();
    overwrite_index(&fx, &skip_marked_index(&fx, fx.api_id));

    let response = handle_evidence_query_with_runner(
        &fx.paths,
        &query_input(&fx.token, EvidenceRole::Coder, "anySymbol"),
        Arc::new(FakeCodeGraphRunner::with_stdout("[]")),
    )
    .expect("skipped target query succeeds");

    assert!(!response.text.is_empty());
    assert_eq!(response.text, member_source_less_evidence_copy("api"));
    assert_eq!(
        response.not_searched.expect("not_searched").message_zh,
        response.text
    );
    assert!(!response.truncated);
}

/// 片8-3：跳过支路审计零命中 + 未检索原因；预算按实际回包文本核算。
#[test]
fn skipped_target_audit_records_zero_hits_and_reason() {
    let fx = setup();
    overwrite_index(&fx, &skip_marked_index(&fx, fx.api_id));

    let response = handle_evidence_query_with_runner(
        &fx.paths,
        &query_input(&fx.token, EvidenceRole::Coder, "anySymbol"),
        Arc::new(FakeCodeGraphRunner::with_stdout("[]")),
    )
    .expect("skipped target query succeeds");

    let audit = read_first_audit(&fx.paths);
    assert_eq!(audit.hit_count, 0);
    assert_eq!(audit.snapshot_refs, Vec::<String>::new());
    assert_eq!(
        audit.not_searched_reason.as_deref(),
        Some(MEMBER_SOURCE_LESS_CODE)
    );
    assert_eq!(audit.result_chars, response.text.chars().count());
    assert_eq!(
        audit.budget_remaining,
        EVIDENCE_ATTEMPT_CHAR_QUOTA - audit.result_chars
    );
    assert_eq!(response.budget_remaining, audit.budget_remaining);
}

/// 片8-3b：跳过支路预算必须真消费——配额用尽 ⇒ 既有 429，不因跳过豁免。
#[test]
fn skipped_target_budget_exhaustion_returns_429_error() {
    let fx = setup();
    overwrite_index(&fx, &skip_marked_index(&fx, fx.api_id));
    let ledger = EvidenceBudgetLedger::new(fx.paths.clone());
    assert_eq!(
        ledger
            .consume(&fx.attempt, "warmup", EVIDENCE_ATTEMPT_CHAR_QUOTA - 1)
            .unwrap(),
        BudgetOutcome::Accepted { remaining: 1 }
    );

    let err = handle_evidence_query_with_runner(
        &fx.paths,
        &query_input(&fx.token, EvidenceRole::Coder, "anySymbol"),
        Arc::new(FakeCodeGraphRunner::with_stdout("[]")),
    )
    .expect_err("quota exhausted on skip branch");
    assert_eq!(err, EvidenceError::BudgetExhausted);
}

/// 片8-4：普通目标照旧检索，但结果取 ACL ∩ 钉住世代覆盖范围——被跳过
/// 成员的命中不得出现；自身 not_searched 为 None、审计无原因。
#[test]
fn normal_target_results_exclude_skipped_member_paths() {
    let fx = setup();
    overwrite_index(&fx, &skip_marked_index(&fx, fx.web_id));
    let hits_json = serde_json::json!([
        {"node": {"name": "hit", "filePath": "web/x.ts", "startLine": 1}},
    ]);
    let runner = Arc::new(FakeCodeGraphRunner::with_stdout(&hits_json.to_string()));

    let response = handle_evidence_query_with_runner(
        &fx.paths,
        &query_input(&fx.token, EvidenceRole::Coder, "hit"),
        runner.clone(),
    )
    .expect("normal target query still runs");

    assert_eq!(runner_call_count(&runner), 1);
    assert_eq!(response.text, "");
    assert!(response.not_searched.is_none());
    let audit = read_first_audit(&fx.paths);
    assert_eq!(audit.hit_count, 0);
    assert_eq!(audit.not_searched_reason, None);
}

/// 片8-5：跳过不放松凭据链——坏 token ⇒ Unauthorized；非 Running ⇒ Forbidden。
#[test]
fn skipped_target_still_enforces_acl_and_token() {
    let fx = setup();
    overwrite_index(&fx, &skip_marked_index(&fx, fx.api_id));

    let unauthorized = handle_evidence_query_with_runner(
        &fx.paths,
        &query_input("deadbeef", EvidenceRole::Coder, "anySymbol"),
        Arc::new(FakeCodeGraphRunner::with_stdout("[]")),
    )
    .expect_err("bad token still rejected");
    assert_eq!(unauthorized, EvidenceError::Unauthorized);

    let mut completed = fx.attempt.clone();
    completed.status = CodingAttemptStatus::Completed;
    write_json(&attempt_record_path(&fx.paths), &completed)
        .expect("write completed attempt");
    let forbidden = handle_evidence_query_with_runner(
        &fx.paths,
        &query_input(&fx.token, EvidenceRole::Coder, "anySymbol"),
        Arc::new(FakeCodeGraphRunner::with_stdout("[]")),
    )
    .expect_err("non-running attempt still rejected");
    assert_eq!(forbidden, EvidenceError::Forbidden);
}

/// 片8-6：老世代（无 skip_reason）⇒ 正常检索，not_searched 不出现。
#[test]
fn legacy_generation_without_skip_reason_searches_normally() {
    let fx = setup();
    let hits_json = serde_json::json!([
        {"node": {"name": "hit", "filePath": "web/x.ts", "startLine": 1}},
    ]);
    let runner = Arc::new(FakeCodeGraphRunner::with_stdout(&hits_json.to_string()));

    let response = handle_evidence_query_with_runner(
        &fx.paths,
        &query_input(&fx.token, EvidenceRole::Coder, "hit"),
        runner.clone(),
    )
    .expect("legacy generation searches normally");

    assert_eq!(runner_call_count(&runner), 1);
    assert_eq!(response.text, "web/x.ts:1 hit\n");
    assert!(response.not_searched.is_none());
}

/// 片8-7：输入校验在分叉之前——空串/超长查询词对跳过目标同样被拒。
#[test]
fn invalid_query_is_rejected_before_the_skip_branch() {
    let fx = setup();
    overwrite_index(&fx, &skip_marked_index(&fx, fx.api_id));
    let runner = Arc::new(FakeCodeGraphRunner::with_stdout("[]"));
    let too_long = "x".repeat(513);

    for query in ["", too_long.as_str()] {
        let err = handle_evidence_query_with_runner(
            &fx.paths,
            &query_input(&fx.token, EvidenceRole::Coder, query),
            runner.clone(),
        )
        .expect_err("invalid query rejected before skip branch");
        assert!(matches!(err, EvidenceError::InvalidQuery { .. }));
    }
    assert_eq!(runner_call_count(&runner), 0);
}

/// 片8-8：坏 pin（指向不存在的世代）在分叉之前被拒——不是 200。
#[test]
fn bad_pinned_record_is_rejected_before_the_skip_branch() {
    let fx = setup();
    overwrite_index(&fx, &skip_marked_index(&fx, fx.api_id));
    write_json(
        &pin_file_path(&fx.paths),
        &EvidenceIndexPinRecord {
            pinned_aggregate_index_id: "aggregate_index_missing".to_string(),
            pinned_at: "2026-08-14T00:00:00Z".to_string(),
        },
    )
    .expect("pre-write bad pin");

    let err = handle_evidence_query_with_runner(
        &fx.paths,
        &query_input(&fx.token, EvidenceRole::Coder, "anySymbol"),
        Arc::new(FakeCodeGraphRunner::with_stdout("[]")),
    )
    .expect_err("bad pin rejected before skip branch");
    assert!(matches!(
        err,
        EvidenceError::QueryFailed { code, .. } if code == "evidence_index_unavailable"
    ));
}

/// 片8-9：有效 pin + 目标被跳过 + 库目录缺失 ⇒ 仍 200 + not_searched
///（跳过支路不进库目录守卫，不落 evidence_index_unavailable）。
#[test]
fn skipped_target_returns_not_searched_even_when_library_dir_missing() {
    let fx = setup();
    overwrite_index(&fx, &skip_marked_index(&fx, fx.api_id));
    std::fs::remove_dir_all(fx.repo.join(".codegraph"))
        .expect("remove codegraph library dir");
    let runner = Arc::new(FakeCodeGraphRunner::with_stdout("[]"));

    let response = handle_evidence_query_with_runner(
        &fx.paths,
        &query_input(&fx.token, EvidenceRole::Coder, "anySymbol"),
        runner.clone(),
    )
    .expect("skip branch bypasses library dir guard");

    assert_eq!(
        response.not_searched.expect("not_searched").code,
        MEMBER_SOURCE_LESS_CODE
    );
    assert_eq!(runner_call_count(&runner), 0);
}

/// 片8-10：普通检索与未检索的审计各自保留 query 原文与事实，互不覆盖
///（attempt A 先在默认世代正常命中；attempt B 钉住记 web 为跳过的新世代）。
#[test]
fn original_query_and_audit_reason_are_both_preserved() {
    let fx = setup();
    // A（target=api）在默认世代（无跳过）先查：跨仓命中保留。
    let hits_json = serde_json::json!([
        {"node": {"name": "crossWeb", "filePath": "web/src/app.ts", "startLine": 10}},
    ]);
    handle_evidence_query_with_runner(
        &fx.paths,
        &query_input(&fx.token, EvidenceRole::Coder, "aOriginalQuery"),
        Arc::new(FakeCodeGraphRunner::with_stdout(&hits_json.to_string())),
    )
    .expect("A normal query succeeds");
    let audit_a = read_first_audit(&fx.paths);
    assert_eq!(audit_a.query, "aOriginalQuery");
    assert_eq!(audit_a.hit_count, 1);
    assert_eq!(audit_a.snapshot_refs, vec!["web/src/app.ts".to_string()]);
    assert_eq!(audit_a.not_searched_reason, None);

    // 激活新世代（web 被记为跳过）；增殖 attempt B（target=web）首查钉住它。
    let g2_id = format!("aggregate_index_{}", Uuid::new_v4().simple());
    let mut g2 = skip_marked_index(&fx, fx.web_id);
    g2.aggregate_index_id = g2_id.clone();
    AggregateIndexStore::new(fx.paths.clone())
        .create(PROJECT_ID, g2)
        .expect("create skip-marked generation");
    LogicalCodebaseStore::new(fx.paths.clone())
        .set_active_aggregate_index(PROJECT_ID, &g2_id)
        .expect("activate skip-marked generation");
    let web_snapshot = target_snapshot(
        fx.web_id,
        fx.web_checkout_id,
        &fx.repo.join("web"),
        "rev-web",
        1,
    );
    let mut attempt_b = attempt_fixture(CodingAttemptStatus::Running, Some(web_snapshot));
    attempt_b.id = format!("coding_attempt_{}", Uuid::new_v4().simple());
    attempt_b.attempt_no = 2;
    attempt_b.scope = CodingAttemptScope::WorkItemGroup;
    attempt_b.work_item_group_id = Some("work_item_plan_0001".to_string());
    attempt_b.branch_name = format!("aria/issues/{ISSUE_ID}/{}", fx.web_id.0);
    write_json(
        &fx.paths
            .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
            .join("coding-attempts")
            .join(format!("{}.json", attempt_b.id)),
        &attempt_b,
    )
    .expect("write attempt B record");
    let token_b = crate::product::logical_codebase::evidence_token::issue_evidence_token(
        &fx.paths, &fx.repo, &attempt_b,
    )
    .expect("issue evidence token for attempt B");

    let response_b = handle_evidence_query_with_runner(
        &fx.paths,
        &query_input(&token_b, EvidenceRole::Reviewer, "bOriginalQuery"),
        Arc::new(FakeCodeGraphRunner::with_stdout("[]")),
    )
    .expect("B skip query succeeds");
    assert_eq!(
        response_b.not_searched.expect("B not_searched").code,
        MEMBER_SOURCE_LESS_CODE
    );

    // B 分区审计保留自己的 query 原文与未检索事实；A 的审计不被覆盖。
    let audit_b: EvidenceAuditRecord = serde_json::from_str(
        std::fs::read_to_string(attempt_partition_file(
            &fx.paths,
            &attempt_b.id,
            "evidence-audit.jsonl",
        ))
        .expect("B audit file")
        .lines()
        .next()
        .expect("B first audit line"),
    )
    .expect("parse B audit record");
    assert_eq!(audit_b.query, "bOriginalQuery");
    assert_eq!(audit_b.hit_count, 0);
    assert_eq!(
        audit_b.not_searched_reason.as_deref(),
        Some(MEMBER_SOURCE_LESS_CODE)
    );
    assert_eq!(read_first_audit(&fx.paths).query, "aOriginalQuery");
}
