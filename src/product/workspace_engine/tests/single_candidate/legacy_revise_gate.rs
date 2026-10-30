// REQ-WSC-12：SC legacy 逐轮闸门用例拆分（单文件行数门禁）。
#[tokio::test]
async fn sc_legacy_revise_gate_forces_human_confirm_on_consecutive_identical_findings() {
    let (_tmp, lifecycle, _plan_id, mut engine) =
        make_work_item_plan_engine_with_accepted_contract_drafts();
    single_candidate_record(
        &lifecycle,
        &mut engine,
        SingleCandidatePhase::Evaluate,
        RunPolicy::Interactive,
    );
    engine.start_review().await;
    complete_single_candidate_review(&mut engine, plain_revise_verdict("gap: CT-001 capability"))
        .await;
    // L2 重钉（T5/REQ-RET-02）：review_decision 阶段唯一消息族已删除——SC 首轮
    // revise 改道 human gate（typed 三命令可应答），不再进入 review decision。
    assert_eq!(
        engine.session().stage,
        WorkspaceStage::HumanConfirm,
        "首轮 revise 即进 human gate（review_decision 消息族已随 L2 退役）"
    );

    // 第二轮：author 重跑后同一 candidate 再次收到实质相同 findings。
    engine.start_review().await;
    complete_single_candidate_review(&mut engine, plain_revise_verdict("gap: CT-001 capability"))
        .await;
    assert_eq!(
        engine.session().stage,
        WorkspaceStage::HumanConfirm,
        "连续 2 轮同指纹非 advisory findings 必须强制 human_confirm"
    );
    assert_eq!(
        engine.session().session_status,
        WorkspaceSessionStatus::WaitingForHuman
    );
    let human_gate_summary = engine
        .timeline_nodes
        .iter()
        .rev()
        .find(|node| node.node_type == TimelineNodeType::HumanConfirm)
        .and_then(|node| node.summary.clone())
        .unwrap_or_default();
    assert!(
        human_gate_summary.contains("repeated_findings"),
        "闸门原因必须可见于 timeline：{human_gate_summary}"
    );
    assert!(
        human_gate_summary.contains("1 项"),
        "闸门摘要应说明重复指纹数量：{human_gate_summary}"
    );
}
