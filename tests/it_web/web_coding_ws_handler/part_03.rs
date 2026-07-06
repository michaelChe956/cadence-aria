#[tokio::test]
async fn coding_ws_code_review_blocked_stops_without_analyst_rework() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let provider = Arc::new(ReviewerBlockedProvider::code_review());
    let app = app_with_full_chain_attempt_and_provider(root.path(), provider.clone());
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
    let mut saw_code_review_blocked = false;
    let mut saw_rework_node = false;
    let mut stopped_at_code_review = false;
    for _ in 0..120 {
        match timeout(Duration::from_millis(500), recv_json(&mut ws)).await {
            Ok(CodingWsOutMessage::CodingGateRequired { gate })
                if gate.kind == CodingGateKind::StageGate =>
            {
                if let Some(stage) = gate.stage.clone()
                    && confirmed_gates.insert(gate.gate_id)
                {
                    send_json(&mut ws, &CodingWsInMessage::StageGateConfirm { stage }).await;
                }
            }
            Ok(CodingWsOutMessage::CodeReviewComplete { report })
                if report.verdict == ReviewVerdict::Blocked =>
            {
                saw_code_review_blocked = true;
            }
            Ok(CodingWsOutMessage::CodingTimelineNodeCreated { node })
                if node.stage == CodingExecutionStage::Rework =>
            {
                saw_rework_node = true;
            }
            Ok(CodingWsOutMessage::CodingSessionState { status, stage, .. })
                if saw_code_review_blocked && stage == CodingExecutionStage::CodeReview =>
            {
                assert_eq!(status, CodingAttemptStatus::Running);
                stopped_at_code_review = true;
                break;
            }
            Ok(CodingWsOutMessage::CodingProtocolError { code, message }) => {
                panic!("unexpected coding protocol error {code}: {message}");
            }
            Ok(_) | Err(_) => {}
        }
    }

    assert!(
        saw_code_review_blocked,
        "code review blocked report missing"
    );
    assert!(
        stopped_at_code_review,
        "code review blocked did not stop at CodeReview"
    );
    assert!(
        !saw_rework_node,
        "code review blocked must not enter analyst rework"
    );
    assert!(
        provider.analyst_prompts().is_empty(),
        "analyst should not run after code review blocked"
    );

    let attempt = store
        .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("attempt");
    assert_eq!(attempt.status, CodingAttemptStatus::Running);
    assert_eq!(attempt.stage, CodingExecutionStage::CodeReview);
    let runs = store
        .list_role_runs("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("role runs");
    let reviewer_run = runs
        .iter()
        .find(|run| run.role == CodingProviderRole::CodeReviewer)
        .expect("code reviewer role run");
    assert_eq!(reviewer_run.status, CodingRoleRunStatus::Blocked);
    assert!(
        runs.iter().all(|run| run.role != CodingProviderRole::Analyst),
        "analyst role runs should not be created"
    );

    ws.close(None).await.expect("close ws");
    server.abort();
}

#[tokio::test]
async fn coding_ws_internal_pr_review_blocked_stops_without_analyst_rework() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let provider = Arc::new(ReviewerBlockedProvider::internal_pr_review());
    let app = app_with_full_chain_attempt_and_provider(root.path(), provider.clone());
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
    let mut saw_internal_review_blocked = false;
    let mut saw_rework_node = false;
    let mut stopped_at_internal_review = false;
    for _ in 0..180 {
        match timeout(Duration::from_millis(500), recv_json(&mut ws)).await {
            Ok(CodingWsOutMessage::CodingGateRequired { gate })
                if gate.kind == CodingGateKind::StageGate =>
            {
                if let Some(stage) = gate.stage.clone()
                    && confirmed_gates.insert(gate.gate_id)
                {
                    send_json(&mut ws, &CodingWsInMessage::StageGateConfirm { stage }).await;
                }
            }
            Ok(CodingWsOutMessage::InternalPrReviewComplete { review })
                if review.verdict == ReviewVerdict::Blocked =>
            {
                saw_internal_review_blocked = true;
            }
            Ok(CodingWsOutMessage::CodingTimelineNodeCreated { node })
                if node.stage == CodingExecutionStage::Rework =>
            {
                saw_rework_node = true;
            }
            Ok(CodingWsOutMessage::CodingSessionState { status, stage, .. })
                if saw_internal_review_blocked && stage == CodingExecutionStage::InternalPrReview =>
            {
                assert_eq!(status, CodingAttemptStatus::Running);
                stopped_at_internal_review = true;
                break;
            }
            Ok(CodingWsOutMessage::CodingProtocolError { code, message }) => {
                panic!("unexpected coding protocol error {code}: {message}");
            }
            Ok(_) | Err(_) => {}
        }
    }

    assert!(
        saw_internal_review_blocked,
        "internal review blocked report missing"
    );
    assert!(
        stopped_at_internal_review,
        "internal review blocked did not stop at InternalPrReview"
    );
    assert!(
        !saw_rework_node,
        "internal review blocked must not enter analyst rework"
    );
    assert!(
        provider.analyst_prompts().is_empty(),
        "analyst should not run after internal review blocked"
    );

    let attempt = store
        .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("attempt");
    assert_eq!(attempt.status, CodingAttemptStatus::Running);
    assert_eq!(attempt.stage, CodingExecutionStage::InternalPrReview);
    let runs = store
        .list_role_runs("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("role runs");
    let internal_run = runs
        .iter()
        .find(|run| run.role == CodingProviderRole::InternalReviewer)
        .expect("internal reviewer role run");
    assert_eq!(internal_run.status, CodingRoleRunStatus::Blocked);
    assert_eq!(
        internal_run.reason_code.as_deref(),
        Some("internal_review_blocked")
    );
    assert!(
        runs.iter().all(|run| run.role != CodingProviderRole::Analyst),
        "analyst role runs should not be created"
    );

    ws.close(None).await.expect("close ws");
    server.abort();
}
