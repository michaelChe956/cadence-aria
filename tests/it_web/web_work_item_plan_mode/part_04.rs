#[tokio::test]
async fn outline_author_confirm_reject_starts_revision_provider_over_websocket() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls().await;
    let (app, _root, prompts) =
        app_with_confirmed_story_and_design_and_streaming_raw_outputs(vec![
            QueuedSplitOutput::Json(valid_outline_output()),
            QueuedSplitOutput::Pending,
        ])
        .await;
    let (_session_id, _plan_id, mut ws) = prepare_plan_and_start(&app, false).await;

    ws.send(Message::Text(
        json!({ "type": "author_decision", "decision": "reject" })
            .to_string()
            .into(),
    ))
    .await
    .expect("send outline reject");
    let messages = recv_ws_until(&mut ws, Duration::from_secs(15), |messages| {
        messages.iter().any(|message| {
            message["type"] == "execution_event"
                && message["event"]["title"] == "Provider Prompt"
        })
    })
    .await;
    assert!(messages.iter().any(|message| {
        message["type"] == "timeline_node_created"
            && message["node"]["node_type"] == "work_item_plan_outline_run"
    }));
    assert!(messages.iter().any(|message| {
        message["type"] == "execution_event" && message["event"]["title"] == "Provider Prompt"
    }));
    let prompts = wait_for_recorded_prompts(&prompts, 2).await;
    assert_eq!(prompts.len(), 2, "reject must start exactly one revision provider");
    ws.close(None).await.ok();
}

#[tokio::test]
async fn persisted_outline_revision_intent_resumes_incremental_prompt_on_new_app_websocket() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls().await;
    let (app, root, _initial_prompts) =
        app_with_confirmed_story_and_design_and_streaming_raw_outputs(vec![
            QueuedSplitOutput::Json(valid_outline_output()),
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
    let messages = recv_ws_until(&mut ws, Duration::from_secs(15), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "human_confirm"
        })
    })
    .await;
    assert!(messages.iter().any(|message| {
        message["type"] == "timeline_node_created"
            && message["node"]["node_type"] == "human_confirm"
    }));
    ws.close(None).await.ok();

    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = cadence_aria::product::lifecycle_store::LifecycleStore::new(app_paths.clone());
    let session_record = lifecycle
        .get_workspace_session(&session_id)
        .expect("persisted workspace session");
    let checkpoint_store = std::sync::Arc::new(
        cadence_aria::product::checkpoint_store::CheckpointStore::new(
            app_paths.issue_lifecycle_root("project_0001", "issue_0001"),
        ),
    );
    let (event_tx, _event_rx) = tokio::sync::mpsc::channel(8);
    let mut session =
        cadence_aria::product::workspace_engine::WorkspaceSession::from_record(session_record);
    session.repository_path = Some(root.path().join("repo"));
    let mut engine = cadence_aria::product::workspace_engine::WorkspaceEngine::new_persistent(
        checkpoint_store,
        lifecycle.clone(),
        event_tx,
        session,
    );
    engine
        .handle_human_confirm(
            cadence_aria::web::workspace_ws_types::HumanConfirmDecision::RequestChange,
            Some(json!({
                "description": "按影响闭环契约修订 Outline，并明确集成测试 owner"
            })),
        )
        .await
        .expect("persist revision before provider spawn");
    let run_node_id = engine
        .active_timeline_node_id()
        .expect("persisted active outline revision run");
    let run_detail = lifecycle
        .load_node_detail(&session_id, &run_node_id)
        .expect("persisted run detail");
    assert!(run_detail.is_revision);
    assert!(run_detail
        .revision_feedback
        .as_deref()
        .expect("persisted revision feedback")
        .contains("按影响闭环契约修订 Outline"));
    drop(engine);
    drop(app);

    let (restarted_app, restarted_prompts) = app_for_existing_root_with_streaming_raw_outputs(
        root.path(),
        vec![QueuedSplitOutput::Pending],
    );
    let mut reconnected_ws = connect_ws(restarted_app, &session_id).await;
    let resumed_messages = recv_ws_until(
        &mut reconnected_ws,
        Duration::from_secs(15),
        |messages| {
            messages.iter().any(|message| {
                message["type"] == "execution_event"
                    && message["event"]["title"] == "Provider Prompt"
            })
        },
    )
    .await;
    assert!(resumed_messages.iter().any(|message| {
        message["type"] == "execution_event"
            && message["event"]["title"] == "Provider Prompt"
            && message["event"]["node_id"] == run_node_id
    }));
    let prompts = wait_for_recorded_prompts(&restarted_prompts, 1).await;
    assert_eq!(prompts.len(), 1, "reconnect must resume exactly one provider");
    let prompt = &prompts[0];
    assert!(prompt.contains("[impact_closure_contract]"));
    assert!(prompt.contains("tests/it_core/**"));
    assert!(prompt.contains("owner_mapping"));
    assert!(prompt.contains("按影响闭环契约修订 Outline"));
    assert!(!prompt.contains("[confirmed_story_specs]"));
    assert!(!prompt.contains("[repository_structure_summary]"));
    reconnected_ws.close(None).await.ok();
}
