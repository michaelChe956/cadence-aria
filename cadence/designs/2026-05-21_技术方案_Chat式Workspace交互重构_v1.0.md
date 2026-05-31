# Chat 式 Workspace 交互重构设计

> 版本：v1.0 | 日期：2026-05-21

## 1. 概述

### 1.1 目标

将 WorkspacePage 从当前的"stage panel + tab 详情"模式重构为 **chat 对话流**模式。所有交互事件（用户输入、provider 流式输出、执行事件、权限请求、Artifact 更新、Gate 确认）统一在一个 chat 时间线中按序展示，区分输入和输出，形成类似 Claude Code / vibe-kanban 的对话式体验。

### 1.2 参考

- vibe-kanban `DisplayConversationEntry.tsx`：`NormalizedEntry` 多类型 chat entry 模型
- vibe-kanban `ChatEntryContainer`：统一的 entry 容器（variant 区分角色）
- vibe-kanban `ChatToolSummary`：tool call 摘要行
- vibe-kanban `ChatApprovalCard`：plan 确认卡片
- 本项目 `2026-05-16_对话式Workspace统一设计_v1.0.md`：三栏布局原始设计
- 本项目 `2026-05-20_Workspace产品工作台优化_v1.0.md`：协议层重塑（context_note / start_generation 拆分）

### 1.3 设计决策记录

| # | 决策点 | 选择 | 理由 |
|---|--------|------|------|
| 1 | 整体布局 | 三栏：Timeline 节点列表 + Chat 流 + Artifact 面板 | 过程（chat）和结果（Artifact）分离；权限请求不被 tab 遮挡 |
| 2 | 输入框语义 | 发送 = context_note，独立"开始生成"按钮 | 协议语义清晰，不会误触发生成 |
| 3 | 流式输出展示 | 单条 assistant entry 持续追加，tool calls/permission 穿插为独立 entry | 和 Claude Code / vibe-kanban 心智模型一致 |
| 4 | Author/Reviewer 区分 | 角色标签 + 颜色区分 | review 过程是重要决策依据，不应默认折叠 |
| 5 | HumanConfirm 修改意见 | 复用 chat 输入框发送 | 保持交互一致性，修改意见作为 user message 进入对话上下文 |
| 6 | 左侧导航 | 保留 Timeline 节点列表，点击跳转 chat 流对应位置 | 多轮 review 场景需要细粒度导航 |
| 7 | 实施策略 | 新建 ChatWorkspacePage，完成后删除旧 WorkspacePage | 避免"改到一半两边都不能用"，可独立验证 |

---

## 2. 页面布局

### 2.1 三栏结构

```
┌─────────────────────────────────────────────────────────────────┐
│ TopBar: [← 返回] [Story Spec #001] [Claude Code / Codex] [●连接] │
├──────────┬────────────────────────────────┬─────────────────────┤
│          │                                │                     │
│ Timeline │       Chat Area                │   Artifact Pane     │
│ Nodes    │                                │                     │
│          │  ┌─[user]──────────────────┐   │   Markdown 渲染     │
│ ○ ctx-1  │  │ 登录功能需要支持手机号... │   │   + Diff toggle     │
│ ○ ctx-2  │  └────────────────────────┘   │                     │
│ ● run-1  │  ┌─[Claude Code]──────────┐   │   ┌──────────────┐  │
│   (运行中) │  │ 正在生成 Story Spec... │   │   │ # Story Spec │  │
│          │  │ (流式追加中)            │   │   │              │  │
│          │  ├─ 🔧 Read src/auth.rs   │   │   │ ## 用户故事   │  │
│          │  ├─ 🔐 权限请求 [允许][拒绝]│   │   │ ...          │  │
│          │  │ 继续生成中...           │   │   │              │  │
│          │  └────────────────────────┘   │   └──────────────┘  │
│          │                                │                     │
│          ├────────────────────────────────┤                     │
│          │ [输入框...        ] [发送] [▶生成] │                     │
├──────────┴────────────────────────────────┴─────────────────────┤
│ StatusBar: PrepareContext | 已连接 | 耗时 2m30s                    │
└─────────────────────────────────────────────────────────────────┘
```

### 2.2 响应式行为

- **大屏（≥1024px）**：三栏并排
- **中屏（768-1023px）**：Artifact 面板可折叠，折叠后 chat 区域占满
- **小屏（<768px）**：Timeline 节点列表折叠为顶部下拉，Artifact 面板折叠为底部抽屉

### 2.3 Artifact 面板

- 固定展示当前最新版本的产物 Markdown 渲染
- 顶部：版本号选择器 + Diff toggle（对比上一版本）
- 当 `artifact_update` 事件到达时自动刷新内容
- 可通过按钮折叠/展开，折叠后 chat 区域扩展

---

## 3. Chat Entry 类型系统

### 3.1 Entry 类型定义

```typescript
type ChatEntryType =
  | "context_note"        // 用户补充上下文
  | "start_generation"    // 用户触发生成（分隔标记）
  | "provider_stream"     // provider 流式输出（assistant message）
  | "execution_event"     // 执行事件（tool call 摘要行）
  | "permission_request"  // 权限请求卡片
  | "permission_response" // 权限响应结果
  | "artifact_update"     // 产物更新通知
  | "review_verdict"      // reviewer 审核结论
  | "gate_prompt"         // 硬 gate 确认卡片
  | "human_decision"      // 用户确认/修改决策
  | "stage_change"        // 阶段变更分隔线
  | "error";              // 错误信息

interface ChatEntry {
  id: string;
  type: ChatEntryType;
  role: "user" | "author" | "reviewer" | "system";
  content: string;
  timestamp: string;
  node_id?: string;       // 关联的 Timeline 节点 ID
  metadata?: Record<string, unknown>;
}
```

### 3.2 Entry 展示规格

| Entry 类型 | 角色 | 展示形态 | 交互 |
|-----------|------|---------|------|
| `context_note` | user | 用户消息气泡（左对齐，蓝色边框） | 无 |
| `start_generation` | system | 水平分隔线 + "▶ 开始生成" 标签 + Provider 快照 | 无 |
| `provider_stream` | author/reviewer | assistant 消息气泡，流式追加，角色标签区分颜色 | 自动滚动 |
| `execution_event` | system | 摘要行（图标 + 一行描述），可点击展开详情 | 点击展开 |
| `permission_request` | system | 琥珀色卡片，工具名 + 描述 + 风险级别 + [允许][拒绝] | 按钮操作 |
| `permission_response` | user | 小标签（绿色"已允许" / 红色"已拒绝"） | 无 |
| `artifact_update` | system | 小标签"产物已更新 → v2"，右侧面板同步刷新 | 点击跳转右侧面板 |
| `review_verdict` | reviewer | 结论卡片（pass 绿色 / revise 橙色 + 摘要文本） | 无 |
| `gate_prompt` | system | 确认卡片，展示产物摘要 + [确认][要求修改][终止] 按钮 | 按钮操作 |
| `human_decision` | user | 用户消息气泡（"已确认" / 修改意见文本） | 无 |
| `stage_change` | system | 水平分隔线 + 阶段名标签 | 无 |
| `error` | system | 红色错误卡片（错误码 + 消息） | 无 |

### 3.3 角色标签设计

| 角色 | 标签文本 | 颜色 |
|------|---------|------|
| user | "你" | 蓝色 `--aria-primary` |
| author (Claude Code) | "Claude Code" | 紫色 |
| author (Codex) | "Codex" | 绿色 |
| reviewer (Codex) | "Codex (Reviewer)" | 橙色 |
| reviewer (Claude Code) | "Claude Code (Reviewer)" | 橙色 |
| system | 无标签，用分隔线/小标签样式 | 灰色 |

---

## 4. 阶段与输入框状态

### 4.1 状态矩阵

| 阶段 | 输入框 | "发送"按钮 | "开始生成"按钮 | 其他按钮 |
|------|--------|-----------|-------------|---------|
| PrepareContext | 可用 | 发送 context_note | 可用 | — |
| Running | 禁用 | 禁用 | 隐藏 | [中止] 红色 |
| CrossReview | 禁用 | 禁用 | 隐藏 | [中止] 红色 |
| ReviewDecision | 禁用 | 禁用 | 隐藏 | 路径选择内联于 review_verdict entry |
| HumanConfirm | 可用 | "发送修改意见" | 隐藏 | [确认][终止] 内联于 gate_prompt entry |
| Revision | 禁用 | 禁用 | 隐藏 | [中止] 红色 |
| Completed | 禁用 | 禁用 | 隐藏 | — |

### 4.2 HumanConfirm 阶段特殊行为

当 gate_prompt entry 出现后：
1. 输入框切换为"修改意见"模式（placeholder 变为"输入修改意见..."）
2. 用户可以直接点 gate_prompt 卡片上的 [确认] 按钮（不需要输入）
3. 用户输入修改意见后点"发送"，等同于 `human_confirm { decision: "request-change", payload: feedback }`
4. 发送后，输入框恢复禁用状态，流程进入 Revision

### 4.3 ReviewDecision 阶段特殊行为

当 review_verdict entry 出现后，entry 内部展示路径选择按钮：
- [接受修订建议] → `select_revision_path("revise")`
- [补充上下文后修订] → `select_revision_path("revise-with-context")`
- [跳过，人工处理] → `select_revision_path("skip-to-human")`

---

## 5. 左侧 Timeline 节点列表

### 5.1 节点类型与图标

| 节点类型 | 图标 | 说明 |
|---------|------|------|
| `context_note` | 💬 | 用户补充的上下文 |
| `start_generation` | ▶ | 生成触发点 |
| `author_run` | 🤖 | author provider 执行 |
| `reviewer_run` | 👁 | reviewer provider 执行 |
| `human_confirm` | ✋ | 人工确认节点 |
| `aborted_by_disconnect` | ⚠ | 断开中止 |

### 5.2 交互

- 点击节点 → chat 流 `scrollIntoView` 到该节点关联的第一个 chat entry
- 当前活跃节点（正在执行的 provider run）高亮显示
- 已完成节点显示 ✓ 标记
- 中止/错误节点显示红色标记

### 5.3 与 Chat Entry 的关联

每个 Timeline 节点通过 `node_id` 关联一组 chat entries。映射关系：

- `context_note` 节点 → 对应的 `context_note` chat entry
- `author_run` 节点 → 该 run 期间的所有 entries（provider_stream + execution_event + permission_request + artifact_update）
- `reviewer_run` 节点 → 该 run 期间的 entries + review_verdict
- `human_confirm` 节点 → gate_prompt + human_decision entries

---

## 6. 数据流与 Store 扩展

### 6.1 复用现有基础设施

| 模块 | 处理方式 |
|------|---------|
| `useWorkspaceWs.ts` | 保留复用，不修改 |
| `workspace-ws-store.ts` | 扩展，新增 `chatEntries` 字段 |
| WebSocket 协议 | 不修改（context_note / start_generation / abort / permission_response / human_confirm） |
| 后端 SSE 事件 | 不修改 |

### 6.2 Store 扩展

```typescript
// workspace-ws-store.ts 新增字段
interface WorkspaceWsState {
  // ... 现有字段保留 ...

  // 新增：chat entries 列表
  chatEntries: ChatEntry[];

  // 新增：actions
  appendChatEntry: (entry: ChatEntry) => void;
  updateStreamingEntry: (entryId: string, content: string) => void;
}
```

### 6.3 SSE 事件 → Chat Entry 映射

| SSE 事件 | 生成的 Chat Entry |
|---------|-----------------|
| `session_state` (初始快照) | 从历史 timeline nodes 重建 chatEntries |
| `context_note_ack` | `context_note` entry |
| `provider_locked` | `start_generation` entry |
| `stage_change` | `stage_change` entry |
| `stream_chunk` | 追加到当前 `provider_stream` entry 的 content |
| `execution_event` | `execution_event` entry |
| `permission_request` | `permission_request` entry |
| `artifact_update` | `artifact_update` entry |
| `review_verdict` / `reviewer_completed` | `review_verdict` entry |
| `gate_opened` | `gate_prompt` entry |
| `protocol_error` | `error` entry |
| `aborted_by_disconnect` | `error` entry |

### 6.4 用户操作 → Chat Entry + WebSocket 消息

| 用户操作 | Chat Entry | WebSocket 消息 |
|---------|-----------|---------------|
| 输入框发送 | `context_note` entry（乐观插入） | `context_note { content }` |
| 点击"开始生成" | `start_generation` entry（乐观插入） | `start_generation { provider_config, reviewer_enabled }` |
| 点击"允许"权限 | `permission_response` entry | `permission_response { id, approved: true }` |
| 点击"拒绝"权限 | `permission_response` entry | `permission_response { id, approved: false }` |
| 点击"确认" | `human_decision` entry("已确认") | `human_confirm { decision: "confirm" }` |
| 发送修改意见 | `human_decision` entry(意见文本) | `human_confirm { decision: "request-change", payload }` |
| 点击"终止" | `human_decision` entry("已终止") | `human_confirm { decision: "terminate" }` |
| 点击"中止" | — | `abort` |

---

## 7. 组件结构

```
ChatWorkspacePage
├── TopBar
│   ├── BackButton
│   ├── EntityTitle              # "Story Spec #001"
│   ├── ProviderConfigButton     # Provider 配置弹窗入口
│   └── ConnectionIndicator      # WebSocket 连接状态
├── DisconnectBanner             # 断开重连提示（复用现有）
├── MainContent (三栏 grid)
│   ├── TimelineNodeList         # 左侧节点导航
│   │   └── TimelineNodeItem     # 单个节点按钮
│   ├── ChatArea
│   │   ├── ChatEntryList        # entry 列表（虚拟滚动）
│   │   │   ├── UserContextEntry
│   │   │   ├── StartGenerationEntry
│   │   │   ├── ProviderStreamEntry
│   │   │   │   └── InlineExecutionEvent  # 内嵌的 tool call 摘要
│   │   │   ├── PermissionRequestEntry
│   │   │   ├── PermissionResponseEntry
│   │   │   ├── ArtifactUpdateEntry
│   │   │   ├── ReviewVerdictEntry
│   │   │   ├── GatePromptEntry
│   │   │   ├── HumanDecisionEntry
│   │   │   ├── StageChangeEntry
│   │   │   └── ErrorEntry
│   │   └── ChatInputBar
│   │       ├── TextArea
│   │       ├── SendButton
│   │       ├── StartGenerationButton
│   │       └── AbortButton
│   └── ArtifactPane
│       ├── VersionSelector
│       ├── DiffToggle
│       └── MarkdownRenderer
└── StatusBar
    ├── StageIndicator
    ├── ConnectionStatus
    └── ElapsedTime
```

---

## 8. 与现有代码的关系

### 8.1 新建

| 文件 | 说明 |
|------|------|
| `web/src/pages/ChatWorkspacePage.tsx` | 新页面主组件 |
| `web/src/components/chat-workspace/ChatEntryList.tsx` | entry 列表容器 |
| `web/src/components/chat-workspace/ChatInputBar.tsx` | 输入框 + 按钮 |
| `web/src/components/chat-workspace/ArtifactPane.tsx` | 右侧产物面板 |
| `web/src/components/chat-workspace/entries/*.tsx` | 各类型 entry 组件 |
| `web/src/state/chat-entries.ts` | chatEntries 相关 store 逻辑 |

### 8.2 复用

| 文件 | 说明 |
|------|------|
| `web/src/hooks/useWorkspaceWs.ts` | WebSocket 连接管理 |
| `web/src/hooks/useWorkspaceWsReconnect.ts` | 重连逻辑 |
| `web/src/hooks/useUnloadGuard.ts` | 页面离开拦截 |
| `web/src/state/workspace-ws-store.ts` | 扩展，不破坏现有接口 |
| `web/src/components/workspace/DisconnectBanner.tsx` | 直接复用 |
| `web/src/components/workspace/ProviderConfigPanel.tsx` | 弹窗内复用 |

### 8.3 完成后删除

| 文件 | 原因 |
|------|------|
| `web/src/pages/WorkspacePage.tsx` | 被 ChatWorkspacePage 替换 |
| `web/src/components/workspace/PrepareContextPanel.tsx` | 逻辑融入 ChatInputBar |
| `web/src/components/workspace/NodeDetailPanel.tsx` | 内容拆散到各 entry 组件 |
| `web/src/components/workspace/StageActionsBar.tsx` | 逻辑融入 ChatInputBar |
| `web/src/components/workspace/stages/HumanConfirmStagePanel.tsx` | 替换为 GatePromptEntry |
| `web/src/components/workspace/stages/ReviewDecisionStagePanel.tsx` | 替换为 ReviewVerdictEntry |

### 8.4 路由切换

```typescript
// 开发完成后，路由从 WorkspacePage 切换到 ChatWorkspacePage
// web/src/App.tsx 或路由配置中
<Route path="/workbench/workspace/:sessionId" component={ChatWorkspacePage} />
```

---

## 9. 关键交互流程

### 9.1 完整 Story Spec 生成流程（chat 视角）

```
[user] 登录功能需要支持手机号和邮箱两种方式
[user] 参考竞品 A 的登录流程
─── ▶ 开始生成 (Claude Code / sonnet) ───
─── 阶段: Running ───
[Claude Code] 正在分析需求...
  🔧 Read src/auth/login.rs
  🔧 Search "phone login"
  🔐 权限请求: Bash `grep -r "auth" src/` [允许] [拒绝]
  → [已允许]
[Claude Code] 根据分析，生成 Story Spec...
  📄 产物已更新 → v1
─── 阶段: CrossReview ───
[Codex (Reviewer)] 审查 Story Spec v1...
  审核结论: revise — "缺少错误处理场景的验收标准"
  [接受修订建议] [补充上下文后修订] [跳过，人工处理]
─── 阶段: Revision ───
[Claude Code] 根据 reviewer 反馈修订...
  📄 产物已更新 → v2
─── 阶段: CrossReview ───
[Codex (Reviewer)] 审查 Story Spec v2...
  审核结论: pass — "验收标准完整，覆盖正常和异常场景"
─── 阶段: HumanConfirm ───
[系统] Story Spec v2 已通过审核，请确认
  [确认] [要求修改] [终止]
[user] 已确认
─── 阶段: Completed ───
```

### 9.2 断开重连恢复

1. WebSocket 断开 → DisconnectBanner 显示
2. 自动重连成功 → 服务端发送 `session_state` 完整快照
3. 前端从快照重建 `chatEntries` 列表
4. chat 流恢复到断开前的状态，自动滚动到底部
5. 如果断开期间 provider 被中止 → 插入 `aborted_by_disconnect` error entry

---

## 10. 测试策略

### 10.1 单元测试

- `ChatEntryList`：渲染各类型 entry、自动滚动、节点跳转
- `ChatInputBar`：阶段状态切换、按钮可用性、发送逻辑
- `ArtifactPane`：版本切换、diff toggle
- Store 扩展：chatEntries 追加、流式更新、快照重建

### 10.2 E2E 测试

- 完整 PrepareContext → Running → Review → HumanConfirm 流程
- 权限请求内联操作
- 断开重连后 chat 流恢复
- 左侧节点点击跳转
- Artifact 面板版本切换和 diff

### 10.3 现有 E2E 迁移

现有 `web/e2e/` 下的测试需要适配新组件的 `data-testid`：
- `stage-ui.spec.ts` → 适配 ChatInputBar 的阶段状态
- `permission-link.spec.ts` → 适配 PermissionRequestEntry
- `timeline-audit.spec.ts` → 适配 ChatEntryList + TimelineNodeList
- `websocket-reconnect.spec.ts` → 适配断开恢复后的 chat 流重建
- `disconnect-strategy.spec.ts` → 适配 DisconnectBanner + error entry
