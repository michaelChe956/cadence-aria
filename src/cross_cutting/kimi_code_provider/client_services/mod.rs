//! `KimiClientServiceDispatcher`: concurrent, non-blocking dispatch of
//! server→client `terminal/*` and `fs/*` requests from the kimi ACP server.
//!
//! Every request is recognized on the JsonRpcPeer read loop and immediately
//! handed to a spawned task, so a pending `terminal/wait_for_exit` never
//! blocks the main prompt queue and `kill`/`release`/session cancel stay
//! concurrent with it.

mod fs_handlers;
mod fs_service;
mod policy;
mod sandbox;
mod terminal;
mod terminal_handlers;

use std::path::PathBuf;
use std::sync::Arc;

use crate::cross_cutting::provider_boundary::ProviderBoundaryPlan;

use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::approval_bridge::ApprovalBridge;
use crate::cross_cutting::json_rpc_peer::JsonRpcPeer;
use crate::cross_cutting::streaming_provider::{BaselineTreeRef, ProviderEvent, RiskLevel};
use crate::protocol::contracts::AdapterRole;

use self::fs_handlers::{handle_fs_read, handle_fs_write};
use self::fs_service::FsError;
// 批2 Task 8:受信岗位授予(T2 冻结执行合同消费面)经 dispatcher 注入;
// policy 面 pub(crate) re-export 供三处测试(policy/mod/tool_policy)与
// T10/T20 接线共用同一入口。
#[cfg(test)]
pub(crate) use self::policy::test_fixtures;
pub(crate) use self::policy::{
    ClientAction, ClientServicePolicy, PolicyDecision, TrustedDutyGrant,
};
use self::sandbox::canonicalize_root;
use self::terminal::{TerminalError, TerminalManager};
// 批2 Task 7（Bug45）：会话（kimi_code_provider::session）注入同一
// manager 实例并读真实活动 registry——同源状态，不另造第二份。
pub(crate) use self::terminal::TerminalManager as SessionTerminalManager;
use self::terminal_handlers::{
    handle_terminal_create, handle_terminal_kill, handle_terminal_output, handle_terminal_release,
    handle_terminal_wait,
};

/// Adapter-local constant: kimi's boolean client-capability dialect. It never
/// leaks into the generic ACP layer.
///
/// 批2 Task 8:岗位感知广告(`ClientServicePolicy::advertised_capabilities`,
/// Author 不广告代码编辑 fs 写)与本函数共用同一布尔方言形状;会话
/// initialize 接线归 T10/T20,当前直连会话保持全集(广告≠授权,请求仍
/// 逐条过 policy gate)。
#[cfg(test)] // 仅本模块单测(布尔方言断言)消费;生产走 advertised_client_capabilities。
pub fn kimi_client_capabilities() -> Value {
    json!({
        "fs": {"readTextFile": true, "writeTextFile": true},
        "terminal": true
    })
}

/// 批2 Task 20b:kimi 布尔方言广告面的岗位感知单源——Author 职责不
/// 广告代码编辑 fs 写(AuthorToolPolicy v1 `builtin_write_code_file`
/// 语义);其余职责(含 Reviewer 窗口合同)保持全集(广告≠授权,请求
/// 仍逐条过 policy gate)。会话 initialize 接线消费本函数;`None` =
/// 无受信授予的直连会话(全集,行为不变)。
pub(crate) fn advertised_client_capabilities(
    duty: Option<crate::cross_cutting::provider_execution::TrustedProviderRole>,
) -> Value {
    let fs_write_advertised = !matches!(
        duty,
        Some(crate::cross_cutting::provider_execution::TrustedProviderRole::Author)
    );
    json!({
        "fs": {"readTextFile": true, "writeTextFile": fs_write_advertised},
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
    bridge: Arc<ApprovalBridge>,
    event_tx: mpsc::Sender<ProviderEvent>,
    terminal: TerminalManager,
    /// 冻结会话环境(批2 Task 6):与 provider 进程 spawn 消费的同一 env
    /// map——T3 白名单环境经 `StreamingProviderInput::env_vars` 传输注入,
    /// dispatcher 与主会话绑定同一个冻结合同,不能根据 ACP 参数造
    /// profile。terminal 子进程 `env_clear` 后只注入该 map(裸名 program
    /// 经其 PATH 解析;legacy 形态的受信 SHELL 取其 `SHELL`)。
    session_env: std::collections::BTreeMap<String, String>,
    /// REQ-PIB-02：基线会话锚点。`Some` 时 fs 读路由基线树、terminal 一律
    /// 拒绝（见 `evaluate_policy` 前置与 `fs_handlers`）；`None` 行为不变。
    baseline_tree: Option<BaselineTreeRef>,
    /// LC target 写边界(Task 4c):`Some` 时宿主 fs read/write 与 terminal
    /// 消费同一不可伪造 boundary plan——fs 写唯一锚定 target root;terminal
    /// 的 cwd 锚定面取 plan 的 target root。批2 Task 6:plan 只决定 fs 写面
    /// 与 terminal cwd 锚定面,不再是物理执行边界(Host 单路径)。
    /// `None`(direct 会话)行为不变,原 24 格 ClientServicePolicy 决策表
    /// 保持原值。
    target_boundary: Option<ProviderBoundaryPlan>,
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
    Terminal(TerminalError),
    Fs(FsError),
    Internal(String),
}

impl ClientServiceError {
    fn code(&self) -> i64 {
        match self {
            ClientServiceError::Rejected(_) => ERROR_REJECTED,
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
    /// 批2 Task 8:直连形态构造(无受信授予,行为=承载角色映射职责矩阵)。
    #[allow(dead_code)] // 生产路径统一走 new_with_trusted_duty;此直连入口供矩阵 harness。
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        peer: JsonRpcPeer<W>,
        session_id: String,
        working_dir: PathBuf,
        role: AdapterRole,
        permission_mode: crate::cross_cutting::streaming_provider::ProviderPermissionMode,
        session_env: std::collections::BTreeMap<String, String>,
        bridge: Arc<ApprovalBridge>,
        event_tx: mpsc::Sender<ProviderEvent>,
        cancel: CancellationToken,
        terminal: TerminalManager,
        baseline_tree: Option<BaselineTreeRef>,
        target_boundary: Option<ProviderBoundaryPlan>,
    ) -> Self {
        Self::new_with_trusted_duty(
            peer,
            session_id,
            working_dir,
            role,
            permission_mode,
            session_env,
            bridge,
            event_tx,
            cancel,
            terminal,
            baseline_tree,
            target_boundary,
            None,
        )
    }

    /// 12+1 个参数均为构造期一次性注入的运行时依赖(peer/session/
    /// working_dir/role/permission_mode/session_env/bridge/event_tx/cancel/
    /// terminal/baseline_tree/target_boundary/trusted_duty),无自然聚合
    /// 语义且仅在构造使用一次,重构为 struct 会引入额外间接层,故允许
    /// clippy::too_many_arguments。
    /// 批2 Task 7(Bug45):`terminal` 由会话注入(会话 stall 判定与产品
    /// 双层看门狗读同一真实活动 registry,同源不另造状态)。
    /// 批2 Task 8:`trusted_duty` 为受信岗位授予(T2 冻结执行合同消费
    /// 面;`None` 等价 [`Self::new`] 直连形态;生产注入归 T10/T20,测试
    /// 注入可信 fixture)。
    #[allow(clippy::too_many_arguments)]
    pub fn new_with_trusted_duty(
        peer: JsonRpcPeer<W>,
        session_id: String,
        working_dir: PathBuf,
        role: AdapterRole,
        permission_mode: crate::cross_cutting::streaming_provider::ProviderPermissionMode,
        session_env: std::collections::BTreeMap<String, String>,
        bridge: Arc<ApprovalBridge>,
        event_tx: mpsc::Sender<ProviderEvent>,
        cancel: CancellationToken,
        terminal: TerminalManager,
        baseline_tree: Option<BaselineTreeRef>,
        target_boundary: Option<ProviderBoundaryPlan>,
        trusted_duty: Option<TrustedDutyGrant>,
    ) -> Self {
        let root = canonicalize_root(&working_dir).unwrap_or(working_dir);
        // Task 4c 一致性早门:LC plan 的进程 cwd 必须即会话根(plan 与 root
        // 同源于 start_validated 的 envelope;不一致说明装配链被伪造,
        // fail-closed 拒绝构造)。
        if let Some(plan) = target_boundary.as_ref()
            && plan.working_directory() != root
        {
            // 无自然错误通道(dispatcher 构造无 Result):退化为无 plan 的
            // direct 形态会被写成「host 全可写」,绝不可接受——直接 panic
            // fail-closed(装配 bug 属程序错误,不是可运行状态)。
            panic!(
                "lc target boundary plan working directory {} disagrees with the session root {}",
                plan.working_directory().display(),
                root.display()
            );
        }
        // Command output is served through `terminal/output`
        // request/response (see `handle_terminal_output`); kimi 0.38.0
        // ignores unsolicited `terminal/output` notifications.

        let policy = ClientServicePolicy::new(role, permission_mode);
        let policy = match trusted_duty {
            Some(grant) => policy.with_trusted_duty(grant),
            None => policy,
        };
        let state = Arc::new(ClientServiceState {
            session_id,
            root,
            policy,
            session_env,
            bridge,
            event_tx,
            terminal,
            target_boundary,
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
    // REQ-PIB-02：基线会话 terminal 一律拒绝——Host terminal 读到的是工作区
    // 检出而非基线树（F-57 现场通道：未提交污染与 `.worktrees/` 兄弟件对
    // terminal 可见），命令语法收窄也不足以绑定基线 ref。前置判定独立于
    // policy.rs 决策矩阵（24 格冻结，零改动）。
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

    /// Host 执行态测试环境:PATH 供裸名工具/脚本内命令解析;SHELL 钉
    /// /bin/sh(legacy 形态确定性)。
    fn host_env() -> std::collections::BTreeMap<String, String> {
        let mut env = std::collections::BTreeMap::new();
        if let Some(path) = std::env::var_os("PATH") {
            env.insert("PATH".to_string(), path.to_string_lossy().into_owned());
        }
        env.insert("SHELL".to_string(), "/bin/sh".to_string());
        env
    }

    /// 批2 Task 8 反转:Reviewer 直连会话 terminal 正向执行(Host 单路径
    /// 物理可达是正确观察);FsWrite 无有效窗口引用拒绝(可区分文案);
    /// FsRead 仍锚定授权根(越界拒绝——fs API 拒绝越界同样是正确观察,
    /// 两种观察互不否定)。
    #[cfg(unix)]
    #[tokio::test]
    async fn reviewer_wire_session_terminal_positive_and_fs_write_denied_without_window() {
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
            host_env(),
            Arc::new(ApprovalBridge::new(
                ProviderPermissionMode::Auto,
                event_tx.clone(),
            )),
            event_tx,
            CancellationToken::new(),
            TerminalManager::new(),
            None,
            None,
        );
        let (server_reader, mut server_writer) = tokio::io::split(server);
        let mut server_reader = tokio::io::BufReader::new(server_reader);

        // 1) terminal 正向:真实 Host 子进程执行成功(非拒绝)。
        assert!(dispatcher.dispatch(
            "terminal/create",
            json!(0),
            json!({
                "sessionId": "reviewer-session",
                "command": "/bin/sh",
                "args": ["-c", "printf reviewer-terminal-ok"],
                "cwd": "",
                "outputByteLimit": 4194304
            })
        ));
        let reply = read_reply(&mut server_reader, 0).await;
        let terminal_id = reply["result"]["terminalId"]
            .as_str()
            .expect("reviewer terminal must be positively allowed")
            .to_string();
        assert!(dispatcher.dispatch(
            "terminal/wait_for_exit",
            json!(1),
            json!({"sessionId": "reviewer-session", "terminalId": terminal_id})
        ));
        let reply = read_reply(&mut server_reader, 1).await;
        assert_eq!(reply["result"]["exitCode"], json!(0), "{reply}");

        // 2) FsWrite 无窗拒绝:稳定可区分文案(非旧「角色只读」整段拒绝)。
        assert!(dispatcher.dispatch(
            "fs/write_text_file",
            json!(2),
            json!({
                "sessionId": "reviewer-session",
                "path": "notes.txt",
                "content": "x"
            })
        ));
        let reply = read_reply(&mut server_reader, 2).await;
        assert_eq!(reply["error"]["code"], json!(ERROR_REJECTED), "{reply}");
        assert!(
            reply["error"]["message"]
                .as_str()
                .expect("message")
                .contains("review window"),
            "{reply}"
        );
        assert!(!dir.path().join("notes.txt").exists());

        // 3) FsRead 越界仍拒绝:授权根/no-follow 合同独立有效。
        assert!(dispatcher.dispatch(
            "fs/read_text_file",
            json!(3),
            json!({"sessionId": "reviewer-session", "path": "../escape.txt"})
        ));
        let reply = read_reply(&mut server_reader, 3).await;
        assert_eq!(reply["error"]["code"], json!(ERROR_FS), "{reply}");

        let _ = server_writer.shutdown().await;
        drop(dispatcher);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn kimi_038_terminal_flow_serves_output_as_request_response() {
        // 批2 Task 6:Host 单路径无 bwrap 依赖,不再按 bwrap 存在性跳过。
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
            host_env(),
            Arc::new(ApprovalBridge::new(
                ProviderPermissionMode::Auto,
                event_tx.clone(),
            )),
            event_tx,
            CancellationToken::new(),
            TerminalManager::new(),
            None,
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
            host_env(),
            Arc::new(ApprovalBridge::new(
                ProviderPermissionMode::Auto,
                event_tx.clone(),
            )),
            event_tx,
            CancellationToken::new(),
            TerminalManager::new(),
            Some(BaselineTreeRef {
                repo_path: repo.path().to_path_buf(),
                branch: "main".to_string(),
            }),
            None,
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
            host_env(),
            Arc::new(ApprovalBridge::new(
                ProviderPermissionMode::Auto,
                event_tx.clone(),
            )),
            event_tx,
            CancellationToken::new(),
            TerminalManager::new(),
            Some(BaselineTreeRef {
                repo_path: repo.path().to_path_buf(),
                branch: "main".to_string(),
            }),
            None,
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

    /// Task 4c(`lcg_t04_kimi_host_fs_and_terminal_share_target_boundary`):
    /// LC 会话的宿主 fs read/write 与 terminal 消费同一不可伪造 target
    /// boundary plan——fs 读锚定 host root;fs 写唯一锚定 plan 的 target
    /// root;terminal 实际 cwd 可为 target,不改变宿主读根(provider 进程
    /// cwd 由 start_validated 冻结,另由启动测试钉住)。批2 Task 6:terminal
    /// 在 Host 单路径执行,host 物理可达(无 bwrap 只读面)。
    #[cfg(unix)]
    #[tokio::test]
    async fn lcg_t04_kimi_host_fs_and_terminal_share_target_boundary() {
        // 批2 Task 6:Host 单路径无 bwrap 依赖(bwrap 只读面断言已按
        // Host 语义反转)。
        use crate::cross_cutting::provider_boundary::{ProviderBoundaryMode, ProviderBoundaryPlan};

        let dir = tempfile::tempdir().expect("lc root dir");
        let host_root = canonicalize_root(dir.path()).expect("canonical host root");
        let target = host_root.join("member-worktree");
        std::fs::create_dir_all(&target).expect("create target worktree");
        std::fs::write(host_root.join("host-note.txt"), "host-read-only-face\n")
            .expect("seed host note");

        let plan = ProviderBoundaryPlan::new(
            ProviderBoundaryMode::TargetWriteOnly,
            host_root.clone(),
            Some(target.clone()),
            Vec::new(),
        );
        let (client, server) = tokio::io::duplex(64 * 1024);
        let (reader, writer) = tokio::io::split(client);
        let peer = JsonRpcPeer::new(reader, writer);
        let (event_tx, _events) = mpsc::channel(32);
        let dispatcher = KimiClientServiceDispatcher::new(
            peer,
            "executor-lc".to_string(),
            host_root.clone(),
            AdapterRole::Executor,
            ProviderPermissionMode::Auto,
            host_env(),
            Arc::new(ApprovalBridge::new(
                ProviderPermissionMode::Auto,
                event_tx.clone(),
            )),
            event_tx,
            CancellationToken::new(),
            TerminalManager::new(),
            None,
            Some(plan),
        );
        let (server_reader, mut server_writer) = tokio::io::split(server);
        let mut server_reader = tokio::io::BufReader::new(server_reader);

        // 1) fs 读:host root 只读面不变(plan 的进程 cwd = 会话根)。
        assert!(dispatcher.dispatch(
            "fs/read_text_file",
            json!(20),
            json!({"sessionId": "executor-lc", "path": "host-note.txt"})
        ));
        let reply = read_reply(&mut server_reader, 20).await;
        assert_eq!(
            reply["result"]["content"],
            json!("host-read-only-face\n"),
            "{reply}"
        );

        // 2) fs 写:target 内允许(相对路径锚定 plan target)。
        assert!(dispatcher.dispatch(
            "fs/write_text_file",
            json!(21),
            json!({
                "sessionId": "executor-lc",
                "path": "target-note.txt",
                "content": "written inside target"
            })
        ));
        let reply = read_reply(&mut server_reader, 21).await;
        assert!(reply.get("result").is_some(), "{reply}");
        assert_eq!(
            std::fs::read_to_string(target.join("target-note.txt"))
                .expect("target write landed inside the target root"),
            "written inside target"
        );

        // 3) fs 写:target 外(host root 内)必须拒绝,文件不落盘。
        let host_escape = host_root.join("host-escape.txt");
        assert!(dispatcher.dispatch(
            "fs/write_text_file",
            json!(22),
            json!({
                "sessionId": "executor-lc",
                "path": host_escape.to_string_lossy().as_ref(),
                "content": "must not land"
            })
        ));
        let reply = read_reply(&mut server_reader, 22).await;
        assert_eq!(reply["error"]["code"], json!(ERROR_FS), "{reply}");
        assert!(!host_escape.exists());

        // 4) terminal:同一 plan 的 target 写面——target 内写允许。
        assert!(dispatcher.dispatch(
            "terminal/create",
            json!(23),
            json!({
                "sessionId": "executor-lc",
                "command": "/bin/bash",
                "args": ["-c", "echo terminal-ok > terminal-inside.txt"],
                "cwd": target.to_string_lossy().as_ref(),
                "outputByteLimit": 4194304
            })
        ));
        let reply = read_reply(&mut server_reader, 23).await;
        let terminal_id = reply["result"]["terminalId"]
            .as_str()
            .expect("terminalId")
            .to_string();
        assert!(dispatcher.dispatch(
            "terminal/wait_for_exit",
            json!(24),
            json!({"sessionId": "executor-lc", "terminalId": terminal_id})
        ));
        let reply = read_reply(&mut server_reader, 24).await;
        assert_eq!(reply["result"]["exitCode"], json!(0), "{reply}");
        assert!(target.join("terminal-inside.txt").exists());

        // 5) 批2 Task 6 反转:Host 单路径下 terminal 无 bwrap 只读面——
        //    host root 物理可达且写入落盘(执行隔离已删,fs API 的 target
        //    锚定仍由上方 fs 写断言钉住;terminal cwd 不改读根/进程 cwd)。
        let host_write = host_root.join("host-write.txt");
        assert!(dispatcher.dispatch(
            "terminal/create",
            json!(25),
            json!({
                "sessionId": "executor-lc",
                "command": "/bin/bash",
                "args": [
                    "-c",
                    &format!("echo host > {}", host_write.to_string_lossy())
                ],
                "cwd": target.to_string_lossy().as_ref(),
                "outputByteLimit": 4194304
            })
        ));
        let reply = read_reply(&mut server_reader, 25).await;
        let host_terminal = reply["result"]["terminalId"]
            .as_str()
            .expect("terminalId")
            .to_string();
        assert!(dispatcher.dispatch(
            "terminal/wait_for_exit",
            json!(26),
            json!({"sessionId": "executor-lc", "terminalId": host_terminal})
        ));
        let reply = read_reply(&mut server_reader, 26).await;
        assert_eq!(
            reply["result"]["exitCode"],
            json!(0),
            "host terminal write must succeed on the host path: {reply}"
        );
        assert!(
            host_write.exists(),
            "host single path: terminal writes land at real host paths"
        );

        let _ = server_writer.shutdown().await;
        drop(dispatcher);
    }

    /// P3-b 聚合 fixture(/tmp 下两成员仓):LC 会话的 terminal Read 真实
    /// 上游文件、native Read(fs/read_text_file)同面、target 写成功;
    /// fs 写唯一锚定 target(上游 fs 写拒绝且不落盘)。批2 Task 6:terminal
    /// 为 Host 单路径,上游 terminal 写物理可达(EROFS 断言已反转)。
    #[cfg(unix)]
    #[tokio::test]
    async fn lc_terminal_reads_real_upstream_under_tmp_aggregate_root() {
        // 批2 Task 6:Host 单路径无 bwrap 依赖(上游 EROFS 断言已按 Host
        // 语义反转)。
        use crate::cross_cutting::provider_boundary::{ProviderBoundaryMode, ProviderBoundaryPlan};

        // 遮蔽 bug 只在 /tmp 子路径显形:聚合 fixture 必须钉在 /tmp 下。
        let fixture = tempfile::Builder::new()
            .prefix("p3b-lc-agg-")
            .tempdir_in("/tmp")
            .expect("aggregate fixture under /tmp");
        let aggregate = canonicalize_root(fixture.path()).expect("canonical aggregate root");
        assert!(
            aggregate.starts_with("/tmp"),
            "fixture must live under /tmp to expose the tmpfs shadowing"
        );
        let upstream = aggregate.join("upstream");
        let target = aggregate.join("downstream");
        std::fs::create_dir_all(upstream.join("src")).expect("mkdir upstream member");
        std::fs::create_dir_all(&target).expect("mkdir downstream member");
        std::fs::write(
            upstream.join("src/lib.rs"),
            "pub fn upstream_truth() -> u32 { 42 }\n",
        )
        .expect("seed upstream source");

        let plan = ProviderBoundaryPlan::new(
            ProviderBoundaryMode::TargetWriteOnly,
            aggregate.clone(),
            Some(target.clone()),
            Vec::new(),
        );
        let (client, server) = tokio::io::duplex(64 * 1024);
        let (reader, writer) = tokio::io::split(client);
        let peer = JsonRpcPeer::new(reader, writer);
        let (event_tx, _events) = mpsc::channel(32);
        let dispatcher = KimiClientServiceDispatcher::new(
            peer,
            "executor-lc-p3b".to_string(),
            aggregate.clone(),
            AdapterRole::Executor,
            ProviderPermissionMode::Auto,
            host_env(),
            Arc::new(ApprovalBridge::new(
                ProviderPermissionMode::Auto,
                event_tx.clone(),
            )),
            event_tx,
            CancellationToken::new(),
            TerminalManager::new(),
            None,
            Some(plan),
        );
        let (server_reader, mut server_writer) = tokio::io::split(server);
        let mut server_reader = tokio::io::BufReader::new(server_reader);

        async fn run_terminal(
            dispatcher: &KimiClientServiceDispatcher<tokio::io::WriteHalf<tokio::io::DuplexStream>>,
            server_reader: &mut tokio::io::BufReader<tokio::io::ReadHalf<tokio::io::DuplexStream>>,
            next_id: &mut i64,
            script: &str,
            cwd: &str,
        ) -> (Value, Value) {
            let create_id = *next_id;
            *next_id += 1;
            assert!(dispatcher.dispatch(
                "terminal/create",
                json!(create_id),
                json!({
                    "sessionId": "executor-lc-p3b",
                    "command": "/bin/bash",
                    "args": ["-c", script],
                    "cwd": cwd,
                    "outputByteLimit": 4194304
                })
            ));
            let reply = read_reply(server_reader, create_id).await;
            let terminal_id = reply["result"]["terminalId"]
                .as_str()
                .expect("terminalId")
                .to_string();
            let wait_id = *next_id;
            *next_id += 1;
            assert!(dispatcher.dispatch(
                "terminal/wait_for_exit",
                json!(wait_id),
                json!({"sessionId": "executor-lc-p3b", "terminalId": terminal_id})
            ));
            let wait_reply = read_reply(server_reader, wait_id).await;
            let output_id = *next_id;
            *next_id += 1;
            assert!(dispatcher.dispatch(
                "terminal/output",
                json!(output_id),
                json!({"sessionId": "executor-lc-p3b", "terminalId": terminal_id})
            ));
            let output_reply = read_reply(server_reader, output_id).await;
            (wait_reply, output_reply)
        }

        // 会话 cwd=聚合根(kimi CLI 不转发模型 cwd 的现场形态),face root
        // 由 resolve_cwd 锚定到 target 成员仓。
        let session_cwd = aggregate.to_string_lossy().into_owned();

        // 1) terminal Read:真实上游文件在原宿主路径可见(遮蔽修复的直接
        //    证据;修复前 /tmp 被私有 tmpfs 盖住,cat 以 ENOENT 失败)。
        let upstream_file = upstream.join("src/lib.rs");
        let (wait_reply, output_reply) = run_terminal(
            &dispatcher,
            &mut server_reader,
            &mut 30,
            &format!("cat {}", upstream_file.to_string_lossy()),
            &session_cwd,
        )
        .await;
        assert_eq!(
            wait_reply["result"]["exitCode"],
            json!(0),
            "terminal must read the real upstream file at its host path: {wait_reply} {output_reply}"
        );
        assert!(
            output_reply["result"]["output"]
                .as_str()
                .unwrap_or_default()
                .contains("upstream_truth"),
            "upstream content must be the real file, not a shadowed tmpfs: {output_reply}"
        );

        // 2) terminal 写 target:成功(face root 锚定 + rw bind 回归)。
        let (wait_reply, output_reply) = run_terminal(
            &dispatcher,
            &mut server_reader,
            &mut 40,
            "echo target-write-ok > note.txt",
            &session_cwd,
        )
        .await;
        assert_eq!(
            wait_reply["result"]["exitCode"],
            json!(0),
            "target write must keep working: {wait_reply} {output_reply}"
        );
        assert_eq!(
            std::fs::read_to_string(target.join("note.txt"))
                .expect("target write landed in the member worktree"),
            "target-write-ok\n"
        );

        // 3) 批2 Task 6 反转:Host 单路径下无 authority EROFS 面——上游
        //    物理可达且写入落盘(native fs 写仍唯一锚定 target,见 5))。
        let upstream_note = upstream.join("host-reach.txt");
        let (wait_reply, output_reply) = run_terminal(
            &dispatcher,
            &mut server_reader,
            &mut 50,
            &format!("echo host-reach > {}", upstream_note.to_string_lossy()),
            &session_cwd,
        )
        .await;
        assert_eq!(
            wait_reply["result"]["exitCode"],
            json!(0),
            "upstream write must succeed on the host path: {wait_reply} {output_reply}"
        );
        assert!(
            upstream_note.exists(),
            "host single path: no read-only mount keeps upstream writes off"
        );

        // 4) native Read 同面:绝对路径锚定聚合根读真实上游(LC 读面回归锁)。
        assert!(dispatcher.dispatch(
            "fs/read_text_file",
            json!(60),
            json!({
                "sessionId": "executor-lc-p3b",
                "path": upstream_file.to_string_lossy().as_ref()
            })
        ));
        let reply = read_reply(&mut server_reader, 60).await;
        assert_eq!(
            reply["result"]["content"],
            json!("pub fn upstream_truth() -> u32 { 42 }\n"),
            "native read must serve the real upstream file: {reply}"
        );

        // 5) native 写上游:唯一写面是 target,上游路径拒绝且不落盘。
        let evil_native = upstream.join("evil-native.txt");
        assert!(dispatcher.dispatch(
            "fs/write_text_file",
            json!(61),
            json!({
                "sessionId": "executor-lc-p3b",
                "path": evil_native.to_string_lossy().as_ref(),
                "content": "must not land"
            })
        ));
        let reply = read_reply(&mut server_reader, 61).await;
        assert_eq!(
            reply["error"]["code"],
            json!(ERROR_FS),
            "native write must stay anchored at the target root: {reply}"
        );
        assert!(!evil_native.exists());

        let _ = server_writer.shutdown().await;
        drop(dispatcher);
    }

    // ==== 批2 Task 8:受信岗位授予(dispatcher 面)====

    use super::test_fixtures::frozen_duty_grant;
    use crate::cross_cutting::provider_execution::TrustedProviderRole;

    /// 批2 Task 8:Reviewer 携带有效窗口引用(受信 fixture,T20 接真窗)
    /// 时授权根内 FsWrite 通过且落盘;越界写拒绝且目标未改;terminal
    /// 正向——「fs API 拒绝越界」与「Host terminal 物理可达」两种观察在
    /// 同一会话内同时成立,互不放松。
    #[cfg(unix)]
    #[tokio::test]
    async fn reviewer_grant_with_window_writes_inside_authorized_root() {
        let dir = tempfile::tempdir().expect("dir");
        let (client, server) = tokio::io::duplex(16 * 1024);
        let (reader, writer) = tokio::io::split(client);
        let peer = JsonRpcPeer::new(reader, writer);
        let (event_tx, _events) = mpsc::channel(32);
        let dispatcher = KimiClientServiceDispatcher::new_with_trusted_duty(
            peer,
            "reviewer-window".to_string(),
            dir.path().to_path_buf(),
            AdapterRole::Reviewer,
            ProviderPermissionMode::Auto,
            host_env(),
            Arc::new(ApprovalBridge::new(
                ProviderPermissionMode::Auto,
                event_tx.clone(),
            )),
            event_tx,
            CancellationToken::new(),
            TerminalManager::new(),
            None,
            None,
            Some(frozen_duty_grant(
                TrustedProviderRole::Reviewer,
                AdapterRole::Reviewer,
                Some("window_t8"),
            )),
        );
        let (server_reader, mut server_writer) = tokio::io::split(server);
        let mut server_reader = tokio::io::BufReader::new(server_reader);

        // 1) 授权根内 FsWrite 通过(窗口化 Reviewer 的正向写)。
        assert!(dispatcher.dispatch(
            "fs/write_text_file",
            json!(0),
            json!({
                "sessionId": "reviewer-window",
                "path": "review-note.txt",
                "content": "windowed write"
            })
        ));
        let reply = read_reply(&mut server_reader, 0).await;
        assert!(reply.get("result").is_some(), "{reply}");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("review-note.txt"))
                .expect("windowed reviewer write lands"),
            "windowed write"
        );

        // 2) 越界写拒绝且目标未改(授权根/no-follow 合同不因窗口放宽)。
        let escape = dir
            .path()
            .parent()
            .expect("root parent")
            .join("t8-escape.txt");
        assert!(dispatcher.dispatch(
            "fs/write_text_file",
            json!(1),
            json!({
                "sessionId": "reviewer-window",
                "path": "../t8-escape.txt",
                "content": "must not land"
            })
        ));
        let reply = read_reply(&mut server_reader, 1).await;
        assert_eq!(reply["error"]["code"], json!(ERROR_FS), "{reply}");
        assert!(!escape.exists(), "escape target must stay unchanged");

        // 3) terminal 正向(Host 单路径物理可达)。
        assert!(dispatcher.dispatch(
            "terminal/create",
            json!(2),
            json!({
                "sessionId": "reviewer-window",
                "command": "/bin/sh",
                "args": ["-c", "printf reviewer-window-terminal-ok"],
                "cwd": "",
                "outputByteLimit": 4194304
            })
        ));
        let reply = read_reply(&mut server_reader, 2).await;
        let terminal_id = reply["result"]["terminalId"]
            .as_str()
            .expect("reviewer terminal must stay positive")
            .to_string();
        assert!(dispatcher.dispatch(
            "terminal/wait_for_exit",
            json!(3),
            json!({"sessionId": "reviewer-window", "terminalId": terminal_id})
        ));
        let reply = read_reply(&mut server_reader, 3).await;
        assert_eq!(reply["result"]["exitCode"], json!(0), "{reply}");

        let _ = server_writer.shutdown().await;
        drop(dispatcher);
    }

    /// 批2 Task 8:Author(Orchestrator 承载)——代码编辑 fs 写请求拒绝
    /// (AuthorToolPolicy v1 `builtin_write_code_file` 语义,不落盘,不按
    /// 扩展名另建过滤);fs 读与 terminal 可用;作者文档走平台通道。
    #[cfg(unix)]
    #[tokio::test]
    async fn author_grant_denies_fs_write_read_and_terminal_usable() {
        let dir = tempfile::tempdir().expect("dir");
        std::fs::write(dir.path().join("brief.md"), "author reads").expect("seed");
        let (client, server) = tokio::io::duplex(16 * 1024);
        let (reader, writer) = tokio::io::split(client);
        let peer = JsonRpcPeer::new(reader, writer);
        let (event_tx, _events) = mpsc::channel(32);
        let dispatcher = KimiClientServiceDispatcher::new_with_trusted_duty(
            peer,
            "author-session".to_string(),
            dir.path().to_path_buf(),
            AdapterRole::Orchestrator,
            ProviderPermissionMode::Auto,
            host_env(),
            Arc::new(ApprovalBridge::new(
                ProviderPermissionMode::Auto,
                event_tx.clone(),
            )),
            event_tx,
            CancellationToken::new(),
            TerminalManager::new(),
            None,
            None,
            Some(frozen_duty_grant(
                TrustedProviderRole::Author,
                AdapterRole::Orchestrator,
                None,
            )),
        );
        let (server_reader, mut server_writer) = tokio::io::split(server);
        let mut server_reader = tokio::io::BufReader::new(server_reader);

        // 1) 代码编辑 fs 写拒绝:author 岗位合同,不落盘。
        assert!(dispatcher.dispatch(
            "fs/write_text_file",
            json!(0),
            json!({
                "sessionId": "author-session",
                "path": "story.md",
                "content": "must not land"
            })
        ));
        let reply = read_reply(&mut server_reader, 0).await;
        assert_eq!(reply["error"]["code"], json!(ERROR_REJECTED), "{reply}");
        assert!(
            reply["error"]["message"]
                .as_str()
                .expect("message")
                .contains("author duty"),
            "{reply}"
        );
        assert!(!dir.path().join("story.md").exists());

        // 2) fs 读可用。
        assert!(dispatcher.dispatch(
            "fs/read_text_file",
            json!(1),
            json!({"sessionId": "author-session", "path": "brief.md"})
        ));
        let reply = read_reply(&mut server_reader, 1).await;
        assert_eq!(reply["result"]["content"], json!("author reads"), "{reply}");

        // 3) terminal 可用(read/terminal/report/document 通道的一部分)。
        assert!(dispatcher.dispatch(
            "terminal/create",
            json!(2),
            json!({
                "sessionId": "author-session",
                "command": "/bin/sh",
                "args": ["-c", "printf author-terminal-ok"],
                "cwd": "",
                "outputByteLimit": 4194304
            })
        ));
        let reply = read_reply(&mut server_reader, 2).await;
        let terminal_id = reply["result"]["terminalId"]
            .as_str()
            .expect("author terminal must stay usable")
            .to_string();
        assert!(dispatcher.dispatch(
            "terminal/wait_for_exit",
            json!(3),
            json!({"sessionId": "author-session", "terminalId": terminal_id})
        ));
        let reply = read_reply(&mut server_reader, 3).await;
        assert_eq!(reply["result"]["exitCode"], json!(0), "{reply}");

        let _ = server_writer.shutdown().await;
        drop(dispatcher);
    }

    /// 批2 Task 8 反转:Splitter/Handoff 直连会话的合法操作不整段拒绝
    /// ——fs 读/terminal 正向;fs 写拒绝(职责级,不落盘)。
    #[cfg(unix)]
    #[tokio::test]
    async fn splitter_and_handoff_wire_sessions_run_legal_operations() {
        for (role, session) in [
            (AdapterRole::WorkItemSplitter, "splitter-session"),
            (AdapterRole::Handoff, "handoff-session"),
        ] {
            let dir = tempfile::tempdir().expect("dir");
            std::fs::write(dir.path().join("item.md"), "split input").expect("seed");
            let (client, server) = tokio::io::duplex(16 * 1024);
            let (reader, writer) = tokio::io::split(client);
            let peer = JsonRpcPeer::new(reader, writer);
            let (event_tx, _events) = mpsc::channel(32);
            let dispatcher = KimiClientServiceDispatcher::new(
                peer,
                session.to_string(),
                dir.path().to_path_buf(),
                role.clone(),
                ProviderPermissionMode::Auto,
                host_env(),
                Arc::new(ApprovalBridge::new(
                    ProviderPermissionMode::Auto,
                    event_tx.clone(),
                )),
                event_tx,
                CancellationToken::new(),
                TerminalManager::new(),
                None,
                None,
            );
            let (server_reader, mut server_writer) = tokio::io::split(server);
            let mut server_reader = tokio::io::BufReader::new(server_reader);

            // 1) fs 读正向。
            assert!(dispatcher.dispatch(
                "fs/read_text_file",
                json!(0),
                json!({"sessionId": session, "path": "item.md"})
            ));
            let reply = read_reply(&mut server_reader, 0).await;
            assert_eq!(
                reply["result"]["content"],
                json!("split input"),
                "{role:?}: {reply}"
            );

            // 2) terminal 正向。
            assert!(dispatcher.dispatch(
                "terminal/create",
                json!(1),
                json!({
                    "sessionId": session,
                    "command": "/bin/sh",
                    "args": ["-c", "printf planning-terminal-ok"],
                    "cwd": "",
                    "outputByteLimit": 4194304
                })
            ));
            let reply = read_reply(&mut server_reader, 1).await;
            let terminal_id = reply["result"]["terminalId"]
                .as_str()
                .unwrap_or_else(|| panic!("{role:?}: terminal must stay legal: {reply}"))
                .to_string();
            assert!(dispatcher.dispatch(
                "terminal/wait_for_exit",
                json!(2),
                json!({"sessionId": session, "terminalId": terminal_id})
            ));
            let reply = read_reply(&mut server_reader, 2).await;
            assert_eq!(reply["result"]["exitCode"], json!(0), "{role:?}: {reply}");

            // 3) fs 写拒绝且不落盘。
            assert!(dispatcher.dispatch(
                "fs/write_text_file",
                json!(3),
                json!({
                    "sessionId": session,
                    "path": "mutation.txt",
                    "content": "must not land"
                })
            ));
            let reply = read_reply(&mut server_reader, 3).await;
            assert_eq!(
                reply["error"]["code"],
                json!(ERROR_REJECTED),
                "{role:?}: {reply}"
            );
            assert!(!dir.path().join("mutation.txt").exists(), "{role:?}");

            let _ = server_writer.shutdown().await;
            drop(dispatcher);
        }
    }
}
