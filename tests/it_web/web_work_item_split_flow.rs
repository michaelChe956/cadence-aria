use axum::http::{Method, StatusCode};
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::coding_attempt_store::CodingAttemptStore;
use cadence_aria::product::lifecycle_store::LifecycleStore;
use cadence_aria::product::models::WorkItemStatus;
use cadence_aria::product::work_item_split_validator::WorkItemSplitValidator;
use cadence_aria::web::app::build_web_router;
use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::state::WebAppState;
use serde_json::{Value, json};
use std::path::Path;
use std::sync::Arc;
use tempfile::tempdir;

use crate::web_coding_attempt_api::{
    bootstrap_story_and_design, git_repo, prepare_attempt_with_worktree, request_json,
};
use crate::web_work_item_generation::{
    MockSplitProviderAdapter, app_with_confirmed_story_and_design, valid_split_output,
};

#[tokio::test]
async fn work_item_split_flow_blocks_frontend_until_backend_handoff_exists() {
    let (app, root) = app_with_confirmed_story_and_design(valid_split_output()).await;

    let (status, generated) = request_json(
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
    assert_eq!(generated["work_items"].as_array().unwrap().len(), 3);
    assert_eq!(generated["work_item_plan"]["status"], "draft");
    assert_eq!(generated["repository_profile"]["confidence"], "high");
    assert_eq!(generated["verification_plans"].as_array().unwrap().len(), 3);
    let plan_id = generated["work_item_plan"]["plan_id"].as_str().unwrap();

    let (status, confirmed) = request_json(
        app.clone(),
        Method::POST,
        &format!("/api/projects/project_0001/issues/issue_0001/work-item-plans/{plan_id}/confirm"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        confirmed["work_items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| {
                item["plan_status"] == "confirmed" && item["verification_plan_ref"].is_string()
            })
    );

    let (status, blocked) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0002/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(blocked["code"], "work_item_dependency_not_completed");

    mark_work_item_completed_with_handoff(
        root.path(),
        "project_0001",
        "issue_0001",
        "work_item_0001",
        "handoffs/work_item_0001.json",
    );

    let (status, attempt) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0002/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(attempt["branch_name"], "aria/issues/issue_0001");
}

#[tokio::test]
async fn work_item_split_records_risk_when_integration_and_e2e_are_skipped() {
    let (app, root) =
        app_with_confirmed_story_and_design(split_output_without_integration_e2e()).await;

    let (status, response) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items:generate",
        json!({
            "title": "登录会话拆分实现",
            "story_spec_ids": ["story_spec_0001"],
            "design_spec_ids": ["design_spec_0001"],
            "include_integration_tests": false,
            "include_e2e_tests": false,
            "force_frontend_backend_split": true,
            "require_execution_plan_confirm": false
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(response["work_items"].as_array().unwrap().len(), 2);

    // The generate response currently suppresses warning-level validator findings;
    // verify the risk is recorded at the store level.
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths);
    let plan_id = response["work_item_plan"]["plan_id"].as_str().unwrap();
    let plan = lifecycle
        .get_issue_work_item_plan("project_0001", "issue_0001", plan_id)
        .expect("plan");
    let work_items = lifecycle
        .list_work_items("project_0001", "issue_0001")
        .expect("work items");
    let profile_ref = plan.repository_profile_ref.as_deref().expect("profile ref");
    let profile = lifecycle
        .get_repository_profile("project_0001", "issue_0001", profile_ref)
        .expect("profile");
    let verification_plans = lifecycle
        .list_verification_plans("project_0001", "issue_0001")
        .expect("verification plans");
    let report =
        WorkItemSplitValidator::validate(&plan, &work_items, Some(&profile), &verification_plans);
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.code == "integration_or_e2e_skipped_risk")
    );
}

#[tokio::test]
async fn work_item_split_e2e_item_waits_for_backend_and_frontend() {
    let (app, _root) = app_with_confirmed_story_and_design(split_output_with_e2e()).await;

    let (status, response) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items:generate",
        json!({
            "title": "登录会话拆分实现",
            "story_spec_ids": ["story_spec_0001"],
            "design_spec_ids": ["design_spec_0001"],
            "include_integration_tests": false,
            "include_e2e_tests": true,
            "force_frontend_backend_split": true,
            "require_execution_plan_confirm": false
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let items = response["work_items"].as_array().unwrap();
    assert_eq!(items.len(), 3);
    let e2e_item = items
        .iter()
        .find(|item| item["kind"] == "e2e")
        .expect("e2e item");
    let deps: Vec<&str> = e2e_item["depends_on"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap())
        .collect();
    assert!(deps.contains(&"work_item_0001"));
    assert!(deps.contains(&"work_item_0002"));
}

#[tokio::test]
async fn dirty_shared_worktree_blocks_next_work_item_until_manual_gate_resolved() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(
        WebAppState::new(
            root.path().to_path_buf(),
            WebRuntime::new_fake(root.path().to_path_buf()),
        )
        .with_provider_adapter(Arc::new(MockSplitProviderAdapter {
            output: two_ready_split_output(),
            revision_output: None,
        })),
    );
    bootstrap_confirmed_split_plan_with_two_ready_work_items(app.clone(), root.path(), repo.path())
        .await;

    let (_status, first) =
        create_coding_attempt(app.clone(), root.path(), repo.path(), "work_item_0001").await;
    dirty_issue_shared_worktree(repo.path(), "issue_0001");

    let (status, failed) = abort_attempt(app.clone(), first["attempt_id"].as_str().unwrap()).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(failed["code"], "shared_worktree_dirty_manual_gate");

    let (status, second) =
        create_coding_attempt(app, root.path(), repo.path(), "work_item_0002").await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(second["code"], "issue_worktree_active");
}

async fn bootstrap_confirmed_split_plan_with_two_ready_work_items(
    app: axum::Router,
    _root_path: &Path,
    repo_path: &Path,
) {
    bootstrap_story_and_design(app.clone(), repo_path).await;

    let (status, generated) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items:generate",
        json!({
            "title": "登录会话拆分实现",
            "story_spec_ids": ["story_spec_0001"],
            "design_spec_ids": ["design_spec_0001"],
            "include_integration_tests": false,
            "include_e2e_tests": false,
            "force_frontend_backend_split": true,
            "require_execution_plan_confirm": false
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let plan_id = generated["work_item_plan"]["plan_id"].as_str().unwrap();
    let (status, _confirmed) = request_json(
        app,
        Method::POST,
        &format!("/api/projects/project_0001/issues/issue_0001/work-item-plans/{plan_id}/confirm"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

async fn create_coding_attempt(
    app: axum::Router,
    root_path: &Path,
    repo_path: &Path,
    work_item_id: &str,
) -> (StatusCode, Value) {
    let (status, attempt) = request_json(
        app.clone(),
        Method::POST,
        &format!(
            "/api/projects/project_0001/issues/issue_0001/work-items/{work_item_id}/coding-attempts"
        ),
        json!({}),
    )
    .await;

    if status == StatusCode::OK {
        let store = CodingAttemptStore::new(ProductAppPaths::new(root_path.join(".aria")));
        prepare_attempt_with_worktree(
            &store,
            repo_path,
            "project_0001",
            "issue_0001",
            attempt["attempt_id"].as_str().unwrap(),
        );
    }

    (status, attempt)
}

async fn abort_attempt(app: axum::Router, attempt_id: &str) -> (StatusCode, Value) {
    request_json(
        app,
        Method::POST,
        &format!("/api/coding-attempts/{attempt_id}/abort"),
        json!({}),
    )
    .await
}

fn dirty_issue_shared_worktree(repo_path: &Path, issue_id: &str) {
    let flag = repo_path
        .join(".worktrees")
        .join("aria-issues")
        .join(issue_id)
        .join("dirty-flag.txt");
    std::fs::create_dir_all(flag.parent().unwrap()).expect("worktree parent");
    std::fs::write(&flag, "dirty\n").expect("dirty flag");
}

fn mark_work_item_completed_with_handoff(
    root_path: &Path,
    project_id: &str,
    issue_id: &str,
    work_item_id: &str,
    handoff_ref: &str,
) {
    let app_paths = ProductAppPaths::new(root_path.join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths);
    lifecycle
        .update_work_item_execution_status(
            project_id,
            issue_id,
            work_item_id,
            WorkItemStatus::Completed,
        )
        .expect("complete work item");
    lifecycle
        .update_work_item_handoff_summary(
            project_id,
            issue_id,
            work_item_id,
            Some(handoff_ref.to_string()),
            None,
        )
        .expect("set handoff summary");
}

fn split_output_without_integration_e2e() -> Value {
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
                "depends_on": [],
                "exclusive_write_scopes": ["web/src/session/**"],
                "forbidden_write_scopes": ["src/product/**"],
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
            }
        ]
    })
}

fn split_output_with_e2e() -> Value {
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
                "title": "端到端测试：会话过期",
                "kind": "e2e",
                "sequence_hint": 30,
                "depends_on": [0, 1],
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
                "scope": "e2e",
                "commands": [
                    {
                        "label": "cargo test e2e session",
                        "command": "cargo test --test session_e2e",
                        "cwd": "",
                        "purpose": "e2e tests",
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

fn two_ready_split_output() -> Value {
    split_output_without_integration_e2e()
}
