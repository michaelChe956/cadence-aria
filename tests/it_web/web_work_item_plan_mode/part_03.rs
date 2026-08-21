async fn enable_plan_raw_reviewer_completion(
    app: &axum::Router,
    session_id: &str,
    raw_text: &str,
) {
    let (status, response) = request_json(
        app.clone(),
        Method::POST,
        &format!("/api/test/workspace-sessions/{session_id}/review-fixture"),
        json!({
            "verdict": "needs_human",
            "summary": "raw outline review completion",
            "comments": "",
            "raw_text": raw_text,
            "findings": []
        }),
    )
    .await;
    assert_eq!(
        status,
        axum::http::StatusCode::OK,
        "enable raw outline review fixture failed: {response}"
    );
}

#[tokio::test]
async fn outline_human_confirm_request_change_starts_dedicated_revision_over_websocket() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls().await;
    let (app, root, prompts) = app_with_confirmed_story_and_design_and_streaming_raw_outputs(vec![
        QueuedSplitOutput::Json(valid_outline_output()),
        QueuedSplitOutput::Pending,
    ])
    .await;
    let (session_id, _plan_id, mut ws) = prepare_plan_and_start(&app, true).await;
    let raw_completion = concat!(
        "Outline review 业务内容保持不变。\n",
        "<ARIA_STRUCTURED_OUTPUT nonce=\"__NONCE__\">",
        "{\"verdict\":\"needs_human\",\"review_scope\":\"outline\",",
        "\"generation_round_id\":\"round_001\",\"summary\":\"Outline review 封装损坏\",",
        "\"findings\":[]}",
        "</ARIA_STRUCTURED_OUTPUT>"
    );
    enable_plan_raw_reviewer_completion(&app, &session_id, raw_completion).await;
    enable_plan_raw_reviewer_completion(&app, &session_id, raw_completion).await;

    ws.send(Message::Text(
        json!({ "type": "author_decision", "decision": "accept" })
            .to_string()
            .into(),
    ))
    .await
    .expect("send outline accept");
    let review_messages = recv_ws_until(&mut ws, Duration::from_secs(15), |messages| {
        messages
            .iter()
            .any(|message| message["type"] == "review_complete")
            && messages.iter().any(|message| {
                message["type"] == "timeline_node_created"
                    && message["node"]["node_type"] == "human_confirm"
            })
    })
    .await;
    let review_complete = review_messages
        .iter()
        .find(|message| message["type"] == "review_complete")
        .expect("outline review_complete");
    assert_eq!(review_complete["verdict"], "needs_human");
    assert_eq!(
        review_complete["structured_output_diagnostic"]["code"],
        "missing_json_nonce"
    );
    assert_eq!(
        review_complete["structured_output_diagnostic"]["repair_attempted"],
        true
    );
    assert_eq!(
        review_complete["structured_output_diagnostic"]["repair_succeeded"],
        false
    );
    let human_node_id = review_messages
        .iter()
        .find(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "human_confirm"
        })
        .and_then(|message| message["node"]["node_id"].as_str())
        .expect("human confirm node id")
        .to_string();

    ws.send(Message::Text(
        json!({
            "type": "human_confirm",
            "decision": "request-change",
            "payload": {
                "description": "按影响闭环契约修订 Outline，并明确集成测试 owner",
                "source": "human"
            }
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send human confirm request change");
    let revision_messages = recv_ws_until(&mut ws, Duration::from_secs(15), |messages| {
        let outline_run_node_ids = messages
            .iter()
            .filter_map(|message| {
                (message["type"] == "timeline_node_created"
                    && message["node"]["node_type"] == "work_item_plan_outline_run")
                    .then(|| message["node"]["node_id"].as_str())
                    .flatten()
            })
            .collect::<Vec<_>>();
        !outline_run_node_ids.is_empty()
            && messages.iter().any(|message| {
                message["type"] == "execution_event"
                    && message["event"]["title"] == "Provider Prompt"
                    && message["event"]["node_id"]
                        .as_str()
                        .is_some_and(|node_id| outline_run_node_ids.contains(&node_id))
            })
    })
    .await;
    let outline_run = revision_messages
        .iter()
        .find(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_plan_outline_run"
        })
        .expect("dedicated outline revision node");
    let outline_run_node_id = outline_run["node"]["node_id"]
        .as_str()
        .expect("outline run node id");
    assert_eq!(outline_run["node"]["stage"], "running");
    assert_eq!(outline_run["node"]["status"], "active");
    assert!(revision_messages.iter().all(|message| {
        !(message["type"] == "timeline_node_created"
            && message["node"]["node_type"] == "revision")
    }));
    let prompt_event = revision_messages
        .iter()
        .find(|message| {
            message["type"] == "execution_event"
                && message["event"]["title"] == "Provider Prompt"
                && message["event"]["node_id"] == outline_run_node_id
        })
        .expect("outline revision Provider Prompt event");
    assert!(prompt_event["event"]["detail"]
        .as_str()
        .expect("prompt event detail")
        .contains("增量返修提示词"));

    let prompts = wait_for_recorded_prompts(&prompts, 2).await;
    assert_eq!(prompts.len(), 2, "one initial run and one outline revision");
    let revision_prompt = prompts[1].clone();
    assert!(revision_prompt.contains("[impact_closure_contract]"));
    assert!(revision_prompt.contains("tests/it_core/**"));
    assert!(revision_prompt.contains("owner_mapping"));
    assert!(revision_prompt.contains("按影响闭环契约修订 Outline"));

    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = cadence_aria::product::lifecycle_store::LifecycleStore::new(app_paths);
    let timeline = lifecycle
        .load_timeline_nodes_for_issue_session("project_0001", "issue_0001", &session_id)
        .expect("load persisted timeline");
    assert!(!timeline.iter().any(|node| {
        node.node_type
            == cadence_aria::web::workspace_ws_types::TimelineNodeType::Revision
            && matches!(
                node.status,
                cadence_aria::web::workspace_ws_types::TimelineNodeStatus::Active
                    | cadence_aria::web::workspace_ws_types::TimelineNodeStatus::Paused
            )
    }));
    let human_node = timeline
        .iter()
        .find(|node| node.node_id == human_node_id)
        .expect("persisted source human confirm node");
    assert_eq!(
        human_node.status,
        cadence_aria::web::workspace_ws_types::TimelineNodeStatus::Completed
    );
    assert_eq!(
        human_node.summary.as_deref(),
        Some("Human Confirm 已请求返修 WorkItemPlan Outline")
    );
    assert_eq!(
        lifecycle
            .get_workspace_session(&session_id)
            .expect("workspace session")
            .status,
        cadence_aria::product::models::WorkspaceSessionStatus::Open
    );
    let session_record = lifecycle
        .get_workspace_session(&session_id)
        .expect("workspace session for recovery");
    let checkpoint_store = std::sync::Arc::new(
        cadence_aria::product::checkpoint_store::CheckpointStore::new(
            lifecycle
                .app_paths()
                .issue_lifecycle_root("project_0001", "issue_0001"),
        ),
    );
    let (event_tx, _event_rx) = tokio::sync::mpsc::channel(8);
    let recovered = cadence_aria::product::workspace_engine::WorkspaceEngine::new_persistent(
        checkpoint_store,
        lifecycle.clone(),
        event_tx,
        cadence_aria::product::workspace_engine::WorkspaceSession::from_record(session_record),
    );
    assert_eq!(recovered.current_stage().as_str(), "running");
    assert_eq!(
        recovered.active_timeline_node_id().as_deref(),
        Some(outline_run_node_id)
    );

    let (detail_status, outline_detail) = request_json(
        app.clone(),
        Method::GET,
        &format!(
            "/api/workspace-sessions/{session_id}/timeline-node-details/{outline_run_node_id}"
        ),
        json!({}),
    )
    .await;
    assert_eq!(detail_status, axum::http::StatusCode::OK);
    assert_eq!(outline_detail["prompt"], revision_prompt);

    ws.close(None).await.ok();
}
