#[test]
fn parse_review_verdict_malformed_findings_require_user_triage() {
    let output = r#"建议返修，但 findings 结构不合规。

```json
{
  "verdict": "revise",
  "summary": "返修意图不结构化",
  "findings": [
{
  "severity": "must_fix",
  "message": 42
}
  ]
}
```"#;

    let verdict = WorkspaceEngine::parse_review_verdict(output);

    assert_eq!(verdict.verdict, ReviewVerdictType::NeedsHuman);
    assert_eq!(verdict.review_gate, ReviewGate::UserTriageRequired);
    assert!(verdict.findings.is_empty());
}

#[test]
fn work_item_plan_review_revise_batch_maps_to_needs_human_generic_verdict_with_extension() {
    let json = r#"{
        "verdict": "revise_batch",
        "review_scope": "batch",
        "summary": "整组需要重写",
        "generation_round_id": "round_0001",
        "batch_id": "batch_0001"
    }"#;

    let verdict = parse_work_item_plan_review_json(
        json,
        "batch review comments",
        &["outline_api".to_string()],
        WorkItemPlanReviewScope::Batch,
    )
    .expect("work item plan review");

    assert_eq!(verdict.verdict, ReviewVerdictType::NeedsHuman);
    assert_eq!(verdict.review_gate, ReviewGate::UserTriageRequired);
    let review = verdict
        .work_item_plan_review
        .expect("work item plan extension");
    assert_eq!(review.verdict, WorkItemPlanReviewVerdict::ReviseBatch);
    assert_eq!(review.review_action, WorkItemPlanReviewAction::ReviseBatch);
    assert_eq!(
        review.gates,
        vec![WorkItemPlanReviewGate::RequiresBatchRevision]
    );
}

#[test]
fn work_item_plan_item_review_pass_with_strong_finding_requires_current_item_revision() {
    let json = r#"{
        "verdict": "pass",
        "review_scope": "item",
        "target_outline_id": "outline_api",
        "generation_round_id": "round_0001",
        "draft_id": "draft_0001",
        "summary": "整体可继续，但存在运行时阻塞问题",
        "affects_items": [{ "target_outline_id": "outline_api" }],
        "findings": [{
            "severity": "must_fix",
            "message": "sync 方法在 tokio runtime 内 block_on 会 panic",
            "evidence": "snapshot 被 tokio::spawn 调用",
            "required_action": "当前 draft 需明确 spawn_blocking 或改为 async 方案"
        }]
    }"#;

    let verdict = parse_work_item_plan_review_json(
        json,
        "raw comments",
        &["outline_api".to_string()],
        WorkItemPlanReviewScope::Item,
    )
    .expect("work item plan review");

    assert_eq!(verdict.verdict, ReviewVerdictType::Revise);
    assert_eq!(verdict.review_gate, ReviewGate::RequiresRevision);
    assert_eq!(verdict.findings.len(), 1);
    assert_eq!(
        verdict.findings[0].severity,
        ReviewFindingSeverity::MustFix
    );
    let review = verdict
        .work_item_plan_review
        .expect("work item plan extension");
    assert_eq!(review.verdict, WorkItemPlanReviewVerdict::Revise);
    assert_eq!(review.review_action, WorkItemPlanReviewAction::ReviseCurrentItem);
    assert_eq!(
        review.gates,
        vec![WorkItemPlanReviewGate::RequiresCurrentItemRevision]
    );
}

#[test]
fn work_item_plan_outline_review_pass_with_strong_finding_requires_outline_revision() {
    let json = r#"{
        "verdict": "pass",
        "review_scope": "outline",
        "generation_round_id": "round_0001",
        "summary": "整体可继续，但依赖图存在阻塞问题",
        "affects_items": [{ "target_outline_id": "outline_api" }],
        "findings": [{
            "severity": "must_fix",
            "message": "依赖图遗漏必需前置 item",
            "evidence": "outline_api 消费 outline_store 但 depends_on 为空",
            "required_action": "补充 depends_on"
        }]
    }"#;

    let verdict = parse_work_item_plan_review_json(
        json,
        "raw comments",
        &["outline_api".to_string(), "outline_store".to_string()],
        WorkItemPlanReviewScope::Outline,
    )
    .expect("work item plan review");

    assert_eq!(verdict.verdict, ReviewVerdictType::Revise);
    assert_eq!(verdict.review_gate, ReviewGate::RequiresRevision);
    let review = verdict
        .work_item_plan_review
        .expect("work item plan extension");
    assert_eq!(review.verdict, WorkItemPlanReviewVerdict::Revise);
    assert_eq!(review.review_action, WorkItemPlanReviewAction::ReviseOutline);
    assert_eq!(
        review.gates,
        vec![WorkItemPlanReviewGate::RequiresPlanReopen]
    );
}

#[test]
fn parse_work_item_plan_review_value_reports_invalid_target_outline_id() {
    let json = r#"{
        "verdict": "plan_reopen_required",
        "review_scope": "item",
        "summary": "outline 不可局部修复",
        "target_outline_id": "outline_missing",
        "generation_round_id": "round_0001",
        "draft_id": "draft_0001"
    }"#;

    let value = serde_json::from_str(json).expect("review json");
    let error = parse_work_item_plan_review_value(
        &value,
        "raw comments",
        &["outline_api".to_string()],
        WorkItemPlanReviewScope::Item,
    )
    .expect_err("invalid target outline should fail");

    assert_eq!(
        error,
        ReviewStructuredOutputErrorCode::InvalidOutlineReference
    );
}

#[test]
fn work_item_plan_review_drops_invalid_affects_items_below_threshold() {
    let json = r#"{
        "verdict": "needs_human",
        "review_scope": "batch",
        "summary": "部分 item 需要人工判断",
        "generation_round_id": "round_0001",
        "affects_items": [
            { "target_outline_id": "outline_api" },
            { "target_outline_id": "outline_missing" }
        ]
    }"#;

    let verdict = parse_work_item_plan_review_json(
        json,
        "",
        &["outline_api".to_string(), "outline_ui".to_string()],
        WorkItemPlanReviewScope::Batch,
    )
    .expect("work item plan review");

    let review = verdict
        .work_item_plan_review
        .expect("work item plan extension");
    assert_eq!(review.affects_items.len(), 1);
    assert_eq!(
        review.affects_items[0].target_outline_id.as_deref(),
        Some("outline_api")
    );
    assert!(
        review
            .warnings
            .iter()
            .any(|warning| warning.contains("outline_missing"))
    );
}

#[test]
fn parse_work_item_plan_review_value_reports_too_many_invalid_affects_items() {
    let json = r#"{
        "verdict": "needs_human",
        "review_scope": "batch",
        "summary": "引用大量不存在 item",
        "generation_round_id": "round_0001",
        "affects_items": [
            { "target_outline_id": "outline_api" },
            { "target_outline_id": "outline_missing_1" },
            { "target_outline_id": "outline_missing_2" }
        ]
    }"#;

    let value = serde_json::from_str(json).expect("review json");
    let error = parse_work_item_plan_review_value(
        &value,
        "raw comments",
        &["outline_api".to_string(), "outline_ui".to_string()],
        WorkItemPlanReviewScope::Batch,
    )
    .expect_err("too many invalid references should fail");

    assert_eq!(
        error,
        ReviewStructuredOutputErrorCode::InvalidOutlineReference
    );
}

#[test]
fn parse_work_item_plan_review_value_rejects_cross_scope_verdicts() {
    let cases = [
        (WorkItemPlanReviewScope::Outline, "revise_batch"),
        (WorkItemPlanReviewScope::Outline, "plan_reopen_required"),
        (WorkItemPlanReviewScope::Item, "revise_batch"),
        (WorkItemPlanReviewScope::Batch, "revise"),
    ];

    for (scope, verdict) in cases {
        let review_scope = match &scope {
            WorkItemPlanReviewScope::Outline => "outline",
            WorkItemPlanReviewScope::Item => "item",
            WorkItemPlanReviewScope::Batch => "batch",
        };
        let value = serde_json::json!({
            "verdict": verdict,
            "review_scope": review_scope,
            "generation_round_id": "round_0001"
        });

        assert_eq!(
            parse_work_item_plan_review_value(&value, "", &[], scope.clone()),
            Err(ReviewStructuredOutputErrorCode::InvalidVerdict),
            "scope {scope:?} must reject verdict {verdict}"
        );
    }
}

#[test]
fn parse_work_item_plan_review_value_rejects_payload_scope_mismatch_before_routing() {
    let cases = [
        (WorkItemPlanReviewScope::Outline, "item"),
        (WorkItemPlanReviewScope::Outline, "batch"),
        (WorkItemPlanReviewScope::Item, "outline"),
        (WorkItemPlanReviewScope::Item, "batch"),
        (WorkItemPlanReviewScope::Batch, "outline"),
        (WorkItemPlanReviewScope::Batch, "item"),
    ];

    for (expected_scope, payload_scope) in cases {
        for verdict in ["pass", "needs_human"] {
            let value = serde_json::json!({
                "verdict": verdict,
                "review_scope": payload_scope,
                "generation_round_id": "round_0001",
                "summary": "cross scope",
                "findings": []
            });

            let error = parse_work_item_plan_review_value(
                &value,
                "",
                &["outline_a".to_string()],
                expected_scope.clone(),
            )
            .expect_err("payload review_scope mismatch must fail before verdict routing");

            assert_eq!(
                error,
                ReviewStructuredOutputErrorCode::InvalidReviewScope,
                "expected={expected_scope:?}, payload={payload_scope}, verdict={verdict}"
            );
            assert_eq!(error.as_str(), "invalid_review_scope");
        }
    }
}

#[test]
fn parse_work_item_plan_review_value_requires_payload_scope() {
    let value = serde_json::json!({
        "verdict": "pass",
        "generation_round_id": "round_0001",
        "summary": "missing scope",
        "findings": []
    });

    let error = parse_work_item_plan_review_value(
        &value,
        "",
        &["outline_a".to_string()],
        WorkItemPlanReviewScope::Outline,
    )
    .expect_err("missing review_scope must fail strict parsing");

    assert_eq!(error, ReviewStructuredOutputErrorCode::InvalidReviewScope);
}

#[test]
fn review_complete_event_preserves_work_item_plan_extension() {
    let extension = WorkItemPlanReviewComplete {
        verdict: WorkItemPlanReviewVerdict::PlanReopenRequired,
        review_scope: WorkItemPlanReviewScope::Item,
        target_outline_id: Some("outline_api".to_string()),
        generation_round_id: "round_0001".to_string(),
        draft_id: Some("draft_0001".to_string()),
        batch_id: None,
        review_action: WorkItemPlanReviewAction::ReviseOutline,
        gates: vec![WorkItemPlanReviewGate::RequiresPlanReopen],
        affects_items: Vec::new(),
        warnings: Vec::new(),
    };
    let verdict = ReviewVerdict {
        verdict: ReviewVerdictType::NeedsHuman,
        comments: "需要重开 outline".to_string(),
        summary: "需要重开 Outline".to_string(),
        findings: Vec::new(),
        review_gate: ReviewGate::UserTriageRequired,
        work_item_plan_review: Some(extension.clone()),
        structured_output_diagnostic: Some(
            crate::web::workspace_ws_types::StructuredOutputDiagnostic {
                code: "invalid_outline_reference".to_string(),
                message: "review target outline reference is invalid".to_string(),
                repair_attempted: false,
                repair_succeeded: false,
                raw_output_preview: Some("invalid outline".to_string()),
            },
        ),
    };

    let event = review_complete_event_from_verdict("node_review_001".to_string(), 2, &verdict);

    match event {
        EngineEvent::ReviewComplete {
            work_item_plan_review: Some(actual),
            structured_output_diagnostic: Some(diagnostic),
            ..
        } => {
            assert_eq!(actual, extension);
            assert_eq!(diagnostic.code, "invalid_outline_reference");
        }
        _ => panic!("expected review extension"),
    }
}

#[tokio::test]
async fn optional_review_findings_route_author_confirm_or_human_confirm_for_all_workspace_types() {
    for workspace_type in [
        WorkspaceType::Story,
        WorkspaceType::Design,
        WorkspaceType::WorkItem,
    ] {
        let (_tmp, store) = setup();
        let (tx, _) = mpsc::channel(64);
        let mut session = make_session(&format!("sess_optional_review_{workspace_type:?}"));
        session.workspace_type = workspace_type.clone();
        session.review_rounds = 2;
        session.artifact = Some(artifact_payload("# Artifact\n\n可用版本"));
        let mut engine = WorkspaceEngine::new(store, tx, session);
        engine.start_review().await;

        engine
            .drive_review_session(
                Arc::new(ReviewVerdictStreamingProvider {
                    output: r#"建议补充说明。

```json
{
  "verdict": "revise",
  "summary": "仅有可选建议",
  "findings": [
{
  "severity": "suggestion",
  "message": "可补充说明",
  "evidence": "当前主路径完整",
  "required_action": "可后续优化"
}
  ]
}
```"#,
                    provider_type: Arc::new(Mutex::new(None)),
                    prompt: Arc::new(Mutex::new(None)),
                }),
                empty_provider_commands(),
            )
            .await;

        // spec-design-dialog-revision T5：Story/Design 可选建议 review 完成统一回 AuthorConfirm（报告进对话流），
        // WorkItem 维持既有 HumanConfirm 路由（design.md「WorkItem 不受影响」）。
        match workspace_type {
            WorkspaceType::Story | WorkspaceType::Design => {
                assert_eq!(
                    engine.session().stage,
                    WorkspaceStage::AuthorConfirm,
                    "{workspace_type:?} optional findings 必须回 AuthorConfirm（报告进对话流）"
                );
                assert!(
                    engine
                        .timeline_nodes
                        .iter()
                        .any(|node| node.node_type == TimelineNodeType::AuthorConfirm),
                    "{workspace_type:?} should create author_confirm node"
                );
            }
            WorkspaceType::WorkItem => {
                assert_eq!(engine.session().stage, WorkspaceStage::HumanConfirm);
                assert!(
                    engine
                        .timeline_nodes
                        .iter()
                        .any(|node| node.node_type == TimelineNodeType::HumanConfirm),
                    "{workspace_type:?} should create human_confirm node"
                );
            }
            other => panic!("unexpected workspace type {other:?}"),
        }
        assert!(
            !engine
                .timeline_nodes
                .iter()
                .any(|node| node.node_type == TimelineNodeType::ReviewDecision),
            "{workspace_type:?} should not block optional review findings"
        );
    }
}

#[tokio::test]
async fn work_item_plan_outline_optional_findings_pause_for_user_choice() {
    let (_tmp, _checkpoint_store, _lifecycle, _plan_id, mut engine) =
        make_work_item_plan_engine_with_draft_candidate("sess_wip_outline_optional_review");
    let (tx, mut rx) = mpsc::channel(64);
    engine.event_tx = tx;
    engine.session.artifact = Some(ArtifactPayload::WorkItemPlanOutlineCandidate {
        outline_candidate: Box::new(WorkItemPlanOutlineCandidateDto {
            outline: test_work_item_plan_outline(Vec::new()),
            design_context_gaps: vec![],
            validator_findings: vec![],
            context_blockers: vec![],
            current_generation_round_id: Some("round_0001".to_string()),
            selected_generation_mode: None,
        }),
    });
    engine.begin_work_item_plan_outline_review_run().await;

    engine
        .drive_review_session(
            Arc::new(ReviewVerdictStreamingProvider {
                output: r#"当前 outline 可以继续，但建议补充 handoff 描述。

```json
{
  "verdict": "pass",
  "review_scope": "outline",
  "generation_round_id": "round_0001",
  "summary": "仅有可选建议",
  "findings": [
{
  "severity": "suggestion",
  "message": "handoff 描述可以更明确",
  "evidence": "handoff_strategy 只有简短描述",
  "required_action": "可补充上下游交接说明"
}
  ]
}
```"#,
                provider_type: Arc::new(Mutex::new(None)),
                prompt: Arc::new(Mutex::new(None)),
            }),
            empty_provider_commands(),
        )
        .await;

    assert_eq!(engine.session().stage, WorkspaceStage::ReviewDecision);
    let active_node = engine
        .timeline_nodes
        .iter()
        .find(|node| Some(&node.node_id) == engine.active_node_id.as_ref())
        .expect("active review decision node");
    assert_eq!(active_node.node_type, TimelineNodeType::ReviewDecision);
    assert_eq!(active_node.status, TimelineNodeStatus::Paused);
    assert!(
        !engine
            .timeline_nodes
            .iter()
            .any(|node| node.node_type == TimelineNodeType::WorkItemGenerationMode),
        "optional findings should wait for user choice before generation mode"
    );
    let mut decision_options = None;
    while let Ok(event) = rx.try_recv() {
        if let EngineEvent::ReviewDecisionRequired { options, .. } = event {
            decision_options = Some(options);
        }
    }
    assert_eq!(
        decision_options,
        Some(vec![
            "apply_optional_findings".to_string(),
            "skip_optional_findings".to_string(),
        ])
    );
}

// 退役留档（T5/REQ-RET-02）：`work_item_plan_outline_optional_choice_can_skip_and_continue` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// 退役留档（T5/REQ-RET-02）：`work_item_plan_optional_outline_review_actions_survive_session_restore` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// 退役留档（T5/REQ-RET-02）：`request_outline_revision_is_allowed_from_outline_confirm_node` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。
