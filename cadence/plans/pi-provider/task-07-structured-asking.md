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
- Modify: `src/cross_cutting/pi_provider/mod.rs`（`build_args` 加 `-e`；`include_str!` 嵌入 + 缓存落盘；版本检测）
- Modify: `src/cross_cutting/pi_provider/session.rs`（reader loop 加 `extension_ui_request(select)` → `ChoiceRequest` 映射；command loop 加 `ChoiceResponse` → `extension_ui_response(value)` 回传）
- Modify: `src/cross_cutting/pi_provider/parse.rs`（加 `parse_pi_select_request`）
- Modify: `src/web/workspace_context/prompts.rs`（Pi 指引从「输出文本暂停信号」改为「使用 `ask_user` 工具提问」）
- Test: `src/cross_cutting/pi_provider/tests.rs`（duplex 测试覆盖 select 往返 + 工具不拦截 + 版本检测）

## Interfaces

- Consumes: `ProviderEvent::ChoiceRequest`、`ChoiceRequestSource::ProviderChoice`、`ProviderCommand::ChoiceResponse`、`JsonRpcPeer::send`、`ChoiceOptionData`。
- Produces: `aria-ask.ts` 经 `include_str!` 嵌入；Pi 提问时 `ChoiceRequest(source=ProviderChoice)` 往返映射。

## 关键设计决策

**1. select 在 reader loop 的处理位置：** `extension_ui_request(select)` 不是 response（不匹配 command id），不能走 `dispatch_pi_response`。它在 `handle_pi_event` 之前处理——select 到达时 session 应暂停消费后续事件、发 ChoiceRequest、等 command_rx 返回 ChoiceResponse。具体：在两个 reader loop（handshake 和 main）的 `next_incoming` 分支里，`dispatch_pi_response` 之后、`handle_pi_event` 之前，加一个 `parse_pi_select_request` 分支。

**2. ChoiceResponse 回传：** session 的 `command_rx.recv()` 当前只处理 `Abort`。加一个 `ChoiceResponse` 分支：构造 `{type:"extension_ui_response", id, value}` 并 `peer.send()`。`id` 来自 select 请求的 `id`（需暂存），`value` 来自 `selected_option_ids[0]`（单选）或 `free_text`。

**3. select 期间暂停输出：** 收到 select 后，session 进入"等待回答"状态——reader loop 暂停（不再调 `next_incoming`），直到 ChoiceResponse 到来。这样 Pi 的后续事件不会被消费（它们也不会来——Pi 在 `ctx.ui.select` 里阻塞）。

**4. ProviderChoice source：** 映射时 source 设为 `ChoiceRequestSource::ProviderChoice`（**不是** TextFallback）。这确保 `mapping.rs:316` 会 `register_choice`，使答案能回传到存活的 Pi 进程。

**5. include_str! 交付：** `const ARIA_ASK_EXTENSION: &str = include_str!("aria-ask.ts");`。运行时按内容哈希写入 Aria 缓存目录（如 `dirs::cache_dir()/cadence-aria/pi-ask-<sha8>.ts`），内容不变复用。`build_args` 加 `-e <该路径>`。

**6. 版本检测（task 2.3）：** 启动前调 `pi --version` 解析版本号。当前支持 `0.83.0+`。版本过低时 `start()` 返回可诊断错误（不 panic、不 spawn）。复用 Task 1 健康检查已记录的版本信息。

## Steps

- [ ] **Step 1: 写 `aria-ask.ts`** — 照 spike 验证过的扩展源码（`ask_user` 工具 + `ctx.ui.select` + `promptGuidelines`），确认内容与 spike 一致。

- [ ] **Step 2: 写失败测试 —— `build_args` 含 `-e`** — 断言 args 含 `-e` 且路径指向存在的缓存文件。`include_str!` 嵌入 + 缓存落盘逻辑实现在 `mod.rs`。

- [ ] **Step 3: 实现 `include_str!` + 缓存落盘 + `build_args` 加 `-e`** — `ensure_ask_extension()` 函数：按内容哈希写缓存目录，返回路径。`build_args` 调用它。

- [ ] **Step 4: 写失败测试 —— `parse_pi_select_request`** — 解析 `extension_ui_request(method=select)`，返回 `PiSelectRequest { id, title, options }`。

- [ ] **Step 5: 实现 `parse_pi_select_request`** — 在 `parse.rs` 加函数。

- [ ] **Step 6: 写失败测试 —— session select 往返（duplex）** — fake Pi 发 `extension_ui_request(select)` → 断言 session 发出 `ProviderEvent::ChoiceRequest(source=ProviderChoice)` 且题目/选项完整 → 发 `ProviderCommand::ChoiceResponse` → 断言 fake Pi 收到 `extension_ui_response(value)` → 断言 Pi 后续输出正常转发。

- [ ] **Step 7: 实现 session select 映射 + ChoiceResponse 回传** — 在两个 reader loop 加 select 分支；command loop 加 ChoiceResponse 分支；select 期间暂停输出消费。

- [ ] **Step 8: 写失败测试 —— 工具调用不产生授权请求** — fake Pi 发 `tool_execution_start` → 断言 session 不发 `ChoiceRequest` 或 `PermissionRequest`，直接发 `ProviderEvent::ToolCall`。

- [ ] **Step 9: 写失败测试 —— 版本检测** — mock `pi --version` 返回低版本 → 断言 `start()` 返回错误且不 spawn。

- [ ] **Step 10: 实现版本检测** — 在 `start()` 前检查版本（复用健康检查数据或单独探测），过低则返回可诊断错误。

- [ ] **Step 11: 更新 Pi prompt 指引** — `workspace_context/prompts.rs` 的 Pi 分支从「输出文本暂停信号」改为「使用 `ask_user` 工具提问并等待回答」。

- [ ] **Step 12: 全量测试 + Commit**

## 完成检查

- [ ] 2.2：`aria-ask.ts` 经 `include_str!` 交付；`ask_user → extension_ui_request(select) → ChoiceRequest(source=ProviderChoice)` 往返；题目/选项完整；`ChoiceResponse.value → extension_ui_response.value` 同进程续跑；普通工具不产生授权；Pi prompt 指引已更新。
- [ ] 2.3：版本检测覆盖；不兼容版本启动前报可操作错误；兼容/不兼容版本测试覆盖。
