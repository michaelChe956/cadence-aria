# WorkItem 拆分 P8 前端 Coding Prepare 执行计划展示 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Coding Workspace Prepare 阶段展示 `WorkItemExecutionPlan`；默认非阻塞，开启确认门禁时要求用户确认或请求修改。

**Architecture:** 后端 P6 已在 coding attempt snapshot 和 WS state 中提供 execution plan。本计划只改 Coding Workspace 前端：API types、store、WS hydration 和 `CodingWorkspacePage` 展示/操作，不改 Product Workbench。

**Tech Stack:** React 19、TypeScript、Zustand、Vitest、Testing Library。

---

## 前置交付摘要

执行本计划前确认：

- P6 `CodingAttemptSnapshotResponse` 已包含 `work_item_execution_plan` 和 `work_item_handoff`。
- P6 已增加 confirm/change-request HTTP API，前端需要在 `web/src/api/client.ts` 增加调用。
- 后端表达门禁的规则是：`require_execution_plan_confirm=false` 时 draft 不阻塞；为 true 时必须确认后才能进入 Coder。

## 计划大小边界

本计划不做：

- 不改后端。
- 不改 Product Workbench Work Item 列。
- 不写 Playwright E2E。
- 不重构 Coding Workspace 整体布局。

如果后端字段缺失，停止并回到 P6 补契约。

## 文件结构

- Modify: `web/src/api/types.ts`
  - 新增 `WorkItemExecutionPlan`、`WorkItemHandoff` 类型。
  - 扩展 `CodingAttemptSnapshotResponse` 和 WS state type。
- Modify: `web/src/api/types.test.ts`
- Modify: `web/src/api/client.ts`
  - 新增 confirm/change-request API。
- Modify: `web/src/state/coding-workspace-store.ts`
  - 存储 execution plan/handoff。
- Modify: `web/src/state/coding-workspace-store.test.ts`
- Modify: `web/src/hooks/useCodingWorkspaceWs.ts`
  - snapshot/WS 消息写入 store。
- Modify: `web/src/hooks/useCodingWorkspaceWs.test.tsx`
- Modify: `web/src/pages/CodingWorkspacePage.tsx`
  - Prepare 阶段展示执行计划面板。
- Modify: `web/src/pages/CodingWorkspacePage.test.tsx`

## 任务 1：Add Execution Plan Types And Store State

**文件：**

- Modify: `web/src/api/types.ts`
- Modify: `web/src/api/types.test.ts`
- Modify: `web/src/state/coding-workspace-store.ts`
- Modify: `web/src/state/coding-workspace-store.test.ts`

- [ ] **步骤 1：编写失败态 type/store tests**

Append to `web/src/api/types.test.ts`:

```ts
it("describes work item execution plan and handoff in coding snapshots", () => {
  const plan: WorkItemExecutionPlan = {
    id: "work_item_execution_plan_0001",
    project_id: "project_0001",
    issue_id: "issue_0001",
    work_item_id: "work_item_0001",
    attempt_id: "coding_attempt_0001",
    status: "draft",
    goal: "实现后端 API",
    allowed_write_scopes: ["src/product/**"],
    forbidden_write_scopes: ["web/**"],
    dependency_handoffs: [],
    story_refs: ["story_spec_0001"],
    design_refs: ["design_spec_0001"],
    openspec_refs: ["REQ-001"],
    superpowers_contract: "use superpowers:test-driven-development",
    tdd_contract: "先写失败测试，再写实现",
    verification_commands: ["cargo test --locked --test it_product backend_api"],
    risk_notes: [],
    created_at: "2026-06-16T00:00:00Z",
    updated_at: "2026-06-16T00:00:00Z",
  };

  expect(plan.allowed_write_scopes).toEqual(["src/product/**"]);
});
```

Append to `web/src/state/coding-workspace-store.test.ts`:

```ts
it("stores work item execution plan from snapshot", () => {
  const store = useCodingWorkspaceStore.getState();
  store.reset();

  store.applySnapshot({
    ...codingSnapshot(),
    work_item_execution_plan: executionPlan(),
    work_item_handoff: null,
  });

  expect(useCodingWorkspaceStore.getState().workItemExecutionPlan?.goal).toBe("实现后端 API");
});
```

- [ ] **步骤 2：运行 tests 并确认失败**

运行:

```bash
pnpm -C web test -- --run types
pnpm -C web test -- --run coding-workspace-store
```

预期：类型或状态字段缺失导致失败。

- [ ] **步骤 3：添加 types and store fields**

在 `web/src/api/types.ts`, define `WorkItemExecutionPlan`, `WorkItemHandoff`, `WorkItemDependencyHandoffRef`.

在 store state add:

```ts
workItemExecutionPlan: WorkItemExecutionPlan | null;
workItemHandoff: WorkItemHandoff | null;
```

Update reset and snapshot application paths.

- [ ] **步骤 4：运行 tests 并确认通过**

运行步骤 2 的命令。

预期：通过。

## 任务 2：Hydrate Execution Plan From WS/Snapshot

**文件：**

- Modify: `web/src/hooks/useCodingWorkspaceWs.ts`
- Modify: `web/src/hooks/useCodingWorkspaceWs.test.tsx`

- [ ] **步骤 1：编写失败态 hydration test**

追加:

```tsx
it("hydrates work item execution plan from coding session state", async () => {
  const { emitMessage } = renderCodingWsHook();

  emitMessage({
    type: "coding_session_state",
    ...codingSessionState(),
    work_item_execution_plan: executionPlan(),
    work_item_handoff: null,
  });

  await waitFor(() => {
    expect(useCodingWorkspaceStore.getState().workItemExecutionPlan?.status).toBe("draft");
  });
});
```

- [ ] **步骤 2：运行 hook test 并确认失败**

运行:

```bash
pnpm -C web test -- --run useCodingWorkspaceWs
```

预期: WS handler ignores new fields.

- [ ] **步骤 3：更新 WS mapping**

When receiving initial snapshot or `coding_session_state`, set store execution plan and handoff fields.

P6 已将字段加入 HTTP snapshot 和 `coding_session_state`，本测试必须覆盖 WS state hydration。

- [ ] **步骤 4：运行 hook test 并确认通过**

Run command from Step 2.

预期：通过。

## 任务 3：Add Prepare Execution Plan Panel

**文件：**

- Modify: `web/src/pages/CodingWorkspacePage.tsx`
- Modify: `web/src/pages/CodingWorkspacePage.test.tsx`
- Modify: `web/src/api/client.ts`

- [ ] **步骤 1：编写失败态 render tests**

Append to `web/src/pages/CodingWorkspacePage.test.tsx`:

```tsx
it("shows work item execution plan during prepare stage as non blocking by default", () => {
  useCodingWorkspaceStore.setState({
    ...readyCodingState(),
    stage: "prepare_context",
    workItemExecutionPlan: executionPlan({ status: "draft" }),
  });

  render(<CodingWorkspacePage attemptId="coding_attempt_0001" onBack={vi.fn()} />);

  expect(screen.getByText("执行计划")).toBeInTheDocument();
  expect(screen.getByText("实现后端 API")).toBeInTheDocument();
  expect(screen.getByText("src/product/**")).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "确认执行计划" })).not.toBeInTheDocument();
});

it("shows confirm and change request actions when execution plan confirmation is required", () => {
  useCodingWorkspaceStore.setState({
    ...readyCodingState(),
    stage: "prepare_context",
    workItemExecutionPlan: executionPlan({
      status: "draft",
      require_confirmation: true,
    }),
  });

  render(<CodingWorkspacePage attemptId="coding_attempt_0001" onBack={vi.fn()} />);

  expect(screen.getByRole("button", { name: "确认执行计划" })).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "请求修改" })).toBeInTheDocument();
});
```

- [ ] **步骤 2：运行 page tests 并确认失败**

运行:

```bash
pnpm -C web test -- --run CodingWorkspacePage
```

预期：面板缺失导致失败。

- [ ] **步骤 3：添加 API client methods**

在 `web/src/api/client.ts`:

```ts
export function confirmWorkItemExecutionPlan(attemptId: string): Promise<WorkItemExecutionPlan>
export function requestWorkItemExecutionPlanChange(attemptId: string, payload: { note: string }): Promise<WorkItemExecutionPlan>
```

使用 the exact P6 routes.

- [ ] **步骤 4：实现 panel**

在 `CodingWorkspacePage.tsx`, render a compact Prepare panel when `workItemExecutionPlan` exists:

- Goal.
- Allowed/forbidden write scopes.
- Dependency handoffs.
- Verification commands.
- Risk notes.
- Confirm/change request buttons only when confirmation is required and status is not `confirmed`.

不要 render large explanatory text.

- [ ] **步骤 5：运行 render tests 并确认通过**

Run command from Step 2.

预期：通过。

## 任务 4：Confirm And Change Request Interactions

**文件：**

- Modify: `web/src/pages/CodingWorkspacePage.tsx`
- Modify: `web/src/pages/CodingWorkspacePage.test.tsx`

- [ ] **步骤 1：编写失败态 interaction tests**

追加:

```tsx
it("confirms execution plan and updates store", async () => {
  const user = userEvent.setup();
  const api = mockCodingWsApi();
  vi.mocked(confirmWorkItemExecutionPlan).mockResolvedValue(
    executionPlan({ status: "confirmed" }),
  );
  useCodingWorkspaceStore.setState({
    ...readyCodingState(),
    workItemExecutionPlan: executionPlan({ status: "draft", require_confirmation: true }),
  });

  render(<CodingWorkspacePage attemptId="coding_attempt_0001" onBack={vi.fn()} />);

  await user.click(screen.getByRole("button", { name: "确认执行计划" }));

  expect(confirmWorkItemExecutionPlan).toHaveBeenCalledWith("coding_attempt_0001");
  expect(useCodingWorkspaceStore.getState().workItemExecutionPlan?.status).toBe("confirmed");
});
```

- [ ] **步骤 2：运行 interaction tests 并确认失败**

运行:

```bash
pnpm -C web test -- --run CodingWorkspacePage
```

预期：操作入口缺失导致失败。

- [ ] **步骤 3：实现 actions**

On confirm:

- Call API.
- Update store with returned plan.
- Surface error in existing page error area if request fails.

On request change:

- Use a compact text input/dialog if existing page patterns support it.
- Call API with note.
- Update store with returned plan.

- [ ] **步骤 4：运行 interaction tests 并确认通过**

Run command from Step 2.

预期：通过。

## 最终验证

运行:

```bash
pnpm -C web test -- --run types
pnpm -C web test -- --run coding-workspace-store
pnpm -C web test -- --run useCodingWorkspaceWs
pnpm -C web test -- --run CodingWorkspacePage
pnpm -C web build
```

预期:

- Types, store, hook and page tests pass.
- Production build passes.

## 提交

```bash
git add web/src/api/types.ts web/src/api/types.test.ts web/src/api/client.ts web/src/state/coding-workspace-store.ts web/src/state/coding-workspace-store.test.ts web/src/hooks/useCodingWorkspaceWs.ts web/src/hooks/useCodingWorkspaceWs.test.tsx web/src/pages/CodingWorkspacePage.tsx web/src/pages/CodingWorkspacePage.test.tsx
git commit -m "feat: show work item execution plan in coding prepare"
```
