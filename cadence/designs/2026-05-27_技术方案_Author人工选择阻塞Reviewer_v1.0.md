# Author 人工选择阻塞 Reviewer 技术方案

## 文档信息

- 文档类型：技术方案
- 日期：2026-05-27
- 版本：v1.0
- 分支：`product-workbench-issue-lifecycle`
- Worktree：`.worktrees/product-workbench-issue-lifecycle`
- 关联场景：真实 E2E 中 Story Spec author 询问 `climb_stairs(n)` 对 `n <= 0` 的处理方式后，系统错误进入 reviewer。

## 背景

真实 E2E 中，author 输出了需要人工确认的选项：

```text
climb_stairs(n) 对 n <= 0 应该如何处理？
A. 返回 0
B. 抛出 ValueError
C. 不定义该行为
```

当前 Workspace Engine 会把任何 author `Completed` 输出都视为候选产物，立即写入 Artifact、创建 checkpoint，并调用 `start_review_or_skip()`。因此这类人工问题会被当成 Story Spec 送入 reviewer，破坏了“author 需要用户确认时应停在 author 阶段”的产品流程。

## 目标

1. author 需要人工回答时，必须在 author 对话流中显示选择请求，不进入 reviewer。
2. 用户可以选择一个或多个选项，也可以输入自由文本。
3. 用户提交后，系统把回答作为补充上下文继续 author 生成。
4. 只有 author 生成有效候选产物后，才写 Artifact、创建 checkpoint 并进入 reviewer。
5. 结构化 `AskUserQuestion` / `requestUserInput` 仍作为首选通道；文本兜底只处理真实 provider 未走结构化事件的情况。

## 非目标

- 不改 Coding Workspace 的执行闭环。
- 不引入新的前端大型交互模式；复用现有 `choice_request` 气泡。
- 不把所有 open items 都强制阻塞，只有明显的人工选择问题阻塞 reviewer。

## 现状分析

已存在能力：

- `ProviderEvent::ChoiceRequest`、`ProviderCommand::ChoiceResponse`
- WebSocket `choice_request` / `choice_response`
- 前端 `ChoiceRequestEntry`
- Claude `AskUserQuestion` 与 Codex `item/tool/requestUserInput` 桥接

缺口：

- `complete_assistant_message()` 无条件把 author completed 输出推进到 reviewer。
- `workspace_context` prompt 未明确要求使用结构化 ask-user 工具。
- 缺少文本型人工问题的兜底识别和续跑机制。

## 设计

### 1. Prompt 约束

在 `src/web/workspace_context.rs` 的 workflow discipline 中明确：

- 如果需要用户选择或补充信息，必须使用结构化 ask-user 机制。
- 不得把“需要确认的问题 + 选项”作为最终候选产物输出。
- 如果 provider 环境无法发起结构化交互，才允许输出清晰的人工选择问题，daemon 会把它转换为选择卡片并暂停 reviewer。

### 2. 文本问题兜底识别

在 `src/product/workspace_engine.rs` 中新增一个小型解析器，只识别高置信模式：

- 输出包含问号或“需要确认/请选择/如何处理”
- 至少包含两个选项行，格式支持：
  - `A. ...`
  - `A、...`
  - `A) ...`
  - `1. ...`
- 输出不包含有效候选产物所需的关键 heading，例如 Story 的 `## 功能需求`、`## 成功标准`

命中后生成内部 `PendingAuthorChoice`，并通过已有 `EngineEvent::ChoiceRequest` 推送给前端。

### 3. 阻塞 reviewer

当 author completed 内容被识别为人工问题：

- 不调用 `extract_artifact_content`
- 不 `append_version`
- 不 `update_artifact`
- 不 `create_checkpoint`
- 不 `start_review_or_skip`
- 当前 `author_run` timeline node 标记为 `paused`
- Workspace stage 保持 `running`
- active run 清空，允许后续 choice response 触发新的 author run

### 4. Choice Response 续跑

当前 `choice_response` 只转发给 active provider run。文本兜底场景没有 active run，因此需要在 WebSocket handler 中增加分支：

- 若存在 active run，保持现有行为，转发给 provider。
- 若无 active run，则调用 engine 处理 pending author choice。
- engine 将选择结果写入一条 user/context 消息，例如：

```text
用户回答了 author 的确认问题：
问题：climb_stairs(n) 对 n <= 0 应该如何处理？
选择：A. 返回 0
补充：...
```

然后通过 `ProviderRunKind::Author` 重新启动 author。

### 5. UI 复用

前端继续使用 `ChoiceRequestEntry`。它已经支持：

- radio / checkbox
- free text
- resolved 状态
- 嵌入 author message group

本轮只需确保后端发送的 `choice_request` 能挂到当前 author node，前端即可显示在 author 对话流里。

## 错误处理

- 如果用户提交未知 choice id，返回 `CHOICE_ID_UNMATCHED`。
- 如果 pending author choice 已被取消或刷新丢失，返回协议错误并提示重新开始生成。
- 如果用户不选项但填写自由文本，允许继续。
- 如果 author 第二次仍输出人工问题，继续显示新的选择请求，不进入 reviewer。

## 测试策略

### 后端集成测试

新增 WebSocket 集成测试：

1. 使用 scripted author provider 输出 `climb_stairs` 人工选择问题。
2. 断言收到 `choice_request`。
3. 断言不会收到 `cross_review`、`reviewer_run` 或 `message_complete`。
4. 发送 `choice_response`。
5. 断言第二轮 author prompt 包含用户选择。
6. 第二轮 author 输出有效 Story Spec 后，才进入 reviewer 或 human confirm。

### 前端测试

现有 `ChoiceRequestEntry` 和 `useWorkspaceWs` 已覆盖结构化选择渲染和发送；如后端 payload 不新增字段，本轮不增加前端测试。

### 回归验证

- `cargo test --locked --test workspace_ws_integration <新增测试名>`
- `cargo test-approval-bridge`
- `cargo check --locked`

## 验收标准

- 真实 E2E 中 author 输出 A/B/C 人工问题时，页面显示 author 气泡内的选择卡片。
- 页面不进入 reviewer，不出现 reviewer 输出。
- 用户提交选择后，author 继续生成 Story Spec。
- 生成有效 Story Spec 后才进入 reviewer。
