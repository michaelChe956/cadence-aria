#[derive(Debug, Clone, Copy)]
enum RepairTerminalMode {
    StartError,
    EmptyCompletion,
    Failed,
    PermissionTimeout,
    StreamClosed,
    WaitForAbort,
}

#[derive(Clone)]
struct RepairTerminalProvider {
    mode: RepairTerminalMode,
    first_session_id: Option<String>,
    starts: Arc<AtomicUsize>,
    resume_provider_session_ids: Arc<Mutex<Vec<Option<String>>>>,
    held_event_senders: Arc<Mutex<Vec<mpsc::Sender<ProviderEvent>>>>,
}

impl RepairTerminalProvider {
    fn new(mode: RepairTerminalMode, first_session_id: Option<String>) -> Self {
        Self {
            mode,
            first_session_id,
            starts: Arc::new(AtomicUsize::new(0)),
            resume_provider_session_ids: Arc::new(Mutex::new(Vec::new())),
            held_event_senders: Arc::new(Mutex::new(Vec::new())),
        }
    }

    async fn session_with_event(event: Option<ProviderEvent>) -> ProviderSession {
        let (event_tx, event_rx) = mpsc::channel(4);
        let (command_tx, _command_rx) = mpsc::channel(4);
        if let Some(event) = event {
            event_tx.send(event).await.unwrap();
        }
        drop(event_tx);
        ProviderSession {
            events: event_rx,
            commands: command_tx,
        }
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for RepairTerminalProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let start = self.starts.fetch_add(1, Ordering::SeqCst) + 1;
        self.resume_provider_session_ids
            .lock()
            .unwrap()
            .push(input.resume_provider_session_id.clone());
        if start == 1 {
            let output = missing_json_nonce_output(
                r#"{"verdict":"revise","summary":"必须修复","findings":[{"severity":"must_fix","message":"缺少失败路径","evidence":"当前产物遗漏","required_action":"补充失败路径"}]}"#,
            );
            let output = input
                .structured_output_contract
                .as_ref()
                .map(|contract| output.replace("__NONCE__", &contract.nonce))
                .unwrap_or(output);
            let completion = ProviderCompletion::from_output(
                output,
                input.structured_output_contract.as_ref(),
                self.first_session_id.clone(),
            );
            return Ok(Self::session_with_event(Some(ProviderEvent::Completed(completion))).await);
        }

        match self.mode {
            RepairTerminalMode::StartError => Err(ProviderAdapterError::execution_failed(
                None,
                String::new(),
                "repair provider start failed",
                1,
            )),
            RepairTerminalMode::EmptyCompletion => {
                Ok(Self::session_with_event(Some(ProviderEvent::Completed(
                    ProviderCompletion::plain(String::new(), Some("repair-session-2".to_string())),
                )))
                .await)
            }
            RepairTerminalMode::Failed => {
                Ok(Self::session_with_event(Some(ProviderEvent::Failed {
                    message: "repair provider failed".to_string(),
                }))
                .await)
            }
            RepairTerminalMode::PermissionTimeout => {
                let (event_tx, event_rx) = mpsc::channel(4);
                let (command_tx, _command_rx) = mpsc::channel(4);
                event_tx
                    .send(ProviderEvent::PermissionRequest(
                        crate::cross_cutting::streaming_provider::PermissionRequestData {
                            id: "permission-repair".to_string(),
                            tool_name: "shell".to_string(),
                            description: "run repair validation".to_string(),
                            risk_level: crate::cross_cutting::streaming_provider::RiskLevel::Medium,
                        },
                    ))
                    .await
                    .unwrap();
                event_tx
                    .send(ProviderEvent::PermissionTimeout {
                        permission_id: "permission-repair".to_string(),
                    })
                    .await
                    .unwrap();
                drop(event_tx);
                Ok(ProviderSession {
                    events: event_rx,
                    commands: command_tx,
                })
            }
            RepairTerminalMode::StreamClosed => Ok(Self::session_with_event(None).await),
            RepairTerminalMode::WaitForAbort => {
                let (event_tx, event_rx) = mpsc::channel(4);
                let (command_tx, _command_rx) = mpsc::channel(4);
                self.held_event_senders.lock().unwrap().push(event_tx);
                Ok(ProviderSession {
                    events: event_rx,
                    commands: command_tx,
                })
            }
        }
    }

    async fn run_streaming(
        &self,
        _input: &AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        Err(ProviderAdapterError::execution_failed(
            None,
            String::new(),
            "run_streaming is not used by WorkspaceEngine",
            0,
        ))
    }
}

async fn persistent_queued_review_engine_for(
    _session_id: &str,
    workspace_type: WorkspaceType,
    artifact: ArtifactPayload,
) -> (
    TempDir,
    Arc<CheckpointStore>,
    LifecycleStore,
    WorkspaceEngine,
    mpsc::Receiver<EngineEvent>,
    String,
) {
    let (tmp, checkpoint_store) = setup();
    let lifecycle_store = LifecycleStore::new(ProductAppPaths::new(tmp.path().join(".aria")));
    let entity_id = match workspace_type {
        WorkspaceType::Story => "story_spec_0001",
        WorkspaceType::Design => "design_spec_0001",
        WorkspaceType::WorkItem => "work_item_0001",
        WorkspaceType::WorkItemPlan => "work_item_plan_0001",
    };
    let session_record = lifecycle_store
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: entity_id.to_string(),
            workspace_type,
            author_provider: ProviderName::ClaudeCode,
            reviewer_provider: ProviderName::Codex,
            review_rounds: 2,
            superpowers_enabled: true,
            openspec_enabled: true,
        })
        .expect("persistent review session");
    let mut session = WorkspaceSession::from_record(session_record);
    session.artifact = Some(artifact);
    let (tx, rx) = mpsc::channel(64);
    let mut engine = WorkspaceEngine::new_persistent(
        checkpoint_store.clone(),
        lifecycle_store.clone(),
        tx,
        session,
    );
    engine.start_review().await;
    let review_node_id = engine
        .active_node_id
        .clone()
        .expect("persistent active review node");
    (
        tmp,
        checkpoint_store,
        lifecycle_store,
        engine,
        rx,
        review_node_id,
    )
}

#[tokio::test]
async fn review_structured_output_repair_succeeds_for_all_general_workspace_types() {
    let review_json = r#"{
        "verdict": "revise",
        "summary": "补充失败路径",
        "findings": [{
            "severity": "must_fix",
            "message": "缺少失败路径",
            "evidence": "Artifact 未覆盖失败路径",
            "required_action": "补充失败路径说明"
        }]
    }"#;
    let cases = vec![
        (
            WorkspaceType::Story,
            artifact_payload(&complete_story_artifact("补充格式修复", "修复后可继续审核")),
        ),
        (
            WorkspaceType::Design,
            artifact_payload(&complete_design_artifact(
                "保持业务 payload",
                "复用 reviewer session",
            )),
        ),
        (
            WorkspaceType::WorkItem,
            artifact_payload(&complete_work_item_artifact("修复 reviewer 结构化输出")),
        ),
    ];

    for (workspace_type, artifact) in cases {
        let session_id = format!("sess_review_repair_success_{workspace_type:?}");
        let provider = QueuedReviewProvider::new(vec![
            missing_json_nonce_output(review_json),
            valid_structured_output(review_json),
        ]);
        let (_tmp, mut engine, mut rx, review_node_id) =
            queued_review_engine_for(&session_id, workspace_type, artifact).await;

        engine
            .drive_review_session(Arc::new(provider.clone()), empty_provider_commands())
            .await;

        assert_eq!(provider.starts.load(Ordering::SeqCst), 2);
        let review_nodes = engine
            .timeline_nodes
            .iter()
            .filter(|node| node.node_type == TimelineNodeType::ReviewerRun)
            .collect::<Vec<_>>();
        assert_eq!(review_nodes.len(), 1);
        assert_eq!(review_nodes[0].node_id, review_node_id);
        assert_eq!(review_nodes[0].round, Some(1));
        let prompts = provider.prompts.lock().unwrap();
        assert_eq!(prompts.len(), 2);
        assert!(prompts[1].contains("missing_json_nonce"));
        assert!(prompts[1].contains("Artifact 未覆盖失败路径"));
        assert!(prompts[1].contains("不得改变 verdict、summary、findings"));
        drop(prompts);
        assert_eq!(
            provider.resume_provider_session_ids.lock().unwrap()[1],
            Some("review-session-1".to_string())
        );

        let verdict = engine
            .latest_review_verdict
            .as_ref()
            .expect("repaired review verdict");
        assert_eq!(verdict.verdict, ReviewVerdictType::Revise);
        assert_eq!(verdict.findings.len(), 1);
        assert_eq!(verdict.findings[0].message, "缺少失败路径");
        let diagnostic = verdict
            .structured_output_diagnostic
            .as_ref()
            .expect("repair diagnostic");
        assert!(diagnostic.repair_attempted);
        assert!(diagnostic.repair_succeeded);
        assert!(diagnostic.raw_output_preview.is_none());
        assert_eq!(
            repair_event_statuses(&mut rx),
            vec![
                (
                    ProviderExecutionEventStatus::Started,
                    Some(review_node_id.clone())
                ),
                (
                    ProviderExecutionEventStatus::Completed,
                    Some(review_node_id)
                ),
            ]
        );
    }
}

#[tokio::test]
async fn repair_terminal_paths_close_started_event_as_failed() {
    let mut covered_workspace_terminal_cases = 0;
    for (workspace_type, artifact) in [
        (
            WorkspaceType::Story,
            artifact_payload(&complete_story_artifact(
                "repair failure must degrade safely",
                "review remains available for human triage",
            )),
        ),
        (
            WorkspaceType::Design,
            artifact_payload(&complete_design_artifact(
                "repair failure must preserve the review",
                "return a persisted diagnostic",
            )),
        ),
        (
            WorkspaceType::WorkItem,
            artifact_payload(&complete_work_item_artifact(
                "verify structured review repair failure fallback",
            )),
        ),
    ] {
        for mode in [
            RepairTerminalMode::StartError,
            RepairTerminalMode::EmptyCompletion,
            RepairTerminalMode::Failed,
            RepairTerminalMode::PermissionTimeout,
            RepairTerminalMode::StreamClosed,
        ] {
            let provider = RepairTerminalProvider::new(mode, Some("review-session-1".to_string()));
            let session_id = format!("sess_repair_terminal_{workspace_type:?}_{mode:?}");
            let (_tmp, checkpoint_store, lifecycle_store, mut engine, mut rx, review_node_id) =
                persistent_queued_review_engine_for(
                    &session_id,
                    workspace_type.clone(),
                    artifact.clone(),
                )
                .await;

            engine
                .drive_review_session(Arc::new(provider), empty_provider_commands())
                .await;

            // spec-design-dialog-revision T5：Story/Design repair 终态 review 完成统一回 AuthorConfirm
            // （报告进对话流，reviewer 结论不自动定稿）；WorkItem 维持既有 HumanConfirm 路由。
            match workspace_type {
                WorkspaceType::Story | WorkspaceType::Design => {
                    assert_eq!(
                        engine.session.stage,
                        WorkspaceStage::AuthorConfirm,
                        "{workspace_type:?} repair terminal mode {mode:?} must safely degrade 回 AuthorConfirm"
                    );
                }
                WorkspaceType::WorkItem => {
                    assert_eq!(
                        engine.session.stage,
                        WorkspaceStage::HumanConfirm,
                        "{workspace_type:?} repair terminal mode {mode:?} must safely degrade"
                    );
                }
                other => panic!("unexpected workspace type {other:?}"),
            }
            assert_eq!(
                engine
                    .timeline_nodes
                    .iter()
                    .find(|node| node.node_id == review_node_id)
                    .expect("completed review node")
                    .status,
                TimelineNodeStatus::Completed
            );
            let verdict = engine.latest_review_verdict.as_ref().unwrap_or_else(|| {
                panic!("{workspace_type:?} repair terminal mode {mode:?} verdict")
            });
            assert_eq!(verdict.verdict, ReviewVerdictType::NeedsHuman);
            let diagnostic = verdict
                .structured_output_diagnostic
                .as_ref()
                .unwrap_or_else(|| {
                    panic!("{workspace_type:?} repair terminal mode {mode:?} diagnostic")
                });
            assert_eq!(diagnostic.code, "missing_json_nonce");
            assert!(diagnostic.repair_attempted);
            assert!(!diagnostic.repair_succeeded);

            let persisted_session = lifecycle_store
                .get_workspace_session(&engine.session.session_id)
                .expect("persisted repair terminal session");
            assert_eq!(
                persisted_session.status,
                WorkspaceSessionStatus::WaitingForHuman
            );
            let persisted_detail = lifecycle_store
                .load_node_detail(&engine.session.session_id, &review_node_id)
                .expect("persisted repair review node detail");
            assert_eq!(persisted_detail.status, TimelineNodeStatus::Completed);
            let persisted_verdict: ReviewVerdict = serde_json::from_value(
                persisted_detail
                    .verdict
                    .expect("persisted repair fallback verdict"),
            )
            .expect("decode persisted repair fallback verdict");
            let persisted_diagnostic = persisted_verdict
                .structured_output_diagnostic
                .expect("persisted repair diagnostic");
            assert!(persisted_diagnostic.repair_attempted);
            assert!(!persisted_diagnostic.repair_succeeded);

            let (reload_tx, _reload_rx) = mpsc::channel(8);
            let reloaded = WorkspaceEngine::new_persistent(
                checkpoint_store,
                lifecycle_store,
                reload_tx,
                WorkspaceSession::from_record(persisted_session),
            );
            match workspace_type {
                WorkspaceType::Story | WorkspaceType::Design => {
                    assert_eq!(
                        reloaded.session.stage,
                        WorkspaceStage::AuthorConfirm,
                        "{workspace_type:?} repair terminal mode {mode:?} reload 后 stage 必须一致回 AuthorConfirm"
                    );
                }
                WorkspaceType::WorkItem => {
                    assert_eq!(reloaded.session.stage, WorkspaceStage::HumanConfirm);
                }
                other => panic!("unexpected workspace type {other:?}"),
            }
            assert_eq!(
                reloaded
                    .timeline_nodes
                    .iter()
                    .find(|node| node.node_id == review_node_id)
                    .expect("reloaded review node")
                    .status,
                TimelineNodeStatus::Completed
            );
            assert!(
                reloaded
                    .latest_review_verdict
                    .as_ref()
                    .and_then(|verdict| verdict.structured_output_diagnostic.as_ref())
                    .is_some_and(|diagnostic| {
                        diagnostic.repair_attempted && !diagnostic.repair_succeeded
                    })
            );
            assert_eq!(
                repair_event_statuses(&mut rx),
                vec![
                    (
                        ProviderExecutionEventStatus::Started,
                        Some(review_node_id.clone())
                    ),
                    (ProviderExecutionEventStatus::Failed, Some(review_node_id)),
                ],
                "{workspace_type:?} repair terminal mode {mode:?} execution event"
            );
            covered_workspace_terminal_cases += 1;
        }
    }
    assert_eq!(covered_workspace_terminal_cases, 15);
}

#[tokio::test]
async fn repair_permission_timeout_resolves_persisted_request_before_fallback_reload() {
    let provider = RepairTerminalProvider::new(
        RepairTerminalMode::PermissionTimeout,
        Some("review-session-1".to_string()),
    );
    let (_tmp, checkpoint_store, lifecycle_store, mut engine, _rx, review_node_id) =
        persistent_queued_review_engine_for(
            "sess_repair_permission_timeout_reload",
            WorkspaceType::Story,
            artifact_payload(&complete_story_artifact(
                "permission timeout must be persisted",
                "fallback review remains available",
            )),
        )
        .await;

    engine
        .drive_review_session(Arc::new(provider), empty_provider_commands())
        .await;

    // spec-design-dialog-revision T5：Story repair permission timeout 终态 review 完成回 AuthorConfirm（报告进对话流）。
    assert_eq!(engine.session.stage, WorkspaceStage::AuthorConfirm);
    assert_eq!(
        engine
            .timeline_nodes
            .iter()
            .find(|node| node.node_id == review_node_id)
            .expect("completed timeout review node")
            .status,
        TimelineNodeStatus::Completed
    );
    let detail = lifecycle_store
        .load_node_detail(&engine.session.session_id, &review_node_id)
        .expect("permission timeout review detail");
    assert_eq!(detail.permission_events.len(), 1);
    assert_eq!(
        detail.permission_events[0]
            .response
            .as_ref()
            .expect("timeout response")["status"],
        "timeout"
    );
    assert!(
        detail
            .permission_events
            .iter()
            .all(|event| event.response.is_some())
    );
    let verdict: ReviewVerdict = serde_json::from_value(
        detail
            .verdict
            .clone()
            .expect("permission timeout fallback verdict"),
    )
    .expect("decode permission timeout fallback verdict");
    assert_eq!(verdict.verdict, ReviewVerdictType::NeedsHuman);
    assert!(
        verdict
            .structured_output_diagnostic
            .as_ref()
            .is_some_and(|diagnostic| {
                diagnostic.repair_attempted && !diagnostic.repair_succeeded
            })
    );

    let persisted_session = lifecycle_store
        .get_workspace_session(&engine.session.session_id)
        .expect("permission timeout session");
    assert_eq!(
        persisted_session.status,
        WorkspaceSessionStatus::WaitingForHuman
    );
    let (reload_tx, _reload_rx) = mpsc::channel(8);
    let reloaded = WorkspaceEngine::new_persistent(
        checkpoint_store,
        lifecycle_store.clone(),
        reload_tx,
        WorkspaceSession::from_record(persisted_session),
    );
    assert_eq!(reloaded.session.stage, WorkspaceStage::AuthorConfirm);
    assert_eq!(
        reloaded
            .timeline_nodes
            .iter()
            .find(|node| node.node_id == review_node_id)
            .expect("reloaded timeout review node")
            .status,
        TimelineNodeStatus::Completed
    );
    let reloaded_detail = lifecycle_store
        .load_node_detail(&reloaded.session.session_id, &review_node_id)
        .expect("reloaded permission timeout detail");
    assert!(
        reloaded_detail
            .permission_events
            .iter()
            .all(|event| event.response.is_some())
    );
}

#[tokio::test]
async fn repair_abort_closes_started_event_as_failed() {
    let provider = RepairTerminalProvider::new(
        RepairTerminalMode::WaitForAbort,
        Some("review-session-1".to_string()),
    );
    let starts = provider.starts.clone();
    let abort_starts = starts.clone();
    let (_tmp, mut engine, mut rx, review_node_id) =
        queued_review_engine("sess_repair_terminal_abort").await;
    let (command_tx, command_rx) = mpsc::channel(4);
    let abort = tokio::spawn(async move {
        while abort_starts.load(Ordering::SeqCst) < 2 {
            tokio::task::yield_now().await;
        }
        command_tx.send(ProviderCommand::Abort).await.unwrap();
    });

    engine
        .drive_review_session(Arc::new(provider), command_rx)
        .await;
    abort.await.unwrap();

    assert_eq!(starts.load(Ordering::SeqCst), 2);
    assert_eq!(engine.session().stage, WorkspaceStage::PrepareContext);
    assert!(engine.latest_review_verdict.is_none());

    assert_eq!(
        repair_event_statuses(&mut rx),
        vec![
            (
                ProviderExecutionEventStatus::Started,
                Some(review_node_id.clone())
            ),
            (ProviderExecutionEventStatus::Failed, Some(review_node_id)),
        ]
    );
}

#[tokio::test]
async fn missing_or_blank_first_provider_session_id_starts_one_fresh_session_repair() {
    for first_session_id in [None, Some("   ".to_string())] {
        let provider =
            RepairTerminalProvider::new(RepairTerminalMode::StartError, first_session_id);
        let (_tmp, mut engine, mut rx, _review_node_id) =
            queued_review_engine("sess_repair_missing_provider_session").await;

        engine
            .drive_review_session(Arc::new(provider.clone()), empty_provider_commands())
            .await;

        assert_eq!(provider.starts.load(Ordering::SeqCst), 2);
        assert_eq!(
            provider.resume_provider_session_ids.lock().unwrap()[1],
            None
        );
        let diagnostic = engine
            .latest_review_verdict
            .as_ref()
            .and_then(|verdict| verdict.structured_output_diagnostic.as_ref())
            .expect("missing provider session diagnostic");
        assert_eq!(diagnostic.code, "missing_json_nonce");
        assert!(diagnostic.repair_attempted);
        assert!(!diagnostic.repair_succeeded);
        assert_eq!(
            repair_event_statuses(&mut rx),
            vec![
                (
                    ProviderExecutionEventStatus::Started,
                    engine
                        .timeline_nodes
                        .iter()
                        .find(|node| node.node_type == TimelineNodeType::ReviewerRun)
                        .map(|node| node.node_id.clone()),
                ),
                (
                    ProviderExecutionEventStatus::Failed,
                    engine
                        .timeline_nodes
                        .iter()
                        .find(|node| node.node_type == TimelineNodeType::ReviewerRun)
                        .map(|node| node.node_id.clone()),
                ),
            ]
        );
    }
}
