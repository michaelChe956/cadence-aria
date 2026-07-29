fn authoritative_group_blocker_rules_fixture() -> Vec<BlockerRule> {
    vec![
        BlockerRule {
            reason_code: "current_work_item_contract_invalid".to_string(),
            route: BlockerRoute::PlanRepairCurrent,
            target_contract_refs: Vec::new(),
        },
        BlockerRule {
            reason_code: "story_contract_invalid".to_string(),
            route: BlockerRoute::StoryAmendment,
            target_contract_refs: Vec::new(),
        },
        BlockerRule {
            reason_code: "design_contract_invalid".to_string(),
            route: BlockerRoute::DesignAmendment,
            target_contract_refs: Vec::new(),
        },
        BlockerRule {
            reason_code: "operational_environment_blocked".to_string(),
            route: BlockerRoute::OperationalGate,
            target_contract_refs: Vec::new(),
        },
        BlockerRule {
            reason_code: "verification_evidence_incomplete".to_string(),
            route: BlockerRoute::VerificationRetry,
            target_contract_refs: Vec::new(),
        },
    ]
}

#[derive(Clone, Copy, Debug)]
enum ReviewerTriggeredReworkCase {
    Plan,
    Story,
    Design,
    Operational,
    Verification,
    Invalid,
    Implementation,
}

impl ReviewerTriggeredReworkCase {
    fn label(self) -> &'static str {
        match self {
            Self::Plan => "plan",
            Self::Story => "story",
            Self::Design => "design",
            Self::Operational => "operational",
            Self::Verification => "verification",
            Self::Invalid => "invalid",
            Self::Implementation => "implementation",
        }
    }

    fn expected_route(self) -> Option<&'static str> {
        match self {
            Self::Plan => Some("start_plan_repair"),
            Self::Story => Some("start_story_amendment"),
            Self::Design => Some("start_design_amendment"),
            Self::Operational => Some("open_operational_gate"),
            Self::Verification => Some("retry_verification"),
            Self::Invalid => Some("stop_for_human_triage"),
            Self::Implementation => None,
        }
    }

    fn rework_output(self) -> String {
        let finding = match self {
            Self::Plan => rework_plan_finding(
                "current_work_item_invalid",
                "current_work_item_contract_invalid",
                "plan_repair",
                Some(serde_json::json!({
                    "kind": "current_work_item",
                    "logical_work_item_ids": ["work_item_0001"],
                    "work_item_revision_ids": ["work_item_revision_0001"]
                })),
            ),
            Self::Story => rework_plan_finding(
                "story_amendment_required",
                "story_contract_invalid",
                "story_amendment",
                None,
            ),
            Self::Design => rework_plan_finding(
                "design_amendment_required",
                "design_contract_invalid",
                "design_amendment",
                None,
            ),
            Self::Operational => rework_plan_finding(
                "operational_blocker",
                "operational_environment_blocked",
                "operational_gate",
                None,
            ),
            Self::Verification => rework_plan_finding(
                "verification_incomplete",
                "verification_evidence_incomplete",
                "verification_retry",
                None,
            ),
            Self::Invalid => rework_plan_finding(
                "story_amendment_required",
                "story_contract_invalid",
                "plan_repair",
                None,
            ),
            Self::Implementation => return serde_json::json!({
                "plan_defect_findings": []
            })
            .to_string(),
        };
        serde_json::json!({"plan_defect_findings": [finding]}).to_string()
    }
}

fn rework_plan_finding(
    defect_class: &str,
    reason_code: &str,
    recommended_route: &str,
    repair_target: Option<serde_json::Value>,
) -> serde_json::Value {
    serde_json::json!({
        "finding_id": "coder_rework_plan_defect_0001",
        "severity": "error",
        "defect_class": defect_class,
        "reason_code": reason_code,
        "message": "reviewer-triggered rework exposed a plan defect",
        "evidence": [],
        "contract_refs": [],
        "capability_refs": [],
        "repair_target": repair_target,
        "recommended_route": recommended_route,
        "confidence": "high"
    })
}

struct ReviewerTriggeredReworkProvider {
    case: ReviewerTriggeredReworkCase,
    executor_calls: std::sync::atomic::AtomicUsize,
    reviewer_calls: std::sync::atomic::AtomicUsize,
}

impl ReviewerTriggeredReworkProvider {
    fn new(case: ReviewerTriggeredReworkCase) -> Self {
        Self {
            case,
            executor_calls: std::sync::atomic::AtomicUsize::new(0),
            reviewer_calls: std::sync::atomic::AtomicUsize::new(0),
        }
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for ReviewerTriggeredReworkProvider {
    async fn run_streaming(
        &self,
        input: &AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        let (tx, rx) = mpsc::channel(8);
        let full_output = match input.role {
            AdapterRole::Executor => {
                let call = self
                    .executor_calls
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if call == 0 {
                    let worktree = input
                        .worktree_path
                        .as_ref()
                        .map(PathBuf::from)
                        .expect("worktree path");
                    fs::write(worktree.join("src/lib.rs"), CLIMB_STAIRS_LIB).map_err(|error| {
                        ProviderAdapterError::incompatible_output(error.to_string(), "", "")
                    })?;
                    "implemented climb_stairs".to_string()
                } else {
                    self.case.rework_output()
                }
            }
            AdapterRole::Reviewer => {
                let call = self
                    .reviewer_calls
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if call == 0 {
                    serde_json::json!({
                        "verdict": "request_changes",
                        "summary": "reviewer requested a legal implementation rework",
                        "findings": [{
                            "severity": "error",
                            "file_path": "src/lib.rs",
                            "message": "implementation must be corrected",
                            "required_action": "fix implementation",
                            "defect_class": "implementation_defect",
                            "recommended_route": "coder_rework"
                        }]
                    })
                    .to_string()
                } else {
                    serde_json::json!({
                        "verdict": "blocked",
                        "summary": "second reviewer invocation proves rework did not safe-stop",
                        "findings": []
                    })
                    .to_string()
                }
            }
            _ => "ok".to_string(),
        };
        tx.try_send(StreamChunk::Done { full_output })
            .expect("send provider output");
        Ok(rx)
    }
}

async fn run_reviewer_triggered_rework_case(
    case: ReviewerTriggeredReworkCase,
) -> (CodingExecutionAttempt, usize, bool, bool) {
    let root = tempdir().expect("root");
    let provider = Arc::new(ReviewerTriggeredReworkProvider::new(case));
    let app = app_with_group_full_chain_attempt_and_provider(root.path(), provider.clone());
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

    let mut confirmed_gates = HashSet::new();
    let mut code_review_gate_count = 0;
    let mut saw_route = false;
    let mut saw_second_review_gate = false;
    let mut saw_protocol_error = false;
    let mut emitted_plan_repair_request = None;
    let mut saw_plan_repair_state = false;
    for _ in 0..180 {
        match timeout(Duration::from_millis(500), recv_json(&mut ws)).await {
            Ok(CodingWsOutMessage::CodingGateRequired { gate })
                if gate.kind == CodingGateKind::StageGate =>
            {
                if gate.stage == Some(CodingExecutionStage::CodeReview) {
                    code_review_gate_count += 1;
                    saw_second_review_gate = code_review_gate_count > 1;
                }
                if let Some(stage) = gate.stage.clone()
                    && confirmed_gates.insert(gate.gate_id)
                {
                    send_json(&mut ws, &CodingWsInMessage::StageGateConfirm { stage }).await;
                }
            }
            Ok(CodingWsOutMessage::CodingChatEntryCreated { entry })
                if case.expected_route().is_some_and(|expected| {
                    entry.metadata.as_ref().is_some_and(|metadata| {
                        metadata.get("source").and_then(|value| value.as_str()) == Some("coding")
                            && metadata
                                .get("plan_defect_route")
                                .and_then(|value| value.as_str())
                                == Some(expected)
                    })
                }) =>
            {
                saw_route = true;
            }
            Ok(CodingWsOutMessage::CodingSessionState { status, stage, .. })
                if matches!(case, ReviewerTriggeredReworkCase::Plan)
                    && stage == CodingExecutionStage::Coding
                    && status == CodingAttemptStatus::AwaitingPlanAmendment =>
            {
                saw_plan_repair_state = true;
                if emitted_plan_repair_request.is_some() {
                    break;
                }
            }
            Ok(CodingWsOutMessage::CodingSessionState { stage, .. })
                if saw_route
                    && !matches!(case, ReviewerTriggeredReworkCase::Plan)
                    && stage == CodingExecutionStage::Coding =>
            {
                break;
            }
            Ok(CodingWsOutMessage::PlanRepairRequired { request, .. })
                if matches!(case, ReviewerTriggeredReworkCase::Plan) =>
            {
                emitted_plan_repair_request = Some(*request);
                saw_route = true;
                if saw_plan_repair_state {
                    break;
                }
            }
            Ok(CodingWsOutMessage::CodeReviewComplete { .. })
                if provider
                    .reviewer_calls
                    .load(std::sync::atomic::Ordering::SeqCst)
                    > 1 =>
            {
                break;
            }
            Ok(CodingWsOutMessage::CodingProtocolError { .. }) => {
                saw_protocol_error = true;
                break;
            }
            Ok(_) => {}
            Err(_) if saw_route && !matches!(case, ReviewerTriggeredReworkCase::Plan) => break,
            Err(_) if matches!(case, ReviewerTriggeredReworkCase::Plan) => continue,
            Err(_) => panic!("{} rework case timed out", case.label()),
        }
    }

    let attempt = store
        .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("attempt");
    let reviewer_calls = provider
        .reviewer_calls
        .load(std::sync::atomic::Ordering::SeqCst);
    if matches!(case, ReviewerTriggeredReworkCase::Plan) {
        let revision_store = WorkItemRevisionStore::new(store.paths());
        let lineage = revision_store
            .get_plan_lineage("project_0001", "issue_0001", "work_item_plan_0001")
            .expect("lineage");
        let requests = revision_store
            .list_repair_requests(&lineage)
            .expect("repair requests");
        assert_eq!(requests.len(), 1);
        assert_eq!(
            emitted_plan_repair_request.as_ref(),
            requests.first(),
            "PlanRepairRequired must expose the durable request"
        );
        assert_eq!(requests[0].trigger_review_id, None);
        assert_eq!(
            requests[0].trigger_finding_id,
            "coder_rework_plan_defect_0001"
        );
        let unit = store
            .get_active_coding_unit(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("active unit")
            .expect("active unit");
        let trigger_run = store
            .list_coding_unit_runs(&attempt, &unit.id)
            .expect("unit runs")
            .into_iter()
            .find(|run| run.id == requests[0].trigger_unit_run_id)
            .expect("trigger unit run");
        assert_eq!(trigger_run.verification_retry_count, 0, "{}", case.label());
        assert_eq!(trigger_run.operational_retry_count, 0, "{}", case.label());
        assert_eq!(trigger_run.plan_repair_count, 1, "{}", case.label());
        assert_eq!(
            trigger_run.status,
            cadence_aria::product::coding_models::CodingUnitRunStatus::BlockedByPlanDefect,
            "{}",
            case.label()
        );
    } else if case.expected_route().is_some() {
        let unit_run = store.get_active_unit_run(&attempt).expect("active unit run");
        assert_eq!(unit_run.verification_retry_count, 0, "{}", case.label());
        assert_eq!(unit_run.operational_retry_count, 0, "{}", case.label());
        assert_eq!(unit_run.plan_repair_count, 0, "{}", case.label());
        assert_eq!(
            unit_run.status,
            cadence_aria::product::coding_models::CodingUnitRunStatus::Running,
            "{}",
            case.label()
        );
        let revision_store = WorkItemRevisionStore::new(store.paths());
        let lineage = revision_store
            .get_plan_lineage("project_0001", "issue_0001", "work_item_plan_0001")
            .expect("lineage");
        assert!(
            revision_store
                .list_open_repair_requests(&lineage)
                .expect("repair requests")
                .is_empty(),
            "{} must not create Task 3 request",
            case.label()
        );
        if matches!(case, ReviewerTriggeredReworkCase::Invalid) {
            let gates = store
                .list_open_blocked_gates("project_0001", "issue_0001", "coding_attempt_0001")
                .expect("blocked gates");
            assert_eq!(
                gates.len(),
                1,
                "{} must open a human triage gate instead of stalling silently",
                case.label()
            );
            assert_eq!(
                gates[0].reason_code.as_deref(),
                Some("coding_output_human_triage")
            );
        } else {
            assert!(
                store
                    .list_open_blocked_gates("project_0001", "issue_0001", "coding_attempt_0001")
                    .expect("blocked gates")
                    .is_empty(),
                "{} must not create a generic blocked gate",
                case.label()
            );
        }
    }

    ws.close(None).await.expect("close ws");
    server.abort();
    (attempt, reviewer_calls, saw_second_review_gate, saw_protocol_error)
}

#[tokio::test]
async fn coding_plan_repair_reviewer_triggered_rework_routes_plan_and_safe_stops_other_sources() {
    let _guard = WS_TEST_LOCK.lock().await;
    for case in [
        ReviewerTriggeredReworkCase::Plan,
        ReviewerTriggeredReworkCase::Story,
        ReviewerTriggeredReworkCase::Design,
        ReviewerTriggeredReworkCase::Operational,
        ReviewerTriggeredReworkCase::Verification,
        ReviewerTriggeredReworkCase::Invalid,
    ] {
        let (attempt, reviewer_calls, saw_second_review_gate, saw_protocol_error) =
            run_reviewer_triggered_rework_case(case).await;
        assert!(!saw_protocol_error, "{} must fail closed", case.label());
        assert_eq!(reviewer_calls, 1, "{} must not invoke reviewer again", case.label());
        assert!(
            !saw_second_review_gate,
            "{} must not open another CodeReview gate",
            case.label()
        );
        assert_eq!(
            attempt.status,
            if matches!(case, ReviewerTriggeredReworkCase::Plan) {
                CodingAttemptStatus::AwaitingPlanAmendment
            } else if matches!(case, ReviewerTriggeredReworkCase::Invalid) {
                CodingAttemptStatus::Blocked
            } else {
                CodingAttemptStatus::Running
            },
            "{}",
            case.label()
        );
        assert_eq!(attempt.stage, CodingExecutionStage::Coding, "{}", case.label());
    }
}

#[tokio::test]
async fn coding_plan_repair_reviewer_triggered_implementation_rework_continues_code_review() {
    let _guard = WS_TEST_LOCK.lock().await;
    let (attempt, reviewer_calls, saw_second_review_gate, saw_protocol_error) =
        run_reviewer_triggered_rework_case(ReviewerTriggeredReworkCase::Implementation).await;

    assert!(!saw_protocol_error);
    assert_eq!(reviewer_calls, 2);
    assert!(saw_second_review_gate);
    assert_eq!(attempt.stage, CodingExecutionStage::CodeReview);
}
