use crate::protocol::contracts::{AdapterOutput, TimeoutStatus};
use crate::protocol::provider_errors::ProviderErrorCode;
use serde_json::Value;

pub const STRUCTURED_OUTPUT_START: &str = "<ARIA_STRUCTURED_OUTPUT>";
pub const STRUCTURED_OUTPUT_END: &str = "</ARIA_STRUCTURED_OUTPUT>";

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
        let structured_output = parse_last_structured_output(&input.prompt)?;
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
    let Some(start_index) = stdout.rfind(STRUCTURED_OUTPUT_START) else {
        return Ok(None);
    };
    let json_start = start_index + STRUCTURED_OUTPUT_START.len();
    let after_start = &stdout[json_start..];
    let Some(end_index) = after_start.find(STRUCTURED_OUTPUT_END) else {
        return Err(ProviderAdapterError::parse_error(
            "missing structured output end sentinel",
            stdout.to_string(),
            String::new(),
        ));
    };
    let json_text = after_start[..end_index].trim();
    serde_json::from_str(json_text).map(Some).map_err(|error| {
        ProviderAdapterError::parse_error(
            format!("invalid structured output json: {error}"),
            stdout.to_string(),
            String::new(),
        )
    })
}
