use serde_json::Value;

use crate::cross_cutting::streaming_provider::{
    ChoiceOptionData, ChoiceQuestionData, CodexApprovalCategory, ProviderExecutionEvent,
    ProviderExecutionEventKind, ProviderExecutionEventStatus, UsageReportData,
};

/// 分类后的 codex 审批请求（GC6）：rpc_id 原样保留用于应答回带；category 按
/// method + `_meta.codex_approval_kind` 精确分类；fileChange 的 diff 关联信息
/// 通过 request_id（item id）在 session 层关联缓存 item，不使用自然语言 reason。
#[derive(Debug, Clone)]
pub(crate) struct CodexApprovalRequest {
    pub(crate) rpc_id: Value,
    pub(crate) category: CodexApprovalCategory,
    pub(crate) server_name: Option<String>,
    pub(crate) tool_name: Option<String>,
    pub(crate) request_id: String,
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
    pub(crate) questions: Vec<ChoiceQuestionData>,
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

/// 缓存 fileChange item 摘要（item id → "path (changeType)"），供
/// `item/fileChange/requestApproval` 经 item id 关联 diff 信息；不使用自然语言
/// `reason`。仅接受 `item/started` / `item/completed` 通知中的 fileChange item。
pub(crate) fn parse_file_change_summary(value: &Value) -> Option<(String, String)> {
    let method = value.get("method")?.as_str()?;
    if method != "item/started" && method != "item/completed" {
        return None;
    }
    let item = value.pointer("/params/item")?;
    if item.get("type").and_then(Value::as_str)? != "fileChange" {
        return None;
    }
    let id = item.get("id").and_then(Value::as_str)?.to_string();
    let path = item
        .get("path")
        .or_else(|| item.get("filePath"))
        .and_then(Value::as_str)
        .unwrap_or("file");
    let change_type = item
        .get("changeType")
        .or_else(|| item.get("change_type"))
        .and_then(Value::as_str)
        .unwrap_or("change");
    Some((id, format!("{path} ({change_type})")))
}

/// 按 GC6 冻结规则分类审批请求：method 名 + `_meta.codex_approval_kind` 精确判别，
/// 自然语言 `reason` 不作为分类依据；未知 elicitation / 未知 item 形态返回
/// `Unknown { method }`（保留原 method，不得静默不应答）。
pub(crate) fn parse_approval_request(value: &Value) -> Option<CodexApprovalRequest> {
    let method = value.get("method")?.as_str()?.to_string();
    let params = value.get("params").unwrap_or(value);
    let rpc_id = value.get("id").cloned().unwrap_or(Value::Null);

    let approval_request_id = || {
        params
            .get("requestId")
            .or_else(|| params.get("itemId"))
            .and_then(Value::as_str)
            .map(ToString::to_string)
            .or_else(|| rpc_id_string(&rpc_id))
            .unwrap_or_else(|| "codex_approval".to_string())
    };

    if method == "codex/server_request" {
        let server_request = value.get("params")?;
        if server_request.get("type")?.as_str()? != "command_execution_request_approval" {
            return None;
        }
        let request_params = server_request.get("params").unwrap_or(server_request);
        return Some(CodexApprovalRequest {
            rpc_id: value
                .get("id")
                .cloned()
                .or_else(|| server_request.get("request_id").cloned())
                .unwrap_or(Value::Null),
            category: CodexApprovalCategory::CommandExecution,
            server_name: None,
            tool_name: Some("command".to_string()),
            request_id: server_request
                .get("request_id")
                .and_then(Value::as_str)
                .map(ToString::to_string)
                .or_else(|| rpc_id_string(value.get("id").unwrap_or(&Value::Null)))
                .unwrap_or_else(|| "codex_command".to_string()),
            description: command_description(request_params)
                .unwrap_or_else(|| "Codex command approval request".to_string()),
        });
    }

    if method == "item/commandExecution/requestApproval" {
        return Some(CodexApprovalRequest {
            rpc_id,
            category: CodexApprovalCategory::CommandExecution,
            server_name: None,
            tool_name: Some("command".to_string()),
            request_id: params
                .get("itemId")
                .and_then(Value::as_str)
                .map(ToString::to_string)
                .or_else(|| rpc_id_string(value.get("id").unwrap_or(&Value::Null)))
                .unwrap_or_else(|| "codex_command".to_string()),
            description: command_description(params)
                .unwrap_or_else(|| "Codex command approval request".to_string()),
        });
    }

    if method == "item/fileChange/requestApproval" {
        return Some(CodexApprovalRequest {
            rpc_id,
            category: CodexApprovalCategory::FileChange,
            server_name: None,
            tool_name: Some("file_change".to_string()),
            // diff 关联键：item id（session 层用缓存 item 补全描述，不读 reason）
            request_id: params
                .get("itemId")
                .and_then(Value::as_str)
                .map(ToString::to_string)
                .or_else(|| rpc_id_string(value.get("id").unwrap_or(&Value::Null)))
                .unwrap_or_else(|| "codex_file_change".to_string()),
            description: "Codex file change approval request".to_string(),
        });
    }

    if method == "mcpServer/elicitation/request" {
        // 仅 `_meta.codex_approval_kind="mcp_tool_call"` 是 MCP（Task 0 实测契约），
        // 其余（含无 marker）均为未知 elicitation，保留原 method。
        let kind = value
            .pointer("/params/_meta/codex_approval_kind")
            .and_then(Value::as_str);
        let server_name = params
            .get("serverName")
            .and_then(Value::as_str)
            .map(ToString::to_string);
        let category = if kind == Some("mcp_tool_call") {
            CodexApprovalCategory::McpToolCall
        } else {
            CodexApprovalCategory::Unknown {
                method: method.clone(),
            }
        };
        let request_id = approval_request_id();
        let description = format!(
            "MCP tool call approval via {}",
            server_name.as_deref().unwrap_or("unknown server")
        );
        return Some(CodexApprovalRequest {
            rpc_id,
            category,
            server_name,
            tool_name: params
                .get("toolName")
                .or_else(|| value.pointer("/params/_meta/tool_name"))
                .and_then(Value::as_str)
                .map(ToString::to_string),
            request_id,
            description,
        });
    }

    // 未知 `item/*/requestApproval` 形态：返回 decline，保留原 method。
    if method.starts_with("item/") && method.ends_with("/requestApproval") {
        let request_id = approval_request_id();
        return Some(CodexApprovalRequest {
            rpc_id,
            category: CodexApprovalCategory::Unknown { method },
            server_name: None,
            tool_name: None,
            request_id,
            description: "Codex approval request".to_string(),
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
    let questions = value
        .pointer("/params/questions")
        .and_then(Value::as_array)?
        .iter()
        .filter_map(parse_user_input_question)
        .collect::<Vec<_>>();
    let first_question = questions.first()?;
    let question_id = first_question.id.clone();
    let question_text = if questions.len() > 1 {
        format!("请确认 {} 个问题", questions.len())
    } else {
        first_question.prompt.clone()
    };
    let options = first_question.options.clone();
    let allow_free_text = first_question.allow_free_text;

    Some(CodexUserInputRequest {
        rpc_id,
        id,
        question_id,
        prompt: question_text,
        options,
        allow_free_text,
        questions,
    })
}

fn parse_user_input_question(question: &Value) -> Option<ChoiceQuestionData> {
    let id = question.get("id").and_then(Value::as_str)?.to_string();
    let prompt = question
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
    Some(ChoiceQuestionData {
        id,
        prompt,
        options,
        allow_multiple: false,
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

/// 解析 codex `turn/completed`（或 legacy `codex/event` 的 `turn_completed`）附带的 usage。
///
/// app-server 新协议字段为 camelCase（`inputTokens` / `outputTokens` /
/// `cachedInputTokens`），legacy codex/event 为 snake_case（`input_tokens` / ...）。
/// 两种形状都尝试；任一字段缺失记 `None`，整个 usage 缺失返回 `None`（best-effort）。
pub(crate) fn parse_codex_usage(value: &Value, role: &'static str) -> Option<UsageReportData> {
    let usage = value
        .pointer("/params/usage")
        .or_else(|| value.pointer("/params/msg/usage"))?;
    let field = |names: &[&str]| {
        names
            .iter()
            .find_map(|name| usage.get(*name).and_then(Value::as_u64))
    };
    // 嵌套兑底：部分版本把用量包在 total_token_usage 对象里（JSON pointer 访问，非扁平 key）
    let nested = |pointer: &str| usage.pointer(pointer).and_then(Value::as_u64);
    let report = UsageReportData {
        role: role.to_string(),
        input_tokens: field(&["inputTokens", "input_tokens"])
            .or_else(|| nested("/total_token_usage/input_tokens"))
            .or_else(|| nested("/total_token_usage/prompt_tokens")),
        output_tokens: field(&["outputTokens", "output_tokens"])
            .or_else(|| nested("/total_token_usage/output_tokens")),
        cache_read_tokens: field(&["cachedInputTokens", "cached_input_tokens"])
            .or_else(|| nested("/total_token_usage/cached_input_tokens")),
        cache_creation_tokens: field(&["cacheWriteTokens", "cache_write_tokens"]),
    };
    report.has_any_tokens().then_some(report)
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
