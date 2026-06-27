#[tokio::test]
async fn coding_ws_testing_blocked_waits_for_human_result_review_before_analyst() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let app =
        app_with_full_chain_attempt_and_provider(root.path(), Arc::new(TestingBlockedProvider));
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/ws/coding-attempts/coding_attempt_0001");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;

    send_json(&mut ws, &CodingWsInMessage::StartCoding).await;

    let gate = wait_for_testing_result_review_gate(&mut ws).await;
    assert_eq!(gate.kind, CodingGateKind::Blocked);
    assert_eq!(gate.stage, Some(CodingExecutionStage::Testing));
    assert_eq!(gate.role, Some(CodingProviderRole::Tester));
    assert_eq!(
        gate.reason_code.as_deref(),
        Some("testing_result_review_required")
    );
    assert!(
        gate.description.contains("测试被阻塞"),
        "expected blocked testing summary, got {}",
        gate.description
    );

    send_json(
        &mut ws,
        &CodingWsInMessage::GateResponse {
            gate_id: gate.gate_id,
            action_id: "accept_testing_result".to_string(),
            extra_context: None,
        },
    )
    .await;

    let mut saw_analyst = false;
    for _ in 0..80 {
        match recv_json(&mut ws).await {
            CodingWsOutMessage::CodingTimelineNodeCreated { node }
                if node.stage == CodingExecutionStage::Rework =>
            {
                saw_analyst = true;
                break;
            }
            CodingWsOutMessage::CodingProtocolError { code, message } => {
                panic!("unexpected coding protocol error {code}: {message}");
            }
            _ => {}
        }
    }

    assert!(
        saw_analyst,
        "testing blocked did not enter analyst after accept"
    );

    let attempt = store
        .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("attempt");
    assert_eq!(attempt.status, CodingAttemptStatus::Running);
    assert!(
        matches!(
            attempt.stage,
            CodingExecutionStage::Rework | CodingExecutionStage::CodeReview
        ),
        "expected Rework or CodeReview after accept, got {:?}",
        attempt.stage
    );

    let nodes = store
        .get_timeline_nodes("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("nodes");
    assert!(
        nodes
            .iter()
            .any(|node| node.stage == CodingExecutionStage::Rework)
    );

    ws.close(None).await.expect("close ws");
    server.abort();
}

#[tokio::test]
async fn coding_ws_testing_completion_waits_for_human_result_review() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let app = app_with_full_chain_attempt(root.path());
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/ws/coding-attempts/coding_attempt_0001");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;

    send_json(&mut ws, &CodingWsInMessage::StartCoding).await;

    let gate = wait_for_testing_result_review_gate(&mut ws).await;
    assert_eq!(gate.kind, CodingGateKind::Blocked);
    assert_eq!(gate.stage, Some(CodingExecutionStage::Testing));
    assert_eq!(gate.role, Some(CodingProviderRole::Tester));
    assert_eq!(
        gate.reason_code.as_deref(),
        Some("testing_result_review_required")
    );
    assert!(
        gate.available_actions
            .iter()
            .any(|action| action.action_id == "accept_testing_result"
                && action.action_type == CodingGateActionType::AcceptTestingResult),
        "expected accept_testing_result action, got {:?}",
        gate.available_actions
    );
    assert!(
        gate.available_actions
            .iter()
            .any(|action| action.action_id == "rerun_testing"
                && action.action_type == CodingGateActionType::RerunTesting),
        "expected rerun_testing action, got {:?}",
        gate.available_actions
    );
    assert!(
        gate.evidence_refs
            .iter()
            .any(|reference| reference == "testing_report_0001.json"),
        "expected testing report evidence ref, got {:?}",
        gate.evidence_refs
    );

    assert!(
        store
            .list_open_blocked_gates("project_0001", "issue_0001", "coding_attempt_0001")
            .expect("open gates")
            .iter()
            .any(|gate| gate.reason_code.as_deref() == Some("testing_result_review_required"))
    );
    let attempt = store
        .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("attempt");
    assert_eq!(attempt.status, CodingAttemptStatus::Blocked);
    assert_eq!(attempt.stage, CodingExecutionStage::Testing);
    assert!(
        store
            .list_role_runs("project_0001", "issue_0001", "coding_attempt_0001")
            .expect("role runs")
            .iter()
            .all(|run| run.role != CodingProviderRole::Analyst),
        "analyst must not start before human accepts tester result"
    );

    ws.close(None).await.expect("close ws");
    server.abort();
}

#[tokio::test]
async fn coding_ws_accept_testing_result_enters_analyst_with_testing_report_evidence() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let app = app_with_full_chain_attempt(root.path());
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/ws/coding-attempts/coding_attempt_0001");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;

    send_json(&mut ws, &CodingWsInMessage::StartCoding).await;
    let gate = wait_for_testing_result_review_gate(&mut ws).await;

    send_json(
        &mut ws,
        &CodingWsInMessage::GateResponse {
            gate_id: gate.gate_id,
            action_id: "accept_testing_result".to_string(),
            extra_context: None,
        },
    )
    .await;

    let mut saw_analyst = false;
    for _ in 0..80 {
        match recv_json(&mut ws).await {
            CodingWsOutMessage::CodingTimelineNodeCreated { node }
                if node.stage == CodingExecutionStage::Rework =>
            {
                saw_analyst = true;
                break;
            }
            CodingWsOutMessage::CodingProtocolError { code, message } => {
                panic!("unexpected coding protocol error {code}: {message}");
            }
            _ => {}
        }
    }
    assert!(saw_analyst, "accepting tester result did not start analyst");

    let runs = store
        .list_role_runs("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("role runs");
    let analyst_run = runs
        .iter()
        .find(|run| run.role == CodingProviderRole::Analyst)
        .expect("analyst role run");
    let evidence_ref = analyst_run
        .artifact_refs
        .iter()
        .find(|reference| reference.contains("analyst_evidence"))
        .expect("analyst evidence ref");
    let evidence = store
        .read_attempt_artifact_text("coding_attempt_0001", evidence_ref)
        .expect("analyst evidence");
    assert!(
        evidence.contains("\"id\": \"testing_report_0001\""),
        "expected TestingReport JSON evidence, got {evidence}"
    );

    ws.close(None).await.expect("close ws");
    server.abort();
}

#[tokio::test]
async fn coding_ws_rerun_testing_result_review_reexecutes_tester() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let provider = Arc::new(RerunTestingProvider::default());
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
    let first_gate = wait_for_testing_result_review_gate(&mut ws).await;
    assert_eq!(provider.testing_execute_calls(), 1);

    send_json(
        &mut ws,
        &CodingWsInMessage::GateResponse {
            gate_id: first_gate.gate_id,
            action_id: "rerun_testing".to_string(),
            extra_context: None,
        },
    )
    .await;

    let mut confirmed_stage_gates = HashSet::new();
    let mut second_gate = None;
    for _ in 0..120 {
        match recv_json(&mut ws).await {
            CodingWsOutMessage::CodingGateRequired { gate }
                if gate.kind == CodingGateKind::StageGate =>
            {
                if let Some(stage) = gate.stage
                    && confirmed_stage_gates.insert(gate.gate_id)
                {
                    send_json(&mut ws, &CodingWsInMessage::StageGateConfirm { stage }).await;
                }
            }
            CodingWsOutMessage::CodingGateRequired { gate }
                if gate.reason_code.as_deref() == Some("testing_result_review_required")
                    && gate
                        .evidence_refs
                        .iter()
                        .any(|reference| reference == "testing_report_0002.json") =>
            {
                second_gate = Some(gate);
                break;
            }
            CodingWsOutMessage::CodingSessionState { pending_gates, .. } => {
                if let Some(gate) = pending_gates.iter().find(|gate| {
                    gate.reason_code.as_deref() == Some("testing_result_review_required")
                        && gate
                            .evidence_refs
                            .iter()
                            .any(|reference| reference == "testing_report_0002.json")
                }) {
                    second_gate = Some(gate.clone());
                    break;
                }
                for gate in pending_gates
                    .into_iter()
                    .filter(|gate| gate.kind == CodingGateKind::StageGate)
                {
                    if let Some(stage) = gate.stage
                        && confirmed_stage_gates.insert(gate.gate_id)
                    {
                        send_json(&mut ws, &CodingWsInMessage::StageGateConfirm { stage }).await;
                    }
                }
            }
            CodingWsOutMessage::CodingProtocolError { code, message } => {
                panic!("unexpected coding protocol error {code}: {message}");
            }
            _ => {}
        }
    }

    let second_gate = second_gate.expect("second testing result review gate");
    assert_eq!(second_gate.stage, Some(CodingExecutionStage::Testing));
    assert_eq!(provider.testing_execute_calls(), 2);
    let reports = store
        .list_testing_reports("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("testing reports");
    assert_eq!(reports.len(), 2);
    let runs = store
        .list_role_runs("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("role runs");
    let tester_runs = runs
        .iter()
        .filter(|run| run.role == CodingProviderRole::Tester)
        .collect::<Vec<_>>();
    assert_eq!(tester_runs.len(), 2);
    assert_eq!(tester_runs[0].status, CodingRoleRunStatus::Superseded);
    assert_eq!(tester_runs[1].trigger, CodingRoleRunTrigger::ManualRerun);

    ws.close(None).await.expect("close ws");
    server.abort();
}

#[tokio::test]
async fn coding_ws_code_review_blocked_enters_analyst_before_coding() {
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
    let mut saw_analyst_after_code_review = false;
    let mut saw_coding_gate_after_analyst = false;
    let mut accepted_testing_result_gates = HashSet::new();
    for _ in 0..160 {
        match recv_json(&mut ws).await {
            CodingWsOutMessage::CodingGateRequired { gate }
                if gate.kind == CodingGateKind::Blocked
                    && gate.stage.as_ref() == Some(&CodingExecutionStage::CodeReview) =>
            {
                panic!("code review blocked should be routed to analyst, got gate {gate:?}");
            }
            CodingWsOutMessage::CodingGateRequired { gate }
                if is_testing_result_review_gate(&gate)
                    && accepted_testing_result_gates.insert(gate.gate_id.clone()) =>
            {
                respond_to_testing_result_review_gate(&mut ws, &gate).await;
            }
            CodingWsOutMessage::CodingSessionState {
                status,
                pending_gates,
                ..
            } => {
                if let Some(gate) = pending_gates.iter().find(|gate| {
                    gate.kind == CodingGateKind::Blocked
                        && gate.stage.as_ref() == Some(&CodingExecutionStage::CodeReview)
                }) {
                    panic!("code review blocked should be routed to analyst, got gate {gate:?}");
                }
                let mut responded_to_testing_result = false;
                if status == CodingAttemptStatus::Blocked {
                    for gate in pending_gates
                        .iter()
                        .filter(|gate| is_testing_result_review_gate(gate))
                    {
                        if accepted_testing_result_gates.insert(gate.gate_id.clone()) {
                            respond_to_testing_result_review_gate(&mut ws, gate).await;
                            responded_to_testing_result = true;
                        }
                    }
                }
                if responded_to_testing_result {
                    continue;
                }
                let stage_gates = pending_gates
                    .iter()
                    .filter(|gate| gate.kind == CodingGateKind::StageGate)
                    .filter_map(|gate| {
                        gate.stage
                            .clone()
                            .map(|stage| (gate.gate_id.clone(), stage))
                    })
                    .collect::<Vec<_>>();
                if saw_analyst_after_code_review
                    && stage_gates
                        .iter()
                        .any(|(_, stage)| *stage == CodingExecutionStage::Coding)
                {
                    saw_coding_gate_after_analyst = true;
                    break;
                }
                for (gate_id, stage) in stage_gates {
                    if confirmed_gates.insert(gate_id) {
                        send_json(&mut ws, &CodingWsInMessage::StageGateConfirm { stage }).await;
                    }
                }
            }
            CodingWsOutMessage::CodingGateRequired { gate }
                if gate.kind == CodingGateKind::StageGate =>
            {
                if saw_analyst_after_code_review
                    && gate.stage.as_ref() == Some(&CodingExecutionStage::Coding)
                {
                    saw_coding_gate_after_analyst = true;
                    break;
                }
                if let Some(stage) = gate.stage.clone()
                    && confirmed_gates.insert(gate.gate_id)
                {
                    send_json(&mut ws, &CodingWsInMessage::StageGateConfirm { stage }).await;
                }
            }
            CodingWsOutMessage::CodeReviewComplete { report }
                if report.verdict == ReviewVerdict::Blocked =>
            {
                saw_code_review_blocked = true;
            }
            CodingWsOutMessage::CodingTimelineNodeCreated { node }
                if saw_code_review_blocked && node.stage == CodingExecutionStage::Rework =>
            {
                saw_analyst_after_code_review = true;
            }
            CodingWsOutMessage::CodingProtocolError { code, message } => {
                panic!("unexpected coding protocol error {code}: {message}");
            }
            _ => {}
        }
    }

    assert!(
        saw_code_review_blocked,
        "code review blocked report missing"
    );
    assert!(
        saw_analyst_after_code_review,
        "code review blocked did not enter analyst"
    );
    assert!(
        saw_coding_gate_after_analyst,
        "analyst next_stage=coding did not route back to coder"
    );
    assert!(
        provider
            .analyst_prompts()
            .iter()
            .any(|prompt| prompt.contains("Previous Stage: CodeReview")),
        "analyst did not receive code review evidence"
    );

    let attempt = store
        .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("attempt");
    assert_eq!(attempt.status, CodingAttemptStatus::Running);
    assert_eq!(attempt.stage, CodingExecutionStage::Coding);

    ws.close(None).await.expect("close ws");
    server.abort();
}

#[tokio::test]
async fn coding_ws_internal_pr_review_blocked_enters_analyst_before_final_confirm() {
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
    let mut saw_analyst_after_internal_review = false;
    let mut completed = false;
    let mut accepted_testing_result_gates = HashSet::new();
    for _ in 0..220 {
        match recv_json(&mut ws).await {
            CodingWsOutMessage::CodingGateRequired { gate }
                if gate.kind == CodingGateKind::Blocked
                    && gate.stage.as_ref() == Some(&CodingExecutionStage::InternalPrReview) =>
            {
                panic!("internal review blocked should be routed to analyst, got gate {gate:?}");
            }
            CodingWsOutMessage::CodingGateRequired { gate }
                if is_testing_result_review_gate(&gate)
                    && accepted_testing_result_gates.insert(gate.gate_id.clone()) =>
            {
                respond_to_testing_result_review_gate(&mut ws, &gate).await;
            }
            CodingWsOutMessage::CodingSessionState {
                status,
                stage,
                pending_gates,
                ..
            } => {
                if let Some(gate) = pending_gates.iter().find(|gate| {
                    gate.kind == CodingGateKind::Blocked
                        && gate.stage.as_ref() == Some(&CodingExecutionStage::InternalPrReview)
                }) {
                    panic!(
                        "internal review blocked should be routed to analyst, got gate {gate:?}"
                    );
                }
                let mut responded_to_testing_result = false;
                if status == CodingAttemptStatus::Blocked {
                    for gate in pending_gates
                        .iter()
                        .filter(|gate| is_testing_result_review_gate(gate))
                    {
                        if accepted_testing_result_gates.insert(gate.gate_id.clone()) {
                            respond_to_testing_result_review_gate(&mut ws, gate).await;
                            responded_to_testing_result = true;
                        }
                    }
                }
                if responded_to_testing_result {
                    continue;
                }
                let stage_gates = pending_gates
                    .iter()
                    .filter(|gate| gate.kind == CodingGateKind::StageGate)
                    .filter_map(|gate| {
                        gate.stage
                            .clone()
                            .map(|stage| (gate.gate_id.clone(), stage))
                    })
                    .collect::<Vec<_>>();
                for (gate_id, stage) in stage_gates {
                    if confirmed_gates.insert(gate_id) {
                        send_json(&mut ws, &CodingWsInMessage::StageGateConfirm { stage }).await;
                    }
                }
                if saw_analyst_after_internal_review
                    && status == CodingAttemptStatus::Completed
                    && stage == CodingExecutionStage::FinalConfirm
                {
                    completed = true;
                    break;
                }
            }
            CodingWsOutMessage::CodingGateRequired { gate }
                if gate.kind == CodingGateKind::StageGate =>
            {
                if let Some(stage) = gate.stage.clone()
                    && confirmed_gates.insert(gate.gate_id)
                {
                    send_json(&mut ws, &CodingWsInMessage::StageGateConfirm { stage }).await;
                }
            }
            CodingWsOutMessage::InternalPrReviewComplete { review }
                if review.verdict == ReviewVerdict::Blocked =>
            {
                saw_internal_review_blocked = true;
            }
            CodingWsOutMessage::CodingTimelineNodeCreated { node }
                if saw_internal_review_blocked && node.stage == CodingExecutionStage::Rework =>
            {
                saw_analyst_after_internal_review = true;
            }
            CodingWsOutMessage::CodingProtocolError { code, message } => {
                panic!("unexpected coding protocol error {code}: {message}");
            }
            _ => {}
        }
    }

    assert!(
        saw_internal_review_blocked,
        "internal review blocked report missing"
    );
    assert!(
        saw_analyst_after_internal_review,
        "internal review blocked did not enter analyst"
    );
    assert!(
        completed,
        "analyst next_stage=final_confirm did not complete final confirm path"
    );
    assert!(
        provider
            .analyst_prompts()
            .iter()
            .any(|prompt| prompt.contains("Previous Stage: InternalPrReview")),
        "analyst did not receive internal review evidence"
    );

    let attempt = store
        .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("attempt");
    assert_eq!(attempt.status, CodingAttemptStatus::Completed);
    assert_eq!(attempt.stage, CodingExecutionStage::FinalConfirm);

    ws.close(None).await.expect("close ws");
    server.abort();
}

