#[tokio::test]
async fn outline_human_confirm_revision_is_recoverable_before_provider_spawn() {
    let (_tmp, checkpoint_store, lifecycle, _plan_id, mut engine) =
        make_work_item_plan_engine_with_draft_candidate("sess_outline_recovery_window");
    let persisted_session = lifecycle
        .list_workspace_sessions("project_0001", "issue_0001")
        .expect("workspace sessions")
        .into_iter()
        .next()
        .expect("persisted workspace session");
    let session_id = persisted_session.id;
    engine.session.session_id = session_id.clone();
    prepare_work_item_plan_outline_artifact(&mut engine).await;
    engine
        .complete_active_node(Some("准备 Outline review".to_string()))
        .await;
    let review_node_id = engine.begin_work_item_plan_outline_review_run().await;
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
            raw_output_preview: None,
        }),
    });
    engine
        .enter_human_confirm(Some("等待人工确认 Outline review".to_string()))
        .await;

    let outcome = engine
        .handle_human_confirm(
            HumanConfirmDecision::RequestChange,
            Some(serde_json::json!({"description": "补齐共享影响闭环", "source": "human"})),
        )
        .await
        .expect("human request change");
    assert!(matches!(
        outcome,
        ReviewDecisionOutcome::StartWorkItemPlanOutlineRevision { .. }
    ));

    let active_node_id = engine
        .active_timeline_node_id()
        .expect("active outline revision node");
    assert_eq!(
        engine.active_node_type(),
        Some(TimelineNodeType::WorkItemPlanOutlineRun)
    );
    let persisted_timeline = lifecycle
        .load_timeline_nodes_for_issue_session("project_0001", "issue_0001", &session_id)
        .expect("persisted timeline");
    let active_node = persisted_timeline
        .iter()
        .find(|node| node.node_id == active_node_id)
        .expect("persisted active outline run");
    assert_eq!(active_node.node_type, TimelineNodeType::WorkItemPlanOutlineRun);
    assert_eq!(active_node.status, TimelineNodeStatus::Active);
    let active_detail = lifecycle
        .load_node_detail(&session_id, &active_node_id)
        .expect("persisted outline revision detail");
    let active_detail_json = serde_json::to_value(active_detail).expect("serialize node detail");
    assert_eq!(active_detail_json["is_revision"], true);
    assert!(active_detail_json["revision_feedback"]
        .as_str()
        .expect("persisted revision feedback")
        .contains("补齐共享影响闭环"));
    assert_eq!(
        persisted_timeline
            .iter()
            .filter(|node| node.node_type == TimelineNodeType::WorkItemPlanOutlineRun)
            .count(),
        1
    );

    let session_record = lifecycle
        .get_workspace_session(&session_id)
        .expect("persisted workspace session");
    assert_eq!(session_record.status, WorkspaceSessionStatus::Open);
    let (event_tx, _event_rx) = mpsc::channel(8);
    let recovered = WorkspaceEngine::new_persistent(
        checkpoint_store,
        lifecycle,
        event_tx,
        WorkspaceSession::from_record(session_record),
    );
    assert_eq!(recovered.current_stage(), WorkspaceStage::Running);
    assert_eq!(
        recovered.active_node_type(),
        Some(TimelineNodeType::WorkItemPlanOutlineRun)
    );
}
