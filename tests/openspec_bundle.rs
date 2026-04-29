use cadence_aria::cross_cutting::openspec_constraints::{
    DefaultDocumentOps, OpenSpecError, bootstrap_openspec_skeleton, build_openspec_source_manifest,
    check_bundle_stale, compile_constraint_bundle,
};
use cadence_aria::protocol::constraints::{
    BundleStatus, OpenSpecBootstrapStatus, OpenSpecSourceKind,
};
use serde_json::json;
use std::fs;
use std::path::Path;

#[test]
fn bootstrap_openspec_skeleton_creates_scope_files_and_marks_task_bootstrapped() {
    let workspace = tempfile::tempdir().expect("tempdir");
    let task_state_path = workspace
        .path()
        .join(".aria/runtime/tasks/task-001/task.json");
    fs::create_dir_all(task_state_path.parent().expect("task state parent"))
        .expect("create task state parent");
    fs::write(
        &task_state_path,
        serde_json::to_vec_pretty(&json!({
            "task_id": "task-001",
            "change_id": "sample-change",
            "openspec_bootstrap_status": "bootstrap_pending"
        }))
        .expect("task state json"),
    )
    .expect("write task state");

    let document_ops = DefaultDocumentOps;
    let status = bootstrap_openspec_skeleton(
        &"sample-change".to_string(),
        &task_state_path,
        &document_ops,
    )
    .expect("bootstrap skeleton");

    assert_eq!(status, OpenSpecBootstrapStatus::Bootstrapped);
    assert!(
        workspace
            .path()
            .join("openspec/changes/sample-change/proposal.md")
            .exists()
    );
    assert!(
        workspace
            .path()
            .join("openspec/changes/sample-change/specs/main/spec.md")
            .exists()
    );
    assert!(
        workspace
            .path()
            .join("openspec/changes/sample-change/design.md")
            .exists()
    );
    assert!(
        workspace
            .path()
            .join("openspec/changes/sample-change/tasks.md")
            .exists()
    );

    let task_state: serde_json::Value =
        serde_json::from_slice(&fs::read(&task_state_path).expect("read task state"))
            .expect("task state json");
    assert_eq!(
        task_state["openspec_bootstrap_status"],
        json!("bootstrapped")
    );

    let second = bootstrap_openspec_skeleton(
        &"sample-change".to_string(),
        &task_state_path,
        &document_ops,
    )
    .expect_err("bootstrap should be idempotency guarded by task state");
    assert_eq!(second, OpenSpecError::BootstrapAlreadyComplete);
}

#[test]
fn compile_constraint_bundle_extracts_manifest_and_constraints_from_fixture() {
    let manifest = build_openspec_source_manifest(sample_change_dir()).expect("manifest");

    assert_eq!(manifest.len(), 4);
    assert!(manifest.iter().all(|source| source.sha256.len() == 64));
    assert_eq!(manifest[0].kind, OpenSpecSourceKind::Proposal);
    assert_eq!(manifest[1].kind, OpenSpecSourceKind::Spec);
    assert_eq!(manifest[2].kind, OpenSpecSourceKind::Design);
    assert_eq!(manifest[3].kind, OpenSpecSourceKind::Tasks);

    let bundle = compile_constraint_bundle(
        &"sample-change".to_string(),
        &manifest,
        vec![],
        "N11".to_string(),
    )
    .expect("compile bundle");

    assert_eq!(bundle.bundle_status, BundleStatus::Ready);
    assert_eq!(bundle.change_id, "sample-change");
    assert_eq!(bundle.compiled_from_projection_refs, Vec::<String>::new());
    assert_eq!(
        bundle.proposal_constraints.business_intent,
        vec!["Users need to create runtime tasks from the REPL.".to_string()]
    );
    assert_eq!(
        bundle.proposal_constraints.scope,
        vec!["Compile task creation rules into the Phase 1 runtime contract.".to_string()]
    );
    assert_eq!(
        bundle.requirement_constraints.requirement_ids,
        vec!["REQ-001".to_string()]
    );
    assert_eq!(
        bundle.requirement_constraints.scenario_ids,
        vec!["SCN-001".to_string()]
    );
    assert_eq!(
        bundle.requirement_constraints.success_criteria_ids,
        vec!["AC-001".to_string()]
    );
    assert_eq!(
        bundle.design_constraints.design_decision_ids,
        vec!["DD-001".to_string()]
    );
    assert_eq!(
        bundle.design_constraints.component_ids,
        vec!["CMP-001".to_string()]
    );
    assert_eq!(
        bundle.design_constraints.risk_ids,
        vec!["RISK-001".to_string()]
    );
    assert_eq!(
        bundle.task_constraints.task_ids,
        vec!["TASK-001".to_string()]
    );
    assert_eq!(
        bundle
            .task_constraints
            .related_requirement_ids_by_task
            .get("TASK-001")
            .expect("task requirement refs"),
        &vec!["REQ-001".to_string()]
    );
    assert_eq!(
        bundle
            .task_constraints
            .related_design_decision_ids_by_task
            .get("TASK-001")
            .expect("task design refs"),
        &vec!["DD-001".to_string()]
    );
    assert_eq!(
        bundle
            .task_constraints
            .acceptance_target_ids_by_task
            .get("TASK-001")
            .expect("task acceptance refs"),
        &vec!["AC-001".to_string()]
    );
    assert_eq!(
        bundle.traceability_requirements.required_requirement_ids,
        vec!["REQ-001".to_string()]
    );
    assert_eq!(
        bundle.coverage_model.required_ids,
        vec![
            "AC-001".to_string(),
            "DD-001".to_string(),
            "REQ-001".to_string(),
            "TASK-001".to_string()
        ]
    );
}

#[test]
fn check_bundle_stale_detects_hash_change_and_missing_source() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let change_dir = tempdir.path().join("openspec/changes/sample-change");
    copy_dir(sample_change_dir(), &change_dir);

    let manifest = build_openspec_source_manifest(&change_dir).expect("manifest");
    let bundle = compile_constraint_bundle(
        &"sample-change".to_string(),
        &manifest,
        vec![],
        "N11".to_string(),
    )
    .expect("compile bundle");

    fs::write(
        change_dir.join("tasks.md"),
        "- [ ] TASK-001 Implement runtime task creation. Reqs: REQ-001; Designs: DD-001; Acceptance: AC-001\n- [ ] TASK-002 Verify stale detection. Reqs: REQ-001; Designs: DD-001; Acceptance: AC-001\n",
    )
    .expect("mutate tasks");
    let changed_manifest = build_openspec_source_manifest(&change_dir).expect("changed manifest");
    assert_eq!(
        check_bundle_stale(&bundle, &changed_manifest),
        BundleStatus::Stale
    );

    let mut missing_manifest = manifest;
    missing_manifest.pop();
    assert_eq!(
        check_bundle_stale(&bundle, &missing_manifest),
        BundleStatus::Blocked
    );
}

#[test]
fn compile_constraint_bundle_maps_missing_or_empty_sources_to_blocking_errors() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let change_dir = tempdir.path().join("openspec/changes/sample-change");
    copy_dir(sample_change_dir(), &change_dir);

    fs::remove_file(change_dir.join("design.md")).expect("remove design");
    let manifest = build_openspec_source_manifest(&change_dir).expect("manifest");
    let missing = compile_constraint_bundle(
        &"sample-change".to_string(),
        &manifest,
        vec![],
        "N11".to_string(),
    )
    .expect_err("missing design should block N11");
    assert_eq!(
        missing,
        OpenSpecError::SourceMissing {
            kind: OpenSpecSourceKind::Design,
            blocked_node: "N11".to_string()
        }
    );

    fs::write(
        change_dir.join("design.md"),
        "# Design\n\n## Decisions\n\nNo explicit decisions yet.\n",
    )
    .expect("write empty design");
    let manifest = build_openspec_source_manifest(&change_dir).expect("manifest");
    let empty_design = compile_constraint_bundle(
        &"sample-change".to_string(),
        &manifest,
        vec![],
        "N11".to_string(),
    )
    .expect_err("design must contain decision or component ids");
    assert_eq!(empty_design, OpenSpecError::DesignConstraintsEmpty);
}

fn sample_change_dir() -> &'static Path {
    Path::new("tests/fixtures/openspec/changes/sample-change")
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("create target dir");
    for entry in fs::read_dir(from).expect("read source dir") {
        let entry = entry.expect("dir entry");
        let source_path = entry.path();
        let target_path = to.join(entry.file_name());
        if source_path.is_dir() {
            copy_dir(&source_path, &target_path);
        } else {
            if let Some(parent) = target_path.parent() {
                fs::create_dir_all(parent).expect("create file parent");
            }
            fs::copy(&source_path, &target_path).expect("copy file");
        }
    }
}
