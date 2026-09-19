// 退役留档（T5/REQ-RET-02）：`work_item_plan_outline_optional_choice_can_apply_findings` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

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
  "severity": "suggestion",
  "message": "验证说明可以更明确",
  "evidence": "verification_plan 只有命令",
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

// 退役留档（T5/REQ-RET-02）：`work_item_plan_item_optional_choice_can_skip_and_continue` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// 退役留档（T5/REQ-RET-02）：`work_item_plan_item_optional_choice_can_apply_findings` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

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
  "severity": "suggestion",
  "message": "handoff 可以更明确",
  "evidence": "batch 内 handoff_summary 较短",
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

// 退役留档（T5/REQ-RET-02）：`work_item_plan_batch_optional_choice_can_skip_and_compile` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// 退役留档（T5/REQ-RET-02）：`work_item_plan_batch_optional_choice_can_apply_findings` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// 退役留档（T5/REQ-RET-02）：`accepting_work_item_draft_updates_current_artifact_without_new_version` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

#[tokio::test]
async fn finishing_work_item_draft_or_batch_run_clears_short_lived_repair_state() {
    let (_tmp, _checkpoint_store, _lifecycle, _plan_id, mut engine) =
        make_work_item_plan_engine_with_draft_candidate("sess_wip_clear_draft_repair_state");
    for node_type in [TimelineNodeType::WorkItemDraftRun, TimelineNodeType::WorkItemBatchRun] {
        engine.session.stage = WorkspaceStage::Running;
        let node_id = engine
            .create_timeline_node(TimelineNodeDraft {
                node_type,
                agent: Some(ProviderName::Codex),
                stage: WorkspaceStage::Running,
                round: None,
                title: "Draft".to_string(),
                summary: None,
                status: TimelineNodeStatus::Active,
            })
            .await;
        engine.active_node_id = Some(node_id);
        engine.remember_one_draft_repair("outline_a", Vec::new(), None);
        engine.mark_active_run_started("draft-run".to_string());

        engine.mark_active_run_finished("draft-run");

        assert!(engine.work_item_draft_repair_states.is_empty());
    }
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
        generation_diagnostics: None,
        candidate: {
            let logical_work_item_id = format!(
                "wi_{}",
                outline_id.strip_prefix("outline_").unwrap_or(outline_id)
            );
            let mut contract = crate::product::work_item_contract::canonical_contract_fixture(
                &logical_work_item_id,
            );
            contract.identity.title = format!("{outline_id} draft");
            contract.identity.kind = "backend".to_string();
            contract.goal.summary = format!("实现 {outline_id}");
            contract.input_contracts.clear();
            contract.handoff_contract.provided_contract_refs.clear();
            contract.write_policy.exclusive_scopes = vec![format!("src/{outline_id}.rs")];
            contract.write_policy.forbidden_scopes.clear();
            contract.verification_checks[0].check_id = format!("cmd_{outline_id}");
            contract.verification_checks[0].command =
                Some(format!("cargo test --locked --lib {outline_id}"));
            WorkItemDraftCandidate {
                target_repository_id: None,
                outline_id: outline_id.to_string(),
                logical_work_item_id,
                verification_plan:
                    crate::product::models::WorkItemDraftVerificationPlan {
                        checks: contract.verification_checks.clone(),
                    },
                canonical_contract_candidate: contract,
            }
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
