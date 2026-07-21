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
async fn legacy_coding_ws_reports_scope_mismatch_for_unique_corrupt_alias() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            project_id: "project_0002".to_string(),
            issue_id: "issue_0002".to_string(),
            work_item_id: "work_item_0002".to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/issues/issue_0002".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Fake,
                reviewer: Some(ProviderName::Fake),
                review_rounds: 1,
            },
            max_auto_rework: 2,
        })
        .expect("real attempt");
    let corrupt_alias_id = "coding_attempt_corrupt_alias";
    let corrupt_alias_path = root.path().join(format!(
        ".aria/projects/project_0001/issues/issue_0001/coding-attempts/{corrupt_alias_id}.json"
    ));
    fs::create_dir_all(corrupt_alias_path.parent().expect("parent"))
        .expect("create alias parent");
    fs::write(
        corrupt_alias_path,
        serde_json::to_vec_pretty(&attempt).expect("serialize attempt"),
    )
    .expect("write corrupt alias");
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move { axum::serve(listener, app).await.expect("serve") });
    let (mut ws, _) = connect_async(format!("ws://{addr}/ws/coding-attempts/{corrupt_alias_id}"))
        .await
        .expect("connect ws");

    match recv_json(&mut ws).await {
        CodingWsOutMessage::CodingProtocolError { code, .. } => {
            assert_eq!(code, "coding_attempt_scope_mismatch");
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
        .save_timeline_node(&attempt, CodingTimelineNode {
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
        .create_blocked_gate(&attempt, CreateBlockedGateInput {
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

    let mut saw_running_state = false;
    let mut saw_review_complete = false;
    let mut saw_review_request = false;
    for _ in 0..80 {
        let Ok(message) = timeout(Duration::from_millis(500), recv_json(&mut ws)).await else {
            continue;
        };
        match message {
            CodingWsOutMessage::CodingSessionState {
                project_id,
                issue_id,
                status,
                stage,
                ..
            } => {
                assert_eq!(project_id, "project_0001");
                assert_eq!(issue_id, "issue_0001");
                saw_running_state |= status == CodingAttemptStatus::Running;
                saw_review_request |= stage == CodingExecutionStage::ReviewRequest;
            }
            CodingWsOutMessage::CodeReviewComplete { .. } => saw_review_complete = true,
            CodingWsOutMessage::CodingStageChange {
                stage: CodingExecutionStage::ReviewRequest,
            } => saw_review_request = true,
            CodingWsOutMessage::CodingProtocolError { code, message } => {
                panic!("unexpected recovery protocol error {code}: {message}");
            }
            _ => {}
        }
        if saw_running_state && saw_review_complete && saw_review_request {
            break;
        }
    }
    assert!(saw_running_state, "expected recovered Running session state");
    assert!(saw_review_complete, "recovery runner did not complete code review");
    assert!(
        saw_review_request,
        "recovery runner did not advance to ReviewRequest"
    );
    assert_eq!(
        store
            .list_code_review_reports("project_0001", "issue_0001", "coding_attempt_0001")
            .expect("target code review reports")
            .len(),
        1
    );
    assert!(
        store
            .list_code_review_reports("project_0001", "issue_0002", "coding_attempt_0001")
            .expect("other code review reports")
            .is_empty()
    );
    server.abort();
}

struct ScopedSessionProvider;

#[async_trait::async_trait]
impl StreamingProviderAdapter for ScopedSessionProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel(8);
        tokio::spawn(async move {
            if cancel.is_cancelled() {
                return;
            }
            let _ = event_tx
                .send(ProviderEvent::Completed(ProviderCompletion::plain(
                    "scoped coder output",
                    Some("scoped-coder-session".to_string()),
                )))
                .await;
        });
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }
}

#[tokio::test]
async fn scoped_start_coding_persists_target_provider_output_and_conversation() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let app = app_with_full_chain_attempt_and_provider(root.path(), Arc::new(ScopedSessionProvider));
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let mut duplicate = store
        .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("attempt");
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
    send_json(&mut ws, &CodingWsInMessage::StartCoding).await;

    let mut confirmed_coding_gate = false;
    let mut reached_next_stage_gate = false;
    for _ in 0..80 {
        let Ok(message) = timeout(Duration::from_millis(500), recv_json(&mut ws)).await else {
            continue;
        };
        match message {
            CodingWsOutMessage::CodingGateRequired { gate }
                if gate.kind == CodingGateKind::StageGate
                    && gate.stage == Some(CodingExecutionStage::Coding) =>
            {
                confirmed_coding_gate = true;
                send_json(
                    &mut ws,
                    &CodingWsInMessage::StageGateConfirm {
                        stage: CodingExecutionStage::Coding,
                    },
                )
                .await;
            }
            CodingWsOutMessage::CodingGateRequired { gate }
                if gate.kind == CodingGateKind::StageGate
                    && gate.stage == Some(CodingExecutionStage::CodeReview) =>
            {
                reached_next_stage_gate = true;
                break;
            }
            CodingWsOutMessage::CodingProtocolError { code, message } => {
                panic!("unexpected scoped provider persistence error {code}: {message}");
            }
            _ => {}
        }
    }
    assert!(confirmed_coding_gate, "missing scoped Coding stage gate");
    assert!(
        reached_next_stage_gate,
        "runner did not reach the CodeReview stage gate after coder completion"
    );

    let target = store
        .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("target attempt");
    let other = store
        .get_attempt("project_0001", "issue_0002", "coding_attempt_0001")
        .expect("other attempt");
    assert!(target.provider_conversations.iter().any(|conversation| {
        conversation.provider_session_id == "scoped-coder-session"
    }));
    assert!(other.provider_conversations.is_empty());
    let coder_run = store
        .list_role_runs("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("target role runs")
        .into_iter()
        .find(|run| run.role == CodingProviderRole::Coder)
        .expect("coder role run");
    let raw_ref = coder_run
        .raw_provider_output_refs
        .first()
        .expect("coder raw output ref");
    let target_raw = root
        .path()
        .join(".aria/projects/project_0001/issues/issue_0001/coding-attempts/coding_attempt_0001")
        .join(raw_ref);
    let other_raw = root
        .path()
        .join(".aria/projects/project_0001/issues/issue_0002/coding-attempts/coding_attempt_0001")
        .join(raw_ref);
    assert_eq!(
        std::fs::read_to_string(target_raw).expect("target raw output"),
        "scoped coder output"
    );
    assert!(!other_raw.exists());
    server.abort();
}

#[tokio::test]
async fn scoped_context_note_updates_only_target_issue_for_legacy_duplicate() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let app = app_with_attempt(root.path());
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let mut duplicate = store
        .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("attempt");
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
        &CodingWsInMessage::ContextNote {
            content: "只写入 issue_0001".to_string(),
        },
    )
    .await;

    match recv_json(&mut ws).await {
        CodingWsOutMessage::CodingChatEntryCreated { entry } => {
            assert_eq!(entry.attempt_id, "coding_attempt_0001");
            assert_eq!(entry.content.as_deref(), Some("只写入 issue_0001"));
        }
        other => panic!("expected target issue chat entry, got {other:?}"),
    }
    let target_notes = store
        .list_context_notes("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("target notes");
    let other_notes = store
        .list_context_notes("project_0001", "issue_0002", "coding_attempt_0001")
        .expect("other notes");
    let target_entries = store
        .list_chat_entries("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("target entries");
    let other_entries = store
        .list_chat_entries("project_0001", "issue_0002", "coding_attempt_0001")
        .expect("other entries");
    assert_eq!(target_notes.len(), 1);
    assert_eq!(target_entries.len(), 1);
    assert!(other_notes.is_empty());
    assert!(other_entries.is_empty());
    server.abort();
}

#[tokio::test]
async fn scoped_start_coding_persists_target_timeline_without_partial_state() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let app = app_with_full_chain_attempt(root.path());
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let mut duplicate = store
        .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("attempt");
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
    send_json(&mut ws, &CodingWsInMessage::StartCoding).await;

    let mut saw_stage_change = false;
    let mut saw_timeline_node = false;
    let mut saw_coding_gate = false;
    for _ in 0..40 {
        let Ok(message) = timeout(Duration::from_millis(500), recv_json(&mut ws)).await else {
            continue;
        };
        match message {
            CodingWsOutMessage::CodingStageChange {
                stage: CodingExecutionStage::WorktreePrepare,
            } => saw_stage_change = true,
            CodingWsOutMessage::CodingTimelineNodeCreated { node }
                if node.stage == CodingExecutionStage::WorktreePrepare
                    && node.status == CodingTimelineNodeStatus::Running =>
            {
                saw_timeline_node = true;
            }
            CodingWsOutMessage::CodingGateRequired { gate }
                if gate.kind == CodingGateKind::StageGate
                    && gate.stage == Some(CodingExecutionStage::Coding) =>
            {
                saw_coding_gate = true;
            }
            CodingWsOutMessage::CodingProtocolError { code, message } => {
                panic!("unexpected scoped start protocol error {code}: {message}");
            }
            _ => {}
        }
        if saw_stage_change && saw_timeline_node && saw_coding_gate {
            break;
        }
    }

    let target = store
        .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("target attempt");
    let other = store
        .get_attempt("project_0001", "issue_0002", "coding_attempt_0001")
        .expect("other attempt");
    let target_nodes = store
        .get_timeline_nodes("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("target nodes");
    let other_nodes = store
        .get_timeline_nodes("project_0001", "issue_0002", "coding_attempt_0001")
        .expect("other nodes");
    assert_eq!(
        (target.status.clone(), target.stage.clone(), target_nodes.len()),
        (
            CodingAttemptStatus::Running,
            CodingExecutionStage::WorktreePrepare,
            1,
        ),
        "target attempt must not persist a stage transition without its timeline node",
    );
    assert!(saw_stage_change, "expected scoped stage change");
    assert!(saw_timeline_node, "expected scoped timeline node");
    assert!(saw_coding_gate, "expected scoped Coding stage gate");
    assert_eq!(target_nodes[0].stage, CodingExecutionStage::WorktreePrepare);
    assert_eq!(other.status, CodingAttemptStatus::Created);
    assert_eq!(other.stage, CodingExecutionStage::PrepareContext);
    assert!(other_nodes.is_empty());
    server.abort();
}

#[tokio::test]
async fn scoped_abort_notifies_only_target_issue_runner_for_legacy_duplicate() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let _setup = app_with_attempt(root.path());
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let mut duplicate = store
        .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("attempt");
    duplicate.issue_id = "issue_0002".to_string();
    store
        .save_coding_attempt(&duplicate)
        .expect("duplicate legacy attempt");
    let state = WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    );
    let first_key = CodingAttemptRunKey::new(
        "project_0001",
        "issue_0001",
        "coding_attempt_0001",
    );
    let second_key = CodingAttemptRunKey::new(
        "project_0001",
        "issue_0002",
        "coding_attempt_0001",
    );
    let (first_tx, mut first_rx) = mpsc::channel(1);
    let (second_tx, mut second_rx) = mpsc::channel(1);
    state
        .coding_runs
        .insert_cancellable(&first_key, first_tx)
        .expect("first runner");
    let second_run_id = state
        .coding_runs
        .insert_cancellable(&second_key, second_tx)
        .expect("second runner")
        .run_id();
    let app = build_web_router(state.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move { axum::serve(listener, app).await.expect("serve") });

    let url = format!(
        "ws://{addr}/ws/projects/project_0001/issues/issue_0002/coding-attempts/coding_attempt_0001"
    );
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    send_json(&mut ws, &CodingWsInMessage::AbortAttempt).await;

    assert_eq!(
        timeout(Duration::from_secs(1), second_rx.recv())
            .await
            .expect("target runner abort timeout")
            .expect("target runner abort"),
        CodingRunnerCommand::AbortAttempt,
    );
    assert!(first_rx.try_recv().is_err());
    assert_eq!(state.coding_runs.runner_count(&first_key), 1);
    assert_eq!(state.coding_runs.runner_count(&second_key), 1);
    state.coding_runs.remove(&second_key, second_run_id);
    assert_eq!(state.coding_runs.runner_count(&second_key), 0);
    timeout(Duration::from_secs(1), async {
        loop {
            let target = store
                .get_attempt("project_0001", "issue_0002", "coding_attempt_0001")
                .expect("target attempt");
            if target.status == CodingAttemptStatus::Aborted {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("target attempt abort timeout");
    let untouched = store
        .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("untouched attempt");
    assert_eq!(untouched.status, CodingAttemptStatus::Created);
    server.abort();
}
