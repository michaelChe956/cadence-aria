#[tokio::test]
async fn work_item_plan_outline_human_confirm_change_survives_persisted_timeline_restore() {
    let _guard = WS_TEST_LOCK.lock().await;
    let (app, repo, _prompts) =
        app_with_confirmed_story_and_design_and_streaming_outputs(vec![valid_outline_output()])
            .await;
    let (_status, prepare_resp) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-item-plans:prepare",
        json!({
            "title": "Outline HumanConfirm 恢复测试 Plan",
            "story_spec_ids": ["story_spec_0001"],
            "design_spec_ids": ["design_spec_0001"],
            "author_provider": "fake",
            "reviewer_provider": null,
            "review_rounds": 1,
            "superpowers_enabled": false,
            "openspec_enabled": true,
            "include_integration_tests": true,
            "include_e2e_tests": false,
            "force_frontend_backend_split": true,
            "require_execution_plan_confirm": false
        }),
    )
    .await;
    let session_id = prepare_resp["workspace_session"]["workspace_session_id"]
        .as_str()
        .expect("workspace session id")
        .to_string();
    let mut ws = connect_ws(app, &session_id).await;
    ws.send(Message::Text(
        json!({
            "type": "start_generation",
            "provider_config": { "author": "fake", "reviewer": null, "review_rounds": 0 },
            "reviewer_enabled": false
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send start_generation");
    let _messages = recv_ws_until(&mut ws, Duration::from_secs(15), |messages| {
        messages.iter().any(|message| message["type"] == "artifact_update")
            && messages.iter().any(|message| {
                message["type"] == "stage_change" && message["stage"] == "author_confirm"
            })
    })
    .await;
    ws.close(None).await.ok();

    let app_paths = ProductAppPaths::new(repo.path().join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths);
    let mut timeline = lifecycle
        .load_timeline_nodes_for_issue_session("project_0001", "issue_0001", &session_id)
        .expect("load persisted timeline");
    let now = chrono::Utc::now().to_rfc3339();
    for node in &mut timeline {
        if matches!(
            node.status,
            TimelineNodeStatus::Active | TimelineNodeStatus::Paused
        ) {
            node.status = TimelineNodeStatus::Completed;
            node.completed_at = Some(now.clone());
            node.duration_ms = Some(0);
        }
    }
    let provider_config_snapshot = timeline
        .last()
        .expect("existing timeline node")
        .provider_config_snapshot
        .clone();
    let review_node_id = format!("timeline_node_{:03}", timeline.len() + 1);
    timeline.push(TimelineNode {
        node_id: review_node_id,
        node_type: TimelineNodeType::WorkItemPlanOutlineReview,
        agent: None,
        stage: WorkspaceStage::CrossReview,
        round: Some(1),
        status: TimelineNodeStatus::Completed,
        title: "WorkItemPlan Outline Review Round 1".to_string(),
        summary: Some("结构化输出降级到人工确认".to_string()),
        started_at: now.clone(),
        completed_at: Some(now.clone()),
        duration_ms: Some(0),
        artifact_ref: Some("artifact_current".to_string()),
        provider_config_snapshot: provider_config_snapshot.clone(),
        retry: None,
    });
    let human_node_id = format!("timeline_node_{:03}", timeline.len() + 1);
    timeline.push(TimelineNode {
        node_id: human_node_id.clone(),
        node_type: TimelineNodeType::HumanConfirm,
        agent: None,
        stage: WorkspaceStage::HumanConfirm,
        round: None,
        status: TimelineNodeStatus::Active,
        title: "人工确认".to_string(),
        summary: Some("等待人工确认 Outline review".to_string()),
        started_at: now,
        completed_at: None,
        duration_ms: None,
        artifact_ref: Some("artifact_current".to_string()),
        provider_config_snapshot,
        retry: None,
    });
    lifecycle
        .save_timeline_nodes(&session_id, &timeline)
        .expect("persist outline review and human confirm timeline");
    lifecycle
        .update_workspace_session_status(
            &session_id,
            cadence_aria::product::models::WorkspaceSessionStatus::WaitingForHuman,
        )
        .expect("persist waiting status");

    let mut recovered = recover_engine(&repo, &session_id);
    assert_eq!(recovered.current_stage().as_str(), "human_confirm");
    let outcome = recovered
        .handle_human_confirm(
            cadence_aria::web::workspace_ws_types::HumanConfirmDecision::RequestChange,
            Some(json!({"description": "刷新后继续返修 Outline"})),
        )
        .await
        .expect("restored human confirm request change");

    let cadence_aria::product::workspace_engine::ReviewDecisionOutcome::StartWorkItemPlanOutlineRevision {
        feedback,
    } = outcome
    else {
        panic!("expected restored Outline revision outcome");
    };
    assert!(
        feedback
            .as_deref()
            .is_some_and(|value| value.contains("刷新后继续返修 Outline"))
    );
    assert_eq!(recovered.current_stage().as_str(), "running");
    let persisted_timeline = lifecycle
        .load_timeline_nodes_for_issue_session("project_0001", "issue_0001", &session_id)
        .expect("reload persisted timeline");
    assert!(
        !persisted_timeline.iter().any(|node| {
            node.node_type == TimelineNodeType::Revision
                && matches!(
                    node.status,
                    TimelineNodeStatus::Active | TimelineNodeStatus::Paused
                )
        }),
        "restored Outline revision must not create a generic Revision node"
    );
    let human_node = persisted_timeline
        .iter()
        .find(|node| node.node_id == human_node_id)
        .expect("persisted human confirm node");
    assert_eq!(human_node.status, TimelineNodeStatus::Completed);
    assert_eq!(
        human_node.summary.as_deref(),
        Some("Human Confirm 已请求返修 WorkItemPlan Outline")
    );
    assert_eq!(
        lifecycle
            .get_workspace_session(&session_id)
            .expect("persisted workspace session")
            .status,
        cadence_aria::product::models::WorkspaceSessionStatus::Open
    );
}
