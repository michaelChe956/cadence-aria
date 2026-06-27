#[tokio::test]
async fn coding_ws_prepare_context_sends_work_item_context_and_updates_provider_selection() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let app = app_with_confirmed_work_item_context(root.path());
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/ws/coding-attempts/coding_attempt_0001");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");

    match recv_json(&mut ws).await {
        CodingWsOutMessage::CodingSessionState {
            work_item_markdown,
            verification_commands,
            provider_config_snapshot,
            ..
        } => {
            let markdown = work_item_markdown.expect("work item markdown");
            assert!(markdown.contains("实现爬楼梯问题"));
            assert!(markdown.contains("climb_stairs"));
            assert_eq!(
                verification_commands.as_ref(),
                &vec!["uv run python -m unittest discover -s tests -v".to_string()]
            );
            assert_eq!(provider_config_snapshot.author, ProviderName::Fake);
        }
        other => panic!("expected coding session state, got {other:?}"),
    }

    send_json(
        &mut ws,
        &CodingWsInMessage::ProviderSelect {
            role: "author".to_string(),
            provider: ProviderName::Codex,
        },
    )
    .await;

    assert_eq!(
        recv_json(&mut ws).await,
        CodingWsOutMessage::CodingProviderConfigUpdated {
            role: CodingProviderRole::Coder,
            provider: ProviderName::Codex,
        }
    );
    match recv_json(&mut ws).await {
        CodingWsOutMessage::CodingSessionState {
            provider_config_snapshot,
            work_item_markdown,
            ..
        } => {
            assert_eq!(provider_config_snapshot.author, ProviderName::Codex);
            assert_eq!(provider_config_snapshot.reviewer, Some(ProviderName::Fake));
            assert!(
                work_item_markdown
                    .as_deref()
                    .unwrap_or_default()
                    .contains("实现爬楼梯问题")
            );
        }
        other => panic!("expected updated coding session state, got {other:?}"),
    }

    ws.close(None).await.expect("close ws");
    server.abort();
}

#[tokio::test]
async fn coding_ws_context_note_persists_and_echoes_chat_entry() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let app = app_with_attempt(root.path());
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
        &CodingWsInMessage::ContextNote {
            content: "请优先使用 unittest".to_string(),
        },
    )
    .await;

    match recv_json(&mut ws).await {
        CodingWsOutMessage::CodingChatEntryCreated { entry } => {
            assert_eq!(entry.id, "coding_chat_entry_0001");
            assert_eq!(entry.attempt_id, "coding_attempt_0001");
            assert_eq!(entry.node_id, None);
            assert_eq!(entry.role, CodingAgentRole::Author);
            assert_eq!(entry.entry_type, CodingEntryType::UserMessage);
            assert_eq!(entry.content.as_deref(), Some("请优先使用 unittest"));
            assert_eq!(
                entry.metadata.as_ref().and_then(|value| {
                    value
                        .get("context_note_id")
                        .and_then(|context_note_id| context_note_id.as_str())
                }),
                Some("coding_context_note_0001")
            );
        }
        other => panic!("expected coding chat entry echo, got {other:?}"),
    }

    let notes = store
        .list_context_notes("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("list context notes");
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].id, "coding_context_note_0001");
    assert_eq!(notes[0].content, "请优先使用 unittest");
    assert!(notes[0].consumed_by_rework_round.is_none());

    ws.close(None).await.expect("close ws");
    server.abort();
}

#[tokio::test]
async fn coding_ws_start_coding_pushes_engine_stage_and_timeline_events() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let app = app_with_attempt(root.path());
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/ws/coding-attempts/coding_attempt_0001");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;

    send_json(&mut ws, &CodingWsInMessage::StartCoding).await;

    assert_eq!(
        recv_json(&mut ws).await,
        CodingWsOutMessage::CodingStageChange {
            stage: CodingExecutionStage::WorktreePrepare
        }
    );
    match recv_json(&mut ws).await {
        CodingWsOutMessage::CodingTimelineNodeCreated { node } => {
            assert_eq!(node.id, "coding_node_0001");
            assert_eq!(node.stage, CodingExecutionStage::WorktreePrepare);
            assert_eq!(node.status, CodingTimelineNodeStatus::Running);
        }
        other => panic!("expected coding timeline node event, got {other:?}"),
    }
    let updated = store
        .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("updated attempt");
    assert_eq!(updated.status, CodingAttemptStatus::Running);
    assert_eq!(updated.stage, CodingExecutionStage::WorktreePrepare);

    ws.close(None).await.expect("close ws");
    server.abort();
}

#[tokio::test]
async fn coding_ws_start_coding_waits_at_stage_gate_and_confirm_resumes_runner() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let app = app_with_full_chain_attempt(root.path());
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/ws/coding-attempts/coding_attempt_0001");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;

    send_json(&mut ws, &CodingWsInMessage::StartCoding).await;

    let gate = wait_for_stage_gate(&mut ws, CodingExecutionStage::Coding).await;
    assert_eq!(gate.kind, CodingGateKind::StageGate);
    assert_eq!(gate.role, Some(CodingProviderRole::Coder));
    assert_eq!(
        gate.provider_snapshot
            .as_ref()
            .map(|snapshot| &snapshot.coder),
        Some(&ProviderName::Fake)
    );
    assert!(
        store
            .list_open_stage_gates("project_0001", "issue_0001", "coding_attempt_0001")
            .expect("open stage gates")
            .iter()
            .any(|gate| gate.stage == CodingExecutionStage::Coding)
    );

    send_json(
        &mut ws,
        &CodingWsInMessage::StageGateConfirm {
            stage: CodingExecutionStage::Coding,
        },
    )
    .await;

    let node = wait_for_timeline_node(&mut ws, CodingExecutionStage::Coding).await;
    assert_eq!(node.status, CodingTimelineNodeStatus::Running);

    ws.close(None).await.expect("close ws");
    server.abort();
}

#[tokio::test]
async fn coding_ws_provider_select_during_stage_gate_updates_roles_and_refreshes_gate() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let app = app_with_full_chain_attempt(root.path());
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/ws/coding-attempts/coding_attempt_0001");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;

    send_json(&mut ws, &CodingWsInMessage::StartCoding).await;

    let gate = wait_for_stage_gate(&mut ws, CodingExecutionStage::Coding).await;
    let original_expires_at = gate.expires_at.expect("gate expires_at");
    tokio::time::sleep(Duration::from_millis(20)).await;

    send_json(
        &mut ws,
        &CodingWsInMessage::ProviderSelect {
            role: "tester".to_string(),
            provider: ProviderName::Codex,
        },
    )
    .await;

    assert_eq!(
        wait_for_provider_config_update(&mut ws).await,
        CodingWsOutMessage::CodingProviderConfigUpdated {
            role: CodingProviderRole::Tester,
            provider: ProviderName::Codex,
        }
    );
    let refreshed_gate = wait_for_stage_gate(&mut ws, CodingExecutionStage::Coding).await;
    assert_ne!(
        refreshed_gate.expires_at.as_deref(),
        Some(original_expires_at.as_str())
    );
    assert_eq!(
        refreshed_gate
            .provider_snapshot
            .as_ref()
            .map(|snapshot| &snapshot.tester),
        Some(&ProviderName::Codex)
    );
    assert_eq!(
        store
            .get_role_provider_config_snapshot("project_0001", "issue_0001", "coding_attempt_0001")
            .expect("role provider snapshot")
            .tester,
        ProviderName::Codex
    );

    ws.close(None).await.expect("close ws");
    server.abort();
}

#[tokio::test]
async fn coding_ws_permission_mode_select_updates_role_config() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let app = app_with_attempt(root.path());
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
        &CodingWsInMessage::PermissionModeSelect {
            role: "tester".to_string(),
            permission_mode: CodingProviderPermissionMode::Supervised,
        },
    )
    .await;

    assert_eq!(
        wait_for_provider_config_update(&mut ws).await,
        CodingWsOutMessage::CodingProviderConfigUpdated {
            role: CodingProviderRole::Tester,
            provider: ProviderName::Fake,
        }
    );
    match recv_json(&mut ws).await {
        CodingWsOutMessage::CodingSessionState {
            role_provider_config_snapshot,
            ..
        } => {
            assert_eq!(
                role_provider_config_snapshot.permission_mode_for_role(&CodingProviderRole::Tester),
                CodingProviderPermissionMode::Supervised
            );
        }
        other => panic!("expected updated coding session state, got {other:?}"),
    }

    let snapshot = store
        .get_role_provider_config_snapshot("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("role config");
    assert_eq!(
        snapshot.permission_mode_for_role(&CodingProviderRole::Tester),
        CodingProviderPermissionMode::Supervised
    );

    ws.close(None).await.expect("close ws");
    server.abort();
}

#[tokio::test]
async fn coding_ws_stage_gate_timeout_auto_starts_stage() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let app = app_with_full_chain_attempt(root.path());
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/ws/coding-attempts/coding_attempt_0001");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;

    send_json(&mut ws, &CodingWsInMessage::StartCoding).await;

    let gate = wait_for_stage_gate(&mut ws, CodingExecutionStage::Coding).await;
    let node = wait_for_timeline_node(&mut ws, CodingExecutionStage::Coding).await;
    assert_eq!(node.status, CodingTimelineNodeStatus::Running);
    let gates = store
        .list_stage_gates("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("list stage gates");
    let expired = gates
        .iter()
        .find(|candidate| candidate.gate_id == gate.gate_id)
        .expect("expired gate");
    assert_eq!(
        expired.status,
        cadence_aria::product::coding_models::CodingStageGateStatus::Expired
    );

    ws.close(None).await.expect("close ws");
    server.abort();
}

#[tokio::test]
async fn coding_ws_provider_select_rejects_current_running_stage_role_without_gate() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let app = app_with_running_testing_attempt(root.path());
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
        &CodingWsInMessage::ProviderSelect {
            role: "tester".to_string(),
            provider: ProviderName::Codex,
        },
    )
    .await;

    match recv_json(&mut ws).await {
        CodingWsOutMessage::CodingProtocolError { code, .. } => {
            assert_eq!(code, "coding_provider_role_locked");
        }
        other => panic!("expected provider lock error, got {other:?}"),
    }

    ws.close(None).await.expect("close ws");
    server.abort();
}

#[tokio::test]
async fn coding_ws_abort_during_stage_gate_cancels_gate_before_snapshot() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let app = app_with_full_chain_attempt(root.path());
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/ws/coding-attempts/coding_attempt_0001");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;

    send_json(&mut ws, &CodingWsInMessage::StartCoding).await;
    let gate = wait_for_stage_gate(&mut ws, CodingExecutionStage::Coding).await;
    match recv_json(&mut ws).await {
        CodingWsOutMessage::CodingSessionState { pending_gates, .. } => {
            assert_eq!(pending_gates.len(), 1);
        }
        other => panic!("expected stage gate session state, got {other:?}"),
    }

    send_json(&mut ws, &CodingWsInMessage::AbortAttempt).await;

    match recv_json(&mut ws).await {
        CodingWsOutMessage::CodingSessionState {
            status,
            pending_gates,
            ..
        } => {
            assert_eq!(status, CodingAttemptStatus::Aborted);
            assert!(pending_gates.is_empty());
        }
        other => panic!("expected aborted session state, got {other:?}"),
    }
    let gates = store
        .list_stage_gates("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("list gates");
    let cancelled = gates
        .iter()
        .find(|candidate| candidate.gate_id == gate.gate_id)
        .expect("cancelled gate");
    assert_eq!(
        cancelled.status,
        cadence_aria::product::coding_models::CodingStageGateStatus::Cancelled
    );

    ws.close(None).await.expect("close ws");
    server.abort();
}

#[tokio::test]
async fn coding_ws_start_coding_keeps_socket_responsive_while_runner_is_active() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let app = app_with_hanging_coding_attempt(root.path());
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/ws/coding-attempts/coding_attempt_0001");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;

    send_json(&mut ws, &CodingWsInMessage::StartCoding).await;
    let _gate = wait_for_stage_gate(&mut ws, CodingExecutionStage::Coding).await;
    send_json(
        &mut ws,
        &CodingWsInMessage::StageGateConfirm {
            stage: CodingExecutionStage::Coding,
        },
    )
    .await;

    let mut saw_hanging_chunk = false;
    for _ in 0..8 {
        match recv_json(&mut ws).await {
            CodingWsOutMessage::CodingStreamChunk { content, .. }
                if content == "hanging provider started" =>
            {
                saw_hanging_chunk = true;
                break;
            }
            CodingWsOutMessage::CodingProtocolError { code, message } => {
                panic!("unexpected coding protocol error {code}: {message}");
            }
            _ => {}
        }
    }
    assert!(saw_hanging_chunk, "expected hanging provider to start");

    send_json(&mut ws, &CodingWsInMessage::CodingPing).await;
    assert_eq!(recv_json(&mut ws).await, CodingWsOutMessage::CodingPong);

    ws.close(None).await.expect("close ws");
    server.abort();
}

#[tokio::test]
async fn coding_ws_start_coding_drives_full_happy_path_to_final_confirm() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let app = app_with_full_chain_attempt(root.path());
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/ws/coding-attempts/coding_attempt_0001");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;

    send_json(&mut ws, &CodingWsInMessage::StartCoding).await;

    let mut stages = Vec::new();
    let mut final_snapshot_seen = false;
    let mut final_chat_entries = Vec::new();
    let mut confirmed_gates = HashSet::new();
    for _ in 0..80 {
        let message = recv_json(&mut ws).await;
        match message {
            CodingWsOutMessage::CodingTimelineNodeCreated { node } => {
                stages.push(node.stage);
            }
            CodingWsOutMessage::CodingGateRequired { gate } => {
                if respond_to_testing_result_review_gate(&mut ws, &gate).await {
                    continue;
                }
                assert_eq!(
                    gate.kind,
                    CodingGateKind::StageGate,
                    "unexpected non-stage gate: {:?} {:?}",
                    gate.reason_code,
                    gate.description
                );
                if let Some(stage) = gate.stage.clone()
                    && confirmed_gates.insert(gate.gate_id)
                {
                    send_json(&mut ws, &CodingWsInMessage::StageGateConfirm { stage }).await;
                }
            }
            CodingWsOutMessage::CodingSessionState {
                status,
                stage,
                chat_entries,
                ..
            } if status == CodingAttemptStatus::Completed
                && stage == CodingExecutionStage::FinalConfirm =>
            {
                final_chat_entries = *chat_entries;
                final_snapshot_seen = true;
                break;
            }
            CodingWsOutMessage::CodingProtocolError { code, message } => {
                panic!("unexpected coding protocol error {code}: {message}");
            }
            _ => {}
        }
    }

    assert!(
        final_snapshot_seen,
        "expected final_confirm snapshot over websocket"
    );
    for expected in [
        CodingExecutionStage::WorktreePrepare,
        CodingExecutionStage::Coding,
        CodingExecutionStage::Testing,
        CodingExecutionStage::Rework,
        CodingExecutionStage::CodeReview,
        CodingExecutionStage::ReviewRequest,
        CodingExecutionStage::InternalPrReview,
        CodingExecutionStage::FinalConfirm,
    ] {
        assert!(
            stages.contains(&expected),
            "missing timeline stage {expected:?}; got {stages:?}"
        );
    }
    assert_eq!(
        stages
            .iter()
            .filter(|stage| **stage == CodingExecutionStage::Rework)
            .count(),
        3,
        "expected rework after testing, code review, and internal review; got {stages:?}"
    );
    assert!(
        final_chat_entries
            .iter()
            .any(|entry| matches!(entry.entry_type, CodingEntryType::AnalystVerdict { .. })),
        "expected persisted analyst verdict chat entry"
    );
    assert!(
        final_chat_entries.iter().any(|entry| {
            entry
                .metadata
                .as_ref()
                .and_then(|value| value.get("source"))
                .and_then(|value| value.as_str())
                == Some("code_review")
        }),
        "expected persisted code review chat entry"
    );
    assert!(
        final_chat_entries.iter().any(|entry| {
            entry
                .metadata
                .as_ref()
                .and_then(|value| value.get("source"))
                .and_then(|value| value.as_str())
                == Some("internal_pr_review")
        }),
        "expected persisted internal PR review chat entry"
    );

    let attempt = store
        .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("updated attempt");
    assert_eq!(attempt.status, CodingAttemptStatus::Completed);
    assert_eq!(attempt.stage, CodingExecutionStage::FinalConfirm);
    let worktree = attempt.worktree_path.as_ref().expect("worktree path");
    assert_ne!(worktree, &root.path().join("repo"));
    assert!(worktree.join("src/lib.rs").is_file());

    let report = store
        .list_testing_reports("project_0001", "issue_0001", &attempt.id)
        .expect("testing reports")
        .pop()
        .expect("testing report");
    assert_eq!(report.overall_status, TestingOverallStatus::Passed);
    assert!(report.backend_verified);

    let review_request = store
        .list_review_requests("project_0001", "issue_0001", &attempt.id)
        .expect("review requests")
        .pop()
        .expect("review request");
    assert_eq!(review_request.push_status, PushStatus::Pushed);
    assert!(attempt.head_commit.is_some());

    assert_eq!(
        store
            .list_internal_pr_reviews("project_0001", "issue_0001", &attempt.id)
            .expect("internal reviews")
            .len(),
        1
    );

    ws.close(None).await.expect("close ws");
    server.abort();
}

