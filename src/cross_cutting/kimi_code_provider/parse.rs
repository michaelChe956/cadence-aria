use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum KimiSessionUpdate {
    AgentThoughtChunk {
        content: String,
    },
    AgentMessageChunk {
        content: String,
    },
    ToolCall {
        tool_call_id: String,
        title: String,
        kind: String,
        status: String,
        arguments: Value,
    },
    ToolCallUpdate {
        tool_call_id: String,
        status: String,
        content: String,
    },
    SessionInfoUpdate {
        title: String,
    },
    UsageUpdate {
        used: u64,
        size: u64,
    },
    AvailableCommandsUpdate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct KimiPermissionRequest {
    pub request_id: Value,
    pub tool_call_id: String,
    pub title: String,
    pub options: Vec<KimiPermissionOption>,
    pub content_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct KimiPermissionOption {
    pub option_id: String,
    pub name: String,
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum KimiPromptResult {
    StopReason(String),
    Error(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Parsed {
    SessionUpdate(KimiSessionUpdate),
    RequestPermission(KimiPermissionRequest),
    PromptResult(KimiPromptResult),
    Unknown(String),
}

pub(crate) fn parse_message(v: &Value) -> Parsed {
    if v.get("method").and_then(Value::as_str) == Some("session/update") {
        let update = v.pointer("/params/update").unwrap_or(&Value::Null);
        let kind = update
            .get("sessionUpdate")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let text = |value: Option<&Value>| extract_text(value.unwrap_or(&Value::Null));
        let parsed = match kind {
            "agent_thought_chunk" => KimiSessionUpdate::AgentThoughtChunk {
                content: text(update.get("content")),
            },
            "agent_message_chunk" => KimiSessionUpdate::AgentMessageChunk {
                content: text(update.get("content")),
            },
            "tool_call" => KimiSessionUpdate::ToolCall {
                tool_call_id: string_field(update, &["toolCallId", "tool_call_id"]),
                title: string_field(update, &["title", "name"]),
                kind: string_field(update, &["kind"]),
                status: string_field(update, &["status"]),
                arguments: parse_tool_arguments(
                    update
                        .get("rawInput")
                        .or_else(|| update.get("arguments"))
                        .or_else(|| update.get("input")),
                ),
            },
            "tool_call_update" => KimiSessionUpdate::ToolCallUpdate {
                tool_call_id: string_field(update, &["toolCallId", "tool_call_id"]),
                status: string_field(update, &["status"]),
                content: text(update.get("content")),
            },
            "session_info_update" => KimiSessionUpdate::SessionInfoUpdate {
                title: string_field(update, &["title"]),
            },
            "usage_update" => KimiSessionUpdate::UsageUpdate {
                used: update
                    .get("used")
                    .and_then(Value::as_u64)
                    .unwrap_or_else(|| {
                        update
                            .pointer("/usage/used")
                            .and_then(Value::as_u64)
                            .unwrap_or_default()
                    }),
                size: update
                    .get("size")
                    .and_then(Value::as_u64)
                    .unwrap_or_else(|| {
                        update
                            .pointer("/usage/size")
                            .and_then(Value::as_u64)
                            .unwrap_or_default()
                    }),
            },
            "available_commands_update" => KimiSessionUpdate::AvailableCommandsUpdate,
            _ => return Parsed::Unknown("session/update".to_string()),
        };
        return Parsed::SessionUpdate(parsed);
    }

    if v.get("method").and_then(Value::as_str) == Some("session/request_permission") {
        let params = v.get("params").unwrap_or(&Value::Null);
        let tool_call = params.get("toolCall").unwrap_or(&Value::Null);
        let options = params
            .get("options")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .map(|item| KimiPermissionOption {
                        option_id: string_field(item, &["optionId", "option_id"]),
                        name: string_field(item, &["name"]),
                        kind: string_field(item, &["kind"]),
                    })
                    .collect()
            })
            .unwrap_or_default();
        return Parsed::RequestPermission(KimiPermissionRequest {
            request_id: v.get("id").cloned().unwrap_or(Value::Null),
            tool_call_id: string_field(tool_call, &["toolCallId", "tool_call_id"]),
            title: string_field(tool_call, &["title"]),
            options,
            content_text: extract_text(tool_call.get("content").unwrap_or(&Value::Null)),
        });
    }

    if v.get("id").is_some() && (v.get("result").is_some() || v.get("error").is_some()) {
        if let Some(error) = v.get("error") {
            return Parsed::PromptResult(KimiPromptResult::Error(
                error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("ACP request failed")
                    .to_string(),
            ));
        }
        let result = v.get("result").unwrap_or(&Value::Null);
        if let Some(stop_reason) = result.get("stopReason").and_then(Value::as_str) {
            return Parsed::PromptResult(KimiPromptResult::StopReason(stop_reason.to_string()));
        }
    }
    Parsed::Unknown(
        v.get("method")
            .and_then(Value::as_str)
            .unwrap_or("response")
            .to_string(),
    )
}

pub(crate) fn parse_tool_arguments(value: Option<&Value>) -> Value {
    let Some(value) = value else {
        return json!({});
    };
    match value {
        Value::String(raw) => serde_json::from_str(raw).unwrap_or_else(|_| json!({})),
        other => other.clone(),
    }
}

fn string_field(value: &Value, names: &[&str]) -> String {
    names
        .iter()
        .find_map(|name| value.get(*name).and_then(Value::as_str))
        .unwrap_or_default()
        .to_string()
}

fn extract_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(items) => items.iter().map(extract_text).collect(),
        Value::Object(map) => map
            .get("text")
            .map(extract_text)
            .or_else(|| map.get("content").map(extract_text))
            .unwrap_or_default(),
        _ => String::new(),
    }
}
