//! 规划入口必须在真实 HTTP 请求中同步 stale 聚合索引。
//!
//! 这里故意不替换 `PlanningContextResolver` 的 freshness 依赖：fixture 建立两个真实
//! Git checkout 与 CodeGraph 索引，Story 生成路由会经过生产 resolver。第一次请求读取
//! fresh index；随后外部 Git 提交造成成员证据漂移，第二次请求必须同步并发布新 active
//! index 后仍成功生成 Story。

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Condvar, Mutex};

use async_trait::async_trait;
use axum::http::{Method, StatusCode};
use cadence_aria::cross_cutting::bounded_command_runner::{
    BoundedCommandError, BoundedCommandRequest, BoundedCommandResult, BoundedCommandRunner,
};
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::issue_store::{CreateProductIssueInput, IssueStore};
use cadence_aria::product::logical_codebase::{
    AggregateInitializationStepKind, CheckoutAvailability, CheckoutKind, CodebaseMemberRecord,
    IssueCodebaseSelection, IssueCodebaseSelectionStore, LogicalCodebaseManifest,
    LogicalCodebaseStore, LogicalRepositoryId, MemberStatus, RepositoryCheckoutId,
    RepositoryCheckoutRecord, RepositorySourceIdentity, RepositoryType,
    aggregate_index::{
        AggregateIndexMemberSnapshot, AggregateIndexOperation, AggregateIndexRecord,
        AggregateIndexStatus, AggregateIndexStore, CodeGraphCli, CodeGraphExcludeGenerator,
    },
    aggregate_initialization_coordinator::{
        AggregateInitializationCoordinator, AggregateInitializationError,
        AggregatePreflightService, AggregatePreflightSnapshot, AggregateProviderTurnDriver,
        AggregateSkillsPreparation, MachineSkillsPreparation,
    },
    aggregate_initialization_store::AggregateInitializationOperationStore,
    policy::AggregatePolicyArtifactStore,
};
use cadence_aria::product::project_store::{CreateProjectInput, ProjectStore};
use cadence_aria::web::app::build_web_router;
use cadence_aria::web::handlers::AggregateInitializationDependencies;
use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::state::{InitializationRunRegistry, WebAppState};
use cadence_aria::web::test_controls::TestControls;
use serde_json::json;
use tempfile::TempDir;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{assert_error, request};

const AGGREGATE_INDEX_REBUILD_PATH: &str =
    "/api/projects/project_0001/logical-codebase/aggregate-indexes/rebuild";
const AGGREGATE_INDEX_ACTIVE_PATH: &str =
    "/api/projects/project_0001/logical-codebase/aggregate-indexes/active";

struct NoopSkills;

#[async_trait]
impl AggregateSkillsPreparation for NoopSkills {
    async fn prepare_skills(
        &self,
        _project_id: &str,
        _operation_id: &str,
        _cancellation: CancellationToken,
    ) -> Result<MachineSkillsPreparation, AggregateInitializationError> {
        Ok(MachineSkillsPreparation {
            source_digest: "sha256:test-source".to_string(),
            link_digest: "sha256:test-links".to_string(),
            skills_root: PathBuf::from("/test-skills"),
            warnings: Vec::new(),
        })
    }
}

struct NoopPreflight;

impl AggregatePreflightService for NoopPreflight {
    fn inspect(
        &self,
        _project_id: &str,
        manifest: &LogicalCodebaseManifest,
        cancellation: &CancellationToken,
    ) -> Result<AggregatePreflightSnapshot, AggregateInitializationError> {
        if cancellation.is_cancelled() {
            return Err(AggregateInitializationError::Cancelled);
        }
        Ok(AggregatePreflightSnapshot {
            aggregate_root: manifest.provider_context_root.display().to_string(),
            index_excludes_assets: true,
            members: Vec::new(),
            manifest_revision: manifest.membership_revision,
            manifest_digest: "sha256:test-manifest".to_string(),
        })
    }
}

struct NoopProvider;

#[async_trait]
impl AggregateProviderTurnDriver for NoopProvider {
    async fn run_turn(
        &self,
        _project_id: &str,
        _operation_id: &str,
        _step: AggregateInitializationStepKind,
        _preflight: &AggregatePreflightSnapshot,
        cancellation: CancellationToken,
    ) -> Result<String, AggregateInitializationError> {
        if cancellation.is_cancelled() {
            return Err(AggregateInitializationError::Cancelled);
        }
        Ok("turn complete".to_string())
    }
}

#[derive(Default)]
struct BlockingIndexCliState {
    started: bool,
    released: bool,
}

#[derive(Clone)]
struct BlockingIndexCli {
    gate: Arc<(Mutex<BlockingIndexCliState>, Condvar)>,
}

impl BlockingIndexCli {
    fn new() -> Self {
        Self {
            gate: Arc::new((Mutex::new(BlockingIndexCliState::default()), Condvar::new())),
        }
    }

    fn release(&self) {
        let (state, wake) = &*self.gate;
        state.lock().expect("blocking CLI gate").released = true;
        wake.notify_all();
    }

    fn wait_until_started(&self) {
        let (state, wake) = &*self.gate;
        let mut state = state.lock().expect("blocking CLI gate");
        while !state.started {
            state = wake.wait(state).expect("blocking CLI gate wait");
        }
    }

    fn success(stdout: impl Into<String>) -> BoundedCommandResult {
        BoundedCommandResult {
            exit_code: Some(0),
            stdout: stdout.into(),
            stderr: String::new(),
            timed_out: false,
            cancelled: false,
            stdout_truncated: false,
            stderr_truncated: false,
            duration_ms: 0,
        }
    }
}

#[async_trait]
impl BoundedCommandRunner for BlockingIndexCli {
    async fn run(
        &self,
        request: BoundedCommandRequest,
    ) -> Result<BoundedCommandResult, BoundedCommandError> {
        let command = request.argv.first().map(String::as_str).unwrap_or_default();
        match command {
            "--version" => Ok(Self::success("1.5.0\n")),
            "init" | "sync" => {
                let (state, wake) = &*self.gate;
                let mut state = state.lock().expect("blocking CLI gate");
                state.started = true;
                wake.notify_all();
                while !state.released {
                    state = wake.wait(state).expect("blocking CLI gate wait");
                }
                Ok(Self::success(""))
            }
            "files" => Ok(Self::success(
                r#"[{"path":"api/lib.rs"},{"path":"web/lib.rs"}]"#,
            )),
            "query" => {
                let query = request.argv.get(1).map(String::as_str).unwrap_or_default();
                if query == "crossRepoGreeting" {
                    Ok(Self::success(
                        r#"[{"path":"api/lib.rs"},{"path":"web/lib.rs"}]"#,
                    ))
                } else {
                    Ok(Self::success("[]"))
                }
            }
            _ => Err(BoundedCommandError::Io {
                details: format!("unexpected fake codegraph command: {command}"),
            }),
        }
    }
}

const PROJECT_ID: &str = "project_0001";
const ISSUE_ID: &str = "issue_0001";

struct PlanningHttpFixture {
    _workspace: TempDir,
    paths: ProductAppPaths,
    api_root: PathBuf,
    api_member_id: LogicalRepositoryId,
    web_member_id: LogicalRepositoryId,
    test_controls: TestControls,
    blocking_index_cli: Option<BlockingIndexCli>,
    app: axum::Router,
}

impl PlanningHttpFixture {
    fn new() -> Self {
        Self::new_with_runner(None, false, true)
    }

    fn with_blocking_index_cli() -> Self {
        let cli = BlockingIndexCli::new();
        Self::new_with_runner(Some(cli), false, true)
    }

    fn with_initialization() -> Self {
        let cli = BlockingIndexCli::new();
        cli.release();
        Self::new_with_runner(Some(cli), true, false)
    }

    fn new_with_runner(
        index_runner: Option<BlockingIndexCli>,
        with_initialization: bool,
        seed_active: bool,
    ) -> Self {
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
        if seed_active {
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
        }
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

        let state = WebAppState::new(root.clone(), WebRuntime::new_fake(root.clone()));
        let test_controls = state.test_controls.clone();
        let blocking_index_cli = index_runner.clone();
        let index = index_runner.map(|runner| {
            Arc::new(AggregateIndexOperation::with_snapshot_dependencies(
                LogicalCodebaseStore::new(paths.clone()),
                AggregateIndexStore::new(paths.clone()),
                CodeGraphCli::new(Arc::new(runner), "fake-codegraph".to_string()),
                CodeGraphExcludeGenerator,
                cadence_aria::product::logical_codebase::aggregate_index::
                    AggregateIndexSnapshotCollector::for_paths(paths.clone()),
            ))
        });
        let state = if let Some(index) = index.clone() {
            // Snapshot Git commands still use the production runner; only the
            // controllable fake CodeGraph CLI is replaced for this HTTP test.
            state.with_aggregate_index_operation(index)
        } else {
            state
        };
        let state = if with_initialization {
            let coordinator = AggregateInitializationCoordinator::new(
                paths.clone(),
                AggregateInitializationOperationStore::new(paths.clone()),
                Arc::new(NoopSkills),
                Arc::new(NoopPreflight),
                Arc::new(NoopProvider),
                Arc::new(|| "2026-08-18T00:00:00Z".to_string()),
            );
            state.with_aggregate_initialization_dependencies(
                AggregateInitializationDependencies::with_index(
                    Arc::new(coordinator),
                    InitializationRunRegistry::default(),
                    index.expect("initialization fixture index operation"),
                ),
            )
        } else {
            state
        };
        Self {
            _workspace: workspace,
            paths,
            api_root,
            api_member_id,
            web_member_id,
            test_controls,
            blocking_index_cli: if with_initialization {
                None
            } else {
                blocking_index_cli
            },
            app: build_web_router(state),
        }
    }

    fn active_index(&self) -> AggregateIndexRecord {
        AggregateIndexStore::new(self.paths.clone())
            .active_required(PROJECT_ID)
            .expect("read active aggregate index")
    }

    fn spawn_rebuild(&self) -> JoinHandle<(StatusCode, serde_json::Value)> {
        let app = self.app.clone();
        tokio::spawn(async move {
            request(&app, Method::POST, AGGREGATE_INDEX_REBUILD_PATH, json!({})).await
        })
    }

    fn release_cli(&self) {
        self.blocking_index_cli
            .as_ref()
            .expect("blocking index CLI")
            .release();
    }

    async fn wait_until_building(&self) {
        for _ in 0..100 {
            let (status, body) = request(
                &self.app,
                Method::GET,
                AGGREGATE_INDEX_ACTIVE_PATH,
                json!({}),
            )
            .await;
            if status == StatusCode::OK && body["state"] == "rebuilding" {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        panic!("aggregate index rebuild did not become visible as rebuilding");
    }

    async fn create_issue_with_primary(
        &self,
        repository_id: &str,
    ) -> (StatusCode, serde_json::Value) {
        request(
            &self.app,
            Method::POST,
            &format!("/api/projects/{PROJECT_ID}/issues"),
            json!({
                "repository_id": repository_id,
                "title": "multi-repo issue",
                "description": "created through multi-repo entrypoint"
            }),
        )
        .await
    }

    fn load_selection(&self, issue_id: &str) -> Option<IssueCodebaseSelection> {
        IssueCodebaseSelectionStore::new(self.paths.clone())
            .load(PROJECT_ID, issue_id)
            .expect("load selection")
    }

    fn issue_count(&self) -> usize {
        IssueStore::new(self.paths.clone())
            .list(PROJECT_ID)
            .expect("list issues")
            .len()
    }

    fn fail_next_selection_save(&self) {
        self.test_controls.fail_next_issue_selection_save();
    }

    fn fail_next_issue_delete(&self) {
        self.test_controls.fail_next_issue_delete();
    }

    fn deactivate_web_member(&self) {
        let logical = LogicalCodebaseStore::new(self.paths.clone());
        let mut member = logical
            .load_member(PROJECT_ID, self.web_member_id)
            .expect("load web member")
            .expect("web member exists");
        member.status = MemberStatus::Removed;
        logical
            .save_member(PROJECT_ID, &member)
            .expect("deactivate web member");
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
    generate_story_for(app, ISSUE_ID).await
}

async fn generate_story_for(app: &axum::Router, issue_id: &str) -> (StatusCode, serde_json::Value) {
    request(
        app,
        Method::POST,
        &format!("/api/projects/{PROJECT_ID}/issues/{issue_id}/story-specs:generate"),
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
async fn multi_repo_issue_creation_writes_all_members_and_compensates_selection_failure() {
    let fixture = PlanningHttpFixture::new();

    let (status, issue) = fixture.create_issue_with_primary("repository_api").await;
    assert_eq!(status, StatusCode::OK, "unexpected response: {issue}");
    let issue_id = issue["issue_id"].as_str().expect("created issue id");
    let selection = fixture
        .load_selection(issue_id)
        .expect("selection persisted");
    assert_eq!(
        selection.selection_policy,
        cadence_aria::product::logical_codebase::SelectionPolicy::AllMembers
    );

    let (story_status, story) = generate_story_for(&fixture.app, issue_id).await;
    assert_eq!(
        story_status,
        StatusCode::OK,
        "Story is not reachable: {story}"
    );

    let issue_count_before_failure = fixture.issue_count();
    fixture.fail_next_selection_save();
    assert_error(
        fixture.create_issue_with_primary("repository_api").await,
        StatusCode::UNPROCESSABLE_ENTITY,
        "issue_selection_write_failed",
    );
    assert_eq!(fixture.issue_count(), issue_count_before_failure);
}

#[tokio::test]
async fn multi_repo_issue_creation_reports_orphan_when_compensation_delete_fails() {
    let fixture = PlanningHttpFixture::new();
    let issue_count_before_failure = fixture.issue_count();
    fixture.fail_next_selection_save();
    fixture.fail_next_issue_delete();

    assert_error(
        fixture.create_issue_with_primary("repository_api").await,
        StatusCode::INTERNAL_SERVER_ERROR,
        "product_store_error",
    );
    assert_eq!(fixture.issue_count(), issue_count_before_failure + 1);
    assert!(fixture.load_selection("issue_0002").is_none());
}

#[tokio::test]
async fn multi_repo_issue_creation_rejects_non_active_member_as_primary() {
    let fixture = PlanningHttpFixture::new();
    fixture.deactivate_web_member();

    assert_error(
        fixture.create_issue_with_primary("repository_web").await,
        StatusCode::NOT_FOUND,
        "repository_not_found",
    );
    assert_eq!(fixture.issue_count(), 1);
}

#[tokio::test]
async fn aggregate_initialization_completion_eventually_exposes_active_index() {
    let fixture = PlanningHttpFixture::with_initialization();
    let (status, accepted) = request(
        &fixture.app,
        Method::POST,
        "/api/projects/project_0001/logical-codebase/initializations",
        json!({"idempotency_key":"index-active-e2e"}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::ACCEPTED,
        "initialization response: {accepted}"
    );
    let operation_id = accepted["operation_id"].as_str().expect("operation id");

    let mut completed = None;
    for _ in 0..100 {
        let (status, body) = request(
            &fixture.app,
            Method::GET,
            &format!("/api/projects/project_0001/logical-codebase/initializations/{operation_id}"),
            json!(null),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "initialization status: {body}");
        if body["status"] == "completed" {
            completed = Some(body);
            break;
        }
        assert_ne!(body["status"], "failed", "initialization failed: {body}");
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    assert!(completed.is_some(), "initialization did not complete");

    for _ in 0..100 {
        let (status, body) = request(
            &fixture.app,
            Method::GET,
            AGGREGATE_INDEX_ACTIVE_PATH,
            json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "active response: {body}");
        if body["state"] == "active" {
            assert!(body["revision"].as_u64().is_some());
            assert!(body["indexed_at"].as_str().is_some());
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    panic!("initialization completed without active aggregate index");
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
async fn aggregate_index_endpoints_expose_building_and_reject_concurrent_rebuilds() {
    let fixture = PlanningHttpFixture::with_blocking_index_cli();
    let first = fixture.spawn_rebuild();
    fixture.wait_until_building().await;
    fixture
        .blocking_index_cli
        .as_ref()
        .expect("blocking index CLI")
        .wait_until_started();

    let (status, body) = request(
        &fixture.app,
        Method::GET,
        AGGREGATE_INDEX_ACTIVE_PATH,
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "active response: {body}");
    assert_eq!(body["state"], "rebuilding");

    assert_error(
        request(
            &fixture.app,
            Method::POST,
            AGGREGATE_INDEX_REBUILD_PATH,
            json!({}),
        )
        .await,
        StatusCode::CONFLICT,
        "aggregate_index_rebuild_in_progress",
    );

    fixture.release_cli();
    let (status, body) = first.await.expect("first rebuild task");
    assert_eq!(status, StatusCode::OK, "rebuild response: {body}");
    assert_eq!(body["state"], "active");
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
