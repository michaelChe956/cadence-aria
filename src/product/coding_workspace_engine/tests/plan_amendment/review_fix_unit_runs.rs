use super::*;

#[tokio::test]
async fn coding_amendment_recovery_rejects_forged_deterministic_unit_run_without_writes() {
    let fixture = amendment_fixture().await;
    prepare_application_phase(
        &fixture,
        CodingAmendmentApplicationPhase::PlanBindingWritten,
    );
    fixture
        .store
        .materialize_unit_runs_from_manifest(
            &fixture.attempt,
            &fixture.manifest,
            fixture.attempt.head_commit.as_deref(),
        )
        .unwrap();
    let runs = fixture
        .store
        .list_unit_runs_by_logical_id(&fixture.attempt, "work_item_0001")
        .unwrap();
    let mut forged = runs.last().unwrap().clone();
    forged.operational_retry_count = 99;
    let path = fixture.store.coding_unit_run_path(
        &fixture.attempt.project_id,
        &fixture.attempt.issue_id,
        &fixture.attempt.id,
        &forged.unit_id,
        &forged.id,
    );
    std::fs::write(&path, serde_json::to_vec_pretty(&forged).unwrap()).unwrap();
    let attempt_before = fixture
        .store
        .get_attempt(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .unwrap();
    let journal_before = fixture
        .store
        .get_amendment_application_journal(&fixture.attempt, &fixture.manifest.id)
        .unwrap();

    let error = fixture
        .engine
        .recover_plan_amendment(&attempt_before)
        .await
        .expect_err("forged same-ID UnitRun must fail closed");

    assert!(error.to_string().contains("identity_mismatch"));
    assert_eq!(
        fixture
            .store
            .get_attempt(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id,
            )
            .unwrap(),
        attempt_before
    );
    assert_eq!(
        fixture
            .store
            .get_amendment_application_journal(&fixture.attempt, &fixture.manifest.id)
            .unwrap(),
        journal_before
    );
    assert_eq!(
        fixture
            .store
            .list_unit_runs_by_logical_id(&fixture.attempt, "work_item_0001")
            .unwrap()
            .last()
            .unwrap(),
        &forged
    );
}

#[tokio::test]
async fn coding_amendment_status_update_rejects_forged_deterministic_unit_run() {
    let fixture = amendment_fixture().await;
    prepare_application_phase(
        &fixture,
        CodingAmendmentApplicationPhase::PlanBindingWritten,
    );
    fixture
        .store
        .materialize_unit_runs_from_manifest(
            &fixture.attempt,
            &fixture.manifest,
            fixture.attempt.head_commit.as_deref(),
        )
        .unwrap();
    let mut forged = fixture
        .store
        .list_unit_runs_by_logical_id(&fixture.attempt, "work_item_0001")
        .unwrap()
        .pop()
        .unwrap();
    forged.reviewer_execution_context_hash = Some("forged_context_hash".to_string());
    let path = fixture.store.coding_unit_run_path(
        &fixture.attempt.project_id,
        &fixture.attempt.issue_id,
        &fixture.attempt.id,
        &forged.unit_id,
        &forged.id,
    );
    std::fs::write(&path, serde_json::to_vec_pretty(&forged).unwrap()).unwrap();

    let error = fixture
        .store
        .set_materialized_amendment_unit_run_status(
            &fixture.attempt,
            &fixture.manifest,
            "work_item_0001",
            CodingUnitRunStatus::Running,
        )
        .expect_err("status update must validate the full immutable run identity");

    assert!(error.to_string().contains("identity_mismatch"));
    assert_eq!(
        fixture
            .store
            .list_unit_runs_by_logical_id(&fixture.attempt, "work_item_0001")
            .unwrap()
            .pop()
            .unwrap(),
        forged
    );
}

#[tokio::test]
async fn coding_amendment_supersedes_only_active_replacement_source_runs() {
    let fixture = amendment_fixture().await;
    let source = fixture
        .store
        .list_coding_units(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .unwrap()
        .into_iter()
        .find(|unit| unit.logical_work_item_id == "work_item_0003")
        .unwrap();
    seed_unit_run(
        &fixture.store,
        &fixture.plan,
        &fixture.attempt,
        &source,
        "coding_unit_run_replacement_completed",
        CodingUnitRunStatus::Completed,
        Some("commit_replacement_completed"),
    );
    let completed = fixture
        .store
        .list_coding_unit_runs(&fixture.attempt, &source.id)
        .unwrap()
        .remove(0);
    let mut active = completed.clone();
    active.id = "coding_unit_run_replacement_active".to_string();
    active.execution_no = 2;
    active.status = CodingUnitRunStatus::Running;
    active.completion_commit = None;
    fixture
        .store
        .create_coding_unit_run(&fixture.attempt, &active)
        .unwrap();
    prepare_application_phase(
        &fixture,
        CodingAmendmentApplicationPhase::PlanBindingWritten,
    );
    let mut manifest = fixture.manifest.clone();
    manifest
        .unaffected_units
        .retain(|logical_id| logical_id != "work_item_0003");
    manifest.replacement_units.insert(
        "work_item_0003".to_string(),
        vec!["work_item_0001".to_string()],
    );

    fixture
        .store
        .materialize_unit_runs_from_manifest(
            &fixture.attempt,
            &manifest,
            fixture.attempt.head_commit.as_deref(),
        )
        .unwrap();
    fixture
        .store
        .materialize_unit_runs_from_manifest(
            &fixture.attempt,
            &manifest,
            fixture.attempt.head_commit.as_deref(),
        )
        .unwrap();

    let source_runs = fixture
        .store
        .list_coding_unit_runs(&fixture.attempt, &source.id)
        .unwrap();
    assert_eq!(source_runs[0], completed);
    assert_eq!(source_runs[0].status, CodingUnitRunStatus::Completed);
    assert_eq!(source_runs[1].status, CodingUnitRunStatus::Superseded);
    assert_eq!(source_runs[1].completion_commit, None);
}

#[tokio::test]
async fn coding_amendment_rejects_revised_unit_reused_as_replacement_source() {
    let fixture = amendment_fixture().await;
    prepare_application_phase(
        &fixture,
        CodingAmendmentApplicationPhase::PlanBindingWritten,
    );
    let mut manifest = fixture.manifest.clone();
    manifest
        .unaffected_units
        .retain(|logical_id| logical_id != "work_item_0003");
    manifest.replacement_units.insert(
        "work_item_0001".to_string(),
        vec!["work_item_0003".to_string()],
    );

    let error = fixture
        .store
        .materialize_unit_runs_from_manifest(
            &fixture.attempt,
            &manifest,
            fixture.attempt.head_commit.as_deref(),
        )
        .expect_err("revised and replacement-source partitions must be disjoint");

    assert!(
        error
            .to_string()
            .contains("coding_amendment_replacement_source")
    );
}

#[tokio::test]
async fn coding_amendment_await_handoff_keeps_attempt_provider_blocked() {
    let fixture = amendment_fixture().await;
    let mut manifest = fixture.manifest.clone();
    manifest.resume_target.mode = AmendmentResumeMode::AwaitHandoff;
    let manifest_path = fixture
        .store
        .paths()
        .issue_root(&fixture.attempt.project_id, &fixture.attempt.issue_id)
        .join("work-item-revisions")
        .join(&fixture.plan.id)
        .join("amendment-manifests")
        .join(format!("{}.json", manifest.id));
    std::fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    let lifecycle = LifecycleStore::new(fixture.store.paths());
    let mut snapshot = lifecycle
        .load_plan_repair_session_state(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.child_session_id,
        )
        .unwrap()
        .unwrap();
    snapshot.amendment = Some(manifest.clone());
    lifecycle
        .save_plan_repair_session_state(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.child_session_id,
            &snapshot,
        )
        .unwrap();

    let updated = fixture
        .engine
        .apply_plan_amendment(&fixture.attempt, &manifest)
        .await
        .unwrap();

    assert_eq!(updated.status, CodingAttemptStatus::AwaitingPlanAmendment);
    assert_eq!(updated.stage, CodingExecutionStage::Coding);
    assert!(
        fixture
            .store
            .ensure_provider_run_allowed(&updated)
            .unwrap_err()
            .to_string()
            .contains("plan_amendment_blocks_provider_run")
    );
    let run = fixture
        .store
        .list_unit_runs_by_logical_id(&updated, "work_item_0001")
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(run.status, CodingUnitRunStatus::AwaitingAmendment);
}
