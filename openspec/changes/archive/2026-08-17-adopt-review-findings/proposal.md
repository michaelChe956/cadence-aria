# Proposal: adopt-review-findings

## Why

spec-design-dialog-revision 落地后，review 报告作为建议进入对话流，author 与 reviewer 是独立 provider 会话——用户采纳 review 意见时必须手动复制/转述 findings 到反馈框，体验摩擦明显，导致"一键采纳"缺失。

## What Changes

- AuthorConfirm 阶段（Story/Design），当对话流存在 review 报告时，输入框旁新增**「采纳 Review 意见」快捷按钮**：点击后将最新 review 报告的格式化文本预填入反馈输入框（用户可编辑/删减后手动点「发送反馈」），不自动发送。
- 预填格式：引导语（"按以下 review 意见修订："）+ 报告全文（与对话流渲染同源）。
- 按钮仅预填，不改变既有 Revise 通道语义（空反馈仍被拒绝；反馈仍为纯自由文本）。

## Capabilities

### New Capabilities

- `adopt-review-findings`: AuthorConfirm 阶段一键带入 review 报告文本至反馈输入框。

### Modified Capabilities

（无——`spec-design-dialog-revision` 的 Requirement 均不受影响，本 change 仅新增前端辅助入口。）

## Impact

- 受影响代码：`web/src/components/chat-workspace/ChatInputBar.tsx`（按钮 + 预填）、前端 store/类型（取最新 review 报告文本的现有数据源接线）、`ChatWorkspacePage.tsx`（props 传递）；无后端改动。
- 非目标：不做"一键直接发送"（保留用户编辑窗口）；不做逐条勾选采纳；不改 WS 协议。
