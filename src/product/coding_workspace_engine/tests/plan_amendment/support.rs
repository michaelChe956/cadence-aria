use super::*;

pub(super) fn seed_unit_run(
    store: &CodingAttemptStore,
    plan: &WorkItemPlanLineage,
    attempt: &CodingExecutionAttempt,
    unit: &crate::product::coding_models::CodingExecutionUnit,
    id: &str,
    status: CodingUnitRunStatus,
    completion_commit: Option<&str>,
) {
    let revision_store = WorkItemRevisionStore::new(store.paths());
    let revision = revision_store
        .get_work_item_revision(
            plan,
            &unit.logical_work_item_id,
            &unit.work_item_revision_id,
        )
        .unwrap();
    let bundle = revision_store
        .get_work_item_projection_bundle(plan, &revision.work_item_projection_bundle_id)
        .unwrap();
    store
        .create_coding_unit_run(
            attempt,
            &CodingUnitRun {
                id: id.to_string(),
                unit_id: unit.id.clone(),
                execution_no: 1,
                work_item_revision_id: revision.id,
                resolved_handoff_revision_ids: Vec::new(),
                canonical_contract_hash: bundle.canonical_contract_hash,
                projection_bundle_id: bundle.id,
                projection_compiler_version: bundle.compiler_version,
                coder_provider_renderer_version: "coder-v1".to_string(),
                reviewer_provider_renderer_version: "reviewer-v1".to_string(),
                internal_reviewer_provider_renderer_version: None,
                coder_projection_hash: bundle.coder_projection_hash,
                reviewer_projection_hash: bundle.reviewer_projection_hash,
                coder_execution_context_hash: None,
                reviewer_execution_context_hash: None,
                internal_reviewer_execution_context_hash: None,
                status,
                unit_rework_count: 4,
                verification_retry_count: 3,
                operational_retry_count: 2,
                plan_repair_count: 0,
                start_commit: Some("commit_start".to_string()),
                completion_commit: completion_commit.map(str::to_string),
                created_at: "2026-07-19T00:00:00Z".to_string(),
                updated_at: "2026-07-19T00:00:00Z".to_string(),
            },
        )
        .unwrap();
}

pub(super) fn prepare_application_phase(
    fixture: &AmendmentFixture,
    phase: CodingAmendmentApplicationPhase,
) {
    fixture
        .store
        .load_or_prepare_amendment_application(&fixture.attempt, &fixture.manifest)
        .unwrap();
    if phase == CodingAmendmentApplicationPhase::Started {
        return;
    }
    fixture
        .store
        .update_attempt_status(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
            CodingAttemptStatus::ApplyingPlanAmendment,
        )
        .unwrap();
    fixture
        .store
        .update_plan_binding_from_manifest(&fixture.attempt, &fixture.manifest)
        .unwrap();
    fixture
        .store
        .advance_amendment_application_journal(
            &fixture.attempt,
            &fixture.manifest.id,
            CodingAmendmentApplicationPhase::PlanBindingWritten,
            None,
            "2026-07-19T00:00:03Z".to_string(),
        )
        .unwrap();
    if phase == CodingAmendmentApplicationPhase::PlanBindingWritten {
        return;
    }
    fixture
        .store
        .materialize_unit_runs_from_manifest(
            &fixture.attempt,
            &fixture.manifest,
            fixture.attempt.head_commit.as_deref(),
        )
        .unwrap();
    fixture
        .store
        .advance_amendment_application_journal(
            &fixture.attempt,
            &fixture.manifest.id,
            CodingAmendmentApplicationPhase::UnitRunsWritten,
            None,
            "2026-07-19T00:00:04Z".to_string(),
        )
        .unwrap();
    if phase == CodingAmendmentApplicationPhase::UnitRunsWritten {
        return;
    }
    fixture
        .store
        .set_resume_target_from_manifest(&fixture.attempt, &fixture.manifest)
        .unwrap();
    let journal = fixture
        .store
        .advance_amendment_application_journal(
            &fixture.attempt,
            &fixture.manifest.id,
            CodingAmendmentApplicationPhase::ResumeTargetWritten,
            None,
            "2026-07-19T00:00:05Z".to_string(),
        )
        .unwrap();
    assert_eq!(journal.phase, phase);
}
