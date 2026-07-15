async fn queued_work_item_plan_outline_review_engine(
    session_id: &str,
) -> (
    TempDir,
    WorkspaceEngine,
    mpsc::Receiver<EngineEvent>,
    String,
) {
    let (tmp, _checkpoint_store, _lifecycle, _plan_id, mut engine) =
        make_work_item_plan_engine_with_draft_candidate(session_id);
    let (tx, rx) = mpsc::channel(64);
    engine.event_tx = tx;
    engine.session.artifact = Some(ArtifactPayload::WorkItemPlanOutlineCandidate {
        outline_candidate: Box::new(WorkItemPlanOutlineCandidateDto {
            outline: test_work_item_plan_outline(Vec::new()),
            design_context_gaps: Vec::new(),
            validator_findings: Vec::new(),
            context_blockers: Vec::new(),
            current_generation_round_id: Some("round_0001".to_string()),
            selected_generation_mode: None,
        }),
    });
    engine.begin_work_item_plan_outline_review_run().await;
    let review_node_id = engine
        .active_node_id
        .clone()
        .expect("active work item plan outline review node");
    (tmp, engine, rx, review_node_id)
}

fn work_item_plan_outline_revise_json() -> &'static str {
    r#"{
        "verdict": "revise",
        "review_scope": "outline",
        "generation_round_id": "round_0001",
        "summary": "调整 Outline 拆分边界",
        "findings": [{
            "severity": "must_fix",
            "message": "Outline 写入范围重叠",
            "evidence": "outline_a 与 outline_b 共享写入范围",
            "impact": "并行实现会产生冲突",
            "required_action": "拆分 exclusive_write_scopes"
        }]
    }"#
}

#[tokio::test]
async fn work_item_plan_outline_review_repair_preserves_scope_and_business_payload() {
    let review_json = work_item_plan_outline_revise_json();
    let provider = QueuedReviewProvider::new(vec![
        missing_end_nonce_output(review_json),
        valid_structured_output(review_json),
    ]);
    let (_tmp, mut engine, mut rx, review_node_id) =
        queued_work_item_plan_outline_review_engine("sess_wip_outline_repair_success").await;

    engine
        .drive_review_session(Arc::new(provider.clone()), empty_provider_commands())
        .await;

    assert_eq!(provider.starts.load(Ordering::SeqCst), 2);
    assert_eq!(
        provider.resume_provider_session_ids.lock().unwrap()[1],
        Some("review-session-1".to_string())
    );
    let review_nodes = engine
        .timeline_nodes
        .iter()
        .filter(|node| node.node_type == TimelineNodeType::WorkItemPlanOutlineReview)
        .collect::<Vec<_>>();
    assert_eq!(review_nodes.len(), 1);
    assert_eq!(review_nodes[0].node_id, review_node_id);
    assert_eq!(review_nodes[0].round, Some(1));

    let verdict = engine
        .latest_review_verdict
        .as_ref()
        .expect("repaired work item plan verdict");
    assert_eq!(verdict.verdict, ReviewVerdictType::Revise);
    assert_eq!(verdict.findings.len(), 1);
    assert_eq!(verdict.findings[0].message, "Outline 写入范围重叠");
    let review = verdict
        .work_item_plan_review
        .as_ref()
        .expect("work item plan outline review extension");
    assert_eq!(review.verdict, WorkItemPlanReviewVerdict::Revise);
    assert_eq!(review.review_scope, WorkItemPlanReviewScope::Outline);
    assert_eq!(review.generation_round_id, "round_0001");
    assert_eq!(
        review.review_action,
        WorkItemPlanReviewAction::ReviseOutline
    );
    let diagnostic = verdict
        .structured_output_diagnostic
        .as_ref()
        .expect("repair success diagnostic");
    assert_eq!(diagnostic.code, "missing_end_nonce");
    assert!(diagnostic.repair_attempted);
    assert!(diagnostic.repair_succeeded);
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

#[tokio::test]
async fn work_item_plan_outline_repair_provider_failure_safely_degrades() {
    let provider = RepairTerminalProvider::new(
        RepairTerminalMode::StartError,
        Some("review-session-1".to_string()),
    );
    let (_tmp, mut engine, mut rx, review_node_id) =
        queued_work_item_plan_outline_review_engine("sess_wip_outline_repair_terminal").await;

    engine
        .drive_review_session(Arc::new(provider), empty_provider_commands())
        .await;

    assert_eq!(engine.session.stage, WorkspaceStage::HumanConfirm);
    let verdict = engine
        .latest_review_verdict
        .as_ref()
        .expect("work item plan fallback verdict");
    assert_eq!(verdict.verdict, ReviewVerdictType::NeedsHuman);
    assert!(verdict.work_item_plan_review.is_none());
    let diagnostic = verdict
        .structured_output_diagnostic
        .as_ref()
        .expect("work item plan repair diagnostic");
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
        ]
    );
}

#[tokio::test]
async fn work_item_plan_outline_review_repair_rejects_payload_change() {
    let changed_json = r#"{
        "verdict": "pass",
        "review_scope": "outline",
        "generation_round_id": "round_0001",
        "summary": "Outline 可以继续",
        "findings": []
    }"#;
    let provider = QueuedReviewProvider::new(vec![
        missing_end_nonce_output(work_item_plan_outline_revise_json()),
        valid_structured_output(changed_json),
    ]);
    let (_tmp, mut engine, mut rx, review_node_id) =
        queued_work_item_plan_outline_review_engine("sess_wip_outline_repair_changed").await;

    engine
        .drive_review_session(Arc::new(provider.clone()), empty_provider_commands())
        .await;

    assert_eq!(provider.starts.load(Ordering::SeqCst), 2);
    let verdict = engine
        .latest_review_verdict
        .as_ref()
        .expect("fallback verdict");
    assert_eq!(verdict.verdict, ReviewVerdictType::NeedsHuman);
    assert!(verdict.work_item_plan_review.is_none());
    let diagnostic = verdict
        .structured_output_diagnostic
        .as_ref()
        .expect("payload change diagnostic");
    assert_eq!(diagnostic.code, "repair_payload_changed");
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
        ]
    );
}

#[tokio::test]
async fn work_item_plan_outline_review_repair_keeps_active_scope_schema_strict() {
    let mismatched_scope_json = r#"{
        "verdict": "plan_reopen_required",
        "review_scope": "item",
        "generation_round_id": "round_0001",
        "summary": "请求重开 Outline",
        "findings": []
    }"#;
    let provider = QueuedReviewProvider::new(vec![
        missing_end_nonce_output(mismatched_scope_json),
        valid_structured_output(mismatched_scope_json),
    ]);
    let (_tmp, mut engine, mut rx, review_node_id) =
        queued_work_item_plan_outline_review_engine("sess_wip_outline_repair_scope_mismatch").await;

    engine
        .drive_review_session(Arc::new(provider.clone()), empty_provider_commands())
        .await;

    assert_eq!(provider.starts.load(Ordering::SeqCst), 2);
    let verdict = engine
        .latest_review_verdict
        .as_ref()
        .expect("fallback verdict");
    assert_eq!(verdict.verdict, ReviewVerdictType::NeedsHuman);
    assert!(verdict.work_item_plan_review.is_none());
    let diagnostic = verdict
        .structured_output_diagnostic
        .as_ref()
        .expect("scope schema diagnostic");
    assert_eq!(diagnostic.code, "invalid_review_scope");
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
        ]
    );
}
