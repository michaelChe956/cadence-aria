//! `KimiClientServiceDispatcher`: concurrent, non-blocking dispatch of
//! server→client `terminal/*` and `fs/*` requests from the kimi ACP server.
//!
//! Every request is recognized on the JsonRpcPeer read loop and immediately
//! handed to a spawned task, so a pending `terminal/wait_for_exit` never
//! blocks the main prompt queue and `kill`/`release`/session cancel stay
//! concurrent with it.

mod fs_service;
mod grammar;
mod policy;
mod sandbox;
mod terminal;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::approval_bridge::ApprovalBridge;
use crate::cross_cutting::json_rpc_peer::JsonRpcPeer;
use crate::cross_cutting::streaming_provider::{
    ProviderEvent, ProviderExecutionEvent, ProviderExecutionEventKind,
    ProviderExecutionEventStatus, RiskLevel,
};
use crate::protocol::contracts::AdapterRole;

use self::fs_service::{FsError, read_text_file, write_text_file};
use self::grammar::{Binary, GrammarError, ParsedCommand, parse_command};
use self::policy::{ClientAction, ClientServicePolicy, PolicyDecision};
use self::sandbox::{
    canonical_path_of_fd, canonicalize_root, open_dir_no_follow, probe_bwrap,
    resolve_trusted_binary, validate_path_no_follow,
};
use self::terminal::{
    TerminalCommand, TerminalError, TerminalIsolation, TerminalManager, TerminalOutputEvent,
};

/// Adapter-local constant: kimi's boolean client-capability dialect. It never
/// leaks into the generic ACP layer.
pub fn kimi_client_capabilities() -> Value {
    json!({
        "fs": {"readTextFile": true, "writeTextFile": true},
        "terminal": true
    })
}

const ERROR_REJECTED: i64 = -32000;
const ERROR_UNKNOWN_TERMINAL: i64 = -32001;
const ERROR_FS: i64 = -32002;

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

struct ClientServiceState {
    session_id: String,
    root: PathBuf,
    policy: ClientServicePolicy,
    permission_mode: crate::cross_cutting::streaming_provider::ProviderPermissionMode,
    bridge: Arc<ApprovalBridge>,
    event_tx: mpsc::Sender<ProviderEvent>,
    terminal: TerminalManager,
    bwrap: Option<PathBuf>,
    cancel: CancellationToken,
}

pub struct KimiClientServiceDispatcher<W> {
    peer: JsonRpcPeer<W>,
    state: Arc<ClientServiceState>,
}

#[derive(Debug)]
enum ClientServiceError {
    Rejected(String),
    Grammar(GrammarError),
    Terminal(TerminalError),
    Fs(FsError),
    Internal(String),
}

impl ClientServiceError {
    fn code(&self) -> i64 {
        match self {
            ClientServiceError::Rejected(_) => ERROR_REJECTED,
            ClientServiceError::Grammar(_) => ERROR_REJECTED,
            ClientServiceError::Terminal(TerminalError::UnknownTerminal(_)) => {
                ERROR_UNKNOWN_TERMINAL
            }
            ClientServiceError::Terminal(_) => ERROR_REJECTED,
            ClientServiceError::Fs(_) => ERROR_FS,
            ClientServiceError::Internal(_) => ERROR_REJECTED,
        }
    }
}

impl std::fmt::Display for ClientServiceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientServiceError::Rejected(detail) => write!(formatter, "{detail}"),
            ClientServiceError::Grammar(error) => write!(formatter, "{error}"),
            ClientServiceError::Terminal(error) => write!(formatter, "{error}"),
            ClientServiceError::Fs(error) => write!(formatter, "{error}"),
            ClientServiceError::Internal(detail) => write!(formatter, "{detail}"),
        }
    }
}

impl<W> KimiClientServiceDispatcher<W>
where
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    /// 8 个参数均为构造期一次性注入的运行时依赖（peer/session/working_dir/role/permission_mode/bridge/event_tx/cancel），
    /// 无自然聚合语义且仅在 `new` 使用一次，重构为 struct 会引入额外间接层，故允许 clippy::too_many_arguments。
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        peer: JsonRpcPeer<W>,
        session_id: String,
        working_dir: PathBuf,
        role: AdapterRole,
        permission_mode: crate::cross_cutting::streaming_provider::ProviderPermissionMode,
        bridge: Arc<ApprovalBridge>,
        event_tx: mpsc::Sender<ProviderEvent>,
        cancel: CancellationToken,
    ) -> Self {
        let root = canonicalize_root(&working_dir).unwrap_or(working_dir);
        let bwrap = probe_bwrap();
        let (output_tx, mut output_rx) = mpsc::channel::<TerminalOutputEvent>(64);
        let terminal = TerminalManager::new(output_tx);

        // Output pump: forwards terminal output notifications without ever
        // blocking the terminal tasks or the main RPC read loop.
        {
            let peer = peer.clone();
            let session_id = session_id.clone();
            tokio::spawn(async move {
                while let Some(event) = output_rx.recv().await {
                    let message = json!({
                        "jsonrpc": "2.0",
                        "method": "terminal/output",
                        "params": {
                            "sessionId": session_id,
                            "terminalId": event.terminal_id,
                            "stream": event.stream.as_str(),
                            "output": event.output,
                        }
                    });
                    if peer.send(message).await.is_err() {
                        continue;
                    }
                }
            });
        }

        let state = Arc::new(ClientServiceState {
            session_id,
            root,
            policy: ClientServicePolicy::new(role, permission_mode.clone()),
            permission_mode,
            bridge,
            event_tx,
            terminal,
            bwrap,
            cancel,
        });

        Self { peer, state }
    }

    /// Recognize a server→client request. Returns `true` when the method was
    /// a client-service request and has been dispatched to a spawned task.
    pub fn dispatch(&self, method: &str, id: Value, params: Value) -> bool {
        if !is_client_service_method(method) {
            return false;
        }
        let peer = self.peer.clone();
        let state = Arc::clone(&self.state);
        let method = method.to_string();
        tokio::spawn(async move {
            handle_request(peer, state, &method, id, params).await;
        });
        true
    }
}

impl<W> Drop for KimiClientServiceDispatcher<W> {
    fn drop(&mut self) {
        self.state.terminal.cleanup_all();
        self.state.cancel.cancel();
    }
}

fn is_client_service_method(method: &str) -> bool {
    matches!(
        method,
        "terminal/create"
            | "terminal/wait_for_exit"
            | "terminal/kill"
            | "terminal/release"
            | "fs/read_text_file"
            | "fs/write_text_file"
    )
}

async fn handle_request<W>(
    peer: JsonRpcPeer<W>,
    state: Arc<ClientServiceState>,
    method: &str,
    id: Value,
    params: Value,
) where
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let result = match method {
        "terminal/create" => handle_terminal_create(&state, &params)
            .await
            .map(|terminal_id| json!({"terminalId": terminal_id})),
        "terminal/wait_for_exit" => handle_terminal_wait(&state, &params).await.map(|result| {
            json!({
                "exitCode": result.exit_code,
                "timedOut": result.timed_out,
                "killed": result.killed,
                "truncated": result.truncated,
            })
        }),
        "terminal/kill" => handle_terminal_kill(&state, &params)
            .await
            .map(|_| json!({})),
        "terminal/release" => handle_terminal_release(&state, &params)
            .await
            .map(|_| json!({})),
        "fs/read_text_file" => handle_fs_read(&state, &params)
            .await
            .map(|content| json!({"content": content})),
        "fs/write_text_file" => handle_fs_write(&state, &params).await.map(|_| json!({})),
        _ => Err(ClientServiceError::Internal(
            "unknown client service method".into(),
        )),
    };

    let response = match result {
        Ok(result) => json!({"jsonrpc":"2.0","id":id,"result":result}),
        Err(error) => json!({
            "jsonrpc":"2.0","id":id,
            "error":{"code":error.code(),"message":error.to_string()}
        }),
    };
    if peer.send(response).await.is_err() {
        tracing::debug!(target: "kimi_code_provider", method, "kimi client service response send failed");
    }
}

fn check_session(state: &ClientServiceState, params: &Value) -> Result<(), ClientServiceError> {
    if let Some(session_id) = params.get("sessionId").and_then(Value::as_str)
        && session_id != state.session_id
    {
        return Err(ClientServiceError::Rejected(
            "sessionId does not match the active session".to_string(),
        ));
    }
    Ok(())
}

async fn evaluate_policy(
    state: &ClientServiceState,
    action: ClientAction,
    description: &str,
) -> Result<(), ClientServiceError> {
    match state.policy.evaluate(action) {
        PolicyDecision::Allow => Ok(()),
        PolicyDecision::Deny(reason) => Err(ClientServiceError::Rejected(reason.to_string())),
        PolicyDecision::RequireApproval => {
            let risk = match action {
                ClientAction::Terminal | ClientAction::FsWrite => RiskLevel::High,
                ClientAction::FsRead => RiskLevel::Low,
            };
            let tool_name = match action {
                ClientAction::Terminal => "terminal",
                ClientAction::FsWrite => "fs_write_text_file",
                ClientAction::FsRead => "fs_read_text_file",
            };
            let decision = state
                .bridge
                .request_tool(tool_name, description, risk, state.cancel.clone())
                .await
                .map_err(|error| ClientServiceError::Internal(error.details))?;
            if decision.approved {
                Ok(())
            } else {
                Err(ClientServiceError::Rejected(
                    decision
                        .reason
                        .unwrap_or_else(|| "rejected by user".to_string()),
                ))
            }
        }
    }
}

fn isolation_for(state: &ClientServiceState) -> Result<TerminalIsolation, ClientServiceError> {
    match state.permission_mode {
        crate::cross_cutting::streaming_provider::ProviderPermissionMode::Auto => state
            .bwrap
            .clone()
            .map(|bwrap| TerminalIsolation::Bubblewrap { bwrap })
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

/// Resolve a terminal cwd inside the authorized root (or the root itself),
/// rejecting symlinks via `openat` + `O_NOFOLLOW` and returning the canonical
/// path of the anchored directory fd.
fn resolve_cwd(
    state: &ClientServiceState,
    cwd: Option<&str>,
) -> Result<PathBuf, ClientServiceError> {
    let Some(cwd) = cwd.filter(|cwd| !cwd.is_empty()) else {
        return Ok(state.root.clone());
    };
    let rel = Path::new(cwd);
    if rel.is_absolute() || cwd.split('/').any(|component| component == "..") {
        return Err(ClientServiceError::Rejected(
            "terminal cwd must stay inside the authorized root".to_string(),
        ));
    }
    let fd = open_dir_no_follow(&state.root, rel)
        .map_err(|error| ClientServiceError::Rejected(format!("terminal cwd rejected: {error}")))?;
    let canonical = canonical_path_of_fd(&fd)
        .map_err(|error| ClientServiceError::Rejected(format!("terminal cwd: {error}")))?;
    if !canonical.starts_with(&state.root) {
        return Err(ClientServiceError::Rejected(
            "terminal cwd is outside the authorized root".to_string(),
        ));
    }
    Ok(canonical)
}

async fn handle_terminal_create(
    state: &ClientServiceState,
    params: &Value,
) -> Result<String, ClientServiceError> {
    check_session(state, params)?;
    let command = params
        .get("command")
        .and_then(Value::as_str)
        .filter(|command| !command.trim().is_empty())
        .ok_or_else(|| ClientServiceError::Rejected("terminal command is required".to_string()))?;

    let parsed = parse_command(command).map_err(ClientServiceError::Grammar)?;

    evaluate_policy(state, ClientAction::Terminal, command).await?;

    let isolation = isolation_for(state)?;

    let binary = resolve_trusted_binary(parsed.binary.name()).ok_or_else(|| {
        ClientServiceError::Internal(format!(
            "trusted binary not found: {}",
            parsed.binary.name()
        ))
    })?;

    let cwd = resolve_cwd(state, params.get("cwd").and_then(Value::as_str))?;
    for operand in &parsed.path_operands {
        validate_path_no_follow(&cwd, Path::new(operand)).map_err(|error| {
            ClientServiceError::Rejected(format!("path operand rejected: {operand}: {error}"))
        })?;
    }

    let final_argv = build_final_argv(&parsed);
    let terminal_command = TerminalCommand {
        argv: final_argv,
        binary,
        root: state.root.clone(),
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
        command.to_string(),
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

async fn handle_terminal_wait(
    state: &ClientServiceState,
    params: &Value,
) -> Result<terminal::TerminalResult, ClientServiceError> {
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

async fn handle_terminal_kill(
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

async fn handle_terminal_release(
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

async fn handle_fs_read(
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

async fn handle_fs_write(
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
    use crate::cross_cutting::streaming_provider::ProviderPermissionMode;
    use std::time::Duration;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

    #[test]
    fn kimi_client_capabilities_are_boolean_dialect() {
        let capabilities =
            serde_json::from_value::<serde_json::Map<String, Value>>(kimi_client_capabilities())
                .expect("capabilities object");
        assert_eq!(capabilities["terminal"], json!(true));
        assert_eq!(capabilities["fs"]["readTextFile"], json!(true));
        assert_eq!(capabilities["fs"]["writeTextFile"], json!(true));
    }

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
        let (terminal_tx, _terminal_rx) = mpsc::channel(1);
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
            terminal: TerminalManager::new(terminal_tx),
            bwrap: None,
            cancel: CancellationToken::new(),
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
    async fn reviewer_role_is_read_only_and_confined_to_authorized_root() {
        let dir = tempfile::tempdir().expect("dir");
        let (client, server) = tokio::io::duplex(16 * 1024);
        let (reader, writer) = tokio::io::split(client);
        let peer = JsonRpcPeer::new(reader, writer);
        let (event_tx, _events) = mpsc::channel(32);
        let dispatcher = KimiClientServiceDispatcher::new(
            peer,
            "reviewer-session".to_string(),
            dir.path().to_path_buf(),
            AdapterRole::Reviewer,
            ProviderPermissionMode::Auto,
            Arc::new(ApprovalBridge::new(
                ProviderPermissionMode::Auto,
                event_tx.clone(),
            )),
            event_tx,
            CancellationToken::new(),
        );
        let (server_reader, mut server_writer) = tokio::io::split(server);
        let mut server_reader = tokio::io::BufReader::new(server_reader);

        // Reviewer cannot execute terminals or write files even though the
        // root is the authorized worktree: the role boundary precedes any
        // root-based authorization.
        let cases = [
            (
                "terminal/create",
                json!({
                    "sessionId": "reviewer-session",
                    "command": "git status",
                    "cwd": ""
                }),
                ERROR_REJECTED,
            ),
            (
                "fs/write_text_file",
                json!({
                    "sessionId": "reviewer-session",
                    "path": "notes.txt",
                    "content": "x"
                }),
                ERROR_REJECTED,
            ),
            // The single permitted action (fs read) is still confined to the
            // authorized root; escaping paths are rejected.
            (
                "fs/read_text_file",
                json!({
                    "sessionId": "reviewer-session",
                    "path": "../escape.txt"
                }),
                ERROR_FS,
            ),
        ];
        for (index, (method, params, expected_code)) in cases.iter().enumerate() {
            assert!(dispatcher.dispatch(method, json!(index), params.clone()));
            let reply = {
                let mut line = String::new();
                tokio::time::timeout(std::time::Duration::from_secs(2), async {
                    loop {
                        line.clear();
                        server_reader
                            .read_line(&mut line)
                            .await
                            .expect("read reply");
                        if !line.trim().is_empty() {
                            break;
                        }
                    }
                })
                .await
                .expect("reply within timeout");
                serde_json::from_str::<Value>(&line).expect("reply json")
            };
            assert_eq!(reply["id"], json!(index), "{method}");
            assert_eq!(
                reply["error"]["code"],
                json!(expected_code),
                "{method}: {reply}"
            );
            if *expected_code == ERROR_REJECTED {
                assert!(
                    reply["error"]["message"]
                        .as_str()
                        .expect("message")
                        .contains("reviewer"),
                    "{method}: {reply}"
                );
            }
        }
        let _ = server_writer.shutdown().await;
        drop(dispatcher);
    }
}
