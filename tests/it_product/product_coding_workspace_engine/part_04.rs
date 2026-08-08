struct CountingProvider {
    starts: Arc<AtomicUsize>,
    seen_modes: Arc<Mutex<Vec<ProviderPermissionMode>>>,
    fail_on_start: bool,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for CountingProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        self.seen_modes
            .lock()
            .expect("seen modes lock")
            .push(input.permission_mode);
        if self.fail_on_start {
            return Err(ProviderAdapterError::execution_failed(
                None,
                String::new(),
                "pi failed",
                0,
            ));
        }
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel(8);
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::Completed(
                    cadence_aria::cross_cutting::streaming_provider::ProviderCompletion::plain(
                        "done",
                        Some("sess-1".into()),
                    ),
                ))
                .await;
        });
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }

    async fn run_streaming(
        &self,
        _input: &AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        Err(ProviderAdapterError::execution_failed(
            None,
            String::new(),
            "unused",
            0,
        ))
    }
}

#[tokio::test]
async fn pi_role_with_supervised_mode_normalized_to_auto() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");

    let seen_modes = Arc::new(Mutex::new(Vec::new()));
    let provider = CountingProvider {
        starts: Arc::new(AtomicUsize::new(0)),
        seen_modes: Arc::clone(&seen_modes),
        fail_on_start: false,
    };
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    let config = CodingRoleProviderConfigSnapshot {
        coder: ProviderName::Pi,
        code_reviewer: ProviderName::ClaudeCode,
        internal_reviewer: ProviderName::ClaudeCode,
        review_rounds: 1,
        permission_modes: CodingRolePermissionModes {
            coder: CodingProviderPermissionMode::Supervised,
            code_reviewer: CodingProviderPermissionMode::Auto,
            internal_reviewer: CodingProviderPermissionMode::Auto,
        },
    };
    store
        .update_role_provider_config_snapshot("project_0001", "issue_0001", &attempt.id, config)
        .expect("write pi config");
    engine
        .execute_coding(&attempt, &provider, &CodingExecutionContext::default())
        .await
        .expect("execute coding");

    let modes = seen_modes.lock().expect("seen modes lock");
    assert_eq!(
        modes[0],
        ProviderPermissionMode::Auto,
        "Pi role must normalize to Auto"
    );
}

#[tokio::test]
async fn pi_failure_does_not_trigger_fresh_retry() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");

    let pi_starts = Arc::new(AtomicUsize::new(0));
    let provider = CountingProvider {
        starts: Arc::clone(&pi_starts),
        seen_modes: Arc::new(Mutex::new(Vec::new())),
        fail_on_start: true,
    };
    let config = CodingRoleProviderConfigSnapshot {
        coder: ProviderName::Pi,
        code_reviewer: ProviderName::ClaudeCode,
        internal_reviewer: ProviderName::ClaudeCode,
        review_rounds: 1,
        permission_modes: CodingRolePermissionModes::default(),
    };
    store
        .update_role_provider_config_snapshot("project_0001", "issue_0001", &attempt.id, config)
        .expect("write pi config");
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    let result = engine
        .execute_coding(&attempt, &provider, &CodingExecutionContext::default())
        .await;

    assert!(result.is_err(), "Pi startup failure must return an error");
    assert_eq!(
        pi_starts.load(Ordering::SeqCst),
        1,
        "Pi starts only once without a fresh retry"
    );
}

#[tokio::test]
async fn coding_prompt_includes_rework_fix_hints() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    store
        .save_rework_instruction(&attempt, &CodingReworkInstruction {
            id: "coding_rework_instruction_0001".to_string(),
            attempt_id: attempt.id.clone(),
            source_stage: CodingExecutionStage::CodeReview,
            rework_round: 1,
            summary: "reviewer 要求移除运行产物".to_string(),
            fix_hints: vec!["移除 __pycache__ 和 .pyc 文件".to_string()],
            questions: vec!["确认 git diff 只包含业务文件".to_string()],
            created_at: "2026-05-29T00:00:00Z".to_string(),
            consumed_by_node_id: None,
            consumed_at: None,
        })
        .expect("save rework instruction");
    let captured_prompt = Arc::new(Mutex::new(None));
    let provider = PromptCapturingProvider {
        prompt: Arc::clone(&captured_prompt),
    };
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    engine
        .execute_coding(&attempt, &provider, &CodingExecutionContext::default())
        .await
        .expect("execute coding");

    let prompt = captured_prompt
        .lock()
        .expect("prompt lock")
        .clone()
        .expect("captured prompt");
    assert!(prompt.contains("上一轮修复要求"));
    assert!(prompt.contains("来源阶段: CodeReview"));
    assert!(prompt.contains("reviewer 要求移除运行产物"));
    assert!(prompt.contains("移除 __pycache__ 和 .pyc 文件"));
    assert!(prompt.contains("确认 git diff 只包含业务文件"));
    let consumed = store
        .latest_unconsumed_rework_instruction("project_0001", "issue_0001", &attempt.id)
        .expect("latest instruction");
    assert_eq!(consumed, None);
}

#[tokio::test]
async fn execute_coding_includes_unconsumed_context_notes_and_consumes_them() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let mut attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            ..create_input()
        })
        .expect("create attempt");
    attempt = store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    attempt = store
        .increment_attempt_rework_count("project_0001", "issue_0001", &attempt.id)
        .expect("first rework");
    let old_note = store
        .create_context_note(&attempt, "不要带入本轮 Coder prompt".to_string())
        .expect("old context note");
    store
        .mark_context_notes_consumed("project_0001", "issue_0001", &attempt.id, &[old_note.id], 1)
        .expect("consume old note");
    let new_note = store
        .create_context_note(
            &attempt,
            "请优先修复 provider_install SSE 订阅".to_string(),
        )
        .expect("new context note");
    let captured_prompt = Arc::new(Mutex::new(None));
    let provider = PromptCapturingProvider {
        prompt: Arc::clone(&captured_prompt),
    };
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    engine
        .execute_coding(&attempt, &provider, &CodingExecutionContext::default())
        .await
        .expect("execute coding");

    let prompt = captured_prompt
        .lock()
        .expect("prompt lock")
        .clone()
        .expect("captured prompt");
    assert!(prompt.contains("本轮补充上下文"));
    assert!(prompt.contains("请优先修复 provider_install SSE 订阅"));
    assert!(!prompt.contains("不要带入本轮 Coder prompt"));
    let notes = store
        .list_context_notes("project_0001", "issue_0001", &attempt.id)
        .expect("context notes");
    let consumed_note = notes
        .iter()
        .find(|note| note.id == new_note.id)
        .expect("new note persisted");
    assert_eq!(consumed_note.consumed_by_rework_round, Some(1));
}

#[tokio::test]
async fn execute_coding_emits_prompt_for_coder_provider() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_role_provider_config_snapshot(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingRoleProviderConfigSnapshot {
                coder: ProviderName::Codex,
                code_reviewer: ProviderName::Fake,
                internal_reviewer: ProviderName::Fake,
                review_rounds: 1,
                permission_modes: CodingRolePermissionModes::default(),
            },
        )
        .expect("set role provider snapshot");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let captured_input = Arc::new(Mutex::new(None));
    let provider = InputCapturingProvider {
        input: Arc::clone(&captured_input),
        output: "done".to_string(),
    };
    let (tx, mut rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);

    engine
        .execute_coding(&attempt, &provider, &CodingExecutionContext::default())
        .await
        .expect("execute coding");

    let _node = rx.recv().await.expect("coding node created");
    match rx.recv().await.expect("provider prompt event") {
        CodingWsOutMessage::CodingExecutionEvent { event } => {
            assert_eq!(event.agent, Some(ProviderName::Codex));
        }
        other => panic!("expected provider prompt event, got {other:?}"),
    }
    let input = captured_input
        .lock()
        .expect("input lock")
        .clone()
        .expect("captured input");
    assert_eq!(input.provider_type, ProviderType::Codex);
}

#[tokio::test]
async fn execute_coding_forwards_provider_execution_and_tool_events() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_role_provider_config_snapshot(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingRoleProviderConfigSnapshot {
                coder: ProviderName::Codex,
                code_reviewer: ProviderName::Fake,
                internal_reviewer: ProviderName::Fake,
                review_rounds: 1,
                permission_modes: CodingRolePermissionModes::default(),
            },
        )
        .expect("set role provider snapshot");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let provider = EventEmittingCodingProvider;
    let (tx, mut rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);

    engine
        .execute_coding(&attempt, &provider, &CodingExecutionContext::default())
        .await
        .expect("execute coding");

    let events = drain_events(&mut rx);
    let execution_events = events
        .iter()
        .filter_map(|event| match event {
            CodingWsOutMessage::CodingExecutionEvent { event } => Some(event),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert!(
        execution_events
            .iter()
            .any(|event| event.event_id == "command_0001"
                && event.agent == Some(ProviderName::Codex)
                && event.kind == WsExecutionEventKind::Command
                && event.status == WsExecutionEventStatus::Completed
                && event.title == "Run tests"
                && event.command.as_deref() == Some("uv run pytest")
                && event.output.as_deref() == Some("1 passed")),
        "expected command execution event, got {events:?}"
    );
    assert!(
        execution_events
            .iter()
            .any(|event| event.event_id == "tool_0001"
                && event.kind == WsExecutionEventKind::Command
                && event.status == WsExecutionEventStatus::Started
                && event.title == "run_command"
                && event
                    .detail
                    .as_deref()
                    .is_some_and(|detail| detail.contains("uv run pytest"))),
        "expected tool call execution event, got {events:?}"
    );
    assert!(
        execution_events
            .iter()
            .any(|event| event.event_id == "tool_0001"
                && event.kind == WsExecutionEventKind::Command
                && event.status == WsExecutionEventStatus::Completed
                && event.title == "run_command"
                && event.command.as_deref() == Some("uv run pytest")
                && event.output.as_deref() == Some("1 passed")),
        "expected tool result execution event, got {events:?}"
    );
}

#[tokio::test]
async fn execute_coding_persists_coder_role_run_and_full_output_chat_entry() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let provider = FileWritingStreamingProvider;
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    engine
        .execute_coding(&attempt, &provider, &CodingExecutionContext::default())
        .await
        .expect("execute coding");

    let role_runs = store
        .list_role_runs("project_0001", "issue_0001", &attempt.id)
        .expect("role runs");
    assert_eq!(role_runs.len(), 1);
    let role_run = &role_runs[0];
    assert_eq!(role_run.role, CodingProviderRole::Coder);
    assert_eq!(role_run.stage, CodingExecutionStage::Coding);
    assert_eq!(role_run.status, CodingRoleRunStatus::Completed);
    assert_eq!(role_run.trigger, CodingRoleRunTrigger::Initial);
    assert_eq!(role_run.node_id.as_deref(), Some("coding_node_0001"));
    assert_eq!(role_run.run_no, 1);
    assert!(role_run.completed_at.is_some());
    assert_eq!(
        role_run.raw_provider_output_refs,
        vec![
            "provider-raw/coding/provider_stream_attempt_0001.txt".to_string(),
            "provider-raw/coding/coder_output_0001.txt".to_string(),
        ]
    );

    let events = store
        .list_role_run_events("project_0001", "issue_0001", &attempt.id, &role_run.id)
        .expect("role run events");
    assert!(
        events
            .iter()
            .any(|event| event.event_type == CodingRoleRunEventType::ProviderPrompt)
    );
    assert!(
        events.iter().any(|event| {
            event.event_type == CodingRoleRunEventType::TextDelta
                && event.payload["content"] == "created generated.txt"
        }),
        "expected text delta event, got {events:?}"
    );
    assert!(
        events
            .iter()
            .any(|event| event.event_type == CodingRoleRunEventType::MessageComplete)
    );

    let chat_entries = store
        .list_chat_entries("project_0001", "issue_0001", &attempt.id)
        .expect("chat entries");
    assert_eq!(chat_entries.len(), 1);
    let entry = &chat_entries[0];
    assert_eq!(entry.node_id.as_deref(), Some("coding_node_0001"));
    assert_eq!(entry.role, CodingAgentRole::Author);
    assert_eq!(entry.entry_type, CodingEntryType::AssistantMessage);
    assert_eq!(entry.content.as_deref(), Some("done"));
    let metadata = entry.metadata.as_ref().expect("entry metadata");
    assert_eq!(metadata["source"], "coding");
    assert_eq!(metadata["provider"], "fake");
    assert_eq!(metadata["role_run_id"].as_str(), Some(role_run.id.as_str()));
    assert_eq!(metadata["run_no"], 1);
    assert_eq!(metadata["raw_provider_output_ref"], "provider-raw/coding/coder_output_0001.txt");
    assert_eq!(metadata["started_at"].as_str(), Some(role_run.started_at.as_str()));
    assert_eq!(
        metadata["completed_at"].as_str(),
        Some(role_run.completed_at.as_ref().expect("completed_at").as_str())
    );
}

#[tokio::test]
async fn execute_coding_forwards_provider_permission_choice_and_status_events() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let provider = ControlEventCodingProvider;
    let (tx, mut rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);

    let error = engine
        .execute_coding(&attempt, &provider, &CodingExecutionContext::default())
        .await
        .expect_err("unresolved provider choice should block completion");
    assert_eq!(
        error.to_string(),
        "coding_provider_stream_failed: provider_choice_unresolved"
    );

    let events = drain_events(&mut rx);
    assert!(
        events.iter().any(|event| {
            matches!(
                event,
                CodingWsOutMessage::CodingPermissionRequest {
                    id,
                    tool_name,
                    description,
                    ..
                } if id == "permission_0001"
                    && tool_name == "shell"
                    && description == "Run uv test command"
            )
        }),
        "expected permission request event, got {events:?}"
    );
    assert!(
        events.iter().any(|event| {
            matches!(
                event,
                CodingWsOutMessage::CodingChoiceRequest {
                    id,
                    prompt,
                    source,
                    options,
                    allow_multiple,
                    allow_free_text,
                } if id == "choice_0001"
                    && prompt == "Select implementation strategy"
                    && source == "provider_choice"
                    && options.len() == 1
                    && !allow_multiple
                    && *allow_free_text
            )
        }),
        "expected choice request event, got {events:?}"
    );
    assert!(
        events.iter().any(|event| {
            matches!(
                event,
                CodingWsOutMessage::CodingExecutionEvent { event }
                    if event.event_id == "coding_node_0001_provider_status_running"
                        && event.status == WsExecutionEventStatus::Running
                        && event.title == "Provider running"
            )
        }),
        "expected visible provider status event, got {events:?}"
    );
}

#[tokio::test]
async fn execute_coding_forwards_permission_responses_to_provider_session() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let provider = PermissionAwaitingProvider;
    let (event_tx, mut event_rx) = mpsc::channel(16);
    let (command_tx, mut command_rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), event_tx);
    let context = CodingExecutionContext::default();

    let execute =
        engine.execute_coding_with_commands(&attempt, &provider, &context, &mut command_rx);
    tokio::pin!(execute);
    let mut saw_permission_request = false;

    let updated = loop {
        tokio::select! {
            result = &mut execute => break result.expect("execute coding"),
            event = event_rx.recv() => {
                if matches!(
                    event,
                    Some(CodingWsOutMessage::CodingPermissionRequest { ref id, .. })
                        if id == "permission_0001"
                ) {
                    saw_permission_request = true;
                    command_tx
                        .send(cadence_aria::product::coding_workspace_runner::CodingRunnerCommand::PermissionResponse {
                            id: "permission_0001".to_string(),
                            approved: true,
                            reason: Some("approved by test".to_string()),
                        })
                        .await
                        .expect("send permission response");
                }
            }
        }
    };

    assert!(saw_permission_request);
    assert_eq!(updated.stage, CodingExecutionStage::Coding);
}

#[tokio::test]
async fn execute_coding_persists_provider_choice_and_resumes_after_response() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let provider = ChoiceAwaitingProvider;
    let (event_tx, mut event_rx) = mpsc::channel(16);
    let (command_tx, mut command_rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), event_tx);
    let context = CodingExecutionContext::default();

    let execute =
        engine.execute_coding_with_commands(&attempt, &provider, &context, &mut command_rx);
    tokio::pin!(execute);
    let mut saw_choice_request = false;

    let updated = loop {
        tokio::select! {
            result = &mut execute => break result.expect("execute coding"),
            event = event_rx.recv() => {
                if let Some(CodingWsOutMessage::CodingChoiceRequest {
                    id,
                    prompt,
                    source,
                    ..
                }) = event
                    && id == "choice_0001"
                {
                    saw_choice_request = true;
                    assert_eq!(prompt, "Select implementation strategy");
                    assert_eq!(source, "request_user_input");
                    let open = store
                        .list_open_choice_gates("project_0001", "issue_0001", &attempt.id)
                        .expect("open choice gates");
                    assert_eq!(open.len(), 1);
                    assert_eq!(open[0].choice_id, "choice_0001");
                    assert_eq!(open[0].status, CodingChoiceGateStatus::Open);
                    assert_eq!(
                        store
                            .get_attempt("project_0001", "issue_0001", &attempt.id)
                            .expect("attempt")
                            .status,
                        CodingAttemptStatus::WaitingForHuman
                    );
                    command_tx
                        .send(cadence_aria::product::coding_workspace_runner::CodingRunnerCommand::ChoiceResponse {
                            id: "choice_0001".to_string(),
                            selected_option_ids: vec!["backend_first".to_string()],
                            free_text: Some("先控制范围".to_string()),
                        })
                        .await
                        .expect("send choice response");
                }
            }
        }
    };

    assert!(saw_choice_request);
    assert_eq!(updated.stage, CodingExecutionStage::Coding);
    assert_eq!(
        store
            .list_open_choice_gates("project_0001", "issue_0001", &attempt.id)
            .expect("open choice gates")
            .len(),
        0
    );
}

#[tokio::test]
async fn execute_coding_blocks_when_provider_completes_before_choice_response() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let provider = ControlEventCodingProvider;
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    let error = engine
        .execute_coding(&attempt, &provider, &CodingExecutionContext::default())
        .await
        .expect_err("provider cannot complete with unresolved choice");

    assert_eq!(
        error.to_string(),
        "coding_provider_stream_failed: provider_choice_unresolved"
    );
    assert_eq!(
        store
            .get_attempt("project_0001", "issue_0001", &attempt.id)
            .expect("attempt")
            .status,
        CodingAttemptStatus::WaitingForHuman
    );
    assert_eq!(
        store
            .list_open_choice_gates("project_0001", "issue_0001", &attempt.id)
            .expect("open choice gates")
            .len(),
        1
    );
}

#[tokio::test]
async fn execute_coding_blocks_later_permission_before_choice_response() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let provider = ChoiceThenPermissionProvider;
    let (tx, mut rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    let error = engine
        .execute_coding(&attempt, &provider, &CodingExecutionContext::default())
        .await
        .expect_err("provider cannot request permission with unresolved choice");

    assert_eq!(
        error.to_string(),
        "coding_provider_stream_failed: provider_choice_unresolved"
    );
    assert_eq!(
        store
            .get_attempt("project_0001", "issue_0001", &attempt.id)
            .expect("attempt")
            .status,
        CodingAttemptStatus::WaitingForHuman
    );
    let events = drain_events(&mut rx);
    assert!(events.iter().any(|event| {
        matches!(
            event,
            CodingWsOutMessage::CodingChoiceRequest { id, .. } if id == "choice_0001"
        )
    }));
    assert!(
        !events.iter().any(|event| {
            matches!(
                event,
                CodingWsOutMessage::CodingPermissionRequest { id, .. } if id == "permission_0001"
            )
        }),
        "pending choice must block later permission requests"
    );
}

#[tokio::test]
async fn execute_coding_stops_forwarding_provider_events_after_abort_command() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let provider = PermissionAwaitingProvider;
    let (event_tx, mut event_rx) = mpsc::channel(16);
    let (command_tx, mut command_rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), event_tx);
    let context = CodingExecutionContext::default();

    let execute =
        engine.execute_coding_with_commands(&attempt, &provider, &context, &mut command_rx);
    tokio::pin!(execute);
    let mut abort_sent = false;

    let error = loop {
        tokio::select! {
            result = &mut execute => break result.expect_err("abort should stop coding execution"),
            event = event_rx.recv() => {
                if !abort_sent
                    && matches!(
                        event,
                        Some(CodingWsOutMessage::CodingPermissionRequest { ref id, .. })
                            if id == "permission_0001"
                    )
                {
                    abort_sent = true;
                    command_tx
                        .send(cadence_aria::product::coding_workspace_runner::CodingRunnerCommand::AbortAttempt)
                        .await
                        .expect("send abort");
                }
            }
        }
    };

    assert_eq!(error.to_string(), "coding_aborted");
    assert!(abort_sent);
}
