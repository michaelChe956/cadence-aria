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
    /// C5 Task 1：载体身份唯一投影——冗余的顶层 `logical_repository_id`
    /// 已删除（前端 Enable/Rebind 原样回传；单仓入口由 C5 消费同一 union）。
    pub enrollment_target: crate::product::logical_codebase::EnrollmentTarget,
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
        state.test_provider_enabled,
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
    // C5 Task 2：载体判定收敛到唯一 resolver 入口（GET 与 PUT Enable
    // 同一判定，Review Focus 5）；issue 记录为权威 repo_id 来源。
    let issue = crate::product::issue_store::IssueStore::new(paths.clone())
        .get(&project_id, &issue_id)
        .map_err(product_store_api_error)?;
    let carrier = super::support::resolve_automation_carrier(&paths, &project_id, &issue)?;
    let resolved_options =
        resolve_enrollment_options_with_provider_workspace_config(&state, &query)?;
    match &carrier {
        super::support::AutomationCarrierResolution::SingleRepository { target } => {
            // 单仓载体：跳过 gateway 谓词不误拒（Review Focus 5；A10）；
            // 投影只含单仓形态 enrollment_target + resolved_options。
            Ok(Json(AutomationTargetDto {
                enrollment_target: target.clone(),
                resolved_options,
            }))
        }
        super::support::AutomationCarrierResolution::LogicalCodebase { resolution } => {
            let manifest = resolution.manifest.clone().ok_or_else(|| {
                invalid_scope("automation target requires a logical codebase manifest routing")
            })?;
            let selection = resolution.selection.as_ref().ok_or_else(|| {
                invalid_scope("automation target requires an explicit logical codebase selection")
            })?;
            let candidates = logical_repository_ids_for_preflight(&manifest, selection);
            let SingleCandidatePreflightDecision::Eligible { repository_id } =
                preflight_single_repository_candidate(&candidates)
            else {
                return Err(invalid_scope(
                    "automation target requires exactly one logical repository",
                ));
            };
            // C5 Task 3：唯一 logical target 确认后做完整角色链静态预检
            //（author→coder→reviewer 逐角色）——与最终 PUT Enable 同一
            // carrier、同一判定，投影阶段即拒绝确定性不支持的组合，
            // 一次列全全部违规角色。
            super::automation_gateway_preflight::validate_role_chain_for_enrollment(
                &resolved_options.author_provider,
                &resolved_options.reviewer_provider,
                &carrier,
                true,
                state.test_provider_enabled,
            )?;
            let target_repository = crate::product::logical_codebase::LogicalRepositoryId(
                uuid::Uuid::parse_str(&repository_id).map_err(|_| {
                    invalid_scope("automation target logical repository id is not a valid uuid")
                })?,
            );
            Ok(Json(AutomationTargetDto {
                enrollment_target:
                    crate::product::logical_codebase::EnrollmentTarget::LogicalCodebase {
                        logical_codebase_id: manifest.logical_codebase_id.to_string(),
                        logical_repository_id: target_repository,
                    },
                resolved_options,
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    use super::super::automation_enrollment_test_support::{
        ISSUE_ID, PROJECT_ID, SINGLE_LOGICAL_ID, enrollment_body, enrollment_file_exists,
        put_enrollment, response_json, seed_fixture,
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
        // C5 Task 1：DTO 不再暴露冗余顶层 logical 身份。
        assert!(body.get("logical_repository_id").is_none());
        assert_eq!(body["enrollment_target"]["kind"], "logical_codebase");
        assert_eq!(
            body["enrollment_target"]["logical_repository_id"],
            SINGLE_LOGICAL_ID
        );
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
        // C5 Task 3：缺省 author=codex 在 LC 载体下按角色链语义被静态拒
        //（plan_author/coder 违规）——显式 fake author 保持缺省解析断言。
        let refused = get_automation_target(&fixture.router(), "").await;
        assert_eq!(refused.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let payload = response_json(refused).await;
        assert_eq!(payload["code"], "automation_role_chain_unsupported");

        let response = get_automation_target(&fixture.router(), "?author_provider=fake").await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        // fake runtime availability 恒真：缺省与 prepare 同源（reviewer=
        // claude_code、review_rounds=1、superpowers/openspec 默认开、
        // plan 缺省同 prepare）。
        assert_eq!(body["resolved_options"]["author_provider"], "fake");
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

    // P2 GAP-F（Task 0.2）：enrollment 静态 gateway reviewer 组合预检——GET
    // 投影与最终 PUT Enable 同源拒绝确定性不支持的 reviewer（KimiCode/Pi 无
    // gateway dialect），可用性门先通过，红灯只来自 gateway 缺口。
    #[tokio::test]
    async fn automation_target_rejects_gateway_unsupported_reviewer_before_enable() {
        let fixture = seed_fixture(1, true);
        let root = fixture._root.path().to_path_buf();
        let state = WebAppState::with_provider_availability(
            root.clone(),
            WebRuntime::new_fake(root),
            |_| true,
        );
        let app = build_web_router(state);
        let response =
            get_automation_target(&app, "?author_provider=pi&reviewer_provider=kimi_code").await;
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(
            response_json(response).await["code"],
            "automation_role_chain_unsupported"
        );
        assert!(!enrollment_file_exists(&fixture));

        let mut enable = enrollment_body(&fixture, 1, 1);
        enable["command"]["options"]["reviewer_provider"] = serde_json::json!("kimi_code");
        let response = put_enrollment(&app, enable).await;
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(
            response_json(response).await["code"],
            "automation_role_chain_unsupported"
        );
        assert!(!enrollment_file_exists(&fixture));
    }

    /// Codex reviewer 在当前固定 danger-full-access sandbox 下被 gateway 路由
    /// 静态拒绝（与 `enforce_route_policy` 同源）；GET 与 PUT 同码。
    #[tokio::test]
    async fn automation_target_rejects_codex_reviewer_under_default_sandbox() {
        let fixture = seed_fixture(1, true);
        let root = fixture._root.path().to_path_buf();
        let state = WebAppState::with_provider_availability(
            root.clone(),
            WebRuntime::new_fake(root),
            |_| true,
        );
        let app = build_web_router(state);
        let response = get_automation_target(&app, "?reviewer_provider=codex").await;
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let payload = response_json(response).await;
        assert_eq!(payload["code"], "automation_role_chain_unsupported");
        let violations = payload["details"]["violations"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        assert!(
            violations
                .iter()
                .any(|violation| violation["reason_code"] == "codex_danger_full_access_unsupported"),
            "violations should carry the stable gateway verdict code, got: {payload}"
        );

        let mut enable = enrollment_body(&fixture, 1, 1);
        enable["command"]["options"]["reviewer_provider"] = serde_json::json!("codex");
        let response = put_enrollment(&app, enable).await;
        assert_eq!(
            response_json(response).await["code"],
            "automation_role_chain_unsupported"
        );
        assert!(!enrollment_file_exists(&fixture));
    }

    /// 测试运行 `test_provider_enabled` 下 Fake reviewer 保持 fixture 可用：
    /// 预检不误伤 Fake，仍建立原同键 enrollment。
    #[tokio::test]
    async fn automation_target_allows_fake_reviewer_in_fake_runtime() {
        let fixture = seed_fixture(1, true);
        let app = fixture.router();
        // C5 Task 3：显式 fake author 避开缺省 codex 的路由禁令；测试运行
        // test_provider_enabled 下 Fake 全链豁免，仍建立原同键 enrollment。
        let response =
            get_automation_target(&app, "?author_provider=fake&reviewer_provider=fake").await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response_json(response).await["resolved_options"]["reviewer_provider"],
            "fake"
        );

        let enable = put_enrollment(&app, enrollment_body(&fixture, 1, 1)).await;
        assert_eq!(enable.status(), StatusCode::OK);
        assert!(enrollment_file_exists(&fixture));
    }

    /// C5 Task 3（REQ-WIGA-C5-PREFLIGHT、Review Focus 5）：LC 载体下
    /// author=Pi 使 coder 派生无 gateway 启动能力——GET 投影与 PUT Enable
    /// 同一判定 422，payload 逐角色列出违规（含 coder 角色），零写入。
    #[tokio::test]
    async fn automation_target_rejects_lc_pi_coder_with_role_chain_before_enable() {
        let fixture = seed_fixture(1, true);
        let root = fixture._root.path().to_path_buf();
        let state = WebAppState::with_provider_availability(
            root.clone(),
            WebRuntime::new_fake(root),
            |_| true,
        );
        let app = build_web_router(state);
        let response =
            get_automation_target(&app, "?author_provider=pi&reviewer_provider=claude_code").await;
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let payload = response_json(response).await;
        assert_eq!(payload["code"], "automation_role_chain_unsupported");
        let violations = payload["details"]["violations"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let roles: Vec<&str> = violations
            .iter()
            .map(|violation| violation["role"].as_str().unwrap())
            .collect();
        assert_eq!(roles, vec!["plan_author", "coder"], "{payload}");
        assert!(!enrollment_file_exists(&fixture));

        // 同一输入走 PUT Enable：判定逐字节一致（同一函数调用）。
        let mut enable = enrollment_body(&fixture, 1, 1);
        enable["command"]["options"]["author_provider"] = serde_json::json!("pi");
        let response = put_enrollment(&app, enable).await;
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let payload = response_json(response).await;
        assert_eq!(payload["code"], "automation_role_chain_unsupported");
        let violations: Vec<serde_json::Value> = payload["details"]["violations"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let roles: Vec<String> = violations
            .iter()
            .map(|violation| violation["role"].as_str().unwrap_or_default().to_string())
            .collect();
        assert_eq!(roles, vec!["plan_author", "coder"], "{payload}");
        assert!(!enrollment_file_exists(&fixture));
    }
}

#[cfg(test)]
mod single_repository_tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    use super::super::automation_enrollment_test_support::{
        ISSUE_ID, PROJECT_ID, REPOSITORY_ID, enrollment_file_exists, put_enrollment, response_json,
        seed_fixture, seed_single_repository_fixture, single_repository_enable_body,
    };
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

    /// C5 Task 2：单仓 issue 的 GET 投影返回真实物理仓 target
    ///（REQ-ROUTE-C5-TARGET），DTO 不含任何 logical 身份字段。
    #[tokio::test]
    async fn single_repository_target_projection_returns_real_repository_identity() {
        let fixture = seed_single_repository_fixture();
        let app = fixture.router();
        let response =
            get_automation_target(&app, "?author_provider=fake&reviewer_provider=fake").await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body["enrollment_target"]["kind"], "single_repository");
        assert_eq!(body["enrollment_target"]["repository_id"], REPOSITORY_ID);
        assert!(body.get("logical_repository_id").is_none());
        assert!(
            body["enrollment_target"]
                .get("logical_repository_id")
                .is_none()
                && body["enrollment_target"]
                    .get("logical_codebase_id")
                    .is_none(),
            "single repository target must not grow a logical stand-in: {body}"
        );
        assert_eq!(body["resolved_options"]["author_provider"], "fake");
        assert_eq!(body["resolved_options"]["reviewer_provider"], "fake");
    }

    /// C5 Task 2：单仓 Enable 可授权——enrollment target 为该物理仓身份、
    /// 无任何 logical repository 身份、binding v1 建立、幂等。
    #[tokio::test]
    async fn single_repository_enable_authorizes_physical_target() {
        let fixture = seed_single_repository_fixture();
        let app = fixture.router();
        let response = put_enrollment(&app, single_repository_enable_body(&fixture)).await;
        assert_eq!(response.status(), StatusCode::OK);
        let payload = response_json(response).await;
        assert_eq!(payload["enabled"], true);
        assert_eq!(payload["target"]["kind"], "single_repository");
        assert_eq!(payload["target"]["repository_id"], REPOSITORY_ID);
        assert!(payload.get("logical_repository_id").is_none());
        assert_eq!(payload["binding_history"]["current"]["binding_version"], 1);

        // 同键同 payload 幂等：返回同一 enrollment。
        let again = put_enrollment(&app, single_repository_enable_body(&fixture)).await;
        assert_eq!(again.status(), StatusCode::OK);
        let replay = response_json(again).await;
        assert_eq!(replay["enrollment_id"], payload["enrollment_id"]);
    }

    /// C5 Task 2：错仓／跨载体被拒（REQ-WIGA-01）——repository_id 不一致、
    /// 单仓 issue 提交 LogicalCodebase target、LC issue 提交单仓 target
    /// 均 422，零 enrollment 写入。
    #[tokio::test]
    async fn single_repository_enable_rejects_wrong_or_cross_carrier_target() {
        let fixture = seed_single_repository_fixture();
        let app = fixture.router();

        // 错仓：repository_id 指向另一物理仓。
        let mut wrong_repo = single_repository_enable_body(&fixture);
        wrong_repo["command"]["target"]["repository_id"] = serde_json::json!("repo-other");
        let response = put_enrollment(&app, wrong_repo).await;
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let payload = response_json(response).await;
        assert_eq!(payload["code"], "automation_enrollment_invalid_scope");

        // 跨载体：单仓 issue 提交 LogicalCodebase target。
        let mut cross = single_repository_enable_body(&fixture);
        cross["command"]["target"] = serde_json::json!({
            "kind": "logical_codebase",
            "logical_codebase_id": "00000000-0000-0000-0000-0000000000c5",
            "logical_repository_id": "00000000-0000-0000-0000-000000000001",
        });
        let response = put_enrollment(&app, cross).await;
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);

        // LC issue 提交单仓 target（反向跨载体）。
        let lc_fixture = seed_fixture(1, true);
        let lc_app = lc_fixture.router();
        let response = put_enrollment(&lc_app, single_repository_enable_body(&lc_fixture)).await;
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(
            response_json(response).await["code"],
            "automation_enrollment_invalid_scope"
        );

        assert!(!enrollment_file_exists(&fixture));
        assert!(!enrollment_file_exists(&lc_fixture));
    }

    /// C5 Task 2（Review Focus 2）：单仓物理仓与另一登记记录解析到同一
    /// git 根 → 409 `repository_routing_source_identity_mismatch`，
    /// 不选择任一记录继续。
    #[tokio::test]
    async fn single_repository_authority_conflict_fails_closed_with_409() {
        let fixture = seed_single_repository_fixture();
        // 追加别名记录：与 repo-1 指向同一物理 git 根。
        let repos_path = fixture.paths.project_root(PROJECT_ID).join("repos.json");
        let mut repositories: Vec<crate::product::models::RepositoryRecord> =
            crate::product::json_store::read_json(&repos_path).unwrap();
        let alias = repositories[0].clone();
        repositories.push(crate::product::models::RepositoryRecord {
            id: "repo-1-alias".to_string(),
            name: "repo-1-alias".to_string(),
            ..alias
        });
        crate::product::json_store::write_json(&repos_path, &repositories).unwrap();

        let app = fixture.router();
        let response = get_automation_target(&app, "?author_provider=fake").await;
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let payload = response_json(response).await;
        assert_eq!(
            payload["code"], "repository_routing_source_identity_mismatch",
            "authority conflict must surface the routing conflict code: {payload}"
        );

        let response = put_enrollment(&app, single_repository_enable_body(&fixture)).await;
        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert!(!enrollment_file_exists(&fixture));
    }

    /// C5 Task 2：缺仓与仓未登记——issue 无 repo_id 或指向不存在记录
    /// → 422 指明缺失仓库身份，不写 enrollment。
    #[tokio::test]
    async fn single_repository_target_requires_registered_repo_id() {
        // issue.repo_id 指向未登记仓。
        let fixture = seed_single_repository_fixture();
        let issue_path = fixture
            .paths
            .issue_root(PROJECT_ID, ISSUE_ID)
            .join("issue.json");
        let mut issue: serde_json::Value =
            crate::product::json_store::read_json(&issue_path).unwrap();
        issue["repo_id"] = serde_json::json!("repo-unregistered");
        crate::product::json_store::write_json(&issue_path, &issue).unwrap();
        let app = fixture.router();
        let response = get_automation_target(&app, "?author_provider=fake").await;
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let payload = response_json(response).await;
        assert!(
            payload["message"]
                .as_str()
                .unwrap()
                .to_lowercase()
                .contains("repositor"),
            "should point at the missing repository identity: {payload}"
        );
        let response = put_enrollment(&app, single_repository_enable_body(&fixture)).await;
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert!(!enrollment_file_exists(&fixture));

        // issue.repo_id 缺失（null）。
        let fixture = seed_single_repository_fixture();
        let issue_path = fixture
            .paths
            .issue_root(PROJECT_ID, ISSUE_ID)
            .join("issue.json");
        let mut issue: serde_json::Value =
            crate::product::json_store::read_json(&issue_path).unwrap();
        issue["repo_id"] = serde_json::Value::Null;
        crate::product::json_store::write_json(&issue_path, &issue).unwrap();
        let app = fixture.router();
        let response = get_automation_target(&app, "?author_provider=fake").await;
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert!(
            response_json(response).await["message"]
                .as_str()
                .unwrap()
                .to_lowercase()
                .contains("repositor"),
            "should point at the missing repository identity"
        );
        assert!(!enrollment_file_exists(&fixture));
    }

    /// C5 Task 3（Review Focus 5／A10 单仓不误拒）：单仓 issue 的
    /// author/coder/reviewer 派生为 LC gateway 不支持但本机可用的 provider
    ///（Pi/KimiCode）→ 投影与 Enable 成功、enrollment 写入。
    #[tokio::test]
    async fn single_repository_allows_locally_available_gateway_unsupported_providers() {
        let fixture = seed_single_repository_fixture();
        let root = fixture._root.path().to_path_buf();
        let state = WebAppState::with_provider_availability(
            root.clone(),
            WebRuntime::new_fake(root),
            |_| true,
        );
        let app = build_web_router(state);
        let response =
            get_automation_target(&app, "?author_provider=pi&reviewer_provider=kimi_code").await;
        assert_eq!(response.status(), StatusCode::OK);
        let payload = response_json(response).await;
        assert_eq!(payload["resolved_options"]["author_provider"], "pi");
        assert_eq!(
            payload["resolved_options"]["reviewer_provider"],
            "kimi_code"
        );

        let mut enable = single_repository_enable_body(&fixture);
        enable["command"]["options"]["author_provider"] = serde_json::json!("pi");
        enable["command"]["options"]["reviewer_provider"] = serde_json::json!("kimi_code");
        let response = put_enrollment(&app, enable).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert!(enrollment_file_exists(&fixture));
        let payload = response_json(response).await;
        assert_eq!(payload["target"]["kind"], "single_repository");
    }
}
