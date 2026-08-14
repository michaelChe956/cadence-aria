//! pointer-publications Web API（Task 10 契约 5 端点）。
//!
//! 端点前缀 `/api/projects/{project_id}/logical-codebase/pointer-publications`：
//! POST 创建批次（body `{ batch_kind }`）、GET 列表 / 单个、POST retry-repo
//! （body `{ member_repo_id }`）、POST revoke。稳定码映射见
//! `pointer_publish_error_mapping.rs`。

use super::pointer_publish_error_mapping::{pointer_publish_api_error, pointer_store_api_error};
use super::support::{product_app_paths, product_store_api_error};

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use crate::product::app_paths::ProductAppPaths;
use crate::product::json_store::validate_relative_id;
use crate::product::logical_codebase::{
    LogicalCodebaseManifest, PointerPublicationBatchKind, PointerPublicationStore,
    PointerPublishCoordinator,
};
use crate::web::error::{ApiError, ApiResult};
use crate::web::state::WebAppState;

#[derive(Debug, Deserialize)]
pub struct CreatePointerPublicationRequest {
    pub batch_kind: PointerPublicationBatchKind,
}

#[derive(Debug, Deserialize)]
pub struct RetryPointerPublicationRepoRequest {
    pub member_repo_id: String,
}

pub async fn create_pointer_publication(
    State(state): State<WebAppState>,
    Path(project_id): Path<String>,
    Json(request): Json<CreatePointerPublicationRequest>,
) -> ApiResult<Response> {
    validate_project_id(&project_id)?;
    let paths = product_app_paths(&state);
    let manifest = load_manifest(&paths, &project_id)?;
    let coordinator = PointerPublishCoordinator::new(paths);
    let publication = coordinator
        .publish_all(
            &project_id,
            &manifest.logical_codebase_id.to_string(),
            request.batch_kind,
        )
        .await
        .map_err(pointer_publish_api_error)?;
    Ok((StatusCode::OK, Json(publication)).into_response())
}

pub async fn list_pointer_publications(
    State(state): State<WebAppState>,
    Path(project_id): Path<String>,
) -> ApiResult<Response> {
    validate_project_id(&project_id)?;
    let store = PointerPublicationStore::new(product_app_paths(&state));
    let publications = store
        .list_publications(&project_id)
        .map_err(pointer_store_api_error)?;
    Ok(Json(publications).into_response())
}

pub async fn get_pointer_publication(
    State(state): State<WebAppState>,
    Path((project_id, publication_id)): Path<(String, String)>,
) -> ApiResult<Response> {
    validate_project_id(&project_id)?;
    validate_publication_id(&publication_id)?;
    let store = PointerPublicationStore::new(product_app_paths(&state));
    let publication = store
        .load_publication(&project_id, &publication_id)
        .map_err(pointer_store_api_error)?;
    Ok(Json(publication).into_response())
}

pub async fn retry_pointer_publication_repo(
    State(state): State<WebAppState>,
    Path((project_id, publication_id)): Path<(String, String)>,
    Json(request): Json<RetryPointerPublicationRepoRequest>,
) -> ApiResult<Response> {
    validate_project_id(&project_id)?;
    validate_publication_id(&publication_id)?;
    validate_relative_id(&request.member_repo_id).map_err(|error| {
        ApiError::validation(
            "invalid_pointer_request",
            format!("invalid member_repo_id: {error}"),
        )
    })?;
    let coordinator = PointerPublishCoordinator::new(product_app_paths(&state));
    let publication = coordinator
        .retry_member_repo(&project_id, &publication_id, &request.member_repo_id)
        .await
        .map_err(pointer_publish_api_error)?;
    Ok((StatusCode::OK, Json(publication)).into_response())
}

pub async fn revoke_pointer_publication(
    State(state): State<WebAppState>,
    Path((project_id, publication_id)): Path<(String, String)>,
) -> ApiResult<Response> {
    validate_project_id(&project_id)?;
    validate_publication_id(&publication_id)?;
    let coordinator = PointerPublishCoordinator::new(product_app_paths(&state));
    let publication = coordinator
        .revoke(&project_id, &publication_id)
        .await
        .map_err(pointer_publish_api_error)?;
    Ok((StatusCode::OK, Json(publication)).into_response())
}

fn load_manifest(paths: &ProductAppPaths, project_id: &str) -> ApiResult<LogicalCodebaseManifest> {
    let store = crate::product::logical_codebase::LogicalCodebaseStore::new(paths.clone());
    store
        .load_manifest(project_id)
        .map_err(product_store_api_error)?
        .ok_or_else(|| {
            ApiError::runtime(
                "pointer_not_found",
                "logical codebase manifest is missing; register members first",
                serde_json::json!({}),
            )
        })
}

fn validate_project_id(project_id: &str) -> ApiResult<()> {
    validate_relative_id(project_id).map_err(|error| {
        ApiError::validation("invalid_project_id", format!("invalid project id: {error}"))
    })
}

fn validate_publication_id(publication_id: &str) -> ApiResult<()> {
    validate_relative_id(publication_id).map_err(|error| {
        ApiError::validation(
            "invalid_pointer_request",
            format!("invalid publication id: {error}"),
        )
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::product::logical_codebase::{
        CheckoutAvailability, CheckoutKind, CodebaseMemberRecord, LogicalRepositoryId,
        MemberStatus, PointerPublication, PointerPublicationBatchKind, PointerPublicationEntry,
        PointerPublicationEntryState, PointerPublicationStatus, RepositoryCheckoutId,
        RepositoryCheckoutRecord, RepositorySourceIdentity, RepositoryType,
    };
    use crate::web::app::build_web_router;
    use crate::web::runtime::WebRuntime;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use std::path::Path as StdPath;
    use std::process::Command as StdCommand;
    use tempfile::tempdir;
    use tower::ServiceExt;

    fn git(repo: &StdPath, args: &[&str]) {
        let output = StdCommand::new("git")
            .args(args)
            .current_dir(repo)
            .output()
            .expect("run git fixture command");
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    struct Fixture {
        root: tempfile::TempDir,
        member_repo_id: String,
    }

    fn setup() -> Fixture {
        let root = tempdir().expect("root");
        let member_repo_id = LogicalRepositoryId(uuid::Uuid::new_v4());
        let checkout_id = RepositoryCheckoutId(uuid::Uuid::new_v4());
        let repo_path = root.path().join("api");
        std::fs::create_dir_all(&repo_path).unwrap();
        git(&repo_path, &["init"]);
        git(&repo_path, &["config", "user.email", "test@example.com"]);
        git(&repo_path, &["config", "user.name", "Test User"]);
        std::fs::write(repo_path.join("README.md"), "base\n").unwrap();
        git(&repo_path, &["add", "README.md"]);
        git(&repo_path, &["commit", "-m", "base"]);
        let remote_path = root.path().join("api-origin.git");
        std::fs::create_dir_all(&remote_path).unwrap();
        git(&remote_path, &["init", "--bare"]);
        git(
            &repo_path,
            &["remote", "add", "origin", remote_path.to_str().unwrap()],
        );
        git(&repo_path, &["push", "-u", "origin", "master"]);
        git(&repo_path, &["branch", "-m", "main"]);
        git(&repo_path, &["push", "-u", "origin", "main"]);

        let paths = ProductAppPaths::new(root.path().join(".aria"));
        let aggregate_root = root.path().join("aggregate-root");
        std::fs::create_dir_all(&aggregate_root).unwrap();
        let manifest =
            LogicalCodebaseManifest::new("project_0001", aggregate_root, vec![member_repo_id]);
        let store = crate::product::logical_codebase::LogicalCodebaseStore::new(paths);
        store.save_manifest("project_0001", &manifest).unwrap();
        let now = "2026-08-14T00:00:00Z".to_string();
        store
            .save_member(
                "project_0001",
                &CodebaseMemberRecord {
                    logical_repository_id: member_repo_id,
                    physical_repository_id: format!("repo_{}", member_repo_id.0),
                    alias: "api".to_string(),
                    role: "service".to_string(),
                    ordinal: 1,
                    source_identity: RepositorySourceIdentity::from_git_parts(
                        &repo_path,
                        repo_path.join(".git"),
                        Some(format!(
                            "ssh://git@example.test/acme/{}.git",
                            member_repo_id.0
                        )),
                    ),
                    repo_type: RepositoryType::Unknown,
                    tech_stack: Vec::new(),
                    owner: None,
                    tags: Vec::new(),
                    default_ref: None,
                    checkout_ids: vec![checkout_id],
                    status: MemberStatus::Active,
                    created_at: now.clone(),
                    updated_at: now.clone(),
                },
            )
            .unwrap();
        store
            .save_checkout(
                "project_0001",
                &RepositoryCheckoutRecord {
                    checkout_id,
                    logical_repository_id: member_repo_id,
                    physical_repository_id: format!("repo_{}", member_repo_id.0),
                    kind: CheckoutKind::Main,
                    canonical_path: repo_path,
                    checkout_path_hash: format!("sha256:{}", member_repo_id.0),
                    git_dir_identity: format!("sha256:git-{}", member_repo_id.0),
                    revision: None,
                    availability: CheckoutAvailability::Available,
                    observed_at: now.clone(),
                    created_at: now.clone(),
                    updated_at: now,
                },
            )
            .unwrap();

        Fixture {
            root,
            member_repo_id: member_repo_id.0.to_string(),
        }
    }

    fn test_app(root: &StdPath) -> axum::Router {
        let root = root.to_path_buf();
        let state = WebAppState::new(root.clone(), WebRuntime::new_fake(root));
        build_web_router(state)
    }

    async fn post_json(
        app: &axum::Router,
        uri: &str,
        body: serde_json::Value,
    ) -> axum::http::Response<Body> {
        app.clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(uri)
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    async fn get(app: &axum::Router, uri: &str) -> axum::http::Response<Body> {
        app.clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    async fn body_json(response: axum::http::Response<Body>) -> serde_json::Value {
        let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn create_pointer_publication_runs_full_batch_and_returns_publication() {
        let fixture = setup();
        let app = test_app(fixture.root.path());
        let response = post_json(
            &app,
            "/api/projects/project_0001/logical-codebase/pointer-publications",
            serde_json::json!({"batch_kind": "full"}),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = body_json(response).await;
        assert_eq!(body["status"], "completed_all");
        assert_eq!(body["entries"].as_array().unwrap().len(), 1);
        assert_eq!(body["entries"][0]["state"], "review_created");
        assert_eq!(body["entries"][0]["member_repo_id"], fixture.member_repo_id);
    }

    #[tokio::test]
    async fn create_pointer_publication_rejects_second_in_progress_batch_with_busy() {
        let fixture = setup();
        // 预置一个 InProgress 批次：发布锁必须拒绝新批次。
        let paths = ProductAppPaths::new(fixture.root.path().join(".aria"));
        let store = PointerPublicationStore::new(paths.clone());
        let manifest = crate::product::logical_codebase::LogicalCodebaseStore::new(paths)
            .load_manifest("project_0001")
            .unwrap()
            .unwrap();
        let now = "2026-08-14T00:00:00Z".to_string();
        store
            .create_publication(PointerPublication {
                id: "pub-seeded".to_string(),
                project_id: "project_0001".to_string(),
                logical_codebase_id: manifest.logical_codebase_id.to_string(),
                batch_kind: PointerPublicationBatchKind::Full,
                entries: vec![PointerPublicationEntry {
                    member_repo_id: fixture.member_repo_id.clone(),
                    state: PointerPublicationEntryState::Pending,
                    branch_name: None,
                    commit_sha: None,
                    push_error: None,
                    conflict_detail: None,
                }],
                status: PointerPublicationStatus::InProgress,
                created_at: now.clone(),
                updated_at: now,
            })
            .unwrap();

        let app = test_app(fixture.root.path());
        let response = post_json(
            &app,
            "/api/projects/project_0001/logical-codebase/pointer-publications",
            serde_json::json!({"batch_kind": "full"}),
        )
        .await;
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body = body_json(response).await;
        assert_eq!(body["code"], "pointer_publish_busy");
    }

    #[tokio::test]
    async fn list_and_get_pointer_publications_return_persisted_batch() {
        let fixture = setup();
        let app = test_app(fixture.root.path());
        let created = body_json(
            post_json(
                &app,
                "/api/projects/project_0001/logical-codebase/pointer-publications",
                serde_json::json!({"batch_kind": "full"}),
            )
            .await,
        )
        .await;
        let publication_id = created["id"].as_str().unwrap().to_string();

        let listed = body_json(
            get(
                &app,
                "/api/projects/project_0001/logical-codebase/pointer-publications",
            )
            .await,
        )
        .await;
        assert_eq!(listed.as_array().unwrap().len(), 1);

        let single = body_json(
            get(
                &app,
                &format!(
                    "/api/projects/project_0001/logical-codebase/pointer-publications/{publication_id}"
                ),
            )
            .await,
        )
        .await;
        assert_eq!(single["id"], publication_id);
    }

    #[tokio::test]
    async fn revoke_marks_publication_and_is_idempotent() {
        let fixture = setup();
        let app = test_app(fixture.root.path());
        let created = body_json(
            post_json(
                &app,
                "/api/projects/project_0001/logical-codebase/pointer-publications",
                serde_json::json!({"batch_kind": "full"}),
            )
            .await,
        )
        .await;
        let publication_id = created["id"].as_str().unwrap().to_string();

        let revoked = post_json(
            &app,
            &format!(
                "/api/projects/project_0001/logical-codebase/pointer-publications/{publication_id}/revoke"
            ),
            serde_json::json!({}),
        )
        .await;
        assert_eq!(revoked.status(), StatusCode::OK);
        let body = body_json(revoked).await;
        assert_eq!(body["status"], "revoked");
        assert_eq!(body["entries"][0]["state"], "revoked");

        // 重复 revoke 幂等
        let again = post_json(
            &app,
            &format!(
                "/api/projects/project_0001/logical-codebase/pointer-publications/{publication_id}/revoke"
            ),
            serde_json::json!({}),
        )
        .await;
        assert_eq!(again.status(), StatusCode::OK);
        assert_eq!(body_json(again).await["status"], "revoked");
    }
}
