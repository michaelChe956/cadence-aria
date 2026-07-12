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
            let output = missing_end_nonce_output(
                r#"{"verdict":"revise","summary":"必须修复","findings":[{"severity":"must_fix","message":"缺少失败路径","evidence":"当前产物遗漏","impact":"无法验收","required_action":"补充失败路径"}]}"#,
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
            RepairTerminalMode::PermissionTimeout => Ok(Self::session_with_event(Some(
                ProviderEvent::PermissionTimeout {
                    permission_id: "permission-repair".to_string(),
                },
            ))
            .await),
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

#[tokio::test]
async fn review_structured_output_repair_succeeds_for_all_general_workspace_types() {
    let review_json = r#"{
        "verdict": "revise",
        "summary": "补充失败路径",
        "findings": [{
            "severity": "must_fix",
            "message": "缺少失败路径",
            "evidence": "Artifact 未覆盖失败路径",
            "impact": "下一阶段无法验收异常流程",
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
            missing_end_nonce_output(review_json),
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
        assert!(prompts[1].contains("missing_end_nonce"));
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
    for mode in [
        RepairTerminalMode::StartError,
        RepairTerminalMode::EmptyCompletion,
        RepairTerminalMode::Failed,
        RepairTerminalMode::PermissionTimeout,
        RepairTerminalMode::StreamClosed,
    ] {
        let provider = RepairTerminalProvider::new(mode, Some("review-session-1".to_string()));
        let session_id = format!("sess_repair_terminal_{mode:?}");
        let (_tmp, mut engine, mut rx, review_node_id) = queued_review_engine(&session_id).await;

        engine
            .drive_review_session(Arc::new(provider), empty_provider_commands())
            .await;

        assert_eq!(
            engine.session.stage,
            WorkspaceStage::HumanConfirm,
            "repair terminal mode {mode:?} must safely degrade the review"
        );
        let verdict = engine
            .latest_review_verdict
            .as_ref()
            .unwrap_or_else(|| panic!("repair terminal mode {mode:?} must persist a verdict"));
        assert_eq!(verdict.verdict, ReviewVerdictType::NeedsHuman);
        let diagnostic = verdict
            .structured_output_diagnostic
            .as_ref()
            .unwrap_or_else(|| panic!("repair terminal mode {mode:?} diagnostic"));
        assert_eq!(diagnostic.code, "missing_end_nonce");
        assert!(diagnostic.repair_attempted);
        assert!(!diagnostic.repair_succeeded);
        assert_eq!(
            repair_event_statuses(&mut rx),
            vec![
                (
                    ProviderExecutionEventStatus::Started,
                    Some(review_node_id.clone())
                ),
                (ProviderExecutionEventStatus::Failed, Some(review_node_id)),
            ],
            "repair terminal mode {mode:?} must close its execution event"
        );
    }
}

#[tokio::test]
async fn repair_abort_closes_started_event_as_failed() {
    let provider = RepairTerminalProvider::new(
        RepairTerminalMode::WaitForAbort,
        Some("review-session-1".to_string()),
    );
    let starts = provider.starts.clone();
    let (_tmp, mut engine, mut rx, review_node_id) =
        queued_review_engine("sess_repair_terminal_abort").await;
    let (command_tx, command_rx) = mpsc::channel(4);
    let abort = tokio::spawn(async move {
        while starts.load(Ordering::SeqCst) < 2 {
            tokio::task::yield_now().await;
        }
        command_tx.send(ProviderCommand::Abort).await.unwrap();
    });

    engine
        .drive_review_session(Arc::new(provider), command_rx)
        .await;
    abort.await.unwrap();

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
        assert_eq!(diagnostic.code, "missing_end_nonce");
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
