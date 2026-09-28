//! C4 Task 10：A03/A04 真实产品面故障注入与跨层验收（it_web）。
//!
//! - A03 `a03_new_lc_reaches_planning_ready_without_manual_seed_or_duplicate_provider_turn`：
//!   全新 LC 经 REST（project/LC/preflight/confirmed registration/initialization/
//!   bootstrap action）零手工 seed 达成 `planning_ready`；deterministic checkpoint
//!   （pre_check provider turn）注入一次中断；GET 只读投影不推进；带 expected
//!   revision 的显式 Retry 由原编排链从 checkpoint 续跑（已完成 provider turn
//!   不重跑）；同 command 重放 `replayed` 且 provider start count 不变；成员仓
//!   HEAD/dirty 全程不变。
//! - A04 `a04_failed_identity_and_missing_rules_require_product_repair_before_read_switch`：
//!   既有 migration fault injector + 真实 git remote 漂移产生 Failed identity
//!   journal；GET identity-repair 不经普通成员列表（Task 7 HTTP 断言）；mapping
//!   未确认前 read mode/authority JSON/member count/provider 启动计数全不变；
//!   错误 mapping 拒绝、正确 mapping staged 后 Revalidate 才切读（journal 保留
//!   repair audit）；删除实际成员 `.claude/rules/language.md` 后 provider/index
//!   admission 零启动（生产 driver 的 admission 预检接线），恢复同一规则来源后
//!   原链继续并冻结 policy digest。
//!
//! 全程同一 `WebAppState`：无服务重启、无 registry 清理、无 journal 删除、无
//! `repos.json`/权威 JSON 编辑脱困（repos.json 仅作为 legacy 存量布局在场景
//! 构造期 seed 一次）。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use cadence_aria::cross_cutting::bounded_command_runner::{
    BoundedCommandError, BoundedCommandRequest, BoundedCommandResult, BoundedCommandRunner,
};
use cadence_aria::cross_cutting::provider_adapter::{ProviderAdapter, ProviderAdapterError};
use cadence_aria::cross_cutting::provider_availability_gate::{
    ProviderAvailabilityGate, ProviderHealthSource,
};
use cadence_aria::cross_cutting::provider_health::{ProviderHealthEntry, ProviderHealthSnapshot};
use cadence_aria::cross_cutting::provider_registry::ProviderRegistry;
use cadence_aria::cross_cutting::streaming_provider::{
    FakeStreamingProvider, ProviderSession, StreamingProviderAdapter, StreamingProviderInput,
};
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::json_store::{ProductStoreError, write_json};
use cadence_aria::product::logical_codebase::aggregate_index::{
    AggregateIndexOperation, AggregateIndexSnapshotCollector, AggregateIndexStore, CodeGraphCli,
    CodeGraphExcludeGenerator,
};
use cadence_aria::product::logical_codebase::{
    IdentityMigrationExecutor, MigrationFaultInjector, RepositoryIdentityMapping,
};
use cadence_aria::product::models::{ProviderName, RepositoryRecord};
use cadence_aria::web::app::build_web_router;
use cadence_aria::web::events::EventHub;
use cadence_aria::web::gateway_factory::LogicalCodebaseGatewayFactory;
use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::state::WebAppState;
use chrono::Utc;
use serde_json::{Value, json};
use tempfile::tempdir;
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

const PROJECT_ID: &str = "project_0001";

// ---------------------------------------------------------------------------
// HTTP / git helpers。
// ---------------------------------------------------------------------------

async fn request_json(
    app: &axum::Router,
    method: Method,
    uri: &str,
    body: Value,
) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .expect("request"),
        )
        .await
        .expect("response");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, value)
}

async fn poll_until<F>(
    app: &axum::Router,
    uri: &str,
    predicate: F,
    description: &str,
) -> Value
where
    F: Fn(&Value) -> bool,
{
    let mut last = Value::Null;
    for _ in 0..600 {
        let (status, body) = request_json(app, Method::GET, uri, json!({})).await;
        assert_eq!(status, StatusCode::OK, "poll {description}: {body}");
        last = body;
        if predicate(&last) {
            return last;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("poll {description} did not converge: {last}");
}

fn bootstrap_uri(lc_id: &str) -> String {
    format!("/api/projects/{PROJECT_ID}/logical-codebases/{lc_id}/bootstrap")
}

fn bootstrap_step<'a>(projection: &'a Value, step: &str) -> &'a Value {
    projection["steps"]
        .as_array()
        .unwrap_or_else(|| panic!("projection steps missing: {projection}"))
        .iter()
        .find(|candidate| candidate["step"] == step)
        .unwrap_or_else(|| panic!("step {step} missing: {projection}"))
}

fn run_git(path: &std::path::Path, arguments: &[&str]) {
    let output = std::process::Command::new("git")
        .args(arguments)
        .current_dir(path)
        .output()
        .expect("git starts");
    assert!(
        output.status.success(),
        "git {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_repo_at(path: &std::path::Path) {
    std::fs::create_dir_all(path).expect("create repo dir");
    run_git(path, &["init", "-q"]);
    run_git(path, &["config", "user.email", "test@example.com"]);
    run_git(path, &["config", "user.name", "Test User"]);
}

fn commit_rule_file(path: &std::path::Path, content: &str) {
    let rules = path.join(".claude/rules");
    std::fs::create_dir_all(&rules).expect("create rules dir");
    std::fs::write(rules.join("language.md"), content).expect("write language.md");
    run_git(path, &["add", "."]);
    run_git(path, &["commit", "-q", "-m", "seed member rules"]);
}

fn git_head(path: &std::path::Path) -> String {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(path)
        .output()
        .expect("git rev-parse");
    assert!(output.status.success(), "git rev-parse HEAD failed");
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn git_porcelain(path: &std::path::Path) -> String {
    let output = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(path)
        .output()
        .expect("git status");
    String::from_utf8_lossy(&output.stdout).to_string()
}

/// `.aria` 下 per-LC authority 子树（manifest/members/checkouts/policy）的
/// 字节级清单——证明“mapping 未确认前 authority JSON 不变”。
fn authority_inventory(root: &Path, lc_id: &str) -> BTreeMap<String, Vec<u8>> {
    let mut inventory = BTreeMap::new();
    let subtree = root.join(".aria").join("logical-codebases").join(lc_id);
    let mut stack = vec![subtree];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let relative = path.strip_prefix(root).unwrap_or(&path);
                inventory.insert(
                    relative.to_string_lossy().into_owned(),
                    std::fs::read(&path).unwrap_or_default(),
                );
            }
        }
    }
    inventory
}

/// 生产 machine_skills 步骤在 fake runtime 下以 workspace root 为 home——
/// 预置本地技能源目录，避免在线 clone（与 web 层生产依赖测试同构）。
fn seed_local_skills_source(root: &Path) {
    let skills_source = root.join(".agents/Cadence-skills/cadence-init/skills/demo");
    std::fs::create_dir_all(&skills_source).expect("create skills source");
    std::fs::write(skills_source.join("SKILL.md"), "# demo\n").expect("write demo skill");
}

/// Fake CodeGraph CLI：成员名单驱动 acceptance 断言（与 LcOperationsFixture
/// 同构），保证 A03/A04 的 aggregate index 首建确定性成功。
#[derive(Clone)]
struct FakeCodeGraphCli {
    member_names: Vec<String>,
}

#[async_trait]
impl BoundedCommandRunner for FakeCodeGraphCli {
    async fn run(
        &self,
        request: BoundedCommandRequest,
    ) -> Result<BoundedCommandResult, BoundedCommandError> {
        let command = request.argv.first().map(String::as_str).unwrap_or_default();
        let ok = |stdout: String| BoundedCommandResult {
            exit_code: Some(0),
            stdout,
            stderr: String::new(),
            timed_out: false,
            cancelled: false,
            stdout_truncated: false,
            stderr_truncated: false,
            duration_ms: 0,
        };
        match command {
            "--version" => Ok(ok("1.6.0\n".to_string())),
            "init" | "sync" => Ok(ok(String::new())),
            "files" => {
                let files = self
                    .member_names
                    .iter()
                    .map(|name| format!(r#"{{"path":"{name}/lib.rs"}}"#))
                    .collect::<Vec<_>>()
                    .join(",");
                Ok(ok(format!("[{files}]")))
            }
            "query" => {
                let query = request.argv.get(1).map(String::as_str).unwrap_or_default();
                if query == "crossRepoGreeting" {
                    let hits = self
                        .member_names
                        .iter()
                        .map(|name| format!(r#"{{"path":"{name}/lib.rs"}}"#))
                        .collect::<Vec<_>>()
                        .join(",");
                    Ok(ok(format!("[{hits}]")))
                } else {
                    Ok(ok("[]".to_string()))
                }
            }
            _ => Err(BoundedCommandError::Io {
                details: format!("unexpected fake codegraph command: {command}"),
            }),
        }
    }
}

fn fake_index_operation(
    paths: ProductAppPaths,
    member_names: &[&str],
) -> Arc<AggregateIndexOperation> {
    Arc::new(AggregateIndexOperation::with_snapshot_dependencies(
        cadence_aria::product::logical_codebase::LogicalCodebaseStore::new(paths.clone()),
        AggregateIndexStore::new(paths.clone()),
        CodeGraphCli::new(
            Arc::new(FakeCodeGraphCli {
                member_names: member_names.iter().map(|s| s.to_string()).collect(),
            }),
            "fake-codegraph".to_string(),
        ),
        CodeGraphExcludeGenerator,
        AggregateIndexSnapshotCollector::for_paths(paths),
    ))
}

// ---------------------------------------------------------------------------
// A03：一次性 pre_check provider turn 中断的流式 provider（test-only）。
// ---------------------------------------------------------------------------

struct FaultOncePreCheckStreamingProvider {
    fired: AtomicBool,
}

#[async_trait]
impl StreamingProviderAdapter for FaultOncePreCheckStreamingProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        // deterministic checkpoint：首个 pre_check turn 在真实 gateway 启动路径
        // 内被注入一次中断（audit 只记录成功启动，中断不计 launch）。
        if input.prompt.contains("aggregate initialization turn: pre_check")
            && !self.fired.swap(true, Ordering::SeqCst)
        {
            return Err(ProviderAdapterError::execution_failed(
                None,
                String::new(),
                "test-injected interruption during the pre_check provider turn",
                0,
            ));
        }
        FakeStreamingProvider.start(input, cancel).await
    }
}

struct AlwaysHealthy(Arc<ProviderHealthSnapshot>);

impl ProviderHealthSource for AlwaysHealthy {
    fn snapshot(&self) -> Arc<ProviderHealthSnapshot> {
        self.0.clone()
    }

    fn degraded(&self) -> bool {
        false
    }
}

fn always_available_gate() -> Arc<ProviderAvailabilityGate> {
    let checked_at = Utc::now();
    let snapshot = Arc::new(ProviderHealthSnapshot {
        schema_version: 1,
        generation: 1,
        checked_at,
        providers: [ProviderName::ClaudeCode, ProviderName::Codex]
            .into_iter()
            .map(|provider| ProviderHealthEntry {
                provider,
                command: "stub".to_string(),
                available: true,
                version: Some("1.0".to_string()),
                reason_code: None,
                reason: None,
                checked_at,
            })
            .collect(),
    });
    Arc::new(ProviderAvailabilityGate::new(Arc::new(AlwaysHealthy(
        snapshot,
    ))))
}

struct StubSyncAdapter;

impl ProviderAdapter for StubSyncAdapter {
    fn run(&self, _input: &cadence_aria::protocol::contracts::AdapterInput) -> Result<cadence_aria::protocol::contracts::AdapterOutput, ProviderAdapterError> {
        Ok(cadence_aria::protocol::contracts::AdapterOutput {
            exit_code: Some(0),
            stdout: String::new(),
            stderr: String::new(),
            structured_output: None,
            files_modified: Vec::new(),
            duration_ms: 0,
            timeout_status: cadence_aria::protocol::contracts::TimeoutStatus::NotTimedOut,
        })
    }
}

/// A03 fixture：fault factory（经 `with_gateway_factory` 重建生产依赖，provider
/// turn 走生产 `GatewayFactoryProviderTurnDriver` + admission 预检）+ fake
/// CodeGraph CLI index operation。
async fn a03_app() -> (axum::Router, Arc<LogicalCodebaseGatewayFactory>, PathBuf) {
    let root = tempdir().expect("root");
    let root_path = root.path().to_path_buf();
    seed_local_skills_source(&root_path);
    let paths = ProductAppPaths::new(root_path.join(".aria"));

    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::ClaudeCode,
        Arc::new(FaultOncePreCheckStreamingProvider {
            fired: AtomicBool::new(false),
        }),
    );
    let factory = Arc::new(LogicalCodebaseGatewayFactory::new(
        paths.clone(),
        Arc::new(registry),
        Arc::new(StubSyncAdapter),
        always_available_gate(),
    ));

    let state = WebAppState::with_events(
        root_path.clone(),
        WebRuntime::new_fake(root_path.clone()),
        EventHub::new(),
    )
    .with_gateway_factory(factory.clone())
    .with_aggregate_index_operation(fake_index_operation(paths.clone(), &["alpha", "beta"]));
    let app = build_web_router(state);
    // manifest/登记事实须在 tempdir 存活期间使用——泄漏 root（与既有 it_web
    // fixture 同款取舍）。
    std::mem::forget(root);
    (app, factory, root_path)
}

/// A04 fixture：生产默认依赖（fake-mode registry + 生产 admission driver）+
/// fake CodeGraph CLI index operation。
fn a04_app(root_path: &Path) -> (axum::Router, Arc<LogicalCodebaseGatewayFactory>) {
    let paths = ProductAppPaths::new(root_path.join(".aria"));
    // 与生产 driver 相同的 admission 接线（with_gateway_factory 重建生产依赖），
    // registry 用恒可用 fake streaming provider——默认 factory 的 provider
    // gate 会做真实 ClaudeCode 健康探测，测试环境不可用。
    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::ClaudeCode, Arc::new(FakeStreamingProvider));
    let factory = Arc::new(LogicalCodebaseGatewayFactory::new(
        paths.clone(),
        Arc::new(registry),
        Arc::new(StubSyncAdapter),
        always_available_gate(),
    ));
    let state = WebAppState::with_events(
        root_path.to_path_buf(),
        WebRuntime::new_fake(root_path.to_path_buf()),
        EventHub::new(),
    )
    .with_gateway_factory(factory.clone())
    .with_aggregate_index_operation(fake_index_operation(paths, &["repository", "repository_0002"]));
    (build_web_router(state), factory)
}

/// 经 REST 完成 project + LC + preflight + confirmed registration。
async fn register_lc_members(
    app: &axum::Router,
    aggregate_root: &Path,
    member_paths: &[PathBuf],
) -> String {
    let (status, _) = request_json(
        app,
        Method::POST,
        "/api/projects",
        json!({"name":"A03 cold start","description":null}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, logical) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/logical-codebases",
        json!({"name":"Platform","aggregate_root":aggregate_root}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{logical}");
    let lc_id = logical["id"].as_str().expect("logical id").to_string();

    let (status, preflight) = request_json(
        app,
        Method::POST,
        &format!(
            "/api/projects/project_0001/logical-codebases/{lc_id}/registrations/preflight"
        ),
        json!({"aggregate_root":aggregate_root,"candidate_paths":[],"auto_discover":true}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{preflight:?}");
    let preflight_id = preflight["preflight_id"]
        .as_str()
        .expect("preflight id")
        .to_string();

    let confirmed_paths: Vec<String> = member_paths
        .iter()
        .map(|path| path.display().to_string())
        .collect();
    let (status, batch) = request_json(
        app,
        Method::POST,
        &format!("/api/projects/project_0001/logical-codebases/{lc_id}/registrations"),
        json!({
            "preflight_id": preflight_id,
            "aggregate_root": aggregate_root,
            "confirmed_paths": confirmed_paths,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{batch}");
    assert_eq!(batch["status"], "completed");
    lc_id
}

// ---------------------------------------------------------------------------
// A03：LC 冷启动 → 中断 → 显式续跑 → planning_ready → 重放。
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a03_new_lc_reaches_planning_ready_without_manual_seed_or_duplicate_provider_turn() {
    let (app, factory, root_path) = a03_app().await;
    let aggregate_root = root_path.join("aggregate-root");
    let member_a = aggregate_root.join("alpha");
    let member_b = aggregate_root.join("beta");
    for member in [&member_a, &member_b] {
        git_repo_at(member);
        commit_rule_file(member, "# language rule\n\n- Use Rust 2024 edition.\n");
    }
    let head_a = git_head(&member_a);
    let head_b = git_head(&member_b);

    // 零手工 seed：全部事实经 REST 产品动作产生。
    let lc_id = register_lc_members(&app, &aggregate_root, &[member_a.clone(), member_b.clone()]).await;

    // 冷启动投影：identity/manifest 完成（登记产物），rules_policy 尚无
    // policy artifact（未开始），member_index/aggregate 未开始。
    let uri = bootstrap_uri(&lc_id);
    let initial = poll_until(&app, &uri, |projection| {
        bootstrap_step(projection, "identity")["status"] == "completed"
            && bootstrap_step(projection, "manifest_checkout")["status"] == "completed"
    }, "initial registration facts")
    .await;
    assert_eq!(bootstrap_step(&initial, "rules_policy")["status"], "not_started");
    assert_eq!(bootstrap_step(&initial, "member_index")["status"], "not_started");
    assert_eq!(
        bootstrap_step(&initial, "aggregate_index_active")["status"],
        "not_started"
    );
    assert_eq!(initial["planning_ready"], false);
    let membership_revision = initial["membership_revision"].as_u64().expect("revision");
    let authority_root = initial["authority_root"]
        .as_str()
        .expect("authority root")
        .to_string();
    let canonical_aggregate_root =
        std::fs::canonicalize(&aggregate_root).expect("canonicalize aggregate root");
    assert_eq!(
        authority_root,
        canonical_aggregate_root.to_string_lossy().to_string(),
        "authority root must freeze the manifest aggregate root"
    );

    // member index：经产品 initialization 端点启动（首 turn 的 gateway factory
    // build 顺带 ensure_bootstrap 聚合 policy artifact）。
    let (status, accepted) = request_json(
        &app,
        Method::POST,
        &format!("/api/projects/{PROJECT_ID}/logical-codebases/{lc_id}/initializations"),
        json!({"idempotency_key":"key-a03-cold-start"}),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{accepted}");
    let operation_id = accepted["operation_id"]
        .as_str()
        .expect("operation id")
        .to_string();

    // deterministic checkpoint：pre_check provider turn 注入一次中断 →
    // operation Failed；provider 启动计数仍为 0（中断先于成功启动）。
    let interrupted = poll_until(
        &app,
        &uri,
        |projection| bootstrap_step(projection, "member_index")["status"] == "failed",
        "member_index fails at the injected pre_check interruption",
    )
    .await;
    let member_index = bootstrap_step(&interrupted, "member_index");
    assert_eq!(
        member_index["failure"]["reason_code"], "aggregate_pre_check_failed",
        "failure must name the interrupted turn: {member_index}"
    );
    assert_eq!(factory.audit().stream_launches(), 0);

    // “关闭页面”后再 GET：纯投影，不推进、不重复执行（两次 GET 语义一致，
    // provider 计数不变）。
    let reread = request_json(&app, Method::GET, &uri, json!({})).await;
    assert_eq!(reread.0, StatusCode::OK);
    assert_eq!(
        bootstrap_step(&reread.1, "member_index")["status"],
        "failed"
    );
    assert_eq!(factory.audit().stream_launches(), 0);

    // 从 REST 点击带 expected revision 的显式 Retry：原编排链从 checkpoint
    // 续跑（已完成 machine_skills/aggregate_preflight 不重跑，pre_check 及其
    // 后续 turn 各启动一次）。
    let action_uri = format!("{}/actions", bootstrap_uri(&lc_id));
    let command_id = "cmd-a03-member-index-retry-1";
    let (status, result) = request_json(
        &app,
        Method::POST,
        &action_uri,
        json!({
            "command_id": command_id,
            "step": "member_index",
            "action": "retry",
            "expected_revision": membership_revision,
            "expected_object_id": operation_id,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["outcome"], "accepted");

    let ready = poll_until(
        &app,
        &uri,
        |projection| projection["planning_ready"] == true,
        "bootstrap reaches planning_ready after the explicit retry",
    )
    .await;
    for step in [
        "identity",
        "manifest_checkout",
        "rules_policy",
        "member_index",
        "aggregate_index_active",
    ] {
        assert_eq!(
            bootstrap_step(&ready, step)["status"],
            "completed",
            "step {step} must complete: {ready}"
        );
    }
    // 唯一 authority + 冻结 policy digest。
    assert_eq!(
        ready["authority_root"].as_str().unwrap(),
        authority_root,
        "authority root must stay frozen on the same per-LC subtree"
    );
    let policy_digest = ready["policy"]["policy_digest"]
        .as_str()
        .expect("policy digest")
        .to_string();
    assert!(policy_digest.starts_with("sha256:"));

    // 恰好三个 provider turn（pre_check 重试 + rule_and_mcp_config +
    // openspec_and_examples）；已完成 provider turn 计数不增加。
    assert_eq!(factory.audit().stream_launches(), 3);

    // 成员仓零 Git 写副作用。
    assert_eq!(git_porcelain(&member_a), "");
    assert_eq!(git_porcelain(&member_b), "");
    assert_eq!(git_head(&member_a), head_a);
    assert_eq!(git_head(&member_b), head_b);

    // 重放同一 command：replayed，同 operation/index 身份，provider 计数不变。
    let (status, replay) = request_json(
        &app,
        Method::POST,
        &action_uri,
        json!({
            "command_id": command_id,
            "step": "member_index",
            "action": "retry",
            "expected_revision": membership_revision,
            "expected_object_id": operation_id,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{replay}");
    assert_eq!(replay["outcome"], "replayed");
    assert_eq!(replay["projection"]["planning_ready"], true);
    assert_eq!(
        bootstrap_step(&replay["projection"], "member_index")["object_id"],
        operation_id
    );
    assert_eq!(factory.audit().stream_launches(), 3);
}

// ---------------------------------------------------------------------------
// A04：Failed identity journal 修复 + 成员规则缺失的 admission 零启动。
// ---------------------------------------------------------------------------

/// 场景构造：legacy 布局 + 既有 fault injector 在 authority 写后中断一次；
/// 真实 git remote 漂移让原链复核 fail-closed 落 Failed journal。
struct FailFirstAuthorityWrite;

impl MigrationFaultInjector for FailFirstAuthorityWrite {
    fn after_authority_write(
        &self,
        _project_id: &str,
        _mapping: &RepositoryIdentityMapping,
    ) -> Result<(), ProductStoreError> {
        Err(ProductStoreError::Io(
            "test-injected interruption after the authority write".to_string(),
        ))
    }
}

fn legacy_record(id: &str, path: &Path) -> RepositoryRecord {
    RepositoryRecord {
        id: id.to_string(),
        project_id: PROJECT_ID.to_string(),
        name: id.to_string(),
        path: path.to_path_buf(),
        repo_hash: format!("legacy-hash-{id}"),
        runtime_root: PathBuf::from("/unused/.aria/runtime"),
        default_policy_preset: "manual-write".to_string(),
        default_provider_mode: "fake".to_string(),
        created_at: "2026-09-29T00:00:00Z".to_string(),
        updated_at: "2026-09-29T00:00:00Z".to_string(),
        logical_repository_id: None,
        primary_checkout_id: None,
        identity_schema_version: 0,
    }
}

fn legacy_lc_id(project_id: &str) -> String {
    use sha2::Digest;
    let digest = format!("{:x}", sha2::Sha256::digest(project_id.as_bytes()));
    format!("logical_codebase_{}", &digest[..32])
}

#[tokio::test]
async fn a04_failed_identity_and_missing_rules_require_product_repair_before_read_switch() {
    let root = tempdir().expect("root");
    let root_path = root.path().to_path_buf();
    seed_local_skills_source(&root_path);
    let paths = ProductAppPaths::new(root_path.join(".aria"));

    // legacy 存量布局：两个真实 git 成员仓（含实际规则文件；代表性查询
    // 的验收要求命中 ≥2 个成员）+ repos.json seed。
    let legacy_root = root_path.join("legacy");
    let repository = legacy_root.join("repository");
    let repository_b = legacy_root.join("repository_0002");
    for (member, remote) in [
        (&repository, "ssh://git@example.test/acme/api.git"),
        (&repository_b, "ssh://git@example.test/acme/web.git"),
    ] {
        git_repo_at(member);
        std::fs::write(member.join("lib.rs"), "pub fn cross_repo_greeting() {}\n")
            .expect("write lib");
        run_git(member, &["add", "."]);
        run_git(member, &["commit", "-q", "-m", "init"]);
        commit_rule_file(member, "# language rule\n\n- Use Rust 2024 edition.\n");
        run_git(member, &["remote", "add", "origin", remote]);
    }

    let (app, factory) = a04_app(&root_path);
    let (status, _) = request_json(
        &app,
        Method::POST,
        "/api/projects",
        json!({"name":"A04 identity repair","description":null}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    write_json(
        &paths
            .project_root(PROJECT_ID)
            .join("repos.json"),
        &vec![
            legacy_record("repository_0001", &repository),
            legacy_record("repository_0002", &repository_b),
        ],
    )
    .expect("seed legacy repos.json");

    // 既有 migration fault injector：authority 写后中断（journal 停在
    // WritingAuthority，registry 已有 active 条目）。
    IdentityMigrationExecutor::with_fault_injector(
        paths.clone(),
        Arc::new(FailFirstAuthorityWrite),
    )
    .ensure_through_authority(PROJECT_ID)
    .expect_err("injected interruption must surface");

    // 真实 source digest 漂移：origin remote 改名 → 原链复核 fail-closed 落
    // Failed journal（产品路径，非 JSON 编辑）。
    run_git(
        &repository,
        &["remote", "set-url", "origin", "ssh://git@example.test/acme/renamed.git"],
    );
    IdentityMigrationExecutor::new(paths.clone())
        .ensure_identity_schema(PROJECT_ID)
        .expect_err("drifted source identity must fail closed");

    let lc_id = legacy_lc_id(PROJECT_ID);
    let repair_uri = format!(
        "/api/projects/{PROJECT_ID}/logical-codebases/{lc_id}/identity-repair"
    );
    let bootstrap = bootstrap_uri(&lc_id);
    let members_uri = format!(
        "/api/projects/{PROJECT_ID}/logical-codebases/{lc_id}/members"
    );

    // Task 7 HTTP 断言：GET repair 直读 journal/registry/repos.json 低层事实，
    // 不经普通成员列表；digest/候选/影响范围可见。
    let (status, diagnostic) = request_json(&app, Method::GET, &repair_uri, json!({})).await;
    assert_eq!(status, StatusCode::OK, "{diagnostic}");
    assert_eq!(diagnostic["phase"], "failed");
    assert_eq!(diagnostic["project_id"], PROJECT_ID);
    assert!(diagnostic["source_repos_digest"].as_str().unwrap_or("").starts_with("sha256:"));
    assert_eq!(
        diagnostic["observed_source_repos_digest"],
        diagnostic["source_repos_digest"]
    );
    assert_eq!(diagnostic["mappings"].as_array().map(Vec::len), Some(2));
    let conflicts = diagnostic["conflicts"].as_array().expect("conflicts");
    assert!(
        conflicts
            .iter()
            .any(|conflict| conflict.as_str().unwrap_or("").starts_with("mapping_source_identity_mismatch")),
        "digest drift must be visible: {diagnostic}"
    );
    assert!(!diagnostic["impact"].as_array().unwrap().is_empty());
    // journal_updated_at 可补读（POST 的 expected 身份来源）。
    let journal_updated_at = diagnostic["journal_updated_at"]
        .as_str()
        .expect("journal updated_at")
        .to_string();

    // mapping 未确认前的零变更基线：authority 子树字节、成员数、provider
    // 启动计数、read mode（未切读）。
    let authority_before = authority_inventory(&root_path, &lc_id);
    let (status, members_before) = request_json(&app, Method::GET, &members_uri, json!({})).await;
    assert_eq!(status, StatusCode::OK, "{members_before}");
    let member_count_before = members_before["members"].as_array().map(Vec::len);
    assert_eq!(factory.audit().stream_launches(), 0);
    assert_ne!(diagnostic["read_mode"], json!("logical"));

    // 提交错误 mapping：仍 waiting（拒绝、零写入）。
    let (status, rejected) = request_json(
        &app,
        Method::POST,
        &repair_uri,
        json!({
            "command_id": "cmd-a04-wrong-mapping-1",
            "expected_journal_updated_at": journal_updated_at,
            "action": "submit_mapping",
            "mapping": {
                "legacy_repository_id": "repository_0001",
                "source_identity_digest": "sha256:wrong-digest",
                "logical_repository_id": "00000000-0000-0000-0000-000000000001",
                "primary_checkout_id": "00000000-0000-0000-0000-000000000002",
                "physical_repository_id": "repository_0001",
                "idempotency_key": "",
            },
        }),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{rejected}");
    assert_eq!(rejected["code"], "identity_repair_rejected");
    assert_eq!(authority_inventory(&root_path, &lc_id), authority_before);

    // 恢复同一实际来源（origin remote 改回）→ 冲突清空、候选可见。
    run_git(
        &repository,
        &["remote", "set-url", "origin", "ssh://git@example.test/acme/api.git"],
    );
    let (status, clean) = request_json(&app, Method::GET, &repair_uri, json!({})).await;
    assert_eq!(status, StatusCode::OK, "{clean}");
    assert_eq!(clean["conflicts"], json!([]));
    let candidates = clean["candidates"].as_array().expect("candidates");
    assert_eq!(candidates.len(), 1);
    let candidate = &candidates[0];
    let journal_updated_at = clean["journal_updated_at"]
        .as_str()
        .expect("journal updated_at after clean")
        .to_string();

    // 提交完整 mapping：staged（journal 保留 Failed + repair audit），read
    // mode/authority 仍未切换。
    let (status, staged) = request_json(
        &app,
        Method::POST,
        &repair_uri,
        json!({
            "command_id": "cmd-a04-correct-mapping-1",
            "expected_journal_updated_at": journal_updated_at,
            "action": "submit_mapping",
            "mapping": {
                "legacy_repository_id": candidate["legacy_repository_id"],
                "source_identity_digest": candidate["source_identity_digest"],
                "logical_repository_id": candidate["logical_repository_id"],
                "primary_checkout_id": candidate["primary_checkout_id"],
                "physical_repository_id": candidate["physical_repository_id"],
                "idempotency_key": "",
            },
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{staged}");
    assert_eq!(staged["phase"], "failed");
    let journal_updated_at = staged["journal_updated_at"]
        .as_str()
        .expect("journal updated_at after staging")
        .to_string();
    assert_ne!(
        journal_updated_at,
        clean["journal_updated_at"].as_str().unwrap_or(""),
        "staging must append the durable repair audit"
    );
    assert_eq!(authority_inventory(&root_path, &lc_id), authority_before);
    assert_ne!(staged["read_mode"], json!("logical"));

    // Revalidate：核验成功后才切换读；原 journal 保留（audit 可补读）。
    let (status, repaired) = request_json(
        &app,
        Method::POST,
        &repair_uri,
        json!({
            "command_id": "cmd-a04-revalidate-1",
            "expected_journal_updated_at": journal_updated_at,
            "action": "revalidate",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{repaired}");
    // 切读 marker（`switching_reads` + read_mode=logical_authoritative）即
    // 核验成功后的 durable 终态；此前任何阶段 read_mode 都不得是 logical。
    assert_eq!(repaired["phase"], "switching_reads");
    assert_eq!(repaired["read_mode"], "logical_authoritative");
    // 成员数与 provider 计数在修复全程不变。
    let (status, members_after) = request_json(&app, Method::GET, &members_uri, json!({})).await;
    assert_eq!(status, StatusCode::OK, "{members_after}");
    assert_eq!(
        members_after["members"].as_array().map(Vec::len),
        member_count_before
    );
    assert_eq!(factory.audit().stream_launches(), 0);

    // ---- 缺失成员规则：admission 零启动 → 产品准备动作恢复 → 原链继续 ----
    let rules_path = repository.join(".claude/rules/language.md");
    std::fs::remove_file(&rules_path).expect("delete the actual member rules");

    // bootstrap 投影的 rules_policy 步只读检查：waiting_for_human +
    // member_rules_missing，提供 Prepare/Retry。
    let (status, waiting) = request_json(&app, Method::GET, &bootstrap, json!({})).await;
    assert_eq!(status, StatusCode::OK, "{waiting}");
    let rules_step = bootstrap_step(&waiting, "rules_policy");
    assert_eq!(rules_step["status"], "waiting_for_human");
    assert_eq!(rules_step["failure"]["reason_code"], "member_rules_missing");
    let allowed = rules_step["allowed_actions"].as_array().expect("actions");
    assert!(allowed.contains(&json!("prepare")) && allowed.contains(&json!("retry")));

    // identity 修复完成后（marker 终态非 Failed、成员在册）冷启动身份步完成。
    assert_eq!(bootstrap_step(&waiting, "identity")["status"], "completed");

    // 触发 provider/index admission：成员规则缺失 → provider 零启动。
    let (status, accepted) = request_json(
        &app,
        Method::POST,
        &format!("/api/projects/{PROJECT_ID}/logical-codebases/{lc_id}/initializations"),
        json!({"idempotency_key":"key-a04-rules-missing"}),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{accepted}");
    let operation_id = accepted["operation_id"]
        .as_str()
        .expect("operation id")
        .to_string();
    let denied = poll_until(
        &app,
        &bootstrap,
        |projection| bootstrap_step(projection, "member_index")["status"] == "failed",
        "admission denies the provider turn while member rules are missing",
    )
    .await;
    // 拒绝原因落在 durable operation 记录的 error.message（投影 detail 只带
    // stage 摘要）：admission 预检拒绝必须可补读。
    let (_, operation_denied) = request_json(
        &app,
        Method::GET,
        &format!(
            "/api/projects/{PROJECT_ID}/logical-codebases/{lc_id}/initializations/{operation_id}"
        ),
        json!({}),
    )
    .await;
    let denial_message = operation_denied["error"]["message"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(
        denial_message.contains("admission"),
        "durable error must name the admission denial: {operation_denied}"
    );
    assert_eq!(
        factory.audit().stream_launches(),
        0,
        "missing member rules must keep the provider at zero launches"
    );

    // 通过产品准备动作恢复同一实际规则来源（真实成员仓规则文件），等待面
    // 随 GET 补读消失。
    std::fs::create_dir_all(repository.join(".claude/rules")).expect("recreate rules dir");
    std::fs::write(&rules_path, "# language rule\n\n- Use Rust 2024 edition.\n")
        .expect("restore language.md");

    // 显式 Retry：原链继续（admission 通过，三个 provider turn 各启动一次，
    // detached index build 落 active）。
    let (status, ready_result) = request_json(
        &app,
        Method::POST,
        &format!("{}/actions", bootstrap),
        json!({
            "command_id": "cmd-a04-member-index-retry-1",
            "step": "member_index",
            "action": "retry",
            "expected_revision": denied["membership_revision"].as_u64(),
            "expected_object_id": operation_id,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{ready_result}");
    assert_eq!(ready_result["outcome"], "accepted");

    let ready = poll_until(
        &app,
        &bootstrap,
        |projection| projection["planning_ready"] == true,
        "the original chain continues after the rules are restored",
    )
    .await;
    assert_eq!(
        factory.audit().stream_launches(),
        3,
        "the restored chain must launch exactly three provider turns"
    );
    // 冻结正确的 envelope/policy digest：两次补读一致且非空。
    let digest = ready["policy"]["policy_digest"]
        .as_str()
        .expect("policy digest")
        .to_string();
    assert!(digest.starts_with("sha256:"));
    let (status, reread) = request_json(&app, Method::GET, &bootstrap, json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(reread["policy"]["policy_digest"].as_str(), Some(digest.as_str()));
    assert_eq!(reread["planning_ready"], true);
}

