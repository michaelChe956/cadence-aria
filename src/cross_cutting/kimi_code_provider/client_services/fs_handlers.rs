//! `fs/read_text_file` / `fs/write_text_file` request handlers: JSON-RPC
//! param extraction, session check, and policy gate, delegating the
//! root-anchored file IO to `fs_service`.

use serde_json::Value;

use super::fs_service::{read_text_file, write_text_file};
use super::policy::ClientAction;
use super::{ClientServiceError, ClientServiceState, check_session, evaluate_policy};

pub(super) async fn handle_fs_read(
    state: &ClientServiceState,
    params: &Value,
) -> Result<String, ClientServiceError> {
    check_session(state, params)?;
    let path = params
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| ClientServiceError::Rejected("fs path is required".to_string()))?;
    evaluate_policy(state, ClientAction::FsRead, path).await?;
    read_text_file(&state.root, path).map_err(ClientServiceError::Fs)
}

pub(super) async fn handle_fs_write(
    state: &ClientServiceState,
    params: &Value,
) -> Result<(), ClientServiceError> {
    check_session(state, params)?;
    let path = params
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| ClientServiceError::Rejected("fs path is required".to_string()))?;
    let content = params
        .get("content")
        .and_then(Value::as_str)
        .ok_or_else(|| ClientServiceError::Rejected("fs content is required".to_string()))?;
    evaluate_policy(state, ClientAction::FsWrite, path).await?;
    write_text_file(&state.root, path, content).map_err(ClientServiceError::Fs)
}
