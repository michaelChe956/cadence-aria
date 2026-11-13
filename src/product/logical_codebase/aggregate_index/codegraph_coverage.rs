//! codegraph SDK 成员覆盖验收（只读）。
//!
//! 背景：`codegraph files --json` 的全量文件清单随聚合根规模线性膨胀
//! （现场 3.1 GB 库 ⇒ ~230 MB 载荷），1 MiB/64 MiB 抓取上限都装不下 ⇒
//! 验收必失败。而验收真正需要的只是「每个成员目录有没有被索引的文件」。
//! 本模块走 codegraph 库 API（`CodeGraph.open` + `hasFilesUnder`）直接
//! 回答是/否，载荷几百字节；顺带取索引健康字段（stale/pending/state/
//! fileCount）供上层做健康闸（warning，不阻断激活）。
//!
//! SDK 是全局安装的 npm 包，不能被普通脚本直接 import ⇒ 按绝对路径
//! `import(<npm-sdk.js>)`（CJS interop：`mod.default ?? mod` 取
//! `CodeGraph`）。Node 用 codegraph 平台包自带的二进制（不要求用户装
//! Node）；PATH 上的 node 需 ≥ 22.5 才回退采用。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::bounded_command_runner::{
    BoundedCommandError, BoundedCommandRequest, BoundedCommandResult, BoundedCommandRunner,
    TokioBoundedCommandRunner,
};
use crate::product::json_store::validate_relative_id;
// npm 布局与前缀语义与安装链同源（只读复用，不引入第二套约定）。
use crate::product::provider_installation::effective_prefix::{
    npm_package_dir, npm_prefix_from_real_path,
};
use crate::product::provider_installation::source_table::resolve_npm_prefix;

use super::AggregateIndexError;

/// codegraph 的 npm 包名（`<prefix>/lib/node_modules/<pkg>` 布局用）。
pub const CODEGRAPH_NPM_PACKAGE: &str = "@colbymchenry/codegraph";

/// PATH 回退 node 的最低版本（自带 node 无此门槛）。
pub const MIN_NODE_VERSION: (u64, u64, u64) = (22, 5, 0);

/// helper 执行超时（SDK 只做目录覆盖判定 + 健康字段，实测 ~8 ms/open）。
const HELPER_TIMEOUT_SECS: u64 = 60;
/// `npm root -g` 探针超时——口径同 provider_installation 的
/// `CommandNpmGlobalPrefixProbe`（10s / 4096 B）。
const NPM_PROBE_TIMEOUT_SECS: u64 = 10;
const PROBE_OUTPUT_LIMIT_BYTES: usize = 4096;
const HELPER_OUTPUT_LIMIT_BYTES: usize = 1024 * 1024;
const HELPER_STDERR_LIMIT_BYTES: usize = 64 * 1024;

/// SDK 覆盖验收的结果：每成员覆盖判定 + 原样健康字段。
#[derive(Debug, Clone, PartialEq)]
pub struct CoverageOutcome {
    /// 成员名 ⇒ `hasFilesUnder(member)`（目录边界精确，优于 CLI 裸前缀）。
    pub member_covered: BTreeMap<String, bool>,
    /// `getStats()` 原样（取不到则 None）。
    pub stats: Option<Value>,
    /// 健康字段聚合（state/pending/stale；取不到的字段被省略）。
    pub index: Option<Value>,
}

/// 聚合成员覆盖探针——生产实现走 codegraph SDK（[`CodeGraphCoverage`]），
/// DI/测试可注入桩。
pub trait AggregateCoverageProbe: Send + Sync {
    fn coverage(
        &self,
        root: &Path,
        member_names: &[String],
    ) -> Result<CoverageOutcome, AggregateIndexError>;
}

/// DI/测试构造器的默认探针：恒报 SDK 不可用 ⇒ 验收回退既有 CLI files
/// 路径（与今天的行为一致）。生产入口 `AggregateIndexOperation::new`
/// 装配 [`CodeGraphCoverage`] 的环境解析探针。
pub struct CliFallbackCoverageProbe;

impl AggregateCoverageProbe for CliFallbackCoverageProbe {
    fn coverage(
        &self,
        _root: &Path,
        _member_names: &[String],
    ) -> Result<CoverageOutcome, AggregateIndexError> {
        Err(AggregateIndexError::Failed {
            code: "codegraph_sdk_not_found",
            message: "coverage probe not configured for this operation; accepting via the \
                      CLI files fallback"
                .to_string(),
        })
    }
}

/// SDK（npm-sdk.js）与 node 可执行的解析候选；测试可整体注入替换，
/// 生产由 [`CoveragePaths::resolve_from_environment`] 从环境解析。
#[derive(Clone, Default)]
pub struct CoveragePaths {
    /// 候选1：我们安装前缀下 npm 标准布局的 `npm-sdk.js` 绝对路径。
    pub install_prefix_sdk: Option<PathBuf>,
    /// 候选2：`npm root -g` 探针结果（全局 node_modules 根）。
    pub npm_global_root: Option<PathBuf>,
    /// 候选3：codegraph 可执行路径（真实路径反推包根）。
    pub codegraph_executable: Option<PathBuf>,
    /// node 候选1：codegraph 平台包自带的 node 可执行（按序取第一个
    /// 存在且可执行者；空 ⇒ 从解析出的 SDK 包根下扫描
    /// `node_modules/@colbymchenry/codegraph-*/node`）。
    pub bundled_nodes: Vec<PathBuf>,
    /// node 候选2：PATH 上的 node 可执行（需版本 ≥ 22.5 才采用）。
    pub path_node: Option<PathBuf>,
    /// node 候选3 的来源：宿主 HOME（`~/.codegraph/bundles/<target>-<ver>/` 是
    /// codegraph 的**自愈缓存**布局）。`None` ⇒ 不扫缓存（测试默认关闭，
    /// 否则会读到开发机真实缓存）。
    pub home_dir: Option<PathBuf>,
}

impl CoveragePaths {
    /// 生产解析：读进程环境与 PATH；候选1 已命中时跳过 `npm root -g`
    /// 探针（口径同 provider_installation 的 `CommandNpmGlobalPrefixProbe`：
    /// 10s 超时 / 4096 B 输出 / 首个非空行且路径存在）。
    pub fn resolve_from_environment(runner: &Arc<dyn BoundedCommandRunner>) -> Self {
        let install_prefix_sdk = install_prefix_sdk_candidate();
        let npm_global_root = if install_prefix_sdk.as_deref().is_some_and(Path::is_file) {
            None
        } else {
            probe_npm_global_root(runner)
        };
        Self {
            install_prefix_sdk,
            npm_global_root,
            codegraph_executable: find_executable_in_path("codegraph"),
            bundled_nodes: Vec::new(),
            path_node: find_executable_in_path("node"),
            home_dir: std::env::var_os("HOME")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from),
        }
    }
}

/// SDK 覆盖验收执行器。
pub struct CodeGraphCoverage {
    runner: Arc<dyn BoundedCommandRunner>,
    /// 候选注入（测试 seam）；`None` = 生产环境解析。
    paths: Option<CoveragePaths>,
    /// helper 脚本源（`include_str!` 内嵌随源码走，运行时落临时文件）。
    helper_source: String,
    /// helper 落盘目录（`None` = 系统 temp；测试注入以断言清理）。
    scratch_dir: Option<PathBuf>,
}

impl CodeGraphCoverage {
    /// 生产装配：Tokio 运行器的环境解析探针。
    pub fn production() -> Self {
        Self {
            runner: Arc::new(TokioBoundedCommandRunner),
            paths: None,
            helper_source: include_str!("codegraph_coverage_helper.mjs").to_string(),
            scratch_dir: None,
        }
    }

    /// 测试/DI 装配：固定候选与 helper 落盘目录。
    pub fn with_paths(
        runner: Arc<dyn BoundedCommandRunner>,
        paths: CoveragePaths,
        scratch_dir: Option<PathBuf>,
    ) -> Self {
        Self {
            runner,
            paths: Some(paths),
            helper_source: include_str!("codegraph_coverage_helper.mjs").to_string(),
            scratch_dir,
        }
    }

    fn run_helper(
        &self,
        node: &Path,
        helper: &Path,
        sdk_path: &Path,
        root: &Path,
        member_names: &[String],
    ) -> Result<CoverageOutcome, AggregateIndexError> {
        let members_json = serde_json::to_string(member_names).map_err(|error| {
            coverage_failed(format!("serialize members JSON for helper: {error}"))
        })?;
        let request = BoundedCommandRequest {
            executable: node.display().to_string(),
            argv: vec![
                helper.display().to_string(),
                "--sdk".to_string(),
                sdk_path.display().to_string(),
                "--root".to_string(),
                root.display().to_string(),
                "--members".to_string(),
                members_json,
            ],
            working_dir: root.to_path_buf(),
            timeout: Duration::from_secs(HELPER_TIMEOUT_SECS),
            cancellation: CancellationToken::new(),
            environment: sanitized_environment(),
            stdout_limit: HELPER_OUTPUT_LIMIT_BYTES,
            stderr_limit: HELPER_STDERR_LIMIT_BYTES,
        };
        let output = run_bounded_sync(&self.runner, request)
            .map_err(|error| coverage_failed(error.to_string()))?;
        if output.timed_out {
            return Err(coverage_failed(format!(
                "coverage helper timed out after {HELPER_TIMEOUT_SECS}s"
            )));
        }
        parse_helper_output(&output, member_names)
    }
}

impl AggregateCoverageProbe for CodeGraphCoverage {
    fn coverage(
        &self,
        root: &Path,
        member_names: &[String],
    ) -> Result<CoverageOutcome, AggregateIndexError> {
        for member in member_names {
            validate_relative_id(member)?;
        }
        let paths = match &self.paths {
            Some(injected) => injected.clone(),
            None => CoveragePaths::resolve_from_environment(&self.runner),
        };
        let sdk_path = resolve_sdk_path(&paths).ok_or_else(|| AggregateIndexError::Failed {
            code: "codegraph_sdk_not_found",
            message: format!(
                "未找到 codegraph 的 npm-sdk.js（安装前缀 / npm root -g / 可执行反推均未\
                 命中）；可手动安装：npm install -g {CODEGRAPH_NPM_PACKAGE}"
            ),
        })?;
        // 方案 §五 包 4「回退保底」：**Node 不可用不得阻断聚合验收** ——
        // 归入 operation 既有的「SDK 覆盖验收不可用 ⇒ 回退 CLI files」分支
        // （保留 64 MiB 上限与专用截断码），而不是直接失败。
        let node =
            resolve_node_executable(&paths, &sdk_path, &self.runner).map_err(
                |error| match error {
                    AggregateIndexError::Failed {
                        code: "codegraph_node_unavailable",
                        message,
                    } => AggregateIndexError::Failed {
                        code: "codegraph_sdk_incompatible",
                        message: format!(
                            "{message}（SDK 覆盖验收不可用，已回退 CLI files 验收路径）"
                        ),
                    },
                    other => other,
                },
            )?;
        let helper_path = write_helper_file(&self.helper_source, self.scratch_dir.as_deref())?;
        let result = self.run_helper(&node, &helper_path, &sdk_path, root, member_names);
        let _ = std::fs::remove_file(&helper_path);
        result
    }
}

/// helper 的 stdout 协议（成功与失败共用一个 JSON 形态）。
#[derive(Deserialize)]
struct HelperOutput {
    ok: bool,
    #[serde(default)]
    members: BTreeMap<String, bool>,
    #[serde(default)]
    stats: Option<Value>,
    #[serde(default)]
    index: Option<Value>,
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    message: Option<String>,
}

fn parse_helper_output(
    output: &BoundedCommandResult,
    member_names: &[String],
) -> Result<CoverageOutcome, AggregateIndexError> {
    let stdout = output.stdout.trim();
    let parsed: HelperOutput = serde_json::from_str(stdout).map_err(|error| {
        coverage_failed(format!(
            "helper stdout 不是合法 JSON（exit {:?}）：{error}；stdout 前 200 字节 {:?}；\
             stderr：{}",
            output.exit_code,
            &stdout[..stdout.len().min(200)],
            output.stderr.trim()
        ))
    })?;
    if !parsed.ok {
        let code = parsed.code.as_deref().unwrap_or("unknown");
        let message = parsed
            .message
            .unwrap_or_else(|| "helper failed without a message".to_string());
        if code == "sdk_incompatible" {
            return Err(AggregateIndexError::Failed {
                code: "codegraph_sdk_incompatible",
                message: format!("codegraph SDK 不兼容：{message}"),
            });
        }
        return Err(coverage_failed(format!(
            "helper 报告失败（{code}）：{message}"
        )));
    }
    let mut member_covered = BTreeMap::new();
    for member in member_names {
        let covered =
            parsed.members.get(member).copied().ok_or_else(|| {
                coverage_failed(format!("helper 结果缺少成员 {member} 的覆盖判定"))
            })?;
        member_covered.insert(member.clone(), covered);
    }
    Ok(CoverageOutcome {
        member_covered,
        stats: parsed.stats,
        index: parsed.index,
    })
}

/// 把内嵌 helper 写到唯一临时文件（目录不存在则创建）。
fn write_helper_file(
    source: &str,
    scratch_dir: Option<&Path>,
) -> Result<PathBuf, AggregateIndexError> {
    let dir = scratch_dir
        .map(Path::to_path_buf)
        .unwrap_or_else(std::env::temp_dir);
    std::fs::create_dir_all(&dir)
        .map_err(|error| coverage_failed(format!("create helper scratch dir: {error}")))?;
    let path = dir.join(format!(
        "codegraph-coverage-helper-{}.mjs",
        uuid::Uuid::new_v4()
    ));
    std::fs::write(&path, source)
        .map_err(|error| coverage_failed(format!("write helper temp file: {error}")))?;
    Ok(path)
}

fn coverage_failed(message: String) -> AggregateIndexError {
    AggregateIndexError::Failed {
        code: "codegraph_coverage_failed",
        message,
    }
}

/// 按序取第一个真实存在的 `npm-sdk.js`：
/// 1. 安装前缀 npm 标准布局；2. `npm root -g`；3. codegraph 可执行反推。
pub fn resolve_sdk_path(paths: &CoveragePaths) -> Option<PathBuf> {
    if let Some(candidate) = paths
        .install_prefix_sdk
        .as_deref()
        .filter(|candidate| candidate.is_file())
    {
        return Some(candidate.to_path_buf());
    }
    if let Some(global_root) = paths.npm_global_root.as_deref() {
        let candidate = global_root.join(CODEGRAPH_NPM_PACKAGE).join("npm-sdk.js");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    let executable = paths.codegraph_executable.as_deref()?;
    let real = std::fs::canonicalize(executable).ok()?;
    let prefix = npm_prefix_from_real_path(&real)?;
    let candidate = npm_package_dir(&prefix, CODEGRAPH_NPM_PACKAGE).join("npm-sdk.js");
    candidate.is_file().then_some(candidate)
}

/// node 解析：自带 node（存在且可执行）优先；PATH node 需 `--version`
/// 实测 ≥ 22.5；都不可用 ⇒ `codegraph_node_unavailable`。
pub fn resolve_node_executable(
    paths: &CoveragePaths,
    sdk_path: &Path,
    runner: &Arc<dyn BoundedCommandRunner>,
) -> Result<PathBuf, AggregateIndexError> {
    let mut bundled = paths.bundled_nodes.clone();
    if bundled.is_empty() {
        bundled = bundled_node_candidates(sdk_path, paths.home_dir.as_deref());
    }
    for candidate in &bundled {
        if is_executable_file(candidate) {
            return Ok(candidate.clone());
        }
    }
    if let Some(path_node) = paths
        .path_node
        .as_deref()
        .filter(|candidate| is_executable_file(candidate))
        && path_node_meets_minimum(path_node, runner)
    {
        return Ok(path_node.to_path_buf());
    }
    Err(AggregateIndexError::Failed {
        code: "codegraph_node_unavailable",
        message: "需要 Node.js ≥ 22.5 运行 codegraph SDK 覆盖验收：未找到 codegraph 平台\
                 包自带的 node，PATH 上的 node 缺失或版本低于 22.5；可安装 Node.js \
                 ≥ 22.5，或重装 codegraph（npm install -g @colbymchenry/codegraph）"
            .to_string(),
    })
}

/// `@colbymchenry` 作用域目录名。
const CODEGRAPH_NPM_SCOPE: &str = "@colbymchenry";

/// 平台包目录名的后缀 `<platform>-<arch>`（npm `os`/`cpu` 口径：`linux-x64`
/// / `darwin-arm64` / `win32-x64`；`std::env::consts` 的 `macos`/`x86_64`
/// 与 npm 拼写不同 ⇒ 显式映射）。
fn codegraph_platform_target() -> String {
    let platform = match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "win32",
        other => other,
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        other => other,
    };
    format!("{platform}-{arch}")
}

/// codegraph 自带 node 的全部候选（按采纳优先级）：
/// ① **nested** —— `<主包>/node_modules/@colbymchenry/codegraph-<target>/node`；
/// ② **hoisted** —— 与主包**同级**的 `<node_modules>/@colbymchenry/codegraph-<target>/node`
///    （实测 1.6.2 真实布局：`libtest/node_modules/@colbymchenry/` 下
///    `codegraph` 与 `codegraph-linux-x64` 并列，且 node 在平台包里）；
/// ③ **自愈缓存** —— `~/.codegraph/bundles/<target>-<version>/node`
///    （`npm-shim.js` 下载缓存，同平台多版本时版本高者优先）。
///
/// 目录名按**当前平台** `<platform>-<arch>` 匹配（不得硬编码单一平台）。
fn bundled_node_candidates(sdk_path: &Path, home_dir: Option<&Path>) -> Vec<PathBuf> {
    let Some(package_root) = sdk_path.parent() else {
        return Vec::new();
    };
    let target = codegraph_platform_target();
    let mut candidates: Vec<PathBuf> = Vec::new();
    // ① 主包内 node_modules（nested 布局）。
    candidates.extend(platform_package_nodes(
        &package_root.join("node_modules").join(CODEGRAPH_NPM_SCOPE),
        &target,
    ));
    // ② 每一层上层 `node_modules/@colbymchenry`（hoisted 布局 —— 真实资产）。
    for ancestor in package_root.ancestors().skip(1) {
        if ancestor
            .file_name()
            .is_some_and(|name| name == "node_modules")
        {
            candidates.extend(platform_package_nodes(
                &ancestor.join(CODEGRAPH_NPM_SCOPE),
                &target,
            ));
        }
    }
    // ③ 自愈缓存。
    if let Some(home) = home_dir {
        candidates.extend(cached_bundle_nodes(home, &target));
    }
    candidates
}

/// 作用域目录里与主包同级的平台包自带 node：**当前平台优先**，其余
/// `codegraph-*` 兜底（排序保证确定性）。
fn platform_package_nodes(scoped: &Path, target: &str) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(scoped) else {
        return Vec::new();
    };
    let exact = format!("codegraph-{target}");
    let mut names: Vec<String> = entries
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name.starts_with("codegraph-"))
        .collect();
    names.sort();
    names.sort_by_key(|name| *name != exact);
    names
        .into_iter()
        .map(|name| scoped.join(name).join(node_executable_name()))
        .collect()
}

/// 自愈缓存里的自带 node：`<home>/.codegraph/bundles/<target>-<version>/node`
/// （同平台多版本 ⇒ 版本高者优先）。
fn cached_bundle_nodes(home: &Path, target: &str) -> Vec<PathBuf> {
    let bundles = home.join(".codegraph").join("bundles");
    let Ok(entries) = std::fs::read_dir(&bundles) else {
        return Vec::new();
    };
    let prefix = format!("{target}-");
    let mut names: Vec<String> = entries
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name.starts_with(&prefix))
        .collect();
    names.sort_by(|left, right| {
        version_key(right)
            .cmp(&version_key(left))
            .then_with(|| right.cmp(left))
    });
    names
        .into_iter()
        .map(|name| bundles.join(name).join(node_executable_name()))
        .collect()
}

/// 目录名 `<target>-<version>` ⇒ 版本数字段（不可解析段记 0）。
fn version_key(name: &str) -> Vec<u64> {
    name.rsplit('-')
        .next()
        .unwrap_or_default()
        .split('.')
        .map(|part| part.parse::<u64>().unwrap_or(0))
        .collect()
}

fn node_executable_name() -> &'static str {
    if cfg!(windows) { "node.exe" } else { "node" }
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// PATH 回退 node 的版本闸：`node --version` 实测 ≥ 22.5 才采用
///（无法解析/命令失败均 fail-closed 不采用）。
fn path_node_meets_minimum(node: &Path, runner: &Arc<dyn BoundedCommandRunner>) -> bool {
    let request = BoundedCommandRequest {
        executable: node.display().to_string(),
        argv: vec!["--version".to_string()],
        working_dir: std::env::temp_dir(),
        timeout: Duration::from_secs(NPM_PROBE_TIMEOUT_SECS),
        cancellation: CancellationToken::new(),
        environment: sanitized_environment(),
        stdout_limit: PROBE_OUTPUT_LIMIT_BYTES,
        stderr_limit: PROBE_OUTPUT_LIMIT_BYTES,
    };
    let Ok(output) = run_bounded_sync(runner, request) else {
        return false;
    };
    output.exit_code == Some(0)
        && parse_node_version(&output.stdout).is_some_and(|version| version >= MIN_NODE_VERSION)
}

/// 解析 `node --version` 输出（`v22.5.0`、`v24.16.0-nightly.1` 等）；
/// 无法解析 ⇒ None（fail-closed，不采用）。
pub fn parse_node_version(output: &str) -> Option<(u64, u64, u64)> {
    let trimmed = output.trim();
    let token = trimmed.strip_prefix('v').unwrap_or(trimmed);
    let numeric: String = token
        .chars()
        .take_while(|character| character.is_ascii_digit() || *character == '.')
        .collect();
    let mut segments = numeric.split('.');
    let major = leading_u64(segments.next()?)?;
    let minor = leading_u64(segments.next()?)?;
    let patch = leading_u64(segments.next()?)?;
    if segments.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

/// 取段的前导数字（`0-nightly.1` ⇒ 0；空段 ⇒ None）。
fn leading_u64(segment: &str) -> Option<u64> {
    let digits: String = segment
        .chars()
        .take_while(|character| character.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

/// 以同步方式跑一次有界命令（镜像 `codegraph_cli` 的线程 + 单线程
/// runtime 阻塞口径，避免把 async 传染给上层验收调用链）。
fn run_bounded_sync(
    runner: &Arc<dyn BoundedCommandRunner>,
    request: BoundedCommandRequest,
) -> Result<BoundedCommandResult, BoundedCommandError> {
    let runner = runner.clone();
    std::thread::spawn(
        move || -> Result<BoundedCommandResult, BoundedCommandError> {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| BoundedCommandError::Io {
                    details: error.to_string(),
                })?;
            runtime.block_on(runner.run(request))
        },
    )
    .join()
    .map_err(|_| BoundedCommandError::Io {
        details: "coverage runner thread panicked".to_string(),
    })?
}

/// 我们自己安装时用的前缀（NPM_PREFIX 覆盖 → `$HOME/.npm-global`）下的
/// npm 标准布局 `npm-sdk.js` 候选。
fn install_prefix_sdk_candidate() -> Option<PathBuf> {
    let prefix = resolve_npm_prefix(
        std::env::var("NPM_PREFIX").ok().as_deref(),
        std::env::var_os("HOME").map(PathBuf::from).as_deref(),
    );
    Some(npm_package_dir(&prefix, CODEGRAPH_NPM_PACKAGE).join("npm-sdk.js"))
}

/// `npm root -g` 探针：全局 node_modules 根（首非空行且确为目录）。
fn probe_npm_global_root(runner: &Arc<dyn BoundedCommandRunner>) -> Option<PathBuf> {
    let request = BoundedCommandRequest {
        executable: "npm".to_string(),
        argv: vec!["root".to_string(), "-g".to_string()],
        working_dir: std::env::temp_dir(),
        timeout: Duration::from_secs(NPM_PROBE_TIMEOUT_SECS),
        cancellation: CancellationToken::new(),
        environment: sanitized_environment(),
        stdout_limit: PROBE_OUTPUT_LIMIT_BYTES,
        stderr_limit: PROBE_OUTPUT_LIMIT_BYTES,
    };
    let output = run_bounded_sync(runner, request).ok()?;
    if output.exit_code != Some(0) || output.timed_out {
        return None;
    }
    let line = output
        .stdout
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())?;
    let path = PathBuf::from(line);
    path.is_dir().then_some(path)
}

fn find_executable_in_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|directory| directory.join(executable_file_name(name)))
        .find(|candidate| is_executable_file(candidate))
}

fn executable_file_name(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

fn sanitized_environment() -> BTreeMap<String, String> {
    let mut environment = std::env::var("PATH")
        .map(|path| BTreeMap::from([("PATH".to_string(), path)]))
        .unwrap_or_default();
    if let Some(home) = std::env::var_os("HOME") {
        environment.insert("HOME".to_string(), home.to_string_lossy().into_owned());
    }
    environment
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    /// 覆盖验收桩 runner：按可执行名/argv 分发
    /// `--version` 与 helper 调用（`--members`），其余 panic。
    #[derive(Default)]
    struct FakeCoverageRunner {
        state: Mutex<FakeCoverageState>,
    }

    #[derive(Default)]
    struct FakeCoverageState {
        node_version: Option<String>,
        helper_stdout: Option<String>,
        helper_exit_code: Option<i32>,
        requests: Vec<BoundedCommandRequest>,
    }

    impl FakeCoverageRunner {
        fn node_version(&self, version: &str) {
            self.state.lock().unwrap().node_version = Some(version.to_string());
        }

        fn helper_stdout(&self, stdout: &str) {
            self.state.lock().unwrap().helper_stdout = Some(stdout.to_string());
        }

        fn helper_exit_code(&self, code: i32) {
            self.state.lock().unwrap().helper_exit_code = Some(code);
        }

        fn requests(&self) -> Vec<BoundedCommandRequest> {
            std::mem::take(&mut self.state.lock().unwrap().requests)
        }
    }

    #[async_trait::async_trait]
    impl BoundedCommandRunner for FakeCoverageRunner {
        async fn run(
            &self,
            request: BoundedCommandRequest,
        ) -> Result<BoundedCommandResult, BoundedCommandError> {
            let mut state = self.state.lock().unwrap();
            state.requests.push(request.clone());
            let stdout = if request.argv.len() == 1 && request.argv[0] == "--version" {
                state.node_version.clone().unwrap_or_else(|| {
                    panic!("unexpected node --version probe without scripted version")
                })
            } else if request.argv.iter().any(|argument| argument == "--members") {
                state.helper_stdout.clone().unwrap_or_else(|| {
                    panic!("unexpected helper invocation without scripted stdout")
                })
            } else {
                panic!("unexpected coverage runner argv: {:?}", request.argv)
            };
            let exit_code = if request.argv.iter().any(|argument| argument == "--members") {
                state.helper_exit_code.unwrap_or(0)
            } else {
                0
            };
            Ok(BoundedCommandResult {
                exit_code: Some(exit_code),
                stdout,
                stderr: String::new(),
                timed_out: false,
                cancelled: false,
                stdout_truncated: false,
                stderr_truncated: false,
                duration_ms: 1,
            })
        }
    }

    struct CoverageFixture {
        _temp: tempfile::TempDir,
        runner: Arc<FakeCoverageRunner>,
        sdk: PathBuf,
        node: PathBuf,
        scratch: PathBuf,
        root: PathBuf,
    }

    fn executable_file(path: &Path) -> PathBuf {
        std::fs::write(path, "#!/bin/sh\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = std::fs::metadata(path).unwrap().permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(path, permissions).unwrap();
        }
        path.to_path_buf()
    }

    fn coverage_fixture() -> CoverageFixture {
        let temp = tempfile::tempdir().unwrap();
        let sdk_dir = temp.path().join("codegraph-pkg");
        std::fs::create_dir_all(&sdk_dir).unwrap();
        let sdk = sdk_dir.join("npm-sdk.js");
        std::fs::write(&sdk, "module.exports = {};").unwrap();
        let node = executable_file(&temp.path().join("bundled-node"));
        let scratch = temp.path().join("scratch");
        std::fs::create_dir_all(&scratch).unwrap();
        let root = temp.path().join("aggregate-root");
        std::fs::create_dir_all(&root).unwrap();
        CoverageFixture {
            _temp: temp,
            runner: Arc::new(FakeCoverageRunner::default()),
            sdk,
            node,
            scratch,
            root,
        }
    }

    impl CoverageFixture {
        fn paths(&self) -> CoveragePaths {
            CoveragePaths {
                install_prefix_sdk: Some(self.sdk.clone()),
                npm_global_root: None,
                codegraph_executable: None,
                bundled_nodes: vec![self.node.clone()],
                path_node: None,
                home_dir: None,
            }
        }

        fn coverage(&self) -> CodeGraphCoverage {
            CodeGraphCoverage::with_paths(
                self.runner.clone(),
                self.paths(),
                Some(self.scratch.clone()),
            )
        }

        fn members(&self) -> Vec<String> {
            vec!["api".to_string(), "web".to_string()]
        }

        fn runner_handle(&self) -> Arc<dyn BoundedCommandRunner> {
            self.runner.clone()
        }

        fn helper_stdout(&self, stdout: &str) {
            self.runner.helper_stdout(stdout);
        }

        fn helper_exit_code(&self, code: i32) {
            self.runner.helper_exit_code(code);
        }
    }

    #[test]
    fn coverage_reports_member_covered_from_sdk() {
        let fixture = coverage_fixture();
        fixture.helper_stdout(
            r#"{"ok":true,"members":{"api":true,"web":false},"stats":{"fileCount":2},"index":{"state":"ready","pending":0,"stale":false}}"#,
        );

        let outcome = fixture
            .coverage()
            .coverage(&fixture.root, &fixture.members())
            .unwrap();

        assert_eq!(
            outcome.member_covered,
            BTreeMap::from([("api".to_string(), true), ("web".to_string(), false),])
        );
        assert_eq!(
            outcome
                .stats
                .as_ref()
                .and_then(|stats| stats.get("fileCount")),
            Some(&serde_json::json!(2))
        );
        assert_eq!(
            outcome.index.as_ref().and_then(|index| index.get("state")),
            Some(&serde_json::json!("ready"))
        );
        // helper 写到注入的 scratch 目录且执行后被清理。
        let requests = fixture.runner.requests();
        let helper_invocation = requests
            .iter()
            .find(|request| request.argv.iter().any(|argument| argument == "--members"))
            .expect("helper must be invoked through the injected node");
        assert_eq!(helper_invocation.executable, fixture.node);
        let helper_path = PathBuf::from(&helper_invocation.argv[0]);
        assert!(
            helper_path.starts_with(&fixture.scratch),
            "helper must be written under the injected scratch dir: {}",
            helper_path.display()
        );
        assert!(
            !helper_path.exists(),
            "helper temp file must be cleaned up after execution"
        );
    }

    #[test]
    fn coverage_errors_sdk_not_found_when_no_candidate_exists() {
        let fixture = coverage_fixture();
        let coverage = CodeGraphCoverage::with_paths(
            fixture.runner.clone(),
            CoveragePaths::default(),
            Some(fixture.scratch.clone()),
        );

        let error = coverage
            .coverage(&fixture.root, &fixture.members())
            .unwrap_err();
        assert!(
            matches!(&error, AggregateIndexError::Failed { code, .. }
                if *code == "codegraph_sdk_not_found"),
            "expected codegraph_sdk_not_found, got: {error}"
        );
    }

    #[test]
    fn coverage_maps_helper_failure_to_coverage_failed() {
        let fixture = coverage_fixture();
        fixture.helper_stdout(
            r#"{"ok":false,"code":"open_failed","message":"CodeGraph not initialized"}"#,
        );
        fixture.helper_exit_code(1);

        let error = fixture
            .coverage()
            .coverage(&fixture.root, &fixture.members())
            .unwrap_err();
        assert!(
            matches!(&error, AggregateIndexError::Failed { code, .. }
                if *code == "codegraph_coverage_failed"
                    && error.to_string().contains("open_failed")
                    && error.to_string().contains("CodeGraph not initialized")),
            "expected codegraph_coverage_failed carrying helper code/message, got: {error}"
        );
    }

    #[test]
    fn coverage_maps_sdk_incompatible_helper_report() {
        let fixture = coverage_fixture();
        fixture.helper_stdout(
            r#"{"ok":false,"code":"sdk_incompatible","message":"no hasFilesUnder"}"#,
        );
        fixture.helper_exit_code(1);

        let error = fixture
            .coverage()
            .coverage(&fixture.root, &fixture.members())
            .unwrap_err();
        assert!(
            matches!(&error, AggregateIndexError::Failed { code, .. }
                if *code == "codegraph_sdk_incompatible"),
            "expected codegraph_sdk_incompatible, got: {error}"
        );
    }

    #[test]
    fn coverage_resolves_bundled_node_before_path_node() {
        let fixture = coverage_fixture();
        fixture.runner.node_version("v99.0.0");
        let path_node = executable_file(&fixture._temp.path().join("path-node"));
        let paths = CoveragePaths {
            path_node: Some(path_node),
            bundled_nodes: vec![fixture.node.clone()],
            ..fixture.paths()
        };

        let resolved =
            resolve_node_executable(&paths, &fixture.sdk, &fixture.runner_handle()).unwrap();

        assert_eq!(resolved, fixture.node);
        assert!(
            fixture.runner.requests().is_empty(),
            "bundled node must be adopted without a --version probe"
        );
    }

    #[test]
    fn coverage_rejects_path_node_below_minimum_version() {
        let fixture = coverage_fixture();
        let path_node = executable_file(&fixture._temp.path().join("path-node"));
        let paths = CoveragePaths {
            path_node: Some(path_node.clone()),
            bundled_nodes: Vec::new(),
            ..fixture.paths()
        };

        // 低于 22.5 ⇒ 不采用。
        fixture.runner.node_version("v18.20.0");
        let error =
            resolve_node_executable(&paths, &fixture.sdk, &fixture.runner_handle()).unwrap_err();
        assert!(
            matches!(&error, AggregateIndexError::Failed { code, .. }
                if *code == "codegraph_node_unavailable"),
            "node 18 must be rejected, got: {error}"
        );

        // 恰为 22.5 ⇒ 采用。
        fixture.runner.node_version("v22.5.0");
        let resolved =
            resolve_node_executable(&paths, &fixture.sdk, &fixture.runner_handle()).unwrap();
        assert_eq!(resolved, path_node);

        // 版本不可解析 ⇒ fail-closed 不采用。
        fixture.runner.node_version("version unknown");
        let error =
            resolve_node_executable(&paths, &fixture.sdk, &fixture.runner_handle()).unwrap_err();
        assert!(
            matches!(&error, AggregateIndexError::Failed { code, .. }
                if *code == "codegraph_node_unavailable"),
            "unparseable node version must be rejected, got: {error}"
        );
    }

    #[test]
    fn coverage_reports_node_unavailable_when_no_node() {
        let fixture = coverage_fixture();
        let paths = CoveragePaths {
            bundled_nodes: Vec::new(),
            path_node: None,
            ..fixture.paths()
        };

        let error =
            resolve_node_executable(&paths, &fixture.sdk, &fixture.runner_handle()).unwrap_err();
        assert!(
            matches!(&error, AggregateIndexError::Failed { code, message, .. }
                if *code == "codegraph_node_unavailable"
                    && message.contains("22.5")),
            "node-unavailable message must state the Node.js >= 22.5 requirement, got: {error}"
        );
    }

    // ---- S-C：自愈缓存根遵循 CODEGRAPH_INSTALL_DIR + 异平台兜底包校验 ----

    /// 本机可执行格式的 4 字节头。
    fn native_executable_header() -> &'static [u8] {
        #[cfg(target_os = "linux")]
        {
            b"\x7fELF"
        }
        #[cfg(target_os = "macos")]
        {
            b"\xcf\xfa\xed\xfe"
        }
        #[cfg(target_os = "windows")]
        {
            b"MZ\x90\x00"
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
        {
            b"\x7fELF"
        }
    }

    /// 保证**不等于**本机格式的头（模拟异平台残留包里的 node）。
    fn foreign_executable_header() -> &'static [u8] {
        #[cfg(target_os = "linux")]
        {
            b"MZ\x90\x00"
        }
        #[cfg(not(target_os = "linux"))]
        {
            b"\x7fELF"
        }
    }

    fn executable_with_bytes(path: &Path, bytes: &[u8]) -> PathBuf {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, bytes).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = std::fs::metadata(path).unwrap().permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(path, permissions).unwrap();
        }
        path.to_path_buf()
    }

    /// 与当前平台**不同**的目录名（异平台残留包的目录名）。
    fn foreign_platform_dir_name() -> String {
        if codegraph_platform_target() == "linux-x64" {
            "codegraph-darwin-arm64".to_string()
        } else {
            "codegraph-linux-x64".to_string()
        }
    }

    /// 自愈缓存根：上游 `npm-shim.js:113` / `npm-sdk.js:68` 是
    /// `CODEGRAPH_INSTALL_DIR || ~/.codegraph` ⇒ 环境变量优先，空值视为未设置。
    #[test]
    fn codegraph_cache_root_prefers_install_dir_over_home() {
        let home = Path::new("/home/dev");
        assert_eq!(
            codegraph_cache_root(Some(home), Some(PathBuf::from("/opt/codegraph"))),
            Some(PathBuf::from("/opt/codegraph")),
            "CODEGRAPH_INSTALL_DIR 必须优先于 ~/.codegraph"
        );
        assert_eq!(
            codegraph_cache_root(Some(home), None),
            Some(home.join(".codegraph")),
            "未设置时保持 ~/.codegraph 默认"
        );
        assert_eq!(
            codegraph_cache_root(Some(home), Some(PathBuf::new())),
            Some(home.join(".codegraph")),
            "空值视为未设置"
        );
        assert_eq!(codegraph_cache_root(None, None), None);
    }

    /// 指定缓存根（`CODEGRAPH_INSTALL_DIR`）下的自愈缓存必须命中。
    #[test]
    fn bundled_node_honors_codegraph_install_dir() {
        let temp = tempfile::tempdir().unwrap();
        let (_, sdk) = sdk_package_dir(&temp.path().join("prefix"));
        let install_dir = temp.path().join("cg-install");
        let cached = executable_under(
            &install_dir
                .join("bundles")
                .join(format!("{}-1.6.2", codegraph_platform_target())),
            node_executable_name(),
        );

        let candidates = bundled_node_candidates_with_cache_root(&sdk, Some(install_dir.clone()));

        assert!(
            candidates.contains(&cached),
            "CODEGRAPH_INSTALL_DIR 下的自愈缓存必须命中：{candidates:?}"
        );
    }

    /// 异平台残留包里的 node 在 `+x` 口径下同样「可执行」，但二进制格式不属于
    /// 本机 ⇒ 不得作兜底候选（否则必然 exec 失败、硬失败且绕过 CLI 回退）。
    #[test]
    fn platform_package_nodes_skips_foreign_package_without_native_binary() {
        let temp = tempfile::tempdir().unwrap();
        let scope = temp.path().join("@colbymchenry");
        let current = executable_under(&scope.join(platform_dir_name()), node_executable_name());
        let foreign = executable_with_bytes(
            &scope
                .join(foreign_platform_dir_name())
                .join(node_executable_name()),
            foreign_executable_header(),
        );

        let candidates = platform_package_nodes(&scope, &codegraph_platform_target());

        assert!(
            candidates.contains(&current),
            "当前平台包必须命中：{candidates:?}"
        );
        assert!(
            !candidates.contains(&foreign),
            "异平台残留包（二进制格式不符）不得作兜底候选：{candidates:?}"
        );
    }

    /// 兜底包的采用条件是**可执行性校验通过**而不是目录名：目录名不是当前
    /// 平台、但二进制确实是本机格式（手工放置 / 符号目录）⇒ 可以采用。
    #[test]
    fn platform_package_nodes_keeps_foreign_dir_with_native_binary() {
        let temp = tempfile::tempdir().unwrap();
        let scope = temp.path().join("@colbymchenry");
        let usable = executable_with_bytes(
            &scope
                .join(foreign_platform_dir_name())
                .join(node_executable_name()),
            native_executable_header(),
        );

        let candidates = platform_package_nodes(&scope, &codegraph_platform_target());

        assert!(
            candidates.contains(&usable),
            "本机格式的兜底包必须可以采用：{candidates:?}"
        );
    }

    #[test]
    fn parse_node_version_accepts_v_prefixed_triplets() {
        assert_eq!(parse_node_version("v22.5.0\n"), Some((22, 5, 0)));
        assert_eq!(parse_node_version("v24.16.0"), Some((24, 16, 0)));
        assert_eq!(
            parse_node_version("v24.17.0-nightly.1\n"),
            Some((24, 17, 0))
        );
        assert_eq!(parse_node_version("garbage"), None);
        assert_eq!(parse_node_version(""), None);
    }

    #[test]
    fn resolve_sdk_path_prefers_first_existing_candidate() {
        let fixture = coverage_fixture();
        // 注：fixture 的 install_prefix_sdk 真实存在 ⇒ 候选1 命中。
        let paths = fixture.paths();
        assert_eq!(
            resolve_sdk_path(&paths).as_deref(),
            Some(fixture.sdk.as_path())
        );

        // 候选1 缺席 ⇒ npm root -g 布局命中。
        let global_root = fixture._temp.path().join("global-node-modules");
        let global_sdk_dir = global_root.join("@colbymchenry/codegraph");
        std::fs::create_dir_all(&global_sdk_dir).unwrap();
        let global_sdk = global_sdk_dir.join("npm-sdk.js");
        std::fs::write(&global_sdk, "module.exports = {};").unwrap();
        let paths = CoveragePaths {
            install_prefix_sdk: Some(fixture._temp.path().join("missing/npm-sdk.js")),
            npm_global_root: Some(global_root),
            ..CoveragePaths::default()
        };
        assert_eq!(
            resolve_sdk_path(&paths).as_deref(),
            Some(global_sdk.as_path())
        );

        // 候选1/2 缺席 ⇒ codegraph 可执行真实路径反推命中。
        let prefix = fixture._temp.path().join("derived-prefix");
        let bin_dir = prefix
            .join("lib")
            .join("node_modules")
            .join("@colbymchenry")
            .join("codegraph")
            .join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        let codegraph_exe = bin_dir.join("codegraph");
        std::fs::write(&codegraph_exe, "#!/bin/sh\nexit 0\n").unwrap();
        let paths = CoveragePaths {
            install_prefix_sdk: None,
            npm_global_root: Some(fixture._temp.path().join("missing-node-modules")),
            codegraph_executable: Some(codegraph_exe.clone()),
            ..CoveragePaths::default()
        };
        let expected = prefix
            .join("lib")
            .join("node_modules")
            .join("@colbymchenry")
            .join("codegraph")
            .join("npm-sdk.js");
        std::fs::write(&expected, "module.exports = {};").unwrap();
        assert_eq!(
            resolve_sdk_path(&paths).as_deref(),
            Some(expected.as_path())
        );
    }
    include!("codegraph_coverage_tests.inc.rs");
}
