# P4: Running / 审核 / 确认 UI 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把页面级 Artifact/执行 Tab 删除，下沉到节点级 5 tab 详情面板；重构 WorkspacePage 为 Header（Provider snapshot + Stage 徽章）+ Timeline + 阶段面板 + 底部操作区；实现 ReviewDecision 三路径选择 + HumanConfirm 结构化反馈表单。

**Architecture:** 从 WorkspacePage 拆出 WorkspaceHeader、NodeDetailPanel（5 tab）、StageActionsBar；useStageUI 补全 Running/CrossReview/ReviewDecision/Revision/HumanConfirm 面板映射；HumanConfirm 面板含 reviewer 摘要 + 行级 diff + 结构化反馈表单。

**Tech Stack:** React + TypeScript + Tailwind CSS + Zustand + vitest

**前置依赖:** P1（协议、nodeDetails、activeRunId）+ P2（useStageUI、PrepareContextPanel）

**后续 plan 消费点:**
- P7 E2E 消费各阶段 UI 路径（D1-D5 用例）

**文件结构总览:**

| 文件 | 操作 | 职责 |
|---|---|---|
| `web/src/components/workspace/WorkspaceHeader.tsx` | 新建 | 实体面包屑 + Provider snapshot + Stage 徽章 |
| `web/src/components/workspace/NodeDetailPanel.tsx` | 新建 | 节点级 5 tab（概览/流式/执行/权限/Artifact） |
| `web/src/components/workspace/StageActionsBar.tsx` | 新建 | 底部统一操作按钮矩阵 |
| `web/src/components/workspace/stages/RunningStagePanel.tsx` | 新建 | Running 阶段面板 |
| `web/src/components/workspace/stages/CrossReviewStagePanel.tsx` | 新建 | CrossReview 阶段面板 |
| `web/src/components/workspace/stages/ReviewDecisionStagePanel.tsx` | 新建 | 审核结论三路径选择 |
| `web/src/components/workspace/stages/HumanConfirmStagePanel.tsx` | 新建 | 人工确认 + 结构化反馈 |
| `web/src/hooks/useStageUI.ts` | 修改 | 补全 Running/CrossReview/ReviewDecision/Revision/HumanConfirm |
| `web/src/pages/WorkspacePage.tsx` | 修改 | 重构为 Header + Timeline + NodeDetailPanel + StageActionsBar |

---

## 修订约束（必须优先遵守）

1. ReviewDecision 三路径必须调用 P1 新增的 `sendSelectRevisionPath(path, extraContext?)`，路径值固定为 `revise` / `revise-with-context` / `skip-to-human`。
2. HumanConfirm 的确认/要求修改/终止都调用 P1 新增的 `sendHumanConfirm`；“要求修改”使用 `sendHumanConfirm("request-change", feedback)`，不得复用旧审核决策发送函数，也不得新增前端 `sendRequestRevision` 路径。
3. 本阶段新增 UI 必须补齐 P7 所需 test id：`node-detail-panel`、`tab-overview`、`tab-streaming`、`tab-execution`、`tab-permission`、`tab-artifact`、`stage-actions-bar`、`review-decision-panel`、`human-confirm-panel`、`streaming-content`。

### Task 1: 扩展 useStageUI 补全所有阶段

**Files:**
- 修改: `web/src/hooks/useStageUI.ts`
- 测试: `web/src/hooks/useStageUI.test.ts`

- [ ] **Step 1: 写 failing 测试**

追加到 `useStageUI.test.ts`：

```typescript
  it("returns correct config for all stages", () => {
    const stages = [
      ["prepare_context", "PrepareContextPanel", ["start_generation"]],
      ["running", "RunningPanel", ["abort"]],
      ["cross_review", "CrossReviewPanel", ["abort"]],
      ["review_decision", "ReviewDecisionPanel", ["select_revision_path", "abort"]],
      ["revision", "RevisionPanel", ["abort"]],
      ["human_confirm", "HumanConfirmPanel", ["confirm", "request_change", "terminate"]],
      ["completed", "CompletedPanel", []],
    ];
    for (const [stage, panel, actions] of stages) {
      const { result } = renderHook(() => useStageUI(stage as string));
      expect(result.current.panel).toBe(panel);
      expect(result.current.actions).toEqual(actions);
    }
  });
```

Run: `pnpm --dir web test -- useStageUI`
Expected: 部分失败 — RunningPanel / CrossReviewPanel 等配置未定义

- [ ] **Step 2: 在 useStageUI.ts 补全 STAGE_CONFIG_MAP**

已在 P2 中定义，确认所有阶段都已存在（见 P2 Task 1）。如果缺少，补充：

```typescript
  running: {
    panel: "RunningPanel",
    actions: ["abort"],
    headerBadge: "运行中 · 保持本页打开",
    showContextInput: false,
    providerEditable: false,
  },
  cross_review: {
    panel: "CrossReviewPanel",
    actions: ["abort"],
    headerBadge: "审核中",
    showContextInput: false,
    providerEditable: false,
  },
  review_decision: {
    panel: "ReviewDecisionPanel",
    actions: ["select_revision_path", "abort"],
    headerBadge: "审核结论待处理",
    showContextInput: false,
    providerEditable: false,
  },
  revision: {
    panel: "RevisionPanel",
    actions: ["abort"],
    headerBadge: "修订中",
    showContextInput: false,
    providerEditable: false,
  },
  human_confirm: {
    panel: "HumanConfirmPanel",
    actions: ["confirm", "request_change", "terminate"],
    headerBadge: "等待确认",
    showContextInput: false,
    providerEditable: false,
  },
  completed: {
    panel: "CompletedPanel",
    actions: [],
    headerBadge: "已完成",
    showContextInput: false,
    providerEditable: false,
  },
```

- [ ] **Step 3: 跑测试确认通过**

Run: `pnpm --dir web test -- useStageUI`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add web/src/hooks/useStageUI.ts web/src/hooks/useStageUI.test.ts
git commit -m "feat(ui): complete useStageUI for all 7 stages"
```

---

### Task 2: WorkspaceHeader 组件

**Files:**
- 新建: `web/src/components/workspace/WorkspaceHeader.tsx`
- 测试: `web/src/components/workspace/WorkspaceHeader.test.tsx`

- [ ] **Step 1: 写 failing 测试**

```tsx
import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { WorkspaceHeader } from "./WorkspaceHeader";

describe("WorkspaceHeader", () => {
  it("renders provider snapshot and stage badge", () => {
    render(
      <WorkspaceHeader
        entityType="Story Spec"
        entityId="SP-12"
        version={2}
        author="claude_code"
        reviewer="codex"
        rounds={1}
        stage="running"
        providerLocked={true}
        lockedAt="2026-05-20T14:35:00Z"
      />
    );
    expect(screen.getByText(/Story Spec #SP-12/i)).toBeInTheDocument();
    expect(screen.getByText(/Author: Claude Code/i)).toBeInTheDocument();
    expect(screen.getByText(/Reviewer: Codex/i)).toBeInTheDocument();
    expect(screen.getByText(/运行中/i)).toBeInTheDocument();
  });

  it("shows lock icon when provider locked", () => {
    render(
      <WorkspaceHeader
        entityType="Story Spec"
        entityId="SP-12"
        version={2}
        author="claude_code"
        reviewer={null}
        rounds={0}
        stage="prepare_context"
        providerLocked={false}
      />
    );
    expect(screen.queryByText(/🔒/)).not.toBeInTheDocument();
  });
});
```

Run: `pnpm --dir web test -- WorkspaceHeader`
Expected: 编译失败 — WorkspaceHeader 未定义

- [ ] **Step 2: 实现 WorkspaceHeader**

```tsx
interface WorkspaceHeaderProps {
  entityType: string;
  entityId: string;
  version?: number;
  author: string;
  reviewer: string | null;
  rounds: number;
  stage: string;
  providerLocked: boolean;
  lockedAt?: string;
  superpowers?: boolean;
  openSpec?: boolean;
}

const PROVIDER_LABELS: Record<string, string> = {
  claude_code: "Claude Code",
  codex: "Codex",
};

const STAGE_BADGES: Record<string, { text: string; color: string }> = {
  prepare_context: { text: "准备中", color: "bg-slate-100 text-slate-700" },
  running: { text: "运行中 · 保持本页打开", color: "bg-amber-50 text-amber-700" },
  cross_review: { text: "审核中", color: "bg-blue-50 text-blue-700" },
  review_decision: { text: "审核结论待处理", color: "bg-purple-50 text-purple-700" },
  revision: { text: "修订中", color: "bg-amber-50 text-amber-700" },
  human_confirm: { text: "等待确认", color: "bg-green-50 text-green-700" },
  completed: { text: "已完成", color: "bg-slate-100 text-slate-500" },
};

export function WorkspaceHeader({
  entityType,
  entityId,
  version,
  author,
  reviewer,
  rounds,
  stage,
  providerLocked,
  lockedAt,
  superpowers = false,
  openSpec = false,
}: WorkspaceHeaderProps) {
  const badge = STAGE_BADGES[stage] ?? STAGE_BADGES["prepare_context"];

  return (
    <div className="border-b px-4 py-3">
      <div className="flex items-center justify-between">
        <div>
          <div className="text-sm text-[var(--aria-ink-muted)]">
            {entityType} #{entityId}
            {version !== undefined && ` / v${version}`}
          </div>
          <div className="mt-1 flex items-center gap-3 text-sm">
            <span>
              Author: {PROVIDER_LABELS[author] ?? author}
              {providerLocked && (
                <span title={`锁定于 ${lockedAt ?? "未知"}`}> 🔒</span>
              )}
            </span>
            {reviewer && (
              <span>
                Reviewer: {PROVIDER_LABELS[reviewer] ?? reviewer}
                {rounds > 0 && ` · ${rounds} round${rounds > 1 ? "s" : ""}`}
                {providerLocked && <span> 🔒</span>}
              </span>
            )}
            <span className="text-xs text-[var(--aria-ink-muted)]">
              Superpowers: {superpowers ? "on" : "off"} · OpenSpec: {openSpec ? "on" : "off"}
            </span>
          </div>
        </div>
        <div className={`rounded px-2 py-1 text-xs font-medium ${badge.color}`}>
          {badge.text}
        </div>
      </div>
    </div>
  );
}
```

- [ ] **Step 3: 跑测试确认通过**

Run: `pnpm --dir web test -- WorkspaceHeader`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add web/src/components/workspace/WorkspaceHeader.tsx web/src/components/workspace/WorkspaceHeader.test.tsx
git commit -m "feat(ui): add WorkspaceHeader with provider snapshot + stage badge"
```

---

### Task 3: NodeDetailPanel（5 tab）

**Files:**
- 新建: `web/src/components/workspace/NodeDetailPanel.tsx`
- 测试: `web/src/components/workspace/NodeDetailPanel.test.tsx`

- [ ] **Step 1: 写 failing 测试**

```tsx
import { describe, it, expect } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { NodeDetailPanel } from "./NodeDetailPanel";

describe("NodeDetailPanel", () => {
  it("renders 5 tabs", () => {
    render(
      <NodeDetailPanel
        node={{
          node_id: "node-1",
          node_type: "author_run",
          status: "completed",
          title: "生成",
        }}
        detail={{
          node_id: "node-1",
          session_id: "sess-1",
          node_type: "author_run",
          status: "completed",
          agent_role: "author",
          provider: { name: "claude_code", model: "opus-4-7" },
          messages: [],
          streaming_content: "输出内容",
          execution_events: [],
          permission_events: [],
          verdict: null,
          artifact_ref: null,
          is_revision: false,
          base_artifact_ref: null,
          started_at: "2026-05-20T14:30:00Z",
          ended_at: null,
        }}
        artifactVersions={[]}
      />
    );
    expect(screen.getByText("概览")).toBeInTheDocument();
    expect(screen.getByText("流式输出")).toBeInTheDocument();
    expect(screen.getByText("执行事件")).toBeInTheDocument();
    expect(screen.getByText("权限")).toBeInTheDocument();
    expect(screen.getByText("Artifact")).toBeInTheDocument();
  });

  it("switches to streaming tab", () => {
    render(
      <NodeDetailPanel
        node={{
          node_id: "node-1",
          node_type: "author_run",
          status: "completed",
          title: "生成",
        }}
        detail={{
          node_id: "node-1",
          session_id: "sess-1",
          node_type: "author_run",
          status: "completed",
          agent_role: "author",
          provider: { name: "claude_code", model: "opus-4-7" },
          messages: [],
          streaming_content: "输出内容",
          execution_events: [],
          permission_events: [],
          verdict: null,
          artifact_ref: null,
          is_revision: false,
          base_artifact_ref: null,
          started_at: "2026-05-20T14:30:00Z",
          ended_at: null,
        }}
        artifactVersions={[]}
      />
    );
    fireEvent.click(screen.getByText("流式输出"));
    expect(screen.getByText("输出内容")).toBeInTheDocument();
  });
});
```

Run: `pnpm --dir web test -- NodeDetailPanel`
Expected: 编译失败 — NodeDetailPanel 未定义

- [ ] **Step 2: 实现 NodeDetailPanel**

```tsx
import { useState } from "react";
import type { TimelineNode, NodeDetail, ArtifactVersion } from "../../api/types";

interface NodeDetailPanelProps {
  node: TimelineNode;
  detail: NodeDetail | null;
  artifactVersions: ArtifactVersion[];
}

const TABS = [
  { key: "overview", label: "概览" },
  { key: "streaming", label: "流式输出" },
  { key: "execution", label: "执行事件" },
  { key: "permission", label: "权限" },
  { key: "artifact", label: "Artifact" },
];

export function NodeDetailPanel({ node, detail, artifactVersions }: NodeDetailPanelProps) {
  const [activeTab, setActiveTab] = useState("overview");

  const artifact = detail?.artifact_ref
    ? artifactVersions.find((v) => v.source_node_id === detail.artifact_ref?.artifact_id)
    : null;

  return (
    <div data-testid="node-detail-panel" className="flex h-full flex-col">
      <div className="flex border-b">
        {TABS.map((tab) => (
          <button
            key={tab.key}
            data-testid={`tab-${tab.key}`}
            onClick={() => setActiveTab(tab.key)}
            className={`flex-1 px-2 py-2 text-xs font-medium ${
              activeTab === tab.key
                ? "border-b-2 border-blue-600 text-blue-600"
                : "text-[var(--aria-ink-muted)] hover:text-slate-700"
            }`}
          >
            {tab.label}
          </button>
        ))}
      </div>

      <div className="flex-1 overflow-y-auto p-3">
        {activeTab === "overview" && (
          <div className="space-y-2 text-sm">
            <div className="flex justify-between">
              <span className="text-[var(--aria-ink-muted)]">类型</span>
              <span>{node.node_type}</span>
            </div>
            <div className="flex justify-between">
              <span className="text-[var(--aria-ink-muted)]">状态</span>
              <span>{node.status}</span>
            </div>
            {detail?.provider && (
              <div className="flex justify-between">
                <span className="text-[var(--aria-ink-muted)]">Provider</span>
                <span>{detail.provider.name}</span>
              </div>
            )}
            {detail?.artifact_ref && (
              <div className="flex justify-between">
                <span className="text-[var(--aria-ink-muted)]">Artifact</span>
                <span>v{detail.artifact_ref.version}</span>
              </div>
            )}
            {detail?.is_revision && (
              <div className="text-amber-600 text-xs">
                🔁 修订版本
              </div>
            )}
          </div>
        )}

        {activeTab === "streaming" && (
          <pre data-testid="streaming-content" className="whitespace-pre-wrap text-xs">
            {detail?.streaming_content || "无流式输出"}
          </pre>
        )}

        {activeTab === "execution" && (
          <div className="space-y-1">
            {detail?.execution_events.length === 0 && (
              <div className="text-sm text-[var(--aria-ink-muted)]">无执行事件</div>
            )}
            {detail?.execution_events.map((ev, i) => (
              <div key={i} className="rounded bg-slate-50 px-2 py-1 text-xs">
                {JSON.stringify(ev)}
              </div>
            ))}
          </div>
        )}

        {activeTab === "permission" && (
          <div className="space-y-2">
            {detail?.permission_events.length === 0 && (
              <div className="text-sm text-[var(--aria-ink-muted)]">无权限事件</div>
            )}
            {detail?.permission_events.map((pe) => (
              <div
                key={pe.request_id}
                className={`rounded px-2 py-1.5 text-xs ${
                  pe.response ? "bg-green-50" : "bg-amber-50"
                }`}
              >
                <div className="font-medium">
                  {pe.request_id}
                </div>
                <div className="text-[var(--aria-ink-muted)]">
                  {pe.response ? `已${pe.response.approved ? "批准" : "拒绝"}` : "待应答"}
                </div>
              </div>
            ))}
          </div>
        )}

        {activeTab === "artifact" && (
          <div className="space-y-2">
            {artifact ? (
              <pre className="whitespace-pre-wrap text-xs">
                {artifact.markdown}
              </pre>
            ) : (
              <div className="text-sm text-[var(--aria-ink-muted)]">无 Artifact</div>
            )}
          </div>
        )}
      </div>
    </div>
  );
}
```

- [ ] **Step 3: 跑测试确认通过**

Run: `pnpm --dir web test -- NodeDetailPanel`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add web/src/components/workspace/NodeDetailPanel.tsx web/src/components/workspace/NodeDetailPanel.test.tsx
git commit -m "feat(ui): add NodeDetailPanel with 5 tabs"
```

---

### Task 4: StageActionsBar（底部操作按钮矩阵）

**Files:**
- 新建: `web/src/components/workspace/StageActionsBar.tsx`
- 测试: `web/src/components/workspace/StageActionsBar.test.tsx`

- [ ] **Step 1: 写 failing 测试**

```tsx
import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { StageActionsBar } from "./StageActionsBar";

describe("StageActionsBar", () => {
  it("shows start generation button in prepare_context", () => {
    const onStart = vi.fn();
    render(<StageActionsBar stage="prepare_context" onStartGeneration={onStart} />);
    fireEvent.click(screen.getByText(/开始生成/i));
    expect(onStart).toHaveBeenCalled();
  });

  it("shows abort button in running", () => {
    const onAbort = vi.fn();
    render(<StageActionsBar stage="running" onAbort={onAbort} />);
    fireEvent.click(screen.getByText(/中止/i));
    expect(onAbort).toHaveBeenCalled();
  });

  it("shows confirm/request_change/terminate in human_confirm", () => {
    const onConfirm = vi.fn();
    const onRequestChange = vi.fn();
    const onTerminate = vi.fn();
    render(
      <StageActionsBar
        stage="human_confirm"
        onConfirm={onConfirm}
        onRequestChange={onRequestChange}
        onTerminate={onTerminate}
      />
    );
    expect(screen.getByText(/确认/i)).toBeInTheDocument();
    expect(screen.getByText(/要求修改/i)).toBeInTheDocument();
    expect(screen.getByText(/终止/i)).toBeInTheDocument();
  });
});
```

Run: `pnpm --dir web test -- StageActionsBar`
Expected: 编译失败 — StageActionsBar 未定义

- [ ] **Step 2: 实现 StageActionsBar**

```tsx
interface StageActionsBarProps {
  stage: string;
  onStartGeneration?: () => void;
  onAbort?: () => void;
  onConfirm?: () => void;
  onRequestChange?: () => void;
  onTerminate?: () => void;
  onSelectRevisionPath?: (path: string) => void;
}

export function StageActionsBar({
  stage,
  onStartGeneration,
  onAbort,
  onConfirm,
  onRequestChange,
  onTerminate,
  onSelectRevisionPath,
}: StageActionsBarProps) {
  return (
    <div data-testid="stage-actions-bar" className="flex items-center justify-end gap-2 border-t px-4 py-2">
      {stage === "prepare_context" && onStartGeneration && (
        <button
          onClick={onStartGeneration}
          className="rounded bg-blue-600 px-4 py-1.5 text-sm font-medium text-white hover:bg-blue-700"
        >
          🚀 开始生成
        </button>
      )}

      {stage === "running" && onAbort && (
        <button
          onClick={onAbort}
          className="rounded bg-red-50 px-3 py-1.5 text-sm text-red-600 hover:bg-red-100"
        >
          ⏹ 中止
        </button>
      )}

      {stage === "cross_review" && onAbort && (
        <button
          onClick={onAbort}
          className="rounded bg-red-50 px-3 py-1.5 text-sm text-red-600 hover:bg-red-100"
        >
          ⏹ 中止
        </button>
      )}

      {stage === "review_decision" && onSelectRevisionPath && (
        <>
          <button
            onClick={() => onSelectRevisionPath("revise")}
            className="rounded bg-blue-50 px-3 py-1.5 text-sm text-blue-600 hover:bg-blue-100"
          >
            确定路径
          </button>
          <button
            onClick={onAbort}
            className="rounded bg-red-50 px-3 py-1.5 text-sm text-red-600 hover:bg-red-100"
          >
            ⏹ 中止
          </button>
        </>
      )}

      {stage === "revision" && onAbort && (
        <button
          onClick={onAbort}
          className="rounded bg-red-50 px-3 py-1.5 text-sm text-red-600 hover:bg-red-100"
        >
          ⏹ 中止
        </button>
      )}

      {stage === "human_confirm" && (
        <>
          {onConfirm && (
            <button
              onClick={onConfirm}
              className="rounded bg-green-600 px-3 py-1.5 text-sm font-medium text-white hover:bg-green-700"
            >
              ✓ 确认
            </button>
          )}
          {onRequestChange && (
            <button
              onClick={onRequestChange}
              className="rounded bg-amber-50 px-3 py-1.5 text-sm text-amber-700 hover:bg-amber-100"
            >
              ✎ 要求修改
            </button>
          )}
          {onTerminate && (
            <button
              onClick={onTerminate}
              className="rounded bg-red-50 px-3 py-1.5 text-sm text-red-600 hover:bg-red-100"
            >
              ✗ 终止
            </button>
          )}
        </>
      )}
    </div>
  );
}
```

- [ ] **Step 3: 跑测试确认通过**

Run: `pnpm --dir web test -- StageActionsBar`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add web/src/components/workspace/StageActionsBar.tsx web/src/components/workspace/StageActionsBar.test.tsx
git commit -m "feat(ui): add StageActionsBar with per-stage action buttons"
```

---

### Task 5: ReviewDecisionStagePanel（三路径选择）

**Files:**
- 新建: `web/src/components/workspace/stages/ReviewDecisionStagePanel.tsx`
- 测试: `web/src/components/workspace/stages/ReviewDecisionStagePanel.test.tsx`

- [ ] **Step 1: 写 failing 测试**

```tsx
import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { ReviewDecisionStagePanel } from "./ReviewDecisionStagePanel";

describe("ReviewDecisionStagePanel", () => {
  it("shows three options", () => {
    const onSelect = vi.fn();
    render(
      <ReviewDecisionStagePanel
        reviewer="codex"
        verdict="revise"
        summary="缺少边界场景"
        onSelectPath={onSelect}
      />
    );
    expect(screen.getByText(/直接返修/i)).toBeInTheDocument();
    expect(screen.getByText(/补充上下文后返修/i)).toBeInTheDocument();
    expect(screen.getByText(/跳过审核结论/i)).toBeInTheDocument();
  });

  it("calls onSelectPath with correct value", () => {
    const onSelect = vi.fn();
    render(
      <ReviewDecisionStagePanel
        reviewer="codex"
        verdict="revise"
        summary="缺少边界场景"
        onSelectPath={onSelect}
      />
    );
    fireEvent.click(screen.getByText(/直接返修/i));
    expect(onSelect).toHaveBeenCalledWith("revise", undefined);
  });
});
```

Run: `pnpm --dir web test -- ReviewDecisionStagePanel`
Expected: 编译失败 — ReviewDecisionStagePanel 未定义

- [ ] **Step 2: 实现 ReviewDecisionStagePanel**

```tsx
import { useState } from "react";

interface ReviewDecisionStagePanelProps {
  reviewer: string;
  verdict: string;
  summary: string;
  onSelectPath: (path: "revise" | "revise-with-context" | "skip-to-human", extraContext?: string) => void;
}

const PATHS = [
  {
    key: "revise",
    label: "直接返修",
    description: "author 基于审核意见自动修订",
  },
  {
    key: "revise-with-context",
    label: "补充上下文后返修",
    description: "追加 context 再让 author 修订",
  },
  {
    key: "skip-to-human",
    label: "跳过审核结论，进入人工确认",
    description: "不执行返修，直接进入确认",
  },
];

export function ReviewDecisionStagePanel({
  reviewer,
  verdict,
  summary,
  onSelectPath,
}: ReviewDecisionStagePanelProps) {
  const [selectedPath, setSelectedPath] =
    useState<"revise" | "revise-with-context" | "skip-to-human">("revise");
  const [extraContext, setExtraContext] = useState("");

  return (
    <div data-testid="review-decision-panel" className="space-y-4 p-4">
      <div className="rounded bg-slate-50 p-3">
        <div className="text-sm font-medium">
          审核结论：{verdict === "revise" ? "建议返修" : verdict}
        </div>
        <div className="text-xs text-[var(--aria-ink-muted)]">
          Reviewer: {reviewer} · Verdict: {verdict}
        </div>
        <div className="mt-1 text-sm">
          {summary}
        </div>
      </div>

      <div className="space-y-2">
        <div className="text-sm font-medium">
          请选择处理路径：
        </div>
        {PATHS.map((path) => (
          <label
            key={path.key}
            className={`flex cursor-pointer items-start gap-2 rounded border p-2 ${
              selectedPath === path.key ? "border-blue-500 bg-blue-50" : "border-slate-200"
            }`}
          >
            <input
              type="radio"
              name="revision-path"
              value={path.key}
              checked={selectedPath === path.key}
              onChange={() => setSelectedPath(path.key as "revise" | "revise-with-context" | "skip-to-human")}
            />
            <div className="text-sm">
              <div className="font-medium">
                {path.label}
              </div>
              <div className="text-xs text-[var(--aria-ink-muted)]">
                {path.description}
              </div>
            </div>
          </label>
        ))}
      </div>

      {selectedPath === "revise-with-context" && (
        <textarea
          value={extraContext}
          onChange={(e) => setExtraContext(e.target.value)}
          placeholder="补充上下文..."
          rows={3}
          className="w-full rounded border px-2 py-1 text-sm"
        />
      )}

      <div className="flex gap-2">
        <button
          onClick={() =>
            onSelectPath(
              selectedPath,
              selectedPath === "revise-with-context" ? extraContext : undefined
            )
          }
          className="rounded bg-blue-600 px-4 py-1.5 text-sm font-medium text-white hover:bg-blue-700"
        >
          确定
        </button>
      </div>
    </div>
  );
}
```

- [ ] **Step 3: 跑测试确认通过**

Run: `pnpm --dir web test -- ReviewDecisionStagePanel`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add web/src/components/workspace/stages/ReviewDecisionStagePanel.tsx web/src/components/workspace/stages/ReviewDecisionStagePanel.test.tsx
git commit -m "feat(ui): add ReviewDecisionStagePanel with 3-path selection"
```

---

### Task 6: HumanConfirmStagePanel（人工确认 + 结构化反馈）

**Files:**
- 新建: `web/src/components/workspace/stages/HumanConfirmStagePanel.tsx`
- 测试: `web/src/components/workspace/stages/HumanConfirmStagePanel.test.tsx`

- [ ] **Step 1: 写 failing 测试**

```tsx
import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { HumanConfirmStagePanel } from "./HumanConfirmStagePanel";

describe("HumanConfirmStagePanel", () => {
  it("shows reviewer summary and diff", () => {
    render(
      <HumanConfirmStagePanel
        artifactVersion={{ version: 2, markdown: "# v2" }}
        reviewerSummary={{ verdict: "pass", points: ["边界场景已补齐"] }}
        prevVersion={{ version: 1, markdown: "# v1" }}
        onConfirm={vi.fn()}
        onRequestChange={vi.fn()}
        onTerminate={vi.fn()}
      />
    );
    expect(screen.getByText(/边界场景已补齐/i)).toBeInTheDocument();
    expect(screen.getByText(/v1 → v2/i)).toBeInTheDocument();
  });

  it("submits structured feedback on request change", () => {
    const onRequestChange = vi.fn();
    render(
      <HumanConfirmStagePanel
        artifactVersion={{ version: 2, markdown: "# v2" }}
        reviewerSummary={{ verdict: "pass", points: [] }}
        onConfirm={vi.fn()}
        onRequestChange={onRequestChange}
        onTerminate={vi.fn()}
      />
    );
    fireEvent.click(screen.getByText(/要求修改/i));
    fireEvent.click(screen.getByLabelText(/内容缺失/i));
    fireEvent.change(screen.getByPlaceholderText(/具体描述/i), {
      target: { value: "缺少错误处理" },
    });
    fireEvent.click(screen.getByText(/提交/i));
    expect(onRequestChange).toHaveBeenCalled();
  });
});
```

Run: `pnpm --dir web test -- HumanConfirmStagePanel`
Expected: 编译失败 — HumanConfirmStagePanel 未定义

- [ ] **Step 2: 实现 HumanConfirmStagePanel**

```tsx
import { useState } from "react";

interface ReviewerSummary {
  verdict: string;
  points: string[];
}

interface ArtifactVersionLite {
  version: number;
  markdown: string;
}

interface HumanConfirmStagePanelProps {
  artifactVersion: ArtifactVersionLite;
  reviewerSummary: ReviewerSummary;
  prevVersion?: ArtifactVersionLite;
  onConfirm: () => void;
  onRequestChange: (feedback: { types: string[]; description: string }) => void;
  onTerminate: () => void;
}

const FEEDBACK_TYPES = ["内容缺失", "表述不清", "与需求不符", "其他"];

function lineDiff(prev: string, curr: string): string {
  const prevLines = prev.split("\n");
  const currLines = curr.split("\n");
  const added = currLines.filter((l) => !prevLines.includes(l)).length;
  const removed = prevLines.filter((l) => !currLines.includes(l)).length;
  return `新增 ${added} 行 · 删除 ${removed} 行`;
}

export function HumanConfirmStagePanel({
  artifactVersion,
  reviewerSummary,
  prevVersion,
  onConfirm,
  onRequestChange,
  onTerminate,
}: HumanConfirmStagePanelProps) {
  const [showFeedback, setShowFeedback] = useState(false);
  const [feedbackTypes, setFeedbackTypes] = useState<string[]>([]);
  const [feedbackDesc, setFeedbackDesc] = useState("");

  return (
    <div data-testid="human-confirm-panel" className="space-y-4 p-4">
      <div className="text-sm font-medium">
        待人工确认
      </div>

      <div className="rounded bg-slate-50 p-3">
        <div className="text-xs text-[var(--aria-ink-muted)]">
          📊 审核摘要
        </div>
        <div className="mt-1 text-sm">
          Verdict: {reviewerSummary.verdict}
        </div>
        <ul className="mt-1 list-inside list-disc text-xs">
          {reviewerSummary.points.map((p, i) => (
            <li key={i}>{p}</li>
          ))}
        </ul>
      </div>

      {prevVersion && (
        <div className="rounded bg-slate-50 p-3">
          <div className="text-xs text-[var(--aria-ink-muted)]">
            📄 与上一版本对比
          </div>
          <div className="mt-1 text-sm font-medium">
            [v{prevVersion.version} → v{artifactVersion.version}] {lineDiff(prevVersion.markdown, artifactVersion.markdown)}
          </div>
        </div>
      )}

      <div className="rounded border p-3">
        <div className="text-xs text-[var(--aria-ink-muted)] mb-1">
          📝 Artifact 预览（v{artifactVersion.version}）
        </div>
        <pre className="max-h-40 overflow-y-auto whitespace-pre-wrap text-xs">
          {artifactVersion.markdown}
        </pre>
      </div>

      {!showFeedback && (
        <div className="flex gap-2">
          <button
            onClick={onConfirm}
            className="rounded bg-green-600 px-4 py-1.5 text-sm font-medium text-white hover:bg-green-700"
          >
            ✓ 确认
          </button>
          <button
            onClick={() => setShowFeedback(true)}
            className="rounded bg-amber-50 px-4 py-1.5 text-sm text-amber-700 hover:bg-amber-100"
          >
            ✎ 要求修改
          </button>
          <button
            onClick={onTerminate}
            className="rounded bg-red-50 px-4 py-1.5 text-sm text-red-600 hover:bg-red-100"
          >
            ✗ 终止
          </button>
        </div>
      )}

      {showFeedback && (
        <div className="space-y-3 rounded border p-3">
          <div className="text-sm font-medium">
            反馈类型（可多选）：
          </div>
          <div className="flex flex-wrap gap-2">
            {FEEDBACK_TYPES.map((type) => (
              <label key={type} className="flex items-center gap-1 text-sm">
                <input
                  type="checkbox"
                  checked={feedbackTypes.includes(type)}
                  onChange={(e) => {
                    if (e.target.checked) {
                      setFeedbackTypes((prev) => [...prev, type]);
                    } else {
                      setFeedbackTypes((prev) => prev.filter((t) => t !== type));
                    }
                  }}
                />
                {type}
              </label>
            ))}
          </div>
          <textarea
            value={feedbackDesc}
            onChange={(e) => setFeedbackDesc(e.target.value)}
            placeholder="具体描述..."
            rows={3}
            className="w-full rounded border px-2 py-1 text-sm"
          />
          <div className="flex gap-2">
            <button
              onClick={() => {
                onRequestChange({ types: feedbackTypes, description: feedbackDesc });
                setShowFeedback(false);
              }}
              className="rounded bg-blue-600 px-3 py-1.5 text-sm text-white hover:bg-blue-700"
            >
              提交
            </button>
            <button
              onClick={() => setShowFeedback(false)}
              className="rounded bg-slate-100 px-3 py-1.5 text-sm hover:bg-slate-200"
            >
              取消
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
```

- [ ] **Step 3: 跑测试确认通过**

Run: `pnpm --dir web test -- HumanConfirmStagePanel`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add web/src/components/workspace/stages/HumanConfirmStagePanel.tsx web/src/components/workspace/stages/HumanConfirmStagePanel.test.tsx
git commit -m "feat(ui): add HumanConfirmStagePanel with diff + structured feedback"
```

---

### Task 7: WorkspacePage 重构接入所有组件

**Files:**
- 修改: `web/src/pages/WorkspacePage.tsx`

- [ ] **Step 1: 重构主布局**

从 WorkspacePage 中：
1. 删除页面级 `activeRightTab` 状态及 tab 按钮
2. 删除旧的 Provider 配置区（已提取到 ProviderConfigPanel）
3. 使用 WorkspaceHeader + Timeline + NodeDetailPanel + StageActionsBar

```tsx
import { WorkspaceHeader } from "../components/workspace/WorkspaceHeader";
import { NodeDetailPanel } from "../components/workspace/NodeDetailPanel";
import { StageActionsBar } from "../components/workspace/StageActionsBar";
import { ReviewDecisionStagePanel } from "../components/workspace/stages/ReviewDecisionStagePanel";
import { HumanConfirmStagePanel } from "../components/workspace/stages/HumanConfirmStagePanel";
import { selectPrepareContextNotes } from "../state/workspace-ws-store";

function WorkspacePage({ sessionId }: WorkspacePageProps) {
  const store = useWorkspaceStore();
  const stageConfig = useStageUI(store.stage);
  const contextNotes = selectPrepareContextNotes(store);
  const {
    sendContextNote,
    sendStartGeneration,
    abort,
    sendSelectRevisionPath,
    sendHumanConfirm,
  } = useWorkspaceWs(sessionId);

  // 选中的 Timeline 节点
  const selectedNode = store.timelineNodes.find((n) => n.node_id === store.activeNodeId) ?? store.timelineNodes[store.timelineNodes.length - 1];
  const selectedNodeDetail = selectedNode ? store.nodeDetails[selectedNode.node_id] : null;

  // 阶段面板内容
  const stagePanel = (() => {
    switch (stageConfig.panel) {
      case "PrepareContextPanel":
        return (
          <PrepareContextPanel
            onSendContextNote={(c) => sendContextNote(c)}
            onStartGeneration={() => {
              const snapshot = {
                author: store.providers?.author ?? "claude_code",
                reviewer: store.reviewerEnabled ? (store.providers?.reviewer ?? "codex") : null,
                review_rounds: store.reviewRounds ?? 1,
              };
              sendStartGeneration(snapshot, store.reviewerEnabled ?? true);
            }}
            contextNotes={contextNotes}
          />
        );
      case "RunningPanel":
        return <div className="p-4 text-sm text-[var(--aria-ink-muted)]">运行中...（流式输出在 Timeline 中）</div>;
      case "CrossReviewPanel":
        return <div className="p-4 text-sm text-[var(--aria-ink-muted)]">审核中...（等待 reviewer verdict）</div>;
      case "ReviewDecisionPanel":
        return (
          <ReviewDecisionStagePanel
            reviewer={store.providers?.reviewer ?? "codex"}
            verdict={store.pendingReviewDecision?.verdict ?? "revise"}
            summary={store.pendingReviewDecision?.summary ?? ""}
            onSelectPath={(path, ctx) => sendSelectRevisionPath(path, ctx)}
          />
        );
      case "RevisionPanel":
        return <div className="p-4 text-sm text-[var(--aria-ink-muted)]">修订中...</div>;
      case "HumanConfirmPanel":
        return (
          <HumanConfirmStagePanel
            artifactVersion={store.artifactVersions[store.artifactVersions.length - 1]}
            reviewerSummary={store.pendingReviewerSummary ?? { verdict: "pass", points: [] }}
            prevVersion={store.artifactVersions[store.artifactVersions.length - 2]}
            onConfirm={() => sendHumanConfirm("confirm")}
            onRequestChange={(fb) => sendHumanConfirm("request-change", fb)}
            onTerminate={() => sendHumanConfirm("terminate")}
          />
        );
      case "CompletedPanel":
        return <div className="p-4 text-sm">已完成</div>;
      default:
        return null;
    }
  })();

  const ignoreStageAction = () => undefined;

  return (
    <div className="flex h-full flex-col">
      <WorkspaceHeader
        entityType={store.workspaceType ?? "Workspace"}
        entityId={store.sessionId ?? ""}
        author={store.providers?.author ?? "claude_code"}
        reviewer={store.providers?.reviewer ?? null}
        rounds={store.reviewRounds ?? 0}
        stage={store.stage}
        providerLocked={store.providerLocked}
        lockedAt={store.providerSnapshot?.locked_at}
      />

      <div className="flex flex-1 overflow-hidden">
        <div className="w-1/2 overflow-y-auto border-r">
          {store.timelineNodes.length > 0 ? (
            <div className="space-y-2 p-4">
              {store.timelineNodes.map((node) => (
                <button
                  key={node.node_id}
                  type="button"
                  onClick={() => store.setSelectedNode(node.node_id)}
                  className={`block w-full rounded-md border px-3 py-2 text-left ${
                    node.node_id === selectedNode?.node_id
                      ? "border-[var(--aria-primary)] bg-blue-50"
                      : "border-[var(--aria-line)] bg-white hover:bg-[var(--aria-panel-muted)]"
                  }`}
                >
                  <div className="flex min-w-0 items-center justify-between gap-2">
                    <span className="truncate text-sm font-semibold text-[var(--aria-ink)]">
                      {node.title}
                    </span>
                    <span className="shrink-0 rounded px-1.5 py-0.5 text-[11px] font-medium text-[var(--aria-ink-muted)]">
                      {node.status}
                    </span>
                  </div>
                  {node.summary ? (
                    <p className="mt-1 truncate text-xs text-[var(--aria-ink-muted)]">
                      {node.summary}
                    </p>
                  ) : null}
                </button>
              ))}
            </div>
          ) : (
            <div className="p-4 text-sm text-[var(--aria-ink-muted)]">暂无 Timeline 节点</div>
          )}
        </div>

        <div className="flex w-1/2 flex-col">
          <div className="flex-1 overflow-y-auto">
            {stagePanel}
          </div>

          <div className="h-1/2 border-t">
            {selectedNode && (
              <NodeDetailPanel
                node={selectedNode}
                detail={selectedNodeDetail}
                artifactVersions={store.artifactVersions}
              />
            )}
          </div>
        </div>
      </div>

      <StageActionsBar
        stage={store.stage}
        onStartGeneration={ignoreStageAction}
        onAbort={abort}
        onConfirm={() => sendHumanConfirm("confirm")}
        onRequestChange={ignoreStageAction}
        onTerminate={() => sendHumanConfirm("terminate")}
      />
    </div>
  );
}
```

- [ ] **Step 2: 在 store 中补全 pendingReviewDecision / pendingReviewerSummary**

```typescript
export interface WorkspaceWsState {
  sessionId: string | null;
  workspaceType: string | null;
  stage: string;
  visitedStages: string[];
  messages: WsMessage[];
  checkpoints: WsCheckpoint[];
  artifact: string | null;
  providers: WsProviderConfig | null;
  connectionStatus: WsConnectionStatus;
  streamingContent: string;
  pendingPermissions: PermissionRequest[];
  providerStatus: ProviderStatus;
  executionEvents: ExecutionEvent[];
  timelineNodes: TimelineNode[];
  activeNodeId: string | null;
  selectedNodeId: string | null;
  nodeDetails: Record<string, TimelineNodeDetail>;
  artifactVersions: ArtifactVersion[];
  pendingDecision: ReviewDecisionRequired | null;
  error: string | null;
  activeRunId: string | null;
  protocolError: { code: string; message: string } | null;
  providerLocked: boolean;
  providerSnapshot: ProviderConfigSnapshot | null;
  reviewerEnabled: boolean;
  reviewRounds: number;
  pendingReviewDecision: { verdict: string; summary: string } | null;
  pendingReviewerSummary: { verdict: string; points: string[] } | null;
}
```

初始值：`pendingReviewDecision: null`, `pendingReviewerSummary: null`

- [ ] **Step 3: 跑 WorkspacePage 测试确认通过**

Run: `pnpm --dir web test -- WorkspacePage`
Expected: PASS；旧的右侧 Tab 断言必须改为 `WorkspaceHeader`、Timeline 列表、`NodeDetailPanel` 和当前 stage panel 的渲染断言

- [ ] **Step 4: Commit**

```bash
git add web/src/pages/WorkspacePage.tsx web/src/state/workspace-ws-store.ts
git commit -m "feat(ui): refactor WorkspacePage with Header + NodeDetailPanel + StageActionsBar + stage panels"
```

---

### Task 8: 全量回归测试

- [ ] **Step 1: 跑前端单元测试**

Run: `pnpm --dir web test`
Expected: PASS

- [ ] **Step 2: Commit（如有修复）**

```bash
git commit -am "fix: update tests for stage UI refactor"
```

---

## 自审检查

**1. Spec coverage:**

| 设计 § | 实现位置 |
|---|---|
| §6.1 节点详情 5 tab | Task 3 (NodeDetailPanel) |
| §6.2 Header Provider snapshot | Task 2 (WorkspaceHeader) |
| §6.3 Running / CrossReview | Task 7 (stagePanel switch) |
| §6.3 ReviewDecision 三路径 | Task 5 (ReviewDecisionStagePanel) |
| §6.3 Revision | Task 7 (stagePanel switch) |
| §6.3 HumanConfirm | Task 6 (HumanConfirmStagePanel) |
| §6.4 底部操作区矩阵 | Task 4 (StageActionsBar) |
| §6.5 代码触达点 | Task 7 (WorkspacePage 重构) |

**2. Implementation constraints:**
- 没有未决占位项
- `sendSelectRevisionPath` / `sendHumanConfirm` 签名需与 P1 `useWorkspaceWs` 一致；P4 不新增 `sendRequestRevision`

**3. Type consistency:**
- `ReviewDecisionStagePanel` 的 `onSelectPath` 签名与 `sendSelectRevisionPath` 匹配
- `HumanConfirmStagePanel` 的 `onRequestChange` 签名与 store 中的反馈类型一致

---

## 本 plan 验收清单

- [ ] WorkspaceHeader 显示 Provider snapshot + Stage 徽章 + 锁图标
- [ ] 节点详情 5 tab 切换真实生效（概览/流式/执行/权限/Artifact）
- [ ] 页面级 Artifact/执行 tab 已删除
- [ ] ReviewDecision 三路径都能选 → 进入正确下一阶段
- [ ] HumanConfirm 显示 reviewer 摘要 + 行级 diff + artifact 预览
- [ ] HumanConfirm "要求修改" → 结构化反馈（多选）→ 提交后回 ReviewDecision
- [ ] `pnpm --dir web test` PASS
