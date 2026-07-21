# Lifecycle 卡片复合身份定位修复 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修复不同 Issue 下 Story Spec、Design Spec、Work Item Group 使用相同局部 ID 时，右侧抽屉、Workspace 跳转和后续生成操作串到其他 Issue 的问题。

**Architecture:** 前端以 `kind:issueId:entityId` 作为生命周期卡片唯一身份，Zustand 抽屉状态、卡片选中状态和 `/workbench?focus=` 统一传递该复合键。后端 ID、`.aria` 数据和 Workspace Session 契约保持不变，查找时禁止退回单实体 ID 模糊匹配。

**Tech Stack:** TypeScript、React 19、Zustand、TanStack Router、Vitest、Testing Library、pnpm

## Global Constraints

- 必须在 `/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0715` 中执行。
- 必须先写失败测试并观察正确 RED，再修改生产代码。
- Story Spec、Design Spec、Work Item Group 必须同时覆盖。
- 不修改后端 ID 生成策略，不迁移 `.aria` 数据，不改变 Workspace Session API。
- 前端包管理器只使用 `pnpm`。
- 保留 worktree 中现有未跟踪问题笔记，不纳入本次提交。

---

### Task 1: 用纯函数测试锁定复合身份规则

**Files:**
- Create: `web/src/components/lifecycle/IssueLifecycleWorkbenchParts.identity.test.ts`
- Modify: `web/src/components/lifecycle/IssueLifecycleWorkbenchParts.tsx`

**Interfaces:**
- Consumes: `LifecycleCard`、`LifecycleColumns`
- Produces: `lifecycleEntityKey(kind, issueId, entityId)`、更新后的 `lifecycleCardKey(card)`、精确匹配的 `findCardInColumns(columns, entityKey)`

- [ ] **Step 1: 写 Story、Design、Work Item Group 共享局部 ID 的失败测试**

```typescript
import { describe, expect, it } from "vitest";
import type {
  LifecycleCard,
  LifecycleColumns,
} from "../../state/lifecycle-workbench-store";
import {
  findCardInColumns,
  lifecycleCardKey,
} from "./IssueLifecycleWorkbenchParts";

function testCard(
  kind: "story_spec" | "design_spec" | "work_item_group",
  issueId: string,
  id: string,
): LifecycleCard {
  return {
    kind,
    issueId,
    id,
    title: `${issueId} ${kind}`,
    status: "confirmed",
    version: kind === "work_item_group" ? null : 1,
    preview: null,
    sourceIds: [],
    artifactVersions: [],
    ...(kind === "work_item_group"
      ? { childWorkItemIds: [], raw: {} }
      : { raw: {} }),
  } as LifecycleCard;
}

describe.each([
  ["story_spec", "story_spec_0001", "story_spec"],
  ["design_spec", "design_spec_0001", "design_spec"],
  ["work_item_group", "issue_work_item_plan_0001", "work_item"],
] as const)("%s composite identity", (kind, id, column) => {
  it("selects the matching issue and rejects a bare entity id", () => {
    const issueOne = testCard(kind, "issue_0001", id);
    const issueTwo = testCard(kind, "issue_0002", id);
    const columns = {
      issue: [],
      story_spec: column === "story_spec" ? [issueOne, issueTwo] : [],
      design_spec: column === "design_spec" ? [issueOne, issueTwo] : [],
      work_item: column === "work_item" ? [issueOne, issueTwo] : [],
    } as LifecycleColumns;

    expect(lifecycleCardKey(issueTwo)).toBe(`${kind}:issue_0002:${id}`);
    expect(findCardInColumns(columns, lifecycleCardKey(issueTwo))).toBe(issueTwo);
    expect(findCardInColumns(columns, id)).toBeNull();
  });
});
```

- [ ] **Step 2: 运行测试并确认 RED**

```bash
cd web
pnpm test -- IssueLifecycleWorkbenchParts.identity
```

Expected: 三组测试因当前键缺少 `issueId`、查找仍接受单实体 ID 而失败。

- [ ] **Step 3: 实现最小复合键和精确查找**

```typescript
export function lifecycleEntityKey(
  kind: LifecycleCardData["kind"],
  issueId: string,
  entityId: string,
) {
  return `${kind}:${issueId}:${entityId}`;
}

export function lifecycleCardKey(card: LifecycleCardData) {
  return lifecycleEntityKey(card.kind, card.issueId, card.id);
}

export function findCardInColumns(
  columns: LifecycleColumns,
  entityKey: string | null,
): LifecycleCardData | null {
  if (!entityKey) return null;
  return (
    [...columns.issue, ...columns.story_spec, ...columns.design_spec, ...columns.work_item]
      .find((card) => lifecycleCardKey(card) === entityKey) ?? null
  );
}
```

- [ ] **Step 4: 运行测试并确认 GREEN**

```bash
cd web
pnpm test -- IssueLifecycleWorkbenchParts.identity
```

Expected: 3 tests passed，单实体 ID 查找返回 `null`。

---

### Task 2: 先用 UI 回归复现三种卡片串 Issue

**Files:**
- Modify: `web/src/components/lifecycle/IssueLifecycleWorkbench.test-data.ts`
- Modify: `web/src/components/lifecycle/IssueLifecycleWorkbench.test-utils.ts`
- Modify: `web/src/components/lifecycle/IssueLifecycleWorkbench.drawer.test.tsx`

**Interfaces:**
- Consumes: 现有 `lifecycleFetch` 与 `IssueLifecycleWorkbench`
- Produces: 两个 Issue 共享局部实体 ID 的测试现场和三类 UI 回归用例

- [ ] **Step 1: 扩展测试夹具生成两个共享局部 ID 的 Issue**

为 `lifecycleFetch` 增加 `sharedLifecycleIdsAcrossIssues?: boolean`。启用时项目接口返回 `Issue One` 与 `Issue Two`；两个 lifecycle 都使用：

```typescript
story_spec_id: "story_spec_0001"
design_spec_id: "design_spec_0001"
work_item_plan.id: "issue_work_item_plan_0001"
```

各自标题、子 Work Item 和 Session ID 必须区分 Issue：

```typescript
story.title = `${issueId} Story`;
design.title = `${issueId} Design`;
workItem.title = `${issueId} Child`;
session.workspace_session_id = `workspace_session_${issueId}_${workspaceType}`;
```

- [ ] **Step 2: 写三类表驱动 UI 回归测试**

```typescript
it.each([
  ["Story Spec", "issue_0002 Story", "issue_0002 Story", "workspace_session_issue_0002_story"],
  ["Design Spec", "issue_0002 Design", "issue_0002 Design", "workspace_session_issue_0002_design"],
  ["Work Item", "Work Item Group", "issue_0002 Child", "workspace_session_issue_0002_work_item_plan"],
] as const)(
  "opens issue2 %s drawer and workspace when local ids collide",
  async (regionName, cardName, drawerText, sessionId) => {
    vi.stubGlobal("fetch", lifecycleFetch({ sharedLifecycleIdsAcrossIssues: true }));
    const user = userEvent.setup();
    const onOpenWorkspace = vi.fn();
    render(<IssueLifecycleWorkbench onOpenWorkspace={onOpenWorkspace} />);

    await user.click(await screen.findByRole("button", { name: "Issue Two" }));
    const region = screen.getByRole("region", { name: `${regionName} 内容` });
    await user.click(within(region).getByRole("button", { name: cardName }));

    expect(screen.getByTestId("lifecycle-card-drawer")).toHaveTextContent(drawerText);
    await user.click(screen.getByTestId("drawer-open-workspace"));
    await waitFor(() => expect(onOpenWorkspace).toHaveBeenCalledWith(sessionId));
  },
);
```

- [ ] **Step 3: 运行测试并确认 RED**

```bash
cd web
pnpm test -- IssueLifecycleWorkbench.drawer
```

Expected: 当前卡片点击仍把单实体 ID 交给抽屉；至少 Story、Design、Work Item Group 用例无法展示并打开 issue2 的正确 Workspace。

---

### Task 3: 统一抽屉状态、URL 和所有调用入口

**Files:**
- Modify: `web/src/state/lifecycle-workbench-store.ts`
- Modify: `web/src/components/lifecycle/IssueLifecycleWorkbench.tsx`
- Modify: `web/src/components/lifecycle/LifecycleColumn.tsx`
- Modify: `web/src/app-shell.tsx`
- Modify: `web/src/router.tsx`
- Modify: `web/src/components/lifecycle/IssueLifecycleWorkbench.test-utils.ts`
- Modify: `web/src/components/lifecycle/IssueLifecycleWorkbench.test.tsx`
- Modify: `web/src/state/lifecycle-workbench-store.test.ts`

**Interfaces:**
- Consumes: Task 1 的 `lifecycleEntityKey`、`lifecycleCardKey` 和 Task 2 的失败 UI 回归
- Produces: `focusedEntityKey` 状态、`focusEntityKey` 属性、复合键格式 Router `focus`

- [ ] **Step 1: 修改 store 测试和受控 URL 测试，要求复合键命名**

```typescript
useLifecycleWorkbenchStore.getState().openDrawer(
  "design_spec:issue_0001:design_spec_0001",
);
expect(useLifecycleWorkbenchStore.getState().focusedEntityKey).toBe(
  "design_spec:issue_0001:design_spec_0001",
);
```

```tsx
<IssueLifecycleWorkbench
  focusEntityKey="story_spec:issue_0001:story_spec_0001"
  onDrawerFocusChange={onDrawerFocusChange}
/>
```

- [ ] **Step 2: 运行新增断言并确认 RED**

```bash
cd web
pnpm test -- lifecycle-workbench-store IssueLifecycleWorkbench.test
```

Expected: `focusedEntityKey`、`focusEntityKey` 尚不存在，测试失败。

- [ ] **Step 3: 修改 Zustand store**

```typescript
export interface LifecycleWorkbenchState {
  focusedEntityKey: string | null;
  isDrawerOpen: boolean;
}

export interface LifecycleWorkbenchActions {
  openDrawer: (entityKey: string) => void;
  closeDrawer: () => void;
}

openDrawer: (entityKey) => set({ focusedEntityKey: entityKey, isDrawerOpen: true }),
closeDrawer: () => set({ focusedEntityKey: null, isDrawerOpen: false }),
```

- [ ] **Step 4: 修改组件与 Router 属性命名**

`IssueLifecycleWorkbench`、`AppShell` 与 Router 使用：

```typescript
focusEntityKey?: string | null;
onDrawerFocusChange?: (entityKey: string | null) => void;
```

Router 保留单个 `focus` 查询参数，但值为复合键：

```tsx
<AppShell focusEntityKey={search.focus ?? null} onDrawerFocusChange={syncDrawerFocus} />
```

- [ ] **Step 5: 替换所有抽屉、选中、删除和生成后聚焦入口**

```typescript
const cardKey = lifecycleCardKey(card);
setSelectedCardKey(cardKey);
openDrawer(cardKey);
```

生成新 Design Spec 后：

```typescript
const nextKey = lifecycleEntityKey("design_spec", card.issueId, nextId);
setSelectedCardKey(nextKey);
openDrawer(nextKey);
```

生成 Story/Design 后选中状态、Issue 删除动画键、Drawer 删除后的关闭判断都必须使用 `lifecycleEntityKey` 或 `lifecycleCardKey`。`LifecycleColumn` 的选中比较改为包含 `issueId`。抽屉实例使用复合 React key：

```tsx
<LifecycleCardDrawer
  key={lifecycleCardKey(focusedEntity)}
  entity={toDrawerEntity(focusedEntity, allWorkItems, codingAttempts)}
  {...handlers}
/>
```

- [ ] **Step 6: 运行 Task 1-3 定向测试并确认 GREEN**

```bash
cd web
pnpm test -- IssueLifecycleWorkbenchParts.identity IssueLifecycleWorkbench lifecycle-workbench-store
```

Expected: 复合身份纯函数、Story/Design/Work Item Group 跨 Issue UI、现有抽屉与 store 测试全部通过。

---

### Task 4: 全量验证、提交与推送

**Files:**
- Verify: `web/src/**`
- Verify: `cadence/designs/2026-07-15_技术方案_Lifecycle卡片复合身份定位修复_v1.0.md`
- Verify: `cadence/plans/2026-07-15_计划文档_缺陷修复_Lifecycle卡片复合身份定位_v1.0.md`

**Interfaces:**
- Consumes: Tasks 1-3 的实现与测试
- Produces: 可推送的 `feat-b-0715` 原子修复提交

- [ ] **Step 1: 运行全部前端测试**

```bash
cd web
pnpm test
```

Expected: 全部 Vitest 测试通过，0 failures。

- [ ] **Step 2: 运行 TypeScript 检查和生产构建**

```bash
cd web
pnpm tsc -b
pnpm build
```

Expected: 两条命令退出码均为 0；允许现有 Vite 大 chunk 警告，不得出现编译错误。

- [ ] **Step 3: 检查改动范围**

```bash
git diff --check
git status --short
git diff --stat
```

Expected: 无 whitespace error；现有未跟踪容量问题笔记保持未跟踪且不纳入提交。

- [ ] **Step 4: 提交代码修复与计划**

```bash
git add web/src cadence/plans/2026-07-15_计划文档_缺陷修复_Lifecycle卡片复合身份定位_v1.0.md
git commit -m "fix: scope lifecycle drawer focus by issue"
```

- [ ] **Step 5: 推送分支**

```bash
git push origin feat-b-0715
```

Expected: 远端 `feat-b-0715` 包含设计提交与代码修复提交。
