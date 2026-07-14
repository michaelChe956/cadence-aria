# Coding Runner 重复启动交互缺陷分析

## 文档信息

- 日期：2026-07-14
- 模块：Coding Workspace
- 现场 Attempt：`coding_attempt_0001`
- 现场错误：`coding_runner_already_started: coding runner is already active for this socket`
- 当前结论：非阻断错误，后端互斥行为正确，但前端交互和错误表达容易让用户误判流程失败。

## 现场现象

用户在 Work Item 6 收尾恢复后，多次点击【继续 Coding】。第一次点击成功启动 runner，并完成以下动作：

1. 提交 Work Item 6，commit 为 `76428a098fc740d698f705892389cdf075645a14`。
2. 生成 Unit 6 handoff。
3. 把 `coding_unit_0006` 更新为 `completed`。
4. 激活 `coding_unit_0007`。
5. 启动 Work Item 7 Coder。

第二次点击发生时，同一 WebSocket 已存在活动 runner，后端互斥保护返回：

```text
coding_runner_already_started: coding runner is already active for this socket
```

现场检查确认 Work Item 7 的 `coding_node_0029` 和 `coding_role_run_0028` 正在运行，因此该错误没有阻断第一次点击启动的流程。

## 问题判断

后端拒绝重复 runner 是必要保护，避免同一个 Attempt 并发启动两个流水线。真正的问题位于用户体验层：

1. 用户点击后，按钮没有足够快地进入禁用或 loading 状态，允许连续点击。
2. 重复启动被展示为通用错误，用户无法判断第一次请求是否已经成功。
3. 文案没有说明“现有 Coding 正在继续，本次重复操作已忽略”。
4. UI 没有同时突出当前运行节点，使互斥提示看起来像新的流程失败。

## 后续优化建议

### 前端

1. 用户点击【开始 Coding】或【继续 Coding】后立即本地禁用按钮，不等待 WebSocket round-trip。
2. 在收到 runner 活跃、Coding 节点 running 或角色执行 running 状态后持续禁用按钮。
3. runner 结束、进入人工 gate、失败或连接重建并确认无活动 runner 后再恢复按钮。
4. 将重复点击提示改为非阻断信息：

   ```text
   Coding 已在执行，本次重复操作已忽略。
   ```

5. 提示旁同时展示当前 Work Item、节点和角色，例如“Work Item 7 · Coder 正在运行”。

### 后端与 WebSocket 协议

1. 保留 `CodingRunRegistry` 的互斥保护。
2. 对同一 socket、同一 attempt 的重复 start 命令考虑返回幂等确认或当前 session state，而不是 fatal/protocol error 语义。
3. 如果继续保留错误码，应增加 `recoverable` 或 `already_running` 分类，便于前端选择 toast 等级和文案。
4. 不得因为重复点击中止、替换或重启已经存在的 runner。

## 回归测试建议

1. 前端快速双击只发送一次 start 命令。
2. start 命令发出后按钮同步进入 disabled/loading。
3. 收到 `coding_runner_already_started` 时显示非阻断提示，不把 Attempt 标记为失败。
4. 重复 start 后，原 runner 继续产出 timeline、role-run 和最终状态。
5. WebSocket 重连后，如果 runner 仍活跃，按钮继续保持禁用。
6. runner 结束或进入人工 gate 后，按钮按真实 session state 恢复可用。

## 验收标准

- 连续点击不会启动第二个 runner。
- 用户不会看到含义模糊的失败提示。
- 页面明确显示已有 Coding 正在运行。
- 重复操作不会影响原 runner、当前 Work Item 或后续 Unit 推进。
- 前端和后端测试同时覆盖重复点击、重连和 runner 完成三个阶段。
