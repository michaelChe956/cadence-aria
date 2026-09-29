// ---------------------------------------------------------------------------
// P0 1.3（REQ-WIGA-05）：三条 workspace 转发路径逐字段透传完整 answers 与
// 两层回执 receipt——provider 等待者收到时 answers 不变、receipt 同一身份。
// ---------------------------------------------------------------------------

fn choice_delivery_two_question_request() -> ChoiceRequestData {
    ChoiceRequestData {
        id: "choice_delivery_fwd".to_string(),
        prompt: "确认交付策略。".to_string(),
        options: Vec::new(),
        allow_multiple: false,
        allow_free_text: true,
        questions: vec![
            ChoiceQuestionData {
                id: "q-1".to_string(),
                prompt: "是否包含集成测试？".to_string(),
                options: vec![ChoiceOptionData {
                    id: "yes".to_string(),
                    label: "包含".to_string(),
                    description: None,
                }],
                allow_multiple: false,
                allow_free_text: false,
            },
            ChoiceQuestionData {
                id: "q-2".to_string(),
                prompt: "评审轮次？".to_string(),
                options: vec![ChoiceOptionData {
                    id: "one".to_string(),
                    label: "一轮".to_string(),
                    description: None,
                }],
                allow_multiple: false,
                allow_free_text: false,
            },
        ],
        source: ChoiceRequestSource::AskUserQuestion,
    }
}

fn choice_delivery_two_answers() -> Vec<ChoiceAnswerData> {
    vec![
        ChoiceAnswerData {
            question_id: "q-1".to_string(),
            selected_option_ids: vec!["yes".to_string()],
            free_text: None,
        },
        ChoiceAnswerData {
            question_id: "q-2".to_string(),
            selected_option_ids: vec!["one".to_string()],
            free_text: None,
        },
    ]
}

type ChoiceDeliveryCapture = std::sync::Arc<
    std::sync::Mutex<
        Option<(
            Vec<ChoiceAnswerData>,
            crate::cross_cutting::choice_delivery::ChoiceDeliverySignal,
        )>,
    >,
>;

#[tokio::test]
async fn choice_delivery_author_path_forwards_full_answers_and_receipt() {
    use crate::cross_cutting::choice_delivery::{ChoiceDeliverySignal, ChoiceReplyState};

    let (event_tx, _event_rx) = mpsc::channel(32);
    let mut session = make_session("sess_choice_delivery_author");
    session.workspace_type = WorkspaceType::Story;
    session.author_provider = ProviderName::ClaudeCode;
    let checkpoint_tmp = TempDir::new().unwrap();
    let mut engine = WorkspaceEngine::new(
        Arc::new(CheckpointStore::new(checkpoint_tmp.path().to_path_buf())),
        event_tx,
        session,
    );
    let (provider_event_tx, provider_event_rx) = mpsc::channel(8);
    let (provider_command_tx, mut provider_command_rx) = mpsc::channel(8);
    let (command_tx, command_rx) = mpsc::channel(8);

    let (receipt, mut status) = ChoiceDeliverySignal::new();
    let sent_receipt = receipt.clone();
    let two_answers = choice_delivery_two_answers();
    let expected_answers = two_answers.clone();
    let captured: ChoiceDeliveryCapture = std::sync::Arc::new(std::sync::Mutex::new(None));
    let responder_captured = std::sync::Arc::clone(&captured);
    let responder = async move {
        provider_event_tx
            .send(ProviderEvent::ChoiceRequest(
                choice_delivery_two_question_request(),
            ))
            .await
            .expect("send choice request");
        // 与 drive 并发：命令先入队，drive 轮询时转发给 provider 会话。
        command_tx
            .send(ProviderCommand::ChoiceResponse {
                id: "choice_delivery_fwd".to_string(),
                selected_option_ids: Vec::new(),
                free_text: None,
                answers: two_answers.clone(),
                receipt: Some(sent_receipt),
            })
            .await
            .expect("send choice response");
        while let Some(command) = provider_command_rx.recv().await {
            if let ProviderCommand::ChoiceResponse {
                id,
                answers,
                receipt,
                ..
            } = command
                && id == "choice_delivery_fwd"
            {
                *responder_captured.lock().unwrap() =
                    Some((answers, receipt.expect("receipt forwarded")));
                let _ = provider_event_tx
                    .send(ProviderEvent::Completed(
                        crate::cross_cutting::streaming_provider::ProviderCompletion::plain(
                            "author output".to_string(),
                            Some("provider-choice-delivery-author".to_string()),
                        ),
                    ))
                    .await;
                break;
            }
        }
    };

    let drive = engine.drive_provider_session(ProviderSessionDriveInput {
        session: Ok(ProviderSession {
            native_session_id: None,
            events: provider_event_rx,
            commands: provider_command_tx,
        }),
        command_rx,
        node_id: Some("timeline_node_choice_delivery_author".to_string()),
        agent: Some(ProviderName::ClaudeCode),
        role: ProviderConversationRole::Author,
        artifact_retry: None,
        revision_resume_fallback: None,
    });
    tokio::join!(drive, responder);
    let (forwarded_answers, forwarded_receipt) = captured
        .lock()
        .unwrap()
        .take()
        .expect("provider side captured forwarded command");
    assert_eq!(
        forwarded_answers, expected_answers,
        "author path answers 不变"
    );
    assert_eq!(forwarded_receipt, receipt, "author path receipt 同一身份");

    // provider 等待者接收后推进 Delivered（两层回执第二层）。
    forwarded_receipt.deliver();
    assert_eq!(*status.borrow(), ChoiceReplyState::Delivered);
}

#[tokio::test]
async fn choice_delivery_work_item_plan_path_forwards_full_answers_and_receipt() {
    use crate::cross_cutting::choice_delivery::{ChoiceDeliverySignal, ChoiceReplyState};

    let (_tmp, _checkpoint_store, lifecycle, _plan_id, mut engine) =
        make_work_item_plan_engine_with_draft_candidate("sess_choice_delivery_wip");
    engine.session.session_id = lifecycle
        .list_workspace_sessions("project_0001", "issue_0001")
        .expect("workspace sessions")
        .into_iter()
        .find(|session| session.workspace_type == WorkspaceType::WorkItemPlan)
        .expect("work item plan session")
        .id;
    let node_id = engine.begin_work_item_plan_author_run().await;
    let (provider_event_tx, provider_event_rx) = mpsc::channel(8);
    let (provider_command_tx, mut provider_command_rx) = mpsc::channel(8);
    let (command_tx, mut command_rx) = mpsc::channel(8);
    let (receipt, status) = ChoiceDeliverySignal::new();
    let sent_receipt = receipt.clone();
    let two_answers = choice_delivery_two_answers();
    let captured: ChoiceDeliveryCapture = std::sync::Arc::new(std::sync::Mutex::new(None));
    let provider_captured = std::sync::Arc::clone(&captured);
    tokio::spawn(async move {
        provider_event_tx
            .send(ProviderEvent::TextDelta {
                content: "draft".to_string(),
            })
            .await
            .expect("send text delta");
        while let Some(command) = provider_command_rx.recv().await {
            if let ProviderCommand::ChoiceResponse {
                id,
                answers,
                receipt,
                ..
            } = command
                && id == "choice_delivery_fwd"
            {
                *provider_captured.lock().unwrap() =
                    Some((answers, receipt.expect("receipt forwarded")));
                let _ = provider_event_tx
                    .send(ProviderEvent::Completed(
                        crate::cross_cutting::streaming_provider::ProviderCompletion::plain(
                            "wip output".to_string(),
                            Some("provider-choice-delivery-wip".to_string()),
                        ),
                    ))
                    .await;
                break;
            }
        }
    });
    command_tx
        .send(ProviderCommand::ChoiceResponse {
            id: "choice_delivery_fwd".to_string(),
            selected_option_ids: Vec::new(),
            free_text: None,
            answers: two_answers.clone(),
            receipt: Some(sent_receipt),
        })
        .await
        .expect("send choice response");

    let output = engine
        .drive_work_item_plan_provider_session_to_output(
            Ok(ProviderSession {
                native_session_id: None,
                events: provider_event_rx,
                commands: provider_command_tx,
            }),
            &mut command_rx,
            node_id,
            ProviderName::ClaudeCode,
        )
        .await
        .expect("collector output");
    assert_eq!(output, "wip output");
    let (forwarded_answers, forwarded_receipt) = captured
        .lock()
        .unwrap()
        .take()
        .expect("provider side captured forwarded command");
    assert_eq!(forwarded_answers, two_answers, "wip path answers 不变");
    assert_eq!(forwarded_receipt, receipt, "wip path receipt 同一身份");
    forwarded_receipt.deliver();
    assert_eq!(*status.borrow(), ChoiceReplyState::Delivered);
}

#[tokio::test]
async fn choice_delivery_review_path_forwards_full_answers_and_receipt() {
    use crate::cross_cutting::choice_delivery::{ChoiceDeliverySignal, ChoiceReplyState};
    use crate::cross_cutting::streaming_provider::StreamingProviderAdapter;

    let tmp = TempDir::new().unwrap();
    let checkpoint_store = Arc::new(CheckpointStore::new(tmp.path().join("checkpoints")));
    let lifecycle_store = LifecycleStore::new(ProductAppPaths::new(tmp.path().join(".aria")));
    let session_record = lifecycle_store
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: "story_spec_0001".to_string(),
            workspace_type: WorkspaceType::Story,
            author_provider: ProviderName::Codex,
            reviewer_provider: Some(ProviderName::ClaudeCode),

            review_rounds: 1,
            superpowers_enabled: true,
            openspec_enabled: true,
            work_item_plan_options: None,
        })
        .unwrap();
    let mut engine = WorkspaceEngine::new(
        checkpoint_store,
        mpsc::channel(64).0,
        WorkspaceSession::from_record(session_record),
    );
    engine.session.artifact = Some(artifact_payload("# Artifact\n\n可评审版本"));
    engine.start_review().await;

    struct ReceiptCapturingReviewProvider {
        captured: ChoiceDeliveryCapture,
    }
    #[async_trait::async_trait]
    impl StreamingProviderAdapter for ReceiptCapturingReviewProvider {
        async fn start(
            &self,
            _input: crate::cross_cutting::streaming_provider::StreamingProviderInput,
            _cancel: tokio_util::sync::CancellationToken,
        ) -> Result<ProviderSession, ProviderAdapterError> {
            let (event_tx, event_rx) = mpsc::channel(8);
            let (command_tx, mut command_rx) = mpsc::channel(8);
            let captured = std::sync::Arc::clone(&self.captured);
            tokio::spawn(async move {
                let _ = event_tx
                    .send(ProviderEvent::ChoiceRequest(
                        choice_delivery_two_question_request(),
                    ))
                    .await;
                while let Some(command) = command_rx.recv().await {
                    if let ProviderCommand::ChoiceResponse {
                        id,
                        answers,
                        receipt,
                        ..
                    } = command
                        && id == "choice_delivery_fwd"
                    {
                        *captured.lock().unwrap() =
                            Some((answers, receipt.expect("receipt forwarded")));
                        let _ = event_tx
                            .send(ProviderEvent::Completed(
                                crate::cross_cutting::streaming_provider::ProviderCompletion::plain(
                                    "# Artifact\n\n可评审版本".to_string(),
                                    Some("provider-choice-delivery-review".to_string()),
                                ),
                            ))
                            .await;
                        break;
                    }
                }
            });
            Ok(ProviderSession {
                native_session_id: None,
                events: event_rx,
                commands: command_tx,
            })
        }

        async fn run_streaming(
            &self,
            _input: &AdapterInput,
            _cancel: tokio_util::sync::CancellationToken,
        ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
            Err(ProviderAdapterError::execution_failed(
                None,
                String::new(),
                "run_streaming is not used by WorkspaceEngine",
                0,
            ))
        }
    }

    let (receipt, mut status) = ChoiceDeliverySignal::new();
    let sent_receipt = receipt.clone();
    let two_answers = choice_delivery_two_answers();
    let captured: ChoiceDeliveryCapture = std::sync::Arc::new(std::sync::Mutex::new(None));
    let provider = ReceiptCapturingReviewProvider {
        captured: std::sync::Arc::clone(&captured),
    };
    let (command_tx, command_rx) = mpsc::channel(8);
    command_tx
        .send(ProviderCommand::ChoiceResponse {
            id: "choice_delivery_fwd".to_string(),
            selected_option_ids: Vec::new(),
            free_text: None,
            answers: two_answers.clone(),
            receipt: Some(sent_receipt),
        })
        .await
        .expect("send choice response");

    engine
        .drive_review_session(Arc::new(provider), command_rx)
        .await;

    let (forwarded_answers, forwarded_receipt) = captured
        .lock()
        .unwrap()
        .take()
        .expect("provider side captured forwarded command");
    assert_eq!(forwarded_answers, two_answers, "review path answers 不变");
    assert_eq!(forwarded_receipt, receipt, "review path receipt 同一身份");
    forwarded_receipt.deliver();
    assert_eq!(*status.borrow(), ChoiceReplyState::Delivered);
}
