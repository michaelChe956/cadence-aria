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
