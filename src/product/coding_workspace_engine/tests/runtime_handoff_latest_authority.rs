fn advance_latest_runtime_registration(
    fixture: &RuntimeHandoffFixture,
    complete: bool,
) -> (
    crate::product::coding_models::CodingExecutionUnit,
    CodingUnitRun,
) {
    let unit = runtime_registration_unit(fixture);
    fixture
        .store
        .start_pending_coding_unit_run(&fixture.attempt, &unit.id)
        .unwrap();
    let mut advanced_unit = fixture
        .store
        .update_coding_unit_status(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
            &unit.id,
            CodingExecutionUnitStatus::Running,
            Some("Newer runtime Handoff execution started".to_string()),
        )
        .unwrap();
    let mut advanced_run = fixture
        .store
        .list_coding_unit_runs(&fixture.attempt, &unit.id)
        .unwrap()
        .into_iter()
        .max_by_key(|run| run.execution_no)
        .unwrap();
    if complete {
        advanced_run = fixture
            .store
            .complete_coding_unit_run(
                &fixture.attempt,
                &advanced_run.id,
                "commit_registration_newer_runtime_complete",
            )
            .unwrap();
        fixture
            .store
            .update_coding_unit_completion_commit(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id,
                &unit.id,
                advanced_run.completion_commit.clone(),
            )
            .unwrap();
        advanced_unit = fixture
            .store
            .update_coding_unit_status(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id,
                &unit.id,
                CodingExecutionUnitStatus::Completed,
                Some("Newer runtime Handoff execution completed".to_string()),
            )
            .unwrap();
    }
    (advanced_unit, advanced_run)
}

#[test]
fn coding_runtime_handoff_placeholder_historical_fixed_tuple_replay_is_noop() {
    for (historical_status, complete_newer) in [
        (CodingUnitRunStatus::Pending, false),
        (CodingUnitRunStatus::Stale, true),
    ] {
        let fixture = runtime_handoff_fixture(
            RuntimeContractChange::CompatibleExtension,
            CodingUnitRunStatus::AwaitingAmendment,
        );
        complete_runtime_registration_placeholder(
            &fixture,
            &["handoff_revision_0002".to_string()],
        );
        let historical = resolve_runtime_registration(
            &fixture,
            &["handoff_revision_0003".to_string()],
            historical_status.clone(),
        );
        let newer = resolve_runtime_registration(
            &fixture,
            &["handoff_revision_0004".to_string()],
            CodingUnitRunStatus::Pending,
        );
        let (unit_before, newer_before) =
            advance_latest_runtime_registration(&fixture, complete_newer);
        assert_eq!(newer_before.id, newer.id);
        assert_eq!(
            newer_before.status,
            if complete_newer {
                CodingUnitRunStatus::Completed
            } else {
                CodingUnitRunStatus::Running
            }
        );
        let runs_before = fixture
            .store
            .list_coding_unit_runs(&fixture.attempt, &historical.unit_id)
            .unwrap();

        let replayed = resolve_runtime_registration(
            &fixture,
            &["handoff_revision_0003".to_string()],
            historical_status,
        );

        assert_eq!(replayed, historical);
        assert_eq!(runtime_registration_unit(&fixture), unit_before);
        assert_eq!(
            fixture
                .store
                .list_coding_unit_runs(&fixture.attempt, &historical.unit_id)
                .unwrap(),
            runs_before
        );
    }
}

#[tokio::test]
async fn coding_runtime_handoff_authority_historical_pointer_is_zero_write() {
    let fixture = runtime_handoff_fixture(
        RuntimeContractChange::CompatibleExtension,
        CodingUnitRunStatus::AwaitingAmendment,
    );
    append_runtime_source_handoff(
        &fixture,
        "handoff_revision_0003",
        3,
        "2026-07-21T00:00:00Z",
    );

    assert_runtime_handoff_authority_zero_write(&fixture, &fixture.next_handoff).await;
}
