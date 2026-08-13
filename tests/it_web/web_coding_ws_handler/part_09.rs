#[tokio::test]
async fn coding_ws_session_state_includes_group_units() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let app = app_with_group_attempt(root.path());
    let attempt_id = "coding_attempt_0001";
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/ws/coding-attempts/{attempt_id}");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let state = match ws.next().await {
        Some(Ok(Message::Text(text))) => {
            serde_json::from_str::<serde_json::Value>(&text).expect("session state json")
        }
        other => panic!("expected text websocket message, got {other:?}"),
    };

    assert_eq!(state["type"], "coding_session_state");
    assert_eq!(state["attempt_scope"], "work_item_group");
    assert_eq!(state["work_item_group_id"], "work_item_plan_0001");
    assert_eq!(state["current_work_item_id"], "work_item_0001");
    assert_eq!(state["units"].as_array().expect("units").len(), 2);
    assert_eq!(state["units"][0]["status"], "running");
    assert_eq!(state["units"][1]["status"], "pending");

    ws.close(None).await.expect("close ws");
    server.abort();
}

fn app_with_group_attempt(root_path: &std::path::Path) -> axum::Router {
    let app_paths = ProductAppPaths::new(root_path.join(".aria"));
    let repo = root_path.join("repo");
    init_simple_git_repo(&repo);
    let repository = RepositoryStore::new(app_paths.clone())
        .create(CreateRepositoryInput {
            project_id: "project_0001".to_string(),
            name: "repo".to_string(),
            path: repo,
            default_policy_preset: Some("manual-write".to_string()),
            default_provider_mode: Some("fake".to_string()),
            idempotency_key: "coding-ws-part09-group-repository".to_string(),
        })
        .expect("create repository");
    let lifecycle = LifecycleStore::new(app_paths.clone());
    lifecycle
        .create_work_item(CreateWorkItemInput {
            id: Some("work_item_0001".to_string()),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: repository.id.clone(),
            story_spec_ids: Vec::new(),
            design_spec_ids: Vec::new(),
            title: "Coding work item 1".to_string(),
            work_item_set_id: Some("work_item_plan_0001".to_string()),
            sequence_hint: Some(10),
            plan_status: WorkItemPlanStatus::Confirmed,
            ..Default::default()
        })
        .expect("create work item 1");
    lifecycle
        .create_work_item(CreateWorkItemInput {
            id: Some("work_item_0002".to_string()),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: repository.id,
            story_spec_ids: Vec::new(),
            design_spec_ids: Vec::new(),
            title: "Coding work item 2".to_string(),
            work_item_set_id: Some("work_item_plan_0001".to_string()),
            sequence_hint: Some(20),
            plan_status: WorkItemPlanStatus::Confirmed,
            ..Default::default()
        })
        .expect("create work item 2");
    lifecycle
        .create_issue_work_item_plan(CreateIssueWorkItemPlanInput {
            id: Some("work_item_plan_0001".to_string()),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            source_story_spec_ids: Vec::new(),
            source_design_spec_ids: Vec::new(),
            options: IssueWorkItemPlanOptions {
                include_integration_tests: false,
                include_e2e_tests: false,
                force_frontend_backend_split: false,
                require_execution_plan_confirm: false,
            },
            status: IssueWorkItemPlanStatus::Confirmed,
            work_item_ids: vec!["work_item_0001".to_string(), "work_item_0002".to_string()],
            repository_profile_ref: None,
            verification_plan_ids: Vec::new(),
            dependency_graph: vec![cadence_aria::product::models::IssueWorkItemDependencyEdge {
                from_work_item_id: "work_item_0001".to_string(),
                to_work_item_id: "work_item_0002".to_string(),
            }],
            created_from_provider_run: None,
            validator_findings: Vec::new(),
        })
        .expect("create work item plan");

    let store = CodingAttemptStore::new(app_paths);
    let attempt = create_legacy_group_coding_attempt_fixture(
        &store,
        CreateGroupCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_work_item_id: "work_item_0001".to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Fake,
                reviewer: Some(ProviderName::Fake),
                review_rounds: 1,
                permission_modes: cadence_aria::product::models::WorkspaceRolePermissionModes::default(),
            },
            target_snapshot: None,
            max_auto_rework: 2,
        },
    );
    seed_authoritative_group_plan_fixture(&store, &attempt, true);
    store
        .create_coding_unit(CreateCodingExecutionUnitInput {
            attempt_id: attempt.id.clone(),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            logical_work_item_id: "work_item_0001".to_string(),
            work_item_revision_id: "work_item_revision_0001".to_string(),
            dependency_logical_work_item_ids: Vec::new(),
            order_index: 0,
            status: CodingExecutionUnitStatus::Running,
        })
        .expect("create coding unit 1");
    store
        .create_coding_unit(CreateCodingExecutionUnitInput {
            attempt_id: attempt.id,
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            logical_work_item_id: "work_item_0002".to_string(),
            work_item_revision_id: "work_item_revision_0002".to_string(),
            dependency_logical_work_item_ids: vec!["work_item_0001".to_string()],
            order_index: 1,
            status: CodingExecutionUnitStatus::Pending,
        })
        .expect("create coding unit 2");

    build_web_router(WebAppState::new(
        root_path.to_path_buf(),
        WebRuntime::new_fake(root_path.to_path_buf()),
    ))
}

#[tokio::test]
async fn coding_ws_stage_gate_confirm_resolves_persisted_gate() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let app = app_with_attempt(root.path());
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("attempt");
    let gate = store
        .create_stage_gate(
            &attempt,
            CodingExecutionStage::CodeReview,
            CodingProviderRole::CodeReviewer,
            "2026-05-28T00:00:05Z".to_string(),
            CodingRoleProviderConfigSnapshot::from(ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::Fake),
                review_rounds: 1,
                permission_modes: cadence_aria::product::models::WorkspaceRolePermissionModes::default(),
            }),
        )
        .expect("create stage gate");
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/ws/coding-attempts/coding_attempt_0001");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;

    send_json(
        &mut ws,
        &CodingWsInMessage::StageGateConfirm {
            stage: CodingExecutionStage::CodeReview,
        },
    )
    .await;

    match recv_json(&mut ws).await {
        CodingWsOutMessage::CodingSessionState { pending_gates, .. } => {
            assert!(pending_gates.is_empty());
        }
        other => panic!("expected coding session state, got {other:?}"),
    }
    let gates = store
        .list_stage_gates("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("list stage gates");
    assert_eq!(gates.len(), 1);
    assert_eq!(gates[0].gate_id, gate.gate_id);
    assert_eq!(
        gates[0].status,
        cadence_aria::product::coding_models::CodingStageGateStatus::Confirmed
    );

    ws.close(None).await.expect("close ws");
    server.abort();
}
#[cfg(unix)]
#[tokio::test]
async fn group_attempt_runs_group_final_review_when_review_request_push_fails() {
    use std::os::unix::fs::PermissionsExt;

    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let app = app_with_group_full_chain_attempt_and_provider(
        root.path(),
        Arc::new(IndependentCodeReviewPlanDefectProvider::approve_all()),
    );
    // 注入拒绝 push 的 pre-receive hook，使 review_request push 必然失败
    let hook = root.path().join("remote.git/hooks/pre-receive");
    fs::write(&hook, "#!/bin/sh\nexit 1\n").expect("rejecting hook");
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).expect("chmod hook");
    let rejected_output = Command::new("git")
        .args(["push", "origin", "HEAD:aria/issues/issue_0001"])
        .current_dir(root.path().join("repo"))
        .output()
        .expect("probe rejected push");
    let rejected_stderr = String::from_utf8_lossy(&rejected_output.stderr);
    assert!(
        !rejected_output.status.success(),
        "pre-receive hook must reject the probe push"
    );
    assert!(
        rejected_stderr.contains("[remote rejected]"),
        "push rejection diagnostic must remain machine-detectable: {rejected_stderr}"
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/ws/coding-attempts/coding_attempt_0001");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;
    send_json(&mut ws, &CodingWsInMessage::StartCoding).await;

    let mut confirmed_gates = HashSet::new();
    let mut bound_first_handoff = false;
    let mut saw_failed_review_request = false;
    let mut reached_human_final_confirm = false;
    for _ in 0..320 {
        match timeout(Duration::from_secs(2), recv_json(&mut ws)).await {
            Ok(CodingWsOutMessage::CodingGateRequired { gate }) => {
                if gate.kind == CodingGateKind::StageGate
                    && let Some(stage) = gate.stage.clone()
                    && confirmed_gates.insert(gate.gate_id)
                {
                    send_json(&mut ws, &CodingWsInMessage::StageGateConfirm { stage }).await;
                }
            }
            Ok(CodingWsOutMessage::CodingSessionState {
                current_work_item_id,
                ..
            }) if current_work_item_id.as_deref() == Some("work_item_0002")
                && !bound_first_handoff =>
            {
                bind_completed_first_unit_handoff_revision(&store);
                bound_first_handoff = true;
            }
            Ok(CodingWsOutMessage::ReviewRequestUpdate { review_request }) => {
                if review_request.push_status == PushStatus::Failed {
                    saw_failed_review_request = true;
                }
            }
            Ok(CodingWsOutMessage::CodingSessionState { status, stage, .. })
                if status == CodingAttemptStatus::WaitingForHuman
                    && stage == CodingExecutionStage::FinalConfirm =>
            {
                reached_human_final_confirm = true;
                break;
            }
            Ok(CodingWsOutMessage::InternalPrReviewComplete { .. }) => {
                panic!("fresh group must not run the removed provider group review");
            }
            Ok(CodingWsOutMessage::CodingProtocolError { code, message })
                if code == "coding_start_failed" && message.contains("git_push_indeterminate") =>
            {
                panic!(
                    "review-request push rejection must be recorded as Failed, not indeterminate"
                );
            }
            Ok(CodingWsOutMessage::CodingProtocolError { code, message }) => {
                panic!("unexpected coding protocol error {code}: {message}");
            }
            Ok(_) => {}
            Err(_) => {
                let attempt = store
                    .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
                    .expect("attempt after timeout");
                let requests = store
                    .list_review_requests("project_0001", "issue_0001", "coding_attempt_0001")
                    .expect("review requests");
                panic!(
                    "timed out before FinalConfirm: status={:?} stage={:?} current={:?} requests={requests:?}",
                    attempt.status, attempt.stage, attempt.current_work_item_id,
                );
            }
        }
    }
    assert!(
        saw_failed_review_request,
        "expected push to fail and emit a Failed review request"
    );
    assert!(
        reached_human_final_confirm,
        "group must reach human FinalConfirm even when review_request push fails"
    );
    let requests = store
        .list_review_requests("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("review requests");
    let request = requests.last().expect("review request persisted");
    assert_eq!(request.push_status, PushStatus::Failed);
    assert!(
        request.push_error.is_some(),
        "push_error should be recorded on the failed review request"
    );
    let attempt = store
        .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("attempt");
    assert_eq!(attempt.status, CodingAttemptStatus::WaitingForHuman);
    assert_eq!(attempt.stage, CodingExecutionStage::FinalConfirm);
    assert!(
        store
            .list_internal_pr_reviews("project_0001", "issue_0001", "coding_attempt_0001")
            .expect("internal reviews")
            .is_empty(),
        "fresh groups must not persist provider group reviews"
    );

    ws.close(None).await.expect("close ws");
    server.abort();
}

#[cfg(unix)]
#[tokio::test]
async fn retry_push_after_runner_exit_falls_back_to_direct_handling() {
    use std::os::unix::fs::PermissionsExt;

    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let app = app_with_group_full_chain_attempt_and_provider(
        root.path(),
        Arc::new(IndependentCodeReviewPlanDefectProvider::approve_all()),
    );
    // 注入拒绝 push 的 hook，使第一次 review_request push 失败。
    let hook = root.path().join("remote.git/hooks/pre-receive");
    fs::write(&hook, "#!/bin/sh\nexit 1\n").expect("rejecting hook");
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).expect("chmod hook");

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/ws/coding-attempts/coding_attempt_0001");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;
    send_json(&mut ws, &CodingWsInMessage::StartCoding).await;

    let mut confirmed_gates = HashSet::new();
    let mut bound_first_handoff = false;
    let mut saw_failed_review_request = false;
    for _ in 0..320 {
        match timeout(Duration::from_secs(2), recv_json(&mut ws)).await {
            Ok(CodingWsOutMessage::CodingGateRequired { gate }) => {
                if gate.kind == CodingGateKind::StageGate
                    && let Some(stage) = gate.stage.clone()
                    && confirmed_gates.insert(gate.gate_id)
                {
                    send_json(&mut ws, &CodingWsInMessage::StageGateConfirm { stage }).await;
                }
            }
            Ok(CodingWsOutMessage::CodingSessionState {
                current_work_item_id,
                ..
            }) if current_work_item_id.as_deref() == Some("work_item_0002")
                && !bound_first_handoff =>
            {
                bind_completed_first_unit_handoff_revision(&store);
                bound_first_handoff = true;
            }
            Ok(CodingWsOutMessage::ReviewRequestUpdate { review_request }) => {
                if review_request.push_status == PushStatus::Failed {
                    saw_failed_review_request = true;
                }
            }
            Ok(CodingWsOutMessage::CodingSessionState { status, stage, .. })
                if status == CodingAttemptStatus::WaitingForHuman
                    && stage == CodingExecutionStage::FinalConfirm =>
            {
                break;
            }
            Ok(CodingWsOutMessage::CodingProtocolError { code, message }) => {
                panic!("unexpected coding protocol error {code}: {message}");
            }
            Ok(_) => {}
            Err(_) => panic!("timed out before FinalConfirm"),
        }
    }
    assert!(saw_failed_review_request, "first push must fail");

    // runner 已在 FinalConfirm 退出、command channel 关闭；移除 hook 让重推成功。
    fs::remove_file(&hook).expect("remove rejecting hook");

    send_json(&mut ws, &CodingWsInMessage::RetryPush).await;

    // 观察重推的响应（修复前此处完全静默），直到直接处理路径推送 session state
    // 或报错；超时继续等（execute_review_request 的 git push 可能较慢）。
    let mut saw_retry_response = false;
    let mut retried_pushed = false;
    let mut observed = Vec::new();
    for _ in 0..60 {
        match timeout(Duration::from_secs(2), recv_json(&mut ws)).await {
            Ok(CodingWsOutMessage::ReviewRequestUpdate { review_request }) => {
                saw_retry_response = true;
                observed.push(format!("review_request:{:?}", review_request.push_status));
                if review_request.push_status == PushStatus::Pushed {
                    retried_pushed = true;
                }
            }
            Ok(CodingWsOutMessage::CodingSessionState { status, stage, .. }) => {
                saw_retry_response = true;
                observed.push(format!("state:{status:?}:{stage:?}"));
                break;
            }
            Ok(CodingWsOutMessage::CodingProtocolError { code, message }) => {
                saw_retry_response = true;
                observed.push(format!("error:{code}:{message}"));
                break;
            }
            Ok(other) => {
                observed.push(format!("other:{other:?}"));
            }
            Err(_) => {}
        }
    }

    // 权威断言：以 store 为准，重推必须把 push_status 从 Failed 转为 Pushed，
    // 且 attempt stage 不得倒退（仍 FinalConfirm）。
    let requests = store
        .list_review_requests("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("review requests");
    let request = requests.last().expect("review request persisted");
    assert_eq!(
        request.push_status,
        PushStatus::Pushed,
        "RetryPush after runner exit must retry the push; observed={observed:?} saw_retry_response={saw_retry_response} retried_pushed={retried_pushed}"
    );
    let attempt_after = store
        .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("attempt after retry");
    assert_eq!(
        attempt_after.stage,
        CodingExecutionStage::FinalConfirm,
        "RetryPush must not regress the attempt stage from FinalConfirm"
    );

    ws.close(None).await.expect("close ws");
    server.abort();
}
