use cadence_aria::task_run::store::{TaskRunStore, preflight_workspace};
use serde_json::json;
use std::fs;
use std::process::Command;

#[test]
fn preflight_requires_git_worktree_and_openspec_config() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    fs::create_dir_all(tempdir.path().join("openspec")).expect("openspec dir");
    fs::write(
        tempdir.path().join("openspec/config.yaml"),
        "project: naruto\n",
    )
    .expect("openspec config");
    git(tempdir.path(), &["init", "-b", "main"]);

    let result = preflight_workspace(tempdir.path()).expect("preflight");

    assert_eq!(result.workspace_root, tempdir.path());
    assert!(result.openspec_config.ends_with("openspec/config.yaml"));
}

#[test]
fn preflight_rejects_workspace_without_openspec_config() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    git(tempdir.path(), &["init", "-b", "main"]);

    let error = preflight_workspace(tempdir.path()).expect_err("missing openspec config");

    assert_eq!(error.code, "openspec_config_missing");
}

#[test]
fn store_initializes_task_root_state_and_report() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let store = TaskRunStore::new(tempdir.path(), "task_0001");

    let task_state_path = store
        .write_task_state(&json!({
            "task_id": "task_0001",
            "phase": "intake",
            "change_id": "aria-login-jwt",
            "openspec_bootstrap_status": "bootstrap_pending"
        }))
        .expect("write state");
    let report_path = store
        .write_json_report(
            "final-report.json",
            &json!({
                "task_id": "task_0001",
                "status": "completed"
            }),
        )
        .expect("write report");

    assert!(task_state_path.ends_with(".aria/runtime/tasks/task_0001/state.json"));
    assert!(task_state_path.exists());
    assert!(report_path.ends_with(".aria/runtime/tasks/task_0001/reports/final-report.json"));
    assert!(report_path.exists());
}

fn git(cwd: &std::path::Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("git command");
    assert!(
        output.status.success(),
        "git {:?} failed\nstdout={}\nstderr={}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
