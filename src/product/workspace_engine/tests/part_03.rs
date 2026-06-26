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

#[tokio::test]
async fn work_item_plan_outline_optional_choice_can_apply_findings() {
    let (_tmp, _checkpoint_store, _lifecycle, _plan_id, mut engine) =
        make_work_item_plan_engine_with_draft_candidate("sess_wip_outline_apply_optional");
    engine.session.stage = WorkspaceStage::ReviewDecision;
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
    engine.latest_review_verdict = Some(ReviewVerdict {
        verdict: ReviewVerdictType::Pass,
        comments: "当前 outline 可以继续，但有可选建议".to_string(),
        summary: "仅有可选建议".to_string(),
        findings: vec![ReviewFinding {
            severity: ReviewFindingSeverity::Suggestion,
            message: "handoff 描述可以更明确".to_string(),
            evidence: "handoff_strategy 只有简短描述".to_string(),
            impact: "不影响 Draft 生成".to_string(),
            required_action: "补充上下游交接说明".to_string(),
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
        .handle_review_decision("apply_optional_findings".to_string(), None)
        .await
        .expect("apply optional findings should restart outline author");

    let ReviewDecisionOutcome::StartWorkItemPlanOutlineRevision { feedback } = outcome else {
        panic!("expected outline revision outcome");
    };
    let feedback = feedback.expect("outline revision feedback");
    assert!(feedback.contains("Reviewer 摘要: 仅有可选建议"));
    assert!(feedback.contains("Reviewer 审核意见:\n当前 outline 可以继续，但有可选建议"));
    assert!(feedback.contains("[suggestion] handoff 描述可以更明确"));
    assert_eq!(engine.session().stage, WorkspaceStage::Running);
    assert!(
        !engine
            .timeline_nodes
            .iter()
            .any(|node| node.node_type == TimelineNodeType::Revision),
        "optional outline findings should use WorkItemPlan outline revision, not generic revision"
    );
}

#[tokio::test]
async fn work_item_plan_item_optional_findings_pause_for_user_choice() {
    let (_tmp, _checkpoint_store, _lifecycle, plan_id, mut engine) =
        make_work_item_plan_engine_with_draft_candidate("sess_wip_item_optional_review");
    prepare_work_item_plan_outline_artifact(&mut engine).await;
    save_serial_work_item_plan_index(&engine, &plan_id, "outline_a");
    let draft_payload =
        work_item_draft_artifact_payload(&plan_id, "outline_a", "draft_a", WorkItemDraftStatus::Draft);
    engine.update_artifact(draft_payload).await;
    let review_node_id = engine
        .create_timeline_node(TimelineNodeDraft {
            node_type: TimelineNodeType::WorkItemDraftReview,
            agent: Some(ProviderName::Codex),
            stage: WorkspaceStage::CrossReview,
            round: Some(1),
            title: "Work Item Draft Review".to_string(),
            summary: None,
            status: TimelineNodeStatus::Active,
        })
        .await;
    engine.active_node_id = Some(review_node_id);

    engine
        .drive_review_session(
            Arc::new(ReviewVerdictStreamingProvider {
                output: r#"当前 draft 可以继续，但建议补充验证说明。

```json
{
  "verdict": "pass",
  "review_scope": "item",
  "target_outline_id": "outline_a",
  "generation_round_id": "round_0001",
  "draft_id": "draft_a",
  "summary": "仅有 minor 建议",
  "findings": [
{
  "severity": "minor",
  "message": "验证说明可以更明确",
  "evidence": "verification_plan 只有命令",
  "impact": "不影响当前 draft 使用",
  "required_action": "补充 manual check 说明"
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
    assert!(
        !engine
            .timeline_nodes
            .iter()
            .any(|node| node.node_type == TimelineNodeType::WorkItemDraftRun),
        "optional item findings should wait for user choice before next draft"
    );
}

#[tokio::test]
async fn work_item_plan_item_optional_choice_can_skip_and_continue() {
    let (_tmp, _checkpoint_store, _lifecycle, plan_id, mut engine) =
        make_work_item_plan_engine_with_draft_candidate("sess_wip_item_skip_optional");
    prepare_work_item_plan_outline_artifact(&mut engine).await;
    save_serial_work_item_plan_index(&engine, &plan_id, "outline_a");
    engine.session.stage = WorkspaceStage::ReviewDecision;
    engine.latest_review_verdict = Some(optional_work_item_plan_pass_review(
        WorkItemPlanReviewScope::Item,
        Some("outline_a"),
        Some("draft_a"),
        None,
    ));
    engine
        .enter_review_decision(1, "仅有可选建议".to_string())
        .await;

    let outcome = engine
        .handle_review_decision("skip_optional_findings".to_string(), None)
        .await
        .expect("skip optional item findings should continue original pass route");

    assert_eq!(
        outcome,
        ReviewDecisionOutcome::StartWorkItemDraft { feedback: None }
    );
    assert_eq!(engine.session().stage, WorkspaceStage::Running);
    let active_node = engine
        .timeline_nodes
        .iter()
        .find(|node| Some(&node.node_id) == engine.active_node_id.as_ref())
        .expect("active draft run node");
    assert_eq!(active_node.node_type, TimelineNodeType::WorkItemDraftRun);
    assert_eq!(active_node.summary.as_deref(), Some("outline_b · pending"));
}

#[tokio::test]
async fn work_item_plan_item_optional_choice_can_apply_findings() {
    let (_tmp, _checkpoint_store, _lifecycle, plan_id, mut engine) =
        make_work_item_plan_engine_with_draft_candidate("sess_wip_item_apply_optional");
    prepare_work_item_plan_outline_artifact(&mut engine).await;
    save_serial_work_item_plan_index(&engine, &plan_id, "outline_a");
    engine.update_artifact(work_item_draft_artifact_payload(
        &plan_id,
        "outline_a",
        "draft_a",
        WorkItemDraftStatus::Accepted,
    ))
    .await;
    engine.session.stage = WorkspaceStage::ReviewDecision;
    engine.latest_review_verdict = Some(optional_work_item_plan_pass_review(
        WorkItemPlanReviewScope::Item,
        Some("outline_a"),
        Some("draft_a"),
        None,
    ));
    engine
        .enter_review_decision(1, "仅有可选建议".to_string())
        .await;

    let outcome = engine
        .handle_review_decision("apply_optional_findings".to_string(), None)
        .await
        .expect("apply optional item findings should rewrite current draft");

    assert_eq!(
        outcome,
        ReviewDecisionOutcome::StartWorkItemDraft { feedback: None }
    );
    assert_eq!(engine.session().stage, WorkspaceStage::Running);
    let active_node = engine
        .timeline_nodes
        .iter()
        .find(|node| Some(&node.node_id) == engine.active_node_id.as_ref())
        .expect("active draft run node");
    assert_eq!(active_node.node_type, TimelineNodeType::WorkItemDraftRun);
    assert_eq!(active_node.summary.as_deref(), Some("outline_a · pending"));
    assert!(
        !engine
            .timeline_nodes
            .iter()
            .any(|node| node.node_type == TimelineNodeType::Revision),
        "optional item findings should use WorkItemDraft rewrite, not generic revision"
    );
}

#[tokio::test]
async fn work_item_plan_batch_optional_findings_pause_for_user_choice() {
    let (_tmp, _checkpoint_store, _lifecycle, plan_id, mut engine) =
        make_work_item_plan_engine_with_draft_candidate("sess_wip_batch_optional_review");
    prepare_work_item_plan_outline_artifact(&mut engine).await;
    let draft_records = save_batch_work_item_plan_index_with_accepted_drafts(&engine, &plan_id);
    engine
        .update_artifact(ArtifactPayload::WorkItemBatchState {
            batch_state: Box::new(WorkItemBatchStatePayload {
                batch_id: "batch_0001".to_string(),
                generation_round_id: "round_0001".to_string(),
                queue: vec![
                    "outline_a".to_string(),
                    "outline_b".to_string(),
                    "outline_c".to_string(),
                ],
                draft_records,
                batch_status: WorkItemBatchStatus::ReviewPending,
                failure_summary: vec![],
            }),
        })
        .await;
    let review_node_id = engine
        .create_timeline_node(TimelineNodeDraft {
            node_type: TimelineNodeType::WorkItemBatchReview,
            agent: Some(ProviderName::Codex),
            stage: WorkspaceStage::CrossReview,
            round: Some(1),
            title: "Work Item Batch Review".to_string(),
            summary: None,
            status: TimelineNodeStatus::Active,
        })
        .await;
    engine.active_node_id = Some(review_node_id);

    engine
        .drive_review_session(
            Arc::new(ReviewVerdictStreamingProvider {
                output: r#"当前 batch 可以继续，但建议补充 handoff。

```json
{
  "verdict": "pass",
  "review_scope": "batch",
  "generation_round_id": "round_0001",
  "batch_id": "batch_0001",
  "summary": "仅有 optional 建议",
  "findings": [
{
  "severity": "optional",
  "message": "handoff 可以更明确",
  "evidence": "batch 内 handoff_summary 较短",
  "impact": "不影响 compile",
  "required_action": "补充 handoff_summary"
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
    assert!(
        !engine
            .timeline_nodes
            .iter()
            .any(|node| node.node_type == TimelineNodeType::WorkItemPlanCompile),
        "optional batch findings should wait for user choice before compile"
    );
}

#[tokio::test]
async fn work_item_plan_batch_optional_choice_can_skip_and_compile() {
    let (_tmp, _checkpoint_store, _lifecycle, plan_id, mut engine) =
        make_work_item_plan_engine_with_draft_candidate("sess_wip_batch_skip_optional");
    prepare_work_item_plan_outline_artifact(&mut engine).await;
    save_batch_work_item_plan_index_with_accepted_drafts(&engine, &plan_id);
    engine.session.stage = WorkspaceStage::ReviewDecision;
    engine.latest_review_verdict = Some(optional_work_item_plan_pass_review(
        WorkItemPlanReviewScope::Batch,
        None,
        None,
        Some("batch_0001"),
    ));
    engine
        .enter_review_decision(1, "仅有可选建议".to_string())
        .await;

    let outcome = engine
        .handle_review_decision("skip_optional_findings".to_string(), None)
        .await
        .expect("skip optional batch findings should compile");

    assert_eq!(outcome, ReviewDecisionOutcome::HumanConfirm);
    assert_eq!(engine.session().stage, WorkspaceStage::HumanConfirm);
    assert!(matches!(
        engine.session().artifact,
        Some(ArtifactPayload::WorkItemPlanCompileReport { .. })
    ));
    assert!(
        engine
            .timeline_nodes
            .iter()
            .any(|node| node.node_type == TimelineNodeType::WorkItemPlanCompile
                && node.status == TimelineNodeStatus::Completed),
        "skipping optional batch findings should run final compile"
    );
}

#[tokio::test]
async fn work_item_plan_batch_optional_choice_can_apply_findings() {
    let (_tmp, _checkpoint_store, _lifecycle, plan_id, mut engine) =
        make_work_item_plan_engine_with_draft_candidate("sess_wip_batch_apply_optional");
    prepare_work_item_plan_outline_artifact(&mut engine).await;
    save_batch_work_item_plan_index_with_accepted_drafts(&engine, &plan_id);
    engine.session.stage = WorkspaceStage::ReviewDecision;
    engine.latest_review_verdict = Some(optional_work_item_plan_pass_review(
        WorkItemPlanReviewScope::Batch,
        None,
        None,
        Some("batch_0001"),
    ));
    engine
        .enter_review_decision(1, "仅有可选建议".to_string())
        .await;

    let outcome = engine
        .handle_review_decision("apply_optional_findings".to_string(), None)
        .await
        .expect("apply optional batch findings should rewrite batch");

    assert_eq!(outcome, ReviewDecisionOutcome::StartWorkItemBatch);
    assert_eq!(engine.session().stage, WorkspaceStage::Running);
    let active_node = engine
        .timeline_nodes
        .iter()
        .find(|node| Some(&node.node_id) == engine.active_node_id.as_ref())
        .expect("active batch run node");
    assert_eq!(active_node.node_type, TimelineNodeType::WorkItemBatchRun);
    let input = engine
        .build_current_work_item_batch_draft_streaming_input()
        .expect("batch streaming input");
    assert!(
        input
            .prompt
            .contains("当前版本可以继续，但有可选建议"),
        "batch rewrite prompt should include optional review feedback"
    );
}

#[tokio::test]
async fn accepting_work_item_draft_updates_current_artifact_without_new_version() {
    let (_tmp, _checkpoint_store, _lifecycle, plan_id, mut engine) =
        make_work_item_plan_engine_with_draft_candidate("sess_wip_accept_draft_no_duplicate");
    prepare_work_item_plan_outline_artifact(&mut engine).await;
    save_serial_work_item_plan_index(&engine, &plan_id, "outline_a");
    engine.session.stage = WorkspaceStage::AuthorConfirm;
    let confirm_node_id = engine
        .create_timeline_node(TimelineNodeDraft {
            node_type: TimelineNodeType::WorkItemDraftConfirm,
            agent: None,
            stage: WorkspaceStage::AuthorConfirm,
            round: None,
            title: "Work Item Draft 确认".to_string(),
            summary: Some("等待用户确认".to_string()),
            status: TimelineNodeStatus::Paused,
        })
        .await;
    engine.active_node_id = Some(confirm_node_id);
    engine
        .update_artifact(work_item_draft_artifact_payload(
            &plan_id,
            "outline_a",
            "draft_a",
            WorkItemDraftStatus::Draft,
        ))
        .await;
    let version_count_before = engine.artifact_versions.len();
    let current_version_before = engine
        .artifact_versions
        .iter()
        .find(|version| version.is_current)
        .map(|version| version.version)
        .expect("current artifact version");

    let outcome = engine
        .handle_work_item_draft_decision(
            "outline_a".to_string(),
            WorkItemDraftDecisionDto::Accept,
            None,
        )
        .await
        .expect("accept current draft");

    assert_eq!(outcome, WorkItemDraftDecisionOutcome::StartReview);
    assert_eq!(engine.artifact_versions.len(), version_count_before);
    let current_version = engine
        .artifact_versions
        .iter()
        .find(|version| version.is_current)
        .expect("current artifact version");
    assert_eq!(current_version.version, current_version_before);
    let ArtifactPayload::WorkItemDraftCandidate { draft_candidate } = &current_version.payload
    else {
        panic!("expected work item draft artifact");
    };
    assert_eq!(
        draft_candidate.draft_record.status,
        WorkItemDraftStatus::Accepted
    );
}

async fn prepare_work_item_plan_outline_artifact(engine: &mut WorkspaceEngine) {
    engine.update_artifact(work_item_plan_outline_artifact()).await;
}

fn work_item_plan_outline_artifact() -> ArtifactPayload {
    ArtifactPayload::WorkItemPlanOutlineCandidate {
        outline_candidate: Box::new(WorkItemPlanOutlineCandidateDto {
            outline: test_work_item_plan_outline(Vec::new()),
            design_context_gaps: vec![],
            validator_findings: vec![],
            context_blockers: vec![],
            current_generation_round_id: Some("round_0001".to_string()),
            selected_generation_mode: Some(WorkItemGenerationModeDto::Serial),
        }),
    }
}

fn save_serial_work_item_plan_index(
    engine: &WorkspaceEngine,
    plan_id: &str,
    active_outline_id: &str,
) {
    let store = engine.work_item_plan_store().expect("work item plan store");
    let now = chrono::Utc::now().to_rfc3339();
    store
        .save_active_index(&WorkItemPlanDraftActiveIndex {
            project_id: engine.session.project_id.clone(),
            issue_id: engine.session.issue_id.clone(),
            plan_id: plan_id.to_string(),
            current_generation_round_id: "round_0001".to_string(),
            outline_state: "confirmed".to_string(),
            active_outline_id: Some(active_outline_id.to_string()),
            outline_to_current_draft_id: BTreeMap::from([(
                active_outline_id.to_string(),
                format!("draft_{active_outline_id}"),
            )]),
            draft_statuses: BTreeMap::new(),
            batches: vec![],
            updated_at: now,
        })
        .expect("save active index");
}

fn save_batch_work_item_plan_index_with_accepted_drafts(
    engine: &WorkspaceEngine,
    plan_id: &str,
) -> Vec<WorkItemDraftRecord> {
    let store = engine.work_item_plan_store().expect("work item plan store");
    let now = chrono::Utc::now().to_rfc3339();
    let outline_ids = ["outline_a", "outline_b", "outline_c"];
    let mut outline_to_current_draft_id = BTreeMap::new();
    let mut draft_statuses = BTreeMap::new();
    let mut draft_records = Vec::new();

    for outline_id in outline_ids {
        let draft_id = format!("draft_{outline_id}");
        let record = test_work_item_draft_record(
            plan_id,
            outline_id,
            &draft_id,
            WorkItemDraftStatus::Accepted,
            WorkItemGenerationMode::Batch,
            Some("batch_0001"),
        );
        store.put_draft_record(&record).expect("put draft record");
        outline_to_current_draft_id.insert(outline_id.to_string(), draft_id.clone());
        draft_statuses.insert(draft_id, WorkItemDraftStatus::Accepted);
        draft_records.push(record);
    }

    store
        .save_active_index(&WorkItemPlanDraftActiveIndex {
            project_id: engine.session.project_id.clone(),
            issue_id: engine.session.issue_id.clone(),
            plan_id: plan_id.to_string(),
            current_generation_round_id: "round_0001".to_string(),
            outline_state: "confirmed".to_string(),
            active_outline_id: None,
            outline_to_current_draft_id,
            draft_statuses,
            batches: vec![WorkItemBatchRecord {
                batch_id: "batch_0001".to_string(),
                generation_round_id: "round_0001".to_string(),
                mode: WorkItemGenerationMode::Batch,
                item_draft_ids: draft_records
                    .iter()
                    .map(|record| record.draft_id.clone())
                    .collect(),
                status: WorkItemBatchStatus::ReviewPending,
                validation_failed_ids: vec![],
                created_at: now.clone(),
            }],
            updated_at: now,
        })
        .expect("save active index");
    draft_records
}

fn work_item_draft_artifact_payload(
    plan_id: &str,
    outline_id: &str,
    draft_id: &str,
    status: WorkItemDraftStatus,
) -> ArtifactPayload {
    ArtifactPayload::WorkItemDraftCandidate {
        draft_candidate: Box::new(WorkItemDraftCandidatePayload {
            draft_record: test_work_item_draft_record(
                plan_id,
                outline_id,
                draft_id,
                status,
                WorkItemGenerationMode::Serial,
                None,
            ),
            validator_findings: vec![],
            can_accept: true,
        }),
    }
}

fn test_work_item_draft_record(
    plan_id: &str,
    outline_id: &str,
    draft_id: &str,
    status: WorkItemDraftStatus,
    generation_mode: WorkItemGenerationMode,
    batch_id: Option<&str>,
) -> WorkItemDraftRecord {
    let now = chrono::Utc::now().to_rfc3339();
    let accepted_at = if status == WorkItemDraftStatus::Accepted {
        Some(now.clone())
    } else {
        None
    };
    WorkItemDraftRecord {
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        plan_id: plan_id.to_string(),
        draft_id: draft_id.to_string(),
        outline_id: outline_id.to_string(),
        generation_round_id: "round_0001".to_string(),
        batch_id: batch_id.map(str::to_string),
        attempt_index: 1,
        outline_version_ref: "outline_001".to_string(),
        generation_mode,
        candidate: WorkItemDraftCandidate {
            outline_id: outline_id.to_string(),
            title: format!("{outline_id} draft"),
            kind: WorkItemKind::Backend,
            goal: format!("实现 {outline_id}"),
            implementation_context: format!("实现 src/{outline_id}.rs"),
            exclusive_write_scopes: vec![format!("src/{outline_id}.rs")],
            forbidden_write_scopes: vec![],
            depends_on_outline_ids: vec![],
            required_handoff_from_outline_ids: vec![],
            handoff_summary: format!("{outline_id} handoff"),
            verification_plan: serde_json::json!({
                "commands": [{
                    "id": format!("cmd_{outline_id}"),
                    "label": "cargo test",
                    "command": format!("cargo test --locked --lib {outline_id}"),
                    "cwd": "",
                    "purpose": "unit tests",
                    "required": true,
                    "timeout_seconds": 120,
                    "safety": "approved"
                }],
                "manual_checks": [],
                "required_gates": []
            }),
        },
        status,
        active: true,
        superseded_by_draft_id: None,
        supersede_reason: None,
        copied_from_draft_id: None,
        review_node_id: None,
        review_verdict_ref: None,
        generated_from_node_id: "timeline_node_draft".to_string(),
        accepted_at,
        superseded_at: None,
        created_at: now.clone(),
        updated_at: now,
    }
}

fn optional_work_item_plan_pass_review(
    review_scope: WorkItemPlanReviewScope,
    target_outline_id: Option<&str>,
    draft_id: Option<&str>,
    batch_id: Option<&str>,
) -> ReviewVerdict {
    ReviewVerdict {
        verdict: ReviewVerdictType::Pass,
        comments: "当前版本可以继续，但有可选建议".to_string(),
        summary: "仅有可选建议".to_string(),
        findings: vec![ReviewFinding {
            severity: ReviewFindingSeverity::Optional,
            message: "补充说明".to_string(),
            evidence: "主路径完整".to_string(),
            impact: "不影响继续".to_string(),
            required_action: "可补充说明".to_string(),
        }],
        review_gate: ReviewGate::UserConfirmAllowed,
        work_item_plan_review: Some(WorkItemPlanReviewComplete {
            verdict: WorkItemPlanReviewVerdict::Pass,
            review_scope,
            target_outline_id: target_outline_id.map(str::to_string),
            generation_round_id: "round_0001".to_string(),
            draft_id: draft_id.map(str::to_string),
            batch_id: batch_id.map(str::to_string),
            review_action: WorkItemPlanReviewAction::Continue,
            gates: vec![],
            affects_items: vec![],
            warnings: vec![],
        }),
    }
}

#[tokio::test]
async fn strong_review_findings_enter_review_decision_for_all_workspace_types() {
    for workspace_type in [
        WorkspaceType::Story,
        WorkspaceType::Design,
        WorkspaceType::WorkItem,
    ] {
        let (_tmp, store) = setup();
        let (tx, _) = mpsc::channel(64);
        let mut session = make_session(&format!("sess_strong_review_{workspace_type:?}"));
        session.workspace_type = workspace_type.clone();
        session.review_rounds = 2;
        session.artifact = Some(artifact_payload("# Artifact\n\n缺少验收标准"));
        let mut engine = WorkspaceEngine::new(store, tx, session);
        engine.start_review_or_skip().await;

        engine
            .drive_review_session(
                Arc::new(ReviewVerdictStreamingProvider {
                    output: r#"必须补充验收标准。

```json
{
  "verdict": "revise",
  "summary": "必须补充验收标准",
  "findings": [
{
  "severity": "strong_recommend_fix",
  "message": "验收标准不足",
  "evidence": "Artifact 未列出可测试验收值",
  "impact": "下一阶段无法判断实现是否完成",
  "required_action": "补充明确验收标准"
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
        assert!(
            engine
                .timeline_nodes
                .iter()
                .any(|node| node.node_type == TimelineNodeType::ReviewDecision),
            "{workspace_type:?} should require revision for strong findings"
        );
    }
}

#[tokio::test]
async fn revise_without_findings_enters_user_triage_for_all_workspace_types() {
    for workspace_type in [
        WorkspaceType::Story,
        WorkspaceType::Design,
        WorkspaceType::WorkItem,
    ] {
        let (_tmp, store) = setup();
        let (tx, _) = mpsc::channel(64);
        let mut session = make_session(&format!("sess_triage_review_{workspace_type:?}"));
        session.workspace_type = workspace_type.clone();
        session.review_rounds = 2;
        session.artifact = Some(artifact_payload("# Artifact\n\n需要人工裁决的版本"));
        let mut engine = WorkspaceEngine::new(store, tx, session);
        engine.start_review_or_skip().await;

        engine
            .drive_review_session(
                Arc::new(ReviewVerdictStreamingProvider {
                    output: r#"Reviewer 明确要求返修，但未输出结构化 finding。

```json
{
  "verdict": "revise",
  "summary": "返修意图需要人工判断"
}
```"#,
                    provider_type: Arc::new(Mutex::new(None)),
                    prompt: Arc::new(Mutex::new(None)),
                }),
                empty_provider_commands(),
            )
            .await;

        assert_eq!(engine.session().stage, WorkspaceStage::HumanConfirm);
        assert_eq!(
            engine
                .latest_review_verdict
                .as_ref()
                .expect("latest review verdict")
                .review_gate,
            ReviewGate::UserTriageRequired
        );
        assert!(
            engine
                .timeline_nodes
                .iter()
                .any(|node| node.node_type == TimelineNodeType::HumanConfirm),
            "{workspace_type:?} should create human_confirm node for user triage"
        );
        assert!(
            !engine
                .timeline_nodes
                .iter()
                .any(|node| node.node_type == TimelineNodeType::ReviewDecision),
            "{workspace_type:?} should not auto-revise unstructured review intent"
        );
    }
}

#[tokio::test]
async fn malformed_findings_enter_user_triage_for_all_workspace_types() {
    for workspace_type in [
        WorkspaceType::Story,
        WorkspaceType::Design,
        WorkspaceType::WorkItem,
    ] {
        let (_tmp, store) = setup();
        let (tx, _) = mpsc::channel(64);
        let mut session = make_session(&format!("sess_malformed_review_{workspace_type:?}"));
        session.workspace_type = workspace_type.clone();
        session.review_rounds = 2;
        session.artifact = Some(artifact_payload("# Artifact\n\n需要人工裁决的版本"));
        let mut engine = WorkspaceEngine::new(store, tx, session);
        engine.start_review_or_skip().await;

        engine
            .drive_review_session(
                Arc::new(ReviewVerdictStreamingProvider {
                    output: r#"Reviewer 明确要求返修，但 findings 结构错误。

```json
{
  "verdict": "revise",
  "summary": "findings 无法可靠解析",
  "findings": [{"severity": "must_fix", "message": 42}]
}
```"#,
                    provider_type: Arc::new(Mutex::new(None)),
                    prompt: Arc::new(Mutex::new(None)),
                }),
                empty_provider_commands(),
            )
            .await;

        assert_eq!(engine.session().stage, WorkspaceStage::HumanConfirm);
        assert_eq!(
            engine
                .latest_review_verdict
                .as_ref()
                .expect("latest review verdict")
                .review_gate,
            ReviewGate::UserTriageRequired
        );
    }
}

#[test]
fn review_prompt_limits_revise_to_strong_findings() {
    let (_tmp, store) = setup();
    let (tx, _) = mpsc::channel(8);
    let mut session = make_session("sess_review_prompt_gate");
    session.artifact = Some(artifact_payload("# Story Spec\n\n可用版本"));
    let engine = WorkspaceEngine::new(store, tx, session);

    let input = engine.build_review_input().expect("review input");

    assert!(
        input
            .prompt
            .contains("blocking|must_fix|strong_recommend_fix")
    );
    assert!(input.prompt.contains("suggestion|minor|optional"));
    assert!(
        input
            .prompt
            .contains("没有强返修 finding 时，必须允许用户确认当前版本")
    );
    assert!(
        !input
            .prompt
            .contains("High/Medium 问题、建议改动或可执行返修项，必须使用 `revise`")
    );
    assert!(
        input
            .prompt
            .contains("如果输出 `verdict=revise`，必须给出至少一个结构化 finding")
    );
}

#[test]
fn detect_author_choice_request_accepts_markdown_bold_bulleted_options() {
    let output = "感谢提供项目上下文。\n\n\
        在生成 Story Spec 之前，我有几个问题需要确认：\n\n\
        **问题 1：弹窗触发时机**\n\n\
        根据 Issue 描述，弹窗是在\"启动 aria 后\"触发。请问这里的\"启动 aria\"具体指什么时机？\n\n\
        - **A)** 用户运行 `aria` 命令启动 daemon 时（Rust 后端启动时）\n\
        - **B)** 用户打开 Web 工作台页面时（前端首次加载时）\n\
        - **C)** 两者都需要（后端启动时检测，前端展示弹窗）\n";

    let (prompt, options) = detect_author_choice_request(output, &WorkspaceType::Story)
        .expect("markdown bold bulleted options should become a choice request");

    assert!(prompt.contains("弹窗触发时机"));
    assert_eq!(options.len(), 3);
    assert_eq!(options[0].id, "A");
    assert!(options[0].label.contains("用户运行 `aria`"));
    assert_eq!(options[1].id, "B");
    assert_eq!(options[2].id, "C");
}

#[test]
fn detect_author_choice_request_uses_nearest_question_for_codex_numbered_options() {
    let output = "我会先读取本仓库规则和必须使用的技能说明，然后根据未决点用结构化提问确认范围，再产出候选 Story Spec。\
        规则侧已经明确：这次最终只输出候选 Markdown，不落盘、不改 OpenSpec。\
        结构化提问工具当前不可用，我先用文本方式提问：\n\n\
        首次启动检测到缺失 Claude Code/Codex 时，Aria 应采用哪种安装策略？\n\n\
        1. `确认后安装`：弹窗展示将执行的 npm 安装命令，用户点击安装后才执行。\n\
        2. `自动静默安装`：检测缺失后直接运行 npm 安装。\n\
        3. `只检查不安装`：只展示缺失与命令，由用户自行安装。\n\n\
        我建议选 `确认后安装`，因为它满足“自动检查与自动安装”。";

    let (prompt, options) = detect_author_choice_request(output, &WorkspaceType::Story)
        .expect("Codex numbered text question should become a choice request");

    assert_eq!(
        prompt,
        "首次启动检测到缺失 Claude Code/Codex 时，Aria 应采用哪种安装策略？"
    );
    assert!(!prompt.contains("我会先读取本仓库规则"));
    assert!(!prompt.contains("结构化提问工具当前不可用"));
    assert_eq!(options.len(), 3);
    assert_eq!(options[0].id, "1");
    assert!(options[0].label.contains("确认后安装"));
    assert_eq!(options[1].id, "2");
    assert_eq!(options[2].id, "3");
}

struct ReviewVerdictStreamingProvider {
    output: &'static str,
    provider_type: Arc<Mutex<Option<ProviderType>>>,
    prompt: Arc<Mutex<Option<String>>>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for ReviewVerdictStreamingProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        *self.provider_type.lock().unwrap() = Some(input.provider_type.clone());
        *self.prompt.lock().unwrap() = Some(input.prompt.clone());
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel(8);
        let output = self.output.to_string();
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::TextDelta {
                    content: output.clone(),
                })
                .await;
            let _ = event_tx
                .send(ProviderEvent::Completed {
                    full_output: output,
                    provider_session_id: None,
                })
                .await;
        });
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }

    async fn run_streaming(
        &self,
        _input: &AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        Err(ProviderAdapterError::execution_failed(
            None,
            String::new(),
            "run_streaming is not used by WorkspaceEngine",
            0,
        ))
    }
}

#[tokio::test]
async fn drive_review_session_pass_enters_human_confirm() {
    let (_tmp, store) = setup();
    let (tx, mut rx) = mpsc::channel(64);
    let session = make_session("sess_review_pass");
    let mut engine = WorkspaceEngine::new(store, tx, session);

    engine
        .handle_user_message(
            "start".to_string(),
            Arc::new(FakeStreamingProvider),
            empty_provider_commands(),
        )
        .await;
    engine
        .handle_author_decision(AuthorDecision::Accept)
        .await
        .unwrap();
    assert_eq!(engine.session().stage, WorkspaceStage::CrossReview);

    let provider_type = Arc::new(Mutex::new(None));
    let prompt = Arc::new(Mutex::new(None));
    engine
        .drive_review_session(
            Arc::new(ReviewVerdictStreamingProvider {
                output: "审核通过。\n\n```json\n{\"verdict\":\"pass\",\"summary\":\"可以确认\"}\n```",
                provider_type: provider_type.clone(),
                prompt: prompt.clone(),
            }),
            empty_provider_commands(),
        )
        .await;

    assert_eq!(*provider_type.lock().unwrap(), Some(ProviderType::Codex));
    assert!(
        prompt
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .contains("# Story Spec")
    );
    assert_eq!(engine.session().stage, WorkspaceStage::HumanConfirm);
    match engine.build_session_state() {
        WsOutMessage::SessionState { timeline_nodes, .. } => {
            assert!(timeline_nodes.iter().any(|node| {
                node.node_type == TimelineNodeType::ReviewerRun
                    && node.status == TimelineNodeStatus::Completed
                    && node.summary.as_deref() == Some("可以确认")
            }));
        }
        _ => panic!("expected SessionState"),
    }

    let mut saw_review_complete = false;
    while let Ok(event) = rx.try_recv() {
        if let EngineEvent::ReviewComplete {
            verdict,
            summary,
            findings,
            review_gate,
            ..
        } = event
        {
            assert_eq!(verdict, ReviewVerdictType::Pass);
            assert_eq!(summary, "可以确认");
            assert!(findings.is_empty());
            assert_eq!(review_gate, ReviewGate::UserConfirmAllowed);
            saw_review_complete = true;
        }
    }
    assert!(saw_review_complete);
}

#[tokio::test]
async fn drive_review_session_strong_revise_pauses_for_decision() {
    let (_tmp, store) = setup();
    let (tx, _) = mpsc::channel(64);
    let session = make_session("sess_review_revise");
    let mut engine = WorkspaceEngine::new(store, tx, session);

    engine
        .handle_user_message(
            "start".to_string(),
            Arc::new(FakeStreamingProvider),
            empty_provider_commands(),
        )
        .await;

    engine
        .drive_review_session(
            Arc::new(ReviewVerdictStreamingProvider {
                output: r#"需要补充失败路径。

```json
{
  "verdict": "revise",
  "summary": "补充失败路径",
  "findings": [
{
  "severity": "must_fix",
  "message": "缺少失败路径",
  "evidence": "Artifact 未覆盖失败路径",
  "impact": "下一阶段无法验收异常流程",
  "required_action": "补充失败路径说明"
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
    match engine.build_session_state() {
        WsOutMessage::SessionState {
            timeline_nodes,
            active_node_id,
            ..
        } => {
            let active = timeline_nodes
                .iter()
                .find(|node| Some(&node.node_id) == active_node_id.as_ref())
                .expect("active review decision node");
            assert_eq!(active.node_type, TimelineNodeType::ReviewDecision);
            assert_eq!(active.status, TimelineNodeStatus::Paused);
        }
        _ => panic!("expected SessionState"),
    }
}
