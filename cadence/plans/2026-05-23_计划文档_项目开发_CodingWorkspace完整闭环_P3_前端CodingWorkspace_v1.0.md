# Coding Workspace P3：前端 Coding Workspace

## 文档信息

- 文档类型：计划文档
- 分支：`product-workbench-issue-lifecycle`
- 制定日期：2026-05-23
- 版本：v1.0
- 前置：P2（后端 Engine 与 WebSocket）
- 产出：CodingWorkspacePage、组件、Store、Hook、路由

---

## 一、目标

实现 Coding Workspace 的完整前端：

1. 新增路由 `/workbench/coding/$attemptId`
2. 新建 `CodingWorkspacePage` 页面
3. 新建 `coding-workspace-store`（zustand）
4. 新建 `useCodingWorkspaceWs` hook
5. 新建 Coding Timeline、Artifact Tabs（Diff/Tests/Review/Git/Logs）组件
6. 修改 `LifecycleCardDrawer` 添加"开始 Coding"入口
7. 扩展 `ChatEntryRenderer` 支持 coding entry 类型

---

## 二、任务清单

### 2.1 路由注册

**文件**：`web/src/router.tsx`（修改）

| # | 任务 | 测试先行 | 验收 |
|---|------|---------|------|
| 1.1 | 新增 `/workbench/coding/$attemptId` 路由 | — | 路由匹配正确 |
| 1.2 | 新增 `CodingWorkspaceRouteComponent` | — | 传递 attemptId 和 onBack |

**代码变更**：

````typescript
// 新增 import
import { CodingWorkspacePage } from "./pages/CodingWorkspacePage";

// 新增路由组件
function CodingWorkspaceRouteComponent() {
  const { attemptId } = useParams({ from: "/workbench/coding/$attemptId" });
  const navigate = useNavigate();
  return (
    <CodingWorkspacePage
      attemptId={attemptId}
      onBack={() => void navigate({ to: "/workbench" })}
    />
  );
}

// 新增路由定义
const codingWorkspaceRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/workbench/coding/$attemptId",
  component: CodingWorkspaceRouteComponent,
});

// routeTree 添加
const routeTree = rootRoute.addChildren([
  indexRoute, workbenchRoute, workspaceRoute, codingWorkspaceRoute
]);
````

### 2.2 coding-workspace-store（新建）

**文件**：`web/src/state/coding-workspace-store.ts`（新建）

| # | 任务 | 测试先行 | 验收 |
|---|------|---------|------|
| 2.1 | 定义 `CodingWorkspaceState` 类型 | 类型测试 | 所有字段与后端模型对应 |
| 2.2 | 定义 `CodingWorkspaceActions` | — | 所有 action 方法签名 |
| 2.3 | 实现 `setSessionState` | 单元测试 | 从 snapshot 初始化全部状态 |
| 2.4 | 实现 `updateStage` | 单元测试 | stage 变化更新 |
| 2.5 | 实现 Timeline 管理（add/update node） | 单元测试 | 节点增删改 |
| 2.6 | 实现 Artifact 管理（diff/tests/review/git） | 单元测试 | 各 tab 数据更新 |
| 2.7 | 实现 Gate 管理（pending gates） | 单元测试 | gate 添加/解决 |
| 2.8 | 实现 `buildCodingChatEntries` | 单元测试 | 从 timeline + events 构建 entries |
| 2.9 | 实现 `reset` | 单元测试 | 重置到初始状态 |

**State 结构**：

````typescript
interface CodingWorkspaceState {
  // Attempt 基础
  attemptId: string | null;
  workItemId: string | null;
  issueId: string | null;
  projectId: string | null;
  status: CodingAttemptStatus | null;
  stage: CodingExecutionStage | null;
  branchName: string | null;
  baseBranch: string | null;
  worktreePath: string | null;
  reworkCount: number;
  maxAutoRework: number;
  headCommit: string | null;
  pushedRemote: string | null;
  providerConfigSnapshot: ProviderConfigSnapshot | null;

  // Timeline
  timelineNodes: CodingTimelineNode[];
  activeNodeId: string | null;
  selectedNodeId: string | null;

  // Chat
  chatEntries: ChatEntry[];
  streamingContent: string | null;
  activeStreamNodeId: string | null;

  // Artifacts
  activeTab: "diff" | "tests" | "review" | "git" | "logs";
  diffSummary: DiffSummary | null;
  testingReport: TestingReport | null;
  codeReviewReports: CodeReviewReport[];
  internalPrReview: InternalPrReview | null;
  reviewRequest: ReviewRequest | null;
  logs: LogEntry[];

  // 连接与 Gate
  connectionStatus: "connecting" | "connected" | "disconnected" | "reconnecting";
  pendingGates: CodingGateRequired[];
  protocolError: { code: string; message: string } | null;

  // Tab 联动
  tabLockedByUser: boolean;
}
````

### 2.3 useCodingWorkspaceWs（新建）

**文件**：`web/src/hooks/useCodingWorkspaceWs.ts`（新建）

| # | 任务 | 测试先行 | 验收 |
|---|------|---------|------|
| 3.1 | 实现 WebSocket 连接到 `/ws/coding-attempts/:attemptId` | 单元测试 | 连接建立 |
| 3.2 | 实现握手（CodingHello） | 单元测试 | 发送 attempt_id + last_seen_node_id |
| 3.3 | 实现服务端消息分发 | 单元测试 | 每种消息类型正确更新 store |
| 3.4 | 实现客户端动作方法 | 单元测试 | startCoding/contextNote/permission/gate/confirm/abort |
| 3.5 | 实现 ping/pong 心跳 | — | 25s 间隔 |
| 3.6 | 实现断连重连 | 单元测试 | 指数退避，重连后恢复 snapshot |

**消息处理映射**：

| 服务端消息 | Store action |
|-----------|-------------|
| `coding_session_state` | `setSessionState` + `buildCodingChatEntries` |
| `coding_stage_change` | `updateStage` + 追加 stage_change entry |
| `coding_timeline_node_created` | `addTimelineNode` |
| `coding_timeline_node_updated` | `updateTimelineNode` |
| `coding_execution_event` | 追加 execution_event entry + 更新 logs |
| `coding_stream_chunk` | 更新 streamingContent |
| `coding_message_complete` | 完成 streaming entry |
| `testing_report_update` | `setTestingReport` + 追加 testing_summary entry |
| `code_review_complete` | `addCodeReviewReport` + 追加 review_verdict entry |
| `review_request_update` | `setReviewRequest` + 追加 artifact_update entry |
| `internal_pr_review_complete` | `setInternalPrReview` + 追加 review_verdict entry |
| `coding_gate_required` | `addPendingGate` + 追加 gate_prompt entry |
| `coding_protocol_error` | `setProtocolError` |

### 2.4 CodingWorkspacePage（新建）

**文件**：`web/src/pages/CodingWorkspacePage.tsx`（新建）

| # | 任务 | 测试先行 | 验收 |
|---|------|---------|------|
| 4.1 | 页面骨架（三栏布局） | 组件测试 | Header + Timeline + Chat + Artifacts + StatusBar |
| 4.2 | 连接 useCodingWorkspaceWs | 组件测试 | 连接建立，snapshot 加载 |
| 4.3 | Header 渲染 | 组件测试 | 标题、status badge、stage、branch |
| 4.4 | StatusBar 渲染 | 组件测试 | stage、连接状态、耗时 |
| 4.5 | 页面卸载保护 | 组件测试 | running 时提示 |

**Props**：

````typescript
interface CodingWorkspacePageProps {
  attemptId: string;
  onBack: () => void;
}
````

### 2.5 CodingTimeline（新建）

**文件**：`web/src/components/coding-workspace/CodingTimeline.tsx`（新建）

| # | 任务 | 测试先行 | 验收 |
|---|------|---------|------|
| 5.1 | 节点列表渲染 | 组件测试 | 按顺序展示所有节点 |
| 5.2 | 节点图标映射 | 组件测试 | 每种 stage 对应正确图标 |
| 5.3 | 节点状态样式 | 组件测试 | pending/running/completed/failed/blocked 样式 |
| 5.4 | 点击交互 | 组件测试 | 点击节点 → 更新 selectedNodeId + 切换 tab |
| 5.5 | active 节点高亮 | 组件测试 | 当前 active 节点视觉突出 |

### 2.6 CodingArtifactTabs（新建）

**文件**：`web/src/components/coding-workspace/CodingArtifactTabs.tsx`（新建）

| # | 任务 | 测试先行 | 验收 |
|---|------|---------|------|
| 6.1 | Tab 容器和切换逻辑 | 组件测试 | 5 个 tab 切换正确 |
| 6.2 | Tab 联动逻辑 | 组件测试 | Timeline 点击自动切换，用户手动切后锁定 |

### 2.7 DiffTab（新建）

**文件**：`web/src/components/coding-workspace/DiffTab.tsx`（新建）

| # | 任务 | 测试先行 | 验收 |
|---|------|---------|------|
| 7.1 | 变更统计展示 | 组件测试 | +N -M，文件数 |
| 7.2 | 文件列表 | 组件测试 | 路径 + 增删行数 + 状态 |
| 7.3 | Monaco diff viewer 集成 | 组件测试 | 点击文件展开 diff |
| 7.4 | 空状态 | 组件测试 | 无变更时提示 |

### 2.8 TestsTab（新建）

**文件**：`web/src/components/coding-workspace/TestsTab.tsx`（新建）

| # | 任务 | 测试先行 | 验收 |
|---|------|---------|------|
| 8.1 | 整体状态 badge | 组件测试 | passed/failed/running/blocked |
| 8.2 | 命令卡片列表 | 组件测试 | 命令文本 + 状态 + exit code + 耗时 |
| 8.3 | stdout/stderr 展开 | 组件测试 | MonacoViewer 展示 |
| 8.4 | Provider claim 折叠区 | 组件测试 | 标注"仅供参考" |

### 2.9 ReviewTab（新建）

**文件**：`web/src/components/coding-workspace/ReviewTab.tsx`（新建）

| # | 任务 | 测试先行 | 验收 |
|---|------|---------|------|
| 9.1 | 按类型分组（Code Review / Internal PR Review） | 组件测试 | 两组分开展示 |
| 9.2 | Verdict badge | 组件测试 | approve/request_changes/blocked |
| 9.3 | Findings 卡片 | 组件测试 | severity + 文件 + 行号 + 描述 |
| 9.4 | 多轮分组 | 组件测试 | 按轮次展示 |

### 2.10 GitTab（新建）

**文件**：`web/src/components/coding-workspace/GitTab.tsx`（新建）

| # | 任务 | 测试先行 | 验收 |
|---|------|---------|------|
| 10.1 | Branch 信息 | 组件测试 | base → attempt branch |
| 10.2 | Commit 信息 | 组件测试 | sha + message + 时间 |
| 10.3 | Push 状态 | 组件测试 | not_pushed/pushed/failed |
| 10.4 | Review Request 展示 | 组件测试 | URL 链接或手动指引 |
| 10.5 | Worktree 路径（可复制） | 组件测试 | 点击复制 |

### 2.11 LogsTab（新建）

**文件**：`web/src/components/coding-workspace/LogsTab.tsx`（新建）

| # | 任务 | 测试先行 | 验收 |
|---|------|---------|------|
| 11.1 | 事件流列表 | 组件测试 | 时间戳 + agent 标签 + 内容 |
| 11.2 | Agent 筛选 | 组件测试 | author/tester/reviewer/system |
| 11.3 | 长内容 Monaco 展示 | 组件测试 | plaintext 模式 |

### 2.12 Coding Chat Entry 扩展

**文件**：`web/src/components/chat-workspace/ChatEntryRenderer.tsx`（修改）

| # | 任务 | 测试先行 | 验收 |
|---|------|---------|------|
| 12.1 | 新增 `testing_summary` entry 渲染 | 组件测试 | 系统卡片，pass/fail 摘要 |
| 12.2 | 新增 `review_verdict` entry 渲染（coding 版） | 组件测试 | reviewer 气泡 + verdict |
| 12.3 | 新增 `gate_prompt` entry 渲染（coding 版） | 组件测试 | 动态按钮组 |

**新增 entry 渲染器文件**：
- `web/src/components/coding-workspace/entries/TestingSummaryEntry.tsx`
- `web/src/components/coding-workspace/entries/ReviewVerdictEntry.tsx`

### 2.13 LifecycleCardDrawer 入口修改

**文件**：`web/src/components/lifecycle/LifecycleCardDrawer.tsx`（修改）

| # | 任务 | 测试先行 | 验收 |
|---|------|---------|------|
| 13.1 | Work Item drawer 新增 coding 区块 | 组件测试 | 展示 execution status、attempt 信息 |
| 13.2 | 按状态展示不同按钮 | 组件测试 | 见前端设计 7.1 节 |
| 13.3 | "开始 Coding" 按钮调用创建 API | 组件测试 | POST 成功后路由跳转 |
| 13.4 | "进入 Coding Workspace" 按钮 | 组件测试 | 路由跳转到 coding page |

**按钮逻辑**：

````typescript
// 判断逻辑
function getCodingAction(workItem: LifecycleWorkItemDto): CodingAction {
  if (workItem.plan_status !== "confirmed") return { type: "none" };
  
  const attempt = workItem.latest_attempt;
  if (!attempt) return { type: "start_coding" };
  
  switch (attempt.status) {
    case "running":
    case "waiting_for_human":
      return { type: "enter_workspace", attemptId: attempt.attempt_id };
    case "blocked":
      return { type: "handle_blocker", attemptId: attempt.attempt_id };
    case "completed":
      return { type: "view_result", attemptId: attempt.attempt_id };
    default:
      return { type: "start_coding" };
  }
}
````

### 2.14 IssueLifecycleWorkbench 集成

**文件**：`web/src/components/lifecycle/IssueLifecycleWorkbench.tsx`（修改）

| # | 任务 | 测试先行 | 验收 |
|---|------|---------|------|
| 14.1 | Work Item 卡片状态标记 | 组件测试 | 可编码/running/completed 标记 |
| 14.2 | 路由跳转到 coding workspace | 组件测试 | 使用 navigate 而非 window.location |

### 2.15 API 客户端

**文件**：`web/src/api/coding-attempts.ts`（新建）

| # | 任务 | 验收 |
|---|------|------|
| 15.1 | `createCodingAttempt(projectId, issueId, workItemId)` | 返回 attempt_id |
| 15.2 | `getCodingAttempt(attemptId)` | 返回 attempt snapshot |
| 15.3 | `abortCodingAttempt(attemptId)` | 中止成功 |

API 客户端必须补充 `web/src/api/coding-attempts.test.ts`，覆盖 URL、payload、错误透传和 snapshot 字段映射。

---

## 三、完成标准

- [ ] `pnpm test` 全部通过
- [ ] `pnpm exec tsc --noEmit` 无错误
- [ ] 路由跳转正确
- [ ] CodingWorkspacePage 三栏布局渲染正确
- [ ] WebSocket 连接建立，snapshot 加载
- [ ] Timeline 节点展示和交互正确
- [ ] 5 个 Artifact tabs 渲染正确
- [ ] "开始 Coding" 入口在 Plan confirmed 后可见
- [ ] blocked gate 动态按钮渲染正确
- [ ] 断连重连后 snapshot 恢复

### 3.1 2026-05-23 实施审计说明

本轮 P3 以前端可用闭环为优先级，已覆盖路由、页面骨架、zustand store、Coding WebSocket hook、API client、Lifecycle Drawer 入口、Work Item 卡片 coding 状态标记、pending gate 动态按钮、Timeline 到 Artifact tab 联动、context note 输入、Review findings 展示与 Git ReviewRequest 展示。

当前实现允许将 `CodingTimeline`、`CodingArtifactTabs`、`DiffTab`、`TestsTab`、`ReviewTab`、`GitTab`、`LogsTab` 先内联在 `CodingWorkspacePage.tsx` 中，以降低 P3 MVP 的跨文件变更面。后续如果继续扩展 Diff/Tests/Logs 的复杂交互，再按本计划第 2.5-2.11 节拆出独立组件文件。

以下能力仍属于 P3 后续增强，不作为本轮 MVP 阻断项：

- `DiffTab` 的 DiffSummary 数据模型、文件级 diff 列表与 Monaco diff viewer。
- `TestsTab` 的 stdout/stderr artifact 拉取、展开与 MonacoViewer 展示。
- `LogsTab` 的 agent 筛选与长日志 plaintext viewer。
- coding 专用 `testing_summary` / `review_verdict` entry 组件文件拆分。
- Git tab 的 worktree 路径复制交互。

---

## 四、不在本阶段范围

- 后端 Engine 实现（P2）
- 数据模型（P1）
- E2E 真实验收（需要 P1+P2+P3 全部完成后）
- GitLab/GitHub 外部平台集成
- Worktree 清理 UI
