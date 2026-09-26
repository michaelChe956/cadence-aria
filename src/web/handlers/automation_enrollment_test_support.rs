//! P0 enrollment / P1 automation-target HTTP 测试共用 fixture。
//!
//! 从 `automation_enrollment.rs` 测试模块原样抽出（生产签名零变化）：
//! 播种 project + issue + N 成员 logical codebase + 已确认 story/design，
//! 并提供 enrollment PUT/GET/binding 的真实 HTTP 调用 helper。

#![cfg(test)]

use tower::ServiceExt;

use axum::body::Body;
use tempfile::TempDir;

use crate::product::app_paths::ProductAppPaths;
use crate::product::issue_store::{CreateProductIssueInput, IssueStore};
use crate::product::lifecycle_store::{
    AppendSpecVersionInput, CreateDesignSpecInput, CreateIssueWorkItemPlanInput,
    CreateStorySpecInput, CreateWorkspaceSessionInput, LifecycleStore, WorkItemPlanSessionOptions,
};
use crate::product::logical_codebase::aggregate_index::{
    AggregateIndexMemberSnapshot, AggregateIndexRecord, AggregateIndexStatus, AggregateIndexStore,
};
use crate::product::logical_codebase::policy::AggregatePolicyArtifactStore;
use crate::product::logical_codebase::{
    CheckoutAvailability, CheckoutKind, CodebaseMemberRecord, IssueCodebaseSelection,
    IssueCodebaseSelectionStore, LogicalCodebaseManifest, LogicalCodebaseStore,
    LogicalRepositoryId, MemberStatus, RepositoryCheckoutId, RepositoryCheckoutRecord,
    RepositorySourceIdentity, RepositoryType,
};
use crate::product::models::{
    IssueWorkItemPlanOptions, IssueWorkItemPlanStatus, LifecycleConfirmationStatus, ProviderName,
    RepositoryRecord, WorkspaceType,
};
use crate::product::project_store::{CreateProjectInput, ProjectStore};
use crate::product::work_item_plan_policy::{RunPolicy, WorkItemPlanFlowKind};
use crate::web::app::build_web_router;
use crate::web::runtime::WebRuntime;
use crate::web::state::WebAppState;

pub(crate) const PROJECT_ID: &str = "project_0001";
pub(crate) const ISSUE_ID: &str = "issue_0001";
pub(crate) const REPOSITORY_ID: &str = "repo-1";
pub(crate) const SINGLE_LOGICAL_ID: &str = "00000000-0000-0000-0000-000000000001";

pub(crate) struct Fixture {
    pub(crate) _root: TempDir,
    pub(crate) paths: ProductAppPaths,
    pub(crate) lifecycle: LifecycleStore,
    pub(crate) story_id: String,
    pub(crate) design_id: String,
}

impl Fixture {
    pub(crate) fn router(&self) -> axum::Router {
        let root = self._root.path();
        build_web_router(WebAppState::new(
            root.to_path_buf(),
            WebRuntime::new_fake(root.to_path_buf()),
        ))
    }
}

/// 播种：project + issue + N 成员 logical codebase + 已确认 story/design（version 1）。
pub(crate) fn seed_fixture(member_count: usize, confirm_design: bool) -> Fixture {
    let root = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    ProjectStore::new(paths.clone())
        .create(CreateProjectInput {
            name: "enrollment test project".to_string(),
            description: None,
        })
        .unwrap();
    IssueStore::new(paths.clone())
        .create(CreateProductIssueInput {
            project_id: PROJECT_ID.to_string(),
            repo_id: Some(REPOSITORY_ID.to_string()),
            logical_codebase_id: None,
            title: "enrollment test issue".to_string(),
            description: None,
            change_id: None,
            base_branch: None,
        })
        .unwrap();

    let names = ["checkout-enroll-a", "checkout-enroll-b"];
    let members = names
        .into_iter()
        .take(member_count)
        .enumerate()
        .map(|(index, name)| {
            (
                LogicalRepositoryId(uuid::Uuid::from_u128(index as u128 + 1)),
                name,
            )
        })
        .collect::<Vec<_>>();
    if !members.is_empty() {
        seed_logical_codebase(&paths, &members);
    }

    let lifecycle = LifecycleStore::new(paths.clone());
    let story = lifecycle
        .create_story_spec(CreateStorySpecInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            repository_id: REPOSITORY_ID.to_string(),
            title: "enrollment story".to_string(),
            aggregate_codebase: None,
        })
        .unwrap();
    lifecycle
        .append_version(AppendSpecVersionInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            entity_id: story.id.clone(),
            markdown: "# story".to_string(),
            provider_run_refs: Vec::new(),
            review_refs: Vec::new(),
            confirmed_by: None,
        })
        .unwrap();
    lifecycle
        .update_spec_confirmation_status(
            PROJECT_ID,
            ISSUE_ID,
            &story.id,
            LifecycleConfirmationStatus::Confirmed,
        )
        .unwrap();

    let design = lifecycle
        .create_design_spec(CreateDesignSpecInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            story_spec_ids: vec![story.id.clone()],
            title: "enrollment design".to_string(),
            aggregate_codebase: None,
        })
        .unwrap();
    lifecycle
        .append_version(AppendSpecVersionInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            entity_id: design.id.clone(),
            markdown: "# design".to_string(),
            provider_run_refs: Vec::new(),
            review_refs: Vec::new(),
            confirmed_by: None,
        })
        .unwrap();
    if confirm_design {
        lifecycle
            .update_spec_confirmation_status(
                PROJECT_ID,
                ISSUE_ID,
                &design.id,
                LifecycleConfirmationStatus::Confirmed,
            )
            .unwrap();
    }

    Fixture {
        _root: root,
        paths,
        lifecycle,
        story_id: story.id,
        design_id: design.id,
    }
}

fn seed_logical_codebase(paths: &ProductAppPaths, members: &[(LogicalRepositoryId, &str)]) {
    // planning resolver 读取 manifest 的 provider_context_root 时要求目录存在。
    std::fs::create_dir_all(paths.root().join("aggregate-root")).unwrap();
    let manifest = LogicalCodebaseManifest::new(
        PROJECT_ID,
        paths.root().join("aggregate-root"),
        members.iter().map(|(id, _)| *id).collect(),
    );
    LogicalCodebaseStore::new(paths.clone())
        .save_manifest(PROJECT_ID, &manifest)
        .unwrap();
    let now = "2026-09-26T00:00:00Z".to_string();
    let mut repositories = Vec::new();
    for (index, (logical_id, checkout_name)) in members.iter().enumerate() {
        let physical_repository_id = format!("physical-{checkout_name}");
        let checkout_path = paths.root().join(checkout_name);
        let source_identity = RepositorySourceIdentity::from_git_parts(
            &checkout_path,
            checkout_path.join(".git"),
            None,
        );
        let checkout_id = RepositoryCheckoutId(uuid::Uuid::new_v4());
        LogicalCodebaseStore::new(paths.clone())
            .save_member(
                PROJECT_ID,
                &CodebaseMemberRecord {
                    logical_repository_id: *logical_id,
                    physical_repository_id: physical_repository_id.clone(),
                    alias: REPOSITORY_ID.to_string(),
                    role: "repository".to_string(),
                    ordinal: (index + 1) as u32,
                    source_identity: source_identity.clone(),
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
        LogicalCodebaseStore::new(paths.clone())
            .save_checkout(
                PROJECT_ID,
                &RepositoryCheckoutRecord {
                    checkout_id,
                    logical_repository_id: *logical_id,
                    physical_repository_id: physical_repository_id.clone(),
                    kind: CheckoutKind::Main,
                    canonical_path: checkout_path.clone(),
                    checkout_path_hash: format!("sha256:{checkout_name}"),
                    git_dir_identity: source_identity.git_dir_identity(),
                    revision: Some("abcdef".to_string()),
                    availability: CheckoutAvailability::Available,
                    observed_at: now.clone(),
                    created_at: now.clone(),
                    updated_at: now.clone(),
                },
            )
            .unwrap();
        repositories.push(RepositoryRecord {
            id: physical_repository_id,
            project_id: PROJECT_ID.to_string(),
            name: REPOSITORY_ID.to_string(),
            path: checkout_path,
            repo_hash: format!("sha256:{checkout_name}"),
            runtime_root: paths.root().join(checkout_name).join(".aria/runtime"),
            default_policy_preset: "manual-write".to_string(),
            default_provider_mode: "fake".to_string(),
            created_at: now.clone(),
            logical_repository_id: Some(*logical_id),
            identity_schema_version: 1,
            primary_checkout_id: Some(checkout_id),
            updated_at: now.clone(),
        });
    }
    crate::product::json_store::write_json(
        &paths.project_root(PROJECT_ID).join("repos.json"),
        &repositories,
    )
    .unwrap();
    // active aggregate index + bootstrap policy：prepare 的 planning resolver 必读。
    let mut index = AggregateIndexRecord::building(
        "aggregate_index_0001".to_string(),
        PROJECT_ID.to_string(),
        1,
        members
            .iter()
            .map(|(logical_id, _)| {
                AggregateIndexMemberSnapshot::indexed(
                    *logical_id,
                    RepositoryCheckoutId(uuid::Uuid::nil()),
                    "abc123".to_string(),
                    false,
                    now.clone(),
                )
            })
            .collect(),
        now.clone(),
    );
    index.status = AggregateIndexStatus::Active;
    let index_store = AggregateIndexStore::new(paths.clone());
    index_store.create(PROJECT_ID, index.clone()).unwrap();
    index_store.replace_active(PROJECT_ID, index).unwrap();
    // 单测环境无 git/codegraph：降级为 last-known-good（active_required 仍可读，
    // planning resolver 的 freshness assess 不再探测真实仓库）。
    index_store
        .degrade_last_known_good(PROJECT_ID, "unit-test fixture: no git probing".to_string())
        .unwrap();
    AggregatePolicyArtifactStore::new(paths.clone())
        .ensure_bootstrap(&manifest)
        .unwrap();
    IssueCodebaseSelectionStore::new(paths.clone())
        .save(&IssueCodebaseSelection::explicit(
            PROJECT_ID,
            ISSUE_ID,
            members.iter().map(|(id, _)| *id).collect(),
            Vec::new(),
            Vec::new(),
            None,
        ))
        .unwrap();
}

pub(crate) fn enrollment_body(
    fixture: &Fixture,
    story_version: u32,
    design_version: u32,
) -> serde_json::Value {
    serde_json::json!({
        "expected_revision": null,
        "command": {
            "type": "enable",
            "selection_key": "human-choice-1",
            "source": {
                "stories": [{"id": fixture.story_id, "version": story_version}],
                "designs": [{"id": fixture.design_id, "version": design_version}]
            },
            "options": {
                "author_provider": "fake",
                "reviewer_provider": "fake",
                "review_rounds": 1,
                "superpowers_enabled": false,
                "openspec_enabled": false,
                "plan_options": {
                    "include_integration_tests": true,
                    "include_e2e_tests": false,
                    "force_frontend_backend_split": false,
                    "require_execution_plan_confirm": false
                }
            },
            "logical_repository_id": SINGLE_LOGICAL_ID
        }
    })
}

pub(crate) async fn put_enrollment(
    app: &axum::Router,
    body: serde_json::Value,
) -> axum::http::Response<Body> {
    app.clone()
        .oneshot(
            axum::http::Request::builder()
                .method("PUT")
                .uri(format!(
                    "/api/projects/{PROJECT_ID}/issues/{ISSUE_ID}/automation-enrollment"
                ))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap()
}

pub(crate) async fn get_enrollment(app: &axum::Router) -> axum::http::Response<Body> {
    app.clone()
        .oneshot(
            axum::http::Request::builder()
                .uri(format!(
                    "/api/projects/{PROJECT_ID}/issues/{ISSUE_ID}/automation-enrollment"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
}

pub(crate) async fn post_binding(
    app: &axum::Router,
    body: serde_json::Value,
) -> axum::http::Response<Body> {
    app.clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri(format!(
                    "/api/projects/{PROJECT_ID}/issues/{ISSUE_ID}/automation-enrollment/binding"
                ))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap()
}

/// 直接经 store 显式新建 plan + WorkItemPlan 会话（绑定只认显式对象）。
pub(crate) fn create_plan_and_session(
    fixture: &Fixture,
    run_policy: RunPolicy,
) -> (String, String) {
    let plan = fixture
        .lifecycle
        .create_issue_work_item_plan(CreateIssueWorkItemPlanInput {
            id: None,
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            source_story_spec_ids: vec![fixture.story_id.clone()],
            source_design_spec_ids: vec![fixture.design_id.clone()],
            options: IssueWorkItemPlanOptions {
                include_integration_tests: true,
                include_e2e_tests: false,
                force_frontend_backend_split: false,
                require_execution_plan_confirm: false,
            },
            status: IssueWorkItemPlanStatus::Draft,
            work_item_ids: Vec::new(),
            repository_profile_ref: None,
            verification_plan_ids: Vec::new(),
            dependency_graph: Vec::new(),
            created_from_provider_run: None,
            validator_findings: Vec::new(),
        })
        .unwrap();
    let session = fixture
        .lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            entity_id: plan.id.clone(),
            workspace_type: WorkspaceType::WorkItemPlan,
            author_provider: ProviderName::Fake,
            reviewer_provider: ProviderName::Fake,
            review_rounds: 1,
            superpowers_enabled: false,
            openspec_enabled: false,
            work_item_plan_options: Some(WorkItemPlanSessionOptions {
                flow_kind: WorkItemPlanFlowKind::SingleCandidate,
                run_policy,
                rollout_snapshot: false,
            }),
        })
        .unwrap();
    (plan.id, session.id)
}

pub(crate) async fn response_json(response: axum::http::Response<Body>) -> serde_json::Value {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

pub(crate) fn enrollment_file_exists(fixture: &Fixture) -> bool {
    fixture
        .paths
        .issue_root(PROJECT_ID, ISSUE_ID)
        .join("automation-enrollment.json")
        .exists()
}
