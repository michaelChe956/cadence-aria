#[tokio::test]
async fn batch_accept_all_runs_final_compile_and_publishes_revision_entities() {
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

    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths.clone());
    assert!(
        lifecycle
            .list_work_items("project_0001", "issue_0001")
            .expect("list work items before compile")
            .is_empty(),
        "Draft 阶段不能提前写入真实 WorkItem"
    );
    assert!(
        lifecycle
            .list_verification_plans("project_0001", "issue_0001")
            .expect("list verification plans before compile")
            .is_empty(),
        "Draft 阶段不能提前写入真实 VerificationPlan"
    );

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
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_plan_compile"
        }) && messages
            .iter()
            .any(|message| message["type"] == "stage_change" && message["stage"] == "human_confirm")
    })
    .await;
    assert!(
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_plan_compile"
        }),
        "accept_all should enter work_item_plan_compile, got {messages:?}"
    );

    let legacy_work_items = lifecycle
        .list_work_items("project_0001", "issue_0001")
        .expect("list work items after compile");
    let legacy_verification_plans = lifecycle
        .list_verification_plans("project_0001", "issue_0001")
        .expect("list verification plans after compile");
    let plan = lifecycle
        .get_issue_work_item_plan("project_0001", "issue_0001", &plan_id)
        .expect("get compiled plan");
    let child_sessions = lifecycle
        .list_workspace_sessions("project_0001", "issue_0001")
        .expect("list workspace sessions");
    let work_item_sessions: Vec<_> = child_sessions
        .iter()
        .filter(|session| session.workspace_type == WorkspaceType::WorkItem)
        .collect();

    assert!(legacy_work_items.is_empty());
    assert!(legacy_verification_plans.is_empty());
    assert_eq!(work_item_sessions.len(), 3);
    assert!(
        work_item_sessions
            .iter()
            .all(|session| session.work_item_runtime_binding.is_some()),
        "Final Compile 进入 human_confirm 前必须持久化每个 Work Item Session 的 RuntimeBinding"
    );
    assert!(
        work_item_sessions.iter().all(|session| {
            session.messages.first().is_some_and(|message| {
                message.role == "system" && message.content.contains("[work_item_context]")
            })
        }),
        "Final Compile 进入 human_confirm 前必须持久化每个 Work Item Session 的 Revision 上下文"
    );
    assert_eq!(plan.status, IssueWorkItemPlanStatus::Confirmed);
    assert_eq!(plan.work_item_ids.len(), 3);
    assert_eq!(plan.verification_plan_ids.len(), 3);
    assert_eq!(plan.dependency_graph.len(), 2);
    let revision_store = WorkItemRevisionStore::new(app_paths.clone());
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
    assert_eq!(plan_revision.revision_no, 1);
    assert_eq!(plan_revision.work_item_bindings.len(), 3);
    assert_eq!(
        revision_store
            .get_dependency_graph_revision(&lineage, &plan_revision.dependency_graph_revision_id)
            .expect("load dependency graph revision")
            .edges
            .len(),
        2
    );
    revision_store
        .get_plan_validation_report(&lineage, &plan_revision.validation_report_ref)
        .expect("load plan validation report");
    let plan_projection = revision_store
        .get_plan_projection_bundle(&lineage, &plan_revision.plan_projection_bundle_id)
        .expect("load plan projection bundle");
    assert_eq!(
        plan.work_item_ids,
        plan_projection
            .coder_group_context
            .ordered_logical_work_item_ids
    );

    let (status, lifecycle_view) = request_json(
        app.clone(),
        Method::GET,
        "/api/issues/issue_0001/lifecycle?project_id=project_0001",
        json!({}),
    )
    .await;
    assert_eq!(
        status,
        axum::http::StatusCode::OK,
        "Schema v2 lifecycle request failed: {lifecycle_view}"
    );
    let lifecycle_work_items = lifecycle_view["work_items"]
        .as_array()
        .expect("lifecycle work items must be an array");
    assert_eq!(
        lifecycle_work_items.len(),
        plan_projection.human_group_projection.work_items.len(),
        "Schema v2 lifecycle must derive cards from the Human Group Projection when no Legacy Work Item exists: {lifecycle_view}"
    );
    for human_item in &plan_projection.human_group_projection.work_items {
        let card = lifecycle_work_items
            .iter()
            .find(|card| card["work_item_id"] == human_item.logical_work_item_id)
            .unwrap_or_else(|| {
                panic!(
                    "missing lifecycle card for Schema v2 logical work item {}: {lifecycle_view}",
                    human_item.logical_work_item_id
                )
            });
        assert_eq!(card["repository_id"], "repository_0001");
        assert_eq!(card["title"], human_item.title);
        assert_eq!(card["plan_status"], "confirmed");
        assert_eq!(card["execution_status"], "pending");
        assert_eq!(card["source_work_item_plan_id"], plan_id);
        assert_eq!(card["depends_on"], json!(human_item.depends_on));
        assert_eq!(
            card["exclusive_write_scopes"],
            json!(human_item.scope_summary.owned_scopes)
        );
        assert_eq!(
            card["forbidden_write_scopes"],
            json!(human_item.scope_summary.forbidden_scopes)
        );
        assert!(card["latest_attempt"].is_null());
    }

    let (status, group_attempt) = request_json(
        app.clone(),
        Method::POST,
        &format!(
            "/api/projects/project_0001/issues/issue_0001/work-item-plans/{plan_id}/coding-attempts"
        ),
        json!({}),
    )
    .await;
    assert_eq!(
        status,
        axum::http::StatusCode::OK,
        "Schema v2 group coding attempt creation failed: {group_attempt}"
    );
    assert_eq!(group_attempt["attempt_scope"], "work_item_group");
    assert_eq!(group_attempt["work_item_group_id"], plan_id);
    let group_attempt_id = group_attempt["attempt_id"]
        .as_str()
        .expect("group coding attempt id");
    let coding_store = CodingAttemptStore::new(app_paths.clone());
    let group_units = coding_store
        .list_coding_units("project_0001", "issue_0001", group_attempt_id)
        .expect("list Schema v2 group units");
    let (status, lifecycle_with_attempt) = request_json(
        app.clone(),
        Method::GET,
        "/api/issues/issue_0001/lifecycle?project_id=project_0001",
        json!({}),
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::OK, "{lifecycle_with_attempt}");
    let lifecycle_cards = lifecycle_with_attempt["work_items"]
        .as_array()
        .expect("Schema v2 lifecycle cards after group attempt");
    for unit in &group_units {
        let card = lifecycle_cards
            .iter()
            .find(|card| card["work_item_id"] == unit.logical_work_item_id)
            .expect("Schema v2 lifecycle card for group unit");
        assert_eq!(card["latest_attempt"]["attempt_id"], group_attempt_id);
        assert_eq!(
            card["execution_status"],
            serde_json::to_value(&unit.status).expect("serialize unit status")
        );
    }
    let (status, delete_attempt_response) = request_json(
        app.clone(),
        Method::DELETE,
        &format!(
            "/api/projects/project_0001/issues/issue_0001/coding-attempts/{group_attempt_id}"
        ),
        json!({}),
    )
    .await;
    assert_eq!(
        status,
        axum::http::StatusCode::NO_CONTENT,
        "Schema v2 attempt deletion must resolve the repository from Issue metadata, not a Legacy Work Item: {delete_attempt_response}"
    );
    assert!(
        lifecycle
            .list_work_items("project_0001", "issue_0001")
            .expect("list legacy work items after Schema v2 attempt deletion")
            .is_empty(),
        "Schema v2 attempt deletion must not create or read Legacy Work Item data"
    );
    for logical_id in &plan.work_item_ids {
        let revision_id = plan_revision
            .work_item_bindings
            .get(logical_id)
            .expect("stable work item binding");
        let work_item_revision = revision_store
            .get_work_item_revision(&lineage, logical_id, revision_id)
            .expect("load work item revision");
        revision_store
            .get_verification_plan_revision(
                &lineage,
                &work_item_revision.verification_plan_revision_id,
            )
            .expect("load verification plan revision");
        revision_store
            .get_work_item_projection_bundle(
                &lineage,
                &work_item_revision.work_item_projection_bundle_id,
            )
            .expect("load work item projection bundle");
    }
    assert_eq!(
        work_item_sessions
            .iter()
            .map(|session| session.entity_id.as_str())
            .collect::<std::collections::BTreeSet<_>>(),
        plan.work_item_ids
            .iter()
            .map(String::as_str)
            .collect::<std::collections::BTreeSet<_>>()
    );
    let issue = IssueStore::new(app_paths.clone())
        .get("project_0001", "issue_0001")
        .expect("load issue for runtime repository");
    let expected_repository_id = issue.repo_id.expect("issue repository id");
    for session in &work_item_sessions {
        let binding = session
            .work_item_runtime_binding
            .as_ref()
            .expect("runtime binding");
        let projection_bundle = revision_store
            .get_work_item_projection_bundle(&lineage, &binding.projection_bundle_id)
            .expect("load human projection bundle");
        let context = &session.messages[0].content;
        assert!(context.contains(&format!("plan_id: {}", binding.plan_id)));
        assert!(context.contains(&format!(
            "plan_revision_id: {}",
            binding.plan_revision_id
        )));
        assert!(context.contains(&format!(
            "work_item_revision_id: {}",
            binding.work_item_revision_id
        )));
        assert!(context.contains(&format!(
            "projection_bundle_id: {}",
            binding.projection_bundle_id
        )));
        assert!(context.contains(&format!(
            "verification_plan_revision_id: {}",
            binding.verification_plan_revision_id
        )));
        assert!(context.contains(&format!(
            "human_projection_hash: {}",
            binding.human_projection_hash
        )));
        assert!(context.contains(&format!(
            "title: {}",
            projection_bundle.human_projection.title
        )));
        assert!(context.contains(&format!(
            "goal: {}",
            projection_bundle.human_projection.goal
        )));
        let verification_plan = revision_store
            .get_verification_plan_revision(&lineage, &binding.verification_plan_revision_id)
            .expect("load revision verification plan");
        assert!(context.contains("[verification_checks]"));
        for check in &verification_plan.verification_checks {
            assert!(context.contains(&check.check_id));
        }
        for forbidden in [
            "[work_item_plan_source]",
            "canonical_contract",
            "coder_projection",
            "reviewer_projection",
            "criterion_refs",
        ] {
            assert!(
                !context.contains(forbidden),
                "Work Item Human Context must not expose {forbidden}: {context}"
            );
        }
        assert_eq!(
            workspace_repository_for_session(&app_paths, &lifecycle, session)
                .expect("Work Item RuntimeBinding must resolve repository without Legacy Work Item")
                .id,
            expected_repository_id
        );
    }
    let presentation_session = (*work_item_sessions[0]).clone();
    let original_message_count = presentation_session.messages.len();
    let presentation_binding = presentation_session
        .work_item_runtime_binding
        .as_ref()
        .expect("presentation runtime binding");
    revision_store
        .put_human_presentation_revision(
            &lineage,
            &HumanPresentationRevision {
                id: "human_presentation_revision_0001".to_string(),
                source_plan_projection_bundle_id: None,
                source_work_item_projection_bundle_id: Some(
                    presentation_binding.projection_bundle_id.clone(),
                ),
                supersedes: None,
                human_summary: "先完成库导出，再由服务与界面消费。".to_string(),
                why_split: Some("降低并行修改冲突。".to_string()),
                dependency_explanation: vec!["服务依赖库导出的稳定接口。".to_string()],
                risk_explanation: vec!["变更公共导出时必须保留兼容性。".to_string()],
                source_refs: vec!["design_spec_0001".to_string()],
                normative: false,
                used_by_provider: false,
                created_at: "2026-07-26T00:00:00Z".to_string(),
            },
        )
        .expect("save human presentation");
    let refreshed_session = ensure_workspace_context_message(
        &app_paths,
        &lifecycle,
        presentation_session,
    )
    .expect("refresh Work Item human context");
    assert_eq!(
        refreshed_session.messages.len(),
        original_message_count,
        "refreshing Revision-backed context must replace the existing system message"
    );
    let refreshed_context = &refreshed_session.messages[0].content;
    for expected in [
        "human_presentation_id: human_presentation_revision_0001",
        "human_summary: 先完成库导出，再由服务与界面消费。",
        "why_split: 降低并行修改冲突。",
        "dependency_explanation: [服务依赖库导出的稳定接口。]",
        "risk_explanation: [变更公共导出时必须保留兼容性。]",
    ] {
        assert!(refreshed_context.contains(expected), "missing {expected}");
    }

    let store = WorkItemPlanStore::new(app_paths);
    let index = store
        .load_active_index("project_0001", "issue_0001", &plan_id)
        .expect("load active index")
        .expect("active index");
    let compile_dir = root
        .path()
        .join(".aria/projects/project_0001/issues/issue_0001/work_item_plan_compiles")
        .join(&plan_id);
    let compile_files: Vec<_> = fs::read_dir(&compile_dir)
        .expect("read compile tx dir")
        .collect::<Result<Vec<_>, _>>()
        .expect("compile dir entries");
    assert_eq!(compile_files.len(), 1);
    let compile_tx: Value =
        serde_json::from_slice(&fs::read(compile_files[0].path()).expect("read compile tx"))
            .expect("compile tx json");
    assert_eq!(compile_tx["status"], "committed");
    assert_eq!(compile_tx["plan_commit_state"], "committed");
    assert_eq!(compile_tx["created_work_item_ids"], json!([]));
    assert_eq!(compile_tx["created_verification_plan_ids"], json!([]));
    assert_eq!(compile_tx["previous_plan_snapshot"]["status"], "draft");
    assert_eq!(
        compile_tx["active_draft_ids"]
            .as_array()
            .expect("draft ids")
            .len(),
        index.outline_to_current_draft_id.len()
    );

    ws.send(Message::Text(
        json!({ "type": "human_confirm", "decision": "confirm" })
            .to_string()
            .into(),
    ))
    .await
    .expect("send final human confirm");
    let completed_messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages
            .iter()
            .any(|message| message["type"] == "stage_change" && message["stage"] == "completed")
    })
    .await;
    assert!(
        completed_messages
            .iter()
            .any(|message| message["type"] == "stage_change" && message["stage"] == "completed"),
        "final human confirm after compile should complete workspace, got {completed_messages:?}"
    );
    let work_item_sessions_after_confirm = lifecycle
        .list_workspace_sessions("project_0001", "issue_0001")
        .expect("list workspace sessions after final confirm")
        .into_iter()
        .filter(|session| session.workspace_type == WorkspaceType::WorkItem)
        .collect::<Vec<_>>();
    assert_eq!(
        work_item_sessions_after_confirm.len(),
        3,
        "final human confirm must not create duplicate WorkItem sessions"
    );

    let (status, delete_response) = request_json(
        app.clone(),
        Method::DELETE,
        &format!(
            "/api/projects/project_0001/issues/issue_0001/work-item-plans/{plan_id}"
        ),
        json!({}),
    )
    .await;
    assert_eq!(
        status,
        axum::http::StatusCode::OK,
        "Schema v2 plan deletion must not require a Legacy Work Item: {delete_response}"
    );
    assert_eq!(delete_response["status"], "deleted");
    assert!(
        lifecycle
            .list_work_items("project_0001", "issue_0001")
            .expect("list legacy work items after Schema v2 deletion")
            .is_empty(),
        "Schema v2 plan deletion must leave the Legacy Work Item store untouched"
    );
    let (status, lifecycle_after_delete) = request_json(
        app.clone(),
        Method::GET,
        "/api/issues/issue_0001/lifecycle?project_id=project_0001",
        json!({}),
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::OK);
    assert!(
        lifecycle_after_delete["work_item_plans"]
            .as_array()
            .expect("lifecycle work item plans")
            .is_empty()
    );
    assert!(
        lifecycle_after_delete["work_items"]
            .as_array()
            .expect("lifecycle work items")
            .is_empty()
    );
    assert!(
        lifecycle_after_delete["workspace_sessions"]
            .as_array()
            .expect("lifecycle workspace sessions")
            .iter()
            .all(|session| session["workspace_type"] != "work_item"),
        "Schema v2 plan deletion must remove only its child Work Item session metadata"
    );

    ws.close(None).await.ok();
}
