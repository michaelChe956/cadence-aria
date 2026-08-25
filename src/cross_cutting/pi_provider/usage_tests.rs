// 迁移自 tests.rs（large_file_guard 1200 行红线拆分）：本地 usage / 会话文件相关测试。
use std::path::PathBuf;

use super::parse::parse_pi_usage;
use super::parse_pi_session_file;

#[test]
fn parse_session_file_from_get_state_response_requires_absolute_path() {
    let absolute = serde_json::json!({
        "type": "response",
        "command": "get_state",
        "success": true,
        "data": { "sessionFile": "/home/user/.pi/agent/sessions/session.jsonl" }
    });
    assert_eq!(
        parse_pi_session_file(&absolute),
        Some(PathBuf::from("/home/user/.pi/agent/sessions/session.jsonl"))
    );

    let relative = serde_json::json!({
        "type": "response",
        "command": "get_state",
        "success": true,
        "data": { "sessionFile": "sessions/session.jsonl" }
    });
    assert!(parse_pi_session_file(&relative).is_none());
}

#[test]
fn parse_pi_usage_extracts_cost_snapshot_from_get_state_response() {
    let response = serde_json::json!({
        "type": "response",
        "command": "get_state",
        "success": true,
        "data": {
            "sessionId": "pi_session_1",
            "cost": {
                "input": 100.5,
                "output": 42,
                "cacheRead": 7,
                "cacheWrite": 9
            }
        }
    });
    let report = parse_pi_usage(&response, "author").expect("usage should parse");
    assert_eq!(report.role, "author");
    assert_eq!(report.input_tokens, Some(100));
    assert_eq!(report.output_tokens, Some(42));
    assert_eq!(report.cache_read_tokens, Some(7));
    assert_eq!(report.cache_creation_tokens, Some(9));
}

#[test]
fn parse_pi_usage_returns_none_without_cost() {
    let response =
        serde_json::json!({ "type": "response", "command": "get_state", "success": true });
    assert!(parse_pi_usage(&response, "reviewer").is_none());
}
