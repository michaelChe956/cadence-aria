//! `fs/read_text_file` / `fs/write_text_file` request handlers: JSON-RPC
//! param extraction, session check, and policy gate, delegating the
//! root-anchored file IO to `fs_service`.

use serde_json::Value;

use super::fs_service::{
    baseline_tree_relative_path, read_baseline_text_file, read_text_file, write_text_file,
};
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
    // REQ-PIB-02 通道层路由：基线会话的 fs 读改走基线树（git show
    // refs/heads/<base>:<path>，不 checkout 不触工作区）——工作区检出内容
    //（含未提交污染与 `.worktrees/` 兄弟件）对基线会话不可见（F-57 根除）。
    if let Some(baseline) = state.baseline_tree.as_ref() {
        // F-58：kimi 原生 Read 委托 host 时总发送按其 cwd 解析后的绝对路径
        // ——先以会话根锚定归一为树内相对路径再走基线树（根外绝对路径与
        // `..` 在归一层拒绝，fail-closed 不变）。
        let tree_path =
            baseline_tree_relative_path(&state.root, path).map_err(ClientServiceError::Fs)?;
        return read_baseline_text_file(
            &baseline.repo_path,
            &baseline.branch,
            &tree_path.to_string_lossy(),
        )
        .map_err(ClientServiceError::Fs);
    }
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
