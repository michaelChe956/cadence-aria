#[tokio::test]
async fn work_item_plan_outline_human_confirm_change_uses_outline_revision() {
    let (_tmp, _checkpoint_store, lifecycle, _plan_id, mut engine) =
        make_work_item_plan_engine_with_draft_candidate("sess_wip_outline_human_change");
    let persisted_session = lifecycle
        .list_workspace_sessions("project_0001", "issue_0001")
        .expect("workspace sessions")
        .into_iter()
        .next()
        .expect("persisted workspace session");
    engine.session.session_id = persisted_session.id;
    prepare_work_item_plan_outline_artifact(&mut engine).await;
    engine.complete_active_node(Some("准备 Outline review".to_string())).await;

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
            Some("结构化输出降级到人工确认".to_string()),
        )
        .await;
    engine.latest_review_verdict = Some(ReviewVerdict {
        verdict: ReviewVerdictType::NeedsHuman,
        comments: "review 输出缺少结束 nonce".to_string(),
        summary: "需要人工确认 Outline review".to_string(),
        findings: Vec::new(),
        review_gate: ReviewGate::UserTriageRequired,
        work_item_plan_review: None,
        structured_output_diagnostic: Some(StructuredOutputDiagnostic {
            code: "missing_end_nonce".to_string(),
            message: "missing structured output end nonce".to_string(),
            repair_attempted: true,
            repair_succeeded: false,
            raw_output_preview: Some("[must_fix] 伪造 finding 不得进入返修反馈".to_string()),
        }),
    });
    engine
        .enter_human_confirm(Some("等待人工确认 Outline review".to_string()))
        .await;

    let outcome = engine
        .handle_human_confirm(
            HumanConfirmDecision::RequestChange,
            Some(serde_json::json!({"description": "请补齐共享状态影响面"})),
        )
        .await
        .expect("human request change should revise outline");

    let ReviewDecisionOutcome::StartWorkItemPlanOutlineRevision { feedback } = outcome else {
        panic!("expected WorkItemPlan Outline revision outcome");
    };
    let feedback = feedback.expect("outline revision feedback");
    assert!(feedback.contains("[impact_closure_contract]"));
    assert!(feedback.contains("请补齐共享状态影响面"));
    assert!(!feedback.contains("伪造 finding"));
    assert_eq!(engine.session().stage, WorkspaceStage::Running);
    assert!(
        !engine.timeline_nodes.iter().any(|node| {
            node.node_type == TimelineNodeType::Revision
                && matches!(
                    node.status,
                    TimelineNodeStatus::Active | TimelineNodeStatus::Paused
                )
        }),
        "outline human revision must not create an active generic Revision node"
    );
    let human_confirm = engine
        .timeline_nodes
        .iter()
        .find(|node| node.node_type == TimelineNodeType::HumanConfirm)
        .expect("human confirm node");
    assert_eq!(human_confirm.status, TimelineNodeStatus::Completed);
    assert_eq!(
        human_confirm.summary.as_deref(),
        Some("Human Confirm 已请求返修 WorkItemPlan Outline")
    );
}

#[tokio::test]
async fn generic_human_confirm_request_change_remains_for_story_design_and_work_item() {
    for workspace_type in [
        WorkspaceType::Story,
        WorkspaceType::Design,
        WorkspaceType::WorkItem,
    ] {
        let (_tmp, store) = setup();
        let (tx, _) = mpsc::channel(64);
        let mut session = make_session(&format!("sess_generic_human_{workspace_type:?}"));
        session.workspace_type = workspace_type.clone();
        session.artifact = Some(artifact_payload("# Generic candidate"));
        let mut engine = WorkspaceEngine::new(store, tx, session);
        engine
            .enter_human_confirm(Some("等待人工确认".to_string()))
            .await;

        let outcome = engine
            .handle_human_confirm(
                HumanConfirmDecision::RequestChange,
                Some(serde_json::json!({"description": "补充边界条件"})),
            )
            .await
            .expect("generic human request change");

        assert_eq!(
            outcome,
            ReviewDecisionOutcome::StartRevision,
            "{workspace_type:?} should keep the generic revision path"
        );
        assert_eq!(engine.session().stage, WorkspaceStage::Revision);
        assert!(
            engine.timeline_nodes.iter().any(|node| {
                node.node_type == TimelineNodeType::Revision
                    && node.status == TimelineNodeStatus::Active
            }),
            "{workspace_type:?} should create an active generic Revision node"
        );
    }
}
