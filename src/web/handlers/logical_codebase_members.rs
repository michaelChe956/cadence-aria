use axum::Json;
use axum::extract::{Path, State};
use serde::Serialize;

use super::support::{product_app_paths, product_store_api_error};
use crate::product::json_store::validate_relative_id;
use crate::product::logical_codebase::{LogicalCodebaseStore, MemberStatus};
use crate::web::error::{ApiError, ApiResult};
use crate::web::state::WebAppState;

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct LogicalCodebaseMemberDto {
    pub logical_repository_id: String,
    pub alias: String,
    pub status: MemberStatus,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct LogicalCodebaseMembersResponse {
    pub members: Vec<LogicalCodebaseMemberDto>,
}

pub async fn list_logical_codebase_members(
    State(state): State<WebAppState>,
    Path(project_id): Path<String>,
) -> ApiResult<Json<LogicalCodebaseMembersResponse>> {
    validate_project_id(&project_id)?;
    let store = LogicalCodebaseStore::new(product_app_paths(&state));
    if store
        .load_manifest(&project_id)
        .map_err(product_store_api_error)?
        .is_none()
    {
        return Ok(Json(LogicalCodebaseMembersResponse {
            members: Vec::new(),
        }));
    }
    let members = store
        .list_members(&project_id)
        .map_err(product_store_api_error)?
        .into_iter()
        .map(|member| LogicalCodebaseMemberDto {
            logical_repository_id: member.logical_repository_id.0.to_string(),
            alias: member.alias,
            status: member.status,
        })
        .collect();
    Ok(Json(LogicalCodebaseMembersResponse { members }))
}

fn validate_project_id(project_id: &str) -> ApiResult<()> {
    validate_relative_id(project_id).map_err(|error| {
        ApiError::validation("invalid_project_id", format!("invalid project id: {error}"))
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::logical_codebase::{
        CodebaseMemberRecord, LogicalCodebaseManifest, LogicalCodebaseStore, LogicalRepositoryId,
        MemberStatus, RepositorySourceIdentity, RepositoryType,
    };
    use crate::web::app::build_web_router;
    use crate::web::runtime::WebRuntime;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use serde_json::Value;
    use tempfile::tempdir;
    use tower::ServiceExt;
    use uuid::Uuid;

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

    async fn body_json(response: axum::http::Response<Body>) -> Value {
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    fn app(root: &std::path::Path) -> axum::Router {
        let root = root.to_path_buf();
        build_web_router(WebAppState::new(root.clone(), WebRuntime::new_fake(root)))
    }

    fn member(id: LogicalRepositoryId, alias: &str, status: MemberStatus) -> CodebaseMemberRecord {
        CodebaseMemberRecord {
            logical_repository_id: id,
            physical_repository_id: format!("physical-{alias}"),
            alias: alias.to_string(),
            role: "service".to_string(),
            ordinal: 1,
            source_identity: RepositorySourceIdentity {
                scheme: "test".to_string(),
                key_digest: format!("digest-{alias}"),
                canonical_git_dir: "/tmp/test/.git".into(),
                canonical_origin: None,
                first_seen_path_hash: format!("hash-{alias}"),
            },
            repo_type: RepositoryType::Unknown,
            tech_stack: Vec::new(),
            owner: None,
            tags: Vec::new(),
            default_ref: None,
            checkout_ids: Vec::new(),
            status,
            created_at: "2026-08-14T00:00:00Z".to_string(),
            updated_at: "2026-08-14T00:00:00Z".to_string(),
        }
    }

    #[tokio::test]
    async fn list_members_projects_without_manifest_returns_empty_array() {
        let root = tempdir().unwrap();
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        let store = LogicalCodebaseStore::new(paths);
        let logical_repository_id = LogicalRepositoryId(Uuid::new_v4());
        store
            .save_member(
                "project_0001",
                &member(logical_repository_id, "orphan", MemberStatus::Active),
            )
            .unwrap();
        let response = get(
            &app(root.path()),
            "/api/projects/project_0001/logical-codebase/members",
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            body_json(response).await,
            serde_json::json!({ "members": [] })
        );
    }

    #[tokio::test]
    async fn list_members_projects_with_manifest_projects_projection_fields() {
        let root = tempdir().unwrap();
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        let logical_repository_id = LogicalRepositoryId(Uuid::new_v4());
        let store = LogicalCodebaseStore::new(paths);
        store
            .save_manifest(
                "project_0001",
                &LogicalCodebaseManifest::new(
                    "project_0001",
                    root.path().join("aggregate-root"),
                    vec![logical_repository_id],
                ),
            )
            .unwrap();
        store
            .save_member(
                "project_0001",
                &member(logical_repository_id, "api", MemberStatus::Removed),
            )
            .unwrap();

        let response = get(
            &app(root.path()),
            "/api/projects/project_0001/logical-codebase/members",
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            body_json(response).await,
            serde_json::json!({
                "members": [{
                    "logical_repository_id": logical_repository_id.0.to_string(),
                    "alias": "api",
                    "status": "removed"
                }]
            })
        );
    }
}
