# Spec Workspace Review 三态 Gate 优化技术方案

## 背景

当前 Story Spec、Design Spec、Work Item 共用 `WorkspaceEngine` 的 review gate 链路。Reviewer 完成后，后端解析末尾 JSON，按 findings 严重性把结果分为：

```text
requires_revision      -> review_decision
user_confirm_allowed   -> human_confirm
```

这解决了“普通建议不应阻塞用户确认”的问题，但真实 Work Item E2E 暴露出两个产品语义缺口：

1. `verdict=revise` 但 reviewer 没有输出结构化 findings 时，当前会降级为 `needs_human/user_confirm_allowed`，页面容易表达成“可确认当前版本”，弱化了 reviewer 明确想返修的意图。
2. `minor/optional/suggestion` 这类非阻塞建议目前只能由用户通过输入框手动要求修改，缺少清晰的“采纳建议并返修”路径。

## 目标

- 保留强阻断问题自动进入返修决策的现有能力。
- 非阻塞建议继续允许用户确认当前版本。
- 对非阻塞建议提供显式“采纳建议并返修”操作。
- 对非结构化 `revise` 进入人工裁决态，既不自动返修，也不伪装为完全通过。
- `review_complete` 实时事件携带完整 `findings` 与 `review_gate`，避免刚完成 review 时 UI 信息不完整。
- Story、Design、Work Item 三类 workspace 使用同一套后端分类、前端展示和回归测试。

## 非目标

- 不取消 reviewer，不降低 review 轮次。
- 不改变 Coding Workspace 独立 Code Review 流程。
- 不让前端绕过后端状态机直接确认阻塞性 review。
- 不在本轮重构完整 timeline/detail hydration 体系。

## 方案总览

将 Review Gate 从二态扩展为三态：

```text
Reviewer Output
      |
      v
parse ReviewVerdict JSON
      |
      v
ReviewGate classifier
      |
      +-- requires_revision -----> review_decision
      |                             接受修订建议 / 补充上下文后修订 / 跳过人工处理
      |
      +-- user_confirm_allowed --> human_confirm
      |                             确认当前版本 / 采纳建议并返修 / 终止
      |
      +-- user_triage_required --> human_confirm
                                    按 reviewer 意见返修 / 确认当前版本 / 终止
```

核心原则：

- Reviewer 的强问题可以阻断。
- Reviewer 的弱建议只能建议。
- Reviewer 的非结构化返修意图必须交给用户裁决。

## Review Gate 定义

扩展 `ReviewGate`：

```rust
pub enum ReviewGate {
    RequiresRevision,
    UserConfirmAllowed,
    UserTriageRequired,
}
```

序列化值：

| Rust | JSON | 含义 |
| --- | --- | --- |
| `RequiresRevision` | `requires_revision` | 存在明确强返修 finding，系统应阻断继续确认 |
| `UserConfirmAllowed` | `user_confirm_allowed` | 产物可确认，reviewer 仅给出非阻塞建议或通过 |
| `UserTriageRequired` | `user_triage_required` | reviewer 表达返修或输出异常，但缺少可自动归类的结构化强 finding |

## 分类规则

Review JSON 仍使用现有合约：

```json
{
  "verdict": "pass|revise|needs_human",
  "summary": "一句话摘要",
  "findings": [
    {
      "severity": "blocking|must_fix|strong_recommend_fix|suggestion|minor|optional",
      "message": "问题描述",
      "evidence": "当前产物中的具体证据",
      "impact": "为什么影响或不影响下一阶段",
      "required_action": "需要作者执行的最小动作"
    }
  ]
}
```

分类规则：

| 输入情况 | 归一化 verdict | review_gate | 阶段 |
| --- | --- | --- | --- |
| 存在 `blocking/must_fix/strong_recommend_fix` | `revise` | `requires_revision` | `review_decision` |
| `verdict=pass`，没有强 finding | `pass` | `user_confirm_allowed` | `human_confirm` |
| `verdict=needs_human`，没有强 finding | `needs_human` | `user_triage_required` | `human_confirm` |
| `verdict=revise`，只有 `suggestion/minor/optional` | `needs_human` | `user_confirm_allowed` | `human_confirm` |
| `verdict=revise`，没有 findings | `needs_human` | `user_triage_required` | `human_confirm` |
| `verdict=revise`，findings 字段存在但无法解析为有效 finding | `needs_human` | `user_triage_required` | `human_confirm` |
| reviewer 输出无可解析 JSON | `needs_human` | `user_triage_required` | `human_confirm` |

说明：

- `verdict=revise` 但只有弱建议，说明 reviewer 希望改进但没有发现阻塞问题，用户应能直接确认，也能一键采纳建议返修。
- `verdict=revise` 且没有 findings 时，系统无法知道返修边界，不能自动推进 revision，应让用户裁决。
- 不再把不可解析输出默认视为“可确认当前版本”，避免 UI 误导。

## 后端状态机

`complete_review()` 继续负责唯一分流。

```text
ReviewGate::RequiresRevision:
  - 持久化完整 verdict/findings/review_gate
  - ArtifactVersion.review_verdict = revise
  - timeline reviewer_run = completed
  - stage = review_decision
  - 创建 review_decision paused node
  - 发送 review_decision_required

ReviewGate::UserConfirmAllowed:
  - 持久化完整 verdict/findings/review_gate
  - ArtifactVersion.review_verdict = pass 或 needs_human
  - timeline reviewer_run = completed
  - stage = human_confirm
  - 创建 human_confirm active node

ReviewGate::UserTriageRequired:
  - 持久化完整 verdict/comments/summary/review_gate
  - ArtifactVersion.review_verdict = needs_human
  - timeline reviewer_run = completed
  - stage = human_confirm
  - 创建 human_confirm active node
```

`human_confirm` 的 `request-change` 继续复用现有 revision 链路：

- 用户点击“采纳建议并返修”时，前端把 optional findings 整理为 `payload.description`。
- 用户点击“按 reviewer 意见返修”时，前端把 reviewer comments 与 summary 整理为 `payload.description`。
- 后端已有 `human_confirm_payload_description()` 可消费该 description 并写入 `pending_revision_context`。

## WebSocket 合约

扩展 `ReviewComplete` 输出：

```json
{
  "type": "review_complete",
  "node_id": "timeline_node_004",
  "round": 1,
  "verdict": "pass|revise|needs_human",
  "comments": "...",
  "summary": "...",
  "findings": [],
  "review_gate": "requires_revision|user_confirm_allowed|user_triage_required"
}
```

兼容性：

- 新字段为附加字段，旧前端忽略不破坏。
- 前端 store 在实时事件中直接创建带 findings/review_gate 的 `review_verdict` entry。
- 刷新恢复仍以 `NodeDetail.verdict` 为最终完整来源。

## 前端交互

### ReviewVerdictEntry

标题规则：

| review_gate | 标题 |
| --- | --- |
| `requires_revision` | 需要解决后再继续 |
| `user_confirm_allowed` | 可确认当前版本 |
| `user_triage_required` | 需要判断 reviewer 意图 |

Findings 分组不变：

```text
需要解决:
- blocking
- must_fix
- strong_recommend_fix

可选建议:
- suggestion
- minor
- optional
```

### GatePromptEntry

`human_confirm` 阶段按 gate 展示操作：

| review_gate | 主按钮 | 次按钮 |
| --- | --- | --- |
| `user_confirm_allowed` | 确认使用当前版本 | 采纳建议并返修、终止 |
| `user_triage_required` | 按 reviewer 意见返修 | 确认当前版本、终止 |
| 无 review_gate | 确认产物 | 终止 |

“采纳建议并返修”与“按 reviewer 意见返修”都发送：

```json
{
  "type": "human_confirm",
  "decision": "request-change",
  "payload": {
    "description": "..."
  }
}
```

### review_decision 阶段

强阻断路径保留现有三按钮：

- 接受修订建议
- 补充上下文后修订
- 跳过，人工处理

## Prompt 调整

Reviewer prompt 增加两条明确规则：

- 如果输出 `verdict=revise`，必须给出至少一个结构化 finding；否则系统会进入人工裁决，而不是自动返修。
- 如果只有风格、措辞、补充说明、未来增强或非必要优化，请输出 `pass` 或 `needs_human`，并把 finding 标为 `suggestion/minor/optional`。

这能减少 provider 输出 “revise 但无 findings” 的概率，但后端仍必须防御该情况。

## Story/Design/Work Item 联动

本变更作用于共享 `WorkspaceEngine`、WebSocket contract、chat workspace store 与 entry 组件，因此 Story、Design、Work Item 默认同时受影响。

验证必须覆盖三类 workspace：

- Story：`verdict=revise` 且 no findings -> `user_triage_required/human_confirm`
- Design：只有 optional findings -> `user_confirm_allowed/human_confirm`
- Work Item：有 `strong_recommend_fix` -> `requires_revision/review_decision`

## 兼容与迁移

- 旧落盘 `NodeDetail.verdict` 没有 `review_gate` 时，前端按 `user_confirm_allowed` 兼容，但后端恢复时若能从 reviewer message 重新解析，应按新规则计算。
- 旧实时事件没有 findings/review_gate 时，前端保持当前摘要展示。
- 已完成 workspace 不做批量迁移。

## 风险

- `user_triage_required` 会让部分原本可直接确认的异常输出变得更谨慎，但这符合“不误导用户”的目标。
- “采纳建议并返修”自动生成的 revision context 可能较长，需要限制只包含 severity/message/required_action，避免把完整 reviewer 输出重复灌给 author。
- 如果 reviewer 长期不输出结构化 findings，用户会频繁看到人工裁决态；prompt 调整和后续 E2E 需要观察该比例。

## 验收标准

- 强 finding 仍进入 `review_decision`，不能直接确认。
- optional/minor/suggestion findings 进入 `human_confirm`，可确认，也可采纳建议返修。
- `verdict=revise` 且无 findings 进入 `human_confirm`，页面文案为“需要判断 reviewer 意图”。
- 实时 review 完成后无需刷新即可看到 findings 分组和正确按钮。
- Story、Design、Work Item 三类 workspace 的共享路径均有回归测试。
