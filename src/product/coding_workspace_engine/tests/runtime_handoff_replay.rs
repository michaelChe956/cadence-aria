fn runtime_registration_unit(
    fixture: &RuntimeHandoffFixture,
) -> crate::product::coding_models::CodingExecutionUnit {
    fixture
        .store
        .list_coding_units(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .unwrap()
        .into_iter()
        .find(|unit| unit.logical_work_item_id == "wi_registration")
        .unwrap()
}

fn set_runtime_handoff_pointer(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    logical_work_item_id: &str,
    handoff_id: &str,
) {
    let source = store
        .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .unwrap()
        .into_iter()
        .find(|unit| unit.logical_work_item_id == logical_work_item_id)
        .unwrap();
    store
        .update_coding_unit_latest_handoff_revision_id(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &source.id,
            Some(handoff_id.to_string()),
        )
        .unwrap();
}

fn seed_runtime_source_handoff_runs(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
) {
    let unit = store
        .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .unwrap()
        .into_iter()
        .find(|unit| unit.logical_work_item_id == "wi_core")
        .unwrap();
    let revision_store = WorkItemRevisionStore::new(store.paths());
    let lineage = revision_store
        .get_plan_lineage(&attempt.project_id, &attempt.issue_id, "work_item_plan_0001")
        .unwrap();
    let revision = revision_store
        .get_work_item_revision(&lineage, "wi_core", &unit.work_item_revision_id)
        .unwrap();
    let bundle = revision_store
        .get_work_item_projection_bundle(&lineage, &revision.work_item_projection_bundle_id)
        .unwrap();
    let providers = store
        .get_role_provider_config_snapshot(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .unwrap();
    for (suffix, execution_no) in [("0001", 1), ("0002", 2)] {
        store
            .create_coding_unit_run(
                attempt,
                &CodingUnitRun {
                    id: format!("coding_unit_run_handoff_revision_{suffix}"),
                    unit_id: unit.id.clone(),
                    execution_no,
                    work_item_revision_id: revision.id.clone(),
                    resolved_handoff_revision_ids: Vec::new(),
                    canonical_contract_hash: revision.canonical_contract_hash.clone(),
                    projection_bundle_id: bundle.id.clone(),
                    projection_compiler_version: bundle.compiler_version.clone(),
                    coder_provider_renderer_version:
                        crate::product::work_item_projection::renderer_for(&providers.coder)
                        .renderer_version()
                        .to_string(),
                    reviewer_provider_renderer_version:
                        crate::product::work_item_projection::renderer_for(
                            &providers.code_reviewer,
                        )
                        .renderer_version()
                        .to_string(),
                    internal_reviewer_provider_renderer_version: None,
                    coder_projection_hash: bundle.coder_projection_hash.clone(),
                    reviewer_projection_hash: bundle.reviewer_projection_hash.clone(),
                    coder_execution_context_hash: None,
                    reviewer_execution_context_hash: None,
                    internal_reviewer_execution_context_hash: None,
                    status: CodingUnitRunStatus::Completed,
                    unit_rework_count: 0,
                    verification_retry_count: 0,
                    operational_retry_count: 0,
                    plan_repair_count: 1,
                    start_commit: Some(format!("commit_handoff_revision_{suffix}")),
                    completion_commit: Some(format!("commit_handoff_revision_{suffix}")),
                    created_at: format!("2026-07-20T00:00:0{execution_no}Z"),
                    updated_at: format!("2026-07-20T00:00:0{execution_no}Z"),
                },
            )
            .unwrap();
    }
}

fn create_runtime_authority_other_attempt_run(
    fixture: &RuntimeHandoffFixture,
    template: &CodingUnitRun,
) -> CodingUnitRun {
    let other = fixture
        .store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: fixture.attempt.project_id.clone(),
            issue_id: "issue_other".to_string(),
            plan_id: "work_item_plan_other".to_string(),
            current_work_item_id: "wi_core".to_string(),
            base_branch: "HEAD".to_string(),
            branch_name: "aria/issues/issue_other".to_string(),
            worktree_path: None,
            provider_config_snapshot: fixture.attempt.provider_config_snapshot.clone(),
            max_auto_rework: 2,
        })
        .unwrap();
    let unit = fixture
        .store
        .create_coding_unit(CreateCodingExecutionUnitInput {
            attempt_id: other.id.clone(),
            project_id: other.project_id.clone(),
            issue_id: other.issue_id.clone(),
            plan_id: "work_item_plan_other".to_string(),
            logical_work_item_id: "wi_core".to_string(),
            work_item_revision_id: template.work_item_revision_id.clone(),
            dependency_logical_work_item_ids: Vec::new(),
            order_index: 0,
            status: CodingExecutionUnitStatus::Completed,
        })
        .unwrap();
    let mut run = template.clone();
    run.id = "coding_unit_run_other_attempt".to_string();
    run.unit_id = unit.id;
    run.execution_no = 1;
    fixture.store.create_coding_unit_run(&other, &run).unwrap();
    run
}

async fn assert_runtime_handoff_authority_zero_write(
    fixture: &RuntimeHandoffFixture,
    next_handoff: &HandoffRevision,
) {
    let units_before = fixture
        .store
        .list_coding_units(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .unwrap();
    let runs_before = units_before
        .iter()
        .map(|unit| {
            fixture
                .store
                .list_coding_unit_runs(&fixture.attempt, &unit.id)
                .unwrap()
        })
        .collect::<Vec<_>>();

    let error = fixture
        .engine
        .apply_completed_handoff(&fixture.attempt, next_handoff)
        .await
        .expect_err("forged Handoff must fail closed");

    assert!(
        error
            .to_string()
            .contains("runtime_handoff_authority_conflict"),
        "{error}"
    );
    assert_eq!(
        fixture
            .store
            .list_coding_units(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id,
            )
            .unwrap(),
        units_before
    );
    assert_eq!(
        units_before
            .iter()
            .map(|unit| {
                fixture
                    .store
                    .list_coding_unit_runs(&fixture.attempt, &unit.id)
                    .unwrap()
            })
            .collect::<Vec<_>>(),
        runs_before
    );
}

#[tokio::test]
async fn coding_runtime_handoff_authority_forged_next_is_zero_write() {
    for case in [
        "unknown_run",
        "other_attempt",
        "wrong_unit",
        "wrong_revision",
        "non_completed",
        "commit_mismatch",
    ] {
        let fixture = runtime_handoff_fixture(
            RuntimeContractChange::CompatibleExtension,
            CodingUnitRunStatus::AwaitingAmendment,
        );
        let source_runs = fixture
            .store
            .list_unit_runs_by_logical_id(&fixture.attempt, "wi_core")
            .unwrap();
        let authoritative = source_runs.last().unwrap().clone();
        let other_attempt = (case == "other_attempt")
            .then(|| create_runtime_authority_other_attempt_run(&fixture, &authoritative));
        let non_completed = (case == "non_completed").then(|| {
            let mut run = authoritative.clone();
            run.id = "coding_unit_run_non_completed".to_string();
            run.execution_no = 3;
            run.status = CodingUnitRunStatus::Failed;
            run.completion_commit = None;
            fixture
                .store
                .create_coding_unit_run(&fixture.attempt, &run)
                .unwrap();
            run
        });
        let mut forged = handoff(
            &format!("handoff_revision_authority_{case}"),
            &["registration_contract"],
            &[("registration_contract", &["registration_ready"])],
            "authority_hash",
        );
        forged.coding_unit_run_id = match case {
            "unknown_run" => "coding_unit_run_unknown".to_string(),
            "other_attempt" => other_attempt.as_ref().unwrap().id.clone(),
            "wrong_unit" => "coding_unit_run_registration_0001".to_string(),
            "non_completed" => non_completed.as_ref().unwrap().id.clone(),
            _ => authoritative.id.clone(),
        };
        forged.work_item_revision_id = if case == "wrong_revision" {
            "work_item_revision_wrong".to_string()
        } else {
            authoritative.work_item_revision_id.clone()
        };
        forged.commit_sha = if case == "commit_mismatch" {
            "commit_mismatch".to_string()
        } else if case == "other_attempt" {
            other_attempt
                .as_ref()
                .unwrap()
                .completion_commit
                .clone()
                .unwrap()
        } else if case == "non_completed" {
            "commit_non_completed".to_string()
        } else {
            authoritative.completion_commit.clone().unwrap()
        };
        forged.created_at = "2026-07-21T00:00:00Z".to_string();
        let revision_store = WorkItemRevisionStore::new(fixture.store.paths());
        let lineage = revision_store
            .get_plan_lineage(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                "work_item_plan_0001",
            )
            .unwrap();
        revision_store
            .put_handoff_revision(&lineage, &forged)
            .unwrap();

        assert_runtime_handoff_authority_zero_write(&fixture, &forged).await;
    }
}

fn append_runtime_source_handoff(
    fixture: &RuntimeHandoffFixture,
    id: &str,
    execution_no: u32,
    created_at: &str,
) -> HandoffRevision {
    let mut run = fixture
        .store
        .list_unit_runs_by_logical_id(&fixture.attempt, "wi_core")
        .unwrap()
        .into_iter()
        .max_by_key(|run| run.execution_no)
        .unwrap();
    run.id = format!("coding_unit_run_{id}");
    run.execution_no = execution_no;
    run.start_commit = Some(format!("commit_{id}"));
    run.completion_commit = Some(format!("commit_{id}"));
    run.created_at = created_at.to_string();
    run.updated_at = created_at.to_string();
    fixture
        .store
        .create_coding_unit_run(&fixture.attempt, &run)
        .unwrap();
    let mut handoff = fixture.next_handoff.clone();
    handoff.id = id.to_string();
    handoff.coding_unit_run_id = run.id;
    handoff.commit_sha = run.completion_commit.unwrap();
    handoff.created_at = created_at.to_string();
    let revision_store = WorkItemRevisionStore::new(fixture.store.paths());
    let lineage = revision_store
        .get_plan_lineage(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            "work_item_plan_0001",
        )
        .unwrap();
    revision_store
        .put_handoff_revision(&lineage, &handoff)
        .unwrap();
    set_runtime_handoff_pointer(
        &fixture.store,
        &fixture.attempt,
        "wi_core",
        &handoff.id,
    );
    handoff
}

#[tokio::test]
async fn coding_runtime_handoff_exact_previous_ignores_orphan_handoff() {
    let fixture = runtime_handoff_fixture(
        RuntimeContractChange::BreakingChange,
        CodingUnitRunStatus::AwaitingAmendment,
    );
    let mut orphan = fixture.next_handoff.clone();
    orphan.id = "handoff_revision_orphan".to_string();
    orphan.coding_unit_run_id = "coding_unit_run_orphan".to_string();
    orphan.created_at = "2026-07-23T00:00:00Z".to_string();
    let revision_store = WorkItemRevisionStore::new(fixture.store.paths());
    let lineage = revision_store
        .get_plan_lineage(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            "work_item_plan_0001",
        )
        .unwrap();
    revision_store
        .put_handoff_revision(&lineage, &orphan)
        .unwrap();

    let result = fixture
        .engine
        .apply_completed_handoff(&fixture.attempt, &fixture.next_handoff)
        .await
        .unwrap();

    assert_eq!(result.newly_stale_units, vec!["wi_registration"]);
}

#[tokio::test]
async fn coding_runtime_handoff_exact_previous_uses_execution_order_not_created_at() {
    let fixture = runtime_handoff_fixture(
        RuntimeContractChange::CompatibleExtension,
        CodingUnitRunStatus::NeedsRevalidation,
    );
    let next = append_runtime_source_handoff(
        &fixture,
        "handoff_revision_0003",
        3,
        "2026-07-19T00:00:00Z",
    );

    let result = fixture
        .engine
        .apply_completed_handoff(&fixture.attempt, &next)
        .await
        .unwrap();

    assert!(result.resumed_units.is_empty());
    assert!(result.revalidation_units.is_empty());
    assert!(result.newly_stale_units.is_empty());
    assert!(result.conditional_units_released.is_empty());
    assert_eq!(result.propagation_stopped_at, Some("wi_core".to_string()));
}

#[tokio::test]
async fn coding_runtime_handoff_exact_previous_alias_ambiguity_is_zero_write() {
    let fixture = runtime_handoff_fixture(
        RuntimeContractChange::CompatibleExtension,
        CodingUnitRunStatus::AwaitingAmendment,
    );
    let revision_store = WorkItemRevisionStore::new(fixture.store.paths());
    let lineage = revision_store
        .get_plan_lineage(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            "work_item_plan_0001",
        )
        .unwrap();
    let mut alias = revision_store
        .get_handoff_revision(&lineage, "wi_core", "handoff_revision_0001")
        .unwrap();
    alias.id = "handoff_revision_previous_alias".to_string();
    alias.created_at = "2026-07-23T00:00:00Z".to_string();
    revision_store
        .put_handoff_revision(&lineage, &alias)
        .unwrap();

    assert_runtime_handoff_authority_zero_write(&fixture, &fixture.next_handoff).await;
}

#[tokio::test]
async fn coding_runtime_handoff_explicit_revalidation_precedes_resume_status() {
    for waiting_status in [
        CodingUnitRunStatus::AwaitingAmendment,
        CodingUnitRunStatus::Pending,
    ] {
        let fixture = runtime_handoff_fixture(
            RuntimeContractChange::CompatibleExtension,
            waiting_status,
        );

        let result = fixture
            .engine
            .apply_completed_handoff(&fixture.attempt, &fixture.next_handoff)
            .await
            .unwrap();

        assert!(result.resumed_units.is_empty());
        assert_eq!(result.revalidation_units, vec!["wi_registration"]);
        let run = fixture
            .store
            .list_unit_runs_by_logical_id(&fixture.attempt, "wi_registration")
            .unwrap()
            .pop()
            .unwrap();
        assert_eq!(run.status, CodingUnitRunStatus::NeedsRevalidation);
    }
}

fn resolve_runtime_registration(
    fixture: &RuntimeHandoffFixture,
    resolved_handoff_revision_ids: &[String],
    status: CodingUnitRunStatus,
) -> CodingUnitRun {
    fixture
        .store
        .resolve_runtime_handoff_unit_run(
            &fixture.attempt,
            "plan_amendment_0001",
            "wi_registration",
            resolved_handoff_revision_ids,
            status,
        )
        .unwrap()
}

fn complete_runtime_registration_placeholder(
    fixture: &RuntimeHandoffFixture,
    resolved_handoff_revision_ids: &[String],
) -> CodingUnitRun {
    let pending = resolve_runtime_registration(
        fixture,
        resolved_handoff_revision_ids,
        CodingUnitRunStatus::Pending,
    );
    let unit = runtime_registration_unit(fixture);
    fixture
        .store
        .start_pending_coding_unit_run(&fixture.attempt, &unit.id)
        .unwrap();
    fixture
        .store
        .update_coding_unit_status(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
            &unit.id,
            CodingExecutionUnitStatus::Running,
            Some("Runtime Handoff execution started".to_string()),
        )
        .unwrap();
    let completed = fixture
        .store
        .complete_coding_unit_run(
            &fixture.attempt,
            &pending.id,
            "commit_registration_runtime_complete",
        )
        .unwrap();
    fixture
        .store
        .update_coding_unit_completion_commit(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
            &unit.id,
            completed.completion_commit.clone(),
        )
        .unwrap();
    fixture
        .store
        .update_coding_unit_status(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
            &unit.id,
            CodingExecutionUnitStatus::Completed,
            Some("Runtime Handoff execution completed".to_string()),
        )
        .unwrap();
    completed
}

#[test]
fn coding_runtime_handoff_placeholder_unresolved_resolves_in_place() {
    let fixture = runtime_handoff_fixture(
        RuntimeContractChange::CompatibleExtension,
        CodingUnitRunStatus::AwaitingAmendment,
    );
    let unit = runtime_registration_unit(&fixture);
    let placeholder_id = format!("coding_unit_run_{}_plan_amendment_0001", unit.id);

    let resolved = resolve_runtime_registration(
        &fixture,
        &["handoff_revision_0002".to_string()],
        CodingUnitRunStatus::Pending,
    );

    assert_eq!(resolved.id, placeholder_id);
    assert_eq!(resolved.execution_no, 2);
    assert_eq!(resolved.status, CodingUnitRunStatus::Pending);
    assert_eq!(
        resolved.resolved_handoff_revision_ids,
        vec!["handoff_revision_0002"]
    );
    assert_eq!(runtime_registration_unit(&fixture).status, CodingExecutionUnitStatus::Pending);
}

#[test]
fn coding_runtime_handoff_placeholder_same_tuple_running_replay_is_unchanged() {
    let fixture = runtime_handoff_fixture(
        RuntimeContractChange::CompatibleExtension,
        CodingUnitRunStatus::AwaitingAmendment,
    );
    let handoffs = vec!["handoff_revision_0002".to_string()];
    let pending = resolve_runtime_registration(&fixture, &handoffs, CodingUnitRunStatus::Pending);
    let unit = runtime_registration_unit(&fixture);
    fixture
        .store
        .start_pending_coding_unit_run(&fixture.attempt, &unit.id)
        .unwrap();
    let running_unit = fixture
        .store
        .update_coding_unit_status(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
            &unit.id,
            CodingExecutionUnitStatus::Running,
            Some("Runtime Handoff execution started".to_string()),
        )
        .unwrap();
    let running = fixture
        .store
        .list_coding_unit_runs(&fixture.attempt, &unit.id)
        .unwrap()
        .into_iter()
        .find(|run| run.id == pending.id)
        .unwrap();

    let replayed = resolve_runtime_registration(&fixture, &handoffs, CodingUnitRunStatus::Pending);

    assert_eq!(replayed, running);
    assert_eq!(runtime_registration_unit(&fixture), running_unit);
}

#[test]
fn coding_runtime_handoff_placeholder_same_tuple_completed_replay_is_unchanged() {
    let fixture = runtime_handoff_fixture(
        RuntimeContractChange::CompatibleExtension,
        CodingUnitRunStatus::AwaitingAmendment,
    );
    let handoffs = vec!["handoff_revision_0002".to_string()];
    let completed = complete_runtime_registration_placeholder(&fixture, &handoffs);
    let completed_unit = runtime_registration_unit(&fixture);

    let replayed = resolve_runtime_registration(&fixture, &handoffs, CodingUnitRunStatus::Pending);

    assert_eq!(replayed, completed);
    assert_eq!(runtime_registration_unit(&fixture), completed_unit);
}

fn complete_non_registration_runtime_units(fixture: &RuntimeHandoffFixture) {
    for unit in fixture
        .store
        .list_coding_units(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .unwrap()
        .into_iter()
        .filter(|unit| unit.logical_work_item_id != "wi_registration")
    {
        fixture
            .store
            .update_coding_unit_status(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id,
                &unit.id,
                CodingExecutionUnitStatus::Completed,
                Some("Completed before runtime Handoff replay".to_string()),
            )
            .unwrap();
    }
}

#[tokio::test]
async fn coding_runtime_handoff_placeholder_different_tuple_converges_unit_and_replay() {
    let fixture = runtime_handoff_fixture(
        RuntimeContractChange::CompatibleExtension,
        CodingUnitRunStatus::AwaitingAmendment,
    );
    let first_handoffs = vec!["handoff_revision_0002".to_string()];
    let completed = complete_runtime_registration_placeholder(&fixture, &first_handoffs);
    complete_non_registration_runtime_units(&fixture);
    assert!(
        fixture
            .engine
            .group_attempt_ready_for_final_review(&fixture.attempt)
            .unwrap()
    );
    let next_handoffs = vec!["handoff_revision_0003".to_string()];

    let created = resolve_runtime_registration(
        &fixture,
        &next_handoffs,
        CodingUnitRunStatus::Pending,
    );
    assert_eq!(
        runtime_registration_unit(&fixture).status,
        CodingExecutionUnitStatus::Pending
    );
    assert!(
        !fixture
            .engine
            .group_attempt_ready_for_final_review(&fixture.attempt)
            .unwrap()
    );

    fixture
        .store
        .update_coding_unit_status(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
            &completed.unit_id,
            CodingExecutionUnitStatus::Completed,
            Some("Simulated crash-window stale Unit state".to_string()),
        )
        .unwrap();
    assert!(
        fixture
            .engine
            .group_attempt_ready_for_final_review(&fixture.attempt)
            .unwrap()
    );

    let replayed = resolve_runtime_registration(
        &fixture,
        &next_handoffs,
        CodingUnitRunStatus::Pending,
    );

    assert_ne!(created.id, completed.id);
    assert!(created.id.starts_with("coding_unit_run_runtime_handoff_"));
    assert_eq!(created.execution_no, 3);
    assert_eq!(created.status, CodingUnitRunStatus::Pending);
    assert_eq!(created.resolved_handoff_revision_ids, next_handoffs);
    assert_eq!(replayed, created);
    assert_eq!(
        runtime_registration_unit(&fixture).status,
        CodingExecutionUnitStatus::Pending
    );
    assert!(
        !fixture
            .engine
            .group_attempt_ready_for_final_review(&fixture.attempt)
            .unwrap()
    );
    let runs = fixture
        .store
        .list_coding_unit_runs(&fixture.attempt, &completed.unit_id)
        .unwrap();
    assert_eq!(runs.len(), 3);
    assert_eq!(runs[1], completed);

    fixture
        .engine
        .advance_to_next_group_unit(&fixture.attempt)
        .await
        .unwrap();
    let running_unit = runtime_registration_unit(&fixture);
    assert_eq!(running_unit.status, CodingExecutionUnitStatus::Running);
    let running = fixture
        .store
        .list_coding_unit_runs(&fixture.attempt, &completed.unit_id)
        .unwrap()
        .into_iter()
        .find(|run| run.id == created.id)
        .unwrap();
    assert_eq!(running.status, CodingUnitRunStatus::Running);

    let advanced_replay = resolve_runtime_registration(
        &fixture,
        &["handoff_revision_0003".to_string()],
        CodingUnitRunStatus::Pending,
    );
    assert_eq!(advanced_replay, running);
    assert_eq!(runtime_registration_unit(&fixture), running_unit);
}

#[test]
fn coding_runtime_handoff_placeholder_different_tuple_stale_converges_unit() {
    let fixture = runtime_handoff_fixture(
        RuntimeContractChange::BreakingChange,
        CodingUnitRunStatus::AwaitingAmendment,
    );
    let completed = complete_runtime_registration_placeholder(
        &fixture,
        &["handoff_revision_0002".to_string()],
    );
    complete_non_registration_runtime_units(&fixture);

    let created = resolve_runtime_registration(
        &fixture,
        &["handoff_revision_0003".to_string()],
        CodingUnitRunStatus::Stale,
    );

    assert_eq!(created.status, CodingUnitRunStatus::Stale);
    assert_eq!(
        runtime_registration_unit(&fixture).status,
        CodingExecutionUnitStatus::Stale
    );
    assert!(
        !fixture
            .engine
            .group_attempt_ready_for_final_review(&fixture.attempt)
            .unwrap()
    );
    let runs = fixture
        .store
        .list_coding_unit_runs(&fixture.attempt, &completed.unit_id)
        .unwrap();
    assert_eq!(runs.len(), 3);
    assert_eq!(runs[1], completed);
    assert_eq!(runs[2], created);
}

include!("runtime_handoff_latest_authority.rs");
