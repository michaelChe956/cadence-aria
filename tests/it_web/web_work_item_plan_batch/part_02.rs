// 退役留档（T5/REQ-RET-02）：`batch_accept_skips_review_when_reviewer_disabled` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// 退役留档（T5/REQ-RET-02）：`batch_review_revise_batch_automatically_rewrites_once` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// 退役留档（T5/REQ-RET-02）：`batch_review_plan_reopen_supersedes_drafts_and_sets_outline_revising` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// 退役留档（T5/REQ-RET-02）：`batch_confirm_rewrite_batch_supersedes_current_batch_drafts` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// 退役留档（T5/REQ-RET-02）：`batch_oversized_review_feedback_fails_batch_run_node` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

fn valid_draft_output(outline_id: &str) -> Value {
    valid_draft_output_with_title(outline_id, "实现后端登录会话 API")
}

fn valid_draft_output_with_title(outline_id: &str, task_statement: &str) -> Value {
    let stable_title = match outline_id {
        "outline_frontend_expiry" => "实现前端会话过期提示",
        "outline_integration_session" => "集成测试：会话过期端到端",
        _ => "实现后端登录会话 API",
    };
    let mut output = valid_canonical_draft_output(outline_id, stable_title);
    output["draft"]["canonical_contract"]["tasks"][0]["statement"] = json!(task_statement);
    output
}

fn valid_frontend_draft_output() -> Value {
    valid_frontend_draft_output_with_title("实现前端会话过期提示")
}

fn valid_frontend_draft_output_with_title(title: &str) -> Value {
    valid_draft_output_with_title("outline_frontend_expiry", title)
}

fn valid_integration_draft_output() -> Value {
    valid_integration_draft_output_with_title("集成测试：会话过期端到端")
}

fn valid_integration_draft_output_with_title(title: &str) -> Value {
    valid_draft_output_with_title("outline_integration_session", title)
}

fn invalid_draft_output_missing_scope(outline_id: &str) -> Value {
    let mut output = valid_draft_output(outline_id);
    output["draft"]["canonical_contract"]["write_policy"]["exclusive_scopes"] = json!([]);
    output
}
