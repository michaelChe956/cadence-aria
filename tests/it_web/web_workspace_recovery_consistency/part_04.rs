async fn enable_raw_reviewer_completion(
    app: &axum::Router,
    session_id: &str,
    raw_text: &str,
) {
    let (status, response) = request_json(
        app.clone(),
        Method::POST,
        &format!("/api/test/workspace-sessions/{session_id}/review-fixture"),
        json!({
            "verdict": "pass",
            "summary": "raw reviewer completion",
            "comments": "",
            "raw_text": raw_text,
            "findings": []
        }),
    )
    .await;
    assert_eq!(
        status,
        axum::http::StatusCode::OK,
        "enable raw review fixture failed: {response}"
    );
}

#[tokio::test]
async fn reviewer_repair_failure_live_diagnostic_matches_reloaded_node_detail() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls().await;
    let (app, _repo, _prompts) =
        app_with_confirmed_story_and_design_and_streaming_outputs(vec![valid_outline_output()])
            .await;

    let (status, response) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/story-specs:generate",
        json!({
            "title": "Reviewer repair recovery consistency",
            "author_provider": "fake",
            "reviewer_provider": "codex",
            "review_rounds": 1,
            "superpowers_enabled": false,
            "openspec_enabled": true
        }),
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::OK, "{response}");
    let session_id = response["workspace_session"]["workspace_session_id"]
        .as_str()
        .expect("workspace session id")
        .to_string();

    let mut ws = connect_ws(app.clone(), &session_id).await;
    ws.send(Message::Text(
        json!({
            "type": "start_generation",
            "provider_config": { "author": "fake", "reviewer": "codex", "review_rounds": 1 },
            "reviewer_enabled": true
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send start_generation");
    let _messages = recv_ws_until(&mut ws, Duration::from_secs(15), |messages| {
        messages.iter().any(|message| {
            message["type"] == "stage_change" && message["stage"] == "author_confirm"
        })
    })
    .await;

    let raw_completion = concat!(
        "审核内容保持不变。\n",
        "<ARIA_STRUCTURED_OUTPUT nonce=\"__NONCE__\">",
        "{\"verdict\":\"pass\",\"summary\":\"内容可确认但封装损坏\",\"findings\":[]}",
        "</ARIA_STRUCTURED_OUTPUT>"
    );
    enable_raw_reviewer_completion(&app, &session_id, raw_completion).await;
    enable_raw_reviewer_completion(&app, &session_id, raw_completion).await;

    ws.send(Message::Text(
        json!({ "type": "author_decision", "decision": "accept" })
            .to_string()
            .into(),
    ))
    .await
    .expect("send author accept");
    let messages = recv_ws_until(&mut ws, Duration::from_secs(15), |messages| {
        messages
            .iter()
            .any(|message| message["type"] == "review_complete")
            && messages.iter().any(|message| {
                message["type"] == "stage_change" && message["stage"] == "human_confirm"
            })
    })
    .await;
    let live_review = messages
        .iter()
        .find(|message| message["type"] == "review_complete")
        .expect("live review_complete");
    let review_node_id = live_review["node_id"]
        .as_str()
        .expect("review node id")
        .to_string();
    let live_diagnostic = &live_review["structured_output_diagnostic"];
    assert_eq!(live_review["round"], 1);
    assert_eq!(live_review["verdict"], "needs_human");
    assert_eq!(live_review["findings"], json!([]));
    assert_eq!(live_diagnostic["code"], "missing_end_nonce");
    assert_eq!(live_diagnostic["repair_attempted"], true);
    assert_eq!(live_diagnostic["repair_succeeded"], false);
    assert!(
        live_diagnostic["raw_output_preview"]
            .as_str()
            .expect("raw output preview")
            .chars()
            .count()
            <= 2_048
    );
    assert_eq!(
        messages
            .iter()
            .filter(|message| {
                message["type"] == "timeline_node_created"
                    && message["node"]["node_type"] == "reviewer_run"
            })
            .count(),
        1,
        "repair must reuse the same business review node"
    );
    assert!(messages.iter().any(|message| {
        message["type"] == "execution_event"
            && message["event"]["event_id"] == "structured_output_repair"
            && message["event"]["status"] == "failed"
    }));
    ws.close(None).await.ok();

    let mut restored_ws = connect_ws(app.clone(), &session_id).await;
    let restored_messages =
        recv_ws_until(&mut restored_ws, Duration::from_secs(5), |messages| {
            messages
                .iter()
                .any(|message| message["type"] == "session_state")
        })
        .await;
    let restored_state = restored_messages
        .iter()
        .find(|message| message["type"] == "session_state")
        .expect("restored session state");
    assert!(restored_state["timeline_nodes"]
        .as_array()
        .expect("timeline nodes")
        .iter()
        .any(|node| node["node_id"] == review_node_id));

    let (detail_status, restored_detail) = request_json(
        app.clone(),
        Method::GET,
        &format!(
            "/api/workspace-sessions/{session_id}/timeline-node-details/{review_node_id}"
        ),
        json!({}),
    )
    .await;
    assert_eq!(detail_status, axum::http::StatusCode::OK);
    assert_eq!(restored_detail["verdict"]["findings"], json!([]));
    assert_eq!(
        live_review["structured_output_diagnostic"],
        restored_detail["verdict"]["structured_output_diagnostic"]
    );
    assert!(restored_detail["execution_events"]
        .as_array()
        .expect("execution events")
        .iter()
        .any(|event| {
            event["event_id"] == "structured_output_repair" && event["status"] == "failed"
        }));

    restored_ws.close(None).await.ok();
}
