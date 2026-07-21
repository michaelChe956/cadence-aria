# Story Spec Workspace 中止与 AskUserQuestion 续跑 Bug 修复计划

> 文档类型：计划文档
> 创建日期：2026-06-02
> 版本：v1.1
> 分支：fix_e2e_test（worktree）

## 一、Bug 描述

1. **Bug 1（中止失效）**：Story Spec Workspace 的「中止」按钮无法中止 Claude Code provider 执行；Codex 可能存在类似问题。
2. **Bug 2（选择后卡住）**：Story Spec Workspace 的 author 角色使用 Claude Code provider 时，Claude 发起 `AskUserQuestion` 后，用户选择完成会卡住；预期应在同一个 Claude Code provider 进程内继续执行。

## 二、当前进度与事实约束

- 当前 `fix_e2e_test` worktree 与 `main` 指向同一提交：`01588b6 fix: abort workspace provider runs across reconnects (#17)`。
- 该提交已新增 workspace active run registry 与跨连接 abort 逻辑，因此 Bug 1 不能按旧链路重复分析，必须覆盖当前代码下的真实 abort/cancel 竞争路径。
- 当前 `ApprovalBridge::request_choice` 在 cancel token 触发时会返回 `Err("choice request cancelled")`；只有 `ProviderCommand::Abort` 先到达 bridge 时，pending choice 才会转成 `ChoiceDecision { free_text: Some("aborted") }`。
- 因此 Bug 1 的回归测试必须覆盖：
  - WebSocket abort 能让 workspace 回到 `prepare_context`；
  - provider 事件流不能继续完成 partial assistant message；
  - Claude fixture 进程或 provider event channel 必须关闭；
  - 如 abort 与 choice pending 竞争，不能因为写回错误格式导致外部 provider 残留。

## 三、根因分析

### Bug 2：Claude control_response 回写格式错误（确定性根因）

Claude 在 `--permission-prompt-tool=stdio` 模式下，`AskUserQuestion` 会走：

```json
{"type":"control_request","request_id":"<id>","request":{"subtype":"can_use_tool","tool_name":"AskUserQuestion","input":{...}}}
```

当前 Cadence 写回格式为：

```json
{"type":"control_response","request_id":"<id>","response":{"behavior":"allow","updatedInput":{...}}}
```

Claude SDK 实际要求的 control response 格式为：

```json
{"type":"control_response","response":{"subtype":"success","request_id":"<id>","response":{"behavior":"allow","updatedInput":{...}}}}
```

需要同时修复：

- `write_control_response`：普通工具批准/拒绝。
- `write_choice_control_response`：`AskUserQuestion` 用户选择回写。

当前测试缺口：

- `tests/fixtures/provider/claude_stream_json_fixture.sh` 只匹配是否包含 `control_response`，没有校验 JSON 结构，因此旧实现也会通过。

### Bug 1：中止失效需基于当前代码重验

当前链路已有：

- WebSocket `Abort` 调用 `abort_active_run`；
- `abort_active_run` 优先向 active run 的 `command_tx` 发送 `ProviderCommand::Abort`，再 cancel run token；
- engine 收到 `ProviderCommand::Abort` 后向 provider session 转发 abort、cancel 当前 run，并进入 aborted finish；
- Claude provider 在 `read_claude_stream` 返回 aborted 或 error 时调用 `start_kill`。

待验证风险：

- 当 Claude 正停在 `AskUserQuestion` pending 状态时，abort 可能先触发 bridge 的 aborted choice，也可能先触发 cancel token。
- 若走 aborted choice 路径，provider 会尝试向 Claude 写回一次 choice control_response；旧格式可能导致 Claude 不退出或延迟退出。
- 修复 Bug 2 后，应验证中止不会被错误 control_response 放大成 provider 残留。

## 四、实施计划

### 阶段 1：TDD 修复 Claude control_response 格式

1. 新增 Rust 单元测试，精确断言 control_response payload：
   - 普通工具 approve 的结构必须是 SDK 格式；
   - 普通工具 deny 的结构必须是 SDK 格式，deny reason 位于内层 payload；
   - `AskUserQuestion` 的 choice response 必须是 SDK 格式，`updatedInput.answers` 保留用户回答。
2. 先运行定向测试，确认当前实现红灯。
3. 实现最小修复：
   - 提取纯 JSON helper 构造 control_response；
   - `write_control_response` 和 `write_choice_control_response` 只负责调用 helper 并写入 stdin。
4. 运行同一组定向测试，确认绿灯。

### 阶段 2：新增 Claude AskUserQuestion 续跑集成回归

1. 新增或内联一个 Claude fixture：
   - 收到 user input 后输出 `AskUserQuestion` control_request；
   - 校验收到的 control_response 必须符合 SDK 格式；
   - 校验 `updatedInput.answers` 包含用户选择；
   - 校验通过后输出 `result` 并退出。
2. 新增 provider 级测试：
   - 启动 ClaudeCode provider；
   - 等待 `ProviderEvent::ChoiceRequest`；
   - 发送 `ProviderCommand::ChoiceResponse`；
   - 断言同一 provider session 输出 `Completed`。

### 阶段 3：中止回归覆盖

1. 新增 provider 级测试：
   - Claude fixture 发出 `AskUserQuestion` 后挂起；
   - 测试侧发送 `ProviderCommand::Abort` 或 cancel；
   - 断言 provider event channel 关闭或进入 aborted，不输出 completed。
2. 若需要，再新增 WebSocket 层测试：
   - 使用 author provider 为 Claude fixture 的 workspace；
   - 等待 `ChoiceRequest`；
   - 发送 `WsInMessage::Abort`；
   - 断言 stage 回到 `prepare_context`，没有 `MessageComplete`。
3. 若 Bug 2 修复后 Bug 1 无法复现，则将 Bug 1 结论记录为“由错误 control_response 在 pending choice 场景下放大导致”，保留回归测试防止后续退化。

### 阶段 4：Codex 验证

1. 不复用 Claude 的 SDK control_response 结构到 Codex。
2. 对照当前 Codex JSON-RPC fixture 与真实 app-server 协议，确认：
   - choice response 能继续同一 session；
   - abort 能关闭 provider event stream；
   - 若存在协议差异，再单独修复。

## 五、验证命令

必须遵循项目规则，直接使用宿主机 cargo，不使用 Docker，不加 `-j 1`。

定向验证：

```bash
cargo test --locked --lib control_response_uses_sdk
cargo test --locked --lib claude_provider
```

必要集成验证：

```bash
cargo test --locked --test workspace_ws_integration workspace_ws
```

最终验证：

```bash
cargo fmt --check
cargo check --locked
cargo test --locked
```

## 六、验收标准

- Claude 普通工具 permission response 与 `AskUserQuestion` choice response 均写回 SDK 格式。
- 用户选择 `AskUserQuestion` 后，同一个 Claude Code provider session 能继续并完成。
- pending choice 状态下中止不会产生 completed assistant message，workspace 回到可继续操作状态。
- 已有跨连接 abort 回归测试继续通过。
- Codex 不因 Claude 修复引入协议变更。
