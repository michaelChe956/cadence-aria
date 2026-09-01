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
        .get_attempt_for_work_item_group("project_0001", "issue_plan_0001", "work_item_plan_0001")
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
        .get_attempt_for_work_item_group("project_0001", "issue_plan_0001", "work_item_plan_0001")
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
        },
    );
    assert!(matches!(
        response,
        crate::web::workspace_ws_types::WsOutMessage::AdvanceCompleted {
            command_id,
            attempt_id: response_attempt_id,
            workspace_entry: response_workspace_entry,
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
#[tokio::test]
async fn advance_checkpoint_recovery_reuses_prepared_attempt_and_unit_ids() {
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
        .get_attempt_for_work_item_group("project_0001", "issue_plan_0001", "work_item_plan_0001")
        .unwrap()
        .expect("seeded group attempt");
    coding_store
        .delete_attempt("project_0001", "issue_plan_0001", &seeded_attempt.id)
        .unwrap();
    seed_advance_draft_records(&app_paths);
    let authoritative = coding_store
        .resolve_authoritative_group_plan_binding_for_revision(
            "project_0001",
            "issue_plan_0001",
            "work_item_plan_0001",
            "plan_revision_0001",
        )
        .unwrap();
    let repository = RepositoryStore::new(app_paths.clone())
        .list("project_0001")
        .unwrap()
        .into_iter()
        .next()
        .expect("fixture repository");
    let group = coding_store
        .prepare_group_initialization_with_admission(
            &CreateGroupCodingAttemptInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_plan_0001".to_string(),
                plan_id: "work_item_plan_0001".to_string(),
                current_work_item_id: authoritative.units[0].logical_work_item_id.clone(),
                base_branch: "HEAD".to_string(),
                branch_name: "aria/issues/issue_plan_0001".to_string(),
                worktree_path: Some(root.path().join("worktree")),
                provider_config_snapshot: ProviderConfigSnapshot {
                    author: ProviderName::Codex,
                    reviewer: Some(ProviderName::ClaudeCode),
                    review_rounds: 1,
                    permission_modes: Default::default(),
                },
                target_snapshot: None,
                max_auto_rework: 2,
            },
            "plan_revision_0001",
            &authoritative.units,
            crate::product::coding_models::CodingAdmissionKind::ScAdvance,
        )
        .unwrap();
    let expected_attempt_id = group.attempt.id.clone();
    let expected_unit_ids = group
        .units
        .iter()
        .map(|unit| unit.id.clone())
        .collect::<Vec<_>>();
    let expected_lease_id = group.worktree_lease_id.clone();
    let lifecycle = LifecycleStore::new(app_paths.clone());
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
    let mut engine = WorkspaceEngine::new_persistent(
        Arc::new(CheckpointStore::new(root.path().join("checkpoints"))),
        lifecycle.clone(),
        event_tx,
        WorkspaceSession::from_record(session_record),
    );
    let outcome = engine
        .handle_advance(AdvanceInput {
            command_id: "command_checkpoint_prepared".to_string(),
            project_id: "project_0001".to_string(),
            issue_id: "issue_plan_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
        })
        .await
        .expect("resume prepared checkpoint");
    assert!(matches!(outcome, AdvanceOutcome::Completed { .. }));
    let persisted_attempts = coding_store
        .list_attempts_for_issue("project_0001", "issue_plan_0001")
        .unwrap();
    assert_eq!(persisted_attempts.len(), 1);
    assert_eq!(persisted_attempts[0].id, expected_attempt_id);
    assert_eq!(
        persisted_attempts[0].admission_kind,
        crate::product::coding_models::CodingAdmissionKind::ScAdvance
    );
    let persisted_units = coding_store
        .list_coding_units("project_0001", "issue_plan_0001", &expected_attempt_id)
        .unwrap();
    assert_eq!(
        persisted_units
            .iter()
            .map(|unit| unit.id.clone())
            .collect::<Vec<_>>(),
        expected_unit_ids
    );
    assert_eq!(
        coding_store
            .get_group_initialization("project_0001", "issue_plan_0001", "work_item_plan_0001")
            .unwrap()
            .worktree_lease_id,
        expected_lease_id
    );
    let replay = engine
        .handle_advance(AdvanceInput {
            command_id: "command_checkpoint_prepared_replay".to_string(),
            project_id: "project_0001".to_string(),
            issue_id: "issue_plan_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
        })
        .await
        .expect("replay completed advance");
    assert!(
        matches!(replay, AdvanceOutcome::Replayed { record } if record.status == AdvanceStatus::Ready)
    );
    assert_eq!(
        coding_store
            .list_attempts_for_issue("project_0001", "issue_plan_0001")
            .unwrap()
            .len(),
        1
    );
    assert_eq!(repository.id, "repository_0001");
}

#[tokio::test]
async fn advance_checkpoint_recovery_reuses_materialized_units_without_duplicates() {
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
        .get_attempt_for_work_item_group("project_0001", "issue_plan_0001", "work_item_plan_0001")
        .unwrap()
        .expect("seeded group attempt");
    coding_store
        .delete_attempt("project_0001", "issue_plan_0001", &seeded_attempt.id)
        .unwrap();
    seed_advance_draft_records(&app_paths);
    let authoritative = coding_store
        .resolve_authoritative_group_plan_binding_for_revision(
            "project_0001",
            "issue_plan_0001",
            "work_item_plan_0001",
            "plan_revision_0001",
        )
        .unwrap();
    let worktree_path = root.path().join("worktree");
    let group_input = CreateGroupCodingAttemptInput {
        project_id: "project_0001".to_string(),
        issue_id: "issue_plan_0001".to_string(),
        plan_id: "work_item_plan_0001".to_string(),
        current_work_item_id: authoritative.units[0].logical_work_item_id.clone(),
        base_branch: "HEAD".to_string(),
        branch_name: "aria/issues/issue_plan_0001".to_string(),
        worktree_path: Some(worktree_path.clone()),
        provider_config_snapshot: ProviderConfigSnapshot {
            author: ProviderName::Codex,
            reviewer: Some(ProviderName::ClaudeCode),
            review_rounds: 1,
            permission_modes: Default::default(),
        },
        target_snapshot: None,
        max_auto_rework: 2,
    };
    let mut group = coding_store
        .prepare_group_initialization_with_admission(
            &group_input,
            "plan_revision_0001",
            &authoritative.units,
            crate::product::coding_models::CodingAdmissionKind::ScAdvance,
        )
        .unwrap();
    let guard = coding_store
        .acquire_work_item_attempt_creation(
            "project_0001",
            "issue_plan_0001",
            &group.lock_work_item_id,
        )
        .unwrap();
    coding_store
        .ensure_group_initialization_attempt(&group, &guard)
        .unwrap();
    group = coding_store
        .advance_group_initialization_phase(
            &group,
            crate::product::coding_attempt_store::CodingGroupInitializationPhase::AttemptPersisted,
        )
        .unwrap();
    let lifecycle = LifecycleStore::new(app_paths.clone());
    lifecycle
        .upsert_issue_shared_worktree(
            crate::product::lifecycle_store::UpsertIssueSharedWorktreeInput {
                project_id: group.project_id.clone(),
                issue_id: group.issue_id.clone(),
                repository_id: "repository_0001".to_string(),
                branch_name: group.attempt.branch_name.clone(),
                worktree_path: worktree_path.clone(),
                base_branch: group.attempt.base_branch.clone(),
            },
        )
        .unwrap();
    let lease = lifecycle
        .try_acquire_issue_worktree_lock(
            &group.project_id,
            &group.issue_id,
            &group.lock_work_item_id,
            &group.worktree_lease_id,
        )
        .unwrap();
    assert!(lease.acquired);
    lifecycle
        .bind_issue_worktree_lock_to_attempt(
            &group.project_id,
            &group.issue_id,
            &group.lock_work_item_id,
            &group.attempt.id,
        )
        .unwrap();
    group = coding_store
        .advance_group_initialization_phase(
            &group,
            crate::product::coding_attempt_store::CodingGroupInitializationPhase::WorktreeBound,
        )
        .unwrap();
    coding_store
        .ensure_group_initialization_plan_binding(&group)
        .unwrap();
    group = coding_store
        .advance_group_initialization_phase(
            &group,
            crate::product::coding_attempt_store::CodingGroupInitializationPhase::PlanBindingSaved,
        )
        .unwrap();
    for index in 0..group.units.len() {
        coding_store
            .ensure_group_initialization_unit(&group, index)
            .unwrap();
    }
    group = coding_store
        .advance_group_initialization_phase(
            &group,
            crate::product::coding_attempt_store::CodingGroupInitializationPhase::UnitsMaterialized,
        )
        .unwrap();
    let expected_attempt_id = group.attempt.id.clone();
    let expected_unit_ids = group
        .units
        .iter()
        .map(|unit| unit.id.clone())
        .collect::<Vec<_>>();
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
    let mut engine = WorkspaceEngine::new_persistent(
        Arc::new(CheckpointStore::new(root.path().join("checkpoints"))),
        lifecycle,
        event_tx,
        WorkspaceSession::from_record(session_record),
    );
    let outcome = engine
        .handle_advance(AdvanceInput {
            command_id: "command_checkpoint_materialized".to_string(),
            project_id: "project_0001".to_string(),
            issue_id: "issue_plan_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
        })
        .await
        .expect("resume materialized checkpoint");
    assert!(matches!(outcome, AdvanceOutcome::Completed { .. }));
    assert_eq!(
        coding_store
            .list_attempts_for_issue("project_0001", "issue_plan_0001")
            .unwrap()
            .iter()
            .map(|attempt| attempt.id.clone())
            .collect::<Vec<_>>(),
        vec![expected_attempt_id.clone()]
    );
    assert_eq!(
        coding_store
            .list_coding_units("project_0001", "issue_plan_0001", &expected_attempt_id)
            .unwrap()
            .iter()
            .map(|unit| unit.id.clone())
            .collect::<Vec<_>>(),
        expected_unit_ids
    );
}

#[tokio::test]
async fn advance_initialization_replay_resumes_same_record_attempt_and_units() {
    let checkpoints = [
        AdvanceInitializationFailpoint::RecordPersisted,
        AdvanceInitializationFailpoint::JournalPrepared,
        AdvanceInitializationFailpoint::AttemptPersisted,
        AdvanceInitializationFailpoint::WorktreeBound,
        AdvanceInitializationFailpoint::PlanBindingSaved,
        AdvanceInitializationFailpoint::UnitsMaterialized,
    ];
    for checkpoint in checkpoints {
        let (root, app_paths, lifecycle, mut engine) = advance_fixture().await;
        let command_id = format!("command_failpoint_{checkpoint:?}");
        let request = fixture_input(&command_id);
        let _failpoint = register_advance_initialization_failpoint(
            &request,
            checkpoint,
            AdvanceInitializationFailpointMode::Crash,
        );
        let first = tokio::spawn(async move { engine.handle_advance(request).await });
        assert!(
            first.await.is_err(),
            "{checkpoint:?} must interrupt the engine"
        );

        let advance_store = AdvanceStore::new(app_paths.clone());
        let first_record = advance_store
            .get_advance_by_command_id("project_0001", "issue_plan_0001", &command_id)
            .unwrap()
            .expect("record is durable before every checkpoint");
        let first_outer = advance_store
            .get_advance_initialization(&first_record)
            .unwrap();
        let coding_store =
            crate::product::coding_attempt_store::CodingAttemptStore::new(app_paths.clone());
        let first_group = coding_store
            .get_group_initialization("project_0001", "issue_plan_0001", "work_item_plan_0001")
            .ok();
        let first_attempt_id = first_group.as_ref().map(|group| group.attempt.id.clone());
        let first_unit_ids = first_group.as_ref().map(|group| {
            group
                .units
                .iter()
                .map(|unit| unit.id.clone())
                .collect::<Vec<_>>()
        });
        match checkpoint {
            AdvanceInitializationFailpoint::RecordPersisted => {
                assert!(first_outer.is_none());
                assert!(first_group.is_none());
            }
            AdvanceInitializationFailpoint::JournalPrepared => {
                assert_eq!(
                    first_outer.as_ref().map(|journal| journal.phase),
                    Some(AdvanceInitializationPhase::JournalPrepared)
                );
                assert_eq!(
                    first_group.as_ref().map(|group| group.phase),
                    Some(crate::product::coding_attempt_store::CodingGroupInitializationPhase::Prepared)
                );
            }
            AdvanceInitializationFailpoint::AttemptPersisted => {
                assert_eq!(
                    first_outer.as_ref().map(|journal| journal.phase),
                    Some(AdvanceInitializationPhase::JournalPrepared)
                );
                assert_eq!(
                    first_group.as_ref().map(|group| group.phase),
                    Some(crate::product::coding_attempt_store::CodingGroupInitializationPhase::AttemptPersisted)
                );
            }
            AdvanceInitializationFailpoint::WorktreeBound => {
                assert_eq!(
                    first_outer.as_ref().map(|journal| journal.phase),
                    Some(AdvanceInitializationPhase::AttemptPersisted)
                );
                assert_eq!(
                    first_group.as_ref().map(|group| group.phase),
                    Some(crate::product::coding_attempt_store::CodingGroupInitializationPhase::WorktreeBound)
                );
            }
            AdvanceInitializationFailpoint::PlanBindingSaved => {
                assert_eq!(
                    first_outer.as_ref().map(|journal| journal.phase),
                    Some(AdvanceInitializationPhase::PlanBindingSaved)
                );
                assert_eq!(
                    first_group.as_ref().map(|group| group.phase),
                    Some(crate::product::coding_attempt_store::CodingGroupInitializationPhase::PlanBindingSaved)
                );
            }
            AdvanceInitializationFailpoint::UnitsMaterialized => {
                assert_eq!(
                    first_outer.as_ref().map(|journal| journal.phase),
                    Some(AdvanceInitializationPhase::UnitsMaterialized)
                );
                assert_eq!(
                    first_group.as_ref().map(|group| group.phase),
                    Some(crate::product::coding_attempt_store::CodingGroupInitializationPhase::UnitsMaterialized)
                );
            }
            AdvanceInitializationFailpoint::GroupAttemptPersisted => unreachable!(),
        }

        let mut restarted = build_advance_engine(&root, lifecycle);
        let outcome = restarted
            .handle_advance(fixture_input(&command_id))
            .await
            .expect("restart must resume the interrupted initialization");
        assert!(matches!(outcome, AdvanceOutcome::Completed { .. }));
        let final_record = advance_store
            .get_advance_by_command_id("project_0001", "issue_plan_0001", &command_id)
            .unwrap()
            .expect("final record");
        assert_eq!(final_record.id, first_record.id);
        let final_group = coding_store
            .get_group_initialization("project_0001", "issue_plan_0001", "work_item_plan_0001")
            .unwrap();
        if let Some(first_attempt_id) = first_attempt_id {
            assert_eq!(final_group.attempt.id, first_attempt_id);
        }
        if let Some(first_unit_ids) = first_unit_ids {
            assert_eq!(
                final_group
                    .units
                    .iter()
                    .map(|unit| unit.id.clone())
                    .collect::<Vec<_>>(),
                first_unit_ids
            );
        }
        assert_eq!(
            coding_store
                .list_attempts_for_issue("project_0001", "issue_plan_0001")
                .unwrap()
                .len(),
            1
        );
        let replay = restarted
            .handle_advance(fixture_input(&format!("{command_id}_replay")))
            .await
            .expect("completed advance replay");
        assert!(matches!(
            replay,
            AdvanceOutcome::Replayed { record } if record.status == AdvanceStatus::Ready
        ));
    }
}

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
