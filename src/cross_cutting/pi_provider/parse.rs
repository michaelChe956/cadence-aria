use serde_json::Value;

use crate::cross_cutting::streaming_provider::UsageReportData;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PiToolStart {
    pub(crate) tool_call_id: String,
    pub(crate) tool_name: String,
    pub(crate) args: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PiToolEnd {
    pub(crate) tool_call_id: String,
    pub(crate) tool_name: String,
    pub(crate) output: String,
    pub(crate) is_error: bool,
}

pub(crate) struct PiSelectRequest {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) options: Vec<String>,
}

pub(crate) fn parse_pi_select_request(value: &Value) -> Option<PiSelectRequest> {
    (value.get("type").and_then(Value::as_str) == Some("extension_ui_request")).then_some(())?;
    (value.get("method").and_then(Value::as_str) == Some("select")).then_some(())?;
    Some(PiSelectRequest {
        id: value.get("id")?.as_str()?.to_string(),
        title: value.get("title")?.as_str()?.to_string(),
        options: value
            .get("options")?
            .as_array()?
            .iter()
            .map(|option| option.as_str().map(ToString::to_string))
            .collect::<Option<Vec<_>>>()?,
    })
}

pub(crate) fn parse_pi_text_delta(value: &Value) -> Option<String> {
    (value.get("type").and_then(Value::as_str) == Some("message_update")).then_some(())?;
    let event = value.get("assistantMessageEvent")?;
    (event.get("type").and_then(Value::as_str) == Some("text_delta")).then_some(())?;
    event
        .get("delta")
        .and_then(Value::as_str)
        .filter(|delta| !delta.is_empty())
        .map(ToString::to_string)
}

pub(crate) fn parse_pi_tool_start(value: &Value) -> Option<PiToolStart> {
    (value.get("type").and_then(Value::as_str) == Some("tool_execution_start")).then_some(())?;
    Some(PiToolStart {
        tool_call_id: value.get("toolCallId")?.as_str()?.to_string(),
        tool_name: value.get("toolName")?.as_str()?.to_string(),
        args: value.get("args").cloned().unwrap_or(Value::Null),
    })
}

pub(crate) fn parse_pi_tool_end(value: &Value) -> Option<PiToolEnd> {
    (value.get("type").and_then(Value::as_str) == Some("tool_execution_end")).then_some(())?;
    let result = value.get("result").cloned().unwrap_or(Value::Null);
    Some(PiToolEnd {
        tool_call_id: value.get("toolCallId")?.as_str()?.to_string(),
        tool_name: value.get("toolName")?.as_str()?.to_string(),
        output: pi_result_output(&result),
        is_error: value
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

pub(crate) fn is_pi_terminal(value: &Value) -> bool {
    value.get("type").and_then(Value::as_str) == Some("agent_settled")
}

pub(crate) fn parse_pi_session_id(value: &Value) -> Option<String> {
    pi_get_state_data(value, "sessionId")
}

/// Pi RPC `get_state` 返回的绝对本地会话记录路径。该文件由 Pi CLI 管理，仅在
/// 协议 `data.cost` 缺失时用作 usage fallback；缺失或非绝对路径时不读取。
pub(crate) fn parse_pi_session_file(value: &Value) -> Option<std::path::PathBuf> {
    let path = pi_get_state_data(value, "sessionFile")?;
    let path = std::path::PathBuf::from(path);
    path.is_absolute().then_some(path)
}

fn pi_get_state_data(value: &Value, field: &str) -> Option<String> {
    (value.get("type").and_then(Value::as_str) == Some("response")).then_some(())?;
    (value.get("command").and_then(Value::as_str) == Some("get_state")).then_some(())?;
    (value.get("success").and_then(Value::as_bool) == Some(true)).then_some(())?;
    value
        .pointer(&format!("/data/{field}"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

/// 解析 pi RPC `get_state` 响应中的 `data.cost` 用量快照。
///
/// pi 的 get_state 响应见过 `cost: { input, output, cacheRead, cacheWrite }`（数值，
/// 可能为浮点）；此处统一取整为 u64。任一字段缺失记 `None`，整个 cost 缺失返回
/// `None`（best-effort，不视为错误）。注意：该计数为会话累计值。
pub(crate) fn parse_pi_usage(value: &Value, role: &str) -> Option<UsageReportData> {
    let cost = value.pointer("/data/cost")?;
    let number = |name: &str| -> Option<u64> {
        cost.get(name)
            .and_then(Value::as_number)
            .and_then(|number| {
                number
                    .as_u64()
                    .or_else(|| number.as_f64().map(|float| float as u64))
            })
    };
    let report = UsageReportData {
        role: role.to_string(),
        input_tokens: number("input"),
        output_tokens: number("output"),
        cache_read_tokens: number("cacheRead"),
        cache_creation_tokens: number("cacheWrite"),
    };
    report.has_any_tokens().then_some(report)
}

pub(crate) fn parse_pi_failure(value: &Value) -> Option<String> {
    if value.get("type").and_then(Value::as_str) == Some("response")
        && value.get("success").and_then(Value::as_bool) == Some(false)
    {
        let message = value
            .pointer("/error/message")
            .and_then(Value::as_str)
            .or_else(|| value.pointer("/data/message").and_then(Value::as_str))
            .or_else(|| value.get("error").and_then(Value::as_str))
            .unwrap_or("Pi command failed");
        return Some(message.to_string());
    }

    matches!(
        value.get("type").and_then(Value::as_str),
        Some("error" | "agent_error")
    )
    .then(|| {
        value
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("Pi provider reported an error")
            .to_string()
    })
}

fn pi_result_output(result: &Value) -> String {
    if let Some(output) = result.as_str() {
        return output.to_string();
    }
    result
        .get("content")
        .and_then(Value::as_array)
        .map(|content| {
            content
                .iter()
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .filter(|output| !output.is_empty())
        .unwrap_or_else(|| result.to_string())
}
