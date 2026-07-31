# Task 2: Pi 流式会话适配（tasks 2.1）

> 自包含任务文件。执行前请先读 `00-overview.md` 的 Global Constraints 与跨任务接口契约。依赖 Task 1 产出的 `ProviderName::Pi`/`ProviderType::Pi`。

**Goal:** 实现 Pi RPC 流式适配器（Auto-only，无授权扩展），覆盖流式输出、会话标识、恢复、取消和错误映射，并注册到生产/测试 registry。

**对应 spec requirement:**
- 「Pi 以流式会话执行并支持控制操作」（流式输出、取消、恢复）
- 「所选 Provider 的失败直接报告且不切换」（fail-fast）

**Files:**
- Create: `src/cross_cutting/pi_provider/mod.rs`（`PiProvider` 实现 `StreamingProviderAdapter`）
- Create: `src/cross_cutting/pi_provider/session.rs`（驱动 RPC 往返）
- Create: `src/cross_cutting/pi_provider/parse.rs`（Pi 事件 JSON → `ProviderEvent`）
- Create: `src/cross_cutting/pi_provider/tests.rs` + `src/cross_cutting/pi_provider/tests/fixtures/*.jsonl`（协议冻结 fixture）
- Modify: `src/cross_cutting/mod.rs`（`pub mod pi_provider;`）
- Modify: `src/web/state.rs:296-339`（`default_provider_registry()` 生产 + 测试分支注册 Pi）

**Interfaces:**
- Consumes: `StreamingProviderInput`（`working_dir`/`prompt`/`permission_mode`/`resume_provider_session_id`/`env_vars`/`timeout_secs`）、`JsonRpcPeer`、`CancellationToken`、`ProviderRegistry::register_gated`、`ProviderCompletion::from_output`。
- Produces: `PiProvider::new(command: PathBuf)`；`impl StreamingProviderAdapter for PiProvider`；生产/测试 registry 含 `ProviderName::Pi`。**Task 3/4 经 registry 选 Pi。**

**架构要点（Auto-only）：**
- **无授权扩展**：不创建 `aria-gate.ts`，不传 `-e`，不做授权 UI 往返。Pi 工具调用直接执行。
- **无 `-e` 扩展**：`build_args()` 不含 `-e` 参数（Auto-only 不需要扩展）。
- **权限模式**：Pi 始终以 Auto 运行，`permission_mode` 不影响 Pi 行为（Pi 无 Supervised）。
- **会话标识**：从 `get_state` 的响应或协议事件拿 `sessionId`，通过 `ProviderCompletion::from_output(..., Some(session_id))` 返回，由上层 engine 持久化 `ProviderConversationRef`（adapter 不直接持久化）。

---

## Step 1: 录制并提交协议冻结 fixture

参照 Phase 0 spike，用真实 Pi 录制 Auto 模式的 JSONL 会话，提交到 `src/cross_cutting/pi_provider/tests/fixtures/`（**不**用 `/tmp`）。每行用一个 envelope 标注方向（不要用自由文本，保证可解析）：

```json
{"direction": "client_to_pi", "payload": {"id": "p1", "type": "prompt", "message": "..."}}
{"direction": "pi_to_client", "payload": {"type": "message_update", "assistantMessageEvent": {"type": "text_delta", "delta": "..."}}}
```

录制三条：

1. `auto_text.jsonl`：一次文本输出 + 一次工具执行 + `agent_settled`。
2. `auto_cancel.jsonl`：prompt 后发 `abort`，会话终止。
3. `resume.jsonl`：输入携带旧 `resume_provider_session_id`，`get_state` 返回对应 session，完成事件含 `provider_session_id`（供 Step 8 的恢复测试）。

录制命令（Auto-only，无 `-e`）：

```bash
pi --mode rpc --no-session --session-dir <tmp> --no-skills --no-prompt-templates --no-context-files
```

（注：fixture 录制时用 `--no-session --session-dir <tmp>` **仅作录制隔离**，不是产品运行参数，也不验证恢复；正式实现**不加** `--no-session`、**不传** `--session-dir`，靠 `--session-id` 续接。恢复由 `resume.jsonl` fixture 与 Step 8 的 session-level 测试验证。）

- [ ] 提交 fixture 到 `tests/fixtures/`。

## Step 2: 从 fixture 确认协议包络（技术路线定案：pi_provider 独立分流）

写 fixture loader（`tests.rs` 内）解析 envelope，提取 inbound（pi_to_client）/outbound（client_to_pi）序列。

确认关键协议点（写成 `parse.rs` 的依据）：
- Pi 命令响应包络：`{"type":"response","id":...,"command":...,"success":...,"data":...}`（**不是** JSON-RPC 的 `result`）。
- **技术路线（高4，选项 2）：** `JsonRpcPeer::is_response()`（`json_rpc_peer.rs:196-202`）只识别含 `result`/`response`/`error` 的对象，**不识别** Pi 的 `type:"response"`/`data` 包络。因此**不扩展 `JsonRpcPeer`**（它是 Claude/Codex 共用，改它风险大），改由 `pi_provider/session.rs` **独立维护 response 分流**：
  - `pi_provider` 不用 `JsonRpcPeer::request()`；而是 `JsonRpcPeer::send()` 写命令、用 `next_incoming()` 读 stdout。
  - session.rs 维护 `pending_by_id: HashMap<String, oneshot::Sender<Value>>`。
  - reader loop 逐行读：若 `type=="response"` 且有匹配 `id`，走 `pending_by_id` 的 oneshot；否则作为事件走事件处理。
  - 命令超时、`success:false`、乱序 event/response 都需处理。
- `sessionId` 从 `get_state` 响应的 `data.sessionId` 获取。

测试：解析 fixture 每行的 `type`，断言能识别 `response`/`message_update`/`tool_execution_*`/`agent_settled`。另写一个单元测试验证 response 与 event 分流逻辑（mock pending map）。

- [ ] Run: `cargo test -p cadence-aria pi_provider`
- Expected: FAIL -- `pi_provider` 模块不存在

## Step 3: 建 `pi_provider` 模块骨架

`src/cross_cutting/pi_provider/mod.rs`：

```rust
use std::path::PathBuf;

mod parse;
mod session;

#[cfg(test)]
pub mod tests;

pub(crate) use parse::*;

pub const PI_COMMAND: &str = "pi";

#[derive(Debug, Clone)]
pub struct PiProvider {
    command: PathBuf,
}

impl PiProvider {
    pub fn new(command: PathBuf) -> Self {
        Self { command }
    }
}
```

`src/cross_cutting/mod.rs` 加 `pub mod pi_provider;`。

- [ ] Run: `cargo test -p cadence-aria pi_provider`
- Expected: 模块编译通过（测试仍 FAIL 因 parse 未实现）

## Step 4: 写失败测试 —— parse.rs 事件映射

`src/cross_cutting/pi_provider/tests.rs`：

```rust
#[test]
fn parse_text_delta_from_message_update() {
    let event = serde_json::json!({
        "type": "message_update",
        "assistantMessageEvent": { "type": "text_delta", "contentIndex": 0, "delta": "Hello" }
    });
    assert_eq!(parse_pi_text_delta(&event).as_deref(), Some("Hello"));
}

#[test]
fn parse_tool_execution_events() {
    let start = serde_json::json!({"type": "tool_execution_start", "toolCallId": "c1", "toolName": "bash"});
    assert!(parse_pi_tool_start(&start).is_some());
    let end = serde_json::json!({"type": "tool_execution_end", "toolCallId": "c1", "toolName": "bash", "isError": false});
    assert!(parse_pi_tool_end(&end).is_some());
}

#[test]
fn parse_agent_settled_as_terminal() {
    assert!(is_pi_terminal(&serde_json::json!({"type": "agent_settled"})));
}

#[test]
fn parse_session_id_from_get_state_response() {
    let resp = serde_json::json!({"type": "response", "command": "get_state", "success": true, "data": {"sessionId": "sess-1"}});
    assert_eq!(parse_pi_session_id(&resp).as_deref(), Some("sess-1"));
}
```

- [ ] Run: `cargo test -p cadence-aria pi_provider`
- Expected: FAIL —— parse 函数未定义

## Step 5: 实现 parse.rs

`src/cross_cutting/pi_provider/parse.rs` 实现：
- `parse_pi_text_delta(value: &Value) -> Option<String>`（从 `message_update.assistantMessageEvent.text_delta.delta` 提取）
- `parse_pi_tool_start(value: &Value) -> Option<PiToolStart>` / `parse_pi_tool_end(value: &Value) -> Option<PiToolEnd>`
- `is_pi_terminal(value: &Value) -> bool`（`type == "agent_settled"`）
- `parse_pi_session_id(value: &Value) -> Option<String>`（从 `get_state` 响应 `data.sessionId` 提取）

- [ ] Run: `cargo test -p cadence-aria pi_provider`
- Expected: PASS

## Step 6: 写失败测试 —— `build_args()`（Auto-only，无 -e、无 --session-dir）

`tests.rs` 加：

```rust
#[test]
fn build_args_rpc_mode_auto_only() {
    let provider = PiProvider::new("pi".into());
    let args = provider.build_args(None);
    assert!(args.contains(&"--mode".to_string()));
    assert!(args.contains(&"rpc".to_string()));
    assert!(!args.contains(&"-e".to_string()));               // Auto-only，无扩展
    assert!(!args.contains(&"--session-dir".to_string()));   // Pi 用默认 ~/.pi
    assert!(!args.contains(&"--no-extensions".to_string())); // 保留用户全局扩展
    assert!(!args.contains(&"--session-id".to_string()));    // 首次运行不传
}

#[test]
fn build_args_resume_includes_session_id() {
    let provider = PiProvider::new("pi".into());
    let args = provider.build_args(Some("sess-123"));
    assert!(args.contains(&"--session-id".to_string()));
    assert!(args.contains(&"sess-123".to_string()));
}
```

- [ ] Run: `cargo test -p cadence-aria pi_provider`
- Expected: FAIL —— `build_args` 未定义

## Step 7: 实现 `build_args()`

`mod.rs` 加：

```rust
impl PiProvider {
    /// 构造 pi RPC 命令行（Auto-only）。
    /// - 不含 -e：无授权扩展。
    /// - 不设 --session-dir：Pi 用默认 ~/.pi。
    /// - 不设 --no-extensions：保留用户全局扩展。
    /// - cwd 由 spawn 时传 working_dir（项目代码库目录）。
    pub(crate) fn build_args(&self, resume_session_id: Option<&str>) -> Vec<String> {
        let mut args = vec!["--mode".to_string(), "rpc".to_string()];
        if let Some(sid) = resume_session_id.map(str::trim).filter(|s| !s.is_empty()) {
            args.push("--session-id".to_string());
            args.push(sid.to_string());
        }
        args
    }
}
```

- [ ] Run: `cargo test -p cadence-aria pi_provider`
- Expected: PASS

## Step 8: 写失败测试 —— session 驱动（duplex 模拟 Pi peer）

用 `tokio::io::duplex` 模拟 Pi 的 stdin/stdout。`JsonRpcPeer::new(reader, writer)` 需要读写两半。fake Pi 协程：先写入预置的 inbound JSONL（来自 fixture），同时读取并断言 outbound。

`tests.rs` 加（完整可运行框架）：

```rust
use tokio::io::duplex;

#[tokio::test]
async fn session_sends_prompt_and_emits_text_until_settled() {
    let (client_io, mut pi_io) = duplex(8192);
    // client_io 交给 run_pi_session；pi_io 由本测试扮演 fake Pi
    // fake Pi：读 prompt 命令 → 回 response(success) → 写 message_update(text_delta) → 写 agent_settled
    // 断言：run_pi_session 把 text_delta 转成 ProviderEvent::TextDelta，遇 agent_settled 发 Completed
}

#[tokio::test]
async fn session_captures_session_id_into_completion() {
    // fake Pi：get_state 响应含 sessionId → 断言 ProviderCompletion.provider_session_id == Some(sessionId)
}

#[tokio::test]
async fn session_aborts_on_provider_command_abort() {
    // 发 ProviderCommand::Abort 到 command channel
    // 断言 fake Pi 读到 abort 命令，run_pi_session 发 aborted 状态并终止，后续输出不消费
}

#[tokio::test]
async fn session_demultiplexes_response_by_id() {
    // fake Pi：先发一个 event 再发该命令的 response（乱序）
    // 断言 session 的 pending_by_id 正确分流：response 走 oneshot，event 走事件处理
}

#[tokio::test]
async fn session_failure_is_terminal_no_retry() {
    // fake Pi：中途写错误事件 / EOF → 断言 run_pi_session 发 Failed，不重试（start 不调第二次）
}
```

注：每个测试需明确 `run_pi_session` 的参数签名、`JsonRpcPeer` 的构造、`ProviderEvent` 的 drain 方式。fake Pi 协程的读写时序要与真实 Pi 一致（参照 fixture）。

- [ ] Run: `cargo test -p cadence-aria pi_provider`
- Expected: FAIL —— `run_pi_session` 未实现

## Step 9: 实现 `StreamingProviderAdapter for PiProvider` + `run_pi_session`

`mod.rs` 仿 `codex_provider/mod.rs` 的 `start()`：`ProcessManager::spawn` + `JsonRpcPeer::new(stdout, stdin)` + 创建 `(event_tx, event_rx)` 与 `(command_tx, command_rx)` + `tokio::spawn(run_pi_session(...))`，返回 `ProviderSession { events: event_rx, commands: command_tx }`。Auto-only **不需要** `ApprovalBridge`（Pi 无 Supervised）。

`session.rs` 的 `run_pi_session(peer, command_rx, event_tx, input, cancel)`：
- 维护 `pending_by_id: HashMap<String, oneshot::Sender<Value>>`（高4 选项 2 独立分流）。
- 启动后 `send` get_state 命令，按 id 等 response，拿 `sessionId`（首次无 `--session-id`，供续接）。
- `send` prompt 命令，按 id 等 response ack。
- reader loop（`select!` 含 `command_rx`、`cancel`、`next_incoming`）：
  - `ProviderCommand::Abort` -> `send` abort 命令 -> 发 aborted 状态 -> 终止
  - `next_incoming` 行：`type=="response"` 且匹配 pending id -> oneshot；否则作为事件处理
  - `message_update` 文本增量 -> `ProviderEvent::TextDelta`
  - `tool_execution_*` -> 工具执行事件
  - 错误事件 -> `ProviderEvent::Failed`（fail-fast，不重试）
  - `agent_settled` -> `ProviderCompletion::from_output(full_output, contract, Some(session_id))` -> `ProviderEvent::Completed`
- EOF/进程退出 -> `ProviderEvent::Failed`
## Step 10: 写失败测试 —— 生产 registry 含 Pi

`src/web/state.rs` 相关测试：构造生产模式 `default_provider_registry`，断言 `registry.get(&ProviderName::Pi).is_some()`。

- [ ] Run: `cargo test -p cadence-aria state`
- Expected: FAIL —— 生产 registry 只注册 Claude/Codex

## Step 11: `default_provider_registry()` 注册 Pi

`src/web/state.rs:296-339`：
- **测试模式**（`test_provider_enabled`）分支：在 Claude/Codex 的 Fake 注册后加 `registry.register(ProviderName::Pi, Arc::new(TestControlledFakeStreamingProvider::new(...)))`。
- **生产模式**分支：在 Codex 的 `register_gated` 后加：

```rust
    registry.register_gated(
        ProviderName::Pi,
        Arc::new(PiProvider::new(PathBuf::from("pi"))),
        provider_gate,
    );
```

`use` 引入 `crate::cross_cutting::pi_provider::PiProvider`。

- [ ] Run: `cargo test -p cadence-aria state`
- Expected: PASS

## Step 12: 全量受影响测试 + Commit

- [ ] Run:

```bash
cargo test -p cadence-aria pi_provider
cargo test -p cadence-aria state
git add src/cross_cutting/pi_provider/ src/cross_cutting/mod.rs src/web/state.rs
git commit -m "feat(pi): add Pi RPC streaming adapter (Auto-only) and registry registration"
```

---

## 完成检查（对应 tasks 2.1）

- [ ] Pi RPC 会话适配覆盖流式输出、会话标识、恢复、取消和错误映射（Auto-only，无授权扩展）。
