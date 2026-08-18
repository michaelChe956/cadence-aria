use axum::http::{Method, StatusCode};
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::logical_codebase::{
    CodebaseMemberRecord, LogicalCodebaseManifest, LogicalCodebaseStore, LogicalRepositoryId,
    MemberStatus, RepositorySourceIdentity, RepositoryType,
};
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
async fn multi_repo_blocks_legacy_mutation_projects_members_and_single_repo_rejects_existing_logical_route()
 {
    let root = tempdir().unwrap();
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let projects = ProjectStore::new(paths.clone());
    projects
        .create(cadence_aria::product::project_store::CreateProjectInput {
            name: "multi".into(),
            description: None,
            multi_repo: true,
        })
        .unwrap();
    projects
        .create(cadence_aria::product::project_store::CreateProjectInput {
            name: "single".into(),
            description: None,
            multi_repo: false,
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

    let state = WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    );
    let app = build_web_router(state);
    let body = assert_error(
        request(
            &app,
            Method::POST,
            "/api/projects/project_0001/repositories",
            json!({}),
        )
        .await,
        StatusCode::CONFLICT,
        "legacy_repository_endpoint_on_multi_repo",
    );
    assert!(
        body["message"]
            .as_str()
            .unwrap()
            .contains("请使用逻辑代码库登记端点")
    );

    let body = assert_error(
        request(
            &app,
            Method::DELETE,
            "/api/projects/project_0001/repositories/repository_0001",
            json!({}),
        )
        .await,
        StatusCode::CONFLICT,
        "legacy_repository_endpoint_on_multi_repo",
    );
    assert!(
        body["message"]
            .as_str()
            .unwrap()
            .contains("请使用逻辑代码库登记端点")
    );

    let (status, body) = request(
        &app,
        Method::GET,
        "/api/projects/project_0001/repositories",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["repositories"][0]["name"], "api");

    let body = assert_error(
        request(
            &app,
            Method::GET,
            "/api/projects/project_0001/repository-initializations/operation_0001",
            json!({}),
        )
        .await,
        StatusCode::CONFLICT,
        "legacy_repository_endpoint_on_multi_repo",
    );
    assert!(
        body["message"]
            .as_str()
            .unwrap()
            .contains("请使用逻辑代码库登记端点")
    );

    assert_error(
        request(
            &app,
            Method::POST,
            "/api/projects/project_0002/logical-codebase/initializations",
            json!({"idempotency_key":"single-repo-key"}),
        )
        .await,
        StatusCode::CONFLICT,
        "logical_codebase_feature_disabled",
    );
    assert!(
        !paths.logical_codebase_root("project_0002").exists(),
        "single-repo guard must run before any manifest, batch, index, or operation is persisted"
    );
}
