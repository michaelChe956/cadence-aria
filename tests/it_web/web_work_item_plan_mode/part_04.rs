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
    let revision_prompt = &prompts[1];
    assert!(revision_prompt.contains("增量返修"));
    assert!(revision_prompt.contains("[revision_feedback]"));
    assert!(!revision_prompt.contains("[confirmed_story_specs]"));
    assert!(!revision_prompt.contains("[repository_structure_summary]"));
    ws.close(None).await.ok();
}

async fn persist_outline_revision_before_provider_spawn() -> (tempfile::TempDir, String, String) {
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

    (root, session_id, run_node_id)
}

async fn persist_dedicated_request_outline_revision_before_provider_spawn(
) -> (tempfile::TempDir, String, String) {
    let (app, root, _initial_prompts) =
        app_with_confirmed_story_and_design_and_streaming_raw_outputs(vec![
            QueuedSplitOutput::Json(valid_outline_output()),
        ])
        .await;
    let (session_id, _plan_id, mut ws) = prepare_plan_and_start(&app, false).await;
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
    let feedback = engine
        .request_work_item_plan_outline_revision(Some(
            "用户 context：补齐 API 影响闭环与 tests/it_core owner".to_string(),
        ))
        .await
        .expect("persist dedicated request outline revision")
        .expect("complete persisted revision feedback");
    assert!(feedback.contains("用户 context：补齐 API 影响闭环"));
    let run_node_id = engine
        .active_timeline_node_id()
        .expect("persisted active outline revision run");
    let run_detail = lifecycle
        .load_node_detail(&session_id, &run_node_id)
        .expect("persisted dedicated request run detail");
    assert!(run_detail.is_revision);
    assert!(run_detail
        .revision_feedback
        .as_deref()
        .expect("persisted dedicated request feedback")
        .contains("tests/it_core owner"));
    drop(engine);
    drop(app);

    (root, session_id, run_node_id)
}

fn persisted_run_detail_path(
    root: &tempfile::TempDir,
    session_id: &str,
    run_node_id: &str,
) -> std::path::PathBuf {
    ProductAppPaths::new(root.path().join(".aria"))
        .issue_lifecycle_root("project_0001", "issue_0001")
        .join("workspace-timelines")
        .join(session_id)
        .join("timeline_node_details")
        .join(format!("{run_node_id}.json"))
}

fn outline_revision_journal_path(
    root: &tempfile::TempDir,
    session_id: &str,
) -> std::path::PathBuf {
    ProductAppPaths::new(root.path().join(".aria"))
        .issue_lifecycle_root("project_0001", "issue_0001")
        .join("workspace-transactions")
        .join(session_id)
        .join("work_item_plan_outline_revision.json")
}

#[tokio::test]
async fn corrupt_outline_review_active_index_fails_closed_before_reviewer_start() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls().await;
    let (app, root, _prompts) =
        app_with_confirmed_story_and_design_and_streaming_raw_outputs(vec![
            QueuedSplitOutput::Json(valid_outline_output()),
        ])
        .await;
    let (session_id, plan_id, mut ws) = prepare_plan_and_start(&app, true).await;
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
    let (event_tx, mut event_rx) = tokio::sync::mpsc::channel(16);
    let mut session =
        cadence_aria::product::workspace_engine::WorkspaceSession::from_record(session_record);
    session.repository_path = Some(root.path().join("repo"));
    let mut engine = cadence_aria::product::workspace_engine::WorkspaceEngine::new_persistent(
        checkpoint_store,
        lifecycle,
        event_tx,
        session,
    );
    engine.begin_work_item_plan_outline_review_run().await;

    let active_index_path = app_paths
        .issue_lifecycle_root("project_0001", "issue_0001")
        .join("work_item_plan_drafts")
        .join(&plan_id)
        .join("active_index.json");
    std::fs::create_dir_all(active_index_path.parent().expect("active index parent"))
        .expect("create active index parent");
    std::fs::write(active_index_path, "{corrupt active index")
        .expect("corrupt active index");

    let reviewer_starts =
        std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let (_command_tx, command_rx) = tokio::sync::mpsc::channel(1);
    engine
        .drive_review_session(
            std::sync::Arc::new(CountedReviewerStreamingProvider::new(
                reviewer_starts.clone(),
            )),
            command_rx,
        )
        .await;

    let mut error_message = None;
    let mut provider_prompt_count = 0;
    while let Ok(event) = event_rx.try_recv() {
        match event {
            cadence_aria::product::workspace_engine::EngineEvent::Error { message } => {
                error_message = Some(message);
            }
            cadence_aria::product::workspace_engine::EngineEvent::ExecutionEvent {
                event,
                ..
            } if event.title == "Provider Prompt" => provider_prompt_count += 1,
            _ => {}
        }
    }
    let error_message = error_message.expect("corrupt active index must emit drive error");
    assert!(error_message.contains("load work item plan active index failed"));
    assert!(error_message.contains("product_store_json"));
    assert_eq!(provider_prompt_count, 0);
    assert_eq!(
        reviewer_starts.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "corrupt active index must fail before reviewer provider start"
    );
}

#[tokio::test]
async fn persisted_outline_revision_intent_resumes_incremental_prompt_on_new_app_websocket() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls().await;
    let (root, session_id, run_node_id) = persist_outline_revision_before_provider_spawn().await;

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

#[tokio::test]
async fn dedicated_request_outline_revision_resumes_incremental_prompt_on_new_app_websocket() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls().await;
    let (root, session_id, run_node_id) =
        persist_dedicated_request_outline_revision_before_provider_spawn().await;

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
    assert_eq!(prompts.len(), 1, "reconnect must resume one dedicated request");
    let prompt = &prompts[0];
    assert!(prompt.contains("用户 context：补齐 API 影响闭环"));
    assert!(prompt.contains("tests/it_core owner"));
    assert!(!prompt.contains("[confirmed_story_specs]"));
    assert!(!prompt.contains("[repository_structure_summary]"));
    reconnected_ws.close(None).await.ok();
}

#[tokio::test]
async fn corrupt_outline_resume_detail_fails_closed_without_starting_provider() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls().await;
    let (root, session_id, run_node_id) = persist_outline_revision_before_provider_spawn().await;
    std::fs::write(
        persisted_run_detail_path(&root, &session_id, &run_node_id),
        "{corrupt node detail",
    )
    .expect("corrupt persisted run detail");

    let (restarted_app, restarted_prompts) = app_for_existing_root_with_streaming_raw_outputs(
        root.path(),
        vec![QueuedSplitOutput::Pending],
    );
    let mut reconnected_ws = connect_ws(restarted_app, &session_id).await;
    let messages = recv_ws_until(
        &mut reconnected_ws,
        Duration::from_secs(5),
        |messages| {
            messages.iter().any(|message| {
                message["type"] == "error"
                    || (message["type"] == "execution_event"
                        && message["event"]["title"] == "Provider Prompt")
            })
        },
    )
    .await;
    let error = messages
        .iter()
        .find(|message| message["type"] == "error")
        .expect("corrupt detail must emit websocket error");
    assert!(error["message"]
        .as_str()
        .expect("websocket error message")
        .contains("resume outline run detail failed"));
    assert!(messages.iter().all(|message| {
        !(message["type"] == "execution_event"
            && message["event"]["title"] == "Provider Prompt")
    }));
    assert!(restarted_prompts
        .lock()
        .expect("captured prompts lock")
        .is_empty());
    reconnected_ws.close(None).await.ok();
}

#[tokio::test]
async fn corrupt_outline_revision_journal_fails_closed_without_starting_provider() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls().await;
    let (root, session_id, _run_node_id) = persist_outline_revision_before_provider_spawn().await;
    let journal_path = outline_revision_journal_path(&root, &session_id);
    std::fs::create_dir_all(journal_path.parent().expect("journal parent"))
        .expect("create journal parent");
    std::fs::write(&journal_path, "{corrupt outline revision journal")
        .expect("corrupt outline revision journal");

    let (restarted_app, restarted_prompts) = app_for_existing_root_with_streaming_raw_outputs(
        root.path(),
        vec![QueuedSplitOutput::Pending],
    );
    let mut reconnected_ws = connect_ws(restarted_app, &session_id).await;
    let messages = recv_ws_until(
        &mut reconnected_ws,
        Duration::from_secs(5),
        |messages| messages.iter().any(|message| message["type"] == "error"),
    )
    .await;
    let error = messages
        .iter()
        .find(|message| message["type"] == "error")
        .expect("corrupt journal must emit websocket error");
    assert!(error["message"]
        .as_str()
        .expect("websocket error message")
        .contains("outline revision recovery failed"));
    assert!(messages.iter().all(|message| {
        !(message["type"] == "execution_event"
            && message["event"]["title"] == "Provider Prompt")
    }));
    assert!(restarted_prompts
        .lock()
        .expect("captured prompts lock")
        .is_empty());
    reconnected_ws.close(None).await.ok();
}

#[tokio::test]
async fn missing_legacy_outline_resume_detail_falls_back_to_initial_author_prompt() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls().await;
    let (root, session_id, run_node_id) = persist_outline_revision_before_provider_spawn().await;
    std::fs::remove_file(persisted_run_detail_path(
        &root,
        &session_id,
        &run_node_id,
    ))
    .expect("remove legacy-missing run detail");

    let (restarted_app, restarted_prompts) = app_for_existing_root_with_streaming_raw_outputs(
        root.path(),
        vec![QueuedSplitOutput::Pending],
    );
    let mut reconnected_ws = connect_ws(restarted_app, &session_id).await;
    let messages = recv_ws_until(
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
    assert!(messages.iter().any(|message| {
        message["type"] == "execution_event" && message["event"]["title"] == "Provider Prompt"
    }));
    let prompts = wait_for_recorded_prompts(&restarted_prompts, 1).await;
    assert_eq!(prompts.len(), 1, "legacy missing detail resumes once");
    assert!(prompts[0].contains("[confirmed_story_specs]"));
    assert!(prompts[0].contains("[repository_structure_summary]"));
    reconnected_ws.close(None).await.ok();
}
