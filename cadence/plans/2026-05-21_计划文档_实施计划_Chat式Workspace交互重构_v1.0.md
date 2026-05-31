# 实施计划：Chat 式 Workspace 交互重构

> 版本：v1.1 | 日期：2026-05-21
> 设计文档：`cadence/designs/2026-05-21_技术方案_Chat式Workspace交互重构_v1.0.md`

## 概述

将 WorkspacePage 从 "stage panel + tab 详情" 模式重构为 chat 对话流模式。新建 `ChatWorkspacePage`，完成后删除旧 `WorkspacePage`。

**拆分为 3 个独立 Plan**，每个 Plan 在单次会话中可完成并独立验证。

## 技术栈确认

- React 19 + TypeScript 5.7 + Vite 6
- Zustand v5（状态管理）
- Tailwind CSS v3（手写组件，无 shadcn/ui）
- lucide-react（图标）
- @tanstack/react-router v1（路由）
- Vitest + @testing-library/react（单元测试）
- Playwright（E2E 测试）

---

## Plan 1：数据层 + 核心 Entry 组件

**范围**：Phase 1（数据层）+ Phase 2 的 P0 组件（6 个核心 entry）
**预估文件数**：~10 个新建/修改
**验证点**：`pnpm test` 通过，store 扩展不破坏现有测试，核心 entry 组件可独立渲染

### 1.1 ChatEntry 类型定义

新建 `web/src/state/chat-entries.ts`：

```typescript
export type ChatEntryType =
  | "context_note"
  | "start_generation"
  | "provider_stream"
  | "execution_event"
  | "permission_request"
  | "permission_response"
  | "artifact_update"
  | "review_verdict"
  | "gate_prompt"
  | "human_decision"
  | "stage_change"
  | "error";

export type ChatEntryRole = "user" | "author" | "reviewer" | "system";

export interface ChatEntry {
  id: string;
  type: ChatEntryType;
  role: ChatEntryRole;
  content: string;
  timestamp: string;
  node_id?: string;
  metadata?: Record<string, unknown>;
}
```

### 1.2 Store 扩展

修改 `web/src/state/workspace-ws-store.ts`，追加字段和 actions：

```typescript
// 新增状态
chatEntries: ChatEntry[];
activeStreamEntryId: string | null;

// 新增 actions
appendChatEntry: (entry: ChatEntry) => void;
updateStreamingEntry: (entryId: string, content: string) => void;
finalizeStreamingEntry: (entryId: string) => void;
rebuildChatEntries: () => void;
```

### 1.3 SSE → ChatEntry 映射

修改 `web/src/hooks/useWorkspaceWs.ts` 的 `handleMessage`，在现有处理逻辑之后追加 chatEntry 生成调用。

映射规则：
| SSE 事件 | ChatEntry type | role |
|---------|---------------|------|
| `session_state` | 调用 `rebuildChatEntries()` | — |
| context_note（来自 messages） | `context_note` | user |
| `provider_locked` | `start_generation` | system |
| `stage_change` | `stage_change` | system |
| `stream_chunk` | 追加到 `provider_stream` entry | author/reviewer |
| `execution_event` | `execution_event` | system |
| `permission_request` | `permission_request` | system |
| `artifact_update` | `artifact_update` | system |
| `review_complete` | `review_verdict` | reviewer |
| stage=HumanConfirm | `gate_prompt` | system |
| `protocol_error` / `error` | `error` | system |

### 1.4 核心 Entry 组件（P0）

新建 `web/src/components/chat-workspace/` 目录：

| 文件 | 说明 |
|------|------|
| `ChatEntryContainer.tsx` | 基础容器（role 决定样式） |
| `entries/UserContextEntry.tsx` | 用户消息气泡 |
| `entries/ProviderStreamEntry.tsx` | 流式输出 + Markdown |
| `entries/ExecutionEventEntry.tsx` | tool call 摘要行 |
| `entries/PermissionRequestEntry.tsx` | 权限请求卡片 |
| `entries/PermissionResponseEntry.tsx` | 权限响应标签 |
| `entries/ErrorEntry.tsx` | 错误卡片 |
| `ChatEntryRenderer.tsx` | type → 组件分发 |

### 1.5 测试

- `web/src/state/chat-entries.test.ts` — store actions + 映射逻辑
- `web/src/components/chat-workspace/entries/entries.test.tsx` — 各 entry 组件渲染

### 1.6 验收标准

- [ ] `pnpm test` 全部通过（含现有测试）
- [ ] chatEntries store actions 有完整测试
- [ ] 6 个核心 entry 组件有渲染测试
- [ ] SSE 事件正确映射为 ChatEntry

---

## Plan 2：补充 Entry 组件 + Chat 区域主体

**范围**：Phase 2 的 P1 组件（6 个）+ Phase 3（ChatEntryList + ChatInputBar）
**前置**：Plan 1 完成
**预估文件数**：~12 个新建/修改
**验证点**：`pnpm test` 通过，chat 区域完整可交互

### 2.1 补充 Entry 组件（P1）

| 文件 | 说明 |
|------|------|
| `entries/StartGenerationEntry.tsx` | 分隔线 + "▶ 开始生成" |
| `entries/StageChangeEntry.tsx` | 阶段变更分隔线 |
| `entries/ArtifactUpdateEntry.tsx` | "产物已更新 → vN" 标签 |
| `entries/ReviewVerdictEntry.tsx` | 结论卡片 + 路径选择按钮 |
| `entries/GatePromptEntry.tsx` | 确认卡片 + [确认][修改][终止] |
| `entries/HumanDecisionEntry.tsx` | 用户决策气泡 |
| `entries/index.ts` | 统一导出 |

### 2.2 ChatEntryList

新建 `web/src/components/chat-workspace/ChatEntryList.tsx`：
- 渲染 chatEntries 列表，通过 ChatEntryRenderer 分发
- 自动滚动到底部（新 entry 到达时）
- 暴露 `scrollToEntry(entryId)` 方法
- `data-entry-id` 属性标记 DOM 节点

### 2.3 ChatInputBar

新建 `web/src/components/chat-workspace/ChatInputBar.tsx`：
- 阶段状态矩阵控制输入框/按钮可用性
- PrepareContext：textarea + 发送 + 开始生成
- Running/CrossReview/Revision：禁用 + 中止按钮
- HumanConfirm：textarea（修改意见）+ 发送
- 乐观插入 chatEntry

### 2.4 测试

- `web/src/components/chat-workspace/entries/p1-entries.test.tsx`
- `web/src/components/chat-workspace/ChatEntryList.test.tsx`
- `web/src/components/chat-workspace/ChatInputBar.test.tsx`

### 2.5 验收标准

- [ ] `pnpm test` 全部通过
- [ ] 所有 12 个 entry 类型有测试覆盖
- [ ] ChatInputBar 各阶段状态切换正确
- [ ] 乐观插入 entry 在 UI 中立即可见
- [ ] 自动滚动行为正确

---

## Plan 3：面板 + 页面组装 + 路由切换 + 清理

**范围**：Phase 4（ArtifactPane + TimelineNodeList）+ Phase 5（页面组装）+ Phase 6（E2E + 清理）
**前置**：Plan 2 完成
**预估文件数**：~12 个新建/修改/删除
**验证点**：`pnpm test` + `pnpm test:e2e` 全部通过，旧代码已删除

### 3.1 ArtifactPane

新建 `web/src/components/chat-workspace/ArtifactPane.tsx`：
- Markdown 渲染最新版本产物
- 版本号选择器 + Diff toggle
- 可折叠/展开

### 3.2 TimelineNodeList

新建 `web/src/components/chat-workspace/TimelineNodeList.tsx`：
- 渲染 timelineNodes 列表（图标 + 标题 + 状态）
- 点击节点 → scrollToEntry
- 活跃节点高亮，已完成节点 ✓

### 3.3 ChatWorkspacePage 组装

新建 `web/src/pages/ChatWorkspacePage.tsx`：
- TopBar：返回 + EntityTitle + ProviderConfig + ConnectionIndicator
- DisconnectBanner（复用）
- 三栏 grid：TimelineNodeList + ChatArea + ArtifactPane
- StatusBar：阶段 + 连接 + 耗时
- 响应式：≥1024 三栏 / 768-1023 Artifact 可折叠 / <768 Timeline 下拉 + Artifact 抽屉

### 3.4 路由切换

修改 `web/src/router.tsx`：
```typescript
<Route path="/workbench/workspace/$sessionId" component={ChatWorkspacePage} />
```

### 3.5 E2E 测试适配

- `stage-ui.spec.ts` → ChatInputBar 的 data-testid
- `permission-link.spec.ts` → PermissionRequestEntry
- `timeline-audit.spec.ts` → ChatEntryList + TimelineNodeList
- `websocket-reconnect.spec.ts` → chat 流重建
- `disconnect-strategy.spec.ts` → DisconnectBanner + error entry

### 3.6 旧代码清理

删除：
- `web/src/pages/WorkspacePage.tsx`
- `web/src/pages/WorkspacePage.test.tsx`
- `web/src/components/workspace/PrepareContextPanel.tsx`
- `web/src/components/workspace/NodeDetailPanel.tsx`
- `web/src/components/workspace/StageActionsBar.tsx`
- `web/src/components/workspace/stages/HumanConfirmStagePanel.tsx`
- `web/src/components/workspace/stages/ReviewDecisionStagePanel.tsx`

### 3.7 验收标准

- [ ] `pnpm test` 全部通过
- [ ] `pnpm test:e2e` 全部通过
- [ ] 旧 WorkspacePage 及相关组件已删除
- [ ] 路由指向 ChatWorkspacePage
- [ ] 三栏布局响应式正确

---

## 依赖关系

```
Plan 1 (数据层 + 核心 Entry)
    ↓
Plan 2 (补充 Entry + Chat 区域)
    ↓
Plan 3 (面板 + 组装 + 清理)
```

## 风险与缓解

| 风险 | 缓解措施 |
|------|---------|
| Store 扩展破坏现有功能 | Plan 1 仅追加字段和 actions，不修改现有接口 |
| 流式追加性能问题 | ProviderStreamEntry 使用 `React.memo` + 仅追加 content |
| E2E 测试大面积失败 | Plan 3 集中处理，旧页面在 Plan 3 之前保持可用 |
| 上下文窗口溢出 | 每个 Plan 独立可完成，不跨会话依赖 |
