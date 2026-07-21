# WorkItemPlan Options Dialog Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 Workbench 生成 Work Item Plan 前展示配置弹窗，让用户选择贯通测试、E2E、前后端拆分和子 Work Item 计划确认选项。

**Architecture:** `IssueLifecycleWorkbench` 负责记录待启动的 Design Spec、提交请求和打开 Workspace；新增 `WorkItemPlanOptionsDialog` 只负责展示 4 个 checkbox、提交和取消。两个现有 Work Item Plan 入口共用同一个弹窗和提交函数。

**Tech Stack:** React、TypeScript、Vitest、Testing Library、pnpm。

---

### Task 1: 前端回归测试

**Files:**
- Modify: `web/src/components/lifecycle/IssueLifecycleWorkbench.test.tsx`

- [ ] **Step 1: 写失败测试**

在 `IssueLifecycleWorkbench` describe 内，把现有 `sends default work item split options when preparing plan from a confirmed design` 调整为先打开弹窗再确认，并新增取消和自定义选项测试。

关键测试片段：

```ts
it("opens work item plan options before preparing plan from a confirmed design", async () => {
  const user = userEvent.setup();
  const fetchMock = lifecycleFetch();
  vi.stubGlobal("fetch", fetchMock);

  render(<IssueLifecycleWorkbench />);

  await user.click(await screen.findByText("前端提示设计"));
  await user.click(screen.getByRole("button", { name: "生成 Work Item" }));

  expect(
    await screen.findByRole("dialog", { name: "Work Item Plan 配置" }),
  ).toBeInTheDocument();
  expect(
    fetchMock.mock.calls.some(([url]) =>
      String(url).includes("/work-item-plans:prepare"),
    ),
  ).toBe(false);
});

it("sends selected work item split options after confirming the dialog", async () => {
  const user = userEvent.setup();
  const fetchMock = lifecycleFetch();
  vi.stubGlobal("fetch", fetchMock);
  const onOpenWorkspace = vi.fn();

  render(<IssueLifecycleWorkbench onOpenWorkspace={onOpenWorkspace} />);

  await user.click(await screen.findByText("前端提示设计"));
  await user.click(screen.getByRole("button", { name: "生成 Work Item" }));
  const dialog = await screen.findByRole("dialog", {
    name: "Work Item Plan 配置",
  });

  await user.click(within(dialog).getByLabelText("包含 E2E 测试 Work Item"));
  await user.click(within(dialog).getByLabelText("子 Work Item 执行前需要确认 Plan"));
  await user.click(within(dialog).getByRole("button", { name: "创建并打开 Workspace" }));

  await waitFor(() =>
    expect(onOpenWorkspace).toHaveBeenCalledWith("workspace_session_plan_group_0001"),
  );
  const prepareCall = fetchMock.mock.calls.find(([url]) =>
    String(url).includes("/work-item-plans:prepare"),
  );
  const body = JSON.parse(prepareCall?.[1]?.body as string);
  expect(body).toMatchObject({
    include_integration_tests: true,
    include_e2e_tests: true,
    force_frontend_backend_split: true,
    require_execution_plan_confirm: true,
  });
});

it("does not prepare a work item plan when options dialog is cancelled", async () => {
  const user = userEvent.setup();
  const fetchMock = lifecycleFetch();
  vi.stubGlobal("fetch", fetchMock);

  render(<IssueLifecycleWorkbench />);

  await user.click(await screen.findByText("前端提示设计"));
  await user.click(screen.getByRole("button", { name: "生成 Work Item" }));
  const dialog = await screen.findByRole("dialog", {
    name: "Work Item Plan 配置",
  });

  await user.click(within(dialog).getByRole("button", { name: "取消" }));

  expect(
    screen.queryByRole("dialog", { name: "Work Item Plan 配置" }),
  ).not.toBeInTheDocument();
  expect(
    fetchMock.mock.calls.some(([url]) =>
      String(url).includes("/work-item-plans:prepare"),
    ),
  ).toBe(false);
});
```

- [ ] **Step 2: 运行测试确认失败**

Run:

```bash
pnpm exec vitest --run src/components/lifecycle/IssueLifecycleWorkbench.test.tsx
```

Expected: FAIL，失败原因应是找不到 `Work Item Plan 配置` 弹窗。

### Task 2: 实现配置弹窗

**Files:**
- Create: `web/src/components/lifecycle/WorkItemPlanOptionsDialog.tsx`
- Modify: `web/src/components/lifecycle/IssueLifecycleWorkbench.tsx`

- [ ] **Step 1: 新增弹窗组件**

创建 `WorkItemPlanOptionsDialog.tsx`，导出类型和组件：

```ts
export type WorkItemPlanOptionsFormValue = {
  include_integration_tests: boolean;
  include_e2e_tests: boolean;
  force_frontend_backend_split: boolean;
  require_execution_plan_confirm: boolean;
};
```

组件行为：

- 初始状态来自 `defaultOptions`
- `form` 使用 `role="dialog"`、`aria-label="Work Item Plan 配置"`、`aria-modal="true"`
- 四个 checkbox 的 label 与测试一致
- submit 按钮文案 `创建并打开 Workspace`
- 取消按钮调用 `onClose`
- 提交时禁用按钮并捕获错误，错误显示在 `role="alert"` 内

- [ ] **Step 2: 接入 IssueLifecycleWorkbench 状态**

在 `IssueLifecycleWorkbench.tsx` 中：

```ts
type PendingWorkItemPlanLaunch = {
  card: LifecycleCardData;
};

const DEFAULT_WORK_ITEM_PLAN_OPTIONS = {
  include_integration_tests: true,
  include_e2e_tests: false,
  force_frontend_backend_split: true,
  require_execution_plan_confirm: false,
} satisfies WorkItemPlanOptionsFormValue;
```

新增 state：

```ts
const [pendingWorkItemPlanLaunch, setPendingWorkItemPlanLaunch] =
  useState<PendingWorkItemPlanLaunch | null>(null);
```

把 `handleGenerateNext` 和 `handleLaunchWorkspace` 中 `design_spec -> work_item` 分支改成：

```ts
setError(null);
setPendingWorkItemPlanLaunch({ card });
return;
```

- [ ] **Step 3: 实现提交函数**

在 `IssueLifecycleWorkbench.tsx` 中新增：

```ts
async function handleConfirmWorkItemPlanOptions(
  options: WorkItemPlanOptionsFormValue,
) {
  if (!selectedProjectId || !pendingWorkItemPlanLaunch) {
    setError("缺少 Project 或 Design Spec");
    return;
  }
  const { card } = pendingWorkItemPlanLaunch;
  if (card.kind !== "design_spec") {
    setError("当前实体不能生成 Work Item Plan");
    return;
  }

  const response = await prepareWorkItemPlan(selectedProjectId, card.issueId, {
    title: defaultLaunchTitle({ target: "work_item", card }),
    story_spec_ids: card.raw.story_spec_ids,
    design_spec_ids: [card.id],
    ...options,
  });
  await refresh(selectedProjectId);
  setPendingWorkItemPlanLaunch(null);
  onOpenWorkspace(response.workspace_session.workspace_session_id);
}
```

在 JSX 末尾渲染：

```tsx
{pendingWorkItemPlanLaunch ? (
  <WorkItemPlanOptionsDialog
    defaultOptions={DEFAULT_WORK_ITEM_PLAN_OPTIONS}
    onConfirm={handleConfirmWorkItemPlanOptions}
    onClose={() => setPendingWorkItemPlanLaunch(null)}
  />
) : null}
```

- [ ] **Step 4: 运行定向测试确认通过**

Run:

```bash
pnpm exec vitest --run src/components/lifecycle/IssueLifecycleWorkbench.test.tsx
```

Expected: PASS。

### Task 3: 验证与提交

**Files:**
- Modified files from Task 1 and Task 2

- [ ] **Step 1: 运行前端验证**

Run:

```bash
pnpm exec vitest --run src/components/lifecycle/IssueLifecycleWorkbench.test.tsx
pnpm build
```

Expected: Vitest PASS；`pnpm build` exit 0。

- [ ] **Step 2: 检查 diff**

Run:

```bash
git diff -- web/src/components/lifecycle/IssueLifecycleWorkbench.tsx web/src/components/lifecycle/IssueLifecycleWorkbench.test.tsx web/src/components/lifecycle/WorkItemPlanOptionsDialog.tsx
```

Expected: 只包含 Work Item Plan 配置弹窗、入口接入和测试变更。

- [ ] **Step 3: 提交实现**

Run:

```bash
git add web/src/components/lifecycle/IssueLifecycleWorkbench.tsx web/src/components/lifecycle/IssueLifecycleWorkbench.test.tsx web/src/components/lifecycle/WorkItemPlanOptionsDialog.tsx
git commit -m "feat: add work item plan options dialog"
```

Expected: commit 成功。
