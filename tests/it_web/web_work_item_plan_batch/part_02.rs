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
        draft.candidate.title == "重写后的后端登录会话 API"
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

fn valid_draft_output(outline_id: &str) -> Value {
    valid_draft_output_with_title(outline_id, "实现后端登录会话 API")
}

fn valid_draft_output_with_title(outline_id: &str, title: &str) -> Value {
    json!({
        "draft": {
            "outline_id": outline_id,
            "title": title,
            "kind": "backend",
            "goal": "提供登录会话过期检测与刷新相关 API。",
            "implementation_context": "实现 product service 与 web handler，返回稳定 DTO。",
            "exclusive_write_scopes": ["src/product/session.rs", "src/web/session_handlers.rs"],
            "forbidden_write_scopes": ["web/**"],
            "depends_on_outline_ids": [],
            "required_handoff_from_outline_ids": [],
            "handoff_summary": "输出 SessionStatusDto 与错误语义。",
            "verification_plan": {
                "commands": [
                    {
                        "id": "cmd_backend_session",
                        "label": "cargo test session",
                        "command": "cargo test --locked --lib session",
                        "cwd": "",
                        "purpose": "验证后端 session 逻辑",
                        "required": true,
                        "timeout_seconds": 120,
                        "safety": "approved",
                        "source": "provider"
                    }
                ],
                "manual_checks": [],
                "required_gates": ["cmd_backend_session"]
            }
        }
    })
}

fn valid_frontend_draft_output() -> Value {
    valid_frontend_draft_output_with_title("实现前端会话过期提示")
}

fn valid_frontend_draft_output_with_title(title: &str) -> Value {
    json!({
        "draft": {
            "outline_id": "outline_frontend_expiry",
            "title": title,
            "kind": "frontend",
            "goal": "在前端展示会话过期提示并触发重新登录入口。",
            "implementation_context": "消费后端会话状态 DTO，展示稳定 UI 状态。",
            "exclusive_write_scopes": ["web/src/session/expiry.ts"],
            "forbidden_write_scopes": ["src/product/**"],
            "depends_on_outline_ids": ["outline_backend_session"],
            "required_handoff_from_outline_ids": ["outline_backend_session"],
            "handoff_summary": "输出前端会话过期提示组件。",
            "verification_plan": {
                "commands": [
                    {
                        "id": "cmd_frontend_session",
                        "label": "pnpm web test",
                        "command": "pnpm -C web test",
                        "cwd": "",
                        "purpose": "验证前端 session UI",
                        "required": true,
                        "timeout_seconds": 120,
                        "safety": "approved",
                        "source": "provider"
                    }
                ],
                "manual_checks": [],
                "required_gates": ["cmd_frontend_session"]
            }
        }
    })
}

fn valid_integration_draft_output() -> Value {
    valid_integration_draft_output_with_title("集成测试：会话过期端到端")
}

fn valid_integration_draft_output_with_title(title: &str) -> Value {
    json!({
        "draft": {
            "outline_id": "outline_integration_session",
            "title": title,
            "kind": "integration",
            "goal": "覆盖会话过期到前端提示的贯通路径。",
            "implementation_context": "覆盖后端会话 DTO 到前端提示的集成路径。",
            "exclusive_write_scopes": ["tests/session/expiry.rs"],
            "forbidden_write_scopes": [],
            "depends_on_outline_ids": ["outline_frontend_expiry"],
            "required_handoff_from_outline_ids": ["outline_frontend_expiry"],
            "handoff_summary": "输出端到端验证覆盖。",
            "verification_plan": {
                "commands": [
                    {
                        "id": "cmd_integration_session",
                        "label": "cargo test session integration",
                        "command": "cargo test --locked --test it_web session",
                        "cwd": "",
                        "purpose": "验证会话过期贯通路径",
                        "required": true,
                        "timeout_seconds": 120,
                        "safety": "approved",
                        "source": "provider"
                    }
                ],
                "manual_checks": [],
                "required_gates": ["cmd_integration_session"]
            }
        }
    })
}

fn invalid_draft_output_missing_scope(outline_id: &str) -> Value {
    let mut output = valid_draft_output(outline_id);
    output["draft"]["exclusive_write_scopes"] = json!([]);
    output
}
