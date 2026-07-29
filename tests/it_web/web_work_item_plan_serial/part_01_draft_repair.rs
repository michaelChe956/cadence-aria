#[tokio::test]
async fn serial_local_validation_failure_repairs_once_with_findings() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls().await;
    let (app, root, prompts) = app_with_confirmed_story_and_design_and_streaming_outputs(vec![
        valid_outline_output(),
        invalid_draft_output_missing_scope("outline_backend_session"),
        valid_draft_output("outline_backend_session"),
    ])
    .await;
    let (_session_id, plan_id, mut ws) = prepare_plan_accept_outline_and_select_serial(&app).await;

    let _messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_draft_confirm"
        })
    })
    .await;

    {
        let captured_prompts = prompts.lock().unwrap();
        assert_eq!(captured_prompts.len(), 3, "outline author + failed draft + repair");
        assert!(captured_prompts[2].contains("[draft_validation_findings]"));
        assert!(captured_prompts[2].contains(
            "write_scope_required: draft outline_backend_session must include at least one canonical exclusive write scope"
        ));
    }

    let drafts = WorkItemPlanStore::new(ProductAppPaths::new(root.path().join(".aria")))
        .list_draft_records("project_0001", "issue_0001", &plan_id)
        .expect("list drafts");
    assert_eq!(drafts.len(), 1, "initial invalid candidate must not persist");
    assert_eq!(
        drafts[0].status,
        WorkItemDraftStatus::Draft,
        "repaired draft: {:#?}",
        drafts[0]
    );
    let diagnostics = serde_json::to_value(&drafts[0]).expect("serialize draft")
        ["generation_diagnostics"]
        .clone();
    assert_eq!(diagnostics["auto_repair_attempted"], true);
    assert!(!diagnostics["initial_validation_findings"]
        .as_array()
        .expect("initial findings")
        .is_empty());

    ws.close(None).await.ok();
}

#[tokio::test]
async fn serial_second_local_validation_failure_stops_at_confirm() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls().await;
    let (app, root, prompts) = app_with_confirmed_story_and_design_and_streaming_outputs(vec![
        valid_outline_output(),
        invalid_draft_output_missing_scope("outline_backend_session"),
        invalid_draft_output_missing_scope("outline_backend_session"),
    ])
    .await;
    let (_session_id, plan_id, mut ws) = prepare_plan_accept_outline_and_select_serial(&app).await;

    let messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_draft_confirm"
        })
    })
    .await;
    let artifact = messages
        .iter()
        .rfind(|message| {
            message["type"] == "artifact_update" && message.get("draft_candidate").is_some()
        })
        .expect("failed draft artifact");
    assert_eq!(artifact["draft_candidate"]["can_accept"], false);

    assert_eq!(prompts.lock().unwrap().len(), 3, "must not auto-retry a third time");
    let drafts = WorkItemPlanStore::new(ProductAppPaths::new(root.path().join(".aria")))
        .list_draft_records("project_0001", "issue_0001", &plan_id)
        .expect("list drafts");
    assert_eq!(drafts.len(), 1, "only final invalid candidate should persist");
    assert_eq!(drafts[0].status, WorkItemDraftStatus::ValidationFailed);
    let diagnostics = serde_json::to_value(&drafts[0]).expect("serialize draft")
        ["generation_diagnostics"]
        .clone();
    assert_eq!(diagnostics["auto_repair_attempted"], true);
    assert!(!diagnostics["final_validation_findings"]
        .as_array()
        .expect("final findings")
        .is_empty());

    ws.send(Message::Text(
        json!({
            "type": "work_item_draft_decision",
            "outline_id": "outline_backend_session",
            "decision": "pause",
            "feedback": null
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("pause after the exhausted automatic repair");
    let messages = recv_ws_until(&mut ws, Duration::from_secs(5), |messages| {
        messages
            .iter()
            .any(|message| message["type"] == "stage_change" && message["stage"] == "human_confirm")
    })
    .await;
    assert!(
        messages
            .iter()
            .any(|message| message["type"] == "stage_change" && message["stage"] == "human_confirm"),
        "pause after a second validation failure must enter human confirmation, got {messages:?}"
    );
    assert_eq!(
        prompts.lock().unwrap().len(),
        3,
        "manual pause must not invoke the Provider after the one automatic repair"
    );

    ws.close(None).await.ok();
}

#[tokio::test]
async fn manual_rewrite_merges_validation_findings_and_user_feedback() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls().await;
    let (app, _root, prompts) = app_with_confirmed_story_and_design_and_streaming_outputs(vec![
        valid_outline_output(),
        invalid_draft_output_missing_scope("outline_backend_session"),
        invalid_draft_output_missing_scope("outline_backend_session"),
        invalid_draft_output_missing_scope("outline_backend_session"),
        valid_draft_output("outline_backend_session"),
    ])
    .await;
    let (_session_id, _plan_id, mut ws) = prepare_plan_accept_outline_and_select_serial(&app).await;

    let messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_draft_confirm"
        })
    })
    .await;
    let findings = messages
        .iter()
        .rfind(|message| {
            message["type"] == "artifact_update" && message.get("draft_candidate").is_some()
        })
        .expect("failed draft artifact")["draft_candidate"]["validator_findings"]
        .as_array()
        .expect("validator findings")
        .clone();
    let finding = findings.first().expect("validation finding");
    let code = finding["code"].as_str().expect("finding code").to_string();
    let message = finding["message"]
        .as_str()
        .expect("finding message")
        .to_string();

    ws.send(Message::Text(
        json!({
            "type": "work_item_draft_decision",
            "outline_id": "outline_backend_session",
            "decision": "rewrite",
            "feedback": "补充：保留接口兼容"
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send rewrite");
    let _messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().filter(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_draft_confirm"
        }).count() >= 2
    })
    .await;

    {
        let captured_prompts = prompts.lock().unwrap();
        assert_eq!(captured_prompts.len(), 5);
        let rewrite_prompt = &captured_prompts[3];
        assert!(rewrite_prompt.contains("补充：保留接口兼容"));
        assert!(rewrite_prompt.contains("[draft_validation_findings]"));
        assert!(rewrite_prompt.contains(&format!("{code}: {message}")));
        let repair_prompt = captured_prompts.last().expect("rewrite repair prompt");
        assert!(repair_prompt.contains("补充：保留接口兼容"));
        assert!(repair_prompt.contains("[draft_validation_findings]"));
        assert!(repair_prompt.contains(&format!("{code}: {message}")));
        assert_eq!(
            repair_prompt.matches("[draft_validation_findings]").count(),
            1,
            "automatic repair must not carry findings from the manual rewrite prompt"
        );
        assert_eq!(
            repair_prompt.matches(&format!("{code}: {message}")).count(),
            1,
            "automatic repair must include only the current findings"
        );
    }

    ws.close(None).await.ok();
}

#[tokio::test]
async fn accepted_draft_enters_item_review_when_reviewer_enabled() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls().await;
    let (app, root, _prompts) = app_with_confirmed_story_and_design_and_streaming_outputs(vec![
        valid_outline_output(),
        valid_draft_output("outline_backend_session"),
        valid_frontend_draft_output(),
    ])
    .await;
    let (_session_id, plan_id, mut ws) =
        prepare_plan_accept_outline_and_select_serial_with_reviewer(&app, true).await;

    let messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_draft_confirm"
        })
    })
    .await;
    assert!(
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_draft_confirm"
        }),
        "serial draft should reach confirm before item review test continues, got {messages:?}"
    );
    let index = WorkItemPlanStore::new(ProductAppPaths::new(root.path().join(".aria")))
        .load_active_index("project_0001", "issue_0001", &plan_id)
        .expect("load active index")
        .expect("active index");
    let draft_id = index
        .outline_to_current_draft_id
        .get("outline_backend_session")
        .expect("active backend draft")
        .clone();
    enable_work_item_plan_review_fixture(
        &app,
        &_session_id,
        item_review_pass("outline_backend_session", &draft_id),
    )
    .await;

    ws.send(Message::Text(
        json!({
            "type": "work_item_draft_decision",
            "outline_id": "outline_backend_session",
            "decision": "accept",
            "feedback": null
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send draft accept");

    let messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_draft_review"
        })
    })
    .await;
    assert!(
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_draft_review"
        }),
        "accept should enter item review, got {messages:?}"
    );

    ws.close(None).await.ok();
}

#[tokio::test]
async fn author_decision_is_rejected_on_draft_confirm_node() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls().await;
    let (app, _root, _prompts) = app_with_confirmed_story_and_design_and_streaming_outputs(vec![
        valid_outline_output(),
        valid_draft_output("outline_backend_session"),
    ])
    .await;
    let (_session_id, _plan_id, mut ws) =
        prepare_plan_accept_outline_and_select_serial_with_reviewer(&app, true).await;

    let _messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_draft_confirm"
        })
    })
    .await;

    ws.send(Message::Text(
        json!({ "type": "author_decision", "decision": "accept" })
            .to_string()
            .into(),
    ))
    .await
    .expect("send invalid author decision");

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
    assert_eq!(protocol_error["code"], "INVALID_AUTHOR_DECISION");
    assert!(
        !messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "reviewer_run"
        }),
        "generic reviewer_run must not be created from draft confirm: {messages:?}"
    );

    ws.close(None).await.ok();
}

#[tokio::test]
async fn serial_oversized_feedback_fails_draft_run_node() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls().await;
    let (app, _root, _prompts) = app_with_confirmed_story_and_design_and_streaming_outputs(vec![
        valid_outline_output(),
        valid_draft_output("outline_backend_session"),
    ])
    .await;
    let (_session_id, _plan_id, mut ws) = prepare_plan_accept_outline_and_select_serial(&app).await;

    let _messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_draft_confirm"
        })
    })
    .await;

    // 70KB feedback 使 prompt 超过 64KB 硬兜底，构建必然 fail-closed。
    let oversized = "超".repeat(24_000);
    ws.send(Message::Text(
        json!({
            "type": "work_item_draft_decision",
            "outline_id": "outline_backend_session",
            "decision": "rewrite",
            "feedback": oversized
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send oversized rewrite");

    // `timeline_node_updated` 不带完整 node，仅含 node_id/status，
    // 需与先前 `timeline_node_created` 的 work_item_draft_run 节点关联。
    let draft_run_failed = |messages: &[Value]| {
        let draft_run_node_id = messages
            .iter()
            .rev()
            .find(|message| {
                message["type"] == "timeline_node_created"
                    && message["node"]["node_type"] == "work_item_draft_run"
            })
            .and_then(|message| message["node"]["node_id"].as_str());
        let Some(draft_run_node_id) = draft_run_node_id else {
            return false;
        };
        messages.iter().any(|message| {
            message["type"] == "timeline_node_updated"
                && message["node_id"].as_str() == Some(draft_run_node_id)
                && message["status"] == "failed"
        })
    };

    let messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        draft_run_failed(messages)
    })
    .await;
    assert!(
        draft_run_failed(&messages),
        "draft run node must transition to failed instead of hanging active: {messages:#?}"
    );
    assert!(
        messages.iter().any(|message| {
            message["type"] == "error"
                && message["message"]
                    .as_str()
                    .is_some_and(|text| text.contains("provider-context"))
        }),
        "client must receive the prompt-too-large error: {messages:#?}"
    );

    ws.close(None).await.ok();
}
