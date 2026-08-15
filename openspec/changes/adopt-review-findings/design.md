# Design: adopt-review-findings

## 1. 数据源

前端取"最新 review 报告文本"：与对话流 ReviewVerdictEntry 渲染同源（前端 store 中最新 review verdict 消息 / chat rebuild 产物）。实现时确认 `workspace-ws-store` 与 `workspace-chat-rebuild.ts` 中的现有数据形状，选择最小取数路径；若 store 仅存消息流，则从消息流取最后一条 review 报告消息的原始内容。无后端改动、无协议改动。

## 2. 交互

- `ChatInputBar`（author_confirm 分支）新增 `latestReviewReport?: string` prop；非空时在「发送反馈」按钮左侧渲染「采纳 Review 意见」次级按钮。
- 点击行为：`setInput("按以下 review 意见修订：\n\n" + latestReviewReport)`（整体覆盖式预填，非追加——天然满足"清空后可重复带入不拼接"）。
- 不自动发送、不自动 focus 以外的副作用；空反馈拒绝与纯文本通道语义不变。

## 3. 决策记录

| 决策点 | 选择 | 理由 |
|---|---|---|
| 带入 vs 直接发送 | 预填可编辑 | 保留用户筛选/改写权（reviewer 只是建议，拍板在人） |
| 覆盖式 vs 追加式预填 | 覆盖式 | 重复点击不产生拼接；spec Scenario 3 |
| 按钮显示条件 | 对话流存在 review 报告即显示（不追踪"已处理"） | YAGNI；用户心智简单（带入的永远是最新报告） |

## 4. 测试

- 组件测试（ChatInputBar.test.tsx）：带入预填内容与格式、无报告时按钮不渲染、清空后重复带入一致。
- 回归：既有三动作用例不受影响；`cd web && pnpm tsc -b && pnpm test`。
