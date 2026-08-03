# Task 7: Pi 结构化提问扩展（tasks 2.2, 2.3）

> 自包含任务文件。执行前请先读 `00-overview.md` 的 Global Constraints。依赖 Task 1（`ProviderName::Pi`）与 Task 2（`PiProvider` 已实现并注册）。

**Goal:** 给 Pi 加结构化提问能力——`aria-ask.ts` 扩展 + `include_str!` 交付 + 双向协议映射（`extension_ui_request(select)` ↔ `ChoiceRequest`/`ChoiceResponse`），使 Pi 需要澄清时弹出与 Claude/Codex 一致的选择卡片，答案在同进程内接续。

**对应 spec requirement:**
- 「Pi 支持结构化提问」（ask_user 工具 → select → ChoiceRequest → 同进程续跑）
- 「提问扩展不拦截工具调用」（Auto 模式工具直接执行）

**对应 tasks.md:** 2.2（提问扩展 + 映射 + prompt 指引）、2.3（版本检测）

## Spike（✅ 已通过）

| 能力 | 结果 |
|---|---|
| 扩展加载零 error | ✅ |
| `ask_user` 注册成功 | ✅ |
| LLM 主动调用 | ✅ |
| `select` → `extension_ui_request` 流出 | ✅ |
| 回 `{value}` → 同进程续跑 | ✅ |

**结论：** 技术地基成立，直接进入实现。

## Files

- Create: `src/cross_cutting/pi_provider/aria-ask.ts`（提问扩展源码，`include_str!` 引用）
- Create: `src/cross_cutting/pi_provider/tests/fixtures/select_request.jsonl`（select 协议冻结 fixture）
- Modify: `src/cross_cutting/pi_provider/mod.rs`（`include_str!` + `ensure_ask_extension()` + `build_args` 加 `-e` + 版本检测）
- Modify: `src/cross_cutting/pi_provider/session.rs`（select 映射 + ChoiceResponse 回传 + 重构 command 等待逻辑）
- Modify: `src/cross_cutting/pi_provider/parse.rs`（加 `parse_pi_select_request`）
- Modify: `src/web/workspace_context/prompts.rs`（Pi 指引从「输出文本暂停信号」改为「使用 `ask_user` 工具提问」）
- Test: `src/cross_cutting/pi_provider/tests.rs`

## Interfaces

- Consumes: `ProviderEvent::ChoiceRequest(ChoiceRequestData)`（**adapter 层**类型，定义于 `src/cross_cutting/streaming_provider/mod.rs:95-103`，**不是** `EngineEvent::ChoiceRequest`）；`ChoiceRequestSource::ProviderChoice`（`mod.rs:121`）；`ProviderCommand::ChoiceResponse`（`mod.rs:257-271`）；`JsonRpcPeer::send`；`ChoiceOptionData`（`mod.rs:72-76`）。
- Produces: `aria-ask.ts` 经 `include_str!` 嵌入；Pi 提问时 `ChoiceRequest(source=ProviderChoice)` 往返映射。

## 关键设计决策

**1. select 协议映射规则（option value ↔ ChoiceOptionData）：**

Pi 的 `extension_ui_request(method=select)` 真实线格式（来自 spike fixture）：

```json
{"type":"extension_ui_request","id":"<uuid>","method":"select","title":"<问题>","options":["A","B","C"]}
```

映射到 adapter 层：

| Pi select 字段 | adapter `ChoiceRequestData` 字段 |
|---|---|
| `id` | `id`（暂存，用于回传匹配） |
| `title` | `prompt` |
| `options[i]`（原始字符串数组） | `options`：每个变成 `ChoiceOptionData { id: options[i], label: options[i], description: None }`（Pi 的 option 是纯字符串，无独立 id/label/description 区分） |
| —（无 timeout 时） | `allow_multiple: false`, `allow_free_text: true` |
| `timeout`（若 Pi 发了） | **忽略**（Aria 侧由用户决定何时回答，不受 Pi 的 timeout 约束） |

回传规则：用户 `ChoiceResponse` 到来时——
- `selected_option_ids` 非空：`value = selected_option_ids[0]`（单选取第一个）
- `selected_option_ids` 空但 `free_text` 非空：`value = free_text`
- 两者都空：回 `{cancelled: true}`（用户未作选择）
- 回传包络：`{type:"extension_ui_response", id: <暂存的 select id>, value: <value>}` 或 `{cancelled: true}`

**2. select 在 reader loop 的处理：**

`dispatch_pi_response` 只认 `type=="response"`，不会吞 `extension_ui_request`（已确认 `session.rs:298-309`）。在两个 reader loop（main `:157-175` 和 handshake `:275-292`）的 `next_incoming` 分支里，`dispatch_pi_response` 之后、`parse_pi_failure`/`handle_pi_event` 之前，加 `parse_pi_select_request` 分支。

收到 select 后：
- 发 `ProviderEvent::ChoiceRequest`（source=`ProviderChoice`）
- 进入 `await_pi_choice_response` 等待状态（`tokio::select!` 同时处理 cancel、Abort、匹配 id 的 ChoiceResponse）
- **重构 `await_pi_abort`**（`:181-196`）为 `await_pi_command`，支持 Abort 和 ChoiceResponse 两种 command，不再静默吞掉 ChoiceResponse
- 等待期间 **不调** `next_incoming`（Pi 在 `ctx.ui.select` 里阻塞，不会发后续事件）
- `Abort`/cancel 到来时：发 Pi `abort` 命令、发 `ProviderStatus::Aborted`、终止（与 Task 2 fix round 1 的 abort 逻辑一致）
- 错 id 的 ChoiceResponse：发 protocol error `PERMISSION_ID_UNMATCHED`，继续等待

**3. include_str! + 缓存落盘：**

```rust
const ARIA_ASK_EXTENSION: &str = include_str!("aria-ask.ts");
const MIN_PI_VERSION: &str = "0.83.0";

/// 落盘扩展到缓存目录。内容不变（哈希一致）则复用。
fn ensure_ask_extension() -> Result<PathBuf, ProviderAdapterError> {
    // 用 AriaStatePaths（既有，不需新依赖）的 workspace_root 下 .aria/cache/
    // 或用 std::env::temp_dir() 作为 fallback
    let cache = std::env::temp_dir().join("cadence-aria-pi-ask");
    std::fs::create_dir_all(&cache).map_err(|e| ...)?;
    let hash = sha256(ARIA_ASK_EXTENSION)[..8];
    let path = cache.join(format!("aria-ask-{hash}.ts"));
    if !path.exists() {
        std::fs::write(&path, ARIA_ASK_EXTENSION).map_err(|e| ...)?;
    }
    Ok(path)
}
```

**`build_args` 不做落盘**——`start()` 先调 `ensure_ask_extension()` 拿到路径（处理错误），再把路径传给 `build_args(resume_id, extension_path)`。`build_args` 仍然是纯构造（`Vec<String>`），不改返回类型。

**4. 版本检测（task 2.3）：**

`PiProvider` 没有健康检查数据的访问途径（`PiProvider::new(PathBuf)` 只持有 command path）。版本检测在 `start()` 内自探：

```rust
async fn start(&self, input, cancel) -> Result<ProviderSession, ProviderAdapterError> {
    let version = probe_pi_version(&self.command).await?; // 跑 pi --version，解析
    if version < MIN_PI_VERSION { return Err(incompatible error); }
    let ext = ensure_ask_extension()?;
    let args = self.build_args(input.resume_provider_session_id.as_deref(), &ext);
    // ... spawn ...
}
```

`probe_pi_version` 用 `BoundedCommandRunner`（既有）或有界 timeout 跑 `pi --version`，解析 stdout 中的版本 token。失败（命令缺失/超时/无法解析）**不阻止启动**——返回 `Ok(unknown_version)`，只在能确定低于最低版本时才返回 `Err`。这样 `pi` 不在 PATH 时由 gate 层（`ProviderAvailabilityGate`）拦截，而不是版本检测重复报错。

**不引入新依赖**：版本比较用简单 `split('.')` + 数值比较（Pi 版本是 `major.minor.patch`），不引入 semver crate。

**5. prompt 指引更新：**

`workspace_context/prompts.rs:126` 的 Pi 分支改为（与 Claude/Codex 措辞对齐）：

> "当前 author provider 是 Pi；需要向用户确认时，使用 `ask_user` 工具提问并等待回答。禁止输出文本 A/B/C 选择题作为交互替代；`ask_user` 会经 Aria 弹出选择卡片，用户回答后同一 Pi 进程继续。"

## Steps

- [ ] **Step 1: 写 `aria-ask.ts` + 源码契约测试**

照 spike 验证过的扩展源码写 `aria-ask.ts`。加一个测试断言嵌入的 `ARIA_ASK_EXTENSION` 常量含 `"ask_user"`（工具名）、`"ctx.ui.select"`（调用）、`"promptGuidelines"`（引导），且不含 `"tool_call"`（确认不挂拦截钩子）。

Run: `cargo test -p cadence-aria pi_provider_aria_ask_extension`
Expected: FAIL（常量未定义）

- [ ] **Step 2: `include_str!` + `ensure_ask_extension()` + `build_args` 加 `-e`**

在 `mod.rs` 加 `include_str!` 常量和 `ensure_ask_extension()`（返回 `Result<PathBuf, ProviderAdapterError>`）。改 `build_args(&self, resume_id, ext_path: &Path)` 加 `-e <path>`。`start()` 在 spawn 前调 `ensure_ask_extension()`。

测试（Step 1 的 + `build_args` 含 `-e` 且路径指向存在文件）。

Run: `cargo test -p cadence-aria pi_provider`
Expected: PASS

- [ ] **Step 3: 写 select fixture + parse 测试**

录制/固定 `extension_ui_request(select)` 的 JSON 到 `tests/fixtures/select_request.jsonl`。加 `parse_pi_select_request` 测试：解析 fixture，返回 `PiSelectRequest { id, title, options: Vec<String> }`，断言题目/选项完整保留。

Run: `cargo test -p cadence-aria parse_pi_select`
Expected: FAIL（函数未定义）

- [ ] **Step 4: 实现 `parse_pi_select_request`**

在 `parse.rs` 加函数。

Run: `cargo test -p cadence-aria parse_pi_select`
Expected: PASS

- [ ] **Step 5: 写 session select 往返 duplex 测试**

```rust
#[tokio::test]
async fn session_select_request_maps_to_choice_request_and_forwards_response() {
    // fake Pi 发 extension_ui_request(select, title="格式?", options=["A","B"])
    // 断言 session 发出 ProviderEvent::ChoiceRequest { source: ProviderChoice, prompt: "格式?", options: [A, B] }
    // 发 ProviderCommand::ChoiceResponse { selected_option_ids: ["A"] }
    // 断言 fake Pi 收到 extension_ui_response { value: "A" }
    // fake Pi 继续发 text_delta + agent_settled
    // 断言后续输出正常转发
}

#[tokio::test]
async fn session_select_with_free_text_maps_to_value() {
    // selected_option_ids 空、free_text="自定义"
    // 断言 extension_ui_response { value: "自定义" }
}

#[tokio::test]
async fn session_select_abort_during_wait_sends_pi_abort() {
    // select 等待期间发 Abort
    // 断言 fake Pi 收到 abort 命令，session 发 Aborted
}

#[tokio::test]
async fn session_select_during_handshake_is_handled() {
    // handshake 阶段（get_state response 尚未到达）先到达 select
    // 断言 select 不被吞，ChoiceRequest 正常发出
}
```

Run: `cargo test -p cadence-aria pi_provider`
Expected: FAIL（select 映射未实现）

- [ ] **Step 6: 实现 session select 映射 + 重构 command 等待**

在两个 reader loop 加 `parse_pi_select_request` 分支。重构 `await_pi_abort` 为 `await_pi_command`，支持 Abort + ChoiceResponse。加 `send_pi_choice_response` helper。

Run: `cargo test -p cadence-aria pi_provider`
Expected: PASS

- [ ] **Step 7: 写工具调用不产生授权测试**

```rust
#[tokio::test]
async fn session_tool_call_does_not_produce_permission_or_choice_request() {
    // fake Pi 发 tool_execution_start + tool_execution_end
    // 断言只收到 ProviderEvent::ToolCall + ProviderEvent::ToolResult
    // 断言不收到 ChoiceRequest 或 PermissionRequest
}
```

Run: `cargo test -p cadence-aria session_tool_call`
Expected: PASS（Task 2 已实现此行为，测试是回归锁定）

- [ ] **Step 8: 写版本检测测试**

```rust
#[test]
fn pi_version_below_minimum_returns_error() { /* 0.82.0 < 0.83.0 → Err */ }
#[test]
fn pi_version_at_or_above_minimum_passes() { /* 0.83.0, 0.84.0 → Ok */ }
#[test]
fn pi_version_unparseable_does_not_block() { /* "pi version xyz" → Ok(unknown) */ }
```

Run: `cargo test -p cadence-aria pi_version`
Expected: FAIL（版本比较函数未定义）

- [ ] **Step 9: 实现版本检测**

在 `mod.rs` 加 `probe_pi_version` + 版本比较。`start()` 在 spawn 前调用。

Run: `cargo test -p cadence-aria pi_version`
Expected: PASS

- [ ] **Step 10: 更新 Pi prompt 指引**

`workspace_context/prompts.rs` Pi 分支改为「使用 `ask_user` 工具提问并等待回答」。

Run: `cargo test -p cadence-aria workspace_context`
Expected: PASS

- [ ] **Step 11: 全量门禁 + Commit**

```bash
cargo test -p cadence-aria --lib
cargo test -p cadence-aria pi_provider
cargo clippy -p cadence-aria --all-targets
cargo fmt --check
cd web && npm test && cd ..
```

## 完成检查

- [ ] 2.2：`aria-ask.ts` 经 `include_str!` 交付；`ask_user → extension_ui_request(select) → ChoiceRequest(source=ProviderChoice)` 往返；题目/选项完整；`ChoiceResponse.value → extension_ui_response.value` 同进程续跑；`free_text` 映射为 `value`；`cancelled` 语义已定义；普通工具不产生授权（ToolCall + ToolResult 都产生）；Pi prompt 指引已更新。
- [ ] 2.3：版本检测在 `start()` 内自探（`pi --version`），低于 `0.83.0` 返回可诊断错误；无法解析不阻止启动；兼容/不兼容版本测试覆盖。
