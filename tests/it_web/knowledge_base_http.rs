//! Task 7（LC 知识库模块）：知识库 HTTP 面集成测试。
//!
//! 契约来源：实施计划 Task 7 端点表 + 前端 `web/src/api/types/knowledge-base.ts`
//! （字段名逐字对齐）。夹具范式沿用 `web_lc_operations_api.rs`：真实 project +
//! HTTP 创建的逻辑代码库 + fake runtime / fake provider health（health 与
//! capability 播种助手内联自 `provider_capability_fixture.rs`，该模块未在
//! it_web.rs 注册，本文件自包含）。

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::response::{IntoResponse, Response};
use cadence_aria::cross_cutting::aria_state_paths::AriaStatePaths;
use cadence_aria::cross_cutting::bounded_command_runner::{
    BoundedCommandRequest, BoundedCommandResult, BoundedCommandRunner,
};
use cadence_aria::cross_cutting::provider_availability_gate::ProviderAvailabilityGate;
use cadence_aria::cross_cutting::provider_boundary::PROVIDER_EXECUTION_SEMANTICS_HOST_OPEN;
use cadence_aria::cross_cutting::provider_capabilities::ProviderCapabilityEvidence;
use cadence_aria::cross_cutting::provider_health::{
    ProviderHealthService, SystemProviderHealthClock,
};
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::logical_codebase::knowledge_base::operation::{
    KnowledgeBaseOperation, KnowledgeBaseOperationInput, KnowledgeBaseOperationKind,
    KnowledgeBaseOperationStatus, KnowledgeBaseStepKind, KnowledgeBaseStepRecord,
    KnowledgeBaseStepStatus,
};
use cadence_aria::product::logical_codebase::knowledge_base::store::KnowledgeBaseOperationStore;
use cadence_aria::product::logical_codebase::policy::{
    AggregatePolicyArtifact, AggregatePolicyArtifactStore, ProviderDialect, ProviderWireDialect,
    SessionPolicyAction,
};
use cadence_aria::product::logical_codebase::provider_capability_store::{
    CapabilityEvidence, PROVIDER_CAPABILITY_RECORD_SCHEMA_VERSION, ProviderActionCapability,
    ProviderActionMatrix, ProviderCapabilityRecord, ProviderCapabilityStore,
};
use cadence_aria::product::logical_codebase::provider_gateway::{
    ProviderRefType, ResumeEvidenceState,
};
use cadence_aria::product::logical_codebase::root_recipe_receipt::{
    ROOT_RULE_ENTRY_FILE, RootRecipeCommandVerdict, RootRecipeFilesystemAuditor,
    RootRecipeReceiptStore, root_rule_digest,
};
use cadence_aria::product::logical_codebase::store::LogicalCodebaseStore;
use cadence_aria::product::logical_codebase::{
    AggregateInitializationOperation, AggregateInitializationOperationInput,
    AggregateInitializationOperationStore, AggregateInitializationStepKind,
    root_recipe_command_index,
};
use cadence_aria::web::app::build_web_router;
use cadence_aria::web::error::ApiError;
use cadence_aria::web::events::EventHub;
use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::state::WebAppState;
use serde_json::{Value, json};
use tempfile::{TempDir, tempdir};
use tower::ServiceExt;

const PROJECT_ID: &str = "project_0001";

/// 插件模板 `knowledge-base-bootstrap/user-input/` 的 9 份输入文件名
/// （GET /user-input 的 total_count 基线与种子助手共用）。
const USER_INPUT_FILES: [&str; 9] = [
    "api-scope.md",
    "base-info.md",
    "configuration-scope.md",
    "data-model-scope.md",
    "database-ddl.sql",
    "middleware-scope.md",
    "page-scope.md",
    "product.md",
    "project-scope.md",
];

/// 预检②/模板源根按 owner HOME 解析（`resolve_owner_home_from_env`）。
/// 仓内先例（T7HomeEnvGuard）用 `ExclusiveFileLock` 串行；外部测试 crate 取不到
/// 该 `pub(crate)` 工具，改用模块级互斥锁等价串行——只有本模块读写 HOME。
static HOME_LOCK: Mutex<()> = Mutex::new(());

struct HomeEnvGuard {
    _lock: MutexGuard<'static, ()>,
    previous: Option<OsString>,
}

impl HomeEnvGuard {
    fn acquire(home: &Path) -> Self {
        let lock = HOME_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = std::env::var_os("HOME");
        unsafe { std::env::set_var("HOME", home) };
        Self {
            _lock: lock,
            previous,
        }
    }
}

impl Drop for HomeEnvGuard {
    fn drop(&mut self) {
        match self.previous.clone() {
            Some(previous) => unsafe { std::env::set_var("HOME", previous) },
            None => unsafe { std::env::remove_var("HOME") },
        }
    }
}

async fn request_json(
    app: &axum::Router,
    method: Method,
    uri: &str,
    body: Value,
) -> (StatusCode, Value) {
    // 有界等待：handler 任何形式的挂起都转为测试失败，绝不无限阻塞。
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        app.clone().oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .expect("request"),
        ),
    )
    .await
    .expect("request timed out after 30s")
    .expect("response");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, value)
}

fn run_git(path: &Path, arguments: &[&str]) {
    let status = std::process::Command::new("git")
        .args(arguments)
        .current_dir(path)
        .status()
        .expect("git");
    assert!(status.success(), "git {arguments:?} failed");
}

// ---------------------------------------------------------------------------
// 内联夹具助手（源：tests/it_web/provider_capability_fixture.rs，未注册模块）。
// ---------------------------------------------------------------------------

/// 四家版本探测全成功的 stub runner（L1 health 确定性通过）。
struct AllAvailableRunner;

#[async_trait]
impl BoundedCommandRunner for AllAvailableRunner {
    async fn run(
        &self,
        request: BoundedCommandRequest,
    ) -> Result<
        BoundedCommandResult,
        cadence_aria::cross_cutting::bounded_command_runner::BoundedCommandError,
    > {
        let version = match request.executable.as_str() {
            "codex" => "2.0".to_string(),
            "pi" => "0.83.0".to_string(),
            "kimi" => "0.34.0".to_string(),
            _ => "1.0".to_string(),
        };
        Ok(BoundedCommandResult {
            exit_code: Some(0),
            stdout: version,
            stderr: String::new(),
            timed_out: false,
            cancelled: false,
            stdout_truncated: false,
            stderr_truncated: false,
            duration_ms: 1,
        })
    }
}

async fn with_available_provider_health(state: WebAppState, root: &Path) -> WebAppState {
    let runner: Arc<dyn BoundedCommandRunner> = Arc::new(AllAvailableRunner);
    let health = Arc::new(ProviderHealthService::with_dependencies(
        AriaStatePaths::from_workspace_root(root),
        runner.clone(),
        Arc::new(SystemProviderHealthClock),
        // 第 4 个参数是 **probe 命令超时**（`with_dependencies` 的 `timeout`，
        // 定义见 src/cross_cutting/provider_health.rs:106，唯一用途是探测子
        // 进程超时，:245），**不参与**快照新鲜度判定。快照过期阈值是
        // provider_selection.rs:70 的 `HEALTH_STALE_AFTER_MINUTES = 10`。
        // 选择期刷新条件只有「首次选择（generation == 0）或快照降级
        // （degraded）」（外加 checked_at 超过 10 分钟常量的过期分支），
        // 并非「几乎每次选择都刷新」。
        // 刷新持 refresh 锁跨 tokio::fs 落盘让出，与「注册提交自动推进任务
        // 同步 join 选择线程」（见 aggregate_initialization/handlers.inc.rs
        // 的 select_aggregate_provider_blocking）在单线程测试 runtime 上构成
        // 三元死锁：请求任务持锁等调度、选择线程等锁、测试线程被 join
        // 占死。真正消除死锁的是下面的**预刷快照**（generation ≥ 1、
        // degraded = false、checked_at = now）：使后续选择路径在 10 分钟
        // 常量内零 refresh、零锁竞争。
        std::time::Duration::from_secs(600),
        4096,
    ));
    // 预刷：构造后在任何请求之前把快照刷新鲜（stub 探测 + 一次落盘）。
    // 此后 600 秒内知识库选择链（status/create/preflight）与聚合初始化
    // 自动推进的选择线程全部命中新鲜快照、零刷新、零锁竞争，死锁三
    // 要素全部断开。回归见 concurrent_status_and_create_do_not_deadlock。
    health
        .refresh(tokio_util::sync::CancellationToken::new())
        .await
        .expect("prefetch provider health snapshot");
    let gate = Arc::new(ProviderAvailabilityGate::new(health.clone()));
    state.with_provider_health(health, gate, runner)
}

/// 三个 canonical action 行全 `Confirmed` 的 capability 记录（与
/// `provider_capability_fixture.rs` 的复刻口径一致）。
fn confirmed_capability_record(provider_type: ProviderRefType) -> ProviderCapabilityRecord {
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
        execution_semantics: PROVIDER_EXECUTION_SEMANTICS_HOST_OPEN.to_string(),
    }
}

/// 先物化 bootstrap 行，再把 ClaudeCode 覆盖写为 `Confirmed`。
fn seed_confirmed_claude(paths: &ProductAppPaths) {
    let store = ProviderCapabilityStore::new(paths.clone());
    store
        .ensure_bootstrap()
        .expect("bootstrap capability records");
    store
        .upsert(&confirmed_capability_record(ProviderRefType::ClaudeCode))
        .expect("seed confirmed claude capability");
}

// ---------------------------------------------------------------------------
// 环境种子。
// ---------------------------------------------------------------------------

/// 预检②的七个 knowledge-base skill + 两类模板的 skill 源目录。
fn seed_skills(home: &Path) {
    let skills = [
        "knowledge-base-bootstrap",
        "knowledge-base-base-info",
        "knowledge-base-api",
        "knowledge-base-pages",
        "knowledge-base-overview",
        "knowledge-base-context",
        "knowledge-base-update",
    ];
    for skill in skills {
        let dir = home.join(".claude/skills").join(skill);
        std::fs::create_dir_all(&dir).expect("create skill dir");
        std::fs::write(dir.join("SKILL.md"), format!("# {skill}\n")).expect("write SKILL.md");
    }
    // 初始化输入模板源（knowledge-base-bootstrap/user-input，9 份全量）。
    let bootstrap =
        home.join(".agents/Cadence-skills/cadence-init/skills/knowledge-base-bootstrap/user-input");
    std::fs::create_dir_all(&bootstrap).expect("create bootstrap source");
    for name in USER_INPUT_FILES {
        std::fs::write(bootstrap.join(name), "# 模板\n").expect("write template");
    }
    // 变更包模板源（knowledge-base-update/user-input/change-package/）。
    let update = home
        .join(".agents/Cadence-skills/cadence-init/skills/knowledge-base-update/user-input/change-package");
    std::fs::create_dir_all(&update).expect("create update source");
    for name in [
        "change-summary.md",
        "code-change.md",
        "database-change.md",
        "configuration-change.md",
        "verification.md",
    ] {
        std::fs::write(update.join(name), "# 模板\n").expect("write template");
    }
}

struct KnowledgeBaseFixture {
    _root: TempDir,
    _aggregate_root: TempDir,
    _home: TempDir,
    app: axum::Router,
    lc_id: String,
    paths: ProductAppPaths,
    aggregate_root: PathBuf,
}

impl KnowledgeBaseFixture {
    async fn new() -> Self {
        let root = tempdir().expect("root");
        let aggregate_root = tempdir().expect("aggregate root");
        let home = tempdir().expect("home");
        seed_skills(home.path());
        // 聚合根内放一个成员 git 仓（不登记，仅供目录形态真实）。
        let member = aggregate_root.path().join("alpha");
        std::fs::create_dir_all(&member).expect("create member dir");
        run_git(&member, &["init", "-q"]);
        run_git(&member, &["config", "user.email", "test@example.com"]);
        run_git(&member, &["config", "user.name", "Test User"]);
        std::fs::write(member.join("lib.rs"), "pub fn hello() {}\n").expect("write");
        run_git(&member, &["add", "."]);
        run_git(
            &member,
            &[
                "-c",
                "user.email=t@t",
                "-c",
                "user.name=t",
                "commit",
                "-q",
                "-m",
                "init",
            ],
        );

        let aggregate_root_path = aggregate_root.path().to_path_buf();
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        let state = with_available_provider_health(
            WebAppState::with_events(
                root.path().to_path_buf(),
                WebRuntime::new_fake(root.path().to_path_buf()),
                EventHub::new(),
            ),
            root.path(),
        )
        .await;
        let app = build_web_router(state);

        let (status, _) = request_json(
            &app,
            Method::POST,
            "/api/projects",
            json!({"name":"Knowledge Base","description":null}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let (status, logical) = request_json(
            &app,
            Method::POST,
            "/api/projects/project_0001/logical-codebases",
            json!({"name":"Platform","aggregate_root":aggregate_root_path}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{logical}");
        let lc_id = logical["id"].as_str().expect("logical id").to_string();
        // 经登记端点注册成员（manifest/members/checkouts 落 per-LC 子树；
        // LC create 只落 record，manifest 由登记流程写入——预检①的
        // owning_project 反查依赖它）。
        let (status, preflight) = request_json(
            &app,
            Method::POST,
            &format!(
                "/api/projects/project_0001/logical-codebases/{lc_id}/registrations/preflight"
            ),
            json!({
                "aggregate_root": aggregate_root_path,
                "candidate_paths": [],
                "auto_discover": true
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{preflight:?}");
        let preflight_id = preflight["preflight_id"]
            .as_str()
            .expect("preflight id")
            .to_string();
        let (status, batch) = request_json(
            &app,
            Method::POST,
            &format!("/api/projects/project_0001/logical-codebases/{lc_id}/registrations"),
            json!({
                "preflight_id": preflight_id,
                "aggregate_root": aggregate_root_path,
                "confirmed_paths": [
                    aggregate_root_path.join("alpha").display().to_string(),
                ],
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{batch}");
        assert_eq!(batch["status"], "completed");

        seed_confirmed_claude(&paths);

        Self {
            _root: root,
            _aggregate_root: aggregate_root,
            _home: home,
            app,
            lc_id,
            paths,
            aggregate_root: aggregate_root_path,
        }
    }
    fn kb_root(&self) -> PathBuf {
        self.aggregate_root.join("cadence").join("knowledge-base")
    }

    fn kb_uri(&self, suffix: &str) -> String {
        format!(
            "/api/projects/{PROJECT_ID}/logical-codebases/{}/knowledge-base{suffix}",
            self.lc_id
        )
    }
}

/// POST /initializations（bootstrap）→ 202，响应含 operation_id 与 steps，
/// 且可经 GET /initializations/{operation_id} 轮询。两段式（REQ-KB-14）：
/// 输入已准备且全部确认后，【知识库初始化】才起 bootstrap operation。
#[tokio::test]
async fn create_bootstrap_operation_returns_pollable_operation() {
    let fixture = KnowledgeBaseFixture::new().await;
    fixture.seed_aggregate_initialization();
    fixture.seed_filled_user_inputs();
    let _home = HomeEnvGuard::acquire(fixture._home.path());
    fixture.confirm_all_user_inputs().await;

    let (status, body) = request_json(
        &fixture.app,
        Method::POST,
        &fixture.kb_uri("/initializations"),
        json!({"kind":"bootstrap","idempotency_key":"kb-bootstrap-1"}),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let operation_id = body["operation_id"]
        .as_str()
        .expect("operation_id")
        .to_string();
    assert!(!operation_id.is_empty());
    assert_eq!(body["kind"], "bootstrap");
    assert_eq!(body["status"], "running");
    assert_eq!(body["input"]["provider"], "claude_code");
    assert_eq!(body["provider_selection"]["selected"], "claude_code");
    let steps = body["steps"].as_array().expect("steps");
    assert!(!steps.is_empty());
    assert_eq!(steps[0]["step_id"], "bootstrap");

    let (status, fetched) = request_json(
        &fixture.app,
        Method::GET,
        &fixture.kb_uri(&format!("/initializations/{operation_id}")),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{fetched}");
    assert_eq!(fetched["operation_id"], operation_id.as_str());
    // 有界等待 spawn 的 execute 收口（operation 进入终态）后再结束测试，
    // 避免后台任务在测试结束后拖尾持有资源。
    let settled = fixture.wait_operation_settled(&operation_id).await;
    assert!(
        ["completed", "failed", "cancelled"].contains(&settled.as_str()),
        "unexpected terminal status: {settled}"
    );
}

/// POST /initializations（update）变更包缺文档 → 4xx，
/// code == knowledge_base_change_package_invalid 且 details.violations 非空；
/// 预检失败不创建 operation（active 404）。
#[tokio::test]
async fn create_update_operation_rejected_when_change_package_invalid() {
    let fixture = KnowledgeBaseFixture::new().await;
    fixture.seed_aggregate_initialization();
    fixture.seed_complete_manifest();
    fixture.seed_empty_change_package("CHANGE-demo");
    let _home = HomeEnvGuard::acquire(fixture._home.path());

    let (status, body) = request_json(
        &fixture.app,
        Method::POST,
        &fixture.kb_uri("/initializations"),
        json!({
            "kind":"update",
            "idempotency_key":"kb-update-1",
            "package_path":"user-input/updates/CHANGE-demo"
        }),
    )
    .await;
    assert!(
        status.is_client_error(),
        "expected 4xx, got {status}: {body}"
    );
    assert_eq!(
        body["code"], "knowledge_base_change_package_invalid",
        "{body}"
    );
    let violations = body["details"]["violations"]
        .as_array()
        .expect("violations");
    assert!(!violations.is_empty());
    assert!(violations.iter().all(|violation| {
        violation["code"]
            .as_str()
            .is_some_and(|code| code.starts_with("knowledge_base_change_package_"))
    }));

    // 前置检查失败是请求期产物：不产生 operation。
    let (status, active) = request_json(
        &fixture.app,
        Method::GET,
        &fixture.kb_uri("/initializations/active"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{active}");
    assert_eq!(active["code"], "knowledge_base_operation_not_found");
}

/// 预检不满足（聚合初始化未完成）→ 4xx 且 code ==
/// knowledge_base_requires_aggregate_initialization；不创建 operation。
#[tokio::test]
async fn create_operation_rejected_when_aggregate_initialization_missing() {
    let fixture = KnowledgeBaseFixture::new().await;

    let (status, body) = request_json(
        &fixture.app,
        Method::POST,
        &fixture.kb_uri("/initializations"),
        json!({"kind":"bootstrap","idempotency_key":"kb-bootstrap-missing"}),
    )
    .await;
    assert!(
        status.is_client_error(),
        "expected 4xx, got {status}: {body}"
    );
    assert_eq!(
        body["code"], "knowledge_base_requires_aggregate_initialization",
        "{body}"
    );

    // 门（REQ-KB-13）同样拦截 active 查询：统一 422 前置失败码（零 operation
    // 语义由上面的 create 断言守护）。
    let (status, body) = request_json(
        &fixture.app,
        Method::GET,
        &fixture.kb_uri("/initializations/active"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(
        body["code"], "knowledge_base_requires_aggregate_initialization",
        "{body}"
    );
}

/// provider 选择失败面必须走仓内失败文案单一来源（failure_copy）：
/// 四家 CLI 可用但无人核验 ⇒ message 是中文成品文案（不再是英文原文），
/// code 仍是聚合选择期稳定码，候选表 details（逐候选 + action_hint）
/// 原样透出（候选数随选择器候选集，真机四家、本 stub 环境三家）；
/// 选择期失败零写入（不创建 operation）。
#[tokio::test]
async fn create_operation_provider_unverified_returns_chinese_failure_copy() {
    let fixture = KnowledgeBaseFixture::new().await;
    fixture.seed_aggregate_initialization();
    // 两段式分叉会解析模板源根（owner HOME），先钉住夹具 HOME 隔离真机环境。
    let _home = HomeEnvGuard::acquire(fixture._home.path());
    // 能力面重置回 bootstrap Unknown 行：L1 四家 CLI 全可用、无人核验。
    let capabilities = fixture
        .paths
        .provider_capabilities_root()
        .join("capabilities.json");
    std::fs::remove_file(&capabilities).expect("reset capability rows");
    ProviderCapabilityStore::new(fixture.paths.clone())
        .ensure_bootstrap()
        .expect("bootstrap unverified capability rows");

    let (status, body) = request_json(
        &fixture.app,
        Method::POST,
        &fixture.kb_uri("/initializations"),
        json!({"kind":"bootstrap","idempotency_key":"kb-needs-verification"}),
    )
    .await;
    assert!(
        status.is_client_error(),
        "expected 4xx, got {status}: {body}"
    );
    assert_eq!(
        body["code"], "aggregate_initialization_provider_needs_verification",
        "{body}"
    );
    let message = body["message"].as_str().expect("message");
    assert!(
        message.contains("核验"),
        "message must be the Chinese failure copy: {body}"
    );
    assert!(
        !message.contains("no candidate provider has verified capability"),
        "message must not leak the English original: {body}"
    );
    // 候选表原样透出：逐候选非空 action_hint，全部因未核验被跳过。
    let candidates = body["details"]["candidates"]["candidates"]
        .as_array()
        .expect("candidate table");
    assert!(
        !candidates.is_empty(),
        "write-cell candidates must be listed: {body}"
    );
    assert!(
        candidates.iter().all(|candidate| !candidate["action_hint"]
            .as_str()
            .unwrap_or_default()
            .is_empty()),
        "every candidate carries an action hint: {body}"
    );
    assert!(
        candidates
            .iter()
            .all(|candidate| candidate["skipped_reason"] == "needs_verification"),
        "bootstrap rows are all unverified: {body}"
    );

    let (status, _) = request_json(
        &fixture.app,
        Method::GET,
        &fixture.kb_uri("/initializations/active"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// GET /status：未初始化 → exists == false。
#[tokio::test]
async fn status_endpoint_reports_missing_knowledge_base() {
    let fixture = KnowledgeBaseFixture::new().await;
    // REQ-KB-13 门要求聚合初始化已完成；种三源事实后本用例聚焦
    //「manifest 未初始化 ⇒ exists == false」的投影语义。
    fixture.seed_aggregate_initialization();
    let _home = HomeEnvGuard::acquire(fixture._home.path());

    let (status, body) = request_json(
        &fixture.app,
        Method::GET,
        &fixture.kb_uri("/status"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["exists"], false);
    assert_eq!(body["schema_version"], Value::Null);
    assert_eq!(body["completed_stages"], json!([]));
    assert_eq!(body["skipped_stages"], json!([]));
    // 门已过（①✓②✓），仅输入缺失（③）——skills 在场是可判定的真事实。
    assert_eq!(body["skills_available"], true);
}

/// PUT 一个 .md → 200；GET 同路径 → 内容一致、摘要一致。
#[tokio::test]
async fn file_roundtrip_write_then_read() {
    let fixture = KnowledgeBaseFixture::new().await;
    fixture.seed_aggregate_initialization();
    std::fs::create_dir_all(fixture.kb_root().join("user-input")).expect("kb root");

    let (status, written) = request_json(
        &fixture.app,
        Method::PUT,
        &fixture.kb_uri("/file"),
        json!({"path":"user-input/base-info.md","content":"# 基本信息\n## 工程信息\n- 状态：不适用（测试）\n"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{written}");
    assert_eq!(written["path"], "user-input/base-info.md");
    let digest = written["digest"].as_str().expect("digest").to_string();
    assert_eq!(digest.len(), 64, "sha256 小写十六进制");

    let (status, read) = request_json(
        &fixture.app,
        Method::GET,
        &fixture.kb_uri("/file?path=user-input/base-info.md"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{read}");
    assert_eq!(read["path"], "user-input/base-info.md");
    assert_eq!(read["digest"], digest.as_str());
    let content = read["content"].as_str().expect("content");
    assert!(content.contains("# 基本信息"));
}

/// 带过期 base_digest 写 → 409 knowledge_base_file_conflict。
#[tokio::test]
async fn file_write_conflict_returns_409() {
    let fixture = KnowledgeBaseFixture::new().await;
    fixture.seed_aggregate_initialization();
    std::fs::create_dir_all(fixture.kb_root().join("user-input")).expect("kb root");
    std::fs::write(
        fixture.kb_root().join("user-input").join("base-info.md"),
        "# 已有内容\n",
    )
    .expect("seed file");

    let (status, body) = request_json(
        &fixture.app,
        Method::PUT,
        &fixture.kb_uri("/file"),
        json!({
            "path":"user-input/base-info.md",
            "content":"# 覆盖尝试\n",
            "base_digest":"0000000000000000000000000000000000000000000000000000000000000000"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "knowledge_base_file_conflict");
}

/// 知识库 operation 运行期 → PUT 409 knowledge_base_operation_running。
#[tokio::test]
async fn file_write_during_running_operation_returns_409() {
    let fixture = KnowledgeBaseFixture::new().await;
    fixture.seed_aggregate_initialization();
    std::fs::create_dir_all(fixture.kb_root().join("user-input")).expect("kb root");
    fixture.seed_running_operation();

    let (status, body) = request_json(
        &fixture.app,
        Method::PUT,
        &fixture.kb_uri("/file"),
        json!({"path":"user-input/base-info.md","content":"# 运行期写入\n"}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "knowledge_base_operation_running");
}

/// path=../../etc/passwd → 4xx（路径穿越不是 500）。
#[tokio::test]
async fn file_read_outside_root_returns_4xx() {
    let fixture = KnowledgeBaseFixture::new().await;
    fixture.seed_aggregate_initialization();
    std::fs::create_dir_all(fixture.kb_root()).expect("kb root");

    let (status, body) = request_json(
        &fixture.app,
        Method::GET,
        &fixture.kb_uri("/file?path=../../etc/passwd"),
        json!({}),
    )
    .await;
    assert!(
        status.is_client_error(),
        "expected 4xx, got {status}: {body}"
    );
    assert_eq!(body["code"], "knowledge_base_file_path_escapes");

    let (status, body) = request_json(
        &fixture.app,
        Method::PUT,
        &fixture.kb_uri("/file"),
        json!({"path":"../../etc/passwd","content":"x"}),
    )
    .await;
    assert!(
        status.is_client_error(),
        "expected 4xx, got {status}: {body}"
    );
    assert_eq!(body["code"], "knowledge_base_file_path_escapes");
}

/// 本批新稳定码一律映射到 4xx（绝不落 500）。
///
/// 注：输入确认族两码（`knowledge_base_user_input_confirmation_stale` /
/// `knowledge_base_user_input_not_confirmed`）的 HTTP 状态映射待 controller
/// 在 `src/web/error.rs` 归口登记（当前未登记落 500），登记后加入本表。
#[tokio::test]
async fn new_stable_codes_map_to_non_500_status() {
    let codes = [
        "knowledge_base_requires_aggregate_initialization",
        "knowledge_base_skills_unavailable",
        "knowledge_base_inputs_incomplete",
        "knowledge_base_manifest_missing",
        "knowledge_base_manifest_version_unsupported",
        "knowledge_base_not_initialized",
        "knowledge_base_manifest_invalid",
        "knowledge_base_change_package_invalid",
        "knowledge_base_change_package_path_invalid",
        "knowledge_base_change_package_missing_document",
        "knowledge_base_change_package_field_empty",
        "knowledge_base_change_package_reason_missing",
        "knowledge_base_change_package_domain_matrix_invalid",
        "knowledge_base_change_package_sensitive_content",
        "knowledge_base_change_package_commit_unresolvable",
        "knowledge_base_change_package_scope_mismatch",
        "knowledge_base_file_conflict",
        "knowledge_base_operation_running",
        "knowledge_base_operation_not_found",
        "knowledge_base_write_scope_violation",
        "other_knowledge_base_running",
        "knowledge_base_provider_capability_denied",
        // 文件面子码（Task 5 的 Display 形态，同样禁止 500）。
        "knowledge_base_file_path_escapes",
        "knowledge_base_file_not_found",
        "knowledge_base_file_not_editable",
        "knowledge_base_file_too_large",
        // 请求形状拒绝（kind 未知 / update 缺 package_path）。
        "invalid_knowledge_base_request",
    ];
    for code in codes {
        let response: Response = ApiError::runtime(code, "test", json!({})).into_response();
        let status = response.status();
        assert!(
            status.is_client_error(),
            "{code} must map to 4xx, got {status}"
        );
    }
}

/// 变更包三端点联动：模板创建 → 列表（含 lint 违规计数）→ 逐包 lint。
#[tokio::test]
async fn change_package_template_lint_and_list_roundtrip() {
    let fixture = KnowledgeBaseFixture::new().await;
    fixture.seed_aggregate_initialization();
    let _home = HomeEnvGuard::acquire(fixture._home.path());

    let (status, created) = request_json(
        &fixture.app,
        Method::POST,
        &fixture.kb_uri("/change-packages"),
        json!({"identifier":"demo-1"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{created}");
    assert_eq!(created["id"], "CHANGE-demo-1");
    assert_eq!(created["path"], "user-input/updates/CHANGE-demo-1");

    let (status, packages) = request_json(
        &fixture.app,
        Method::GET,
        &fixture.kb_uri("/change-packages"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{packages}");
    let packages = packages["packages"].as_array().expect("packages");
    let entry = packages
        .iter()
        .find(|entry| entry["id"] == "CHANGE-demo-1")
        .expect("created package listed");
    assert_eq!(entry["path"], "user-input/updates/CHANGE-demo-1");
    assert!(entry["violation_count"].as_u64().is_some());
    assert!(entry["violations"].as_array().is_some());

    let (status, lint) = request_json(
        &fixture.app,
        Method::GET,
        &fixture.kb_uri("/change-packages/lint?path=user-input/updates/CHANGE-demo-1"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{lint}");
    let violations = lint["violations"].as_array().expect("violations");
    let fields = lint["fields"].as_array().expect("fields");
    assert!(!violations.is_empty(), "模板未填字段必须产出违规");
    assert!(!fields.is_empty());
}

/// POST /bootstrap-inputs → 200，created 含相对路径。
#[tokio::test]
async fn bootstrap_inputs_create_templates() {
    let fixture = KnowledgeBaseFixture::new().await;
    fixture.seed_aggregate_initialization();
    let _home = HomeEnvGuard::acquire(fixture._home.path());

    let (status, body) = request_json(
        &fixture.app,
        Method::POST,
        &fixture.kb_uri("/bootstrap-inputs"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let created = body["created"].as_array().expect("created");
    assert!(!created.is_empty());
    assert!(created.contains(&json!("user-input/base-info.md")));
}

/// 修复轮 1（Important）：GET 未知 operation_id ⇒ 404 +
/// code == knowledge_base_operation_not_found（此前经 product_store_api_error
/// 落 500，违反稳定码契约——同 patch 的 active/cancel 与聚合面均为 404）。
#[tokio::test]
async fn get_unknown_operation_returns_404() {
    let fixture = KnowledgeBaseFixture::new().await;
    fixture.seed_aggregate_initialization();

    let (status, body) = request_json(
        &fixture.app,
        Method::GET,
        &fixture.kb_uri("/initializations/op_unknown_0001"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["code"], "knowledge_base_operation_not_found");
}

/// 修复轮 1（Minor）：cancel 端点此前是 12 个端点里唯一零覆盖的。
/// 种子 Running operation → POST cancel → 200 + cancelled 终态；
/// 终态再 cancel ⇒ 409 knowledge_base_operation_running（IdentityMismatch
/// 的既有映射语义）。
#[tokio::test]
async fn cancel_transitions_running_operation_to_cancelled() {
    let fixture = KnowledgeBaseFixture::new().await;
    fixture.seed_aggregate_initialization();
    fixture.seed_running_operation();
    let operation_id =
        cadence_aria::product::logical_codebase::knowledge_base::operation::derive_operation_id(
            PROJECT_ID,
            &fixture.lc_id,
            "seed",
        );
    let cancel_uri = fixture.kb_uri(&format!("/initializations/{operation_id}/cancel"));

    let (status, body) = request_json(&fixture.app, Method::POST, &cancel_uri, json!({})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["operation_id"], operation_id.as_str());
    assert_eq!(body["status"], "cancelled");

    // 终态 operation 再取消：状态机拒绝（非 Running ⇒ IdentityMismatch）。
    let (status, body) = request_json(&fixture.app, Method::POST, &cancel_uri, json!({})).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "knowledge_base_operation_running");
}

/// 修复轮 1（阻塞级回归）：并发的只读 status 与 create 请求不得互相卡死。
///
/// 修复前的并行挂死机理：注册提交自动推进的后台任务在测试 runtime 上
/// 同步 join provider 选择线程，而请求侧 provider health 刷新（首次选择
/// 即刷新）持 refresh 锁跨 tokio::fs 落盘让出——请求任务持锁等待调度、
/// 选择线程等锁、测试线程被 join 占死，三元死锁。
/// 本用例把「同一 fixture 上并发走选择链的请求」钉进回归：若夹具重新
/// 引入请求期刷新竞争（未预刷 / 快照降级），本用例在并行模式下挂住。
///
/// 失败形态：完整三元死锁复发时，测试线程被同步 `join` 钉死，request_json
/// 的 30 秒超时 future 永不被 poll ⇒ 实际失败形态是**测试挂死、需外部
/// 杀掉**（RED 证据：测试进程 exit=124 被外部超时杀掉），而不是有界等待
/// 超时失败。因此守护这条用例的是**预刷快照**，而不是超时机制。
#[tokio::test]
async fn concurrent_status_and_create_do_not_deadlock() {
    let fixture = KnowledgeBaseFixture::new().await;
    // 门（REQ-KB-13）要求先种聚合初始化；再种 user-input 前置已过 ⇒ create
    // 会真实走到 provider 选择链（本用例守护的并发路径），随后因输入缺失
    // 在预检③被拒（4xx、零 operation）。
    fixture.seed_aggregate_initialization();
    let _home = HomeEnvGuard::acquire(fixture._home.path());
    let app = fixture.app.clone();
    let status_uri = fixture.kb_uri("/status");
    let create_uri = fixture.kb_uri("/initializations");

    let (status, create) = tokio::join!(
        request_json(&app, Method::GET, &status_uri, json!({})),
        request_json(
            &app,
            Method::POST,
            &create_uri,
            json!({"kind":"bootstrap","idempotency_key":"kb-concurrent-1"}),
        ),
    );
    assert_eq!(status.0, StatusCode::OK, "{:?} {status:?}", status.0);
    assert_eq!(status.1["exists"], false);
    // 门已过且输入未准备 ⇒ 两段式分叉起起草 operation（202、user_input）；
    // create 仍走完整 provider 选择链（本用例守护的并发路径）。
    assert_eq!(
        create.0,
        StatusCode::ACCEPTED,
        "expected forked user_input operation: {:?}",
        create.1
    );
    assert_eq!(create.1["kind"], "user_input", "{:?}", create.1);
    let operation_id = create.1["operation_id"]
        .as_str()
        .expect("operation_id")
        .to_string();
    // 有界等待 spawn 的 execute 收口，避免后台任务拖尾持有资源。
    fixture.wait_operation_settled(&operation_id).await;
}

// 用户输入面与两段式分叉（本批新增；自主体拆出守 1200 行门禁）。
include!("knowledge_base_http/user_input_http_tests.rs");

// REQ-KB-13 统一前置门：门未满足逐端点拒绝 + 零写副作用；门满足后恢复。
include!("knowledge_base_http/gate_tests.rs");

// 夹具种子助手（三源聚合初始化种子等；自主体拆出守 1200 行门禁）。
include!("knowledge_base_http/fixture_seeds.rs");
