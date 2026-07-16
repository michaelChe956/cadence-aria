#[tokio::test]
async fn scoped_coding_ws_returns_session_state_for_exact_issue() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let app = app_with_group_attempt(root.path());
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move { axum::serve(listener, app).await.expect("serve") });

    let url = format!(
        "ws://{addr}/ws/projects/project_0001/issues/issue_0001/coding-attempts/coding_attempt_0001"
    );
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    match recv_json(&mut ws).await {
        CodingWsOutMessage::CodingSessionState {
            project_id,
            issue_id,
            attempt_id,
            ..
        } => {
            assert_eq!(project_id, "project_0001");
            assert_eq!(issue_id, "issue_0001");
            assert_eq!(attempt_id, "coding_attempt_0001");
        }
        other => panic!("expected session state, got {other:?}"),
    }
    server.abort();
}

#[tokio::test]
async fn legacy_coding_ws_reports_ambiguous_instead_of_not_found() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let template = store
        .create_attempt(CreateCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: "work_item_0001".to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Fake,
                reviewer: Some(ProviderName::Fake),
                review_rounds: 1,
            },
            max_auto_rework: 2,
        })
        .expect("template");
    for issue_id in ["issue_0001", "issue_0002"] {
        let mut legacy = template.clone();
        legacy.id = "coding_attempt_0001".to_string();
        legacy.issue_id = issue_id.to_string();
        store.save_coding_attempt(&legacy).expect("legacy attempt");
    }
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move { axum::serve(listener, app).await.expect("serve") });
    let (mut ws, _) = connect_async(format!(
        "ws://{addr}/ws/coding-attempts/coding_attempt_0001"
    ))
    .await
    .expect("connect ws");

    match recv_json(&mut ws).await {
        CodingWsOutMessage::CodingProtocolError { code, .. } => {
            assert_eq!(code, "coding_attempt_ambiguous");
        }
        other => panic!("expected protocol error, got {other:?}"),
    }
    server.abort();
}

#[tokio::test]
async fn scoped_coding_ws_reports_not_found_for_missing_attempt() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let app = app_with_group_attempt(root.path());
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move { axum::serve(listener, app).await.expect("serve") });

    let url = format!(
        "ws://{addr}/ws/projects/project_0001/issues/issue_0001/coding-attempts/coding_attempt_missing"
    );
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    match recv_json(&mut ws).await {
        CodingWsOutMessage::CodingProtocolError { code, .. } => {
            assert_eq!(code, "coding_attempt_not_found");
        }
        other => panic!("expected protocol error, got {other:?}"),
    }
    server.abort();
}

#[tokio::test]
async fn scoped_coding_ws_reports_scope_mismatch_for_corrupt_identity() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: "work_item_0001".to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Fake,
                reviewer: Some(ProviderName::Fake),
                review_rounds: 1,
            },
            max_auto_rework: 2,
        })
        .expect("attempt");
    let mut corrupt = attempt.clone();
    corrupt.issue_id = "issue_0002".to_string();
    let attempt_path = root
        .path()
        .join(".aria/projects/project_0001/issues/issue_0001/coding-attempts")
        .join(format!("{}.json", attempt.id));
    serde_json::to_writer_pretty(
        std::fs::File::create(attempt_path).expect("open attempt"),
        &corrupt,
    )
    .expect("write corrupt attempt");
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move { axum::serve(listener, app).await.expect("serve") });

    let url = format!(
        "ws://{addr}/ws/projects/project_0001/issues/issue_0001/coding-attempts/{}",
        attempt.id
    );
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    match recv_json(&mut ws).await {
        CodingWsOutMessage::CodingProtocolError { code, .. } => {
            assert_eq!(code, "coding_attempt_scope_mismatch");
        }
        other => panic!("expected protocol error, got {other:?}"),
    }
    server.abort();
}

#[tokio::test]
async fn scoped_coding_ws_keeps_exact_identity_for_business_messages() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let app = app_with_attempt(root.path());
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("attempt");
    let mut duplicate = attempt.clone();
    duplicate.issue_id = "issue_0002".to_string();
    store
        .save_coding_attempt(&duplicate)
        .expect("duplicate legacy attempt");
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move { axum::serve(listener, app).await.expect("serve") });

    let url = format!(
        "ws://{addr}/ws/projects/project_0001/issues/issue_0001/coding-attempts/coding_attempt_0001"
    );
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    match recv_json(&mut ws).await {
        CodingWsOutMessage::CodingSessionState {
            project_id,
            issue_id,
            ..
        } => {
            assert_eq!(project_id, "project_0001");
            assert_eq!(issue_id, "issue_0001");
        }
        other => panic!("expected session state, got {other:?}"),
    }

    send_json(
        &mut ws,
        &CodingWsInMessage::MaxAutoReworkSelect { max_auto_rework: 3 },
    )
    .await;
    match recv_json(&mut ws).await {
        CodingWsOutMessage::CodingSessionState {
            project_id,
            issue_id,
            max_auto_rework,
            ..
        } => {
            assert_eq!(project_id, "project_0001");
            assert_eq!(issue_id, "issue_0001");
            assert_eq!(max_auto_rework, 3);
        }
        other => panic!("expected updated session state, got {other:?}"),
    }
    server.abort();
}

#[tokio::test]
async fn scoped_coding_ws_keeps_exact_identity_for_failed_review_recovery() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let captured = Arc::new(Mutex::new(Vec::new()));
    let (app, store) = app_with_coding_attempt(
        root.path(),
        Arc::new(RetryInternalReviewCaptureProvider {
            captured_prompts: captured,
        }),
    );
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            "coding_attempt_0001",
            CodingAttemptStatus::Running,
        )
        .expect("running");
    store
        .update_attempt_stage(
            "project_0001",
            "issue_0001",
            "coding_attempt_0001",
            CodingExecutionStage::CodeReview,
        )
        .expect("code review");
    let attempt = store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            "coding_attempt_0001",
            CodingAttemptStatus::Blocked,
        )
        .expect("blocked");
    store
        .save_timeline_node(CodingTimelineNode {
            id: "coding_node_0009".to_string(),
            attempt_id: attempt.id.clone(),
            stage: CodingExecutionStage::CodeReview,
            title: "代码审查".to_string(),
            status: CodingTimelineNodeStatus::Failed,
            agent_role: Some(CodingAgentRole::Reviewer),
            summary: Some("review provider interrupted".to_string()),
            started_at: "2026-07-16T00:00:00Z".to_string(),
            completed_at: Some("2026-07-16T00:00:01Z".to_string()),
            artifact_refs: Vec::new(),
        })
        .expect("failed review node");
    let role_run = store
        .create_role_run(
            &attempt,
            CodingExecutionStage::CodeReview,
            CodingProviderRole::CodeReviewer,
            CodingRoleRunTrigger::Initial,
            Some("coding_node_0009".to_string()),
        )
        .expect("review role run");
    store
        .update_role_run_status(
            "project_0001",
            "issue_0001",
            "coding_attempt_0001",
            &role_run.id,
            CodingRoleRunStatus::Failed,
            Some("code_review_provider_interrupted".to_string()),
        )
        .expect("failed review role run");
    let gate = store
        .create_blocked_gate(CreateBlockedGateInput {
            attempt_id: "coding_attempt_0001".to_string(),
            stage: CodingExecutionStage::CodeReview,
            node_id: Some("coding_node_0009".to_string()),
            role: Some(CodingProviderRole::CodeReviewer),
            title: "代码审查中断".to_string(),
            description: "review provider interrupted".to_string(),
            reason_code: Some("code_review_provider_interrupted".to_string()),
            evidence_refs: Vec::new(),
            raw_provider_output_ref: None,
            available_actions: vec![CodingGateAction {
                action_id: "retry_review".to_string(),
                label: "重试代码审查".to_string(),
                action_type: CodingGateActionType::RetryReview,
            }],
        })
        .expect("recovery gate");
    let mut duplicate = store
        .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("attempt")
        .clone();
    duplicate.issue_id = "issue_0002".to_string();
    store
        .save_coding_attempt(&duplicate)
        .expect("duplicate legacy attempt");
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move { axum::serve(listener, app).await.expect("serve") });

    let url = format!(
        "ws://{addr}/ws/projects/project_0001/issues/issue_0001/coding-attempts/coding_attempt_0001"
    );
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;
    send_json(
        &mut ws,
        &CodingWsInMessage::GateResponse {
            gate_id: gate.gate_id,
            action_id: "retry_review".to_string(),
            extra_context: None,
        },
    )
    .await;

    match recv_json(&mut ws).await {
        CodingWsOutMessage::CodingSessionState {
            project_id,
            issue_id,
            status,
            ..
        } => {
            assert_eq!(project_id, "project_0001");
            assert_eq!(issue_id, "issue_0001");
            assert_eq!(status, CodingAttemptStatus::Running);
        }
        other => panic!("expected recovered session state, got {other:?}"),
    }
    server.abort();
}
