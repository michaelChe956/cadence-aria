# WorkItem 对话式 Workspace 生成 WP7：前端 candidate 面板 + WS Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 新增 `WorkItemPlanCandidatePanel`（WorkItem 列表/DAG/RepositoryProfile/findings + 每 WorkItem revert 按钮 + 批量触发 + 确认按钮）；`ChatWorkspacePage` 按 `workspace_type === "work_item_plan"` 分支渲染该面板；`workspace-ws-store` 处理 artifact payload union（markdown 或 candidate）并维护 `workItemPlanCandidate` 状态、新增 `sendRevertWorkItem`、复用 review/confirm/revision 收发。

**Architecture:** WP2a 后端已把 `ArtifactUpdate`/`SessionState.artifact` 切为 union（`{ markdown, diff }` 或 `{ candidate }` 扁平形态）。前端 WS store 按 union 分流：`markdown` 变体维持现有 `artifact` 状态（Story/Design），`candidate` 变体写入新 `workItemPlanCandidate` 状态。`WorkItemPlanCandidatePanel` 读 `workItemPlanCandidate` 渲染 WorkItem 卡片列表（标题/kind/写入范围/验证摘要）、DAG、RepositoryProfile、validator findings；每个 WorkItem `[revert]` 按钮 → `sendRevertWorkItem(work_item_id, feedback)`；底部"重新生成被标记的 N 项" → `sendRequestRevision(feedback)`；"确认计划" → `sendAuthorDecision("accept")`。review/confirm 交互复用 Story/Design 的骨架。

**Tech Stack:** React、TypeScript、Zustand、Vitest、`pnpm`（🔴 禁止 npm/yarn）。

**版本：** v1.0
**创建日期：** 2026-06-17
**目标分支：** `feat-b-0616`
**工作区：** `.worktrees/feat-b-0616`
**依赖总览：** `cadence/plans/2026-06-17_计划文档_实施计划_WorkItem对话式Workspace生成_拆分总览_v1.0.md`（v1.1，WP7 章节）
**设计方案：** `cadence/designs/2026-06-17_技术方案_WorkItem对话式Workspace生成_v1.0.md`（第 412-431 行前端设计、第 274-283 行 AuthorConfirm 与 revert、第 326-348 行 WS 协议）
**前置 WP：** WP6 + 后端 WP2a/WP2b/WP3/WP4/WP5 的 WS 契约

---

## 全局约束（Global Constraints）

- **包管理器**：前端必须用 `pnpm`。
- **测试命令**：`pnpm -C web test -- --run <过滤名>`；`pnpm -C web build`。
- **TDD**：每个 Task 先写失败测试，再写实现。
- **写入范围严格**：只改「File Structure」声明的文件。WP6 → WP7 串行（都改 `web/src/api/types.ts`）。
- **行号是参考**：基于 `feat-b-0616` HEAD `8a2eee4`；实现时以 `grep -n`/IDE 实际为准。

---

## 前置交付摘要（来自 WP6 + 后端 WP2a-WP5）

### 来自 WP6
- `prepareWorkItemPlan` API client 已就位（`web/src/api/client.ts`）。
- `PrepareWorkItemPlanRequest/Response` + `WorkItemPlanCandidateDto` + 子 DTO（`WorkItemPlanDto`/`WorkItemCandidateDto`/`WorkItemCandidateMetaDto`/`WorkItemSplitOptionsDto`/`WorkItemDependencyEdgeDto`/`ValidatorFindingDto`）+ `WorkspaceType` 含 `"work_item_plan"` 已定义（`web/src/api/types.ts`）。
- `IssueLifecycleWorkbench` 入口已调 `prepareWorkItemPlan` 并打开 `ChatWorkspacePage`。
- WP6 已为 `workspace_type` 的 exhaustive switch 加了最小 fallback 分支——WP7 细化 `work_item_plan` 分支。

### 来自后端 WP2a（artifact payload union）
- `WsOutMessage::ArtifactUpdate` JSON 扁平形态：`{ type: "artifact_update", version, markdown?, diff?, candidate? }`（markdown 或 candidate 互斥，无 `payload`/`kind` 包裹）。
- `SessionState.artifact`：`null` 或 `{ markdown, diff? }` 或 `{ candidate }`。

### 来自后端 WP2b/WP3/WP4/WP5（WS 消息时序）
- `StartGeneration` → `ArtifactUpdate`（candidate）→ `StageChange`（author_confirm）。
- `RevertWorkItem { work_item_id, feedback, clear }`（AuthorConfirm 阶段）→ `ArtifactUpdate`（同 version，candidate 的 `meta.reverted` 更新）。
- `RequestRevision { feedback }` → `ArtifactUpdate`（新 version candidate）→ `StageChange`（author_confirm）。
- `AuthorDecision { decision: "accept" }` →（review_rounds>0 则 `ReviewDecisionRequired`/`StreamChunk`/`ReviewComplete`）→ `StageChange`（human_confirm）。
- `ReviewDecisionResponse` → `StageChange`（human_confirm 或 author_confirm）。
- `HumanConfirm { action: "confirm" }` → `StageChange`（completed）。
- review/confirm/revision 消息复用 Story/Design 现有 WS 消息（`sendAuthorDecision`/`sendReviewDecision`/`sendHumanConfirm`/`sendRequestRevision` 现有）。

---

## 关键既有事实（避免重新探查）

实现时用 `grep -n`/IDE 确认。

### `web/src/state/workspace-ws-store.ts`
- Zustand store，维护 `artifact`（markdown String）、`artifactVersions`、`stage`、`messages`、`timelineNodes` 等。`grep -n "artifact\|artifactVersions\|stage\|sendStartGeneration\|sendAuthorDecision\|sendReviewDecision\|sendHumanConfirm\|sendRequestRevision" web/src/state/workspace-ws-store.ts`。
- WS 消息处理（`onMessage`/reducer）：按 `msg.type` 分发。`grep -n "artifact_update\|session_state\|stage_change\|case \"" web/src/state/workspace-ws-store.ts`。
- 现有 send 函数：`sendStartGeneration`/`sendAuthorDecision`/`sendReviewDecision`/`sendRequestRevision`/`sendHumanConfirm`/`sendAbort`。WP7 新增 `sendRevertWorkItem`。

### `web/src/hooks/useWorkspaceWs.ts`
- WS 连接 hook，绑定 store。`grep -n "useWorkspaceWs\|connect\|onMessage" web/src/hooks/useWorkspaceWs.ts`。

### `web/src/pages/ChatWorkspacePage.tsx`
- 按 `workspace_type` 分支渲染（Story/Design 渲染 Markdown Artifact Pane）。`grep -n "workspace_type\|workspaceType\|ArtifactPane\|switch" web/src/pages/ChatWorkspacePage.tsx`。WP6 加了 fallback 分支，WP7 改为 `work_item_plan` 渲染 `WorkItemPlanCandidatePanel`。
- review/confirm/revision 交互骨架（复用）。

### `web/src/components/workspace/`
- 现有组件（如 `ArtifactPane`、review/confirm 面板）。WP7 新增 `WorkItemPlanCandidatePanel.tsx`。`ls web/src/components/workspace/`。

---

## File Structure

| 文件 | 操作 | 职责 / 本 WP 改动 |
|---|---|---|
| `web/src/api/types.ts` | M | `RevertWorkItem` 消息类型（若 WP6 未覆盖）；`ArtifactPayload` union 前端类型（`{ markdown, diff? }` 或 `{ candidate }`） |
| `web/src/state/workspace-ws-store.ts` | M | `artifact` 仅保存 markdown；新增 `workItemPlanCandidate` 状态；`SessionState.artifact` 与 `ArtifactUpdate` 按 union 分流；新增 `sendRevertWorkItem` |
| `web/src/state/workspace-ws-store.test.ts` | M | artifact payload union、candidate 收发、revert 发送测试 |
| `web/src/hooks/useWorkspaceWs.ts` | M | 若 WS 消息分发在 hook 层，适配 union（一般分发在 store，hook 透传） |
| `web/src/hooks/useWorkspaceWs.test.tsx` | M | union 分流测试（若 hook 层有逻辑） |
| `web/src/pages/ChatWorkspacePage.tsx` | M | `workspace_type === "work_item_plan"` 分支渲染 `WorkItemPlanCandidatePanel`；复用 review/confirm/revision 交互 |
| `web/src/pages/ChatWorkspacePage.test.tsx` | M | `work_item_plan` 分支渲染 candidate 面板测试 |
| `web/src/components/workspace/WorkItemPlanCandidatePanel.tsx` | N | 新增：WorkItem 列表/DAG/Profile/findings + revert + 批量触发 + 确认 |
| `web/src/components/workspace/WorkItemPlanCandidatePanel.test.tsx` | N | 新增：列表/DAG/revert 标记/批量触发/确认测试 |

**不改：**
- ❌ `web/src/components/lifecycle/IssueLifecycleWorkbench.tsx`（WP6 已完成）
- ❌ 后端

---

## Task 1：`workspace-ws-store` 处理 artifact payload union + `sendRevertWorkItem`

**目标**：store 的 `artifact` 仅保存 markdown（Story/Design）；新增 `workItemPlanCandidate: WorkItemPlanCandidateDto | null` 状态；`SessionState.artifact` 与 `ArtifactUpdate` 按 union 分流（markdown → `artifact`，candidate → `workItemPlanCandidate`）；新增 `sendRevertWorkItem(work_item_id, feedback, clear)`。

**Files:**
- Modify: `web/src/state/workspace-ws-store.ts`
- Modify: `web/src/api/types.ts`（`RevertWorkItem` 消息类型 + `ArtifactPayload` union 前端类型）
- Test: `web/src/state/workspace-ws-store.test.ts`

**Interfaces:**
- Consumes: WP6 的 `WorkItemPlanCandidateDto`；后端 WP2a 的 union JSON 形态。
- Produces: `workItemPlanCandidate` store 状态；`sendRevertWorkItem`；union 分流的 message handler。

- [ ] **Step 1.1：写失败测试 —— artifact payload union 分流 + RevertWorkItem 发送**

在 `web/src/state/workspace-ws-store.test.ts`。参考现有 store 测试（`grep -n "artifact_update\|session_state\|sendStartGeneration" web/src/state/workspace-ws-store.test.ts`）。

```typescript
  it("artifact_update with markdown payload updates artifact state", () => {
    const store = createTestStore();
    store.handleMessage({ type: "artifact_update", version: 1, markdown: "# Story" });
    expect(store.getState().artifact).toBe("# Story");
    expect(store.getState().workItemPlanCandidate).toBeNull();
  });

  it("artifact_update with candidate payload updates workItemPlanCandidate state", () => {
    const store = createTestStore();
    const candidate = { plan: { id: "plan_1", status: "draft", options: {...}, dependency_graph: [] }, work_items: [{ id: "wi_1", kind: "backend", title: "API", depends_on: [], exclusive_write_scopes: ["src/api"], verification_plan_ref: null, meta: { reverted: false, revert_feedback: null } }], verification_plans: [], repository_profile: null, validator_findings: [] };
    store.handleMessage({ type: "artifact_update", version: 2, candidate });
    expect(store.getState().workItemPlanCandidate).toEqual(candidate);
    expect(store.getState().artifact).toBe(""); // 或 null，以现有初始值为准
  });

  it("session_state artifact candidate hydrates workItemPlanCandidate", () => {
    const store = createTestStore();
    const candidate = { plan: {...}, work_items: [...], ... };
    store.handleMessage({ type: "session_state", workspace_type: "work_item_plan", stage: "author_confirm", artifact: { candidate }, ... });
    expect(store.getState().workItemPlanCandidate).toEqual(candidate);
  });

  it("sendRevertWorkItem sends revert_work_item message", () => {
    const store = createTestStore();
    const send = vi.spyOn(store.getState(), "send");
    store.getState().sendRevertWorkItem("work_item_0001", "拆得太粗", false);
    expect(send).toHaveBeenCalledWith({ type: "revert_work_item", work_item_id: "work_item_0001", feedback: "拆得太粗", clear: false });
  });

  it("candidate meta reverted updates on same-version artifact_update", () => {
    const store = createTestStore();
    store.handleMessage({ type: "artifact_update", version: 1, candidate: { ...candidate, work_items: [{ ...wi, meta: { reverted: false, revert_feedback: null } }] } });
    store.handleMessage({ type: "artifact_update", version: 1, candidate: { ...candidate, work_items: [{ ...wi, meta: { reverted: true, revert_feedback: "拆得太粗" } }] } });
    expect(store.getState().workItemPlanCandidate?.work_items[0].meta.reverted).toBe(true);
  });
```

> 实现者注意：`createTestStore`/`handleMessage`/`send` 以现有 store 测试模式为准（`grep -n "createTestStore\|handleMessage\|send" web/src/state/workspace-ws-store.test.ts`）。若 store 用 `onMessage(msg)` 而非 `handleMessage`，照搬现有命名。

- [ ] **Step 1.2：运行测试，确认失败**

Run: `pnpm -C web test -- --run workspace-ws-store`
Expected: 失败——`workItemPlanCandidate` 状态不存在、union 分流未实现、`sendRevertWorkItem` 未定义。

- [ ] **Step 1.3：`api/types.ts` 加 union 类型 + RevertWorkItem**

`web/src/api/types.ts`：

```typescript
// ArtifactUpdate / SessionState.artifact 的 union 形态（后端 WP2a 扁平 JSON）
export interface ArtifactUpdateMessage {
  type: "artifact_update";
  version: number;
  markdown?: string;
  diff?: string | null;
  candidate?: WorkItemPlanCandidateDto;
}

export interface RevertWorkItemMessage {
  type: "revert_work_item";
  work_item_id: string;
  feedback?: string | null;
  clear: boolean;
}
```

> 现有 `WsInMessage`/`WsOutMessage` 类型（若有统一定义）补 `RevertWorkItemMessage` 到 in 消息联合；`ArtifactUpdateMessage` 替换或兼容现有 `artifact_update` 定义。`grep -n "artifact_update\|WsInMessage\|WsOutMessage\|type.*message" web/src/api/types.ts` 确认现有结构，避免重复定义。

- [ ] **Step 1.4：store 加 `workItemPlanCandidate` 状态 + union 分流 + `sendRevertWorkItem`**

`web/src/state/workspace-ws-store.ts`：

1. **state 加 `workItemPlanCandidate: WorkItemPlanCandidateDto | null`**，初始 `null`。
2. **`artifact_update` handler 分流**：

```typescript
  if (msg.type === "artifact_update") {
    if (msg.candidate) {
      set({ workItemPlanCandidate: msg.candidate });
      // artifact 不变（WorkItemPlan 无 markdown artifact）
    } else if (msg.markdown !== undefined) {
      set({ artifact: msg.markdown, workItemPlanCandidate: null });
    }
    // 更新 artifactVersions（version/summary）——参考现有逻辑
  }
```

3. **`session_state` handler 的 `artifact` 分流**：

```typescript
  // 在 session_state handler 内
  if (msg.artifact) {
    if ("candidate" in msg.artifact && msg.artifact.candidate) {
      set({ workItemPlanCandidate: msg.artifact.candidate, artifact: "" });
    } else if ("markdown" in msg.artifact && msg.artifact.markdown) {
      set({ artifact: msg.artifact.markdown, workItemPlanCandidate: null });
    }
  }
```

4. **`sendRevertWorkItem`**：

```typescript
  sendRevertWorkItem: (workItemId, feedback, clear) => {
    get().send({ type: "revert_work_item", work_item_id: workItemId, feedback: feedback ?? null, clear });
  },
```

> 实现者注意：
> 1. 现有 `artifact` 状态类型是 `string`（markdown）——保持不变，WorkItemPlan 不写 `artifact`。
> 2. `artifactVersions` 的 summary（`markdown_size`/`markdown_preview`）对 candidate 变体由后端派生（WP2a），前端照常展示 summary 列表，无需特殊处理。
> 3. `send` 是现有 WS send 函数（`grep -n "send:" web/src/state/workspace-ws-store.ts`）。
> 4. 顶部 `import { WorkItemPlanCandidateDto, RevertWorkItemMessage } from "../api/types";`。

- [ ] **Step 1.5：运行 Task 1 测试 + 构建**

Run:
```
pnpm -C web test -- --run workspace-ws-store
pnpm -C web build
```
Expected: 新测试 PASS；现有 store 测试全绿（Story/Design 的 markdown artifact 行为不变）；`pnpm -C web build` 全绿。

- [ ] **Step 1.6：提交**

```bash
git add web/src/state/workspace-ws-store.ts web/src/api/types.ts web/src/state/workspace-ws-store.test.ts
git commit -m "feat(WP7): workspace-ws-store artifact payload union 分流 + sendRevertWorkItem"
```

---

## Task 2：`WorkItemPlanCandidatePanel` 组件

**目标**：新增 `WorkItemPlanCandidatePanel`：展示 candidate（WorkItem 列表/DAG/RepositoryProfile/findings）+ 每个 WorkItem `[revert]` 按钮（弹反馈输入）+ 底部"重新生成被标记的 N 项"/"确认计划"按钮。读 store 的 `workItemPlanCandidate` + `sendRevertWorkItem`/`sendRequestRevision`/`sendAuthorDecision`。

**Files:**
- Create: `web/src/components/workspace/WorkItemPlanCandidatePanel.tsx`
- Test: `web/src/components/workspace/WorkItemPlanCandidatePanel.test.tsx`

**Interfaces:**
- Consumes: `workItemPlanCandidate` store 状态；`sendRevertWorkItem`/`sendRequestRevision`/`sendAuthorDecision`；`stage`（判断是否 AuthorConfirm 以显示 revert 按钮）。
- Produces: `WorkItemPlanCandidatePanel` React 组件。

- [ ] **Step 2.1：写失败测试 —— 列表/DAG/revert 标记/批量触发/确认**

`web/src/components/workspace/WorkItemPlanCandidatePanel.test.tsx`：

```typescript
import { render, screen, fireEvent } from "@testing-library/react";
import { WorkItemPlanCandidatePanel } from "./WorkItemPlanCandidatePanel";

const candidate = {
  plan: { id: "plan_1", status: "draft", options: { include_integration_tests: true, include_e2e_tests: false, force_frontend_backend_split: false, require_execution_plan_confirm: false }, dependency_graph: [{ from_work_item_id: "wi_1", to_work_item_id: "wi_2" }] },
  work_items: [
    { id: "wi_1", kind: "backend", title: "后端 API", depends_on: [], exclusive_write_scopes: ["src/api"], verification_plan_ref: "vp_1", meta: { reverted: false, revert_feedback: null } },
    { id: "wi_2", kind: "frontend", title: "前端页面", depends_on: ["wi_1"], exclusive_write_scopes: ["web/src"], verification_plan_ref: "vp_2", meta: { reverted: false, revert_feedback: null } },
  ],
  verification_plans: [],
  repository_profile: null,
  validator_findings: [],
};

describe("WorkItemPlanCandidatePanel", () => {
  it("renders work item list with title/kind/scopes", () => {
    render(<WorkItemPlanCandidatePanel candidate={candidate} stage="author_confirm" onRevert={vi.fn()} onRequestRevision={vi.fn()} onAccept={vi.fn()} />);
    expect(screen.getByText("后端 API")).toBeInTheDocument();
    expect(screen.getByText("前端页面")).toBeInTheDocument();
    expect(screen.getByText(/backend/)).toBeInTheDocument();
  });

  it("renders dependency DAG edges", () => {
    render(<WorkItemPlanCandidatePanel candidate={candidate} stage="author_confirm" onRevert={vi.fn()} onRequestRevision={vi.fn()} onAccept={vi.fn()} />);
    // DAG 边 wi_1 → wi_2 展示（具体断言以 DAG 渲染方式为准）
    expect(screen.getByText(/wi_1.*wi_2|wi_1 → wi_2/)).toBeInTheDocument();
  });

  it("revert button calls onRevert with feedback", async () => {
    const onRevert = vi.fn();
    render(<WorkItemPlanCandidatePanel candidate={candidate} stage="author_confirm" onRevert={onRevert} onRequestRevision={vi.fn()} onAccept={vi.fn()} />);
    fireEvent.click(screen.getAllByRole("button", { name: /revert|重做/ })[0]);
    // 输入反馈
    fireEvent.change(screen.getByPlaceholderText(/反馈/), { target: { value: "拆得太粗" } });
    fireEvent.click(screen.getByRole("button", { name: /确认|提交/ }));
    expect(onRevert).toHaveBeenCalledWith("wi_1", "拆得太粗", false);
  });

  it("shows regenerate button with marked count when items reverted", () => {
    const revertedCandidate = { ...candidate, work_items: [{ ...candidate.work_items[0], meta: { reverted: true, revert_feedback: "粗" } }, candidate.work_items[1]] };
    render(<WorkItemPlanCandidatePanel candidate={revertedCandidate} stage="author_confirm" onRevert={vi.fn()} onRequestRevision={vi.fn()} onAccept={vi.fn()} />);
    expect(screen.getByRole("button", { name: /重新生成被标记的 1 项/ })).toBeInTheDocument();
  });

  it("regenerate button calls onRequestRevision", () => {
    const onRequestRevision = vi.fn();
    const revertedCandidate = { ...candidate, work_items: [{ ...candidate.work_items[0], meta: { reverted: true, revert_feedback: "粗" } }, candidate.work_items[1]] };
    render(<WorkItemPlanCandidatePanel candidate={revertedCandidate} stage="author_confirm" onRevert={vi.fn()} onRequestRevision={onRequestRevision} onAccept={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: /重新生成被标记的 1 项/ }));
    expect(onRequestRevision).toHaveBeenCalled();
  });

  it("confirm button calls onAccept", () => {
    const onAccept = vi.fn();
    render(<WorkItemPlanCandidatePanel candidate={candidate} stage="author_confirm" onRevert={vi.fn()} onRequestRevision={vi.fn()} onAccept={onAccept} />);
    fireEvent.click(screen.getByRole("button", { name: /确认计划/ }));
    expect(onAccept).toHaveBeenCalled();
  });
});
```

> 实现者注意：
> 1. 组件 props 设计为 `candidate`/`stage`/`onRevert`/`onRequestRevision`/`onAccept`（便于测试隔离 store）。实际在 `ChatWorkspacePage` 调用时，从 store 取 `workItemPlanCandidate` + `sendRevertWorkItem`/`sendRequestRevision`/`sendAuthorDecision` 传入。
> 2. DAG 渲染：可用简单文本列表（`wi_1 → wi_2`）或轻量图组件。本 WP 用文本/列表渲染（不引入图库），保持简单。
> 3. revert 反馈输入：点 `[revert]` 弹出 inline 输入框或小 dialog，输入反馈后提交。`clear` 按钮（取消标记）可选。
> 4. validator_findings 展示：warning 列表（severity/code/message/work_item_ids）。
> 5. repository_profile 展示：confidence + detected_layers（若非 null）。

- [ ] **Step 2.2：运行测试，确认失败**

Run: `pnpm -C web test -- --run WorkItemPlanCandidatePanel`
Expected: 失败——组件未定义。

- [ ] **Step 2.3：实现 `WorkItemPlanCandidatePanel`**

`web/src/components/workspace/WorkItemPlanCandidatePanel.tsx`：

```tsx
import { useState } from "react";
import type { WorkItemPlanCandidateDto, WorkItemCandidateDto } from "../../api/types";

interface Props {
  candidate: WorkItemPlanCandidateDto;
  stage: string;
  onRevert: (workItemId: string, feedback: string, clear: boolean) => void;
  onRequestRevision: (feedback?: string) => void;
  onAccept: () => void;
}

export function WorkItemPlanCandidatePanel({ candidate, stage, onRevert, onRequestRevision, onAccept }: Props) {
  const [revertTarget, setRevertTarget] = useState<WorkItemCandidateDto | null>(null);
  const [feedback, setFeedback] = useState("");
  const revertedCount = candidate.work_items.filter((w) => w.meta.reverted).length;
  const isAuthorConfirm = stage === "author_confirm";

  return (
    <div className="work-item-plan-candidate-panel">
      <h3>Work Item Plan 候选</h3>
      {/* DAG */}
      <section>
        <h4>依赖 DAG</h4>
        <ul>
          {candidate.plan.dependency_graph.map((edge, i) => (
            <li key={i}>{edge.from_work_item_id} → {edge.to_work_item_id}</li>
          ))}
        </ul>
      </section>
      {/* WorkItem 列表 */}
      <section>
        <h4>Work Items</h4>
        {candidate.work_items.map((wi) => (
          <div key={wi.id} className="work-item-card" style={{ opacity: wi.meta.reverted ? 0.5 : 1 }}>
            <h5>{wi.title} <small>{wi.kind}</small></h5>
            <p>写入范围: {wi.exclusive_write_scopes.join(", ")}</p>
            <p>依赖: {wi.depends_on.join(", ") || "无"}</p>
            {wi.verification_plan_ref && <p>验证计划: {wi.verification_plan_ref}</p>}
            {wi.meta.reverted && <p style={{ color: "orange" }}>已标记重做: {wi.meta.revert_feedback}</p>}
            {isAuthorConfirm && !wi.meta.reverted && (
              <button onClick={() => { setRevertTarget(wi); setFeedback(""); }}>Revert</button>
            )}
            {isAuthorConfirm && wi.meta.reverted && (
              <button onClick={() => onRevert(wi.id, "", true)}>取消标记</button>
            )}
          </div>
        ))}
      </section>
      {/* Repository Profile */}
      {candidate.repository_profile && (
        <section>
          <h4>Repository Profile</h4>
          <p>置信度: {candidate.repository_profile.confidence}</p>
          {/* detected_layers 等，以 DTO 实际字段为准 */}
        </section>
      )}
      {/* Validator Findings */}
      {candidate.validator_findings.length > 0 && (
        <section>
          <h4>校验发现</h4>
          <ul>
            {candidate.validator_findings.map((f, i) => (
              <li key={i} style={{ color: f.severity === "error" ? "red" : "orange" }}>
                [{f.severity}] {f.code}: {f.message} {f.work_item_ids?.length ? `(${f.work_item_ids.join(", ")})` : ""}
              </li>
            ))}
          </ul>
        </section>
      )}
      {/* revert 反馈输入 */}
      {revertTarget && (
        <div className="revert-feedback-dialog">
          <p>重做 {revertTarget.title} 的反馈</p>
          <input placeholder="反馈（可选）" value={feedback} onChange={(e) => setFeedback(e.target.value)} />
          <button onClick={() => { onRevert(revertTarget.id, feedback, false); setRevertTarget(null); }}>提交</button>
          <button onClick={() => setRevertTarget(null)}>取消</button>
        </div>
      )}
      {/* 底部操作 */}
      {isAuthorConfirm && (
        <div className="actions">
          <button disabled={revertedCount === 0} onClick={() => onRequestRevision()}>
            重新生成被标记的 {revertedCount} 项
          </button>
          <button onClick={onAccept}>确认计划</button>
        </div>
      )}
    </div>
  );
}
```

> 实现者注意：
> 1. `repository_profile` 的字段（confidence/detected_layers 等）以 WP6 定义的 `RepositoryProfileDto` 为准——`grep -n "RepositoryProfileDto" web/src/api/types.ts`。若无定义，补最小字段。
> 2. 样式用 className + inline style（本 WP 不引入 CSS 模块，保持简单；后续可细化）。
> 3. `stage` 判断 AuthorConfirm：`stage === "author_confirm"`。revert 按钮只在 AuthorConfirm 显示。
> 4. `onAccept` = `sendAuthorDecision("accept")`；`onRequestRevision` = `sendRequestRevision(feedback)`；`onRevert` = `sendRevertWorkItem`。

- [ ] **Step 2.4：运行 Task 2 测试 + 构建**

Run:
```
pnpm -C web test -- --run WorkItemPlanCandidatePanel
pnpm -C web build
```
Expected: 新测试 PASS；`pnpm -C web build` 全绿。

- [ ] **Step 2.5：提交**

```bash
git add web/src/components/workspace/WorkItemPlanCandidatePanel.tsx web/src/components/workspace/WorkItemPlanCandidatePanel.test.tsx
git commit -m "feat(WP7): WorkItemPlanCandidatePanel 组件（列表/DAG/revert/批量触发/确认）"
```

---

## Task 3：`ChatWorkspacePage` `work_item_plan` 分支 + 收口验证

**目标**：`ChatWorkspacePage` 按 `workspace_type === "work_item_plan"` 分支渲染 `WorkItemPlanCandidatePanel`（不渲染 Markdown Artifact Pane），从 store 取 `workItemPlanCandidate` + send 函数传入；复用 Story/Design 的 review/confirm/revision 交互骨架。收口全量验证。

**Files:**
- Modify: `web/src/pages/ChatWorkspacePage.tsx`
- Modify: `web/src/pages/ChatWorkspacePage.test.tsx`
- Modify: `web/src/hooks/useWorkspaceWs.ts`（若 hook 层有 union 分发逻辑，Task 1 已在 store 处理，hook 一般无需改；若 hook 透传消息，确认无误）

**Interfaces:**
- Consumes: Task 1 的 `workItemPlanCandidate`/`sendRevertWorkItem`；Task 2 的 `WorkItemPlanCandidatePanel`。
- Produces: `ChatWorkspacePage` 的 `work_item_plan` 分支。

- [ ] **Step 3.1：写失败测试 —— `work_item_plan` 分支渲染 candidate 面板**

`web/src/pages/ChatWorkspacePage.test.tsx`：

```typescript
  it("renders WorkItemPlanCandidatePanel for work_item_plan workspace", () => {
    render(<ChatWorkspacePage {...defaultProps} workspaceType="work_item_plan" />);
    expect(screen.getByText("Work Item Plan 候选")).toBeInTheDocument();
    expect(screen.queryByText(/Markdown Artifact/)).not.toBeInTheDocument();
  });

  it("does not render candidate panel for story workspace", () => {
    render(<ChatWorkspacePage {...defaultProps} workspaceType="story" />);
    expect(screen.queryByText("Work Item Plan 候选")).not.toBeInTheDocument();
  });
```

> 实现者注意：`defaultProps`/`workspaceType` 以现有 `ChatWorkspacePage.test.tsx` 模式为准（`grep -n "workspaceType\|workspace_type\|render.*ChatWorkspacePage" web/src/pages/ChatWorkspacePage.test.tsx`）。若 workspace_type 从 store 取（非 prop），用 mock store 注入。

- [ ] **Step 3.2：运行测试，确认失败**

Run: `pnpm -C web test -- --run ChatWorkspacePage`
Expected: 失败——`work_item_plan` 分支未渲染 candidate 面板（WP6 的 fallback 分支）。

- [ ] **Step 3.3：`ChatWorkspacePage` 加 `work_item_plan` 分支**

`web/src/pages/ChatWorkspacePage.tsx`，找到 `workspace_type` 的 switch/条件（WP6 加了 fallback）。改为：

```tsx
  {workspaceType === "work_item_plan" ? (
    <WorkItemPlanCandidatePanel
      candidate={workItemPlanCandidate}
      stage={stage}
      onRevert={(id, feedback, clear) => sendRevertWorkItem(id, feedback, clear)}
      onRequestRevision={(feedback) => sendRequestRevision(feedback)}
      onAccept={() => sendAuthorDecision("accept")}
    />
  ) : (
    /* 现有 Markdown Artifact Pane（Story/Design） */
  )}
```

> 实现者注意：
> 1. `workItemPlanCandidate`/`stage`/`sendRevertWorkItem`/`sendRequestRevision`/`sendAuthorDecision` 从 `useWorkspaceWs`/store 取（参考现有 Story/Design 怎么取 `artifact`/`stage`/send 函数）。
> 2. review/confirm 交互骨架（review decision 弹窗、human confirm 按钮）复用现有——WorkItemPlan 的 review/confirm 消息与 Story/Design 相同，UI 骨架通用。`grep -n "ReviewDecision\|HumanConfirm\|review\|confirm" web/src/pages/ChatWorkspacePage.tsx` 确认现有骨架。
> 3. 顶部 `import { WorkItemPlanCandidatePanel } from "../components/workspace/WorkItemPlanCandidatePanel";`。
> 4. `candidate` 为 null 时（prepare 后未 start_generation）显示"点开始生成"提示或空状态。

- [ ] **Step 3.4：运行 Task 3 测试 + 全量验证**

Run:
```
pnpm -C web test -- --run ChatWorkspacePage
pnpm -C web test -- --run WorkItemPlanCandidatePanel
pnpm -C web test -- --run workspace-ws-store
pnpm -C web test -- --run useWorkspaceWs
pnpm -C web test -- --run IssueLifecycleWorkbench
pnpm -C web build
```
Expected: 全绿。

- [ ] **Step 3.5：提交**

```bash
git add web/src/pages/ChatWorkspacePage.tsx web/src/pages/ChatWorkspacePage.test.tsx
git commit -m "feat(WP7): ChatWorkspacePage work_item_plan 分支渲染 WorkItemPlanCandidatePanel"
```

---

## Task 4：WP7 收口验证

**目标**：跑完整前端验证，确保 WP7 改动未破坏 Story/Design/WorkItem 既有流程；WorkItemPlan candidate 面板 + WS 收发链路通。

**Files:** 无新增改动；仅运行验证命令。

- [ ] **Step 4.1：全量验证链**

Run:
```
pnpm -C web test -- --run
pnpm -C web build
```
Expected: 全绿。

> `pnpm -C web test -- --run` 跑全部前端测试，覆盖 Story/Design/WorkItem/WorkItemPlan 的 UI + WS store + hooks。

- [ ] **Step 4.2：确认 WP6 成果未破坏**

Run: `pnpm -C web test -- --run IssueLifecycleWorkbench`
Expected: PASS。

- [ ] **Step 4.3：交付摘要（供 WP8 前置交付摘要使用）**

commit 后，把以下内容写入 WP8 plan 的「前置交付摘要」章节：

- 前端 WorkItemPlan 全链路就绪：`prepareWorkItemPlan` → `ChatWorkspacePage`（`work_item_plan` 分支）→ `WorkItemPlanCandidatePanel`（candidate 展示 + revert + 批量触发 + 确认）+ WS store（artifact payload union 分流 + `sendRevertWorkItem`）。
- `workspace-ws-store`：`workItemPlanCandidate` 状态（candidate 变体），`artifact` 状态（markdown 变体）；`SessionState.artifact`/`ArtifactUpdate` 按 union 分流。
- `WorkItemPlanCandidatePanel`：WorkItem 列表/DAG/RepositoryProfile/findings + 每 WorkItem revert + 批量"重新生成被标记的 N 项" + "确认计划"。
- review/confirm/revision 交互复用 Story/Design 骨架（`sendAuthorDecision`/`sendReviewDecision`/`sendRequestRevision`/`sendHumanConfirm`）。
- **WP8 待办**：贯通测试 prepare→author→revert→review→revision→confirm（前端 + 后端 E2E，Fake provider）；四种 workspace type 恢复链路评估；废弃路由 404。

---

## Self-Review（写完后的自查）

**1. Spec 覆盖**（对照总览 v1.1 WP7 目标/写入范围/验证 + 设计方案 :412-431、:274-283、:326-348）：
- ✅ `WorkItemPlanCandidatePanel`（列表/DAG/Profile/findings + revert + 批量触发 + 确认）→ Task 2
- ✅ `ChatWorkspacePage` 按 `work_item_plan` 分支渲染 → Task 3
- ✅ WS store 处理 artifact payload union（markdown 或 candidate）→ Task 1
- ✅ `workItemPlanCandidate` 状态 → Task 1 Step 1.4
- ✅ `sendRevertWorkItem` → Task 1 Step 1.4
- ✅ 复用 `sendStartGeneration`/`sendAuthorDecision`/`sendReviewDecision`/`sendHumanConfirm` → Task 3 Step 3.3（onAccept/onRequestRevision 绑定）
- ✅ 验证命令链 → Task 4
- ✅ 不做项：未改后端、未写 Playwright E2E——均在「不做」清单。

**2. Placeholder 扫描**：
- `RepositoryProfileDto` 字段（Task 2 Step 2.3）：给出 grep 确认指引，若无补最小字段。属可接受。
- DAG 渲染方式（Task 2）：用文本列表，不引入图库。属可接受。
- `defaultProps`/`workspaceType` 测试夹具（Task 3）：给出 grep 确认指引。属可接受。
- review/confirm 交互骨架复用（Task 3 Step 3.3）：给出 grep 确认现有骨架指引。属可接受。

**3. 类型一致性**：
- `workItemPlanCandidate: WorkItemPlanCandidateDto | null` 与后端 WP2a 的 candidate payload 一致。
- `sendRevertWorkItem(work_item_id, feedback, clear)` 与后端 `WsInMessage::RevertWorkItem { work_item_id, feedback: Option<String>, clear: bool }` 一致。
- `ArtifactUpdateMessage` union（markdown?/candidate?）与后端 WP2a 扁平 JSON 一致。
- `WorkItemPlanCandidatePanel` props（candidate/stage/onRevert/onRequestRevision/onAccept）与 store send 函数绑定一致。

**4. 边界风险**：
- **union 分流的 `SessionState.artifact`**（Task 1 Step 1.4）：后端 `SessionState.artifact` 是 `Option<ArtifactPayload>`，JSON 为 `null` 或 `{ markdown, diff? }` 或 `{ candidate }`。前端用 `"candidate" in msg.artifact` / `"markdown" in msg.artifact` 判断——需处理 `null` 情况。已标注。
- **同 version 的 revert 标记更新**（Task 1 Step 1.1）：revert 标记推同 version 的 `ArtifactUpdate`（meta 变化），前端 `workItemPlanCandidate` 直接覆盖为新 candidate——version 不递增，前端不依赖 version 判断是否更新。已标注（测试覆盖）。
- **`candidate` 为 null 时的空状态**（Task 3 Step 3.3）：prepare 后未 start_generation 时 `workItemPlanCandidate` 为 null，面板应显示空状态或"点开始生成"。已标注。
- **review/confirm 骨架复用**（Task 3 Step 3.3）：WorkItemPlan 的 review/confirm 消息与 Story/Design 相同，但 UI 骨架是否完全通用需确认（如 review decision 弹窗、human confirm 按钮是否 workspace_type 无关）。`grep -n "workspace_type\|workspaceType" web/src/pages/ChatWorkspacePage.tsx` 确认无 WorkItemPlan 特殊分支遗漏。已标注。
- **WP6 的 fallback 分支替换**（Task 3 Step 3.3）：WP6 加的最小 fallback 分支由本 WP 的 `WorkItemPlanCandidatePanel` 分支替换，确认无残留 fallback。已标注。

---

## Execution Handoff

本 WP7 plan 已保存至 `cadence/plans/2026-06-17_计划文档_实施计划_WorkItem对话式Workspace生成_WP7_前端candidate面板_v1.0.md`。

**执行方式（待用户选择）：**

1. **Subagent-Driven（推荐）** — 每个 Task 派一个 fresh subagent 实现，Task 间做 two-stage review。
2. **Inline Execution** — 在当前 session 按 superpowers:executing-plans 批量执行，带 checkpoint 审查。

**完成后**：交付 WP7 后，前端全链路就绪。继续 WP8（贯通测试，依赖 WP1-WP7 全部）。WP8 的「前置交付摘要」直接引用本 plan Task 4 Step 4.3 的产出。
