use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use cadence_aria::cross_cutting::provider_adapter::{ProviderAdapter, ProviderAdapterError};
use cadence_aria::cross_cutting::provider_registry::ProviderRegistry;
use cadence_aria::product::models::ProviderName;
use cadence_aria::protocol::contracts::{AdapterInput, AdapterOutput, TimeoutStatus};
use cadence_aria::web::app::build_web_router;
use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::state::WebAppState;
use cadence_aria::web::test_controls::TestControlledFakeStreamingProvider;
use serde_json::{Value, json};
use std::process::Command;
use std::sync::Arc;
use tempfile::tempdir;
use tower::ServiceExt;

#[derive(Debug, Clone)]
pub(crate) struct MockSplitProviderAdapter {
    pub(crate) output: Value,
}

impl ProviderAdapter for MockSplitProviderAdapter {
    fn run(&self, _input: &AdapterInput) -> Result<AdapterOutput, ProviderAdapterError> {
        Ok(AdapterOutput {
            exit_code: Some(0),
            stdout: String::new(),
            stderr: String::new(),
            structured_output: Some(self.output.clone()),
            files_modified: Vec::new(),
            duration_ms: 0,
            timeout_status: TimeoutStatus::NotTimedOut,
        })
    }
}

pub(crate) fn valid_split_output() -> Value {
    json!({
        "repository_profile": {
            "confidence": "high",
            "detected_layers": ["backend", "frontend"],
            "split_recommendation": "frontend_backend",
            "languages": ["rust"],
            "frameworks": [],
            "package_managers": ["cargo"],
            "test_frameworks": [],
            "build_systems": ["cargo"],
            "verification_capabilities": ["cargo test"],
            "uncertainties": []
        },
        "work_items": [
            {
                "title": "实现后端登录会话 API",
                "kind": "backend",
                "sequence_hint": 10,
                "depends_on": [],
                "exclusive_write_scopes": ["src/product/session.rs"],
                "forbidden_write_scopes": ["web/**"],
                "required_handoff_from": [],
                "require_execution_plan_confirm": false
            },
            {
                "title": "实现前端会话过期提示",
                "kind": "frontend",
                "sequence_hint": 20,
                "depends_on": [0],
                "exclusive_write_scopes": ["web/src/session/**"],
                "forbidden_write_scopes": ["src/product/**"],
                "required_handoff_from": [],
                "require_execution_plan_confirm": false
            },
            {
                "title": "集成测试：会话过期端到端",
                "kind": "integration",
                "sequence_hint": 30,
                "depends_on": [1],
                "exclusive_write_scopes": ["tests/session/**"],
                "forbidden_write_scopes": [],
                "required_handoff_from": [],
                "require_execution_plan_confirm": false
            }
        ],
        "verification_plans": [
            {
                "scope": "unit",
                "commands": [
                    {
                        "label": "cargo test backend",
                        "command": "cargo test --lib session",
                        "cwd": "",
                        "purpose": "backend unit tests",
                        "required": true,
                        "timeout_seconds": 120,
                        "safety": "approved"
                    }
                ],
                "manual_checks": [],
                "required_gates": [],
                "risk_notes": [],
                "confidence": "high",
                "fallback_policy": "manual_gate"
            },
            {
                "scope": "unit",
                "commands": [
                    {
                        "label": "cargo test frontend",
                        "command": "cargo test --lib frontend_session",
                        "cwd": "",
                        "purpose": "frontend unit tests",
                        "required": true,
                        "timeout_seconds": 120,
                        "safety": "approved"
                    }
                ],
                "manual_checks": [],
                "required_gates": [],
                "risk_notes": [],
                "confidence": "high",
                "fallback_policy": "manual_gate"
            },
            {
                "scope": "integration",
                "commands": [
                    {
                        "label": "cargo test integration",
                        "command": "cargo test --test session_integration",
                        "cwd": "",
                        "purpose": "integration tests",
                        "required": true,
                        "timeout_seconds": 180,
                        "safety": "approved"
                    }
                ],
                "manual_checks": [],
                "required_gates": [],
                "risk_notes": [],
                "confidence": "high",
                "fallback_policy": "manual_gate"
            }
        ]
    })
}

fn invalid_split_output_missing_e2e() -> Value {
    json!({
        "repository_profile": {
            "confidence": "high",
            "detected_layers": ["backend"],
            "split_recommendation": "backend_only",
            "languages": ["rust"],
            "frameworks": [],
            "package_managers": ["cargo"],
            "test_frameworks": [],
            "build_systems": ["cargo"],
            "verification_capabilities": ["cargo test"],
            "uncertainties": []
        },
        "work_items": [
            {
                "title": "实现后端登录会话 API",
                "kind": "backend",
                "sequence_hint": 10,
                "depends_on": [],
                "exclusive_write_scopes": ["src/product/session.rs"],
                "forbidden_write_scopes": ["web/**"],
                "required_handoff_from": [],
                "require_execution_plan_confirm": false
            }
        ],
        "verification_plans": [
            {
                "scope": "unit",
                "commands": [
                    {
                        "label": "cargo test backend",
                        "command": "cargo test --lib session",
                        "cwd": "",
                        "purpose": "backend unit tests",
                        "required": true,
                        "timeout_seconds": 120,
                        "safety": "approved"
                    }
                ],
                "manual_checks": [],
                "required_gates": [],
                "risk_notes": [],
                "confidence": "high",
                "fallback_policy": "manual_gate"
            }
        ]
    })
}

#[tokio::test]
async fn generate_work_items_accepts_split_options_and_returns_plan_metadata() {
    let (app, _repo) = app_with_confirmed_story_and_design(valid_split_output()).await;

    let (status, response) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items:generate",
        json!({
            "title": "登录会话拆分实现",
            "story_spec_ids": ["story_spec_0001"],
            "design_spec_ids": ["design_spec_0001"],
            "include_integration_tests": true,
            "include_e2e_tests": false,
            "force_frontend_backend_split": true,
            "require_execution_plan_confirm": false
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(response["work_item_plan"]["status"], "draft");
    assert_eq!(
        response["work_item_plan"]["options"]["include_integration_tests"],
        true
    );
    assert_eq!(response["repository_profile"]["confidence"], "high");
    assert_eq!(response["verification_plans"].as_array().unwrap().len(), 3);
    assert_eq!(response["work_items"].as_array().unwrap().len(), 3);
    assert!(
        response["work_items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| {
                item["plan_status"] == "draft" && item["verification_plan_ref"].is_string()
            })
    );
    assert_eq!(response["workspace_sessions"].as_array().unwrap().len(), 3);
    assert_eq!(response["workspace_session"]["entity_id"], "work_item_0001");
    assert!(
        response["validator_findings"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn generate_work_items_creates_backend_frontend_and_integration_items_with_sessions() {
    let (app, _repo) = app_with_confirmed_story_and_design(valid_split_output()).await;

    let (status, response) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items:generate",
        json!({
            "title": "登录会话拆分实现",
            "story_spec_ids": ["story_spec_0001"],
            "design_spec_ids": ["design_spec_0001"],
            "include_integration_tests": true,
            "include_e2e_tests": false,
            "force_frontend_backend_split": true,
            "require_execution_plan_confirm": false
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let items = response["work_items"].as_array().unwrap();
    assert!(items.iter().any(|item| item["kind"] == "backend"));
    assert!(items.iter().any(|item| item["kind"] == "frontend"));
    assert!(items.iter().any(|item| item["kind"] == "integration"));
    assert_eq!(response["workspace_sessions"].as_array().unwrap().len(), 3);
}

#[tokio::test]
async fn generate_work_items_rejects_invalid_confirmed_refs_without_half_created_records() {
    let (app, _repo) =
        app_with_confirmed_story_and_design(invalid_split_output_missing_e2e()).await;

    let (status, response) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items:generate",
        json!({
            "title": "登录会话拆分实现",
            "story_spec_ids": ["story_spec_0001"],
            "design_spec_ids": ["design_spec_0001"],
            "include_integration_tests": true,
            "include_e2e_tests": false,
            "force_frontend_backend_split": true,
            "require_execution_plan_confirm": false
        }),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(response["code"], "work_item_split_invalid");
    let lifecycle = cadence_aria::product::lifecycle_store::LifecycleStore::new(
        cadence_aria::product::app_paths::ProductAppPaths::new(_repo.path().join(".aria")),
    );
    assert!(
        lifecycle
            .list_work_items("project_0001", "issue_0001")
            .unwrap()
            .is_empty()
    );
    assert!(
        lifecycle
            .list_issue_work_item_plans("project_0001", "issue_0001")
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn confirm_issue_work_item_plan_marks_work_items_confirmed() {
    let (app, _repo) = app_with_confirmed_story_and_design(valid_split_output()).await;

    let (status, response) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items:generate",
        json!({
            "title": "登录会话拆分实现",
            "story_spec_ids": ["story_spec_0001"],
            "design_spec_ids": ["design_spec_0001"],
            "include_integration_tests": true,
            "include_e2e_tests": false,
            "force_frontend_backend_split": true,
            "require_execution_plan_confirm": false
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let plan_id = response["work_item_plan"]["plan_id"].as_str().unwrap();

    let (status, response) = request_json(
        app,
        Method::POST,
        &format!("/api/projects/project_0001/issues/issue_0001/work-item-plans/{plan_id}/confirm"),
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(response["work_item_plan"]["status"], "confirmed");
    assert!(
        response["work_items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["plan_status"] == "confirmed")
    );
}

#[tokio::test]
async fn request_change_keeps_split_work_items_not_codeable() {
    let (app, _repo) = app_with_confirmed_story_and_design(valid_split_output()).await;

    let (status, response) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items:generate",
        json!({
            "title": "登录会话拆分实现",
            "story_spec_ids": ["story_spec_0001"],
            "design_spec_ids": ["design_spec_0001"],
            "include_integration_tests": true,
            "include_e2e_tests": false,
            "force_frontend_backend_split": true,
            "require_execution_plan_confirm": false
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let plan_id = response["work_item_plan"]["plan_id"].as_str().unwrap();

    let (status, response) = request_json(
        app,
        Method::POST,
        &format!(
            "/api/projects/project_0001/issues/issue_0001/work-item-plans/{plan_id}/change-request"
        ),
        json!({"note": "需要补充 e2e"}),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(response["work_item_plan"]["status"], "change_requested");
    assert!(
        response["work_items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["plan_status"] == "draft")
    );
}

pub(crate) async fn request_json(
    app: axum::Router,
    method: Method,
    uri: &str,
    body: Value,
) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("request");
    let response = app.oneshot(request).await.expect("response");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).expect("json")
    };
    (status, value)
}

async fn bootstrap_project_repo_issue_and_specs(
    app: axum::Router,
    repo: &std::path::Path,
) -> axum::Router {
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects",
        json!({"name":"Lifecycle","description":null}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/repositories",
        json!({"name":"Repo","path":repo}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues",
        json!({"title":"登录会话过期","description":"描述","repository_id":"repository_0001"}),
    )
    .await;

    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/story-specs:generate",
        json!({
            "title":"登录会话过期提示",
            "author_provider":"fake",
            "reviewer_provider":"codex",
            "review_rounds":3,
            "superpowers_enabled":false,
            "openspec_enabled":true
        }),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/workspace-sessions/workspace_session_0001/confirm",
        json!({"confirmed_by":"human"}),
    )
    .await;

    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/design-specs:generate",
        json!({
            "title":"会话过期后端设计",
            "story_spec_ids":["story_spec_0001"],
            "design_kind":"backend",
            "author_provider":"codex",
            "reviewer_provider":"claude_code",
            "review_rounds":2,
            "superpowers_enabled":true,
            "openspec_enabled":true
        }),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/workspace-sessions/workspace_session_0002/confirm",
        json!({"confirmed_by":"human"}),
    )
    .await;

    app
}

pub(crate) async fn app_with_confirmed_story_and_design(
    output: Value,
) -> (axum::Router, tempfile::TempDir) {
    let root = tempdir().expect("root");
    let repo = root.path().join("repo");
    std::fs::create_dir_all(&repo).expect("create repo dir");
    let status = Command::new("git")
        .args(["init"])
        .current_dir(&repo)
        .status()
        .expect("git init");
    assert!(status.success());

    let runtime = WebRuntime::new_fake(root.path().to_path_buf());
    let state = WebAppState::new(root.path().to_path_buf(), runtime)
        .with_provider_adapter(Arc::new(MockSplitProviderAdapter { output }));
    let app = build_web_router(state);
    let app = bootstrap_project_repo_issue_and_specs(app, &repo).await;

    (app, root)
}

/// 与 `app_with_confirmed_story_and_design` 相同，但额外把 codex/claude_code 也注册为
/// TestControlledFakeStreamingProvider，以便在 review 阶段通过 review fixture 注入固定 verdict。
pub(crate) async fn app_with_confirmed_story_and_design_and_test_providers(
    output: Value,
) -> (axum::Router, tempfile::TempDir) {
    let root = tempdir().expect("root");
    let repo = root.path().join("repo");
    std::fs::create_dir_all(&repo).expect("create repo dir");
    let status = Command::new("git")
        .args(["init"])
        .current_dir(&repo)
        .status()
        .expect("git init");
    assert!(status.success());

    let runtime = WebRuntime::new_fake(root.path().to_path_buf());
    let mut state = WebAppState::new(root.path().to_path_buf(), runtime)
        .with_provider_adapter(Arc::new(MockSplitProviderAdapter { output }));

    let mut registry = ProviderRegistry::new();
    let test_controls = cadence_aria::web::test_controls::TestControls::default();
    registry.register(
        ProviderName::Fake,
        Arc::new(TestControlledFakeStreamingProvider::new(
            test_controls.clone(),
        )),
    );
    registry.register(
        ProviderName::Codex,
        Arc::new(TestControlledFakeStreamingProvider::new(
            test_controls.clone(),
        )),
    );
    registry.register(
        ProviderName::ClaudeCode,
        Arc::new(TestControlledFakeStreamingProvider::new(
            test_controls.clone(),
        )),
    );
    state.test_controls = test_controls;
    state.provider_registry = Arc::new(registry);

    let app = build_web_router(state);
    let app = bootstrap_project_repo_issue_and_specs(app, &repo).await;

    (app, root)
}

#[tokio::test]
async fn prepare_work_item_plan_creates_draft_plan_and_session_without_generating() {
    let (app, _repo) = app_with_confirmed_story_and_design(valid_split_output()).await;

    let (status, response) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-item-plans:prepare",
        json!({
            "title": "爬楼梯问题 Work Item Plan",
            "story_spec_ids": ["story_spec_0001"],
            "design_spec_ids": ["design_spec_0001"],
            "author_provider": "fake",
            "reviewer_provider": "codex",
            "review_rounds": 1,
            "superpowers_enabled": true,
            "openspec_enabled": true,
            "include_integration_tests": true,
            "include_e2e_tests": false,
            "force_frontend_backend_split": true,
            "require_execution_plan_confirm": false
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(response["work_item_plan"]["status"], "draft");
    assert_eq!(
        response["work_item_plan"]["options"]["include_integration_tests"],
        true
    );
    assert_eq!(
        response["work_item_plan"]["options"]["include_e2e_tests"],
        false
    );
    assert_eq!(
        response["work_item_plan"]["options"]["force_frontend_backend_split"],
        true
    );
    assert_eq!(
        response["work_item_plan"]["options"]["require_execution_plan_confirm"],
        false
    );
    assert!(
        response["work_item_plan"]["work_item_ids"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        response["work_item_plan"]["verification_plan_ids"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        response["work_item_plan"]["dependency_graph"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        response["workspace_session"]["workspace_type"],
        "work_item_plan"
    );
    assert_eq!(
        response["workspace_session"]["entity_id"],
        response["work_item_plan"]["id"]
    );

    let lifecycle = cadence_aria::product::lifecycle_store::LifecycleStore::new(
        cadence_aria::product::app_paths::ProductAppPaths::new(_repo.path().join(".aria")),
    );
    assert!(
        lifecycle
            .list_work_items("project_0001", "issue_0001")
            .unwrap()
            .is_empty()
    );

    let first_message = &response["workspace_session"]["messages"][0]["content"];
    assert!(
        first_message
            .as_str()
            .unwrap()
            .contains("候选 work item plan 生成器")
    );
}
