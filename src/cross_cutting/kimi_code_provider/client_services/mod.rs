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
    if let Some(session_id) = params.get("sessionId").and_then(Value::as_str) {
        if session_id != state.session_id {
            return Err(ClientServiceError::Rejected(
                "sessionId does not match the active session".to_string(),
            ));
        }
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
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default()
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
}
