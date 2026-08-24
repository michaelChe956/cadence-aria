# Workbench 流水线工作台 UX 实施 Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将 `/workbench` Issue 生命周期工作台改造为「紧凑队列（行内 4 阶段 mini-graph，可按下一步动作分组/过滤/折叠）+ 阶段标签工作区 + 可折叠双密度外壳」，并修复 Issue 归属高亮丢失的选择语义缺陷。

**Architecture:** 纯前端改造。新增派生层纯函数 `deriveIssueQueue` 与展示组件 `IssueQueueRow` / `IssueQueue` / `StageStepper` / `LogicalCodebaseSummaryBar`；重构 `IssueLifecycleDetail` 为「吸顶 Issue 头 + 阶段步进器 + 单阶段 tab」；`IssueLifecycleWorkbench` 外壳改 `100dvh` 双密度并统一选择语义。零 API 变更，分组/过滤/阶段判定全部基于已加载 `IssueLifecycleResponse[]` 在前端派生。

**Tech Stack:** React 18 + TypeScript + Vite + Vitest + Testing Library + Tailwind（`--aria-*` 令牌）+ Zustand（仅既有 drawer store）。

**Spec:** `openspec/changes/workbench-pipeline-workspace-ux/`（proposal.md / design.md / specs/lifecycle-workbench-pipeline-ux/spec.md / tasks.md —— 本 Plan 只能展开该契约，不得重定义范围与验收）

## Global Constraints

- 🔴 **禁止 `git add -A` / `git add .`**：worktree 存在另一需求的未提交改动（`cadence/reports/design-weak-model-campaign/run_campaign.mjs` 与 `gate-manifest-revised.json`），每次提交 MUST 显式 `git add <本任务文件>`。
- 🔴 **零 API 变更**：不得修改 `web/src/api/**`、`src/**`（Rust）、任何请求路径/参数/响应类型/WebSocket 消息。
- 🔴 不新增任何第三方依赖（不引入虚拟列表/表格库）。
- 保留既有区域名：`Issue 卡片列表`、`Issue 生命周期详情`、`Story Spec 内容`、`Design Spec 内容`、`Work Item 内容`；保留既有 testid：`lifecycle-card-{kind}`、`lifecycle-card-title`、`selected-issue-preview`、`lifecycle-card-drawer`、`work-item-repository-group-*`、`lc-selector-*`、`codebase-list-item`。新元素一律用新 testid。
- 测试命令：定向 `cd web && pnpm vitest run <file>`；全量 `cd web && pnpm test`；类型 `cd web && pnpm tsc -b`。
- Commit message 使用 Conventional Commits（如 `feat(workbench): ...` / `test(workbench): ...`）。
- 工作目录：`/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo`。

---

### Task 1: 队列派生层 `deriveIssueQueue`

**Files:**
- Create: `web/src/components/lifecycle/issue-queue-derivation.ts`
- Test: `web/src/components/lifecycle/issue-queue-derivation.test.ts`

**Interfaces:**
- Consumes: `IssueLifecycleResponse`（`web/src/api/types`）、`LifecycleWorkItem`、`workItemWaitingReason`（`web/src/state/lifecycle-workbench-store.ts`）。
- Produces（后续 Task 依赖的精确签名）:

```ts
export type IssueStageKey = "story" | "design" | "work_item" | "coding";
export type StagePipState = "done" | "active" | "blocked" | "pending";
export type IssueQueueGroupKey =
  | "needs_story" | "needs_design" | "needs_work_item"
  | "blocked" | "coding" | "completed";
export interface StagePip { stage: IssueStageKey; state: StagePipState; }
export interface IssueQueueRowData {
  issueId: string; title: string; status: string;
  stagePips: StagePip[]; group: IssueQueueGroupKey;
  storyCount: number; designCount: number; workItemCount: number;
}
export interface IssueQueueGroup { key: IssueQueueGroupKey; rows: IssueQueueRowData[]; total: number; }
export const ISSUE_QUEUE_GROUP_ORDER: IssueQueueGroupKey[];
export function defaultCollapsedGroups(): IssueQueueGroupKey[]; // ["completed"]
export function deriveIssueQueue(
  lifecycles: IssueLifecycleResponse[],
  options?: { filterText?: string; perGroupLimit?: number },
): IssueQueueGroup[];
```

分组判定规则（按优先级顺序，命中即停；实现与测试必须逐字一致）:
1. `story_specs.length === 0` → `needs_story`
2. `design_specs.length === 0` → `needs_design`
3. `work_items.length === 0` → `needs_work_item`
4. 任一 work item 使 `workItemWaitingReason(item, allItems)` 非 null → `blocked`
5. 所有 work item 的 `latest_attempt` 均存在且 `status` 属于终态成功集 `{"completed"}` → `completed`（终态枚举以实现时 `CodingAttempt["status"]` 实际值为准，测试 fixture 冻结）
6. 其余 → `coding`

stagePips 规则：`story`/`design`/`work_item` pip：对应产物数量 > 0 → `done`；等于 0 且该阶段为分组判定的下一个动作（needs_story→story、needs_design→design、needs_work_item→work_item）→ `active`；`blocked` 分组时 `work_item` pip 为 `blocked`；其余 `pending`。`coding` pip：`completed` 分组 → `done`；`coding` 分组 → `active`；`blocked` 分组 → `blocked`；其余 `pending`。

过滤：`filterText` 非空时，大小写不敏感匹配 `title` 或 `issueId`，过滤在分组之前应用。`perGroupLimit` 默认 50：`rows` 截断到上限，`total` 保留组内真实总数。

- [ ] **Step 1: 写失败测试** `issue-queue-derivation.test.ts`：表驱动覆盖 6 个分组边界各一例（含优先级冲突：无 design 且 work item 等待依赖 → 仍 `needs_design`）、filterText 匹配 title 与 issueId、大小写不敏感、perGroupLimit 截断且 total 为真实值、`defaultCollapsedGroups()` 返回 `["completed"]`、stagePips 的 done/active/blocked/pending 各一例。Fixture 复用 `IssueLifecycleWorkbench.test-utils.ts` 中 lifecycle 响应构造方式（读该文件后仿写，不 import 其 mock fetch）。
- [ ] **Step 2: 运行确认失败** `cd web && pnpm vitest run src/components/lifecycle/issue-queue-derivation.test.ts` → 预期模块不存在报错。
- [ ] **Step 3: 实现 `issue-queue-derivation.ts`**（纯函数，无 React 依赖，无副作用）。
- [ ] **Step 4: 运行确认通过**，同 Step 2 命令。
- [ ] **Step 5: 提交** `git add web/src/components/lifecycle/issue-queue-derivation.ts web/src/components/lifecycle/issue-queue-derivation.test.ts && git commit -m "feat(workbench): add issue queue derivation with next-action grouping"`

---

### Task 2: 统一选择语义（归属高亮修复）

**Files:**
- Modify: `web/src/components/lifecycle/IssueLifecycleWorkbench.tsx`（`handleSelectCard`、IssueCardList 调用处的 props）
- Modify: `web/src/components/lifecycle/IssueLifecycleWorkbenchParts.tsx`（`IssueCardList` 增加 `focusedIssueId` prop 并据此传 selected）
- Test: `web/src/components/lifecycle/IssueLifecycleWorkbench.test.tsx`

**Interfaces:**
- Consumes: 既有 `LifecycleCardData`、`lifecycleCardKey`。
- Produces: `IssueCardList` 新 prop `focusedIssueId: string | null`；Issue 卡片 `selected = card.issueId === focusedIssueId`（不再比较 `selectedCardKey`）。选中任何子实体后 `focusedIssueId` 同步为其 `issueId`。

- [ ] **Step 1: 写失败回归测试**（加入 `IssueLifecycleWorkbench.test.tsx`，仿既有 `lifecycleFetch()` 用法）：渲染 → 点击「登录会话过期」→ 点击其 Story Spec 卡片按钮「会话过期提示」→ 断言 `screen.getByRole("button", { name: "登录会话过期" })` 的 `aria-pressed` 仍为 `"true"`；再断言 Issue 卡片容器带 `aria-current="true"`（在 `LifecycleCard` 根 div 上加 `aria-current={selected ? "true" : undefined}`，属本 Task 一部分）。
- [ ] **Step 2: 运行确认失败** `cd web && pnpm vitest run src/components/lifecycle/IssueLifecycleWorkbench.test.tsx` → aria-pressed 为 false。
- [ ] **Step 3: 实现**：`handleSelectCard` 非 issue 分支补 `setFocusedIssueId(card.issueId)`；`IssueCardList` 接收 `focusedIssueId` 并以 `card.issueId === focusedIssueId` 计算 selected；`LifecycleCard` 根元素补 `aria-current`。
- [ ] **Step 4: 运行确认通过**（同文件全量）。
- [ ] **Step 5: 提交** `git add web/src/components/lifecycle/IssueLifecycleWorkbench.tsx web/src/components/lifecycle/IssueLifecycleWorkbenchParts.tsx web/src/components/lifecycle/LifecycleCard.tsx web/src/components/lifecycle/IssueLifecycleWorkbench.test.tsx && git commit -m "fix(workbench): keep issue focus highlight driven by focusedIssueId"`

---

### Task 3: `IssueQueueRow` 与 mini-graph

**Files:**
- Create: `web/src/components/lifecycle/IssueQueueRow.tsx`
- Test: `web/src/components/lifecycle/IssueQueueRow.test.tsx`

**Interfaces:**
- Consumes: Task 1 的 `IssueQueueRowData`、`StagePip`。
- Produces:

```ts
export function StageMiniGraph({ pips }: { pips: StagePip[] }): JSX.Element;
export function IssueQueueRow(props: {
  row: IssueQueueRowData;
  focused: boolean;
  onSelect: () => void;
  onGenerateStorySpec?: () => void;
  onDelete?: () => void;
  deleting?: boolean;
}): JSX.Element;
```

视觉：单行 `h-11` 级、标题 `truncate`、pip 为 4 个 `h-2 w-2 rounded-full`（done=阶段色实心如 emerald-500、active=主色 `var(--aria-primary)`、blocked=amber-500、pending=`var(--aria-line)`），pip 间 `w-3` 连接线；`focused` 时 3px 左侧主色条 + `bg-[var(--aria-panel)]` + `aria-current="true"`；生成/删除按钮默认 `opacity-0`，`group-hover:opacity-100 group-focus-within:opacity-100`，行高不变；testid：`issue-queue-row`、`stage-mini-graph`、`stage-pip-{stage}`（带 `data-state`）。

- [ ] **Step 1: 写失败测试**：渲染行 → 断言 4 个 pip 及 `data-state`；`focused` 时 `aria-current="true"` 与左条 class；非 focused 时无 `aria-current`；操作按钮存在且带 `opacity-0`；点击标题触发 `onSelect`。
- [ ] **Step 2–4:** 失败 → 实现 → 通过（`pnpm vitest run src/components/lifecycle/IssueQueueRow.test.tsx`）。
- [ ] **Step 5: 提交** `git add web/src/components/lifecycle/IssueQueueRow.tsx web/src/components/lifecycle/IssueQueueRow.test.tsx && git commit -m "feat(workbench): add compact issue queue row with stage mini-graph"`

---

### Task 4: `IssueQueue`（吸顶过滤条 + 分组折叠 + 显示更多）

**Files:**
- Create: `web/src/components/lifecycle/IssueQueue.tsx`
- Test: `web/src/components/lifecycle/IssueQueue.test.tsx`

**Interfaces:**
- Consumes: Task 1 `deriveIssueQueue`/`IssueQueueGroup`/`ISSUE_QUEUE_GROUP_ORDER`、Task 3 `IssueQueueRow`。
- Produces:

```ts
export function IssueQueue(props: {
  groups: IssueQueueGroup[];
  focusedIssueId: string | null;
  collapsedGroups: IssueQueueGroupKey[];
  onToggleGroup: (key: IssueQueueGroupKey) => void;
  filterText: string;
  onFilterTextChange: (text: string) => void;
  onSelectIssue: (issueId: string) => void;
  onGenerateStorySpec: (issueId: string) => void;
  onDeleteIssue: (issueId: string) => void;
  deletingIssueId?: string | null;
}): JSX.Element;
```

结构：外层 `<section role="region" aria-label="Issue 卡片列表" className="flex min-h-0 flex-col ...">`；吸顶头部（标题 Issues + 总数 chip + `<input aria-label="过滤 Issues">`）；组列表 `overflow-y-auto`；组头按钮（名称 + `{rows.length}/{total}` 计数 + chevron，折叠态不渲染 rows）；组内 `rows.length < total` 时渲染「显示更多（+N）」按钮（本地 state 记录已追加的组）。组名中文映射：`needs_story` 待生成 Story、`needs_design` 待生成 Design、`needs_work_item` 待拆 Work Item、`blocked` 阻塞、`coding` 编码中、`completed` 已完成。

- [ ] **Step 1: 写失败测试**：六个组按 `ISSUE_QUEUE_GROUP_ORDER` 渲染；点组头折叠/展开；过滤输入触发 `onFilterTextChange`；`rows.length<total` 时「显示更多」按钮存在；区域名 `Issue 卡片列表` 存在。
- [ ] **Step 2–4:** 失败 → 实现 → 通过。
- [ ] **Step 5: 提交** `git add web/src/components/lifecycle/IssueQueue.tsx web/src/components/lifecycle/IssueQueue.test.tsx && git commit -m "feat(workbench): add grouped filterable issue queue"`

---

### Task 5: `StageStepper`

**Files:**
- Create: `web/src/components/lifecycle/StageStepper.tsx`
- Test: `web/src/components/lifecycle/StageStepper.test.tsx`

**Interfaces:**
- Produces:

```ts
export type WorkbenchStageKey = "story" | "design" | "work_item";
export function StageStepper(props: {
  stages: { key: WorkbenchStageKey; label: string; count: number; state: StagePipState }[];
  activeStage: WorkbenchStageKey;
  onSelect: (stage: WorkbenchStageKey) => void;
}): JSX.Element;
```

形态：`role="tablist"`；每段 `role="tab"` + `aria-selected` + 计数 chip + pip 色点；段间连接线；testid：`stage-stepper`、`stage-tab-{key}`。

- [ ] **Step 1: 写失败测试**：三段渲染与计数；`aria-selected` 仅 active；点击触发 `onSelect`。
- [ ] **Step 2–4:** 失败 → 实现 → 通过。
- [ ] **Step 5: 提交** `git add web/src/components/lifecycle/StageStepper.tsx web/src/components/lifecycle/StageStepper.test.tsx && git commit -m "feat(workbench): add lifecycle stage stepper"`

---

### Task 6: `IssueLifecycleDetail` 重构为阶段标签工作区

**Files:**
- Modify: `web/src/components/lifecycle/IssueLifecycleWorkbenchParts.tsx`（重构 `IssueLifecycleDetail`；`LifecycleContentSection`/`WorkItemRepositoryGroupSection` 保留并复用）
- Test: `web/src/components/lifecycle/IssueLifecycleWorkbenchParts.test.tsx`、`IssueLifecycleWorkbench.test.tsx`

**Interfaces:**
- Consumes: Task 5 `StageStepper`/`WorkbenchStageKey`；既有 `LifecycleContentSection`、`WorkItemRepositoryGroupSection`。
- Produces: `IssueLifecycleDetail` 新 props 增加 `onGenerateForStage: (stage: WorkbenchStageKey) => void`；内部 state `activeStage`，默认值规则：`storySpecs.length===0 → "story"`；否则 `designSpecs.length===0 → "design"`；否则 `"work_item"`。切换 Issue 时重置为默认阶段。

结构：`<section role="region" aria-label="Issue 生命周期详情" className="flex min-h-0 flex-col">` → 吸顶 Issue 头（保留 `selected-issue-preview`、`line-clamp-6` 与「查看完整 Issue」逻辑不变）→ `StageStepper` → 单阶段面板 `overflow-y-auto`：story → `LifecycleContentSection`(保留 `Story Spec 内容` 区域名)；design → 同理；work_item → 有分组时 `WorkItemRepositoryGroupSection` 全宽，否则 `LifecycleContentSection`。空阶段面板显示「暂无内容」+ 主按钮「生成 Story Spec / 生成 Design Spec / 准备 Work Item Plan」（调 `onGenerateForStage`）。未选 Issue 的空态文案保持不变。

- [ ] **Step 1: 写失败测试**：有 story 无 design 的 Issue → 默认 `stage-tab-design` 为 `aria-selected="true"`；点击 `stage-tab-story` 切换；空 design 面板显示「生成 Design Spec」按钮并触发回调；`selected-issue-preview` 与三个区域名仍在（当前阶段对应区域可见）。
- [ ] **Step 2–4:** 失败 → 实现 → 通过。
- [ ] **Step 5: 提交**（显式列文件）`git commit -m "feat(workbench): rebuild issue detail as stage-tabbed workspace"`

---

### Task 7: 外壳双密度 + 队列折叠 + 接线

**Files:**
- Modify: `web/src/components/lifecycle/IssueLifecycleWorkbench.tsx`
- Test: `web/src/components/lifecycle/IssueLifecycleWorkbench.test.tsx`

**Interfaces:**
- Consumes: Task 1/4/6 产物。
- Produces: 外壳 `h-[100dvh]`；队列区 `w-72 shrink-0`（折叠时 `w-10` 细轨，仅展开按钮 + 计数）；折叠 state：`queueCollapsed: Record<projectId, boolean>`，持久化 `localStorage["aria.workbench.queueCollapsed.<projectId>"]="1"|"0"`；分组折叠 state 同理 `aria.workbench.groups.<projectId>`（JSON 数组，缺省 `defaultCollapsedGroups()`）。`onGenerateForStage` 映射：story → `handleLaunchWorkspace("story", issueCard)`；design → 对最新 story 卡 `handleGenerateNext`；work_item → 对最新 design 卡 `handleGenerateNext`（复用现有 WorkItemPlanOptionsDialog 流程）。`onSelectIssue(issueId)` → 找到 issue 卡走 `handleSelectCard`。渲染：`IssueQueue` 替换 `IssueCardList`（`IssueCardList` 函数保留在 Parts 中但不再被页面使用，避免误删引发大面积测试改动）。

- [ ] **Step 1: 写失败测试**：点折叠按钮 → 队列细轨 testid `issue-queue-collapsed-rail` 出现且工作区仍在；再点展开恢复；`localStorage` 写入断言；切换 Project 后折叠状态互相独立（用既有双 project fixture）。
- [ ] **Step 2–4:** 失败 → 实现 → 通过。
- [ ] **Step 5: 提交** `git commit -m "feat(workbench): add collapsible queue shell with per-project density memory"`

---

### Task 8: `LogicalCodebaseSummaryBar` 运维摘要条

**Files:**
- Create: `web/src/components/lifecycle/LogicalCodebaseSummaryBar.tsx`
- Modify: `web/src/components/lifecycle/IssueLifecycleWorkbench.tsx`（用摘要条包裹现有 `LogicalCodebaseManagementPanel`，面板内部零改动）
- Test: `web/src/components/lifecycle/LogicalCodebaseSummaryBar.test.tsx`

**Interfaces:**

```ts
export function LogicalCodebaseSummaryBar(props: {
  summary: { lcName: string | null; indexState: string | null; publicationStatus: string | null; hasWarning: boolean };
  expanded: boolean;
  onToggle: () => void;
}): JSX.Element;
```

`IssueLifecycleWorkbench` 内：`lcSummaryExpanded: Record<projectId, boolean>`（`localStorage["aria.workbench.lcSummary.<projectId>"]`）；默认折叠渲染摘要条，展开渲染现有面板。`hasWarning` = `aggregateIndex?.state !== "active"` 或 `latestPointerPublication?.status` 含 `failed`/`partial`。摘要条 testid：`lc-summary-bar`；展开状态下面板既有 testid（`pointer-publication-panel` 等）必须仍可见——既有 lc-partition/codebases 测试若假设面板默认可见，同步改为「先点展开」的最小适配。

- [ ] **Step 1: 写失败测试**：默认摘要条可见且面板不可见；`hasWarning` 时警示 class；点「管理」展开后面板 testid 出现；localStorage 记忆。
- [ ] **Step 2–4:** 失败 → 实现 → 通过（含既有受影响测试的最小适配）。
- [ ] **Step 5: 提交** `git commit -m "feat(workbench): collapse logical codebase ops into summary bar"`

---

### Task 9: 轮询上下文冻结回归 + 全量验证

**Files:**
- Test: `web/src/components/lifecycle/IssueLifecycleWorkbench.test.tsx`

- [ ] **Step 1: 写回归测试**：渲染 → 选中 Issue → 点击其 Story 卡（drawer 打开）→ 用 `deferred` 触发一次 refresh 返回相同数据 → 断言 `focusedIssueId` 不变、drawer 仍开、队列折叠状态不变、过滤文本不变。
- [ ] **Step 2:** 失败则修复（预期现状基本满足，失败点即修复点）。
- [ ] **Step 3: 全量验证**：
  - `cd web && pnpm test`（全绿；输出留证）
  - `cd web && pnpm tsc -b`（零错误）
  - `openspec validate "workbench-pipeline-workspace-ux" --strict`
  - `git status --short` 确认另一需求两个文件仍为未提交状态且未被本 Plan 任何提交包含。
- [ ] **Step 4: 提交**（若有测试文件改动，显式列文件）`git commit -m "test(workbench): freeze polling context retention regression"`

---

## Self-Review 记录

- Spec 覆盖：REQ 紧凑队列→Task 3/4；滚动边界/渲染上限→Task 4/7；分组过滤→Task 1/4；选择语义→Task 2；阶段工作区→Task 5/6；折叠双密度→Task 7；轮询冻结→Task 9；运维降级→Task 8；视觉规范→各 Task 视觉要求 + Task 9 走查；契约兼容→Task 4/6 区域名保留 + Task 9 全量测试。
- 占位符扫描：无 TBD/TODO；各 Task 均含具体测试断言与命令。
- 类型一致性：`IssueQueueRowData`/`IssueQueueGroup`/`StagePip`/`WorkbenchStageKey`/`IssueQueue` props 在 Task 1/3/4/5/6/7 间逐一核对一致。
