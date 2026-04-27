use cadence_aria::cross_cutting::artifact_validate::{
    phase1_profile_validator, ArtifactValidateError, ConstraintBundleIndex, ProjectionIndex,
    ProviderRunIndex, TraceabilityIndex,
};
use cadence_aria::protocol::artifacts::ArtifactKind;
use serde_json::json;

#[test]
fn dispatch_package_profile_validates_refs_and_worktask_routing() {
    let artifact = json!({
        "artifact_kind": "dispatch_package",
        "_aria": {
            "profile_version": "phase1.v1",
            "constraint_check_ref": "chk_001",
            "traceability_refs": ["req-001"],
            "provider_run_refs": ["run_001"],
            "projection_refs": ["proj_plan_projection_art_plan_001_0001"],
            "worktask_routing": [
                {
                    "worktask_id": "work_001",
                    "source_work_package_id": "wt-001",
                    "execution_mode": "agent_only",
                    "human_required_reason": null,
                    "allowed_write_scope": ["src/"],
                    "traceability_refs": ["req-001"],
                    "verification_commands": ["cargo test -j 1"]
                }
            ]
        }
    });

    let result = phase1_profile_validator(
        &artifact,
        ArtifactKind::DispatchPackage,
        &ProjectionIndex::with_work_packages(
            vec!["proj_plan_projection_art_plan_001_0001".to_string()],
            vec!["wt-001".to_string()],
        ),
        &ConstraintBundleIndex::with_checks(vec!["chk_001".to_string()]),
        &TraceabilityIndex::with_known_refs(vec!["req-001".to_string()]),
        &ProviderRunIndex::with_runs(vec!["run_001".to_string()]),
    )
    .expect("profile validates");

    assert!(result.valid);
}

#[test]
fn profile_validator_rejects_unknown_projection_and_constraint_refs() {
    let artifact = json!({
        "artifact_kind": "coding_report",
        "_aria": {
            "profile_version": "phase1.v1",
            "constraint_check_ref": "chk_missing",
            "traceability_refs": ["req-001"],
            "provider_run_refs": [],
            "projection_refs": ["proj_missing"]
        }
    });

    let projection_error = phase1_profile_validator(
        &artifact,
        ArtifactKind::CodingReport,
        &ProjectionIndex::with_work_packages(vec![], vec![]),
        &ConstraintBundleIndex::with_checks(vec!["chk_missing".to_string()]),
        &TraceabilityIndex::with_known_refs(vec!["req-001".to_string()]),
        &ProviderRunIndex::default(),
    )
    .expect_err("unknown projection");
    assert_eq!(
        projection_error,
        ArtifactValidateError::ProfileProjectionRefUnknown("proj_missing".to_string())
    );

    let constraint_error = phase1_profile_validator(
        &artifact,
        ArtifactKind::CodingReport,
        &ProjectionIndex::with_work_packages(vec!["proj_missing".to_string()], vec![]),
        &ConstraintBundleIndex::with_checks(vec![]),
        &TraceabilityIndex::with_known_refs(vec!["req-001".to_string()]),
        &ProviderRunIndex::default(),
    )
    .expect_err("unknown constraint check");
    assert_eq!(
        constraint_error,
        ArtifactValidateError::ProfileConstraintRefUnknown("chk_missing".to_string())
    );
}

#[test]
fn report_profile_requires_daemon_normalized_traceability_refs() {
    let artifact = json!({
        "artifact_kind": "testing_report",
        "_aria": {
            "profile_version": "phase1.v1",
            "constraint_check_ref": "chk_001",
            "provider_run_refs": [],
            "projection_refs": []
        }
    });

    let error = phase1_profile_validator(
        &artifact,
        ArtifactKind::TestingReport,
        &ProjectionIndex::default(),
        &ConstraintBundleIndex::with_checks(vec!["chk_001".to_string()]),
        &TraceabilityIndex::with_known_refs(vec!["req-001".to_string()]),
        &ProviderRunIndex::default(),
    )
    .expect_err("traceability refs are required");

    assert_eq!(error, ArtifactValidateError::TraceabilityRefsMissing);
}

#[test]
fn dispatch_profile_rejects_unknown_source_work_package() {
    let artifact = json!({
        "artifact_kind": "dispatch_package",
        "_aria": {
            "profile_version": "phase1.v1",
            "constraint_check_ref": "chk_001",
            "traceability_refs": ["req-001"],
            "provider_run_refs": [],
            "projection_refs": [],
            "worktask_routing": [
                {
                    "worktask_id": "work_001",
                    "source_work_package_id": "wt-999",
                    "execution_mode": "agent_only",
                    "allowed_write_scope": ["src/"],
                    "traceability_refs": ["req-001"],
                    "verification_commands": ["cargo test -j 1"]
                }
            ]
        }
    });

    let error = phase1_profile_validator(
        &artifact,
        ArtifactKind::DispatchPackage,
        &ProjectionIndex::with_work_packages(vec![], vec!["wt-001".to_string()]),
        &ConstraintBundleIndex::with_checks(vec!["chk_001".to_string()]),
        &TraceabilityIndex::with_known_refs(vec!["req-001".to_string()]),
        &ProviderRunIndex::default(),
    )
    .expect_err("unknown work package");

    assert_eq!(
        error,
        ArtifactValidateError::WorktaskRoutingSourceUnknown("wt-999".to_string())
    );
}

#[test]
fn final_review_profile_requires_coverage_summary_shape() {
    let missing = json!({
        "artifact_kind": "final_review",
        "_aria": {
            "profile_version": "phase1.v1",
            "constraint_check_ref": "chk_001",
            "traceability_refs": ["req-001"],
            "provider_run_refs": [],
            "projection_refs": []
        }
    });

    let error = phase1_profile_validator(
        &missing,
        ArtifactKind::FinalReview,
        &ProjectionIndex::default(),
        &ConstraintBundleIndex::with_checks(vec!["chk_001".to_string()]),
        &TraceabilityIndex::with_known_refs(vec!["req-001".to_string()]),
        &ProviderRunIndex::default(),
    )
    .expect_err("coverage summary required");
    assert_eq!(error, ArtifactValidateError::CoverageSummaryMissing);

    let valid = json!({
        "artifact_kind": "final_review",
        "_aria": {
            "profile_version": "phase1.v1",
            "constraint_check_ref": "chk_001",
            "traceability_refs": ["req-001"],
            "provider_run_refs": [],
            "projection_refs": [],
            "coverage_summary": {
                "closed": ["req-001"],
                "uncovered": [],
                "exempted": []
            }
        }
    });
    assert!(
        phase1_profile_validator(
            &valid,
            ArtifactKind::FinalReview,
            &ProjectionIndex::default(),
            &ConstraintBundleIndex::with_checks(vec!["chk_001".to_string()]),
            &TraceabilityIndex::with_known_refs(vec!["req-001".to_string()]),
            &ProviderRunIndex::default(),
        )
        .expect("valid final review")
        .valid
    );
}
