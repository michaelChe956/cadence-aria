#[tokio::test]
async fn coding_plan_repair_group_final_review_orchestrated_path_routes_plan_finding_without_completing()
{
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let app = app_with_group_full_chain_attempt_and_provider(
        root.path(),
        Arc::new(GroupFinalReviewPlanDefectProvider::recovery()),
    );
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
    let mut bound_first_handoff = false;
    let mut saw_group_review = false;
    for _ in 0..560 {
        match timeout(Duration::from_secs(2), recv_json(&mut ws)).await {
            Ok(CodingWsOutMessage::CodingGateRequired { gate })
                if gate.kind == CodingGateKind::StageGate =>
            {
                if let Some(stage) = gate.stage.clone()
                    && confirmed_gates.insert(gate.gate_id)
                {
                    if stage == CodingExecutionStage::InternalPrReview {
                        materialize_completed_unit_run_for_logical(&store, "work_item_0002");
                    }
                    send_json(&mut ws, &CodingWsInMessage::StageGateConfirm { stage }).await;
                }
            }
            Ok(CodingWsOutMessage::CodingSessionState {
                current_work_item_id,
                ..
            }) if current_work_item_id.as_deref() == Some("work_item_0002")
                && !bound_first_handoff =>
            {
                bind_completed_first_unit_handoff_revision(&store);
                bound_first_handoff = true;
            }
            Ok(CodingWsOutMessage::InternalPrReviewComplete { review })
                if !review.findings.is_empty() =>
            {
                assert_eq!(review.findings.len(), 1);
                saw_group_review = true;
                break;
            }
            Ok(CodingWsOutMessage::CodingProtocolError { code, message }) => {
                panic!("unexpected coding protocol error {code}: {message}");
            }
            Ok(_) => {}
            Err(_) => panic!("timed out before recovered GroupFinalReview"),
        }
    }
    assert!(saw_group_review, "expected orchestrated GroupFinalReview");
    tokio::task::yield_now().await;
    let attempt = store
        .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("attempt");
    assert_ne!(attempt.status, CodingAttemptStatus::Completed);
    assert_eq!(attempt.stage, CodingExecutionStage::InternalPrReview);

    ws.close(None).await.expect("close ws");
    server.abort();
}
