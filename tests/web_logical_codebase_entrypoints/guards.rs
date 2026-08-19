use axum::http::{Method, StatusCode};
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::json_store::write_json;
use cadence_aria::product::logical_codebase::{
    CodebaseMemberRecord, LogicalCodebaseManifest, LogicalCodebaseStore, LogicalRepositoryId,
    MemberStatus, RepositorySourceIdentity, RepositoryType,
};
use cadence_aria::product::models::RepositoryRecord;
use cadence_aria::product::project_store::ProjectStore;
use cadence_aria::web::app::build_web_router;
use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::state::WebAppState;
use serde_json::json;
use tempfile::tempdir;

use crate::{assert_error, request};
use uuid::Uuid;

fn member(id: LogicalRepositoryId, alias: &str) -> CodebaseMemberRecord {
    CodebaseMemberRecord {
        logical_repository_id: id,
        physical_repository_id: format!("repository_{alias}"),
        alias: alias.to_string(),
        role: "repository".to_string(),
        ordinal: 1,
        source_identity: RepositorySourceIdentity {
            scheme: "test".to_string(),
            key_digest: format!("digest-{alias}"),
            canonical_git_dir: format!("/tmp/{alias}/.git").into(),
            canonical_origin: None,
            first_seen_path_hash: format!("hash-{alias}"),
        },
        repo_type: RepositoryType::Unknown,
        tech_stack: Vec::new(),
        owner: None,
        tags: Vec::new(),
        default_ref: None,
        checkout_ids: Vec::new(),
        status: MemberStatus::Active,
        created_at: "2026-08-14T00:00:00Z".to_string(),
        updated_at: "2026-08-14T00:00:00Z".to_string(),
    }
}

#[tokio::test]
async fn single_repo_rejects_logical_codebase_routes_without_persisting_artifacts() {
    let root = tempdir().unwrap();
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let projects = ProjectStore::new(paths.clone());
    projects
        .create(cadence_aria::product::project_store::CreateProjectInput {
            name: "multi".into(),
            description: None,
        })
        .unwrap();
    projects
        .create(cadence_aria::product::project_store::CreateProjectInput {
            name: "single".into(),
            description: None,
        })
        .unwrap();
    let logical = LogicalCodebaseStore::new(paths.clone());
    let id = LogicalRepositoryId(Uuid::new_v4());
    logical
        .save_manifest(
            "project_0001",
            &LogicalCodebaseManifest::new("project_0001", root.path().into(), vec![id]),
        )
        .unwrap();
    logical
        .save_member("project_0001", &member(id, "api"))
        .unwrap();
    write_json(
        &paths.project_root("project_0002").join("repos.json"),
        &vec![RepositoryRecord {
            id: "repository_legacy".to_string(),
            project_id: "project_0002".to_string(),
            name: "legacy".to_string(),
            path: root.path().join("legacy"),
            repo_hash: "legacy-hash".to_string(),
            runtime_root: root.path().join("legacy/.aria/runtime"),
            default_policy_preset: "manual-write".to_string(),
            default_provider_mode: "fake".to_string(),
            created_at: "2026-08-18T00:00:00Z".to_string(),
            updated_at: "2026-08-18T00:00:00Z".to_string(),
            logical_repository_id: None,
            primary_checkout_id: None,
            identity_schema_version: 0,
        }],
    )
    .unwrap();
    write_json(
        &paths.project_root("project_0001").join("repos.json"),
        &vec![serde_json::json!({
            "id": "repository_single".to_string(),
            "project_id": "project_0001".to_string(),
            "name": "single".to_string(),
            "path": root.path().join("single"),
            "repo_hash": "single-hash".to_string(),
            "runtime_root": root.path().join("single/.aria/runtime"),
            "default_policy_preset": "manual-write".to_string(),
            "default_provider_mode": "fake".to_string(),
            "created_at": "2026-08-18T00:00:00Z".to_string(),
            "updated_at": "2026-08-18T00:00:00Z".to_string(),
        })],
    )
    .unwrap();

    let state = WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    );
    let app = build_web_router(state);

    // R6（v1.3 §4）：多仓 project 防护语义废除——GET /repositories 只列 repos.json
    // 单仓条目，逻辑成员不投影进来（成员经统一 /codebases 与成员端点呈现）。
    let (status, body) = request(
        &app,
        Method::GET,
        "/api/projects/project_0001/repositories",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let entries = body["repositories"].as_array().expect("repositories");
    assert_eq!(entries.len(), 1, "plain repos.json listing: {body}");
    assert_eq!(entries[0]["name"], "single");

    for (method, uri, body) in [
        (
            Method::POST,
            "/api/projects/project_0002/logical-codebase/initializations",
            json!({"idempotency_key":"single-repo-key"}),
        ),
        (
            Method::GET,
            "/api/projects/project_0002/logical-codebase/aggregate-indexes/active",
            json!({}),
        ),
        (
            Method::POST,
            "/api/projects/project_0002/logical-codebase/aggregate-indexes/rebuild",
            json!({}),
        ),
    ] {
        assert_error(
            request(&app, method, uri, body).await,
            StatusCode::NOT_FOUND,
            "logical_codebase_not_found",
        );
    }
    assert!(
        !paths.logical_codebase_root("project_0002").exists(),
        "single-repo guard must run before any manifest, batch, index, or operation is persisted"
    );

    let (status, issue) = request(
        &app,
        Method::POST,
        "/api/projects/project_0002/issues",
        json!({
            "repository_id": "repository_legacy",
            "title": "legacy issue"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "legacy issue response: {issue}");
    assert!(
        !paths
            .codebase_selection_path("project_0002", issue["issue_id"].as_str().unwrap())
            .exists(),
        "single-repository issue creation must not write codebase-selection.json"
    );
}
