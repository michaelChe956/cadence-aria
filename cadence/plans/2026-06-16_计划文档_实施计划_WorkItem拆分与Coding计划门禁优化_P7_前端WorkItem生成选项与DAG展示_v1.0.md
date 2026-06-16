# WorkItem 拆分 P7 前端 Work Item 生成选项与 DAG 展示 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 前端在生命周期工作台提供 Work Item 生成选项，并在 Work Item 列/Drawer 展示 kind、依赖、写入范围、预算、等待原因、handoff 状态和 Integration/E2E 标识。

**Architecture:** 当前前端生命周期入口是 `web/src/components/lifecycle/IssueLifecycleWorkbench.tsx`，状态归组逻辑在 `web/src/state/lifecycle-workbench-store.ts`。本计划只改 Product lifecycle 前端，不改 Coding Workspace Prepare UI。

**Tech Stack:** React 19、TypeScript、Zustand、Vitest、Testing Library、lucide-react。

---

## 前置交付摘要

执行本计划前确认：

- P3 后端 `GenerateWorkItemsRequest` 已支持 `include_integration_tests`、`include_e2e_tests`、`force_frontend_backend_split`、`require_execution_plan_confirm`。
- `LifecycleWorkItemDto` 已透出 `kind`、`depends_on`、`exclusive_write_scopes`、`forbidden_write_scopes`、`context_budget`、`required_handoff_from`、`handoff_summary_ref`、`completion_commit`、`completion_diff_summary_ref`。
- P5/P6 后端会通过 `latest_attempt`、status 和 handoff 字段表达等待/完成状态。

## 计划大小边界

本计划不做：

- 不改后端。
- 不改 Coding Workspace Prepare UI。
- 不写 Playwright E2E。
- 不改路由结构。

如果发现 API 字段缺失，先停下并回到后端计划补字段，不在前端伪造状态。

## 文件结构

- Modify: `web/src/api/types.ts`
  - 增加 Work Item split 相关类型。
  - 扩展 `GenerateWorkItemsRequest`、`GenerateWorkItemsResponse`、`LifecycleWorkItem`。
- Modify: `web/src/api/types.test.ts`
  - 覆盖协议类型。
- Modify: `web/src/api/client.ts`
  - 保持 `generateWorkItems` 透传新 request payload。
- Modify: `web/src/state/lifecycle-workbench-store.ts`
  - 增加 DAG/等待原因 helper。
- Modify: `web/src/state/lifecycle-workbench-store.test.ts`
  - 覆盖分组、等待依赖、handoff 状态。
- Modify: `web/src/components/lifecycle/IssueLifecycleWorkbench.tsx`
  - 在生成 Work Item 前展示选项控件。
- Modify: `web/src/components/lifecycle/IssueLifecycleWorkbench.test.tsx`
  - 覆盖选项 payload 和 Work Item 列展示。
- Modify: `web/src/components/lifecycle/LifecycleCard.tsx`
  - Work Item 卡片展示 kind/等待状态。
- Modify: `web/src/components/lifecycle/LifecycleCardDrawer.tsx`
  - Drawer 展示范围、依赖、预算和 handoff。
- Modify: corresponding lifecycle component tests if assertions already live there.

## 任务 1：Extend Frontend API Types

**文件：**

- Modify: `web/src/api/types.ts`
- Modify: `web/src/api/types.test.ts`

- [ ] **步骤 1：编写失败态 type contract test**

Append to `web/src/api/types.test.ts`:

```ts
it("describes split work item lifecycle metadata", () => {
  const workItem = {
    work_item_id: "work_item_0001",
    issue_id: "issue_0001",
    repository_id: "repository_0001",
    story_spec_ids: ["story_spec_0001"],
    design_spec_ids: ["design_spec_0001"],
    title: "后端 API",
    plan_status: "confirmed",
    execution_status: "pending",
    kind: "backend",
    work_item_set_id: "work_item_set_0001",
    sequence_hint: 10,
    depends_on: [],
    exclusive_write_scopes: ["src/product/**"],
    forbidden_write_scopes: ["web/**"],
    context_budget: {
      target_context_k: "30-50",
      max_summary_chars: 20000,
      max_handoff_chars: 12000,
      max_code_context_chars: 30000,
      max_context_file_refs: 80,
      max_traceability_refs: 40,
      max_dependency_handoffs: 3,
    },
    required_handoff_from: [],
    require_execution_plan_confirm: false,
    execution_plan_status: "not_started",
    handoff_summary_ref: null,
    completion_commit: null,
    completion_diff_summary_ref: null,
    latest_attempt: null,
    artifact_versions: [],
  } satisfies LifecycleWorkItem;

  const request = {
    title: "登录会话拆分实现",
    story_spec_ids: ["story_spec_0001"],
    design_spec_ids: ["design_spec_0001"],
    include_integration_tests: true,
    include_e2e_tests: false,
    force_frontend_backend_split: true,
    require_execution_plan_confirm: false,
  } satisfies GenerateWorkItemsRequest;

  expect(workItem.kind).toBe("backend");
  expect(request.include_integration_tests).toBe(true);
});
```

- [ ] **步骤 2：运行 type test 并确认失败**

运行:

```bash
pnpm -C web test -- --run types
```

预期：TypeScript 编译失败，直到 the new fields exist.

- [ ] **步骤 3：添加 TypeScript types**

在 `web/src/api/types.ts`, add:

```ts
export type WorkItemKind = "backend" | "frontend" | "integration" | "e2e" | "docs" | "infra" | "other";
export type WorkItemExecutionPlanStatus = "not_started" | "draft" | "confirmed" | "change_requested";

export type WorkItemContextBudget = {
  target_context_k: string;
  max_summary_chars: number;
  max_handoff_chars: number;
  max_code_context_chars: number;
  max_context_file_refs: number;
  max_traceability_refs: number;
  max_dependency_handoffs: number;
};
```

Extend `LifecycleWorkItem` and `GenerateWorkItemsRequest/Response` to match backend.

- [ ] **步骤 4：运行 type test 并确认通过**

重新运行步骤 2 的命令。

预期：通过。

## 任务 2：Add Store Helpers For DAG And Waiting Reasons

**文件：**

- Modify: `web/src/state/lifecycle-workbench-store.ts`
- Modify: `web/src/state/lifecycle-workbench-store.test.ts`

- [ ] **步骤 1：编写失败态 store tests**

追加:

```ts
it("computes work item dependency waiting reasons", () => {
  const backend = lifecycleWorkItem({
    work_item_id: "work_item_0001",
    title: "后端 API",
    kind: "backend",
    execution_status: "pending",
    depends_on: [],
  });
  const frontend = lifecycleWorkItem({
    work_item_id: "work_item_0002",
    title: "前端 UI",
    kind: "frontend",
    execution_status: "pending",
    depends_on: ["work_item_0001"],
  });

  expect(workItemWaitingReason(frontend, [backend, frontend])).toBe(
    "等待依赖完成：后端 API",
  );
});

it("does not block work item when dependencies are completed and handoffs exist", () => {
  const backend = lifecycleWorkItem({
    work_item_id: "work_item_0001",
    title: "后端 API",
    kind: "backend",
    execution_status: "completed",
    handoff_summary_ref: "handoffs/work_item_0001.json",
  });
  const frontend = lifecycleWorkItem({
    work_item_id: "work_item_0002",
    title: "前端 UI",
    kind: "frontend",
    execution_status: "pending",
    depends_on: ["work_item_0001"],
    required_handoff_from: ["work_item_0001"],
  });

  expect(workItemWaitingReason(frontend, [backend, frontend])).toBeNull();
});
```

- [ ] **步骤 2：运行 store tests 并确认失败**

运行:

```bash
pnpm -C web test -- --run lifecycle-workbench-store
```

预期：helper 缺失导致失败。

- [ ] **步骤 3：实现 helpers**

Add:

```ts
export function workItemWaitingReason(
  item: LifecycleWorkItem,
  allItems: LifecycleWorkItem[],
): string | null
```

Rules:

- If any `depends_on` item is missing or not `completed`, return `等待依赖完成：{titles}`.
- If any `required_handoff_from` item lacks `handoff_summary_ref`, return `等待交接摘要：{titles}`.
- If `latest_attempt?.status` is active, return `正在编码`.
- Otherwise return `null`.

Also add `workItemKindLabel(kind: WorkItemKind): string`.

- [ ] **步骤 4：运行 store tests 并确认通过**

重新运行步骤 2 的命令。

预期：通过。

## 任务 3：Add Work Item Generation Options UI

**文件：**

- Modify: `web/src/components/lifecycle/IssueLifecycleWorkbench.tsx`
- Modify: `web/src/components/lifecycle/IssueLifecycleWorkbench.test.tsx`

- [ ] **步骤 1：编写失败态 UI payload test**

Append or update existing generate Work Item test:

```tsx
it("sends work item split options when generating from a confirmed design", async () => {
  const user = userEvent.setup();
  const fetchMock = lifecycleFetch({ confirmedDesign: true });
  vi.stubGlobal("fetch", fetchMock);

  render(<IssueLifecycleWorkbench />);

  await user.click(await screen.findByText("会话过期设计"));
  await user.click(screen.getByRole("button", { name: "生成 Work Item" }));
  await user.click(screen.getByLabelText("生成贯通测试 Work Item"));
  await user.click(screen.getByLabelText("生成 E2E Work Item"));
  await user.click(screen.getByRole("button", { name: "确认生成" }));

  const generateCall = fetchMock.mock.calls.find(([url]) =>
    String(url).includes("/work-items:generate"),
  );
  expect(JSON.parse(generateCall?.[1]?.body as string)).toMatchObject({
    include_integration_tests: true,
    include_e2e_tests: true,
    force_frontend_backend_split: true,
    require_execution_plan_confirm: false,
  });
});
```

- [ ] **步骤 2：运行 UI test 并确认失败**

运行:

```bash
pnpm -C web test -- --run IssueLifecycleWorkbench
```

预期：尚无选项 UI 或 payload，测试失败。

- [ ] **步骤 3：实现 options UI**

在 `IssueLifecycleWorkbench.tsx`:

- Add local state for a small modal/panel when generating Work Item from design.
- Use checkboxes/toggles:
  - `force_frontend_backend_split`: default `true`
  - `include_integration_tests`: default `true`
  - `include_e2e_tests`: default `false`
  - `require_execution_plan_confirm`: default `false`
- Confirm button calls `generateWorkItems()` with selected options.
- Cancel closes panel without API call.

Keep UI compact; do not introduce a landing-style or explanatory block.

- [ ] **步骤 4：运行 UI payload test 并确认通过**

重新运行步骤 2 的命令。

预期：通过。

## 任务 4：Render DAG Metadata On Cards And Drawer

**文件：**

- Modify: `web/src/components/lifecycle/LifecycleCard.tsx`
- Modify: `web/src/components/lifecycle/LifecycleCardDrawer.tsx`
- Modify: `web/src/components/lifecycle/LifecycleCard.test.tsx`
- Modify: `web/src/components/lifecycle/LifecycleCardDrawer.test.tsx`
- Modify: `web/src/components/lifecycle/IssueLifecycleWorkbench.test.tsx`

- [ ] **步骤 1：编写失败态 render tests**

添加 assertions:

```tsx
it("renders work item kind and waiting reason on work item cards", () => {
  render(
    <LifecycleCard
      card={workItemCard({
        kind: "frontend",
        depends_on: ["work_item_0001"],
      })}
      selected={false}
      deleting={false}
      onSelect={vi.fn()}
      onOpenFullIssue={vi.fn()}
    />,
  );

  expect(screen.getByText("前端")).toBeInTheDocument();
  expect(screen.getByText(/等待依赖/)).toBeInTheDocument();
});
```

For drawer:

```tsx
it("renders work item scopes budget and handoff state in drawer", () => {
  render(
    <LifecycleCardDrawer
      entity={workItemDrawerEntity({
        kind: "backend",
        exclusive_write_scopes: ["src/product/**"],
        forbidden_write_scopes: ["web/**"],
        handoff_summary_ref: "handoffs/work_item_0001.json",
      })}
      open
      onClose={vi.fn()}
    />,
  );

  expect(screen.getByText("src/product/**")).toBeInTheDocument();
  expect(screen.getByText("web/**")).toBeInTheDocument();
  expect(screen.getByText("交接摘要已生成")).toBeInTheDocument();
});
```

- [ ] **步骤 2：运行 render tests 并确认失败**

运行:

```bash
pnpm -C web test -- --run LifecycleCard
pnpm -C web test -- --run LifecycleCardDrawer
```

预期：元数据尚未渲染，测试失败。

- [ ] **步骤 3：实现 rendering**

Card:

- Show small kind label.
- Show waiting reason from store helper when present.
- Show `可编码` only when no waiting reason and plan confirmed.

Drawer:

- Show dependencies by ID/title.
- Show allowed and forbidden scopes.
- Show budget proxy compact summary.
- Show `交接摘要已生成` or `等待交接摘要`.
- Show `需要确认执行计划` if `require_execution_plan_confirm=true`.

- [ ] **步骤 4：运行 render tests 并确认通过**

运行步骤 2 的命令。

预期：通过。

## 最终验证

运行:

```bash
pnpm -C web test -- --run types
pnpm -C web test -- --run lifecycle-workbench-store
pnpm -C web test -- --run IssueLifecycleWorkbench
pnpm -C web test -- --run LifecycleCard
pnpm -C web test -- --run LifecycleCardDrawer
pnpm -C web build
```

预期:

- Type and component tests pass.
- Production build passes.

## 提交

```bash
git add web/src/api/types.ts web/src/api/types.test.ts web/src/api/client.ts web/src/state/lifecycle-workbench-store.ts web/src/state/lifecycle-workbench-store.test.ts web/src/components/lifecycle/IssueLifecycleWorkbench.tsx web/src/components/lifecycle/IssueLifecycleWorkbench.test.tsx web/src/components/lifecycle/LifecycleCard.tsx web/src/components/lifecycle/LifecycleCard.test.tsx web/src/components/lifecycle/LifecycleCardDrawer.tsx web/src/components/lifecycle/LifecycleCardDrawer.test.tsx
git commit -m "feat: show split work items in lifecycle workbench"
```
