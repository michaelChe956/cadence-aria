#[tokio::test]
async fn item_review_pass_starts_next_outline() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls().await;
    let (app, root, _prompts) = app_with_confirmed_story_and_design_and_streaming_outputs(vec![
        valid_outline_output(),
        valid_draft_output("outline_backend_session"),
        valid_frontend_draft_output(),
    ])
    .await;
    let (session_id, plan_id, mut ws) =
        prepare_plan_accept_outline_and_select_serial_with_reviewer(&app, true).await;

    let _messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_draft_confirm"
        })
    })
    .await;
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
        &session_id,
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
            message["type"] == "artifact_update"
                && message["draft_candidate"]["draft_record"]["outline_id"]
                    == "outline_frontend_expiry"
        })
    })
    .await;
    assert!(
        messages.iter().any(|message| {
            message["type"] == "artifact_update"
                && message["draft_candidate"]["draft_record"]["outline_id"]
                    == "outline_frontend_expiry"
        }),
        "item review pass should generate next outline draft, got {messages:?}"
    );

    ws.close(None).await.ok();
}

#[tokio::test]
async fn item_review_revise_rewrites_only_current_item() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls().await;
    let (app, root, _prompts) = app_with_confirmed_story_and_design_and_streaming_outputs(vec![
        valid_outline_output(),
        valid_draft_output("outline_backend_session"),
        valid_draft_output_with_title(
            "outline_backend_session",
            "Reviewer 返修后的后端登录会话 API",
        ),
    ])
    .await;
    let (session_id, plan_id, mut ws) =
        prepare_plan_accept_outline_and_select_serial_with_reviewer(&app, true).await;

    let _messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_draft_confirm"
        })
    })
    .await;
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
        &session_id,
        item_review_revise("outline_backend_session", &draft_id),
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
            message["type"] == "artifact_update"
                && message["draft_candidate"]["draft_record"]["candidate"]["title"]
                    == "Reviewer 返修后的后端登录会话 API"
        })
    })
    .await;
    assert!(
        messages.iter().any(|message| {
            message["type"] == "artifact_update"
                && message["draft_candidate"]["draft_record"]["candidate"]["title"]
                    == "Reviewer 返修后的后端登录会话 API"
        }),
        "item review revise should regenerate current outline, got {messages:?}"
    );

    ws.close(None).await.ok();
}

#[tokio::test]
async fn item_review_plan_reopen_marks_outline_revising() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls().await;
    let (app, root, _prompts) = app_with_confirmed_story_and_design_and_streaming_outputs(vec![
        valid_outline_output(),
        valid_draft_output("outline_backend_session"),
    ])
    .await;
    let (session_id, plan_id, mut ws) =
        prepare_plan_accept_outline_and_select_serial_with_reviewer(&app, true).await;

    let _messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_draft_confirm"
        })
    })
    .await;
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
        &session_id,
        item_review_plan_reopen("outline_backend_session", &draft_id),
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
        messages
            .iter()
            .any(|message| message["type"] == "stage_change" && message["stage"] == "human_confirm")
    })
    .await;
    assert!(
        messages
            .iter()
            .any(|message| message["type"] == "stage_change" && message["stage"] == "human_confirm"),
        "plan reopen should enter human confirm, got {messages:?}"
    );

    let index = WorkItemPlanStore::new(ProductAppPaths::new(root.path().join(".aria")))
        .load_active_index("project_0001", "issue_0001", &plan_id)
        .expect("load active index")
        .expect("active index");
    assert_eq!(index.outline_state, "revising");
    assert_eq!(index.active_outline_id, None);

    ws.close(None).await.ok();
}

#[tokio::test]
async fn plan_reopen_required_supersedes_drafts_and_reopens_outline() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls().await;
    let (app, root, _prompts) = app_with_confirmed_story_and_design_and_streaming_outputs(vec![
        valid_outline_output(),
        valid_draft_output("outline_backend_session"),
        valid_frontend_draft_output(),
        valid_integration_draft_output(),
    ])
    .await;
    let (session_id, plan_id, mut ws) =
        prepare_plan_accept_outline_and_select_serial_with_reviewer(&app, true).await;

    let store = WorkItemPlanStore::new(ProductAppPaths::new(root.path().join(".aria")));

    let _messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_draft_confirm"
        })
    })
    .await;
    let index = store
        .load_active_index("project_0001", "issue_0001", &plan_id)
        .expect("load active index after backend draft")
        .expect("active index after backend draft");
    let backend_draft_id = index
        .outline_to_current_draft_id
        .get("outline_backend_session")
        .expect("backend draft id")
        .clone();
    enable_work_item_plan_review_fixture(
        &app,
        &session_id,
        item_review_pass("outline_backend_session", &backend_draft_id),
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
    .expect("send backend draft accept");

    let _messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "artifact_update"
                && message["draft_candidate"]["draft_record"]["outline_id"]
                    == "outline_frontend_expiry"
        })
    })
    .await;
    let index = store
        .load_active_index("project_0001", "issue_0001", &plan_id)
        .expect("load active index after frontend draft")
        .expect("active index after frontend draft");
    let frontend_draft_id = index
        .outline_to_current_draft_id
        .get("outline_frontend_expiry")
        .expect("frontend draft id")
        .clone();
    enable_work_item_plan_review_fixture(
        &app,
        &session_id,
        item_review_pass("outline_frontend_expiry", &frontend_draft_id),
    )
    .await;
    ws.send(Message::Text(
        json!({
            "type": "work_item_draft_decision",
            "outline_id": "outline_frontend_expiry",
            "decision": "accept",
            "feedback": null
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send frontend draft accept");

    let _messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "artifact_update"
                && message["draft_candidate"]["draft_record"]["outline_id"]
                    == "outline_integration_session"
        })
    })
    .await;
    let index = store
        .load_active_index("project_0001", "issue_0001", &plan_id)
        .expect("load active index after integration draft")
        .expect("active index after integration draft");
    let integration_draft_id = index
        .outline_to_current_draft_id
        .get("outline_integration_session")
        .expect("integration draft id")
        .clone();
    enable_work_item_plan_review_fixture(
        &app,
        &session_id,
        item_review_plan_reopen("outline_frontend_expiry", &integration_draft_id),
    )
    .await;
    ws.send(Message::Text(
        json!({
            "type": "work_item_draft_decision",
            "outline_id": "outline_integration_session",
            "decision": "accept",
            "feedback": null
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send integration draft accept");

    let messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages
            .iter()
            .any(|message| message["type"] == "stage_change" && message["stage"] == "human_confirm")
    })
    .await;
    assert!(
        messages
            .iter()
            .any(|message| message["type"] == "stage_change" && message["stage"] == "human_confirm"),
        "plan reopen should pause for human confirm, got {messages:?}"
    );

    let index = store
        .load_active_index("project_0001", "issue_0001", &plan_id)
        .expect("load active index after plan reopen")
        .expect("active index after plan reopen");
    assert_eq!(index.outline_state, "revising");
    assert_eq!(index.active_outline_id, None);
    assert!(index.outline_to_current_draft_id.is_empty());
    for draft_id in [&backend_draft_id, &frontend_draft_id, &integration_draft_id] {
        assert_eq!(
            index.draft_statuses.get(draft_id),
            Some(&WorkItemDraftStatus::Superseded),
            "draft {draft_id} should be superseded after plan reopen"
        );
    }

    let mut drafts = store
        .list_draft_records("project_0001", "issue_0001", &plan_id)
        .expect("list draft history after plan reopen");
    drafts.sort_by(|left, right| left.draft_id.cmp(&right.draft_id));
    assert_eq!(drafts.len(), 3);
    for draft in drafts {
        assert_eq!(draft.status, WorkItemDraftStatus::Superseded);
        assert!(!draft.active);
        assert_eq!(
            draft.supersede_reason,
            Some(WorkItemDraftSupersedeReason::OutlineRevised)
        );
        assert!(
            !draft.candidate.title.is_empty(),
            "superseded draft history should remain readable"
        );
    }

    ws.close(None).await.ok();
}

#[tokio::test]
async fn item_review_revise_affecting_previous_item_downgrades_to_needs_human() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls().await;
    let (app, root, _prompts) = app_with_confirmed_story_and_design_and_streaming_outputs(vec![
        valid_outline_output(),
        valid_draft_output("outline_backend_session"),
        valid_frontend_draft_output(),
    ])
    .await;
    let (session_id, plan_id, mut ws) =
        prepare_plan_accept_outline_and_select_serial_with_reviewer(&app, true).await;

    let _messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_draft_confirm"
        })
    })
    .await;
    let store = WorkItemPlanStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let index = store
        .load_active_index("project_0001", "issue_0001", &plan_id)
        .expect("load active index")
        .expect("active index");
    let backend_draft_id = index
        .outline_to_current_draft_id
        .get("outline_backend_session")
        .expect("active backend draft")
        .clone();
    enable_work_item_plan_review_fixture(
        &app,
        &session_id,
        item_review_pass("outline_backend_session", &backend_draft_id),
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
    .expect("send backend draft accept");

    let _messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "artifact_update"
                && message["draft_candidate"]["draft_record"]["outline_id"]
                    == "outline_frontend_expiry"
        }) && messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_draft_confirm"
        })
    })
    .await;

    let index = store
        .load_active_index("project_0001", "issue_0001", &plan_id)
        .expect("load active index")
        .expect("active index");
    let frontend_draft_id = index
        .outline_to_current_draft_id
        .get("outline_frontend_expiry")
        .expect("active frontend draft")
        .clone();
    enable_work_item_plan_review_fixture(
        &app,
        &session_id,
        item_review_revise("outline_backend_session", &frontend_draft_id),
    )
    .await;

    ws.send(Message::Text(
        json!({
            "type": "work_item_draft_decision",
            "outline_id": "outline_frontend_expiry",
            "decision": "accept",
            "feedback": null
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send frontend draft accept");

    let messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages
            .iter()
            .any(|message| message["type"] == "stage_change" && message["stage"] == "human_confirm")
    })
    .await;
    assert!(
        messages
            .iter()
            .any(|message| message["type"] == "stage_change" && message["stage"] == "human_confirm"),
        "revise targeting previous item should require human confirm, got {messages:?}"
    );

    ws.close(None).await.ok();
}

#[tokio::test]
async fn draft_accept_marks_record_accepted() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls().await;
    let (app, root, _prompts) = app_with_confirmed_story_and_design_and_streaming_outputs(vec![
        valid_outline_output(),
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

    let _messages = recv_ws_until(&mut ws, Duration::from_secs(5), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_updated" && message["status"] == "completed"
        })
    })
    .await;

    let index = WorkItemPlanStore::new(ProductAppPaths::new(root.path().join(".aria")))
        .load_active_index("project_0001", "issue_0001", &plan_id)
        .expect("load active index")
        .expect("active index");
    let draft_id = index
        .outline_to_current_draft_id
        .get("outline_backend_session")
        .expect("active backend draft");
    assert_eq!(
        index.draft_statuses.get(draft_id),
        Some(&cadence_aria::product::models::WorkItemDraftStatus::Accepted)
    );

    ws.close(None).await.ok();
}

#[tokio::test]
async fn draft_rewrite_supersedes_old_draft_and_regenerates_current_outline() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls().await;
    let (app, root, prompts) = app_with_confirmed_story_and_design_and_streaming_outputs(vec![
        valid_outline_output(),
        valid_draft_output("outline_backend_session"),
        valid_draft_output_with_title("outline_backend_session", "重写后的后端登录会话 API"),
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

    ws.send(Message::Text(
        json!({
            "type": "work_item_draft_decision",
            "outline_id": "outline_backend_session",
            "decision": "rewrite",
            "feedback": "请收窄后端会话 API 的范围"
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send draft rewrite");

    let _messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "artifact_update"
                && message["draft_candidate"]["draft_record"]["candidate"]["title"]
                    == "重写后的后端登录会话 API"
        })
    })
    .await;

    let draft_store = WorkItemPlanStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let mut drafts = draft_store
        .list_draft_records("project_0001", "issue_0001", &plan_id)
        .expect("list drafts");
    drafts.sort_by(|left, right| left.draft_id.cmp(&right.draft_id));
    assert_eq!(drafts.len(), 2);
    let old = &drafts[0];
    let new = &drafts[1];
    assert_eq!(old.status, WorkItemDraftStatus::Superseded);
    assert!(!old.active);
    assert_eq!(
        old.superseded_by_draft_id.as_deref(),
        Some(new.draft_id.as_str())
    );
    assert_eq!(
        old.supersede_reason,
        Some(WorkItemDraftSupersedeReason::DirectRewrite)
    );
    assert!(old.superseded_at.is_some());
    assert_eq!(new.status, WorkItemDraftStatus::Draft);
    assert!(new.active);
    assert_eq!(new.attempt_index, old.attempt_index + 1);
    let (has_feedback_prompt, captured_prompts) = {
        let captured_prompts = prompts.lock().expect("captured prompts lock");
        (
            captured_prompts.iter().any(|prompt| {
                prompt.contains("[user_or_reviewer_feedback]")
                    && prompt.contains("请收窄后端会话 API 的范围")
            }),
            captured_prompts.clone(),
        )
    };
    assert!(
        has_feedback_prompt,
        "draft rewrite prompt should include user feedback, got {captured_prompts:?}"
    );

    let index = draft_store
        .load_active_index("project_0001", "issue_0001", &plan_id)
        .expect("load active index")
        .expect("active index");
    assert_eq!(
        index
            .outline_to_current_draft_id
            .get("outline_backend_session"),
        Some(&new.draft_id)
    );
    assert_eq!(
        index.draft_statuses.get(&old.draft_id),
        Some(&WorkItemDraftStatus::Superseded)
    );

    ws.close(None).await.ok();
}

#[tokio::test]
async fn draft_pause_enters_human_confirm_without_regenerating() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls().await;
    let (app, root, _prompts) = app_with_confirmed_story_and_design_and_streaming_outputs(vec![
        valid_outline_output(),
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
    .expect("send draft pause");

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
        "pause should enter human_confirm, got {messages:?}"
    );

    let drafts = WorkItemPlanStore::new(ProductAppPaths::new(root.path().join(".aria")))
        .list_draft_records("project_0001", "issue_0001", &plan_id)
        .expect("list drafts");
    assert_eq!(drafts.len(), 1);

    ws.close(None).await.ok();
}

fn valid_draft_output(outline_id: &str) -> Value {
    valid_draft_output_with_title(outline_id, "实现后端登录会话 API")
}

