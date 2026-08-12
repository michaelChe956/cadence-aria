// Group final readiness persistence regression coverage.

fn final_readiness_finding() -> ReviewFinding {
    ReviewFinding {
        severity: FindingSeverity::Warning,
        file_path: Some("src/product/coding_attempt_store/group_final_readiness.rs".to_string()),
        line: Some(42),
        message: "review evidence is preserved".to_string(),
        required_action: Some("confirm the finding".to_string()),
        source_stage: CodingExecutionStage::CodeReview,
        evidence: vec!["test-output/review-evidence.txt".to_string()],
        plan_defect_evidence: Vec::new(),
        related_requirements: Vec::new(),
        related_design_constraints: Vec::new(),
        related_work_item_tasks: vec!["work_item_0001".to_string()],
        defect_class: PlanDefectClass::ImplementationDefect,
        reason_code: Some("review_evidence".to_string()),
        contract_refs: Vec::new(),
        capability_refs: Vec::new(),
        repair_target: None,
        recommended_route: PlanDefectRoute::CoderRework,
        confidence: None,
    }
}

fn ready_unit(
    unit_id: &str,
    logical_work_item_id: &str,
    unit_run_id: &str,
) -> GroupFinalReadinessUnit {
    GroupFinalReadinessUnit {
        unit_id: unit_id.to_string(),
        logical_work_item_id: logical_work_item_id.to_string(),
        unit_run_id: Some(unit_run_id.to_string()),
        start_commit: Some(format!("start_commit_{unit_id}")),
        completion_commit: Some(format!("completion_commit_{unit_id}")),
        commit_shas: vec![
            format!("start_commit_{unit_id}"),
            format!("completion_commit_{unit_id}"),
        ],
        diff_ref: format!("diffs/{unit_id}.patch"),
        empty_observation: false,
        code_review_report_id: Some(format!("code_review_report_{unit_id}")),
        review_verdict: Some(ReviewVerdict::Approve),
        review_summary: Some(format!("{unit_id} review approved")),
        review_findings: Some(vec![final_readiness_finding()]),
        review_raw_provider_output_ref: Some(format!("provider-raw/code-review/{unit_id}.txt")),
        handoff_revision_id: Some(format!("handoff_revision_{unit_id}")),
        plan_revision_id: Some(format!("plan_revision_{unit_id}")),
    }
}

#[test]
fn group_final_readiness_round_trips_complete_snapshot() {
    let (_tmp, store, attempt) = setup();
    let snapshot = GroupFinalReadinessSnapshot {
        attempt_id: attempt.id.clone(),
        status: GroupFinalReadinessStatus::Complete,
        units: vec![
            ready_unit("unit_0001", "work_item_0001", "unit_run_0001"),
            ready_unit("unit_0002", "work_item_0002", "unit_run_0002"),
        ],
        diagnostics: Vec::new(),
        created_at: "caller-supplied-value-must-not-be-persisted".to_string(),
    };

    store
        .write_group_final_readiness_snapshot(&attempt, &snapshot)
        .expect("write readiness snapshot");

    let restored = store
        .get_group_final_readiness_snapshot(&attempt)
        .expect("read readiness snapshot")
        .expect("snapshot exists");

    assert_eq!(restored.status, GroupFinalReadinessStatus::Complete);
    assert_eq!(restored.units, snapshot.units);
    assert_eq!(
        restored.units[0].commit_shas,
        vec!["start_commit_unit_0001", "completion_commit_unit_0001"]
    );
    assert_eq!(
        restored.units[0].unit_run_id.as_deref(),
        Some("unit_run_0001")
    );
    assert_eq!(
        restored.units[0].start_commit.as_deref(),
        Some("start_commit_unit_0001")
    );
    assert_eq!(
        restored.units[0].completion_commit.as_deref(),
        Some("completion_commit_unit_0001")
    );
    assert_eq!(
        restored.units[0].review_findings,
        Some(vec![final_readiness_finding()])
    );
    assert_eq!(
        restored.units[0].review_raw_provider_output_ref.as_deref(),
        Some("provider-raw/code-review/unit_0001.txt")
    );
    assert!(
        serde_json::to_value(&restored)
            .expect("serialize restored snapshot")
            .get("units")
            .and_then(|units| units.get(0))
            .and_then(|unit| unit.get("review_raw_ref"))
            .is_none()
    );
    assert!(
        serde_json::to_value(&restored)
            .expect("serialize restored snapshot")
            .get("units")
            .and_then(|units| units.get(0))
            .and_then(|unit| unit.get("review_raw_provider_output_ref"))
            .is_some()
    );
    let created_at = chrono::DateTime::parse_from_rfc3339(&restored.created_at)
        .expect("writer stamps snapshot with RFC3339 time");
    assert_eq!(created_at.offset().local_minus_utc(), 0);
    assert_ne!(restored.created_at, snapshot.created_at);
    assert_eq!(
        restored.units[1].handoff_revision_id.as_deref(),
        Some("handoff_revision_unit_0002")
    );
    assert_eq!(
        restored.units[1].plan_revision_id.as_deref(),
        Some("plan_revision_unit_0002")
    );
    assert!(!restored.units[1].empty_observation);
}

#[test]
fn group_final_readiness_empty_observation_requires_empty_git_facts() {
    let (_tmp, store, attempt) = setup();
    let mut unit = ready_unit("unit_0001", "work_item_0001", "unit_run_0001");
    unit.empty_observation = true;
    unit.commit_shas.clear();
    unit.diff_ref.clear();
    let snapshot = GroupFinalReadinessSnapshot {
        attempt_id: attempt.id.clone(),
        status: GroupFinalReadinessStatus::Complete,
        units: vec![unit.clone()],
        diagnostics: Vec::new(),
        created_at: String::new(),
    };

    store
        .write_group_final_readiness_snapshot(&attempt, &snapshot)
        .expect("empty observation snapshot");
    assert!(
        store
            .get_group_final_readiness_snapshot(&attempt)
            .expect("read snapshot")
            .expect("snapshot")
            .units[0]
            .empty_observation
    );

    unit.commit_shas = vec!["completion_commit_unit_0001".to_string()];
    let inconsistent = GroupFinalReadinessSnapshot {
        units: vec![unit],
        ..snapshot
    };
    assert_invalid_group_final_readiness_snapshot(
        store
            .write_group_final_readiness_snapshot(&attempt, &inconsistent)
            .expect_err("empty observation must not include git facts"),
        "empty observation unit unit_0001 must not include git range facts",
    );
}

#[test]
fn group_final_readiness_rejects_snapshot_for_different_attempt() {
    let (_tmp, store, attempt) = setup();
    let mut snapshot = GroupFinalReadinessSnapshot {
        attempt_id: "coding_attempt_other".to_string(),
        status: GroupFinalReadinessStatus::Complete,
        units: vec![ready_unit("unit_0001", "work_item_0001", "unit_run_0001")],
        diagnostics: Vec::new(),
        created_at: String::new(),
    };

    assert!(matches!(
        store.write_group_final_readiness_snapshot(&attempt, &snapshot),
        Err(ProductStoreError::IdentityMismatch {
            kind: "group_final_readiness_snapshot",
            ..
        })
    ));

    snapshot.attempt_id = attempt.id.clone();
    store
        .write_group_final_readiness_snapshot(&attempt, &snapshot)
        .expect("write valid snapshot");
    snapshot.attempt_id = "../other-attempt".to_string();
    write_json(
        &store.group_final_readiness_snapshot_path(&attempt),
        &snapshot,
    )
    .expect("forge mismatched persisted snapshot");

    assert!(matches!(
        store.get_group_final_readiness_snapshot(&attempt),
        Err(ProductStoreError::IdentityMismatch {
            kind: "group_final_readiness_snapshot",
            ..
        })
    ));
}

#[test]
fn group_final_readiness_incomplete_snapshot_keeps_explicit_diagnostics() {
    let (_tmp, store, attempt) = setup();
    let mut unit = ready_unit("unit_0001", "work_item_0001", "unit_run_0001");
    unit.code_review_report_id = None;
    unit.review_verdict = None;
    unit.review_summary = None;
    unit.review_findings = None;
    unit.review_raw_provider_output_ref = None;
    let snapshot = GroupFinalReadinessSnapshot {
        attempt_id: attempt.id.clone(),
        status: GroupFinalReadinessStatus::Incomplete,
        units: vec![unit],
        diagnostics: vec![GroupFinalReadinessDiagnostic {
            kind: GroupFinalReadinessDiagnosticKind::CodeReviewMissing,
            unit_id: Some("unit_0001".to_string()),
            message: "unit_0001 is missing code-review evidence".to_string(),
        }],
        created_at: String::new(),
    };

    store
        .write_group_final_readiness_snapshot(&attempt, &snapshot)
        .expect("write incomplete snapshot");
    let restored = store
        .get_group_final_readiness_snapshot(&attempt)
        .expect("read incomplete snapshot")
        .expect("snapshot exists");

    assert_eq!(restored.status, GroupFinalReadinessStatus::Incomplete);
    assert_eq!(restored.diagnostics, snapshot.diagnostics);
    assert!(restored.units[0].review_verdict.is_none());
}

#[test]
fn group_final_readiness_returns_none_when_snapshot_does_not_exist() {
    let (_tmp, store, attempt) = setup();

    assert_eq!(
        store
            .get_group_final_readiness_snapshot(&attempt)
            .expect("read missing snapshot"),
        None
    );
}

#[test]
fn group_final_readiness_rejects_complete_snapshot_without_authoritative_evidence() {
    let (_tmp, store, attempt) = setup();
    let missing_evidence: [MissingEvidenceCase; 6] = [
        (
            "code_review_report_id",
            |unit: &mut GroupFinalReadinessUnit| unit.code_review_report_id = None,
        ),
        ("review_verdict", |unit: &mut GroupFinalReadinessUnit| {
            unit.review_verdict = None
        }),
        ("review_summary", |unit: &mut GroupFinalReadinessUnit| {
            unit.review_summary = None
        }),
        ("review_findings", |unit: &mut GroupFinalReadinessUnit| {
            unit.review_findings = None
        }),
        (
            "handoff_revision_id",
            |unit: &mut GroupFinalReadinessUnit| unit.handoff_revision_id = None,
        ),
        ("plan_revision_id", |unit: &mut GroupFinalReadinessUnit| {
            unit.plan_revision_id = None
        }),
    ];

    let empty_units = GroupFinalReadinessSnapshot {
        attempt_id: attempt.id.clone(),
        status: GroupFinalReadinessStatus::Complete,
        units: Vec::new(),
        diagnostics: Vec::new(),
        created_at: String::new(),
    };
    assert_invalid_group_final_readiness_snapshot(
        store
            .write_group_final_readiness_snapshot(&attempt, &empty_units)
            .expect_err("complete snapshot without units must be rejected"),
        "complete snapshot must include at least one unit",
    );

    for (field, clear_field) in missing_evidence {
        let mut unit = ready_unit("unit_0001", "work_item_0001", "unit_run_0001");
        clear_field(&mut unit);
        let snapshot = GroupFinalReadinessSnapshot {
            attempt_id: attempt.id.clone(),
            status: GroupFinalReadinessStatus::Complete,
            units: vec![unit],
            diagnostics: Vec::new(),
            created_at: String::new(),
        };

        assert_invalid_group_final_readiness_snapshot(
            store
                .write_group_final_readiness_snapshot(&attempt, &snapshot)
                .expect_err("complete snapshot without authoritative evidence must be rejected"),
            &format!("complete unit unit_0001 is missing {field}"),
        );
    }
}

#[test]
fn group_final_readiness_rejects_empty_or_blank_diagnostics_as_invalid_records() {
    let (_tmp, store, attempt) = setup();
    let empty_diagnostics = GroupFinalReadinessSnapshot {
        attempt_id: attempt.id.clone(),
        status: GroupFinalReadinessStatus::Incomplete,
        units: Vec::new(),
        diagnostics: Vec::new(),
        created_at: String::new(),
    };
    assert_invalid_group_final_readiness_snapshot(
        store
            .write_group_final_readiness_snapshot(&attempt, &empty_diagnostics)
            .expect_err("empty incomplete diagnostics must be rejected"),
        "incomplete snapshot must include diagnostics",
    );

    let blank_message = GroupFinalReadinessSnapshot {
        attempt_id: attempt.id.clone(),
        status: GroupFinalReadinessStatus::Incomplete,
        units: Vec::new(),
        diagnostics: vec![GroupFinalReadinessDiagnostic {
            kind: GroupFinalReadinessDiagnosticKind::CodeReviewMissing,
            unit_id: None,
            message: " \t ".to_string(),
        }],
        created_at: String::new(),
    };
    assert_invalid_group_final_readiness_snapshot(
        store
            .write_group_final_readiness_snapshot(&attempt, &blank_message)
            .expect_err("blank diagnostic messages must be rejected"),
        "diagnostic message must not be empty",
    );
}

#[test]
fn group_final_readiness_rejects_escaping_repair_target_ids() {
    let (_tmp, store, attempt) = setup();
    for repair_target in [
        RepairTarget {
            kind: RepairTargetKind::CurrentWorkItem,
            logical_work_item_ids: vec!["../work-item".to_string()],
            work_item_revision_ids: vec!["work_item_revision_0001".to_string()],
        },
        RepairTarget {
            kind: RepairTargetKind::CurrentWorkItem,
            logical_work_item_ids: vec!["work_item_0001".to_string()],
            work_item_revision_ids: vec!["../work-item-revision".to_string()],
        },
    ] {
        let mut unit = ready_unit("unit_0001", "work_item_0001", "unit_run_0001");
        let mut finding = final_readiness_finding();
        finding.repair_target = Some(repair_target);
        unit.review_findings = Some(vec![finding]);
        let snapshot = GroupFinalReadinessSnapshot {
            attempt_id: attempt.id.clone(),
            status: GroupFinalReadinessStatus::Complete,
            units: vec![unit],
            diagnostics: Vec::new(),
            created_at: String::new(),
        };

        assert!(matches!(
            store.write_group_final_readiness_snapshot(&attempt, &snapshot),
            Err(ProductStoreError::PathEscape(_))
        ));
    }
}

#[test]
fn group_final_readiness_rejects_escaping_artifact_references() {
    let (_tmp, store, attempt) = setup();
    let mut unit = ready_unit("unit_0001", "work_item_0001", "unit_run_0001");
    unit.review_raw_provider_output_ref = Some("../outside.txt".to_string());
    let snapshot = GroupFinalReadinessSnapshot {
        attempt_id: attempt.id.clone(),
        status: GroupFinalReadinessStatus::Complete,
        units: vec![unit],
        diagnostics: Vec::new(),
        created_at: String::new(),
    };

    assert!(matches!(
        store.write_group_final_readiness_snapshot(&attempt, &snapshot),
        Err(ProductStoreError::PathEscape(value)) if value == "../outside.txt"
    ));
}

#[test]
fn group_final_readiness_model_defaults_missing_future_fields() {
    let snapshot: GroupFinalReadinessSnapshot = read_json_from_str(
        r#"{
            "attempt_id": "coding_attempt_0001",
            "status": "incomplete",
            "units": [{
                "unit_id": "unit_0001",
                "logical_work_item_id": "work_item_0001",
                "unit_run_id": "unit_run_0001",
                "start_commit": "start_commit_0001",
                "completion_commit": "completion_commit_0001"
            }]
        }"#,
    );

    assert!(snapshot.diagnostics.is_empty());
    assert!(snapshot.created_at.is_empty());
    assert_eq!(
        snapshot.units[0].unit_run_id.as_deref(),
        Some("unit_run_0001")
    );
    assert_eq!(
        snapshot.units[0].start_commit.as_deref(),
        Some("start_commit_0001")
    );
    assert_eq!(
        snapshot.units[0].completion_commit.as_deref(),
        Some("completion_commit_0001")
    );
    assert!(!snapshot.units[0].empty_observation);
    assert_eq!(snapshot.units[0].commit_shas, Vec::<String>::new());
    assert!(snapshot.units[0].review_verdict.is_none());
}

fn assert_invalid_group_final_readiness_snapshot(error: ProductStoreError, expected_reason: &str) {
    assert!(matches!(
        error,
        ProductStoreError::InvalidRecord {
            kind: "group_final_readiness_snapshot",
            reason,
        } if reason == expected_reason
    ));
}

fn read_json_from_str(value: &str) -> GroupFinalReadinessSnapshot {
    serde_json::from_str(value).expect("deserialize readiness snapshot")
}
