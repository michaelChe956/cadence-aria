use super::*;
use crate::product::advance_store::{AdvanceRecord, AdvanceStatus, AdvanceStore};
use crate::product::models::{PlanRevisionReason, WorkItemPlanLineage, WorkItemPlanRevision};

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
            reviewer_provider: ProviderName::Fake,
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

fn active_lineage(lifecycle: &LifecycleStore) {
    let revision_store =
        crate::product::work_item_revision_store::WorkItemRevisionStore::new(lifecycle.app_paths());
    let lineage = WorkItemPlanLineage {
        id: PLAN_ID.to_string(),
        project_id: PROJECT_ID.to_string(),
        issue_id: ISSUE_ID.to_string(),
        story_spec_refs: Vec::new(),
        design_spec_refs: Vec::new(),
        active_revision_id: Some("revision_advance".to_string()),
        active_amendment_id: None,
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

fn input(command_id: &str) -> AdvanceInput {
    AdvanceInput {
        command_id: command_id.to_string(),
        project_id: PROJECT_ID.to_string(),
        issue_id: ISSUE_ID.to_string(),
        plan_id: PLAN_ID.to_string(),
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
        status: AdvanceStatus::Ready,
        workspace_entry: Some("/workspace/attempt_advance".to_string()),
        error: None,
        created_at: "2026-08-31T00:00:00Z".to_string(),
        updated_at: "2026-08-31T00:00:01Z".to_string(),
    }
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
    active_lineage(&lifecycle);
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
    active_lineage(&lifecycle);
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
