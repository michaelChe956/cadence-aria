//! 诊断轨迹事件投影。
//!
//! 把各 provider 的原生日志记录投影成固定六项白名单事实：
//! `timestamp`、`type`、`model`、`tool_type`、`exit_code`、`error_type`。
//!
//! 约束（安全边界）：
//! - 只在已知协议封装位置读取字段，不递归扫描任意正文。
//! - 所有分类值都来自固定词典：未知一律 `other`（或省略），绝不回显源里的任意字符串。
//! - 时间先经 chrono 校验再统一格式化，绝不回显原始时间字符串。
//! - 输出中永远没有 `role`、会话 ID、命令、工具入参、工具输出、提示正文或错误详情。

use chrono::{DateTime, Utc};
use serde_json::{Map, Value};

/// 事件类型固定词典。
const TYPE_MESSAGE: &str = "message";
const TYPE_TOOL_CALL: &str = "tool_call";
const TYPE_TOOL_RESULT: &str = "tool_result";
const TYPE_COMMAND: &str = "command";
const TYPE_ERROR: &str = "error";
const TYPE_SESSION: &str = "session";
const TYPE_USAGE: &str = "usage";
const TYPE_SYSTEM: &str = "system";
const TYPE_OTHER: &str = "other";

/// 工具类型固定词典。
const TOOL_SHELL: &str = "shell";
const TOOL_FILE_READ: &str = "file_read";
const TOOL_FILE_WRITE: &str = "file_write";
const TOOL_FILE_EDIT: &str = "file_edit";
const TOOL_FILE_LIST: &str = "file_list";
const TOOL_SEARCH: &str = "search";
const TOOL_WEB_FETCH: &str = "web_fetch";
const TOOL_WEB_SEARCH: &str = "web_search";
const TOOL_TASK: &str = "task";
const TOOL_TODO: &str = "todo";
const TOOL_MCP: &str = "mcp";
const TOOL_OTHER: &str = "other";

/// 错误类型固定词典。
const ERROR_RATE_LIMIT: &str = "rate_limit";
const ERROR_AUTH: &str = "auth";
const ERROR_PERMISSION: &str = "permission";
const ERROR_TIMEOUT: &str = "timeout";
const ERROR_NETWORK: &str = "network";
const ERROR_CONTEXT_LENGTH: &str = "context_length";
const ERROR_TOOL: &str = "tool_error";
const ERROR_API: &str = "api";
const ERROR_OVERLOADED: &str = "overloaded";
const ERROR_OTHER: &str = "other";

/// 四家受支持的 provider。
#[derive(Clone, Copy)]
enum ProviderKind {
    ClaudeCode,
    Codex,
    Pi,
    KimiCode,
}

/// 把一条原生日志记录投影成白名单事件事实。
///
/// 未知 provider 或未知记录形态也返回事件事实（`type` 固定为 `other`），
/// 但绝不携带自由文本、代码、请求体或结果体。
pub(super) fn project(provider: &str, record: &Value) -> Value {
    let mut facts = Map::new();
    match provider_kind(provider) {
        Some(ProviderKind::ClaudeCode) => project_claude_code(record, &mut facts),
        Some(ProviderKind::Codex) => project_codex(record, &mut facts),
        Some(ProviderKind::Pi) => project_pi(record, &mut facts),
        Some(ProviderKind::KimiCode) => project_kimi_code(record, &mut facts),
        None => {}
    }
    if !facts.contains_key("type") {
        facts.insert("type".to_string(), Value::String(TYPE_OTHER.to_string()));
    }
    Value::Object(facts)
}

/// provider 名归一：接受 `claude-code` / `claude_code` 两种写法，大小写不敏感。
fn provider_kind(provider: &str) -> Option<ProviderKind> {
    let trimmed = provider.trim();
    const CLAUDE_CODE: &[&str] = &["claude_code", "claude-code"];
    const KIMI_CODE: &[&str] = &["kimi_code", "kimi-code"];
    if CLAUDE_CODE
        .iter()
        .any(|name| trimmed.eq_ignore_ascii_case(name))
    {
        return Some(ProviderKind::ClaudeCode);
    }
    if trimmed.eq_ignore_ascii_case("codex") {
        return Some(ProviderKind::Codex);
    }
    if trimmed.eq_ignore_ascii_case("pi") {
        return Some(ProviderKind::Pi);
    }
    if KIMI_CODE
        .iter()
        .any(|name| trimmed.eq_ignore_ascii_case(name))
    {
        return Some(ProviderKind::KimiCode);
    }
    None
}

fn set_str(facts: &mut Map<String, Value>, key: &str, value: &str) {
    facts.insert(key.to_string(), Value::String(value.to_string()));
}

/// Claude Code：`message.content[]` 协议块。
fn project_claude_code(record: &Value, facts: &mut Map<String, Value>) {
    let message = record.get("message").filter(|value| value.is_object());
    if let Some(timestamp) = record
        .get("timestamp")
        .and_then(format_timestamp)
        .or_else(|| {
            message
                .and_then(|m| m.get("timestamp"))
                .and_then(format_timestamp)
        })
    {
        set_str(facts, "timestamp", &timestamp);
    }
    if let Some(model) = message
        .and_then(|m| m.get("model"))
        .and_then(Value::as_str)
        .or_else(|| record.get("model").and_then(Value::as_str))
    {
        set_str(facts, "model", model_family(model));
    }

    let mut event_type = match record.get("type").and_then(Value::as_str) {
        Some("assistant") | Some("user") | Some("human") => TYPE_MESSAGE,
        Some("system") => TYPE_SYSTEM,
        _ => TYPE_OTHER,
    };
    let mut tool_name: Option<&str> = None;
    let mut tool_error = false;
    if let Some(blocks) = content_blocks(message) {
        for block in blocks {
            let Some(block_type) = block.get("type").and_then(Value::as_str) else {
                continue;
            };
            match block_type {
                "tool_use" | "toolCall" | "tool_call" => {
                    event_type = TYPE_TOOL_CALL;
                    if tool_name.is_none() {
                        tool_name = block
                            .get("name")
                            .and_then(Value::as_str)
                            .or_else(|| block.get("toolName").and_then(Value::as_str));
                    }
                }
                "tool_result" | "toolResult" => {
                    event_type = TYPE_TOOL_RESULT;
                    if block.get("is_error").and_then(Value::as_bool) == Some(true)
                        || block.get("isError").and_then(Value::as_bool) == Some(true)
                    {
                        tool_error = true;
                    }
                }
                _ => {}
            }
        }
    }

    if let Some(name) = tool_name {
        set_str(facts, "tool_type", tool_family(name));
    }
    if tool_error {
        set_str(facts, "error_type", ERROR_TOOL);
    } else if let Some(error_type) = record
        .get("error")
        .and_then(|error| error.get("type"))
        .and_then(Value::as_str)
    {
        set_str(facts, "error_type", error_family(error_type));
    } else if record.get("isApiErrorMessage").and_then(Value::as_bool) == Some(true) {
        set_str(facts, "error_type", ERROR_API);
    }
    set_str(facts, "type", event_type);
}

/// Codex：`type` + `payload` 协议封装。
fn project_codex(record: &Value, facts: &mut Map<String, Value>) {
    let payload = record.get("payload").filter(|value| value.is_object());
    if let Some(timestamp) = record
        .get("timestamp")
        .and_then(format_timestamp)
        .or_else(|| {
            payload
                .and_then(|p| p.get("timestamp"))
                .and_then(format_timestamp)
        })
    {
        set_str(facts, "timestamp", &timestamp);
    }
    if let Some(model) = payload
        .and_then(|p| p.get("model"))
        .and_then(Value::as_str)
        .or_else(|| record.get("model").and_then(Value::as_str))
    {
        set_str(facts, "model", model_family(model));
    }

    let record_type = record
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let payload_type = payload
        .and_then(|p| p.get("type"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let event_type = match record_type {
        "session_meta" => TYPE_SESSION,
        "response_item" => match payload_type {
            "function_call" | "local_shell_call" | "custom_tool_call" | "web_search_call" => {
                TYPE_TOOL_CALL
            }
            "function_call_output" | "local_shell_call_output" | "custom_tool_call_output" => {
                TYPE_TOOL_RESULT
            }
            "message" => TYPE_MESSAGE,
            _ => TYPE_OTHER,
        },
        "event_msg" => match payload_type {
            "exec_command_begin" | "exec_command_end" => TYPE_COMMAND,
            "error" => TYPE_ERROR,
            "token_count" => TYPE_USAGE,
            "agent_message" | "agent_reasoning" | "user_message" => TYPE_MESSAGE,
            _ => TYPE_OTHER,
        },
        _ => TYPE_OTHER,
    };

    if let Some(name) = payload
        .and_then(|p| p.get("name"))
        .and_then(Value::as_str)
        .or_else(|| {
            payload
                .and_then(|p| p.get("tool_name"))
                .and_then(Value::as_str)
        })
    {
        set_str(facts, "tool_type", tool_family(name));
    }
    if let Some(exit_code) = payload
        .and_then(|p| p.get("exit_code"))
        .and_then(exit_code_of)
        .or_else(|| {
            payload
                .and_then(|p| p.get("exitCode"))
                .and_then(exit_code_of)
        })
    {
        facts.insert("exit_code".to_string(), Value::from(exit_code));
    }
    if let Some(error_type) = payload
        .and_then(|p| p.get("error"))
        .and_then(|error| error.get("type"))
        .and_then(Value::as_str)
    {
        set_str(facts, "error_type", error_family(error_type));
    }
    set_str(facts, "type", event_type);
}

/// Pi：`payload` 封装与 `message.content[]`。
fn project_pi(record: &Value, facts: &mut Map<String, Value>) {
    let envelope = record
        .get("payload")
        .filter(|value| value.is_object())
        .unwrap_or(record);
    let message = envelope.get("message").filter(|value| value.is_object());

    if let Some(timestamp) = message
        .and_then(|m| m.get("timestamp"))
        .and_then(format_timestamp)
        .or_else(|| envelope.get("timestamp").and_then(format_timestamp))
        .or_else(|| record.get("timestamp").and_then(format_timestamp))
    {
        set_str(facts, "timestamp", &timestamp);
    }
    if let Some(model) = message
        .and_then(|m| m.get("model"))
        .and_then(Value::as_str)
        .or_else(|| envelope.get("model").and_then(Value::as_str))
    {
        set_str(facts, "model", model_family(model));
    }

    let envelope_type = envelope
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let mut event_type = match envelope_type {
        "tool_execution_start" => TYPE_TOOL_CALL,
        "tool_execution_end" => TYPE_TOOL_RESULT,
        "message_start" | "message_update" | "message_end" | "turn_start" | "turn_end" => {
            TYPE_MESSAGE
        }
        _ => TYPE_OTHER,
    };
    let mut tool_name: Option<&str> = None;
    let mut tool_error = false;
    if matches!(envelope_type, "tool_execution_start" | "tool_execution_end") {
        tool_name = envelope
            .get("toolName")
            .and_then(Value::as_str)
            .or_else(|| envelope.get("name").and_then(Value::as_str));
        tool_error = envelope.get("isError").and_then(Value::as_bool) == Some(true);
    }

    if let Some(blocks) = content_blocks(message) {
        for block in blocks {
            let Some(block_type) = block.get("type").and_then(Value::as_str) else {
                continue;
            };
            match block_type {
                "tool_use" | "toolCall" | "tool_call" => {
                    if event_type == TYPE_MESSAGE {
                        event_type = TYPE_TOOL_CALL;
                    }
                    if tool_name.is_none() {
                        tool_name = block
                            .get("name")
                            .and_then(Value::as_str)
                            .or_else(|| block.get("toolName").and_then(Value::as_str));
                    }
                }
                "tool_result" | "toolResult" => {
                    if event_type == TYPE_MESSAGE {
                        event_type = TYPE_TOOL_RESULT;
                    }
                    if block.get("is_error").and_then(Value::as_bool) == Some(true)
                        || block.get("isError").and_then(Value::as_bool) == Some(true)
                    {
                        tool_error = true;
                    }
                }
                _ => {}
            }
        }
    }

    if let Some(name) = tool_name {
        set_str(facts, "tool_type", tool_family(name));
    }
    if tool_error {
        set_str(facts, "error_type", ERROR_TOOL);
    } else if let Some(error_type) = envelope
        .get("error")
        .and_then(|error| error.get("type"))
        .and_then(Value::as_str)
    {
        set_str(facts, "error_type", error_family(error_type));
    }
    set_str(facts, "type", event_type);
}

/// Kimi Code：JSON-RPC wire 封装与 `usage.record`。
fn project_kimi_code(record: &Value, facts: &mut Map<String, Value>) {
    let wire = record
        .get("wire")
        .filter(|value| value.is_object())
        .unwrap_or(record);
    let params = wire.get("params").filter(|value| value.is_object());
    let update = params
        .and_then(|p| p.get("update"))
        .filter(|value| value.is_object())
        .or_else(|| wire.get("update").filter(|value| value.is_object()))
        .or_else(|| record.get("update").filter(|value| value.is_object()));
    let usage_record = record.get("record").filter(|value| value.is_object());
    let message = wire
        .get("message")
        .filter(|value| value.is_object())
        .or_else(|| record.get("message").filter(|value| value.is_object()));

    if let Some(timestamp) = wire
        .get("timestamp")
        .and_then(format_timestamp)
        .or_else(|| {
            update
                .and_then(|u| u.get("timestamp"))
                .and_then(format_timestamp)
        })
        .or_else(|| record.get("timestamp").and_then(format_timestamp))
    {
        set_str(facts, "timestamp", &timestamp);
    }
    if let Some(model) = usage_record
        .and_then(|r| r.get("model"))
        .and_then(Value::as_str)
        .or_else(|| update.and_then(|u| u.get("model")).and_then(Value::as_str))
        .or_else(|| record.get("model").and_then(Value::as_str))
        .or_else(|| message.and_then(|m| m.get("model")).and_then(Value::as_str))
    {
        set_str(facts, "model", model_family(model));
    }

    let mut event_type = TYPE_OTHER;
    let mut tool_name: Option<&str> = None;
    let mut tool_kind: Option<&str> = None;
    let mut tool_error = false;

    if let Some(update) = update {
        match update.get("sessionUpdate").and_then(Value::as_str) {
            Some("tool_call") | Some("tool_call_update") => {
                event_type = if update.get("sessionUpdate").and_then(Value::as_str)
                    == Some("tool_call_update")
                {
                    TYPE_TOOL_RESULT
                } else {
                    TYPE_TOOL_CALL
                };
                tool_name = update.get("title").and_then(Value::as_str);
                tool_kind = update.get("kind").and_then(Value::as_str);
                tool_error = update.get("status").and_then(Value::as_str) == Some("failed");
            }
            Some("agent_message_chunk")
            | Some("user_message_chunk")
            | Some("agent_thought_chunk") => {
                event_type = TYPE_MESSAGE;
            }
            _ => {}
        }
    }

    if let Some(blocks) = content_blocks(message) {
        for block in blocks {
            let Some(block_type) = block.get("type").and_then(Value::as_str) else {
                continue;
            };
            match block_type {
                "tool_use" | "toolCall" | "tool_call" => {
                    if event_type == TYPE_OTHER {
                        event_type = TYPE_TOOL_CALL;
                    }
                    if tool_name.is_none() {
                        tool_name = block
                            .get("name")
                            .and_then(Value::as_str)
                            .or_else(|| block.get("toolName").and_then(Value::as_str));
                    }
                }
                "tool_result" | "toolResult" => {
                    if event_type == TYPE_OTHER {
                        event_type = TYPE_TOOL_RESULT;
                    }
                    if block.get("is_error").and_then(Value::as_bool) == Some(true)
                        || block.get("isError").and_then(Value::as_bool) == Some(true)
                    {
                        tool_error = true;
                    }
                }
                _ => {}
            }
        }
    }

    if event_type == TYPE_OTHER
        && (usage_record.is_some() || record.get("type").and_then(Value::as_str) == Some("usage"))
    {
        event_type = TYPE_USAGE;
    }

    if let Some(name) = tool_name {
        set_str(facts, "tool_type", tool_family(name));
    } else if let Some(kind) = tool_kind {
        set_str(facts, "tool_type", tool_kind_family(kind));
    }
    if tool_error {
        set_str(facts, "error_type", ERROR_TOOL);
    } else if let Some(error_type) = update
        .and_then(|u| u.get("error"))
        .and_then(|error| error.get("type"))
        .and_then(Value::as_str)
    {
        set_str(facts, "error_type", error_family(error_type));
    }
    set_str(facts, "type", event_type);
}

/// 取消息封装里的内容块（已知协议位置，不递归扫描）。
fn content_blocks(message: Option<&Value>) -> Option<&[Value]> {
    message
        .and_then(|m| m.get("content"))
        .and_then(Value::as_array)
        .map(|blocks| blocks.as_slice())
}

/// 时间：数值纪元秒/毫秒或 RFC3339 字符串，先经 chrono 校验再统一格式化。
fn format_timestamp(raw: &Value) -> Option<String> {
    match raw {
        Value::Number(number) => {
            let value = number.as_f64()?;
            if !value.is_finite() {
                return None;
            }
            let (seconds, nanos) = if value.abs() >= 100_000_000_000.0 {
                let millis = value.trunc();
                (millis / 1000.0, ((millis % 1000.0) * 1_000_000.0) as u32)
            } else {
                let whole = value.trunc();
                (whole, ((value - whole) * 1_000_000_000.0).round() as u32)
            };
            let datetime = DateTime::from_timestamp(seconds as i64, nanos.min(999_999_999))?;
            Some(format_datetime(&datetime))
        }
        Value::String(text) => {
            let parsed = DateTime::parse_from_rfc3339(text).ok()?;
            Some(format_datetime(&parsed.with_timezone(&Utc)))
        }
        _ => None,
    }
}

fn format_datetime(datetime: &DateTime<Utc>) -> String {
    datetime.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

/// 退出码：只接受整数数值。
fn exit_code_of(raw: &Value) -> Option<i64> {
    match raw {
        Value::Number(number) => number.as_i64().or_else(|| {
            number
                .as_f64()
                .filter(|v| v.fract() == 0.0)
                .map(|v| v as i64)
        }),
        _ => None,
    }
}

/// 模型固定词典：标准系列或显式已知型号，未知固定 `other`。
fn model_family(raw: &str) -> &'static str {
    const FAMILIES: &[&str] = &[
        "claude-opus",
        "claude-sonnet",
        "claude-haiku",
        "gpt-5.1-codex",
        "gpt-5-codex",
        "gpt-5.1",
        "gpt-5",
        "gpt-4.1",
        "gpt-4",
        "o4-mini",
        "o3",
        "kimi-k2-thinking",
        "kimi-k2",
        "kimi-k1.5",
        "moonshot-v1",
        "qwen3",
        "glm-4.6",
        "deepseek-v3",
        "gemini-2.5-pro",
        "grok-4",
    ];
    for family in FAMILIES {
        if starts_with_ignore_ascii_case(raw, family) {
            return family;
        }
    }
    "other"
}

/// 大小写不敏感的前缀比较（不分配）。
fn starts_with_ignore_ascii_case(text: &str, prefix: &str) -> bool {
    let text = text.as_bytes();
    let prefix = prefix.as_bytes();
    text.len() >= prefix.len() && text[..prefix.len()].eq_ignore_ascii_case(prefix)
}

/// 工具类型固定词典：工具名 → 类型，未知固定 `other`。
fn tool_family(raw: &str) -> &'static str {
    const NAMES: &[(&str, &str)] = &[
        ("bash", TOOL_SHELL),
        ("shell", TOOL_SHELL),
        ("run_terminal_cmd", TOOL_SHELL),
        ("execute_command", TOOL_SHELL),
        ("exec_command", TOOL_SHELL),
        ("run_command", TOOL_SHELL),
        ("terminal", TOOL_SHELL),
        ("read", TOOL_FILE_READ),
        ("read_file", TOOL_FILE_READ),
        ("view_file", TOOL_FILE_READ),
        ("cat", TOOL_FILE_READ),
        ("write", TOOL_FILE_WRITE),
        ("write_file", TOOL_FILE_WRITE),
        ("create_file", TOOL_FILE_WRITE),
        ("edit", TOOL_FILE_EDIT),
        ("multi_edit", TOOL_FILE_EDIT),
        ("str_replace", TOOL_FILE_EDIT),
        ("apply_patch", TOOL_FILE_EDIT),
        ("notebookedit", TOOL_FILE_EDIT),
        ("glob", TOOL_FILE_LIST),
        ("ls", TOOL_FILE_LIST),
        ("list_dir", TOOL_FILE_LIST),
        ("grep", TOOL_SEARCH),
        ("search", TOOL_SEARCH),
        ("codebase_search", TOOL_SEARCH),
        ("web_fetch", TOOL_WEB_FETCH),
        ("webfetch", TOOL_WEB_FETCH),
        ("fetch", TOOL_WEB_FETCH),
        ("web_search", TOOL_WEB_SEARCH),
        ("websearch", TOOL_WEB_SEARCH),
        ("task", TOOL_TASK),
        ("agent", TOOL_TASK),
        ("subagent", TOOL_TASK),
        ("todo", TOOL_TODO),
        ("todowrite", TOOL_TODO),
        ("update_todo", TOOL_TODO),
    ];
    for (name, family) in NAMES {
        if raw.eq_ignore_ascii_case(name) {
            return family;
        }
    }
    if starts_with_ignore_ascii_case(raw, "mcp__") {
        return TOOL_MCP;
    }
    TOOL_OTHER
}

/// ACP `kind` → 工具类型固定词典。
fn tool_kind_family(raw: &str) -> &'static str {
    const KINDS: &[(&str, &str)] = &[
        ("execute", TOOL_SHELL),
        ("read", TOOL_FILE_READ),
        ("edit", TOOL_FILE_EDIT),
        ("move", TOOL_FILE_EDIT),
        ("delete", TOOL_FILE_WRITE),
        ("search", TOOL_SEARCH),
        ("fetch", TOOL_WEB_FETCH),
    ];
    for (kind, family) in KINDS {
        if raw.eq_ignore_ascii_case(kind) {
            return family;
        }
    }
    TOOL_OTHER
}

/// 错误类型固定词典：未知固定 `other`。
fn error_family(raw: &str) -> &'static str {
    const NAMES: &[(&str, &str)] = &[
        ("rate_limit", ERROR_RATE_LIMIT),
        ("rate_limit_error", ERROR_RATE_LIMIT),
        ("429", ERROR_RATE_LIMIT),
        ("authentication", ERROR_AUTH),
        ("authentication_error", ERROR_AUTH),
        ("invalid_api_key", ERROR_AUTH),
        ("401", ERROR_AUTH),
        ("permission", ERROR_PERMISSION),
        ("permission_error", ERROR_PERMISSION),
        ("permission_denied", ERROR_PERMISSION),
        ("403", ERROR_PERMISSION),
        ("timeout", ERROR_TIMEOUT),
        ("request_timeout", ERROR_TIMEOUT),
        ("deadline_exceeded", ERROR_TIMEOUT),
        ("connection_error", ERROR_NETWORK),
        ("network_error", ERROR_NETWORK),
        ("connection_reset", ERROR_NETWORK),
        ("context_length_exceeded", ERROR_CONTEXT_LENGTH),
        ("context_length", ERROR_CONTEXT_LENGTH),
        ("tool_error", ERROR_TOOL),
        ("tool_use_error", ERROR_TOOL),
        ("api_error", ERROR_API),
        ("internal_server_error", ERROR_API),
        ("500", ERROR_API),
        ("503", ERROR_API),
        ("overloaded", ERROR_OVERLOADED),
        ("overloaded_error", ERROR_OVERLOADED),
        ("529", ERROR_OVERLOADED),
    ];
    for (name, family) in NAMES {
        if raw.eq_ignore_ascii_case(name) {
            return family;
        }
    }
    ERROR_OTHER
}

#[cfg(test)]
mod tests {
    include!("trace_tests.rs");
}
