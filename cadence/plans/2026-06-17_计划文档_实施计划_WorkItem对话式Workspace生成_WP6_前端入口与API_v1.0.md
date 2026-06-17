# WorkItem 对话式 Workspace 生成 WP6：前端入口 + API Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 新增 `prepareWorkItemPlan` API client 与 `PrepareWorkItemPlanRequest/Response` + `WorkItemPlanCandidateDto` 等类型；`WorkspaceSession.workspace_type` 支持 `"work_item_plan"`；`IssueLifecycleWorkbench` 入口从弹窗改为调 `prepareWorkItemPlan` 并打开 `ChatWorkspacePage`；移除 `setPendingWorkItemGenerate` 弹窗逻辑。本 WP 不实现 `WorkItemPlanCandidatePanel`（WP7）。

**Architecture:** WP1 后端已提供 `POST /work-item-plans:prepare` 契约，返回 `{ work_item_plan, workspace_session }`。前端 API client 仿照 `generateStorySpecs`/`generateDesignSpecs` 模式新增 `prepareWorkItemPlan`。`IssueLifecycleWorkbench` 的 `handleGenerateNext(design_spec)` 与 `handleLaunchWorkspace("work_item")` 改为调 prepare 并打开 `ChatWorkspacePage`（对齐 story/design 的 `handleLaunchWorkspace`）。`workspace_type` 联合类型加 `"work_item_plan"`。`WorkItemPlanCandidateDto` 等类型在 WP7 用，本 WP 先定义（与后端 WP1 的 DTO 对齐）。

**Tech Stack:** React、TypeScript、Zustand、Vitest、`pnpm`（🔴 禁止 npm/yarn）。

**版本：** v1.0
**创建日期：** 2026-06-17
**目标分支：** `feat-b-0616`
**工作区：** `.worktrees/feat-b-0616`
**依赖总览：** `cadence/plans/2026-06-17_计划文档_实施计划_WorkItem对话式Workspace生成_拆分总览_v1.0.md`（v1.1，WP6 章节）
**设计方案：** `cadence/designs/2026-06-17_技术方案_WorkItem对话式Workspace生成_v1.0.md`（第 412-431 行前端设计）
**前置 WP：** WP1（prepare API 契约）

---

## 全局约束（Global Constraints）

- **包管理器**：前端必须用 `pnpm`（🔴 禁止 npm/yarn）。
- **测试命令**：`pnpm -C web test -- --run <过滤名>`；构建 `pnpm -C web build`。
- **TDD**：每个 Task 先写失败测试，再写实现。
- **写入范围严格**：只改「File Structure」声明的文件。
- **行号是参考**：基于 `feat-b-0616` HEAD `8a2eee4`；实现时以 `grep -n`/IDE 实际为准。

---

## 前置交付摘要（来自 WP1）

- 后端 `POST /api/projects/{project_id}/issues/{issue_id}/work-item-plans:prepare` 已可用（WP1 Task 3）。
- 请求体 `PrepareWorkItemPlanRequest`：`title / story_spec_ids / design_spec_ids / include_integration_tests? / include_e2e_tests? / force_frontend_backend_split? / require_execution_plan_confirm? / author_provider? / reviewer_provider? / review_rounds? / superpowers_enabled? / openspec_enabled?`（4 个 split 选项 + provider 配置，复用现有 `provider_workspace_config` 解析）。
- 响应 `PrepareWorkItemPlanResponse`：`{ work_item_plan: IssueWorkItemPlan, workspace_session: WorkspaceSessionDto }`。
  - `work_item_plan.status = "draft"`，`work_item_ids`/`verification_plan_ids`/`dependency_graph` 为空。
  - `workspace_session.workspace_type = "work_item_plan"`，`entity_id = plan_id`。
- `WorkspaceType::WorkItemPlan` 的前端序列化值：`"work_item_plan"`。
- `WorkItemPlanCandidateDto` 后端结构（WP1 定义，WP7 前端用）：`{ plan: { id, status, options, dependency_graph }, work_items: [{ id, kind, title, depends_on, exclusive_write_scopes, verification_plan_ref, meta: { reverted, revert_feedback } }], verification_plans, repository_profile, validator_findings }`。

---

## 关键既有事实（避免重新探查）

实现时用 `grep -n`/IDE 确认。

### `web/src/api/client.ts`
- `generateStorySpecs` / `generateDesignSpecs`（prepare API client 的骨架模板）。`grep -n "generateStorySpecs\|generateDesignSpecs\|prepareWorkItem" web/src/api/client.ts`。
- `requestJson`/`apiFetch` 等 HTTP helper（现有）。

### `web/src/api/types.ts`
- `WorkspaceSession` 类型（含 `workspace_type` 字段，联合类型 `WorkspaceType`）。`grep -n "workspace_type\|WorkspaceType\b\|WorkspaceSession" web/src/api/types.ts`。
- `GenerateWorkItemsRequest`/`Response`（若前端有，参考字段；WP5 删后端 Response 但前端可能仍有定义——本 WP 不删前端 `GenerateWorkItems*` 类型，仅新增 prepare 类型）。
- `IssueWorkItemPlan`/`WorkspaceSessionDto` 前端类型（若有，复用；若无，新增）。
- `StorySpec`/`DesignSpec` 相关类型（参考 `source_story_spec_ids`/`source_design_spec_ids`）。

### `web/src/components/lifecycle/IssueLifecycleWorkbench.tsx`
- `handleGenerateNext(design_spec)`：当前调废弃的 `/work-items:generate` 弹窗流程。`grep -n "handleGenerateNext\|setPendingWorkItemGenerate\|work-items:generate\|handleLaunchWorkspace" web/src/components/lifecycle/IssueLifecycleWorkbench.tsx`。
- `handleLaunchWorkspace("story"/"design"/"work_item")`：story/design 走 prepare + ChatWorkspacePage；work_item 当前走旧弹窗。本 WP 把 work_item 对齐 story/design。
- `setPendingWorkItemGenerate` 弹窗状态（移除）。
- `ChatWorkspacePage` 路由跳转（参考 story/design 的跳转方式）。

### `web/src/pages/ChatWorkspacePage.tsx`
- 按 `workspace_type` 分支（WP7 加 `work_item_plan` 分支渲染 candidate 面板；本 WP 不改 ChatWorkspacePage，仅入口打开它）。

---

## File Structure

| 文件 | 操作 | 职责 / 本 WP 改动 |
|---|---|---|
| `web/src/api/types.ts` | M | 新增 `PrepareWorkItemPlanRequest`/`PrepareWorkItemPlanResponse`/`WorkItemPlanCandidateDto`+子 DTO；`WorkspaceType` 联合加 `"work_item_plan"` |
| `web/src/api/types.test.ts` | M | 新增类型的编译/类型测试（若有现成 types.test.ts） |
| `web/src/api/client.ts` | M | 新增 `prepareWorkItemPlan(projectId, issueId, request)` 函数 |
| `web/src/components/lifecycle/IssueLifecycleWorkbench.tsx` | M | `handleGenerateNext`/`handleLaunchWorkspace("work_item")` 改为调 `prepareWorkItemPlan` + 打开 `ChatWorkspacePage`；移除 `setPendingWorkItemGenerate` 弹窗逻辑 |
| `web/src/components/lifecycle/IssueLifecycleWorkbench.test.tsx` | M | 入口测试：Design Spec 入口打开 Workspace（不再弹窗） |

**不改：**
- ❌ `web/src/pages/ChatWorkspacePage.tsx`（WP7 加 `work_item_plan` 分支）
- ❌ `web/src/state/workspace-ws-store.ts` / `useWorkspaceWs.ts`（WP7）
- ❌ `web/src/components/workspace/WorkItemPlanCandidatePanel.tsx`（WP7 新增）
- ❌ 后端

---

## Task 1：API 类型 + client + `workspace_type` 联合扩展

**目标**：`web/src/api/types.ts` 新增 `PrepareWorkItemPlanRequest/Response` + `WorkItemPlanCandidateDto` 及子 DTO（与后端 WP1 对齐）；`WorkspaceType` 联合加 `"work_item_plan"`。`web/src/api/client.ts` 新增 `prepareWorkItemPlan`。

**Files:**
- Modify: `web/src/api/types.ts`
- Modify: `web/src/api/types.test.ts`（若存在；否则在 client 测试覆盖）
- Modify: `web/src/api/client.ts`

**Interfaces:**
- Consumes: WP1 的 `POST /work-item-plans:prepare` 契约。
- Produces: `prepareWorkItemPlan` API client（被 `IssueLifecycleWorkbench` 用）；`WorkItemPlanCandidateDto` 类型（WP7 用）。

- [ ] **Step 1.1：写失败测试 —— `prepareWorkItemPlan` 调用正确端点**

在 `web/src/api/client.ts` 的测试文件（`grep -rn "generateStorySpecs.*test\|client.test" web/src/api/` 定位）或 `web/src/api/types.test.ts`。参考 `generateStorySpecs` 的测试模式。

```typescript
  it("prepareWorkItemPlan posts to work-item-plans:prepare and returns response", async () => {
    const mockResponse = {
      work_item_plan: { id: "issue_work_item_plan_0001", status: "draft", work_item_ids: [], verification_plan_ids: [], dependency_graph: [] },
      workspace_session: { id: "session_0001", workspace_type: "work_item_plan", entity_id: "issue_work_item_plan_0001" },
    };
    fetchMock.mockOnce(JSON.stringify(mockResponse), { status: 200 });

    const result = await prepareWorkItemPlan("project_0001", "issue_0001", {
      title: "登录拆分",
      story_spec_ids: ["story_spec_0001"],
      design_spec_ids: ["design_spec_0001"],
      include_integration_tests: true,
      include_e2e_tests: false,
      force_frontend_backend_split: true,
      require_execution_plan_confirm: false,
      review_rounds: 1,
    });

    expect(fetch).toHaveBeenCalledWith(
      expect.stringContaining("/api/projects/project_0001/issues/issue_0001/work-item-plans:prepare"),
      expect.objectContaining({ method: "POST" }),
    );
    expect(result.work_item_plan.status).toBe("draft");
    expect(result.workspace_session.workspace_type).toBe("work_item_plan");
    expect(result.workspace_session.entity_id).toBe("issue_work_item_plan_0001");
  });
```

> 实现者注意：fetch mock 模式参考现有 `generateStorySpecs` 测试（`grep -rn "generateStorySpecs" web/src/api/` 找测试文件）。若用 msw/axios mock，照搬现有模式。

- [ ] **Step 1.2：运行测试，确认失败**

Run: `pnpm -C web test -- --run prepareWorkItemPlan`
Expected: 失败——`prepareWorkItemPlan` 未定义。

- [ ] **Step 1.3：新增类型定义**

`web/src/api/types.ts`，参考现有 `GenerateWorkItemsRequest`/`StorySpec` 风格。`grep -n "GenerateWorkItemsRequest\|interface.*Spec\|WorkspaceType" web/src/api/types.ts` 定位插入点。

```typescript
export type WorkspaceType = "story" | "design" | "work_item" | "work_item_plan";

export interface PrepareWorkItemPlanRequest {
  title: string;
  story_spec_ids?: string[];
  design_spec_ids?: string[];
  include_integration_tests?: boolean;
  include_e2e_tests?: boolean;
  force_frontend_backend_split?: boolean;
  require_execution_plan_confirm?: boolean;
  author_provider?: string;
  reviewer_provider?: string;
  review_rounds?: number;
  superpowers_enabled?: boolean;
  openspec_enabled?: boolean;
}

export interface PrepareWorkItemPlanResponse {
  work_item_plan: IssueWorkItemPlan;
  workspace_session: WorkspaceSessionDto;
}

// WorkItemPlanCandidateDto（WP7 用，本 WP 先定义）
export interface WorkItemPlanCandidateDto {
  plan: WorkItemPlanDto;
  work_items: WorkItemCandidateDto[];
  verification_plans: VerificationPlanDto[];
  repository_profile: RepositoryProfileDto | null;
  validator_findings: ValidatorFindingDto[];
}

export interface WorkItemPlanDto {
  id: string;
  status: string;
  options: WorkItemSplitOptionsDto;
  dependency_graph: WorkItemDependencyEdgeDto[];
}

export interface WorkItemSplitOptionsDto {
  include_integration_tests: boolean;
  include_e2e_tests: boolean;
  force_frontend_backend_split: boolean;
  require_execution_plan_confirm: boolean;
}

export interface WorkItemDependencyEdgeDto {
  from_work_item_id: string;
  to_work_item_id: string;
}

export interface WorkItemCandidateDto {
  id: string;
  kind: string;
  title: string;
  depends_on: string[];
  exclusive_write_scopes: string[];
  verification_plan_ref: string | null;
  meta: WorkItemCandidateMetaDto;
}

export interface WorkItemCandidateMetaDto {
  reverted: boolean;
  revert_feedback: string | null;
}

export interface ValidatorFindingDto {
  severity: string;
  code: string;
  message: string;
  work_item_ids?: string[];
}
```

> 实现者注意：
> 1. `WorkspaceType` 当前可能是字符串字面量联合或 enum——`grep -n "WorkspaceType\|workspace_type" web/src/api/types.ts` 确认现有定义，加 `"work_item_plan"`。
> 2. `IssueWorkItemPlan`/`WorkspaceSessionDto`/`VerificationPlanDto`/`RepositoryProfileDto`：先 `grep -n "IssueWorkItemPlan\|WorkspaceSessionDto\|VerificationPlanDto\|RepositoryProfileDto" web/src/api/types.ts` 确认是否已有。若有复用，若无新增（字段对齐后端 `src/product/models.rs` + `src/web/types.rs`）。
> 3. `WorkItemPlanCandidateDto` 及子 DTO 本 WP 定义，WP7 `workspace-ws-store`/`WorkItemPlanCandidatePanel` 使用。

- [ ] **Step 1.4：实现 `prepareWorkItemPlan` client**

`web/src/api/client.ts`，参考 `generateStorySpecs`（`grep -n "export async function generateStorySpecs\|generateDesignSpecs" web/src/api/client.ts`）。

```typescript
export async function prepareWorkItemPlan(
  projectId: string,
  issueId: string,
  request: PrepareWorkItemPlanRequest,
): Promise<PrepareWorkItemPlanResponse> {
  const response = await apiFetch(
    `/api/projects/${projectId}/issues/${issueId}/work-item-plans:prepare`,
    {
      method: "POST",
      body: JSON.stringify(request),
    },
  );
  return response.json();
}
```

> `apiFetch`/HTTP helper 以现有 `generateStorySpecs` 用的为准——照搬其 fetch 封装、错误处理、base URL 拼接。顶部 `import { PrepareWorkItemPlanRequest, PrepareWorkItemPlanResponse } from "./types";`。

- [ ] **Step 1.5：运行 Task 1 测试 + 类型检查**

Run:
```
pnpm -C web test -- --run prepareWorkItemPlan
pnpm -C web build
```
Expected: 测试 PASS；`pnpm -C web build`（tsc 类型检查）全绿——`WorkspaceType` 加 `"work_item_plan"` 后，所有 `workspace_type` 的 switch/分支需兼容新值（若有 exhaustive check 报错，本 WP 先加新值的 fallback 分支，WP7 细化）。

> 若 `pnpm -C web build` 报 `workspace_type` 的 exhaustive switch 错误（如 `ChatWorkspacePage`/`workspace-ws-store` 的 switch），**在本 Task 加最小 fallback 分支**（让编译通过），WP7 再细化 `work_item_plan` 分支。这是"加枚举值让类型通过"的必要最小改动。

- [ ] **Step 1.6：提交**

```bash
git add web/src/api/types.ts web/src/api/client.ts web/src/api/types.test.ts
git commit -m "feat(WP6): prepareWorkItemPlan API client + WorkItemPlanCandidateDto 类型"
```

---

## Task 2：`IssueLifecycleWorkbench` 入口改造（弹窗 → ChatWorkspacePage）

**目标**：`handleGenerateNext(design_spec)` 与 `handleLaunchWorkspace("work_item")` 改为调 `prepareWorkItemPlan` 并打开 `ChatWorkspacePage`；移除 `setPendingWorkItemGenerate` 弹窗逻辑（对齐 `handleLaunchWorkspace("story"/"design")`）。

**Files:**
- Modify: `web/src/components/lifecycle/IssueLifecycleWorkbench.tsx`
- Modify: `web/src/components/lifecycle/IssueLifecycleWorkbench.test.tsx`

**Interfaces:**
- Consumes: Task 1 的 `prepareWorkItemPlan`。
- Produces: Design Spec 点"生成下一阶段" → 打开 `ChatWorkspacePage`（`workspace_type=work_item_plan`）。

- [ ] **Step 2.1：写失败测试 —— Design Spec 入口打开 Workspace 不再弹窗**

在 `web/src/components/lifecycle/IssueLifecycleWorkbench.test.tsx`。参考现有 story/design 入口测试（`grep -n "handleLaunchWorkspace\|handleGenerateNext\|ChatWorkspacePage\|generateStorySpecs" web/src/components/lifecycle/IssueLifecycleWorkbench.test.tsx`）。

```typescript
  it("handleGenerateNext from design spec opens ChatWorkspacePage via prepareWorkItemPlan", async () => {
    const mockPrepare = vi.fn().mockResolvedValue({
      work_item_plan: { id: "issue_work_item_plan_0001", status: "draft", work_item_ids: [], verification_plan_ids: [], dependency_graph: [] },
      workspace_session: { id: "session_0001", workspace_type: "work_item_plan", entity_id: "issue_work_item_plan_0001" },
    });
    vi.mocked(prepareWorkItemPlan).mockImplementation(mockPrepare);

    render(<IssueLifecycleWorkbench {...defaultProps} />);
    // 触发 handleGenerateNext(design_spec)
    await act(async () => {
      // 点"生成下一阶段"按钮（以实际 UI 为准）
      fireEvent.click(screen.getByRole("button", { name: /生成下一阶段/ }));
    });

    await waitFor(() => {
      expect(prepareWorkItemPlan).toHaveBeenCalledWith("project_0001", "issue_0001", expect.objectContaining({
        design_spec_ids: ["design_spec_0001"],
      }));
    });
    // 不再弹窗（无 setPendingWorkItemGenerate 弹窗）
    expect(screen.queryByText(/生成选项/)).not.toBeInTheDocument();
    // 打开 ChatWorkspacePage（路由跳转或渲染 ChatWorkspacePage）
    expect(mockNavigate).toHaveBeenCalledWith(expect.stringContaining("session_0001"));
  });

  it("does not call legacy /work-items:generate endpoint", async () => {
    // 断言旧弹窗逻辑（setPendingWorkItemGenerate）已移除
    render(<IssueLifecycleWorkbench {...defaultProps} />);
    // 无弹窗相关 UI
    expect(screen.queryByTestId("work-item-generate-dialog")).not.toBeInTheDocument();
  });
```

> 实现者注意：
> 1. mock 与 render 模式参考现有 `IssueLifecycleWorkbench.test.tsx` 的 story/design 入口测试。
> 2. `mockNavigate` 参考现有路由跳转测试。
> 3. `defaultProps` 夹具参考现有测试。
> 4. 按钮文案/UI 选择器以实际为准——先读现有测试看 story/design 入口怎么触发。

- [ ] **Step 2.2：运行测试，确认失败**

Run: `pnpm -C web test -- --run IssueLifecycleWorkbench`
Expected: 失败——`prepareWorkItemPlan` 未被调用（仍走旧弹窗）或弹窗仍出现。

- [ ] **Step 2.3：改造 `handleGenerateNext` / `handleLaunchWorkspace("work_item")`**

`web/src/components/lifecycle/IssueLifecycleWorkbench.tsx`：

1. **移除弹窗逻辑**：`grep -n "setPendingWorkItemGenerate\|pendingWorkItemGenerate\|work-items:generate" web/src/components/lifecycle/IssueLifecycleWorkbench.tsx`，删除 `pendingWorkItemGenerate` state 与弹窗 JSX。
2. **`handleGenerateNext(design_spec)` 改为**：

```typescript
  const handleGenerateNext = async (designSpec: DesignSpec) => {
    try {
      const response = await prepareWorkItemPlan(projectId, issueId, {
        title: designSpec.title, // 或派生标题
        story_spec_ids: designSpec.story_spec_ids,
        design_spec_ids: [designSpec.id],
        review_rounds: 1, // 默认值，或从 workbench 配置取
      });
      // 打开 ChatWorkspacePage（参考 handleLaunchWorkspace("story"/"design") 的跳转方式）
      navigate(`/projects/${projectId}/issues/${issueId}/workspace/${response.workspace_session.id}`);
    } catch (err) {
      // 错误处理，参考现有 story/design 入口
    }
  };
```

3. **`handleLaunchWorkspace("work_item")` 对齐**：若该函数当前走旧弹窗，改为与 `"story"`/`"design"` 一致（调 prepare + navigate）。参考 `handleLaunchWorkspace("story")` 的实现（`grep -n "handleLaunchWorkspace" web/src/components/lifecycle/IssueLifecycleWorkbench.tsx`）。

> 实现者注意：
> 1. `prepareWorkItemPlan` 的参数（story_spec_ids/design_spec_ids/review_rounds 等）从 `designSpec` 与 workbench 配置派生——参考现有 `handleGenerateNext` 旧实现里怎么组装这些字段（`grep -n "include_integration_tests\|force_frontend_backend_split\|review_rounds" web/src/components/lifecycle/IssueLifecycleWorkbench.tsx`），照搬其默认值。
> 2. 路由路径以实际 `ChatWorkspacePage` 路由定义为准——`grep -rn "ChatWorkspacePage\|workspace/:sessionId\|/workspace/" web/src/` 确认路由。
> 3. 顶部 `import { prepareWorkItemPlan } from "../../api/client";` + 类型 import。
> 4. **保留 story/design 的 `handleLaunchWorkspace` 不变**——只改 work_item 分支。

- [ ] **Step 2.4：运行 Task 2 测试 + 构建**

Run:
```
pnpm -C web test -- --run IssueLifecycleWorkbench
pnpm -C web build
```
Expected: 新测试 PASS；现有 IssueLifecycleWorkbench 测试（story/design 入口）仍全绿；`pnpm -C web build` 全绿。

> 若现有测试有断言旧弹窗行为的（如 `expect(screen.getByTestId("work-item-generate-dialog"))`），删除/迁移这些测试（旧弹窗已移除）。

- [ ] **Step 2.5：提交**

```bash
git add web/src/components/lifecycle/IssueLifecycleWorkbench.tsx web/src/components/lifecycle/IssueLifecycleWorkbench.test.tsx
git commit -m "feat(WP6): IssueLifecycleWorkbench 入口从弹窗改为 prepareWorkItemPlan + ChatWorkspacePage"
```

---

## Task 3：WP6 收口验证

**目标**：跑完整前端验证，确保 WP6 改动未破坏既有 story/design/work_item 入口；prepare API 与类型正确。

**Files:** 无新增改动；仅运行验证命令。

- [ ] **Step 3.1：全量验证链**

Run:
```
pnpm -C web test -- --run types
pnpm -C web test -- --run IssueLifecycleWorkbench
pnpm -C web build
```
Expected: 全绿。

> `pnpm -C web build` 是 tsc 类型检查 + 构建，确保 `WorkspaceType` 加 `"work_item_plan"` 后无类型错误（WP7 会细化 `work_item_plan` 分支，本 WP 的 fallback 分支让编译通过）。

- [ ] **Step 3.2：确认废弃路由无前端残留调用**

Run: `grep -rn "work-items:generate\|work-item-plans.*confirm\|change-request" web/src/`
Expected: 无命中（或仅注释/类型定义，无实际 fetch 调用）。若有残留 `generateWorkItems` client 函数调用，移除（WP5 已删后端路由）。

> 若 `web/src/api/client.ts` 有 `generateWorkItems` 函数（调废弃 `/work-items:generate`），本 WP 删除该函数 + 相关测试（后端路由已删，函数无用）。`grep -n "generateWorkItems" web/src/api/client.ts`。

- [ ] **Step 3.3：交付摘要（供 WP7 前置交付摘要使用）**

commit 后，把以下内容写入 WP7 plan 的「前置交付摘要」章节：

- `prepareWorkItemPlan(projectId, issueId, request) -> PrepareWorkItemPlanResponse` API client 已就位（`web/src/api/client.ts`）。
- `PrepareWorkItemPlanRequest/Response` + `WorkItemPlanCandidateDto` + 子 DTO + `WorkspaceType` 含 `"work_item_plan"` 已定义（`web/src/api/types.ts`）。
- `IssueLifecycleWorkbench` 入口已改造：Design Spec 点"生成下一阶段" → `prepareWorkItemPlan` → 打开 `ChatWorkspacePage`（`workspace_type=work_item_plan`）；弹窗逻辑已移除。
- **WP7 待办**：`ChatWorkspacePage` 按 `workspace_type === "work_item_plan"` 分支渲染 `WorkItemPlanCandidatePanel`；`workspace-ws-store` 处理 artifact payload union（markdown 或 candidate）+ `sendRevertWorkItem` + `workItemPlanCandidate` 状态；复用 review/confirm/revision 收发。

---

## Self-Review（写完后的自查）

**1. Spec 覆盖**（对照总览 v1.1 WP6 目标/写入范围/验证 + 设计方案 :412-431）：
- ✅ `prepareWorkItemPlan` API client → Task 1 Step 1.4
- ✅ `PrepareWorkItemPlanRequest/Response` 类型 → Task 1 Step 1.3
- ✅ `WorkspaceSession.workspace_type` 支持 `"work_item_plan"` → Task 1 Step 1.3（`WorkspaceType` 联合）
- ✅ `IssueLifecycleWorkbench` 入口从弹窗改为 prepare + ChatWorkspacePage → Task 2
- ✅ 移除 `setPendingWorkItemGenerate` 弹窗逻辑 → Task 2 Step 2.3
- ✅ 验证命令链 → Task 3
- ✅ 不做项：未实现 `WorkItemPlanCandidatePanel`（WP7）、未改 `ChatWorkspacePage` 分支（WP7）、未改 `workspace-ws-store`（WP7）、未改后端——均在「不做」清单。
- ✅ `WorkItemPlanCandidateDto` 类型本 WP 先定义（WP7 用）→ Task 1 Step 1.3

**2. Placeholder 扫描**：
- `IssueWorkItemPlan`/`WorkspaceSessionDto`/`VerificationPlanDto`/`RepositoryProfileDto`（Task 1 Step 1.3）：给出 grep 确认是否已有、若无新增的指引。属可接受。
- `handleGenerateNext` 的参数派生（Task 2 Step 2.3）：给出"参考旧实现的默认值组装"指引。属可接受。
- 路由路径（Task 2 Step 2.3）：给出 grep 确认指引。属可接受。
- UI 选择器（Task 2 Step 2.1）：给出"以实际为准，参考现有 story/design 入口测试"指引。属可接受。

**3. 类型一致性**：
- `WorkspaceType` 加 `"work_item_plan"` 与后端 serde `"work_item_plan"` 一致。
- `WorkItemPlanCandidateDto` 字段与后端 WP1 的 `WorkItemPlanCandidateDto`（`workspace_ws_types.rs`）对齐：plan/work_items/verification_plans/repository_profile/validator_findings。
- `WorkItemCandidateMetaDto { reverted, revert_feedback }` 与后端 WP1 一致（与 WP4 revert 标记态对应）。
- `prepareWorkItemPlan` 请求体字段与后端 `PrepareWorkItemPlanRequest`（WP1 `src/web/types.rs`）一致。

**4. 边界风险**：
- **`WorkspaceType` 加新值的 exhaustive switch**（Task 1 Step 1.5）：前端多处 switch `workspace_type` 会报 exhaustive 错误。本 WP 加最小 fallback 分支让编译通过，WP7 细化 `work_item_plan` 分支。已标注。
- **废弃 `generateWorkItems` client 函数**（Task 3 Step 3.2）：WP5 删后端路由后，前端 `generateWorkItems` 函数成 dead code。本 WP 删除该函数 + 测试。已标注 grep 确认。
- **`handleLaunchWorkspace("work_item")` 对齐**（Task 2 Step 2.3）：该函数当前可能既被 work_item 入口用，也被其他地方用。改造时确认不影响 story/design 分支。已标注保留 story/design 不变。
- **prepare 参数默认值**（Task 2 Step 2.3）：`review_rounds`/4 个 split 选项的默认值需与旧弹窗一致（用户预期）。参考旧实现。已标注。

---

## Execution Handoff

本 WP6 plan 已保存至 `cadence/plans/2026-06-17_计划文档_实施计划_WorkItem对话式Workspace生成_WP6_前端入口与API_v1.0.md`。

**执行方式（待用户选择）：**

1. **Subagent-Driven（推荐）** — 每个 Task 派一个 fresh subagent 实现，Task 间做 two-stage review。
2. **Inline Execution** — 在当前 session 按 superpowers:executing-plans 批量执行，带 checkpoint 审查。

**完成后**：交付 WP6 后，继续 WP7（前端 candidate 面板 + WS 收发，依赖本 WP 的类型与 prepare API + 后端 WP2-5 的 WS 契约）。WP7 的「前置交付摘要」直接引用本 plan Task 3 Step 3.3 的产出。
