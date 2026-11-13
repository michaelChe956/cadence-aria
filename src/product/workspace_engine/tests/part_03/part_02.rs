// 退役留档（T5/REQ-RET-02）：`work_item_plan_outline_optional_choice_can_apply_findings` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

#[tokio::test]
async fn work_item_plan_item_optional_findings_pause_for_user_choice() {
    let (_tmp, _checkpoint_store, lifecycle, plan_id, mut engine) =
        make_work_item_plan_engine_with_draft_candidate("sess_wip_item_optional_review");
    prepare_work_item_plan_outline_artifact(&mut engine).await;
    save_serial_work_item_plan_index(&engine, &plan_id, "outline_a");
    let draft_payload = work_item_draft_artifact_payload(
        &lifecycle,
        &plan_id,
        "outline_a",
        "draft_a",
        WorkItemDraftStatus::Draft,
    );
    engine
        .update_artifact(draft_payload)
        .await
        .expect("fixture artifact write");
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
        .drive_review_session_with_window(
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
        .await
        .expect("fixture artifact write");
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
        .drive_review_session_with_window(
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
    for node_type in [
        TimelineNodeType::WorkItemDraftRun,
        TimelineNodeType::WorkItemBatchRun,
    ] {
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
    // REQ-PLN-02 draft_target 铁律：经本入口确认的 outline 会成为 draft
    // 校验链的比对基准，target 必须与 draft 夹具同源（主逻辑成员）。
    // lifecycle 缺席（非持久引擎直连形态）时退回 None（无 draft 链）。
    let logical_target = engine
        .lifecycle_store
        .as_ref()
        .map(fixture_logical_member_id);
    engine
        .update_artifact(work_item_plan_outline_artifact(logical_target))
        .await
        .expect("fixture artifact write");
}

fn work_item_plan_outline_artifact(
    logical_target: Option<crate::product::logical_codebase::LogicalRepositoryId>,
) -> ArtifactPayload {
    ArtifactPayload::WorkItemPlanOutlineCandidate {
        outline_candidate: Box::new(WorkItemPlanOutlineCandidateDto {
            outline: test_work_item_plan_outline(Vec::new(), logical_target),
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
            engine
                .lifecycle_store
                .as_ref()
                .expect("engine fixture keeps a lifecycle store"),
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
    lifecycle: &LifecycleStore,
    plan_id: &str,
    outline_id: &str,
    draft_id: &str,
    status: WorkItemDraftStatus,
) -> ArtifactPayload {
    ArtifactPayload::WorkItemDraftCandidate {
        draft_candidate: Box::new(WorkItemDraftCandidatePayload {
            draft_record: test_work_item_draft_record(
                lifecycle,
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

/// 引擎夹具（`make_work_item_plan_engine_with_draft_candidate`）注册了两个
/// 逻辑成员：主成员（物理投影 `repo-levels`，承接 rep4 WI-001/002 的
/// single_repository 归属与 draft target）与 rep4 WI-003 钉死的固定 uuid
/// 成员（物理投影 `repo-levels-integration`）。REQ-PLN-02：Logical 路由下
/// Final Compile 对每个 accepted draft 校验 `candidate.target_repository_id`
/// 必须落在有效成员集内且与 outline 同源——本助手把主成员（`repo-levels`）
/// 的 logical id 交给 draft/outline 构造。⚠️ 必须按 repos.json 的
/// `repo-levels` 记录精确解析：WI-003 成员 uuid 全零开头，BTree/字典序恒排
/// 首位，`list_members(..).first()` 会选错成员。
fn fixture_logical_member_id(
    lifecycle: &LifecycleStore,
) -> crate::product::logical_codebase::LogicalRepositoryId {
    crate::product::repository_store::RepositoryStore::new(lifecycle.app_paths())
        .list("project_0001")
        .expect("fixture logical codebase projection")
        .into_iter()
        .find(|record| record.id == "repo-levels")
        .and_then(|record| record.logical_repository_id)
        .expect("fixture repo-levels member is registered")
}

/// SC（SingleCandidate）author 调用的**会话物理 target**（与 web runner
/// `run/single_candidate.rs` 同源）：会话业务仓（`workspace_repository_for_session`）。
///
/// 夹具基座走 LC 路线 ⇒ 会话 target 可能是聚合根视图；它只在 legacy
/// （成员表缺席）分支参与 IR target 回落，LC 分支下 per-WI target 一律落
/// 成员物理仓（见 [`fixture_sc_member_targets`]）。
pub(crate) fn fixture_sc_target(engine: &WorkspaceEngine) -> String {
    fixture_sc_session_repository(engine).id.clone()
}

/// SC author 调用的**成员表**（与 web runner 同源）：live 权威解析
/// （`plan_compile_member_targets` → logical→physical + 确定性回落仓）。
///
/// 候选结构门与 Final Compile 共用同一张成员表做归属反查
/// （`work-item-group-structure` 契约：「物理成员仓 ID → 逻辑成员」的反查
/// MUST 只有一个实现，两处共用）——候选 IR 的 per-WI target 必须落成员
/// 物理仓，否则合法候选被判 `target_mismatch[WI-xxx]` 拒收。成员表缺席
/// （legacy 单仓）时该门不核验归属，LC 夹具**不得**沿用 legacy 形态。
pub(crate) fn fixture_sc_member_targets(
    engine: &WorkspaceEngine,
) -> Option<crate::product::work_item_plan_compiler::PlanMemberTargets> {
    let lifecycle = engine
        .lifecycle_store
        .as_ref()
        .expect("fixture engine carries the lifecycle store");
    let session = engine.session();
    let plan = lifecycle
        .get_issue_work_item_plan(&session.project_id, &session.issue_id, &session.entity_id)
        .expect("fixture plan record");
    crate::product::workspace_engine::draft_batch::compile_support::plan_compile_member_targets(
        lifecycle,
        &plan,
        &fixture_sc_session_repository(engine),
    )
    .expect("fixture plan member targets")
}

/// 会话业务仓（durable 会话记录 → `workspace_repository_for_session`）——
/// 与 web runner 解析会话 target 的入口同源。
fn fixture_sc_session_repository(
    engine: &WorkspaceEngine,
) -> crate::product::models::RepositoryRecord {
    let lifecycle = engine
        .lifecycle_store
        .as_ref()
        .expect("fixture engine carries the lifecycle store");
    let record = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("fixture durable session record");
    crate::product::workspace_repository::workspace_repository_for_session(
        &lifecycle.app_paths(),
        lifecycle,
        &record,
    )
    .expect("fixture session business repository")
}

fn test_work_item_draft_record(
    lifecycle: &LifecycleStore,
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
                target_repository_id: Some(fixture_logical_member_id(lifecycle)),
                outline_id: outline_id.to_string(),
                logical_work_item_id,
                verification_plan: crate::product::models::WorkItemDraftVerificationPlan {
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
