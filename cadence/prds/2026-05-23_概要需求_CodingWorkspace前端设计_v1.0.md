# Coding Workspace 前端设计规格

## 文档信息

- 文档类型：概要需求（前端设计规格）
- 分支：`product-workbench-issue-lifecycle`
- 制定日期：2026-05-23
- 版本：v1.0
- 依据：`cadence/designs/2026-05-23_技术方案_CodingWorkspace完整闭环_v1.0.md`

---

## 一、概述

Work Item Plan confirmed 后，用户需要在 Product Workbench 中启动 Coding Workspace，完成代码编写、测试、审查、返工、提交和最终确认的完整闭环。本文档定义 Coding Workspace 的前端交互设计、组件结构和状态管理方案。

---

## 二、已确认决策

| # | 决策点 | 选择 |
|---|---|---|
| 1 | 前端架构方案 | 方案 B：抽取共享层，两个 workspace 各自组合 |
| 2 | 路由方式 | 独立路由 `/workbench/coding/$attemptId`，新建 `CodingWorkspacePage` |
| 3 | 右侧 Artifact tabs | 5 个 tab：Diff、Tests、Review、Git、Logs |
| 4 | Timeline 节点 | 完全独立于 doc workspace，使用 coding 专用节点类型和图标 |
| 5 | Chat 输入 | 支持自由文本输入（补充上下文给 provider） |

---

## 三、页面布局

### 3.1 整体结构

三栏布局，与 doc workspace 对齐：

```
┌─────────────────────────────────────────────────────────────┐
│ Header: Work Item 标题 | attempt status | stage | branch    │
├──────────┬──────────────────────────┬───────────────────────┤
│ Timeline │       Chat Panel         │   Artifact Tabs       │
│ (16rem)  │    (flex-1, 中间)         │  (18-24rem, 右侧)     │
│          │                          │                       │
│ coding   │  provider streaming      │  [Diff][Tests][Review]│
│ timeline │  execution events        │  [Git][Logs]          │
│ nodes    │  permission gates        │                       │
│          │  user messages           │  tab 内容区域          │
│          │  blocked/confirm gates   │                       │
│          │                          │                       │
│          ├──────────────────────────┤                       │
│          │  ChatInputBar            │                       │
│          │  (自由文本 + 操作按钮)     │                       │
└──────────┴──────────────────────────┴───────────────────────┘
│ StatusBar: stage | connection | elapsed                      │
└─────────────────────────────────────────────────────────────┘
```

### 3.2 Header

- 返回按钮（回到 workbench）
- Work Item 标题
- Attempt 状态 badge（running / blocked / completed）
- 当前 stage
- Branch 名称
- Provider 配置按钮
- 连接状态图标

### 3.3 StatusBar

- 当前 stage
- 连接状态
- 当前 active 节点耗时

---

## 四、左栏 — Coding Timeline

### 4.1 节点类型与图标

| 节点类型 | 图标（lucide） | 标题示例 |
|---------|---------------|---------|
| `worktree_prepare` | FolderGit | "创建工作区" |
| `coding` | Code | "代码编写" |
| `testing` | FlaskConical | "执行测试" |
| `code_review` | SearchCode | "代码审查" |
| `rework` | RefreshCw | "返工 #1" |
| `review_request` | GitPullRequest | "提交审查请求" |
| `internal_pr_review` | ShieldCheck | "内部 PR 审查" |
| `final_confirm` | UserCheck | "最终确认" |

### 4.2 节点状态显示

- `pending`：灰色圆点
- `running`：蓝色脉冲动画
- `completed`：绿色勾
- `failed`：红色叉
- `blocked`：橙色感叹号

### 4.3 交互行为

- 点击节点 → 中间 Chat 栏滚动到该节点对应的 entries
- 点击节点 → 右侧 Artifact tabs 自动切换到最相关的 tab：
  - `worktree_prepare` → Git
  - `coding` / `rework` → Diff
  - `testing` → Tests
  - `code_review` / `internal_pr_review` → Review
  - `review_request` → Git
  - `final_confirm` → 不切换
- 用户手动切 tab 后，不再被 Timeline 点击覆盖（直到用户再次点击 Timeline 节点）
- 当前 active 节点高亮显示
- 节点可能出现多次（如 rework 可能有多个节点）

---

## 五、中间栏 — Chat Panel

### 5.1 复用组件

直接复用 doc workspace 的：
- `ChatEntryList`：渲染对话条目列表，支持滚动定位
- `ChatInputBar`：底部输入框（扩展 coding stage 按钮映射）
- `ChatEntryRenderer`：单条 entry 渲染（扩展 coding entry 类型）

### 5.2 Chat Entry 类型

| entry 类型 | 来源 | 展示形式 |
|-----------|------|---------|
| `context_note` | 用户自由输入 | 用户气泡 |
| `provider_stream` | provider coding/rework 输出 | assistant 气泡，流式渲染 |
| `execution_event` | 后端命令执行（git、test） | 系统卡片，命令+状态+耗时 |
| `permission_request` | provider 请求高风险操作 | 系统卡片 + 批准/拒绝按钮 |
| `permission_response` | 用户响应 | 用户气泡 |
| `testing_summary` | 测试完成 | 系统卡片，pass/fail 摘要 |
| `review_verdict` | code review / PR review 结论 | reviewer 气泡 |
| `gate_prompt` | blocked / final confirm | 系统卡片 + 操作按钮组 |
| `artifact_update` | diff/commit 产生 | 系统通知条 |

### 5.3 ChatInputBar Stage 适配

| 当前 stage | 输入框行为 | 操作按钮 |
|-----------|-----------|---------|
| `prepare_context` | 可输入补充上下文 | "开始 Coding" 主按钮 |
| `coding` / `testing` / `code_review` | 可输入 | "中止" 按钮 |
| `rework` | 可输入 | "中止" 按钮 |
| `review_request` / `internal_pr_review` | 可输入 | "中止" 按钮 |
| `final_confirm` | 可输入 | "确认完成" / "要求返工" / "中止" 按钮组 |
| `blocked` | 可输入 | "继续返工" / "暂停" / "放弃" 按钮组 |
| `completed` / `aborted` | 只读 | 无 |

---

## 六、右栏 — Artifact Tabs

### 6.1 Tab 容器

顶部固定一行 tab 按钮：`Diff` | `Tests` | `Review` | `Git` | `Logs`。选中态高亮，内容区域占满剩余高度。

### 6.2 Diff Tab

- 顶部：变更统计（`+42 -15`，N 个文件）
- 文件列表：文件路径 + 增删行数 + 状态（added/modified/deleted）
- 点击文件 → 展开 Monaco diff viewer（复用 `MonacoDiffViewer`）
- 无变更时显示空状态："暂无代码变更"

### 6.3 Tests Tab

- 顶部：整体状态 badge（passed / failed / running / blocked）
- 命令列表：每条命令一个卡片
  - 命令文本
  - 状态图标 + exit code
  - 耗时
  - 展开 → stdout/stderr（MonacoViewer，language=plaintext）
- Provider claim 区域（折叠，标注"仅供参考，非后端验证"）

### 6.4 Review Tab

- Verdict badge（approve / request_changes / blocked）
- Findings 列表：每条 finding 一个卡片
  - severity badge（error / warning / info）
  - 文件路径 + 行号
  - 问题描述
  - 建议操作
- Review 摘要文本
- 多轮 review 按轮次分组展示

### 6.5 Git Tab

- Branch 信息：base branch → attempt branch
- Commit 信息：sha、message、时间
- Push 状态：not_pushed / pushed / failed
- Review Request：
  - 有外部 URL → 可点击链接
  - branch-only → 手动创建指引（可复制命令）
- Worktree 路径（可复制）

### 6.6 Logs Tab

- 按时间排序的事件流
- 每条事件：时间戳 + agent 标签 + 内容
- 支持按 agent 筛选（author / tester / reviewer / system）
- 长内容用 MonacoViewer 展示（language=plaintext）

### 6.7 Tab 联动

Timeline 节点点击时自动切换到相关 tab（见 4.3）。用户手动切 tab 后，Timeline 点击不再覆盖，直到用户再次主动点击 Timeline 节点。

---

## 七、入口与状态联动

### 7.1 LifecycleCardDrawer 入口按钮

| Work Item 状态 | 主按钮 | 辅助按钮 |
|---------------|--------|---------|
| Plan 未确认 | "打开 Plan Workspace" | 无 |
| Plan 已确认，无 active attempt | "开始 Coding" | "打开 Plan Workspace"（查看） |
| Attempt running | "进入 Coding Workspace" | "中止" |
| Attempt blocked | "处理 Blocker" | "查看详情" |
| Attempt completed | "查看结果" | "再次 Coding"（创建新 attempt） |

### 7.2 "开始 Coding" 流程

1. 用户点击"开始 Coding"
2. 前端调用 `POST /api/projects/:pid/issues/:iid/work-items/:wid/coding-attempts`
3. 后端返回 `attempt_id`
4. 前端路由跳转到 `/workbench/coding/$attemptId`
5. CodingWorkspacePage 建立 WebSocket 连接，进入 `prepare_context` 阶段

### 7.3 Work Item 卡片状态标记

- Plan 未确认：无 coding 标记
- Plan 已确认，无 attempt：显示"可编码"标记
- Attempt running：脉冲动画 + 当前 stage
- Attempt completed：绿色完成标记

---

## 八、组件目录结构

```
web/src/
├── pages/
│   └── CodingWorkspacePage.tsx          # 新增
├── components/
│   ├── shared/                          # 已有，复用
│   │   ├── MonacoViewer.tsx
│   │   └── MonacoDiffViewer.tsx
│   ├── chat-workspace/                  # 已有，部分组件复用
│   │   ├── ChatEntryList.tsx            # 复用
│   │   ├── ChatEntryRenderer.tsx        # 复用（扩展 coding entry 类型）
│   │   ├── ChatInputBar.tsx             # 复用（扩展 coding stage 按钮）
│   │   ├── TimelineNodeList.tsx         # doc workspace 专用
│   │   └── ArtifactPane.tsx             # doc workspace 专用
│   ├── coding-workspace/                # 新增
│   │   ├── CodingTimeline.tsx           # coding 节点列表 + 图标映射
│   │   ├── CodingArtifactTabs.tsx       # tab 容器 + 切换逻辑
│   │   ├── DiffTab.tsx
│   │   ├── TestsTab.tsx
│   │   ├── ReviewTab.tsx
│   │   ├── GitTab.tsx
│   │   ├── LogsTab.tsx
│   │   └── entries/                     # coding 特有 entry 渲染器
│   │       ├── TestingSummaryEntry.tsx
│   │       └── ReviewVerdictEntry.tsx
│   ├── workspace/                       # 已有，共享
│   │   ├── DisconnectBanner.tsx         # 复用
│   │   ├── ProviderConfigPanel.tsx      # 复用
│   │   └── WorkspaceHeader.tsx          # 复用（扩展 branch/worktree 显示）
│   └── lifecycle/
│       └── LifecycleCardDrawer.tsx      # 修改：新增 coding 入口按钮
├── state/
│   ├── workspace-ws-store.ts            # 已有，doc workspace 专用
│   ├── coding-workspace-store.ts        # 新增
│   └── chat-entries.ts                  # 已有，复用
├── hooks/
│   ├── useWorkspaceWs.ts                # 已有，doc workspace 专用
│   ├── useCodingWorkspaceWs.ts          # 新增
│   └── useStageUI.ts                    # 已有
└── router.tsx                           # 修改：新增 coding 路由
```

---

## 九、状态管理

### 9.1 coding-workspace-store

新建 `coding-workspace-store.ts`，使用 zustand，包含：

**Attempt 基础信息**：
- `attemptId`、`workItemId`、`issueId`、`projectId`
- `status`（created / running / waiting_for_human / blocked / completed / failed / aborted）
- `stage`（当前 coding stage）
- `branchName`、`baseBranch`、`worktreePath`
- `reworkCount`、`maxAutoRework`
- `headCommit`、`pushedRemote`
- `providerConfigSnapshot`

**Timeline**：
- `timelineNodes`：CodingTimelineNode[]
- `activeNodeId`
- `selectedNodeId`

**Chat**：
- `chatEntries`：ChatEntry[]（复用类型）
- `streamingContent`
- `activeStreamEntryId`

**Artifacts**：
- `activeTab`：当前选中 tab（diff / tests / review / git / logs）
- `diffSummary`：文件变更列表
- `testingReport`：TestingReport
- `reviewFindings`：ReviewFindings[]
- `reviewRequest`：ReviewRequest
- `internalPrReview`：InternalPrReview
- `logs`：LogEntry[]

**连接与 Gate**：
- `connectionStatus`
- `pendingPermissions`
- `protocolError`

### 9.2 useCodingWorkspaceWs

新建 hook，参考 `useWorkspaceWs` 结构：
- 建立 WebSocket 连接到 coding attempt session
- 处理 `coding_*` 前缀的服务端消息
- 暴露客户端动作：`startCoding`、`sendContextNote`、`respondPermission`、`continueRework`、`abortAttempt`、`finalConfirm`、`requestManualPause`

---

## 十、共享边界

| 组件/模块 | 共享方式 |
|-----------|---------|
| `ChatEntryList` | 直接复用 |
| `ChatInputBar` | 直接复用，通过 stage prop 控制按钮 |
| `ChatEntryRenderer` | 复用，新增 coding entry 类型渲染分支 |
| `MonacoViewer` / `MonacoDiffViewer` | 直接复用 |
| `DisconnectBanner` | 直接复用 |
| `ProviderConfigPanel` | 直接复用 |
| `WorkspaceHeader` | 复用，扩展 props 支持 branch/worktree |
| `ChatEntry` 类型 | 复用，扩展新 entry type |
| WebSocket 连接逻辑 | 新建 `useCodingWorkspaceWs`，独立处理 coding 消息 |

**完全独立**：
- `CodingTimeline`
- `CodingArtifactTabs` 及 5 个 tab 组件
- `coding-workspace-store`
- `useCodingWorkspaceWs`

---

## 十一、与后端技术方案的对应

本前端设计对应后端技术方案（`2026-05-23_技术方案_CodingWorkspace完整闭环_v1.0.md`）的以下部分：

- 第七节"前端交互设计"：本文档是其完整前端实现规格
- 第八节"WebSocket 消息"：`useCodingWorkspaceWs` 处理所有 `coding_*` 消息
- 第五节"数据模型"：`coding-workspace-store` 的状态字段与后端模型一一对应
- 第六节"状态机"：ChatInputBar 的 stage 按钮映射反映状态机的人工节点

---

## 十二、不在本文档范围

- 后端 Coding Workspace Engine 实现（见技术方案）
- Git/Worktree 操作逻辑（见技术方案第九节）
- Provider 输入策略（见技术方案第十节）
- 测试命令发现（见技术方案第十节）
- 安全边界（见技术方案第十一节）
