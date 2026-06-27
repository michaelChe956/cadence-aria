#[tokio::test]
async fn recovery_abort_and_rollback_is_rejected_after_plan_commit_marker() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls().await;
    let (app, root, _prompts) = app_with_confirmed_story_and_design_and_streaming_outputs(vec![
        valid_outline_output(),
        valid_draft_output("outline_backend_session"),
        valid_frontend_draft_output(),
        valid_integration_draft_output(),
    ])
    .await;
    let (session_id, plan_id, mut ws) = prepare_plan_accept_outline_and_select_batch(&app).await;

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
    let _messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages
            .iter()
            .any(|message| message["type"] == "stage_change" && message["stage"] == "human_confirm")
    })
    .await;
    ws.close(None).await.ok();

    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let store = WorkItemPlanStore::new(app_paths);
    let mut tx = store
        .list_compile_transactions("project_0001", "issue_0001", &plan_id)
        .expect("list compile tx")
        .into_iter()
        .next()
        .expect("compile tx");
    tx.status = WorkItemPlanCompileStatus::RecoveryRequired;
    tx.plan_commit_state = WorkItemPlanCommitState::Committed;
    tx.step_cursor = "plan_commit_marker_written".to_string();
    tx.failure_reason = Some("simulated recovery after commit marker".to_string());
    store
        .put_compile_transaction(&tx)
        .expect("save recovery tx");

    let mut timeline_nodes = lifecycle
        .load_timeline_nodes(&session_id)
        .expect("load timeline nodes");
    timeline_nodes.push(TimelineNode {
        node_id: "timeline_node_compile_recovery".to_string(),
        node_type: TimelineNodeType::WorkItemPlanCompileRecovery,
        agent: None,
        stage: WsWorkspaceStage::HumanConfirm,
        round: None,
        status: TimelineNodeStatus::Active,
        title: "WorkItemPlan Compile Recovery".to_string(),
        summary: Some("simulated recovery".to_string()),
        started_at: chrono::Utc::now().to_rfc3339(),
        completed_at: None,
        duration_ms: None,
        artifact_ref: None,
        provider_config_snapshot: ProviderConfigSnapshot {
            author: ProviderName::Fake,
            reviewer: None,
            review_rounds: 1,
        },
        retry: None,
    });
    lifecycle
        .save_timeline_nodes(&session_id, &timeline_nodes)
        .expect("save recovery timeline");

    let mut ws = connect_ws(app.clone(), &session_id).await;
    let session_messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "session_state"
                && message["active_node_id"] == "timeline_node_compile_recovery"
        })
    })
    .await;
    assert!(
        session_messages.iter().any(|message| {
            message["type"] == "session_state"
                && message["active_node_id"] == "timeline_node_compile_recovery"
                && message["stage"] == "human_confirm"
        }),
        "session restore should expose active compile recovery node, got {session_messages:?}"
    );

    ws.send(Message::Text(
        json!({
            "type": "work_item_plan_compile_recovery_action",
            "action": "abort_and_rollback",
            "reason": "try rollback after commit marker"
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send recovery rollback");

    let error_messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "protocol_error"
                && message["code"] == "INVALID_COMPILE_RECOVERY_ACTION"
        })
    })
    .await;
    assert!(
        error_messages.iter().any(|message| {
            message["type"] == "protocol_error"
                && message["code"] == "INVALID_COMPILE_RECOVERY_ACTION"
                && message["message"]
                    .as_str()
                    .is_some_and(|message| message.contains("plan_commit_state=committed"))
        }),
        "abort_and_rollback must be rejected after commit marker, got {error_messages:?}"
    );

    let plan = lifecycle
        .get_issue_work_item_plan("project_0001", "issue_0001", &plan_id)
        .expect("load plan after rejected rollback");
    assert_eq!(plan.status, IssueWorkItemPlanStatus::Confirmed);

    ws.close(None).await.ok();
}

#[tokio::test]
async fn recovery_human_triage_keeps_transaction_for_manual_resolution() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls().await;
    let (app, root, _prompts) = app_with_confirmed_story_and_design_and_streaming_outputs(vec![
        valid_outline_output(),
        valid_draft_output("outline_backend_session"),
        valid_frontend_draft_output(),
        valid_integration_draft_output(),
    ])
    .await;
    let (session_id, plan_id, mut ws) = prepare_plan_accept_outline_and_select_batch(&app).await;

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
    let _messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages
            .iter()
            .any(|message| message["type"] == "stage_change" && message["stage"] == "human_confirm")
    })
    .await;
    ws.close(None).await.ok();

    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let store = WorkItemPlanStore::new(app_paths);
    let mut tx = store
        .list_compile_transactions("project_0001", "issue_0001", &plan_id)
        .expect("list compile tx")
        .into_iter()
        .next()
        .expect("compile tx");
    tx.status = WorkItemPlanCompileStatus::RecoveryRequired;
    tx.plan_commit_state = WorkItemPlanCommitState::Committed;
    tx.failure_reason = Some("simulated recovery requires human triage".to_string());
    store
        .put_compile_transaction(&tx)
        .expect("save recovery tx");

    let mut timeline_nodes = lifecycle
        .load_timeline_nodes(&session_id)
        .expect("load timeline nodes");
    timeline_nodes.push(TimelineNode {
        node_id: "timeline_node_compile_recovery".to_string(),
        node_type: TimelineNodeType::WorkItemPlanCompileRecovery,
        agent: None,
        stage: WsWorkspaceStage::HumanConfirm,
        round: None,
        status: TimelineNodeStatus::Active,
        title: "WorkItemPlan Compile Recovery".to_string(),
        summary: Some("simulated recovery".to_string()),
        started_at: chrono::Utc::now().to_rfc3339(),
        completed_at: None,
        duration_ms: None,
        artifact_ref: None,
        provider_config_snapshot: ProviderConfigSnapshot {
            author: ProviderName::Fake,
            reviewer: None,
            review_rounds: 1,
        },
        retry: None,
    });
    lifecycle
        .save_timeline_nodes(&session_id, &timeline_nodes)
        .expect("save recovery timeline");

    let mut ws = connect_ws(app.clone(), &session_id).await;
    let _session_messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "session_state"
                && message["active_node_id"] == "timeline_node_compile_recovery"
        })
    })
    .await;
    ws.send(Message::Text(
        json!({
            "type": "work_item_plan_compile_recovery_action",
            "action": "human_triage",
            "reason": "交给人工整理已创建实体"
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send human triage");
    let messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages
            .iter()
            .any(|message| message["type"] == "stage_change" && message["stage"] == "human_confirm")
    })
    .await;
    assert!(
        messages.iter().any(|message| {
            message["type"] == "timeline_node_updated"
                && message["status"] == "completed"
                && message["summary"] == "Final Compile 转人工处理"
        }),
        "human_triage should complete recovery node, got {messages:?}"
    );

    let saved_tx = store
        .get_compile_transaction("project_0001", "issue_0001", &plan_id, &tx.compile_id)
        .expect("load human triage tx");
    assert_eq!(saved_tx.status, WorkItemPlanCompileStatus::RecoveryRequired);
    assert_eq!(
        saved_tx.failure_reason.as_deref(),
        Some("交给人工整理已创建实体")
    );

    ws.close(None).await.ok();
}

#[tokio::test]
async fn compile_recovery_resumes_after_committed_marker() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls().await;
    let (app, root, _prompts) = app_with_confirmed_story_and_design_and_streaming_outputs(vec![
        valid_outline_output(),
        valid_draft_output("outline_backend_session"),
        valid_frontend_draft_output(),
        valid_integration_draft_output(),
    ])
    .await;
    let (session_id, plan_id, mut ws) = prepare_plan_accept_outline_and_select_batch(&app).await;

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
    let _messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages
            .iter()
            .any(|message| message["type"] == "stage_change" && message["stage"] == "human_confirm")
    })
    .await;
    ws.close(None).await.ok();

    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let store = WorkItemPlanStore::new(app_paths);
    let mut tx = store
        .list_compile_transactions("project_0001", "issue_0001", &plan_id)
        .expect("list compile tx")
        .into_iter()
        .next()
        .expect("compile tx");
    let created_work_item_ids = tx.created_work_item_ids.clone();
    let created_verification_plan_ids = tx.created_verification_plan_ids.clone();
    assert_eq!(created_work_item_ids.len(), 3);
    assert_eq!(created_verification_plan_ids.len(), 3);

    lifecycle
        .restore_issue_work_item_plan_snapshot(
            "project_0001",
            "issue_0001",
            &plan_id,
            &tx.previous_plan_snapshot,
        )
        .expect("simulate crash before plan file update");
    tx.status = WorkItemPlanCompileStatus::RecoveryRequired;
    tx.plan_commit_state = WorkItemPlanCommitState::Committed;
    tx.step_cursor = "plan_commit_marker_written".to_string();
    tx.failure_reason = Some("simulated crash before plan update".to_string());
    store
        .put_compile_transaction(&tx)
        .expect("save recovery tx");

    let mut timeline_nodes = lifecycle
        .load_timeline_nodes(&session_id)
        .expect("load timeline nodes");
    timeline_nodes.push(TimelineNode {
        node_id: "timeline_node_compile_recovery".to_string(),
        node_type: TimelineNodeType::WorkItemPlanCompileRecovery,
        agent: None,
        stage: WsWorkspaceStage::HumanConfirm,
        round: None,
        status: TimelineNodeStatus::Active,
        title: "WorkItemPlan Compile Recovery".to_string(),
        summary: Some("simulated recovery".to_string()),
        started_at: chrono::Utc::now().to_rfc3339(),
        completed_at: None,
        duration_ms: None,
        artifact_ref: None,
        provider_config_snapshot: ProviderConfigSnapshot {
            author: ProviderName::Fake,
            reviewer: None,
            review_rounds: 1,
        },
        retry: None,
    });
    lifecycle
        .save_timeline_nodes(&session_id, &timeline_nodes)
        .expect("save recovery timeline");

    let mut ws = connect_ws(app.clone(), &session_id).await;
    let _session_messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "session_state"
                && message["active_node_id"] == "timeline_node_compile_recovery"
        })
    })
    .await;
    ws.send(Message::Text(
        json!({
            "type": "work_item_plan_compile_recovery_action",
            "action": "continue",
            "reason": "resume committed marker"
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send recovery continue");

    let messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages
            .iter()
            .any(|message| message["type"] == "stage_change" && message["stage"] == "human_confirm")
    })
    .await;
    assert!(
        messages.iter().any(|message| {
            message["type"] == "timeline_node_updated"
                && message["node_id"] == "timeline_node_compile_recovery"
        }),
        "recovery continue should complete recovery node, got {messages:?}"
    );

    let plan = lifecycle
        .get_issue_work_item_plan("project_0001", "issue_0001", &plan_id)
        .expect("load plan after recovery continue");
    assert_eq!(plan.status, IssueWorkItemPlanStatus::Confirmed);
    assert_eq!(plan.work_item_ids, created_work_item_ids);
    assert_eq!(plan.verification_plan_ids, created_verification_plan_ids);

    let tx = store
        .get_compile_transaction("project_0001", "issue_0001", &plan_id, &tx.compile_id)
        .expect("load continued tx");
    assert_eq!(tx.status, WorkItemPlanCompileStatus::Committed);
    assert_eq!(tx.plan_commit_state, WorkItemPlanCommitState::Committed);

    ws.close(None).await.ok();
}

#[tokio::test]
async fn recovery_abort_and_rollback_before_plan_commit_restores_previous_snapshot() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls().await;
    let (app, root, _prompts) = app_with_confirmed_story_and_design_and_streaming_outputs(vec![
        valid_outline_output(),
        valid_draft_output("outline_backend_session"),
        valid_frontend_draft_output(),
        valid_integration_draft_output(),
    ])
    .await;
    let (session_id, plan_id, mut ws) = prepare_plan_accept_outline_and_select_batch(&app).await;

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
    let _messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages
            .iter()
            .any(|message| message["type"] == "stage_change" && message["stage"] == "human_confirm")
    })
    .await;
    ws.close(None).await.ok();

    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let store = WorkItemPlanStore::new(app_paths);
    let mut tx = store
        .list_compile_transactions("project_0001", "issue_0001", &plan_id)
        .expect("list compile tx")
        .into_iter()
        .next()
        .expect("compile tx");
    assert_eq!(
        lifecycle
            .list_work_items("project_0001", "issue_0001")
            .expect("list work items before rollback")
            .len(),
        3
    );

    tx.status = WorkItemPlanCompileStatus::RecoveryRequired;
    tx.plan_commit_state = WorkItemPlanCommitState::NotStarted;
    tx.step_cursor = "committing".to_string();
    tx.failure_reason = Some("simulated recovery before plan commit".to_string());
    store
        .put_compile_transaction(&tx)
        .expect("save recovery tx");

    let mut timeline_nodes = lifecycle
        .load_timeline_nodes(&session_id)
        .expect("load timeline nodes");
    timeline_nodes.push(TimelineNode {
        node_id: "timeline_node_compile_recovery".to_string(),
        node_type: TimelineNodeType::WorkItemPlanCompileRecovery,
        agent: None,
        stage: WsWorkspaceStage::HumanConfirm,
        round: None,
        status: TimelineNodeStatus::Active,
        title: "WorkItemPlan Compile Recovery".to_string(),
        summary: Some("simulated recovery".to_string()),
        started_at: chrono::Utc::now().to_rfc3339(),
        completed_at: None,
        duration_ms: None,
        artifact_ref: None,
        provider_config_snapshot: ProviderConfigSnapshot {
            author: ProviderName::Fake,
            reviewer: None,
            review_rounds: 1,
        },
        retry: None,
    });
    lifecycle
        .save_timeline_nodes(&session_id, &timeline_nodes)
        .expect("save recovery timeline");

    let mut ws = connect_ws(app.clone(), &session_id).await;
    let _session_messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "session_state"
                && message["active_node_id"] == "timeline_node_compile_recovery"
        })
    })
    .await;
    ws.send(Message::Text(
        json!({
            "type": "work_item_plan_compile_recovery_action",
            "action": "abort_and_rollback",
            "reason": "rollback before plan commit"
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send recovery rollback");
    let messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages
            .iter()
            .any(|message| message["type"] == "stage_change" && message["stage"] == "human_confirm")
    })
    .await;
    assert!(
        messages.iter().any(|message| {
            message["type"] == "timeline_node_updated"
                && message["node_id"] == "timeline_node_compile_recovery"
        }),
        "rollback should complete recovery node, got {messages:?}"
    );

    let plan = lifecycle
        .get_issue_work_item_plan("project_0001", "issue_0001", &plan_id)
        .expect("load plan after rollback");
    assert_eq!(plan.status, IssueWorkItemPlanStatus::Draft);
    assert!(plan.work_item_ids.is_empty());
    assert!(plan.verification_plan_ids.is_empty());
    assert!(
        lifecycle
            .list_work_items("project_0001", "issue_0001")
            .expect("list work items after rollback")
            .is_empty()
    );
    assert!(
        lifecycle
            .list_verification_plans("project_0001", "issue_0001")
            .expect("list verification plans after rollback")
            .is_empty()
    );
    assert!(
        lifecycle
            .list_workspace_sessions("project_0001", "issue_0001")
            .expect("list workspace sessions after rollback")
            .into_iter()
            .filter(|session| session.workspace_type == WorkspaceType::WorkItem)
            .collect::<Vec<_>>()
            .is_empty()
    );

    let tx = store
        .get_compile_transaction("project_0001", "issue_0001", &plan_id, &tx.compile_id)
        .expect("load rolled back tx");
    assert_eq!(tx.status, WorkItemPlanCompileStatus::Failed);
    assert_eq!(tx.step_cursor, "rolled_back");
    assert!(tx.created_work_item_ids.is_empty());
    assert!(tx.created_verification_plan_ids.is_empty());
    assert!(tx.child_session_ids.is_empty());

    ws.close(None).await.ok();
}

fn valid_draft_output(outline_id: &str) -> Value {
    json!({
        "draft": {
            "outline_id": outline_id,
            "title": "实现后端登录会话 API",
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

fn unsafe_backend_draft_output() -> Value {
    let mut output = valid_draft_output("outline_backend_session");
    output["draft"]["verification_plan"]["commands"][0]["command"] = json!("rm -rf /");
    output
}

fn unsafe_frontend_draft_output() -> Value {
    let mut output = valid_frontend_draft_output();
    output["draft"]["verification_plan"]["commands"][0]["command"] = json!("rm -rf /");
    output
}

fn valid_frontend_draft_output() -> Value {
    json!({
        "draft": {
            "outline_id": "outline_frontend_expiry",
            "title": "实现前端会话过期提示",
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
    json!({
        "draft": {
            "outline_id": "outline_integration_session",
            "title": "集成测试：会话过期端到端",
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
