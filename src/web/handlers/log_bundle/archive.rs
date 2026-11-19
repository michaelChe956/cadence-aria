//! 诊断日志打包（T3）：runtime 信息原值、`$HOME/.aria/projects` 两类聚合
//! 树 JSON 与每家最近三个会话的轨迹投影，组装为私有临时文件中的 gzip tar。
//!
//! - 只读普通文件（经 `fs::Root` 安全打开），输出成员均为固定权限的普通
//!   文件与安全相对路径，无属主信息、无绝对路径。
//! - 输入/输出与扫描均有界（128 MiB 总预算 + 每类基本份额），超限、失败、
//!   截断都以稳定原因记入 `manifest.json`，局部交付不冒充完整。
//! - 会话正文只经 `trace::project` 白名单投影，坏行/巨行不得原文回退。
//! - 完成后 gzip/tar 收尾并把返回的文件 seek 到开头；错误路径的暂存件随
//!   FD 关闭回收，不以未完成的压缩包冒充成功。

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use flate2::Compression;
use flate2::write::GzEncoder;
use serde_json::{Map, Value, json};

use super::fs::{Kind, Root};
use super::sessions::{self, Session};
use super::trace;

/// 聚合索引世代的受限投影（诊断摘要，不是原始 JSON）。
#[path = "aggregate_projection.rs"]
mod aggregate_projection;

const BUNDLE_VERSION: u64 = 1;
pub(super) const TOTAL_BUDGET_BYTES: u64 = 128 * 1024 * 1024;
pub(super) const RUNTIME_INFO_OUTPUT_BYTES: u64 = 1024 * 1024;
pub(super) const MANIFEST_OUTPUT_BYTES: u64 = 2 * 1024 * 1024;
pub(super) const ARIA_INPUT_CLASS_BYTES: u64 = 32 * 1024 * 1024;
pub(super) const ARIA_OUTPUT_CLASS_BYTES: u64 = 36 * 1024 * 1024;
const ARIA_MEMBER_MAX_BYTES: u64 = 8 * 1024 * 1024;
const TRACE_TAIL_WINDOW_BYTES: u64 = 8 * 1024 * 1024;
pub(super) const TRACE_INPUT_PER_PROVIDER_BYTES: u64 = 24 * 1024 * 1024;
pub(super) const TRACE_OUTPUT_PER_PROVIDER_BYTES: u64 = 4 * 1024 * 1024;
pub(super) const MAX_LINE_BYTES: usize = 1024 * 1024;
const SCAN_ENTRY_BUDGET: usize = 16_384;
const MAX_TREE_DEPTH: usize = 12;
const MAX_SESSIONS_PER_PROVIDER: usize = 3;
const MEMBER_MODE: u32 = 0o600;
const REDACTED: &str = "[REDACTED]";
/// `$HOME/.aria/projects` 下仅收录这两种目录分量树内的普通 `.json`。
const ARIA_TREES: [&str; 2] = ["aggregate-initializations", "aggregate-recipe-receipts"];
/// 聚合索引世代文件只认这两种布局的**直接**子文件（相对 `$HOME/.aria/projects`）。
const GENERATION_LAYOUTS: [&str; 2] = [
    "<project>/logical-codebase/aggregate-indexes/<id>.json",
    "<project>/logical-codebases/<lc>/aggregate-indexes/<id>.json",
];

type Bundle = tar::Builder<GzEncoder<File>>;

pub(super) struct ProviderDef {
    /// wire 名（下划线），用于 `providers/<wire>/...` 成员与 T2 分发。
    pub(super) slug: &'static str,
    /// 直接指向会话根的环境变量（Pi 专用，优先于 home_env）。
    pub(super) sessions_env: Option<&'static str>,
    /// 指向 provider 主目录的环境变量，会话根 = <dir>/<sessions_subdir>。
    pub(super) home_env: Option<&'static str>,
    /// HOME 下的默认主目录（Pi 含 agent 层）。
    pub(super) default_dir: &'static str,
    pub(super) sessions_subdir: &'static str,
    /// 进程环境未给覆盖时，是否回落到启动期已缓存的登录 shell 探测值。
    ///
    /// 只有 Pi 需要：启动 Pi 用的是同一份启动环境（进程 env → 登录 shell
    /// 探测 → 默认根），采集侧必须复刻同一优先级，否则打包的会话根与
    /// 实际跑 Pi 的根不是同一个。其余 provider 不改既有选根行为。
    pub(super) use_shell_probe_root: bool,
}

pub(super) const PROVIDERS: [ProviderDef; 4] = [
    ProviderDef {
        slug: "claude_code",
        sessions_env: None,
        home_env: Some("CLAUDE_CONFIG_DIR"),
        default_dir: ".claude",
        sessions_subdir: "projects",
        use_shell_probe_root: false,
    },
    ProviderDef {
        slug: "codex",
        sessions_env: None,
        home_env: Some("CODEX_HOME"),
        default_dir: ".codex",
        sessions_subdir: "sessions",
        use_shell_probe_root: false,
    },
    ProviderDef {
        slug: "pi",
        sessions_env: Some("PI_CODING_AGENT_SESSION_DIR"),
        home_env: Some("PI_CODING_AGENT_DIR"),
        default_dir: ".pi/agent",
        sessions_subdir: "sessions",
        use_shell_probe_root: true,
    },
    ProviderDef {
        slug: "kimi_code",
        sessions_env: None,
        home_env: Some("KIMI_CODE_HOME"),
        default_dir: ".kimi-code",
        sessions_subdir: "sessions",
        use_shell_probe_root: false,
    },
];

pub(super) const SOURCE_ENV: &str = "env";
pub(super) const SOURCE_DEFAULT_HOME: &str = "default_home";
const SOURCE_INVALID_ENV: &str = "invalid_env";

/// 单个 provider 原生会话根的解析结果（覆盖或 HOME 默认）。
#[derive(Clone)]
pub(super) struct ProviderRoot {
    pub(super) path: Option<PathBuf>,
    pub(super) source: &'static str,
    /// 无效覆盖（相对路径等）的变量名：跳过该家，不回退默认根。
    pub(super) invalid_var: Option<&'static str>,
}

/// 测试可注入的宿主根：`from_host` 从环境解析，测试直接构造。
#[derive(Clone)]
pub(super) struct HostPaths {
    pub(super) home: Option<PathBuf>,
    pub(super) claude_code: ProviderRoot,
    pub(super) codex: ProviderRoot,
    pub(super) pi: ProviderRoot,
    pub(super) kimi_code: ProviderRoot,
}

impl HostPaths {
    /// 测试便捷构造：全部 provider 走 HOME 默认根。
    #[cfg(test)]
    pub(super) fn from_home_default(home: &Path) -> Self {
        let default = |dir: &str, sub: &str| ProviderRoot {
            path: Some(home.join(dir).join(sub)),
            source: SOURCE_DEFAULT_HOME,
            invalid_var: None,
        };
        HostPaths {
            home: Some(home.to_path_buf()),
            claude_code: default(".claude", "projects"),
            codex: default(".codex", "sessions"),
            pi: default(".pi/agent", "sessions"),
            kimi_code: default(".kimi-code", "sessions"),
        }
    }

    /// 从宿主环境解析（只在生产路径调用，测试不得设置全局环境）。
    fn from_env() -> Self {
        let home = nonempty_env_path("HOME").filter(|p| p.is_absolute());
        let resolve = |def: &ProviderDef| -> ProviderRoot {
            resolve_provider_root(
                def,
                home.as_deref(),
                &|var: &str| std::env::var_os(var).map(PathBuf::from),
                &|var: &str| {
                    // 启动期已缓存的登录 shell 探测值：采集时只读缓存，不再启动 shell。
                    if def.use_shell_probe_root {
                        crate::cross_cutting::path_overlay::shell_env(var).map(PathBuf::from)
                    } else {
                        None
                    }
                },
            )
        };
        HostPaths {
            claude_code: resolve(&PROVIDERS[0]),
            codex: resolve(&PROVIDERS[1]),
            pi: resolve(&PROVIDERS[2]),
            kimi_code: resolve(&PROVIDERS[3]),
            home,
        }
    }

    fn provider_root(&self, idx: usize) -> &ProviderRoot {
        match idx {
            0 => &self.claude_code,
            1 => &self.codex,
            2 => &self.pi,
            _ => &self.kimi_code,
        }
    }
}

/// provider 会话根的解析（与启动 provider 的环境优先级一致）。
///
/// 优先级：会话目录变量 > 主目录变量；每级内部是「进程环境 > 启动期已缓存
/// 的登录 shell 探测值」；都没有才用 HOME 默认根。相对路径等无效覆盖一律
/// 失败关闭（记 `invalid_env`，不回退默认根，也不换另一家根）。
pub(super) fn resolve_provider_root(
    def: &ProviderDef,
    home: Option<&Path>,
    process_value: &dyn Fn(&str) -> Option<PathBuf>,
    shell_value: &dyn Fn(&str) -> Option<PathBuf>,
) -> ProviderRoot {
    for (var, join_subdir) in [(def.sessions_env, false), (def.home_env, true)] {
        let Some(var) = var else { continue };
        let Some(value) = override_value(def, var, process_value, shell_value) else {
            continue;
        };
        let path = if join_subdir {
            value.join(def.sessions_subdir)
        } else {
            value
        };
        return if path.is_absolute() {
            ProviderRoot {
                path: Some(path),
                source: SOURCE_ENV,
                invalid_var: None,
            }
        } else {
            ProviderRoot {
                path: None,
                source: SOURCE_INVALID_ENV,
                invalid_var: Some(var),
            }
        };
    }
    match home {
        Some(home) => ProviderRoot {
            path: Some(home.join(def.default_dir).join(def.sessions_subdir)),
            source: SOURCE_DEFAULT_HOME,
            invalid_var: None,
        },
        None => ProviderRoot {
            path: None,
            source: SOURCE_DEFAULT_HOME,
            invalid_var: None,
        },
    }
}

/// 覆盖值：进程环境优先，其次启动期已缓存的登录 shell 探测值。
///
/// 显式空值不指定路径，但仍压住同名 shell 探测值，与子进程启动器一致。
fn override_value(
    def: &ProviderDef,
    var: &str,
    process_value: &dyn Fn(&str) -> Option<PathBuf>,
    shell_value: &dyn Fn(&str) -> Option<PathBuf>,
) -> Option<PathBuf> {
    if let Some(value) = process_value(var) {
        return (!value.as_os_str().is_empty()).then_some(value);
    }
    if !def.use_shell_probe_root {
        return None;
    }
    shell_value(var).filter(|value| !value.as_os_str().is_empty())
}

fn nonempty_env_path(var: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// 输入/输出双轨预算：总量 128 MiB，叠加每类基本份额；授予不足即记
/// 首个耗尽原因，供 manifest 如实上报。
#[derive(Default)]
pub(super) struct Budget {
    total_input_used: u64,
    total_output_used: u64,
    aria_input_used: u64,
    aria_output_used: u64,
    trace_input_used: [u64; 4],
    trace_output_used: [u64; 4],
    runtime_output_used: u64,
    manifest_output_used: u64,
    exhausted: Option<&'static str>,
}

impl Budget {
    fn grant(
        requested: u64,
        class_used: &mut u64,
        class_cap: u64,
        total_used: &mut u64,
        exhausted: &mut Option<&'static str>,
    ) -> u64 {
        let class_left = class_cap.saturating_sub(*class_used);
        let total_left = TOTAL_BUDGET_BYTES.saturating_sub(*total_used);
        let granted = requested.min(class_left).min(total_left);
        *class_used += granted;
        *total_used += granted;
        if granted < requested && exhausted.is_none() {
            *exhausted = Some("budget_exhausted");
        }
        granted
    }

    pub(super) fn grant_aria_input(&mut self, requested: u64) -> u64 {
        let Budget {
            total_input_used,
            aria_input_used,
            exhausted,
            ..
        } = self;
        Self::grant(
            requested,
            aria_input_used,
            ARIA_INPUT_CLASS_BYTES,
            total_input_used,
            exhausted,
        )
    }

    pub(super) fn grant_aria_output(&mut self, requested: u64) -> u64 {
        let Budget {
            total_output_used,
            aria_output_used,
            exhausted,
            ..
        } = self;
        Self::grant(
            requested,
            aria_output_used,
            ARIA_OUTPUT_CLASS_BYTES,
            total_output_used,
            exhausted,
        )
    }

    pub(super) fn grant_trace_input(&mut self, idx: usize, requested: u64) -> u64 {
        let Budget {
            total_input_used,
            trace_input_used,
            exhausted,
            ..
        } = self;
        Self::grant(
            requested,
            &mut trace_input_used[idx],
            TRACE_INPUT_PER_PROVIDER_BYTES,
            total_input_used,
            exhausted,
        )
    }

    pub(super) fn grant_trace_output(&mut self, idx: usize, requested: u64) -> u64 {
        let Budget {
            total_output_used,
            trace_output_used,
            exhausted,
            ..
        } = self;
        Self::grant(
            requested,
            &mut trace_output_used[idx],
            TRACE_OUTPUT_PER_PROVIDER_BYTES,
            total_output_used,
            exhausted,
        )
    }

    pub(super) fn grant_runtime_output(&mut self, requested: u64) -> u64 {
        let Budget {
            total_output_used,
            runtime_output_used,
            exhausted,
            ..
        } = self;
        Self::grant(
            requested,
            runtime_output_used,
            RUNTIME_INFO_OUTPUT_BYTES,
            total_output_used,
            exhausted,
        )
    }

    pub(super) fn grant_manifest_output(&mut self, requested: u64) -> u64 {
        let Budget {
            total_output_used,
            manifest_output_used,
            exhausted,
            ..
        } = self;
        Self::grant(
            requested,
            manifest_output_used,
            MANIFEST_OUTPUT_BYTES,
            total_output_used,
            exhausted,
        )
    }

    #[cfg(test)]
    pub(super) fn exhausted_reason(&self) -> Option<&'static str> {
        self.exhausted
    }
}

/// 从宿主环境采集：runtime 原值 + `$HOME/.aria/projects` 两类聚合树 +
/// 每家最近三会话的轨迹投影。返回已完成 gzip tar 写入并 seek(0) 的私有
/// 临时文件（tempfile 0600，随 FD 关闭回收）。
pub(super) fn from_host(runtime: Value) -> io::Result<File> {
    let host = HostPaths::from_env();
    build(runtime, &host)
}

pub(super) fn build(runtime: Value, host: &HostPaths) -> io::Result<File> {
    build_with_sessions(runtime, host, &[])
}

/// `injected` 仅供测试：按 wire 名覆盖 `sessions::discover` 的结果，
/// 仍走同一条打包管线（预算、清单、成员头一致）。
pub(super) fn build_with_sessions(
    runtime: Value,
    host: &HostPaths,
    injected: &[(&'static str, Vec<Session>)],
) -> io::Result<File> {
    let now = chrono::Utc::now();
    let mtime = now.timestamp().max(0) as u64;
    let utc = now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let mut budget = Budget::default();
    // 私有临时文件（0600）：成功路径 seek(0) 返回，失败路径随 FD 关闭回收。
    let encoder = GzEncoder::new(tempfile::tempfile()?, Compression::default());
    let mut bundle: Bundle = tar::Builder::new(encoder);

    // runtime-info.json：与 GET /api/runtime-info 同源的原始值，不做遮盖。
    let runtime_bytes = serde_json::to_vec(&runtime)?;
    if runtime_bytes.len() as u64 > RUNTIME_INFO_OUTPUT_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "runtime info exceeds reserved budget",
        ));
    }
    if budget.grant_runtime_output(runtime_bytes.len() as u64) != runtime_bytes.len() as u64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "runtime info exceeds total budget",
        ));
    }
    append_member(&mut bundle, "runtime-info.json", &runtime_bytes, mtime)?;

    // Aria 聚合树与各 provider 轨迹的收集：tar 写入、成员时间戳与预算经
    // BundleWriter 一起流转；作用域结束后 bundle/budget 交还本函数做 manifest 计量。
    let mut aria = AriaAccumulator::default();
    let aria_status: &'static str;
    let mut providers_json = Map::new();
    {
        let mut writer = BundleWriter {
            bundle: &mut bundle,
            mtime,
            budget: &mut budget,
        };
        aria_status = match host.home.as_ref() {
            None => "home_unavailable",
            Some(home) => match Root::open(&home.join(".aria").join("projects")) {
                Ok(root) => {
                    let mut scan_left = SCAN_ENTRY_BUDGET;
                    walk_aria(
                        &mut writer,
                        &root,
                        Path::new(""),
                        0,
                        false,
                        &mut aria,
                        &mut scan_left,
                    )?;
                    "ok"
                }
                Err(_) => "root_unavailable",
            },
        };

        // 各 provider 轨迹（wire 名固定顺序，清单确定性）。
        for (idx, def) in PROVIDERS.iter().enumerate() {
            let section = emit_provider(idx, def, host.provider_root(idx), &mut writer, injected)?;
            providers_json.insert(def.slug.to_string(), section);
        }
    }

    // manifest.json 最后写入，包含最终计数与限额使用情况。
    let manifest = build_manifest(&utc, aria_status, &aria, providers_json, &budget);
    let manifest_bytes = serde_json::to_vec(&manifest)?;
    if budget.grant_manifest_output(manifest_bytes.len() as u64) != manifest_bytes.len() as u64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "manifest exceeds reserved budget",
        ));
    }
    append_member(&mut bundle, "manifest.json", &manifest_bytes, mtime)?;

    let encoder = bundle.into_inner()?;
    let mut file = encoder.finish()?;
    file.seek(SeekFrom::Start(0))?;
    Ok(file)
}

fn append_member(bundle: &mut Bundle, path: &str, data: &[u8], mtime: u64) -> io::Result<()> {
    let mut header = tar::Header::new_gnu();
    header.set_size(data.len() as u64);
    header.set_mode(MEMBER_MODE);
    header.set_mtime(mtime);
    header.set_uid(0);
    header.set_gid(0);
    header.set_entry_type(tar::EntryType::Regular);
    // append_data 负责长相对路径的 GNU 扩展头及校验和，不能先用 set_path 截成 100 字节。
    bundle.append_data(&mut header, path, data)
}

/// 归档写入上下文：tar 成员、时间戳与预算在收集过程中一起流转。
struct BundleWriter<'a> {
    bundle: &'a mut Bundle,
    mtime: u64,
    budget: &'a mut Budget,
}

impl BundleWriter<'_> {
    fn append(&mut self, path: &str, data: &[u8]) -> io::Result<()> {
        append_member(self.bundle, path, data, self.mtime)
    }
}

#[derive(Default)]
struct AriaAccumulator {
    included: Vec<String>,
    skipped: Vec<(String, &'static str)>,
    /// 收录了但局部缺失/不可识别的说明（世代投影的白名单缺失项）。
    notes: Vec<(String, String)>,
    scan_noted: bool,
}

fn display_rel(rel: &Path) -> String {
    if rel.as_os_str().is_empty() {
        ".".to_string()
    } else {
        rel.display().to_string()
    }
}

fn note_scan_limit(acc: &mut AriaAccumulator, rel: &Path) {
    if acc.scan_noted {
        return;
    }
    acc.scan_noted = true;
    acc.skipped.push((display_rel(rel), "scan_limit"));
}

/// 安全成员分量：拒绝空、`.`/`..`、分隔符、反斜杠、冒号与控制字符。
fn safe_component(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains(':')
        && !name.chars().any(char::is_control)
}

fn walk_aria(
    writer: &mut BundleWriter<'_>,
    root: &Root,
    rel: &Path,
    depth: usize,
    under_aggregate: bool,
    acc: &mut AriaAccumulator,
    scan: &mut usize,
) -> io::Result<()> {
    if depth > MAX_TREE_DEPTH {
        acc.skipped.push((display_rel(rel), "depth_limit"));
        return Ok(());
    }
    if *scan == 0 {
        note_scan_limit(acc, rel);
        return Ok(());
    }
    let entries = match root.list(rel, scan) {
        Ok(entries) => entries,
        Err(_) => {
            acc.skipped.push((display_rel(rel), "unreadable"));
            return Ok(());
        }
    };
    if *scan == 0 {
        // 预算耗尽：list 可能返回部分结果，保守记截断但保留已收集条目。
        note_scan_limit(acc, rel);
    }
    for entry in entries {
        let Some(name) = entry.name.to_str().map(str::to_string) else {
            acc.skipped
                .push((format!("{}/<non-utf8>", display_rel(rel)), "unsafe_name"));
            continue;
        };
        if !safe_component(&name) {
            acc.skipped
                .push((format!("{}/{}", display_rel(rel), name), "unsafe_name"));
            continue;
        }
        let child = rel.join(&name);
        match entry.kind {
            Kind::Dir => {
                let now_under = under_aggregate || ARIA_TREES.contains(&name.as_str());
                walk_aria(writer, root, &child, depth + 1, now_under, acc, scan)?;
            }
            Kind::File => {
                // 聚合索引世代文件：只认两种规范布局的直接子文件，走白名单投影。
                // `aggregate-indexes` 不进通用递归白名单，嵌套一层不收录。
                if let Some(scope) = aggregate_projection::classify(&child) {
                    match generation_member_bytes(root, &child, &scope, &mut *writer.budget) {
                        Ok((bytes, notes)) => {
                            for note in notes {
                                acc.notes.push((child.display().to_string(), note));
                            }
                            let member = format!("aria/projects/{}", child.display());
                            writer.append(&member, &bytes)?;
                            acc.included.push(member);
                        }
                        Err(reason) => acc.skipped.push((child.display().to_string(), reason)),
                    }
                    continue;
                }
                if !under_aggregate || !name.ends_with(".json") {
                    continue; // 聚合树之外的旁文件/非 JSON 不收录
                }
                match aria_member_bytes(root, &child, &mut *writer.budget) {
                    Ok(bytes) => {
                        let member = format!("aria/projects/{}", child.display());
                        writer.append(&member, &bytes)?;
                        acc.included.push(member);
                    }
                    Err(reason) => acc.skipped.push((child.display().to_string(), reason)),
                }
            }
            Kind::Other => {} // 链接/特殊文件：不跟随、不收录
        }
    }
    Ok(())
}

/// 读单个聚合树 JSON：解析→遮盖→重序列化，结构化 JSON 不得局部伪造；
/// 超大或无效则整个跳过并给出稳定原因。
fn aria_member_bytes(
    root: &Root,
    rel: &Path,
    budget: &mut Budget,
) -> Result<Vec<u8>, &'static str> {
    let value = read_aria_json(root, rel, budget)?;
    encode_aria_json(value, budget)
}

fn read_aria_json(root: &Root, rel: &Path, budget: &mut Budget) -> Result<Value, &'static str> {
    let mut file = root.file(rel).map_err(|_| "unreadable")?;
    let len = file.metadata().map_err(|_| "unreadable")?.len();
    if len > ARIA_MEMBER_MAX_BYTES {
        return Err("oversized");
    }
    if budget.grant_aria_input(len) < len {
        return Err("budget_exhausted");
    }
    let mut buf = vec![0u8; len as usize];
    file.read_exact(&mut buf).map_err(|_| "source_changed")?;
    serde_json::from_slice(&buf).map_err(|_| "invalid_json")
}

fn encode_aria_json(mut value: Value, budget: &mut Budget) -> Result<Vec<u8>, &'static str> {
    redact_credentials(&mut value);
    let bytes = serde_json::to_vec(&value).map_err(|_| "encode_failed")?;
    if budget.grant_aria_output(bytes.len() as u64) < bytes.len() as u64 {
        return Err("budget_exhausted");
    }
    Ok(bytes)
}

/// 读单个聚合索引世代 JSON：解析 → 作用域身份核对 → 白名单投影 → 重序列化。
///
/// 与聚合树的原样收录不同：世代文件只导出白名单事实（状态、时间、成员版本、
/// 可识别的失败码与分诊、实测覆盖与逐成员盘检证据），任意正文、warning
/// 原文、路径与快照都不进包。身份对不上即整份丢弃（稳定原因）。
fn generation_member_bytes(
    root: &Root,
    rel: &Path,
    scope: &aggregate_projection::GenerationScope,
    budget: &mut Budget,
) -> Result<(Vec<u8>, Vec<String>), &'static str> {
    let value = read_aria_json(root, rel, budget)?;
    let projection = aggregate_projection::project(scope, &value)?;
    // 凭据遮盖照旧兜底：白名单字段也可能是被塞进来的敏感串。
    let bytes = encode_aria_json(projection.value, budget)?;
    Ok((bytes, projection.notes))
}

const CREDENTIAL_KEY_EXACT: [&str; 12] = [
    "token",
    "key",
    "apikey",
    "apisecret",
    "secret",
    "password",
    "passwd",
    "authorization",
    "auth",
    "cookie",
    "credential",
    "credentials",
];
const CREDENTIAL_KEY_PARTS: [&str; 13] = [
    "password",
    "passwd",
    "apikey",
    "apisecret",
    "accesstoken",
    "refreshtoken",
    "idtoken",
    "sessiontoken",
    "bearertoken",
    "authorization",
    "credential",
    "privatekey",
    "webhook",
];
const CREDENTIAL_VALUE_PREFIXES: [&str; 13] = [
    "sk-",
    "sk_",
    "Bearer ",
    "bearer ",
    "ghp_",
    "gho_",
    "ghu_",
    "ghs_",
    "github_pat_",
    "xoxb-",
    "xoxp-",
    "AIza",
    "-----BEGIN",
];

fn is_credential_key(key: &str) -> bool {
    let normalized: String = key
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect();
    CREDENTIAL_KEY_EXACT.contains(&normalized.as_str())
        || CREDENTIAL_KEY_PARTS
            .iter()
            .any(|part| normalized.contains(part))
}

fn looks_like_credential(text: &str) -> bool {
    CREDENTIAL_VALUE_PREFIXES
        .iter()
        .any(|prefix| text.starts_with(prefix))
}

/// 遮盖常见凭据键与明显凭据值。不承诺移除全部项目私密。
fn redact_credentials(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, child) in map.iter_mut() {
                if is_credential_key(key) {
                    *child = Value::String(REDACTED.to_string());
                } else {
                    redact_credentials(child);
                }
            }
        }
        Value::Array(items) => {
            for item in items.iter_mut() {
                redact_credentials(item);
            }
        }
        Value::String(text) if looks_like_credential(text) => {
            *text = REDACTED.to_string();
        }
        _ => {}
    }
}

struct FileReport {
    member: String,
    group: usize,
    stream: usize,
    status: &'static str,
    events: usize,
    invalid_lines: usize,
    oversized_lines: usize,
    truncated: bool,
    reasons: Vec<&'static str>,
}

impl FileReport {
    fn into_json(self) -> Value {
        json!({
            "group": self.group,
            "stream": self.stream,
            "member": self.member,
            "status": self.status,
            "events": self.events,
            "skipped_lines": {"invalid_json": self.invalid_lines, "oversized": self.oversized_lines},
            "truncated": self.truncated,
            "reasons": self.reasons,
        })
    }
}

fn emit_provider(
    idx: usize,
    def: &ProviderDef,
    root_info: &ProviderRoot,
    writer: &mut BundleWriter<'_>,
    injected: &[(&'static str, Vec<Session>)],
) -> io::Result<Value> {
    // 打包时刻：会话成员时间戳不可得时的回退值。
    let mtime = writer.mtime;
    let mut warnings: Vec<&'static str> = Vec::new();
    let mut status: &'static str;
    let mut root: Option<Root> = None;
    let discovered;
    let mut sessions: &[Session] = &[];
    if root_info.invalid_var.is_some() {
        // 无效覆盖：跳过该家，不回退 HOME 默认根。
        status = "root_unavailable";
    } else if let Some(path) = root_info.path.as_ref() {
        match Root::open(path) {
            Ok(opened) => {
                if let Some((_, list)) = injected.iter().find(|(slug, _)| *slug == def.slug) {
                    sessions = list;
                } else {
                    discovered = sessions::discover(def.slug, &opened, &mut warnings);
                    sessions = &discovered;
                }
                root = Some(opened);
                status = "ok";
            }
            Err(error) => {
                status = if root_info.source == SOURCE_DEFAULT_HOME
                    && error.kind() == io::ErrorKind::NotFound
                {
                    "no_records"
                } else {
                    "root_unavailable"
                };
            }
        }
    } else {
        status = "no_records";
    }
    if sessions.is_empty() && status == "ok" {
        // 「有候选但认不出原生会话身份」与「压根没有候选」必须分开报：
        // 前者说明根选对了、识别没成功；后者说明这个根没有会话文件。
        status = if warnings.contains(&sessions::PI_UNRECOGNIZED_WARNING) {
            "no_matching_sessions"
        } else {
            "no_records"
        };
    }

    let mut files: Vec<Value> = Vec::new();
    let mut included_files: Vec<String> = Vec::new();
    let mut included_groups = 0;
    if let Some(root) = root.as_ref() {
        for (i, session) in sessions.iter().take(MAX_SESSIONS_PER_PROVIDER).enumerate() {
            let rank = i + 1;
            // 会话成员的 tar 时间戳跟随该会话最后修改时间。
            writer.mtime = session
                .modified
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(mtime, |age| age.as_secs());
            let prior_files = included_files.len();
            for (stream, member_rel) in session.members.iter().enumerate() {
                let report =
                    emit_trace_member(writer, idx, def.slug, rank, stream, root, member_rel)?;
                if report.status == "included" {
                    included_files.push(report.member.clone());
                }
                files.push(report.into_json());
            }
            included_groups += usize::from(included_files.len() > prior_files);
        }
        // 还原打包时刻，避免上一家的会话时间戳泄漏成下一家的回退值。
        writer.mtime = mtime;
    }

    let mut section = Map::new();
    section.insert("status".into(), json!(status));
    section.insert(
        "root_source".into(),
        json!(match root_info.source {
            SOURCE_ENV => "env".to_string(),
            SOURCE_DEFAULT_HOME => "default_home".to_string(),
            _ => format!("invalid_env:{}", root_info.invalid_var.unwrap_or("unknown")),
        }),
    );
    section.insert(
        "selected_groups".into(),
        json!(sessions.len().min(MAX_SESSIONS_PER_PROVIDER)),
    );
    section.insert("included_groups".into(), json!(included_groups));
    section.insert("included_files".into(), json!(included_files));
    section.insert("files".into(), json!(files));
    section.insert("warnings".into(), json!(warnings));
    Ok(Value::Object(section))
}

/// 单个会话成员 → `providers/<wire>/<rank>/<stream>.jsonl`：8 MiB 尾窗、
/// 不追追加、只投影完整可解析行；巨行/坏行不写原文。
fn emit_trace_member(
    writer: &mut BundleWriter<'_>,
    idx: usize,
    slug: &str,
    rank: usize,
    stream: usize,
    root: &Root,
    member_rel: &Path,
) -> io::Result<FileReport> {
    let member = format!("providers/{slug}/{rank}/{stream}.jsonl");
    let mut report = FileReport {
        member: member.clone(),
        group: rank,
        stream,
        status: "included",
        events: 0,
        invalid_lines: 0,
        oversized_lines: 0,
        truncated: false,
        reasons: Vec::new(),
    };
    let mut file = match root.file(member_rel) {
        Ok(file) => file,
        Err(_) => {
            report.status = "unreadable";
            report.reasons.push("open_failed");
            return Ok(report);
        }
    };
    let snapshot_len = match file.metadata() {
        Ok(meta) => meta.len(),
        Err(_) => {
            report.status = "unreadable";
            report.reasons.push("metadata_failed");
            return Ok(report);
        }
    };
    if snapshot_len == 0 {
        report.status = "no_events";
        report.reasons.push("empty_source");
        return Ok(report);
    }
    let window = snapshot_len.min(TRACE_TAIL_WINDOW_BYTES);
    if window < snapshot_len {
        report.truncated = true;
        report.reasons.push("tail_window");
    }
    let start = snapshot_len - window;
    // 多看一个边界字节：若它是换行，窗口首行本身就是完整事件。
    let read_from = start.saturating_sub(1);
    if file.seek(SeekFrom::Start(read_from)).is_err() {
        report.status = "unreadable";
        report.reasons.push("seek_failed");
        return Ok(report);
    }
    let mut remaining = snapshot_len - read_from;
    let mut discard_partial_prefix = start != 0;

    let mut chunk = vec![0u8; 64 * 1024];
    let mut line: Vec<u8> = Vec::new();
    let mut line_overflow = false;
    let mut member_buf: Vec<u8> = Vec::new();
    let mut stopped: Option<&'static str> = None;
    while stopped.is_none() && remaining > 0 {
        let want = remaining.min(chunk.len() as u64) as usize;
        let granted = writer.budget.grant_trace_input(idx, want as u64) as usize;
        if granted == 0 {
            stopped = Some("budget_exhausted");
            break;
        }
        let n = match file.read(&mut chunk[..granted]) {
            Ok(0) => {
                stopped = Some("source_changed");
                break;
            }
            Ok(n) => n,
            Err(_) => {
                stopped = Some("read_failed");
                break;
            }
        };
        remaining -= n as u64;
        for &byte in &chunk[..n] {
            if discard_partial_prefix {
                // 只丢到第一个换行；同一读块中后面的完整事件照常解析。
                discard_partial_prefix = byte != b'\n';
                continue;
            }
            if byte == b'\n' {
                if line_overflow {
                    report.oversized_lines += 1;
                } else if !line.is_empty() {
                    match push_projected_line(
                        slug,
                        idx,
                        &line,
                        &mut member_buf,
                        &mut *writer.budget,
                        &mut report,
                    ) {
                        Ok(()) => {}
                        Err("invalid_json") => report.invalid_lines += 1,
                        Err("budget_exhausted") => stopped = Some("budget_exhausted"),
                        Err(_) => stopped = Some("encode_failed"),
                    }
                }
                line.clear();
                line_overflow = false;
            } else if line.len() >= MAX_LINE_BYTES {
                line_overflow = true;
            } else {
                line.push(byte);
            }
            if stopped.is_some() {
                break;
            }
        }
    }
    if stopped.is_none() {
        // 读满窗口到快照尾：末尾无换行的完整行也算一条记录。
        if line_overflow {
            report.oversized_lines += 1;
        } else if !line.is_empty() {
            match push_projected_line(
                slug,
                idx,
                &line,
                &mut member_buf,
                &mut *writer.budget,
                &mut report,
            ) {
                Ok(()) => {}
                Err("invalid_json") => report.invalid_lines += 1,
                Err("budget_exhausted") => stopped = Some("budget_exhausted"),
                Err(_) => stopped = Some("encode_failed"),
            }
        }
    }
    if let Some(reason) = stopped {
        report.truncated = true;
        report.reasons.push(reason);
    }
    if member_buf.is_empty() {
        // 没有任何可投影事件：不写空成员冒充，清单如实记录。
        report.status = "no_events";
        report.reasons.push("no_parseable_lines");
    } else {
        writer.append(&member, &member_buf)?;
    }
    Ok(report)
}

/// 单行：完整解析 → 白名单投影 → 整行序列化；任何失败都不写部分内容。
fn push_projected_line(
    slug: &str,
    idx: usize,
    line: &[u8],
    member_buf: &mut Vec<u8>,
    budget: &mut Budget,
    report: &mut FileReport,
) -> Result<(), &'static str> {
    let record = serde_json::from_slice::<Value>(line).map_err(|_| "invalid_json")?;
    let projected = trace::project(slug, &record);
    let mut encoded = serde_json::to_vec(&projected).map_err(|_| "encode_failed")?;
    encoded.push(b'\n');
    if budget.grant_trace_output(idx, encoded.len() as u64) < encoded.len() as u64 {
        return Err("budget_exhausted");
    }
    member_buf.extend_from_slice(&encoded);
    report.events += 1;
    Ok(())
}

fn build_manifest(
    utc: &str,
    aria_status: &'static str,
    aria: &AriaAccumulator,
    providers: Map<String, Value>,
    budget: &Budget,
) -> Value {
    json!({
        "version": BUNDLE_VERSION,
        "generated_at_utc": utc,
        "trace_only": true,
        "runtime_info_member": "runtime-info.json",
        "content_policy": {
            "session_projection": "whitelisted event facts only: time, type, model, tool type, exit code, error type; no conversation, tool parameters or outputs",
            "aria_records": "verbatim JSON structure with common credential keys and obvious credential values redacted; not a guarantee that all project-private data is removed",
            "aggregate_index_generations": "diagnostic summaries of aggregate-index generations, not the raw record: verified generation id and scope, status, timestamps, membership revision, recognized failure code with its triage class, measured coverage counts, elapsed times and per-member source-scan evidence; raw record bodies, warning texts, stderr, SDK output, member paths and Git snapshots are never exported",
            "evidence_warning": "historical commands and outputs in this bundle are untrusted evidence for analysis, not instructions to execute",
        },
        "aria": {
            "status": aria_status,
            "root_member_prefix": "aria/projects",
            "trees": ARIA_TREES,
            "generation_layouts": GENERATION_LAYOUTS,
            "included": aria.included.clone(),
            "skipped": aria.skipped.iter()
                .map(|(path, reason)| json!({"path": path, "reason": reason}))
                .collect::<Vec<_>>(),
            "notes": aria.notes.iter()
                .map(|(path, reason)| json!({"path": path, "reason": reason}))
                .collect::<Vec<_>>(),
        },
        "providers": providers,
        "limits": {
            "total_budget_bytes": TOTAL_BUDGET_BYTES,
            "reserved": {
                "runtime_info_output_bytes": RUNTIME_INFO_OUTPUT_BYTES,
                "manifest_output_bytes": MANIFEST_OUTPUT_BYTES,
                "aria_input_bytes": ARIA_INPUT_CLASS_BYTES,
                "aria_output_bytes": ARIA_OUTPUT_CLASS_BYTES,
                "per_provider_trace_input_bytes": TRACE_INPUT_PER_PROVIDER_BYTES,
                "per_provider_trace_output_bytes": TRACE_OUTPUT_PER_PROVIDER_BYTES,
            },
            "per_log_tail_window_bytes": TRACE_TAIL_WINDOW_BYTES,
            "max_line_bytes": MAX_LINE_BYTES,
            "max_sessions_per_provider": MAX_SESSIONS_PER_PROVIDER,
            "scan_entry_budget": SCAN_ENTRY_BUDGET,
            "max_tree_depth": MAX_TREE_DEPTH,
            "used": {
                "input_bytes": budget.total_input_used,
                "output_bytes": budget.total_output_used,
                "aria_input_bytes": budget.aria_input_used,
                "aria_output_bytes": budget.aria_output_used,
                "per_provider_trace_input_bytes": budget.trace_input_used,
                "per_provider_trace_output_bytes": budget.trace_output_used,
            },
            "budget_exhausted": budget.exhausted,
        },
    })
}
