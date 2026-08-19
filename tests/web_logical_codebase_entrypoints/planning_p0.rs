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

    /// v1.3 新寻址（R9）：POST /logical-codebases 创建 LC，不再落地 legacy manifest。
    async fn create_project_and_logical_codebase(&self, name: &str) -> (String, String) {
        let (status, project) = request(
            &self.app,
            Method::POST,
            "/api/projects",
            json!({
                "name": name,
                "description": "real HTTP registration through planning"
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "create project: {project}");
        let project_id = project["project_id"]
            .as_str()
            .expect("project id")
            .to_string();
        let (status, codebase) = request(
            &self.app,
            Method::POST,
            &format!("/api/projects/{project_id}/logical-codebases"),
            json!({
                "name": "P0 aggregate",
                "aggregate_root": self.aggregate_root,
            }),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "create logical codebase: {codebase}"
        );
        let lc_id = codebase["logical_codebase_id"]
            .as_str()
            .or_else(|| codebase["id"].as_str())
            .expect("logical codebase id")
            .to_string();
        (project_id, lc_id)
    }

    /// v1.3 新寻址：preflight auto_discover 自动发现聚合根下直连 Git 成员，再确认提交。
    async fn preflight_and_submit_all(&self, project_id: &str, lc_id: &str) -> serde_json::Value {
        let (status, preflight) = request(
            &self.app,
            Method::POST,
            &format!(
                "/api/projects/{project_id}/logical-codebases/{lc_id}/registrations/preflight"
            ),
            json!({
                "aggregate_root": self.aggregate_root,
                "candidate_paths": self.member_roots,
                "auto_discover": true,
            }),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "registration preflight: {preflight}"
        );
        let items = preflight["items"]
            .as_array()
            .unwrap_or_else(|| panic!("preflight items: {preflight}"));
        assert!(
            items.iter().all(|item| item["class"] == "eligible"),
            "real Git members must be eligible: {preflight}"
        );
        assert_eq!(
            items.len(),
            self.member_roots.len(),
            "auto-discovered members: {preflight}"
        );
        let confirmed_paths: Vec<String> = items
            .iter()
            .map(|item| item["path"].as_str().expect("member path").to_string())
            .collect();

        let (status, batch) = request(
            &self.app,
            Method::POST,
            &format!("/api/projects/{project_id}/logical-codebases/{lc_id}/registrations"),
            json!({
                "aggregate_root": self.aggregate_root,
                "preflight_id": preflight["preflight_id"],
                "confirmed_paths": confirmed_paths,
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "registration submit: {batch}");
        batch
    }

    async fn start_initialization(
        &self,
        project_id: &str,
        lc_id: &str,
        idempotency_key: &str,
    ) -> serde_json::Value {
        let (status, operation) = request(
            &self.app,
            Method::POST,
            &format!("/api/projects/{project_id}/logical-codebases/{lc_id}/initializations"),
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

    async fn poll_until_terminal(
        &self,
        project_id: &str,
        lc_id: &str,
        operation_id: &str,
    ) -> serde_json::Value {
        for _ in 0..200 {
            let (status, operation) = request(
                &self.app,
                Method::GET,
                &format!(
                    "/api/projects/{project_id}/logical-codebases/{lc_id}/initializations/{operation_id}"
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

    async fn wait_for_active_index(&self, project_id: &str, lc_id: &str) -> serde_json::Value {
        let mut last = serde_json::Value::Null;
        for _ in 0..200 {
            let (status, index) = request(
                &self.app,
                Method::GET,
                &format!(
                    "/api/projects/{project_id}/logical-codebases/{lc_id}/aggregate-indexes/active"
                ),
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

    async fn primary_repository_id(&self, project_id: &str, lc_id: &str) -> String {
        // R6：GET /repositories 不再投影逻辑成员，改从成员权威记录直接读取
        // 首个 active 成员的 physical_repository_id（primary 校验接受的任意
        // active 成员）。v1.3（R9）：成员权威记录在 logical-codebases/{lc_id}/ 子树。
        let members_root = self.lc_members_root(project_id, lc_id);
        let mut physical_ids = fs::read_dir(&members_root)
            .expect("registered member records")
            .map(|entry| entry.expect("member entry").path())
            .map(|path| {
                let value: serde_json::Value =
                    serde_json::from_slice(&fs::read(&path).expect("read member record"))
                        .expect("decode member record");
                value
            })
            .filter(|member| member["status"] == "active")
            .filter_map(|member| {
                member["physical_repository_id"]
                    .as_str()
                    .map(str::to_string)
            })
            .collect::<Vec<_>>();
        physical_ids.sort();
        assert!(!physical_ids.is_empty(), "registered physical ids");
        physical_ids.remove(0)
    }

    async fn create_issue_with_primary(
        &self,
        project_id: &str,
        lc_id: &str,
        title: &str,
    ) -> serde_json::Value {
        let repository_id = self.primary_repository_id(project_id, lc_id).await;
        let (status, issue) = request(
            &self.app,
            Method::POST,
            &format!("/api/projects/{project_id}/issues"),
            json!({
                "repository_id": repository_id,
                "logical_codebase_id": lc_id,
                "title": title,
                "description": "selection must be written by the HTTP issue endpoint"
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "create multi-repo issue: {issue}");
        issue
    }

    fn lc_members_root(&self, project_id: &str, lc_id: &str) -> PathBuf {
        self.root
            .join(".aria/projects")
            .join(project_id)
            .join("logical-codebases")
            .join(lc_id)
            .join("members")
    }

    fn selection_is_persisted(&self, project_id: &str, lc_id: &str, issue_id: &str) -> bool {
        self.root
            .join(".aria/projects")
            .join(project_id)
            .join("logical-codebases")
            .join(lc_id)
            .join("selections")
            .join(issue_id)
            .join("codebase-selection.json")
            .is_file()
    }

    fn logical_member_ids(&self, project_id: &str, lc_id: &str) -> Vec<String> {
        let members_root = self.lc_members_root(project_id, lc_id);
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

async fn p0_registered_initialized_issue() -> (P0HttpFixture, String, String, String, Vec<String>) {
    let fixture = P0HttpFixture::real_git_aggregate().await;
    let (project_id, lc_id) = fixture
        .create_project_and_logical_codebase("P0 multi-repository HTTP chain")
        .await;
    let batch = fixture.preflight_and_submit_all(&project_id, &lc_id).await;
    assert_eq!(batch["status"], "completed", "registration batch: {batch}");

    let operation = fixture
        .start_initialization(&project_id, &lc_id, "p0-init-key")
        .await;
    let operation_id = operation["operation_id"].as_str().expect("operation id");
    let completed = fixture
        .poll_until_terminal(&project_id, &lc_id, operation_id)
        .await;
    assert_eq!(
        completed["status"], "completed",
        "initialization: {completed}"
    );
    assert_eq!(
        fixture.wait_for_active_index(&project_id, &lc_id).await["state"],
        "active"
    );

    let issue = fixture
        .create_issue_with_primary(&project_id, &lc_id, "P0 aggregate planning issue")
        .await;
    let issue_id = issue["issue_id"].as_str().expect("issue id").to_string();
    assert!(
        fixture.selection_is_persisted(&project_id, &lc_id, &issue_id),
        "HTTP issue creation must persist codebase selection"
    );
    let members = fixture.logical_member_ids(&project_id, &lc_id);
    (fixture, project_id, lc_id, issue_id, members)
}
#[tokio::test]
async fn reg_init_idx_pln_p0_chain_uses_only_http_routes() {
    let (fixture, project_id, _lc_id, issue_id, members) = p0_registered_initialized_issue().await;
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
    let (fixture, project_id, _lc_id, issue_id, members) = p0_registered_initialized_issue().await;
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

/// R9 多 LC 并存 e2e：同一 project 两个 LC 各自登记、各建 issue，互不串扰
/// （selection 落各自 lc 子树；LC-B 的 issue 拒绝 LC-A 的成员作为 primary）。
#[tokio::test]
async fn two_logical_codebases_coexist_without_cross_interference() {
    let fixture = P0HttpFixture::real_git_aggregate().await;
    let (project_id, lc_a) = fixture
        .create_project_and_logical_codebase("P0 multi-LC chain A")
        .await;
    fixture.preflight_and_submit_all(&project_id, &lc_a).await;

    // 第二个聚合根：单个真实 Git 成员。
    let second_root = fixture.root.join("second-root");
    let svc_root = second_root.join("svc");
    fs::create_dir_all(&svc_root).expect("create second member");
    git(
        &svc_root,
        &[
            "-c",
            "user.name=P0 Test",
            "-c",
            "user.email=p0@example.test",
            "init",
            "-q",
        ],
    );
    fs::write(svc_root.join("lib.rs"), "pub fn svc() {}\n").expect("write member source");
    git(&svc_root, &["add", "lib.rs"]);
    git(
        &svc_root,
        &[
            "-c",
            "user.name=P0 Test",
            "-c",
            "user.email=p0@example.test",
            "commit",
            "-qm",
            "svc baseline",
        ],
    );
    codegraph(&second_root, "init");

    let (status, codebase) = request(
        &fixture.app,
        Method::POST,
        &format!("/api/projects/{project_id}/logical-codebases"),
        json!({
            "name": "P0 LC B",
            "aggregate_root": second_root,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create second LC: {codebase}");
    let lc_b = codebase["logical_codebase_id"]
        .as_str()
        .or_else(|| codebase["id"].as_str())
        .expect("second lc id")
        .to_string();
    assert_ne!(lc_a, lc_b);

    // LC-B 登记走新路径（auto_discover 只发现 svc 一个成员）。
    let (status, preflight) = request(
        &fixture.app,
        Method::POST,
        &format!("/api/projects/{project_id}/logical-codebases/{lc_b}/registrations/preflight"),
        json!({
            "aggregate_root": second_root,
            "candidate_paths": [svc_root],
            "auto_discover": true,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "LC-B preflight: {preflight}");
    let items = preflight["items"].as_array().expect("preflight items");
    assert_eq!(items.len(), 1, "LC-B discovers exactly svc: {preflight}");
    let svc_path = items[0]["path"].as_str().expect("svc path").to_string();
    let (status, batch) = request(
        &fixture.app,
        Method::POST,
        &format!("/api/projects/{project_id}/logical-codebases/{lc_b}/registrations"),
        json!({
            "aggregate_root": second_root,
            "preflight_id": preflight["preflight_id"],
            "confirmed_paths": [svc_path],
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "LC-B submit: {batch}");
    assert_eq!(batch["status"], "completed", "LC-B batch: {batch}");

    // 成员互不串扰：HTTP members 按 lc 返回各自成员。
    let members_of = |lc_id: String| {
        let app = fixture.app.clone();
        let project_id = project_id.clone();
        async move {
            let (status, body) = request(
                &app,
                Method::GET,
                &format!("/api/projects/{project_id}/logical-codebases/{lc_id}/members"),
                json!({}),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "members: {body}");
            body["members"].as_array().expect("members array").clone()
        }
    };
    let members_a = members_of(lc_a.clone()).await;
    let members_b = members_of(lc_b.clone()).await;
    assert_eq!(members_a.len(), 2, "LC-A members: {members_a:?}");
    assert_eq!(members_b.len(), 1, "LC-B members: {members_b:?}");
    let ids_a: Vec<String> = members_a
        .iter()
        .filter_map(|m| m["logical_repository_id"].as_str().map(str::to_string))
        .collect();
    let ids_b: Vec<String> = members_b
        .iter()
        .filter_map(|m| m["logical_repository_id"].as_str().map(str::to_string))
        .collect();
    assert!(
        ids_b.iter().all(|id| !ids_a.contains(id)),
        "no shared logical members: {ids_a:?} vs {ids_b:?}"
    );

    // 两 LC 各建 issue；selection 落各自 lc 子树。
    let issue_a = fixture
        .create_issue_with_primary(&project_id, &lc_a, "LC-A issue")
        .await;
    let issue_b = fixture
        .create_issue_with_primary(&project_id, &lc_b, "LC-B issue")
        .await;
    let issue_a_id = issue_a["issue_id"].as_str().expect("issue A id");
    let issue_b_id = issue_b["issue_id"].as_str().expect("issue B id");
    assert!(
        fixture.selection_is_persisted(&project_id, &lc_a, issue_a_id),
        "LC-A selection must persist in its lc subtree"
    );
    assert!(
        fixture.selection_is_persisted(&project_id, &lc_b, issue_b_id),
        "LC-B selection must persist in its lc subtree"
    );
    assert!(
        !fixture.selection_is_persisted(&project_id, &lc_a, issue_b_id),
        "LC-B issue selection must not land in LC-A subtree"
    );

    // 串扰防护：LC-B 的 issue 不能用 LC-A 的成员做 primary。
    let lc_a_primary = fixture.primary_repository_id(&project_id, &lc_a).await;
    assert_error(
        request(
            &fixture.app,
            Method::POST,
            &format!("/api/projects/{project_id}/issues"),
            json!({
                "repository_id": lc_a_primary,
                "logical_codebase_id": lc_b,
                "title": "cross-LC primary must be rejected",
                "description": null,
            }),
        )
        .await,
        StatusCode::NOT_FOUND,
        "repository_not_found",
    );

    // 统一列表：两个逻辑条目并存。
    let (status, codebases) = request(
        &fixture.app,
        Method::GET,
        &format!("/api/projects/{project_id}/codebases"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{codebases}");
    let logical_entries = codebases["codebases"]
        .as_array()
        .expect("codebases array")
        .iter()
        .filter(|entry| entry["kind"] == "logical")
        .count();
    assert_eq!(logical_entries, 2, "both LCs stay listed: {codebases}");
}

/// R9 旧别名回归：新式 LC 建立后，旧 `/logical-codebase/*` 端点作为
/// 「默认第一个逻辑代码库」兼容别名仍路由到该 LC。
#[tokio::test]
async fn legacy_alias_endpoints_route_to_default_first_logical_codebase() {
    let fixture = P0HttpFixture::real_git_aggregate().await;
    let (project_id, lc_id) = fixture
        .create_project_and_logical_codebase("P0 legacy alias chain")
        .await;
    fixture.preflight_and_submit_all(&project_id, &lc_id).await;

    let (status, members) = request(
        &fixture.app,
        Method::GET,
        &format!("/api/projects/{project_id}/logical-codebase/members"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "legacy alias members: {members}");
    assert_eq!(
        members["members"].as_array().map(Vec::len),
        Some(2),
        "alias resolves the default first LC: {members}"
    );

    // 旧别名登记 preflight 也保持可达（路由层守卫，不真正提交）。
    let (status, preflight) = request(
        &fixture.app,
        Method::POST,
        &format!("/api/projects/{project_id}/logical-codebase/registrations/preflight"),
        json!({
            "aggregate_root": fixture.aggregate_root,
            "candidate_paths": fixture.member_roots,
            "auto_discover": true,
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "legacy alias preflight: {preflight}"
    );
}
