/// 集合不含 `send_to_coder` 时，脚本/陈旧快照直发该动作仍必须被服务端拒绝：
/// 可观察码 `send_to_coder_latest_review_not_actionable` 保留，不落 rework
/// instruction、不改 attempt 状态、不关门。
#[tokio::test]
async fn verdict_derived_triage_gate_rejects_direct_send_to_coder() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    let attempt = attempt_at_code_review(&store, &attempt);
    seed_review_conclusion(
        &store,
        &attempt,
        "code_review_report_0001",
        ReviewVerdict::Approve,
    );
    let engine = triage_test_engine(&store);
    let gated = create_triage_gate(
        &engine,
        &attempt,
        "code_review_output_human_triage",
        "coding_node_0001",
    )
    .await;
    let gate = open_triage_gate(&store, &gated);
    assert!(!gate_action_ids(&gate).contains(&"send_to_coder".to_string()));

    let error = engine
        .handle_blocked_gate_response(
            &gated.project_id,
            &gated.issue_id,
            &gated.id,
            &gate.gate_id,
            "send_to_coder",
            Some("请按审查结论返修".to_string()),
        )
        .await
        .expect_err("集合不含该动作时必须拒绝");
    assert_eq!(
        provider_stream_code(&error),
        "send_to_coder_latest_review_not_actionable",
    );

    assert!(
        store
            .list_rework_instructions(&gated.project_id, &gated.issue_id, &gated.id)
            .expect("rework instructions")
            .is_empty(),
        "被拒绝的提交不得落地 rework instruction",
    );
    let persisted = store
        .get_attempt(&gated.project_id, &gated.issue_id, &gated.id)
        .expect("attempt");
    assert_eq!(persisted.status, CodingAttemptStatus::Blocked);
    assert_eq!(persisted.stage, CodingExecutionStage::CodeReview);
    assert_eq!(
        open_triage_gate(&store, &gated).gate_id,
        gate.gate_id,
        "拒绝不得关闭该门",
    );
}

/// 存量在途门（升级前落了四动作、不回写）：最新 verdict 不可返修时，直发
/// `send_to_coder` 仍被既有前置拒绝，语义与派生门一致。
#[tokio::test]
async fn legacy_four_action_triage_gate_still_rejects_send_to_coder() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    let attempt = attempt_at_code_review(&store, &attempt);
    seed_review_conclusion(
        &store,
        &attempt,
        "code_review_report_0001",
        ReviewVerdict::Approve,
    );
    let blocked = store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Blocked,
        )
        .expect("blocked");
    let gate = store
        .create_blocked_gate(
            &blocked,
            CreateBlockedGateInput {
                attempt_id: blocked.id.clone(),
                stage: CodingExecutionStage::CodeReview,
                node_id: Some("coding_node_0001".to_string()),
                role: Some(CodingProviderRole::CodeReviewer),
                title: "Code Review 结论需人工分诊".to_string(),
                description: "升级前落地的四动作门".to_string(),
                reason_code: Some("code_review_output_human_triage".to_string()),
                evidence_refs: Vec::new(),
                raw_provider_output_ref: None,
                available_actions: vec![
                    coding_gate_action_for_id("retry_review").expect("retry review action"),
                    coding_gate_action_for_id("send_to_coder").expect("send to coder action"),
                    coding_gate_action_for_id("manual_continue").expect("manual continue action"),
                    coding_gate_action_for_id("abort").expect("abort action"),
                ],
            },
        )
        .expect("legacy four action gate");
    // 不回写：存量门没有派生事实字段，动作集合保持落地时的样子。
    assert_eq!(gate.latest_review_verdict, None);

    let engine = triage_test_engine(&store);
    let error = engine
        .handle_blocked_gate_response(
            &blocked.project_id,
            &blocked.issue_id,
            &blocked.id,
            &gate.gate_id,
            "send_to_coder",
            Some("请按审查结论返修".to_string()),
        )
        .await
        .expect_err("verdict 不可返修时存量门也必须拒绝");
    assert_eq!(
        provider_stream_code(&error),
        "send_to_coder_latest_review_not_actionable",
    );
    assert!(
        store
            .list_rework_instructions(&blocked.project_id, &blocked.issue_id, &blocked.id)
            .expect("rework instructions")
            .is_empty(),
    );
    let persisted = store
        .get_attempt(&blocked.project_id, &blocked.issue_id, &blocked.id)
        .expect("attempt");
    assert_eq!(persisted.status, CodingAttemptStatus::Blocked);
    assert_eq!(
        open_triage_gate(&store, &blocked),
        gate,
        "存量门保持原样（不回写、不关闭）",
    );
}
