use super::*;
use crate::product::coding_models::CodingProviderRole;
use crate::product::work_item_projection::{
    CoderExecutionEnvelope, RenderedExecutionContext, ReviewerExecutionEnvelope, renderer_for,
};

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

#[tokio::test]
async fn coding_amendment_completed_replay_accepts_group_completion_head_evolution() {
    let fixture = amendment_fixture().await;
    let worktree = fixture.attempt.worktree_path.as_ref().unwrap();
    let materialization_head = git_stdout(worktree, &["rev-parse", "HEAD"])
        .trim()
        .to_string();
    let prepared = fixture
        .store
        .update_attempt_head_commit(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
            Some(materialization_head.clone()),
        )
        .unwrap();
    let applied = fixture
        .engine
        .apply_plan_amendment(&prepared, &fixture.manifest)
        .await
        .unwrap();
    let materialized = fixture
        .store
        .list_unit_runs_by_logical_id(&applied, "work_item_0001")
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(
        materialized.start_commit.as_deref(),
        Some(materialization_head.as_str())
    );

    std::fs::write(
        worktree.join("completed_after_amendment.rs"),
        "// completed\n",
    )
    .unwrap();
    let review_ready = fixture
        .store
        .update_attempt_stage(
            &applied.project_id,
            &applied.issue_id,
            &applied.id,
            CodingExecutionStage::ReviewRequest,
        )
        .unwrap();
    let completed = fixture
        .engine
        .complete_group_unit_after_code_review(&review_ready)
        .await
        .unwrap();
    let progressed = fixture
        .store
        .update_attempt_stage(
            &completed.project_id,
            &completed.issue_id,
            &completed.id,
            CodingExecutionStage::Coding,
        )
        .unwrap();
    assert_ne!(
        progressed.head_commit.as_deref(),
        Some(materialization_head.as_str())
    );

    let replayed = fixture
        .engine
        .recover_plan_amendment(&progressed)
        .await
        .expect("Completed replay must retain the materialization-time start commit");

    assert_eq!(replayed, progressed);
    let persisted = fixture
        .store
        .list_unit_runs_by_logical_id(&replayed, "work_item_0001")
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(persisted.start_commit, Some(materialization_head));
    assert_eq!(persisted.status, CodingUnitRunStatus::Completed);
    assert_eq!(persisted.completion_commit, progressed.head_commit);
}

#[tokio::test]
async fn coding_amendment_completed_replay_accepts_provider_renderer_context_evolution() {
    let fixture = amendment_fixture().await;
    let applied = fixture
        .engine
        .apply_plan_amendment(&fixture.attempt, &fixture.manifest)
        .await
        .unwrap();
    let mut providers = fixture
        .store
        .get_role_provider_config_snapshot(&applied.project_id, &applied.issue_id, &applied.id)
        .unwrap();
    providers.set_provider_for_role(&CodingProviderRole::Coder, ProviderName::Fake);
    providers.set_provider_for_role(&CodingProviderRole::CodeReviewer, ProviderName::Fake);
    fixture
        .store
        .update_role_provider_config_snapshot(
            &applied.project_id,
            &applied.issue_id,
            &applied.id,
            providers,
        )
        .unwrap();

    let run = fixture
        .store
        .list_unit_runs_by_logical_id(&applied, "work_item_0001")
        .unwrap()
        .pop()
        .unwrap();
    let bundle = fixture
        .revision_store
        .get_work_item_projection_bundle(&fixture.plan, &run.projection_bundle_id)
        .unwrap();
    let repository_state_ref = applied
        .head_commit
        .clone()
        .unwrap_or_else(|| applied.base_branch.clone());
    let renderer = renderer_for(&ProviderName::Fake);
    let coder = renderer
        .render_coder(
            &bundle.coder_projection,
            &CoderExecutionEnvelope {
                repository_state_ref: repository_state_ref.clone(),
                resolved_handoff_revision_ids: run.resolved_handoff_revision_ids.clone(),
                unit_run_id: run.id.clone(),
                previous_actionable_review: None,
                start_commit: run.start_commit.clone(),
            },
        )
        .unwrap();
    fixture
        .store
        .bind_unit_run_execution_context(&applied, &run.id, CodingProviderRole::Coder, &coder)
        .unwrap();
    let reviewer = renderer
        .render_reviewer(
            &bundle.reviewer_projection,
            &ReviewerExecutionEnvelope {
                unit_run_id: run.id.clone(),
                diff_ref: format!("{repository_state_ref}..worktree"),
                test_evidence_refs: Vec::new(),
                handoff_revision_ids: run.resolved_handoff_revision_ids,
                contract_delta_refs: Vec::new(),
                completion_commit: repository_state_ref,
            },
        )
        .unwrap();
    fixture
        .store
        .bind_unit_run_execution_context(
            &applied,
            &run.id,
            CodingProviderRole::CodeReviewer,
            &reviewer,
        )
        .unwrap();

    let replayed = fixture
        .engine
        .recover_plan_amendment(&applied)
        .await
        .expect("Completed replay must accept controlled renderer and context evolution");

    assert_eq!(replayed, applied);
    let persisted = fixture
        .store
        .list_unit_runs_by_logical_id(&replayed, "work_item_0001")
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(
        persisted.coder_provider_renderer_version,
        coder.renderer_version
    );
    assert_eq!(
        persisted.reviewer_provider_renderer_version,
        reviewer.renderer_version
    );
    assert_eq!(
        persisted.coder_execution_context_hash,
        Some(coder.content_hash)
    );
    assert_eq!(
        persisted.reviewer_execution_context_hash,
        Some(reviewer.content_hash)
    );
}
