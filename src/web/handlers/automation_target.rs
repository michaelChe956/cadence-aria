//! P1 Task 1（REQ-WIGA-01/02、REQ-MTG-03）：自动化可用目标只读投影。
//!
//! `GET /automation-target` 复用 P0 PUT 的 routing + 单候选 preflight，
//! 并用 `provider_workspace_config` 把用户 query 解析成服务端已确认的
//! `EnrollmentOptions`（与 prepare 缺省同源）：前端原样把 `resolved_options`
//! 写入 P0 PUT，不猜默认 provider、不从 `repo_id` 推断 UUID。
//! 零/多 target、Legacy/FailClosed、provider 不可用、跨 issue 返回明确错误；
//! 只读，不写 enrollment。

use crate::product::logical_codebase::RepositoryRouting;
use crate::product::models::IssueWorkItemPlanOptions;
use crate::product::models::automation::EnrollmentOptions;
use crate::web::error::ApiResult;
use crate::web::handlers::lifecycle::preflight::{
    SingleCandidatePreflightDecision, logical_repository_ids_for_preflight,
    preflight_single_repository_candidate,
};
use crate::web::state::WebAppState;
use axum::Json;
use axum::extract::{Path, Query, State};
use serde::Deserialize;

use super::automation_enrollment::{ensure_issue_exists, invalid_scope, validate_request_ids};
use super::support::{product_app_paths, product_store_api_error, provider_workspace_config};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct AutomationTargetDto {
    pub logical_repository_id: String,
    pub resolved_options: EnrollmentOptions,
}

/// 与 `PrepareWorkItemPlanRequest` 的 provider/plan 选项同名同类型的只读输入；
/// 缺省项由服务端与 prepare 同源解析，不由前端猜。
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct AutomationTargetQuery {
    pub author_provider: Option<String>,
    pub reviewer_provider: Option<String>,
    pub review_rounds: Option<u32>,
    pub superpowers_enabled: Option<bool>,
    pub openspec_enabled: Option<bool>,
    pub include_integration_tests: Option<bool>,
    pub include_e2e_tests: Option<bool>,
    pub force_frontend_backend_split: Option<bool>,
    pub require_execution_plan_confirm: Option<bool>,
}

/// 与服务端同源的已解析 enrollment options（provider/开关/plan 缺省与 prepare
/// 完全一致）；provider 不可用按现有 `provider_workspace_config` 错误显式失败。
pub(crate) fn resolve_enrollment_options_with_provider_workspace_config(
    state: &WebAppState,
    query: &AutomationTargetQuery,
) -> ApiResult<EnrollmentOptions> {
    let config = provider_workspace_config(
        query.author_provider.as_deref(),
        query.reviewer_provider.as_deref(),
        query.review_rounds,
        query.superpowers_enabled,
        query.openspec_enabled,
        &*state.provider_availability,
    )?;
    Ok(EnrollmentOptions {
        author_provider: config.author_provider,
        reviewer_provider: config.reviewer_provider,
        review_rounds: config.review_rounds,
        superpowers_enabled: config.superpowers_enabled,
        openspec_enabled: config.openspec_enabled,
        plan_options: IssueWorkItemPlanOptions {
            include_integration_tests: query.include_integration_tests.unwrap_or(true),
            include_e2e_tests: query.include_e2e_tests.unwrap_or(false),
            force_frontend_backend_split: query.force_frontend_backend_split.unwrap_or(false),
            require_execution_plan_confirm: query.require_execution_plan_confirm.unwrap_or(false),
        },
    })
}

pub async fn get_automation_target(
    State(state): State<WebAppState>,
    Path((project_id, issue_id)): Path<(String, String)>,
    Query(query): Query<AutomationTargetQuery>,
) -> ApiResult<Json<AutomationTargetDto>> {
    validate_request_ids(&project_id, &issue_id)?;
    ensure_issue_exists(&state, &project_id, &issue_id)?;
    let paths = product_app_paths(&state);
    let routing = RepositoryRouting::load_for_issue(&paths, &project_id, &issue_id)
        .map_err(product_store_api_error)?;
    let RepositoryRouting::Logical {
        manifest,
        selection,
    } = routing
    else {
        return Err(invalid_scope(
            "automation target requires a logical codebase routing",
        ));
    };
    let candidates = logical_repository_ids_for_preflight(&manifest, &selection);
    let SingleCandidatePreflightDecision::Eligible { repository_id } =
        preflight_single_repository_candidate(&candidates)
    else {
        return Err(invalid_scope(
            "automation target requires exactly one logical repository",
        ));
    };
    Ok(Json(AutomationTargetDto {
        logical_repository_id: repository_id,
        resolved_options: resolve_enrollment_options_with_provider_workspace_config(
            &state, &query,
        )?,
    }))
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    use super::super::automation_enrollment_test_support::{
        ISSUE_ID, PROJECT_ID, SINGLE_LOGICAL_ID, enrollment_file_exists, response_json,
        seed_fixture,
    };
    use crate::product::models::ProviderName;
    use crate::web::app::build_web_router;
    use crate::web::runtime::WebRuntime;
    use crate::web::state::WebAppState;

    async fn get_automation_target(app: &axum::Router, query: &str) -> axum::http::Response<Body> {
        app.clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/projects/{PROJECT_ID}/issues/{ISSUE_ID}/automation-target{query}"
                    ))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn automation_target_requires_exactly_one_logical_member() {
        let one = seed_fixture(1, true);
        let ok = get_automation_target(
            &one.router(),
            "?author_provider=fake&reviewer_provider=fake",
        )
        .await;
        assert_eq!(ok.status(), StatusCode::OK);
        let body = response_json(ok).await;
        assert_eq!(body["logical_repository_id"], SINGLE_LOGICAL_ID);
        assert_eq!(body["resolved_options"]["author_provider"], "fake");
        assert_eq!(body["resolved_options"]["reviewer_provider"], "fake");
        assert_eq!(body["resolved_options"]["review_rounds"], 1);
        assert_eq!(
            body["resolved_options"]["plan_options"]["include_integration_tests"],
            true
        );
        assert_eq!(
            body["resolved_options"]["plan_options"]["include_e2e_tests"],
            false
        );

        for members in [0, 2] {
            let fixture = seed_fixture(members, true);
            let response = get_automation_target(&fixture.router(), "?author_provider=fake").await;
            assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
            let payload = response_json(response).await;
            assert_eq!(payload["code"], "automation_enrollment_invalid_scope");
            assert!(!enrollment_file_exists(&fixture));
        }
    }

    #[tokio::test]
    async fn automation_target_resolves_server_defaults_when_query_omits_them() {
        let fixture = seed_fixture(1, true);
        let response = get_automation_target(&fixture.router(), "").await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        // fake runtime availability 恒真：缺省与 prepare 同源（codex/claude_code、
        // review_rounds=1、superpowers/openspec 默认开、plan 缺省同 prepare）。
        assert_eq!(body["resolved_options"]["author_provider"], "codex");
        assert_eq!(body["resolved_options"]["reviewer_provider"], "claude_code");
        assert_eq!(body["resolved_options"]["review_rounds"], 1);
        assert_eq!(body["resolved_options"]["superpowers_enabled"], true);
        assert_eq!(body["resolved_options"]["openspec_enabled"], true);
    }

    #[tokio::test]
    async fn automation_target_rejects_unavailable_provider_without_writing_enrollment() {
        let fixture = seed_fixture(1, true);
        let root = fixture._root.path().to_path_buf();
        let state = WebAppState::with_provider_availability(
            root.clone(),
            WebRuntime::new_fake(root),
            |provider: &ProviderName| !matches!(provider, ProviderName::KimiCode),
        );
        let app = build_web_router(state);
        let response = get_automation_target(&app, "?author_provider=kimi_code").await;
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let payload = response_json(response).await;
        assert_eq!(payload["code"], "provider_unavailable");
        assert!(!enrollment_file_exists(&fixture));
    }

    #[tokio::test]
    async fn automation_target_rejects_unknown_issue_with_not_found() {
        let fixture = seed_fixture(1, true);
        let response = fixture
            .router()
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/projects/project_0001/issues/issue_9999/automation-target")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
