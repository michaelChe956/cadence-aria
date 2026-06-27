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
            "severity": "strong_recommend_fix",
            "message": "sync 方法在 tokio runtime 内 block_on 会 panic",
            "evidence": "snapshot 被 tokio::spawn 调用",
            "impact": "后续实现会在运行时崩溃",
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
        ReviewFindingSeverity::StrongRecommendFix
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
            "impact": "串行生成时会缺少上游 handoff",
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
fn work_item_plan_review_invalid_target_outline_id_downgrades_to_needs_human() {
    let json = r#"{
        "verdict": "plan_reopen_required",
        "summary": "outline 不可局部修复",
        "target_outline_id": "outline_missing",
        "generation_round_id": "round_0001",
        "draft_id": "draft_0001"
    }"#;

    let verdict = parse_work_item_plan_review_json(
        json,
        "raw comments",
        &["outline_api".to_string()],
        WorkItemPlanReviewScope::Item,
    )
    .expect("work item plan review");

    assert_eq!(verdict.verdict, ReviewVerdictType::NeedsHuman);
    assert_eq!(verdict.review_gate, ReviewGate::UserTriageRequired);
    assert!(verdict.work_item_plan_review.is_none());
    assert!(verdict.summary.contains("引用无效"));
}

#[test]
fn work_item_plan_review_drops_invalid_affects_items_below_threshold() {
    let json = r#"{
        "verdict": "needs_human",
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
fn work_item_plan_review_invalid_affects_items_over_half_downgrades() {
    let json = r#"{
        "verdict": "needs_human",
        "summary": "引用大量不存在 item",
        "generation_round_id": "round_0001",
        "affects_items": [
            { "target_outline_id": "outline_api" },
            { "target_outline_id": "outline_missing_1" },
            { "target_outline_id": "outline_missing_2" }
        ]
    }"#;

    let verdict = parse_work_item_plan_review_json(
        json,
        "raw comments",
        &["outline_api".to_string(), "outline_ui".to_string()],
        WorkItemPlanReviewScope::Batch,
    )
    .expect("work item plan review");

    assert_eq!(verdict.verdict, ReviewVerdictType::NeedsHuman);
    assert_eq!(verdict.review_gate, ReviewGate::UserTriageRequired);
    assert!(verdict.work_item_plan_review.is_none());
    assert!(verdict.summary.contains("引用无效"));
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
    };

    let event = review_complete_event_from_verdict("node_review_001".to_string(), 2, &verdict);

    match event {
        EngineEvent::ReviewComplete {
            work_item_plan_review: Some(actual),
            ..
        } => assert_eq!(actual, extension),
        _ => panic!("expected review extension"),
    }
}

#[tokio::test]
async fn optional_review_findings_enter_human_confirm_for_all_workspace_types() {
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
        engine.start_review_or_skip().await;

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
  "severity": "optional",
  "message": "可补充说明",
  "evidence": "当前主路径完整",
  "impact": "不影响下一阶段执行",
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

        assert_eq!(engine.session().stage, WorkspaceStage::HumanConfirm);
        assert!(
            engine
                .timeline_nodes
                .iter()
                .any(|node| node.node_type == TimelineNodeType::HumanConfirm),
            "{workspace_type:?} should create human_confirm node"
        );
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
    let (_tmp, store) = setup();
    let (tx, mut rx) = mpsc::channel(64);
    let mut session = make_session("sess_wip_outline_optional_review");
    session.workspace_type = WorkspaceType::WorkItemPlan;
    session.artifact = Some(ArtifactPayload::WorkItemPlanOutlineCandidate {
        outline_candidate: Box::new(WorkItemPlanOutlineCandidateDto {
            outline: test_work_item_plan_outline(Vec::new()),
            design_context_gaps: vec![],
            validator_findings: vec![],
            context_blockers: vec![],
            current_generation_round_id: Some("round_0001".to_string()),
            selected_generation_mode: None,
        }),
    });
    let mut engine = WorkspaceEngine::new(store, tx, session);
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
  "severity": "optional",
  "message": "handoff 描述可以更明确",
  "evidence": "handoff_strategy 只有简短描述",
  "impact": "不影响 Draft 生成",
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

#[tokio::test]
async fn work_item_plan_outline_optional_choice_can_skip_and_continue() {
    let (_tmp, store) = setup();
    let (tx, _) = mpsc::channel(64);
    let mut session = make_session("sess_wip_outline_skip_optional");
    session.workspace_type = WorkspaceType::WorkItemPlan;
    session.stage = WorkspaceStage::ReviewDecision;
    session.artifact = Some(ArtifactPayload::WorkItemPlanOutlineCandidate {
        outline_candidate: Box::new(WorkItemPlanOutlineCandidateDto {
            outline: test_work_item_plan_outline(Vec::new()),
            design_context_gaps: vec![],
            validator_findings: vec![],
            context_blockers: vec![],
            current_generation_round_id: Some("round_0001".to_string()),
            selected_generation_mode: None,
        }),
    });
    let mut engine = WorkspaceEngine::new(store, tx, session);
    engine.latest_review_verdict = Some(ReviewVerdict {
        verdict: ReviewVerdictType::Pass,
        comments: "当前 outline 可以继续，但有可选建议".to_string(),
        summary: "仅有可选建议".to_string(),
        findings: vec![ReviewFinding {
            severity: ReviewFindingSeverity::Optional,
            message: "handoff 描述可以更明确".to_string(),
            evidence: "handoff_strategy 只有简短描述".to_string(),
            impact: "不影响 Draft 生成".to_string(),
            required_action: "可补充上下游交接说明".to_string(),
        }],
        review_gate: ReviewGate::UserConfirmAllowed,
        work_item_plan_review: Some(WorkItemPlanReviewComplete {
            verdict: WorkItemPlanReviewVerdict::Pass,
            review_scope: WorkItemPlanReviewScope::Outline,
            target_outline_id: None,
            generation_round_id: "round_0001".to_string(),
            draft_id: None,
            batch_id: None,
            review_action: WorkItemPlanReviewAction::Continue,
            gates: Vec::new(),
            affects_items: Vec::new(),
            warnings: Vec::new(),
        }),
    });
    engine
        .enter_review_decision(1, "仅有可选建议".to_string())
        .await;

    let outcome = engine
        .handle_review_decision("skip_optional_findings".to_string(), None)
        .await
        .expect("skip optional findings should continue original pass route");

    assert_eq!(outcome, ReviewDecisionOutcome::HumanConfirm);
    assert_eq!(engine.session().stage, WorkspaceStage::AuthorConfirm);
    let active_node = engine
        .timeline_nodes
        .iter()
        .find(|node| Some(&node.node_id) == engine.active_node_id.as_ref())
        .expect("active generation mode node");
    assert_eq!(active_node.node_type, TimelineNodeType::WorkItemGenerationMode);
}

#[tokio::test]
async fn work_item_plan_optional_outline_review_actions_survive_session_restore() {
    let (_tmp, checkpoint_store, lifecycle, _plan_id, mut engine) =
        make_work_item_plan_engine_with_draft_candidate("sess_wip_optional_restore");
    let persisted_session = lifecycle
        .list_workspace_sessions("project_0001", "issue_0001")
        .expect("workspace sessions")
        .into_iter()
        .next()
        .expect("persisted workspace session");
    engine.session.session_id = persisted_session.id;
    engine
        .update_artifact(ArtifactPayload::WorkItemPlanOutlineCandidate {
            outline_candidate: Box::new(WorkItemPlanOutlineCandidateDto {
                outline: test_work_item_plan_outline(Vec::new()),
                design_context_gaps: vec![],
                validator_findings: vec![],
                context_blockers: vec![],
                current_generation_round_id: Some("round_0001".to_string()),
                selected_generation_mode: None,
            }),
        })
        .await;
    let verdict = ReviewVerdict {
        verdict: ReviewVerdictType::Pass,
        comments: "当前 outline 可以继续，但有可选建议".to_string(),
        summary: "仅有可选建议".to_string(),
        findings: vec![ReviewFinding {
            severity: ReviewFindingSeverity::Minor,
            message: "handoff 描述可以更明确".to_string(),
            evidence: "handoff_strategy 只有简短描述".to_string(),
            impact: "不影响 Draft 生成".to_string(),
            required_action: "可补充上下游交接说明".to_string(),
        }],
        review_gate: ReviewGate::UserConfirmAllowed,
        work_item_plan_review: Some(WorkItemPlanReviewComplete {
            verdict: WorkItemPlanReviewVerdict::Pass,
            review_scope: WorkItemPlanReviewScope::Outline,
            target_outline_id: None,
            generation_round_id: "round_0001".to_string(),
            draft_id: None,
            batch_id: None,
            review_action: WorkItemPlanReviewAction::Continue,
            gates: Vec::new(),
            affects_items: Vec::new(),
            warnings: Vec::new(),
        }),
    };
    let review_node_id = engine
        .create_timeline_node(TimelineNodeDraft {
            node_type: TimelineNodeType::WorkItemPlanOutlineReview,
            agent: Some(ProviderName::Codex),
            stage: WorkspaceStage::CrossReview,
            round: Some(1),
            title: "WorkItemPlan Outline Review Round 1".to_string(),
            summary: None,
            status: TimelineNodeStatus::Active,
        })
        .await;
    engine.active_node_id = Some(review_node_id.clone());
    engine
        .persist_review_verdict(
            &review_node_id,
            serde_json::json!({
                "verdict": verdict.verdict,
                "comments": verdict.comments,
                "summary": verdict.summary,
                "findings": verdict.findings,
                "review_gate": verdict.review_gate,
                "work_item_plan_review": verdict.work_item_plan_review,
            }),
        )
        .await
        .expect("persist review verdict");
    engine
        .update_timeline_node(
            &review_node_id,
            TimelineNodeStatus::Completed,
            Some("仅有可选建议".to_string()),
        )
        .await;
    engine.latest_review_verdict = Some(verdict);
    engine
        .enter_review_decision(1, "仅有可选建议".to_string())
        .await;

    let restored_session_id = engine.session().session_id.clone();
    let apply_session = WorkspaceSession::from_record(
        lifecycle
            .get_workspace_session(&restored_session_id)
            .expect("workspace session should be persisted"),
    );
    let skip_session = WorkspaceSession::from_record(
        lifecycle
            .get_workspace_session(&restored_session_id)
            .expect("workspace session should be persisted"),
    );
    let (apply_tx, _) = mpsc::channel(64);
    let (skip_tx, _) = mpsc::channel(64);
    let mut apply_engine = WorkspaceEngine::new_persistent(
        checkpoint_store.clone(),
        lifecycle.clone(),
        apply_tx,
        apply_session,
    );
    let mut skip_engine =
        WorkspaceEngine::new_persistent(checkpoint_store, lifecycle, skip_tx, skip_session);

    assert_eq!(
        apply_engine.review_decision_options(),
        vec![
            "apply_optional_findings".to_string(),
            "skip_optional_findings".to_string(),
        ],
        "restored engine should keep optional WorkItemPlan review actions"
    );

    let apply_outcome = apply_engine
        .handle_review_decision("apply_optional_findings".to_string(), None)
        .await
        .expect("apply optional findings should work after restore");
    assert!(matches!(
        apply_outcome,
        ReviewDecisionOutcome::StartWorkItemPlanOutlineRevision { .. }
    ));

    let skip_outcome = skip_engine
        .handle_review_decision("skip_optional_findings".to_string(), None)
        .await
        .expect("skip optional findings should work after restore");
    assert_eq!(skip_outcome, ReviewDecisionOutcome::HumanConfirm);
    assert_eq!(skip_engine.session().stage, WorkspaceStage::AuthorConfirm);
    let active_node = skip_engine
        .timeline_nodes
        .iter()
        .find(|node| Some(&node.node_id) == skip_engine.active_node_id.as_ref())
        .expect("active generation mode node");
    assert_eq!(active_node.node_type, TimelineNodeType::WorkItemGenerationMode);
}

#[tokio::test]
async fn request_outline_revision_is_allowed_from_outline_confirm_node() {
    let (_tmp, _checkpoint_store, _lifecycle, _plan_id, mut engine) =
        make_work_item_plan_engine_with_draft_candidate("sess_request_outline_revision_from_confirm");
    engine.session.stage = WorkspaceStage::AuthorConfirm;
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
    let confirm_node_id = engine
        .create_timeline_node(TimelineNodeDraft {
            node_type: TimelineNodeType::WorkItemPlanOutlineConfirm,
            agent: None,
            stage: WorkspaceStage::AuthorConfirm,
            round: None,
            title: "WorkItemPlan Outline 确认".to_string(),
            summary: Some("等待作者确认".to_string()),
            status: TimelineNodeStatus::Paused,
        })
        .await;
    engine.active_node_id = Some(confirm_node_id.clone());

    engine
        .request_work_item_plan_outline_revision(Some("缩小 scope".to_string()))
        .await
        .expect("outline confirm node should allow outline revision");

    assert_eq!(engine.session().stage, WorkspaceStage::Running);
    let confirm_node = engine
        .timeline_nodes
        .iter()
        .find(|node| node.node_id == confirm_node_id)
        .expect("confirm node");
    assert_eq!(confirm_node.status, TimelineNodeStatus::Completed);
    assert!(
        engine
            .timeline_nodes
            .iter()
            .any(|node| node.node_type == TimelineNodeType::WorkItemPlanOutlineRun
                && node.status == TimelineNodeStatus::Active),
        "requesting outline revision from confirm should start a new outline run"
    );
}

