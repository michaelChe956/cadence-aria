#[tokio::test]
async fn batch_accept_skips_review_when_reviewer_disabled() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls().await;
    let (app, root, _prompts) = app_with_confirmed_story_and_design_and_streaming_outputs(vec![
        valid_outline_output(),
        valid_draft_output("outline_backend_session"),
        valid_frontend_draft_output(),
        valid_integration_draft_output(),
    ])
    .await;
    let (_session_id, plan_id, mut ws) = prepare_plan_accept_outline_and_select_batch(&app).await;

    let _messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_batch_confirm"
        })
    })
    .await;
    ws.send(Message::Text(
        json!({
            "type": "work_item_batch_decision",
            "decision": "accept_all",
            "feedback": null,
            "first_affected_outline_id": null
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send batch accept all");

    let messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages
            .iter()
            .any(|message| message["type"] == "stage_change" && message["stage"] == "human_confirm")
    })
    .await;
    assert!(
        !messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_batch_review"
        }),
        "reviewer disabled should skip batch review, got {messages:?}"
    );

    let index = WorkItemPlanStore::new(ProductAppPaths::new(root.path().join(".aria")))
        .load_active_index("project_0001", "issue_0001", &plan_id)
        .expect("load active index")
        .expect("active index");
    let batch = index.batches.last().expect("batch record");
    assert_eq!(batch.status, WorkItemBatchStatus::ReviewDone);

    ws.close(None).await.ok();
}

#[tokio::test]
async fn batch_review_revise_batch_returns_batch_confirm() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls().await;
    let (app, _root, _prompts) = app_with_confirmed_story_and_design_and_streaming_outputs(vec![
        valid_outline_output(),
        valid_draft_output("outline_backend_session"),
        valid_frontend_draft_output(),
        valid_integration_draft_output(),
    ])
    .await;
    let (session_id, _plan_id, mut ws) =
        prepare_plan_accept_outline_and_select_batch_with_reviewer(&app, true).await;

    let _messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_batch_confirm"
        })
    })
    .await;
    enable_work_item_plan_review_fixture(&app, &session_id, batch_review_revise()).await;

    ws.send(Message::Text(
        json!({
            "type": "work_item_batch_decision",
            "decision": "accept_all",
            "feedback": null,
            "first_affected_outline_id": null
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send batch accept all");

    let messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_batch_confirm"
        })
    })
    .await;
    assert!(
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_batch_confirm"
        }),
        "revise_batch should return to batch confirm, got {messages:?}"
    );

    ws.close(None).await.ok();
}

#[tokio::test]
async fn batch_review_plan_reopen_supersedes_drafts_and_sets_outline_revising() {
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
        prepare_plan_accept_outline_and_select_batch_with_reviewer(&app, true).await;

    let _messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_batch_confirm"
        })
    })
    .await;
    enable_work_item_plan_review_fixture(&app, &session_id, batch_review_plan_reopen()).await;

    ws.send(Message::Text(
        json!({
            "type": "work_item_batch_decision",
            "decision": "accept_all",
            "feedback": null,
            "first_affected_outline_id": null
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send batch accept all");

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
        "plan_reopen_required should pause in human confirm, got {messages:?}"
    );

    let store = WorkItemPlanStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let index = store
        .load_active_index("project_0001", "issue_0001", &plan_id)
        .expect("load active index")
        .expect("active index");
    assert_eq!(index.outline_state, "revising");
    assert_eq!(index.active_outline_id, None);
    assert!(index.draft_statuses.values().all(|status| {
        status == &cadence_aria::product::models::WorkItemDraftStatus::Superseded
    }));

    let drafts = store
        .list_draft_records("project_0001", "issue_0001", &plan_id)
        .expect("list drafts");
    assert!(drafts.iter().all(|draft| {
        draft.status == cadence_aria::product::models::WorkItemDraftStatus::Superseded
            && !draft.active
            && draft.supersede_reason
                == Some(cadence_aria::product::models::WorkItemDraftSupersedeReason::OutlineRevised)
    }));

    ws.close(None).await.ok();
}

#[tokio::test]
async fn batch_confirm_rewrite_batch_supersedes_current_batch_drafts() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls().await;
    let (app, root, prompts) = app_with_confirmed_story_and_design_and_streaming_outputs(vec![
        valid_outline_output(),
        valid_draft_output("outline_backend_session"),
        valid_frontend_draft_output(),
        valid_integration_draft_output(),
        valid_draft_output_with_title("outline_backend_session", "重写后的后端登录会话 API"),
        valid_frontend_draft_output_with_title("重写后的前端会话过期提示"),
        valid_integration_draft_output_with_title("重写后的会话过期端到端测试"),
    ])
    .await;
    let (_session_id, plan_id, mut ws) = prepare_plan_accept_outline_and_select_batch(&app).await;

    let _messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_batch_confirm"
        })
    })
    .await;
    ws.send(Message::Text(
        json!({
            "type": "work_item_batch_decision",
            "decision": "rewrite_batch",
            "feedback": "整组拆分过粗，请重写",
            "first_affected_outline_id": null
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send batch rewrite");

    let messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages
            .iter()
            .filter(|message| {
                message["type"] == "timeline_node_created"
                    && message["node"]["node_type"] == "work_item_batch_confirm"
            })
            .count()
            >= 1
    })
    .await;
    assert!(
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_batch_confirm"
        }),
        "rewrite_batch should return to batch confirm after regeneration, got {messages:?}"
    );

    let store = WorkItemPlanStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let drafts = store
        .list_draft_records("project_0001", "issue_0001", &plan_id)
        .expect("list drafts");
    assert_eq!(drafts.len(), 6);
    assert_eq!(
        drafts
            .iter()
            .filter(|draft| draft.status
                == cadence_aria::product::models::WorkItemDraftStatus::Superseded)
            .count(),
        3
    );
    assert!(drafts.iter().any(|draft| {
        draft
            .candidate
            .canonical_contract_candidate
            .tasks[0]
            .statement
            == "重写后的后端登录会话 API"
            && draft.status == cadence_aria::product::models::WorkItemDraftStatus::Draft
    }));

    let index = store
        .load_active_index("project_0001", "issue_0001", &plan_id)
        .expect("load active index")
        .expect("active index");
    assert_eq!(index.batches.len(), 2);
    assert!(index.batches[0].item_draft_ids.iter().all(|draft_id| {
        index.draft_statuses.get(draft_id)
            == Some(&cadence_aria::product::models::WorkItemDraftStatus::Superseded)
    }));
    assert_eq!(index.batches[1].item_draft_ids.len(), 3);

    let prompt_count = prompts.lock().unwrap().len();
    assert_eq!(
        prompt_count, 7,
        "outline author + 3 initial drafts + 3 rewrite drafts"
    );

    ws.close(None).await.ok();
}

#[tokio::test]
async fn batch_oversized_review_feedback_fails_batch_run_node() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls().await;
    let (app, _root, _prompts) = app_with_confirmed_story_and_design_and_streaming_outputs(vec![
        valid_outline_output(),
        valid_draft_output("outline_backend_session"),
        valid_frontend_draft_output(),
        valid_integration_draft_output(),
    ])
    .await;
    let (session_id, _plan_id, mut ws) =
        prepare_plan_accept_outline_and_select_batch_with_reviewer(&app, true).await;

    let _messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_batch_confirm"
        })
    })
    .await;

    // 可选通过的 batch review：summary 约 63KB，保持在新 sentinel 的 64KB JSON 上限内，
    // 但 `apply_optional_findings` 会把 `format_review_feedback(verdict)` 写入
    // `pending_revision_context`，重写 batch 时叠加基础 prompt 后必然触发 64KB
    // provider-context 硬兜底 fail-closed。
    let oversized = "超".repeat(21_000);
    enable_work_item_plan_review_fixture(
        &app,
        &session_id,
        json!({
            "verdict": "pass",
            "review_scope": "batch",
            "summary": oversized,
            "generation_round_id": "round_001",
            "affects_items": [
                { "target_outline_id": "outline_backend_session" },
                { "target_outline_id": "outline_frontend_expiry" },
                { "target_outline_id": "outline_integration_session" }
            ],
            "findings": [
                {
                    "severity": "suggestion",
                    "message": "可选建议：补充边界用例",
                    "evidence": "",
                    "required_action": ""
                }
            ]
        }),
    )
    .await;

    ws.send(Message::Text(
        json!({
            "type": "work_item_batch_decision",
            "decision": "accept_all",
            "feedback": null,
            "first_affected_outline_id": null
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send batch accept all");

    let messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages
            .iter()
            .any(|message| message["type"] == "review_decision_required")
    })
    .await;
    let decision_required = messages
        .iter()
        .find(|message| message["type"] == "review_decision_required")
        .expect("review_decision_required for optional batch review");
    let options = decision_required["options"]
        .as_array()
        .expect("options array");
    assert!(
        options.contains(&json!("apply_optional_findings")),
        "optional batch review must offer apply_optional_findings, got {options:?}"
    );

    ws.send(Message::Text(
        json!({
            "type": "review_decision_response",
            "decision": "apply_optional_findings",
            "extra_context": null
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send apply_optional_findings");

    // `timeline_node_updated` 不带完整 node，仅含 node_id/status，
    // 需与先前 `timeline_node_created` 的 work_item_batch_run 节点关联。
    let batch_run_failed = |messages: &[Value]| {
        let batch_run_node_id = messages
            .iter()
            .rev()
            .find(|message| {
                message["type"] == "timeline_node_created"
                    && message["node"]["node_type"] == "work_item_batch_run"
            })
            .and_then(|message| message["node"]["node_id"].as_str());
        let Some(batch_run_node_id) = batch_run_node_id else {
            return false;
        };
        messages.iter().any(|message| {
            message["type"] == "timeline_node_updated"
                && message["node_id"].as_str() == Some(batch_run_node_id)
                && message["status"] == "failed"
        })
    };

    let messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        batch_run_failed(messages)
    })
    .await;
    assert!(
        batch_run_failed(&messages),
        "batch run node must transition to failed instead of hanging active: {messages:#?}"
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

fn valid_draft_output(outline_id: &str) -> Value {
    valid_draft_output_with_title(outline_id, "实现后端登录会话 API")
}

fn valid_draft_output_with_title(outline_id: &str, task_statement: &str) -> Value {
    let stable_title = match outline_id {
        "outline_frontend_expiry" => "实现前端会话过期提示",
        "outline_integration_session" => "集成测试：会话过期端到端",
        _ => "实现后端登录会话 API",
    };
    let mut output = valid_canonical_draft_output(outline_id, stable_title);
    output["draft"]["canonical_contract"]["tasks"][0]["statement"] = json!(task_statement);
    output
}

fn valid_frontend_draft_output() -> Value {
    valid_frontend_draft_output_with_title("实现前端会话过期提示")
}

fn valid_frontend_draft_output_with_title(title: &str) -> Value {
    valid_draft_output_with_title("outline_frontend_expiry", title)
}

fn valid_integration_draft_output() -> Value {
    valid_integration_draft_output_with_title("集成测试：会话过期端到端")
}

fn valid_integration_draft_output_with_title(title: &str) -> Value {
    valid_draft_output_with_title("outline_integration_session", title)
}

fn invalid_draft_output_missing_scope(outline_id: &str) -> Value {
    let mut output = valid_draft_output(outline_id);
    output["draft"]["canonical_contract"]["write_policy"]["exclusive_scopes"] = json!([]);
    output
}
