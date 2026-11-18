//! 驾驶舱日志打包：本地逻辑会话枚举（T2）。
//!
//! 职责边界：只回答“哪些文件属于同一个逻辑会话、该会话最近一次修改是什么时候”。
//! 不解析会话正文、不写盘、不返回任何原始日志内容；路径与时间只来自安全打开的文件句柄
//! （`fs::Root::file`）与固定深度的目录列举（`fs::Root::list`），扫描条目、探头字节、
//! 探头文件数与单组成员数都有硬上限。
//!
//! provider 原生根目录由调用方打开并传入：
//! - `claude_code` → `~/.claude/projects`：`<project>/<uuid>.jsonl`，子代理流在
//!   `<project>/<uuid>/subagents/agent-*.jsonl`，只依附同目录现存父会话；
//! - `codex` → `~/.codex/sessions`：`YYYY/MM/DD/rollout-*.jsonl`，按头部
//!   `session_meta.payload.id` / `payload.session_id` 去重；
//! - `pi` → `~/.pi/agent/sessions`：`<cwd>/<name>.jsonl`，按首个 `type=session` 记录的 `id` 去重；
//! - `kimi_code` → `~/.kimi-code/sessions`：`<wd>/session_<id>/agents/<agent>/wire.jsonl`，
//!   以父会话为一组，主流与子流不各占名额。

use std::collections::{BTreeMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::Value;

use super::fs::{Entry, Kind, Root};

const PROVIDER_CLAUDE_CODE: &str = "claude_code";
const PROVIDER_CODEX: &str = "codex";
const PROVIDER_PI: &str = "pi";
const PROVIDER_KIMI_CODE: &str = "kimi_code";

/// 每家 provider 最多收录的逻辑会话数。
const MAX_SESSIONS_PER_PROVIDER: usize = 3;
/// 单次目录列举允许返回的条目上限。
const MAX_LIST_ENTRIES: usize = 512;
/// 全部目录列举累计条目预算。
const MAX_TOTAL_DIR_ENTRIES: usize = 20_000;
/// 允许被探头解析的文件数量上限。
const MAX_PROBED_FILES: usize = 2_048;
/// 单文件探头读取字节上限。
const MAX_PROBE_BYTES_PER_FILE: usize = 64 * 1024;
/// 全部探头读取字节上限。
const MAX_TOTAL_PROBE_BYTES: usize = 4 * 1024 * 1024;
/// 单个逻辑会话的成员文件上限。
const MAX_GROUP_MEMBERS: usize = 64;
/// 探头最多解析的行数（Codex 的 `session_meta` 只在前几行）。
const MAX_HEADER_LINES: usize = 4;

const UNKNOWN_PROVIDER: &str = "log_bundle: 未知 provider，已跳过会话枚举";

/// 一个逻辑会话：最近修改时间 + 稳定排序的成员文件（相对 provider 根目录）。
pub(super) struct Session {
    pub(super) modified: SystemTime,
    pub(super) members: Vec<PathBuf>,
}

/// 枚举过程中的中间分组，尚未去重与截断。
struct Group {
    id: String,
    modified: SystemTime,
    members: Vec<PathBuf>,
}

/// 一次探头的结果：文件头若干行与同一文件句柄上的修改时间。
struct ProbeResult {
    values: Vec<Value>,
    modified: Option<SystemTime>,
}

/// 各 provider 的静态 warning 文本（同一 provider 内去重，不含路径或原始内容）。
#[derive(Clone, Copy)]
struct ProviderWarnings {
    no_session: &'static str,
    fewer_than_max: &'static str,
    scan_truncated: &'static str,
    header_missing: &'static str,
    orphan: &'static str,
    duplicate: &'static str,
    unreadable_file: &'static str,
}

impl ProviderWarnings {
    fn claude_code() -> Self {
        Self {
            no_session: "claude_code: 未发现可用会话",
            fewer_than_max: "claude_code: 可用会话少于 3 个",
            scan_truncated: "claude_code: 会话目录不可读或超出扫描预算",
            header_missing: "claude_code: 会话文件缺少可识别内容，已跳过",
            orphan: "claude_code: 存在缺少父会话的子代理目录，已忽略",
            duplicate: "claude_code: 重复会话 ID 已合并",
            unreadable_file: "claude_code: 会话文件不可读，已跳过",
        }
    }

    fn codex() -> Self {
        Self {
            no_session: "codex: 未发现可用会话",
            fewer_than_max: "codex: 可用会话少于 3 个",
            scan_truncated: "codex: 会话目录不可读或超出扫描预算",
            header_missing: "codex: rollout 文件缺少 session_meta 头，已跳过",
            orphan: "codex: 目录结构不符合 YYYY/MM/DD 布局，已忽略",
            duplicate: "codex: 重复会话 ID 已合并",
            unreadable_file: "codex: 会话文件不可读，已跳过",
        }
    }

    fn pi() -> Self {
        Self {
            no_session: "pi: 未发现可用会话",
            fewer_than_max: "pi: 可用会话少于 3 个",
            scan_truncated: "pi: 会话目录不可读或超出扫描预算",
            header_missing: "pi: 会话文件缺少 session 头，已跳过",
            orphan: "pi: 目录结构不符合 <cwd> 布局，已忽略",
            duplicate: "pi: 重复会话 ID 已合并",
            unreadable_file: "pi: 会话文件不可读，已跳过",
        }
    }

    fn kimi_code() -> Self {
        Self {
            no_session: "kimi_code: 未发现可用会话",
            fewer_than_max: "kimi_code: 可用会话少于 3 个",
            scan_truncated: "kimi_code: 会话目录不可读或超出扫描预算",
            header_missing: "kimi_code: 会话缺少 agents/*/wire.jsonl，已跳过",
            orphan: "kimi_code: 目录结构不符合 session_<id> 布局，已忽略",
            duplicate: "kimi_code: 重复会话 ID 已合并",
            unreadable_file: "kimi_code: 会话文件不可读，已跳过",
        }
    }
}

/// 枚举上下文：持有根目录、warning 出口与各类硬预算。
struct Discovery<'a> {
    root: &'a Root,
    warnings: &'a mut Vec<&'static str>,
    spec: ProviderWarnings,
    dir_budget: usize,
    probed_files: usize,
    probed_bytes: usize,
    truncated: bool,
}

impl<'a> Discovery<'a> {
    fn new(root: &'a Root, warnings: &'a mut Vec<&'static str>, spec: ProviderWarnings) -> Self {
        Self {
            root,
            warnings,
            spec,
            dir_budget: MAX_TOTAL_DIR_ENTRIES,
            probed_files: 0,
            probed_bytes: 0,
            truncated: false,
        }
    }

    /// 列出相对目录下的条目（按名称稳定排序）；预算耗尽时返回空并标记截断。
    fn list(&mut self, relative: &Path) -> Vec<Entry> {
        match self.try_list(relative) {
            Ok(entries) => entries,
            Err(error) => {
                if error.kind() != std::io::ErrorKind::NotFound {
                    self.truncated = true;
                }
                Vec::new()
            }
        }
    }

    /// 列举一次目录；相对路径为空即根目录本身。
    fn try_list(&mut self, relative: &Path) -> Result<Vec<Entry>, std::io::Error> {
        if self.dir_budget == 0 {
            self.truncated = true;
            return Err(std::io::Error::other("扫描预算耗尽"));
        }
        let mut remaining = self.dir_budget.min(MAX_LIST_ENTRIES);
        let before = remaining;
        let outcome = self.root.list(relative, &mut remaining);
        let consumed = before.saturating_sub(remaining);
        match outcome {
            Ok(mut entries) => {
                self.dir_budget = self.dir_budget.saturating_sub(consumed.max(entries.len()));
                // 预算被列举耗尽时 `remaining == 0`：可能仍有目录项没被看到，保守记为截断。
                if remaining == 0 {
                    self.truncated = true;
                }
                entries.sort_by(|left, right| left.name.cmp(&right.name));
                Ok(entries)
            }
            Err(error) => {
                self.dir_budget = self.dir_budget.saturating_sub(consumed.max(1));
                Err(error)
            }
        }
    }

    /// 取安全打开文件的修改时间。
    fn modified(&mut self, relative: &Path) -> Option<SystemTime> {
        let file = self.root.file(relative).ok()?;
        let metadata = file.metadata().ok()?;
        metadata.modified().ok()
    }

    /// 读取文件开头的若干行并解析为 JSON（只取头部，不搬运全文）。
    ///
    /// 文件打不开时返回 `None`（调用方记“不可读”）；头部解析不出内容时返回空 `values`。
    fn probe(&mut self, relative: &Path, max_lines: usize) -> Option<ProbeResult> {
        if self.probed_files >= MAX_PROBED_FILES || self.probed_bytes >= MAX_TOTAL_PROBE_BYTES {
            self.truncated = true;
            return None;
        }
        let Ok(mut file) = self.root.file(relative) else {
            return None;
        };
        let modified = file
            .metadata()
            .ok()
            .and_then(|metadata| metadata.modified().ok());
        let allowance = MAX_PROBE_BYTES_PER_FILE.min(MAX_TOTAL_PROBE_BYTES - self.probed_bytes);
        self.probed_files += 1;
        let mut buffer = vec![0u8; allowance];
        let mut filled = 0usize;
        while filled < buffer.len() {
            match file.read(&mut buffer[filled..]) {
                Ok(0) => break,
                Ok(count) => filled += count,
                Err(_) => break,
            }
        }
        self.probed_bytes += filled;

        let text = String::from_utf8_lossy(&buffer[..filled]);
        let mut values = Vec::new();
        for line in text.lines() {
            if values.len() >= max_lines {
                break;
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            match serde_json::from_str::<Value>(trimmed) {
                Ok(value) => values.push(value),
                Err(_) => break,
            }
        }
        Some(ProbeResult { values, modified })
    }

    fn warn(&mut self, message: &'static str) {
        if !self.warnings.contains(&message) {
            self.warnings.push(message);
        }
    }
}

/// 按 provider 枚举本地逻辑会话，最多返回三个（新的在前）。
pub(super) fn discover(
    provider: &str,
    root: &Root,
    warnings: &mut Vec<&'static str>,
) -> Vec<Session> {
    let Some(canonical) = canonical_provider(provider) else {
        if !warnings.contains(&UNKNOWN_PROVIDER) {
            warnings.push(UNKNOWN_PROVIDER);
        }
        return Vec::new();
    };

    let spec = match canonical {
        PROVIDER_CLAUDE_CODE => ProviderWarnings::claude_code(),
        PROVIDER_CODEX => ProviderWarnings::codex(),
        PROVIDER_PI => ProviderWarnings::pi(),
        _ => ProviderWarnings::kimi_code(),
    };

    let mut discovery = Discovery::new(root, warnings, spec);
    let groups = match canonical {
        PROVIDER_CLAUDE_CODE => discover_claude(&mut discovery),
        PROVIDER_CODEX => discover_codex(&mut discovery),
        PROVIDER_PI => discover_pi(&mut discovery),
        _ => discover_kimi(&mut discovery),
    };
    select_sessions(&mut discovery, groups)
}

/// 归一化 provider 名：wire 名用下划线，provider 家族名用连字符，两种写法都接受。
fn canonical_provider(provider: &str) -> Option<&'static str> {
    let normalized = provider.trim().to_ascii_lowercase().replace('-', "_");
    match normalized.as_str() {
        "claude_code" => Some(PROVIDER_CLAUDE_CODE),
        "codex" => Some(PROVIDER_CODEX),
        "pi" => Some(PROVIDER_PI),
        "kimi_code" => Some(PROVIDER_KIMI_CODE),
        _ => None,
    }
}

/// 去重（同一原生 ID 只留最新的一份）、排序（mtime 倒序，ID 稳定打破并列）并截断到三个。
fn select_sessions(discovery: &mut Discovery<'_>, groups: Vec<Group>) -> Vec<Session> {
    let mut groups = groups;
    groups.sort_by(|left, right| {
        right
            .modified
            .cmp(&left.modified)
            .then_with(|| left.id.cmp(&right.id))
    });

    let spec = discovery.spec;
    let mut seen: HashSet<String> = HashSet::new();
    let mut unique: Vec<Group> = Vec::new();
    for group in groups {
        if !seen.insert(group.id.clone()) {
            discovery.warn(spec.duplicate);
            continue;
        }
        unique.push(group);
    }

    if unique.is_empty() {
        discovery.warn(spec.no_session);
    } else if unique.len() < MAX_SESSIONS_PER_PROVIDER {
        discovery.warn(spec.fewer_than_max);
    }
    if discovery.truncated {
        discovery.warn(spec.scan_truncated);
    }

    unique
        .into_iter()
        .take(MAX_SESSIONS_PER_PROVIDER)
        .map(|group| Session {
            modified: group.modified,
            members: group.members,
        })
        .collect()
}

/// Claude Code：`<project>/<uuid>.jsonl` 为父会话，子代理流只依附同目录的现存父会话。
fn discover_claude(discovery: &mut Discovery<'_>) -> Vec<Group> {
    let spec = discovery.spec;
    let mut groups: Vec<Group> = Vec::new();
    for project in discovery.list(Path::new("")) {
        if project.kind != Kind::Dir {
            continue;
        }
        let Some(project_name) = project.name.to_str() else {
            continue;
        };
        let project_path = PathBuf::from(project_name);
        let entries = discovery.list(&project_path);

        let mut parents: BTreeMap<String, PathBuf> = BTreeMap::new();
        let mut directories: BTreeMap<String, PathBuf> = BTreeMap::new();
        for entry in &entries {
            let Some(name) = entry.name.to_str() else {
                continue;
            };
            match entry.kind {
                Kind::File => {
                    let Some(stem) = name.strip_suffix(".jsonl") else {
                        continue;
                    };
                    if !is_safe_session_id(stem) {
                        continue;
                    }
                    parents.insert(stem.to_string(), project_path.join(name));
                }
                Kind::Dir => {
                    if !is_safe_session_id(name) {
                        continue;
                    }
                    directories.insert(name.to_string(), project_path.join(name));
                }
                Kind::Other => {}
            }
        }

        let mut subagents: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
        for (id, directory) in &directories {
            let children = discovery.list(directory);
            let has_subagents = children.iter().any(|entry| {
                entry.kind == Kind::Dir && entry.name.to_str() == Some("subagents")
            });
            if !has_subagents {
                continue;
            }
            if !parents.contains_key(id) {
                discovery.warn(spec.orphan);
                continue;
            }
            let subagents_dir = directory.join("subagents");
            let mut files: Vec<PathBuf> = Vec::new();
            for entry in discovery.list(&subagents_dir) {
                if files.len() >= MAX_GROUP_MEMBERS {
                    break;
                }
                if entry.kind != Kind::File {
                    continue;
                }
                let Some(name) = entry.name.to_str() else {
                    continue;
                };
                if !name.starts_with("agent-") || !name.ends_with(".jsonl") {
                    continue;
                }
                files.push(subagents_dir.join(name));
            }
            subagents.insert(id.clone(), files);
        }

        for (id, parent_path) in &parents {
            let mut members = vec![parent_path.clone()];
            let mut modified = discovery.modified(parent_path);
            if let Some(files) = subagents.get(id) {
                for path in files {
                    if members.len() >= MAX_GROUP_MEMBERS {
                        break;
                    }
                    if let Some(time) = discovery.modified(path) {
                        modified = Some(match modified {
                            Some(current) => current.max(time),
                            None => time,
                        });
                        members.push(path.clone());
                    }
                }
            }
            let Some(modified) = modified else {
                discovery.warn(spec.unreadable_file);
                continue;
            };
            groups.push(Group {
                id: id.clone(),
                modified,
                members,
            });
        }
    }
    groups
}

/// Codex：`YYYY/MM/DD/rollout-*.jsonl`，按头部 `session_meta` 的原生 ID 去重。
fn discover_codex(discovery: &mut Discovery<'_>) -> Vec<Group> {
    let spec = discovery.spec;
    let mut groups: Vec<Group> = Vec::new();
    for year in discovery.list(Path::new("")) {
        if year.kind != Kind::Dir {
            continue;
        }
        let Some(year_name) = year.name.to_str() else {
            continue;
        };
        if !is_digits(year_name, 4) {
            continue;
        }
        let year_path = PathBuf::from(year_name);
        for month in discovery.list(&year_path) {
            if month.kind != Kind::Dir {
                continue;
            }
            let Some(month_name) = month.name.to_str() else {
                continue;
            };
            if !is_digits(month_name, 2) {
                continue;
            }
            let month_path = year_path.join(month_name);
            for day in discovery.list(&month_path) {
                if day.kind != Kind::Dir {
                    continue;
                }
                let Some(day_name) = day.name.to_str() else {
                    continue;
                };
                if !is_digits(day_name, 2) {
                    continue;
                }
                let day_path = month_path.join(day_name);
                for entry in discovery.list(&day_path) {
                    if entry.kind != Kind::File {
                        continue;
                    }
                    let Some(name) = entry.name.to_str() else {
                        continue;
                    };
                    if !name.starts_with("rollout-") || !name.ends_with(".jsonl") {
                        continue;
                    }
                    let path = day_path.join(name);
                    let Some(probe) = discovery.probe(&path, MAX_HEADER_LINES) else {
                        discovery.warn(spec.unreadable_file);
                        continue;
                    };
                    let Some(id) = probe.values.iter().find_map(codex_session_id) else {
                        discovery.warn(spec.header_missing);
                        continue;
                    };
                    let Some(modified) = probe.modified else {
                        discovery.warn(spec.unreadable_file);
                        continue;
                    };
                    groups.push(Group {
                        id,
                        modified,
                        members: vec![path],
                    });
                }
            }
        }
    }
    groups
}

/// Pi：`<cwd>/<name>.jsonl`，按首个 `type=session` 记录的 `id` 去重。
fn discover_pi(discovery: &mut Discovery<'_>) -> Vec<Group> {
    let spec = discovery.spec;
    let mut groups: Vec<Group> = Vec::new();
    for cwd in discovery.list(Path::new("")) {
        if cwd.kind != Kind::Dir {
            continue;
        }
        let Some(cwd_name) = cwd.name.to_str() else {
            continue;
        };
        let cwd_path = PathBuf::from(cwd_name);
        for entry in discovery.list(&cwd_path) {
            if entry.kind != Kind::File {
                continue;
            }
            let Some(name) = entry.name.to_str() else {
                continue;
            };
            if !name.ends_with(".jsonl") {
                continue;
            }
            let path = cwd_path.join(name);
            let Some(probe) = discovery.probe(&path, 1) else {
                discovery.warn(spec.unreadable_file);
                continue;
            };
            let Some(id) = probe.values.first().and_then(pi_session_id) else {
                discovery.warn(spec.header_missing);
                continue;
            };
            let Some(modified) = probe.modified else {
                discovery.warn(spec.unreadable_file);
                continue;
            };
            groups.push(Group {
                id,
                modified,
                members: vec![path],
            });
        }
    }
    groups
}

/// Kimi Code：`<wd>/session_<id>/agents/<agent>/wire.jsonl`，以父会话为一组。
fn discover_kimi(discovery: &mut Discovery<'_>) -> Vec<Group> {
    let spec = discovery.spec;
    let mut groups: Vec<Group> = Vec::new();
    for workspace in discovery.list(Path::new("")) {
        if workspace.kind != Kind::Dir {
            continue;
        }
        let Some(workspace_name) = workspace.name.to_str() else {
            continue;
        };
        let workspace_path = PathBuf::from(workspace_name);
        for session in discovery.list(&workspace_path) {
            if session.kind != Kind::Dir {
                continue;
            }
            let Some(session_name) = session.name.to_str() else {
                continue;
            };
            if !is_kimi_session_dir(session_name) {
                continue;
            }
            let session_path = workspace_path.join(session_name);
            let agents_path = session_path.join("agents");

            let mut streams: Vec<(String, PathBuf, SystemTime)> = Vec::new();
            for agent in discovery.list(&agents_path) {
                if agent.kind != Kind::Dir {
                    continue;
                }
                let Some(agent_name) = agent.name.to_str() else {
                    continue;
                };
                let wire = agents_path.join(agent_name).join("wire.jsonl");
                let Some(modified) = discovery.modified(&wire) else {
                    continue;
                };
                if streams.len() >= MAX_GROUP_MEMBERS {
                    break;
                }
                streams.push((agent_name.to_string(), wire, modified));
            }
            if streams.is_empty() {
                discovery.warn(spec.header_missing);
                continue;
            }
            // 主流优先，其余按名称排序，保证成员顺序稳定。
            streams.sort_by(|left, right| {
                let left_main = left.0 == "main";
                let right_main = right.0 == "main";
                right_main
                    .cmp(&left_main)
                    .then_with(|| left.0.cmp(&right.0))
            });

            let mut modified = SystemTime::UNIX_EPOCH;
            let mut members: Vec<PathBuf> = Vec::new();
            for (_, path, time) in streams {
                modified = modified.max(time);
                members.push(path);
            }
            groups.push(Group {
                id: session_name.to_string(),
                modified,
                members,
            });
        }
    }
    groups
}

/// Codex 原生会话 ID：只认 `session_meta` 记录的 `payload.id` / `payload.session_id`。
fn codex_session_id(record: &Value) -> Option<String> {
    if let Some(kind) = record.get("type").and_then(Value::as_str) {
        if kind != "session_meta" {
            return None;
        }
    }
    let payload = record.get("payload")?;
    let id = payload
        .get("id")
        .and_then(Value::as_str)
        .or_else(|| payload.get("session_id").and_then(Value::as_str))?;
    let id = id.trim();
    if id.is_empty() {
        return None;
    }
    Some(id.to_string())
}

/// Pi 原生会话 ID：首个记录必须是 `type=session` 且带 `id`。
fn pi_session_id(record: &Value) -> Option<String> {
    if record.get("type").and_then(Value::as_str) != Some("session") {
        return None;
    }
    let id = record.get("id").and_then(Value::as_str)?.trim();
    if id.is_empty() {
        return None;
    }
    Some(id.to_string())
}

fn is_safe_session_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-' || character == '_')
}

fn is_kimi_session_dir(name: &str) -> bool {
    match name.strip_prefix("session_") {
        Some(rest) => is_safe_session_id(rest),
        None => false,
    }
}

fn is_digits(value: &str, length: usize) -> bool {
    value.len() == length && value.bytes().all(|byte| byte.is_ascii_digit())
}

#[cfg(test)]
#[path = "sessions_tests.rs"]
mod sessions_tests;
