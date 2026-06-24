use std::process::ExitStatus;

use serde_json::json;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::cross_cutting::streaming_provider::ProviderEvent;

pub(crate) async fn send_provider_event(
    event_tx: &mpsc::Sender<ProviderEvent>,
    event: ProviderEvent,
    cancel: &CancellationToken,
) -> Result<(), ProviderAdapterError> {
    tokio::select! {
        _ = cancel.cancelled() => Err(provider_error("Codex provider cancelled")),
        result = event_tx.send(event) => result.map_err(|_| {
            provider_error("provider event receiver closed")
        }),
    }
}

pub(crate) async fn emit_request_user_input_protocol_error(
    event_tx: &mpsc::Sender<ProviderEvent>,
    source: &str,
    question_id: &str,
    details: &str,
) {
    let message = format!("requestUserInput {source} unresolved: {details}");
    // 直接使用 event_tx.send，因为失败原因可能是 cancel；send_provider_event 会在 cancel 时丢弃事件。
    let _ = event_tx
        .send(ProviderEvent::ProtocolError {
            code: "request_user_input_unresolved".to_string(),
            message,
            context: Some(json!({ "question_id": question_id })),
        })
        .await;
}

pub(crate) fn provider_error(message: impl Into<String>) -> ProviderAdapterError {
    ProviderAdapterError::parse_error(message, String::new(), String::new())
}

pub(crate) fn combine_stderr(process_stderr: String, error_stderr: String) -> String {
    match (process_stderr.trim(), error_stderr.trim()) {
        ("", "") => String::new(),
        (process, "") => process.to_string(),
        ("", write_error) => write_error.to_string(),
        (process, write_error) => format!("{process}\n{write_error}"),
    }
}

pub(crate) fn format_codex_failure(
    details: String,
    status: Result<ExitStatus, std::io::Error>,
    stderr: String,
) -> String {
    let status_text = match status {
        Ok(status) => format!("exit status: {status}"),
        Err(error) => format!("failed to wait for process: {error}"),
    };
    if stderr.trim().is_empty() {
        format!("{details} ({status_text})")
    } else {
        format!("{details} ({status_text}); stderr: {}", stderr.trim())
    }
}
