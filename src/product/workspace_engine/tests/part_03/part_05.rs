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
        .build_current_work_item_draft_streaming_input(None, &RoutingReferenceContext::Legacy)
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

#[derive(Debug, Clone, Copy)]
enum OutlineRevisionEntryCase {
    AuthorConfirmReject,
    RequestRevision,
    RequestOutlineRevision,
    ReviewDecision,
    HumanConfirm,
}

#[tokio::test]
async fn work_item_plan_outline_revision_entry_points_share_preparation_state() {
    for case in [
        OutlineRevisionEntryCase::AuthorConfirmReject,
        OutlineRevisionEntryCase::RequestRevision,
        OutlineRevisionEntryCase::RequestOutlineRevision,
        OutlineRevisionEntryCase::ReviewDecision,
        OutlineRevisionEntryCase::HumanConfirm,
    ] {
        let (_tmp, _checkpoint_store, lifecycle, plan_id, mut engine) =
            make_work_item_plan_engine_with_draft_candidate(&format!(
                "sess_outline_revision_entry_{case:?}"
            ));
        let persisted_session = lifecycle
            .list_workspace_sessions("project_0001", "issue_0001")
            .expect("workspace sessions")
            .into_iter()
            .next()
            .expect("persisted workspace session");
        let session_id = persisted_session.id;
        engine.session.session_id = session_id.clone();
        prepare_work_item_plan_outline_artifact(&mut engine).await;
        save_serial_work_item_plan_index(&engine, &plan_id, "outline_a");
        engine.complete_active_node(Some("进入 Outline revision 测试".to_string())).await;
        engine.work_item_plan_author_retry_count = 2;
        engine.work_item_plan_revision_retry_count = 2;
        lifecycle
            .update_workspace_session_status(
                &session_id,
                WorkspaceSessionStatus::WaitingForHuman,
            )
            .expect("set waiting status");

        let (source_summary, source_node_id) = match case {
            OutlineRevisionEntryCase::AuthorConfirmReject => {
                engine.session.stage = WorkspaceStage::AuthorConfirm;
                let node_id = engine
                    .create_timeline_node(TimelineNodeDraft {
                        node_type: TimelineNodeType::WorkItemPlanOutlineConfirm,
                        agent: None,
                        stage: WorkspaceStage::AuthorConfirm,
                        round: None,
                        title: "WorkItemPlan Outline 确认".to_string(),
                        summary: None,
                        status: TimelineNodeStatus::Active,
                    })
                    .await;
                let outcome = engine
                    .handle_author_decision(AuthorDecision::Reject)
                    .await
                    .expect("author confirm reject");
                assert_eq!(
                    outcome,
                    AuthorDecisionOutcome::StartWorkItemPlanOutlineRevision { feedback: None }
                );
                ("Author Confirm 已请求返修 WorkItemPlan Outline", node_id)
            }
            OutlineRevisionEntryCase::RequestRevision => {
                engine.session.stage = WorkspaceStage::AuthorConfirm;
                let node_id = engine
                    .create_timeline_node(TimelineNodeDraft {
                        node_type: TimelineNodeType::WorkItemPlanOutlineConfirm,
                        agent: None,
                        stage: WorkspaceStage::AuthorConfirm,
                        round: None,
                        title: "WorkItemPlan Outline 确认".to_string(),
                        summary: None,
                        status: TimelineNodeStatus::Active,
                    })
                    .await;
                let outcome = engine
                    .request_work_item_plan_revision(Some("缩小 Outline scope".to_string()))
                    .await
                    .expect("request outline revision");
                assert!(matches!(
                    outcome,
                    ReviewDecisionOutcome::StartWorkItemPlanOutlineRevision { .. }
                ));
                ("Author Confirm 已请求返修 WorkItemPlan Outline", node_id)
            }
            OutlineRevisionEntryCase::RequestOutlineRevision => {
                engine.session.stage = WorkspaceStage::AuthorConfirm;
                let node_id = engine
                    .create_timeline_node(TimelineNodeDraft {
                        node_type: TimelineNodeType::WorkItemPlanOutlineConfirm,
                        agent: None,
                        stage: WorkspaceStage::AuthorConfirm,
                        round: None,
                        title: "WorkItemPlan Outline 确认".to_string(),
                        summary: None,
                        status: TimelineNodeStatus::Active,
                    })
                    .await;
                let feedback = engine
                    .request_work_item_plan_outline_revision(Some(
                        "保留用户 context 并补齐影响闭环".to_string(),
                    ))
                    .await
                    .expect("dedicated request outline revision");
                assert!(feedback
                    .as_deref()
                    .expect("persisted complete feedback")
                    .contains("保留用户 context 并补齐影响闭环"));
                ("Author Confirm 已请求返修 WorkItemPlan Outline", node_id)
            }
            OutlineRevisionEntryCase::ReviewDecision => {
                engine.session.stage = WorkspaceStage::ReviewDecision;
                let node_id = engine
                    .create_timeline_node(TimelineNodeDraft {
                        node_type: TimelineNodeType::ReviewDecision,
                        agent: None,
                        stage: WorkspaceStage::ReviewDecision,
                        round: Some(1),
                        title: "Review 决策".to_string(),
                        summary: None,
                        status: TimelineNodeStatus::Active,
                    })
                    .await;
                let outcome = engine
                    .handle_review_decision(
                        "continue_with_context".to_string(),
                        Some("覆盖共享状态影响面".to_string()),
                    )
                    .await
                    .expect("review decision outline revision");
                assert!(matches!(
                    outcome,
                    ReviewDecisionOutcome::StartWorkItemPlanOutlineRevision { .. }
                ));
                ("Review Decision 已请求返修 WorkItemPlan Outline", node_id)
            }
            OutlineRevisionEntryCase::HumanConfirm => {
                let review_node_id = engine
                    .create_timeline_node(TimelineNodeDraft {
                        node_type: TimelineNodeType::WorkItemPlanOutlineReview,
                        agent: Some(ProviderName::Codex),
                        stage: WorkspaceStage::CrossReview,
                        round: Some(1),
                        title: "WorkItemPlan Outline Review Round 1".to_string(),
                        summary: None,
                        status: TimelineNodeStatus::Active,
                    })
                    .await;
                engine
                    .update_timeline_node(
                        &review_node_id,
                        TimelineNodeStatus::Completed,
                        Some("转人工确认".to_string()),
                    )
                    .await;
                engine
                    .enter_human_confirm(Some("等待人工确认".to_string()))
                    .await;
                let node_id = engine.active_node_id.clone().expect("human confirm node id");
                let outcome = engine
                    .handle_human_confirm(
                        HumanConfirmDecision::RequestChange,
                        Some(serde_json::json!({"description": "修复 Outline"})),
                    )
                    .await
                    .expect("human confirm outline revision");
                assert!(matches!(
                    outcome,
                    ReviewDecisionOutcome::StartWorkItemPlanOutlineRevision { .. }
                ));
                ("Human Confirm 已请求返修 WorkItemPlan Outline", node_id)
            }
        };

        assert_eq!(engine.session().stage, WorkspaceStage::Running, "{case:?}");
        assert_eq!(engine.work_item_plan_author_retry_count, 0, "{case:?}");
        assert_eq!(engine.work_item_plan_revision_retry_count, 0, "{case:?}");
        assert!(
            engine.artifact_versions.iter().all(|version| !version.is_current),
            "{case:?} should reject the latest artifact"
        );
        assert!(
            !engine.timeline_nodes.iter().any(|node| {
                node.node_type == TimelineNodeType::Revision
                    && matches!(
                        node.status,
                        TimelineNodeStatus::Active | TimelineNodeStatus::Paused
                    )
            }),
            "{case:?} should not create a generic Revision node"
        );
        let active_outline_runs = engine
            .timeline_nodes
            .iter()
            .filter(|node| {
                node.node_type == TimelineNodeType::WorkItemPlanOutlineRun
                    && node.status == TimelineNodeStatus::Active
            })
            .collect::<Vec<_>>();
        assert_eq!(
            active_outline_runs.len(),
            1,
            "{case:?} should create exactly one active outline revision run"
        );
        assert_eq!(
            engine.active_node_id.as_deref(),
            Some(active_outline_runs[0].node_id.as_str()),
            "{case:?} active node should be the outline revision run"
        );
        let run_detail = lifecycle
            .load_node_detail(&session_id, &active_outline_runs[0].node_id)
            .expect("persisted outline revision run detail");
        assert!(run_detail.is_revision, "{case:?}");
        if matches!(case, OutlineRevisionEntryCase::RequestOutlineRevision) {
            assert!(run_detail
                .revision_feedback
                .as_deref()
                .expect("persisted request outline feedback")
                .contains("保留用户 context 并补齐影响闭环"));
        }
        let source_node = engine
            .timeline_nodes
            .iter()
            .find(|node| node.node_id == source_node_id)
            .expect("source node");
        assert_eq!(source_node.status, TimelineNodeStatus::Completed, "{case:?}");
        assert_eq!(source_node.summary.as_deref(), Some(source_summary), "{case:?}");
        let active_index = engine
            .work_item_plan_store()
            .expect("work item plan store")
            .load_active_index("project_0001", "issue_0001", &plan_id)
            .expect("load active index")
            .expect("active index");
        assert_eq!(active_index.outline_state, "revising", "{case:?}");
        let persisted_session = lifecycle
            .get_workspace_session(&session_id)
            .expect("persisted workspace session");
        assert_eq!(
            persisted_session.status,
            WorkspaceSessionStatus::Open,
            "{case:?}"
        );
    }
}
