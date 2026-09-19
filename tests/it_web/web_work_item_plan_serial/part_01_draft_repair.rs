#[tokio::test]
async fn serial_local_validation_failure_repairs_once_with_findings() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls().await;
    let (app, root, prompts) = app_with_confirmed_story_and_design_and_streaming_outputs(vec![
        valid_outline_output(),
        invalid_draft_output_missing_scope("outline_backend_session"),
        valid_draft_output("outline_backend_session"),
    ])
    .await;
    let (_session_id, plan_id, mut ws) = prepare_plan_accept_outline_and_select_serial(&app).await;

    let _messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_draft_confirm"
        })
    })
    .await;

    {
        let captured_prompts = prompts.lock().unwrap();
        assert_eq!(captured_prompts.len(), 3, "outline author + failed draft + repair");
        assert!(captured_prompts[2].contains("[draft_validation_findings]"));
        assert!(captured_prompts[2].contains(
            "write_scope_required: draft outline_backend_session must include at least one canonical exclusive write scope"
        ));
    }

    let drafts = WorkItemPlanStore::new(ProductAppPaths::new(root.path().join(".aria")))
        .list_draft_records("project_0001", "issue_0001", &plan_id)
        .expect("list drafts");
    assert_eq!(drafts.len(), 1, "initial invalid candidate must not persist");
    assert_eq!(
        drafts[0].status,
        WorkItemDraftStatus::Draft,
        "repaired draft: {:#?}",
        drafts[0]
    );
    let diagnostics = serde_json::to_value(&drafts[0]).expect("serialize draft")
        ["generation_diagnostics"]
        .clone();
    assert_eq!(diagnostics["auto_repair_attempted"], true);
    assert!(!diagnostics["initial_validation_findings"]
        .as_array()
        .expect("initial findings")
        .is_empty());

    ws.close(None).await.ok();
}

// 退役留档（T5/REQ-RET-02）：`serial_second_local_validation_failure_stops_at_confirm` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// 退役留档（T5/REQ-RET-02）：`manual_rewrite_merges_validation_findings_and_user_feedback` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// 退役留档（T5/REQ-RET-02）：`accepted_draft_enters_item_review_when_reviewer_enabled` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// 退役留档（T5/REQ-RET-02）：`author_decision_is_rejected_on_draft_confirm_node` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// 退役留档（T5/REQ-RET-02）：`serial_oversized_feedback_fails_draft_run_node` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。
