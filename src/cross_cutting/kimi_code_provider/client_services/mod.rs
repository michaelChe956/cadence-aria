//! `KimiClientServiceDispatcher`: concurrent, non-blocking dispatch of
//! server→client `terminal/*` and `fs/*` requests from the kimi ACP server.
//!
//! Every request is recognized on the JsonRpcPeer read loop and immediately
//! handed to a spawned task, so a pending `terminal/wait_for_exit` never
//! blocks the main prompt queue and `kill`/`release`/session cancel stay
//! concurrent with it.

mod fs_handlers;
mod fs_service;
mod grammar;
mod policy;
mod sandbox;
mod terminal;
mod terminal_handlers;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::approval_bridge::ApprovalBridge;
use crate::cross_cutting::json_rpc_peer::JsonRpcPeer;
use crate::cross_cutting::streaming_provider::{BaselineTreeRef, ProviderEvent, RiskLevel};
use crate::protocol::contracts::AdapterRole;

use self::fs_handlers::{handle_fs_read, handle_fs_write};
use self::fs_service::FsError;
use self::grammar::GrammarError;
use self::policy::{ClientAction, ClientServicePolicy, PolicyDecision};
use self::sandbox::{canonicalize_root, frozen_writable_git_paths, probe_bwrap};
use self::terminal::{TerminalError, TerminalManager};
use self::terminal_handlers::{
    handle_terminal_create, handle_terminal_kill, handle_terminal_output, handle_terminal_release,
    handle_terminal_wait,
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

struct ClientServiceState {
    session_id: String,
    root: PathBuf,
    policy: ClientServicePolicy,
    permission_mode: crate::cross_cutting::streaming_provider::ProviderPermissionMode,
    bridge: Arc<ApprovalBridge>,
    event_tx: mpsc::Sender<ProviderEvent>,
    terminal: TerminalManager,
    bwrap: Option<PathBuf>,
    /// REQ-PIB-02：基线会话锚点。`Some` 时 fs 读路由基线树、terminal 一律
    /// 拒绝（见 `evaluate_policy` 前置与 `fs_handlers`）；`None` 行为不变。
    baseline_tree: Option<BaselineTreeRef>,
    /// Extra read-write sandbox binds for the coding role, resolved ONCE at
    /// construction — when the root is still Aria-prepared and untouched by
    /// the coder. Re-resolving per terminal command would follow a
    /// coder-rewritten `.git` pointer at an arbitrary valid host git dir and
    /// aim the next extra rw bind at it (F-17 fix round 2).
    writable_git_paths: Vec<PathBuf>,
    /// 会话私有清理 token：随引擎 run token（父）取消而取消；dispatcher Drop 时
    /// 只取消它，绝不反噬父 token——否则 kimi 正常完成后会 cancel 引擎 run，
    /// biased select 的 cancel 分支会吞掉已入队的 Completed 事件（真机 issue_0035）。
    cleanup_cancel: CancellationToken,
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
        baseline_tree: Option<BaselineTreeRef>,
    ) -> Self {
        let root = canonicalize_root(&working_dir).unwrap_or(working_dir);
        // Freeze the git bind face before the coder can touch the root:
        // per-command re-resolution would follow a swapped `.git` pointer
        // (F-17 fix round 2).
        let writable_git_paths = writable_git_paths_for(&role, &root);
        let bwrap = probe_bwrap();
        // Command output is served through `terminal/output`
        // request/response (see `handle_terminal_output`); kimi 0.38.0
        // ignores unsolicited `terminal/output` notifications.
        let terminal = TerminalManager::new();

        let state = Arc::new(ClientServiceState {
            session_id,
            root,
            policy: ClientServicePolicy::new(role, permission_mode.clone()),
            permission_mode,
            bridge,
            event_tx,
            terminal,
            bwrap,
            writable_git_paths,
            baseline_tree,
            cleanup_cancel: cancel.child_token(),
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
        // 只取消会话私有 child token：解除仍在等待权限的派发 task，但不得取消
        // 引擎 run token（父），否则正常完成会被误判为中止。
        self.state.cleanup_cancel.cancel();
    }
}

fn is_client_service_method(method: &str) -> bool {
    matches!(
        method,
        "terminal/create"
            | "terminal/wait_for_exit"
            | "terminal/output"
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
        "terminal/output" => handle_terminal_output(&state, &params)
            .await
            .map(|output| json!({"output": output})),
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
    // REQ-PIB-02：基线会话 terminal 一律拒绝——bwrap `--ro-bind / /` 是写边界
    // 非读边界（宿主全量只读可见，F-57 现场通道），命令语法收窄也不足以绑定
    // 基线 ref。前置判定独立于 policy.rs 决策矩阵（24 格冻结，零改动）。
    if state.baseline_tree.is_some() && matches!(action, ClientAction::Terminal) {
        return Err(ClientServiceError::Rejected(
            "baseline session: terminal access is denied (REQ-PIB-02; issue baseline tree is the only readable source)"
                .to_string(),
        ));
    }
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
                .request_tool(tool_name, description, risk, state.cleanup_cancel.clone())
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
/// Git paths to bind read-write alongside a writable root, resolved once at
/// construction time — when the root is still Aria-prepared and untouched by
/// the coder — and frozen for the whole attempt via the host-side provider
/// session cache (F-17 fix round 2/4): re-resolving per terminal command, or
/// per retry/rework round, would let a sandboxed coder rewrite the `.git`
/// pointer (or `<gitdir>/gitdir`/`commondir`) and aim the extra rw binds at
/// an arbitrary valid host git dir.
fn writable_git_paths_for(role: &AdapterRole, root: &Path) -> Vec<PathBuf> {
    if matches!(role, AdapterRole::Executor) {
        frozen_writable_git_paths(root)
    } else {
        Vec::new()
    }
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
            None,
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

    #[cfg(unix)]
    #[tokio::test]
    async fn kimi_038_terminal_flow_serves_output_as_request_response() {
        if probe_bwrap().is_none() {
            // Auto mode requires bubblewrap for terminal execution; skip the
            // happy path where no sandbox is installed.
            return;
        }
        let dir = tempfile::tempdir().expect("dir");
        let (client, server) = tokio::io::duplex(64 * 1024);
        let (reader, writer) = tokio::io::split(client);
        let peer = JsonRpcPeer::new(reader, writer);
        let (event_tx, _events) = mpsc::channel(32);
        let dispatcher = KimiClientServiceDispatcher::new(
            peer,
            "executor-session".to_string(),
            dir.path().to_path_buf(),
            AdapterRole::Executor,
            ProviderPermissionMode::Auto,
            Arc::new(ApprovalBridge::new(
                ProviderPermissionMode::Auto,
                event_tx.clone(),
            )),
            event_tx,
            CancellationToken::new(),
            None,
        );
        let (server_reader, mut server_writer) = tokio::io::split(server);
        let mut server_reader = tokio::io::BufReader::new(server_reader);

        // kimi 0.38.0 create shape: interpreter path + args (captured live).
        let create = json!({
            "sessionId": "executor-session",
            "command": "/bin/bash",
            "args": ["-c", "printf probe-output"],
            "cwd": "",
            "outputByteLimit": 4194304
        });
        assert!(dispatcher.dispatch("terminal/create", json!(10), create));
        let reply = read_reply(&mut server_reader, 10).await;
        let terminal_id = reply["result"]["terminalId"]
            .as_str()
            .expect("terminalId")
            .to_string();

        assert!(dispatcher.dispatch(
            "terminal/wait_for_exit",
            json!(11),
            json!({"sessionId": "executor-session", "terminalId": terminal_id})
        ));
        let reply = read_reply(&mut server_reader, 11).await;
        assert_eq!(reply["result"]["exitCode"], json!(0), "{reply}");

        // terminal/output is a request with an id; the result carries the
        // retained output (raw ACP capture, kimi 0.38.0).
        assert!(dispatcher.dispatch(
            "terminal/output",
            json!(12),
            json!({"sessionId": "executor-session", "terminalId": terminal_id})
        ));
        let reply = read_reply(&mut server_reader, 12).await;
        assert_eq!(reply["result"]["output"], json!("probe-output"), "{reply}");

        // Unknown terminals still fail closed with the dedicated code.
        assert!(dispatcher.dispatch(
            "terminal/output",
            json!(13),
            json!({"sessionId": "executor-session", "terminalId": "term-nope"})
        ));
        let reply = read_reply(&mut server_reader, 13).await;
        assert_eq!(
            reply["error"]["code"],
            json!(ERROR_UNKNOWN_TERMINAL),
            "{reply}"
        );

        let _ = server_writer.shutdown().await;
        drop(dispatcher);
    }

    async fn read_reply(
        server_reader: &mut tokio::io::BufReader<tokio::io::ReadHalf<tokio::io::DuplexStream>>,
        id: i64,
    ) -> Value {
        let mut line = String::new();
        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                line.clear();
                server_reader
                    .read_line(&mut line)
                    .await
                    .expect("read reply");
                if let Ok(value) = serde_json::from_str::<Value>(line.trim())
                    && value.get("id") == Some(&json!(id))
                {
                    return value;
                }
            }
        })
        .await
        .expect("reply within timeout")
    }

    /// F-57 现场形态复现（REQ-PIB-02 场景 1/2）：基线会话（baseline_tree=
    /// Some）的 fs 读必须返回基线树内容而非工作区检出——同路径未提交污染
    /// 不可见、`.worktrees/` 兄弟件不可达；terminal 一律拒绝（含 Auto 档）。
    #[tokio::test]
    async fn baseline_session_reads_tree_content_and_denies_terminal() {
        let repo = tempfile::tempdir().expect("repo dir");
        let run = |args: &[&str]| {
            let status = std::process::Command::new("git")
                .args(args)
                .current_dir(repo.path())
                .stdin(std::process::Stdio::null())
                .status()
                .expect("git fixture command");
            assert!(status.success(), "git {args:?}");
        };
        run(&["init", "-b", "main"]);
        run(&["config", "user.email", "test@example.com"]);
        run(&["config", "user.name", "Test User"]);
        std::fs::write(repo.path().join("status.html"), "baseline-content\n")
            .expect("write baseline file");
        run(&["add", "status.html"]);
        run(&["commit", "-m", "baseline"]);
        // 工作区脏值（未提交污染）：若读工作区检出会返回 dirty-content。
        std::fs::write(repo.path().join("status.html"), "dirty-content\n")
            .expect("dirty working tree file");
        // 兄弟 worktree 独有文件：磁盘上存在、main 树内不存在（F-57 现场）。
        run(&[
            "worktree",
            "add",
            ".worktrees/aria-issues/issue_0001",
            "-b",
            "sibling",
        ]);
        std::fs::write(
            repo.path()
                .join(".worktrees/aria-issues/issue_0001/sibling-only.txt"),
            "sibling\n",
        )
        .expect("sibling worktree file");

        let (client, server) = tokio::io::duplex(16 * 1024);
        let (reader, writer) = tokio::io::split(client);
        let peer = JsonRpcPeer::new(reader, writer);
        let (event_tx, _events) = mpsc::channel(32);
        let dispatcher = KimiClientServiceDispatcher::new(
            peer,
            "author-session".to_string(),
            repo.path().to_path_buf(),
            AdapterRole::Orchestrator,
            ProviderPermissionMode::Auto,
            Arc::new(ApprovalBridge::new(
                ProviderPermissionMode::Auto,
                event_tx.clone(),
            )),
            event_tx,
            CancellationToken::new(),
            Some(BaselineTreeRef {
                repo_path: repo.path().to_path_buf(),
                branch: "main".to_string(),
            }),
        );
        let (server_reader, mut server_writer) = tokio::io::split(server);
        let mut server_reader = tokio::io::BufReader::new(server_reader);

        // 场景 1（所见=基线）：基线内路径 → main 树内容，≠工作区脏值。
        assert!(dispatcher.dispatch(
            "fs/read_text_file",
            json!(0),
            json!({"sessionId": "author-session", "path": "status.html"})
        ));
        let reply = read_reply(&mut server_reader, 0).await;
        assert_eq!(
            reply["result"]["content"],
            json!("baseline-content\n"),
            "{reply}"
        );

        // 场景 2（看不到兄弟 worktree）：磁盘存在、树内不存在 → 找不到。
        assert!(dispatcher.dispatch(
            "fs/read_text_file",
            json!(1),
            json!({
                "sessionId": "author-session",
                "path": ".worktrees/aria-issues/issue_0001/sibling-only.txt"
            })
        ));
        let reply = read_reply(&mut server_reader, 1).await;
        assert_eq!(reply["error"]["code"], json!(ERROR_FS), "{reply}");

        // 场景 3（收不住的通道拒绝）：基线会话 terminal 一律拒绝（Auto 档
        // 本应 Allow——bwrap 宿主只读可见是读泄漏面）。
        assert!(dispatcher.dispatch(
            "terminal/create",
            json!(2),
            json!({
                "sessionId": "author-session",
                "command": "git status",
                "cwd": ""
            })
        ));
        let reply = read_reply(&mut server_reader, 2).await;
        assert_eq!(reply["error"]["code"], json!(ERROR_REJECTED), "{reply}");
        assert!(
            reply["error"]["message"]
                .as_str()
                .unwrap_or_default()
                .contains("baseline"),
            "{reply}"
        );

        let _ = server_writer.shutdown().await;
        drop(dispatcher);
    }

    /// F-58 现场形态复现（issue_0002 story session_0007）：kimi 原生 Read 在
    /// ACP 模式下经 `fs/read_text_file` 委托 host（capabilities
    /// `fs.readTextFile`），且总发送按其 cwd 解析后的**绝对路径**——`AGENTS.md`
    /// 以 `/…/<repo>/AGENTS.md` 形态到达，基线路由修复前一律被
    /// `validate_tree_relative_path` 以对路径拒绝 → author 判定环境阻塞、拒写
    /// spec。修复后：根内绝对路径归一为树内相对读基线树；根外绝对路径与
    /// `.worktrees` 兄弟件仍不可达。
    #[tokio::test]
    async fn baseline_session_native_read_absolute_path_routes_tree() {
        let repo = tempfile::tempdir().expect("repo dir");
        let run = |args: &[&str]| {
            let status = std::process::Command::new("git")
                .args(args)
                .current_dir(repo.path())
                .stdin(std::process::Stdio::null())
                .status()
                .expect("git fixture command");
            assert!(status.success(), "git {args:?}");
        };
        run(&["init", "-b", "main"]);
        run(&["config", "user.email", "test@example.com"]);
        run(&["config", "user.name", "Test User"]);
        std::fs::write(repo.path().join("AGENTS.md"), "baseline-agents\n")
            .expect("write baseline AGENTS.md");
        std::fs::write(repo.path().join("CLAUDE.md"), "baseline-claude\n")
            .expect("write baseline CLAUDE.md");
        run(&["add", "AGENTS.md", "CLAUDE.md"]);
        run(&["commit", "-m", "baseline"]);
        // 工作区脏值：若读工作区检出会返回 workspace-*。
        std::fs::write(repo.path().join("AGENTS.md"), "workspace-agents\n")
            .expect("dirty AGENTS.md");
        std::fs::write(repo.path().join("CLAUDE.md"), "workspace-claude\n")
            .expect("dirty CLAUDE.md");
        // 兄弟 worktree 独有文件：磁盘存在、main 树内不存在。
        run(&[
            "worktree",
            "add",
            ".worktrees/aria-issues/issue_0001",
            "-b",
            "sibling",
        ]);
        std::fs::write(
            repo.path()
                .join(".worktrees/aria-issues/issue_0001/sibling-only.txt"),
            "sibling\n",
        )
        .expect("sibling worktree file");

        let (client, server) = tokio::io::duplex(16 * 1024);
        let (reader, writer) = tokio::io::split(client);
        let peer = JsonRpcPeer::new(reader, writer);
        let (event_tx, _events) = mpsc::channel(32);
        let dispatcher = KimiClientServiceDispatcher::new(
            peer,
            "author-session".to_string(),
            repo.path().to_path_buf(),
            AdapterRole::Orchestrator,
            ProviderPermissionMode::Auto,
            Arc::new(ApprovalBridge::new(
                ProviderPermissionMode::Auto,
                event_tx.clone(),
            )),
            event_tx,
            CancellationToken::new(),
            Some(BaselineTreeRef {
                repo_path: repo.path().to_path_buf(),
                branch: "main".to_string(),
            }),
        );
        let (server_reader, mut server_writer) = tokio::io::split(server);
        let mut server_reader = tokio::io::BufReader::new(server_reader);

        // 场景 1（现场形态）：绝对路径读 AGENTS.md/CLAUDE.md → 基线树内容。
        assert!(dispatcher.dispatch(
            "fs/read_text_file",
            json!(0),
            json!({
                "sessionId": "author-session",
                "path": repo.path().join("AGENTS.md").to_string_lossy()
            })
        ));
        let reply = read_reply(&mut server_reader, 0).await;
        assert_eq!(
            reply["result"]["content"],
            json!("baseline-agents\n"),
            "{reply}"
        );

        assert!(dispatcher.dispatch(
            "fs/read_text_file",
            json!(1),
            json!({
                "sessionId": "author-session",
                "path": repo.path().join("CLAUDE.md").to_string_lossy()
            })
        ));
        let reply = read_reply(&mut server_reader, 1).await;
        assert_eq!(
            reply["result"]["content"],
            json!("baseline-claude\n"),
            "{reply}"
        );

        // 场景 2（根外绝对路径不可达）：现场干扰项 /tmp/gomoku.pid。
        assert!(dispatcher.dispatch(
            "fs/read_text_file",
            json!(2),
            json!({"sessionId": "author-session", "path": "/tmp/gomoku.pid"})
        ));
        let reply = read_reply(&mut server_reader, 2).await;
        assert_eq!(reply["error"]["code"], json!(ERROR_FS), "{reply}");

        // 场景 3（兄弟 worktree 根内词法、树外）：NotFound，内容不可达。
        assert!(dispatcher.dispatch(
            "fs/read_text_file",
            json!(3),
            json!({
                "sessionId": "author-session",
                "path": repo
                    .path()
                    .join(".worktrees/aria-issues/issue_0001/sibling-only.txt")
                    .to_string_lossy()
            })
        ));
        let reply = read_reply(&mut server_reader, 3).await;
        assert_eq!(reply["error"]["code"], json!(ERROR_FS), "{reply}");
        assert!(
            reply["error"]["message"]
                .as_str()
                .unwrap_or_default()
                .contains("baseline tree"),
            "{reply}"
        );

        // 场景 4（回归护栏）：相对路径基线读取不回退。
        assert!(dispatcher.dispatch(
            "fs/read_text_file",
            json!(4),
            json!({"sessionId": "author-session", "path": "AGENTS.md"})
        ));
        let reply = read_reply(&mut server_reader, 4).await;
        assert_eq!(
            reply["result"]["content"],
            json!("baseline-agents\n"),
            "{reply}"
        );

        let _ = server_writer.shutdown().await;
        drop(dispatcher);
    }
}
