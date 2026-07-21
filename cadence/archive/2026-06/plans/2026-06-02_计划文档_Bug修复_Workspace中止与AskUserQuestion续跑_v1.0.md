# Story Spec Workspace 两个 Bug 修复计划

> 文档类型：计划文档
> 创建日期：2026-06-02
> 版本：v1.0
> 分支：fix_e2e_test（worktree）

## 一、Bug 描述

1. **Bug 1（中止失效）**：story spec workspace 的「中止」按钮无法中止 claude code provider 的执行；codex 估计有类似问题。
2. **Bug 2（选择后卡住）**：story spec workspace 的 author 角色使用 claude code provider，当 claude 发起 AskUserQuestion 让用户选择、用户选择完成后就卡住，理论上应继续在同一 claude code provider 内执行。

## 二、根因分析（含本机 claude CLI v2.1.160 实测）

### Bug 2 —— control_response 回写格式错误（确定性根因，已实测确认）

claude 在 `--permission-prompt-tool=stdio` 模式下，AskUserQuestion 走 `control_request{subtype:"can_use_tool", tool_name:"AskUserQuestion"}`（实测确认，与用户「看到选项卡片」一致）。cadence 解析输入侧正确，但**回写 control_response 的格式 claude 不认**，导致 claude 永久卡住。

- ❌ cadence 现状（`src/cross_cutting/claude_code_provider.rs:211-248`）：

    ```json
    {"type":"control_response","request_id":"<id>","response":{"behavior":"allow","updatedInput":{...}}}
    ```

- ✅ claude 要求的 SDK 格式（从 claude 自己发的 control_response 反推，并实测通过）：

    ```json
    {"type":"control_response","response":{"subtype":"success","request_id":"<id>","response":{"behavior":"allow","updatedInput":{...}}}}
    ```

- 差异：`request_id` 必须移入 `response` 内层；必须加 `subtype:"success"`；真正 payload 再嵌一层 `response`。
- 实测对比：SDK 格式回写后 claude 立即继续（输出「你选了茶」+ result is_error=false）；cadence 格式回写则 120s 零输出卡死。
- 影响范围：`write_control_response`（普通工具批准/拒绝）与 `write_choice_control_response`（AskUserQuestion）两个函数结构相同，**都错**。
- 测试缺口：`tests/fixtures/provider/claude_stream_json_fixture.sh:20` 仅用 `*'"control_response"'*` 宽松匹配，不校验结构，因此 bug 从未被测出。

### Bug 1 —— 中止失效（机制正确，待真实复现定位）

- 实测确认 `command_group` 的 `killpg(pgid, SIGKILL)` 能完整杀掉 claude 主进程 + 所有 MCP 子进程（node/uv/python），无残留。
- cancel 传播链代码完整：`run.cancel → engine.cancel(use_run_token) → provider.start(cancel) → read_claude_stream / bridge`。
- engine `drive_provider_session` 的 abort/cancel 分支齐全。
- **疑点**：bug2 导致 claude 卡死后，run 处于「等待用户选择」状态。此时中止：engine 先发 `Abort` 给 bridge → `request_choice` 返回 aborted-decision（非 Err）→ provider 走 `write_choice_control_response`（错误格式，claude 不退）→ 回顶层 loop → cancel 分支命中 → 返回 Aborted → `start_kill`。理论上仍能 kill。
- **结论**：bug1 的真实表现（子进程没死 / engine 没回中止态 / 前端按钮问题）需在真实服务复现确认，不盲改。很可能 bug1 的「等待选择时中止不了」本质是 bug2 的副作用，修完 bug2 后需重新验证。

### Codex（次要，待验证）

codex 走 JSON-RPC（不同协议），choice/permission 回写经 `write_approval_response` / `write_user_input_response`。需单独对照真实 codex app-server 协议验证。

## 三、修复方案

### 阶段 1：Bug 2 确定性修复（优先，TDD）

1. **先加回归测试（红）**：
   - 强化 `claude_stream_json_fixture.sh`：校验收到的 control_response 必须是 SDK 格式（`response.subtype=="success"`、`response.request_id` 存在、payload 在 `response.response`），不符则报错退出。
   - 新增 fixture + 测试模拟 AskUserQuestion（can_use_tool）交互，断言 cadence 写回 SDK 格式后 fixture 能继续到 result。
   - 普通工具批准/拒绝同样覆盖。
   - 运行测试 → 当前实现应失败（红）。
2. **修实现（绿）**：
   - 改 `write_control_response`（:211）与 `write_choice_control_response`（:229）为 SDK 格式。
   - 运行测试 → 通过（绿）。

### 阶段 2：真实服务复现 + Bug 1 定位

3. 启动 aria 服务 + 真实 claude code，复现：
   - 验证 bug2 修复：AskUserQuestion 选择后能在同一 provider 继续执行。
   - 复现 bug1：观察中止时子进程是否被杀、engine 是否回中止态、前端按钮行为。
4. 根据复现结果对 bug1 定向修复（若 bug2 修复后 bug1 消失则确认为副作用）。

### 阶段 3：Codex 验证（按需）

5. 对照真实 codex app-server 协议验证 codex 的 abort 与 choice 回写，必要时修复。

## 四、验证方式

- 单元/集成测试：`cargo test --locked`（遵循项目命令规范，禁止 `-j 1`）。
- 定向：`cargo test --locked --lib claude` 跑 provider 测试。
- 端到端：用户同意跑真实服务，启动 aria + 真实 claude code 确认两个 bug。

## 五、待确认项

- bug1 是否在 bug2 修复后自动消失（需真实复现）。
- codex 是否需要同步修复（用户提到「估计也有」）。
