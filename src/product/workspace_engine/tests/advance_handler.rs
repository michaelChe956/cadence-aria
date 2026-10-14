use super::*;
use crate::product::advance_store::{
    AdvanceInitializationPhase, AdvanceRecord, AdvanceStatus, AdvanceStore,
};
use crate::product::coding_attempt_store::CreateGroupCodingAttemptInput;
use crate::product::models::{PlanRevisionReason, WorkItemPlanLineage, WorkItemPlanRevision};
use crate::product::models::{
    WorkItemDraftCandidate, WorkItemDraftRecord, WorkItemDraftStatus,
    WorkItemDraftVerificationPlan, WorkItemGenerationMode,
};
use crate::product::repository_store::RepositoryStore;
#[cfg(test)]
use crate::product::workspace_engine::{
    AdvanceInitializationFailpoint, AdvanceInitializationFailpointMode,
    register_advance_initialization_failpoint,
};

const PROJECT_ID: &str = "project_advance";
const ISSUE_ID: &str = "issue_advance";
const PLAN_ID: &str = "plan_advance";
const COMMAND_ID: &str = "command_advance";

fn plan_options() -> IssueWorkItemPlanOptions {
    IssueWorkItemPlanOptions {
        include_integration_tests: false,
        include_e2e_tests: false,
        force_frontend_backend_split: false,
        require_execution_plan_confirm: false,
    }
}

fn engine_fixture(root: &TempDir, lifecycle: LifecycleStore) -> WorkspaceEngine {
    let record = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            entity_id: PLAN_ID.to_string(),
            workspace_type: WorkspaceType::WorkItemPlan,
            author_provider: ProviderName::Fake,
            reviewer_provider: Some(ProviderName::Fake),

            review_rounds: 0,
            superpowers_enabled: false,
            openspec_enabled: false,
            work_item_plan_options: None,
        })
        .unwrap();
    let (event_tx, _event_rx) = mpsc::channel(8);
    WorkspaceEngine::new_persistent(
        Arc::new(CheckpointStore::new(root.path().join("checkpoints"))),
        lifecycle,
        event_tx,
        WorkspaceSession::from_record(record),
    )
}

fn confirmed_plan(lifecycle: &LifecycleStore, work_item_ids: Vec<String>) {
    lifecycle
        .create_issue_work_item_plan(CreateIssueWorkItemPlanInput {
            id: Some(PLAN_ID.to_string()),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            source_story_spec_ids: Vec::new(),
            source_design_spec_ids: Vec::new(),
            options: plan_options(),
            status: IssueWorkItemPlanStatus::Confirmed,
            work_item_ids,
            repository_profile_ref: None,
            verification_plan_ids: Vec::new(),
            dependency_graph: Vec::new(),
            created_from_provider_run: None,
            validator_findings: Vec::new(),
        })
        .unwrap();
}

fn active_lineage(lifecycle: &LifecycleStore, active_amendment_id: Option<&str>) {
    let revision_store =
        crate::product::work_item_revision_store::WorkItemRevisionStore::new(lifecycle.app_paths());
    let lineage = WorkItemPlanLineage {
        id: PLAN_ID.to_string(),
        project_id: PROJECT_ID.to_string(),
        issue_id: ISSUE_ID.to_string(),
        story_spec_refs: Vec::new(),
        design_spec_refs: Vec::new(),
        active_revision_id: Some("revision_advance".to_string()),
        active_amendment_id: active_amendment_id.map(str::to_string),
        created_at: "2026-08-31T00:00:00Z".to_string(),
        updated_at: "2026-08-31T00:00:00Z".to_string(),
    };
    revision_store.put_plan_lineage(&lineage).unwrap();
    revision_store
        .put_plan_revision(
            &lineage,
            &WorkItemPlanRevision {
                id: "revision_advance".to_string(),
                plan_id: PLAN_ID.to_string(),
                revision_no: 1,
                supersedes: None,
                reason: PlanRevisionReason::InitialCompile,
                work_item_bindings: Default::default(),
                dependency_graph_revision_id: "graph_advance".to_string(),
                validation_report_ref: "validation_advance".to_string(),
                plan_projection_bundle_id: "projection_advance".to_string(),
                publication_provenance_ref: None,
                created_at: "2026-08-31T00:00:00Z".to_string(),
            },
        )
        .unwrap();
}

fn seed_advance_draft_records(app_paths: &ProductAppPaths) {
    let revision_store =
        crate::product::work_item_revision_store::WorkItemRevisionStore::new(app_paths.clone());
    let lineage = revision_store
        .get_plan_lineage("project_0001", "issue_plan_0001", "work_item_plan_0001")
        .unwrap();
    let plan_store = WorkItemPlanStore::new(app_paths.clone());
    for (logical_id, revision_id) in [
        ("wi_core", "work_item_revision_wi_core_0001"),
        ("wi_registration", "work_item_revision_wi_registration_0001"),
        ("wi_unrelated", "work_item_revision_wi_unrelated_0001"),
    ] {
        let revision = revision_store
            .get_work_item_revision(&lineage, logical_id, revision_id)
            .unwrap();
        plan_store
            .put_draft_record(&WorkItemDraftRecord {
                project_id: "project_0001".to_string(),
                issue_id: "issue_plan_0001".to_string(),
                plan_id: "work_item_plan_0001".to_string(),
                draft_id: revision.source_draft_revision_id.clone(),
                outline_id: format!("outline_{logical_id}"),
                generation_round_id: "round_advance".to_string(),
                batch_id: None,
                attempt_index: 1,
                outline_version_ref: "outline_version_advance".to_string(),
                generation_mode: WorkItemGenerationMode::Serial,
                generation_diagnostics: None,
                candidate: WorkItemDraftCandidate {
                    target_repository_id: None,
                    outline_id: format!("outline_{logical_id}"),
                    logical_work_item_id: logical_id.to_string(),
                    canonical_contract_candidate: revision.canonical_contract,
                    verification_plan: WorkItemDraftVerificationPlan { checks: Vec::new() },
                },
                status: WorkItemDraftStatus::Accepted,
                active: true,
                superseded_by_draft_id: None,
                supersede_reason: None,
                copied_from_draft_id: None,
                review_node_id: None,
                review_verdict_ref: None,
                generated_from_node_id: "advance_fixture".to_string(),
                accepted_at: Some("2026-08-31T00:00:00Z".to_string()),
                superseded_at: None,
                created_at: "2026-08-31T00:00:00Z".to_string(),
                updated_at: "2026-08-31T00:00:00Z".to_string(),
            })
            .unwrap();
    }
}

fn input(command_id: &str) -> AdvanceInput {
    AdvanceInput {
        command_id: command_id.to_string(),
        project_id: PROJECT_ID.to_string(),
        issue_id: ISSUE_ID.to_string(),
        plan_id: PLAN_ID.to_string(),
    }
}

fn build_advance_engine(root: &TempDir, lifecycle: LifecycleStore) -> WorkspaceEngine {
    let session_record = lifecycle
        .list_workspace_sessions("project_0001", "issue_plan_0001")
        .unwrap()
        .into_iter()
        .find(|session| {
            session.workspace_type == WorkspaceType::WorkItemPlan
                && session.entity_id == "work_item_plan_0001"
        })
        .expect("fixture plan workspace session");
    let (event_tx, _event_rx) = mpsc::channel(8);
    WorkspaceEngine::new_persistent(
        Arc::new(CheckpointStore::new(root.path().join("checkpoints"))),
        lifecycle,
        event_tx,
        WorkspaceSession::from_record(session_record),
    )
}

async fn advance_fixture() -> (TempDir, ProductAppPaths, LifecycleStore, WorkspaceEngine) {
    let root = tempfile::tempdir().unwrap();
    crate::web::test_controls::PlanRepairFixtureRuntime::seed(
        root.path(),
        crate::web::test_controls::PlanRepairFixtureControl::default(),
    )
    .await
    .expect("seed authoritative advance fixture");
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let coding_store =
        crate::product::coding_attempt_store::CodingAttemptStore::new(app_paths.clone());
    let seeded_attempt = coding_store
        .get_attempt_for_work_item_group(
            "project_0001",
            "issue_plan_0001",
            "work_item_plan_0001",
            None,
        )
        .unwrap()
        .expect("seeded group attempt");
    coding_store
        .delete_attempt("project_0001", "issue_plan_0001", &seeded_attempt.id)
        .unwrap();
    seed_advance_draft_records(&app_paths);
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let engine = build_advance_engine(&root, lifecycle.clone());
    (root, app_paths, lifecycle, engine)
}

fn fixture_input(command_id: &str) -> AdvanceInput {
    AdvanceInput {
        command_id: command_id.to_string(),
        project_id: "project_0001".to_string(),
        issue_id: "issue_plan_0001".to_string(),
        plan_id: "work_item_plan_0001".to_string(),
    }
}

fn stored_record(command_id: &str) -> AdvanceRecord {
    AdvanceRecord {
        id: format!("advance_{command_id}"),
        command_id: command_id.to_string(),
        project_id: PROJECT_ID.to_string(),
        issue_id: ISSUE_ID.to_string(),
        plan_id: PLAN_ID.to_string(),
        plan_revision_id: "revision_advance".to_string(),
        attempt_id: Some("attempt_advance".to_string()),
        target_attempts: Vec::new(),
        status: AdvanceStatus::Ready,
        workspace_entry: Some("/workspace/attempt_advance".to_string()),
        error: None,
        created_at: "2026-08-31T00:00:00Z".to_string(),
        updated_at: "2026-08-31T00:00:01Z".to_string(),
    }
}

#[tokio::test]
async fn advance_valid_path_persists_ready_group_and_emits_completion() {
    let root = tempfile::tempdir().unwrap();
    crate::web::test_controls::PlanRepairFixtureRuntime::seed(
        root.path(),
        crate::web::test_controls::PlanRepairFixtureControl::default(),
    )
    .await
    .expect("seed complete authoritative advance fixture");

    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let coding_store =
        crate::product::coding_attempt_store::CodingAttemptStore::new(app_paths.clone());
    let seeded_attempt = coding_store
        .get_attempt_for_work_item_group(
            "project_0001",
            "issue_plan_0001",
            "work_item_plan_0001",
            None,
        )
        .unwrap()
        .expect("seeded group attempt");
    coding_store
        .delete_attempt("project_0001", "issue_plan_0001", &seeded_attempt.id)
        .unwrap();
    seed_advance_draft_records(&app_paths);

    let lifecycle = LifecycleStore::new(app_paths.clone());
    let session_record = lifecycle
        .list_workspace_sessions("project_0001", "issue_plan_0001")
        .unwrap()
        .into_iter()
        .find(|session| {
            session.workspace_type == WorkspaceType::WorkItemPlan
                && session.entity_id == "work_item_plan_0001"
        })
        .expect("seeded plan workspace session");
    let (event_tx, _event_rx) = mpsc::channel(8);
    let mut engine = WorkspaceEngine::new_persistent(
        Arc::new(CheckpointStore::new(root.path().join("checkpoints"))),
        lifecycle.clone(),
        event_tx,
        WorkspaceSession::from_record(session_record),
    );

    let outcome = engine
        .handle_advance(AdvanceInput {
            command_id: "command_valid_path".to_string(),
            project_id: "project_0001".to_string(),
            issue_id: "issue_plan_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
        })
        .await
        .expect("valid advance path");
    let (record, attempt_id, workspace_entry) = match outcome {
        AdvanceOutcome::Completed {
            record,
            attempt_id,
            workspace_entry,
            ..
        } => (record, attempt_id, workspace_entry),
        other => panic!("valid advance must complete, got {other:?}"),
    };
    assert_eq!(record.status, AdvanceStatus::Ready);
    assert_eq!(record.attempt_id.as_deref(), Some(attempt_id.as_str()));
    assert_eq!(record.workspace_entry, Some(workspace_entry.clone()));

    let response = crate::web::handlers::map_advance_outcome(
        "command_valid_path".to_string(),
        AdvanceOutcome::Completed {
            record: record.clone(),
            attempt_id: attempt_id.clone(),
            workspace_entry: workspace_entry.clone(),
            target_attempts: Vec::new(),
        },
    );
    assert!(matches!(
        response,
        crate::web::workspace_ws_types::WsOutMessage::AdvanceCompleted {
            command_id,
            attempt_id: response_attempt_id,
            workspace_entry: response_workspace_entry,
            ..
        } if command_id == "command_valid_path"
            && response_attempt_id == attempt_id
            && response_workspace_entry == workspace_entry
    ));

    let advance_store = AdvanceStore::new(app_paths.clone());
    let persisted_record = advance_store
        .get_advance_for_plan("project_0001", "issue_plan_0001", "work_item_plan_0001")
        .unwrap()
        .expect("durable ready advance record");
    assert_eq!(persisted_record, record);
    let outer = advance_store
        .get_advance_initialization(&persisted_record)
        .unwrap()
        .expect("durable advance initialization journal");
    assert_eq!(outer.phase, AdvanceInitializationPhase::Ready);

    let coding_store = crate::product::coding_attempt_store::CodingAttemptStore::new(app_paths);
    let attempt = coding_store
        .get_attempt("project_0001", "issue_plan_0001", &attempt_id)
        .unwrap();
    assert_eq!(
        attempt.admission_kind,
        crate::product::coding_models::CodingAdmissionKind::ScAdvance
    );
    let group = coding_store
        .get_group_initialization("project_0001", "issue_plan_0001", "work_item_plan_0001")
        .unwrap();
    assert_eq!(
        group.phase,
        crate::product::coding_attempt_store::CodingGroupInitializationPhase::Completed
    );
    assert_eq!(group.attempt.id, attempt.id);
    assert_eq!(
        group.plan_binding.bound_plan_revision_id,
        "plan_revision_0001"
    );
    let units = coding_store
        .list_coding_units("project_0001", "issue_plan_0001", &attempt_id)
        .unwrap();
    assert_eq!(units.len(), group.units.len());
    assert_eq!(
        units
            .iter()
            .map(|unit| unit.logical_work_item_id.as_str())
            .collect::<Vec<_>>(),
        group
            .units
            .iter()
            .map(|unit| unit.logical_work_item_id.as_str())
            .collect::<Vec<_>>()
    );
    assert!(lifecycle.get_workspace_session(&attempt_id).is_err());
    assert!(
        lifecycle
            .list_workspace_sessions("project_0001", "issue_plan_0001")
            .unwrap()
            .iter()
            .all(|session| session.provider_start_ledger.is_empty())
    );
}
include!("advance_handler_parts/checkpoint_recovery.inc.rs");

#[tokio::test]
async fn advance_initialization_engine_failure_is_durable_on_record_and_journal() {
    let (root, app_paths, lifecycle, mut engine) = advance_fixture().await;
    let request = fixture_input("command_engine_failure");
    let _failpoint = register_advance_initialization_failpoint(
        &request,
        AdvanceInitializationFailpoint::PlanBindingSaved,
        AdvanceInitializationFailpointMode::Error,
    );
    let error = engine
        .handle_advance(request)
        .await
        .expect_err("injected engine failure must be returned");
    assert!(error.contains("advance_initialization_failpoint:PlanBindingSaved"));
    let store = AdvanceStore::new(app_paths);
    let record = store
        .get_advance_by_command_id("project_0001", "issue_plan_0001", "command_engine_failure")
        .unwrap()
        .expect("failed record");
    assert_eq!(record.status, AdvanceStatus::Failed);
    assert!(record.error.is_some());
    let journal = store
        .get_advance_initialization(&record)
        .unwrap()
        .expect("failed initialization journal");
    assert_eq!(journal.error, record.error);
    assert!(
        lifecycle
            .list_workspace_sessions("project_0001", "issue_plan_0001")
            .unwrap()
            .iter()
            .all(|session| session.provider_start_ledger.is_empty())
    );
    let _ = root;
}

#[tokio::test]
async fn advance_rejects_active_amendment_without_any_durable_write() {
    let root = TempDir::new().unwrap();
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths.clone());
    confirmed_plan(&lifecycle, Vec::new());
    active_lineage(&lifecycle, Some("amendment_active"));
    let mut engine = engine_fixture(&root, lifecycle);
    let outcome = engine.handle_advance(input(COMMAND_ID)).await.unwrap();
    assert!(
        matches!(outcome, AdvanceOutcome::Rejected { code, .. } if code == "ADVANCE_ACTIVE_PLAN_REVISION")
    );
    assert!(
        !app_paths
            .issue_root(PROJECT_ID, ISSUE_ID)
            .join("advance-records")
            .exists()
    );
}

#[tokio::test]
async fn advance_rejects_unconfirmed_plan_without_any_durable_write() {
    let root = TempDir::new().unwrap();
    let lifecycle = LifecycleStore::new(ProductAppPaths::new(root.path().join(".aria")));
    lifecycle
        .create_issue_work_item_plan(CreateIssueWorkItemPlanInput {
            id: Some(PLAN_ID.to_string()),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            source_story_spec_ids: Vec::new(),
            source_design_spec_ids: Vec::new(),
            options: plan_options(),
            status: IssueWorkItemPlanStatus::Draft,
            work_item_ids: Vec::new(),
            repository_profile_ref: None,
            verification_plan_ids: Vec::new(),
            dependency_graph: Vec::new(),
            created_from_provider_run: None,
            validator_findings: Vec::new(),
        })
        .unwrap();
    let mut engine = engine_fixture(&root, lifecycle.clone());
    let outcome = engine.handle_advance(input(COMMAND_ID)).await.unwrap();
    assert!(
        matches!(outcome, AdvanceOutcome::Rejected { code, .. } if code == "ADVANCE_PLAN_NOT_CONFIRMED")
    );
    assert!(
        !root
            .path()
            .join(".aria/projects/project_advance/issues/issue_advance/advance-records")
            .exists()
    );
}

#[tokio::test]
async fn advance_rejects_missing_child_session_without_any_durable_write() {
    let root = TempDir::new().unwrap();
    let lifecycle = LifecycleStore::new(ProductAppPaths::new(root.path().join(".aria")));
    confirmed_plan(&lifecycle, vec!["work_item_missing".to_string()]);
    active_lineage(&lifecycle, None);
    let mut engine = engine_fixture(&root, lifecycle);
    let outcome = engine.handle_advance(input(COMMAND_ID)).await.unwrap();
    assert!(
        matches!(outcome, AdvanceOutcome::Rejected { code, .. } if code == "ADVANCE_CHILD_SESSION_MISSING")
    );
    assert!(
        !root
            .path()
            .join(".aria/projects/project_advance/issues/issue_advance/advance-records")
            .exists()
    );
}

#[tokio::test]
async fn advance_rejects_active_compile_or_revision_without_any_durable_write() {
    let root = TempDir::new().unwrap();
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths.clone());
    confirmed_plan(&lifecycle, Vec::new());
    active_lineage(&lifecycle, None);
    WorkItemPlanStore::new(app_paths)
        .put_compile_transaction(&WorkItemPlanCompileTransaction {
            compile_id: "compile_active".to_string(),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            plan_id: PLAN_ID.to_string(),
            flow_kind: None,
            source_revision_id: None,
            source_revision_ref: None,
            plan_candidate_ir_ref: None,
            mechanical_report_ref: None,
            publication_provenance_ref: None,
            publication_provenance_content_hash: None,
            generation_round_id: "round_advance".to_string(),
            outline_version_ref: "outline_advance".to_string(),
            active_draft_ids: Vec::new(),
            status: WorkItemPlanCompileStatus::Preparing,
            plan_commit_state: WorkItemPlanCommitState::NotStarted,
            step_cursor: "preparing".to_string(),
            outline_to_work_item_id: Default::default(),
            outline_to_verification_plan_id: Default::default(),
            created_work_item_ids: Vec::new(),
            created_verification_plan_ids: Vec::new(),
            child_session_ids: Vec::new(),
            validator_findings: Vec::new(),
            abort_requested_at: None,
            failure_reason: None,
            previous_plan_snapshot: lifecycle
                .get_issue_work_item_plan(PROJECT_ID, ISSUE_ID, PLAN_ID)
                .unwrap(),
            created_at: "2026-08-31T00:00:00Z".to_string(),
            updated_at: "2026-08-31T00:00:00Z".to_string(),
            committed_at: None,
        })
        .unwrap();
    let mut engine = engine_fixture(&root, lifecycle);
    let outcome = engine.handle_advance(input(COMMAND_ID)).await.unwrap();
    assert!(
        matches!(outcome, AdvanceOutcome::Rejected { code, .. } if code == "ADVANCE_ACTIVE_PLAN_COMPILE")
    );
    assert!(
        !root
            .path()
            .join(".aria/projects/project_advance/issues/issue_advance/advance-records")
            .exists()
    );
}

#[tokio::test]
async fn advance_replay_by_command_id_precedes_changed_preconditions() {
    let root = TempDir::new().unwrap();
    let store = AdvanceStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let record = stored_record(COMMAND_ID);
    store.put_record(&record).unwrap();
    let lifecycle = LifecycleStore::new(store.app_paths());
    let mut engine = engine_fixture(&root, lifecycle);
    let outcome = engine.handle_advance(input(COMMAND_ID)).await.unwrap();
    assert_eq!(outcome, AdvanceOutcome::Replayed { record });
}

#[tokio::test]
async fn advance_replay_by_plan_id_precedes_changed_preconditions() {
    let root = TempDir::new().unwrap();
    let store = AdvanceStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let record = stored_record("different_command");
    store.put_record(&record).unwrap();
    let lifecycle = LifecycleStore::new(store.app_paths());
    let mut engine = engine_fixture(&root, lifecycle);
    let outcome = engine.handle_advance(input(COMMAND_ID)).await.unwrap();
    assert_eq!(outcome, AdvanceOutcome::Replayed { record });
}

// ============================================================================
// per-issue-base-branch REQ-PIB-03 场景 1（T3.1，k3 终审 F1 修复配套）：
// advance 新 attempt 的 fork 基线必须读 issue.base_branch（三面同源解析链），
// 不得回退当前检出或 HEAD。
// ============================================================================

fn lock_fixture_issue_base_branch(app_paths: &ProductAppPaths, base_branch: &str) {
    let issue_store = crate::product::issue_store::IssueStore::new(app_paths.clone());
    let mut issue = issue_store
        .get("project_0001", "issue_plan_0001")
        .expect("fixture issue");
    issue.base_branch = Some(base_branch.to_string());
    crate::product::json_store::write_json(
        &app_paths
            .issue_root("project_0001", "issue_plan_0001")
            .join("issue.json"),
        &issue,
    )
    .expect("lock fixture issue base branch");
}

#[tokio::test]
async fn advance_initialization_forks_from_issue_base_branch() {
    let (root, app_paths, lifecycle, mut engine) = advance_fixture().await;
    // issue 基线锁定 feature/x（兄弟分支，当前检出仍是主干）。
    super::advance_split_targets::run_git(&root.path().join("worktree"), &["branch", "feature/x"]);
    lock_fixture_issue_base_branch(&app_paths, "feature/x");

    let outcome = engine
        .handle_advance(fixture_input("command_t31_feature_base"))
        .await
        .expect("advance completes with locked base branch");
    assert!(matches!(outcome, AdvanceOutcome::Completed { .. }));

    // journal 与共享 worktree 登记都必须按 issue 锁定基线（fork 契约），
    // 而非仓库当前检出（旧实现回落 current_git_branch→HEAD）。
    let journal = crate::product::coding_attempt_store::CodingAttemptStore::new(app_paths.clone())
        .get_group_initialization("project_0001", "issue_plan_0001", "work_item_plan_0001")
        .expect("advance group journal");
    assert_eq!(journal.attempt.base_branch, "feature/x");

    let shared = lifecycle
        .get_issue_shared_worktree("project_0001", "issue_plan_0001")
        .expect("read shared worktree")
        .expect("shared worktree bound");
    assert_eq!(shared.base_branch, "feature/x");
}

#[tokio::test]
async fn advance_initialization_fails_closed_when_issue_base_branch_deleted() {
    let (_root, app_paths, _lifecycle, mut engine) = advance_fixture().await;
    // 锁定分支从未在仓库创建（等价于创建后被删）。
    lock_fixture_issue_base_branch(&app_paths, "gone");

    let error = engine
        .handle_advance(fixture_input("command_t31_missing_base"))
        .await
        .expect_err("missing base branch must fail closed");
    assert!(
        error.contains("基准分支不存在"),
        "unexpected advance error: {error}"
    );

    // fail-closed：不写 group journal；advance record 落 Failed + 诊断。
    let coding_store =
        crate::product::coding_attempt_store::CodingAttemptStore::new(app_paths.clone());
    assert!(matches!(
        coding_store.get_group_initialization(
            "project_0001",
            "issue_plan_0001",
            "work_item_plan_0001"
        ),
        Err(crate::product::json_store::ProductStoreError::NotFound { .. })
    ));
    let record = AdvanceStore::new(app_paths)
        .get_advance_for_plan("project_0001", "issue_plan_0001", "work_item_plan_0001")
        .expect("read advance record")
        .expect("advance record persisted");
    assert_eq!(record.status, AdvanceStatus::Failed);
    assert!(
        record
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("基准分支不存在"),
        "advance record error must carry the diagnosis: {record:?}"
    );
}

/// C1 Task 7（REQ-ADV-C1-RETRY）：Failed advance 显式 retry——独立 durable
/// retry 事实、原失败审计字段保留、同一 attempt 续做到 Ready；普通
/// advance 对 Failed 一律 Replayed 不隐式 retry；副作用门未确认只写
/// NeedsHuman 不重跑；过期 checkpoint 拒绝且 durable 不变；同 command
/// 重放返回原 retry 结果。
#[tokio::test]
async fn retry_initialization_preserves_failed_record_and_attempt() {
    use crate::product::issue_automation_store::IssueAutomationStore;
    use crate::product::logical_codebase::EnrollmentTarget;
    use crate::product::models::automation::{
        EnrollmentOptions, EnrollmentSource, EnrollmentWriteCommand, OperationState,
        RetryInitializationRequest, SourceRevisionRef,
    };

    let (root, app_paths, _lifecycle, mut engine) = advance_fixture().await;
    let advance_store = AdvanceStore::new(app_paths.clone());
    let coding_store =
        crate::product::coding_attempt_store::CodingAttemptStore::new(app_paths.clone());

    // 在 PlanBindingSaved（≥ WorktreeBound）注入失败：可能存在外部副作用
    // 状态，先走副作用门。
    let request = fixture_input("command_retry_side_effect");
    let _failpoint = register_advance_initialization_failpoint(
        &request,
        AdvanceInitializationFailpoint::PlanBindingSaved,
        AdvanceInitializationFailpointMode::Error,
    );
    engine
        .handle_advance(request)
        .await
        .expect_err("injected engine failure must be returned");
    let record = advance_store
        .get_advance_for_plan("project_0001", "issue_plan_0001", "work_item_plan_0001")
        .unwrap()
        .expect("failed record");
    assert_eq!(record.status, AdvanceStatus::Failed);
    let journal = advance_store
        .get_advance_initialization(&record)
        .unwrap()
        .expect("failed initialization journal");
    let failed_error = journal.error.clone().expect("journal failure fact");
    let attempt_id = journal.attempt_id.clone();
    let created_at_before = record.created_at.clone();
    let attempts_before = coding_store
        .list_attempts_for_issue("project_0001", "issue_plan_0001")
        .unwrap()
        .len();

    // 普通 advance（异 command）对 Failed 一律 Replayed，不创建 retry/attempt。
    let replay = engine
        .handle_advance(fixture_input("command_retry_plain_advance"))
        .await
        .expect("plain advance replays the failed fact");
    assert!(
        matches!(replay, AdvanceOutcome::Replayed { ref record } if record.status == AdvanceStatus::Failed),
        "plain advance must not implicitly retry: {replay:?}"
    );
    assert!(
        advance_store
            .get_retry_initialization("project_0001", "issue_plan_0001", "retry_cmd_0001")
            .unwrap()
            .is_none()
    );

    // enrollment binding（store 级 Enable + 显式 target → binding v1）。
    let automation = IssueAutomationStore::new(app_paths.clone());
    automation
        .compare_and_set(
            "project_0001",
            "issue_plan_0001",
            None,
            EnrollmentWriteCommand::Enable {
                selection_key: "selection_retry".to_string(),
                source: EnrollmentSource {
                    stories: vec![SourceRevisionRef {
                        id: "story_spec_0001".to_string(),
                        version: 1,
                    }],
                    designs: vec![],
                },
                options: EnrollmentOptions {
                    author_provider: crate::product::models::ProviderName::Fake,
                    reviewer_provider: crate::product::models::ProviderName::Fake,
                    review_rounds: 1,
                    superpowers_enabled: false,
                    openspec_enabled: false,
                    plan_options: plan_options(),
                },
                target: EnrollmentTarget::SingleRepository {
                    repository_id: "repository_0001".to_string(),
                },
            },
        )
        .expect("enable enrollment");
    let binding = automation
        .get("project_0001", "issue_plan_0001")
        .unwrap()
        .expect("enrollment")
        .binding_history
        .expect("binding history")
        .current;

    // 副作用门：未确认 → NeedsHuman（outcome=None），只写 retry 事实，
    // 不重跑步骤（record 仍 Failed、失败审计保留）。
    let unconfirmed = RetryInitializationRequest {
        command_id: "retry_cmd_0001".to_string(),
        expected_binding: binding.clone(),
        expected_attempt_id: attempt_id.clone(),
        expected_checkpoint: journal.phase,
        confirm_unknown_side_effect: false,
    };
    let result = engine
        .retry_initialization(&unconfirmed)
        .await
        .expect("side-effect gate answers without error");
    assert_eq!(result.state, OperationState::NeedsHuman);
    assert!(result.outcome.is_none());
    let saved = advance_store
        .get_retry_initialization("project_0001", "issue_plan_0001", "retry_cmd_0001")
        .unwrap()
        .expect("durable retry fact");
    assert_eq!(saved.state, OperationState::NeedsHuman);
    let unchanged = advance_store
        .get_advance_for_plan("project_0001", "issue_plan_0001", "work_item_plan_0001")
        .unwrap()
        .expect("record");
    assert_eq!(unchanged.status, AdvanceStatus::Failed);
    let unchanged_journal = advance_store
        .get_advance_initialization(&unchanged)
        .unwrap()
        .expect("journal");
    assert_eq!(
        unchanged_journal.error.as_deref(),
        Some(failed_error.as_str())
    );

    // 过期 checkpoint：拒绝且 durable 不变（不写 retry 事实）。
    let stale = RetryInitializationRequest {
        command_id: "retry_cmd_0003".to_string(),
        expected_binding: binding.clone(),
        expected_attempt_id: attempt_id.clone(),
        expected_checkpoint: AdvanceInitializationPhase::JournalPrepared,
        confirm_unknown_side_effect: true,
    };
    let error = engine
        .retry_initialization(&stale)
        .await
        .expect_err("stale checkpoint rejected");
    assert!(error.contains("checkpoint expired"), "{error}");
    assert!(
        advance_store
            .get_retry_initialization("project_0001", "issue_plan_0001", "retry_cmd_0003")
            .unwrap()
            .is_none()
    );

    // 确认后续做：Accepted + 同一 attempt 到 Ready；attempt 数量/id 不变；
    // 原 created_at 不变；失败审计保留在 record.error（不改写为成功前史）。
    let confirmed = RetryInitializationRequest {
        command_id: "retry_cmd_0002".to_string(),
        expected_binding: binding.clone(),
        expected_attempt_id: attempt_id.clone(),
        expected_checkpoint: journal.phase,
        confirm_unknown_side_effect: true,
    };
    let result = engine
        .retry_initialization(&confirmed)
        .await
        .expect("confirmed retry continues the same attempt");
    assert_eq!(result.state, OperationState::Accepted);
    let outcome = result.outcome.clone();
    let AdvanceOutcome::Completed { record: ready, .. } = outcome.expect("retry outcome") else {
        panic!("confirmed retry must complete to Ready");
    };
    assert_eq!(ready.status, AdvanceStatus::Ready);
    assert_eq!(ready.attempt_id.as_deref(), Some(attempt_id.as_str()));
    assert_eq!(ready.created_at, created_at_before);
    assert_eq!(
        coding_store
            .list_attempts_for_issue("project_0001", "issue_plan_0001")
            .unwrap()
            .len(),
        attempts_before
    );
    let saved = advance_store
        .get_retry_initialization("project_0001", "issue_plan_0001", "retry_cmd_0002")
        .unwrap()
        .expect("durable retry fact");
    assert_eq!(saved.state, OperationState::Accepted);
    assert_eq!(saved.attempt_id, attempt_id);

    // 同 command 重放：Replayed（Ready 记录幂等投影，不建第二链）。
    let replayed = engine
        .retry_initialization(&confirmed)
        .await
        .expect("same-command replay");
    assert_eq!(replayed.state, OperationState::Replayed);
    assert!(matches!(
        replayed.outcome,
        Some(AdvanceOutcome::Replayed { ref record }) if record.status == AdvanceStatus::Ready
    ));
    assert_eq!(
        coding_store
            .list_attempts_for_issue("project_0001", "issue_plan_0001")
            .unwrap()
            .len(),
        attempts_before
    );
    let _ = root;
}
// ─── fix-cascade-deletion Task D2.1d＋H3：advance 及 retry 真实入口 lease 与旧重放拒收 ───
//
// REQ-DEL-06/07（plan D2.1d Step 1）＋设计 2026-10-14 H3：advance 入口在
// engine/arbitration 锁外持 owning Plan run lease——准入在持期间删除独占
// 立即 Busy（零墓碑零破坏）；purging/deleted 身份守卫先于 command_id/
// plan_id 旧幂等命中：旧 Ready record 不回放（不产生 AdvanceCompleted 旧
// 入口），同/新 command、retry 一律稳定拒绝、零 attempt/retry JSON 新增；
// Ready 判定 false；WS 面经 map_advance_outcome 仍真实 Rejected。
#[tokio::test]
async fn cascade_d2_1d_deleted_advance_cannot_replay() {
    use crate::product::entity_deletion_store::{
        DeletionEntityKind, DeletionPhase, EntityDeletionStore, EntityScope,
    };
    use crate::product::entity_operation_store::{EntityOperationError, EntityOperationStore};
    use crate::product::models::automation::RetryInitializationRequest;
    use crate::product::workspace_engine::AdvancePlanAdmission;

    let root = tempfile::tempdir().unwrap();
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let store = AdvanceStore::new(app_paths.clone());
    let record = stored_record(COMMAND_ID);
    store.put_record(&record).unwrap();
    let lifecycle = LifecycleStore::new(app_paths.clone());
    // engine fixture 先于任何墓碑建立（session 创建自身受 D1.3a 准入约束）。
    let mut engine = engine_fixture(&root, lifecycle);
    let operations = EntityOperationStore::new(app_paths.clone());
    let deletions = EntityDeletionStore::new(app_paths.clone());
    let scope = EntityScope::new(PROJECT_ID, ISSUE_ID, DeletionEntityKind::Plan, PLAN_ID);

    // (a) 入口准入在持（advance_plan/retry HTTP 同一获取面）：删除独占立即
    //     Busy，零墓碑零 pending（零破坏拒绝）。
    let admission = AdvancePlanAdmission::acquire(&app_paths, PROJECT_ID, ISSUE_ID, PLAN_ID)
        .expect("plan run admission for a live plan");
    match operations.try_acquire_delete(&scope) {
        Err(EntityOperationError::Busy) => {}
        Err(other) => panic!("expected Busy, got: {other:?}"),
        Ok(_) => panic!("delete must be busy while the advance admission is held"),
    }
    assert!(
        deletions.get(&scope).unwrap().is_none(),
        "busy rejection must leave zero tombstone"
    );
    drop(admission);

    // (b) purging 墓碑先于旧幂等命中：同 command（Ready record 命中位）与
    //     新 command 都稳定 Rejected，绝不 Replayed→AdvanceCompleted。
    let delete_lease = operations
        .try_acquire_delete(&scope)
        .expect("delete exclusive after the admission closed");
    deletions.begin(&delete_lease).expect("purging tombstone");
    drop(delete_lease);
    assert_eq!(
        deletions.get(&scope).unwrap().map(|tomb| tomb.phase),
        Some(DeletionPhase::Purging)
    );

    for command in [COMMAND_ID, "command_d21d_late"] {
        let outcome = engine.handle_advance(input(command)).await.unwrap();
        match &outcome {
            AdvanceOutcome::Rejected { code, reason, .. } => {
                assert_eq!(code, "ADVANCE_PLAN_DELETING", "{command}: {reason}");
            }
            other => panic!("purging plan must reject {command}, got {other:?}"),
        }
        // WS 面仍真实 Rejected（不发 AdvanceCompleted 旧入口）。
        assert!(matches!(
            crate::web::handlers::map_advance_outcome(command.to_string(), outcome),
            crate::web::workspace_ws_types::WsOutMessage::AdvanceRejected { .. }
        ));
    }

    // retry 面：身份守卫先于 Failed 前置/命令账本——稳定拒绝且零 retry 事实。
    let binding: crate::product::models::automation::EnrollmentBindingIdentity =
        serde_json::from_value(serde_json::json!({
            "binding_version": 1,
            "enrollment_id": "enrollment_d21d",
            "plan_id": PLAN_ID,
            "session_id": "session_d21d",
            "source": { "stories": [], "designs": [] },
            "target": { "kind": "single_repository", "repository_id": "repository_0001" },
            "author_provider": "fake",
            "reviewer_provider": "fake"
        }))
        .expect("binding identity");
    let retry_error = engine
        .retry_initialization(&RetryInitializationRequest {
            command_id: "retry_d21d_0001".to_string(),
            expected_binding: binding,
            expected_attempt_id: "attempt_advance".to_string(),
            expected_checkpoint: AdvanceInitializationPhase::JournalPrepared,
            confirm_unknown_side_effect: true,
        })
        .await
        .expect_err("retry must reject the purging plan");
    assert!(
        retry_error.contains("entity_deleting"),
        "retry rejection must carry the stable deletion code: {retry_error}"
    );
    assert!(
        AdvanceStore::new(app_paths.clone())
            .get_retry_initialization(PROJECT_ID, ISSUE_ID, "retry_d21d_0001")
            .unwrap()
            .is_none(),
        "rejected retry must leave zero retry facts"
    );

    // 零新增：advance record 仍是原一条，coding attempts 面零新建。
    assert_eq!(
        AdvanceStore::new(app_paths.clone())
            .list_plan_artifacts(PROJECT_ID, ISSUE_ID, PLAN_ID)
            .unwrap()
            .advance_ids
            .len(),
        1,
        "rejected entries must not add advance records"
    );
    assert!(
        crate::product::coding_attempt_store::CodingAttemptStore::new(app_paths.clone())
            .list_attempts_for_issue(PROJECT_ID, ISSUE_ID)
            .unwrap()
            .is_empty(),
        "rejected entries must leave zero coding attempts"
    );
    // Ready 判定 false（删除事实优先，D1.2c 面在此一并钉住）。
    assert!(
        !AdvanceStore::new(app_paths.clone())
            .advance_is_ready_for_attempt(PROJECT_ID, ISSUE_ID, PLAN_ID, "attempt_advance")
            .unwrap()
    );

    // (c) Deleted 完成态同样稳定拒绝（不复活旧轮）。
    let delete_lease = operations
        .try_acquire_delete(&scope)
        .expect("purging reentry lease");
    deletions
        .complete(&delete_lease)
        .expect("complete tombstone");
    drop(delete_lease);
    let outcome = engine.handle_advance(input(COMMAND_ID)).await.unwrap();
    match &outcome {
        AdvanceOutcome::Rejected { code, .. } => assert_eq!(code, "ADVANCE_PLAN_DELETED"),
        other => panic!("deleted tombstone must reject, got {other:?}"),
    }
}

// ─── D2.1d＋H3：执行轮退役围栏先于旧 Ready 回放（不回放旧 attempt）───
//
// 设计 2026-10-14 A/H3 节：clearing 窗口内任何 advance 入口拒绝；轮次切换
// 完成后退役 command（旧 Ready record 的原 command）不再回放；新 command
// 不被退役围栏拦截，沿正常前置检查进入新轮。
#[tokio::test]
async fn cascade_d2_1d_retired_round_fences_old_replay_and_new_round() {
    use crate::product::coding_plan_run::{CodingPlanRunStore, CodingPlanWorkspaceResetChecklist};
    use crate::product::entity_deletion_store::{DeletionEntityKind, EntityScope};
    use crate::product::entity_operation_store::EntityOperationStore;

    let root = tempfile::tempdir().unwrap();
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let store = AdvanceStore::new(app_paths.clone());
    store.put_record(&stored_record(COMMAND_ID)).unwrap();
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let operations = EntityOperationStore::new(app_paths.clone());
    let run_store = CodingPlanRunStore::new(app_paths.clone());
    let scope = EntityScope::new(PROJECT_ID, ISSUE_ID, DeletionEntityKind::Plan, PLAN_ID);

    let checklist = CodingPlanWorkspaceResetChecklist {
        command_id: "close_d21d_0001".to_string(),
        retired_command_ids: vec![COMMAND_ID.to_string()],
        retired_attempt_ids: vec!["attempt_advance".to_string()],
        retired_advance_ids: vec![format!("advance_{COMMAND_ID}")],
        pending_attempt_ids: Vec::new(),
        pending_advance_ids: Vec::new(),
        pending_retry_command_ids: Vec::new(),
        pending_target_repository_ids: Vec::new(),
    };
    let delete_lease = operations.try_acquire_delete(&scope).expect("retire lease");
    run_store
        .retire_workspace_round_with_lease(&delete_lease, &checklist)
        .expect("freeze clearing fence");
    drop(delete_lease);

    // clearing 窗口：旧 command 与新 command 都拒绝，不回放旧 Ready。
    let mut engine = engine_fixture(&root, lifecycle);
    for command in [COMMAND_ID, "command_d21d_next_round"] {
        let outcome = engine.handle_advance(input(command)).await.unwrap();
        match &outcome {
            AdvanceOutcome::Rejected { code, reason, .. } => {
                assert_eq!(
                    code, "ADVANCE_PLAN_WORKSPACE_CLEARING",
                    "{command}: {reason}"
                );
            }
            other => panic!("clearing window must reject {command}, got {other:?}"),
        }
    }

    // 轮次切换收口（clearing 清空、退役事实保留）：旧 command 拒绝且不
    // 回放；新 command 穿透围栏进入正常前置（本 fixture 无 plan 记录 →
    // 原 ADVANCE_PLAN_NOT_FOUND，证围栏未扩大化）。
    let delete_lease = operations
        .try_acquire_delete(&scope)
        .expect("complete lease");
    run_store
        .complete_workspace_reset_with_lease(&delete_lease, "close_d21d_0001")
        .expect("complete the round switch");
    drop(delete_lease);

    let outcome = engine.handle_advance(input(COMMAND_ID)).await.unwrap();
    match &outcome {
        AdvanceOutcome::Rejected { code, reason, .. } => {
            assert_eq!(code, "ADVANCE_PLAN_RUN_RETIRED", "{reason}");
        }
        other => panic!("retired command must not replay, got {other:?}"),
    }
    let outcome = engine
        .handle_advance(input("command_d21d_next_round"))
        .await
        .unwrap();
    match &outcome {
        AdvanceOutcome::Replayed { record } => {
            assert_eq!(record.command_id, COMMAND_ID);
        }
        other => panic!(
            "new command must pass the fence into the normal plan-level replay path, got {other:?}"
        ),
    }
}
