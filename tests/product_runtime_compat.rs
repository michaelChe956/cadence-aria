use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::compatibility_scan::{
    CompatibilityScanInput, rebuild_index_from_runtime,
};
use cadence_aria::product::project_store::{CreateProjectInput, ProjectStore};
use cadence_aria::product::repository_store::RepositoryStore;
use serde_json::json;
use std::fs;
use std::path::Path;
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

fn write_state(task_root: &Path, change_id: Option<&str>) {
    fs::create_dir_all(task_root).expect("task dir");
    let mut state = json!({
        "task_id": "task_0001",
        "phase": "planning",
        "request_text": "Build project workbench"
    });
    if let Some(change_id) = change_id {
        state["change_id"] = json!(change_id);
    }

    fs::write(
        task_root.join("state.json"),
        serde_json::to_vec_pretty(&state).expect("json"),
    )
    .expect("state");
}

#[test]
fn rebuilds_issue_binding_from_existing_runtime_task() {
    let app = tempdir().expect("app");
    let repo = git_repo();
    let task_root = repo.path().join(".aria/runtime/tasks/task_0001");
    write_state(&task_root, Some("project-workbench"));

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

#[test]
fn keeps_existing_binding_when_runtime_change_id_changes() {
    let app = tempdir().expect("app");
    let repo = git_repo();
    let task_root = repo.path().join(".aria/runtime/tasks/task_0001");
    let app_paths = ProductAppPaths::new(app.path().join(".aria"));
    write_state(&task_root, Some("project-workbench"));

    rebuild_index_from_runtime(CompatibilityScanInput {
        app_paths: app_paths.clone(),
        repo_path: repo.path().to_path_buf(),
        project_name: "Recovered".to_string(),
    })
    .expect("first scan");

    write_state(&task_root, Some("renamed-change"));
    let second = rebuild_index_from_runtime(CompatibilityScanInput {
        app_paths,
        repo_path: repo.path().to_path_buf(),
        project_name: "Recovered".to_string(),
    })
    .expect("second scan");

    assert_eq!(second.issues_created, 0);
    assert_eq!(second.bindings_created, 0);
}

#[test]
fn creates_named_project_instead_of_reusing_unrelated_project() {
    let app = tempdir().expect("app");
    let repo = git_repo();
    let task_root = repo.path().join(".aria/runtime/tasks/task_0001");
    let app_paths = ProductAppPaths::new(app.path().join(".aria"));
    write_state(&task_root, Some("project-workbench"));

    ProjectStore::new(app_paths.clone())
        .create(CreateProjectInput {
            name: "Other".to_string(),
            description: None,
        })
        .expect("other project");

    let summary = rebuild_index_from_runtime(CompatibilityScanInput {
        app_paths: app_paths.clone(),
        repo_path: repo.path().to_path_buf(),
        project_name: "Recovered".to_string(),
    })
    .expect("scan");

    assert_eq!(summary.projects_created, 1);

    let repository_store = RepositoryStore::new(app_paths);
    let other_repo = repository_store
        .find_by_path("project_0001", repo.path())
        .expect("find other project repo");
    let recovered_repo = repository_store
        .find_by_path("project_0002", repo.path())
        .expect("find recovered project repo");

    assert!(other_repo.is_none());
    assert!(recovered_repo.is_some());
}

#[cfg(unix)]
#[test]
fn reports_tasks_root_metadata_error() {
    let app = tempdir().expect("app");
    let repo = git_repo();
    let runtime_root = repo.path().join(".aria/runtime");
    fs::create_dir_all(&runtime_root).expect("runtime root");
    std::os::unix::fs::symlink("tasks", runtime_root.join("tasks")).expect("tasks symlink");

    let error = rebuild_index_from_runtime(CompatibilityScanInput {
        app_paths: ProductAppPaths::new(app.path().join(".aria")),
        repo_path: repo.path().to_path_buf(),
        project_name: "Recovered".to_string(),
    })
    .expect_err("scan should report path error");
    let message = error.to_string();

    assert!(message.contains("try_exists"));
    assert!(message.contains(".aria/runtime/tasks"));
}
