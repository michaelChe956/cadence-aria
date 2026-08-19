//! v1.3 R5：issue 唯一归属代码库（单仓/逻辑），routing/resolver 按 lc_id 子树解析。
//!
//! 这里建立**两个真实的逻辑代码库**（各自独立 `logical-codebases/{lc_id}/` 子树、
//! 真实 Git member checkout、active index、policy），经真实 HTTP 创建逻辑 issue，
//! 断言：lc_id 归属持久化、primary 校验、selection 按 lc_id 键落盘、Story 可达、
//! 以及混合 project 下两 codebase 的 issue 互不串扰。

use std::fs;
use std::path::PathBuf;

use axum::http::{Method, StatusCode};
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::issue_store::IssueStore;
use cadence_aria::product::logical_codebase::{
    CheckoutAvailability, CheckoutKind, CodebaseMemberRecord, IssueCodebaseSelectionStore,
    LogicalCodebaseCreateInput, LogicalCodebaseManifest, LogicalCodebaseStore, LogicalRepositoryId,
    MemberStatus, RepositoryCheckoutId, RepositoryCheckoutRecord, RepositorySourceIdentity,
    RepositoryType,
    aggregate_index::{
        AggregateIndexMemberSnapshot, AggregateIndexRecord, AggregateIndexStatus,
        AggregateIndexStore,
    },
    policy::AggregatePolicyArtifactStore,
};
use cadence_aria::product::project_store::{CreateProjectInput, ProjectStore};
use cadence_aria::web::app::build_web_router;
use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::state::WebAppState;
use serde_json::json;
use tempfile::TempDir;
use uuid::Uuid;

use crate::planning::{git, git_stdout};
use crate::{assert_error, request};

const PROJECT_ID: &str = "project_0001";

struct IssueAttributionFixture {
    _workspace: TempDir,
    paths: ProductAppPaths,
    lc_a: String,
    lc_b: String,
    app: axum::Router,
}

fn seed_logical_codebase(paths: &ProductAppPaths, name: &str, aggregate_root: PathBuf) -> String {
    // 创建真实逻辑代码库记录 + 空子树（POST /logical-codebases 的 store 层语义）。
    let record = LogicalCodebaseStore::new(paths.clone())
        .create(
            PROJECT_ID,
            LogicalCodebaseCreateInput {
                name: name.to_string(),
                aggregate_root: aggregate_root.clone(),
            },
        )
        .expect("create logical codebase record");
    let lc_id = record.id;
    let logical = LogicalCodebaseStore::for_lc(paths.clone(), lc_id.clone());

    // 真实 Git member checkout：resolver 的 freshness 会跑真实 git 证据采集。
    let member_root = aggregate_root.join("svc");
    fs::create_dir_all(&member_root).expect("create member checkout");
    git(&member_root, &["init", "-q"]);
    fs::write(member_root.join("lib.rs"), "pub fn svc() {}\n").expect("write member source");
    git(&member_root, &["add", "lib.rs"]);
    git(
        &member_root,
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "-qm",
            "initial member source",
        ],
    );

    let member_id = LogicalRepositoryId(Uuid::new_v4());
    let checkout_id = RepositoryCheckoutId(Uuid::new_v4());
    let manifest =
        LogicalCodebaseManifest::new(PROJECT_ID, aggregate_root.clone(), vec![member_id]);
    logical
        .save_manifest(PROJECT_ID, &manifest)
        .expect("save lc manifest");
    let now = "2026-08-19T00:00:00Z".to_string();
    logical
        .save_member(
            PROJECT_ID,
            &CodebaseMemberRecord {
                logical_repository_id: member_id,
                physical_repository_id: format!("repository_{name}"),
                alias: name.to_string(),
                role: "service".to_string(),
                ordinal: 1,
                source_identity: RepositorySourceIdentity::from_git_parts(
                    &member_root,
                    member_root.join(".git"),
                    None,
                ),
                repo_type: RepositoryType::Backend,
                tech_stack: vec!["rust".to_string()],
                owner: None,
                tags: Vec::new(),
                default_ref: Some("main".to_string()),
                checkout_ids: vec![checkout_id],
                status: MemberStatus::Active,
                created_at: now.clone(),
                updated_at: now.clone(),
            },
        )
        .expect("save lc member");
    logical
        .save_checkout(
            PROJECT_ID,
            &RepositoryCheckoutRecord {
                checkout_id,
                logical_repository_id: member_id,
                physical_repository_id: format!("repository_{name}"),
                kind: CheckoutKind::Main,
                canonical_path: member_root.clone(),
                checkout_path_hash: format!("sha256:checkout-{name}"),
                git_dir_identity: format!("sha256:git-dir-{name}"),
                revision: Some(git_stdout(&member_root, &["rev-parse", "HEAD"])),
                availability: CheckoutAvailability::Available,
                observed_at: now.clone(),
                created_at: now.clone(),
                updated_at: now.clone(),
            },
        )
        .expect("save lc checkout");

    // active index（fresh：与 manifest 修订号、git HEAD 一致，不触发 CodeGraph 同步）。
    let snapshots = vec![AggregateIndexMemberSnapshot::indexed(
        member_id,
        checkout_id,
        git_stdout(&member_root, &["rev-parse", "HEAD"]),
        false,
        now.clone(),
    )];
    let mut active = AggregateIndexRecord::building(
        format!("index_{name}"),
        PROJECT_ID.to_string(),
        manifest.membership_revision,
        snapshots,
        now,
    );
    active.status = AggregateIndexStatus::Active;
    active.codegraph_root = aggregate_root.clone();
    active.config_digest = "fixture".to_string();
    AggregateIndexStore::for_lc(paths.clone(), lc_id.clone())
        .create(PROJECT_ID, active)
        .expect("publish lc active index");
    AggregatePolicyArtifactStore::for_lc(paths.clone(), lc_id.clone())
        .ensure_bootstrap(&manifest)
        .expect("bootstrap lc aggregate policy");

    lc_id
}

impl IssueAttributionFixture {
    fn new() -> Self {
        let workspace = tempfile::tempdir().expect("workspace");
        let root = workspace.path().to_path_buf();
        let paths = ProductAppPaths::new(root.join(".aria"));
        ProjectStore::new(paths.clone())
            .create(CreateProjectInput {
                name: "codebase kinds".to_string(),
                description: None,
            })
            .expect("create project");
        let lc_a = seed_logical_codebase(&paths, "svc-a", root.join("aggregate-a"));
        let lc_b = seed_logical_codebase(&paths, "svc-b", root.join("aggregate-b"));
        let state = WebAppState::new(root.clone(), WebRuntime::new_fake(root.clone()));
        Self {
            _workspace: workspace,
            paths,
            lc_a,
            lc_b,
            app: build_web_router(state),
        }
    }

    async fn create_issue(
        &self,
        logical_codebase_id: &str,
        repository_id: &str,
    ) -> (StatusCode, serde_json::Value) {
        request(
            &self.app,
            Method::POST,
            &format!("/api/projects/{PROJECT_ID}/issues"),
            json!({
                "repository_id": repository_id,
                "logical_codebase_id": logical_codebase_id,
                "title": "logical codebase issue",
                "description": "attributed to exactly one codebase"
            }),
        )
        .await
    }
}

#[tokio::test]
async fn logical_issue_persists_lc_attribution_selection_and_reaches_story() {
    let fixture = IssueAttributionFixture::new();

    let (status, issue) = fixture
        .create_issue(&fixture.lc_a, "repository_svc-a")
        .await;
    assert_eq!(status, StatusCode::OK, "create logical issue: {issue}");
    let issue_id = issue["issue_id"].as_str().expect("issue id").to_string();

    // 归属持久化到 issue 记录。
    let stored = IssueStore::new(fixture.paths.clone())
        .get(PROJECT_ID, &issue_id)
        .expect("load issue");
    assert_eq!(
        stored.logical_codebase_id.as_deref(),
        Some(fixture.lc_a.as_str())
    );

    // selection 以 lc_id 键落盘在 LC 子树，且记录自述 lc_id 归属。
    let selection =
        IssueCodebaseSelectionStore::for_lc(fixture.paths.clone(), fixture.lc_a.clone())
            .load(PROJECT_ID, &issue_id)
            .expect("load lc selection")
            .expect("selection persisted");
    assert_eq!(
        selection.selection_policy,
        cadence_aria::product::logical_codebase::SelectionPolicy::AllMembers
    );
    assert_eq!(
        selection.logical_codebase_id.as_deref(),
        Some(fixture.lc_a.as_str())
    );
    let lc_selection_path =
        fixture
            .paths
            .lc_codebase_selection_path(PROJECT_ID, &fixture.lc_a, &issue_id);
    assert!(
        lc_selection_path.is_file(),
        "selection keyed by lc_id: {lc_selection_path:?}"
    );

    // Story 可达：routing/resolver 从 lc_id 子树解析 manifest/index/policy。
    let (story_status, story) = request(
        &fixture.app,
        Method::POST,
        &format!("/api/projects/{PROJECT_ID}/issues/{issue_id}/story-specs:generate"),
        json!({
            "title": "logical codebase Story",
            "author_provider": "fake",
            "reviewer_provider": "codex",
            "review_rounds": 1,
            "superpowers_enabled": false,
            "openspec_enabled": false
        }),
    )
    .await;
    assert_eq!(story_status, StatusCode::OK, "Story reachable: {story}");
}

#[tokio::test]
async fn logical_issue_rejects_repository_outside_lc_active_members() {
    let fixture = IssueAttributionFixture::new();

    // repository_svc-b 属于另一逻辑代码库，不能作为 svc-a 的 primary。
    assert_error(
        fixture
            .create_issue(&fixture.lc_a, "repository_svc-b")
            .await,
        StatusCode::NOT_FOUND,
        "repository_not_found",
    );
    // 未知 repository 同样 404。
    assert_error(
        fixture
            .create_issue(&fixture.lc_a, "repository_unknown")
            .await,
        StatusCode::NOT_FOUND,
        "repository_not_found",
    );
    // 未知 lc_id → 404 logical_codebase_not_found。
    assert_error(
        fixture
            .create_issue("logical_codebase_missing", "repository_svc-a")
            .await,
        StatusCode::NOT_FOUND,
        "logical_codebase_not_found",
    );
}

#[tokio::test]
async fn mixed_project_two_codebases_issues_do_not_cross_interfere() {
    let fixture = IssueAttributionFixture::new();

    let (status_a, issue_a) = fixture
        .create_issue(&fixture.lc_a, "repository_svc-a")
        .await;
    assert_eq!(status_a, StatusCode::OK, "create issue in LC A: {issue_a}");
    let issue_a_id = issue_a["issue_id"]
        .as_str()
        .expect("issue A id")
        .to_string();

    let (status_b, issue_b) = fixture
        .create_issue(&fixture.lc_b, "repository_svc-b")
        .await;
    assert_eq!(status_b, StatusCode::OK, "create issue in LC B: {issue_b}");
    let issue_b_id = issue_b["issue_id"]
        .as_str()
        .expect("issue B id")
        .to_string();

    // 各 issue 的 selection 归属各自的 LC 子树，互不重叠。
    let selection_a =
        IssueCodebaseSelectionStore::for_lc(fixture.paths.clone(), fixture.lc_a.clone())
            .load(PROJECT_ID, &issue_a_id)
            .expect("load LC A selection")
            .expect("LC A selection");
    let selection_b =
        IssueCodebaseSelectionStore::for_lc(fixture.paths.clone(), fixture.lc_b.clone())
            .load(PROJECT_ID, &issue_b_id)
            .expect("load LC B selection")
            .expect("LC B selection");
    assert_eq!(
        selection_a.logical_codebase_id.as_deref(),
        Some(fixture.lc_a.as_str())
    );
    assert_eq!(
        selection_b.logical_codebase_id.as_deref(),
        Some(fixture.lc_b.as_str())
    );

    // Story 各自可达，且 inventory 只含本 LC 成员（不串扰）。
    let (_, story_a) = request(
        &fixture.app,
        Method::POST,
        &format!("/api/projects/{PROJECT_ID}/issues/{issue_a_id}/story-specs:generate"),
        json!({
            "title": "LC A Story",
            "author_provider": "fake",
            "reviewer_provider": "codex",
            "review_rounds": 1,
            "superpowers_enabled": false,
            "openspec_enabled": false
        }),
    )
    .await;
    let (_, story_b) = request(
        &fixture.app,
        Method::POST,
        &format!("/api/projects/{PROJECT_ID}/issues/{issue_b_id}/story-specs:generate"),
        json!({
            "title": "LC B Story",
            "author_provider": "fake",
            "reviewer_provider": "codex",
            "review_rounds": 1,
            "superpowers_enabled": false,
            "openspec_enabled": false
        }),
    )
    .await;

    let context_a = story_a["workspace_session"]["messages"]
        .as_array()
        .and_then(|messages| {
            messages.iter().find(|message| {
                message["role"] == "system"
                    && message["content"]
                        .as_str()
                        .is_some_and(|content| content.contains("Workspace 生成任务已准备"))
            })
        })
        .expect("LC A context message");
    let context_b = story_b["workspace_session"]["messages"]
        .as_array()
        .and_then(|messages| {
            messages.iter().find(|message| {
                message["role"] == "system"
                    && message["content"]
                        .as_str()
                        .is_some_and(|content| content.contains("Workspace 生成任务已准备"))
            })
        })
        .expect("LC B context message");

    let content_a = context_a["content"].as_str().expect("LC A inventory");
    let content_b = context_b["content"].as_str().expect("LC B inventory");
    assert!(content_a.contains("svc-a"), "LC A inventory: {content_a}");
    assert!(content_b.contains("svc-b"), "LC B inventory: {content_b}");
    assert!(
        !content_a.contains("svc-b"),
        "cross-LC leak in A: {content_a}"
    );
    assert!(
        !content_b.contains("svc-a"),
        "cross-LC leak in B: {content_b}"
    );
}
