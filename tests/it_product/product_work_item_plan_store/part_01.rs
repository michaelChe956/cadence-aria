use std::collections::BTreeMap;
use std::fs;

use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::json_store::ProductStoreError;
use cadence_aria::product::lifecycle_store::LifecycleStore;
use cadence_aria::product::models::{
    DesignContextCapabilities, IssueWorkItemPlan, IssueWorkItemPlanOptions,
    IssueWorkItemPlanStatus, OutlineContextBlockerResolution, OutlineContextIndex,
    WorkItemBatchRecord, WorkItemBatchStatus, WorkItemDraftCandidate, WorkItemDraftRecord,
    WorkItemDraftStatus, WorkItemDraftSupersedeReason, WorkItemGenerationMode, WorkItemKind,
    WorkItemOutline, WorkItemOutlineDependencyEdge, WorkItemOutlineSessionFit, WorkItemPlanCommitState,
    WorkItemPlanCompileStatus, WorkItemPlanCompileTransaction, WorkItemPlanDraftActiveIndex,
    WorkItemPlanOutline, WorkspaceType,
};
use cadence_aria::product::work_item_plan_store::{
    WorkItemPlanStore, compact_outline_context_index, copy_draft_for_current_round,
    mark_downstream_superseded, mark_draft_active, mark_draft_record_superseded, next_batch_id,
    next_draft_id, next_generation_round_id, outline_rewrite_invalidation_plan,
};
use tempfile::tempdir;

#[test]
fn work_item_plan_models_roundtrip() {
    let outline = WorkItemPlanOutline {
        id: "outline_artifact_1".to_string(),
        project_id: "project_1".to_string(),
        issue_id: "issue_1".to_string(),
        source_story_spec_ids: vec!["story_1".to_string()],
        source_design_spec_ids: vec!["design_1".to_string()],
        strategy_summary: "按后端、前端和验收拆分".to_string(),
        work_item_outlines: vec![WorkItemOutline {
            outline_id: "outline_001".to_string(),
            title: "实现后端 store".to_string(),
            kind: WorkItemKind::Backend,
            goal: "持久化 draft record".to_string(),
            scope: vec!["src/product".to_string()],
            non_goals: vec!["不写真实 work item".to_string()],
            estimated_context_tokens: Some(12_000),
            session_fit: Some(WorkItemOutlineSessionFit::FitsSingleAgentSession),
            source_story_spec_ids: vec!["story_1".to_string()],
            source_design_spec_ids: vec!["design_1".to_string()],
            exclusive_write_scopes: vec!["src/product".to_string()],
            forbidden_write_scopes: vec!["web".to_string()],
            depends_on: vec![],
            verification_intent: vec!["cargo test --locked --test it_product".to_string()],
            handoff_notes: "生成给后续 item 的摘要".to_string(),
        }],
        dependency_graph: vec![WorkItemOutlineDependencyEdge {
            from_outline_id: "outline_001".to_string(),
            to_outline_id: "outline_002".to_string(),
        }],
        risks: vec!["路径逃逸".to_string()],
        handoff_strategy: "逐项传递 handoff_summary".to_string(),
        status: "confirmed".to_string(),
    };

    let outline_json = serde_json::to_value(&outline).unwrap();
    assert_eq!(outline_json["work_item_outlines"][0]["kind"], "backend");
    let outline_back: WorkItemPlanOutline = serde_json::from_value(outline_json).unwrap();
    assert_eq!(outline_back.work_item_outlines[0].outline_id, "outline_001");

    let draft_record = WorkItemDraftRecord {
        project_id: "project_1".to_string(),
        issue_id: "issue_1".to_string(),
        plan_id: "plan_1".to_string(),
        draft_id: "draft_001".to_string(),
        outline_id: "outline_001".to_string(),
        generation_round_id: "round_001".to_string(),
        batch_id: None,
        attempt_index: 1,
        outline_version_ref: "artifact://outline/1".to_string(),
        generation_mode: WorkItemGenerationMode::Serial,
        candidate: WorkItemDraftCandidate {
            outline_id: "outline_001".to_string(),
            title: "实现后端 store".to_string(),
            kind: WorkItemKind::Backend,
            goal: "持久化 draft record".to_string(),
            implementation_context: "复用 json_store 原子写".to_string(),
            exclusive_write_scopes: vec!["src/product".to_string()],
            forbidden_write_scopes: vec!["web".to_string()],
            depends_on_outline_ids: vec![],
            required_handoff_from_outline_ids: vec![],
            handoff_summary: "store 可供编译阶段读取".to_string(),
            verification_plan: serde_json::json!({"commands": ["cargo test --locked"]}),
        },
        status: WorkItemDraftStatus::Draft,
        active: true,
        superseded_by_draft_id: None,
        supersede_reason: None,
        copied_from_draft_id: None,
        review_node_id: None,
        review_verdict_ref: None,
        generated_from_node_id: "node_1".to_string(),
        accepted_at: None,
        superseded_at: None,
        created_at: "2026-06-22T10:00:00Z".to_string(),
        updated_at: "2026-06-22T10:00:00Z".to_string(),
    };

    let draft_json = serde_json::to_value(&draft_record).unwrap();
    assert_eq!(draft_json["generation_mode"], "serial");
    assert!(draft_json.get("batch_id").is_none());
    let draft_back: WorkItemDraftRecord = serde_json::from_value(draft_json).unwrap();
    assert_eq!(draft_back.status, WorkItemDraftStatus::Draft);

    let active_index = WorkItemPlanDraftActiveIndex {
        project_id: "project_1".to_string(),
        issue_id: "issue_1".to_string(),
        plan_id: "plan_1".to_string(),
        current_generation_round_id: "round_001".to_string(),
        outline_state: "confirmed".to_string(),
        active_outline_id: Some("outline_001".to_string()),
        outline_to_current_draft_id: BTreeMap::from([(
            "outline_001".to_string(),
            "draft_001".to_string(),
        )]),
        draft_statuses: BTreeMap::from([("draft_001".to_string(), WorkItemDraftStatus::Draft)]),
        batches: vec![WorkItemBatchRecord {
            batch_id: "batch_20260622_001".to_string(),
            generation_round_id: "round_001".to_string(),
            mode: WorkItemGenerationMode::Batch,
            item_draft_ids: vec!["draft_001".to_string()],
            status: WorkItemBatchStatus::Completed,
            validation_failed_ids: vec![],
            created_at: "2026-06-22T10:00:00Z".to_string(),
        }],
        updated_at: "2026-06-22T10:00:00Z".to_string(),
    };

    let active_json = serde_json::to_value(&active_index).unwrap();
    assert_eq!(active_json["batches"][0]["status"], "completed");
    assert_eq!(active_json["active_outline_id"], "outline_001");
    let active_back: WorkItemPlanDraftActiveIndex = serde_json::from_value(active_json).unwrap();
    assert_eq!(
        active_back.active_outline_id.as_deref(),
        Some("outline_001")
    );
    assert_eq!(active_back.batches[0].mode, WorkItemGenerationMode::Batch);

    let compile_tx = WorkItemPlanCompileTransaction {
        compile_id: "compile_001".to_string(),
        project_id: "project_1".to_string(),
        issue_id: "issue_1".to_string(),
        plan_id: "plan_1".to_string(),
        generation_round_id: "round_001".to_string(),
        outline_version_ref: "artifact://outline/1".to_string(),
        active_draft_ids: vec!["draft_001".to_string()],
        status: WorkItemPlanCompileStatus::Preparing,
        plan_commit_state: WorkItemPlanCommitState::NotStarted,
        step_cursor: "start".to_string(),
        outline_to_work_item_id: BTreeMap::new(),
        outline_to_verification_plan_id: BTreeMap::new(),
        created_work_item_ids: vec![],
        created_verification_plan_ids: vec![],
        child_session_ids: vec![],
        validator_findings: vec![],
        abort_requested_at: None,
        failure_reason: None,
        previous_plan_snapshot: sample_plan(),
        created_at: "2026-06-22T10:00:00Z".to_string(),
        updated_at: "2026-06-22T10:00:00Z".to_string(),
        committed_at: None,
    };

    let compile_json = serde_json::to_value(&compile_tx).unwrap();
    assert_eq!(compile_json["status"], "preparing");
    assert!(compile_json.get("previous_plan_snapshot").is_some());
    let compile_back: WorkItemPlanCompileTransaction =
        serde_json::from_value(compile_json).unwrap();
    assert_eq!(
        compile_back.previous_plan_snapshot.status,
        IssueWorkItemPlanStatus::Draft
    );

    let context_index = OutlineContextIndex {
        project_id: "project_1".to_string(),
        issue_id: "issue_1".to_string(),
        plan_id: "plan_1".to_string(),
        generation_round_id: "round_001".to_string(),
        blocker_resolutions: vec![OutlineContextBlockerResolution {
            blocker_node_id: "node_blocker_1".to_string(),
            resolution_node_id: "node_resolution_1".to_string(),
            resolution_artifact_ref:
                "context_blocker_resolution://node_blocker_1/node_resolution_1".to_string(),
            estimated_tokens: 120,
            created_at: "2026-06-22T10:00:00Z".to_string(),
            summary: None,
            merged_count: None,
        }],
        design_context_gaps: vec!["missing_test_strategy".to_string()],
        design_context_capabilities: DesignContextCapabilities {
            has_architecture: true,
            has_module_breakdown: true,
            has_tech_stack: true,
            has_test_strategy: false,
            has_key_paths: false,
        },
        updated_at: "2026-06-22T10:00:00Z".to_string(),
    };

    let context_json = serde_json::to_value(&context_index).unwrap();
    assert!(
        context_json["blocker_resolutions"][0]
            .get("summary")
            .is_none()
    );
    let context_back: OutlineContextIndex = serde_json::from_value(context_json).unwrap();
    assert_eq!(context_back.blocker_resolutions[0].estimated_tokens, 120);

    let superseded = WorkItemDraftStatus::Superseded;
    let reason = WorkItemDraftSupersedeReason::AncestorRewritten;
    assert_eq!(serde_json::to_value(superseded).unwrap(), "superseded");
    assert_eq!(serde_json::to_value(reason).unwrap(), "ancestor_rewritten");
}

#[test]
fn draft_store_writes_immutable_records_under_round_dir() {
    let root = tempdir().expect("tempdir");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let store = WorkItemPlanStore::new(paths);
    let record = sample_draft_record("draft_001", "round_001");

    store.put_draft_record(&record).expect("put draft");

    let expected_path = root
        .path()
        .join(".aria/projects/project_1/issues/issue_1/work_item_plan_drafts/plan_1/round_001/draft_001.json");
    assert!(expected_path.exists());

    let loaded = store
        .get_draft_record("project_1", "issue_1", "plan_1", "round_001", "draft_001")
        .expect("get draft");
    assert_eq!(loaded, record);

    let listed = store
        .list_draft_records("project_1", "issue_1", "plan_1")
        .expect("list drafts");
    assert_eq!(listed, vec![record]);

    let error = store
        .get_draft_record("../bad", "issue_1", "plan_1", "round_001", "draft_001")
        .expect_err("path escape should fail");
    assert!(matches!(error, ProductStoreError::PathEscape(_)));
}

#[test]
fn active_index_tracks_current_round_and_batches() {
    let root = tempdir().expect("tempdir");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let store = WorkItemPlanStore::new(paths);
    let index = sample_active_index();

    store.save_active_index(&index).expect("save index");

    let expected_path = root.path().join(
        ".aria/projects/project_1/issues/issue_1/work_item_plan_drafts/plan_1/active_index.json",
    );
    assert!(expected_path.exists());

    let loaded = store
        .load_active_index("project_1", "issue_1", "plan_1")
        .expect("load active index")
        .expect("active index should exist");
    assert_eq!(loaded.current_generation_round_id, "round_001");
    assert_eq!(loaded.active_outline_id.as_deref(), Some("outline_001"));
    assert_eq!(loaded.batches[0].batch_id, "batch_20260622_001");
}

#[test]
fn compile_transaction_roundtrips_with_previous_plan_snapshot() {
    let root = tempdir().expect("tempdir");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let store = WorkItemPlanStore::new(paths);
    let tx = sample_compile_transaction();

    store.put_compile_transaction(&tx).expect("put compile tx");

    let expected_path = root.path().join(
        ".aria/projects/project_1/issues/issue_1/work_item_plan_compiles/plan_1/compile_001.json",
    );
    assert!(expected_path.exists());

    let loaded = store
        .get_compile_transaction("project_1", "issue_1", "plan_1", "compile_001")
        .expect("get compile tx");
    assert_eq!(loaded.previous_plan_snapshot, sample_plan());

    let error = store
        .get_compile_transaction("project_1", "issue_1", "../bad", "compile_001")
        .expect_err("path escape should fail");
    assert!(matches!(error, ProductStoreError::PathEscape(_)));
}

#[test]
fn outline_context_index_uses_atomic_write() {
    let root = tempdir().expect("tempdir");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let store = WorkItemPlanStore::new(paths);
    let index = sample_context_index();

    store
        .save_outline_context_index(&index)
        .expect("save context index");

    let dir = root
        .path()
        .join(".aria/projects/project_1/issues/issue_1/work_item_plan_outlines/plan_1");
    let expected_path = dir.join("outline_context_index.json");
    assert!(expected_path.exists());

    let loaded = store
        .load_outline_context_index("project_1", "issue_1", "plan_1")
        .expect("load context index")
        .expect("context index should exist");
    assert_eq!(
        loaded.blocker_resolutions[0].blocker_node_id,
        "node_blocker_1"
    );

    let leftovers: Vec<_> = fs::read_dir(dir)
        .expect("read context dir")
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
        .collect();
    assert!(leftovers.is_empty());
}

#[test]
fn accepting_new_draft_supersedes_previous_active_for_outline() {
    let mut index = sample_active_index();
    index
        .draft_statuses
        .insert("draft_002".to_string(), WorkItemDraftStatus::Draft);

    mark_draft_active(
        &mut index,
        "outline_001",
        "draft_002",
        WorkItemDraftStatus::Accepted,
    );

    assert_eq!(
        index.outline_to_current_draft_id.get("outline_001"),
        Some(&"draft_002".to_string())
    );
    assert_eq!(
        index.draft_statuses.get("draft_001"),
        Some(&WorkItemDraftStatus::Superseded)
    );
    assert_eq!(
        index.draft_statuses.get("draft_002"),
        Some(&WorkItemDraftStatus::Accepted)
    );

    mark_downstream_superseded(
        &mut index,
        &["outline_001".to_string()],
        WorkItemDraftSupersedeReason::AncestorRewritten,
    );

    assert!(
        !index
            .outline_to_current_draft_id
            .contains_key("outline_001")
    );
    assert_eq!(
        index.draft_statuses.get("draft_002"),
        Some(&WorkItemDraftStatus::Superseded)
    );
}

#[test]
fn copying_draft_creates_new_draft_id_and_records_source() {
    let mut index = sample_active_index();
    index
        .draft_statuses
        .insert("draft_009".to_string(), WorkItemDraftStatus::Superseded);
    let source = sample_draft_record("draft_001", "round_001");

    let copied =
        copy_draft_for_current_round(&index, &source, "node_copy_1", "2026-06-22T11:00:00Z");

    assert_eq!(copied.draft_id, "draft_010");
    assert_eq!(copied.generation_round_id, "round_001");
    assert_eq!(copied.copied_from_draft_id.as_deref(), Some("draft_001"));
    assert_eq!(copied.status, WorkItemDraftStatus::Draft);
    assert!(copied.active);
    assert_eq!(copied.batch_id, None);
}

#[test]
fn direct_rewrite_supersedes_target_and_downstream() {
    let invalidation =
        outline_rewrite_invalidation_plan(&sample_dependency_outline(), "outline_001")
            .expect("invalidation plan");

    assert_eq!(
        invalidation,
        vec![
            (
                "outline_001".to_string(),
                WorkItemDraftSupersedeReason::DirectRewrite
            ),
            (
                "outline_002".to_string(),
                WorkItemDraftSupersedeReason::AncestorRewritten
            ),
            (
                "outline_003".to_string(),
                WorkItemDraftSupersedeReason::AncestorRewritten
            )
        ]
    );

    let mut direct = sample_draft_record_for_outline("draft_001", "outline_001");
    let mut downstream = sample_draft_record_for_outline("draft_002", "outline_002");
    mark_draft_record_superseded(
        &mut direct,
        Some("draft_004".to_string()),
        WorkItemDraftSupersedeReason::DirectRewrite,
        "2026-06-22T12:00:00Z",
    );
    mark_draft_record_superseded(
        &mut downstream,
        None,
        WorkItemDraftSupersedeReason::AncestorRewritten,
        "2026-06-22T12:00:00Z",
    );

    assert_eq!(direct.status, WorkItemDraftStatus::Superseded);
    assert!(!direct.active);
    assert_eq!(direct.superseded_by_draft_id.as_deref(), Some("draft_004"));
    assert_eq!(
        direct.supersede_reason,
        Some(WorkItemDraftSupersedeReason::DirectRewrite)
    );
    assert_eq!(
        downstream.supersede_reason,
        Some(WorkItemDraftSupersedeReason::AncestorRewritten)
    );
}

#[test]
fn ancestor_rewritten_draft_can_be_copied_and_revalidated() {
    let mut index = sample_active_index();
    index
        .draft_statuses
        .insert("draft_002".to_string(), WorkItemDraftStatus::Superseded);
    let mut source = sample_draft_record_for_outline("draft_002", "outline_002");
    mark_draft_record_superseded(
        &mut source,
        None,
        WorkItemDraftSupersedeReason::AncestorRewritten,
        "2026-06-22T12:00:00Z",
    );

    let copied =
        copy_draft_for_current_round(&index, &source, "node_copy_2", "2026-06-22T12:10:00Z");

    assert_ne!(copied.draft_id, source.draft_id);
    assert_eq!(copied.copied_from_draft_id.as_deref(), Some("draft_002"));
    assert_eq!(copied.status, WorkItemDraftStatus::Draft);
    assert!(copied.active);
    assert_eq!(copied.supersede_reason, None);
    assert_eq!(copied.superseded_at, None);
}

#[test]
fn direct_rewrite_cannot_opt_out() {
    let invalidation =
        outline_rewrite_invalidation_plan(&sample_dependency_outline(), "outline_002")
            .expect("invalidation plan");

    assert!(
        invalidation.iter().any(|(outline_id, reason)| {
            outline_id == "outline_002" && reason == &WorkItemDraftSupersedeReason::DirectRewrite
        }),
        "direct rewrite target must always be invalidated"
    );
}

#[test]
fn batch_id_sequence_is_scoped_to_generation_round() {
    let mut index = sample_active_index();
    index.current_generation_round_id = "round_002".to_string();

    assert_eq!(next_generation_round_id(&index), "round_003");
    assert_eq!(next_draft_id(&index), "draft_002");
    assert_eq!(
        next_batch_id(&index, "2026-06-22T11:00:00Z"),
        "batch_20260622_001"
    );

    index.batches.push(WorkItemBatchRecord {
        batch_id: "batch_20260622_001".to_string(),
        generation_round_id: "round_002".to_string(),
        mode: WorkItemGenerationMode::Batch,
        item_draft_ids: vec![],
        status: WorkItemBatchStatus::Generating,
        validation_failed_ids: vec![],
        created_at: "2026-06-22T11:00:00Z".to_string(),
    });

    assert_eq!(
        next_batch_id(&index, "2026-06-22T11:30:00Z"),
        "batch_20260622_002"
    );
}

#[test]
fn draft_store_rejects_invalid_outline_state_and_batch_id_semantics() {
    let root = tempdir().expect("tempdir");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let store = WorkItemPlanStore::new(paths);

    let mut index = sample_active_index();
    index.outline_state = "drafting".to_string();
    let error = store
        .save_active_index(&index)
        .expect_err("invalid outline_state should fail");
    assert!(matches!(error, ProductStoreError::Json(_)));

    let mut serial = sample_draft_record("draft_001", "round_001");
    serial.batch_id = Some("batch_20260622_001".to_string());
    let error = store
        .put_draft_record(&serial)
        .expect_err("serial draft should not have batch_id");
    assert!(matches!(error, ProductStoreError::Json(_)));

    let mut batch = sample_draft_record("draft_002", "round_001");
    batch.generation_mode = WorkItemGenerationMode::Batch;
    batch.batch_id = None;
    let error = store
        .put_draft_record(&batch)
        .expect_err("batch draft should have batch_id");
    assert!(matches!(error, ProductStoreError::Json(_)));
}

#[test]
fn outline_context_index_keeps_at_most_20_resolutions() {
    let mut index = sample_context_index_with_resolution_count(21, 100);

    compact_outline_context_index(&mut index);

    assert_eq!(index.blocker_resolutions.len(), 20);
    assert_eq!(index.blocker_resolutions[0].merged_count, Some(2));
    assert!(index.blocker_resolutions[0].summary.is_some());
    assert_eq!(
        index.blocker_resolutions[1].blocker_node_id,
        "node_blocker_003"
    );
}

#[test]
fn outline_context_index_summarizes_when_estimated_tokens_exceed_threshold() {
    let mut index = sample_context_index_with_resolution_count(10, 1000);

    compact_outline_context_index(&mut index);

    let total_tokens: u32 = index
        .blocker_resolutions
        .iter()
        .map(|resolution| resolution.estimated_tokens)
        .sum();
    assert!(total_tokens <= 8000);
    assert!(index.blocker_resolutions[0].summary.is_some());
    assert!(
        index.blocker_resolutions[0]
            .merged_count
            .unwrap_or_default()
            > 1
    );
}

#[test]
fn draft_store_does_not_create_real_work_items_or_verification_plans() {
    let root = tempdir().expect("tempdir");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let draft_store = WorkItemPlanStore::new(paths.clone());
    let lifecycle_store = LifecycleStore::new(paths);

    draft_store
        .put_draft_record(&sample_draft_record("draft_001", "round_001"))
        .expect("put draft");

    assert!(
        lifecycle_store
            .list_work_items("project_1", "issue_1")
            .expect("list work items")
            .is_empty()
    );
    assert!(
        lifecycle_store
            .list_verification_plans("project_1", "issue_1")
            .expect("list verification plans")
            .is_empty()
    );
    assert!(
        lifecycle_store
            .list_workspace_sessions("project_1", "issue_1")
            .expect("list workspace sessions")
            .into_iter()
            .all(|session| session.workspace_type != WorkspaceType::WorkItem)
    );
}

fn sample_draft_record(draft_id: &str, generation_round_id: &str) -> WorkItemDraftRecord {
    WorkItemDraftRecord {
        project_id: "project_1".to_string(),
        issue_id: "issue_1".to_string(),
        plan_id: "plan_1".to_string(),
        draft_id: draft_id.to_string(),
        outline_id: "outline_001".to_string(),
        generation_round_id: generation_round_id.to_string(),
        batch_id: None,
        attempt_index: 1,
        outline_version_ref: "artifact://outline/1".to_string(),
        generation_mode: WorkItemGenerationMode::Serial,
        candidate: WorkItemDraftCandidate {
            outline_id: "outline_001".to_string(),
            title: "实现后端 store".to_string(),
            kind: WorkItemKind::Backend,
            goal: "持久化 draft record".to_string(),
            implementation_context: "复用 json_store 原子写".to_string(),
            exclusive_write_scopes: vec!["src/product".to_string()],
            forbidden_write_scopes: vec!["web".to_string()],
            depends_on_outline_ids: vec![],
            required_handoff_from_outline_ids: vec![],
            handoff_summary: "store 可供编译阶段读取".to_string(),
            verification_plan: serde_json::json!({"commands": ["cargo test --locked"]}),
        },
        status: WorkItemDraftStatus::Draft,
        active: true,
        superseded_by_draft_id: None,
        supersede_reason: None,
        copied_from_draft_id: None,
        review_node_id: None,
        review_verdict_ref: None,
        generated_from_node_id: "node_1".to_string(),
        accepted_at: None,
        superseded_at: None,
        created_at: "2026-06-22T10:00:00Z".to_string(),
        updated_at: "2026-06-22T10:00:00Z".to_string(),
    }
}

fn sample_draft_record_for_outline(draft_id: &str, outline_id: &str) -> WorkItemDraftRecord {
    let mut record = sample_draft_record(draft_id, "round_001");
    record.outline_id = outline_id.to_string();
    record.candidate.outline_id = outline_id.to_string();
    record
}

fn sample_dependency_outline() -> WorkItemPlanOutline {
    WorkItemPlanOutline {
        id: "outline_artifact_1".to_string(),
        project_id: "project_1".to_string(),
        issue_id: "issue_1".to_string(),
        source_story_spec_ids: vec!["story_1".to_string()],
        source_design_spec_ids: vec!["design_1".to_string()],
        strategy_summary: "按依赖链拆分".to_string(),
        work_item_outlines: vec![
            sample_outline("outline_001", WorkItemKind::Backend),
            sample_outline("outline_002", WorkItemKind::Frontend),
            sample_outline("outline_003", WorkItemKind::Integration),
        ],
        dependency_graph: vec![
            WorkItemOutlineDependencyEdge {
                from_outline_id: "outline_001".to_string(),
                to_outline_id: "outline_002".to_string(),
            },
            WorkItemOutlineDependencyEdge {
                from_outline_id: "outline_002".to_string(),
                to_outline_id: "outline_003".to_string(),
            },
        ],
        risks: Vec::new(),
        handoff_strategy: "逐项传递".to_string(),
        status: "confirmed".to_string(),
    }
}

fn sample_outline(outline_id: &str, kind: WorkItemKind) -> WorkItemOutline {
    WorkItemOutline {
        outline_id: outline_id.to_string(),
        title: format!("item {outline_id}"),
        kind,
        goal: "goal".to_string(),
        scope: vec![format!("scope/{outline_id}")],
        non_goals: Vec::new(),
        estimated_context_tokens: Some(12_000),
        session_fit: Some(WorkItemOutlineSessionFit::FitsSingleAgentSession),
        source_story_spec_ids: vec!["story_1".to_string()],
        source_design_spec_ids: vec!["design_1".to_string()],
        exclusive_write_scopes: vec![format!("scope/{outline_id}")],
        forbidden_write_scopes: Vec::new(),
        depends_on: Vec::new(),
        verification_intent: vec!["cargo test --locked".to_string()],
        handoff_notes: "handoff".to_string(),
    }
}
