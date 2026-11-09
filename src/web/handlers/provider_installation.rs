//! provider / codegraph 安装更新的 HTTP 面（块2 片7A / REQ-PCI-01~06）。
//!
//! - `GET  /api/providers/install-targets`：五类对象一次返回（独立路径，
//!   避免与 `{job_id}` 路由冲突）。
//! - `POST /api/providers/{target}/install`：立即返回 `{job_id}`（异步
//!   作业，不阻塞请求）；Kimi 缺 `acknowledge_remote_script=true` → 400
//!   （REQ-PCI-01：远程脚本必须先告知来源，防绕过）。
//! - `GET  /api/providers/install/{job_id}`：轮询作业状态与结果。
//!
//! 不实现 version-cache 查询端点：24h 缓存是 `registry_version` 的内部
//! 实现细节（tasks.md 82 条契约不含该端点）。

use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::bounded_command_runner::{BoundedCommandError, BoundedCommandRequest};
use crate::cross_cutting::provider_health::ProviderHealthReasonCode;
use crate::cross_cutting::semver::compare_semver;
use crate::product::app_paths::ProductAppPaths;
use crate::product::provider_installation::InstallTarget;
use crate::product::provider_installation::job_store::InstallJobStore;
use crate::product::provider_installation::registry_version::{
    RegistryVersionService, ReqwestRegistrySource,
};
use crate::product::provider_installation::runner::InstallJobExecution;
use crate::product::provider_installation::source_table::{
    INSTALL_SOURCE_TABLE, install_source_label, npm_registry, row_for_wire,
};
use crate::web::error::ApiError;
use crate::web::state::WebAppState;

/// codegraph 本地探测超时（与 provider health 探测同量级）。
const CODEGRAPH_PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const CODEGRAPH_PROBE_OUTPUT_LIMIT: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InstallTargetStatusDto {
    pub target: String,
    pub display_name: String,
    pub cli_available: bool,
    pub local_version: Option<String>,
    pub latest_version: Option<String>,
    pub latest_checked_at: Option<DateTime<Utc>>,
    pub install_source: String,
    /// install / update / none / verify（REQ-PCI-02 按钮规则）。
    pub action: &'static str,
}

#[derive(Debug, Serialize)]
pub struct InstallTargetsResponse {
    pub targets: Vec<InstallTargetStatusDto>,
}

#[derive(Debug, Default, Deserialize)]
pub struct InstallRequestBody {
    /// 仅 Kimi 必填：确认「将执行来自 code.kimi.com 的安装脚本」。
    pub acknowledge_remote_script: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct InstallStartResponse {
    pub job_id: String,
}

fn product_paths(state: &WebAppState) -> ProductAppPaths {
    ProductAppPaths::new(state.workspace_root.join(".aria"))
}

fn install_job_store(state: &WebAppState) -> InstallJobStore {
    InstallJobStore::new(
        product_paths(state)
            .provider_installation_root()
            .join("jobs"),
    )
}

fn error_response(status: StatusCode, code: &str, message: impl Into<String>) -> Response {
    (status, Json(ApiError::validation(code, message))).into_response()
}

/// 按钮显示规则（REQ-PCI-02）：
/// 未安装（命令缺失）→ `install`；已安装但探针失败 → `verify`；
/// 版本落后（registry latest 精确比较）→ `update`；否则 `none`。
fn resolve_install_action(
    cli_available: bool,
    probe_passed: bool,
    local_version: Option<&str>,
    latest_version: Option<&str>,
) -> &'static str {
    if !cli_available {
        return "install";
    }
    if !probe_passed {
        return "verify";
    }
    if let (Some(local), Some(latest)) = (local_version, latest_version)
        && compare_semver(local, latest) == std::cmp::Ordering::Less
    {
        return "update";
    }
    "none"
}

/// codegraph 不在 provider health 矩阵内：本地状态用 `codegraph --version`
/// 现场探测（与建索引用的 CLI 同名同口径）。
struct CodegraphProbe {
    /// 进程能起来（命令存在）。
    available: bool,
    /// 版本探测成功（退出 0 且解析出版本）。
    probe_passed: bool,
    version: Option<String>,
}

async fn probe_codegraph(state: &WebAppState) -> CodegraphProbe {
    let mut environment = std::collections::BTreeMap::new();
    if let Some(path) = std::env::var_os("PATH") {
        environment.insert("PATH".to_string(), path.to_string_lossy().into_owned());
    }
    let request = BoundedCommandRequest {
        executable: "codegraph".to_string(),
        argv: vec!["--version".to_string()],
        working_dir: state.workspace_root.clone(),
        timeout: CODEGRAPH_PROBE_TIMEOUT,
        cancellation: CancellationToken::new(),
        environment,
        stdout_limit: CODEGRAPH_PROBE_OUTPUT_LIMIT,
        stderr_limit: CODEGRAPH_PROBE_OUTPUT_LIMIT,
    };
    match state.command_runner.run(request).await {
        // 进程能起来 ⇒ CLI 已安装；退出 0 ⇒ 版本探测通过。
        Ok(result) => CodegraphProbe {
            available: true,
            probe_passed: result.exit_code == Some(0) && !result.timed_out,
            version: (result.exit_code == Some(0) && !result.timed_out)
                .then(|| {
                    result
                        .stdout
                        .lines()
                        .map(str::trim)
                        .find(|line| !line.is_empty())
                        .map(str::to_string)
                })
                .flatten(),
        },
        // 命令缺失 → 未安装；I/O 异常按「已装但探测未过」处理（不给
        // 误导性的【安装】按钮）。
        Err(BoundedCommandError::CommandMissing { .. }) => CodegraphProbe {
            available: false,
            probe_passed: false,
            version: None,
        },
        Err(BoundedCommandError::Io { .. }) => CodegraphProbe {
            available: true,
            probe_passed: false,
            version: None,
        },
    }
}

/// 一次返回五类对象的安装/更新状态（REQ-PCI-01/02/04）。
pub async fn get_install_targets(State(state): State<WebAppState>) -> Json<InstallTargetsResponse> {
    let paths = product_paths(&state);
    let snapshot = state.provider_health.latest_diagnostic();
    let registry = npm_registry();
    let version_service = RegistryVersionService::new(
        paths.provider_version_cache_path(),
        Arc::new(ReqwestRegistrySource::new()),
    );
    let now = Utc::now();
    let codegraph = probe_codegraph(&state).await;

    let mut targets = Vec::with_capacity(INSTALL_SOURCE_TABLE.len());
    for row in INSTALL_SOURCE_TABLE.iter() {
        let Some(target) = InstallTarget::parse_wire(row.target_wire) else {
            continue;
        };
        let (cli_available, probe_passed, local_version) = match target {
            InstallTarget::Provider(provider) => {
                let entry = snapshot.entry(&provider);
                let available = entry.is_some_and(|entry| entry.available);
                let command_missing = entry
                    .and_then(|entry| entry.reason_code)
                    .is_some_and(|code| matches!(code, ProviderHealthReasonCode::CommandMissing));
                // 命令缺失 → 未安装；其它不可用（版本太低/探测失败）→
                // 已安装但探针未过（REQ-PCI-02 行 2）。
                (
                    available || !command_missing,
                    available,
                    entry.and_then(|entry| entry.version.clone()),
                )
            }
            InstallTarget::CodeGraph => (
                codegraph.available,
                codegraph.probe_passed,
                codegraph.version.clone(),
            ),
        };
        // Kimi 非 npm 安装源，但其**版本来源**是 npm 包（REQ-PCI-07）⇒ 同样查
        // registry 的 latest；查不到（离线）→ None，action 退化为 verify/none。
        let latest = match row.version_package {
            Some(package) => version_service
                .latest_version(&registry, package, now)
                .await
                .ok()
                .map(|entry| (entry.latest_version, entry.checked_at)),
            None => None,
        };
        let (latest_version, latest_checked_at) = latest
            .map_or((None, None), |(version, checked)| {
                (Some(version), Some(checked))
            });
        // 先按本地/最新版本算 action，再搬进 DTO（避免 move 后再借用）。
        let action = resolve_install_action(
            cli_available,
            probe_passed,
            local_version.as_deref(),
            latest_version.as_deref(),
        );
        targets.push(InstallTargetStatusDto {
            target: row.target_wire.to_string(),
            display_name: row.display_name.to_string(),
            cli_available,
            local_version,
            latest_version,
            latest_checked_at,
            install_source: install_source_label(row),
            action,
        });
    }
    Json(InstallTargetsResponse { targets })
}

/// 发起安装：立即返回 `{job_id}`，安装异步执行（REQ-PCI-05）。
pub async fn post_provider_install(
    State(state): State<WebAppState>,
    Path(target_wire): Path<String>,
    body: Option<Json<InstallRequestBody>>,
) -> Response {
    // 未知对象（含 fake）→ 400。
    let Some(target) = InstallTarget::parse_wire(&target_wire) else {
        return error_response(
            StatusCode::BAD_REQUEST,
            "install_target_unknown",
            format!("未知安装对象：{target_wire}"),
        );
    };
    let row = row_for_wire(&target_wire).expect("parsed target has source row");
    // Kimi（远程脚本安装源）：必须显式确认「将执行来自 code.kimi.com 的
    // 安装脚本」（REQ-PCI-01：不静默执行远程脚本）。
    if row.install_script.is_some() {
        let acknowledged = body
            .as_ref()
            .and_then(|Json(request)| request.acknowledge_remote_script)
            == Some(true);
        if !acknowledged {
            return error_response(
                StatusCode::BAD_REQUEST,
                "install_acknowledgement_required",
                "该对象通过官方脚本安装：将执行来自 code.kimi.com 的安装脚本；请在确认来源后重试（acknowledge_remote_script=true）",
            );
        }
    }
    let store = install_job_store(&state);
    // durable 先写 running，再 spawn 执行（REQ-PCI-05 作业流程）。
    let job = match store.create(&target_wire, Utc::now()) {
        Ok(job) => job,
        Err(error) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "install_job_store_error",
                error.to_string(),
            );
        }
    };
    let execution = InstallJobExecution::production(
        store,
        state.command_runner.clone(),
        state.workspace_root.clone(),
        state.provider_health.clone(),
    );
    let job_id = job.job_id.clone();
    tokio::spawn(async move {
        execution.run(&job_id, target).await;
    });
    // 立即返回作业标识，不等待安装完成。
    (
        StatusCode::OK,
        Json(InstallStartResponse { job_id: job.job_id }),
    )
        .into_response()
}

/// 轮询安装作业（durable；页面刷新不丢进度）。
pub async fn get_install_job(
    State(state): State<WebAppState>,
    Path(job_id): Path<String>,
) -> Response {
    let store = install_job_store(&state);
    match store.get(&job_id) {
        Ok(Some(job)) => (StatusCode::OK, Json(job)).into_response(),
        Ok(None) => error_response(
            StatusCode::NOT_FOUND,
            "install_job_not_found",
            format!("找不到安装作业：{job_id}"),
        ),
        Err(error) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "install_job_store_error",
            error.to_string(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use axum::Json;
    use axum::extract::State;
    use axum::http::StatusCode;
    use axum::response::{IntoResponse, Response};
    use chrono::{TimeDelta, Utc};
    use serde_json::{Value, json};
    use tempfile::tempdir;

    use super::{
        get_install_job, get_install_targets, post_provider_install, resolve_install_action,
    };
    use crate::cross_cutting::aria_state_paths::AriaStatePaths;
    use crate::cross_cutting::bounded_command_runner::{
        BoundedCommandError, BoundedCommandRequest, BoundedCommandResult, BoundedCommandRunner,
    };
    use crate::cross_cutting::provider_availability_gate::ProviderAvailabilityGate;
    use crate::cross_cutting::provider_health::{ProviderHealthService, SystemProviderHealthClock};
    use crate::web::runtime::WebRuntime;
    use crate::web::state::WebAppState;

    // ---- 测试基建（与 providers.rs 测试同款脚本 runner） ----

    struct ScriptedRunner {
        results: Mutex<VecDeque<Result<BoundedCommandResult, BoundedCommandError>>>,
        delay: Duration,
    }

    impl ScriptedRunner {
        fn new(
            results: Vec<Result<BoundedCommandResult, BoundedCommandError>>,
            delay: Duration,
        ) -> Arc<Self> {
            Arc::new(Self {
                results: Mutex::new(VecDeque::from(results)),
                delay,
            })
        }
    }

    #[async_trait::async_trait]
    impl BoundedCommandRunner for ScriptedRunner {
        async fn run(
            &self,
            _request: BoundedCommandRequest,
        ) -> Result<BoundedCommandResult, BoundedCommandError> {
            if !self.delay.is_zero() {
                tokio::time::sleep(self.delay).await;
            }
            self.results
                .lock()
                .expect("scripted results")
                .pop_front()
                .expect("scripted result")
        }
    }

    fn success(version: &str) -> Result<BoundedCommandResult, BoundedCommandError> {
        Ok(BoundedCommandResult {
            exit_code: Some(0),
            stdout: format!("cli {version}"),
            stderr: String::new(),
            timed_out: false,
            cancelled: false,
            stdout_truncated: false,
            stderr_truncated: false,
            duration_ms: 1,
        })
    }

    fn missing(program: &str) -> Result<BoundedCommandResult, BoundedCommandError> {
        Err(BoundedCommandError::CommandMissing {
            executable: program.to_string(),
            details: "not found".to_string(),
        })
    }

    fn install_success() -> Result<BoundedCommandResult, BoundedCommandError> {
        Ok(BoundedCommandResult {
            exit_code: Some(0),
            stdout: "added 1 package in 3s".to_string(),
            stderr: String::new(),
            timed_out: false,
            cancelled: false,
            stdout_truncated: false,
            stderr_truncated: false,
            duration_ms: 1,
        })
    }

    /// 4 个版本探测结果 → 一个 health 服务（claude/codex/pi/kimi 顺序）。
    fn health_service(
        root: &Path,
        results: Vec<Result<BoundedCommandResult, BoundedCommandError>>,
    ) -> Arc<ProviderHealthService> {
        Arc::new(ProviderHealthService::with_dependencies(
            AriaStatePaths::from_workspace_root(root),
            ScriptedRunner::new(results, Duration::ZERO),
            Arc::new(SystemProviderHealthClock),
            Duration::from_secs(1),
            4096,
        ))
    }

    fn state(
        root: &Path,
        health: Arc<ProviderHealthService>,
        install_runner: Arc<dyn BoundedCommandRunner>,
    ) -> WebAppState {
        let gate = Arc::new(ProviderAvailabilityGate::new(health.clone()));
        let mut state =
            WebAppState::new(root.to_path_buf(), WebRuntime::new_fake(root.to_path_buf()))
                .with_provider_health(health, gate, install_runner);
        state.test_provider_enabled = false;
        state
    }

    /// 预置 24h durable 版本缓存，避免 handler 测试触网。
    fn seed_version_cache(root: &Path, entries: Value) {
        let cache_path = root.join(".aria/state/provider-installation/version-cache.json");
        std::fs::create_dir_all(cache_path.parent().expect("parent")).expect("create cache dir");
        std::fs::write(
            &cache_path,
            serde_json::to_vec(&json!({ "entries": entries })).unwrap(),
        )
        .expect("seed cache");
    }

    async fn response_json(response: Response) -> Value {
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        serde_json::from_slice(&body).expect("json body")
    }

    async fn poll_job_terminal(state: &WebAppState, job_id: &str) -> Value {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        loop {
            let response = get_install_job(
                State(state.clone()),
                axum::extract::Path(job_id.to_string()),
            )
            .await;
            assert_eq!(response.status(), StatusCode::OK);
            let job = response_json(response).await;
            if job["status"] != json!("running") {
                return job;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "install job did not reach terminal state in time"
            );
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    // ---- resolve_install_action 规则 ----

    #[test]
    fn install_target_action_rules_follow_spec() {
        // 未安装 → install
        assert_eq!(
            resolve_install_action(false, false, None, Some("2.0.0")),
            "install"
        );
        // 已安装但探针失败 → verify（不显示安装）
        assert_eq!(
            resolve_install_action(true, false, Some("1.0.0"), Some("2.0.0")),
            "verify"
        );
        // 版本落后 → update
        assert_eq!(
            resolve_install_action(true, true, Some("1.0.0"), Some("2.0.0")),
            "update"
        );
        // 预发布本地 < 正式最新 → update
        assert_eq!(
            resolve_install_action(true, true, Some("1.5.0-rc.1"), Some("1.5.0")),
            "update"
        );
        // 版本最新 → none
        assert_eq!(
            resolve_install_action(true, true, Some("2.0.0"), Some("2.0.0")),
            "none"
        );
        // registry 不可知（离线 / 查询失败）→ none（已装且探过）
        assert_eq!(
            resolve_install_action(true, true, Some("2.0.0"), None),
            "none"
        );
    }

    // ---- GET install-targets ----

    #[tokio::test]
    async fn install_targets_endpoint_lists_five_targets_with_actions() {
        let root = tempdir().expect("root");
        // health：claude ok、codex 缺失、pi ok（落后）、kimi ok。
        let health = health_service(
            root.path(),
            vec![
                success("1.0.0"),
                missing("codex"),
                success("0.85.0"),
                success("0.34.0"),
            ],
        );

        // handler 读取 latest_diagnostic——先刷新一次填入脚本化探测结果。
        health
            .refresh(tokio_util::sync::CancellationToken::new())
            .await
            .expect("refresh health");
        // codegraph 探测：命令缺失。
        let state = state(
            root.path(),
            health,
            ScriptedRunner::new(vec![missing("codegraph")], Duration::ZERO),
        );
        // 预置缓存：claude=1.0.0（最新）、codex=2.0.0（无关，缺装）、
        // pi=1.5.0（落后）、codegraph=1.6.2（无关，缺装）。
        // pi 的「落后」必须是**精确 semver 下真落后**（REQ-PCI-07）：
        // 本地 0.85.0（devbox pin）vs latest 1.5.0（块2 技术方案
        // §3.6.9 未验证项 1 实测的 registry latest）⇒ 0.85.0 < 1.5.0。
        // （旧的 0.9.0 在 semver 下**新于** 0.85.0 —— minor 9 < 85。）
        let checked_at = (Utc::now() - TimeDelta::hours(1)).to_rfc3339();
        seed_version_cache(
            root.path(),
            json!({
                "@anthropic-ai/claude-code": {
                    "package": "@anthropic-ai/claude-code",
                    "latest_version": "1.0.0",
                    "checked_at": checked_at
                },
                "@openai/codex": {
                    "package": "@openai/codex",
                    "latest_version": "2.0.0",
                    "checked_at": checked_at
                },
                "@earendil-works/pi-coding-agent": {
                    "package": "@earendil-works/pi-coding-agent",
                    "latest_version": "1.5.0",
                    "checked_at": checked_at
                },
                "@colbymchenry/codegraph": {
                    "package": "@colbymchenry/codegraph",
                    "latest_version": "1.6.2",
                    "checked_at": checked_at
                },
                "@moonshot-ai/kimi-code": {
                    "package": "@moonshot-ai/kimi-code",
                    "latest_version": "2.5.2",
                    "checked_at": checked_at
                }
            }),
        );

        let response = get_install_targets(State(state)).await;
        let payload = response_json(response.into_response()).await;

        let targets = payload["targets"].as_array().expect("targets array");
        assert_eq!(targets.len(), 5, "五类对象一次返回: {payload}");

        let by_target = |wire: &str| {
            targets
                .iter()
                .find(|dto| dto["target"] == json!(wire))
                .unwrap_or_else(|| panic!("missing {wire}: {payload}"))
                .clone()
        };

        let claude = by_target("claude_code");
        assert_eq!(claude["cli_available"], json!(true));
        assert_eq!(claude["local_version"], json!("1.0.0"));
        assert_eq!(claude["latest_version"], json!("1.0.0"));
        assert_eq!(claude["action"], json!("none"));
        assert_eq!(
            claude["install_source"],
            json!("npm @anthropic-ai/claude-code")
        );
        assert!(claude["latest_checked_at"].is_string());

        let codex = by_target("codex");
        assert_eq!(codex["cli_available"], json!(false));
        assert_eq!(codex["action"], json!("install"));

        let pi = by_target("pi");
        assert_eq!(pi["cli_available"], json!(true));
        assert_eq!(pi["local_version"], json!("0.85.0"));
        assert_eq!(pi["latest_version"], json!("1.5.0"));
        assert_eq!(pi["action"], json!("update"));

        // Kimi：版本来源 = npm 包 `@moonshot-ai/kimi-code`（REQ-PCI-07）⇒
        // 参与「有新版」判定；本地 0.34.0 < latest 2.5.2 ⇒ update。
        // **安装源仍是官方脚本**（REQ-PCI-01 不变）。
        let kimi = by_target("kimi_code");
        assert_eq!(kimi["cli_available"], json!(true));
        assert_eq!(kimi["local_version"], json!("0.34.0"));
        assert_eq!(kimi["latest_version"], json!("2.5.2"));
        assert!(kimi["latest_checked_at"].is_string());
        assert_eq!(kimi["action"], json!("update"));
        assert_eq!(
            kimi["install_source"],
            json!("official script https://code.kimi.com/kimi-code/install.sh")
        );

        let codegraph = by_target("codegraph");
        assert_eq!(codegraph["cli_available"], json!(false));
        assert_eq!(codegraph["action"], json!("install"));
        assert_eq!(
            codegraph["install_source"],
            json!("npm @colbymchenry/codegraph")
        );
    }

    /// Kimi **未安装** → 【安装】而不是【更新】：版本判定与安装判定相互独立
    /// —— 命令缺失时永远是 install（REQ-PCI-02 / REQ-PCI-07）。
    #[tokio::test]
    async fn install_targets_reports_install_for_missing_kimi() {
        let root = tempdir().expect("root");
        // health：claude ok、codex ok、pi ok、**kimi 缺失**。
        let health = health_service(
            root.path(),
            vec![
                success("1.0.0"),
                success("0.166.0"),
                success("1.3.0"),
                missing("kimi"),
            ],
        );
        health
            .refresh(tokio_util::sync::CancellationToken::new())
            .await
            .expect("refresh health");
        let state = state(
            root.path(),
            health,
            ScriptedRunner::new(vec![missing("codegraph")], Duration::ZERO),
        );
        // Kimi 的版本来源缓存（latest 2.5.2）——版本判定仍生效。
        let checked_at = (Utc::now() - TimeDelta::hours(1)).to_rfc3339();
        seed_version_cache(
            root.path(),
            json!({
                "@moonshot-ai/kimi-code": {
                    "package": "@moonshot-ai/kimi-code",
                    "latest_version": "2.5.2",
                    "checked_at": checked_at
                }
            }),
        );

        let response = get_install_targets(State(state)).await;
        let payload = response_json(response.into_response()).await;
        let targets = payload["targets"].as_array().expect("targets array");
        let kimi = targets
            .iter()
            .find(|dto| dto["target"] == json!("kimi_code"))
            .expect("kimi target");

        assert_eq!(kimi["cli_available"], json!(false));
        assert_eq!(kimi["local_version"], Value::Null);
        assert_eq!(kimi["latest_version"], json!("2.5.2"));
        assert_eq!(
            kimi["action"],
            json!("install"),
            "命令缺失时必须给【安装】，不能给【更新】"
        );
    }

    // ---- POST /install + GET /install/{job_id} ----

    /// 测试点 #8：POST 立即返回 {job_id}（不等待）；GET 轮询得终态。
    #[tokio::test]
    async fn install_target_post_returns_immediately_and_polls_to_completed() {
        let root = tempdir().expect("root");
        let health = health_service(
            root.path(),
            vec![
                success("1.0.0"),
                success("2.1.7"),
                success("0.85.0"),
                success("0.34.0"),
            ],
        );
        // 安装命令延迟 300ms，保证 POST 返回时作业仍是 running。
        let state = state(
            root.path(),
            health.clone(),
            ScriptedRunner::new(vec![install_success()], Duration::from_millis(300)),
        );

        let response = post_provider_install(
            State(state.clone()),
            axum::extract::Path("codex".to_string()),
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let started = response_json(response).await;
        let job_id = started["job_id"].as_str().expect("job_id").to_string();
        assert!(!job_id.is_empty());

        // 立即查询：仍是 running（安装尚未完成）。
        let running = response_json(
            get_install_job(State(state.clone()), axum::extract::Path(job_id.clone())).await,
        )
        .await;
        assert_eq!(running["status"], json!("running"));
        assert_eq!(running["target"], json!("codex"));

        // 轮询到终态：completed + 装完重探得到的版本。
        let done = poll_job_terminal(&state, &job_id).await;
        assert_eq!(done["status"], json!("completed"), "{done}");
        assert_eq!(done["outcome"]["installed_version"], json!("2.1.7"));
        // 重探确实发生（REQ-PCI-05 装完重新探测）。
        assert!(state.provider_health.latest_diagnostic().generation >= 1);
    }

    /// 测试点 #8：Kimi 缺 acknowledge_remote_script → 400（防绕过告知）。
    #[tokio::test]
    async fn install_target_post_rejects_kimi_without_acknowledgement() {
        let root = tempdir().expect("root");
        let health = health_service(
            root.path(),
            vec![
                success("1.0.0"),
                success("2.0.0"),
                success("0.85.0"),
                success("0.34.0"),
            ],
        );
        let state = state(
            root.path(),
            health,
            ScriptedRunner::new(vec![install_success()], Duration::ZERO),
        );

        for body in [None, Some(json!(false)), Some(json!(null))] {
            let request_body = body.as_ref().map(|ack| {
                Json(super::InstallRequestBody {
                    acknowledge_remote_script: ack.as_bool(),
                })
            });
            let response = post_provider_install(
                State(state.clone()),
                axum::extract::Path("kimi_code".to_string()),
                request_body,
            )
            .await;
            assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{body:?}");
            let payload = response_json(response).await;
            assert_eq!(payload["code"], json!("install_acknowledgement_required"));
            assert!(
                payload["message"]
                    .as_str()
                    .is_some_and(|message| message.contains("code.kimi.com")),
                "必须告知脚本来源: {payload}"
            );
        }

        // 未确认时不留任何作业痕迹。
        let jobs_dir = root.path().join(".aria/state/provider-installation/jobs");
        assert!(
            !jobs_dir.exists() || std::fs::read_dir(&jobs_dir).expect("jobs dir").count() == 0,
            "拒绝请求不得创建作业"
        );

        // 确认后放行。
        let response = post_provider_install(
            State(state.clone()),
            axum::extract::Path("kimi_code".to_string()),
            Some(Json(super::InstallRequestBody {
                acknowledge_remote_script: Some(true),
            })),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let started = response_json(response).await;
        let job_id = started["job_id"].as_str().expect("job_id").to_string();
        let done = poll_job_terminal(&state, &job_id).await;
        assert_eq!(done["status"], json!("completed"), "{done}");
    }

    #[tokio::test]
    async fn install_target_post_unknown_target_is_rejected() {
        let root = tempdir().expect("root");
        let health = health_service(
            root.path(),
            vec![
                success("1.0.0"),
                success("2.0.0"),
                success("0.85.0"),
                success("0.34.0"),
            ],
        );
        let state = state(
            root.path(),
            health,
            ScriptedRunner::new(vec![], Duration::ZERO),
        );
        let response =
            post_provider_install(State(state), axum::extract::Path("fake".to_string()), None)
                .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let payload = response_json(response).await;
        assert_eq!(payload["code"], json!("install_target_unknown"));
    }

    #[tokio::test]
    async fn install_job_unknown_id_is_not_found() {
        let root = tempdir().expect("root");
        let health = health_service(
            root.path(),
            vec![
                success("1.0.0"),
                success("2.0.0"),
                success("0.85.0"),
                success("0.34.0"),
            ],
        );
        let state = state(
            root.path(),
            health,
            ScriptedRunner::new(vec![], Duration::ZERO),
        );
        let response =
            get_install_job(State(state), axum::extract::Path("no-such-job".to_string())).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let payload = response_json(response).await;
        assert_eq!(payload["code"], json!("install_job_not_found"));
    }

    /// 测试点 #9：安装成功 → Completed，未写任何 capability 记录、
    /// 未触发核验探针（spy 计数 0）；重探走 provider health。
    #[tokio::test]
    async fn install_success_does_not_write_capability_or_trigger_probes() {
        use crate::cross_cutting::provider_boundary::ProviderBoundaryError;
        use crate::product::logical_codebase::provider_capability_revalidate::{
            ProbeInvocation, ProbeRunner, ProviderCapabilityRevalidateService, VersionSource,
        };

        let root = tempdir().expect("root");
        let health = health_service(
            root.path(),
            vec![
                success("1.0.0"),
                success("2.1.7"),
                success("0.85.0"),
                success("0.34.0"),
            ],
        );
        let probe_calls = Arc::new(AtomicUsize::new(0));
        let calls_for_probe = probe_calls.clone();
        let probe_runner: ProbeRunner = Arc::new(move |_invocation: ProbeInvocation| {
            let calls = calls_for_probe.clone();
            Box::pin(async move {
                calls.fetch_add(1, Ordering::SeqCst);
                Err(ProviderBoundaryError::ProbeFailed(
                    "capability probe must not run during install".to_string(),
                ))
            })
        });
        let version_source: VersionSource = Arc::new(|_program: &str| Ok("1.0.0".to_string()));
        let capability_service = Arc::new(
            ProviderCapabilityRevalidateService::new(
                crate::product::app_paths::ProductAppPaths::new(root.path().join(".aria")),
            )
            .with_version_source(version_source)
            .with_probe_runner(probe_runner),
        );

        let gate = Arc::new(ProviderAvailabilityGate::new(health.clone()));
        let state = WebAppState::new(
            root.path().to_path_buf(),
            WebRuntime::new_fake(root.path().to_path_buf()),
        )
        .with_provider_health(
            health.clone(),
            gate,
            ScriptedRunner::new(vec![install_success()], Duration::ZERO),
        )
        .with_capability_revalidate_service(capability_service);

        let response = post_provider_install(
            State(state.clone()),
            axum::extract::Path("codex".to_string()),
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let started = response_json(response).await;
        let job_id = started["job_id"].as_str().expect("job_id").to_string();

        let done = poll_job_terminal(&state, &job_id).await;
        assert_eq!(done["status"], json!("completed"), "{done}");

        // 未触发任何能力核验探针（安装 ≠ 核验，REQ-PCI-05）。
        assert_eq!(
            probe_calls.load(Ordering::SeqCst),
            0,
            "安装成功不得自动触发核验探针"
        );

        // 未写任何 capability 记录（durable 工件扫描）。
        fn assert_no_capability_artifacts(directory: &Path) {
            let entries = std::fs::read_dir(directory).expect("read dir");
            for entry in entries {
                let entry = entry.expect("dir entry");
                let name = entry.file_name().to_string_lossy().into_owned();
                assert!(
                    !name.to_lowercase().contains("capability"),
                    "安装流程不得写 capability 记录: {}",
                    entry.path().display()
                );
                if entry.path().is_dir() {
                    assert_no_capability_artifacts(&entry.path());
                }
            }
        }
        let aria_root = root.path().join(".aria");
        if aria_root.exists() {
            assert_no_capability_artifacts(&aria_root);
        }

        // 装完确实重探了（provider health generation 前进）。
        assert!(state.provider_health.latest_diagnostic().generation >= 1);
    }
}
