# CodingWorkspace Group Unit 气泡展示问题记录

## 背景

在 group coding attempt 中，一个 attempt 会串行执行多个 work item。每个 work item 对应一个 group unit。当前 group unit 在 Code Reviewer 通过后会完成提交、生成 handoff、标记当前 unit 完成，并切换到下一个 unit。

## 当前问题

页面聊天气泡只展示 Coder / Code Reviewer 等 provider 消息，group unit 的关键状态没有明确气泡展示。用户在端到端测试时会看到 Code Reviewer 结束后到下一个 Coder 开始之间存在等待，但页面没有说明这段时间正在做 group unit 收尾。

## 后续目标

在 Coding Workspace 页面增加 group unit 相关的气泡 message，至少展示：

- 当前 work item 完成收尾开始。
- 当前 work item handoff 已生成。
- 当前 group unit 已完成并切换到下一个 work item。
- 下一个 work item 的 Coder 即将开始。

## 约束

- group unit 机制本身保留，不作为旧概念删除。
- 只补齐页面可见性，避免把 group unit 误解为新的 Agent 角色。
- 气泡文案应明确这是系统状态 / group unit 状态，不应显示为 Coder 或 Code Reviewer 输出。
