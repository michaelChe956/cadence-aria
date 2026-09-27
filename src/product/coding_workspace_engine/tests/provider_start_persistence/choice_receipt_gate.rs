// 从 provider_start_persistence.rs 拆出以满足 large_file_guard 的 1200 行上限
// （纯移动，无行为变化）：P0 1.3（REQ-WIGA-05）coding 双题回执 gate 组。

use super::*;

// ---------------------------------------------------------------------------
// P0 1.3（REQ-WIGA-05）：coding 双题 gate 只在回执 Delivered（provider 等待者
// 真正接收）后 resolve；Resolving 期间 gate 仍 Open，answers 原样透传。
// ---------------------------------------------------------------------------

use crate::cross_cutting::streaming_provider::ChoiceAnswerData;

struct ReceiptGatedChoiceProvider {
    release: std::sync::Arc<tokio::sync::Notify>,
    forwarded_answers: std::sync::Arc<std::sync::Mutex<Option<Vec<ChoiceAnswerData>>>>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for ReceiptGatedChoiceProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (event_tx, event_rx) = mpsc::channel(4);
        let (command_tx, mut command_rx) = mpsc::channel(4);
        let release = std::sync::Arc::clone(&self.release);
        let forwarded_answers = std::sync::Arc::clone(&self.forwarded_answers);
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::ChoiceRequest(provider_choice_request()))
                .await;
            tokio::select! {
                _ = cancel.cancelled() => {}
                command = command_rx.recv() => {
                    if let Some(ProviderCommand::ChoiceResponse { answers, receipt: Some(receipt), .. }) = command {
                        // provider 命令消费者已领取 → Resolving；测试栅栏控制
                        // 「真正接收」时机，期间回执不得冒充 Delivered。
                        receipt.mark_resolving();
                        *forwarded_answers.lock().unwrap() = Some(answers);
                        release.notified().await;
                        receipt.deliver();
                        let _ = event_tx
                            .send(ProviderEvent::Completed(
                                crate::cross_cutting::streaming_provider::ProviderCompletion::plain(
                                    "receipt gated choice output".to_string(),
                                    Some("provider-receipt-gated".to_string()),
                                ),
                            ))
                            .await;
                    }
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
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        Err(ProviderAdapterError::execution_failed(
            None,
            String::new(),
            "run_streaming is not used by CodingWorkspaceEngine",
            0,
        ))
    }
}

#[tokio::test]
async fn choice_delivery_coding_gate_stays_open_until_receipt_delivered() {
    use crate::cross_cutting::choice_delivery::{ChoiceDeliverySignal, ChoiceReplyState};

    let (_root, store, attempt) = running_attempt_with_worktree();
    let role_run = store
        .create_role_run(
            &attempt,
            CodingExecutionStage::Coding,
            CodingProviderRole::Coder,
            CodingRoleRunTrigger::Initial,
            Some("coding_invocation_choice_receipt".to_string()),
        )
        .expect("role run");
    let (event_tx, _event_rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), event_tx);
    let release = std::sync::Arc::new(tokio::sync::Notify::new());
    let forwarded_answers: std::sync::Arc<std::sync::Mutex<Option<Vec<ChoiceAnswerData>>>> =
        std::sync::Arc::new(std::sync::Mutex::new(None));
    let provider = ReceiptGatedChoiceProvider {
        release: std::sync::Arc::clone(&release),
        forwarded_answers: std::sync::Arc::clone(&forwarded_answers),
    };
    let (legacy_input, input) = provider_invocation_inputs(&attempt);
    let provider_name = ProviderName::Codex;
    let (command_tx, mut command_rx) = mpsc::channel(4);

    let invocation = engine.run_provider_stream_invocation(CodingProviderStreamRun {
        attempt: &attempt,
        node_id: "coding_invocation_choice_receipt",
        role_run: Some(&role_run),
        provider: &provider,
        legacy_input: &legacy_input,
        input,
        provider_name: &provider_name,
        provider_role: CodingProviderRole::Coder,
        command_rx: &mut command_rx,
        allow_legacy_stream_fallback: false,
        timeout: Some(Duration::from_secs(8)),
        timeout_reason_code: None,
        suppress_failure_side_effects: false,
        validated_input: None,
    });
    tokio::pin!(invocation);
    tokio::select! {
        biased;
        () = wait_for_open_provider_choice(&store, &attempt) => {}
        outcome = &mut invocation => {
            panic!("provider invocation completed before choice gate opened: {outcome:?}");
        }
    }

    let (receipt, mut status) = ChoiceDeliverySignal::new();
    let two_answers = vec![
        ChoiceAnswerData {
            question_id: "q-1".to_string(),
            selected_option_ids: vec!["continue".to_string()],
            free_text: None,
        },
        ChoiceAnswerData {
            question_id: "q-2".to_string(),
            selected_option_ids: Vec::new(),
            free_text: Some("两题各自独立作答".to_string()),
        },
    ];
    command_tx
        .send(CodingRunnerCommand::ChoiceResponse {
            id: "provider_choice_0001".to_string(),
            selected_option_ids: vec!["continue".to_string()],
            free_text: None,
            answers: two_answers.clone(),
            receipt: Some(receipt),
        })
        .await
        .expect("send coding choice response");

    // provider 已领取（Resolving）但等待者未「真正接收」——回执不得是
    // Delivered，durable gate 必须仍 Open。等待期间并发驱动 invocation。
    let wait_resolving = async {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while *status.borrow() != ChoiceReplyState::Resolving {
            assert!(
                tokio::time::Instant::now() < deadline,
                "receipt never reached Resolving, current: {:?}",
                *status.borrow()
            );
            tokio::task::yield_now().await;
        }
    };
    tokio::select! {
        biased;
        () = wait_resolving => {}
        outcome = &mut invocation => {
            panic!("invocation completed while waiting for Resolving: {outcome:?}");
        }
    }
    assert_ne!(*status.borrow(), ChoiceReplyState::Delivered);
    assert_open_provider_choice(&store, &attempt);

    // 释放栅栏：provider 等待者接收 → Delivered → gate resolve、attempt 放行。
    release.notify_one();
    let _outcome = invocation.await;
    assert!(
        *status.borrow() == ChoiceReplyState::Delivered,
        "receipt should be Delivered after provider waiter consumed, current: {:?}",
        *status.borrow()
    );
    let persisted = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("persisted attempt");
    assert_ne!(persisted.status, CodingAttemptStatus::WaitingForHuman);
    let open_gates = store
        .list_open_choice_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("open choice gates");
    assert!(
        open_gates.is_empty(),
        "gate must be resolved after Delivered"
    );
    assert_eq!(
        forwarded_answers.lock().unwrap().clone(),
        Some(two_answers),
        "coding path answers 原样透传 provider"
    );
}
