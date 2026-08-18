//! 规划入口必须在真实 HTTP 请求中同步 stale 聚合索引。
//!
//! 这里故意不替换 `PlanningContextResolver` 的 freshness 依赖：fixture 建立两个真实
//! Git checkout 与 CodeGraph 索引，Story 生成路由会经过生产 resolver。第一次请求读取
//! fresh index；随后外部 Git 提交造成成员证据漂移，第二次请求必须同步并发布新 active
//! index 后仍成功生成 Story。

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use axum::http::{Method, StatusCode};
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::issue_store::{CreateProductIssueInput, IssueStore};
use cadence_aria::product::logical_codebase::{
    CheckoutAvailability, CheckoutKind, CodebaseMemberRecord, IssueCodebaseSelection,
    IssueCodebaseSelectionStore, LogicalCodebaseManifest, LogicalCodebaseStore,
    LogicalRepositoryId, MemberStatus, RepositoryCheckoutId, RepositoryCheckoutRecord,
    RepositorySourceIdentity, RepositoryType,
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

use crate::{assert_error, request};

const PROJECT_ID: &str = "project_0001";
const ISSUE_ID: &str = "issue_0001";

struct PlanningHttpFixture {
    _workspace: TempDir,
    paths: ProductAppPaths,
    api_root: PathBuf,
    api_member_id: LogicalRepositoryId,
    app: axum::Router,
}

impl PlanningHttpFixture {
    fn new() -> Self {
        let workspace = tempfile::tempdir().expect("workspace");
        let root = workspace.path().to_path_buf();
        let paths = ProductAppPaths::new(root.join(".aria"));
        ProjectStore::new(paths.clone())
            .create(CreateProjectInput {
                name: "planning stale sync".to_string(),
                description: None,
                multi_repo: true,
            })
            .expect("create multi-repo project");

        let aggregate_root = root.join("aggregate-root");
        let api_root = aggregate_root.join("api");
        let web_root = aggregate_root.join("web");
        for (repository, source) in [
            (
                &api_root,
                "pub fn cross_repo_greeting() -> &'static str { \"api\" }\n",
            ),
            (
                &web_root,
                "pub fn cross_repo_greeting() -> &'static str { \"web\" }\n",
            ),
        ] {
            fs::create_dir_all(repository).expect("create member checkout");
            git(repository, &["init", "-q"]);
            fs::write(repository.join("lib.rs"), source).expect("write member source");
            git(repository, &["add", "lib.rs"]);
            git(
                repository,
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
        }
        codegraph(&aggregate_root, "init");

        let api_member_id = LogicalRepositoryId(Uuid::from_u128(1));
        let web_member_id = LogicalRepositoryId(Uuid::from_u128(2));
        let api_checkout_id = RepositoryCheckoutId(Uuid::from_u128(101));
        let web_checkout_id = RepositoryCheckoutId(Uuid::from_u128(102));
        let manifest = LogicalCodebaseManifest::new(
            PROJECT_ID,
            aggregate_root.clone(),
            vec![api_member_id, web_member_id],
        );
        let logical = LogicalCodebaseStore::new(paths.clone());
        logical
            .save_manifest(PROJECT_ID, &manifest)
            .expect("save manifest");
        let now = "2026-08-18T00:00:00Z".to_string();
        for (member_id, checkout_id, alias, root) in [
            (api_member_id, api_checkout_id, "api", &api_root),
            (web_member_id, web_checkout_id, "web", &web_root),
        ] {
            logical
                .save_member(
                    PROJECT_ID,
                    &CodebaseMemberRecord {
                        logical_repository_id: member_id,
                        physical_repository_id: format!("repository_{alias}"),
                        alias: alias.to_string(),
                        role: "service".to_string(),
                        ordinal: if alias == "api" { 1 } else { 2 },
                        source_identity: RepositorySourceIdentity::from_git_parts(
                            root,
                            root.join(".git"),
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
                .expect("save member");
            logical
                .save_checkout(
                    PROJECT_ID,
                    &RepositoryCheckoutRecord {
                        checkout_id,
                        logical_repository_id: member_id,
                        physical_repository_id: format!("repository_{alias}"),
                        kind: CheckoutKind::Main,
                        canonical_path: root.clone(),
                        checkout_path_hash: format!("sha256:checkout-{alias}"),
                        git_dir_identity: format!("sha256:git-dir-{alias}"),
                        revision: Some(git_stdout(root, &["rev-parse", "HEAD"])),
                        availability: CheckoutAvailability::Available,
                        observed_at: now.clone(),
                        created_at: now.clone(),
                        updated_at: now.clone(),
                    },
                )
                .expect("save checkout");
        }

        let snapshots = [
            (api_member_id, api_checkout_id, &api_root),
            (web_member_id, web_checkout_id, &web_root),
        ]
        .into_iter()
        .map(|(member_id, checkout_id, root)| {
            AggregateIndexMemberSnapshot::indexed(
                member_id,
                checkout_id,
                git_stdout(root, &["rev-parse", "HEAD"]),
                false,
                now.clone(),
            )
        })
        .collect();
        let mut active = AggregateIndexRecord::building(
            "aggregate_index_before_stale".to_string(),
            PROJECT_ID.to_string(),
            manifest.membership_revision,
            snapshots,
            now,
        );
        active.status = AggregateIndexStatus::Active;
        active.codegraph_root = aggregate_root;
        active.config_digest = "fixture".to_string();
        AggregateIndexStore::new(paths.clone())
            .create(PROJECT_ID, active)
            .expect("publish initial active index");
        // Create the issue before its selection: IssueStore derives the next id from the
        // issue root, which also contains selection subdirectories.
        IssueStore::new(paths.clone())
            .create(CreateProductIssueInput {
                project_id: PROJECT_ID.to_string(),
                repo_id: None,
                title: "stale index planning".to_string(),
                description: Some("exercise fresh planning read".to_string()),
                change_id: None,
            })
            .expect("create logical issue");
        IssueCodebaseSelectionStore::new(paths.clone())
            .save(&IssueCodebaseSelection::all_members(
                PROJECT_ID, ISSUE_ID, None,
            ))
            .expect("save issue selection");
        AggregatePolicyArtifactStore::new(paths.clone())
            .ensure_bootstrap(&manifest)
            .expect("bootstrap aggregate policy");

        let state = WebAppState::new(root.clone(), WebRuntime::new_fake(root));
        Self {
            _workspace: workspace,
            paths,
            api_root,
            api_member_id,
            app: build_web_router(state),
        }
    }

    fn active_index(&self) -> AggregateIndexRecord {
        AggregateIndexStore::new(self.paths.clone())
            .active_required(PROJECT_ID)
            .expect("read active aggregate index")
    }

    fn make_api_member_drift(&self) {
        fs::write(
            self.api_root.join("lib.rs"),
            "pub fn cross_repo_greeting() -> &'static str { \"api changed\" }\n",
        )
        .expect("change external member revision");
        git(&self.api_root, &["add", "lib.rs"]);
        git(
            &self.api_root,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "-qm",
                "external member drift",
            ],
        );
    }
}

fn git(root: &Path, arguments: &[&str]) {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(root)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_stdout(root: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(root)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("git utf8 output")
        .trim()
        .to_string()
}

fn codegraph(root: &Path, action: &str) {
    let output = Command::new("codegraph")
        .args([action, "."])
        .current_dir(root)
        .output()
        .expect("run codegraph");
    assert!(
        output.status.success(),
        "codegraph {action} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

async fn generate_story(app: &axum::Router) -> (StatusCode, serde_json::Value) {
    request(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/story-specs:generate",
        json!({
            "title": "fresh planning Story",
            "author_provider": "fake",
            "reviewer_provider": "codex",
            "review_rounds": 1,
            "superpowers_enabled": false,
            "openspec_enabled": false
        }),
    )
    .await
}

#[tokio::test]
async fn aggregate_index_active_endpoint_returns_missing_without_record() {
    let workspace = tempfile::tempdir().expect("workspace");
    let paths = ProductAppPaths::new(workspace.path().join(".aria"));
    ProjectStore::new(paths)
        .create(CreateProjectInput {
            name: "missing aggregate index".to_string(),
            description: None,
            multi_repo: true,
        })
        .expect("create project");
    let state = WebAppState::new(
        workspace.path().to_path_buf(),
        WebRuntime::new_fake(workspace.path().to_path_buf()),
    );
    let app = build_web_router(state);

    let (status, body) = request(
        &app,
        Method::GET,
        "/api/projects/project_0001/logical-codebase/aggregate-indexes/active",
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "active response: {body}");
    assert_eq!(body["state"], "missing");
    assert!(body["revision"].is_null());
    assert!(body["indexed_at"].is_null());
}

#[tokio::test]
async fn aggregate_index_active_endpoint_projects_active_record() {
    let fixture = PlanningHttpFixture::new();

    let (status, body) = request(
        &fixture.app,
        Method::GET,
        "/api/projects/project_0001/logical-codebase/aggregate-indexes/active",
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "active response: {body}");
    assert_eq!(body["state"], "active");
    assert_eq!(body["revision"], fixture.active_index().membership_revision);
    assert!(body["indexed_at"].as_str().is_some());
    assert!(body["warning"].is_null());
}

#[tokio::test]
async fn aggregate_index_active_endpoint_projects_building_as_rebuilding() {
    let fixture = PlanningHttpFixture::new();
    let mut building = fixture.active_index();
    building.aggregate_index_id = "aggregate_index_building".to_string();
    building.status = AggregateIndexStatus::Building;
    building.updated_at = "9999-01-01T00:00:00Z".to_string();
    AggregateIndexStore::new(fixture.paths.clone())
        .create(PROJECT_ID, building)
        .expect("persist building index record");

    let (status, body) = request(
        &fixture.app,
        Method::GET,
        "/api/projects/project_0001/logical-codebase/aggregate-indexes/active",
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "active response: {body}");
    assert_eq!(body["state"], "rebuilding");
}

#[tokio::test]
async fn aggregate_index_rebuild_endpoint_returns_conflict_while_project_is_registered() {
    let workspace = tempfile::tempdir().expect("workspace");
    let paths = ProductAppPaths::new(workspace.path().join(".aria"));
    ProjectStore::new(paths)
        .create(CreateProjectInput {
            name: "registered aggregate index rebuild".to_string(),
            description: None,
            multi_repo: true,
        })
        .expect("create project");
    let state = WebAppState::new(
        workspace.path().to_path_buf(),
        WebRuntime::new_fake(workspace.path().to_path_buf()),
    );
    let lease = state
        .aggregate_index_rebuilds
        .try_register(PROJECT_ID)
        .expect("register rebuild");
    let app = build_web_router(state);

    assert_error(
        request(
            &app,
            Method::POST,
            "/api/projects/project_0001/logical-codebase/aggregate-indexes/rebuild",
            json!({}),
        )
        .await,
        StatusCode::CONFLICT,
        "aggregate_index_rebuild_in_progress",
    );
    drop(lease);
}

#[tokio::test]
async fn aggregate_index_rebuild_endpoint_returns_active_projection() {
    let fixture = PlanningHttpFixture::new();

    let (status, body) = request(
        &fixture.app,
        Method::POST,
        "/api/projects/project_0001/logical-codebase/aggregate-indexes/rebuild",
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "rebuild response: {body}");
    assert_eq!(body["state"], "active");
    assert!(body["revision"].as_u64().is_some());
    assert!(body["indexed_at"].as_str().is_some());
}

#[tokio::test]
async fn aggregate_index_failed_latest_record_projects_last_known_good_as_degraded() {
    let fixture = PlanningHttpFixture::new();
    let mut failed = fixture.active_index();
    failed.aggregate_index_id = "aggregate_index_failed_latest".to_string();
    failed.status = AggregateIndexStatus::Failed;
    failed.warning = Some("initial build failed".to_string());
    failed.updated_at = "9999-01-01T00:00:00Z".to_string();
    AggregateIndexStore::new(fixture.paths.clone())
        .create(PROJECT_ID, failed)
        .expect("persist failed index record");

    let (status, body) = request(
        &fixture.app,
        Method::GET,
        "/api/projects/project_0001/logical-codebase/aggregate-indexes/active",
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "active response: {body}");
    assert_eq!(body["state"], "degraded");
    assert_eq!(body["revision"], fixture.active_index().membership_revision);
    assert_eq!(body["warning"], "initial build failed");
}

#[tokio::test]
async fn stale_story_planning_read_syncs_index_and_returns_normal_response() {
    let fixture = PlanningHttpFixture::new();

    let (status, first) = generate_story(&fixture.app).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "initial fresh planning response: {first}"
    );
    let stale_index_id = fixture.active_index().aggregate_index_id;

    // A real external checkout commit makes the published member snapshot stale.
    fixture.make_api_member_drift();

    let (status, second) = generate_story(&fixture.app).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "planning response after stale index synchronization: {second}"
    );
    assert!(
        second["story_specs"]
            .as_array()
            .is_some_and(|stories| !stories.is_empty()),
        "successful Story response must contain a generated Story: {second}"
    );

    let refreshed = fixture.active_index();
    assert_eq!(refreshed.status, AggregateIndexStatus::Active);
    assert_ne!(refreshed.aggregate_index_id, stale_index_id);
    let api_snapshot = refreshed
        .member_snapshots
        .iter()
        .find(|snapshot| snapshot.logical_repository_id == fixture.api_member_id)
        .expect("refreshed API member snapshot");
    assert_eq!(
        api_snapshot.revision,
        git_stdout(&fixture.api_root, &["rev-parse", "HEAD"]),
        "synchronized index must contain external member's new revision"
    );
}
