// `trace::project` 的白名单投影测试。
//
// 覆盖四家 provider 的原生记录形态，断言：只输出六项白名单事实、
// 嵌套命令入参/输出/正文不外带、伪装成字段值的秘密不外带、
// 真实工具与退出码事实仍保留、绝不输出 role。

use super::project;
use serde_json::Value;

const ALLOWED_KEYS: [&str; 6] = [
    "timestamp",
    "type",
    "model",
    "tool_type",
    "exit_code",
    "error_type",
];

fn assert_only_allowed_keys(event: &Value) {
    let object = event.as_object().expect("投影结果必须是对象");
    for (key, value) in object {
        assert!(
            ALLOWED_KEYS.contains(&key.as_str()),
            "出现白名单外的键：{key}"
        );
        match key.as_str() {
            "exit_code" => assert!(value.is_i64(), "exit_code 必须是数字"),
            _ => assert!(value.is_string(), "{key} 必须是字符串"),
        }
    }
}

fn assert_absent(event: &Value, forbidden: &str) {
    let rendered = event.to_string();
    assert!(
        !rendered.contains(forbidden),
        "投影结果泄露了禁止内容 {forbidden}：{rendered}"
    );
}

#[test]
fn claude_code_projects_only_whitelisted_tool_facts() {
    let record = serde_json::json!({
        "type": "assistant",
        "timestamp": "2026-05-06T09:08:09.123+02:00",
        "sessionId": "session-secret",
        "cwd": "/home/user/private-project",
        "uuid": "uuid-secret",
        "message": {
            "role": "assistant",
            "model": "claude-sonnet-4-5-20250929",
            "content": [
                {"type": "text", "text": "SECRET-BODY"},
                {
                    "type": "tool_use",
                    "id": "toolu_1",
                    "name": "Bash",
                    "input": {"command": "SECRET-COMMAND"}
                }
            ]
        }
    });

    let event = project("claude_code", &record);

    assert_eq!(event["type"], "tool_call");
    assert_eq!(event["tool_type"], "shell");
    assert_eq!(event["model"], "claude-sonnet");
    assert_eq!(event["timestamp"], "2026-05-06T07:08:09.123Z");
    assert_only_allowed_keys(&event);
    for forbidden in [
        "SECRET-BODY",
        "SECRET-COMMAND",
        "session-secret",
        "uuid-secret",
        "private-project",
        "toolu_1",
        "role",
    ] {
        assert_absent(&event, forbidden);
    }
}

#[test]
fn claude_code_tool_result_keeps_error_fact_without_content() {
    let record = serde_json::json!({
        "type": "user",
        "timestamp": "2026-05-06T07:08:10.000Z",
        "message": {
            "role": "user",
            "content": [
                {
                    "type": "tool_result",
                    "tool_use_id": "toolu_1",
                    "is_error": true,
                    "content": "SECRET-STDOUT"
                }
            ]
        }
    });

    let event = project("claude_code", &record);

    assert_eq!(event["type"], "tool_result");
    assert_eq!(event["error_type"], "tool_error");
    assert_only_allowed_keys(&event);
    assert_absent(&event, "SECRET-STDOUT");
    assert_absent(&event, "role");
}

#[test]
fn codex_projects_session_item_and_command_events() {
    let function_call = serde_json::json!({
        "timestamp": "2026-01-02T03:04:05.000Z",
        "type": "response_item",
        "payload": {
            "type": "function_call",
            "name": "shell",
            "call_id": "call-secret",
            "arguments": "{\"command\":\"SECRET-COMMAND\"}"
        }
    });
    let event = project("codex", &function_call);
    assert_eq!(event["type"], "tool_call");
    assert_eq!(event["tool_type"], "shell");
    assert_only_allowed_keys(&event);
    assert_absent(&event, "SECRET-COMMAND");
    assert_absent(&event, "call-secret");

    let exec_end = serde_json::json!({
        "timestamp": "2026-01-02T03:04:06.000Z",
        "type": "event_msg",
        "payload": {
            "type": "exec_command_end",
            "call_id": "call-secret",
            "exit_code": 0,
            "stdout": "SECRET-STDOUT"
        }
    });
    let event = project("codex", &exec_end);
    assert_eq!(event["type"], "command");
    assert_eq!(event["exit_code"], 0);
    assert_only_allowed_keys(&event);
    assert_absent(&event, "SECRET-STDOUT");

    let session_meta = serde_json::json!({
        "timestamp": "2026-01-02T03:04:00.000Z",
        "type": "session_meta",
        "payload": {
            "id": "session-secret",
            "cwd": "/home/user/private-project",
            "instructions": "SECRET-INSTRUCTIONS"
        }
    });
    let event = project("codex", &session_meta);
    assert_eq!(event["type"], "session");
    assert_only_allowed_keys(&event);
    assert_absent(&event, "SECRET-INSTRUCTIONS");
    assert_absent(&event, "session-secret");
}

#[test]
fn codex_keeps_model_and_error_facts_from_known_positions() {
    let turn_context = serde_json::json!({
        "timestamp": "2026-01-02T03:04:01.000Z",
        "type": "turn_context",
        "payload": {"model": "gpt-5-codex", "cwd": "/home/user/private-project"}
    });
    let event = project("codex", &turn_context);
    assert_eq!(event["model"], "gpt-5-codex");
    assert_absent(&event, "private-project");

    let error = serde_json::json!({
        "timestamp": "2026-01-02T03:04:07.000Z",
        "type": "event_msg",
        "payload": {
            "type": "error",
            "error": {"type": "rate_limit_error", "message": "SECRET-DETAIL"}
        }
    });
    let event = project("codex", &error);
    assert_eq!(event["type"], "error");
    assert_eq!(event["error_type"], "rate_limit");
    assert_only_allowed_keys(&event);
    assert_absent(&event, "SECRET-DETAIL");
}

#[test]
fn pi_projects_message_content_tool_calls() {
    let record = serde_json::json!({
        "direction": "pi_to_client",
        "payload": {
            "type": "message_start",
            "message": {
                "role": "assistant",
                "model": "claude-opus-5",
                "timestamp": 1785652953923i64,
                "content": [
                    {"type": "toolCall", "id": "toolu_x", "name": "bash", "arguments": {"command": "SECRET-COMMAND"}}
                ]
            }
        }
    });

    let event = project("pi", &record);

    assert_eq!(event["type"], "tool_call");
    assert_eq!(event["tool_type"], "shell");
    assert_eq!(event["model"], "claude-opus");
    assert_eq!(event["timestamp"], "2026-08-02T06:42:33.923Z");
    assert_only_allowed_keys(&event);
    for forbidden in ["SECRET-COMMAND", "toolu_x", "role", "pi_to_client"] {
        assert_absent(&event, forbidden);
    }
}

#[test]
fn pi_projects_tool_execution_end_error_fact() {
    let record = serde_json::json!({
        "direction": "pi_to_client",
        "payload": {
            "type": "tool_execution_end",
            "toolCallId": "tool-secret",
            "toolName": "bash",
            "isError": true,
            "result": {"output": "SECRET-STDOUT"}
        }
    });

    let event = project("pi", &record);

    assert_eq!(event["type"], "tool_result");
    assert_eq!(event["tool_type"], "shell");
    assert_eq!(event["error_type"], "tool_error");
    assert_only_allowed_keys(&event);
    assert_absent(&event, "SECRET-STDOUT");
    assert_absent(&event, "tool-secret");
}

#[test]
fn kimi_code_projects_wire_tool_call_and_usage_record() {
    let tool_call = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "session/update",
        "params": {
            "sessionId": "session-secret",
            "update": {
                "sessionUpdate": "tool_call",
                "toolCallId": "tool-secret",
                "title": "Bash",
                "kind": "execute",
                "status": "pending",
                "rawInput": "{\"command\":\"SECRET-COMMAND\"}"
            }
        }
    });
    let event = project("kimi_code", &tool_call);
    assert_eq!(event["type"], "tool_call");
    assert_eq!(event["tool_type"], "shell");
    assert_only_allowed_keys(&event);
    for forbidden in ["SECRET-COMMAND", "session-secret", "tool-secret"] {
        assert_absent(&event, forbidden);
    }

    let usage = serde_json::json!({
        "type": "usage",
        "record": {"model": "kimi-k2-thinking", "total_tokens": 12}
    });
    let event = project("kimi_code", &usage);
    assert_eq!(event["type"], "usage");
    assert_eq!(event["model"], "kimi-k2-thinking");
    assert_only_allowed_keys(&event);
}

#[test]
fn kimi_code_projects_tool_call_update_failure() {
    let record = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "session/update",
        "params": {
            "update": {
                "sessionUpdate": "tool_call_update",
                "toolCallId": "tool-secret",
                "status": "failed",
                "content": [{"type": "content", "content": {"type": "text", "text": "SECRET-STDOUT"}}]
            }
        }
    });

    let event = project("kimi_code", &record);

    assert_eq!(event["type"], "tool_result");
    assert_eq!(event["error_type"], "tool_error");
    assert_only_allowed_keys(&event);
    assert_absent(&event, "SECRET-STDOUT");
}

#[test]
fn disguised_secret_values_never_leave_the_projection() {
    let record = serde_json::json!({
        "type": "response_item",
        "payload": {
            "type": "function_call",
            "name": "sk-secret-token",
            "model": "sk-secret-token",
            "arguments": "{\"command\":\"sk-secret-token\"}",
            "error": {"type": "sk-secret-token", "message": "sk-secret-token"}
        }
    });

    let event = project("codex", &record);

    assert_eq!(event["type"], "tool_call");
    assert_eq!(event["model"], "other");
    assert_eq!(event["tool_type"], "other");
    assert_eq!(event["error_type"], "other");
    assert_only_allowed_keys(&event);
    assert_absent(&event, "sk-secret-token");
}

#[test]
fn unknown_records_still_yield_event_facts() {
    let record = serde_json::json!({"totally": "unknown", "shape": {"nested": true}});

    let event = project("claude_code", &record);

    assert_eq!(event["type"], "other");
    assert_only_allowed_keys(&event);

    let event = project("codex", &record);
    assert_eq!(event["type"], "other");
    assert_only_allowed_keys(&event);
}

#[test]
fn unknown_provider_projects_nothing_but_whitelisted_keys() {
    let record = serde_json::json!({
        "type": "assistant",
        "timestamp": "2026-05-06T07:08:09.123Z",
        "message": {"role": "assistant", "content": [{"type": "text", "text": "SECRET-BODY"}]}
    });

    let event = project("mystery_provider", &record);

    assert_eq!(event["type"], "other");
    assert_only_allowed_keys(&event);
    assert_absent(&event, "SECRET-BODY");
}
