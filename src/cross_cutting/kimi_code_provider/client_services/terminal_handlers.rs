//! Terminal request handlers for the kimi client services: the five
//! `terminal/*` endpoints plus their command-form and cwd-anchoring helpers.
//!
//! 批2 Task 6:全角色 Host 单路径——普通 `command+args[]` 原样 argv,legacy
//! 单字符串用受信 SHELL 的 `-c`;不再有 isolation_for/bwrap/固定解释器/
//! 只读 grammar。cwd 仍走原 canonical 身份校验(no-follow openat 锚定),
//! 未声明/出界 cwd 回退面根只是起始目录选择,不是物理隔离。

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use serde_json::{Value, json};

use crate::cross_cutting::streaming_provider::{
    ProviderEvent, ProviderExecutionEvent, ProviderExecutionEventKind, ProviderExecutionEventStatus,
};
#[cfg(test)]
use crate::protocol::contracts::AdapterRole;

use super::policy::ClientAction;
use super::sandbox::{canonical_path_of_fd, open_dir_no_follow};
use super::terminal::{TerminalCommand, TerminalResult};
use super::{ClientServiceError, ClientServiceState, check_session, evaluate_policy};

use crate::cross_cutting::provider_boundary::ProviderBoundaryMode;
#[cfg(test)]
use crate::cross_cutting::provider_boundary::ProviderBoundaryPlan;

#[cfg(test)]
use super::policy::ClientServicePolicy;
#[cfg(test)]
use super::terminal::TerminalManager;

/// 受信 SHELL:legacy 单字符串形态的解释器。取冻结会话环境的 `SHELL`
/// (与 provider 进程同一环境合同;缺失时回退 POSIX `/bin/sh`)——不来自
/// ACP 参数,宿主也不施加固定解释器白名单。
fn trusted_shell(env: &BTreeMap<String, String>) -> String {
    env.get("SHELL")
        .filter(|shell| !shell.is_empty())
        .cloned()
        .unwrap_or_else(|| "/bin/sh".to_string())
}
/// Task 4c 读写面分离:terminal 的授权面根——LC 会话(TargetWriteOnly)为
/// boundary plan 的 target root(cwd 锚定以它为起始面;read-only plan/
/// direct 会话为会话根)。terminal 实际 cwd 只在该面内解析,不改变
/// provider 进程 cwd(进程 cwd 由 start_validated 冻结为 canonical LC
/// root)。批2 Task 6:face root 只决定 cwd 锚定面,不再是物理写边界。
fn terminal_face_root(state: &ClientServiceState) -> Result<PathBuf, ClientServiceError> {
    match state.target_boundary.as_ref() {
        Some(plan) => match plan.mode() {
            ProviderBoundaryMode::TargetWriteOnly => {
                plan.target_root().map(ToOwned::to_owned).ok_or_else(|| {
                    ClientServiceError::Rejected(
                        "lc target boundary plan carries no writable target".to_string(),
                    )
                })
            }
            ProviderBoundaryMode::ReadOnly => Ok(state.root.clone()),
        },
        None => Ok(state.root.clone()),
    }
}

/// Resolve a terminal cwd inside the authorized face root (or the root
/// itself), rejecting symlinks via `openat` + `O_NOFOLLOW` and returning the
/// canonical path of the anchored directory fd. Absolute paths are tolerated
/// when they lexically point beneath the face root (the fs_service anchoring
/// pattern), then re-anchored through the same no-follow walk.
///
/// kimi-9 现场修订(r10 2026-10-09):kimi CLI(2.0.2)不转发模型 Bash 工具
/// 的 `cwd` 参数——`terminal/create` 的 params.cwd 恒为会话启动 cwd(argv
/// 形态下模型 cwd 只进脚本的 `cd '<dir>' &&` 前缀)。LC executor 的会话
/// cwd=canonical 聚合根,是 face root(成员 worktree)的祖先,旧实现按
/// 「cwd 出界即拒」在执行前拒绝 executor 的每一次 terminal 调用(现场
/// 两 run 30/30 全拒,含模型 cwd 与守卫提示根 220 字节逐字节相同的调用;
/// coder 60 分钟 16 轮 plan-defect blocked 后泵超时,coding fresh 落
/// 「缺完成产物」)。修:CLI cwd 无法锚定进 face root(出界/父遍历/前缀
/// 混淆/no-follow 拒绝)时不再拒绝,锚定 face_root 本身——这是起始目录
/// 选择而非物理隔离(Host 单路径下脚本自带 `cd` 不受挂载面约束)。
fn resolve_cwd(
    state: &ClientServiceState,
    cwd: Option<&str>,
) -> Result<PathBuf, ClientServiceError> {
    let face_root = terminal_face_root(state)?;
    let Some(cwd) = cwd.filter(|cwd| !cwd.is_empty()) else {
        return Ok(face_root.to_path_buf());
    };
    Ok(anchor_cwd_within(&face_root, cwd).unwrap_or_else(|| face_root.to_path_buf()))
}

/// Try to anchor `cwd` inside `face_root`: absolute paths must strip the
/// root without `..` components, relative paths must be `..`-free; the
/// surviving remainder walks the no-follow `openat` chain and must
/// canonicalize back beneath the root. `None` = cannot be anchored (caller
/// falls back to the face root; the requested path is never echoed back).
fn anchor_cwd_within(face_root: &Path, cwd: &str) -> Option<PathBuf> {
    let rel: PathBuf = if Path::new(cwd).is_absolute() {
        let rel = Path::new(cwd).strip_prefix(face_root).ok()?;
        if rel
            .components()
            .any(|component| component == Component::ParentDir)
        {
            return None;
        }
        rel.to_path_buf()
    } else if cwd.split('/').any(|component| component == "..") {
        return None;
    } else {
        PathBuf::from(cwd)
    };
    let fd = open_dir_no_follow(face_root, &rel).ok()?;
    let canonical = canonical_path_of_fd(&fd).ok()?;
    canonical.starts_with(face_root).then_some(canonical)
}

pub(super) async fn handle_terminal_create(
    state: &ClientServiceState,
    params: &Value,
) -> Result<String, ClientServiceError> {
    check_session(state, params)?;
    // 批2 Task 7（Bug45）：会话终结（总 deadline 耗尽/用户取消，经
    // cleanup_cancel 送达）后到达的 terminal/create 在 spawn 前拒绝——
    // 超剩余总预算的启动是配置问题（启动数 0，不占并发名额）；在途
    // terminal 的回收由 dispatcher Drop→cleanup_all 沿既有 kill 链承担。
    if state.cleanup_cancel.is_cancelled() {
        return Err(ClientServiceError::Rejected(
            "terminal create rejected: the invocation has ended (total deadline exhausted or \
             cancelled); starting a command beyond the remaining invocation budget is a \
             configuration problem"
                .to_string(),
        ));
    }
    let command = params
        .get("command")
        .and_then(Value::as_str)
        .filter(|command| !command.trim().is_empty())
        .ok_or_else(|| ClientServiceError::Rejected("terminal command is required".to_string()))?;

    // 批2 Task 6 原生命令形态:
    // - argv 形态(带 `args[]`):`command` + `args[]` 原样 argv——解释器/
    //   工具与参数均由 ACP server 决定(如 `/bin/bash -c <script>`、
    //   `mvn --version`、`node -e ...`),宿主不再施加固定解释器白名单,
    //   也不改写/注入参数。
    // - legacy 形态(无 `args`):单字符串经受信 SHELL 的 `-c` 执行,管道/
    //   重定向等 shell 操作符与原参数语义保留。
    let (program, args, display) = if let Some(array) = params.get("args").and_then(Value::as_array)
    {
        let args: Vec<String> = array
            .iter()
            .map(|arg| arg.as_str().unwrap_or_default().to_string())
            .collect();
        let display = format!("{command} {}", args.join(" "));
        (command.to_string(), args, display)
    } else {
        let shell = trusted_shell(&state.session_env);
        (
            shell,
            vec!["-c".to_string(), command.to_string()],
            command.to_string(),
        )
    };

    evaluate_policy(state, ClientAction::Terminal, &display).await?;

    // Task 4c 读写面分离:cwd 锚定 boundary plan 的 face root(LC Coding=
    // target;terminal 实际 cwd 不改 provider 进程 cwd)。
    let cwd = resolve_cwd(state, params.get("cwd").and_then(Value::as_str))?;
    let terminal_command = TerminalCommand {
        program,
        args,
        cwd,
        env: state.session_env.clone(),
    };

    // 进程先登记后启动:`create` 占住并发名额并登记命令,`start` 才
    // spawn(spawn 失败由 manager 释放名额)。
    let terminal_id = state
        .terminal
        .create(terminal_command.clone())
        .await
        .map_err(ClientServiceError::Terminal)?;
    // 批2 Task 7 修复轮(F2)：create() 占名额与 dispatcher Drop→cleanup_all
    // 之间存在竞态窗口——Drop 只收割当时已登记的 terminal，过门后插入的
    // 登记若不复核会被 start() 照常 spawn，且 120s 独立上限已删，孤儿进程
    // 将无时间上限。create 后复核取消态：已终结则立刻 release 回收名额并
    // 拒绝（无逃逸登记、无逃逸 spawn）。
    if state.cleanup_cancel.is_cancelled() {
        let _ = state.terminal.release(&terminal_id).await;
        return Err(ClientServiceError::Rejected(
            "terminal create rejected: the invocation has ended before spawn (total deadline \
             exhausted or cancelled); starting a command beyond the remaining invocation budget \
             is a configuration problem"
                .to_string(),
        ));
    }

    emit_terminal_lifecycle_event(
        state,
        &terminal_id,
        ProviderExecutionEventStatus::Started,
        display,
        Some(format!(
            "{} {}",
            terminal_command.program,
            terminal_command.args.join(" ")
        )),
        Some(terminal_command.cwd.to_string_lossy().into_owned()),
        None,
        None,
    )
    .await;

    state
        .terminal
        .start(&terminal_id)
        .map_err(ClientServiceError::Terminal)?;

    Ok(terminal_id)
}

pub(super) async fn handle_terminal_wait(
    state: &ClientServiceState,
    params: &Value,
) -> Result<TerminalResult, ClientServiceError> {
    check_session(state, params)?;
    evaluate_policy(state, ClientAction::Terminal, "terminal wait").await?;
    let terminal_id = params
        .get("terminalId")
        .and_then(Value::as_str)
        .ok_or_else(|| ClientServiceError::Rejected("terminalId is required".to_string()))?;
    let result = state
        .terminal
        .wait_for_exit(terminal_id)
        .await
        .map_err(ClientServiceError::Terminal)?;
    emit_terminal_lifecycle_event(
        state,
        terminal_id,
        if result.timed_out || result.killed {
            ProviderExecutionEventStatus::Failed
        } else {
            ProviderExecutionEventStatus::Completed
        },
        format!("terminal {terminal_id}"),
        None,
        None,
        Some(
            json!({
                "terminalId": terminal_id,
                "timedOut": result.timed_out,
                "killed": result.killed,
                "truncated": result.truncated,
            })
            .to_string(),
        ),
        result.exit_code,
    )
    .await;
    Ok(result)
}

/// Serve kimi's `terminal/output` request: return the retained combined
/// output of a finished (or running) terminal. kimi 0.38.0 asks for output
/// as a request with an id after `terminal/wait_for_exit`; it does not
/// consume unsolicited `terminal/output` notifications.
pub(super) async fn handle_terminal_output(
    state: &ClientServiceState,
    params: &Value,
) -> Result<String, ClientServiceError> {
    check_session(state, params)?;
    let terminal_id = params
        .get("terminalId")
        .and_then(Value::as_str)
        .ok_or_else(|| ClientServiceError::Rejected("terminalId is required".to_string()))?;
    state
        .terminal
        .output(terminal_id)
        .map_err(ClientServiceError::Terminal)
}

pub(super) async fn handle_terminal_kill(
    state: &ClientServiceState,
    params: &Value,
) -> Result<(), ClientServiceError> {
    check_session(state, params)?;
    evaluate_policy(state, ClientAction::Terminal, "terminal kill").await?;
    let terminal_id = params
        .get("terminalId")
        .and_then(Value::as_str)
        .ok_or_else(|| ClientServiceError::Rejected("terminalId is required".to_string()))?;
    state
        .terminal
        .kill(terminal_id)
        .await
        .map_err(ClientServiceError::Terminal)
}

pub(super) async fn handle_terminal_release(
    state: &ClientServiceState,
    params: &Value,
) -> Result<(), ClientServiceError> {
    check_session(state, params)?;
    evaluate_policy(state, ClientAction::Terminal, "terminal release").await?;
    let terminal_id = params
        .get("terminalId")
        .and_then(Value::as_str)
        .ok_or_else(|| ClientServiceError::Rejected("terminalId is required".to_string()))?;
    state
        .terminal
        .release(terminal_id)
        .await
        .map_err(ClientServiceError::Terminal)
}

/// 批2 Task 7（Bug45）：terminal 生命周期 Execution 事件用稳定
/// event_id（`kimi_terminal_{terminal_id}`）——Started 与终态同 id，消费方
/// 据此配对在途状态；不再沿 Started/Completed 各造不同 event_id。发送
/// 改为 awaited send（可靠投递，通道关闭=会话已终结，忽略），不依赖
/// 可能 try_send 丢失的 UI 日志；产品看门狗的暂停判定另经
/// `ProviderEvent::TerminalActivity` 快照（同源 registry），本事件只是
/// 可观测性投影。
#[allow(clippy::too_many_arguments)]
async fn emit_terminal_lifecycle_event(
    state: &ClientServiceState,
    terminal_id: &str,
    status: ProviderExecutionEventStatus,
    title: String,
    command: Option<String>,
    cwd: Option<String>,
    output: Option<String>,
    exit_code: Option<i32>,
) {
    let _ = state
        .event_tx
        .send(ProviderEvent::Execution(ProviderExecutionEvent {
            event_id: format!("kimi_terminal_{terminal_id}"),
            kind: ProviderExecutionEventKind::Command,
            status,
            title,
            detail: None,
            command,
            cwd,
            output,
            exit_code,
        }))
        .await;
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    use crate::cross_cutting::approval_bridge::ApprovalBridge;
    use crate::cross_cutting::streaming_provider::{
        ProviderCommand, ProviderEvent, ProviderPermissionMode,
    };

    fn test_state(role: AdapterRole) -> (Arc<ClientServiceState>, mpsc::Receiver<ProviderEvent>) {
        let (event_tx, events) = mpsc::channel(512);
        let state = Arc::new(ClientServiceState {
            session_id: "client-service-test".to_string(),
            root: std::env::temp_dir(),
            policy: ClientServicePolicy::new(role, ProviderPermissionMode::Auto),
            bridge: Arc::new(ApprovalBridge::new(
                ProviderPermissionMode::Auto,
                event_tx.clone(),
            )),
            event_tx,
            terminal: TerminalManager::new(),
            session_env: BTreeMap::new(),
            cleanup_cancel: CancellationToken::new().child_token(),
            baseline_tree: None,
            target_boundary: None,
        });
        (state, events)
    }

    /// 批2 Task 7（Bug45）改形：terminal 生命周期 Execution 事件改用
    /// 稳定 event_id（`kimi_terminal_{terminal_id}`）后，唯一性 = 不同
    /// terminal 不同 id——并发 create 各自登记的 terminal_id 恒异。
    #[cfg(unix)]
    #[tokio::test]
    async fn execution_event_ids_are_unique_under_concurrency() {
        let _dir = tempfile::tempdir().expect("dir");
        // test_state 的 root=系统临时目录已可作 cwd（argv 形态不经 SHELL）。
        let (state, mut events) = test_state(AdapterRole::Executor);
        // 并发上限 4：Finished（未 release）仍占并发名额，用满额并发。
        let creates: Vec<_> = (0..4)
            .map(|_| {
                let state = Arc::clone(&state);
                tokio::spawn(async move {
                    let params = json!({
                        "sessionId": "client-service-test",
                        "command": "/bin/echo",
                        "args": ["concurrent"],
                        "cwd": ""
                    });
                    handle_terminal_create(&state, &params)
                        .await
                        .expect("concurrent create")
                })
            })
            .collect();
        let mut terminals = Vec::new();
        for create in creates {
            terminals.push(create.await.expect("create task"));
        }
        let mut ids = Vec::new();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        while ids.len() < 4 {
            match tokio::time::timeout_at(deadline, events.recv()).await {
                Ok(Some(ProviderEvent::Execution(event))) => ids.push(event.event_id),
                other => panic!("unexpected event while collecting ids: {other:?}"),
            }
        }
        let mut unique = ids.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), ids.len(), "event ids must be unique");
        for terminal in terminals {
            state.terminal.release(&terminal).await.expect("release");
        }
    }

    /// 批2 Task 7（Bug45）红测：同一 terminal 的 Started 与终态 Execution
    /// 事件必须共用稳定 event_id——不沿 Started/Completed 各造不同
    /// event_id（现状：进程级自增 seq，两条事件两个 id，消费方无法配对
    /// 维护在途集合）。
    #[cfg(unix)]
    #[tokio::test]
    async fn terminal_lifecycle_events_share_stable_event_id() {
        let _dir = tempfile::tempdir().expect("dir");
        let (state, mut events) = test_state(AdapterRole::Executor);
        let params = json!({
            "sessionId": "client-service-test",
            "command": "/bin/echo",
            "args": ["stable-id"],
            "cwd": ""
        });
        let terminal_id = handle_terminal_create(&state, &params)
            .await
            .expect("create");
        let wait_params = json!({
            "sessionId": "client-service-test",
            "terminalId": terminal_id,
        });
        handle_terminal_wait(&state, &wait_params)
            .await
            .expect("wait");

        let mut lifecycle = Vec::new();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        while lifecycle.len() < 2 {
            match tokio::time::timeout_at(deadline, events.recv()).await {
                Ok(Some(ProviderEvent::Execution(event))) => lifecycle.push(event),
                other => panic!("unexpected event: {other:?}"),
            }
        }
        assert_eq!(
            lifecycle[0].status,
            ProviderExecutionEventStatus::Started,
            "第一条生命周期事件是 Started"
        );
        assert_eq!(
            lifecycle[1].status,
            ProviderExecutionEventStatus::Completed,
            "wait 后终态事件: {:?}",
            lifecycle[1]
        );
        let expected_id = format!("kimi_terminal_{terminal_id}");
        assert_eq!(
            lifecycle[0].event_id, expected_id,
            "Started 事件用 terminal 稳定 id"
        );
        assert_eq!(
            lifecycle[1].event_id, expected_id,
            "终态事件与 Started 共用同一 event_id（可配对）"
        );
        state.terminal.release(&terminal_id).await.expect("release");
    }

    /// 批2 Task 7（Bug45）红测：会话终结（总 deadline 耗尽/用户取消经
    /// cleanup_cancel 送达）后到达的 terminal/create 必须在 spawn 前拒绝
    /// ——超剩余总预算的启动是配置问题，不得占用并发名额（启动数 0）。
    /// 现状：无该门，迟到 create 会真实 spawn。
    #[cfg(unix)]
    #[tokio::test]
    async fn terminal_create_after_session_end_is_rejected_before_start() {
        let _dir = tempfile::tempdir().expect("dir");
        let (state, _events) = test_state(AdapterRole::Executor);
        state.cleanup_cancel.cancel();
        let params = json!({
            "sessionId": "client-service-test",
            "command": "/bin/echo",
            "args": ["late"],
            "cwd": ""
        });
        let error = handle_terminal_create(&state, &params)
            .await
            .expect_err("session 结束后的 create 必须在 spawn 前拒绝");
        let message = error.to_string();
        assert!(
            message.contains("invocation"),
            "拒绝原因必须是预算/配置问题: {message}"
        );
        assert!(
            !state.terminal.has_active_terminals(),
            "迟到 create 不得登记/启动任何 terminal"
        );
    }

    /// 批2 Task 7 修复轮(F2)钉测：terminal/create 与「dispatcher Drop→
    /// cleanup_all + cleanup_cancel」交错时无逃逸登记——create 要么在
    /// spawn 前被门/复核拒绝，要么其进程被 manager 级收编 token 级联
    /// 收割（120s 独立上限已删，孤儿不允许无时间上限）。
    #[cfg(unix)]
    #[tokio::test]
    async fn terminal_create_racing_session_end_never_escapes() {
        for round in 0..8 {
            let _dir = tempfile::tempdir().expect("dir");
            let (state, _events) = test_state(AdapterRole::Executor);
            let params = json!({
                "sessionId": "client-service-test",
                "command": "/bin/sleep",
                "args": ["30"],
                "cwd": ""
            });
            let create = {
                let state = Arc::clone(&state);
                let params = params.clone();
                tokio::spawn(async move { handle_terminal_create(&state, &params).await })
            };
            // Drop 形态：cleanup_all（此刻可能为空）+ cleanup_cancel。
            tokio::task::yield_now().await;
            state.terminal.cleanup_all();
            state.cleanup_cancel.cancel();
            match create.await.expect("create task") {
                Err(error) => {
                    assert!(
                        error.to_string().contains("invocation"),
                        "round {round}: 拒绝原因必须是预算/配置问题: {error}"
                    );
                }
                Ok(terminal_id) => {
                    // 过门竞态胜出：登记必须被级联收割（有界收口）。
                    let result = tokio::time::timeout(
                        Duration::from_secs(10),
                        state.terminal.wait_for_exit(&terminal_id),
                    )
                    .await
                    .expect("round {round}: 竞态登记不得无上限悬置")
                    .expect("wait racing terminal");
                    assert!(
                        result.killed,
                        "round {round}: 级联必须收割竞态登记: {result:?}"
                    );
                    state
                        .terminal
                        .release(&terminal_id)
                        .await
                        .expect("release racing terminal");
                }
            }
            assert!(
                !state.terminal.has_active_terminals(),
                "round {round}: 不得残留逃逸登记"
            );
        }
    }

    fn cwd_state(root: PathBuf) -> Arc<ClientServiceState> {
        let (event_tx, _events) = mpsc::channel(32);
        Arc::new(ClientServiceState {
            session_id: "client-service-test".to_string(),
            root: root.clone(),
            policy: ClientServicePolicy::new(AdapterRole::Executor, ProviderPermissionMode::Auto),
            bridge: Arc::new(ApprovalBridge::new(
                ProviderPermissionMode::Auto,
                event_tx.clone(),
            )),
            event_tx,
            terminal: TerminalManager::new(),
            session_env: BTreeMap::new(),
            cleanup_cancel: CancellationToken::new().child_token(),
            baseline_tree: None,
            target_boundary: None,
        })
    }

    #[tokio::test]
    async fn terminal_cwd_tolerates_absolute_path_inside_root() {
        let dir = tempfile::tempdir().expect("dir");
        std::fs::create_dir_all(dir.path().join("nested/dir")).expect("mkdir nested");
        let state = cwd_state(dir.path().canonicalize().expect("canonical root"));

        // Absolute cwd equal to the root itself anchors at the root.
        let resolved = resolve_cwd(&state, Some(state.root.to_str().expect("utf-8 root")))
            .expect("absolute cwd at root accepted");
        assert_eq!(resolved, state.root);

        // Absolute cwd pointing at a subdirectory beneath the root.
        let absolute = state.root.join("nested/dir");
        let resolved = resolve_cwd(&state, Some(absolute.to_str().expect("utf-8 path")))
            .expect("absolute cwd inside root accepted");
        assert_eq!(resolved, absolute.canonicalize().expect("canonical subdir"));
    }

    #[tokio::test]
    async fn terminal_cwd_anchors_outside_cli_cwd_at_face_root() {
        // kimi-9 现场修订:kimi CLI 不转发模型 cwd,params.cwd 恒为会话
        // cwd;出界 cwd(绝对出界/父遍历/前缀混淆)不再拒绝,锚定 face
        // root——这是起始目录选择,不是物理隔离(Host 单路径)。
        let dir = tempfile::tempdir().expect("dir");
        std::fs::create_dir_all(dir.path().join("nested/dir")).expect("mkdir nested");
        let outside = tempfile::tempdir().expect("outside");
        let state = cwd_state(dir.path().canonicalize().expect("canonical root"));
        let escapes = [
            "/etc".to_string(),
            outside
                .path()
                .canonicalize()
                .expect("canonical outside")
                .to_string_lossy()
                .into_owned(),
            "../escape".to_string(),
            "nested/../../escape".to_string(),
            state
                .root
                .join("nested/../../escape")
                .to_string_lossy()
                .into_owned(),
        ];
        for escape in &escapes {
            let resolved =
                resolve_cwd(&state, Some(escape)).expect("outside cli cwd anchors at face root");
            assert_eq!(resolved, state.root, "escape={escape}");
        }
    }

    #[tokio::test]
    async fn terminal_cwd_anchors_prefix_confusion_sibling_at_face_root() {
        let parent = tempfile::tempdir().expect("parent");
        let root = parent.path().join("xxx_root");
        let evil = parent.path().join("xxx_root_evil");
        std::fs::create_dir_all(&root).expect("root mkdir");
        std::fs::create_dir_all(&evil).expect("evil mkdir");
        let state = cwd_state(root.canonicalize().expect("canonical root"));

        let resolved = resolve_cwd(&state, Some(evil.to_str().expect("utf-8 path")))
            .expect("prefix-confusion sibling anchors at face root");
        assert_eq!(resolved, state.root);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn terminal_cwd_anchors_symlink_escape_at_face_root() {
        let dir = tempfile::tempdir().expect("dir");
        let outside = tempfile::tempdir().expect("outside");
        let state = cwd_state(dir.path().canonicalize().expect("canonical root"));

        // Symlink inside the root pointing outside: the no-follow walk
        // refuses to anchor through it; the cwd falls back to the face root
        // instead of executing anywhere outside it.
        std::os::unix::fs::symlink(
            outside.path().canonicalize().expect("canonical outside"),
            state.root.join("leak"),
        )
        .expect("symlink");
        let escape = state.root.join("leak");
        let resolved = resolve_cwd(&state, Some(escape.to_str().expect("utf-8 path")))
            .expect("symlink escape anchors at face root");
        assert_eq!(resolved, state.root);

        // Symlink pointing inside the root is likewise not anchored through
        // (no-follow); execution stays at the face root.
        std::fs::create_dir_all(state.root.join("nested/dir")).expect("mkdir nested");
        std::os::unix::fs::symlink("nested", state.root.join("link")).expect("symlink");
        let internal = state.root.join("link");
        let resolved = resolve_cwd(&state, Some(internal.to_str().expect("utf-8 path")))
            .expect("internal symlink anchors at face root");
        assert_eq!(resolved, state.root);
    }

    #[tokio::test]
    async fn terminal_cwd_anchors_ancestor_session_cwd_at_target_face_root_kimi9() {
        // kimi-9 coding fresh 现场回归(r10 2026-10-09):coding executor
        // 会话 cwd=canonical 聚合根,face root=target boundary 的成员
        // worktree(聚合根的严格子目录)。kimi CLI(2.0.2)不转发模型 Bash
        // 工具的 cwd,terminal/create 恒携带会话 cwd → 旧语义把 executor
        // 的每一次 terminal 调用在执行前拒绝(现场两 run 30/30 全拒,含
        // 模型 cwd 与守卫提示根 220 字节逐字节相同的调用),coder 60 分钟
        // 16 轮 plan-defect blocked 后泵超时。锚定语义下:祖先 cwd 落到
        // 成员 worktree,Bash 恢复可用。
        let aggregate = tempfile::tempdir().expect("aggregate");
        let member = aggregate
            .path()
            .join("alpha/.worktrees/aria-issues/issue_0001");
        std::fs::create_dir_all(&member).expect("member mkdir");
        let canonical_member = member.canonicalize().expect("canonical member");
        let (event_tx, _events) = mpsc::channel(32);
        let state = Arc::new(ClientServiceState {
            session_id: "kimi9-ancestor-cwd-regression".to_string(),
            root: aggregate
                .path()
                .canonicalize()
                .expect("canonical aggregate"),
            policy: ClientServicePolicy::new(AdapterRole::Executor, ProviderPermissionMode::Auto),
            bridge: Arc::new(ApprovalBridge::new(
                ProviderPermissionMode::Auto,
                event_tx.clone(),
            )),
            event_tx,
            terminal: TerminalManager::new(),
            session_env: BTreeMap::new(),
            cleanup_cancel: CancellationToken::new().child_token(),
            baseline_tree: None,
            target_boundary: Some(ProviderBoundaryPlan::new(
                ProviderBoundaryMode::TargetWriteOnly,
                aggregate
                    .path()
                    .canonicalize()
                    .expect("canonical aggregate"),
                Some(canonical_member.clone()),
                Vec::new(),
            )),
        });

        // 会话 cwd(聚合根)= face root 的祖先:锚定到成员 worktree。
        let resolved = resolve_cwd(&state, Some(state.root.to_str().expect("utf-8 aggregate")))
            .expect("ancestor session cwd anchors at the target face root");
        assert_eq!(resolved, canonical_member);

        // 聚合根内、worktree 外的中间目录同样锚定回成员 worktree。
        let intermediate = aggregate
            .path()
            .join("alpha")
            .canonicalize()
            .expect("canonical alpha");
        let resolved = resolve_cwd(&state, Some(intermediate.to_str().expect("utf-8 alpha")))
            .expect("intermediate aggregate dir anchors at the target face root");
        assert_eq!(resolved, canonical_member);
    }

    // —— 批2 Task 6 红测:Kimi 全角色 Host 单路径与原生命令形态 ——

    /// Host 执行态的冻结会话环境:携带宿主 PATH(裸名 program 与脚本内
    /// 工具经它解析)+ SHELL(钉 /bin/sh,legacy 形态确定性);与宿主是否
    /// 安装 bubblewrap 无关——Host 单路径无 bwrap 依赖。
    fn host_env() -> BTreeMap<String, String> {
        let mut env = BTreeMap::new();
        if let Some(path) = std::env::var_os("PATH") {
            env.insert("PATH".to_string(), path.to_string_lossy().into_owned());
        }
        env.insert("SHELL".to_string(), "/bin/sh".to_string());
        env
    }

    /// Auto 档 Host 执行态。
    fn host_state(role: AdapterRole, root: PathBuf) -> Arc<ClientServiceState> {
        let (event_tx, _events) = mpsc::channel(64);
        Arc::new(ClientServiceState {
            session_id: "client-service-test".to_string(),
            root,
            policy: ClientServicePolicy::new(role, ProviderPermissionMode::Auto),
            bridge: Arc::new(ApprovalBridge::new(
                ProviderPermissionMode::Auto,
                event_tx.clone(),
            )),
            event_tx,
            terminal: TerminalManager::new(),
            session_env: host_env(),
            cleanup_cancel: CancellationToken::new().child_token(),
            baseline_tree: None,
            target_boundary: None,
        })
    }

    /// Supervised 档状态 + 审批通道(测试充当审批人)。
    fn supervised_host_state(
        role: AdapterRole,
        root: PathBuf,
    ) -> (
        Arc<ClientServiceState>,
        mpsc::Receiver<ProviderEvent>,
        mpsc::Sender<ProviderCommand>,
    ) {
        let (event_tx, events) = mpsc::channel(64);
        let bridge = Arc::new(ApprovalBridge::new(
            ProviderPermissionMode::Supervised,
            event_tx.clone(),
        ));
        let commands = bridge.command_sender();
        let state = Arc::new(ClientServiceState {
            session_id: "client-service-test".to_string(),
            root,
            policy: ClientServicePolicy::new(role, ProviderPermissionMode::Supervised),
            bridge,
            event_tx,
            terminal: TerminalManager::new(),
            session_env: host_env(),
            cleanup_cancel: CancellationToken::new().child_token(),
            baseline_tree: None,
            target_boundary: None,
        });
        (state, events, commands)
    }

    /// 对第一条 PermissionRequest 回复审批决定。
    async fn answer_first_permission(
        events: &mut mpsc::Receiver<ProviderEvent>,
        commands: &mpsc::Sender<ProviderCommand>,
        approved: bool,
        reason: Option<String>,
    ) {
        while let Some(event) = events.recv().await {
            if let ProviderEvent::PermissionRequest(request) = event {
                commands
                    .send(ProviderCommand::PermissionResponse {
                        id: request.id,
                        approved,
                        reason,
                    })
                    .await
                    .expect("send permission response");
                return;
            }
        }
        panic!("no PermissionRequest arrived");
    }

    /// Task 6 红测①(无 bwrap)+ 批2 Task 8 反转:六职责经 5 个 wire
    /// 角色承载(Author 由 Orchestrator 承载,岗位纪律归 T8);policy
    /// 面允许 terminal 的角色(Auto 档)直接启动 Host 进程并完成。
    /// 批2 Task 8:Reviewer/Splitter/Handoff terminal 不再整段拒绝——
    /// 合法操作正向启动(fs 写面的职责拒绝由 policy/fs 测试另行钉住)。
    #[cfg(unix)]
    #[tokio::test]
    async fn host_terminal_starts_for_permitted_roles_without_bwrap_auto() {
        for role in [
            AdapterRole::Orchestrator,
            AdapterRole::Executor,
            AdapterRole::Reviewer,
            AdapterRole::WorkItemSplitter,
            AdapterRole::Handoff,
        ] {
            let dir = tempfile::tempdir().expect("dir");
            let state = host_state(role.clone(), dir.path().canonicalize().expect("root"));
            let params = json!({
                "sessionId": "client-service-test",
                "command": "/bin/sh",
                "args": ["-c", "echo host-start-ok"],
                "cwd": ""
            });
            let id = handle_terminal_create(&state, &params)
                .await
                .unwrap_or_else(|error| {
                    panic!("host terminal must start without bwrap ({role:?}): {error}")
                });
            let result = state.terminal.wait_for_exit(&id).await.expect("wait");
            assert_eq!(result.exit_code, Some(0), "{role:?}");
            assert!(
                state
                    .terminal
                    .output(&id)
                    .expect("output")
                    .contains("host-start-ok"),
                "{role:?}"
            );
            state.terminal.release(&id).await.expect("release");
        }
    }

    /// Task 6 红测②(非 bash 解释器/普通 argv):`command+args[]` 原样
    /// argv——不再有 bash/sh 固定解释器白名单,也不再限定「单 -c 脚本」;
    /// 参数逐字透传(含内嵌空格)。现状:`interpreter is not allowed` /
    /// `single -c script` 拒绝。
    #[cfg(unix)]
    #[tokio::test]
    async fn argv_form_runs_non_bash_programs_verbatim() {
        let dir = tempfile::tempdir().expect("dir");
        let state = host_state(
            AdapterRole::Executor,
            dir.path().canonicalize().expect("root"),
        );

        let params = json!({
            "sessionId": "client-service-test",
            "command": "/bin/echo",
            "args": ["argv-ok", "two  spaces"],
            "cwd": ""
        });
        let id = handle_terminal_create(&state, &params)
            .await
            .unwrap_or_else(|error| panic!("non-bash program must run verbatim: {error}"));
        let result = state.terminal.wait_for_exit(&id).await.expect("wait");
        assert_eq!(result.exit_code, Some(0));
        assert_eq!(
            state.terminal.output(&id).expect("output"),
            "argv-ok two  spaces\n"
        );
        state.terminal.release(&id).await.expect("release");

        let params = json!({
            "sessionId": "client-service-test",
            "command": "/usr/bin/printf",
            "args": ["%s-%s\\n", "multi", "arg"],
            "cwd": ""
        });
        let id = handle_terminal_create(&state, &params)
            .await
            .expect("multi-arg argv form");
        let result = state.terminal.wait_for_exit(&id).await.expect("wait");
        assert_eq!(result.exit_code, Some(0));
        assert_eq!(state.terminal.output(&id).expect("output"), "multi-arg\n");
        state.terminal.release(&id).await.expect("release");
    }

    /// Task 6 红测③(legacy 构建语法):无 `args` 的单字符串经受信 SHELL
    /// `-c` 执行,管道/重定向等 shell 操作符与原参数语义保留。Author
    /// 职责由 Orchestrator 承载,岗位纪律不在本面用 grammar 实现(归 T8)。
    /// 现状:封闭 grammar 以 "shell operator is not allowed" 拒绝。
    #[cfg(unix)]
    #[tokio::test]
    async fn legacy_single_string_keeps_shell_semantics_via_trusted_shell() {
        let dir = tempfile::tempdir().expect("dir");
        let state = host_state(
            AdapterRole::Orchestrator,
            dir.path().canonicalize().expect("root"),
        );
        let params = json!({
            "sessionId": "client-service-test",
            "command": "printf 'legacy-pipe' | tr a-z A-Z > redirect.txt && cat redirect.txt",
            "cwd": ""
        });
        let id = handle_terminal_create(&state, &params)
            .await
            .unwrap_or_else(|error| {
                panic!("legacy shell form must keep operator semantics: {error}")
            });
        let result = state.terminal.wait_for_exit(&id).await.expect("wait");
        assert_eq!(result.exit_code, Some(0));
        assert!(
            state
                .terminal
                .output(&id)
                .expect("output")
                .contains("LEGACY-PIPE"),
            "output: {}",
            state.terminal.output(&id).expect("output")
        );
        state.terminal.release(&id).await.expect("release");
    }

    /// Task 6 红测④(Supervised 审批通过):审批放行后同样启动 Host
    /// (不偷换 Auto 为 Supervised 拒绝)。现状:审批通过后仍死于
    /// bwrap 缺失。
    #[cfg(unix)]
    #[tokio::test]
    async fn supervised_approval_granted_starts_host_terminal() {
        let dir = tempfile::tempdir().expect("dir");
        let (state, mut events, commands) = supervised_host_state(
            AdapterRole::Executor,
            dir.path().canonicalize().expect("root"),
        );
        let create = tokio::spawn({
            let state = Arc::clone(&state);
            async move {
                handle_terminal_create(
                    &state,
                    &json!({
                        "sessionId": "client-service-test",
                        "command": "/bin/sh",
                        "args": ["-c", "echo supervised-host-ok"],
                        "cwd": ""
                    }),
                )
                .await
            }
        });
        answer_first_permission(&mut events, &commands, true, None).await;
        let id = create
            .await
            .expect("create task")
            .expect("approved supervised terminal starts on host");
        let result = state.terminal.wait_for_exit(&id).await.expect("wait");
        assert_eq!(result.exit_code, Some(0));
        assert!(
            state
                .terminal
                .output(&id)
                .expect("output")
                .contains("supervised-host-ok")
        );
        state.terminal.release(&id).await.expect("release");
    }

    /// Task 6 红测⑤(Supervised 拒绝):拒绝审批 → 该工具不执行,但会话
    /// 继续——同一 state 上后续审批通过的调用照常执行(现状:后续调用
    /// 死于 bwrap 缺失 → 后半红)。AskUserQuestion 等待语义由
    /// approval_tests 的既有断言钉住,本面不重复。
    #[cfg(unix)]
    #[tokio::test]
    async fn supervised_rejection_skips_execution_and_session_continues() {
        let dir = tempfile::tempdir().expect("dir");
        let (state, mut events, commands) = supervised_host_state(
            AdapterRole::Orchestrator,
            dir.path().canonicalize().expect("root"),
        );
        let denied = tokio::spawn({
            let state = Arc::clone(&state);
            async move {
                handle_terminal_create(
                    &state,
                    &json!({
                        "sessionId": "client-service-test",
                        "command": "/bin/sh",
                        "args": ["-c", "echo must-not-run"],
                        "cwd": ""
                    }),
                )
                .await
            }
        });
        answer_first_permission(
            &mut events,
            &commands,
            false,
            Some("user said no".to_string()),
        )
        .await;
        let error = denied
            .await
            .expect("create task")
            .expect_err("rejected approval must not execute");
        assert!(error.to_string().contains("user said no"), "{error}");

        let granted = tokio::spawn({
            let state = Arc::clone(&state);
            async move {
                handle_terminal_create(
                    &state,
                    &json!({
                        "sessionId": "client-service-test",
                        "command": "/bin/sh",
                        "args": ["-c", "echo resumed-ok"],
                        "cwd": ""
                    }),
                )
                .await
            }
        });
        answer_first_permission(&mut events, &commands, true, None).await;
        let id = granted
            .await
            .expect("create task")
            .expect("session continues after a rejected approval");
        let result = state.terminal.wait_for_exit(&id).await.expect("wait");
        assert_eq!(result.exit_code, Some(0));
        assert!(
            state
                .terminal
                .output(&id)
                .expect("output")
                .contains("resumed-ok")
        );
        state.terminal.release(&id).await.expect("release");
    }

    /// A1-a(Bug45 批2 环境回退底座)绿证:terminal 子进程继承宿主环境,
    /// dispatcher 的冻结会话环境(session_env)作为显式覆盖层注入——
    /// 自定义键可见、宿主键(session_env 未携带的 HOME)仍从宿主继承;
    /// 两种命令形态(argv/legacy)消费同一覆盖语义。legacy 的受信
    /// SHELL 回退(env 取 SHELL,缺失回退 POSIX /bin/sh)语义不变。
    #[cfg(unix)]
    #[tokio::test]
    async fn terminal_children_inherit_host_env_with_session_overrides() {
        let dir = tempfile::tempdir().expect("dir");
        let (state, _events) = {
            let (event_tx, events) = mpsc::channel(64);
            // session_env 不携带 HOME——宿主 HOME 只能经继承到达子进程,
            // 以此区分「覆盖层注入」与「宿主继承」两个来源。
            let mut env = host_env();
            env.insert("T6_FROZEN_ENV".to_string(), "frozen-env-ok".to_string());
            let state = Arc::new(ClientServiceState {
                session_id: "client-service-test".to_string(),
                root: dir.path().canonicalize().expect("root"),
                policy: ClientServicePolicy::new(
                    AdapterRole::Executor,
                    ProviderPermissionMode::Auto,
                ),
                bridge: Arc::new(ApprovalBridge::new(
                    ProviderPermissionMode::Auto,
                    event_tx.clone(),
                )),
                event_tx,
                terminal: TerminalManager::new(),
                session_env: env,
                cleanup_cancel: CancellationToken::new().child_token(),
                baseline_tree: None,
                target_boundary: None,
            });
            (state, events)
        };
        let host_home = std::env::var("HOME").expect("host HOME");

        // argv 形态:覆盖层自定义键可见 + 宿主 HOME 继承可见。
        let params = json!({
            "sessionId": "client-service-test",
            "command": "/bin/sh",
            "args": [
                "-c",
                "printf '%s' \"$T6_FROZEN_ENV\"; printf ':%s' \"$HOME\"; test -z \"$T6_ABSENT_VAR\""
            ],
            "cwd": ""
        });
        let id = handle_terminal_create(&state, &params)
            .await
            .expect("argv form inherits host env with session overrides");
        let result = state.terminal.wait_for_exit(&id).await.expect("wait");
        assert_eq!(result.exit_code, Some(0));
        assert_eq!(
            state.terminal.output(&id).expect("output"),
            format!("frozen-env-ok:{host_home}")
        );
        state.terminal.release(&id).await.expect("release");

        // legacy 形态:受信 SHELL(-c)同语义执行(宿主继承+会话覆盖)。
        let params = json!({
            "sessionId": "client-service-test",
            "command": "printf 'legacy-%s:%s' \"$T6_FROZEN_ENV\" \"$HOME\"",
            "cwd": ""
        });
        let id = handle_terminal_create(&state, &params)
            .await
            .expect("legacy form inherits host env with session overrides");
        let result = state.terminal.wait_for_exit(&id).await.expect("wait");
        assert_eq!(result.exit_code, Some(0));
        assert_eq!(
            state.terminal.output(&id).expect("output"),
            format!("legacy-frozen-env-ok:{host_home}")
        );
        state.terminal.release(&id).await.expect("release");
    }

    /// Task 6 绿证(受信 SHELL 选择):SHELL 取冻结环境值;空/缺失回退
    /// POSIX `/bin/sh`(不来自 ACP 参数,无固定解释器白名单)。
    #[test]
    fn trusted_shell_follows_frozen_env_with_posix_fallback() {
        let with_shell = BTreeMap::from([
            ("SHELL".to_string(), "/bin/bash".to_string()),
            ("PATH".to_string(), "/usr/bin".to_string()),
        ]);
        assert_eq!(trusted_shell(&with_shell), "/bin/bash");
        assert_eq!(trusted_shell(&BTreeMap::new()), "/bin/sh");
        assert_eq!(
            trusted_shell(&BTreeMap::from([("SHELL".to_string(), String::new())])),
            "/bin/sh"
        );
    }
}
