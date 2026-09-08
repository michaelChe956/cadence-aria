use crate::cross_cutting::structured_output::parse_last_structured_output_value;
use crate::protocol::contracts::{AdapterOutput, AdapterRole, TimeoutStatus};
use crate::protocol::provider_errors::ProviderErrorCode;
use serde_json::{Value, json};

/// Prefix used only by stream-display filters. Protocol producers must use
/// [`structured_output_sentinel`] so every block includes the JSON nonce envelope.
pub const STRUCTURED_OUTPUT_START_PREFIX: &str = "<ARIA_STRUCTURED_OUTPUT";
pub const STRUCTURED_OUTPUT_END: &str = "</ARIA_STRUCTURED_OUTPUT>";
pub const DEFAULT_PROVIDER_TIMEOUT_SECS: u64 = 3 * 60 * 60;

/// 诊断直通（claude×轻 握手谜团第 2 轮）：错误 details 追加 stderr 尾部时的
/// 默认有界上限（字节；UTF-8 字符边界安全，丢头保尾——致命错误通常在 stderr
/// 末尾）。供 workspace 层错误包装（gateway 映射 / provider 驱动入口）把
/// adapter stderr（含 claude D③ 快照）带到驱动 result 与 WS error 消息。
pub const PROVIDER_ERROR_STDERR_TAIL_BYTES: usize = 500;

pub trait ProviderAdapter {
    fn run(
        &self,
        input: &crate::protocol::contracts::AdapterInput,
    ) -> Result<AdapterOutput, ProviderAdapterError>;
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{code:?}: {details}")]
pub struct ProviderAdapterError {
    pub code: ProviderErrorCode,
    pub details: String,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
    pub timeout_status: TimeoutStatus,
    pub duration_ms: u64,
}

impl ProviderAdapterError {
    pub fn command_missing(details: impl Into<String>) -> Self {
        Self::new(ProviderErrorCode::ProviderCommandMissing, details)
    }

    pub fn provider_unavailable(details: impl Into<String>) -> Self {
        Self::new(ProviderErrorCode::ProviderUnavailable, details)
    }

    pub fn unauthorized(
        details: impl Into<String>,
        stdout: impl Into<String>,
        stderr: impl Into<String>,
    ) -> Self {
        Self::with_output(
            ProviderErrorCode::ProviderUnauthorized,
            details,
            stdout,
            stderr,
            None,
            TimeoutStatus::NotTimedOut,
            0,
        )
    }

    pub fn permission_denied(
        details: impl Into<String>,
        stdout: impl Into<String>,
        stderr: impl Into<String>,
    ) -> Self {
        Self::with_output(
            ProviderErrorCode::ProviderPermissionDenied,
            details,
            stdout,
            stderr,
            None,
            TimeoutStatus::NotTimedOut,
            0,
        )
    }

    pub fn incompatible_output(
        details: impl Into<String>,
        stdout: impl Into<String>,
        stderr: impl Into<String>,
    ) -> Self {
        Self::with_output(
            ProviderErrorCode::ProviderIncompatibleOutput,
            details,
            stdout,
            stderr,
            Some(0),
            TimeoutStatus::NotTimedOut,
            0,
        )
    }

    pub fn timeout(stdout: impl Into<String>, stderr: impl Into<String>, duration_ms: u64) -> Self {
        Self::with_output(
            ProviderErrorCode::ProviderTimeout,
            "provider command timed out",
            stdout,
            stderr,
            None,
            TimeoutStatus::HardTimeoutKilled,
            duration_ms,
        )
    }

    pub fn timeout_with_details(
        details: impl Into<String>,
        stdout: impl Into<String>,
        stderr: impl Into<String>,
        duration_ms: u64,
    ) -> Self {
        Self::with_output(
            ProviderErrorCode::ProviderTimeout,
            details,
            stdout,
            stderr,
            None,
            TimeoutStatus::HardTimeoutKilled,
            duration_ms,
        )
    }

    pub fn parse_error(
        details: impl Into<String>,
        stdout: impl Into<String>,
        stderr: impl Into<String>,
    ) -> Self {
        Self::with_output(
            ProviderErrorCode::ProviderParseError,
            details,
            stdout,
            stderr,
            Some(0),
            TimeoutStatus::NotTimedOut,
            0,
        )
    }

    pub fn execution_failed(
        exit_code: Option<i32>,
        stdout: impl Into<String>,
        stderr: impl Into<String>,
        duration_ms: u64,
    ) -> Self {
        Self::with_output(
            ProviderErrorCode::ProviderExecutionFailed,
            "provider command exited unsuccessfully",
            stdout,
            stderr,
            exit_code,
            TimeoutStatus::NotTimedOut,
            duration_ms,
        )
    }

    /// turn 完成但去空白后输出为空（含一次有界重试后再空）。
    pub fn provider_empty_output(details: impl Into<String>) -> Self {
        Self::new(ProviderErrorCode::ProviderEmptyOutput, details)
    }

    /// 诊断直通（claude×轻 握手谜团第 2 轮）：把 stderr 的有界尾部（丢头保尾，
    /// UTF-8 字符边界安全）追加到错误 details 文本。workspace 层错误包装原本
    /// 只保留 details 文本，stderr 字段在包装点被丢弃；claude 的 D③ 快照已把
    /// stderr 并入 details 时不重复追加（内容已在则跳过）。
    pub fn append_bounded_stderr_tail(details: &mut String, stderr: &str, tail_limit_bytes: usize) {
        let stderr = stderr.trim();
        if stderr.is_empty() {
            return;
        }
        let tail = bounded_char_tail(stderr, tail_limit_bytes);
        if details.contains(tail.as_str()) {
            return;
        }
        details.push_str(&format!(
            "\nprovider stderr (last {} bytes): {tail}",
            tail.len()
        ));
    }

    fn new(code: ProviderErrorCode, details: impl Into<String>) -> Self {
        Self::with_output(
            code,
            details,
            String::new(),
            String::new(),
            None,
            TimeoutStatus::NotTimedOut,
            0,
        )
    }

    fn with_output(
        code: ProviderErrorCode,
        details: impl Into<String>,
        stdout: impl Into<String>,
        stderr: impl Into<String>,
        exit_code: Option<i32>,
        timeout_status: TimeoutStatus,
        duration_ms: u64,
    ) -> Self {
        Self {
            code,
            details: details.into(),
            stdout: stdout.into(),
            stderr: stderr.into(),
            exit_code,
            timeout_status,
            duration_ms,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct FakeProviderAdapter;

impl ProviderAdapter for FakeProviderAdapter {
    fn run(
        &self,
        input: &crate::protocol::contracts::AdapterInput,
    ) -> Result<AdapterOutput, ProviderAdapterError> {
        let structured_output = match parse_last_structured_output(&input.prompt) {
            Ok(output) => output,
            Err(error) => {
                if input.role == AdapterRole::WorkItemSplitter {
                    None
                } else {
                    return Err(error);
                }
            }
        };
        let structured_output =
            structured_output.or_else(|| default_structured_output_for_role(&input.role));
        Ok(AdapterOutput {
            exit_code: Some(0),
            stdout: input.prompt.clone(),
            stderr: String::new(),
            structured_output,
            files_modified: Vec::new(),
            duration_ms: 0,
            timeout_status: TimeoutStatus::NotTimedOut,
        })
    }
}

pub fn parse_last_structured_output(stdout: &str) -> Result<Option<Value>, ProviderAdapterError> {
    parse_last_structured_output_value(stdout).map_err(|error| {
        ProviderAdapterError::parse_error(error.message, stdout.to_string(), String::new())
    })
}

/// Renders an unambiguous structured-output block for adapters and test providers.
///
/// The parser removes this transport-only envelope before returning business JSON.
/// A producer-supplied field named `nonce` is overwritten deliberately so it cannot
/// disagree with the start tag.
pub fn structured_output_sentinel(nonce: &str, payload: &Value) -> String {
    let mut payload = payload.clone();
    payload
        .as_object_mut()
        .expect("structured output payload must be a JSON object")
        .insert("nonce".to_string(), Value::String(nonce.to_string()));
    format!("<ARIA_STRUCTURED_OUTPUT nonce=\"{nonce}\">{payload}</ARIA_STRUCTURED_OUTPUT>")
}

fn default_structured_output_for_role(role: &AdapterRole) -> Option<Value> {
    match role {
        AdapterRole::Handoff => {
            return Some(json!({
                "summary": "Completed work item handoff",
                "files_changed": [],
                "diff_summary": "",
                "tests_run": [],
                "test_result_summary": "passed",
                "api_or_contract_changes": [],
                "next_work_item_notes": []
            }));
        }
        AdapterRole::WorkItemSplitter => {}
        _ => return None,
    }
    Some(json!({
        "repository_profile": {
            "confidence": "high",
            "detected_layers": ["backend"],
            "split_recommendation": "single_work_item",
            "languages": ["rust"],
            "frameworks": [],
            "package_managers": [],
            "test_frameworks": [],
            "build_systems": [],
            "verification_capabilities": [],
            "uncertainties": []
        },
        "work_items": [
            {
                "title": "Implement work item",
                "kind": "backend",
                "sequence_hint": 10,
                "depends_on": [],
                "exclusive_write_scopes": ["src/"],
                "forbidden_write_scopes": [],
                "require_execution_plan_confirm": false
            }
        ],
        "verification_plans": [
            {
                "scope": "unit",
                "commands": [
                    {
                        "id": "cmd_001",
                        "label": "Run tests",
                        "command": "cargo test",
                        "cwd": "",
                        "purpose": "Run unit tests",
                        "required": true,
                        "timeout_seconds": 300,
                        "safety": "approved"
                    }
                ],
                "manual_checks": [],
                "required_gates": [],
                "risk_notes": [],
                "confidence": "high",
                "fallback_policy": "manual_gate"
            }
        ]
    }))
}

/// 字节上限内保留尾部（丢头保尾），起始切割点推进到 UTF-8 字符边界。
fn bounded_char_tail(text: &str, limit_bytes: usize) -> String {
    if text.len() <= limit_bytes {
        return text.to_string();
    }
    let mut start = text.len() - limit_bytes;
    while start < text.len() && !text.is_char_boundary(start) {
        start += 1;
    }
    text[start..].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_bounded_stderr_tail_keeps_tail_and_drops_head() {
        let stderr = format!("{}SENTINEL_TAIL", "x".repeat(700));
        let mut details = "provider start failed".to_string();
        ProviderAdapterError::append_bounded_stderr_tail(
            &mut details,
            &stderr,
            PROVIDER_ERROR_STDERR_TAIL_BYTES,
        );
        assert!(details.contains("SENTINEL_TAIL"));
        assert!(details.contains("provider stderr (last "));
        assert!(!details.contains(&"x".repeat(600)));
    }

    #[test]
    fn append_bounded_stderr_tail_skips_empty_and_duplicate() {
        let mut details = String::new();
        ProviderAdapterError::append_bounded_stderr_tail(&mut details, "   \n\t ", 500);
        assert!(details.is_empty());

        let stderr = "fatal: mcp connection refused";
        let mut details = format!("boom\nprovider stderr (last 28 bytes): {stderr}");
        ProviderAdapterError::append_bounded_stderr_tail(&mut details, stderr, 500);
        assert_eq!(
            details.matches("provider stderr").count(),
            1,
            "已包含同一 stderr 尾部时不得重复追加: {details}"
        );
    }

    #[test]
    fn append_bounded_stderr_tail_is_char_boundary_safe() {
        // 多字节中文字符，切割点落在字符中间时必须推进到边界。
        let stderr = "错".repeat(400) + "尾部哨兵";
        let mut details = "failed".to_string();
        ProviderAdapterError::append_bounded_stderr_tail(&mut details, &stderr, 500);
        assert!(details.contains("尾部哨兵"));
        let appended = details.rsplit("bytes): ").next().unwrap();
        assert!(std::str::from_utf8(appended.as_bytes()).is_ok());
    }

    #[test]
    fn bounded_char_tail_short_input_passthrough() {
        assert_eq!(bounded_char_tail("short", 500), "short");
    }
}
