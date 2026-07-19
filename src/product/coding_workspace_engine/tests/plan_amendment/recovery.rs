use super::*;

#[tokio::test]
async fn coding_amendment_recovers_failed_phase_and_clears_error() {
    let fixture = amendment_fixture().await;
    let bundle_path = fixture
        .store
        .paths()
        .issue_root(&fixture.attempt.project_id, &fixture.attempt.issue_id)
        .join("work-item-revisions")
        .join(&fixture.plan.id)
        .join("work-item-projection-bundles/projection_bundle_0101.json");
    let bundle = std::fs::read(&bundle_path).unwrap();
    std::fs::remove_file(&bundle_path).unwrap();
    fixture
        .engine
        .apply_plan_amendment(&fixture.attempt, &fixture.manifest)
        .await
        .expect_err("materialization failure must stop at PlanBindingWritten");
    std::fs::write(&bundle_path, bundle).unwrap();
    let failed = fixture
        .store
        .get_attempt(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .unwrap();
    let recovered = fixture
        .engine
        .recover_plan_amendment(&failed)
        .await
        .expect("recover from last durable phase");

    assert_eq!(recovered.status, CodingAttemptStatus::Running);
    let journal = fixture
        .store
        .get_amendment_application_journal(&recovered, &fixture.manifest.id)
        .unwrap();
    assert_eq!(journal.phase, CodingAmendmentApplicationPhase::Completed);
    assert_eq!(journal.error, None);
    assert_eq!(
        fixture
            .store
            .list_unit_runs_by_logical_id(&recovered, "work_item_0001")
            .unwrap()
            .len(),
        2
    );
}
