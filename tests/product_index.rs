use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::id::repo_hash_for_path;
use cadence_aria::product::issue_store::{CreateProductIssueInput, IssueStore};
use cadence_aria::product::json_store::{read_json, write_json};
use cadence_aria::product::models::{IssuePhase, IssueStatus, ProjectRecord};
use cadence_aria::product::project_store::{CreateProjectInput, ProjectStore};
use cadence_aria::product::repository_store::{CreateRepositoryInput, RepositoryStore};
use cadence_aria::product::runtime_binding_store::{
    CreateRuntimeBindingInput, RuntimeBindingStore,
};
use std::process::Command;
use tempfile::tempdir;

fn git_repo() -> tempfile::TempDir {
    let dir = tempdir().expect("tempdir");
    let status = Command::new("git")
        .args(["init"])
        .current_dir(dir.path())
        .status()
        .expect("git init");
    assert!(status.success());
    dir
}

#[test]
fn product_paths_resolve_under_injected_app_root() {
    let root = tempdir().expect("tempdir");
    let paths = ProductAppPaths::new(root.path().join(".aria"));

    assert_eq!(paths.root(), root.path().join(".aria"));
    assert_eq!(paths.projects_root(), root.path().join(".aria/projects"));
    assert_eq!(
        paths.last_project_path(),
        root.path().join(".aria/state/last-project.json")
    );
}

#[test]
fn repo_hash_is_stable_for_canonical_path_text() {
    let hash_a = repo_hash_for_path("/tmp/example-repo");
    let hash_b = repo_hash_for_path("/tmp/example-repo");
    let hash_c = repo_hash_for_path("/tmp/other-repo");

    assert_eq!(hash_a, hash_b);
    assert_ne!(hash_a, hash_c);
    assert_eq!(hash_a.len(), 12);
}

#[test]
fn project_and_issue_enums_serialize_as_snake_case() {
    let project = ProjectRecord {
        id: "project_0001".to_string(),
        name: "Aria".to_string(),
        description: Some("project workbench".to_string()),
        created_at: "2026-05-14T00:00:00Z".to_string(),
        updated_at: "2026-05-14T00:00:00Z".to_string(),
        last_opened_at: None,
    };

    let project_json = serde_json::to_value(project).expect("project json");
    assert_eq!(project_json["id"], "project_0001");

    let phase = serde_json::to_string(&IssuePhase::Clarification).expect("phase");
    let status = serde_json::to_string(&IssueStatus::InProgress).expect("status");

    assert_eq!(phase, "\"clarification\"");
    assert_eq!(status, "\"in_progress\"");
}

#[test]
fn creates_project_repository_issue_and_runtime_binding() {
    let app = tempdir().expect("app");
    let repo = git_repo();
    let paths = ProductAppPaths::new(app.path().join(".aria"));

    let project = ProjectStore::new(paths.clone())
        .create(CreateProjectInput {
            name: "Aria".to_string(),
            description: Some("Workbench".to_string()),
        })
        .expect("project");

    let repository = RepositoryStore::new(paths.clone())
        .create(CreateRepositoryInput {
            project_id: project.id.clone(),
            name: "cadence-aria".to_string(),
            path: repo.path().to_path_buf(),
            default_policy_preset: None,
            default_provider_mode: None,
        })
        .expect("repository");

    let repos_path = paths.project_root(&project.id).join("repos.json");
    assert!(repos_path.exists());
    assert!(
        !paths
            .project_root(&project.id)
            .join(format!("repositories/{}.json", repository.id))
            .exists()
    );

    let repositories = RepositoryStore::new(paths.clone())
        .list(&project.id)
        .expect("repositories");
    assert_eq!(repositories, vec![repository.clone()]);

    let found_repository = RepositoryStore::new(paths.clone())
        .find_by_path(&project.id, repo.path())
        .expect("find repository");
    assert_eq!(
        found_repository.as_ref().map(|record| &record.id),
        Some(&repository.id)
    );

    let issue = IssueStore::new(paths.clone())
        .create(CreateProductIssueInput {
            project_id: project.id.clone(),
            repo_id: Some(repository.id.clone()),
            title: "Add project workbench".to_string(),
            description: Some("Manage issues".to_string()),
            change_id: None,
        })
        .expect("issue");

    let binding = RuntimeBindingStore::new(paths.clone())
        .create(CreateRuntimeBindingInput {
            project_id: project.id.clone(),
            issue_id: issue.id.clone(),
            repo_id: repository.id.clone(),
            change_id: issue.change_id.clone(),
            task_id: Some("task_0001".to_string()),
            session_id: Some("sess_task_0001".to_string()),
            runtime_root: repository.runtime_root.clone(),
        })
        .expect("binding");

    assert_eq!(repository.project_id, project.id);
    assert_eq!(issue.repo_id.as_deref(), Some(repository.id.as_str()));
    assert_eq!(binding.issue_id, issue.id);
    assert_eq!(
        binding.task_root.as_deref(),
        Some(repository.runtime_root.join("tasks/task_0001").as_path())
    );

    let found_binding = RuntimeBindingStore::new(paths)
        .find_by_repo_and_task(&project.id, &issue.id, &repository.id, "task_0001")
        .expect("find binding");
    assert_eq!(
        found_binding.as_ref().map(|record| &record.id),
        Some(&binding.id)
    );
}

#[test]
fn find_by_repo_and_task_returns_error_for_corrupt_binding_json() {
    let app = tempdir().expect("app");
    let repo = git_repo();
    let paths = ProductAppPaths::new(app.path().join(".aria"));

    let project = ProjectStore::new(paths.clone())
        .create(CreateProjectInput {
            name: "Aria".to_string(),
            description: Some("Workbench".to_string()),
        })
        .expect("project");

    let repository = RepositoryStore::new(paths.clone())
        .create(CreateRepositoryInput {
            project_id: project.id.clone(),
            name: "cadence-aria".to_string(),
            path: repo.path().to_path_buf(),
            default_policy_preset: None,
            default_provider_mode: None,
        })
        .expect("repository");

    let issue = IssueStore::new(paths.clone())
        .create(CreateProductIssueInput {
            project_id: project.id.clone(),
            repo_id: Some(repository.id.clone()),
            title: "Add project workbench".to_string(),
            description: Some("Manage issues".to_string()),
            change_id: None,
        })
        .expect("issue");

    let binding = RuntimeBindingStore::new(paths.clone())
        .create(CreateRuntimeBindingInput {
            project_id: project.id.clone(),
            issue_id: issue.id.clone(),
            repo_id: repository.id.clone(),
            change_id: issue.change_id.clone(),
            task_id: Some("task_0001".to_string()),
            session_id: Some("sess_task_0001".to_string()),
            runtime_root: repository.runtime_root.clone(),
        })
        .expect("binding");

    let corrupt_path = paths
        .issue_root(&project.id, &issue.id)
        .join("bindings")
        .join("binding_9999.json");
    std::fs::create_dir_all(corrupt_path.parent().expect("binding dir")).expect("create dir");
    std::fs::write(&corrupt_path, "{not valid json").expect("write corrupt json");

    let result = RuntimeBindingStore::new(paths).find_by_repo_and_task(
        &project.id,
        &issue.id,
        &repository.id,
        "task_0001",
    );

    assert!(result.is_err());
    assert_eq!(binding.id, "binding_0001");
}

#[test]
fn write_json_overwrites_existing_file_without_leftover_temp_files() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("value.json");

    write_json(&path, &serde_json::json!({ "version": 1 })).expect("write initial");
    write_json(&path, &serde_json::json!({ "version": 2 })).expect("overwrite");

    let value: serde_json::Value = read_json(&path).expect("read overwritten");
    assert_eq!(value["version"], 2);

    let mut entries = std::fs::read_dir(dir.path())
        .expect("dir entries")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .to_string()
        })
        .collect::<Vec<_>>();
    entries.sort();
    assert_eq!(entries, vec!["value.json".to_string()]);
}

#[test]
fn write_json_error_does_not_remove_existing_target_directory() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("value.json");
    std::fs::create_dir(&path).expect("target dir");

    let result = write_json(&path, &serde_json::json!({ "version": 1 }));

    assert!(result.is_err());
    assert!(path.is_dir());
}

#[test]
fn product_store_error_paths_do_not_drop_iteration_errors_or_delete_targets() {
    let runtime_binding_store = include_str!("../src/product/runtime_binding_store.rs");
    assert!(!runtime_binding_store.contains("filter_map(|entry| entry.ok()"));

    let json_store = include_str!("../src/product/json_store.rs");
    assert!(!json_store.contains("remove_file(target_path)"));
}
