#[tokio::test]
async fn work_item_plan_item_required_revision_feedback_includes_findings() {
    let (_tmp, _checkpoint_store, _lifecycle, plan_id, mut engine) =
        make_work_item_plan_engine_with_draft_candidate("sess_wip_item_required_review_feedback");
    prepare_work_item_plan_outline_artifact(&mut engine).await;
    save_serial_work_item_plan_index(&engine, &plan_id, "outline_a");
    engine
        .update_artifact(work_item_draft_artifact_payload(
            &plan_id,
            "outline_a",
            "draft_a",
            WorkItemDraftStatus::Accepted,
        ))
        .await;
    let review_node_id = engine
        .create_timeline_node(TimelineNodeDraft {
            node_type: TimelineNodeType::WorkItemDraftReview,
            agent: Some(ProviderName::Codex),
            stage: WorkspaceStage::CrossReview,
            round: Some(1),
            title: "Work Item Draft Review".to_string(),
            summary: None,
            status: TimelineNodeStatus::Active,
        })
        .await;
    engine.active_node_id = Some(review_node_id);

    engine
        .drive_review_session(
            Arc::new(ReviewVerdictStreamingProvider {
                output: r#"当前 draft 需要返修。

```json
{
  "verdict": "pass",
  "review_scope": "item",
  "target_outline_id": "outline_a",
  "generation_round_id": "round_0001",
  "draft_id": "draft_a",
  "summary": "运行时方案存在阻塞问题",
  "findings": [
{
  "severity": "strong_recommend_fix",
  "message": "sync 方法在 tokio runtime 内 block_on 会 panic",
  "evidence": "snapshot 被 tokio::spawn 调用",
  "impact": "后续实现会在运行时崩溃",
  "required_action": "当前 draft 需明确 spawn_blocking 或改为 async 方案"
}
  ]
}
```"#,
                provider_type: Arc::new(Mutex::new(None)),
                prompt: Arc::new(Mutex::new(None)),
            }),
            empty_provider_commands(),
        )
        .await;

    assert_eq!(engine.session().stage, WorkspaceStage::Running);
    let active_node = engine
        .timeline_nodes
        .iter()
        .find(|node| Some(&node.node_id) == engine.active_node_id.as_ref())
        .expect("active draft run node");
    assert_eq!(active_node.node_type, TimelineNodeType::WorkItemDraftRun);

    let input = engine
        .build_current_work_item_draft_streaming_input(None)
        .expect("draft streaming input");
    assert!(input.prompt.contains("[review_findings]"));
    assert!(input.prompt.contains("evidence: snapshot 被 tokio::spawn 调用"));
    assert!(input.prompt.contains("impact: 后续实现会在运行时崩溃"));
    assert!(input
        .prompt
        .contains("required_action: 当前 draft 需明确 spawn_blocking 或改为 async 方案"));
}

#[tokio::test]
async fn work_item_plan_item_plan_reopen_review_decision_restarts_outline_revision() {
    let (_tmp, _checkpoint_store, _lifecycle, plan_id, mut engine) =
        make_work_item_plan_engine_with_draft_candidate("sess_wip_item_plan_reopen");
    prepare_work_item_plan_outline_artifact(&mut engine).await;
    save_serial_work_item_plan_index(&engine, &plan_id, "outline_a");
    engine
        .update_artifact(work_item_draft_artifact_payload(
            &plan_id,
            "outline_a",
            "draft_a",
            WorkItemDraftStatus::Accepted,
        ))
        .await;
    engine.session.stage = WorkspaceStage::ReviewDecision;
    engine.latest_review_verdict = Some(ReviewVerdict {
        verdict: ReviewVerdictType::NeedsHuman,
        comments: "当前 item 暴露出 Outline 边界错误".to_string(),
        summary: "需要重开 Outline".to_string(),
        findings: vec![ReviewFinding {
            severity: ReviewFindingSeverity::MustFix,
            message: "item 写入范围漏掉共享 provider match".to_string(),
            evidence: "src/product/work_item_split_engine/types.rs:86".to_string(),
            impact: "只修当前 draft 会继续遗漏边界。".to_string(),
            required_action: "回到 Outline，把 provider metadata 状态边界扩到所有 ProviderName match。"
                .to_string(),
        }],
        review_gate: ReviewGate::UserTriageRequired,
        work_item_plan_review: Some(WorkItemPlanReviewComplete {
            verdict: WorkItemPlanReviewVerdict::PlanReopenRequired,
            review_scope: WorkItemPlanReviewScope::Item,
            target_outline_id: Some("outline_backend_metadata_state".to_string()),
            generation_round_id: "round_0001".to_string(),
            draft_id: Some("draft_001".to_string()),
            batch_id: None,
            review_action: WorkItemPlanReviewAction::ReviseOutline,
            gates: vec![WorkItemPlanReviewGate::RequiresPlanReopen],
            affects_items: vec![],
            warnings: vec![],
        }),
        structured_output_diagnostic: None,
    });
    engine
        .enter_review_decision(1, "需要重开 Outline".to_string())
        .await;

    let outcome = engine
        .handle_review_decision("continue".to_string(), None)
        .await
        .expect("item plan reopen should restart outline revision");

    let ReviewDecisionOutcome::StartWorkItemPlanOutlineRevision { feedback } = outcome else {
        panic!("expected outline revision outcome");
    };
    let feedback = feedback.expect("outline feedback");
    assert!(feedback.contains("[review_findings]"));
    assert!(feedback.contains("required_action: 回到 Outline"));
    assert_eq!(engine.session().stage, WorkspaceStage::Running);
}
