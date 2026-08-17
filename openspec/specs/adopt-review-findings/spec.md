# adopt-review-findings Specification

## Purpose
AuthorConfirm 阶段为存在 review 报告的 Story/Design 会话提供一键带入报告文本的反馈辅助入口，消除手动复制摩擦。

## Requirements

### Requirement: 一键带入 review 报告

Story/Design workspace 处于 AuthorConfirm 阶段且对话流存在 review 报告时，用户可将报告文本一键带入反馈输入框；带入仅为预填，发送仍由用户显式完成。

#### Scenario: 带入最新 review 报告
- **WHEN** AuthorConfirm 阶段对话流存在 review 报告，用户点击「采纳 Review 意见」按钮
- **THEN** 反馈输入框被预填为引导语 + 最新 review 报告的格式化文本（与对话流渲染同源），用户可编辑后点「发送反馈」触发既有修订流程

#### Scenario: 无 review 报告时按钮不可见
- **WHEN** AuthorConfirm 阶段对话流不存在任何 review 报告
- **THEN** 不展示「采纳 Review 意见」按钮，输入区交互与既有行为完全一致

#### Scenario: 带入后清空可重复带入
- **WHEN** 用户带入后手动清空输入框并再次点击按钮
- **THEN** 再次预填同一最新报告文本，无重复拼接
