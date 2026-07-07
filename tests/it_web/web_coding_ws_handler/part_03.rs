#[tokio::test]
async fn coding_ws_code_review_blocked_stops_at_reviewer_gate() {
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
    let mut saw_code_review_blocked_gate = false;
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
            Ok(CodingWsOutMessage::CodingGateRequired { gate })
                if gate.kind == CodingGateKind::Blocked
                    && gate.stage == Some(CodingExecutionStage::CodeReview) =>
            {
                assert_eq!(gate.role, Some(CodingProviderRole::CodeReviewer));
                assert_eq!(gate.reason_code.as_deref(), Some("code_review_blocked"));
                assert!(
                    gate.available_actions
                        .iter()
                        .any(|action| action.action_id == "retry_review")
                );
                saw_code_review_blocked_gate = true;
            }
            Ok(CodingWsOutMessage::CodeReviewComplete { report })
                if report.verdict == ReviewVerdict::Blocked =>
            {
                saw_code_review_blocked = true;
            }
            Ok(CodingWsOutMessage::CodingSessionState {
                status,
                stage,
                pending_gates,
                ..
            })
                if saw_code_review_blocked && stage == CodingExecutionStage::CodeReview =>
            {
                assert_eq!(status, CodingAttemptStatus::Blocked);
                assert!(
                    pending_gates.iter().any(|gate| {
                        gate.kind == CodingGateKind::Blocked
                            && gate.reason_code.as_deref() == Some("code_review_blocked")
                    }),
                    "blocked review gate missing from session state"
                );
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
        saw_code_review_blocked_gate,
        "code review blocked gate missing"
    );
    assert!(
        stopped_at_code_review,
        "code review blocked did not stop at CodeReview"
    );
    let attempt = store
        .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("attempt");
    assert_eq!(attempt.status, CodingAttemptStatus::Blocked);
    assert_eq!(attempt.stage, CodingExecutionStage::CodeReview);
    let runs = store
        .list_role_runs("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("role runs");
    let reviewer_run = runs
        .iter()
        .find(|run| run.role == CodingProviderRole::CodeReviewer)
        .expect("code reviewer role run");
    assert_eq!(reviewer_run.status, CodingRoleRunStatus::Blocked);

    ws.close(None).await.expect("close ws");
    server.abort();
}

#[tokio::test]
async fn coding_ws_single_work_item_skips_internal_review_blocked_provider() {
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
    let mut completed_after_review_request = false;
    let mut saw_internal_review = false;
    let mut saw_internal_review_blocked_gate = false;
    let mut observed = Vec::new();
    for _ in 0..180 {
        match timeout(Duration::from_millis(500), recv_json(&mut ws)).await {
            Ok(CodingWsOutMessage::CodingGateRequired { gate })
                if gate.kind == CodingGateKind::StageGate =>
            {
                observed.push(format!("gate:{:?}", gate.stage.as_ref()));
                if let Some(stage) = gate.stage.clone()
                    && confirmed_gates.insert(gate.gate_id)
                {
                    send_json(&mut ws, &CodingWsInMessage::StageGateConfirm { stage }).await;
                }
            }
            Ok(CodingWsOutMessage::CodingGateRequired { gate })
                if gate.kind == CodingGateKind::Blocked
                    && gate.stage == Some(CodingExecutionStage::InternalPrReview) =>
            {
                saw_internal_review_blocked_gate = true;
            }
            Ok(CodingWsOutMessage::InternalPrReviewComplete { .. }) => saw_internal_review = true,
            Ok(CodingWsOutMessage::CodingSessionState {
                status,
                stage,
                ..
            })
                if status == CodingAttemptStatus::Completed
                    && stage == CodingExecutionStage::ReviewRequest =>
            {
                observed.push(format!("state:{status:?}:{stage:?}"));
                completed_after_review_request = true;
                break;
            }
            Ok(CodingWsOutMessage::CodingProtocolError { code, message }) => {
                panic!("unexpected coding protocol error {code}: {message}");
            }
            Ok(_) | Err(_) => {}
        }
    }

    assert!(
        completed_after_review_request,
        "single WorkItem should complete after ReviewRequest; observed={observed:?}"
    );
    assert!(
        !saw_internal_review,
        "single WorkItem must not run InternalPrReview"
    );
    assert!(
        !saw_internal_review_blocked_gate,
        "single WorkItem must not expose InternalPrReview blocked gate"
    );
    let attempt = store
        .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("attempt");
    assert_eq!(attempt.status, CodingAttemptStatus::Completed);
    assert_eq!(attempt.stage, CodingExecutionStage::ReviewRequest);
    let runs = store
        .list_role_runs("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("role runs");
    assert!(
        runs.iter()
            .all(|run| run.role != CodingProviderRole::InternalReviewer),
        "internal reviewer role runs should not be created for single WorkItem"
    );
    assert!(
        store
            .list_internal_pr_reviews("project_0001", "issue_0001", "coding_attempt_0001")
            .expect("internal reviews")
            .is_empty(),
        "single WorkItem should not persist internal reviews"
    );

    ws.close(None).await.expect("close ws");
    server.abort();
}
