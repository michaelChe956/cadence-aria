//! Terminal request handlers for the kimi client services: the five
//! `terminal/*` endpoints plus their command-construction, isolation, and
//! cwd-anchoring helpers.

use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Value, json};

use crate::cross_cutting::streaming_provider::{
    ProviderEvent, ProviderExecutionEvent, ProviderExecutionEventKind, ProviderExecutionEventStatus,
};
use crate::protocol::contracts::AdapterRole;

use super::grammar::{Binary, ParsedCommand, parse_command};
use super::policy::ClientAction;
use super::sandbox::{
    canonical_path_of_fd, open_dir_no_follow, resolve_trusted_binary, validate_path_no_follow,
};
use super::terminal::{TerminalCommand, TerminalIsolation, TerminalResult};
use super::{ClientServiceError, ClientServiceState, check_session, evaluate_policy};

use crate::cross_cutting::provider_boundary::{ProviderBoundaryMode, ProviderBoundaryPlan};

#[cfg(test)]
use super::policy::ClientServicePolicy;
#[cfg(test)]
use super::terminal::TerminalManager;
#[cfg(test)]
use super::writable_git_paths_for;

/// Server-injected git hardening config. The grammar forbids the model from
/// supplying `-c`, but the server may add these itself to disable every
/// executable local config item (fsmonitor, external diff/textconv, ssh and
/// credential helpers).
const GIT_HARDENING: [&str; 14] = [
    "-c",
    "core.fsmonitor=false",
    "-c",
    "core.externalDiff=",
    "-c",
    "core.textconv=",
    "-c",
    "diff.external=",
    "-c",
    "diff.textconv=",
    "-c",
    "core.sshCommand=",
    "-c",
    "credential.helper=",
];

fn build_final_argv(parsed: &ParsedCommand) -> Vec<String> {
    let mut out = vec![parsed.binary.name().to_string()];
    match parsed.binary {
        Binary::Git => {
            out.push("--no-pager".to_string());
            out.extend(GIT_HARDENING.iter().map(ToString::to_string));
            let subcommand = &parsed.argv[1];
            out.push(subcommand.clone());
            if subcommand == "diff" {
                out.push("--no-ext-diff".to_string());
                out.push("--no-textconv".to_string());
            }
            out.extend(parsed.argv[2..].iter().cloned());
        }
        _ => out.extend(parsed.argv[1..].iter().cloned()),
    }
    out
}

fn isolation_for(state: &ClientServiceState) -> Result<TerminalIsolation, ClientServiceError> {
    match state.permission_mode {
        crate::cross_cutting::streaming_provider::ProviderPermissionMode::Auto => state
            .bwrap
            .clone()
            .map(|bwrap| {
                // The coding (Executor) contract writes and commits inside
                // its worktree; a linked worktree keeps its git dir outside
                // the root, so those paths are bound with it (F-17). The
                // git bind face is frozen at construction — never
                // re-resolved here: the coder can rewrite `.git` inside
                // the writable root and aim a fresh resolution at any
                // valid host git dir (F-17 fix round 2).
                //
                // Task 4c 读写面分离:LC 会话的可写面由 boundary plan 决定
                // (TargetWriteOnly → face root 可写,ReadOnly → 全只读);
                // direct 会话保持 role 判定原值。
                let writable_root = match state.target_boundary.as_ref() {
                    Some(plan) => plan.mode() == ProviderBoundaryMode::TargetWriteOnly,
                    None => matches!(state.policy.role, AdapterRole::Executor),
                };
                TerminalIsolation::Bubblewrap {
                    bwrap,
                    writable_root,
                    writable_git_paths: state.writable_git_paths.clone(),
                }
            })
            .ok_or_else(|| {
                ClientServiceError::Rejected(
                    "bubblewrap is unavailable; terminal execution is disabled in auto mode"
                        .to_string(),
                )
            }),
        crate::cross_cutting::streaming_provider::ProviderPermissionMode::Supervised => {
            Ok(TerminalIsolation::Unavailable)
        }
    }
}

/// Task 4c 读写面分离:terminal 的授权面根——LC 会话(TargetWriteOnly)为
/// boundary plan 的 target root(bwrap 写面与 cwd 锚定都以它为界;
/// read-only plan/direct 会话为会话根)。terminal 实际 cwd 只在该面内
/// 解析,不改变 provider 进程 cwd(进程 cwd 由 start_validated 冻结为
/// canonical LC root)。
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
/// 混淆/no-follow 拒绝)时不再拒绝,锚定 face_root 本身——terminal 的
/// 执行面与 sandbox 挂载仍以 face root 为界,脚本自带 `cd` 由 sandbox
/// 边界约束,边界语义不放宽。
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
    let command = params
        .get("command")
        .and_then(Value::as_str)
        .filter(|command| !command.trim().is_empty())
        .ok_or_else(|| ClientServiceError::Rejected("terminal command is required".to_string()))?;

    // kimi 0.38.0 sends the argv form: `command` is the interpreter path
    // (e.g. "/bin/bash") and the model's shell script rides in `args`
    // (["-c", "cd '/root' && <script>"]). Verified by raw ACP capture.
    let args = params.get("args").and_then(Value::as_array);

    let (script, parsed) = if let Some(args) = args {
        let interpreter = Path::new(command)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        if interpreter != "bash" && interpreter != "sh" {
            return Err(ClientServiceError::Rejected(format!(
                "terminal interpreter is not allowed: {command}"
            )));
        }
        let args: Vec<&str> = args
            .iter()
            .map(|arg| arg.as_str().unwrap_or_default())
            .collect();
        if args.len() != 2 || !(args[0] == "-c" || args[0] == "-lc") {
            return Err(ClientServiceError::Rejected(
                "terminal args must be a single -c script".to_string(),
            ));
        }
        (args[1].to_string(), None)
    } else {
        // Legacy single-string form: validated against the closed grammar.
        (
            command.to_string(),
            Some(parse_command(command).map_err(ClientServiceError::Grammar)?),
        )
    };

    evaluate_policy(state, ClientAction::Terminal, &script).await?;

    let isolation = isolation_for(state)?;

    let (binary, final_argv) = match parsed.as_ref() {
        Some(parsed) => {
            let binary = resolve_trusted_binary(parsed.binary.name()).ok_or_else(|| {
                ClientServiceError::Internal(format!(
                    "trusted binary not found: {}",
                    parsed.binary.name()
                ))
            })?;
            (binary, build_final_argv(parsed))
        }
        None => resolve_trusted_binary("bash")
            .or_else(|| resolve_trusted_binary("sh"))
            .map(|binary| (binary, vec!["-c".to_string(), script.clone()]))
            .ok_or_else(|| ClientServiceError::Internal("trusted shell not found".to_string()))?,
    };

    // Task 4c 读写面分离:cwd 与 sandbox 面都锚定 boundary plan 的 face
    // root(LC Coding=target;terminal 实际 cwd 不改 provider 进程 cwd)。
    let face_root = terminal_face_root(state)?;
    let cwd = resolve_cwd(state, params.get("cwd").and_then(Value::as_str))?;
    if let Some(parsed) = parsed.as_ref() {
        for operand in &parsed.path_operands {
            validate_path_no_follow(&cwd, Path::new(operand)).map_err(|error| {
                ClientServiceError::Rejected(format!("path operand rejected: {operand}: {error}"))
            })?;
        }
    }
    let terminal_command = TerminalCommand {
        argv: final_argv,
        binary,
        root: face_root,
        cwd,
        isolation,
    };

    let terminal_id = state
        .terminal
        .create(terminal_command.clone())
        .await
        .map_err(ClientServiceError::Terminal)?;

    emit_execution_event(
        state,
        ProviderExecutionEventKind::Command,
        ProviderExecutionEventStatus::Started,
        script.clone(),
        Some(terminal_command.argv.join(" ")),
        Some(terminal_command.cwd.to_string_lossy().into_owned()),
        None,
        None,
    );

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
    emit_execution_event(
        state,
        ProviderExecutionEventKind::Command,
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
    );
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

/// Monotonic counter for execution event ids. Timestamp-based ids can
/// collide when events are emitted concurrently; a process-wide atomic
/// counter is unique by construction.
static EXECUTION_EVENT_SEQ: AtomicU64 = AtomicU64::new(0);

#[allow(clippy::too_many_arguments)]
fn emit_execution_event(
    state: &ClientServiceState,
    kind: ProviderExecutionEventKind,
    status: ProviderExecutionEventStatus,
    title: String,
    command: Option<String>,
    cwd: Option<String>,
    output: Option<String>,
    exit_code: Option<i32>,
) {
    let event_id = format!(
        "kimi_exec_{}",
        EXECUTION_EVENT_SEQ.fetch_add(1, Ordering::Relaxed)
    );
    let _ = state
        .event_tx
        .try_send(ProviderEvent::Execution(ProviderExecutionEvent {
            event_id,
            kind,
            status,
            title,
            detail: None,
            command,
            cwd,
            output,
            exit_code,
        }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    use crate::cross_cutting::approval_bridge::ApprovalBridge;
    use crate::cross_cutting::streaming_provider::ProviderPermissionMode;

    #[test]
    fn git_argv_prepends_no_pager_and_hardening() {
        let parsed = parse_command("git status").expect("parse");
        let argv = build_final_argv(&parsed);
        assert_eq!(argv[0], "git");
        assert_eq!(argv[1], "--no-pager");
        assert!(argv.contains(&"-c".to_string()));
        assert!(argv.contains(&"core.fsmonitor=false".to_string()));
        assert!(argv.contains(&"status".to_string()));

        let parsed = parse_command("git diff --stat").expect("parse");
        let argv = build_final_argv(&parsed);
        let diff_index = argv.iter().position(|arg| arg == "diff").expect("diff");
        assert_eq!(argv[diff_index + 1], "--no-ext-diff");
        assert_eq!(argv[diff_index + 2], "--no-textconv");
    }

    #[test]
    fn non_git_argv_is_passthrough() {
        let parsed = parse_command("rg -n -- foo src").expect("parse");
        let argv = build_final_argv(&parsed);
        assert_eq!(argv, vec!["rg", "-n", "--", "foo", "src"]);
    }

    fn test_state(role: AdapterRole) -> (Arc<ClientServiceState>, mpsc::Receiver<ProviderEvent>) {
        let (event_tx, events) = mpsc::channel(512);
        let state = Arc::new(ClientServiceState {
            session_id: "client-service-test".to_string(),
            root: std::env::temp_dir(),
            policy: ClientServicePolicy::new(role, ProviderPermissionMode::Auto),
            permission_mode: ProviderPermissionMode::Auto,
            bridge: Arc::new(ApprovalBridge::new(
                ProviderPermissionMode::Auto,
                event_tx.clone(),
            )),
            event_tx,
            terminal: TerminalManager::new(),
            bwrap: None,
            writable_git_paths: Vec::new(),
            cleanup_cancel: CancellationToken::new().child_token(),
            baseline_tree: None,
            target_boundary: None,
        });
        (state, events)
    }

    #[tokio::test]
    async fn execution_event_ids_are_unique_under_concurrency() {
        let (state, mut events) = test_state(AdapterRole::Executor);
        let emitter = {
            let state = Arc::clone(&state);
            tokio::spawn(async move {
                for _ in 0..64 {
                    emit_execution_event(
                        &state,
                        ProviderExecutionEventKind::Command,
                        ProviderExecutionEventStatus::Started,
                        "t".to_string(),
                        None,
                        None,
                        None,
                        None,
                    );
                }
            })
        };
        let another = {
            let state = Arc::clone(&state);
            tokio::spawn(async move {
                for _ in 0..64 {
                    emit_execution_event(
                        &state,
                        ProviderExecutionEventKind::Command,
                        ProviderExecutionEventStatus::Started,
                        "t".to_string(),
                        None,
                        None,
                        None,
                        None,
                    );
                }
            })
        };
        emitter.await.expect("emitter");
        another.await.expect("another");
        let mut ids = Vec::new();
        while ids.len() < 128 {
            match tokio::time::timeout(Duration::from_secs(2), events.recv()).await {
                Ok(Some(ProviderEvent::Execution(event))) => ids.push(event.event_id),
                other => panic!("unexpected event while collecting ids: {other:?}"),
            }
        }
        let mut unique = ids.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), ids.len(), "event ids must be unique");
    }

    #[tokio::test]
    async fn kimi_038_create_rejects_unexpected_interpreter_and_args() {
        let (state, _events) = test_state(AdapterRole::Executor);
        // Validation of the kimi 0.38.0 argv form happens before policy and
        // isolation, so these reject deterministically without bwrap.
        let bad_interpreter = json!({
            "sessionId": "client-service-test",
            "command": "/usr/bin/python3",
            "args": ["-c", "print('nope')"],
            "cwd": ""
        });
        let error = handle_terminal_create(&state, &bad_interpreter)
            .await
            .expect_err("interpreter rejected");
        assert!(error.to_string().contains("interpreter is not allowed"));

        let bad_args = json!({
            "sessionId": "client-service-test",
            "command": "/bin/bash",
            "args": ["-c", "echo a", "echo b"],
            "cwd": ""
        });
        let error = handle_terminal_create(&state, &bad_args)
            .await
            .expect_err("args rejected");
        assert!(error.to_string().contains("single -c script"));
    }

    fn cwd_state(root: PathBuf) -> Arc<ClientServiceState> {
        let (event_tx, _events) = mpsc::channel(32);
        Arc::new(ClientServiceState {
            session_id: "client-service-test".to_string(),
            root: root.clone(),
            policy: ClientServicePolicy::new(AdapterRole::Executor, ProviderPermissionMode::Auto),
            permission_mode: ProviderPermissionMode::Auto,
            bridge: Arc::new(ApprovalBridge::new(
                ProviderPermissionMode::Auto,
                event_tx.clone(),
            )),
            event_tx,
            terminal: TerminalManager::new(),
            bwrap: None,
            writable_git_paths: writable_git_paths_for(&AdapterRole::Executor, &root, None),
            cleanup_cancel: CancellationToken::new().child_token(),
            baseline_tree: None,
            target_boundary: None,
        })
    }

    /// F-17:可写沙箱授权只发给 Executor(coder 契约:写路径+commit 责任);
    /// Orchestrator 的终端保持只读挂载语义。bwrap 路径仅作形状参数,
    /// `isolation_for` 只探测 Some/None,不执行该二进制。git bind 面在
    /// 构造期解析一次(与 `ClientServiceState::new` 同款)。
    fn bwrap_state_at(role: AdapterRole, root: PathBuf) -> Arc<ClientServiceState> {
        let (event_tx, _events) = mpsc::channel(32);
        Arc::new(ClientServiceState {
            session_id: "client-service-test".to_string(),
            root: root.clone(),
            policy: ClientServicePolicy::new(role.clone(), ProviderPermissionMode::Auto),
            permission_mode: ProviderPermissionMode::Auto,
            bridge: Arc::new(ApprovalBridge::new(
                ProviderPermissionMode::Auto,
                event_tx.clone(),
            )),
            event_tx,
            terminal: TerminalManager::new(),
            bwrap: Some(PathBuf::from("/usr/bin/bwrap")),
            writable_git_paths: writable_git_paths_for(&role, &root, None),
            cleanup_cancel: CancellationToken::new().child_token(),
            baseline_tree: None,
            target_boundary: None,
        })
    }

    fn bwrap_state(role: AdapterRole) -> Arc<ClientServiceState> {
        let dir = tempfile::tempdir().expect("dir");
        bwrap_state_at(role, dir.path().canonicalize().expect("canonical root"))
    }

    #[tokio::test]
    async fn isolation_grants_writable_root_to_executor_role_only() {
        match isolation_for(&bwrap_state(AdapterRole::Executor)).expect("executor isolation") {
            TerminalIsolation::Bubblewrap { writable_root, .. } => assert!(
                writable_root,
                "coder contract (F-17): executor worktree must be writable inside the sandbox"
            ),
            TerminalIsolation::Unavailable => panic!("expected bubblewrap isolation"),
        }
        match isolation_for(&bwrap_state(AdapterRole::Orchestrator)).expect("orchestrator") {
            TerminalIsolation::Bubblewrap { writable_root, .. } => assert!(
                !writable_root,
                "planning terminal keeps the read-only mount"
            ),
            TerminalIsolation::Unavailable => panic!("expected bubblewrap isolation"),
        }
    }

    /// fix round 2(F-17/k3 P1,安全洞):git bind 面在构造期冻结。root 在
    /// Executor 沙箱内 rw,coder 可改写 `.git` 指针;若每条 terminal 命令
    /// 重跑 rev-parse,后续命令的额外 rw bind 会被瞄准宿主任意合法 git 仓。
    #[tokio::test]
    async fn isolation_git_binds_frozen_at_construction_survive_pointer_swap() {
        fn host_git(args: &[&str], cwd: &Path) {
            let status = std::process::Command::new("git")
                .args(args)
                .current_dir(cwd)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_SYSTEM", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .status()
                .expect("host git");
            assert!(status.success(), "host git {args:?} failed");
        }
        let base = tempfile::tempdir().expect("workspace");
        let base = base.path().canonicalize().expect("canonical base");
        // Aria 准备的真实形态:主仓 + linked worktree(授权根)。
        let main = base.join("main");
        host_git(&["init", "-q", "main"], &base);
        host_git(
            &[
                "-c",
                "user.name=f17",
                "-c",
                "user.email=f17@example.com",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                "base",
            ],
            &main,
        );
        host_git(&["worktree", "add", "../wt", "-b", "f17wt"], &main);
        let root = base.join("wt");
        let root = root.canonicalize().expect("canonical worktree");
        // 宿主上另一个合法 git 仓:coder 想让额外 rw bind 瞄准的目标。
        host_git(&["init", "-q", "evil"], &base);
        let evil_gitdir = base.join("evil").join(".git");

        let state = bwrap_state_at(AdapterRole::Executor, root.clone());
        let before = isolation_for(&state).expect("isolation before swap");
        let TerminalIsolation::Bubblewrap {
            writable_git_paths: before_paths,
            ..
        } = &before
        else {
            panic!("expected bubblewrap isolation");
        };
        assert_eq!(
            before_paths,
            &[main.join(".git").canonicalize().expect("commondir")]
        );

        // coder 在沙箱内改写 `.git` 指针(根内文件,rw 可写)指向 evil 仓。
        std::fs::write(
            root.join(".git"),
            format!("gitdir: {}\n", evil_gitdir.display()),
        )
        .expect("swap pointer");

        // 后续 terminal 命令的 bind 面不得漂移:构造期冻结。
        let after = isolation_for(&state).expect("isolation after swap");
        let TerminalIsolation::Bubblewrap {
            writable_git_paths: after_paths,
            ..
        } = &after
        else {
            panic!("expected bubblewrap isolation");
        };
        assert_eq!(
            after_paths, before_paths,
            "git bind face must stay frozen at construction (F-17 fix round 2)"
        );
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
        // cwd;出界 cwd(绝对出界/父遍历/前缀混淆)不再拒绝,锚定 face root
        // ——终端执行面仍以 face root 为界,脚本自带 cd 由 sandbox 约束。
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
            permission_mode: ProviderPermissionMode::Auto,
            bridge: Arc::new(ApprovalBridge::new(
                ProviderPermissionMode::Auto,
                event_tx.clone(),
            )),
            event_tx,
            terminal: TerminalManager::new(),
            bwrap: None,
            writable_git_paths: Vec::new(),
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
}
