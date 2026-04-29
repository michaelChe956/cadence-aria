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
    assert_eq!(
        bundle.proposal_constraints.business_intent,
        vec!["做一个用户登录功能".to_string()]
    );
    assert!(bundle.requirement_constraints.requirement_ids.is_empty());
    assert!(bundle.design_constraints.design_decision_ids.is_empty());
    assert!(bundle.task_constraints.task_ids.is_empty());
    assert_eq!(bundle.compiled_by_node, "N03");
}
