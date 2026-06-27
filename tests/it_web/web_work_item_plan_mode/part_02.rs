#[tokio::test]
async fn request_outline_revision_on_mode_node_sets_outline_revising() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls().await;
    let (app, root, _prompts) = app_with_confirmed_story_and_design_and_streaming_outputs(vec![
        valid_outline_output(),
        valid_outline_output(),
    ])
    .await;
    let (_session_id, plan_id, mut ws) = prepare_plan_and_start(&app, false).await;

    ws.send(Message::Text(
        json!({ "type": "author_decision", "decision": "accept" })
            .to_string()
            .into(),
    ))
    .await
    .expect("send accept");
    let _messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_generation_mode"
        })
    })
    .await;

    ws.send(Message::Text(
        json!({
            "type": "request_outline_revision",
            "feedback": "先拆出错误状态处理"
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send outline revision");
    let messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        let outline_run_node_ids: Vec<&str> = messages
            .iter()
            .filter_map(|message| {
                if message["type"] == "timeline_node_created"
                    && message["node"]["node_type"] == "work_item_plan_outline_run"
                {
                    message["node"]["node_id"].as_str()
                } else {
                    None
                }
            })
            .collect();
        let saw_provider_prompt = messages.iter().any(|message| {
            message["type"] == "execution_event"
                && message["event"]["title"] == "Provider Prompt"
                && message["event"]["node_id"]
                    .as_str()
                    .is_some_and(|node_id| outline_run_node_ids.contains(&node_id))
        });
        let saw_outline_confirm = messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_plan_outline_confirm"
        });
        !outline_run_node_ids.is_empty() && saw_provider_prompt && saw_outline_confirm
    })
    .await;

    let index = active_index(&root, &plan_id);
    assert_eq!(index.outline_state, "revising");
    assert!(
        messages.iter().any(|message| {
            message["type"] == "execution_event" && message["event"]["title"] == "Provider Prompt"
        }),
        "request_outline_revision should start a provider run for the new outline node"
    );

    ws.close(None).await.ok();
}

#[tokio::test]
async fn select_mode_rejected_outside_generation_mode_node() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls().await;
    let (app, _root, _prompts) =
        app_with_confirmed_story_and_design_and_streaming_outputs(vec![valid_outline_output()])
            .await;
    let (_session_id, _plan_id, mut ws) = prepare_plan_and_start(&app, false).await;

    ws.send(Message::Text(
        json!({ "type": "select_work_item_generation_mode", "mode": "serial" })
            .to_string()
            .into(),
    ))
    .await
    .expect("send invalid mode");
    let messages = recv_ws_until(&mut ws, Duration::from_secs(5), |messages| {
        messages
            .iter()
            .any(|message| message["type"] == "protocol_error")
    })
    .await;
    let protocol_error = messages
        .iter()
        .find(|message| message["type"] == "protocol_error")
        .expect("protocol error");
    assert_eq!(
        protocol_error["code"],
        "WORK_ITEM_GENERATION_MODE_NODE_REQUIRED"
    );

    ws.close(None).await.ok();
}

#[tokio::test]
async fn session_state_restores_generation_mode_node_with_outline_payload() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls().await;
    let (app, _root, _prompts) =
        app_with_confirmed_story_and_design_and_streaming_outputs(vec![valid_outline_output()])
            .await;
    let (session_id, _plan_id, mut ws) = prepare_plan_and_start(&app, false).await;

    ws.send(Message::Text(
        json!({ "type": "author_decision", "decision": "accept" })
            .to_string()
            .into(),
    ))
    .await
    .expect("send accept");
    let _messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_generation_mode"
        })
    })
    .await;
    ws.close(None).await.ok();

    let mut restored = connect_ws(app.clone(), &session_id).await;
    let messages = recv_ws_until(&mut restored, Duration::from_secs(5), |messages| {
        messages
            .iter()
            .any(|message| message["type"] == "session_state")
    })
    .await;
    let state = messages
        .iter()
        .find(|message| message["type"] == "session_state")
        .expect("session state");
    assert_eq!(state["stage"], "author_confirm");
    let active_node_id = state["active_node_id"].as_str().expect("active node id");
    let active_node = state["timeline_nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["node_id"] == active_node_id)
        .expect("active node");
    assert_eq!(active_node["node_type"], "work_item_generation_mode");
    assert!(state["artifact"].get("outline_candidate").is_some());
    assert_eq!(
        state["artifact"]["outline_candidate"]["current_generation_round_id"],
        "round_001"
    );

    restored.close(None).await.ok();
}
