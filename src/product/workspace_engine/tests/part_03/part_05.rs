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
  "severity": "must_fix",
  "message": "sync 方法在 tokio runtime 内 block_on 会 panic",
  "evidence": "snapshot 被 tokio::spawn 调用",
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
        .build_current_work_item_draft_streaming_input(None, &RoutingReferenceContext::Legacy)
        .expect("draft streaming input");
    assert!(input.prompt.contains("[review_findings]"));
    assert!(input.prompt.contains("evidence: snapshot 被 tokio::spawn 调用"));
    assert!(input.prompt.contains("message: sync 方法在 tokio runtime 内 block_on 会 panic"));
    assert!(input
        .prompt
        .contains("required_action: 当前 draft 需明确 spawn_blocking 或改为 async 方案"));
}

// 退役留档（T5/REQ-RET-02）：`work_item_plan_item_plan_reopen_review_decision_restarts_outline_revision` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

#[derive(Debug, Clone, Copy)]
enum OutlineRevisionEntryCase {
    AuthorConfirmReject,
    RequestRevision,
    RequestOutlineRevision,
    ReviewDecision,
    HumanConfirm,
}

// 退役留档（T5/REQ-RET-02）：`work_item_plan_outline_revision_entry_points_share_preparation_state` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。
