//! C4 Task 10：A03/A04 真实产品面故障注入与跨层验收（it_web）。
//!
//! - A03 `a03_new_lc_reaches_planning_ready_without_manual_seed_or_duplicate_provider_turn`：
//!   全新 LC 经 REST（project/LC/preflight/confirmed registration/登记后
//!   自动推进（REQ-REG-10）/bootstrap action）零手工 seed 达成
//!   `planning_ready`；deterministic checkpoint
//!   （pre_check provider turn）注入一次中断；GET 只读投影不推进；带 expected
//!   revision 的显式 Retry 由原编排链从 checkpoint 续跑（已完成 provider turn
//!   不重跑）；同 command 重放 `replayed` 且 provider start count 不变；成员仓
//!   HEAD/dirty 全程不变。
//! - A04 `a04_failed_identity_and_missing_rules_require_product_repair_before_read_switch`：
//!   既有 migration fault injector + 真实 git remote 漂移产生 Failed identity
//!   journal；GET identity-repair 不经普通成员列表（Task 7 HTTP 断言）；mapping
//!   未确认前 read mode/authority JSON/member count/provider 启动计数全不变；
//!   错误 mapping 拒绝、正确 mapping staged 后 Revalidate 才切读（journal 保留
//!   删除实际成员 `.claude/rules/language.md` 后 readiness 停在
//!   `root_recipe_operation_missing` 等待面（Task 1.6 起成员规则谓词归属
//!   provider admission；fix-admission-member-rules-scope 起 bootstrap 相位
//!   凭据豁免成员规则门——裸成员的 recipe turn 不再被 admission 拒绝，
//!   Failed 检查点改由注入的 pre_check 中断保留）；显式 Retry 后裸成员
//!   原链继续至 planning_ready、冻结 policy digest，成员规则零重建。
//!
//! 全程同一 `WebAppState`：无服务重启、无 registry 清理、无 journal 删除、无
//! `repos.json`/权威 JSON 编辑脱困（repos.json 仅作为 legacy 存量布局在场景
//! 构造期 seed 一次）。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
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
    ProviderSession, StreamingProviderAdapter, StreamingProviderInput,
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

async fn poll_until<F>(app: &axum::Router, uri: &str, predicate: F, description: &str) -> Value
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

/// 裸成员基线:main 分支 + 初始提交(REQ-PIB-02 生产不变量),但不含任何
/// `.claude/rules` 材料——owner 裸仓验收形态(fix-admission-member-rules-scope)。
fn git_commit_initial(path: &std::path::Path) {
    std::fs::write(path.join("README.md"), "# bare member\n").expect("write README");
    run_git(path, &["add", "."]);
    run_git(path, &["commit", "-q", "-m", "bare member baseline"]);
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
// A03/A04：一次性中断首个 provider turn 的流式 provider（test-only）。
// Task 4（aggregate-policy-root-publication）起在 RuleAndMcpConfig 轮向
// 聚合根生成 AGENTS/language 规则材料，使末命令发布收口有真实材料可聚合。
//
// Task 7 分流收口后 gateway 唯一经 `start_validated` 分发（绝不回落裸
// `start`），且库外（it_web）无法拆 `ValidatedStreamingProviderInput`
// （`into_parts` 为 pub(crate)），prompt 不可匹配——本 fixture 改按确定性
// 轮次合同注入：bootstrap 编排的首个 provider turn 恒为 pre_check
// （machine_skills/aggregate_preflight 是确定性步骤，不启动 provider），
// 首个 validated start 注入一次中断；中断后的首个成功 turn 是 pre_check
// 重试，第二个成功 turn 恒为 rule_and_mcp_config（真实 provider 生成根
// 规则的位置），在该轮向聚合根落盘材料。此前本 fixture 只实现 `start`，
// 生产路径全部命中 trait 默认的「validated start is not supported」，
// 注入从未生效（基线即红）。
// ---------------------------------------------------------------------------

struct FaultOncePreCheckStreamingProvider {
    fired: AtomicBool,
    successful_starts: AtomicUsize,
    /// provider spawn cwd（= canonical 聚合根；fixture 构造时已知）。
    aggregate_root: PathBuf,
}

impl FaultOncePreCheckStreamingProvider {
    fn new(aggregate_root: PathBuf) -> Self {
        Self {
            fired: AtomicBool::new(false),
            successful_starts: AtomicUsize::new(0),
            aggregate_root,
        }
    }

    async fn start_turn(&self) -> Result<ProviderSession, ProviderAdapterError> {
        // deterministic checkpoint：首个 pre_check turn 在真实 gateway 启动路径
        // 内被注入一次中断（audit 只记录成功启动，中断不计 launch）。
        if !self.fired.swap(true, Ordering::SeqCst) {
            return Err(ProviderAdapterError::execution_failed(
                None,
                String::new(),
                "test-injected interruption during the pre_check provider turn",
                0,
            ));
        }
        let successful = self.successful_starts.fetch_add(1, Ordering::SeqCst);
        // RuleAndMcpConfig 命令时机（第二个成功 turn）：真实 provider 生成
        // 根规则的位置——测试 provider 在聚合根落盘 AGENTS 入口与 language
        // 规则，使末命令发布收口有真实材料可聚合；该写入落在命令审计
        // 窗口内，重试链将其作为 allowlisted 变更冻结。
        if successful == 1 {
            let root = &self.aggregate_root;
            std::fs::write(root.join("AGENTS.md"), A03_ROOT_AGENTS).expect("write root AGENTS.md");
            std::fs::create_dir_all(root.join(".claude").join("rules"))
                .expect("create root rules dir");
            std::fs::write(
                root.join(".claude").join("rules").join("language.md"),
                A03_ROOT_LANGUAGE,
            )
            .expect("write root language rule");
        }
        // 轮次输出无结构化契约（aggregate recipe turn 消费自由文本摘要），
        // 以 plain completion 立即完结（同 ReviewStreamingAdapter 先例）。
        let (event_tx, events) = tokio::sync::mpsc::channel(4);
        let (commands, _command_rx) = tokio::sync::mpsc::channel(4);
        tokio::spawn(async move {
            let _ = event_tx
                .send(
                    cadence_aria::cross_cutting::streaming_provider::ProviderEvent::Completed(
                        cadence_aria::cross_cutting::streaming_provider::ProviderCompletion::plain(
                            "aggregate recipe fixture turn output".to_string(),
                            None,
                        ),
                    ),
                )
                .await;
        });
        Ok(ProviderSession {
            native_session_id: None,
            events,
            commands,
        })
    }
}

const A03_ROOT_AGENTS: &str =
    "# aggregate root rules\n\n- Members follow their .claude/rules language rules.\n";
const A03_ROOT_LANGUAGE: &str = "# language rule\n\n- Use Rust 2024 edition.\n";

#[async_trait]
impl StreamingProviderAdapter for FaultOncePreCheckStreamingProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.start_turn().await
    }

    /// 生产 gateway 唯一分发入口（Task 7 分流收口）。
    async fn start_validated(
        &self,
        _validated: cadence_aria::cross_cutting::session_launch::ValidatedStreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.start_turn().await
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
    fn run(
        &self,
        _input: &cadence_aria::protocol::contracts::AdapterInput,
    ) -> Result<cadence_aria::protocol::contracts::AdapterOutput, ProviderAdapterError> {
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
    crate::provider_capability_fixture::seed_local_skills_source(&root_path);
    let paths = ProductAppPaths::new(root_path.join(".aria"));

    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::ClaudeCode,
        Arc::new(FaultOncePreCheckStreamingProvider::new(
            root_path.join("aggregate-root"),
        )),
    );
    let factory = Arc::new(LogicalCodebaseGatewayFactory::new(
        paths.clone(),
        Arc::new(registry),
        Arc::new(StubSyncAdapter),
        always_available_gate(),
    ));

    let state = crate::provider_capability_fixture::with_available_provider_health(
        WebAppState::with_events(
            root_path.clone(),
            WebRuntime::new_fake(root_path.clone()),
            EventHub::new(),
        ),
        &root_path,
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
/// fake CodeGraph CLI index operation。`aggregate_root` 是 LC 的 canonical
/// 根（fixture provider 在 rule_config 轮向其落盘根材料）。
fn a04_app(
    root_path: &Path,
    aggregate_root: &Path,
) -> (axum::Router, Arc<LogicalCodebaseGatewayFactory>) {
    let paths = ProductAppPaths::new(root_path.join(".aria"));
    // 与生产 driver 相同的 admission 接线（with_gateway_factory 重建生产依赖），
    // registry 用恒可用 fake streaming provider——默认 factory 的 provider
    // gate 会做真实 ClaudeCode 健康探测，测试环境不可用。Task 1.8 重钉：
    // 注入 pre_check 一次中断。fix-admission-member-rules-scope 起 bootstrap
    // 相位凭据豁免成员规则门（裸成员可推进 recipe 链），为「中断→显式
    // Retry→readiness 闭环」保留 Failed 检查点。
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::ClaudeCode,
        Arc::new(FaultOncePreCheckStreamingProvider::new(
            aggregate_root.to_path_buf(),
        )),
    );
    let factory = Arc::new(LogicalCodebaseGatewayFactory::new(
        paths.clone(),
        Arc::new(registry),
        Arc::new(StubSyncAdapter),
        always_available_gate(),
    ));
    let state = crate::provider_capability_fixture::with_available_provider_health(
        WebAppState::with_events(
            root_path.to_path_buf(),
            WebRuntime::new_fake(root_path.to_path_buf()),
            EventHub::new(),
        ),
        root_path,
    )
    .with_gateway_factory(factory.clone())
    .with_aggregate_index_operation(fake_index_operation(
        paths,
        &["repository", "repository_0002"],
    ));
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
        &format!("/api/projects/project_0001/logical-codebases/{lc_id}/registrations/preflight"),
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
    // 片4（REQ-BOOT-07）：选择器前置——capability 记录作用域 = 项目级全局
    // （temp 根由 aggregate_root 的父目录恢复）。
    let root = aggregate_root
        .parent()
        .expect("fixture root is the aggregate root's parent");
    let paths = ProductAppPaths::new(root.join(".aria"));
    crate::provider_capability_fixture::seed_confirmed_claude(&paths, "project_0001");
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
    // 裸成员仓(fix-admission-member-rules-scope):无任何 .claude/rules 材料
    // ——路B 验收锚:bootstrap 凭据豁免成员规则门,根材料由 recipe 生成,
    // 成员文件树/Git 状态零写(不预写规则文件,不靠成员材料证明根有效)。
    for member in [&member_a, &member_b] {
        git_repo_at(member);
        git_commit_initial(member);
    }
    let head_a = git_head(&member_a);
    let head_b = git_head(&member_b);

    // 零手工 seed：全部事实经 REST 产品动作产生。
    let lc_id =
        register_lc_members(&app, &aggregate_root, &[member_a.clone(), member_b.clone()]).await;

    // 登记产物投影：identity/manifest 由登记提交同步完成（resume 产物）。
    // REQ-REG-10 起登记提交成功即后台自动推进首建链（rules_policy prepare
    // → 根配方 operation），后三步不再有可稳定观察的「全 not_started /
    // planning_ready==false」窗口——过时前提删除，其状态改由下方
    // 「注入中断 → failed 收敛」统一断言。
    let uri = bootstrap_uri(&lc_id);
    let initial = poll_until(
        &app,
        &uri,
        |projection| {
            bootstrap_step(projection, "identity")["status"] == "completed"
                && bootstrap_step(projection, "manifest_checkout")["status"] == "completed"
        },
        "initial registration facts",
    )
    .await;
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

    // deterministic checkpoint：首建 run 由 REQ-REG-10 登记后自动推进驱动
    // （唯一根配方 operation，idempotency_key 服务端派生；不再手工 POST
    // /initializations——那会开出第二个 operation，其完成态会把本检查点的
    // failed 投影覆盖成 completed）。首个 provider turn 恒为 pre_check：
    // 一次性中断被该 run 消费 → operation Failed，且自动推进对失败配方
    // 绝不重试（失败步骤停住等待人工）；provider 启动计数仍为 0（中断
    // 先于成功启动）。
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
    // operation 身份从投影取回：自动推进派生的 idempotency_key 对应唯一
    // 根配方 operation（member_index 步 object_id = 最新 operation id），
    // 下方显式 Retry 以 expected_object_id 精确钉住它。
    let operation_id = member_index["object_id"]
        .as_str()
        .expect("auto-advanced root recipe operation id")
        .to_string();
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

    // Task 4（生产同构）：根规则材料（AGENTS + .claude/rules/language.md）
    // 由 provider 在 RuleAndMcpConfig 轮生成（fixture 按轮次合同在第二个
    // 成功 turn 落盘，见 FaultOncePreCheckStreamingProvider）——中断窗口
    // 无需再手工预写 AGENTS；重试的命令审计窗口将其作为 allowlisted
    // 变更冻结，末命令收口发布真正文并冻结最终 receipt。成员仓保持裸态
    // 全程（bootstrap 凭据豁免成员规则门，不写成员规则）。

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

    // 成员仓零 Git 写副作用,且全程无任何规则文件被写入(裸成员零写:
    // bootstrap 凭据豁免成员规则门,不复制/生成成员规则)。
    assert!(!member_a.join(".claude/rules/language.md").exists());
    assert!(!member_b.join(".claude/rules/language.md").exists());
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
// A04：Failed identity journal 修复 + 成员规则缺失的 readiness 等待面与恢复链。
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
    crate::provider_capability_fixture::seed_local_skills_source(&root_path);
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

    let (app, factory) = a04_app(&root_path, &legacy_root);
    let (status, _) = request_json(
        &app,
        Method::POST,
        "/api/projects",
        json!({"name":"A04 identity repair","description":null}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    // 片4（REQ-BOOT-07）：选择器前置，种 project 作用域 Confirmed。必须在
    // project 记录落地后：seed 会先建 `projects/project_0001/` 目录，若先
    // seed 再 POST /api/projects，序号扫描（只看目录名）会把新 project 分到
    // project_0002，后续硬编码 project_0001 的路由即 404 project_not_found。
    crate::provider_capability_fixture::seed_confirmed_claude(&paths, PROJECT_ID);
    write_json(
        &paths.project_root(PROJECT_ID).join("repos.json"),
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
        &[
            "remote",
            "set-url",
            "origin",
            "ssh://git@example.test/acme/renamed.git",
        ],
    );
    IdentityMigrationExecutor::new(paths.clone())
        .ensure_identity_schema(PROJECT_ID)
        .expect_err("drifted source identity must fail closed");

    let lc_id = legacy_lc_id(PROJECT_ID);
    let repair_uri =
        format!("/api/projects/{PROJECT_ID}/logical-codebases/{lc_id}/identity-repair");
    let bootstrap = bootstrap_uri(&lc_id);
    let members_uri = format!("/api/projects/{PROJECT_ID}/logical-codebases/{lc_id}/members");

    // Task 7 HTTP 断言：GET repair 直读 journal/registry/repos.json 低层事实，
    // 不经普通成员列表；digest/候选/影响范围可见。
    let (status, diagnostic) = request_json(&app, Method::GET, &repair_uri, json!({})).await;
    assert_eq!(status, StatusCode::OK, "{diagnostic}");
    assert_eq!(diagnostic["phase"], "failed");
    assert_eq!(diagnostic["project_id"], PROJECT_ID);
    assert!(
        diagnostic["source_repos_digest"]
            .as_str()
            .unwrap_or("")
            .starts_with("sha256:")
    );
    assert_eq!(
        diagnostic["observed_source_repos_digest"],
        diagnostic["source_repos_digest"]
    );
    assert_eq!(diagnostic["mappings"].as_array().map(Vec::len), Some(2));
    let conflicts = diagnostic["conflicts"].as_array().expect("conflicts");
    assert!(
        conflicts.iter().any(|conflict| conflict
            .as_str()
            .unwrap_or("")
            .starts_with("mapping_source_identity_mismatch")),
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
        &[
            "remote",
            "set-url",
            "origin",
            "ssh://git@example.test/acme/api.git",
        ],
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

    // ---- 缺失成员规则：readiness 等待面 → 注入中断检查点 → 恢复 → 原链继续 ----
    let rules_path = repository.join(".claude/rules/language.md");
    std::fs::remove_file(&rules_path).expect("delete the actual member rules");

    // bootstrap 投影的 rules_policy 步只读检查（Task 1.6 起为 root
    // authority 三源谓词，成员规则检查归属 provider admission 预检）：
    // waiting_for_human + root_recipe_operation_missing（policy 可解析但
    // 本 LC 尚无 root recipe operation），提供 Prepare。
    let (status, waiting) = request_json(&app, Method::GET, &bootstrap, json!({})).await;
    assert_eq!(status, StatusCode::OK, "{waiting}");
    let rules_step = bootstrap_step(&waiting, "rules_policy");
    assert_eq!(rules_step["status"], "waiting_for_human");
    assert_eq!(
        rules_step["failure"]["reason_code"],
        "root_recipe_operation_missing"
    );
    let allowed = rules_step["allowed_actions"].as_array().expect("actions");
    assert!(allowed.contains(&json!("prepare")));

    // identity 修复完成后（marker 终态非 Failed、成员在册）冷启动身份步完成。
    assert_eq!(bootstrap_step(&waiting, "identity")["status"], "completed");

    // 触发 root recipe(成员规则仍缺失)。fix-admission-member-rules-scope
    // 新合同(取代 lcg Task 3a/A9「任何相位阻断」):有效 bootstrap 凭据豁免
    // 成员规则门——裸成员的 provider turn 不再被 admission 预检拒绝
    // (member_language_rules_missing 不进 durable error),失败检查点改由
    // 注入的 pre_check 中断保留(第一次启动即消费,deterministic checkpoint,
    // 中断先于成功启动,launch 计数不变)。
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
        "the injected pre_check interruption fails the first bootstrap run",
    )
    .await;
    // 失败事实落在 durable operation 记录的 error(投影 detail 只带 stage
    // 摘要):pre_check turn 的失败可补读,且必须是注入的中断而非
    // admission 阻断(凭据豁免已生效)。
    let (_, operation_denied) = request_json(
        &app,
        Method::GET,
        &format!(
            "/api/projects/{PROJECT_ID}/logical-codebases/{lc_id}/initializations/{operation_id}"
        ),
        json!({}),
    )
    .await;
    assert_eq!(
        operation_denied["error"]["code"], "aggregate_pre_check_failed",
        "durable error must name the interrupted turn: {operation_denied}"
    );
    // 契约迁移(报 Main):error.message 已是 copy 标题,机器事实按契约在
    // details.action/detail(dto/aggregate_initialization.rs 明文,
    // operation_http.rs 同款先例),断言语义不变。
    let durable_details = &operation_denied["error"]["details"];
    let durable_action = durable_details["action"].as_str().unwrap_or("");
    let durable_detail = durable_details["detail"].as_str().unwrap_or("");
    assert!(
        !durable_detail.contains("member_language_rules_missing")
            && !durable_action.contains("member_language_rules_missing"),
        "bootstrap credential waives the member rules gate, got: {operation_denied}"
    );
    assert!(
        !durable_detail.contains("provider admission preflight denied")
            && !durable_action.contains("provider admission preflight denied"),
        "the failure must be the injected interruption, not an admission denial: {operation_denied}"
    );
    assert!(
        durable_action.contains("gateway start_streaming failed")
            && durable_detail
                .contains("test-injected interruption during the pre_check provider turn"),
        "the injected turn failure must be observable and attributable: {operation_denied}"
    );
    assert_eq!(
        factory.audit().stream_launches(),
        0,
        "the injected interruption must fire before a successful launch"
    );

    // Task 4(生产同构,与 a03 同款):根规则材料由 provider 在
    // RuleAndMcpConfig 轮生成(fixture 轮次合同),窗口内无需手工预写
    // AGENTS;成员规则不恢复(真实裸成员推进 planning-ready)。

    // 显式 Retry:注入中断已消费,原链继续(三个 provider turn 各启动一次,
    // detached index build 落 active);成员文件树零写(bootstrap 不生成
    // 成员规则——规则文件保持删除状态)。
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

    let ready = poll_until(
        &app,
        &bootstrap,
        |projection| projection["planning_ready"] == true,
        "the bare member chain reaches planning_ready after the explicit retry",
    )
    .await;
    assert_eq!(
        factory.audit().stream_launches(),
        3,
        "the bare chain must launch exactly three provider turns"
    );
    // 成员仓零写:被删除的规则文件不被重建(bootstrap 不复制/生成成员规则)。
    assert!(
        !rules_path.exists(),
        "member rules must stay absent (zero write)"
    );
    // 冻结正确的 envelope/policy digest：两次补读一致且非空。
    let digest = ready["policy"]["policy_digest"]
        .as_str()
        .expect("policy digest")
        .to_string();
    assert!(digest.starts_with("sha256:"));
    let (status, reread) = request_json(&app, Method::GET, &bootstrap, json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        reread["policy"]["policy_digest"].as_str(),
        Some(digest.as_str())
    );
    assert_eq!(reread["planning_ready"], true);
}
