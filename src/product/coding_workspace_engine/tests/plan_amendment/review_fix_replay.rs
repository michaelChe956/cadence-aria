use super::*;
use crate::product::coding_models::CodingProviderRole;
use crate::product::work_item_projection::RenderedExecutionContext;

#[tokio::test]
async fn coding_amendment_completed_replay_allows_bound_execution_context() {
    let fixture = amendment_fixture().await;
    let applied = fixture
        .engine
        .apply_plan_amendment(&fixture.attempt, &fixture.manifest)
        .await
        .unwrap();
    let run = fixture
        .store
        .list_unit_runs_by_logical_id(&applied, "work_item_0001")
        .unwrap()
        .pop()
        .unwrap();
    let rendered = RenderedExecutionContext {
        text: "runtime-bound execution context".to_string(),
        renderer_version: run.coder_provider_renderer_version.clone(),
        content_hash: "runtime_context_hash_0001".to_string(),
    };
    let bound = fixture
        .store
        .bind_unit_run_execution_context(&applied, &run.id, CodingProviderRole::Coder, &rendered)
        .unwrap();

    let replayed = fixture
        .engine
        .recover_plan_amendment(&applied)
        .await
        .expect("Completed replay must accept runtime execution-context binding");

    assert_eq!(replayed, applied);
    assert_eq!(
        fixture
            .store
            .list_unit_runs_by_logical_id(&replayed, "work_item_0001")
            .unwrap()
            .pop()
            .unwrap(),
        bound
    );
}

#[tokio::test]
async fn coding_amendment_completed_replay_allows_completed_unit_run() {
    let fixture = amendment_fixture().await;
    let applied = fixture
        .engine
        .apply_plan_amendment(&fixture.attempt, &fixture.manifest)
        .await
        .unwrap();
    let run = fixture
        .store
        .list_unit_runs_by_logical_id(&applied, "work_item_0001")
        .unwrap()
        .pop()
        .unwrap();
    let completed = fixture
        .store
        .complete_coding_unit_run(&applied, &run.id, "commit_after_amendment")
        .unwrap();

    let replayed = fixture
        .engine
        .recover_plan_amendment(&applied)
        .await
        .expect("Completed replay must accept a normally completed UnitRun");

    assert_eq!(replayed, applied);
    assert_eq!(
        fixture
            .store
            .list_unit_runs_by_logical_id(&replayed, "work_item_0001")
            .unwrap()
            .pop()
            .unwrap(),
        completed
    );
}

#[tokio::test]
async fn coding_amendment_completed_replay_preserves_later_attempt_stage() {
    for stage in [
        CodingExecutionStage::Testing,
        CodingExecutionStage::CodeReview,
    ] {
        let fixture = amendment_fixture().await;
        let applied = fixture
            .engine
            .apply_plan_amendment(&fixture.attempt, &fixture.manifest)
            .await
            .unwrap();
        let progressed = fixture
            .store
            .update_attempt_stage(
                &applied.project_id,
                &applied.issue_id,
                &applied.id,
                stage.clone(),
            )
            .unwrap();

        let replayed = fixture
            .engine
            .recover_plan_amendment(&progressed)
            .await
            .expect("Completed replay must not regress a progressed Attempt");

        assert_eq!(replayed, progressed);
    }
}
