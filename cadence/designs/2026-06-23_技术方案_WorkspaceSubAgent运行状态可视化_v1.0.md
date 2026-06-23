# Workspace Sub-agent 运行状态可视化技术方案

## 文档信息

- 文档类型：技术方案
- 版本：v1.0
- 日期：2026-06-23
- 分支：feat-b-0616
- 状态：待评审

## 背景

WorkItemPlan Outline 真实 E2E 中出现“页面像卡住”的体验。后端排查显示 WebSocket 仍在正常发送 `pong` 与 `execution_event`，Provider 进程也仍在运行；当前 active node 是 `work_item_plan_outline_run`，author 使用 `claude_code`，节点详情中 `streaming_content` 为空，但存在运行中的 sub-agent 工具事件：

- `title = "Agent"`
- `status = "started"`
- `detail = "Explore frontend dialog patterns"` 或类似任务描述

这说明系统并未真正卡死，而是主 author 正在等待 sub-agent 返回。当前前端把这类事件当作普通“执行事件”折叠展示，且没有在 author 主区域形成明确的运行中反馈，用户只能看到空白或静止状态。

## 目标

1. 当 author / reviewer / WorkItemPlan draft provider 启动 sub-agent 时，前端明确展示“sub-agent 正在运行”。
2. 即使 provider 暂时没有 `stream_chunk` 或 `streaming_content`，用户也能看到当前等待的是哪个 sub-agent。
3. 刷新页面后，通过 session state 中已有 `timeline_node_details.execution_events` 恢复 sub-agent 运行状态。
4. 不改变后端协议，不新增 WebSocket 消息类型；优先复用现有 `ExecutionEvent`。
5. 适用于 Story、Design、WorkItem、WorkItemPlan 的共享 Workspace 聊天链路。

## 非目标

- 不实现 sub-agent 取消、强制终止或超时治理。
- 不展示 sub-agent 内部完整实时日志；第一版只展示任务名、状态和完成后的摘要输出。
- 不修改 provider prompt，不要求 provider 额外输出可读进度。
- 不把全部 execution event 展开成主聊天内容，避免噪声过大。
- 不新增后端持久化字段；如果未来需要更精确的 sub-agent 类型，可另做协议扩展。

## 现有数据基础

前端已有 `ExecutionEvent` 类型：

```text
ExecutionEvent
- event_id
- node_id
- agent
- kind
- status: started | running | waiting_approval | completed | failed | aborted
- title
- detail
- command
- cwd
- output
- exit_code
```

真实 sub-agent 事件形态：

```text
{
  "kind": "command",
  "title": "Agent",
  "status": "started",
  "detail": "Explore frontend dialog patterns",
  "output": null
}
```

完成时可能变为：

```text
{
  "kind": "command",
  "title": "Agent",
  "status": "completed",
  "detail": null,
  "output": "Here is a comprehensive summary ..."
}
```

当前 store 已经把实时 `execution_event` upsert 到：

- `executionEvents`
- 对应 `nodeDetails[node_id].execution_events`

聊天区也会从 `nodeDetails` 重建 execution event entry。因此第一版可以完全在前端识别和渲染，不需要后端改造。

## 方案选择

### 方案 A：只改 ExecutionEventEntry 文案

把 `title = "Agent"` 的事件显示成“Sub-agent 运行中”。

优点是改动最小。缺点是如果 active node 没有 stream 内容，用户仍可能在主气泡区看不到明确占位；Timeline 也不会提示当前节点为什么还在 active。

### 方案 B：聊天区 sub-agent 状态行 + 空流占位

在 chat entries 重建阶段识别 sub-agent 事件：

- 运行中 sub-agent 生成专用 execution event 行。
- 如果当前 node 没有 provider stream，但存在运行中 sub-agent，则生成一个 provider-stream 风格的占位气泡。
- 完成后同一事件行变为完成状态，可展开查看 output。

优点是能直接解决“看起来卡住”的核心体验，且不改后端。缺点是 Timeline 左侧仍只能看到 node active，需要用户看右侧聊天区确认细节。

### 方案 C：方案 B + Timeline 活动摘要

在方案 B 基础上，Timeline active node 下方增加简短活动摘要：

- `运行中 · 1 个 sub-agent · Explore frontend dialog patterns`
- 多个运行中时显示 `运行中 · 2 个 sub-agent`

优点是左右两侧都能传达当前系统仍在工作。缺点是 `TimelineNodeList` 当前只接收 `nodes`，需要额外传入 node details 或预计算摘要，改动略大。

推荐采用方案 C，但分两步实现：第一步先做方案 B，第二步补 Timeline 摘要。若实现时间有限，方案 B 已能解决主要问题。

## UI 设计

### 聊天区运行中行

当 event 满足以下条件时，使用 sub-agent 专用展示：

```text
event.title === "Agent"
event.status === "started" || event.status === "running"
```

展示文案：

```text
Sub-agent 运行中 · Explore frontend dialog patterns
```

如果 `event.agent` 或 node provider 可用，可显示为：

```text
Claude Code 正在运行 sub-agent · Explore frontend dialog patterns
```

视觉样式：

- 图标：`LoaderCircle`，运行中使用轻量旋转动画。
- 主文本：`Sub-agent 运行中`。
- 次文本：显示 `detail`，无 detail 时显示 `等待子任务返回`。
- 行高稳定，使用现有 `InlineEventRow` 的边框、背景和字号体系。
- 不使用 emoji。
- 动画必须尊重 `prefers-reduced-motion`；若用户减少动画，显示静态图标。

### 聊天区空流占位

当某个 active provider node 同时满足：

- node status 为 `active`
- `streaming_content` 为空
- `messages` 为空
- 存在运行中 sub-agent event

生成一个占位 entry：

```text
Author 正在探索代码库，等待 1 个 sub-agent 返回...
```

若多个 sub-agent：

```text
Author 正在探索代码库，等待 2 个 sub-agent 返回...
```

占位 entry 的角色使用当前 node role：author / reviewer / system。WorkItemPlan Outline author 应展示在 author 侧，而不是 system 侧。

占位 entry 不需要 content_ref，不展开，不持久化为后端消息；它是前端从 node detail 派生的临时可视状态。

### 完成状态行

当 event 满足：

```text
event.title === "Agent"
event.status === "completed"
```

展示文案：

```text
Sub-agent 已完成 · Explore codebase structure
```

如果 `detail` 为空但 `output` 存在，可以从 output 第一行截取摘要；若 output 很长，仍只展示一行，完整 output 放在展开区域。

失败或中止：

```text
Sub-agent 失败 · <detail 或 output 摘要>
Sub-agent 已中止 · <detail 或 output 摘要>
```

失败状态使用 `AlertTriangle` 或现有错误色，完成状态使用 `CheckCircle`，运行中使用 `LoaderCircle`。

### Timeline 活动摘要

`TimelineNodeList` 当前仅接收 `nodes`。为了显示 sub-agent 摘要，有两种实现方式：

1. 给 `TimelineNodeList` 增加可选 `nodeDetails` prop，由组件内部计算摘要。
2. 在页面层预计算 `nodeActivitySummaryById`，传入 `TimelineNodeList`。

推荐第二种，保持 Timeline 组件更轻，减少对 store 结构的耦合。

摘要规则：

- active node 有运行中 sub-agent：显示 `运行中 · 1 个 sub-agent · <detail>`
- 多个运行中：显示 `运行中 · N 个 sub-agent`
- 无运行中 sub-agent 但有最近执行事件：不改变现状，避免噪声。
- node.summary 存在时，sub-agent 摘要优先于 summary；完成后恢复 summary。

## 数据与映射规则

新增前端纯函数：

```text
isSubAgentExecutionEvent(event)
isRunningSubAgentEvent(event)
subAgentEventLabel(event)
runningSubAgentEvents(detail)
subAgentPlaceholderEntry(node, detail, role)
timelineSubAgentSummary(detail)
```

识别规则：

```text
isSubAgentExecutionEvent:
  event.title === "Agent"

isRunningSubAgentEvent:
  isSubAgentExecutionEvent(event)
  && (event.status === "started" || event.status === "running")
```

文案优先级：

1. `event.detail`
2. `event.output` 的第一段短摘要
3. `等待子任务返回`

不要依赖 `event_id` 前缀，因为不同 provider 的 tool id 可能变化。

## 前端改造范围

### `web/src/state/workspace-ws-store.ts`

职责：

- 在 `buildEntriesFromState` 中识别 sub-agent 事件。
- 当 node 无 stream 内容但有 running sub-agent 时，插入占位 entry。
- 调整 `executionEventContent`，为 Agent 事件生成用户可读文案。
- 保持 `content_ref` 对长 output 的按需加载能力。

关键行为：

- running sub-agent event 仍保留 execution event entry，便于用户展开。
- 占位 entry 和 event entry 不能重复表达同一层信息到让页面显得拥挤；占位气泡用于“当前正在等”，event 行用于“具体哪个 sub-agent”。
- completed/failed/aborted 后，运行中占位自动消失。

### `web/src/hooks/useWorkspaceWs.ts`

职责：

- 原有 `execution_event` upsert 逻辑不变。
- 如当前 hook 也有自己的 `executionEventContent`，需与 store 里的文案规则保持一致，避免实时事件和刷新恢复文案不同。

### `web/src/components/chat-workspace/InlineEventRow.tsx`

职责：

- 根据 metadata 判断 `title === "Agent"`。
- 运行中显示 `LoaderCircle`。
- 完成显示 `CheckCircle`。
- 失败显示 `AlertTriangle`。
- 保持可展开 output 行为。

### `web/src/components/chat-workspace/entries/ExecutionEventEntry.tsx`

职责：

- 非 inline 分组场景也使用同一套 sub-agent 图标与文案。
- 不改变普通 command / provider prompt / artifact event 的展示。

### `web/src/components/chat-workspace/TimelineNodeList.tsx`

第二步实现：

- 增加 `activitySummaryByNodeId?: Record<string, string>` prop。
- active node 有 sub-agent 摘要时展示该摘要。
- 不破坏现有 `summary` 展示。

### `web/src/pages/ChatWorkspacePage.tsx`

第二步实现：

- 从 `nodeDetails` 预计算 `activitySummaryByNodeId`。
- 传给 `TimelineNodeList`。

## 交互状态矩阵

| 场景 | 聊天区 | Timeline |
| --- | --- | --- |
| provider 刚开始，无 sub-agent，无 stream | 保持现有状态 | active |
| sub-agent started，无 stream | 显示等待 sub-agent 占位 + running event 行 | 显示 sub-agent 摘要 |
| sub-agent started，有 stream | 显示 stream + running event 行 | 显示 sub-agent 摘要 |
| sub-agent completed | running 占位消失，event 行变完成，可展开 output | 恢复 summary 或 active |
| sub-agent failed | event 行显示失败，可展开 output | 可显示失败摘要，若 node 仍 active 则保持 active |
| 多个 sub-agent running | 占位显示数量，event 行逐条展示 | 显示数量 |
| 刷新恢复 | 从 node detail 重建同样状态 | 从 node detail 重建摘要 |

## 测试策略

### Store 单元测试

新增或扩展 `web/src/state/workspace-ws-store.test.ts`：

1. active node 无 stream 且有 Agent started event 时，生成 sub-agent 等待占位。
2. Agent started event 的 chat entry content 包含 `Sub-agent 运行中` 和 detail。
3. Agent completed event 不生成等待占位，但保留完成 entry。
4. 多个 Agent started event 时，占位显示数量。
5. session_state 恢复路径与实时 execution_event 路径生成一致 entry。

### Hook 测试

扩展 `web/src/hooks/useWorkspaceWs.test.tsx`：

1. 收到 realtime `execution_event` Agent started 后，store 中出现对应 execution event entry。
2. 若 active node 无 stream，出现等待 sub-agent 占位。
3. Agent completed 后，占位消失或变为完成态。

### 组件测试

扩展：

- `InlineEventRow.test.tsx`
- `ExecutionEventEntry` 相关测试
- 如实现 Timeline 摘要，扩展 `TimelineNodeList.test.tsx`

断言：

- running 使用可访问文案 `Sub-agent 运行中`。
- completed / failed / aborted 有不同状态文案。
- 展开后仍能看到 output。
- 不使用 emoji，图标来自 lucide。

## 可访问性与视觉约束

- 运行中动画必须通过 CSS `animate-spin`，并由全局 `prefers-reduced-motion` 规则降级为静态。
- 状态不能只依赖颜色；必须有文本：运行中、已完成、失败、已中止。
- 可点击展开行保持 button 语义和 focus ring。
- 文案需短，不挤压聊天区；长 detail 使用 truncate，展开区显示完整 output。
- 保持现有工作台安静、工具型风格，不引入大面积装饰或营销式卡片。

## 风险与取舍

### 风险：`title === "Agent"` 是 provider 文本约定

当前真实事件已经使用该形态。第一版可接受，但应把识别函数集中，未来如果后端增加 `kind = "sub_agent"` 或 metadata 字段，只改一处。

### 风险：占位 entry 与 execution event 行重复

占位表达“为什么主气泡还没出现”，event 行表达“具体哪个工具在跑”。二者分工明确。若页面拥挤，可只在无 stream 时显示占位。

### 风险：Timeline 需要更多 props

Timeline 摘要作为第二步实现。第一步不改 Timeline 也能明显改善体验。

## 实施顺序

1. 在 store 中新增 sub-agent 识别与文案纯函数。
2. 调整 chat entry 重建逻辑，插入无 stream 时的 running sub-agent 占位。
3. 调整 `InlineEventRow` 和 `ExecutionEventEntry` 的 Agent 状态渲染。
4. 补 store / hook / component 测试。
5. 视实现成本补 Timeline active node 活动摘要。
6. 跑前端测试和构建：`pnpm -C web test`、`pnpm -C web build`。

## 验收标准

1. 当 author 启动 sub-agent 且长时间没有 stream 文本时，页面明确显示 sub-agent 正在运行。
2. 用户能看到 sub-agent 的任务描述，例如 `Explore frontend dialog patterns`。
3. sub-agent 完成后，运行中状态消失或变成完成状态，output 可展开查看。
4. 刷新页面后仍能从 session state 恢复 sub-agent 状态。
5. Story、Design、WorkItem、WorkItemPlan 共用 Workspace 页面都适用，不做 WorkItemPlan 专用分支。
6. 普通 execution event、provider prompt、permission request、stream chunk 的现有展示不回退。
