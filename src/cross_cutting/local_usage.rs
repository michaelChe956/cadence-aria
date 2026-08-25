//! 从 provider CLI 管理的本地会话记录读取 token usage。
//!
//! 这是 ACP usage 缺失时的 best-effort fallback：读取失败、格式变动或记录不完整时
//! 返回 `None`，调用方不得将其视为 provider 会话失败。

use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::cross_cutting::streaming_provider::UsageReportData;

const MAX_TAIL_BYTES: u64 = 128 * 1024;
const MAX_TAIL_LINES: usize = 200;
const MAX_CODEX_SESSION_DEPTH: usize = 4;

/// 从 Pi `data.sessionFile` 指向的 JSONL 文件读取最近一条 usage 记录。
pub(crate) fn read_pi_usage(session_file: &Path, role: &str) -> Option<UsageReportData> {
    latest_json_line(session_file)
        .into_iter()
        .find_map(|value| parse_pi_usage_value(&value, role))
}

/// 从 Codex 本地 rollout 文件读取指定 ACP thread 的最近一条 token_count。
///
/// Codex rollout 首行的 `session_meta.session_id` 与 ACP `threadId` 相同；先核验该
/// 标识，再读取文件尾部，避免把同时运行的其它 Codex 会话的用量归属到当前 turn。
pub(crate) fn read_codex_usage(
    sessions_root: &Path,
    thread_id: &str,
    role: &str,
) -> Option<UsageReportData> {
    let file = find_codex_session_file(sessions_root, thread_id)?;
    latest_json_line(&file)
        .into_iter()
        .find_map(|value| parse_codex_usage_value(&value, role))
}

/// 从 Kimi 当前 ACP session 的全部 agent wire 日志聚合最近一条 per-turn usage。
///
/// `agents/*/wire.jsonl` 的每个 agent 都可能贡献 token；每个日志仅取最近一条
/// `usageScope == "turn"` 的 `usage.record`，避免把已结束的前一 turn 重复累计。
pub(crate) fn read_kimi_usage(
    sessions_root: &Path,
    working_dir: &Path,
    session_id: &str,
    role: &str,
) -> Option<UsageReportData> {
    if !is_safe_session_id(session_id) {
        return None;
    }
    let session_dir = sessions_root
        .join(kimi_work_dir_key(working_dir)?)
        .join(format!("session_{session_id}"));
    let agents_dir = session_dir.join("agents");
    let entries = fs::read_dir(agents_dir).ok()?;

    let mut total = UsageTotal::default();
    let mut found = false;
    for entry in entries.flatten() {
        let wire = entry.path().join("wire.jsonl");
        let Some(value) = latest_json_line(&wire).into_iter().find(is_kimi_turn_usage) else {
            continue;
        };
        let Some(usage) = value.get("usage") else {
            continue;
        };
        let Some(input_other) = unsigned(usage, "inputOther") else {
            continue;
        };
        let Some(output) = unsigned(usage, "output") else {
            continue;
        };
        let Some(cache_read) = unsigned(usage, "inputCacheRead") else {
            continue;
        };
        // Kimi 0.38.0 always writes inputCacheCreation, but treating it as zero when
        // omitted maintains compatibility with older logs while retaining useful usage.
        let cache_creation = unsigned(usage, "inputCacheCreation").unwrap_or(0);
        total.input = total
            .input
            .saturating_add(input_other.saturating_add(cache_creation));
        total.output = total.output.saturating_add(output);
        total.cache_read = total.cache_read.saturating_add(cache_read);
        total.cache_creation = total.cache_creation.saturating_add(cache_creation);
        found = true;
    }
    found.then(|| total.into_report(role))
}

/// 用户目录下的默认 Codex sessions 根。无法定位 home 时返回 `None`。
pub(crate) fn read_default_codex_usage(thread_id: &str, role: &str) -> Option<UsageReportData> {
    read_codex_usage(&home_dir()?.join(".codex/sessions"), thread_id, role)
}

/// 用户目录下的默认 Kimi sessions 根。无法定位 home 时返回 `None`。
pub(crate) fn read_default_kimi_usage(
    working_dir: &Path,
    session_id: &str,
    role: &str,
) -> Option<UsageReportData> {
    read_kimi_usage(
        &home_dir()?.join(".kimi-code/sessions"),
        working_dir,
        session_id,
        role,
    )
}

#[derive(Default)]
struct UsageTotal {
    input: u64,
    output: u64,
    cache_read: u64,
    cache_creation: u64,
}

impl UsageTotal {
    fn into_report(self, role: &str) -> UsageReportData {
        UsageReportData {
            role: role.to_string(),
            input_tokens: Some(self.input),
            output_tokens: Some(self.output),
            cache_read_tokens: Some(self.cache_read),
            cache_creation_tokens: Some(self.cache_creation),
        }
    }
}

fn parse_pi_usage_value(value: &Value, role: &str) -> Option<UsageReportData> {
    let usage = find_nested_usage(value, &["input", "output", "cacheRead"])?;
    report_from_required_fields(usage, "input", "output", "cacheRead", "cacheWrite", role)
}

fn parse_codex_usage_value(value: &Value, role: &str) -> Option<UsageReportData> {
    (value.pointer("/payload/type").and_then(Value::as_str) == Some("token_count")).then_some(())?;
    let usage = value.pointer("/payload/info/last_token_usage")?;
    report_from_required_fields(
        usage,
        "input_tokens",
        "output_tokens",
        "cached_input_tokens",
        "cache_write_input_tokens",
        role,
    )
}

fn report_from_required_fields(
    usage: &Value,
    input_key: &str,
    output_key: &str,
    cache_read_key: &str,
    cache_creation_key: &str,
    role: &str,
) -> Option<UsageReportData> {
    Some(UsageReportData {
        role: role.to_string(),
        input_tokens: Some(unsigned(usage, input_key)?),
        output_tokens: Some(unsigned(usage, output_key)?),
        cache_read_tokens: Some(unsigned(usage, cache_read_key)?),
        cache_creation_tokens: unsigned(usage, cache_creation_key),
    })
}

fn find_nested_usage<'a>(value: &'a Value, required_fields: &[&str]) -> Option<&'a Value> {
    if let Some(object) = value.as_object() {
        if let Some(usage) = object.get("usage")
            && required_fields
                .iter()
                .all(|field| usage.get(*field).is_some())
        {
            return Some(usage);
        }
        return object
            .values()
            .find_map(|child| find_nested_usage(child, required_fields));
    }
    value.as_array().and_then(|items| {
        items
            .iter()
            .find_map(|item| find_nested_usage(item, required_fields))
    })
}

fn is_kimi_turn_usage(value: &Value) -> bool {
    value.get("type").and_then(Value::as_str) == Some("usage.record")
        && value.get("usageScope").and_then(Value::as_str) == Some("turn")
}

fn unsigned(value: &Value, field: &str) -> Option<u64> {
    value.get(field).and_then(Value::as_u64)
}

fn latest_json_line(path: &Path) -> Vec<Value> {
    tail_lines(path)
        .unwrap_or_default()
        .into_iter()
        .rev()
        .filter_map(|line| serde_json::from_str(&line).ok())
        .collect()
}

/// 只读取文件最后 128 KiB / 200 行，避免长会话日志被完整载入内存。
fn tail_lines(path: &Path) -> Option<Vec<String>> {
    let mut file = File::open(path).ok()?;
    let length = file.metadata().ok()?.len();
    let start = length.saturating_sub(MAX_TAIL_BYTES);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut tail = String::new();
    file.read_to_string(&mut tail).ok()?;
    if start > 0 {
        let newline = tail.find('\n')?;
        tail.drain(..=newline);
    }
    let lines = tail.lines().map(ToString::to_string).collect::<Vec<_>>();
    let skip = lines.len().saturating_sub(MAX_TAIL_LINES);
    Some(lines.into_iter().skip(skip).collect())
}

fn find_codex_session_file(root: &Path, thread_id: &str) -> Option<PathBuf> {
    if !is_safe_session_id(thread_id) {
        return None;
    }
    let mut candidates = Vec::new();
    collect_codex_candidates(root, thread_id, 0, &mut candidates);
    candidates.sort_by_key(|path| {
        fs::metadata(path)
            .and_then(|metadata| metadata.modified())
            .ok()
    });
    candidates.reverse();
    candidates
        .into_iter()
        .find(|path| codex_file_declares_session(path, thread_id))
}

fn collect_codex_candidates(
    root: &Path,
    thread_id: &str,
    depth: usize,
    candidates: &mut Vec<PathBuf>,
) {
    if depth > MAX_CODEX_SESSION_DEPTH {
        return;
    }
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_codex_candidates(&path, thread_id, depth + 1, candidates);
        } else if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| {
                name.starts_with("rollout-") && name.ends_with(".jsonl") && name.contains(thread_id)
            })
        {
            candidates.push(path);
        }
    }
}

fn codex_file_declares_session(path: &Path, thread_id: &str) -> bool {
    let Ok(file) = File::open(path) else {
        return false;
    };
    BufReader::new(file).lines().take(4).any(|line| {
        let Ok(value) = line
            .and_then(|line| serde_json::from_str::<Value>(&line).map_err(std::io::Error::other))
        else {
            return false;
        };
        value.pointer("/payload/session_id").and_then(Value::as_str) == Some(thread_id)
            || value.pointer("/payload/id").and_then(Value::as_str) == Some(thread_id)
    })
}

fn kimi_work_dir_key(working_dir: &Path) -> Option<String> {
    let basename = working_dir.file_name()?.to_string_lossy();
    if basename.is_empty() {
        return None;
    }
    let hash = hex::encode(Sha256::digest(working_dir.to_string_lossy().as_bytes()));
    Some(format!("wd_{basename}_{}", &hash[..12]))
}

fn is_safe_session_id(session_id: &str) -> bool {
    !session_id.is_empty()
        && session_id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-')
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use tempfile::tempdir;

    use super::{read_codex_usage, read_kimi_usage, read_pi_usage};

    fn write(path: &Path, content: &str) {
        fs::create_dir_all(path.parent().expect("parent")).expect("create parent");
        fs::write(path, content).expect("write fixture");
    }

    #[test]
    fn local_usage_reads_pi_usage_from_latest_jsonl_record() {
        let root = tempdir().expect("tempdir");
        let session = root.path().join("pi.jsonl");
        write(
            &session,
            "{\"type\":\"message\",\"usage\":{\"input\":3,\"output\":1,\"cacheRead\":2}}\n{\"message\":{\"usage\":{\"input\":23,\"output\":3,\"cacheRead\":14656,\"cacheWrite\":0}}}\n",
        );
        let report = read_pi_usage(&session, "author").expect("usage");
        assert_eq!(report.input_tokens, Some(23));
        assert_eq!(report.output_tokens, Some(3));
        assert_eq!(report.cache_read_tokens, Some(14656));
    }

    #[test]
    fn local_usage_returns_none_for_missing_or_malformed_pi_file() {
        let root = tempdir().expect("tempdir");
        assert!(read_pi_usage(&root.path().join("missing.jsonl"), "author").is_none());
        let malformed = root.path().join("malformed.jsonl");
        write(&malformed, "not json\n{\"usage\":{\"input\":1}}\n");
        assert!(read_pi_usage(&malformed, "author").is_none());
    }

    #[test]
    fn local_usage_reads_codex_last_token_usage_for_matching_thread() {
        let root = tempdir().expect("tempdir");
        let thread = "01a03700-9098-7070-9a21-de7c0d04d709";
        let rollout = root
            .path()
            .join(format!("2026/08/25/rollout-x-{thread}.jsonl"));
        write(
            &rollout,
            &format!(
                "{{\"type\":\"session_meta\",\"payload\":{{\"session_id\":\"{thread}\"}}}}\n{{\"type\":\"event_msg\",\"payload\":{{\"type\":\"token_count\",\"info\":{{\"last_token_usage\":{{\"input_tokens\":16190,\"cached_input_tokens\":2432,\"output_tokens\":482,\"cache_write_input_tokens\":0}}}}}}}}\n"
            ),
        );
        let report = read_codex_usage(root.path(), thread, "reviewer").expect("usage");
        assert_eq!(report.role, "reviewer");
        assert_eq!(report.input_tokens, Some(16190));
        assert_eq!(report.output_tokens, Some(482));
        assert_eq!(report.cache_read_tokens, Some(2432));
    }

    #[test]
    fn local_usage_returns_none_for_missing_or_incomplete_codex_usage() {
        let root = tempdir().expect("tempdir");
        assert!(
            read_codex_usage(
                root.path(),
                "01a03700-9098-7070-9a21-de7c0d04d709",
                "author"
            )
            .is_none()
        );
        let thread = "01a03700-9098-7070-9a21-de7c0d04d709";
        let rollout = root
            .path()
            .join(format!("2026/08/25/rollout-x-{thread}.jsonl"));
        write(
            &rollout,
            &format!(
                "{{\"type\":\"session_meta\",\"payload\":{{\"session_id\":\"{thread}\"}}}}\n{{\"type\":\"event_msg\",\"payload\":{{\"type\":\"token_count\",\"info\":{{\"last_token_usage\":{{\"input_tokens\":1}}}}}}}}\n"
            ),
        );
        assert!(read_codex_usage(root.path(), thread, "author").is_none());
    }

    #[test]
    fn local_usage_aggregates_kimi_turn_usage_from_all_agents() {
        let root = tempdir().expect("tempdir");
        let cwd = Path::new("/workspace/naruto");
        let session = "448df463-188a-45c7-ad93-c3920ef68870";
        let key = "wd_naruto_88aa5146ca53";
        write(
            &root
                .path()
                .join(format!("{key}/session_{session}/agents/main/wire.jsonl")),
            "{\"type\":\"usage.record\",\"usageScope\":\"session\",\"usage\":{\"inputOther\":999,\"inputCacheRead\":999,\"inputCacheCreation\":999,\"output\":999}}\n{\"type\":\"usage.record\",\"usageScope\":\"turn\",\"usage\":{\"inputOther\":30,\"inputCacheRead\":4,\"inputCacheCreation\":2,\"output\":5}}\n",
        );
        write(
            &root.path().join(format!(
                "{key}/session_{session}/agents/subagent/wire.jsonl"
            )),
            "{\"type\":\"usage.record\",\"usageScope\":\"turn\",\"usage\":{\"inputOther\":7,\"inputCacheRead\":9,\"inputCacheCreation\":1,\"output\":3}}\n",
        );
        let report = read_kimi_usage(root.path(), cwd, session, "author").expect("usage");
        assert_eq!(report.input_tokens, Some(40));
        assert_eq!(report.output_tokens, Some(8));
        assert_eq!(report.cache_read_tokens, Some(13));
        assert_eq!(report.cache_creation_tokens, Some(3));
    }

    #[test]
    fn local_usage_returns_none_for_missing_or_malformed_kimi_record() {
        let root = tempdir().expect("tempdir");
        let cwd = Path::new("/workspace/naruto");
        let session = "448df463-188a-45c7-ad93-c3920ef68870";
        assert!(read_kimi_usage(root.path(), cwd, session, "author").is_none());
        let key = "wd_naruto_88aa5146ca53";
        write(
            &root
                .path()
                .join(format!("{key}/session_{session}/agents/main/wire.jsonl")),
            "broken json\n{\"type\":\"usage.record\",\"usageScope\":\"turn\",\"usage\":{\"inputOther\":1}}\n",
        );
        assert!(read_kimi_usage(root.path(), cwd, session, "author").is_none());
    }
}
