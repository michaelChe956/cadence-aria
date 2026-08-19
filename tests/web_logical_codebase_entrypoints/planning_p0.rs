//! P0 生产入口 fixture 与 HTTP 全链测试（REG→INIT→IDX→PLN）。
//!
//! 这里的 fixture 与 `planning` 模块共享真实 Git/CodeGraph helper；业务操作全部
//! 通过 web router 发出，不直接触碰产品 store。

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use axum::http::{Method, StatusCode};
use cadence_aria::cross_cutting::aria_state_paths::AriaStatePaths;
use cadence_aria::cross_cutting::bounded_command_runner::{
    BoundedCommandError, BoundedCommandRequest, BoundedCommandResult, BoundedCommandRunner,
    TokioBoundedCommandRunner,
};
use cadence_aria::cross_cutting::provider_availability_gate::ProviderAvailabilityGate;
use cadence_aria::cross_cutting::provider_health::{
    ProviderHealthService, SystemProviderHealthClock,
};
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::logical_codebase::{LogicalCodebaseManifest, LogicalCodebaseStore};
use cadence_aria::web::app::build_web_router;
use cadence_aria::web::gateway_factory::LogicalCodebaseGatewayFactory;
use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::state::WebAppState;
use serde_json::json;
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

use crate::planning::{codegraph, git, git_stdout};
use crate::{assert_error, request};

/// Returns successful provider-health probes while delegating all production
/// aggregate-index and Git commands to the normal bounded runner.
struct P0CommandRunner {
    inner: TokioBoundedCommandRunner,
}

#[async_trait]
impl BoundedCommandRunner for P0CommandRunner {
    async fn run(
        &self,
        request: BoundedCommandRequest,
    ) -> Result<BoundedCommandResult, BoundedCommandError> {
        if request
            .argv
            .first()
            .is_some_and(|argument| argument == "--version")
            && matches!(request.executable.as_str(), "claude" | "codex")
        {
            return Ok(BoundedCommandResult {
                exit_code: Some(0),
                stdout: "1.0.0\n".to_string(),
                stderr: String::new(),
                timed_out: false,
                cancelled: false,
                stdout_truncated: false,
                stderr_truncated: false,
                duration_ms: 0,
            });
        }
        if request.executable == "codegraph" {
            let stdout = match request.argv.first().map(String::as_str).unwrap_or_default() {
                "--version" => "1.5.0\n",
                "init" | "sync" => "",
                "files" => r#"[{"path":"api/lib.rs"},{"path":"web/lib.rs"}]"#,
                "query"
                    if request
                        .argv
                        .get(1)
                        .is_some_and(|query| query == "crossRepoGreeting") =>
                {
                    r#"[{"path":"api/lib.rs"},{"path":"web/lib.rs"}]"#
                }
                "query" => "[]",
                command => {
                    return Err(BoundedCommandError::Io {
                        details: format!("unexpected P0 fake codegraph command: {command}"),
                    });
                }
            };
            return Ok(BoundedCommandResult {
                exit_code: Some(0),
                stdout: stdout.to_string(),
                stderr: String::new(),
                timed_out: false,
                cancelled: false,
                stdout_truncated: false,
                stderr_truncated: false,
                duration_ms: 0,
            });
        }
        self.inner.run(request).await
    }
}

/// P0 production-entrypoint fixture: after the real filesystem/Git setup every
/// business operation goes through the web router. It deliberately does not
/// seed registration, initialization, index, issue, selection, or lifecycle
/// records through their product stores.
struct P0HttpFixture {
    _workspace: TempDir,
    root: PathBuf,
    aggregate_root: PathBuf,
    member_roots: Vec<PathBuf>,
    app: axum::Router,
}

impl P0HttpFixture {
    async fn real_git_aggregate() -> Self {
        let workspace = tempfile::tempdir().expect("P0 workspace");
        let root = workspace.path().to_path_buf();
        let aggregate_root = root.join("aggregate-root");
        let api_root = aggregate_root.join("api");
        let web_root = aggregate_root.join("web");
        for (member_root, source) in [
            (
                &api_root,
                "pub fn aggregate_api() -> &'static str { \"api\" }\n",
            ),
            (
                &web_root,
                "pub fn aggregate_web() -> &'static str { \"web\" }\n",
            ),
        ] {
            fs::create_dir_all(member_root).expect("create P0 member checkout");
            git(member_root, &["init", "-q"]);
            fs::write(member_root.join("lib.rs"), source).expect("write P0 member source");
            git(member_root, &["add", "lib.rs"]);
            git(
                member_root,
                &[
                    "-c",
                    "user.name=P0 Test",
                    "-c",
                    "user.email=p0@example.test",
                    "commit",
                    "-qm",
                    "P0 member baseline",
                ],
            );
        }
        codegraph(&aggregate_root, "init");

        // Seed the production skill source as an offline local checkout. The
        // production WebAppState then supplies its normal aggregate
        // coordinator/index/gateway wiring; its fake runtime routes provider
        // turns through the existing fake registry without external calls.
        fs::create_dir_all(root.join(".agents/Cadence-skills/cadence-init/skills/p0-http-chain"))
            .expect("seed local Cadence skills source");
        let command_runner: Arc<dyn BoundedCommandRunner> = Arc::new(P0CommandRunner {
            inner: TokioBoundedCommandRunner,
        });
        let provider_health = Arc::new(ProviderHealthService::with_dependencies(
            AriaStatePaths::from_workspace_root(&root),
            command_runner.clone(),
            Arc::new(SystemProviderHealthClock),
            std::time::Duration::from_secs(1),
            4096,
        ));
        provider_health
            .refresh(CancellationToken::new())
            .await
            .expect("seed healthy fake provider probes");
        let provider_gate = Arc::new(ProviderAvailabilityGate::new(provider_health.clone()));
        let state = WebAppState::new(root.clone(), WebRuntime::new_fake(root.clone()))
            .with_provider_health(provider_health, provider_gate.clone(), command_runner);
        let gateway_factory = Arc::new(LogicalCodebaseGatewayFactory::new(
            ProductAppPaths::new(root.join(".aria")),
            state.provider_registry.clone(),
            state.provider_adapter.clone(),
            provider_gate,
        ));
        let app = build_web_router(state.with_gateway_factory(gateway_factory));
        Self {
            _workspace: workspace,
            root,
            aggregate_root,
            member_roots: vec![api_root, web_root],
            app,
        }
    }

    async fn create_multi_repo_project(&self) -> String {
        let (status, project) = request(
            &self.app,
            Method::POST,
            "/api/projects",
            json!({
                "name": "P0 multi-repository HTTP chain",
                "description": "real HTTP registration through planning"
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "create project: {project}");
        let project_id = project["project_id"]
            .as_str()
            .expect("project id")
            .to_string();
        // R1 过渡：旧 logical-codebase 端点是“默认第一个逻辑代码库”的兼容别名，
        // 因此 fixture 直接落地 manifest 以模拟已存在的逻辑代码库。
        LogicalCodebaseStore::new(ProductAppPaths::new(self.root.join(".aria")))
            .save_manifest(
                &project_id,
                &LogicalCodebaseManifest::new(&project_id, self.aggregate_root.clone(), Vec::new()),
            )
            .expect("save logical codebase manifest");
        project_id
    }

    async fn preflight_and_submit_all(&self, project_id: &str) -> serde_json::Value {
        let (status, preflight) = request(
            &self.app,
            Method::POST,
            &format!("/api/projects/{project_id}/logical-codebase/registrations/preflight"),
            json!({
                "aggregate_root": self.aggregate_root,
                "candidate_paths": self.member_roots,
            }),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "registration preflight: {preflight}"
        );
        assert!(
            preflight["items"]
                .as_array()
                .is_some_and(|items| items.iter().all(|item| item["class"] == "eligible")),
            "real Git members must be eligible: {preflight}"
        );

        let (status, batch) = request(
            &self.app,
            Method::POST,
            &format!("/api/projects/{project_id}/logical-codebase/registrations"),
            json!({
                "aggregate_root": self.aggregate_root,
                "preflight_id": preflight["preflight_id"],
                "confirmed_paths": self.member_roots,
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "registration submit: {batch}");
        batch
    }

    async fn start_initialization(
        &self,
        project_id: &str,
        idempotency_key: &str,
    ) -> serde_json::Value {
        let (status, operation) = request(
            &self.app,
            Method::POST,
            &format!("/api/projects/{project_id}/logical-codebase/initializations"),
            json!({"idempotency_key": idempotency_key}),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::ACCEPTED,
            "initialization start: {operation}"
        );
        operation
    }

    async fn poll_until_terminal(&self, project_id: &str, operation_id: &str) -> serde_json::Value {
        for _ in 0..200 {
            let (status, operation) = request(
                &self.app,
                Method::GET,
                &format!(
                    "/api/projects/{project_id}/logical-codebase/initializations/{operation_id}"
                ),
                json!(null),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "initialization poll: {operation}");
            if matches!(
                operation["status"].as_str(),
                Some("completed" | "failed" | "cancelled")
            ) {
                return operation;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("initialization {operation_id} did not reach a terminal state");
    }

    async fn wait_for_active_index(&self, project_id: &str) -> serde_json::Value {
        let mut last = serde_json::Value::Null;
        for _ in 0..200 {
            let (status, index) = request(
                &self.app,
                Method::GET,
                &format!("/api/projects/{project_id}/logical-codebase/aggregate-indexes/active"),
                json!({}),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "aggregate index poll: {index}");
            if index["state"] == "active" {
                return index;
            }
            last = index;
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("aggregate index did not become active: {last}");
    }

    async fn primary_repository_id(&self, project_id: &str) -> String {
        let (status, repositories) = request(
            &self.app,
            Method::GET,
            &format!("/api/projects/{project_id}/repositories"),
            json!({}),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "list logical repositories: {repositories}"
        );
        repositories["repositories"]
            .as_array()
            .and_then(|repositories| repositories.first())
            .and_then(|repository| repository["repository_id"].as_str())
            .expect("registered primary repository id")
            .to_string()
    }

    async fn logical_codebase_id(&self, project_id: &str) -> String {
        let (status, codebases) = request(
            &self.app,
            Method::GET,
            &format!("/api/projects/{project_id}/codebases"),
            json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "list codebases: {codebases}");
        codebases["codebases"]
            .as_array()
            .and_then(|codebases| {
                codebases
                    .iter()
                    .find(|codebase| codebase["kind"] == "logical")
            })
            .and_then(|codebase| codebase["logical_codebase_id"].as_str())
            .expect("registered logical codebase id")
            .to_string()
    }

    async fn create_issue_with_primary(&self, project_id: &str) -> serde_json::Value {
        let repository_id = self.primary_repository_id(project_id).await;
        let logical_codebase_id = self.logical_codebase_id(project_id).await;
        let (status, issue) = request(
            &self.app,
            Method::POST,
            &format!("/api/projects/{project_id}/issues"),
            json!({
                "repository_id": repository_id,
                "logical_codebase_id": logical_codebase_id,
                "title": "P0 aggregate planning issue",
                "description": "selection must be written by the HTTP issue endpoint"
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "create multi-repo issue: {issue}");
        issue
    }

    fn selection_is_persisted(&self, project_id: &str, issue_id: &str) -> bool {
        self.root
            .join(".aria/projects")
            .join(project_id)
            .join("issues")
            .join(issue_id)
            .join("codebase-selection.json")
            .is_file()
    }

    fn logical_member_ids(&self, project_id: &str) -> Vec<String> {
        let members_root = self
            .root
            .join(".aria/projects")
            .join(project_id)
            .join("logical-codebase/members");
        let mut ids = fs::read_dir(members_root)
            .expect("registered member records")
            .map(|entry| entry.expect("member entry").path())
            .map(|path| {
                let value: serde_json::Value =
                    serde_json::from_slice(&fs::read(&path).expect("read member record"))
                        .expect("decode member record");
                value["logical_repository_id"]
                    .as_str()
                    .expect("logical repository id")
                    .to_string()
            })
            .collect::<Vec<_>>();
        ids.sort();
        assert_eq!(ids.len(), 2, "registered logical members");
        ids
    }

    async fn generate_story(
        &self,
        project_id: &str,
        issue_id: &str,
        involved_repository_ids: &[String],
    ) -> serde_json::Value {
        let (status, story) = request(
            &self.app,
            Method::POST,
            &format!("/api/projects/{project_id}/issues/{issue_id}/story-specs:generate"),
            json!({
                "title": "P0 aggregate Story",
                "involved_repository_ids": involved_repository_ids,
                "author_provider": "fake",
                "reviewer_provider": "codex",
                "review_rounds": 1,
                "superpowers_enabled": false,
                "openspec_enabled": false
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "generate aggregate Story: {story}");
        story
    }

    async fn generate_design(
        &self,
        project_id: &str,
        issue_id: &str,
        story_spec_id: &str,
        involved_repository_ids: &[String],
        change_order: &[String],
    ) -> serde_json::Value {
        let (status, design) = request(
            &self.app,
            Method::POST,
            &format!("/api/projects/{project_id}/issues/{issue_id}/design-specs:generate"),
            json!({
                "title": "P0 aggregate Design",
                "story_spec_ids": [story_spec_id],
                "involved_repository_ids": involved_repository_ids,
                "change_order": change_order,
                "author_provider": "fake",
                "reviewer_provider": "codex",
                "review_rounds": 1,
                "superpowers_enabled": false,
                "openspec_enabled": false
            }),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "generate aggregate Design: {design}"
        );
        design
    }

    async fn confirm_workspace(&self, session_id: &str) -> (StatusCode, serde_json::Value) {
        request(
            &self.app,
            Method::POST,
            &format!("/api/workspace-sessions/{session_id}/confirm"),
            json!({"confirmed_by": "P0 HTTP test"}),
        )
        .await
    }

    async fn prepare_work_item_plan(
        &self,
        project_id: &str,
        issue_id: &str,
        story_spec_id: &str,
        design_spec_id: &str,
    ) -> (StatusCode, serde_json::Value) {
        request(
            &self.app,
            Method::POST,
            &format!("/api/projects/{project_id}/issues/{issue_id}/work-item-plans:prepare"),
            json!({
                "title": "P0 target-split Work Item Plan",
                "story_spec_ids": [story_spec_id],
                "design_spec_ids": [design_spec_id],
                "author_provider": "fake",
                "reviewer_provider": "codex",
                "review_rounds": 1,
                "superpowers_enabled": false,
                "openspec_enabled": false
            }),
        )
        .await
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct P0MemberGitSnapshot {
    head: String,
    status_porcelain: String,
    tracked_files: String,
    refs: String,
}

impl P0MemberGitSnapshot {
    fn capture(root: &Path) -> Self {
        Self {
            head: git_stdout(root, &["rev-parse", "HEAD"]),
            status_porcelain: git_stdout(root, &["status", "--porcelain"]),
            tracked_files: git_stdout(root, &["ls-files"]),
            refs: git_stdout(root, &["for-each-ref", "--format=%(refname) %(objectname)"]),
        }
    }
}

async fn p0_registered_initialized_issue() -> (P0HttpFixture, String, String, Vec<String>) {
    let fixture = P0HttpFixture::real_git_aggregate().await;
    let project_id = fixture.create_multi_repo_project().await;
    let batch = fixture.preflight_and_submit_all(&project_id).await;
    assert_eq!(batch["status"], "completed", "registration batch: {batch}");

    let operation = fixture
        .start_initialization(&project_id, "p0-init-key")
        .await;
    let operation_id = operation["operation_id"].as_str().expect("operation id");
    let completed = fixture.poll_until_terminal(&project_id, operation_id).await;
    assert_eq!(
        completed["status"], "completed",
        "initialization: {completed}"
    );
    assert_eq!(
        fixture.wait_for_active_index(&project_id).await["state"],
        "active"
    );

    let issue = fixture.create_issue_with_primary(&project_id).await;
    let issue_id = issue["issue_id"].as_str().expect("issue id").to_string();
    assert!(
        fixture.selection_is_persisted(&project_id, &issue_id),
        "HTTP issue creation must persist codebase selection"
    );
    let members = fixture.logical_member_ids(&project_id);
    (fixture, project_id, issue_id, members)
}
#[tokio::test]
async fn reg_init_idx_pln_p0_chain_uses_only_http_routes() {
    let (fixture, project_id, issue_id, members) = p0_registered_initialized_issue().await;
    let member_git_before = fixture
        .member_roots
        .iter()
        .map(|root| P0MemberGitSnapshot::capture(root))
        .collect::<Vec<_>>();

    let story = fixture
        .generate_story(&project_id, &issue_id, &members)
        .await;
    let story_id = story["story_specs"][0]["story_spec_id"]
        .as_str()
        .expect("generated Story id");
    let story_session = story["workspace_session"]["workspace_session_id"]
        .as_str()
        .expect("Story session id");
    let (status, confirmed_story) = fixture.confirm_workspace(story_session).await;
    assert_eq!(status, StatusCode::OK, "confirm Story: {confirmed_story}");

    let design = fixture
        .generate_design(&project_id, &issue_id, story_id, &members, &members)
        .await;
    let design_id = design["design_specs"][0]["design_spec_id"]
        .as_str()
        .expect("generated Design id");
    let design_session = design["workspace_session"]["workspace_session_id"]
        .as_str()
        .expect("Design session id");
    let (status, confirmed_design) = fixture.confirm_workspace(design_session).await;
    assert_eq!(status, StatusCode::OK, "confirm Design: {confirmed_design}");

    let (status, plan) = fixture
        .prepare_work_item_plan(&project_id, &issue_id, story_id, design_id)
        .await;
    assert_eq!(status, StatusCode::OK, "prepare target-split plan: {plan}");
    assert_eq!(
        plan["work_item_plan"]["source_design_spec_ids"],
        json!([design_id])
    );
    assert!(
        plan["work_item_plan"]["work_item_ids"]
            .as_array()
            .is_some_and(Vec::is_empty),
        "the HTTP :prepare contract intentionally stops at a Draft; per-target splitting is a later workspace compile concern: {plan}"
    );
    assert_eq!(
        member_git_before,
        fixture
            .member_roots
            .iter()
            .map(|root| P0MemberGitSnapshot::capture(root))
            .collect::<Vec<_>>(),
        "REG→INIT→IDX→PLN must not write member main checkout Git state"
    );
}

#[tokio::test]
async fn multi_repo_design_missing_change_order_is_blocked_over_http() {
    let (fixture, project_id, issue_id, members) = p0_registered_initialized_issue().await;
    let story = fixture
        .generate_story(&project_id, &issue_id, &members)
        .await;
    let story_id = story["story_specs"][0]["story_spec_id"]
        .as_str()
        .expect("generated Story id");
    let story_session = story["workspace_session"]["workspace_session_id"]
        .as_str()
        .expect("Story session id");
    let (status, confirmed_story) = fixture.confirm_workspace(story_session).await;
    assert_eq!(status, StatusCode::OK, "confirm Story: {confirmed_story}");

    assert_error(
        request(
            &fixture.app,
            Method::POST,
            &format!("/api/projects/{project_id}/issues/{issue_id}/design-specs:generate"),
            json!({
                "title": "P0 duplicate change order",
                "story_spec_ids": [story_id],
                "involved_repository_ids": members.clone(),
                "change_order": [members[0].clone(), members[0].clone()],
                "author_provider": "fake",
                "reviewer_provider": "codex",
                "review_rounds": 1,
                "superpowers_enabled": false,
                "openspec_enabled": false
            }),
        )
        .await,
        StatusCode::UNPROCESSABLE_ENTITY,
        "change_order_duplicate_repository",
    );

    let missing_order = fixture
        .generate_design(&project_id, &issue_id, story_id, &members, &[])
        .await;
    let missing_session = missing_order["workspace_session"]["workspace_session_id"]
        .as_str()
        .expect("missing-order Design session id");
    assert_error(
        fixture.confirm_workspace(missing_session).await,
        StatusCode::BAD_REQUEST,
        "change_order_required_for_logical_codebase",
    );
    let missing_design_id = missing_order["design_specs"][0]["design_spec_id"]
        .as_str()
        .expect("missing-order Design id");
    assert_error(
        fixture
            .prepare_work_item_plan(&project_id, &issue_id, story_id, missing_design_id)
            .await,
        StatusCode::BAD_REQUEST,
        "design_spec_not_confirmed",
    );

    let with_order = fixture
        .generate_design(&project_id, &issue_id, story_id, &members, &members)
        .await;
    let with_order_session = with_order["workspace_session"]["workspace_session_id"]
        .as_str()
        .expect("ordered Design session id");
    let (status, confirmed) = fixture.confirm_workspace(with_order_session).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "ordered Design must pass: {confirmed}"
    );
}
