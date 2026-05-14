use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::id::repo_hash_for_path;
use cadence_aria::product::models::{IssuePhase, IssueStatus, ProjectRecord};
use tempfile::tempdir;

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
