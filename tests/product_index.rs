use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::id::repo_hash_for_path;
use cadence_aria::product::issue_store::{CreateProductIssueInput, IssueStore};
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

    let issue = IssueStore::new(paths.clone())
        .create(CreateProductIssueInput {
            project_id: project.id.clone(),
            repo_id: repository.id.clone(),
            title: "Add project workbench".to_string(),
            description: Some("Manage issues".to_string()),
            change_id: None,
        })
        .expect("issue");

    let binding = RuntimeBindingStore::new(paths)
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
    assert_eq!(issue.repo_id, repository.id);
    assert_eq!(binding.issue_id, issue.id);
    assert_eq!(
        binding.task_root.as_deref(),
        Some(repository.runtime_root.join("tasks/task_0001").as_path())
    );
}
