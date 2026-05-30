use cadence_aria::task_run::openspec_bootstrap::{
    bootstrap_task_openspec, build_initial_constraint_bundle,
};
use cadence_aria::task_run::store::TaskRunStore;
use serde_json::json;
use std::fs;

#[test]
fn bootstraps_openspec_change_and_seeds_request_proposal() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let store = TaskRunStore::new(tempdir.path(), "task_0001");
    let task_state_path = store
        .write_task_state(&json!({
            "task_id": "task_0001",
            "phase": "intake",
            "change_id": "aria-login-jwt",
            "openspec_bootstrap_status": "bootstrap_pending"
        }))
        .expect("task state");

    let change_dir = bootstrap_task_openspec(
        tempdir.path(),
        "aria-login-jwt",
        "做一个用户登录功能",
        &task_state_path,
    )
    .expect("bootstrap openspec");

    let proposal = fs::read_to_string(change_dir.join("proposal.md")).expect("proposal");
    assert!(proposal.contains("做一个用户登录功能"));
    assert!(change_dir.join("specs/main/spec.md").exists());
    assert!(change_dir.join("design.md").exists());
    assert!(change_dir.join("tasks.md").exists());
}

#[test]
fn initial_constraint_bundle_contains_proposal_constraints_and_empty_later_constraints() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let store = TaskRunStore::new(tempdir.path(), "task_0001");
    let task_state_path = store
        .write_task_state(&json!({
            "task_id": "task_0001",
            "phase": "intake",
            "change_id": "aria-login-jwt",
            "openspec_bootstrap_status": "bootstrap_pending"
        }))
        .expect("task state");
    let change_dir = bootstrap_task_openspec(
        tempdir.path(),
        "aria-login-jwt",
        "做一个用户登录功能",
        &task_state_path,
    )
    .expect("bootstrap openspec");

    let bundle =
        build_initial_constraint_bundle("aria-login-jwt", &change_dir, "做一个用户登录功能")
            .expect("initial bundle");

    assert_eq!(bundle.change_id, "aria-login-jwt");
    let source_paths: Vec<_> = bundle
        .source_manifest
        .iter()
        .map(|source| source.path.as_str())
        .collect();
    for expected_path in [
        "openspec/changes/aria-login-jwt/proposal.md",
        "openspec/changes/aria-login-jwt/specs/main/spec.md",
        "openspec/changes/aria-login-jwt/design.md",
        "openspec/changes/aria-login-jwt/tasks.md",
    ] {
        assert!(
            source_paths.iter().any(|source_path| source_path
                .trim_start_matches(tempdir.path().to_string_lossy().as_ref())
                .trim_start_matches('/')
                == expected_path),
            "source_manifest should contain {expected_path}, got {source_paths:?}"
        );
    }
    assert_eq!(
        bundle.proposal_constraints.business_intent,
        vec!["做一个用户登录功能".to_string()]
    );
    assert!(bundle.requirement_constraints.requirement_ids.is_empty());
    assert!(bundle.requirement_constraints.scenario_ids.is_empty());
    assert!(
        bundle
            .requirement_constraints
            .success_criteria_ids
            .is_empty()
    );
    assert!(bundle.design_constraints.design_decision_ids.is_empty());
    assert!(bundle.design_constraints.component_ids.is_empty());
    assert!(bundle.design_constraints.risk_ids.is_empty());
    assert!(bundle.task_constraints.task_ids.is_empty());
    assert!(bundle.task_constraints.task_sequence.is_empty());
    assert!(
        bundle
            .task_constraints
            .related_requirement_ids_by_task
            .is_empty()
    );
    assert!(
        bundle
            .task_constraints
            .related_design_decision_ids_by_task
            .is_empty()
    );
    assert!(
        bundle
            .task_constraints
            .acceptance_target_ids_by_task
            .is_empty()
    );
    assert_eq!(bundle.compiled_by_node, "N03");
}

#[test]
fn rejects_change_id_that_escapes_openspec_changes() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let store = TaskRunStore::new(tempdir.path(), "task_0001");
    let task_state_path = store
        .write_task_state(&json!({
            "task_id": "task_0001",
            "phase": "intake",
            "change_id": "../outside",
            "openspec_bootstrap_status": "bootstrap_pending"
        }))
        .expect("task state");

    let error = bootstrap_task_openspec(
        tempdir.path(),
        "../outside",
        "做一个用户登录功能",
        &task_state_path,
    )
    .expect_err("escaping change id must fail");

    assert_eq!(error.code, "invalid_change_id");
    assert!(!tempdir.path().join("openspec/outside/proposal.md").exists());
}
