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
            permission_modes: cadence_aria::product::models::WorkspaceRolePermissionModes::default(),
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
            permission_modes: cadence_aria::product::models::WorkspaceRolePermissionModes::default(),
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
    let revision_store = WorkItemRevisionStore::new(app_paths.clone());
    let store = WorkItemPlanStore::new(app_paths);
    let mut tx = store
        .list_compile_transactions("project_0001", "issue_0001", &plan_id)
        .expect("list compile tx")
        .into_iter()
        .next()
        .expect("compile tx");
    assert!(tx.created_work_item_ids.is_empty());
    assert!(tx.created_verification_plan_ids.is_empty());
    assert_eq!(tx.child_session_ids.len(), 3);
    let lineage = revision_store
        .get_plan_lineage("project_0001", "issue_0001", &plan_id)
        .expect("load active plan lineage");
    let active_revision_id = lineage
        .active_revision_id
        .as_deref()
        .expect("active plan revision id");
    let plan_revision = revision_store
        .get_plan_revision(
            "project_0001",
            "issue_0001",
            &plan_id,
            active_revision_id,
        )
        .expect("load active plan revision");
    assert_eq!(plan_revision.work_item_bindings.len(), 3);
    revision_store
        .get_plan_validation_report(&lineage, &plan_revision.validation_report_ref)
        .expect("load plan validation report");
    let plan_projection = revision_store
        .get_plan_projection_bundle(&lineage, &plan_revision.plan_projection_bundle_id)
        .expect("load plan projection bundle");
    let expected_work_item_ids = plan_projection
        .coder_group_context
        .ordered_logical_work_item_ids
        .clone();
    let expected_verification_plan_ids = expected_work_item_ids
        .iter()
        .map(|logical_id| {
            let revision_id = plan_revision
                .work_item_bindings
                .get(logical_id)
                .expect("stable logical binding");
            let work_item_revision = revision_store
                .get_work_item_revision(&lineage, logical_id, revision_id)
                .expect("load work item revision");
            let projection = revision_store
                .get_work_item_projection_bundle(
                    &lineage,
                    &work_item_revision.work_item_projection_bundle_id,
                )
                .expect("load work item projection bundle");
            assert_eq!(projection.work_item_revision_id, work_item_revision.id);
            work_item_revision.verification_plan_revision_id
        })
        .collect::<Vec<_>>();
    let artifact_versions = lifecycle
        .list_artifact_versions(&session_id)
        .expect("list compile artifact versions");
    assert!(artifact_versions.iter().any(|version| {
        matches!(
            &version.payload,
            ArtifactPayload::WorkItemPlanCompileReport { .. }
        )
    }));
    assert!(artifact_versions.iter().any(|version| {
        version.is_current
            && matches!(
                &version.payload,
                ArtifactPayload::WorkItemPlanProjection { projection }
                    if projection.id == plan_projection.id
            )
    }));

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
            permission_modes: cadence_aria::product::models::WorkspaceRolePermissionModes::default(),
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
    assert_eq!(plan.work_item_ids, expected_work_item_ids);
    assert_eq!(
        plan.verification_plan_ids,
        expected_verification_plan_ids
    );
    assert_eq!(
        lifecycle
            .list_workspace_sessions("project_0001", "issue_0001")
            .expect("list child sessions after recovery")
            .into_iter()
            .filter(|session| session.workspace_type == WorkspaceType::WorkItem)
            .count(),
        3
    );

    let tx = store
        .get_compile_transaction("project_0001", "issue_0001", &plan_id, &tx.compile_id)
        .expect("load continued tx");
    assert_eq!(tx.status, WorkItemPlanCompileStatus::Committed);
    assert_eq!(tx.plan_commit_state, WorkItemPlanCommitState::Committed);

    ws.close(None).await.ok();
}

#[tokio::test]
async fn recovery_abort_is_rejected_when_active_plan_revision_exists_with_stale_marker() {
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
    let revision_store = WorkItemRevisionStore::new(app_paths.clone());
    let store = WorkItemPlanStore::new(app_paths);
    let mut tx = store
        .list_compile_transactions("project_0001", "issue_0001", &plan_id)
        .expect("list compile tx")
        .into_iter()
        .next()
        .expect("compile tx");
    let lineage = revision_store
        .get_plan_lineage("project_0001", "issue_0001", &plan_id)
        .expect("load active plan lineage before stale-marker recovery");
    let active_revision_id = lineage
        .active_revision_id
        .clone()
        .expect("active plan revision before stale-marker recovery");
    let plan_revision = revision_store
        .get_plan_revision(
            "project_0001",
            "issue_0001",
            &plan_id,
            &active_revision_id,
        )
        .expect("load active plan revision before stale-marker recovery");
    assert_eq!(plan_revision.work_item_bindings.len(), 3);

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
            permission_modes: cadence_aria::product::models::WorkspaceRolePermissionModes::default(),
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
        messages.iter().any(|message| {
            message["type"] == "protocol_error"
                && message["code"] == "INVALID_COMPILE_RECOVERY_ACTION"
        })
    })
    .await;
    assert!(
        messages.iter().any(|message| {
            message["type"] == "protocol_error"
                && message["code"] == "INVALID_COMPILE_RECOVERY_ACTION"
                && message["message"]
                    .as_str()
                    .is_some_and(|message| message.contains("active PlanRevision"))
        }),
        "active PlanRevision must reject stale-marker rollback, got {messages:?}"
    );

    let plan = lifecycle
        .get_issue_work_item_plan("project_0001", "issue_0001", &plan_id)
        .expect("load plan after rejected stale-marker rollback");
    assert_eq!(plan.status, IssueWorkItemPlanStatus::Confirmed);
    assert_eq!(plan.work_item_ids.len(), 3);
    assert_eq!(plan.verification_plan_ids.len(), 3);
    assert_eq!(
        lifecycle
            .list_workspace_sessions("project_0001", "issue_0001")
            .expect("list workspace sessions after rejected rollback")
            .into_iter()
            .filter(|session| session.workspace_type == WorkspaceType::WorkItem)
            .count(),
        3
    );
    assert_eq!(
        revision_store
            .get_plan_lineage("project_0001", "issue_0001", &plan_id)
            .expect("reload active plan lineage")
            .active_revision_id
            .as_deref(),
        Some(active_revision_id.as_str())
    );

    let tx = store
        .get_compile_transaction("project_0001", "issue_0001", &plan_id, &tx.compile_id)
        .expect("load rejected stale-marker tx");
    assert_eq!(tx.status, WorkItemPlanCompileStatus::RecoveryRequired);
    assert_eq!(tx.step_cursor, "committing");
    assert!(tx.created_work_item_ids.is_empty());
    assert!(tx.created_verification_plan_ids.is_empty());
    assert_eq!(tx.child_session_ids.len(), 3);

    ws.close(None).await.ok();
}

fn valid_draft_output(outline_id: &str) -> Value {
    valid_canonical_draft_output(outline_id, "实现后端登录会话 API")
}

fn unsafe_backend_draft_output() -> Value {
    let mut output = valid_draft_output("outline_backend_session");
    output["draft"]["canonical_contract"]["verification_checks"][0]["command"] =
        json!("rm -rf /");
    output["draft"]["verification_plan"]["checks"][0]["command"] = json!("rm -rf /");
    output
}

fn unsafe_frontend_draft_output() -> Value {
    let mut output = valid_frontend_draft_output();
    output["draft"]["canonical_contract"]["verification_checks"][0]["command"] =
        json!("rm -rf /");
    output["draft"]["verification_plan"]["checks"][0]["command"] = json!("rm -rf /");
    output
}

fn valid_frontend_draft_output() -> Value {
    valid_canonical_draft_output("outline_frontend_expiry", "实现前端会话过期提示")
}

fn valid_integration_draft_output() -> Value {
    let mut output = valid_canonical_draft_output(
        "outline_integration_session",
        "集成测试：会话过期端到端",
    );
    output["draft"]["canonical_contract"]["handoff_contract"]["provided_contract_refs"] =
        json!([]);
    output
}
