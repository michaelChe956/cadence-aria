use std::sync::Arc;

use cadence_aria::cross_cutting::provider_adapter::{ProviderAdapter, ProviderAdapterError};
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::issue_store::{CreateProductIssueInput, IssueStore};
use cadence_aria::product::lifecycle_store::{
    AppendSpecVersionInput, CreateDesignSpecInput, CreateStorySpecInput, CreateWorkItemInput,
    LifecycleStore,
};
use cadence_aria::product::models::{
    IssueRecord, ProviderName, RepositoryRecord, WorkItemKind, WorkItemPlanStatus,
};
use cadence_aria::product::project_store::{CreateProjectInput, ProjectStore};
use cadence_aria::product::repository_store::{CreateRepositoryInput, RepositoryStore};
use cadence_aria::product::work_item_split_engine::{
    RedoSpec, WorkItemSplitEngine, repatch_dependencies,
};
use cadence_aria::protocol::contracts::{AdapterInput, AdapterOutput, TimeoutStatus};
use cadence_aria::web::types::GenerateWorkItemsRequest;
use serde_json::{Value, json};
use tempfile::TempDir;

#[derive(Debug, Clone)]
struct MockSplitProviderAdapter {
    output: Value,
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

fn default_request() -> GenerateWorkItemsRequest {
    GenerateWorkItemsRequest {
        title: "登录会话拆分实现".to_string(),
        story_spec_ids: vec!["story_spec_0001".to_string()],
        design_spec_ids: vec!["design_spec_0001".to_string()],
        include_integration_tests: Some(true),
        include_e2e_tests: Some(false),
        force_frontend_backend_split: Some(true),
        require_execution_plan_confirm: Some(false),
        author_provider: None,
        reviewer_provider: None,
        review_rounds: None,
        superpowers_enabled: None,
        openspec_enabled: None,
        revision_feedback: None,
    }
}

fn valid_split_output() -> Value {
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

fn valid_split_output_with_type_field() -> Value {
    // 真实 claude provider 习惯输出 `type` 而非 schema 要求的 `kind`。
    // 回归 Bug: prompt 未内联 schema + ProviderWorkItem 缺少 alias 时,
    // provider 返回 `type` 会导致 `missing field kind` 解析失败。
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
                "type": "backend",
                "sequence_hint": 10,
                "depends_on": [],
                "exclusive_write_scopes": ["src/product/session.rs"],
                "forbidden_write_scopes": ["web/**"],
                "required_handoff_from": [],
                "require_execution_plan_confirm": false
            },
            {
                "title": "实现前端会话过期提示",
                "type": "frontend",
                "sequence_hint": 20,
                "depends_on": [0],
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

fn redo_only_output() -> Value {
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
                "title": "实现前端会话过期提示（重做）",
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
                        "label": "cargo test frontend redo",
                        "command": "cargo test --lib frontend_session_redo",
                        "cwd": "",
                        "purpose": "frontend unit tests after redo",
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

async fn split_engine_fixture() -> (TempDir, LifecycleStore, IssueRecord, RepositoryRecord) {
    let root = TempDir::new().expect("tempdir");
    let repo_path = root.path().join("repo");
    std::fs::create_dir_all(&repo_path).expect("create repo dir");
    let status = std::process::Command::new("git")
        .args(["init"])
        .current_dir(&repo_path)
        .status()
        .expect("git init");
    assert!(status.success());

    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let project_store = ProjectStore::new(paths.clone());
    let project = project_store
        .create(CreateProjectInput {
            name: "Test".to_string(),
            description: None,
        })
        .expect("create project");

    let repo_store = RepositoryStore::new(paths.clone());
    let repository = repo_store
        .create(CreateRepositoryInput {
            project_id: project.id.clone(),
            name: "repo".to_string(),
            path: repo_path.clone(),
            default_policy_preset: None,
            default_provider_mode: None,
        })
        .expect("create repository");

    let issue_store = IssueStore::new(paths.clone());
    let issue = issue_store
        .create(CreateProductIssueInput {
            project_id: project.id.clone(),
            repo_id: Some(repository.id.clone()),
            title: "登录会话过期".to_string(),
            description: Some("描述".to_string()),
            change_id: None,
        })
        .expect("create issue");

    let lifecycle = LifecycleStore::new(paths);
    let story = lifecycle
        .create_story_spec(CreateStorySpecInput {
            project_id: project.id,
            issue_id: issue.id.clone(),
            repository_id: repository.id.clone(),
            title: "登录用户看到会话过期提示".to_string(),
        })
        .expect("create story spec");
    lifecycle
        .append_version(AppendSpecVersionInput {
            project_id: story.project_id.clone(),
            issue_id: issue.id.clone(),
            entity_id: story.id,
            markdown: "# Story\n\n会话过期提示。".to_string(),
            provider_run_refs: vec![],
            review_refs: vec![],
            confirmed_by: None,
        })
        .expect("append story version");

    let design = lifecycle
        .create_design_spec(CreateDesignSpecInput {
            project_id: story.project_id.clone(),
            issue_id: issue.id.clone(),
            story_spec_ids: vec!["story_spec_0001".to_string()],
            title: "会话过期后端设计".to_string(),
        })
        .expect("create design spec");
    lifecycle
        .append_version(AppendSpecVersionInput {
            project_id: design.project_id,
            issue_id: issue.id.clone(),
            entity_id: design.id,
            markdown: "# Design\n\n后端设计。".to_string(),
            provider_run_refs: vec![],
            review_refs: vec![],
            confirmed_by: None,
        })
        .expect("append design version");

    (root, lifecycle, issue, repository)
}

fn engine_with_output(output: Value) -> WorkItemSplitEngine {
    WorkItemSplitEngine::new(Arc::new(MockSplitProviderAdapter { output }))
}

#[test]
fn repatch_dependencies_reconnects_dependents() {
    use cadence_aria::product::models::IssueWorkItemDependencyEdge;
    use std::collections::HashMap;

    let graph = vec![
        IssueWorkItemDependencyEdge {
            from_work_item_id: "work_item_0001".into(),
            to_work_item_id: "work_item_0002".into(),
        },
        IssueWorkItemDependencyEdge {
            from_work_item_id: "work_item_0001".into(),
            to_work_item_id: "work_item_0003".into(),
        },
        IssueWorkItemDependencyEdge {
            from_work_item_id: "work_item_0002".into(),
            to_work_item_id: "work_item_0003".into(),
        },
    ];

    let mut mapping = HashMap::new();
    mapping.insert("work_item_0001".to_string(), "work_item_0009".to_string());
    let repatched = repatch_dependencies(&graph, &mapping);

    assert!(
        repatched
            .iter()
            .all(|e| e.from_work_item_id != "work_item_0001"
                && e.to_work_item_id != "work_item_0001")
    );
    assert!(
        repatched
            .iter()
            .any(|e| e.from_work_item_id == "work_item_0009"
                && e.to_work_item_id == "work_item_0002")
    );
    assert!(
        repatched
            .iter()
            .any(|e| e.from_work_item_id == "work_item_0009"
                && e.to_work_item_id == "work_item_0003")
    );
    assert!(
        repatched
            .iter()
            .any(|e| e.from_work_item_id == "work_item_0002"
                && e.to_work_item_id == "work_item_0003")
    );
    assert_eq!(repatched.len(), 3);
}

#[tokio::test]
async fn generate_accepts_type_field_as_kind_alias() {
    // 回归 Bug: 真实 claude provider 输出 `work_items[].type` 而非 `kind`。
    // ProviderWorkItem.kind 必须接受 `type` 别名,否则 serde 报
    // `missing field kind`,WorkItemPlan 生成静默失败。
    let (_dir, lifecycle, issue, repository) = split_engine_fixture().await;
    let request = default_request();

    let output = engine_with_output(valid_split_output_with_type_field())
        .generate(
            &request,
            &lifecycle,
            &issue,
            &repository,
            ProviderName::Fake,
        )
        .await
        .expect("split with type alias should parse");

    assert_eq!(output.work_items.len(), 2);
    assert!(
        output
            .work_items
            .iter()
            .any(|wi| wi.kind == WorkItemKind::Backend)
    );
    assert!(
        output
            .work_items
            .iter()
            .any(|wi| wi.kind == WorkItemKind::Frontend)
    );
}

#[tokio::test]
async fn generate_revision_keeps_retained_and_redoes_marked() {
    let (_dir, lifecycle, issue, repository) = split_engine_fixture().await;
    let request = default_request();

    let initial = engine_with_output(valid_split_output())
        .generate(
            &request,
            &lifecycle,
            &issue,
            &repository,
            ProviderName::Fake,
        )
        .await
        .expect("initial split");
    assert_eq!(initial.work_items.len(), 2);

    // Persist the initial records so count_work_items reflects reality.
    for item in &initial.work_items {
        lifecycle
            .create_work_item(CreateWorkItemInput {
                id: Some(item.id.clone()),
                project_id: item.project_id.clone(),
                issue_id: item.issue_id.clone(),
                repository_id: item.repository_id.clone(),
                story_spec_ids: item.story_spec_ids.clone(),
                design_spec_ids: item.design_spec_ids.clone(),
                title: item.title.clone(),
                work_item_set_id: item.work_item_set_id.clone(),
                kind: item.kind.clone(),
                sequence_hint: item.sequence_hint,
                depends_on: item.depends_on.clone(),
                exclusive_write_scopes: item.exclusive_write_scopes.clone(),
                forbidden_write_scopes: item.forbidden_write_scopes.clone(),
                context_budget: item.context_budget.clone(),
                required_handoff_from: item.required_handoff_from.clone(),
                verification_plan_ref: item.verification_plan_ref.clone(),
                require_execution_plan_confirm: item.require_execution_plan_confirm,
                plan_status: WorkItemPlanStatus::Confirmed,
            })
            .expect("persist work item");
    }

    let retained = vec![initial.work_items[0].clone()];
    let redo_specs = vec![RedoSpec {
        old_id: initial.work_items[1].id.clone(),
        feedback: "拆得太粗".to_string(),
    }];

    let output = engine_with_output(redo_only_output())
        .generate_revision(
            &request,
            &lifecycle,
            &issue,
            &repository,
            ProviderName::Fake,
            &retained,
            &redo_specs,
        )
        .await
        .expect("revision");

    assert!(
        output
            .work_items
            .iter()
            .any(|wi| wi.id == initial.work_items[0].id)
    );
    assert!(
        output
            .work_items
            .iter()
            .all(|wi| wi.id != initial.work_items[1].id)
    );
    assert_eq!(output.work_items.len(), retained.len() + redo_specs.len());
}

#[tokio::test]
async fn retained_redo_empty_falls_back_to_full_split() {
    let (_dir, lifecycle, issue, repository) = split_engine_fixture().await;
    let request = default_request();

    let output = engine_with_output(valid_split_output())
        .generate_revision(
            &request,
            &lifecycle,
            &issue,
            &repository,
            ProviderName::Fake,
            &[],
            &[],
        )
        .await
        .expect("revision fallback");

    assert_eq!(output.work_items.len(), 2);
    assert!(
        output
            .work_items
            .iter()
            .any(|wi| wi.kind == WorkItemKind::Backend)
    );
    assert!(
        output
            .work_items
            .iter()
            .any(|wi| wi.kind == WorkItemKind::Frontend)
    );
}
