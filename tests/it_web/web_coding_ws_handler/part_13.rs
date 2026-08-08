#[tokio::test]
async fn coding_plan_repair_group_final_review_orchestrated_path_routes_plan_finding_without_completing()
{
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let app = app_with_group_full_chain_attempt_and_provider(
        root.path(),
        Arc::new(IndependentCodeReviewPlanDefectProvider::recovery()),
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
    let mut retried_code_review = false;
    let mut emitted_plan_repair_request = None;
    let mut saw_plan_repair_state = false;
    for _ in 0..360 {
        match timeout(Duration::from_secs(2), recv_json(&mut ws)).await {
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
                    && gate.stage == Some(CodingExecutionStage::CodeReview)
                    && gate
                        .available_actions
                        .iter()
                        .any(|action| action.action_id == "retry_review") =>
            {
                send_json(
                    &mut ws,
                    &CodingWsInMessage::GateResponse {
                        gate_id: gate.gate_id,
                        action_id: "retry_review".to_string(),
                        extra_context: None,
                    },
                )
                .await;
                retried_code_review = true;
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
            Ok(CodingWsOutMessage::CodingSessionState { status, stage, .. })
                if status == CodingAttemptStatus::AwaitingPlanAmendment
                    && stage == CodingExecutionStage::CodeReview =>
            {
                saw_plan_repair_state = true;
                if emitted_plan_repair_request.is_some() {
                    break;
                }
            }
            Ok(CodingWsOutMessage::PlanRepairRequired { request, .. }) => {
                emitted_plan_repair_request = Some(*request);
                if saw_plan_repair_state {
                    break;
                }
            }
            Ok(CodingWsOutMessage::CodingProtocolError { code, message }) => {
                panic!("unexpected coding protocol error {code}: {message}");
            }
            Ok(_) => {}
            Err(_) if emitted_plan_repair_request.is_some() && saw_plan_repair_state => break,
            Err(_) => {
                let attempt = store
                    .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
                    .expect("attempt after timeout");
                let gates = store
                    .list_open_blocked_gates("project_0001", "issue_0001", "coding_attempt_0001")
                    .expect("blocked gates");
                let reports = store
                    .list_code_review_reports("project_0001", "issue_0001", "coding_attempt_0001")
                    .expect("code review reports");
                panic!(
                    "timed out before recovered CodeReview routed the plan finding: status={:?} stage={:?} current={:?} gates={gates:?} reports={reports:?}",
                    attempt.status, attempt.stage, attempt.current_work_item_id,
                );
            }
        }
    }

    assert!(retried_code_review, "expected independent CodeReview retry gate");
    assert!(
        saw_plan_repair_state,
        "recovered review plan finding must safe-stop the group for Plan Repair"
    );
    let request = emitted_plan_repair_request.expect("expected PlanRepairRequired from CodeReview");
    let attempt = store
        .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("attempt");
    assert_eq!(attempt.status, CodingAttemptStatus::AwaitingPlanAmendment);
    assert_eq!(attempt.stage, CodingExecutionStage::CodeReview);
    assert_ne!(attempt.status, CodingAttemptStatus::Completed);
    let revision_store = WorkItemRevisionStore::new(store.paths());
    let lineage = revision_store
        .get_plan_lineage("project_0001", "issue_0001", "work_item_plan_0001")
        .expect("plan lineage");
    let requests = revision_store
        .list_repair_requests(&lineage)
        .expect("repair requests");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0], request);
    assert_eq!(requests[0].trigger_review_id.as_deref(), Some("code_review_0003"));
    assert_eq!(
        requests[0].trigger_finding_id,
        "code_review_0003_finding_0001"
    );
    assert!(
        store
            .list_internal_pr_reviews("project_0001", "issue_0001", "coding_attempt_0001")
            .expect("internal reviews")
            .is_empty(),
        "fresh groups must not run the removed provider group review"
    );

    ws.close(None).await.expect("close ws");
    server.abort();
}
