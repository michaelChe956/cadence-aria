//! provider / codegraph 安装更新的 HTTP 面（块2 片7A / REQ-PCI-01~06）。
//!
//! - `GET  /api/providers/install-targets`：五类对象一次返回（独立路径，
//!   避免与 `{job_id}` 路由冲突）。
//! - `POST /api/providers/{target}/install`：立即返回 `{job_id}`（异步
//!   作业，不阻塞请求）；Kimi 缺 `acknowledge_remote_script=true` → 400
//!   （REQ-PCI-01：远程脚本必须先告知来源，防绕过）。
//! - `GET  /api/providers/install/{job_id}`：轮询作业状态与结果。
//! - `POST /api/providers/{target}/expose`：把已安装的 CLI 软链到登录
//!   shell PATH 上的可写目录（§五 2.11；用户明确点击才动作，不写 shell
//!   profile、不改 PATH 环境变量；安装与更新共用，任意已装版本可点）。
//!
//! 不实现 version-cache 查询端点：24h 缓存是 `registry_version` 的内部
//! 实现细节（tasks.md 82 条契约不含该端点）。

use std::path::PathBuf;
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
use crate::cross_cutting::path_overlay;
use crate::cross_cutting::provider_health::ProviderHealthReasonCode;
use crate::cross_cutting::semver::compare_semver;
use crate::product::app_paths::ProductAppPaths;
use crate::product::provider_installation::InstallTarget;
use crate::product::provider_installation::effective_prefix::same_dir;
use crate::product::provider_installation::expose::expose_to_terminal_path;
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
    /// 已安装但所在目录不在登录 shell PATH 上（REQ-PCI-08；仅提示，指向
    /// 既有「暴露到终端 PATH」入口，未安装恒 false）。
    pub off_path: bool,
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

/// 装配安装作业执行器。
///
/// 生产：宿主环境解析（PATH / HOME / npm 全局前缀）——`InstallJobExecution::production`。
/// 测试：可注入**隔离执行器**（临时 npm 前缀 + 空 PATH + 临时 HOME）——
/// 否则宿主真装了 codex 时，安装会落到**宿主**前缀、装后版本直接读宿主真实
/// 包的 `package.json`，用例断言随开发机状态漂移（见 tests 模块的
/// `IsolatedInstallEnv`）。
fn install_job_execution(store: InstallJobStore, state: &WebAppState) -> InstallJobExecution {
    #[cfg(test)]
    {
        if let Some(builder) = test_install_execution_builder(state) {
            return builder(store, state);
        }
    }
    InstallJobExecution::production(
        store,
        state.command_runner.clone(),
        state.workspace_root.clone(),
        state.provider_health.clone(),
    )
}

/// 测试用执行器装配器（按 workspace_root 命中 ⇒ 并行用例互不干扰）。
#[cfg(test)]
type TestInstallExecutionBuilder =
    Arc<dyn Fn(InstallJobStore, &WebAppState) -> InstallJobExecution + Send + Sync>;

/// 测试登记的隔离执行器（workspace_root → 装配器）。
#[cfg(test)]
static TEST_INSTALL_EXECUTIONS: std::sync::Mutex<
    Vec<(std::path::PathBuf, TestInstallExecutionBuilder)>,
> = std::sync::Mutex::new(Vec::new());

/// 取该 state 登记的隔离执行器装配器（未登记 ⇒ `None` = 走生产装配）。
#[cfg(test)]
fn test_install_execution_builder(state: &WebAppState) -> Option<TestInstallExecutionBuilder> {
    let registry = TEST_INSTALL_EXECUTIONS.lock().ok()?;
    registry
        .iter()
        .find(|(root, _)| *root == state.workspace_root)
        .map(|(_, builder)| builder.clone())
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

/// off_path 判定查询（REQ-PCI-08；测试 seam：搜索目录与登录 PATH 注入）。
#[derive(Debug, Clone)]
struct OffPathQuery {
    /// 已安装二进制的搜索目录（与 expose 生产查询同口径：overlay 登记目录
    /// → 登录 shell PATH → 已知落点）。
    install_search_dirs: Vec<PathBuf>,
    /// 登录 shell PATH 的目录列表（PATH 顺序）；空 = 探测不可用。
    login_path_dirs: Vec<PathBuf>,
}

/// 生产查询构建：**服务进程 PATH**（`effective_path` 合并后的真实生效序）
/// → 登录 shell PATH 快照 → `~/.npm-global/bin` → `/usr/local/bin`（与
/// `expose` 的生产查询同口径；本面**只读**——不建链、不做可写探测）。
fn off_path_query() -> OffPathQuery {
    let service_path = path_overlay::effective_path(None);
    let login_path = path_overlay::login_shell_path();
    let home = std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    off_path_query_with(&service_path, login_path.as_deref(), home.as_deref())
}

/// [`off_path_query`] 的 seam（服务进程 PATH / 登录 shell PATH / HOME 注入）。
fn off_path_query_with(
    service_path: &str,
    login_path: Option<&str>,
    home: Option<&std::path::Path>,
) -> OffPathQuery {
    let login_path_dirs: Vec<PathBuf> = login_path.map(split_login_path_dirs).unwrap_or_default();
    // 平台「能找到」的口径 = 服务进程 PATH（overlay 合并后的真实生效序）
    // ∪ 登录 shell PATH ∪ 常见落点；缺了服务进程 PATH 就会把「服务自己能
    // 跑、但用户终端 PATH 没有」的 CLI 误判成「未安装 ⇒ off_path=false」。
    let mut install_search_dirs = split_login_path_dirs(service_path);
    extend_unique_dirs(&mut install_search_dirs, login_path_dirs.iter().cloned());
    if let Some(home) = home {
        extend_unique_dirs(
            &mut install_search_dirs,
            std::iter::once(home.join(".npm-global/bin")),
        );
    }
    extend_unique_dirs(
        &mut install_search_dirs,
        std::iter::once(PathBuf::from("/usr/local/bin")),
    );
    OffPathQuery {
        install_search_dirs,
        login_path_dirs,
    }
}

/// 登录 shell PATH 原始串 → 目录列表（保序、去空段；与 `expose` 侧私有
/// `split_path_dirs` 同口径的对齐实现）。
fn split_login_path_dirs(value: &str) -> Vec<PathBuf> {
    std::env::split_paths(value)
        .filter(|dir| !dir.as_os_str().is_empty())
        .collect()
}

/// 去重追加（首现优先；与 `expose` 侧私有 `extend_unique` 同口径）。
fn extend_unique_dirs(target: &mut Vec<PathBuf>, items: impl IntoIterator<Item = PathBuf>) {
    for item in items {
        if !target.contains(&item) {
            target.push(item);
        }
    }
}

/// 判定 off_path（REQ-PCI-08）：已安装（搜索目录里找得到二进制）但其所在
/// 目录**不在登录 shell PATH** 上 ⇒ 用户在自己终端里可能敲不出来。
///
/// 只做提示，不改变安装/更新动作语义；未安装或登录 shell PATH 快照不可用
/// （服务启动 ≤2s / 探测失败）⇒ `false`（宁缺毋滥，不给误导性徽标）。
fn resolve_off_path(command: &str, query: &OffPathQuery) -> bool {
    // 未安装（搜索目录里找不到二进制）⇒ 无从谈 PATH（断链不算已安装，
    // 与 expose::find_installed 同口径）。
    let Some(installed) = query
        .install_search_dirs
        .iter()
        .map(|dir| dir.join(command))
        .find(|path| path.exists())
    else {
        return false;
    };
    if query.login_path_dirs.is_empty() {
        return false;
    }
    let installed_dir = installed.parent().unwrap_or(installed.as_path());
    // 目录成员语义（规范化比较）：登录 shell PATH 任一目录命中即「在 PATH 上」。
    !query
        .login_path_dirs
        .iter()
        .any(|login_dir| same_dir(login_dir, installed_dir))
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
    // off_path 判定（REQ-PCI-08）：登录 shell PATH 快照 + overlay 登记目录
    // 一次构建，五类对象共用（只读，不建链）。
    let off_path_probe = off_path_query();

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
            off_path: cli_available && resolve_off_path(row.command, &off_path_probe),
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
    let execution = install_job_execution(store, &state);
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

#[derive(Debug, Serialize)]
pub struct ExposeResponse {
    pub exposed: bool,
    /// 可直接运行的命令名（如 `claude`）。
    pub command: String,
    /// 软链所在目录（登录 shell PATH 上、真实可写）。
    pub dir: String,
    /// 软链指向的已安装二进制路径。
    pub target: String,
}

/// 把已安装的 CLI 暴露到用户终端 PATH（§五 2.11）：只在登录 shell PATH
/// 上真实可写的目录建**单个软链**；失败如实返回可行动原因（候选目录与
/// 各自不合格原因），不静默、不降级。
pub async fn post_provider_expose(Path(target_wire): Path<String>) -> Response {
    // 未知对象（含 fake）→ 400（与 install 端点同口径）。
    if InstallTarget::parse_wire(&target_wire).is_none() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "install_target_unknown",
            format!("未知安装对象：{target_wire}"),
        );
    }
    match expose_to_terminal_path(&target_wire) {
        Ok(outcome) => (
            StatusCode::OK,
            Json(ExposeResponse {
                exposed: true,
                command: outcome.command,
                dir: outcome.dir.display().to_string(),
                target: outcome.target.display().to_string(),
            }),
        )
            .into_response(),
        Err(error) => error_response(StatusCode::BAD_REQUEST, error.code, error.message),
    }
}

#[cfg(test)]
mod tests {
    // 测试体拆分片段（large_file_guard 1200 行红线，纯移动；夹具与断言逐字保留）。
    include!("provider_installation_tests.inc.rs");
}
