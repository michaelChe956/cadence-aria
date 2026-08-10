# Task 2: Kimi ACP 流式会话适配（task 2.1）

> 自包含任务文件。执行前请先读 `00-overview.md` 的 Global Constraints 与跨任务接口契约。本任务是整个接入的技术核心。

**Goal:** 新增 `src/cross_cutting/kimi_code_provider/`（mod/parse/session/tests），冻结 0.34.0 真实 ACP 往返为 fixture，实现 `KimiCodeProvider` + `impl StreamingProviderAdapter`，复用 `JsonRpcPeer` 驱动 initialize → session/new|load → session/prompt 状态机；覆盖文本流、工具事件、终态（stopReason）、resume、取消、超时、thought、协议降级；注册到生产/测试 registry。

**对应 spec requirement:**
- 「Kimi 以 ACP 流式会话执行并支持控制操作」（含取消/异常退出/stopReason）
- 「Kimi 支持会话恢复（resume）」

**Files:**
- Create: `src/cross_cutting/kimi_code_provider/mod.rs`（`KimiCodeProvider`、版本探测 `kimi_version_command`/`probe_kimi_version`/`ensure_kimi_version_compatible`、`build_args`、spawn、生命周期）
- Create: `src/cross_cutting/kimi_code_provider/parse.rs`（ACP 消息解析）
- Create: `src/cross_cutting/kimi_code_provider/session.rs`（状态机 `run_kimi_session`）
- Create: `src/cross_cutting/kimi_code_provider/tests.rs`（单测）
- Create: `src/cross_cutting/kimi_code_provider/tests/fixtures/*.jsonl`（冻结自 `.pi-subagents/spike/acp/`）
- Create: `tests/fixtures/provider/kimi_acp_*_fixture.sh`（模拟子进程 stdin/stdout/stderr/exit，仿 `tests/fixtures/provider/codex_app_server_*`）
- Modify: `src/cross_cutting/mod.rs`（加 `pub mod kimi_code_provider;`）
- Modify: `src/web/state.rs:371-385`（生产/测试 `default_provider_registry()` 注册 `ProviderName::KimiCode`）

**Interfaces:**
- Consumes: `JsonRpcPeer::new(stdout, stdin)`、`ProcessManager::spawn`、`StreamingProviderInput`（含 `resume_provider_session_id`、`timeout_secs`、`permission_mode`、`prompt`、`working_dir`）、`ProviderEvent`/`ProviderCommand`/`ProviderSession`/`ProviderToolCall`/`ProviderToolResult`、`CancellationToken`。
- Produces: `pub struct KimiCodeProvider { command: PathBuf }`、`KimiCodeProvider::new(command: PathBuf)`、`impl StreamingProviderAdapter for KimiCodeProvider`、`pub(crate) async fn run_kimi_session<W>(peer, command_rx, event_tx, input, cancel)`、ACP fixture 文件。**Task 3-7 依赖。**

**参照：** `codex_provider/mod.rs:53-119`（build_args/spawn/peer/session 派发骨架）、`codex_provider/session.rs:25-189`（JSON-RPC 长连接 + request timeout + abort）、`pi_provider/mod.rs:192-243`（版本探测/门禁）、`pi_provider/tests.rs`（脚本化子进程 fixture 测试范式）。

---

## Step 1: 冻结 ACP fixture

把 `.pi-subagents/spike/acp/acp_session_trace.jsonl` **按 method/场景边界手王截取对应行段**，切分为多个独立 .jsonl（每文件 = 一个完整场景的 JSON-RPC 往返）：
- `initialize.jsonl`（initialize 请求 + result 往返）
- `text_turn.jsonl`（session/new + agent_message_chunk + session/prompt result end_turn）
- `tool_call_turn.jsonl`（tool_call/tool_call_update completed/failed + prompt result）
- `error_exit.jsonl`（进程异常退出 / exit 1 / exit 75）

**注意**：request_permission 场景（普通审批 + AskUserQuestion）**不在此 task 的 fixture**——留给 Task 3 的专用 fixture（`askuser_multiquestion.jsonl` 等），避免本 task 占位逻辑误触 AskUserQuestion。

并创建脚本化子进程 fixture `tests/fixtures/provider/kimi_acp_*_fixture.sh` 等（仿 `codex_app_server_*`）：读取 stdin、按行回放 fixture 到 stdout、写 stderr、按指定 exit code 退出；支持参数化场景。

- [ ] 手工核对：fixture 内容与 spike 0.34.0 真实往返一致（字段名、JSON 结构）；不含 request_permission 场景。

## Step 2: `mod.rs` 骨架 + 版本探测（失败测试先行）

`src/cross_cutting/kimi_code_provider/tests.rs` 写版本探测与门禁测试：

```rust
#[test]
fn kimi_version_command_uses_kimi_binary() {
    let cmd = super::kimi_version_command();
    assert_eq!(cmd.program, "kimi");
    assert_eq!(cmd.args, vec!["--version".to_string()]);
}

#[test]
fn parse_kimi_version_reads_major_minor_patch() {
    let v = super::parse_kimi_version("kimi 0.34.0");
    assert_eq!(v, super::KimiVersion(0, 34, 0));
}

#[test]
fn ensure_kimi_version_rejects_below_0_34_0() {
    let low = super::parse_kimi_version("kimi 0.33.0");
    assert!(super::ensure_kimi_version_compatible(&low).is_err());
    let ok = super::parse_kimi_version("kimi 0.34.0");
    assert!(super::ensure_kimi_version_compatible(&ok).is_ok());
}
```

- [ ] Run: `cargo test --locked --lib kimi_version`
- Expected: FAIL —— 模块不存在

实现 `mod.rs`：
```rust
pub const KIMI_COMMAND: &str = "kimi";
const MIN_KIMI_VERSION: &str = "0.34.0";
pub struct KimiCodeProvider { command: PathBuf }
impl KimiCodeProvider { pub fn new(command: PathBuf) -> Self { Self { command } } }
// build_args -> vec!["acp".to_string()]
// kimi_version_command / probe_kimi_version / parse_kimi_version / ensure_kimi_version_compatible
// impl StreamingProviderAdapter for KimiCodeProvider { fn start(...) -> Result<ProviderSession, ProviderAdapterError> }
```
`start()` 流程：probe 版本+门禁 → `ProcessManager::spawn(kimi, ["acp"], cwd=working_dir)` → `JsonRpcPeer::new` → 起 stderr 读取任务 → spawn `run_kimi_session` → 返回 `ProviderSession{event_rx, command_tx, session_id:None}`。

`src/cross_cutting/mod.rs` 加 `pub mod kimi_code_provider;`。

- [ ] Run: `cargo test --locked --lib kimi_version`
- Expected: PASS

## Step 3: `parse.rs` —— ACP 消息解析（失败测试先行）

`tests.rs` 加解析测试（覆盖各 sessionUpdate 子类型、request_permission、prompt result、arguments 二次解析、未知子类型不 panic）：

```rust
#[test]
fn parses_agent_message_chunk() {
    let msg: serde_json::Value = serde_json::from_str(r#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"hi"}}}}"#).unwrap();
    let parsed = super::parse::parse_message(&msg);
    assert!(matches!(parsed, super::parse::Parsed::SessionUpdate(super::parse::KimiSessionUpdate::AgentMessageChunk{..})));
}
// 类似：AgentThoughtChunk / ToolCall / ToolCallUpdate(completed/failed/in_progress) / SessionInfoUpdate / UsageUpdate
#[test]
fn parses_request_permission_askuserquestion() {
    // 含 options + toolCall.title=="AskUserQuestion"
}
#[test]
fn parses_prompt_result_end_turn() { /* stopReason end_turn */ }
#[test]
fn parses_unknown_sessionupdate_without_panic() { /* 未知 sessionUpdate 子类型 */ }
#[test]
fn parses_tool_arguments_json_string_safely() { /* 损坏 JSON 不 panic */ }
```

- [ ] Run: `cargo test --locked --lib parses_`
- Expected: FAIL

实现 `parse.rs`：
```rust
pub(crate) enum KimiSessionUpdate {
    AgentThoughtChunk { content: String },
    AgentMessageChunk { content: String },
    ToolCall { tool_call_id: String, title: String, kind: String, status: String },
    ToolCallUpdate { tool_call_id: String, status: String, content: String },
    SessionInfoUpdate { title: String },
    UsageUpdate { used: u64, size: u64 },
    AvailableCommandsUpdate,  // 忽略
}
pub(crate) struct KimiPermissionRequest { pub request_id: serde_json::Value, pub tool_call_id: String, pub title: String, pub options: Vec<KimiPermissionOption>, pub content_text: String }
pub(crate) struct KimiPermissionOption { pub option_id: String, pub name: String, pub kind: String }
pub(crate) enum KimiPromptResult { StopReason(String), Error(String) }
pub(crate) enum Parsed { SessionUpdate(KimiSessionUpdate), RequestPermission(KimiPermissionRequest), PromptResult(KimiPromptResult), Unknown(String) }
pub(crate) fn parse_message(v: &Value) -> Parsed { /* 按 method/sessionUpdate 分派 */ }
```
未知子类型 → `Parsed::Unknown`，不 panic。

- [ ] Run: `cargo test --locked --lib parses_`
- Expected: PASS

## Step 4: `session.rs` —— 状态机核心（失败测试先行，用脚本化 fixture）

`tests.rs` 加端到端测试（用 `tests/fixtures/provider/kimi_acp_*_fixture.sh` 作子进程，仿 `pi_provider/tests.rs` 脚本化范式）：

```rust
#[tokio::test]
async fn text_turn_completes_with_full_output() {
    // 启动 kimi_acp_text_fixture.sh → 跑 run_kimi_session → 断言收到 TextDelta("...") + Completed{full_output, provider_session_id:Some}
}
#[tokio::test]
async fn tool_call_emits_toolcall_then_toolresult_once() {
    // 断言 ToolCall + 仅一次 ToolResult(is_error=false)；in_progress 增量不产生 ToolResult
}
#[tokio::test]
async fn failed_tool_result_sets_is_error_true() { /* status=failed → is_error=true */ }
#[tokio::test]
async fn abort_sends_session_cancel_then_terminates_and_aborted_status() { /* 优先 session/cancel，发 Aborted 不发 Failed */ }
#[tokio::test]
async fn process_crash_before_prompt_result_emits_failed() { /* 异常退出 → 一次 Failed */ }
#[tokio::test]
async fn completed_failed_emitted_exactly_once() { /* 不双发 */ }
#[tokio::test]
async fn resume_uses_session_load_when_resume_id_present() {
    // input.resume_provider_session_id=Some → 断言发 session/load 而非 session/new
}
#[tokio::test]
async fn resume_load_failure_emits_failed_not_fallback_new() { /* load 失败 → Failed，不脚到 new */ }
#[tokio::test]
async fn total_timeout_consumes_input_timeout_secs() { /* 超时 → 取消流程 → 一次终态 */ }
```

- [ ] Run: `cargo test --locked --lib kimi_`
- Expected: FAIL

实现 `session.rs` `run_kimi_session`：
1. 发 initialize(id:1)，校验 `protocolVersion==1` + capability（loadSession/resume 用于 resume 路径）。
2. 发 `notifications/initialized`。
3. **resume/new 互斥**：`input.resume_provider_session_id` 存在 → `session/load(sessionId,cwd,permissionMode)`；否则 `session/new(cwd,permissionMode)`。从 response 取 sessionId，上报 `provider_session_id`。load 失败→Failed（不脚 new）。
4. 发 `session/prompt(id:N,input.prompt)`。
5. 事件循环 `select!{ peer.recv / command_rx.recv / timeout / cancel }`：
   - `session/update` → 按 `KimiSessionUpdate` 映射：
     - AgentMessageChunk → `ProviderEvent::TextDelta{content}`，累计 full_output。
     - ToolCall(pending) → `ProviderEvent::ToolCall(ProviderToolCall{tool_use_id,tool_name:title,input:arguments(若已完成)})`。
     - ToolCallUpdate → 按 tool_call_id 缓冲；**仅 completed/failed** 发一次 `ProviderEvent::ToolResult(ProviderToolResult{tool_use_id,output:聚合content,is_error:(status==failed)})`；in_progress 不发 ToolResult。
     - AgentThoughtChunk → 不计 full_output，输出到 tracing（**不写文件**）。
     - UsageUpdate → tracing 指标。
   - `session/request_permission`（id:M）→ **此 task 仅占位**（Task 3 实装 ApprovalBridge/ChoiceRequest 往返）：本 task 的 fixture **不含** request_permission 场景（见 Step 1），故占位逻辑实际不触发；实现时写一个防御性分支：收到 request_permission 则发一次 `ProviderEvent::Failed("request_permission handling not configured until Task 3")` 并在注释标 TODO(Task3)，避免未来误用时静默挂起。
   - `session/prompt`(id:N) result → 终态：`stopReason=="end_turn"` → `Completed(full_output)`（一次）；其他/error → `Failed`。
   - timeout（`input.timeout_secs` 总超时 / 各 request 独立超时 / resume stall）→ 走取消流程。
   - cancel/Abort → 发 ACP `session/cancel` notification → 短超时 drain → 未果 `ProcessManager` 进程组 terminate → 发 `ProviderStatus::Aborted`（**不发 Failed**）。
6. 进程退出：exit 0 且已 Completed 则静默；未发终态→Failed。退出码 0/1/75 映射 Failed 文案区分。
7. **协议降级**：未知 notification→日志忽略；未知 request（有 id）→回 `-32601 Method not found`；request_permission 未知 option kind→按拒绝+保留原 id。

不变量：Completed/Failed 一次且仅一次；provider_session_id 在 load/new response 后上报；full_output 只累计 AgentMessageChunk。

- [ ] Run: `cargo test --locked --lib kimi_`
- Expected: PASS（含 resume/abort/crash/timeout/invariant 全部）

## Step 5: 注册到生产/测试 registry

`src/web/state.rs`：生产分支 `default_provider_registry()` 加
```rust
registry.register_gated(
    ProviderName::KimiCode,
    Arc::new(KimiCodeProvider::new(PathBuf::from("kimi"))),
    /* gate 同 Codex/Pi */);
```
fake 分支注册受控 fake adapter（与 Pi 一致）。import `use crate::cross_cutting::kimi_code_provider::KimiCodeProvider;`。

- [ ] Run: `cargo test --locked --lib web::state` 与 `cargo test --locked --lib provider_registry`
- Expected: Kimi 注册成功；先 FAIL 后 PASS。

## Step 6: 质量检查与提交

- [ ] Run: `cargo fmt --check` / `cargo clippy --all-targets --all-features --locked -- -D warnings` / `cargo test --locked`
- Expected: 全绿
- [ ] Commit:
```bash
git add -A
git commit -m "feat(kimi): Task 2 ACP 流式适配器（mod/parse/session/tests + fixture + resume + 取消session/cancel + 超时 + 终态stopReason + thought不落盘 + 协议降级 + registry注册）"
```
