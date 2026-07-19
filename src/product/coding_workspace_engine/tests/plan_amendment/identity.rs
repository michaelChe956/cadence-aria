use super::*;

#[tokio::test]
async fn coding_amendment_missing_plan_binding_is_zero_write() {
    let fixture = amendment_fixture().await;
    std::fs::remove_file(fixture.store.plan_binding_path(
        &fixture.attempt.project_id,
        &fixture.attempt.issue_id,
        &fixture.attempt.id,
    ))
    .unwrap();

    fixture
        .engine
        .apply_plan_amendment(&fixture.attempt, &fixture.manifest)
        .await
        .expect_err("missing canonical Plan Binding must fail closed");

    assert!(
        fixture
            .store
            .list_amendment_application_journals(&fixture.attempt)
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn coding_amendment_missing_plan_lineage_is_zero_write() {
    let fixture = amendment_fixture().await;
    let lineage = fixture
        .store
        .paths()
        .issue_root(&fixture.attempt.project_id, &fixture.attempt.issue_id)
        .join("work-item-revisions")
        .join(&fixture.plan.id)
        .join("lineage.json");
    std::fs::remove_file(lineage).unwrap();

    fixture
        .engine
        .apply_plan_amendment(&fixture.attempt, &fixture.manifest)
        .await
        .expect_err("missing canonical Plan lineage must fail closed");

    assert!(
        fixture
            .store
            .list_amendment_application_journals(&fixture.attempt)
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn coding_amendment_active_lock_mismatch_is_zero_write() {
    let fixture = amendment_fixture().await;
    fixture
        .revision_store
        .release_active_amendment(&fixture.plan, &fixture.manifest.id)
        .unwrap();
    let plan = fixture
        .revision_store
        .get_plan_lineage(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.plan.id,
        )
        .unwrap();
    fixture
        .revision_store
        .acquire_active_amendment(&plan, "plan_amendment_foreign")
        .unwrap();

    fixture
        .engine
        .apply_plan_amendment(&fixture.attempt, &fixture.manifest)
        .await
        .expect_err("foreign active lock must fail closed");

    assert!(
        fixture
            .store
            .list_amendment_application_journals(&fixture.attempt)
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn coding_amendment_request_status_mismatch_is_zero_write() {
    let fixture = amendment_fixture().await;
    fixture
        .revision_store
        .update_repair_request_status(
            &fixture.plan,
            &fixture.manifest.repair_request_id,
            PlanRepairRequestStatus::InProgress,
        )
        .unwrap();

    fixture
        .engine
        .apply_plan_amendment(&fixture.attempt, &fixture.manifest)
        .await
        .expect_err("non-published request must fail closed");

    assert!(
        fixture
            .store
            .list_amendment_application_journals(&fixture.attempt)
            .unwrap()
            .is_empty()
    );
}
