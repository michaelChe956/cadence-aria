# Workspace 断线手动恢复技术方案

## 1. 背景

Workspace provider 在生成或审核过程中发生 WebSocket 断线时，当前实现会把活动节点标记为失败，追加 `aborted_by_disconnect`，并将会话退回 `prepare_context`。此时页面只提供“开始生成”，导致用户可能误触发新的 Author/Outline 生成，而无法恢复真正中断的步骤。

实际案例 `workspace_session_0003` 中，最后一个有效 Work Item Draft `draft_012` 已接受，但对应 `work_item_draft_review` 因断线失败。用户随后点击“开始生成”，又产生了失败的 `work_item_plan_outline_run`。该 Outline Run 未生成 artifact version，因此没有覆盖 confirmed Outline 或任何已接受 Draft，但它会遮蔽真正需要恢复的审核步骤。

## 2. 目标

- 在 Workspace 断线后提供显式的手动恢复按钮，不自动调用 provider。
- 根据持久化产品状态判断真正缺失的下一步，而不是简单重试 Timeline 中最后一个失败节点。
- 支持 Work Item Plan 的单项 Draft 生成中断和 Draft Review 中断。
- 支持 Story、Design、Work Item 的普通 Reviewer Run 中断。
- 保留失败节点及部分输出用于审计，不将部分输出作为完整 artifact 或审核结论复用。
- 保证误触产生、但没有提交 artifact 的失败运行不会覆盖或改变当前有效产物。

## 3. 非目标

- 不自动重试中断运行。
- 不把“开始生成”改造成隐式恢复入口。
- 不拼接断线前的部分 provider 输出与新运行输出。
- 不在本次范围内恢复 Story、Design、Work Item 的 Author Run 或 Revision。
- 不恢复 Work Item Plan Outline Author Run；已 confirmed 的 Outline 必须继续作为当前 Outline。
- 不删除失败 Timeline 节点。

## 4. 方案比较

### 方案 A：独立恢复动作与手动按钮

新增专用 WebSocket 入站动作，由后端识别可恢复步骤、完成状态校验并启动对应 provider run。前端只展示后端声明的恢复能力。

优点：动作语义明确；后端是唯一状态判断源；可严格防止重复运行和错误 Outline 重生成。

缺点：需要扩展 WebSocket contract、Engine 恢复逻辑和前端状态。

### 方案 B：复用“开始生成”

在 `start_generation` 中检测失败运行并决定恢复或重新生成。

优点：协议改动较少。

缺点：按钮语义不稳定；相同操作在不同隐藏状态下产生不同结果；容易再次误生成 Outline。

### 方案 C：回滚到上一个确认节点

回滚到 Draft Confirm 后重新接受 Draft，以触发 Review。

优点：复用部分既有状态流转。

缺点：当前 Work Item Plan 会话未必存在可用 checkpoint；重复接受 Draft 会增加持久化一致性风险；无法自然处理 Draft 生成中断。

采用方案 A。

## 5. 恢复状态模型

后端计算一个可选的 `RecoverableInterruptedRun`：

```text
RecoverableInterruptedRun
  failed_node_id: String
  operation: work_item_draft_generation | review
  label: String
```

该描述随 `session_state` 返回。前端不得自行推断业务恢复目标；点击按钮后发送描述中的 `failed_node_id`，后端重新计算并校验，防止客户端状态过期。

### 5.1 Work Item Draft 生成中断

满足以下条件时返回 `work_item_draft_generation`：

- workspace type 为 `work_item_plan`；
- 当前阶段为 `prepare_context`，且没有活动 provider run；
- active index 的 `outline_state` 为 `confirmed`；
- `active_outline_id` 存在；
- 当前 outline 对应的 Draft 尚未进入可确认或 accepted 状态；
- Timeline 中存在该 outline 对应、因断线失败的 `work_item_draft_run`；
- 失败节点之后没有为该 outline 提交新的 Draft artifact version。

执行恢复时，为同一个 `active_outline_id` 新建 `work_item_draft_run` retry 节点，并启动 `ProviderRunKind::WorkItemPlanDraft { feedback: None }`。断线前的部分输出只保留在旧 NodeDetail 中，新运行从完整 Draft prompt 重新开始。

### 5.2 Draft 已保存并等待确认

如果当前 outline 已存在有效 Draft Candidate，但会话错误停留在 `prepare_context`，系统不调用 provider，而是恢复 `author_confirm` 和 `work_item_draft_confirm`。前端随后展示既有 Draft 的接受、重写、暂停操作。

这一分支不显示“重新生成”按钮，避免覆盖已保存 Draft。

### 5.3 Work Item Draft Review 中断

满足以下条件时返回 `review`：

- 当前 Draft 为 accepted；
- active index 的 `active_outline_id` 与 Draft 的 `outline_id` 一致；
- 当前 artifact version 仍绑定该 Draft；
- Timeline 中存在该 Draft 对应、因断线失败的 `work_item_draft_review`；
- 失败后没有完成新的 Draft、Review、Revision 或 Compile；
- 后续仅出现未提交 artifact 的失败运行时，原 Review 仍可恢复。

执行恢复时，新建 `work_item_draft_review` retry 节点，切换到 `cross_review`，并启动 `ProviderRunKind::ReviewOnly`。旧 Review 的部分输出不作为 verdict 输入或结果。

### 5.4 普通 Workspace Review 中断

Story、Design、Work Item 共用普通 Reviewer Run 恢复：

- 当前 artifact version 与失败 Review 的 artifact ref 一致；
- 失败节点为 `reviewer_run` 且由断线终止；
- 没有后续成功 artifact、Revision 或 Review；
- reviewer provider 仍可用。

恢复时新建带 retry 元数据的 `reviewer_run`，并使用 `ProviderRunKind::ReviewOnly`。

## 6. Timeline 与幂等性

恢复不会修改旧失败节点。新节点通过既有 `TimelineNodeRetry` 记录：

```text
retry_of_node_id = 原失败节点 ID
retry_attempt = 同一源节点已有 retry 数量 + 1
retry_reason = aborted_by_disconnect
retry_error.code = provider_run_aborted_by_disconnect
```

后端必须满足以下幂等约束：

- 有活动 run 时拒绝恢复；
- 同一恢复请求重复发送时，只有第一次能创建活动节点；
- 客户端提交的 `failed_node_id` 与后端重新计算结果不一致时拒绝；
- provider 启动失败时保留新失败节点，原 artifact 与 active index 不变；
- 任何失败都不得把 partial output 标记为完整 artifact 或 pass verdict。

## 7. WebSocket Contract

新增入站消息：

```json
{
  "type": "retry_interrupted_run",
  "failed_node_id": "timeline_node_054"
}
```

该消息仅允许在 `prepare_context` 阶段处理。后端根据恢复结果启动 `ReviewOnly` 或 `WorkItemPlanDraft`。不可恢复时返回稳定的 `ProtocolError`，至少区分：

- `INTERRUPTED_RUN_NOT_RECOVERABLE`
- `INTERRUPTED_RUN_STATE_CHANGED`
- `INTERRUPTED_RUN_ALREADY_ACTIVE`

`session_state` 增加可选字段：

```json
{
  "recoverable_interrupted_run": {
    "failed_node_id": "timeline_node_054",
    "operation": "review",
    "label": "重试中断审核"
  }
}
```

字段缺失表示当前没有安全恢复动作。

## 8. 前端交互

- `DisconnectBanner` 在 `recoverable_interrupted_run` 存在时显示主按钮。
- `operation=review` 时文案为“重试中断审核”。
- `operation=work_item_draft_generation` 时文案为“重新生成中断的 Work Item Draft”。
- 点击后立即禁用按钮，直到收到新的 active Timeline 节点、错误响应或连接再次断开。
- “查看 Timeline”和“我知道了”继续保留。
- 没有安全恢复动作时不展示重试按钮，仍允许查看 Timeline。
- “开始生成”不承担恢复职责；当存在恢复动作时，应隐藏或禁用“开始生成”，避免用户再次走错流程。

## 9. 数据影响判断

失败运行只有在成功完成并持久化 artifact version 后才会改变当前产物。案例中的失败 `work_item_plan_outline_run` 没有对应 artifact version，因此：

- 不影响 `outline_state=confirmed`；
- 不改变 `active_outline_id`；
- 不覆盖 active Outline；
- 不改变 `draft_012=accepted`；
- 不改变其他已接受 Draft；
- 只新增失败 Timeline 和 NodeDetail 审计记录。

恢复实现必须延续此规则：用 artifact version、active index 和 Draft record 判断有效状态，不用 Timeline 的“最近节点”推断当前产物。

## 10. 测试策略

### 后端 Engine

- Work Item Draft 生成断线后识别同一 outline 的恢复动作。
- 已保存 Draft 不重新生成，而是恢复 Draft Confirm。
- accepted Draft Review 断线后识别 Review 恢复动作。
- 后续失败 Outline Run 未提交 artifact 时，仍恢复更早的有效 Draft Review。
- 后续成功 artifact 提交后，不再恢复旧失败 Review。
- 重复点击只创建一个 active retry 节点。
- retry 节点正确记录 `retry_of_node_id`、attempt、reason 和 error。
- 普通 Story、Design、Work Item Reviewer Run 使用表驱动测试覆盖。

### WebSocket Handler

- `retry_interrupted_run` 仅允许在 `prepare_context`。
- Review 恢复启动 `ProviderRunKind::ReviewOnly`。
- Draft 生成恢复启动 `ProviderRunKind::WorkItemPlanDraft`。
- stale `failed_node_id` 返回稳定 ProtocolError。

### 前端

- DisconnectBanner 对两种 operation 展示正确按钮文案。
- 点击只发送一次恢复消息并进入禁用状态。
- 有恢复动作时隐藏或禁用“开始生成”。
- 无恢复动作时不显示重试按钮。
- Story、Design、Work Item 普通 Review 恢复能力使用表驱动测试覆盖共享页面装配。

## 11. 验收标准

- `workspace_session_0003` 重连后显示“重试中断审核”。
- 点击后创建关联 `timeline_node_054` 的新 `work_item_draft_review` 节点。
- Reviewer 通过后直接进入 Work Item Plan Compile，不重新生成 Outline 或 Draft。
- Work Item Draft 生成断线时，按钮重新生成同一 active outline，已接受的其他 Work Item 不变化。
- 已保存待确认 Draft 不发生 provider 重跑。
- Story、Design、Work Item 的普通 Review 断线可使用同一手动恢复机制。
- 所有定向测试、前端类型检查及项目规定的完整验证通过。
