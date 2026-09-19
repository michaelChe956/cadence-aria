// 退役留档（T5/REQ-RET-02）：`outline_author_confirm_reject_starts_revision_provider_over_websocket` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// 退役留档（T5/REQ-RET-02）：persist_outline_revision_before_provider_spawn 为上者专用夹具，一并退役。

// 退役留档（T5/REQ-RET-02）：persist_dedicated_request_outline_revision_before_provider_spawn
// 夹具（request_outline_revision 通道）随消息族退役（调用方测试同批退役）。

fn persisted_run_detail_path(
    root: &tempfile::TempDir,
    session_id: &str,
    run_node_id: &str,
) -> std::path::PathBuf {
    ProductAppPaths::new(root.path().join(".aria"))
        .issue_lifecycle_root("project_0001", "issue_0001")
        .join("workspace-timelines")
        .join(session_id)
        .join("timeline_node_details")
        .join(format!("{run_node_id}.json"))
}

fn outline_revision_journal_path(
    root: &tempfile::TempDir,
    session_id: &str,
) -> std::path::PathBuf {
    ProductAppPaths::new(root.path().join(".aria"))
        .issue_lifecycle_root("project_0001", "issue_0001")
        .join("workspace-transactions")
        .join(session_id)
        .join("work_item_plan_outline_revision.json")
}

// 退役留档（T5/REQ-RET-02）：`corrupt_outline_review_active_index_fails_closed_before_reviewer_start` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。