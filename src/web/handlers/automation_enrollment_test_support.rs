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
pub(crate) const FOUR_MEMBER_LOGICAL_IDS: [&str; 4] = [
    "00000000-0000-0000-0000-000000000001",
    "00000000-0000-0000-0000-000000000002",
    "00000000-0000-0000-0000-000000000003",
    "00000000-0000-0000-0000-000000000004",
];

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

    let names = [
        "checkout-enroll-a",
        "checkout-enroll-b",
        "checkout-enroll-c",
        "checkout-enroll-d",
    ];
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

/// C5 Task 2：单仓 fixture——project + issue（repo_id 指向已登记真实 git
/// 物理仓）+ 已确认 story/design，无 LC manifest/selection/members。
pub(crate) fn seed_single_repository_fixture() -> Fixture {
    let fixture = seed_fixture(0, true);
    let repo_root = fixture._root.path().join("repo-1");
    std::fs::create_dir_all(&repo_root).unwrap();
    let status = std::process::Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(&repo_root)
        .status()
        .unwrap();
    assert!(
        status.success(),
        "fixture repository must be a real git repo"
    );
    let now = "2026-09-30T00:00:00Z".to_string();
    let repositories = vec![RepositoryRecord {
        id: REPOSITORY_ID.to_string(),
        project_id: PROJECT_ID.to_string(),
        name: "repo-1".to_string(),
        path: repo_root,
        repo_hash: "sha256:fixture-single-repository".to_string(),
        runtime_root: fixture._root.path().join("repo-1/.aria/runtime"),
        default_policy_preset: "manual-write".to_string(),
        default_provider_mode: "fake".to_string(),
        created_at: now.clone(),
        logical_repository_id: None,
        primary_checkout_id: None,
        identity_schema_version: 1,
        updated_at: now,
    }];
    crate::product::json_store::write_json(
        &fixture.paths.project_root(PROJECT_ID).join("repos.json"),
        &repositories,
    )
    .unwrap();
    fixture
}

/// C5 Task 2：单仓 Enable body——target 为服务端单仓投影原样回传形态。
pub(crate) fn single_repository_enable_body(fixture: &Fixture) -> serde_json::Value {
    let mut body = enrollment_body(fixture, 1, 1);
    body["command"]["target"] = serde_json::json!({
        "kind": "single_repository",
        "repository_id": REPOSITORY_ID,
    });
    body
}

/// 批 2 R4/E1：四仓集合 Enable body——target 为 `logical_codebase_members`
///（全部播种成员，升序），`plan_approval=auto` 冻结 AutoIfValid 批准链。
/// story/design 取成员锚定确认对（IR target 解析用）。
pub(crate) fn collection_enable_body(
    story_id: &str,
    design_id: &str,
    logical_codebase_id: &str,
) -> serde_json::Value {
    serde_json::json!({
           "expected_revision": null,
           "command": {
               "type": "enable",
               "selection_key": "r4-e1-four-repository-collection",
               "source": {
                   "stories": [{"id": story_id, "version": 1}],
                   "designs": [{"id": design_id, "version": 1}]
    },
               "options": {
                   // 与单成员 gate fixture 同口径：逻辑 target 的 coding 引擎
                   // 强制 provider gateway，测试态经恒健康 fake gateway 承接
                   // claude_code 方言（行为等同 fake）。
                   "author_provider": "claude_code",
                   "reviewer_provider": "claude_code",
                   "review_rounds": 1,
                   "superpowers_enabled": false,
                   "openspec_enabled": false,
                   "plan_approval": "auto",
                   "plan_options": {
                       "include_integration_tests": false,
                       "include_e2e_tests": false,
                       "force_frontend_backend_split": false,
                       "require_execution_plan_confirm": false
                   }
               },
               "target": {
                   "kind": "logical_codebase_members",
                   "logical_codebase_id": logical_codebase_id,
                   "logical_repository_ids": FOUR_MEMBER_LOGICAL_IDS
               }
           }
       })
}

fn seed_logical_codebase(paths: &ProductAppPaths, members: &[(LogicalRepositoryId, &str)]) {
    // Task 2.8（cwd authority 契约）：真实 LC 拓扑里成员 checkout 位于
    // canonical 聚合根（CommonNonGitParent）之下。本 seed 的 checkout 是
    // paths.root() 的直接子目录，故 manifest.provider_context_root 取
    // paths.root()（真实公共父目录）——cwd 越出 authority 会被 gateway
    // fail-closed。planning resolver 要求 provider_context_root 目录存在
    // （workspace root 本身存在）。
    let manifest = LogicalCodebaseManifest::new(
        PROJECT_ID,
        paths.root().to_path_buf(),
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
            "target": {
                "kind": "logical_codebase",
                "logical_codebase_id": fixture.logical_codebase_id_or_default(),
                "logical_repository_id": SINGLE_LOGICAL_ID
            }
        }
    })
}

/// fixture 播种的 logical codebase id（manifest 随机生成，读取权威值）。
pub(crate) fn fixture_logical_codebase_id(fixture: &Fixture) -> String {
    LogicalCodebaseStore::new(fixture.paths.clone())
        .load_manifest(PROJECT_ID)
        .unwrap()
        .expect("seeded logical codebase manifest")
        .logical_codebase_id
        .to_string()
}

impl Fixture {
    /// 有 manifest 时取权威 logical codebase id；无 manifest 的 0 成员
    /// fixture 返回占位（该场景在 routing 载体判定处即被 422 拒绝，
    /// target 内容不参与判定）。
    fn logical_codebase_id_or_default(&self) -> String {
        LogicalCodebaseStore::new(self.paths.clone())
            .load_manifest(PROJECT_ID)
            .ok()
            .flatten()
            .map(|manifest| manifest.logical_codebase_id.to_string())
            .unwrap_or_else(|| "00000000-0000-0000-0000-0000000000c5".to_string())
    }
}

/// C1 Task 2：显式声明双载体 target 的 enable body（logical 双级齐全）。
pub(crate) fn enrollment_body_with_target(fixture: &Fixture) -> serde_json::Value {
    let mut body = enrollment_body(fixture, 1, 1);
    body["command"]["target"] = serde_json::json!({
        "kind": "logical_codebase",
        "logical_codebase_id": fixture_logical_codebase_id(fixture),
        "logical_repository_id": SINGLE_LOGICAL_ID,
    });
    body
}

/// C1 Task 2：显式 rebind/换代请求 body（新代 plan/session/source/target/provider）。
pub(crate) fn rebind_body(
    fixture: &Fixture,
    command_id: &str,
    expected_policy_revision: u64,
    expected_binding_version: u64,
    plan_id: &str,
    session_id: &str,
) -> serde_json::Value {
    serde_json::json!({
        "command_id": command_id,
        "expected_policy_revision": expected_policy_revision,
        "expected_binding_version": expected_binding_version,
        "binding": {
            "plan_id": plan_id,
            "session_id": session_id,
            "source": {
                "stories": [{"id": fixture.story_id, "version": 1}],
                "designs": [{"id": fixture.design_id, "version": 1}]
            },
            "target": {
                "kind": "logical_codebase",
                "logical_codebase_id": fixture_logical_codebase_id(fixture),
                "logical_repository_id": SINGLE_LOGICAL_ID
            },
            "author_provider": "fake",
            "reviewer_provider": "fake"
        },
        "reason": "recover after failed generation"
    })
}

pub(crate) async fn post_rebind(
    app: &axum::Router,
    body: serde_json::Value,
) -> axum::http::Response<Body> {
    app.clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri(format!(
                    "/api/projects/{PROJECT_ID}/issues/{ISSUE_ID}/automation-enrollment/rebind"
                ))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap()
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
            reviewer_provider: Some(ProviderName::Fake),

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

/// 测试侧「已确认能力」记录形状（与 `automation_gateway_preflight` 测试内
/// `capability_record(provider, true)` 等价）：三个 canonical action 行的
/// launch/resume/write_boundary 全 `Confirmed`，trust `Confirmed`，执行语义
/// 为 host-open，snapshot ref 与 gateway `ProviderRef` 一致。
pub(crate) fn confirmed_capability_record(
    provider_type: crate::product::logical_codebase::provider_gateway::ProviderRefType,
) -> crate::product::logical_codebase::provider_capability_store::ProviderCapabilityRecord {
    use crate::cross_cutting::provider_boundary::PROVIDER_EXECUTION_SEMANTICS_HOST_OPEN;
    use crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence;
    use crate::product::logical_codebase::policy::{
        ProviderDialect, ProviderWireDialect, SessionPolicyAction,
    };
    use crate::product::logical_codebase::provider_capability_store::{
        CapabilityEvidence, PROVIDER_CAPABILITY_RECORD_SCHEMA_VERSION, ProviderActionCapability,
        ProviderActionMatrix, ProviderCapabilityRecord, RootRecipeEvidence,
    };
    use crate::product::logical_codebase::provider_gateway::{
        ProviderRefType, ResumeEvidenceState,
    };

    let (adapter_dialect, wire_dialect) = match provider_type {
        ProviderRefType::ClaudeCode => (
            ProviderDialect::ClaudeCodeCliV1,
            ProviderWireDialect::ClaudeCodeStreamJson,
        ),
        ProviderRefType::Codex => (
            ProviderDialect::CodexCliV1,
            ProviderWireDialect::CodexAppServerRpc,
        ),
        ProviderRefType::Pi => (ProviderDialect::PiRpcV1, ProviderWireDialect::PiRpc),
        ProviderRefType::KimiCode => (ProviderDialect::KimiAcpV1, ProviderWireDialect::KimiAcp),
    };
    let row = |action: SessionPolicyAction| ProviderActionCapability {
        action,
        launch: ProviderCapabilityEvidence::Confirmed,
        resume: ProviderCapabilityEvidence::Confirmed,
        write_boundary: ProviderCapabilityEvidence::Confirmed,
        projection_digest: format!("projection-digest-{action:?}"),
        evidence_ref: format!("probe://{action:?}"),
    };
    ProviderCapabilityRecord {
        provider_type,
        schema_version: PROVIDER_CAPABILITY_RECORD_SCHEMA_VERSION,
        version: "1.4.0".to_string(),
        adapter_dialect,
        wire_dialect,
        capability_snapshot_ref: "cap_managed_snapshot".to_string(),
        evidence: CapabilityEvidence::ProductionVerified,
        resume_evidence: ResumeEvidenceState::Unsupported,
        supported_actions: Vec::new(),
        action_matrix: ProviderActionMatrix::from_rows(vec![
            row(SessionPolicyAction::PlanningReadOnly),
            row(SessionPolicyAction::CodingTargetWrite),
            row(SessionPolicyAction::ReviewReadOnly),
        ])
        .expect("three canonical capability rows"),
        trust: ProviderCapabilityEvidence::Confirmed,
        probed_at: None,
        probe_artifact_ref: None,
        root_recipe_evidence: RootRecipeEvidence::None,
        execution_semantics: PROVIDER_EXECUTION_SEMANTICS_HOST_OPEN.to_string(),
    }
}

/// 测试侧正向 capability 播种：把指定 provider 覆盖写为 `Confirmed` 行。
///
/// 生产语义（`4749a1fc` 起）要求 bootstrap 行保持 Unknown、只有真实 probe
/// 才能写入 Confirmed（Unknown+legacy allow 列表的迁移桥已退役）。正向路径
/// 的 fixture 因此在 `ensure_bootstrap` 之后显式调用本 helper 物化「已探测」
/// 事实；需要断言「能力未确认」的负向场景不得调用，保留 bootstrap Unknown
/// 行即可。`lc_id = None` 写 project 级 legacy 子树，`Some` 写 LC 子树。
pub(crate) fn seed_confirmed_capability_records(
    paths: &ProductAppPaths,
    project_id: &str,
    lc_id: Option<&str>,
    providers: &[crate::product::logical_codebase::provider_gateway::ProviderRefType],
) {
    use crate::product::logical_codebase::provider_capability_store::ProviderCapabilityStore;

    let store = match lc_id {
        Some(lc_id) => ProviderCapabilityStore::for_lc(paths.clone(), lc_id),
        None => ProviderCapabilityStore::new(paths.clone()),
    };
    for provider_type in providers {
        store
            .upsert(project_id, &confirmed_capability_record(*provider_type))
            .expect("seed confirmed capability record");
    }
}

/// 批 2 T3：store 级播种「已终止且绑定指定 plan」的 enrollment（不经
/// HTTP 作用域校验；Enable→bind_plan→terminate_plan）。返回 enrollment_id。
/// 供 runner/gates 干净退出等定向测试复用。
pub(crate) fn seed_terminated_plan_enrollment(
    paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
    plan_id: &str,
    session_id: &str,
) -> String {
    use crate::product::issue_automation_store::IssueAutomationStore;
    use crate::product::models::automation::{
        EnrollmentOptions, EnrollmentSource, EnrollmentTerminationRequest, EnrollmentWriteCommand,
        PlanApprovalPolicy, SourceRevisionRef,
    };
    use crate::product::models::provider::ProviderName;

    let store = IssueAutomationStore::new(paths.clone());
    let enrollment = store
        .compare_and_set(
            project_id,
            issue_id,
            None,
            EnrollmentWriteCommand::Enable {
                command_id: None,
                selection_key: "t3-termination".into(),
                source: EnrollmentSource {
                    stories: vec![SourceRevisionRef {
                        id: "story_0001".into(),
                        version: 1,
                    }],
                    designs: vec![SourceRevisionRef {
                        id: "design_0001".into(),
                        version: 1,
                    }],
                },
                options: EnrollmentOptions {
                    author_provider: ProviderName::Fake,
                    reviewer_provider: ProviderName::Fake,
                    review_rounds: 1,
                    superpowers_enabled: false,
                    openspec_enabled: false,
                    plan_approval: PlanApprovalPolicy::Manual,
                    plan_options: crate::product::models::lifecycle::IssueWorkItemPlanOptions {
                        include_integration_tests: true,
                        include_e2e_tests: false,
                        force_frontend_backend_split: false,
                        require_execution_plan_confirm: false,
                    },
                },
                target: crate::product::logical_codebase::EnrollmentTarget::SingleRepository {
                    repository_id: "repository_0001".to_string(),
                },
            },
        )
        .expect("enable enrollment");
    let bound = store
        .bind_plan(project_id, issue_id, 1, plan_id, session_id)
        .expect("bind plan");
    store
        .terminate_plan(
            project_id,
            issue_id,
            EnrollmentTerminationRequest {
                command_id: format!("cmd-t3-terminate-{plan_id}"),
                expected_enrollment_id: bound.enrollment_id.clone(),
                expected_policy_revision: bound.policy_revision,
                reason: "test termination".to_string(),
            },
        )
        .expect("terminate plan");
    enrollment.enrollment_id
}
