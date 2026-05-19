# product-workbench-issue-lifecycle 正式 E2E 问题记录

## 文档信息

- 文档类型：状态记录
- 日期：2026-05-19
- 版本：v1.0
- 分支：`product-workbench-issue-lifecycle`
- Worktree：`.worktrees/product-workbench-issue-lifecycle`
- 测试目标仓库：`/home/michael/workspace/github/naruto`
- 测试场景：真实 provider 正式 E2E，`Claude Code` author + `Codex` reviewer

## 发现的问题

### 1. PrepareContext 阶段发送补充约束会直接触发 Story Spec 生成

现象：

- 用户从 Issue `爬楼梯问题` 创建 Story Spec Workspace。
- 用户希望在正式执行前额外补充约束，例如明确 Python 函数签名和验收值。
- 在 `准备上下文` 阶段，用户把补充约束填入输入框并点击发送。
- 页面没有停留在 `准备上下文`，而是直接进入 Story Spec 生成流程。

预期：

- `准备上下文` 阶段的输入框应支持补充上下文。
- 发送补充上下文后应继续停留在 `准备上下文`。
- `准备上下文` Timeline 节点应记录或汇总该补充消息。
- Provider 配置仍应保持可修改。
- 只有点击显式的 `开始生成` 按钮时，才应进入生成阶段并锁定 Provider 配置。

实际影响：

- 输入框同时承担“补充上下文”和“开始执行”两个语义，正式流程中容易误触发生成。
- 用户无法在开始生成前可靠补充约束。
- 正式 E2E 中无法验证“Issue 上下文 + 用户补充上下文 + Provider 配置锁定”的完整准备阶段语义。
- 该问题会影响真实 provider 测试成本，因为误触发后会直接调用 `Claude Code` author。

初步判断：

- 当前 WebSocket 入站消息 `user_message` 在 `prepare_context` 阶段被后端视为执行触发。
- 新 UI 已增加 `开始生成` 按钮，但输入框发送消息的语义尚未与“开始生成”解耦。

建议修复方向：

- 将 `prepare_context` 阶段的普通输入改为“追加上下文”行为，不启动 provider。
- 保留 `开始生成` 作为唯一执行触发入口。
- 或新增独立 WebSocket 消息类型，例如 `context_message` / `append_context`，与 `user_message` 执行语义分离。
- 前端在 `prepare_context` 阶段发送输入时使用上下文追加语义；点击 `开始生成` 时再发送执行触发。

### 2. 刷新页面后无法终止正在生成 Story Spec 的 Claude Code

现象：

- Story Spec 生成过程中，`Claude Code` author 正在执行。
- 用户刷新浏览器页面后重新进入同一个 Workspace。
- 页面仍显示生成相关状态，但用户无法有效终止原来的 `Claude Code` 执行。

预期：

- 刷新页面后，前端应能恢复当前运行中的 provider run 状态。
- 如果后端仍有 active run，`中止` 操作应绑定到原 run，并能向对应 provider 发送 abort/cancel。
- 如果刷新导致 WebSocket run 控制通道丢失，后端应在重连时明确标记该 run 已不可控或已被取消，不能展示一个看似可中止但实际无效的状态。

实际影响：

- 真实 provider 可能继续消耗时间和 token。
- 用户失去对长时间运行任务的控制。
- 正式 E2E 中无法可靠验证中止和恢复行为。

初步判断：

- 当前 active provider run 的控制句柄可能只存在于单个 WebSocket handler 进程内存中。
- 浏览器刷新后建立了新的 WebSocket 连接，但新的连接无法接管旧连接中的 `current_run` / command channel。

建议修复方向：

- 将 active run 控制权提升到 session 级 registry，而不是绑定到单个 WebSocket 连接。
- 或在 WebSocket 断开时自动取消当前 provider run，并将 Workspace 状态恢复为可重新执行状态。
- 重连时应返回明确的 provider run 状态和可用操作，不允许 UI 展示无效的 `中止`。

### 3. 刷新页面后原来的流式输出丢失

现象：

- Story Spec 生成过程中，页面已有 `Claude Code` 流式输出。
- 用户刷新页面后重新进入同一个 Workspace。
- 原先已经显示过的输出内容不再可见。

预期：

- 已产生的流式输出应按 Timeline node 持久化，至少能够恢复到最后一次已接收的文本片段。
- 刷新后 `SessionState` 应包含当前节点的历史输出、消息或可恢复的 node detail。
- 如果输出尚未完成，也应展示“已输出部分 + 当前 run 状态”，而不是清空。

实际影响：

- 用户无法审计刷新前已经发生的 provider 行为。
- Timeline 作为执行归集视图的核心价值受损。
- 正式 E2E 无法验证“刷新恢复 session state”对运行中真实 provider 的可观测性。

初步判断：

- `SessionState` 当前恢复了 `timeline_nodes` 和 `artifact_versions`，但运行中节点的 `streaming_content` / node detail 可能仅保存在前端内存中。
- `stream_chunk` 只通过当前 WebSocket 推送，没有增量持久化到 lifecycle store。

建议修复方向：

- 为 Timeline node detail 增加持久化存储，至少保存 `streaming_content` 和 `execution_events`。
- 收到 provider 文本增量时增量写入当前 node detail。
- `SessionState` 增加 node detail 快照，前端刷新后按 node_id 恢复详情面板。

### 4. Claude Code 授权确认按钮点击后授权不生效

现象：

- Story Spec 生成过程中，`Claude Code` 需要授权。
- 页面弹出授权按钮。
- 用户点击确认/允许后，授权没有生效，provider 执行没有按预期继续。

预期：

- 用户点击允许后，前端应发送 `permission_response`。
- 后端应把授权响应转发给当前 `Claude Code` provider run 的 command channel。
- provider 应继续执行，授权卡片应消失或变更为已处理状态。

实际影响：

- 真实 provider 流程会卡在授权点。
- 正式 E2E 无法完成 `Claude Code` author 生成。
- 用户无法区分是授权未发送、后端未转发、还是 provider 未消费授权响应。

初步判断：

- 如果发生在刷新之后，可能与问题 2 同源：新的 WebSocket 连接无法访问旧 provider run 的 command channel。
- 如果未刷新也复现，则需要进一步检查 `permission_response` 的 id、approved 字段、后端转发路径和 Claude Code provider 的 permission bridge。

正式 E2E 复现补充：

- 2026-05-19 真实 provider 测试中，在未刷新页面的情况下也复现了该问题。
- 用户点击授权确认后，生成流程仍卡住，没有继续输出。
- 随后点击 `中止` / 停止按钮也无法停止该 Story Spec 生成。
- UI 仍显示运行中状态，说明授权响应和中止响应至少有一个没有正确传递到当前 `Claude Code` provider run，且前端没有收到有效的状态收敛事件。

建议修复方向：

- 先区分“刷新后授权无效”和“未刷新授权无效”两类复现路径。
- 为 `permission_response` 增加可观测日志或 Timeline execution event，记录前端已发送、后端已接收、provider 已消费三个阶段。
- 将 permission command channel 与 active run registry 持久关联，避免刷新后丢失授权控制能力。
- 同时为 `abort` 增加相同级别的可观测链路，确认前端点击、后端收到、provider command channel 收到、provider 进程退出、Workspace 状态更新五个阶段。

## 当前测试处置

- 以上问题先记录为正式 E2E 发现，不在当前步骤继续扩大修复。
- 真实 provider 正式 E2E 已暴露运行中刷新恢复、授权控制和上下文输入语义问题，不应视为通过。
- 如果继续执行正式 E2E，建议暂时不要刷新运行中的 Workspace，也不要在输入框补充上下文；但这只能作为临时绕行，不能作为正式验收通过依据。
- 若正式验收范围包含运行中刷新恢复、授权、中止和上下文补充，应先修复上述问题后重新执行真实 provider 门禁。
