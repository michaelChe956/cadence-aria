//! 安装执行器（REQ-PCI-03/05 / tasks 11.2、11.6）。
//!
//! - **一个包一次安装**：API 只接受单个 `InstallTarget`，无批量入口
//!   （批量由块3 编排层循环调用）。
//! - npm 形态固定：`install -g --prefix <prefix> --omit=optional
//!   --no-audit --no-fund --allow-scripts=<pkg> <pkg>[@version]`，环境带
//!   `npm_config_registry`（默认阿里镜像，`NPM_REGISTRY` 可覆盖）；
//!   `<prefix>` 避开 sudo。安装子进程的 PATH **前插**目标前缀的 bin。
//! - Kimi 走官方脚本（`curl -fsSL … | bash`）。
//! - 成功后把安装产物的 bin 目录登记进共享 PATH overlay（**仅当**目录里确实
//!   含刚装好的可执行文件）⇒ 平台自身探测立即可见；**之后**再经
//!   `PostInstallProber` 重新探测（`provider_health.refresh` + 绝对路径兜底 +
//!   npm 包 `package.json` 版本 + 一个真实命令）；**安装 ≠ 核验**：不写
//!   capability、不触发核验探针。
//! - 装完探测仍不过 ⇒ 以 `--include=optional` **重试一次**；仍失败 ⇒ 给出
//!   可复制的修复命令（失败经 `failure_classify` 分类落 job）。
//! - 更新语义（2.10）：前缀落点必须与被更新对象一致 —— 已安装且在 PATH 上
//!   ⇒ 装回同一前缀；已安装但不在 PATH 上 ⇒ 从已安装包的真实路径反推前缀；
//!   更新后版本未变 ⇒ 明确报「命令仍是旧版」，不得静默成功。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::bounded_command_runner::{
    BoundedCommandError, BoundedCommandRequest, BoundedCommandResult, BoundedCommandRunner,
};
use crate::cross_cutting::path_overlay;
use crate::cross_cutting::provider_health::{ProviderHealthService, version_token_from_output};

use super::InstallTarget;
use super::effective_prefix::{
    choose_effective_prefix, first_path_dir_with_command, is_writable_dir, npm_package_dir,
    npm_prefix_from_real_path, prefix_bin_is_on_path, same_dir, visible_cli_prefix,
};
use super::failure_classify::{
    InstallFailureKind, InstallFailureSignals, InstallJobFailure, classify_install_failure,
};
use super::job_store::{InstallJobOutcome, InstallJobStore};
use super::source_table::{
    InstallSourceRow, kimi_install_bin_dir, manual_install_command, npm_prefix, npm_registry,
    row_for_wire,
};

/// 安装命令默认超时：npm 全局安装可能要几分钟（设计决策 9 未验证项 4）。
pub const DEFAULT_INSTALL_TIMEOUT: Duration = Duration::from_secs(600);
/// 安装输出上限（stdout/stderr 各自）。
pub const INSTALL_OUTPUT_LIMIT: usize = 64 * 1024;

/// codegraph **真实命令**验证（`status --json`）的专用超时。
///
/// 理由：已初始化索引上 `status` 是**读写**打开（上游 1.6.2
/// `codegraph.ts:1094` 用 `CodeGraph.open(projectPath)`，无 `readOnly`），会
/// 走迁移 / WAL 自愈；3.1 GB 级索引上实测可能远超 5s。若沿用普通的 5s 探测
/// 时限，**健康**安装会被误判为「真实命令失败」⇒ 无谓的
/// `--include=optional` 重试并最终落 failed（假红灯）。故仅对该目标放宽，
/// 其它 provider 的探测口径不变（`--version` / `--help` 都是秒级）。
const CODEGRAPH_VERIFY_TIMEOUT: Duration = Duration::from_secs(30);

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
    /// 安装子进程 PATH **前插**的目录（目标前缀的 bin）——「装的 PATH ≠ 探测的
    /// PATH」从根上消除（magpie `updateEnv` 思路）。
    pub path_prefix: Option<PathBuf>,
    pub manual_command: String,
}

/// npm 安装计划（REQ-PCI-03）：一个包一次安装，形态固定。
///
/// `--omit=optional` 与既有契约一致；另加 `--allow-scripts=<pkg>`（npm ≥12
/// 不带它**不执行包的 install 脚本** —— `@anthropic-ai/claude-code` 有
/// `postinstall: node install.cjs`；npm 10/11 把它当未知配置忽略 ⇒ 兼容）。
pub fn npm_install_plan(
    row: &InstallSourceRow,
    version: Option<&str>,
    registry: &str,
    prefix: &Path,
) -> InstallCommandPlan {
    npm_plan_with_optional_flag(row, version, registry, prefix, "--omit=optional")
}

/// 重试计划（2.7③）：装完探测失败（装上但跑不起来）时用
/// `--include=optional` 再装一次 —— npm 会「留下缺 native 包的安装」却仍
/// exit 0（magpie 实测案例）。与 `--omit=optional` 互斥，故不同时出现。
pub fn npm_retry_plan(
    row: &InstallSourceRow,
    version: Option<&str>,
    registry: &str,
    prefix: &Path,
) -> Option<InstallCommandPlan> {
    row.npm_package?;
    Some(npm_plan_with_optional_flag(
        row,
        version,
        registry,
        prefix,
        "--include=optional",
    ))
}

/// npm 计划（`optional_flag` = `--omit=optional` / `--include=optional`）。
fn npm_plan_with_optional_flag(
    row: &InstallSourceRow,
    version: Option<&str>,
    registry: &str,
    prefix: &Path,
    optional_flag: &str,
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
            optional_flag.to_string(),
            "--no-audit".to_string(),
            "--no-fund".to_string(),
            format!("--allow-scripts={package}"),
            spec,
        ],
        environment,
        path_prefix: Some(prefix.join("bin")),
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
        path_prefix: None,
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
        // 安装子进程的 PATH **前插**目标前缀的 bin：装进去的命令当场就在
        // PATH 上（magpie `updateEnv` 思路：npm 的全局目录跟着跑它的那个 node）。
        if let Some(bin) = plan.path_prefix.as_ref() {
            let host_path = environment.get("PATH").cloned().unwrap_or_default();
            let mut entries = vec![bin.clone()];
            entries.extend(std::env::split_paths(&host_path));
            if let Ok(joined) = std::env::join_paths(entries) {
                environment.insert("PATH".to_string(), joined.to_string_lossy().into_owned());
            }
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

/// 装完核验结果（2.8 三重验证）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostInstallProbeOutcome {
    /// 装后探测到的版本（**只**回填 `installed_version`，不写 capability、
    /// 不触发核验探针 —— REQ-PCI-02「已装但探针未过」的语义不变）。
    pub installed_version: Option<String>,
    /// 真实命令是否跑通（`--version` 不可作为唯一依据：实测
    /// `codegraph --version` 在平台 bundle 缺失时仍返回 1.6.2 / exit 0）。
    pub real_command_ok: bool,
}

/// 装完重探 seam（REQ-PCI-05）：成功安装后重新探测该对象的版本与可用性。
#[async_trait::async_trait]
pub trait PostInstallProber: Send + Sync {
    /// `install_bin_dir` = 本次安装产物的 bin 目录（绝对路径兜底探测用：
    /// PATH 上找不到 CLI 时用 `<bin>/<cmd>` 再探一次）。
    async fn refresh_after_install(
        &self,
        target: InstallTarget,
        install_bin_dir: Option<&Path>,
    ) -> Result<PostInstallProbeOutcome, String>;

    /// 更新**前**的版本快照（REQ-PCI-10：更新前后必须**同源**比较）。
    ///
    /// npm 类由 `package.json` 提供（`npm_package_json_version`）⇒ 默认
    /// `None`；脚本类（Kimi）必须给出与装后同一来源的版本，否则「脚本
    /// exit 0 但版本没变」会静默判成功。
    async fn version_before_update(&self, _target: InstallTarget) -> Option<String> {
        None
    }
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

    /// 跑一次 `--version`（`absolute` = 用 `<bin>/<cmd>` 绝对路径兜底）。
    ///
    /// **必须经 `BoundedCommandRunner`**：否则 PATH 覆盖层（overlay）不生效，
    /// 装完的 CLI 在平台自身探测里仍是「找不到」。
    ///
    /// **返回口径 = provider health 的 `parse_version_token`**（首条非空行的
    /// 版本 token，stdout 优先、其次 stderr）：更新**前**的版本快照必须与装后
    /// 那一侧同源同口径 —— provider 的装后版本来自 `health.refresh` 的解析
    /// token（`2.6.1`），若装前返回原始行（`kimi 2.6.1`），两者永不相等 ⇒
    /// 「脚本 exit 0 但版本没变」会被静默判成功（REQ-PCI-10 复审 R10）。
    async fn probe_version(
        &self,
        command: &str,
        bin_dir: Option<&Path>,
        absolute: bool,
    ) -> Option<String> {
        let executable = match (absolute, bin_dir) {
            (true, Some(dir)) => dir.join(command).to_string_lossy().into_owned(),
            _ => command.to_string(),
        };
        let request = BoundedCommandRequest {
            executable,
            argv: vec!["--version".to_string()],
            working_dir: self.working_dir.clone(),
            timeout: self.timeout,
            cancellation: CancellationToken::new(),
            environment: host_environment(),
            stdout_limit: 4096,
            stderr_limit: 4096,
        };
        match self.command_runner.run(request).await {
            Ok(result) if result.exit_code == Some(0) && !result.timed_out => {
                version_token_from_output(&result.stdout, &result.stderr)
            }
            _ => None,
        }
    }

    /// 真实命令验证（2.8②）：跑一个**非 `--version`** 的子命令，证明 CLI
    /// 真能用（`--version` 在平台 bundle 缺失时也 exit 0 ⇒ 不可作唯一依据）。
    async fn probe_real_command(&self, target: InstallTarget, bin_dir: Option<&Path>) -> bool {
        let Some(row) = row_for_wire(target.wire_label()) else {
            return false;
        };
        let argv = post_install_verification_argv(target.wire_label());
        let timeout = match &target {
            // codegraph 的 `status --json` 在大索引上可能要几十秒
            // （详见 `CODEGRAPH_VERIFY_TIMEOUT`）：只放宽它的探测时限，
            // 其它 provider 保持构造时的口径。
            InstallTarget::CodeGraph => self.timeout.max(CODEGRAPH_VERIFY_TIMEOUT),
            InstallTarget::Provider(_) => self.timeout,
        };
        let mut executables = Vec::new();
        if let Some(dir) = bin_dir {
            executables.push(dir.join(row.command).to_string_lossy().into_owned());
        }
        executables.push(row.command.to_string());
        for executable in executables {
            let request = BoundedCommandRequest {
                executable,
                argv: argv.clone(),
                working_dir: self.working_dir.clone(),
                timeout,
                cancellation: CancellationToken::new(),
                environment: host_environment(),
                stdout_limit: 4096,
                stderr_limit: 4096,
            };
            if let Ok(result) = self.command_runner.run(request).await
                && result.exit_code == Some(0)
                && !result.timed_out
            {
                return true;
            }
        }
        false
    }
}

/// 装后验证用的**真实命令**（不能只 `--version`）。
fn post_install_verification_argv(target_wire: &str) -> Vec<String> {
    match target_wire {
        // codegraph：`status --json` **不要求已建索引**（实测 1.6.2 在未初始化
        // 目录 exit 0，输出 `{"initialized":false,...}`），而任何命令都经由
        // 平台包自带的 node + dist 启动 ⇒ 平台包缺失时 exit 1（实测），故它
        // 既能证明「bundle 真能用」，又不会把「未建索引」误判成失败。
        // **不得**用 `files`：实测 1.6.2 在未初始化目录 `files --json` exit 1
        // （`✗ CodeGraph not initialized`），会把新装的健康 codegraph 判失败。
        "codegraph" => vec!["status".to_string(), "--json".to_string()],
        // provider CLI：`--help` 会真正加载入口与依赖（缺依赖即非零退出）。
        _ => vec!["--help".to_string()],
    }
}

#[async_trait::async_trait]
impl PostInstallProber for HealthRefreshProber {
    async fn refresh_after_install(
        &self,
        target: InstallTarget,
        install_bin_dir: Option<&Path>,
    ) -> Result<PostInstallProbeOutcome, String> {
        let health = self.health.refresh(CancellationToken::new()).await;
        let row = row_for_wire(target.wire_label());
        // ① 版本**优先读 npm 包的 `package.json`**（不执行进程、更快更稳；
        //    magpie `proc.Version` 同款）——读不到才回落 `--version`。
        let mut version = row.and_then(|row| {
            let package = row.npm_package?;
            let prefix = install_bin_dir?.parent()?;
            npm_package_json_version(prefix, package)
        });
        // ② provider 版本走 health 快照（PATH ∪ overlay 口径）。
        if version.is_none()
            && let InstallTarget::Provider(provider) = &target
        {
            version = self
                .health
                .latest_diagnostic()
                .entry(provider)
                .and_then(|entry| entry.version.clone());
        }
        // 读不到版本 ⇒ 跑 `--version`：先 PATH 口径，再**绝对路径**兜底
        // （装到 PATH 之外时 PATH 口径必然 127）。
        if version.is_none() {
            let command = row.map(|row| row.command).unwrap_or("codegraph");
            version = self
                .probe_version(command, install_bin_dir, false)
                .await
                .or(self.probe_version(command, install_bin_dir, true).await);
        }
        // 真实命令验证（独立于版本读取：版本可来自 package.json）。
        let real_command_ok = self.probe_real_command(target, install_bin_dir).await;
        match (version, health) {
            (Some(version), _) => Ok(PostInstallProbeOutcome {
                installed_version: Some(version),
                real_command_ok,
            }),
            (None, Err(error)) => Err(error.to_string()),
            (None, Ok(_)) => Ok(PostInstallProbeOutcome {
                installed_version: None,
                real_command_ok,
            }),
        }
    }

    /// 更新前版本（Kimi 等脚本类）：走**与装后同一个** `--version` 探测口径
    /// （PATH 口径 —— 装前还没有本次安装的 bin 目录），保证前后同源可比。
    async fn version_before_update(&self, target: InstallTarget) -> Option<String> {
        let row = row_for_wire(target.wire_label())?;
        self.probe_version(row.command, None, false).await
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

/// npm 包版本（读 `<prefix>/lib/node_modules/<pkg>/package.json` 的
/// `version`；读不到返回 `None` ⇒ 回落 `--version`）。
///
/// 不执行进程（magpie `proc.Version` 同款）⇒ 更快更稳，也不会因为 CLI
/// 本身跑不起来而读不到版本。
pub fn npm_package_json_version(prefix: &Path, package: &str) -> Option<String> {
    let manifest = prefix
        .join("lib")
        .join("node_modules")
        .join(package)
        .join("package.json");
    let content = std::fs::read_to_string(manifest).ok()?;
    let value: serde_json::Value = serde_json::from_str(&content).ok()?;
    value
        .get("version")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}

/// 脚本安装源（Kimi）的 PATH 提示：官方脚本只改 shell rc ⇒ **运行中的
/// 平台进程 PATH 不变** ⇒ 探测必然 127（根因 C）。文案点名安装目录。
pub fn script_path_hint(command: &str, bin_dir: &Path) -> String {
    format!(
        "已安装到 {}；若 `{command}` 不在 PATH 上，请把 {} 加入 PATH 后重试",
        bin_dir.display(),
        bin_dir.display()
    )
}

/// 修复命令（2.7③ / 2.8③）：npm 类带 `--prefix` 与 `--include=optional`
/// （「装上但缺平台包」的修复句式，magpie `mendHint` 同款）；脚本类给官方
/// 脚本命令。**永不 sudo**。
pub fn install_repair_command(row: &InstallSourceRow, registry: &str, prefix: &Path) -> String {
    match row.npm_package {
        Some(package) => format!(
            "npm install -g --registry={registry} --prefix {} --include=optional {package}@latest",
            prefix.display()
        ),
        None => manual_install_command(row, registry),
    }
}

/// 安装产物的 bin 目录：npm 类 = `<prefix>/bin`；脚本类（Kimi）= **脚本实际
/// 输出**的目录，否则 `<home>/.kimi-code/bin`（**假设**，见 source_table）。
///
/// 登记条件以**实际存在的文件**为准：候选里取确实含该可执行文件的那个。
fn install_bin_dir(
    row: &InstallSourceRow,
    prefix: &Path,
    script_stdout: &str,
    home_dir: Option<&Path>,
) -> Option<PathBuf> {
    if row.npm_package.is_some() {
        return Some(prefix.join("bin"));
    }
    let mut candidates = Vec::new();
    if let Some(dir) = script_installed_bin_dir(script_stdout, row.command) {
        candidates.push(dir);
    }
    if let Some(home) = home_dir {
        candidates.push(kimi_install_bin_dir(home));
    }
    let containing = candidates
        .iter()
        .find(|dir| dir.join(row.command).exists())
        .cloned();
    containing.or_else(|| candidates.into_iter().next())
}

/// 从脚本输出里找安装产物路径（Kimi 官方脚本输出
/// `Installed to /root/.kimi-code/bin/kimi`）⇒ 取其所在目录。
fn script_installed_bin_dir(stdout: &str, command: &str) -> Option<PathBuf> {
    stdout
        .split_whitespace()
        .map(|token| token.trim_matches(|c: char| c == '\'' || c == '"' || c == ',' || c == ';'))
        .filter(|token| token.starts_with('/'))
        .map(PathBuf::from)
        .find(|path| path.file_name().is_some_and(|name| name == command))
        .and_then(|path| path.parent().map(Path::to_path_buf))
}

/// 把「安装产物 bin 目录」登记进 PATH overlay（**仅当该目录确实含刚装好的
/// 可执行文件**）⇒ 平台自身探测立即可见（决策 D1/D2）。
fn register_install_bin_dir(dir: Option<&Path>, command: &str) {
    let Some(dir) = dir else {
        return;
    };
    if !dir.join(command).exists() {
        return;
    }
    path_overlay::register(dir, path_overlay::OverlayPosition::Prepend);
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
    /// 宿主 HOME（脚本类安装源落点推断用；`None` = 运行时读进程环境）。
    pub home_dir: Option<PathBuf>,
    pub clock: Arc<dyn InstallClock>,
}

/// 重试与失败分类所需的静态上下文（行 / 落点 / 产物目录 / 宿主 PATH），
/// 打包传入以避免参数膨胀。
struct RetryContext<'a> {
    row: &'a InstallSourceRow,
    prefix: &'a Path,
    bin_dir: Option<&'a Path>,
    path_value: Option<&'a str>,
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
            home_dir: std::env::var_os("HOME").map(PathBuf::from),
            clock: Arc::new(SystemInstallClock),
        }
    }

    /// 已安装（但**不在 PATH 上**）的对象所在前缀：从
    /// `<prefix>/lib/node_modules/<pkg>` 的真实路径反推（magpie
    /// `fromNodeModules`）。更新必须装回这里，不得改装到别的前缀（2.10②）。
    ///
    /// `npm_global` 必须是**未经过滤**的 `npm prefix -g` 原始结果 ——
    /// 「`<prefix>/bin ∈ PATH`」过滤只适用于**新装候选**的选择：否则「唯一
    /// 已有的安装正好在 npm 全局前缀、而其 bin 不在 PATH 上」（本轮核心
    /// 现场）会被丢弃 ⇒ 更新改装到 fallback（更新了用户根本没在跑的那份）。
    fn installed_prefix(
        &self,
        row: &InstallSourceRow,
        npm_global: Option<&Path>,
    ) -> Option<PathBuf> {
        let package = row.npm_package?;
        let mut candidates: Vec<PathBuf> = Vec::new();
        if let Some(prefix) = npm_global {
            candidates.push(prefix.to_path_buf());
        }
        candidates.push(self.npm_prefix.clone());
        for candidate in candidates {
            let package_dir = npm_package_dir(&candidate, package);
            if package_dir.exists() {
                return npm_prefix_from_real_path(&package_dir);
            }
        }
        None
    }

    /// 宿主 HOME（脚本类安装源落点；测试可注入）。
    fn home_dir(&self) -> Option<PathBuf> {
        self.home_dir
            .clone()
            .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
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
    /// ② **已安装位置反推**（2.10②）—— 用**未经过滤**的 `npm prefix -g` 与
    ///    平台自身前缀扫描已安装包（`<prefix>/lib/node_modules/<pkg>` ⇒ 反推
    ///    前缀）；**只有 visible 不存在（或不可写）时才用** —— 否则机器上
    ///    同时存在两份安装时，更新会写进另一份（2.10①「更新装到 A、用户跑的
    ///    还是 B」）；
    /// ③ **`npm prefix -g`** —— 仅作**新装候选**：真实可写 且
    ///    `<prefix>/bin ∈ PATH`（装到 PATH 之外 ⇒ 装完探测仍失败）；
    /// ④ **fallback** —— `NPM_PREFIX` / `$HOME/.npm-global`。
    pub async fn resolve_effective_prefix(
        &self,
        row: &InstallSourceRow,
        path_value: Option<&str>,
    ) -> PathBuf {
        let visible = visible_cli_prefix(row.command, path_value);
        // `npm prefix -g` 的**原始**结果：已安装扫描必须用它（过滤只适用于
        // 新装候选）。
        let raw_npm_global = match self.npm_prefix_probe.as_ref() {
            Some(probe) => probe.npm_global_prefix().await,
            None => None,
        };
        // ① visible（真实可写）优先：更新必须装回用户手上那份。
        if let Some(prefix) = visible.as_deref().filter(|prefix| is_writable_dir(prefix)) {
            return prefix.to_path_buf();
        }
        // ② 只有 visible 不存在（或不可写）时，才用已安装位置反推。
        if let Some(prefix) = self.installed_prefix(row, raw_npm_global.as_deref()) {
            return prefix;
        }
        // ③ npm 全局前缀仅作新装候选（可写且 `<prefix>/bin ∈ PATH`）→ ④ 兜底。
        let npm_global = raw_npm_global
            .filter(|prefix| is_writable_dir(prefix) && prefix_bin_is_on_path(prefix, path_value));
        choose_effective_prefix(None, npm_global.as_deref(), &self.npm_prefix)
    }

    /// 装完提示（2.5 / 2.8③）：npm 类看 PATH 上生效的命令是不是本次安装的
    /// 那个；**装上但缺平台包**（真实命令跑不起来）时补上可复制的修复命令。
    fn path_hint_for(
        &self,
        row: &InstallSourceRow,
        prefix: &Path,
        bin_dir: Option<&Path>,
        path_value: Option<&str>,
        probe: Option<&PostInstallProbeOutcome>,
    ) -> Option<String> {
        let base = match (row.npm_package, bin_dir) {
            (Some(_), _) => npm_path_hint(row.command, prefix, path_value),
            // 脚本类（Kimi）：官方脚本只改 shell rc ⇒ 平台进程 PATH 不变 ⇒
            // 必须提示把安装目录加进 PATH。
            (None, Some(dir)) => Some(script_path_hint(row.command, dir)),
            (None, None) => None,
        };
        if !probe.is_some_and(|outcome| !outcome.real_command_ok) {
            return base;
        }
        let repair = install_repair_command(row, &self.registry, prefix);
        Some(match base {
            Some(hint) => {
                format!("{hint}；装上了但真实命令跑不起来（可能缺平台包），请手动运行：{repair}")
            }
            None => format!("装上了但真实命令跑不起来（可能缺平台包），请手动运行：{repair}"),
        })
    }

    /// 装上但探针未过（**含「真实命令跑不起来」**）⇒ npm 类以
    /// `--include=optional` **重试一次**；重试后仍失败 ⇒ 按**三种语义**落
    /// 失败载荷（2.8③，不得用同一条含糊文案覆盖）：
    /// ① 没装上（产物里没有该命令）；② 装上但缺平台包（命令在但真实命令
    /// 跑不起来）；③ 装上但不在 PATH（命令能跑但 PATH 上取不到它）。
    async fn retry_with_optional_dependencies(
        &self,
        job_id: &str,
        target: InstallTarget,
        context: RetryContext<'_>,
        real_command_ok_before_retry: bool,
    ) {
        let RetryContext {
            row,
            prefix,
            bin_dir,
            path_value,
        } = context;
        let mut real_command_ok = real_command_ok_before_retry;
        if let Some(retry) = npm_retry_plan(row, None, &self.registry, prefix)
            && let Ok(result) = self.runner.execute(&retry).await
            && result.exit_code == Some(0)
            && !result.timed_out
            && !result.cancelled
        {
            register_install_bin_dir(bin_dir, row.command);
            if let Ok(outcome) = self
                .prober
                .refresh_after_install(target.clone(), bin_dir)
                .await
            {
                real_command_ok = outcome.real_command_ok;
                // 成功判定 = 版本可读 **且** 真实命令可跑（2.8②）。
                if let Some(version) = outcome
                    .installed_version
                    .clone()
                    .filter(|_| outcome.real_command_ok)
                {
                    let path_hint =
                        self.path_hint_for(row, prefix, bin_dir, path_value, Some(&outcome));
                    let completed = InstallJobOutcome {
                        installed_version: Some(version),
                        path_hint,
                    };
                    let _ = self.store.complete(job_id, completed, self.clock.now());
                    return;
                }
            }
        }
        let repair = install_repair_command(row, &self.registry, prefix);
        // 2.8③ 三种失败语义：**没装上**（产物里没有该命令）vs **装上但缺
        // 平台包**（命令在但跑不起来）vs **装上但不在 PATH**（能跑但 PATH
        // 上找不到它）—— 提示要能区分。
        let installed = bin_dir.is_some_and(|dir| dir.join(row.command).exists());
        let message = if !installed {
            format!(
                "没装上（安装产物里没有 `{}` 命令）。请手动运行：{repair}",
                row.command
            )
        } else if !real_command_ok {
            format!("装上了但跑不起来（装完探测未通过，可能缺平台包）。请手动运行：{repair}")
        } else {
            format!(
                "装上了但不在 PATH 上（`{}` 能执行，但 PATH 上找不到它）。请把 {} 加入 PATH 后重试",
                row.command,
                bin_dir
                    .map(|dir| dir.display().to_string())
                    .unwrap_or_default()
            )
        };
        let failure = InstallJobFailure {
            kind: InstallFailureKind::Other,
            message,
            manual_command: repair,
        };
        let _ = self.store.fail(job_id, failure, self.clock.now());
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
        // 更新语义（2.10③ / REQ-PCI-10）：对象**已存在** ⇒ 本次是「更新」⇒
        // 先取当前版本**同源**快照（npm 读 package.json，不执行进程；Kimi 等
        // 脚本类走同一 prober 的 `--version` 口径），更新后版本未变要**明确
        // 报**「命令仍是旧版」，不得静默成功。
        let before_version = match row.npm_package {
            Some(package) => npm_package_json_version(&prefix, package),
            None => self.prober.version_before_update(target.clone()).await,
        };
        match self.runner.execute(&plan).await {
            Ok(result) if result.exit_code == Some(0) && !result.timed_out && !result.cancelled => {
                // 装后登记 overlay（**仅当**目录里确实有刚装好的可执行文件）
                // ⇒ 平台自身的命令可见 PATH 立刻包含新装的 CLI（D1/D2）。
                let bin_dir =
                    install_bin_dir(row, &prefix, &result.stdout, self.home_dir().as_deref());
                register_install_bin_dir(bin_dir.as_deref(), row.command);
                // overlay 登记**之后**再探测（平台自身探测立即可见）。
                let probe = self
                    .prober
                    .refresh_after_install(target.clone(), bin_dir.as_deref())
                    .await
                    .ok();
                let detected = probe
                    .as_ref()
                    .and_then(|outcome| outcome.installed_version.clone());
                // 完成判定 = 「版本可读」**且**「真实命令可跑」：只有其一
                // 不得判成功 —— 「包在、版本读得到，但真实命令跑不起来」是
                // 假绿（2.8②/2.8③）。
                let real_command_ok = probe
                    .as_ref()
                    .is_some_and(|outcome| outcome.real_command_ok);
                // ③ 更新后版本未变 ⇒ 明确报「命令仍是旧版」（不得静默成功）。
                if let (Some(before), Some(after)) =
                    (before_version.as_deref(), detected.as_deref())
                    && before == after
                {
                    let repair = install_repair_command(row, &self.registry, &prefix);
                    let failure = InstallJobFailure {
                        kind: InstallFailureKind::Other,
                        message: format!(
                            "命令仍是旧版（{after}）：更新没有生效。若该版本已是最新可忽略；否则请手动运行：{repair}"
                        ),
                        manual_command: repair,
                    };
                    let _ = self.store.fail(job_id, failure, self.clock.now());
                    return;
                }
                let path_hint = self.path_hint_for(
                    row,
                    &prefix,
                    bin_dir.as_deref(),
                    path_value.as_deref(),
                    probe.as_ref(),
                );
                match detected.filter(|_| real_command_ok) {
                    Some(version) => {
                        let outcome = InstallJobOutcome {
                            installed_version: Some(version),
                            path_hint,
                        };
                        let _ = self.store.complete(job_id, outcome, self.clock.now());
                    }
                    // 装上但探针未过（**含**真实命令跑不起来）⇒ 重试一次
                    // （2.7③ / 2.8③）。
                    None => {
                        self.retry_with_optional_dependencies(
                            job_id,
                            target,
                            RetryContext {
                                row,
                                prefix: &prefix,
                                bin_dir: bin_dir.as_deref(),
                                path_value: path_value.as_deref(),
                            },
                            real_command_ok,
                        )
                        .await;
                    }
                }
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
    include!("runner_update_tests.inc.rs");
}
