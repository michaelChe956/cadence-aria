use cadence_aria::cross_cutting::traceability::{
    TraceabilityError, TraceabilityIndexes, check_coverage_closed, normalize_traceability,
};
use cadence_aria::protocol::projections::{ExecutionMode, PlanProjection, WorkPackageProjection};
use cadence_aria::protocol::traceability::{BindingStatus, CoverageStatus, ManualExemption};
use serde_json::{Value, json};

#[test]
fn normalize_traceability_merges_plan_refs_and_known_candidate_refs() {
    let plan = plan_projection();
    let dispatch = dispatch_package("wt-001");
    let mut report = coding_report("work_001");
    let indexes = TraceabilityIndexes::new(vec![
        "req-001".to_string(),
        "dd-001".to_string(),
        "task-001".to_string(),
        "risk-001".to_string(),
    ]);

    let binding = normalize_traceability(
        &mut report,
        vec!["risk-001".to_string(), "req-999".to_string()],
        &dispatch,
        &plan,
        &indexes,
    )
    .expect("normalize traceability");

    assert_eq!(
        report["_aria"]["traceability_refs"],
        json!(["req-001", "dd-001", "task-001", "risk-001"])
    );
    assert_eq!(binding.related_requirement_ids, vec!["req-001"]);
    assert_eq!(binding.related_design_decision_ids, vec!["dd-001"]);
    assert_eq!(binding.related_task_ids, vec!["task-001"]);
    assert_eq!(binding.related_risk_ids, vec!["risk-001"]);
    assert_eq!(binding.coverage_status, CoverageStatus::Covered);
    assert_eq!(binding.binding_status, BindingStatus::Conflict);

    let conflict: Value = serde_json::from_str(
        binding
            .conflict_reason
            .as_ref()
            .expect("conflict reason json"),
    )
    .expect("conflict json");
    assert_eq!(conflict["reason_codes"], json!(["unknown_ref"]));
    assert_eq!(conflict["rejected_refs"], json!(["req-999"]));
}

#[test]
fn normalize_traceability_rejects_missing_source_work_package() {
    let plan = plan_projection();
    let dispatch = dispatch_package("wt-404");
    let mut report = coding_report("work_001");
    let indexes = TraceabilityIndexes::new(vec!["req-001".to_string()]);

    let error = normalize_traceability(&mut report, vec![], &dispatch, &plan, &indexes)
        .expect_err("source work package must map to PlanProjection.work_packages");

    assert_eq!(
        error,
        TraceabilityError::WorkPackageNotFound("wt-404".to_string())
    );
}

#[test]
fn coverage_checker_reports_closed_uncovered_and_manual_exemptions() {
    let plan = plan_projection();
    let dispatch = dispatch_package("wt-001");
    let mut report = coding_report("work_001");
    let indexes = TraceabilityIndexes::new(vec![
        "req-001".to_string(),
        "dd-001".to_string(),
        "task-001".to_string(),
    ]);
    let binding =
        normalize_traceability(&mut report, vec![], &dispatch, &plan, &indexes).expect("binding");

    let coverage = check_coverage_closed(
        &["req-001".to_string(), "req-002".to_string()],
        &["dd-001".to_string()],
        &["task-001".to_string(), "task-002".to_string()],
        &[binding],
        &[ManualExemption {
            item_id: "task-002".to_string(),
            reason: "由人工 gate 延后处理".to_string(),
            approved_by: Some("user".to_string()),
        }],
    );

    assert_eq!(coverage.closed, vec!["dd-001", "req-001", "task-001"]);
    assert_eq!(coverage.uncovered, vec!["req-002"]);
    assert_eq!(coverage.exempted, vec!["task-002"]);
    assert_eq!(coverage.manual_exemptions.len(), 1);
}

fn plan_projection() -> PlanProjection {
    PlanProjection {
        work_packages: vec![WorkPackageProjection {
            work_package_id: "wt-001".to_string(),
            description: "实现 REPL wire schema".to_string(),
            execution_mode: ExecutionMode::AgentOnly,
            human_required_reason: None,
            traceability_refs: vec![
                "req-001".to_string(),
                "dd-001".to_string(),
                "task-001".to_string(),
            ],
            acceptance_targets: vec!["ac-001".to_string()],
        }],
        dependencies: vec![],
        parallelism_groups: vec![],
    }
}

fn dispatch_package(source_work_package_id: &str) -> Value {
    json!({
        "artifact_kind": "dispatch_package",
        "_aria": {
            "worktask_routing": [
                {
                    "worktask_id": "work_001",
                    "source_work_package_id": source_work_package_id,
                    "execution_mode": "agent_only",
                    "allowed_write_scope": ["src/"],
                    "traceability_refs": ["req-001"],
                    "verification_commands": ["cargo test -j 1"]
                }
            ]
        }
    })
}

fn coding_report(worktask_id: &str) -> Value {
    json!({
        "artifact_kind": "coding_report",
        "artifact_ref": "art_ref_coding_report_0001",
        "worktask_id": worktask_id,
        "files_modified": ["src/lib.rs"],
        "commands_run": ["cargo test -j 1"],
        "candidate_traceability_refs": [],
        "status": "completed",
        "_aria": {
            "profile_version": "phase1.v1"
        }
    })
}
