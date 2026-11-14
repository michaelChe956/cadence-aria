//! 只读诊断端点（`GET /api/providers/diagnostics`）。
//!
//! 客户机（Apple Silicon macOS）一次请求回答三问：**CLI 装到哪了** /
//! **为什么装完没生效** / **codegraph 平台包对不对**。
//!
//! 硬约束：
//! - **只读**：不触发安装、不跑版本探针、不查 registry 网络、不重建索引；
//!   结论只来自 durable 快照（provider health / 安装作业 / 聚合索引记录）、
//!   磁盘只读扫描与进程 env。
//! - **不泄密**：不回显凭据、token、模型配置文件内容、完整进程 env、作业
//!   stdout/stderr 正文；自由文本一律过 `sanitize_diagnostic` 脱敏。
//! - **不新增鉴权面**：路由挂在既有 router 上，继承 `enforce_api_auth` 的
//!   Host/Origin 来源校验（仅 `GET /api/health` 公开）。
//!
//! 拿不到的事实**如实表达「不可用」**（`null` + 原因），不用空值冒充。

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use axum::Json;
use axum::extract::State;
use chrono::Utc;
use serde_json::{Value, json};

use crate::cross_cutting::diagnostic_redaction::{RedactionContext, sanitize_diagnostic};
use crate::cross_cutting::path_overlay;
use crate::cross_cutting::provider_health::ProviderHealthSnapshot;
use crate::cross_cutting::semver::compare_semver;
use crate::product::app_paths::ProductAppPaths;
use crate::product::logical_codebase::aggregate_index::codegraph_coverage::{
    CoveragePaths, resolve_sdk_path,
};
use crate::product::logical_codebase::aggregate_index::{
    AggregateIndexRecord, AggregateIndexStatus, AggregateIndexStore,
};
use crate::product::provider_installation::InstallTarget;
use crate::product::provider_installation::effective_prefix::{
    first_path_dir_with_command, npm_package_dir, npm_prefix_from_real_path, prefix_bin_is_on_path,
    visible_cli_prefix,
};
use crate::product::provider_installation::job_store::{InstallJob, InstallJobStatus};
use crate::product::provider_installation::registry_version::VersionCacheEntry;
use crate::product::provider_installation::source_table::{
    INSTALL_SOURCE_TABLE, InstallSourceRow, install_source_label, kimi_install_bin_dir, npm_prefix,
};
use crate::web::state::WebAppState;

/// 诊断响应 schema 版本。
const DIAGNOSTICS_SCHEMA_VERSION: u32 = 1;

/// 最近安装作业条数上限（只报结论，不带 stdout/stderr 正文）。
const RECENT_INSTALL_JOB_LIMIT: usize = 20;

/// codegraph npm 包名（口径同 `codegraph_coverage::CODEGRAPH_NPM_PACKAGE`）。
const CODEGRAPH_NPM_PACKAGE: &str = "@colbymchenry/codegraph";

/// codegraph 平台包作用域目录（`@colbymchenry`）。
const CODEGRAPH_NPM_SCOPE: &str = "@colbymchenry";

/// 只读诊断端点：一次返回运行环境 / PATH overlay / 五类安装目标 /
/// codegraph 专项 / 最近安装作业 / provider health / 聚合索引状态。
pub async fn provider_diagnostics(State(state): State<WebAppState>) -> Json<Value> {
    Json(build_diagnostics(&state))
}

fn build_diagnostics(state: &WebAppState) -> Value {
    let paths = ProductAppPaths::new(state.workspace_root.join(".aria"));
    let health = state.provider_health.latest_diagnostic();
    let degraded = state.provider_health.degraded();
    // 服务进程实际生效的 PATH（overlay 合并后的真实生效序）。
    let service_path = path_overlay::effective_path(None);
    let login_path = path_overlay::login_shell_path();
    let version_cache = read_version_cache(&paths);

    json!({
        "schema_version": DIAGNOSTICS_SCHEMA_VERSION,
        "generated_at": Utc::now().to_rfc3339(),
        "runtime": runtime_section(),
        "path_overlay": path_overlay_section(&service_path),
        "install_targets": install_targets_section(
            &health,
            &service_path,
            login_path.as_deref(),
            &version_cache,
        ),
        "codegraph": codegraph_section(&service_path),
        "install_jobs": install_jobs_section(&paths),
        "provider_health": provider_health_section(&health, degraded),
        "aggregate_index": aggregate_index_section(&paths),
    })
}

// ---------------------------------------------------------------- 运行环境

fn runtime_section() -> Value {
    let process_path = std::env::var("PATH").unwrap_or_default();
    let segments = split_path_dirs(&process_path);
    json!({
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "platform_target": npm_platform_target(),
        "package_version": env!("CARGO_PKG_VERSION"),
        "git_sha": option_env!("ARIA_GIT_SHA").unwrap_or("unknown"),
        "path_segments": segments,
        "path_segment_count": segments.len(),
    })
}

// ------------------------------------------------------------- PATH overlay

fn path_overlay_section(service_path: &str) -> Value {
    json!({
        "registered": overlay_registered_entries(),
        "effective_path_segments": split_path_dirs(service_path),
    })
}

/// 已登记的 overlay 目录及其注册位置（`prepend` / `append`）。
///
/// `path_overlay` 只公开「目录列表」与「合并后 PATH」，未公开每条登记的
/// 位置；这里用**哨兵 base**反推：`effective_path(哨兵)` 的合并序为
/// `Prepend 目录… → 哨兵 → base 目录… → Append 目录…`，哨兵下标即
/// `Prepend` 条目数；`effective_path("")`（空 base）给出
/// `Prepend 目录… → Append 目录…` 的完整 overlay 序。
fn overlay_registered_entries() -> Vec<Value> {
    let snapshot = path_overlay::snapshot();
    if snapshot.is_empty() {
        return Vec::new();
    }
    let sentinel = PathBuf::from("\u{1}aria-diagnostics-path-probe");
    let probed = std::env::split_paths(&path_overlay::effective_path(Some(
        &sentinel.to_string_lossy(),
    )))
    .filter(|dir| !dir.as_os_str().is_empty())
    .collect::<Vec<PathBuf>>();
    let prepend_count = probed.iter().position(|dir| dir == &sentinel).unwrap_or(0);
    let merged = std::env::split_paths(&path_overlay::effective_path(Some("")))
        .filter(|dir| !dir.as_os_str().is_empty())
        .collect::<Vec<PathBuf>>();
    let prepend_dirs: HashSet<&PathBuf> = merged.iter().take(prepend_count).collect();
    snapshot
        .iter()
        .map(|dir| {
            json!({
                "dir": dir.display().to_string(),
                "position": if prepend_dirs.contains(dir) { "prepend" } else { "append" },
            })
        })
        .collect()
}

// ------------------------------------------------------------- 安装目标

fn install_targets_section(
    health: &ProviderHealthSnapshot,
    service_path: &str,
    login_path: Option<&str>,
    version_cache: &BTreeMap<String, VersionCacheEntry>,
) -> Vec<Value> {
    let probed = health_snapshot_probed(health);
    INSTALL_SOURCE_TABLE
        .iter()
        .map(|row| {
            install_target_entry(row, health, probed, service_path, login_path, version_cache)
        })
        .collect()
}

fn install_target_entry(
    row: &InstallSourceRow,
    health: &ProviderHealthSnapshot,
    probed: bool,
    service_path: &str,
    login_path: Option<&str>,
    version_cache: &BTreeMap<String, VersionCacheEntry>,
) -> Value {
    let mut reasons: Vec<String> = Vec::new();
    let provider = match InstallTarget::parse_wire(row.target_wire) {
        Some(InstallTarget::Provider(name)) => Some(name),
        _ => None,
    };
    let health_entry = provider.as_ref().and_then(|name| health.entry(name));
    let cli_path = first_path_dir_with_command(row.command, Some(service_path))
        .map(|dir| dir.join(row.command));
    // 探针快照已初始化 ⇒ 以探针结论为准（探针才是「装好且能跑」的判据）；
    // 未初始化 ⇒ 退回文件系统事实，并如实标注来源。
    let (cli_available, availability_source) = match (provider.is_some(), probed) {
        (true, true) => (
            health_entry.map(|entry| entry.available).unwrap_or(false),
            "provider_health",
        ),
        _ => (cli_path.is_some(), "filesystem"),
    };
    if !probed && provider.is_some() {
        reasons.push(
            "provider health 快照尚未初始化（服务启动未完成刷新）：可用性取自文件系统".to_string(),
        );
    }
    let prefix = resolve_install_prefix(row, service_path);
    let (local_version, local_version_source) = match provider {
        Some(_) => {
            let version = health_entry.and_then(|entry| entry.version.clone());
            (
                version.clone(),
                version.map(|_| "provider_health".to_string()),
            )
        }
        None => {
            let version = prefix
                .as_deref()
                .and_then(|prefix| package_version(prefix, row));
            (version.clone(), version.map(|_| "package_json".to_string()))
        }
    };
    let latest = row
        .version_package
        .and_then(|package| version_cache.get(package));
    let latest_version = latest.map(|entry| entry.latest_version.clone());
    let action = resolve_action(
        cli_available,
        local_version.is_some(),
        local_version.as_deref(),
        latest_version.as_deref(),
    );
    let (prefix_bin_on_login_path, prefix_bin_on_service_path) = match prefix.as_deref() {
        Some(prefix) => (
            prefix_bin_is_on_path(prefix, login_path),
            prefix_bin_is_on_path(prefix, Some(service_path)),
        ),
        None => (false, false),
    };
    if prefix.is_none() {
        reasons
            .push("未解析到安装前缀（CLI 不在服务进程 PATH 上，已知落点也没有对应包）".to_string());
    }
    if login_path.is_none() {
        reasons.push("登录 shell PATH 探测不可用（服务启动 ≤2s 内未完成或探测失败）".to_string());
    }
    if latest.is_none() {
        reasons.push("registry 版本缓存未命中（只读端点不查网络，最新版本未知）".to_string());
    }
    json!({
        "target": row.target_wire,
        "display_name": row.display_name,
        "command": row.command,
        "cli_available": cli_available,
        "availability_source": availability_source,
        "cli_path": cli_path.map(|path| path.display().to_string()),
        "local_version": local_version,
        "local_version_source": local_version_source,
        "latest_version": latest_version,
        "latest_version_source": latest.map(|_| "registry_cache".to_string()),
        "latest_checked_at": latest.map(|entry| entry.checked_at.to_rfc3339()),
        "action": action,
        "install_source": install_source_label(row),
        "install_prefix": prefix.as_ref().map(|prefix| prefix.display().to_string()),
        "prefix_bin_on_login_path": prefix_bin_on_login_path,
        "prefix_bin_on_service_path": prefix_bin_on_service_path,
        "probe": probe_dto(health_entry, probed),
        "reasons": reasons,
    })
}

fn probe_dto(
    entry: Option<&crate::cross_cutting::provider_health::ProviderHealthEntry>,
    probed: bool,
) -> Value {
    match entry {
        Some(entry) => json!({
            "probed": probed,
            "available": entry.available,
            "reason_code": entry
                .reason_code
                .and_then(|code| serde_json::to_value(code).ok())
                .and_then(|value| value.as_str().map(str::to_string)),
            "reason": entry.reason.as_ref().map(|reason| sanitize(reason)),
        }),
        None => json!({
            "probed": false,
            "available": null,
            "reason_code": null,
            "reason": "该对象不在 provider health 矩阵内（codegraph 用本地文件事实判定）",
        }),
    }
}

/// 按钮动作口径（与 `provider_installation` 的 `resolve_install_action` 同规：
/// 未安装 → `install`；已装但版本不可得 → `verify`；版本落后 → `update`；
/// 否则 `none`）。
fn resolve_action(
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

/// 解析安装前缀（装到哪了）：
/// ① PATH 上生效的 CLI 所在前缀；② 已知落点里已安装的 npm 包反推前缀；
/// ③ Kimi 官方脚本落点（`~/.kimi-code`）。
fn resolve_install_prefix(row: &InstallSourceRow, service_path: &str) -> Option<PathBuf> {
    if let Some(prefix) = visible_cli_prefix(row.command, Some(service_path)) {
        return Some(prefix);
    }
    if let Some(package) = row.npm_package {
        for candidate in known_prefix_candidates() {
            if npm_package_dir(&candidate, package).is_dir() {
                return Some(candidate);
            }
        }
    }
    if row.npm_package.is_none() {
        let bin_dir = kimi_install_bin_dir(&home_dir());
        if bin_dir.is_dir() {
            return bin_dir.parent().map(Path::to_path_buf);
        }
    }
    None
}

fn known_prefix_candidates() -> Vec<PathBuf> {
    let mut candidates = vec![npm_prefix()];
    let home = home_dir();
    if !home.as_os_str().is_empty() {
        let fallback = home.join(".npm-global");
        if !candidates.contains(&fallback) {
            candidates.push(fallback);
        }
    }
    candidates
}

fn package_version(prefix: &Path, row: &InstallSourceRow) -> Option<String> {
    let package = row.npm_package?;
    let manifest = npm_package_dir(prefix, package).join("package.json");
    let bytes = std::fs::read(manifest).ok()?;
    let value: Value = serde_json::from_slice(&bytes).ok()?;
    value
        .get("version")
        .and_then(|version| version.as_str())
        .map(str::to_string)
}

// ------------------------------------------------------------- codegraph

fn codegraph_section(service_path: &str) -> Value {
    let row = INSTALL_SOURCE_TABLE
        .iter()
        .find(|row| row.target_wire == "codegraph")
        .expect("codegraph row in install source table");
    let prefix = resolve_install_prefix(row, service_path);
    let cli_path = first_path_dir_with_command(row.command, Some(service_path))
        .map(|dir| dir.join(row.command));
    let local_version = prefix
        .as_deref()
        .and_then(|prefix| package_version(prefix, row))
        .or_else(|| {
            cli_path
                .as_ref()
                .and_then(|path| std::fs::canonicalize(path).ok())
                .and_then(|real| npm_prefix_from_real_path(&real))
                .and_then(|prefix| package_version(&prefix, row))
        });
    let sdk = resolve_sdk_path_candidate(prefix.as_deref(), cli_path.as_deref(), service_path);
    let platform = platform_package_section(sdk.as_deref());
    let node = resolve_node(sdk.as_deref(), service_path);
    json!({
        "cli_available": cli_path.is_some(),
        "cli_path": cli_path.map(|path| path.display().to_string()),
        "local_version": local_version,
        "local_version_source": local_version.map(|_| "package_json".to_string()),
        "install_prefix": prefix.as_ref().map(|prefix| prefix.display().to_string()),
        "sdk": {
            "available": sdk.is_some(),
            "path": sdk.as_ref().map(|path| path.display().to_string()),
            "reason": sdk
                .as_ref()
                .map(|_| "npm-sdk.js 已就位")
                .or(Some("未找到 npm-sdk.js（安装前缀 / npm 全局根 / CLI 真实路径反推均未命中）")),
        },
        "node": node,
        "platform": platform,
    })
}

/// SDK 入口（`npm-sdk.js`）解析：复用 `codegraph_coverage::resolve_sdk_path`
/// 的同一候选序（安装前缀 → npm 全局根 → CLI 真实路径反推），但不发起
/// `npm root -g` 探针（只读端点不跑子进程）。
fn resolve_sdk_path_candidate(
    prefix: Option<&Path>,
    cli_path: Option<&Path>,
    service_path: &str,
) -> Option<PathBuf> {
    let paths = CoveragePaths {
        install_prefix_sdk: prefix
            .map(|prefix| npm_package_dir(prefix, CODEGRAPH_NPM_PACKAGE).join("npm-sdk.js")),
        npm_global_root: prefix.map(|prefix| prefix.join("lib").join("node_modules")),
        codegraph_executable: cli_path.map(Path::to_path_buf),
        bundled_nodes: Vec::new(),
        path_node: first_path_dir_with_command("node", Some(service_path))
            .map(|dir| dir.join(node_executable_name())),
        home_dir: Some(home_dir()),
    };
    resolve_sdk_path(&paths)
}

/// 当前平台与 codegraph 平台包是否匹配（如 M 芯片上的
/// `codegraph-darwin-arm64` 是否就位、是否残留异平台包）。
fn platform_package_section(sdk: Option<&Path>) -> Value {
    let target = npm_platform_target();
    let expected_name = format!("codegraph-{target}");
    let scopes = platform_scope_dirs(sdk);
    let mut present_path = None;
    let mut foreign_packages: Vec<String> = Vec::new();
    for scope in &scopes {
        let Ok(entries) = std::fs::read_dir(scope) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.starts_with("codegraph-") {
                continue;
            }
            if name == expected_name {
                present_path.get_or_insert_with(|| entry.path().display().to_string());
            } else {
                foreign_packages.push(name);
            }
        }
    }
    foreign_packages.sort();
    foreign_packages.dedup();
    json!({
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "npm_target": target,
        "expected_package": expected_name,
        "package_present": present_path.is_some(),
        "package_path": present_path,
        "foreign_packages": foreign_packages,
        "scope_dirs": scopes
            .iter()
            .map(|scope| scope.display().to_string())
            .collect::<Vec<_>>(),
    })
}

/// `@colbymchenry` 作用域目录（nested 布局 + 每一层 hoisted `node_modules`）。
fn platform_scope_dirs(sdk: Option<&Path>) -> Vec<PathBuf> {
    let Some(sdk) = sdk else {
        return Vec::new();
    };
    let Some(package_root) = sdk.parent() else {
        return Vec::new();
    };
    let mut scopes = vec![package_root.join("node_modules").join(CODEGRAPH_NPM_SCOPE)];
    for ancestor in package_root.ancestors().skip(1) {
        if ancestor
            .file_name()
            .is_some_and(|name| name == "node_modules")
        {
            scopes.push(ancestor.join(CODEGRAPH_NPM_SCOPE));
        }
    }
    scopes
}

/// 自带 node 解析（来源口径：自带平台包 → 自愈缓存 → PATH）。
fn resolve_node(sdk: Option<&Path>, service_path: &str) -> Value {
    let target = npm_platform_target();
    let expected_name = format!("codegraph-{target}");
    for scope in platform_scope_dirs(sdk) {
        let candidate = scope.join(&expected_name).join(node_executable_name());
        if is_executable_file(&candidate) {
            return json!({
                "available": true,
                "path": candidate.display().to_string(),
                "source": "bundled_platform_package",
                "reason": null,
            });
        }
    }
    if let Some(candidate) = cached_bundle_node(&target) {
        return json!({
            "available": true,
            "path": candidate.display().to_string(),
            "source": "self_heal_cache",
            "reason": null,
        });
    }
    if let Some(dir) = first_path_dir_with_command("node", Some(service_path)) {
        let candidate = dir.join(node_executable_name());
        if is_executable_file(&candidate) {
            return json!({
                "available": true,
                "path": candidate.display().to_string(),
                "source": "path",
                "reason": null,
            });
        }
    }
    json!({
        "available": false,
        "path": null,
        "source": null,
        "reason": "未找到可执行的 node（平台包自带 node、自愈缓存、PATH 三处均无）",
    })
}

/// 自愈缓存里的自带 node（`<缓存根>/bundles/<target>-<version>/node`）。
fn cached_bundle_node(target: &str) -> Option<PathBuf> {
    let cache_root = std::env::var_os("CODEGRAPH_INSTALL_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            let home = home_dir();
            if home.as_os_str().is_empty() {
                None
            } else {
                Some(home.join(".codegraph"))
            }
        })?;
    let bundles = cache_root.join("bundles");
    let Ok(entries) = std::fs::read_dir(&bundles) else {
        return None;
    };
    let mut candidates: Vec<PathBuf> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.starts_with(&format!("{target}-")) {
                return None;
            }
            let node = entry.path().join(node_executable_name());
            is_executable_file(&node).then_some(node)
        })
        .collect();
    candidates.sort();
    candidates.pop()
}

// ------------------------------------------------------------- 安装作业

fn install_jobs_section(paths: &ProductAppPaths) -> Value {
    let jobs_root = paths.provider_installation_root().join("jobs");
    let mut jobs: Vec<InstallJob> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&jobs_root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
                continue;
            }
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };
            let Ok(job) = serde_json::from_slice::<InstallJob>(&bytes) else {
                continue;
            };
            jobs.push(job);
        }
    }
    jobs.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| right.job_id.cmp(&left.job_id))
    });
    let total = jobs.len();
    let recent = jobs
        .into_iter()
        .take(RECENT_INSTALL_JOB_LIMIT)
        .map(|job| {
            let status = match job.status {
                InstallJobStatus::Running => "running",
                InstallJobStatus::Completed => "completed",
                InstallJobStatus::Failed => "failed",
            };
            json!({
                "job_id": job.job_id,
                "target": job.target,
                "status": status,
                "installed_version": job
                    .outcome
                    .as_ref()
                    .and_then(|outcome| outcome.installed_version.clone()),
                "path_hint": job
                    .outcome
                    .as_ref()
                    .and_then(|outcome| outcome.path_hint.clone()),
                "failure_code": job.failure.as_ref().and_then(|failure| {
                    serde_json::to_value(failure.kind)
                        .ok()
                        .and_then(|value| value.as_str().map(str::to_string))
                }),
                "failure_message": job
                    .failure
                    .as_ref()
                    .map(|failure| sanitize(&failure.message)),
                "manual_command": job
                    .failure
                    .as_ref()
                    .map(|failure| failure.manual_command.clone()),
                "created_at": job.created_at.to_rfc3339(),
                "updated_at": job.updated_at.to_rfc3339(),
            })
        })
        .collect::<Vec<_>>();
    json!({
        "jobs_root": jobs_root.display().to_string(),
        "total": total,
        "recent": recent,
    })
}

// --------------------------------------------------------- provider health

fn provider_health_section(health: &ProviderHealthSnapshot, degraded: bool) -> Value {
    json!({
        "probed": health_snapshot_probed(health),
        "checked_at": health.checked_at.to_rfc3339(),
        "generation": health.generation,
        "degraded": degraded,
        "real_workflow_blocked": health.real_workflow_blocked(),
        "providers": health
            .providers
            .iter()
            .map(|entry| {
                json!({
                    "provider": serde_json::to_value(&entry.provider)
                        .ok()
                        .and_then(|value| value.as_str().map(str::to_string)),
                    "command": entry.command,
                    "available": entry.available,
                    "version": entry.version,
                    "reason_code": entry
                        .reason_code
                        .and_then(|code| serde_json::to_value(code).ok())
                        .and_then(|value| value.as_str().map(str::to_string)),
                    "reason": entry.reason.as_ref().map(|reason| sanitize(reason)),
                    "checked_at": entry.checked_at.to_rfc3339(),
                })
            })
            .collect::<Vec<_>>(),
    })
}

// ------------------------------------------------------------ 聚合索引

fn aggregate_index_section(paths: &ProductAppPaths) -> Value {
    let mut entries: Vec<Value> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut last_failure: Option<(String, String, String)> = None;
    for project_id in directory_names(&paths.projects_root()) {
        for (scope_id, scope_result) in aggregate_scopes(paths, &project_id) {
            match scope_result {
                Ok(records) => {
                    let fresh: Vec<&AggregateIndexRecord> = records
                        .iter()
                        .filter(|record| seen.insert(record.aggregate_index_id.clone()))
                        .collect();
                    if fresh.is_empty() {
                        continue;
                    }
                    let (state, representative) = aggregate_scope_state(&fresh);
                    for record in &fresh {
                        if matches!(
                            record.status,
                            AggregateIndexStatus::Failed | AggregateIndexStatus::Degraded
                        ) && let Some(warning) = record.warning.as_ref()
                        {
                            let code = if record.status == AggregateIndexStatus::Failed {
                                "aggregate_index_failed"
                            } else {
                                "aggregate_index_degraded"
                            };
                            let candidate = (
                                record.updated_at.clone(),
                                code.to_string(),
                                sanitize(warning),
                            );
                            let replace = last_failure
                                .as_ref()
                                .map(|(updated, _, _)| {
                                    record.updated_at.as_str() > updated.as_str()
                                })
                                .unwrap_or(true);
                            if replace {
                                last_failure = Some(candidate);
                            }
                        }
                    }
                    entries.push(json!({
                        "project_id": project_id,
                        "logical_codebase_id": scope_id,
                        "state": state,
                        "aggregate_index_id": representative
                            .map(|record| record.aggregate_index_id.clone()),
                        "membership_revision": representative
                            .map(|record| record.membership_revision),
                        "codegraph_version": representative
                            .map(|record| record.codegraph_version.clone()),
                        "member_count": representative
                            .map(|record| record.member_snapshots.len()),
                        "updated_at": representative.map(|record| record.updated_at.clone()),
                        "warning": representative
                            .and_then(|record| record.warning.as_ref())
                            .map(|warning| sanitize(warning)),
                    }));
                }
                Err(reason) => {
                    entries.push(json!({
                        "project_id": project_id,
                        "logical_codebase_id": scope_id,
                        "state": "unknown",
                        "aggregate_index_id": null,
                        "membership_revision": null,
                        "codegraph_version": null,
                        "member_count": null,
                        "updated_at": null,
                        "warning": sanitize(&reason),
                    }));
                }
            }
        }
    }
    json!({
        "available": !entries.is_empty(),
        "reason": if entries.is_empty() {
            Some("未找到任何聚合索引记录（该工作区没有项目/逻辑代码库索引世代）".to_string())
        } else {
            None
        },
        "entries": entries,
        "last_failure": last_failure.map(|(_, code, message)| json!({
            "code": code,
            "message": message,
        })),
    })
}

/// 一个项目下的聚合索引作用域：每个逻辑代码库一个 + 项目级 legacy 一个。
fn aggregate_scopes(
    paths: &ProductAppPaths,
    project_id: &str,
) -> Vec<(String, Result<Vec<AggregateIndexRecord>, String>)> {
    let mut scopes: Vec<(String, Result<Vec<AggregateIndexRecord>, String>)> = Vec::new();
    for lc_id in directory_names(&paths.logical_codebases_root(project_id)) {
        let records = AggregateIndexStore::for_lc(paths.clone(), lc_id.clone())
            .records(project_id)
            .map_err(|error| error.to_string());
        scopes.push((lc_id, records));
    }
    let legacy = AggregateIndexStore::new(paths.clone())
        .records(project_id)
        .map_err(|error| error.to_string());
    scopes.push(("legacy".to_string(), legacy));
    scopes
}

/// 作用域状态（口径同 `aggregate_index` handler 的 active 投影）。
fn aggregate_scope_state<'a>(
    records: &[&'a AggregateIndexRecord],
) -> (String, Option<&'a AggregateIndexRecord>) {
    let mut sorted: Vec<&AggregateIndexRecord> = records.to_vec();
    sorted.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
    let degraded = sorted
        .iter()
        .find(|record| record.status == AggregateIndexStatus::Degraded);
    if let Some(record) = degraded {
        return ("degraded".to_string(), Some(record));
    }
    let Some(latest) = sorted.first().copied() else {
        return ("missing".to_string(), None);
    };
    let state = match latest.status {
        AggregateIndexStatus::Active => "active",
        AggregateIndexStatus::Stale => "stale",
        AggregateIndexStatus::Building => "rebuilding",
        AggregateIndexStatus::Degraded => "degraded",
        AggregateIndexStatus::Failed => "failed",
        AggregateIndexStatus::Superseded => "missing",
    };
    (state.to_string(), Some(latest))
}

// ------------------------------------------------------------------ 工具

fn split_path_dirs(value: &str) -> Vec<String> {
    std::env::split_paths(value)
        .filter(|dir| !dir.as_os_str().is_empty())
        .map(|dir| dir.display().to_string())
        .collect()
}

fn directory_names(root: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter(|entry| entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| !name.starts_with('.'))
        .collect();
    names.sort();
    names
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_default()
}

fn sanitize(value: &str) -> String {
    sanitize_diagnostic(value, &RedactionContext::default())
}

fn is_executable_file(path: &Path) -> bool {
    path.is_file() && is_executable(path)
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

fn node_executable_name() -> &'static str {
    if cfg!(windows) { "node.exe" } else { "node" }
}

/// npm 的 `os`/`cpu` 拼写（`darwin` / `win32` / `linux` × `x64` / `arm64`）——
/// 口径同 `codegraph_coverage` 的平台包目录名（该函数为模块私有，此处按同一
/// 拼写规则复刻，供诊断端点判断「平台包对不对」）。
fn npm_platform_target() -> String {
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

/// provider health 快照是否已真正刷新过（generation 0 = 未初始化）。
fn health_snapshot_probed(health: &ProviderHealthSnapshot) -> bool {
    health.generation > 0 && health.checked_at.timestamp() > 0
}

/// registry 版本缓存（**只读缓存**，不查网络）。
fn read_version_cache(paths: &ProductAppPaths) -> BTreeMap<String, VersionCacheEntry> {
    let path = paths.provider_version_cache_path();
    let Ok(bytes) = std::fs::read(&path) else {
        return BTreeMap::new();
    };
    let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
        return BTreeMap::new();
    };
    let Some(entries) = value.get("entries").and_then(|entries| entries.as_object()) else {
        return BTreeMap::new();
    };
    entries
        .iter()
        .filter_map(|(package, entry)| {
            serde_json::from_value::<VersionCacheEntry>(entry.clone())
                .ok()
                .map(|parsed| (package.clone(), parsed))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use axum::Json;
    use axum::body::Body;
    use axum::extract::State;
    use axum::http::{Request, StatusCode};
    use chrono::Utc;
    use serde_json::Value;
    use tempfile::tempdir;
    use tower::ServiceExt;

    use super::{build_diagnostics, provider_diagnostics};
    use crate::cross_cutting::path_overlay::{self, OverlayPosition};
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::provider_installation::failure_classify::{
        InstallFailureKind, InstallJobFailure,
    };
    use crate::product::provider_installation::job_store::InstallJobStore;
    use crate::web::runtime::WebRuntime;
    use crate::web::state::WebAppState;

    fn state(root: &Path) -> WebAppState {
        let mut state =
            WebAppState::new(root.to_path_buf(), WebRuntime::new_fake(root.to_path_buf()));
        state.test_provider_enabled = false;
        state
    }

    /// 响应里绝不能出现的机密类字段名/片段。
    const FORBIDDEN_FRAGMENTS: [&str; 8] = [
        "ANTHROPIC_API_KEY",
        "OPENAI_API_KEY",
        "CLAUDE_CONFIG_DIR",
        "CODEX_HOME",
        "api_key",
        "Authorization",
        "stdout",
        "stderr",
    ];

    fn collect_keys(value: &Value, keys: &mut Vec<String>) {
        match value {
            Value::Object(map) => {
                for (key, child) in map {
                    keys.push(key.clone());
                    collect_keys(child, keys);
                }
            }
            Value::Array(items) => {
                for item in items {
                    collect_keys(item, keys);
                }
            }
            _ => {}
        }
    }

    #[tokio::test]
    async fn diagnostics_response_carries_every_section_and_no_secrets() {
        let root = tempdir().expect("root");
        let Json(body) = provider_diagnostics(State(state(root.path()))).await;

        for section in [
            "runtime",
            "path_overlay",
            "install_targets",
            "codegraph",
            "install_jobs",
            "provider_health",
            "aggregate_index",
        ] {
            assert!(body.get(section).is_some(), "缺少诊断分段：{section}");
        }

        // 运行环境：os / arch / PATH 分段数组（不拼成字符串）。
        assert!(body["runtime"]["os"].is_string());
        assert!(body["runtime"]["arch"].is_string());
        assert!(body["runtime"]["path_segments"].is_array());
        assert!(body["runtime"]["platform_target"].is_string());

        // PATH overlay：登记项带位置 + 合并后的有效 PATH 分段。
        assert!(body["path_overlay"]["registered"].is_array());
        assert!(body["path_overlay"]["effective_path_segments"].is_array());

        // 五个安装目标各带必填字段。
        let targets = body["install_targets"].as_array().expect("targets array");
        assert_eq!(targets.len(), 5, "五类安装目标齐备");
        for target in targets {
            for field in [
                "target",
                "cli_available",
                "local_version",
                "latest_version",
                "action",
                "install_source",
                "install_prefix",
                "prefix_bin_on_login_path",
                "prefix_bin_on_service_path",
            ] {
                assert!(target.get(field).is_some(), "目标缺字段 {field}");
            }
        }

        // codegraph 专项：CLI / SDK / node / 平台包匹配。
        for field in [
            "cli_available",
            "cli_path",
            "local_version",
            "sdk",
            "node",
            "platform",
        ] {
            assert!(
                body["codegraph"].get(field).is_some(),
                "codegraph 缺字段 {field}"
            );
        }
        assert!(body["codegraph"]["platform"]["npm_target"].is_string());
        assert!(body["codegraph"]["platform"]["expected_package"].is_string());

        // provider health 快照：四家 provider。
        let providers = body["provider_health"]["providers"]
            .as_array()
            .expect("providers array");
        assert_eq!(providers.len(), 4, "四家 provider 齐备");

        // 不泄密：既没有机密片段，也没有机密类字段名。
        let serialized = serde_json::to_string(&body).expect("serialized body");
        for fragment in FORBIDDEN_FRAGMENTS {
            assert!(
                !serialized.contains(fragment),
                "响应泄漏机密片段：{fragment}"
            );
        }
        let mut keys = Vec::new();
        collect_keys(&body, &mut keys);
        for key in keys {
            let lowered = key.to_ascii_lowercase();
            assert!(
                !(lowered.contains("token")
                    || lowered.contains("secret")
                    || lowered.contains("password")
                    || lowered == "env"
                    || lowered == "environment"),
                "响应含机密类字段名：{key}"
            );
        }
    }

    #[tokio::test]
    async fn diagnostics_reports_overlay_registration_position() {
        let root = tempdir().expect("root");
        let bin_dir = root.path().join("overlay-bin");
        std::fs::create_dir_all(&bin_dir).expect("overlay bin dir");
        path_overlay::register(&bin_dir, OverlayPosition::Prepend);

        let body = build_diagnostics(&state(root.path()));
        let registered = body["path_overlay"]["registered"]
            .as_array()
            .expect("registered array");
        let entry = registered
            .iter()
            .find(|entry| entry["dir"] == bin_dir.display().to_string())
            .expect("登记项含 overlay 目录");
        assert_eq!(entry["position"], "prepend");
        let effective = body["path_overlay"]["effective_path_segments"]
            .as_array()
            .expect("effective segments");
        assert!(
            effective.contains(&Value::from(bin_dir.display().to_string())),
            "合并后的有效 PATH 应含 prepend 目录"
        );
    }

    #[tokio::test]
    async fn diagnostics_lists_recent_install_jobs_with_failure_code_only() {
        let root = tempdir().expect("root");
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        let store = InstallJobStore::new(paths.provider_installation_root().join("jobs"));
        let job = store.create("codegraph", Utc::now()).expect("create job");
        store
            .fail(
                &job.job_id,
                InstallJobFailure {
                    kind: InstallFailureKind::Network,
                    message: "registry 不可达：npm ERR! network timeout".to_string(),
                    manual_command: "npm install -g --registry=https://registry.npmmirror.com \
                     @colbymchenry/codegraph"
                        .to_string(),
                },
                Utc::now(),
            )
            .expect("fail job");

        let body = build_diagnostics(&state(root.path()));
        let recent = body["install_jobs"]["recent"]
            .as_array()
            .expect("recent jobs");
        assert_eq!(recent.len(), 1);
        let entry = &recent[0];
        assert_eq!(entry["target"], "codegraph");
        assert_eq!(entry["status"], "failed");
        assert_eq!(entry["failure_code"], "network");
        assert!(entry["failure_message"].is_string());
        assert!(entry["installed_version"].is_null());
        let serialized = serde_json::to_string(entry).expect("serialized job");
        assert!(!serialized.contains("stdout"));
        assert!(!serialized.contains("stderr"));
    }

    #[tokio::test]
    async fn diagnostics_marks_unavailable_facts_instead_of_faking_them() {
        let root = tempdir().expect("root");
        let body = build_diagnostics(&state(root.path()));

        // 登录 shell PATH 探测不可用 ⇒ 如实表达「不可用」，不冒充在 PATH 上。
        let targets = body["install_targets"].as_array().expect("targets array");
        for target in targets {
            assert_eq!(target["prefix_bin_on_login_path"], Value::Bool(false));
        }
        // 聚合索引没有任何记录 ⇒ 明确给出原因而不是空数组冒充「正常」。
        assert_eq!(body["aggregate_index"]["available"], Value::Bool(false));
        assert!(body["aggregate_index"]["reason"].is_string());
        // provider health 未刷新 ⇒ 明确标注「尚未初始化」。
        assert_eq!(body["provider_health"]["probed"], Value::Bool(false));
    }

    #[tokio::test]
    async fn diagnostics_route_is_mounted_and_answers() {
        let root = tempdir().expect("root");
        let app = crate::web::app::build_web_router(state(root.path()));
        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/providers/diagnostics")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::OK);
    }
}
