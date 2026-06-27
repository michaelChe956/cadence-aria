use crate::protocol::contracts::{AdapterOutput, AdapterRole, TimeoutStatus};
use crate::protocol::provider_errors::ProviderErrorCode;
use serde_json::{Value, json};

pub const STRUCTURED_OUTPUT_START: &str = "<ARIA_STRUCTURED_OUTPUT>";
pub const STRUCTURED_OUTPUT_END: &str = "</ARIA_STRUCTURED_OUTPUT>";
pub const DEFAULT_PROVIDER_TIMEOUT_SECS: u64 = 3 * 60 * 60;
const STRUCTURED_OUTPUT_START_PREFIX: &str = "<ARIA_STRUCTURED_OUTPUT";
const STRUCTURED_OUTPUT_END_PREFIX: &str = "</ARIA_STRUCTURED_OUTPUT";

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
    let Some(start_index) = stdout.rfind(STRUCTURED_OUTPUT_START_PREFIX) else {
        return Ok(None);
    };
    let after_start_prefix = &stdout[start_index + STRUCTURED_OUTPUT_START_PREFIX.len()..];
    let (nonce, start_tag_len) =
        parse_structured_output_tag(after_start_prefix, "structured output start")?;
    let json_start = start_index + STRUCTURED_OUTPUT_START_PREFIX.len() + start_tag_len;
    let after_start = &stdout[json_start..];
    let Some((end_index, _end_tag_len)) =
        find_structured_output_end(after_start, nonce.as_deref())?
    else {
        return Err(ProviderAdapterError::parse_error(
            "missing structured output end sentinel",
            stdout.to_string(),
            String::new(),
        ));
    };
    let json_text = after_start[..end_index].trim();
    parse_structured_json_text(json_text)
        .or_else(|_| {
            extract_json_candidate(json_text)
                .ok_or_else(|| {
                    ProviderAdapterError::parse_error(
                        "invalid structured output json: no JSON object or array found",
                        stdout.to_string(),
                        String::new(),
                    )
                })
                .and_then(parse_structured_json_text)
        })
        .map(Some)
        .map_err(|mut error| {
            error.stdout = stdout.to_string();
            error
        })
}

fn find_structured_output_end(
    after_start: &str,
    start_nonce: Option<&str>,
) -> Result<Option<(usize, usize)>, ProviderAdapterError> {
    let Some(end_index) = after_start.find(STRUCTURED_OUTPUT_END_PREFIX) else {
        return Ok(None);
    };
    let after_end_prefix = &after_start[end_index + STRUCTURED_OUTPUT_END_PREFIX.len()..];
    let (end_nonce, end_tag_len) =
        parse_structured_output_tag(after_end_prefix, "structured output end")?;
    if start_nonce != end_nonce.as_deref() {
        return Err(ProviderAdapterError::parse_error(
            "structured output nonce mismatch",
            String::new(),
            String::new(),
        ));
    }
    Ok(Some((
        end_index,
        STRUCTURED_OUTPUT_END_PREFIX.len() + end_tag_len,
    )))
}

fn parse_structured_output_tag(
    after_prefix: &str,
    tag_name: &str,
) -> Result<(Option<String>, usize), ProviderAdapterError> {
    let Some(end_offset) = after_prefix.find('>') else {
        return Err(ProviderAdapterError::parse_error(
            format!("missing {tag_name} tag close"),
            String::new(),
            String::new(),
        ));
    };
    let attrs = after_prefix[..end_offset].trim();
    let nonce = parse_structured_output_nonce(attrs).map_err(|details| {
        ProviderAdapterError::parse_error(
            format!("{tag_name} {details}"),
            String::new(),
            String::new(),
        )
    })?;
    Ok((nonce, end_offset + 1))
}

fn parse_structured_output_nonce(attrs: &str) -> Result<Option<String>, &'static str> {
    if attrs.is_empty() {
        return Ok(None);
    }
    let Some(nonce) = attrs
        .strip_prefix("nonce=\"")
        .and_then(|value| value.strip_suffix('"'))
    else {
        return Err("has unsupported attributes");
    };
    if nonce.len() != 8 || !nonce.chars().all(|ch| ch.is_ascii_alphanumeric()) {
        return Err("has invalid nonce");
    }
    Ok(Some(nonce.to_string()))
}

fn parse_structured_json_text(json_text: &str) -> Result<Value, ProviderAdapterError> {
    serde_json::from_str(json_text).map_err(|error| {
        ProviderAdapterError::parse_error(
            format!("invalid structured output json: {error}"),
            String::new(),
            String::new(),
        )
    })
}

fn extract_json_candidate(text: &str) -> Option<&str> {
    let start = text.find(['{', '['])?;
    let close = match text.as_bytes()[start] {
        b'{' => '}',
        b'[' => ']',
        _ => return None,
    };
    let end = text.rfind(close)?;
    (end >= start).then_some(&text[start..=end])
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
                "required_handoff_from": [],
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
