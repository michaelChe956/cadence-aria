// 退役留档（T5/REQ-RET-02）：`recovery_abort_and_rollback_is_rejected_after_plan_commit_marker` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// 退役留档（T5/REQ-RET-02）：`recovery_human_triage_keeps_transaction_for_manual_resolution` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// 退役留档（T5/REQ-RET-02）：`compile_recovery_resumes_after_committed_marker` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// 退役留档（T5/REQ-RET-02）：`recovery_abort_is_rejected_when_active_plan_revision_exists_with_stale_marker` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

fn valid_draft_output(outline_id: &str) -> Value {
    valid_canonical_draft_output(outline_id, "实现后端登录会话 API")
}

fn unsafe_backend_draft_output() -> Value {
    let mut output = valid_draft_output("outline_backend_session");
    output["draft"]["canonical_contract"]["verification_checks"][0]["command"] =
        json!("rm -rf /");
    output["draft"]["verification_plan"]["checks"][0]["command"] = json!("rm -rf /");
    output
}

fn unsafe_frontend_draft_output() -> Value {
    let mut output = valid_frontend_draft_output();
    output["draft"]["canonical_contract"]["verification_checks"][0]["command"] =
        json!("rm -rf /");
    output["draft"]["verification_plan"]["checks"][0]["command"] = json!("rm -rf /");
    output
}

fn valid_frontend_draft_output() -> Value {
    valid_canonical_draft_output("outline_frontend_expiry", "实现前端会话过期提示")
}

fn valid_integration_draft_output() -> Value {
    let mut output = valid_canonical_draft_output(
        "outline_integration_session",
        "集成测试：会话过期端到端",
    );
    output["draft"]["canonical_contract"]["handoff_contract"]["provided_contract_refs"] =
        json!([]);
    output
}
