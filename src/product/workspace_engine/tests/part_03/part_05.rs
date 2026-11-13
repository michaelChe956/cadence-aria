#[tokio::test]
async fn work_item_plan_item_required_revision_feedback_includes_findings() {
    let (_tmp, _checkpoint_store, lifecycle, plan_id, mut engine) =
        make_work_item_plan_engine_with_draft_candidate("sess_wip_item_required_review_feedback");
    prepare_work_item_plan_outline_artifact(&mut engine).await;
    save_serial_work_item_plan_index(&engine, &plan_id, "outline_a");
    engine
        .update_artifact(work_item_draft_artifact_payload(
            &lifecycle,
            &plan_id,
            "outline_a",
            "draft_a",
            WorkItemDraftStatus::Accepted,
        ))
        .await
        .expect("fixture artifact write");
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
        .drive_review_session_with_window(
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
    assert!(
        input
            .prompt
            .contains("evidence: snapshot 被 tokio::spawn 调用")
    );
    assert!(
        input
            .prompt
            .contains("message: sync 方法在 tokio runtime 内 block_on 会 panic")
    );
    assert!(
        input
            .prompt
            .contains("required_action: 当前 draft 需明确 spawn_blocking 或改为 async 方案")
    );
}

// 退役留档（T5/REQ-RET-02）：`work_item_plan_item_plan_reopen_review_decision_restarts_outline_revision` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// 退役留档（T5/REQ-RET-02）：`work_item_plan_outline_revision_entry_points_share_preparation_state` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// ============================================================================
// B2b3（Task 5.4，tasks.md 5.4 / 设计 §5.3）：draft/batch builder 显式
// fresh 模式（resume=None、durable 冻结 feedback、不动内存 pending）与
// provider 运行节点重建（业务代/批次不重建、修复预算不重置）。
// ============================================================================

fn anchor_t16_author_conversation_ref() -> crate::product::models::ProviderConversationRef {
    crate::product::models::ProviderConversationRef {
        role: crate::product::models::ProviderConversationRole::Author,
        provider: ProviderName::ClaudeCode,
        provider_session_id: "native-t16-old".to_string(),
        updated_at: "2026-10-12T00:00:00Z".to_string(),
        last_node_id: Some("timeline_node_001".to_string()),
    }
}

#[tokio::test]
async fn anchor_t16_draft_fresh_builder_and_node_rebuild_keep_generation() {
    let (_tmp, _checkpoint_store, lifecycle, plan_id, mut engine) =
        make_work_item_plan_engine_with_draft_candidate("sess_anchor_t16_draft");
    prepare_work_item_plan_outline_artifact(&mut engine).await;
    save_serial_work_item_plan_index(&engine, &plan_id, "outline_a");
    let store = engine.work_item_plan_store().expect("work item plan store");
    store
        .put_draft_record(&test_work_item_draft_record(
            &lifecycle,
            &plan_id,
            "outline_a",
            "draft_outline_a",
            WorkItemDraftStatus::Accepted,
            WorkItemGenerationMode::Serial,
            None,
        ))
        .expect("put accepted draft record");
    // 旧 native 会话引用在场（fresh 必须不回捞）+ 内存 pending 残留
    //（fresh 消费 durable 冻结值，不读不借内存反馈）。
    engine.session.provider_conversations = vec![anchor_t16_author_conversation_ref()];
    engine.pending_revision_context = Some("内存残留反馈（不得进入 fresh 输入）".to_string());
    // 一次 RetryOnce 修复已发生过的形态：修复预算已占用，fresh 不得重置。
    engine.remember_one_draft_repair("outline_a", Vec::new(), Some("原始用户反馈".to_string()));

    let index_before = store
        .load_active_index(
            &engine.session.project_id,
            &engine.session.issue_id,
            &plan_id,
        )
        .expect("load index before")
        .expect("index before");

    // stale 收口后无活动 draft 节点：fresh 重建 provider 运行节点。
    assert_ne!(
        engine.active_node_type(),
        Some(TimelineNodeType::WorkItemDraftRun)
    );
    engine
        .start_serial_work_item_draft_run_for("outline_a")
        .await
        .expect("fresh draft node rebuild");
    assert_eq!(
        engine.active_node_type(),
        Some(TimelineNodeType::WorkItemDraftRun)
    );

    // 显式 fresh builder：resume=None、durable 冻结 feedback、不借内存 pending。
    let frozen_feedback = "draft 冻结返修反馈（t16）";
    let input = engine
        .build_current_work_item_draft_streaming_input_explicit_fresh(
            Some(frozen_feedback),
            &RoutingReferenceContext::Legacy,
        )
        .expect("fresh draft streaming input");
    assert_eq!(
        input.resume_provider_session_id, None,
        "fresh 不回捞旧 native 会话（conversation 引用在场）"
    );
    assert!(input.prompt.contains(frozen_feedback));
    assert!(
        !input.prompt.contains("内存残留反馈（不得进入 fresh 输入）"),
        "fresh 的 feedback 源是 durable 冻结值，内存 pending 不得混入"
    );
    // pending_revision_context 未被 take（durable 冻结值直供）。
    assert_eq!(
        engine.pending_revision_context.as_deref(),
        Some("内存残留反馈（不得进入 fresh 输入）")
    );

    // payload feedback=None：fresh 输出同样不回落内存 pending。
    let input_none = engine
        .build_current_work_item_draft_streaming_input_explicit_fresh(
            None,
            &RoutingReferenceContext::Legacy,
        )
        .expect("fresh draft streaming input without feedback");
    assert_eq!(input_none.resume_provider_session_id, None);
    assert!(
        !input_none
            .prompt
            .contains("内存残留反馈（不得进入 fresh 输入）")
    );

    // 业务代同原：generation/outline/批次零变化；修复预算未重置。
    let index_after = store
        .load_active_index(
            &engine.session.project_id,
            &engine.session.issue_id,
            &plan_id,
        )
        .expect("load index after")
        .expect("index after");
    assert_eq!(
        index_after.current_generation_round_id,
        index_before.current_generation_round_id
    );
    assert_eq!(
        index_after.active_outline_id,
        index_before.active_outline_id
    );
    assert_eq!(index_after.batches.len(), index_before.batches.len());
    assert_eq!(index_after.draft_statuses, index_before.draft_statuses);
    assert!(
        engine
            .work_item_draft_repair_states
            .contains_key("outline_a"),
        "fresh 重建不得重置 draft 修复预算"
    );
    drop(lifecycle);
}

#[tokio::test]
async fn anchor_t16_batch_fresh_builder_and_node_rebuild_keep_batch() {
    let (_tmp, _checkpoint_store, lifecycle, plan_id, mut engine) =
        make_work_item_plan_engine_with_draft_candidate("sess_anchor_t16_batch");
    prepare_work_item_plan_outline_artifact(&mut engine).await;
    let store = engine.work_item_plan_store().expect("work item plan store");
    let mut outline_to_current_draft_id = BTreeMap::new();
    let mut draft_statuses = BTreeMap::new();
    let mut item_draft_ids = Vec::new();
    for outline_id in ["outline_a", "outline_b", "outline_c"] {
        let draft_id = format!("draft_{outline_id}");
        store
            .put_draft_record(&test_work_item_draft_record(
                &lifecycle,
                &plan_id,
                outline_id,
                &draft_id,
                WorkItemDraftStatus::Accepted,
                WorkItemGenerationMode::Batch,
                Some("batch_0001"),
            ))
            .expect("put batch draft record");
        outline_to_current_draft_id.insert(outline_id.to_string(), draft_id.clone());
        draft_statuses.insert(draft_id.clone(), WorkItemDraftStatus::Accepted);
        item_draft_ids.push(draft_id);
    }
    store
        .save_active_index(&WorkItemPlanDraftActiveIndex {
            project_id: engine.session.project_id.clone(),
            issue_id: engine.session.issue_id.clone(),
            plan_id: plan_id.clone(),
            current_generation_round_id: "round_0001".to_string(),
            outline_state: "confirmed".to_string(),
            active_outline_id: Some("outline_a".to_string()),
            outline_to_current_draft_id,
            draft_statuses,
            batches: vec![WorkItemBatchRecord {
                batch_id: "batch_0001".to_string(),
                generation_round_id: "round_0001".to_string(),
                mode: WorkItemGenerationMode::Batch,
                item_draft_ids,
                status: WorkItemBatchStatus::Generating,
                validation_failed_ids: Vec::new(),
                created_at: "2026-10-12T00:00:00Z".to_string(),
            }],
            updated_at: "2026-10-12T00:00:00Z".to_string(),
        })
        .expect("save batch active index");
    engine.session.provider_conversations = vec![anchor_t16_author_conversation_ref()];
    engine.pending_revision_context =
        Some("内存残留 batch 反馈（不得进入 fresh 输入）".to_string());

    // stale 收口后无活动 batch 节点：fresh 仅重建 provider 运行节点。
    assert_ne!(
        engine.active_node_type(),
        Some(TimelineNodeType::WorkItemBatchRun)
    );
    engine.begin_work_item_batch_fresh_recovery_run().await;
    assert_eq!(
        engine.active_node_type(),
        Some(TimelineNodeType::WorkItemBatchRun)
    );

    let frozen_feedback = "batch 冻结返修反馈（t16）";
    let input = engine
        .build_current_work_item_batch_draft_streaming_input_explicit_fresh(
            Some(frozen_feedback),
            &RoutingReferenceContext::Legacy,
        )
        .expect("fresh batch streaming input");
    assert_eq!(input.resume_provider_session_id, None);
    assert!(input.prompt.contains(frozen_feedback));
    assert!(
        !input
            .prompt
            .contains("内存残留 batch 反馈（不得进入 fresh 输入）"),
        "fresh 的 feedback 源是 durable 冻结值"
    );

    // 同 batch：不另建 batch、generation 不重建、draft 状态同原。
    let index_after = store
        .load_active_index(
            &engine.session.project_id,
            &engine.session.issue_id,
            &plan_id,
        )
        .expect("load index after")
        .expect("index after");
    assert_eq!(index_after.current_generation_round_id, "round_0001");
    assert_eq!(index_after.batches.len(), 1, "batch fresh 不得另建 batch");
    assert_eq!(index_after.batches[0].batch_id, "batch_0001");
    assert_eq!(index_after.batches[0].item_draft_ids.len(), 3);
    assert_eq!(
        index_after.batches[0].status,
        WorkItemBatchStatus::Generating
    );
    // 内存 pending 未被 take（fresh 直供 durable 冻结值）。
    assert!(engine.pending_revision_context.is_some());
}
