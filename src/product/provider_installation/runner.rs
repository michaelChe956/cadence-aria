//! 安装执行器（REQ-PCI-03/05 / tasks 11.2、11.6）。
//!
//! - **一个包一次安装**：API 只接受单个 `InstallTarget`，无批量入口
//!   （批量由块3 编排层循环调用）。
//! - npm 形态固定：`install -g --prefix <prefix> --omit=optional
//!   --no-audit --no-fund <pkg>[@version]`，环境带 `npm_config_registry`
//!   （默认阿里镜像，`NPM_REGISTRY` 可覆盖）；`<prefix>` 避开 sudo。
//! - Kimi 走官方脚本（`curl -fsSL … | bash`）。
//! - 成功后经 `PostInstallProber` 重新探测（`provider_health.refresh`）；
//!   **安装 ≠ 核验**：不写 capability、不触发核验探针。
//! - 失败经 `failure_classify` 分类落 job。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::bounded_command_runner::{
    BoundedCommandError, BoundedCommandRequest, BoundedCommandResult, BoundedCommandRunner,
};
use crate::cross_cutting::provider_health::ProviderHealthService;

use super::InstallTarget;
use super::effective_prefix::{
    choose_effective_prefix, first_path_dir_with_command, is_writable_dir, prefix_bin_is_on_path,
    same_dir, visible_cli_prefix,
};
use super::failure_classify::{
    InstallFailureKind, InstallFailureSignals, InstallJobFailure, classify_install_failure,
};
use super::job_store::{InstallJobOutcome, InstallJobStore};
use super::source_table::{
    InstallSourceRow, manual_install_command, npm_prefix, npm_registry, row_for_wire,
};

/// 安装命令默认超时：npm 全局安装可能要几分钟（设计决策 9 未验证项 4）。
pub const DEFAULT_INSTALL_TIMEOUT: Duration = Duration::from_secs(600);
/// 安装输出上限（stdout/stderr 各自）。
pub const INSTALL_OUTPUT_LIMIT: usize = 64 * 1024;

/// 宿主环境透传基线（计划覆盖层叠在其上；与 `provider_health` 的
/// probe 环境构造同口径）。
fn host_environment() -> BTreeMap<String, String> {
    let mut environment = BTreeMap::new();
    if let Some(path) = std::env::var_os("PATH") {
        environment.insert("PATH".to_string(), path.to_string_lossy().into_owned());
    }
    environment
}

/// 一次安装的完整命令计划（纯数据，可单测）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallCommandPlan {
    pub executable: String,
    pub argv: Vec<String>,
    /// 环境覆盖层（叠加在宿主环境之上）：npm 带 `npm_config_registry`。
    pub environment: BTreeMap<String, String>,
    pub manual_command: String,
}

/// npm 安装计划（REQ-PCI-03）：一个包一次安装，形态固定。
pub fn npm_install_plan(
    row: &InstallSourceRow,
    version: Option<&str>,
    registry: &str,
    prefix: &Path,
) -> InstallCommandPlan {
    let package = row.npm_package.expect("npm 行必有包名");
    let spec = match version {
        Some(version) if !version.trim().is_empty() => format!("{package}@{version}"),
        _ => format!("{package}@latest"),
    };
    let mut environment = BTreeMap::new();
    environment.insert("npm_config_registry".to_string(), registry.to_string());
    InstallCommandPlan {
        executable: "npm".to_string(),
        argv: vec![
            "install".to_string(),
            "-g".to_string(),
            "--prefix".to_string(),
            prefix.to_string_lossy().into_owned(),
            "--omit=optional".to_string(),
            "--no-audit".to_string(),
            "--no-fund".to_string(),
            spec,
        ],
        environment,
        manual_command: manual_install_command(row, registry),
    }
}

/// Kimi 官方脚本安装计划（非 npm；不接受版本锁定）。
pub fn kimi_install_plan(row: &InstallSourceRow, registry: &str) -> InstallCommandPlan {
    let script = row.install_script.expect("Kimi 行必有官方脚本地址");
    InstallCommandPlan {
        executable: "bash".to_string(),
        argv: vec!["-c".to_string(), format!("curl -fsSL {script} | bash")],
        environment: BTreeMap::new(),
        manual_command: manual_install_command(row, registry),
    }
}

/// 按目标派发安装计划；脚本安装源（Kimi）不走 npm 形态。
pub fn install_plan_for(
    target: &InstallTarget,
    version: Option<&str>,
    registry: &str,
    prefix: &Path,
) -> Option<InstallCommandPlan> {
    let row = row_for_wire(target.wire_label())?;
    Some(match row.install_script {
        Some(_) => kimi_install_plan(row, registry),
        None => npm_install_plan(row, version, registry, prefix),
    })
}

/// 跑安装命令的执行器（包装 `BoundedCommandRunner`）。
pub struct ProviderInstallRunner {
    command_runner: Arc<dyn BoundedCommandRunner>,
    working_dir: PathBuf,
    timeout: Duration,
    output_limit: usize,
}

impl ProviderInstallRunner {
    pub fn new(
        command_runner: Arc<dyn BoundedCommandRunner>,
        working_dir: impl Into<PathBuf>,
        timeout: Duration,
    ) -> Self {
        Self {
            command_runner,
            working_dir: working_dir.into(),
            timeout,
            output_limit: INSTALL_OUTPUT_LIMIT,
        }
    }

    /// 计划 → 有界命令请求（宿主 PATH 透传 + 计划覆盖层）。
    pub fn request_for(&self, plan: &InstallCommandPlan) -> BoundedCommandRequest {
        let mut environment = host_environment();
        for (key, value) in &plan.environment {
            environment.insert(key.clone(), value.clone());
        }
        BoundedCommandRequest {
            executable: plan.executable.clone(),
            argv: plan.argv.clone(),
            working_dir: self.working_dir.clone(),
            timeout: self.timeout,
            cancellation: CancellationToken::new(),
            environment,
            stdout_limit: self.output_limit,
            stderr_limit: self.output_limit,
        }
    }

    pub async fn execute(
        &self,
        plan: &InstallCommandPlan,
    ) -> Result<BoundedCommandResult, BoundedCommandError> {
        let request = self.request_for(plan);
        self.command_runner.run(request).await
    }
}

/// 装完重探 seam（REQ-PCI-05）：成功安装后重新探测该对象的版本与可用性。
#[async_trait::async_trait]
pub trait PostInstallProber: Send + Sync {
    async fn refresh_after_install(&self, target: InstallTarget) -> Result<Option<String>, String>;
}

/// 生产实现：provider → `provider_health.refresh` 后回填版本；
/// codegraph 无 health 条目，用同一 runner 跑一次 `--version`。
pub struct HealthRefreshProber {
    health: Arc<ProviderHealthService>,
    command_runner: Arc<dyn BoundedCommandRunner>,
    working_dir: PathBuf,
    timeout: Duration,
}

impl HealthRefreshProber {
    pub fn new(
        health: Arc<ProviderHealthService>,
        command_runner: Arc<dyn BoundedCommandRunner>,
        working_dir: impl Into<PathBuf>,
        timeout: Duration,
    ) -> Self {
        Self {
            health,
            command_runner,
            working_dir: working_dir.into(),
            timeout,
        }
    }
}

#[async_trait::async_trait]
impl PostInstallProber for HealthRefreshProber {
    async fn refresh_after_install(&self, target: InstallTarget) -> Result<Option<String>, String> {
        self.health
            .refresh(CancellationToken::new())
            .await
            .map_err(|error| error.to_string())?;
        match target {
            InstallTarget::Provider(provider) => Ok(self
                .health
                .latest_diagnostic()
                .entry(&provider)
                .and_then(|entry| entry.version.clone())),
            InstallTarget::CodeGraph => {
                let environment = host_environment();
                let request = BoundedCommandRequest {
                    executable: "codegraph".to_string(),
                    argv: vec!["--version".to_string()],
                    working_dir: self.working_dir.clone(),
                    timeout: self.timeout,
                    cancellation: CancellationToken::new(),
                    environment,
                    stdout_limit: 4096,
                    stderr_limit: 4096,
                };
                match self.command_runner.run(request).await {
                    Ok(result) if result.exit_code == Some(0) && !result.timed_out => Ok(result
                        .stdout
                        .lines()
                        .map(str::trim)
                        .find(|line| !line.is_empty())
                        .map(str::to_string)),
                    _ => Ok(None),
                }
            }
        }
    }
}

/// npm 装完命令不在 PATH 上（或 PATH 上生效的仍是别处的旧版本）时的提示
/// （REQ-PCI-03：`--prefix` 前缀的代价由提示兜底）。
///
/// 判定口径：PATH 上**第一个**提供该命令的目录必须就是本次安装前缀的
/// `bin` 目录 —— 否则用户执行的仍是旧版本，必须提示把新前缀放到 PATH
/// 前面（2026-11-09 bug：更新装到 PATH 之外 ⇒ 版本不变且无任何提示）。
pub fn npm_path_hint(command: &str, prefix: &Path, path_value: Option<&str>) -> Option<String> {
    let prefix_bin = prefix.join("bin");
    match first_path_dir_with_command(command, path_value) {
        // PATH 上生效的命令就是本次安装的那个 ⇒ 无需提示。
        Some(dir) if same_dir(&dir, &prefix_bin) => None,
        // PATH 上生效的是别处的旧版本 ⇒ 必须提示换前缀，否则「更新没效果」。
        Some(dir) => Some(format!(
            "已安装到 {}；PATH 上生效的 `{command}` 来自 {}，请把 {} 放到 PATH 前面",
            prefix.display(),
            dir.display(),
            prefix_bin.display()
        )),
        None => Some(format!(
            "已安装到 {}；若 `{command}` 不在 PATH 上，请把 {} 加入 PATH 后重试",
            prefix.display(),
            prefix_bin.display()
        )),
    }
}

/// 作业执行时钟 seam。
pub trait InstallClock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

/// 系统时钟。
pub struct SystemInstallClock;

impl InstallClock for SystemInstallClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// npm 自身全局前缀（`npm prefix -g`）的探测 seam。
///
/// 只在「PATH 上没有该命令」或「那个前缀不可写」时才需要它（CLI 未安装
/// 时的新装落点）⇒ 生产实现是可选的，测试可注入桩。
#[async_trait::async_trait]
pub trait NpmGlobalPrefixProbe: Send + Sync {
    async fn npm_global_prefix(&self) -> Option<PathBuf>;
}

/// 生产实现：跑一次 `npm prefix -g`（宿主 PATH 透传，短超时）。
pub struct CommandNpmGlobalPrefixProbe {
    command_runner: Arc<dyn BoundedCommandRunner>,
    working_dir: PathBuf,
    timeout: Duration,
}

impl CommandNpmGlobalPrefixProbe {
    pub fn new(
        command_runner: Arc<dyn BoundedCommandRunner>,
        working_dir: impl Into<PathBuf>,
        timeout: Duration,
    ) -> Self {
        Self {
            command_runner,
            working_dir: working_dir.into(),
            timeout,
        }
    }
}

#[async_trait::async_trait]
impl NpmGlobalPrefixProbe for CommandNpmGlobalPrefixProbe {
    async fn npm_global_prefix(&self) -> Option<PathBuf> {
        let request = BoundedCommandRequest {
            executable: "npm".to_string(),
            argv: vec!["prefix".to_string(), "-g".to_string()],
            working_dir: self.working_dir.clone(),
            timeout: self.timeout,
            cancellation: CancellationToken::new(),
            environment: host_environment(),
            stdout_limit: 4096,
            stderr_limit: 4096,
        };
        let result = self.command_runner.run(request).await.ok()?;
        if result.exit_code != Some(0) || result.timed_out {
            return None;
        }
        let line = result
            .stdout
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())?;
        let path = PathBuf::from(line);
        path.exists().then_some(path)
    }
}

/// durable 作业执行器：running（create 时已落盘）→ 执行安装 → 成功重探
/// → completed / 失败分类 → failed。
pub struct InstallJobExecution {
    pub store: InstallJobStore,
    pub runner: ProviderInstallRunner,
    pub prober: Arc<dyn PostInstallProber>,
    pub registry: String,
    pub npm_prefix: PathBuf,
    /// 宿主 PATH（`None` = 运行时读进程环境）：决定「PATH 上当前生效的
    /// CLI 在哪个前缀」，安装 / 更新必须落到那个前缀才真的生效。
    pub path_value: Option<String>,
    /// npm 自身全局前缀的探测（可选；`None` = 不探测）。
    pub npm_prefix_probe: Option<Arc<dyn NpmGlobalPrefixProbe>>,
    pub clock: Arc<dyn InstallClock>,
}

impl InstallJobExecution {
    /// 生产装配：镜像/前缀取环境解析值，时钟取系统时钟。
    pub fn production(
        store: InstallJobStore,
        command_runner: Arc<dyn BoundedCommandRunner>,
        working_dir: impl Into<PathBuf>,
        health: Arc<ProviderHealthService>,
    ) -> Self {
        let working_dir = working_dir.into();
        Self {
            store,
            runner: ProviderInstallRunner::new(
                command_runner.clone(),
                working_dir.clone(),
                DEFAULT_INSTALL_TIMEOUT,
            ),
            prober: Arc::new(HealthRefreshProber::new(
                health,
                command_runner.clone(),
                working_dir.clone(),
                Duration::from_secs(5),
            )),
            registry: npm_registry(),
            npm_prefix: npm_prefix(),
            path_value: None,
            npm_prefix_probe: Some(Arc::new(CommandNpmGlobalPrefixProbe::new(
                command_runner,
                working_dir,
                Duration::from_secs(10),
            ))),
            clock: Arc::new(SystemInstallClock),
        }
    }

    /// 宿主 PATH（测试可注入；生产 = 进程环境）。
    fn host_path(&self) -> Option<String> {
        self.path_value
            .clone()
            .or_else(|| std::env::var("PATH").ok())
    }

    /// 解析生效的 npm 前缀（安装 / 更新落到这里才真的生效）。
    ///
    /// 判定链（写死）：
    /// ① **visible** —— PATH 上当前生效的 CLI 所在前缀，且**真实可写**；
    /// ② **`npm prefix -g`** —— **真实可写** 且 **`<prefix>/bin ∈ PATH`**
    ///    （装到 PATH 之外 ⇒ 装完探测仍失败 ⇒ 等于没装）；
    /// ③ **fallback** —— `NPM_PREFIX` / `$HOME/.npm-global`。
    pub async fn resolve_effective_prefix(
        &self,
        row: &InstallSourceRow,
        path_value: Option<&str>,
    ) -> PathBuf {
        let visible = visible_cli_prefix(row.command, path_value);
        // ② `npm prefix -g`：真实可写 **且** `<prefix>/bin ∈ PATH`。
        let npm_global = match self.npm_prefix_probe.as_ref() {
            Some(probe) => probe.npm_global_prefix().await,
            None => None,
        };
        let npm_global = npm_global.filter(|prefix| {
            is_writable_dir(prefix) && prefix_bin_is_on_path(prefix, path_value)
        });
        // ① visible（真实可写）→ ② npm 全局前缀 → ③ 兜底。
        choose_effective_prefix(visible.as_deref(), npm_global.as_deref(), &self.npm_prefix)
    }

    /// 单个目标的安装作业主流程（无批量入口）：执行安装 → 成功重探 +
    /// 落 completed / 失败分类落 failed。**安装 ≠ 核验**：重探只走
    /// provider health，不写 capability、不触发核验探针。
    pub async fn run(&self, job_id: &str, target: InstallTarget) {
        let Some(row) = row_for_wire(target.wire_label()) else {
            let failure = InstallJobFailure {
                kind: InstallFailureKind::Other,
                message: format!("未知安装对象：{}", target.wire_label()),
                manual_command: String::new(),
            };
            let _ = self.store.fail(job_id, failure, self.clock.now());
            return;
        };
        let path_value = self.host_path();
        // 安装 / 更新必须落到「PATH 上当前生效的 CLI 所在前缀」——
        // 装到别处（哪怕可写）⇒ 用户执行的仍是旧版本（2026-11-09 bug）。
        let prefix = match row.npm_package {
            Some(_) => {
                self.resolve_effective_prefix(row, path_value.as_deref())
                    .await
            }
            // 官方脚本安装源（Kimi）：前缀无关。
            None => self.npm_prefix.clone(),
        };
        let Some(plan) = install_plan_for(&target, None, &self.registry, &prefix) else {
            // row_for_wire 已放行 ⇒ 这里必然可达，防御性兜底。
            return;
        };
        match self.runner.execute(&plan).await {
            Ok(result) if result.exit_code == Some(0) && !result.timed_out && !result.cancelled => {
                let detected = self
                    .prober
                    .refresh_after_install(target)
                    .await
                    .ok()
                    .flatten();
                let path_hint = match row.npm_package {
                    Some(_) => npm_path_hint(row.command, &prefix, path_value.as_deref()),
                    None => None,
                };
                let outcome = InstallJobOutcome {
                    installed_version: detected,
                    path_hint,
                };
                let _ = self.store.complete(job_id, outcome, self.clock.now());
            }
            Ok(result) => {
                let signals = InstallFailureSignals {
                    stderr: result.stderr,
                    stdout: result.stdout,
                    timed_out: result.timed_out,
                    command_missing: None,
                };
                let failure = classify_install_failure(row, &signals, &self.registry);
                let _ = self.store.fail(job_id, failure, self.clock.now());
            }
            Err(BoundedCommandError::CommandMissing {
                executable,
                details,
            }) => {
                let signals = InstallFailureSignals {
                    stderr: details,
                    command_missing: Some(executable),
                    ..InstallFailureSignals::default()
                };
                let failure = classify_install_failure(row, &signals, &self.registry);
                let _ = self.store.fail(job_id, failure, self.clock.now());
            }
            Err(BoundedCommandError::Io { details }) => {
                let signals = InstallFailureSignals {
                    stderr: details,
                    ..InstallFailureSignals::default()
                };
                let failure = classify_install_failure(row, &signals, &self.registry);
                let _ = self.store.fail(job_id, failure, self.clock.now());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    include!("runner_overlay_tests.inc.rs");
}
