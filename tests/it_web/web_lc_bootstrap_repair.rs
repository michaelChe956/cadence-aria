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
//! - A05 `a05_zero_change_pre_check_turn_fails_the_step_with_durable_cause`：
//!   REQ-BOOT-17 负例——provider turn 正常结束但零落盘（零动作）时该 step
//!   必须判失败，durable 失败事实含 provider / step / 零变更三项
//!   （cause=root_recipe_turn_zero_change），后续 step 不推进。正例（A03/A04）
//!   的夹具按真实四命令落盘面产生变更，本负例是「零变更 ⇒ 失败」在 HTTP 面
//!   的护栏。
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
use cadence_aria::product::repository_store::RepositoryInitializationStepKind;
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
// `start`）。`ValidatedStreamingProviderInput::into_parts` 自 A1 起已是
// crate 外可见（it_web 集成测试可直接取回 `prompt` 与 spawn `working_dir`）
// ——故本 fixture 按 **prompt 里的真实 recipe 命令**识别当前 turn，不再
// 依赖「第几个成功 turn」的轮次猜测（断点续跑时轮次不可靠）。
//
// REQ-BOOT-17：root recipe 的 provider turn 零动作 ⇒ 该 step 判失败，故本
// fixture 按**真实四命令的落盘面**同构——每个 provider turn 都必须产生真实
// 文件变更（落盘面取自 `ROOT_RECIPE_ALLOWLIST` 的实测证据）：
// - pre_check → `.claude/settings.json`（真实 pre-check 写 `.claude/**`）；
// - rule_config → 根规则材料 `AGENTS.md` + `.claude/rules/language.md`
//   （真实 rule-config 写 `AGENTS.md` / `.claude/rules`）；
// - project-rules-examples → `cadence/project-rules/examples`。
//
// Task 4（aggregate-policy-root-publication）起在 RuleAndMcpConfig 轮向
// 聚合根生成 AGENTS/language 规则材料，使末命令发布收口有真实材料可聚合；
// 该写入落在命令审计窗口内，重试链将其作为 allowlisted 变更冻结。
// ---------------------------------------------------------------------------

/// 夹具 provider 的 turn 行为开关。
#[derive(Debug, Clone)]
struct RecipeProviderOptions {
    /// 首个 pre_check turn 注入一次中断（deterministic checkpoint：中断
    /// 先于成功启动，audit 不计 launch）。
    interrupt_first_pre_check: bool,
    /// REQ-BOOT-17 负例：pre_check turn **零动作**（不产生任何落盘）——
    /// 该 step 必须判失败（零动作 turn 不得静默通过）。
    zero_change_pre_check: bool,
}

impl Default for RecipeProviderOptions {
    fn default() -> Self {
        Self {
            interrupt_first_pre_check: true,
            zero_change_pre_check: false,
        }
    }
}

/// 真实 recipe 命令文本（与产品层 `RepositoryInitializationStepKind::command()`
/// 同源——不硬编码命令串，避免与生产命令漂移）。
fn recipe_command(kind: RepositoryInitializationStepKind) -> &'static str {
    kind.command().expect("recipe command")
}

struct FaultOncePreCheckStreamingProvider {
    options: RecipeProviderOptions,
    fired: AtomicBool,
    successful_starts: AtomicUsize,
}

impl FaultOncePreCheckStreamingProvider {
    fn new(options: RecipeProviderOptions) -> Self {
        Self {
            options,
            fired: AtomicBool::new(false),
            successful_starts: AtomicUsize::new(0),
        }
    }

    /// 真实四命令的落盘面（REQ-BOOT-17 同构）：每个 provider turn 都必须
    /// 产生真实文件变更。每次落盘的内容带 turn 序号——同一 turn 重复执行
    /// （重试/续跑）也是真实变更，夹具绝不产出「零变更」turn。
    fn write_turn_material(&self, prompt: &str, root: &Path, turn: usize) {
        if prompt.contains(recipe_command(RepositoryInitializationStepKind::PreCheck))
            && !self.options.zero_change_pre_check
        {
            let claude_dir = root.join(".claude");
            std::fs::create_dir_all(&claude_dir).expect("create root .claude dir");
            std::fs::write(
                claude_dir.join("settings.json"),
                format!("{{\"pre_check_turn\":{turn}}}\n"),
            )
            .expect("write root claude settings");
        }
        if prompt.contains(recipe_command(RepositoryInitializationStepKind::RuleConfig)) {
            std::fs::write(
                root.join("AGENTS.md"),
                format!("{A03_ROOT_AGENTS}\n- fixture provider turn {turn}.\n"),
            )
            .expect("write root AGENTS.md");
            let rules_dir = root.join(".claude").join("rules");
            std::fs::create_dir_all(&rules_dir).expect("create root rules dir");
            std::fs::write(
                rules_dir.join("language.md"),
                format!("{A03_ROOT_LANGUAGE}\n- fixture provider turn {turn}.\n"),
            )
            .expect("write root language rule");
        }
        if prompt.contains(recipe_command(
            RepositoryInitializationStepKind::ProjectRulesExamples,
        )) {
            let examples = root.join("cadence").join("project-rules").join("examples");
            std::fs::create_dir_all(&examples).expect("create project rules examples dir");
            std::fs::write(
                examples.join("examples.md"),
                format!("{A03_ROOT_EXAMPLES}\n- fixture provider turn {turn}.\n"),
            )
            .expect("write project rules examples");
        }
    }

    async fn start_turn(
        &self,
        input: StreamingProviderInput,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        // deterministic checkpoint：首个 pre_check turn 在真实 gateway 启动路径
        // 内被注入一次中断（audit 只记录成功启动，中断不计 launch）。
        if self.options.interrupt_first_pre_check
            && input
                .prompt
                .contains(recipe_command(RepositoryInitializationStepKind::PreCheck))
            && !self.fired.swap(true, Ordering::SeqCst)
        {
            return Err(ProviderAdapterError::execution_failed(
                None,
                String::new(),
                "test-injected interruption during the pre_check provider turn",
                0,
            ));
        }
        let turn = self.successful_starts.fetch_add(1, Ordering::SeqCst);
        // 真实落盘面（同上）：provider 在自己的 spawn cwd（= canonical 聚合根）
        // 内按当前命令写入材料——每个 turn 都必须留下真实变更。
        self.write_turn_material(&input.prompt, &input.working_dir, turn);
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
/// `/project-rules-examples` 的真实落盘面（`cadence/project-rules/examples`
/// ——见 ROOT_RECIPE_ALLOWLIST 的实测证据）：零动作 turn 会被判失败，故测试
/// provider 必须按真实命令落盘面产生文件变更。
const A03_ROOT_EXAMPLES: &str = "# root rule examples\n\n- Members share one language rule set.\n";

#[async_trait]
impl StreamingProviderAdapter for FaultOncePreCheckStreamingProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.start_turn(input).await
    }

    /// 生产 gateway 唯一分发入口（Task 7 分流收口）。
    async fn start_validated(
        &self,
        validated: cadence_aria::cross_cutting::session_launch::ValidatedStreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        // `into_parts` 自 A1 起 crate 外可见：库外 fixture 借此取回 prompt
        // 与 spawn cwd（生产同构——provider 在自己的 cwd 内落盘）。
        let (input, _launch) = validated.into_parts();
        self.start_turn(input).await
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
    a03_app_with(RecipeProviderOptions::default()).await
}

/// A03 fixture（可调 provider 行为）：REQ-BOOT-17 负例需要「零动作 pre_check
/// turn」的 provider，故把 turn 行为开关参数化。
async fn a03_app_with(
    options: RecipeProviderOptions,
) -> (axum::Router, Arc<LogicalCodebaseGatewayFactory>, PathBuf) {
    let root = tempdir().expect("root");
    let root_path = root.path().to_path_buf();
    crate::provider_capability_fixture::seed_local_skills_source(&root_path);
    let paths = ProductAppPaths::new(root_path.join(".aria"));

    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::ClaudeCode,
        Arc::new(FaultOncePreCheckStreamingProvider::new(options)),
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
/// fake CodeGraph CLI index operation。provider 在真实 recipe 命令时机向
/// LC 的 canonical 根（= provider 的 spawn cwd）落盘根材料。
fn a04_app(root_path: &Path) -> (axum::Router, Arc<LogicalCodebaseGatewayFactory>) {
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
            RecipeProviderOptions::default(),
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
// 场景用例按主题拆入同目录片段文件（行数门禁：主文件与每个片段均 < 1200
// 行）——断言逐字搬移，语义零改动：
// - a03_bootstrap_chain.rs：A03 冷启动 → 中断 → 显式续跑 → planning_ready。
// - a04_identity_repair.rs：A04 Failed identity journal 修复 + 成员规则恢复链。
// - a05_zero_change_guard.rs：A05 REQ-BOOT-17 零动作 turn 判失败负例。
// ---------------------------------------------------------------------------

include!("web_lc_bootstrap_repair/a03_bootstrap_chain.rs");
include!("web_lc_bootstrap_repair/a04_identity_repair.rs");
include!("web_lc_bootstrap_repair/a05_zero_change_guard.rs");
