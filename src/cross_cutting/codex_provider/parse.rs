use serde_json::Value;

use crate::cross_cutting::streaming_provider::{
    ChoiceOptionData, ProviderExecutionEvent, ProviderExecutionEventKind,
    ProviderExecutionEventStatus,
};

#[derive(Debug, Clone)]
pub(crate) struct CodexApprovalRequest {
    pub(crate) rpc_id: Value,
    pub(crate) tool_name: String,
    pub(crate) description: String,
}

#[derive(Debug, Clone)]
pub(crate) struct CodexUserInputRequest {
    pub(crate) rpc_id: Value,
    pub(crate) id: String,
    pub(crate) question_id: String,
    pub(crate) prompt: String,
    pub(crate) options: Vec<ChoiceOptionData>,
    pub(crate) allow_free_text: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct AgentMessageText {
    pub(crate) item_id: String,
    pub(crate) content: String,
    pub(crate) completed: bool,
}

pub(crate) fn parse_agent_message_text(value: &Value) -> Option<AgentMessageText> {
    if value.get("method")?.as_str()? == "item/agentMessage/delta" {
        let content = value
            .pointer("/params/delta")
            .and_then(Value::as_str)
            .filter(|content| !content.is_empty())
            .map(ToString::to_string)?;
        return Some(AgentMessageText {
            item_id: value
                .pointer("/params/itemId")
                .and_then(Value::as_str)
                .unwrap_or("agent_message")
                .to_string(),
            content,
            completed: false,
        });
    }

    if value.get("method")?.as_str()? == "item/completed" {
        let item = value.pointer("/params/item")?;
        if !matches!(
            item.get("type").and_then(Value::as_str),
            Some("agentMessage" | "agent_message")
        ) {
            return None;
        }
        let content = agent_message_completed_text(item)?;
        return Some(AgentMessageText {
            item_id: item
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("agent_message")
                .to_string(),
            content,
            completed: true,
        });
    }

    if value.get("method")?.as_str()? != "codex/event" {
        return None;
    }
    let msg = value.get("params")?.get("msg")?;
    if msg.get("type")?.as_str()? != "item_completed" {
        return None;
    }
    let item = msg.get("item")?;
    if item.get("type")?.as_str()? != "message" || item.get("role")?.as_str()? != "assistant" {
        return None;
    }
    let content = item
        .get("content")?
        .as_array()?
        .iter()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|item| item.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("");

    (!content.is_empty()).then(|| AgentMessageText {
        item_id: item
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("legacy_message")
            .to_string(),
        content,
        completed: true,
    })
}

pub(crate) fn agent_message_completed_text(item: &Value) -> Option<String> {
    if let Some(text) = item
        .get("text")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
    {
        return Some(text.to_string());
    }

    let content = item.get("content")?.as_array()?;
    let text = content
        .iter()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|item| item.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("");
    (!text.is_empty()).then_some(text)
}

pub(crate) fn parse_execution_event(value: &Value) -> Option<ProviderExecutionEvent> {
    let method = value.get("method")?.as_str()?;
    if method != "item/started" && method != "item/completed" {
        return None;
    }

    let item = value.pointer("/params/item")?;
    if !is_command_execution_item(item) {
        return None;
    }

    let item_id = item.get("id").and_then(Value::as_str).unwrap_or("command");
    let command = command_description(item);
    let cwd = item
        .get("cwd")
        .and_then(Value::as_str)
        .or_else(|| value.pointer("/params/cwd").and_then(Value::as_str))
        .map(ToString::to_string);
    let exit_code = item
        .get("exitCode")
        .or_else(|| item.get("exit_code"))
        .and_then(Value::as_i64)
        .and_then(|code| i32::try_from(code).ok());
    let output = command_output(item);

    if method == "item/started" {
        return Some(ProviderExecutionEvent {
            event_id: format!("command_{item_id}"),
            kind: ProviderExecutionEventKind::Command,
            status: ProviderExecutionEventStatus::Started,
            title: "Command started".to_string(),
            detail: None,
            command,
            cwd,
            output: None,
            exit_code: None,
        });
    }

    Some(ProviderExecutionEvent {
        event_id: format!("command_{item_id}"),
        kind: ProviderExecutionEventKind::Command,
        status: if exit_code.is_some_and(|code| code != 0) {
            ProviderExecutionEventStatus::Failed
        } else {
            ProviderExecutionEventStatus::Completed
        },
        title: if exit_code.is_some_and(|code| code != 0) {
            "Command failed".to_string()
        } else {
            "Command completed".to_string()
        },
        detail: exit_code.map(|code| format!("exit code {code}")),
        command,
        cwd,
        output,
        exit_code,
    })
}

pub(crate) fn is_command_execution_item(item: &Value) -> bool {
    matches!(
        item.get("type").and_then(Value::as_str),
        Some("commandExecution" | "command_execution")
    )
}

pub(crate) fn command_output(item: &Value) -> Option<String> {
    ["aggregatedOutput", "aggregated_output", "output", "stdout"]
        .iter()
        .find_map(|field| item.get(field).and_then(Value::as_str))
        .filter(|output| !output.is_empty())
        .map(ToString::to_string)
}

pub(crate) fn parse_approval_request(value: &Value) -> Option<CodexApprovalRequest> {
    let method = value.get("method")?.as_str()?;
    if method == "codex/server_request" {
        let params = value.get("params")?;
        if params.get("type")?.as_str()? != "command_execution_request_approval" {
            return None;
        }
        let request_params = params.get("params").unwrap_or(params);
        return Some(CodexApprovalRequest {
            rpc_id: value
                .get("id")
                .cloned()
                .or_else(|| params.get("request_id").cloned())
                .unwrap_or(Value::Null),
            tool_name: "command".to_string(),
            description: command_description(request_params)
                .unwrap_or_else(|| "Codex command approval request".to_string()),
        });
    }

    if method == "item/commandExecution/requestApproval" {
        let params = value.get("params").unwrap_or(value);
        return Some(CodexApprovalRequest {
            rpc_id: value.get("id").cloned().unwrap_or(Value::Null),
            tool_name: "command".to_string(),
            description: command_description(params)
                .unwrap_or_else(|| "Codex command approval request".to_string()),
        });
    }

    None
}

pub(crate) fn parse_user_input_request(value: &Value) -> Option<CodexUserInputRequest> {
    if value.get("method")?.as_str()? != "item/tool/requestUserInput" {
        return None;
    }

    let rpc_id = value.get("id")?.clone();
    let id = rpc_id_string(&rpc_id)?;
    let question = value
        .pointer("/params/questions")
        .and_then(Value::as_array)?
        .first()?;
    let question_id = question.get("id").and_then(Value::as_str)?.to_string();
    let question_text = question
        .get("question")
        .and_then(Value::as_str)
        .or_else(|| question.get("header").and_then(Value::as_str))?
        .to_string();
    let options = question
        .get("options")
        .and_then(Value::as_array)
        .map(|options| {
            options
                .iter()
                .filter_map(|option| {
                    let label = option.get("label").and_then(Value::as_str)?;
                    Some(ChoiceOptionData {
                        id: label.to_string(),
                        label: label.to_string(),
                        description: option
                            .get("description")
                            .and_then(Value::as_str)
                            .map(ToString::to_string),
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let allow_free_text = options.is_empty()
        || question
            .get("isOther")
            .and_then(Value::as_bool)
            .unwrap_or(false);

    Some(CodexUserInputRequest {
        rpc_id,
        id,
        question_id,
        prompt: question_text,
        options,
        allow_free_text,
    })
}

pub(crate) fn command_description(params: &Value) -> Option<String> {
    let command = params.get("command")?;
    if let Some(command) = command.as_str() {
        return Some(command.to_string());
    }
    let args = command.as_array()?;
    let text = args
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>()
        .join(" ");
    if text.is_empty() { None } else { Some(text) }
}

pub(crate) fn rpc_id_string(value: &Value) -> Option<String> {
    value
        .as_u64()
        .map(|id| id.to_string())
        .or_else(|| value.as_str().map(ToString::to_string))
}

pub(crate) fn is_turn_completed(value: &Value) -> bool {
    value.get("method").and_then(Value::as_str) == Some("turn/completed")
        || value
            .pointer("/params/msg/type")
            .and_then(Value::as_str)
            .is_some_and(|event_type| event_type == "turn_completed")
}

pub(crate) fn parse_failure(value: &Value) -> Option<String> {
    let event_type = value.pointer("/params/msg/type").and_then(Value::as_str)?;
    if event_type == "turn_failed" || event_type == "error" {
        return value
            .pointer("/params/msg/message")
            .and_then(Value::as_str)
            .or_else(|| value.pointer("/params/msg/error").and_then(Value::as_str))
            .map(ToString::to_string)
            .or_else(|| Some("Codex turn failed".to_string()));
    }
    None
}
