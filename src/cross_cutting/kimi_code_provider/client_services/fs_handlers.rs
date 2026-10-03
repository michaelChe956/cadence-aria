//! `fs/read_text_file` / `fs/write_text_file` request handlers: JSON-RPC
//! param extraction, session check, and policy gate, delegating the
//! root-anchored file IO to `fs_service`.

use serde_json::Value;

use crate::cross_cutting::provider_boundary::ProviderBoundaryMode;

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
    // Task 4c 读写面分离:LC 会话的读面 = boundary plan 冻结的 host root
    // (只读面;与 provider 进程 cwd 同源);direct 会话保持会话根。
    let read_root = state
        .target_boundary
        .as_ref()
        .map(|plan| plan.working_directory().to_path_buf())
        .unwrap_or_else(|| state.root.clone());
    // REQ-PIB-02 通道层路由：基线会话的 fs 读改走基线树（git show
    // refs/heads/<base>:<path>，不 checkout 不触工作区）——工作区检出内容
    //（含未提交污染与 `.worktrees/` 兄弟件）对基线会话不可见（F-57 根除）。
    if let Some(baseline) = state.baseline_tree.as_ref() {
        // F-58：kimi 原生 Read 委托 host 时总发送按其 cwd 解析后的绝对路径
        // ——先以会话根锚定归一为树内相对路径再走基线树（根外绝对路径与
        // `..` 在归一层拒绝，fail-closed 不变）。
        let tree_path =
            baseline_tree_relative_path(&read_root, path).map_err(ClientServiceError::Fs)?;
        return read_baseline_text_file(
            &baseline.repo_path,
            &baseline.branch,
            &tree_path.to_string_lossy(),
        )
        .map_err(ClientServiceError::Fs);
    }
    read_text_file(&read_root, path).map_err(ClientServiceError::Fs)
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
    // Task 4c 读写面分离:LC 会话的写面唯一锚定 boundary plan 的 target
    // root(不可伪造;root 与其它成员不可写);read-only action 无写面,
    // 整体拒绝(不依赖角色决策表兜底)。direct 会话保持会话根锚定,
    // 原 24 格决策表不变。
    match state.target_boundary.as_ref() {
        Some(plan) => match plan.mode() {
            ProviderBoundaryMode::TargetWriteOnly => {
                let target_root = plan.target_root().ok_or_else(|| {
                    ClientServiceError::Rejected(
                        "lc target boundary plan carries no writable target".to_string(),
                    )
                })?;
                write_text_file(target_root, path, content).map_err(ClientServiceError::Fs)
            }
            ProviderBoundaryMode::ReadOnly => Err(ClientServiceError::Rejected(
                "lc read-only action has no writable root (target boundary plan)".to_string(),
            )),
        },
        None => write_text_file(&state.root, path, content).map_err(ClientServiceError::Fs),
    }
}
