use super::{CodingWorkspaceEngineError, ProviderStreamOutcome};
use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::protocol::provider_errors::ProviderErrorCode;

/// Provider 调用中允许交给外层协调器自动恢复的技术失败。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RetryableProviderFailure {
    StartIo,
    StreamEnded,
    ConnectionInterrupted,
    ExecutionTimeout,
    Upstream5xx { status: u16 },
}

impl RetryableProviderFailure {
    pub(crate) fn reason_code(&self) -> &'static str {
        match self {
            Self::StartIo => "provider_start_io",
            Self::StreamEnded => "provider_stream_ended",
            Self::ConnectionInterrupted => "provider_connection_interrupted",
            Self::ExecutionTimeout => "provider_execution_timeout",
            Self::Upstream5xx { .. } => "provider_upstream_5xx",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProviderFailureClassification {
    Retryable {
        failure: RetryableProviderFailure,
        reason_code: String,
        message: String,
    },
    NonRetryable {
        reason_code: String,
        interaction_wait: bool,
    },
}

impl ProviderFailureClassification {
    #[allow(dead_code)]
    pub(crate) fn is_retryable(&self) -> bool {
        matches!(self, Self::Retryable { .. })
    }

    #[allow(dead_code)]
    pub(crate) fn is_interaction_wait(&self) -> bool {
        matches!(
            self,
            Self::NonRetryable {
                interaction_wait: true,
                ..
            }
        )
    }
}

/// 一次 Provider 调用的边界结果。协调器只应依据此结果决定是否创建下一次 role run。
#[allow(dead_code)]
#[derive(Debug)]
pub(crate) enum ProviderInvocationOutcome {
    Completed(ProviderStreamOutcome),
    Cancelled,
    NonRetryable {
        reason_code: String,
        error: CodingWorkspaceEngineError,
        interaction_wait: bool,
    },
    RetryableTransport {
        failure: RetryableProviderFailure,
        reason_code: String,
        message: String,
        partial_output: String,
    },
}

impl ProviderInvocationOutcome {
    #[allow(dead_code)]
    pub(crate) fn from_result(
        result: Result<ProviderStreamOutcome, CodingWorkspaceEngineError>,
        partial_output: String,
    ) -> Self {
        match result {
            Ok(outcome) => Self::Completed(outcome),
            Err(CodingWorkspaceEngineError::Aborted) => Self::Cancelled,
            Err(error) => match classify_provider_failure(&error) {
                ProviderFailureClassification::Retryable {
                    failure,
                    reason_code,
                    message,
                } => Self::RetryableTransport {
                    failure,
                    reason_code,
                    message,
                    partial_output,
                },
                ProviderFailureClassification::NonRetryable {
                    reason_code,
                    interaction_wait,
                } => Self::NonRetryable {
                    reason_code,
                    error,
                    interaction_wait,
                },
            },
        }
    }
}

pub(crate) fn classify_provider_failure(
    error: &CodingWorkspaceEngineError,
) -> ProviderFailureClassification {
    match error {
        CodingWorkspaceEngineError::ProviderAdapter(error) => classify_adapter_error(error),
        CodingWorkspaceEngineError::ProviderStream(message) => classify_stream_message(message),
        CodingWorkspaceEngineError::ProviderProtocol(_) => {
            non_retryable("provider_protocol", false)
        }
        CodingWorkspaceEngineError::Aborted => non_retryable("abort_attempt", false),
        _ => non_retryable("provider_business_failure", false),
    }
}

fn classify_adapter_error(error: &ProviderAdapterError) -> ProviderFailureClassification {
    let message = adapter_error_message(error);
    if is_permission_wait(&message) {
        return non_retryable("permission_timeout", true);
    }
    if is_choice_wait(&message) {
        return non_retryable("choice_timeout", true);
    }
    if let Some(classification) = classify_transport_message(&message) {
        return classification;
    }
    match error.code {
        ProviderErrorCode::ProviderTimeout => {
            retryable(RetryableProviderFailure::ExecutionTimeout, message)
        }
        ProviderErrorCode::ProviderExecutionFailed => {
            non_retryable("provider_execution_failed", false)
        }
        ProviderErrorCode::ProviderParseError => non_retryable("provider_parse_error", false),
        ProviderErrorCode::ProviderCommandMissing => {
            non_retryable("provider_command_missing", false)
        }
        ProviderErrorCode::ProviderUnavailable => non_retryable("provider_unavailable", false),
        ProviderErrorCode::ProviderUnauthorized => non_retryable("provider_unauthorized", false),
        ProviderErrorCode::ProviderPermissionDenied => {
            non_retryable("provider_permission_denied", false)
        }
        ProviderErrorCode::ProviderIncompatibleOutput => {
            non_retryable("provider_incompatible_output", false)
        }
    }
}

fn adapter_error_message(error: &ProviderAdapterError) -> String {
    let mut parts = vec![format!("{:?}: {}", error.code, error.details)];
    if !error.stdout.is_empty() {
        parts.push(format!("stdout: {}", error.stdout));
    }
    if !error.stderr.is_empty() {
        parts.push(format!("stderr: {}", error.stderr));
    }
    parts.join("; ")
}

fn classify_stream_message(message: &str) -> ProviderFailureClassification {
    if message.eq_ignore_ascii_case("provider_choice_unresolved") {
        return non_retryable("provider_choice_unresolved", true);
    }
    if is_permission_wait(message) {
        return non_retryable("permission_timeout", true);
    }
    if is_choice_wait(message) {
        return non_retryable("choice_timeout", true);
    }
    if message.to_ascii_lowercase().contains("protocol") {
        return non_retryable("provider_protocol", false);
    }
    if message.to_ascii_lowercase().contains("structured output")
        || message.to_ascii_lowercase().contains("parser")
    {
        return non_retryable("provider_structured_output", false);
    }
    classify_transport_message(message)
        .unwrap_or_else(|| non_retryable("provider_stream_failed", false))
}

fn classify_transport_message(message: &str) -> Option<ProviderFailureClassification> {
    let lower = message.to_ascii_lowercase();
    if let Some(status) = upstream_status(&lower) {
        return Some(retryable(
            RetryableProviderFailure::Upstream5xx { status },
            message.to_string(),
        ));
    }
    if lower.contains("stream ended")
        || lower.contains("stream closed")
        || lower.contains("eof")
        || lower.contains("closed before completion")
    {
        return Some(retryable(
            RetryableProviderFailure::StreamEnded,
            message.to_string(),
        ));
    }
    if lower.contains("connection")
        || lower.contains("broken pipe")
        || lower.contains("process interrupted")
        || lower.contains("process exited")
        || lower.contains("process terminated")
        || crate::cross_cutting::codex_provider::is_resume_stall_failure(message)
        || lower.contains("resume stalled")
    {
        return Some(retryable(
            RetryableProviderFailure::ConnectionInterrupted,
            message.to_string(),
        ));
    }
    if lower.contains("timeout") || lower.contains("timed out") {
        return Some(retryable(
            RetryableProviderFailure::ExecutionTimeout,
            message.to_string(),
        ));
    }
    if is_start_io_message(&lower) {
        return Some(retryable(
            RetryableProviderFailure::StartIo,
            message.to_string(),
        ));
    }
    None
}

fn upstream_status(message: &str) -> Option<u16> {
    let tokens = message
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();

    tokens.iter().enumerate().find_map(|(index, token)| {
        let status = token.parse::<u16>().ok()?;
        if !matches!(status, 503 | 504) {
            return None;
        }
        let context = &tokens[index.saturating_sub(5)..index];
        let nearest_identifier = context
            .iter()
            .enumerate()
            .rev()
            .find_map(|(offset, token)| {
                matches!(*token, "id" | "port" | "count").then_some(offset)
            });
        let nearest_upstream_context = context
            .iter()
            .enumerate()
            .rev()
            .find_map(|(offset, token)| is_upstream_status_context(token).then_some(offset));
        let identifier_is_closer = nearest_identifier
            .zip(nearest_upstream_context)
            .is_some_and(|(identifier, upstream)| identifier > upstream);
        (!identifier_is_closer && nearest_upstream_context.is_some()).then_some(status)
    })
}

fn is_upstream_status_context(token: &str) -> bool {
    matches!(token, "http" | "status" | "upstream" | "gateway")
}

fn is_start_io_message(message: &str) -> bool {
    message.contains("text file busy")
        || message.contains("resource temporarily unavailable")
        || ((message.contains("start") || message.contains("spawn"))
            && (message.contains("i/o")
                || message.contains("io error")
                || message.contains("input/output")))
}

fn is_permission_wait(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("permission") && (lower.contains("timeout") || lower.contains("timed out"))
}

fn is_choice_wait(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("choice") && (lower.contains("timeout") || lower.contains("timed out"))
}

fn retryable(failure: RetryableProviderFailure, message: String) -> ProviderFailureClassification {
    ProviderFailureClassification::Retryable {
        reason_code: failure.reason_code().to_string(),
        failure,
        message,
    }
}

fn non_retryable(reason_code: &str, interaction_wait: bool) -> ProviderFailureClassification {
    ProviderFailureClassification::NonRetryable {
        reason_code: reason_code.to_string(),
        interaction_wait,
    }
}
