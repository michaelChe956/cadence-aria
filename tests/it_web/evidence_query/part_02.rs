// C-4 Task 9（REQ-COD-05 it_web 验收）part 2/3：场景 ⑤⑥⑦⑧。
// 依赖 part_01 的 fixture 与 helper（同模块 include，符号直接可见）。
// 只新增 part_01 未引入的 imports（同模块合并，不得重复引入已存在符号）。

use cadence_aria::product::logical_codebase::evidence_budget::{
    BudgetOutcome, EVIDENCE_QUERY_RESULT_CHAR_LIMIT, EvidenceBudgetLedger,
};

/// 截断尾部标记（与 evidence_mediator 的 TRUNCATION_MARKER 逐字一致，设计 §5.2）。
const TRUNCATION_MARKER: &str = "（结果已截断，请缩小查询范围）";

// ---- ⑤ 单次 12k 截断：超长符号命中渲染 > 12k → truncated + 尾部标记 ----
#[tokio::test]
async fn single_query_over_12k_chars_is_truncated_with_marker() {
    let fx = seed_evidence_fixture_with_options(EvidenceSourceSeed::HugeSymbol, true);
    let app = fx.app();

    // 查询词 "pref" 命中 10 条 ~1398 字符符号定义，渲染后 > 12k。
    let (status, body) = evidence_query(app, &fx.token, "coder", "pref").await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["truncated"], json!(true));
    let text = body["text"].as_str().expect("text");
    assert!(
        text.ends_with(TRUNCATION_MARKER),
        "truncation marker must be appended: {text}"
    );
    assert_eq!(
        text.chars().count(),
        EVIDENCE_QUERY_RESULT_CHAR_LIMIT + TRUNCATION_MARKER.chars().count(),
        "truncated text must be exactly 12k chars + marker"
    );
    assert_eq!(
        body["budget_remaining"],
        json!(EVIDENCE_ATTEMPT_CHAR_QUOTA - text.chars().count())
    );
}

// ---- ⑥ 累计配额超限 → 429 evidence_budget_exhausted ----
#[tokio::test]
async fn cumulative_budget_exhausted_returns_429() {
    let fx = seed_evidence_fixture();
    let ledger = EvidenceBudgetLedger::new(fx.paths.clone());
    assert_eq!(
        ledger
            .consume(&fx.attempt, "warmup", 119_999)
            .expect("warmup consume"),
        BudgetOutcome::Accepted { remaining: 1 }
    );

    let app = fx.app();
    let (status, body) = evidence_query(app, &fx.token, "coder", "cross_repo_symbol").await;

    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
    assert_eq!(body["code"], "evidence_budget_exhausted");
}

// ---- ⑦ Reviewer 可查且审计区分角色 ----
#[tokio::test]
async fn reviewer_query_succeeds_and_audit_distinguishes_role() {
    let fx = seed_evidence_fixture();
    let app = fx.app();

    let (status, body) = evidence_query(app, &fx.token, "reviewer", "cross_repo_symbol").await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body["text"].as_str().expect("text").contains("cross_repo_symbol"),
        "reviewer must see cross-member text"
    );
    let audit = read_first_audit(&fx.paths);
    assert_eq!(audit["role"], "reviewer(role_self_reported)");
    assert_eq!(audit["query"], "cross_repo_symbol");
    assert_eq!(audit["hit_count"], 1);
}

// ---- ⑧ Legacy 单仓 attempt（target_snapshot=None）→ 404 evidence_not_available ----
#[tokio::test]
async fn legacy_single_repo_attempt_returns_404_evidence_not_available() {
    let fx = seed_evidence_fixture();
    let mut legacy = fx.attempt.clone();
    legacy.target_snapshot = None;
    write_json(&attempt_record_path(&fx.paths, ATTEMPT_ID), &legacy)
        .expect("write legacy attempt");

    let app = fx.app();
    let (status, body) = evidence_query(app, &fx.token, "coder", "cross_repo_symbol").await;

    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["code"], "evidence_not_available");
}
