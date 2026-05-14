use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::compatibility_scan::{
    CompatibilityScanInput, rebuild_index_from_runtime,
};
use serde_json::json;
use std::fs;
use std::process::Command;
use tempfile::tempdir;

fn git_repo() -> tempfile::TempDir {
    let dir = tempdir().expect("repo");
    assert!(
        Command::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .status()
            .expect("git init")
            .success()
    );
    dir
}

#[test]
fn rebuilds_issue_binding_from_existing_runtime_task() {
    let app = tempdir().expect("app");
    let repo = git_repo();
    let task_root = repo.path().join(".aria/runtime/tasks/task_0001");
    fs::create_dir_all(&task_root).expect("task dir");
    fs::write(
        task_root.join("state.json"),
        serde_json::to_vec_pretty(&json!({
            "task_id": "task_0001",
            "phase": "planning",
            "change_id": "project-workbench",
            "request_text": "Build project workbench"
        }))
        .expect("json"),
    )
    .expect("state");

    let summary = rebuild_index_from_runtime(CompatibilityScanInput {
        app_paths: ProductAppPaths::new(app.path().join(".aria")),
        repo_path: repo.path().to_path_buf(),
        project_name: "Recovered".to_string(),
    })
    .expect("scan");

    assert_eq!(summary.projects_created, 1);
    assert_eq!(summary.repositories_created, 1);
    assert_eq!(summary.issues_created, 1);
    assert_eq!(summary.bindings_created, 1);

    let second = rebuild_index_from_runtime(CompatibilityScanInput {
        app_paths: ProductAppPaths::new(app.path().join(".aria")),
        repo_path: repo.path().to_path_buf(),
        project_name: "Recovered".to_string(),
    })
    .expect("second scan");

    assert_eq!(second.issues_created, 0);
    assert_eq!(second.bindings_created, 0);
}
