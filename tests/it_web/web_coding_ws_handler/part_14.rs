struct CoderPlanDefectRunnerProvider {
    reviewer_calls: Arc<std::sync::atomic::AtomicUsize>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for CoderPlanDefectRunnerProvider {
    async fn run_streaming(
        &self,
        input: &AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        let (tx, rx) = mpsc::channel(8);
        let full_output = match input.role {
            AdapterRole::Executor => serde_json::json!({
                "plan_defect_findings": [{
                    "finding_id": "coder_plan_defect_0001",
                    "severity": "error",
                    "defect_class": "story_amendment_required",
                    "reason_code": "story_contract_invalid",
                    "message": "the story contract cannot support this implementation",
                    "evidence": [],
                    "contract_refs": [],
                    "capability_refs": [],
                    "repair_target": null,
                    "recommended_route": "story_amendment",
                    "confidence": "high"
                }]
            })
            .to_string(),
            AdapterRole::Reviewer => {
                self.reviewer_calls
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                r#"{"verdict":"approve","summary":"review ok","findings":[]}"#.to_string()
            }
            _ => "ok".to_string(),
        };
        tx.try_send(StreamChunk::Done { full_output })
            .expect("send provider output");
        Ok(rx)
    }
}

#[tokio::test]
async fn coding_plan_repair_coder_plan_finding_safe_stops_before_code_review_runner() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let reviewer_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let app = app_with_full_chain_attempt_and_provider(
        root.path(),
        Arc::new(CoderPlanDefectRunnerProvider {
            reviewer_calls: reviewer_calls.clone(),
        }),
    );
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/ws/coding-attempts/coding_attempt_0001");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;
    send_json(&mut ws, &CodingWsInMessage::StartCoding).await;

    let mut saw_route = false;
    let mut saw_code_review_gate = false;
    let mut saw_code_review_node = false;
    for _ in 0..80 {
        match timeout(Duration::from_secs(1), recv_json(&mut ws)).await {
            Ok(CodingWsOutMessage::CodingGateRequired { gate })
                if gate.stage == Some(CodingExecutionStage::Coding) =>
            {
                send_json(
                    &mut ws,
                    &CodingWsInMessage::StageGateConfirm {
                        stage: CodingExecutionStage::Coding,
                    },
                )
                .await;
            }
            Ok(CodingWsOutMessage::CodingGateRequired { gate })
                if gate.stage == Some(CodingExecutionStage::CodeReview) =>
            {
                saw_code_review_gate = true;
                send_json(
                    &mut ws,
                    &CodingWsInMessage::StageGateConfirm {
                        stage: CodingExecutionStage::CodeReview,
                    },
                )
                .await;
            }
            Ok(CodingWsOutMessage::CodingTimelineNodeCreated { node })
                if node.stage == CodingExecutionStage::CodeReview =>
            {
                saw_code_review_node = true;
                break;
            }
            Ok(CodingWsOutMessage::CodingChatEntryCreated { entry })
                if entry.metadata.as_ref().is_some_and(|metadata| {
                    metadata.get("source").and_then(|value| value.as_str()) == Some("coding")
                        && metadata
                            .get("plan_defect_route")
                            .and_then(|value| value.as_str())
                            == Some("stop_for_human_triage")
                }) =>
            {
                saw_route = true;
            }
            Ok(CodingWsOutMessage::CodingSessionState { stage, .. })
                if saw_route && stage == CodingExecutionStage::Coding =>
            {
                break;
            }
            Ok(CodingWsOutMessage::CodingProtocolError { code, message }) => {
                panic!("unexpected coding protocol error {code}: {message}");
            }
            Ok(_) => {}
            Err(_) if saw_route => break,
            Err(_) => panic!("timed out before coder plan defect route"),
        }
    }

    assert!(saw_route, "expected observable coder plan defect route");
    assert!(!saw_code_review_gate, "must not open CodeReview stage gate");
    assert!(!saw_code_review_node, "must not create CodeReview timeline node");
    assert_eq!(
        reviewer_calls.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "must not invoke reviewer"
    );
    let attempt = store
        .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("attempt");
    assert_eq!(attempt.status, CodingAttemptStatus::Running);
    assert_eq!(attempt.stage, CodingExecutionStage::Coding);

    ws.close(None).await.expect("close ws");
    server.abort();
}
