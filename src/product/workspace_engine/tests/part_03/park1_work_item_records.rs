// park-1（8-19 验收欠账）：compile finalize 必须持久化 per-target
// LifecycleWorkItemRecord（work-items/{id}.json，id=logical/entity 语义），
// 否则 coding 会话创建走 workspace_repository 的 WorkItem 臂
// list_work_items → find(entity_id) 落空 → product_store_not_found:work_item。
//
// 记录从 compiled work_item_revision + 归属 target 派生：
// - Legacy（单仓）：repository_id=story 仓，target_repository_id=None；
// - LC（多仓）：per-WI intent target → 成员物理仓 + 逻辑仓归属。
use crate::product::logical_codebase::EnrollmentTarget;
use crate::product::models::{WorkItemExecutionPlanStatus, WorkItemStatus};
use crate::product::work_item_plan_compiler::PlanMemberTargets;
use crate::product::workspace_engine::compile::work_item_record_attribution;

#[tokio::test]
async fn initial_plan_compile_finalize_persists_per_target_work_item_records() {
    let (_tmp, lifecycle, plan_id, mut engine) =
        make_work_item_plan_engine_with_accepted_contract_drafts();

    let outcome = engine.run_work_item_plan_compile().await.unwrap();

    let plan = lifecycle
        .get_issue_work_item_plan("project_0001", "issue_0001", &plan_id)
        .unwrap();
    let story = lifecycle
        .list_story_specs("project_0001", "issue_0001")
        .unwrap()
        .into_iter()
        .find(|story| plan.source_story_spec_ids.contains(&story.id))
        .expect("plan source story spec");
    let records = lifecycle
        .list_work_items("project_0001", "issue_0001")
        .unwrap()
        .into_iter()
        .map(|record| (record.id.clone(), record))
        .collect::<BTreeMap<String, LifecycleWorkItemRecord>>();

    let wi_a = records
        .get("wi_a")
        .expect("finalize must persist work item record for logical `wi_a`");
    let wi_b = records
        .get("wi_b")
        .expect("finalize must persist work item record for logical `wi_b`");

    // Legacy 单仓：story 仓归属，无逻辑仓 target。
    assert_eq!(wi_a.repository_id, story.repository_id);
    assert_eq!(wi_a.target_repository_id, None);
    assert_eq!(wi_b.repository_id, story.repository_id);
    assert_eq!(wi_b.target_repository_id, None);
    // 派生自 compiled canonical contract（fixture 契约：outline_a draft）。
    assert_eq!(wi_a.title, "outline_a draft");
    assert_eq!(wi_a.kind, WorkItemKind::Backend);
    assert_eq!(
        wi_a.exclusive_write_scopes,
        vec!["src/outline_a.rs".to_string()]
    );
    assert_eq!(wi_a.forbidden_write_scopes, Vec::<String>::new());
    assert_eq!(wi_a.plan_status, WorkItemPlanStatus::Confirmed);
    assert_eq!(wi_a.execution_status, WorkItemStatus::Pending);
    assert_eq!(wi_a.execution_plan_status, WorkItemExecutionPlanStatus::NotStarted);
    assert_eq!(wi_a.source_work_item_plan_id.as_deref(), Some(plan_id.as_str()));
    assert_eq!(wi_a.source_outline_id.as_deref(), Some("outline_a"));
    // source_draft_id 不在 finalize 派生（SC 纪律禁读 legacy compile store）。
    assert_eq!(wi_a.source_draft_id, None);
    assert_eq!(wi_a.sequence_hint, Some(1));
    assert_eq!(wi_a.depends_on, Vec::<String>::new());
    assert_eq!(wi_b.source_outline_id.as_deref(), Some("outline_b"));
    assert_eq!(wi_b.sequence_hint, Some(2));
    assert_eq!(wi_b.depends_on, vec!["wi_a".to_string()]);
    let compiled_a = outcome
        .work_items
        .iter()
        .find(|item| item.work_item_revision.logical_work_item_id == "wi_a")
        .expect("compiled wi_a");
    assert_eq!(
        wi_a.verification_plan_ref.as_deref(),
        Some(compiled_a.verification_plan_revision.id.as_str())
    );

    // 消费链：coding 会话创建的 WorkItem 臂 find(entity_id) 必须命中并解析出仓。
    let child_session = lifecycle
        .list_workspace_sessions("project_0001", "issue_0001")
        .unwrap()
        .into_iter()
        .find(|session| {
            session.workspace_type == WorkspaceType::WorkItem && session.entity_id == "wi_a"
        })
        .expect("child work item session");
    let repository = crate::product::workspace_repository::workspace_repository_for_session(
        &lifecycle.app_paths(),
        &lifecycle,
        &child_session,
    )
    .unwrap();
    assert_eq!(repository.id, story.repository_id);
}

#[tokio::test]
async fn finalize_replay_after_work_item_records_persisted_keeps_records_unique() {
    let (_tmp, lifecycle, plan_id, mut engine) =
        make_work_item_plan_engine_with_accepted_contract_drafts();
    let compile_id = "compile_park1_records_replay".to_string();
    let (mut compile_tx, accepted_drafts) = prepare_initial_compile_transaction(
        &engine,
        &lifecycle,
        &plan_id,
        &compile_id,
        "2026-10-13T00:00:00Z",
    );
    let plan_store = engine.work_item_plan_store().unwrap();
    plan_store.put_compile_transaction(&compile_tx).unwrap();
    engine.compile_initial_plan_revision(&accepted_drafts).unwrap();
    compile_tx.status = WorkItemPlanCompileStatus::RecoveryRequired;
    compile_tx.failure_reason = Some("simulated crash after records persisted".to_string());
    plan_store.put_compile_transaction(&compile_tx).unwrap();
    engine
        .enter_work_item_plan_compile_recovery(Some(
            "simulated crash after records persisted".to_string(),
        ))
        .await;
    let _failpoint = engine.register_work_item_plan_compile_finalizer_failpoint(
        &compile_tx.compile_id,
        WorkItemPlanCompileFinalizerCheckpoint::WorkItemRecordsPersisted,
    );

    // 首次推进：work item records 落盘后中断。
    let error = engine
        .handle_work_item_plan_compile_recovery_action(
            WorkItemPlanCompileRecoveryActionDto::Continue,
            None,
        )
        .await
        .unwrap_err();
    assert!(error.contains("WorkItemRecordsPersisted"));
    let interrupted = lifecycle
        .list_work_items("project_0001", "issue_0001")
        .unwrap();
    assert_eq!(
        interrupted
            .iter()
            .filter(|record| record.id == "wi_a" || record.id == "wi_b")
            .count(),
        2,
        "中断前记录必须已落盘（4 条含 fixture 2 条）"
    );

    // 恢复重放：finalize 从头重跑，records upsert 不得重复建。
    let session_record = lifecycle
        .get_workspace_session(&engine.session.session_id)
        .unwrap();
    let (event_tx, _event_rx) = tokio::sync::mpsc::channel(8);
    let mut recovered = WorkspaceEngine::new_persistent(
        engine.checkpoint_store.clone(),
        lifecycle.clone(),
        event_tx,
        WorkspaceSession::from_record(session_record),
    );
    let recovery = recovered
        .handle_work_item_plan_compile_recovery_action(
            WorkItemPlanCompileRecoveryActionDto::Continue,
            None,
        )
        .await
        .unwrap();
    assert_eq!(recovery, WorkItemPlanCompileRecoveryOutcome::HumanConfirm);
    let replayed = lifecycle
        .list_work_items("project_0001", "issue_0001")
        .unwrap();
    assert_eq!(replayed.len(), interrupted.len(), "重放不得重复建记录");
    let wi_a = replayed
        .iter()
        .find(|record| record.id == "wi_a")
        .expect("wi_a record survives replay");
    assert_eq!(wi_a.plan_status, WorkItemPlanStatus::Confirmed);
}

#[tokio::test]
async fn sc_shaped_outline_map_persists_logical_source_outline_id() {
    let (_tmp, lifecycle, plan_id, mut engine) =
        make_work_item_plan_engine_with_accepted_contract_drafts();
    let compile_id = "compile_park1_sc_outline_domain".to_string();
    let (mut compile_tx, accepted_drafts) = prepare_initial_compile_transaction(
        &engine,
        &lifecycle,
        &plan_id,
        &compile_id,
        "2026-10-13T00:00:00Z",
    );
    engine
        .work_item_plan_store()
        .unwrap()
        .put_compile_transaction(&compile_tx)
        .unwrap();
    let outcome = engine
        .compile_initial_plan_revision(&accepted_drafts)
        .unwrap();
    // SC 路径的 tx 形态(single_candidate.rs:537 / execute):键=outline id
    //(== logical id),值=work_item.id(物理)。source_outline_id 必须落键域。
    compile_tx.outline_to_work_item_id = BTreeMap::from([
        ("wi_a".to_string(), "work_item_phys_001".to_string()),
        ("wi_b".to_string(), "work_item_phys_002".to_string()),
    ]);
    engine
        .persist_compiled_work_item_records_for_test(&lifecycle, &mut compile_tx, &outcome)
        .unwrap();

    let records = lifecycle
        .list_work_items("project_0001", "issue_0001")
        .unwrap();
    let wi_a = records
        .iter()
        .find(|record| record.id == "wi_a")
        .expect("wi_a record");
    let wi_b = records
        .iter()
        .find(|record| record.id == "wi_b")
        .expect("wi_b record");
    assert_eq!(
        wi_a.source_outline_id.as_deref(),
        Some("wi_a"),
        "SC 形态 map 命中键时必须取键(logical/outline id),不得回退取值(物理 work_item id)"
    );
    assert_eq!(wi_b.source_outline_id.as_deref(), Some("wi_b"));
}

fn compile_work_item_record(id: &str, repository_id: &str, title: &str) -> LifecycleWorkItemRecord {
    LifecycleWorkItemRecord {
        id: id.to_string(),
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        repository_id: repository_id.to_string(),
        target_repository_id: None,
        story_spec_ids: vec!["story_spec_0001".to_string()],
        design_spec_ids: vec![],
        title: title.to_string(),
        plan_status: WorkItemPlanStatus::Confirmed,
        execution_status: WorkItemStatus::Pending,
        worktree_path: None,
        work_item_set_id: Some("issue_work_item_plan_0001".to_string()),
        source_work_item_plan_id: Some("issue_work_item_plan_0001".to_string()),
        source_outline_id: Some("outline_a".to_string()),
        source_draft_id: Some("draft_outline_a".to_string()),
        planned_implementation_context: None,
        kind: WorkItemKind::Other,
        sequence_hint: Some(1),
        depends_on: vec![],
        exclusive_write_scopes: vec![],
        forbidden_write_scopes: vec![],
        context_budget: WorkItemContextBudget::default(),
        verification_plan_ref: Some("verification_plan_revision_1".to_string()),
        require_execution_plan_confirm: false,
        execution_plan_status: WorkItemExecutionPlanStatus::NotStarted,
        completion_commit: None,
        completion_diff_summary_ref: None,
        created_at: "2026-10-13T00:00:00Z".to_string(),
        updated_at: "2026-10-13T00:00:00Z".to_string(),
    }
}

#[test]
fn upsert_compiled_work_item_record_is_idempotent_and_preserves_execution_state() {
    let tmp = tempfile::tempdir().unwrap();
    let app_paths = crate::product::app_paths::ProductAppPaths::new(tmp.path().join(".aria"));
    crate::product::project_store::ProjectStore::new(app_paths.clone())
        .create(crate::product::project_store::CreateProjectInput {
            name: "park1 upsert fixture".to_string(),
            description: None,
        })
        .unwrap();
    let lifecycle = LifecycleStore::new(app_paths);

    lifecycle
        .upsert_compiled_work_item_record(compile_work_item_record(
            "WI-001",
            "repository_phys_a",
            "v1 title",
        ))
        .unwrap();
    // 模拟执行期推进。
    lifecycle
        .update_work_item_execution_status("project_0001", "issue_0001", "WI-001", WorkItemStatus::Coding)
        .unwrap();
    lifecycle
        .update_work_item_completion_commit(
            "project_0001",
            "issue_0001",
            "WI-001",
            Some("sha256:abc".to_string()),
        )
        .unwrap();

    // 重放 finalize 派生记录（新归属 + 新标题）。
    let replayed = lifecycle
        .upsert_compiled_work_item_record(compile_work_item_record(
            "WI-001",
            "repository_phys_b",
            "v2 title",
        ))
        .unwrap();

    let records = lifecycle
        .list_work_items("project_0001", "issue_0001")
        .unwrap();
    assert_eq!(records.len(), 1, "重复 upsert 不得产生重复记录");
    assert_eq!(records[0].id, "WI-001");
    assert_eq!(records[0].repository_id, "repository_phys_b");
    assert_eq!(records[0].title, "v2 title");
    assert_eq!(
        records[0].execution_status,
        WorkItemStatus::Coding,
        "compile 派生字段重放必须保留执行状态"
    );
    assert_eq!(
        records[0].completion_commit.as_deref(),
        Some("sha256:abc"),
        "compile 派生字段重放必须保留完成提交"
    );
    assert_eq!(
        records[0].created_at,
        replayed.created_at,
        "created_at 必须保留首次创建值"
    );
}

fn member_targets_fixture() -> PlanMemberTargets {
    let logical_a = LogicalRepositoryId(stable_uuid(1));
    let logical_b = LogicalRepositoryId(stable_uuid(2));
    let mut physical_by_logical = BTreeMap::new();
    physical_by_logical.insert(logical_a, "repository_phys_a".to_string());
    physical_by_logical.insert(logical_b, "repository_phys_b".to_string());
    PlanMemberTargets {
        physical_by_logical,
        fallback_physical_id: Some("repository_phys_a".to_string()),
    }
}

const fn stable_uuid(seed: u16) -> Uuid {
    let mut bytes = [0u8; 16];
    bytes[14] = (seed >> 8) as u8;
    bytes[15] = seed as u8;
    bytes[6] = 0x70;
    bytes[8] = 0x80;
    Uuid::from_bytes(bytes)
}

#[test]
fn attribution_maps_logical_codebase_intent_to_member_physical() {
    let targets = member_targets_fixture();
    let logical_a = *targets.physical_by_logical.keys().next().unwrap();
    let intent = EnrollmentTarget::LogicalCodebase {
        logical_codebase_id: "lc-0001".to_string(),
        logical_repository_id: logical_a,
    };
    let (physical, target) =
        work_item_record_attribution(Some(&targets), "legacy_repo", Some(&intent), "WI-001")
            .unwrap();
    assert_eq!(physical, "repository_phys_a");
    assert_eq!(target, Some(logical_a));
}

#[test]
fn attribution_reverse_maps_single_repository_intent_to_logical_member() {
    let targets = member_targets_fixture();
    let intent = EnrollmentTarget::SingleRepository {
        repository_id: "repository_phys_b".to_string(),
    };
    let (physical, target) =
        work_item_record_attribution(Some(&targets), "legacy_repo", Some(&intent), "WI-002")
            .unwrap();
    assert_eq!(physical, "repository_phys_b");
    assert_eq!(target, Some(LogicalRepositoryId(stable_uuid(2))));
}

#[test]
fn attribution_falls_back_to_deterministic_member_and_maps_logical() {
    let targets = member_targets_fixture();
    let (physical, target) =
        work_item_record_attribution(Some(&targets), "legacy_repo", None, "WI-003").unwrap();
    assert_eq!(physical, "repository_phys_a");
    assert_eq!(target, Some(LogicalRepositoryId(stable_uuid(1))));
}

#[test]
fn attribution_fails_closed_on_missing_or_ineffective_targets() {
    let targets = member_targets_fixture();
    let dangling = EnrollmentTarget::LogicalCodebase {
        logical_codebase_id: "lc-0001".to_string(),
        logical_repository_id: LogicalRepositoryId(stable_uuid(9)),
    };
    let error = work_item_record_attribution(
        Some(&targets),
        "legacy_repo",
        Some(&dangling),
        "WI-004",
    )
    .unwrap_err();
    assert!(
        error.contains("work_item_target_missing"),
        "无效逻辑成员必须 fail-closed：{error}"
    );

    let unknown_physical = EnrollmentTarget::SingleRepository {
        repository_id: "repository_phys_missing".to_string(),
    };
    let error = work_item_record_attribution(
        Some(&targets),
        "legacy_repo",
        Some(&unknown_physical),
        "WI-005",
    )
    .unwrap_err();
    assert!(error.contains("work_item_target_missing"));

    let mut no_fallback = member_targets_fixture();
    no_fallback.fallback_physical_id = None;
    let error =
        work_item_record_attribution(Some(&no_fallback), "legacy_repo", None, "WI-006").unwrap_err();
    assert!(error.contains("work_item_target_missing"));
}

/// 批 2 R1（REQ-WIGA-01）：集合 intent 不能作为单 WI 归属——成员表在场与
/// 缺席（legacy 回落路径）均拒绝，语义错 fail-closed 不落 story 仓兜底。
#[test]
fn attribution_rejects_members_set_intent_with_and_without_member_table() {
    let set_intent = EnrollmentTarget::LogicalCodebaseMembers {
        logical_codebase_id: "lc-0001".to_string(),
        logical_repository_ids: vec![
            LogicalRepositoryId(stable_uuid(1)),
            LogicalRepositoryId(stable_uuid(2)),
        ],
    };
    let targets = member_targets_fixture();
    let error = work_item_record_attribution(
        Some(&targets),
        "legacy_repo",
        Some(&set_intent),
        "WI-set-1",
    )
    .unwrap_err();
    assert!(
        error.contains("work_item_target_missing") && error.contains("集合"),
        "成员表在场时集合 intent 必须拒绝：{error}"
    );

    let error =
        work_item_record_attribution(None, "legacy_repo", Some(&set_intent), "WI-set-2")
            .unwrap_err();
    assert!(
        error.contains("work_item_target_missing") && error.contains("集合"),
        "无成员表（legacy 回落路径）集合 intent 同样拒绝：{error}"
    );
}

#[test]
fn attribution_legacy_keeps_story_repository_without_logical_target() {
    let intent = EnrollmentTarget::SingleRepository {
        repository_id: "repository_phys_b".to_string(),
    };
    let (physical, target) =
        work_item_record_attribution(None, "story_repository", Some(&intent), "WI-007").unwrap();
    assert_eq!(physical, "story_repository");
    assert_eq!(target, None);
}
