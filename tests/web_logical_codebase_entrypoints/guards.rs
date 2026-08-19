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
use tower::ServiceExt;

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

/// R6 concern③ / R9 回归：v1.2 迁移 project（legacy 根 manifest + 成员 +
/// repos.json 兼容投影）经 DELETE /repositories 删除投影记录时，只移除
/// repos.json 条目（legacy 语义），LC 成员权威记录保持 active、逻辑条目
/// 仍在统一 codebases 列表中。
#[tokio::test]
async fn migrated_project_delete_repository_removes_only_projection_entry() {
    let root = tempdir().unwrap();
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let projects = ProjectStore::new(paths.clone());
    projects
        .create(cadence_aria::product::project_store::CreateProjectInput {
            name: "migrated".into(),
            description: None,
        })
        .unwrap();
    // v1.2 迁移形态：legacy 根权威 + repos.json 兼容投影。
    let logical = LogicalCodebaseStore::new(paths.clone());
    let id = LogicalRepositoryId(Uuid::new_v4());
    logical
        .save_manifest(
            "project_0001",
            &LogicalCodebaseManifest::new("project_0001", root.path().into(), vec![id]),
        )
        .unwrap();
    let migrated_member = member(id, "api");
    logical
        .save_member("project_0001", &migrated_member)
        .unwrap();
    write_json(
        &paths.project_root("project_0001").join("repos.json"),
        &vec![RepositoryRecord {
            id: "repository_api".to_string(),
            project_id: "project_0001".to_string(),
            name: "api".to_string(),
            path: root.path().join("api"),
            repo_hash: "api-hash".to_string(),
            runtime_root: root.path().join("api/.aria/runtime"),
            default_policy_preset: "manual-write".to_string(),
            default_provider_mode: "fake".to_string(),
            created_at: "2026-08-18T00:00:00Z".to_string(),
            updated_at: "2026-08-18T00:00:00Z".to_string(),
            logical_repository_id: Some(id),
            primary_checkout_id: None,
            identity_schema_version: 1,
        }],
    )
    .unwrap();

    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));

    // 统一列表先呈现逻辑条目（迁移别名 LC）。
    let (status, codebases) = request(
        &app,
        Method::GET,
        "/api/projects/project_0001/codebases",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{codebases}");
    assert!(
        codebases["codebases"]
            .as_array()
            .is_some_and(|entries| entries.iter().any(|entry| entry["kind"] == "logical")),
        "migrated logical codebase must stay listed: {codebases}"
    );

    // DELETE 投影记录：仅 repos.json 条目被移除（legacy 语义回执）。
    let delete = axum::http::Request::builder()
        .method(Method::DELETE)
        .uri("/api/projects/project_0001/repositories/repository_api")
        .header("content-type", "application/json")
        .header("Idempotency-Key", "migrated-delete-0001")
        .body(axum::body::Body::from("{}".to_string()))
        .unwrap();
    let response = app.clone().oneshot(delete).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let receipt: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 65536)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(receipt["legacy_delete"], true, "{receipt}");

    // repos.json 投影条目消失；LC 成员权威记录仍 active。
    let repositories: Vec<RepositoryRecord> = cadence_aria::product::json_store::read_json(
        &paths.project_root("project_0001").join("repos.json"),
    )
    .unwrap();
    assert!(repositories.is_empty(), "projection entry removed");
    let reloaded = LogicalCodebaseStore::new(paths.clone())
        .load_member("project_0001", id)
        .unwrap()
        .expect("member authority record survives");
    assert_eq!(reloaded.status, MemberStatus::Active);

    // 逻辑条目不受影响。
    let (status, codebases) = request(
        &app,
        Method::GET,
        "/api/projects/project_0001/codebases",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{codebases}");
    assert!(
        codebases["codebases"]
            .as_array()
            .is_some_and(|entries| entries.iter().any(|entry| entry["kind"] == "logical")),
        "logical entry must survive the projection delete: {codebases}"
    );
}
