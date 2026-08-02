use serde_json::Value;

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
    (value.get("type").and_then(Value::as_str) == Some("response")).then_some(())?;
    (value.get("command").and_then(Value::as_str) == Some("get_state")).then_some(())?;
    (value.get("success").and_then(Value::as_bool) == Some(true)).then_some(())?;
    value
        .pointer("/data/sessionId")
        .and_then(Value::as_str)
        .filter(|session_id| !session_id.is_empty())
        .map(ToString::to_string)
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
