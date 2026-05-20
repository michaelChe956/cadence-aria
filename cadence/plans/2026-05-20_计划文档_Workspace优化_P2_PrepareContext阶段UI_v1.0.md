# P2: PrepareContext 阶段 UI 重构 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 PrepareContext 阶段的"统一输入框 → 触发 guess intent"改为"显式 context_note 输入 + 显式开始生成按钮 + Provider 常驻配置 + Reviewer 默认勾选"，让 Stage 切换前用户心智清晰。

**Architecture:** 从 WorkspacePage 中拆出 PrepareContextPanel（含 context_note 输入、Provider 常驻配置、Reviewer 推荐）、重构 Provider 配置区为常驻展开、新增 useStageUI hook 按 stage 决定右侧面板内容。

**Tech Stack:** React + TypeScript + Tailwind CSS + Zustand + vitest

**前置依赖:** P1（协议类型、sendContextNote / sendStartGeneration、provider_locked 事件、nodeDetails）

**后续 plan 消费点:**
- P4 消费 useStageUI hook，追加 Running/CrossReview/ReviewDecision/HumanConfirm 阶段面板

**文件结构总览:**

| 文件 | 操作 | 职责 |
|---|---|---|
| `web/src/components/workspace/PrepareContextPanel.tsx` | 新建 | PrepareContext 阶段专用面板 |
| `web/src/components/workspace/ProviderConfigPanel.tsx` | 新建 | Provider 常驻配置（author + reviewer + rounds + 旗标） |
| `web/src/hooks/useStageUI.ts` | 新建 | 按 stage 返回应渲染的子面板 |
| `web/src/pages/WorkspacePage.tsx` | 修改 | 拆出输入区、接入 useStageUI、移除旧输入逻辑 |
| `web/src/state/workspace-ws-store.ts` | 修改 | 追加 context_notes 列表状态 |
| `web/src/components/workspace/PrepareContextPanel.test.tsx` | 新建 | 面板交互测试 |

---

### Task 1: useStageUI hook

**Files:**
- 新建: `web/src/hooks/useStageUI.ts`
- 测试: `web/src/hooks/useStageUI.test.ts`

- [ ] **Step 1: 写 failing 测试**

```typescript
import { describe, it, expect } from "vitest";
import { useStageUI } from "./useStageUI";
import { renderHook } from "@testing-library/react";

describe("useStageUI", () => {
  it("returns PrepareContextPanel for prepare_context", () => {
    const { result } = renderHook(() => useStageUI("prepare_context"));
    expect(result.current.panel).toBe("PrepareContextPanel");
    expect(result.current.actions).toEqual(["start_generation"]);
  });

  it("returns RunningPanel for running", () => {
    const { result } = renderHook(() => useStageUI("running"));
    expect(result.current.panel).toBe("RunningPanel");
    expect(result.current.actions).toEqual(["abort"]);
  });

  it("returns HumanConfirmPanel for human_confirm", () => {
    const { result } = renderHook(() => useStageUI("human_confirm"));
    expect(result.current.panel).toBe("HumanConfirmPanel");
    expect(result.current.actions).toEqual(["confirm", "request_change", "terminate"]);
  });
});
```

Run: `pnpm --filter web test -- useStageUI`
Expected: 编译失败 — useStageUI 未定义

- [ ] **Step 2: 实现 useStageUI**

```typescript
export type StagePanel =
  | "PrepareContextPanel"
  | "RunningPanel"
  | "CrossReviewPanel"
  | "ReviewDecisionPanel"
  | "RevisionPanel"
  | "HumanConfirmPanel"
  | "CompletedPanel";

export type StageAction =
  | "start_generation"
  | "abort"
  | "confirm"
  | "request_change"
  | "terminate"
  | "select_revision_path";

export interface StageUIConfig {
  panel: StagePanel;
  actions: StageAction[];
  headerBadge: string;
  showContextInput: boolean;
  providerEditable: boolean;
}

const STAGE_CONFIG_MAP: Record<string, StageUIConfig> = {
  prepare_context: {
    panel: "PrepareContextPanel",
    actions: ["start_generation"],
    headerBadge: "准备中",
    showContextInput: true,
    providerEditable: true,
  },
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
};

export function useStageUI(stage: string): StageUIConfig {
  return STAGE_CONFIG_MAP[stage] ?? STAGE_CONFIG_MAP["prepare_context"];
}
```

- [ ] **Step 3: 跑测试确认通过**

Run: `pnpm --filter web test -- useStageUI`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add web/src/hooks/useStageUI.ts web/src/hooks/useStageUI.test.ts
git commit -m "feat(ui): add useStageUI hook with stage-to-panel mapping"
```

---

### Task 2: ProviderConfigPanel 组件（常驻展开）

**Files:**
- 新建: `web/src/components/workspace/ProviderConfigPanel.tsx`
- 测试: `web/src/components/workspace/ProviderConfigPanel.test.tsx`

- [ ] **Step 1: 写 failing 测试**

```tsx
import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { ProviderConfigPanel } from "./ProviderConfigPanel";

describe("ProviderConfigPanel", () => {
  it("renders author and reviewer selects", () => {
    render(
      <ProviderConfigPanel
        providers={{ author: "claude_code", reviewer: "codex" }}
        editable={true}
        onSelectProvider={vi.fn()}
        reviewerEnabled={true}
        onToggleReviewer={vi.fn()}
      />
    );
    expect(screen.getByLabelText(/Author/i)).toBeInTheDocument();
    expect(screen.getByLabelText(/Reviewer/i)).toBeInTheDocument();
    expect(screen.getByLabelText(/启用交叉审核/i)).toBeChecked();
  });

  it("shows warning when reviewer unchecked", () => {
    render(
      <ProviderConfigPanel
        providers={{ author: "claude_code", reviewer: "codex" }}
        editable={true}
        onSelectProvider={vi.fn()}
        reviewerEnabled={false}
        onToggleReviewer={vi.fn()}
      />
    );
    expect(screen.getByText(/未启用交叉审核可能降低 artifact 质量/i)).toBeInTheDocument();
  });

  it("disables selects when not editable", () => {
    render(
      <ProviderConfigPanel
        providers={{ author: "claude_code", reviewer: "codex" }}
        editable={false}
        onSelectProvider={vi.fn()}
        reviewerEnabled={true}
        onToggleReviewer={vi.fn()}
      />
    );
    expect(screen.getByLabelText(/Author/i)).toBeDisabled();
  });
});
```

Run: `pnpm --filter web test -- ProviderConfigPanel`
Expected: 编译失败 — ProviderConfigPanel 未定义

- [ ] **Step 2: 实现 ProviderConfigPanel**

```tsx
import { useState } from "react";
import { WsProviderConfig } from "../../api/types";

interface ProviderConfigPanelProps {
  providers: WsProviderConfig | null;
  editable: boolean;
  onSelectProvider: (role: "author" | "reviewer", provider: string) => void;
  reviewerEnabled: boolean;
  onToggleReviewer: (enabled: boolean) => void;
  rounds?: number;
  onChangeRounds?: (rounds: number) => void;
}

const PROVIDER_OPTIONS = [
  { value: "claude_code", label: "Claude Code" },
  { value: "codex", label: "Codex" },
];

export function ProviderConfigPanel({
  providers,
  editable,
  onSelectProvider,
  reviewerEnabled,
  onToggleReviewer,
  rounds = 1,
  onChangeRounds,
}: ProviderConfigPanelProps) {
  const [showAdvanced, setShowAdvanced] = useState(false);

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between">
        <span className="text-sm font-medium">Provider 配置</span>
        {editable && (
          <span className="text-xs text-amber-600">可编辑</span>
        )}
        {!editable && (
          <span className="text-xs text-slate-400"></svg> 已锁定</span>
        )}
      </div>

      <div className="space-y-2">
        <label className="flex items-center gap-2 text-sm">
          <span className="w-14 text-[var(--aria-ink-muted)]">Author</span>
          <select
            value={providers?.author ?? "claude_code"}
            onChange={(e) => onSelectProvider("author", e.target.value)}
            disabled={!editable}
            className="flex-1 rounded border px-2 py-1 text-sm disabled:bg-slate-100"
          >
            {PROVIDER_OPTIONS.map((p) => (
              <option key={p.value} value={p.value}>
                {p.label}
              </option>
            ))}
          </select>
        </label>

        <div className="flex items-center gap-2">
          <input
            type="checkbox"
            id="reviewer-toggle"
            checked={reviewerEnabled}
            onChange={(e) => onToggleReviewer(e.target.checked)}
            disabled={!editable}
          />
          <label htmlFor="reviewer-toggle" className="text-sm">
            启用交叉审核
          </label>
        </div>

        {reviewerEnabled && (
          <label className="flex items-center gap-2 text-sm">
            <span className="w-14 text-[var(--aria-ink-muted)]">Reviewer</span>
            <select
              value={providers?.reviewer ?? "codex"}
              onChange={(e) => onSelectProvider("reviewer", e.target.value)}
              disabled={!editable}
              className="flex-1 rounded border px-2 py-1 text-sm disabled:bg-slate-100"
            >
              {PROVIDER_OPTIONS.map((p) => (
                <option key={p.value} value={p.value}>
                  {p.label}
                </option>
              ))}
            </select>
          </label>
        )}

        {!reviewerEnabled && editable && (
          <div className="rounded bg-amber-50 px-2 py-1.5 text-xs text-amber-700">
            ⚠️ 未启用交叉审核可能降低 artifact 质量
          </div>
        )}
      </div>

      <button
        onClick={() => setShowAdvanced((v) => !v)}
        className="text-xs text-[var(--aria-ink-muted)] hover:underline"
      >
        {showAdvanced ? "收起高级配置 ▲" : "高级配置 ▼"}
      </button>

      {showAdvanced && (
        <div className="space-y-2 rounded border p-2">
          <label className="flex items-center gap-2 text-sm">
            <span className="w-20">审核轮次</span>
            <input
              type="number"
              min={1}
              max={3}
              value={rounds}
              onChange={(e) => onChangeRounds?.(parseInt(e.target.value, 10))}
              disabled={!editable}
              className="w-16 rounded border px-2 py-1 text-sm disabled:bg-slate-100"
            />
          </label>
        </div>
      )}
    </div>
  );
}
```

- [ ] **Step 3: 跑测试确认通过**

Run: `pnpm --filter web test -- ProviderConfigPanel`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add web/src/components/workspace/ProviderConfigPanel.tsx web/src/components/workspace/ProviderConfigPanel.test.tsx
git commit -m "feat(ui): add ProviderConfigPanel with reviewer toggle + warning"
```

---

### Task 3: PrepareContextPanel 组件

**Files:**
- 新建: `web/src/components/workspace/PrepareContextPanel.tsx`
- 测试: `web/src/components/workspace/PrepareContextPanel.test.tsx`

- [ ] **Step 1: 写 failing 测试**

```tsx
import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { PrepareContextPanel } from "./PrepareContextPanel";

describe("PrepareContextPanel", () => {
  it("sends context_note on submit", () => {
    const sendContextNote = vi.fn();
    render(
      <PrepareContextPanel
        onSendContextNote={sendContextNote}
        onStartGeneration={vi.fn()}
        contextNotes={[]}
      />
    );

    const textarea = screen.getByPlaceholderText(/补充上下文/i);
    fireEvent.change(textarea, { target: { value: "需要支持空查询参数" } });
    fireEvent.click(screen.getByText(/发送上下文/i));

    expect(sendContextNote).toHaveBeenCalledWith("需要支持空查询参数");
  });

  it("sends start_generation on button click", () => {
    const onStart = vi.fn();
    render(
      <PrepareContextPanel
        onSendContextNote={vi.fn()}
        onStartGeneration={onStart}
        contextNotes={[]}
      />
    );

    fireEvent.click(screen.getByText(/开始生成/i));
    expect(onStart).toHaveBeenCalled();
  });

  it("shows context notes list", () => {
    render(
      <PrepareContextPanel
        onSendContextNote={vi.fn()}
        onStartGeneration={vi.fn()}
        contextNotes={["第一条", "第二条"]}
      />
    );

    expect(screen.getByText(/第一条/i)).toBeInTheDocument();
    expect(screen.getByText(/第二条/i)).toBeInTheDocument();
  });
});
```

Run: `pnpm --filter web test -- PrepareContextPanel`
Expected: 编译失败 — PrepareContextPanel 未定义

- [ ] **Step 2: 实现 PrepareContextPanel**

```tsx
import { useState } from "react";

interface PrepareContextPanelProps {
  onSendContextNote: (content: string) => void;
  onStartGeneration: () => void;
  contextNotes: string[];
  disabled?: boolean;
}

export function PrepareContextPanel({
  onSendContextNote,
  onStartGeneration,
  contextNotes,
  disabled = false,
}: PrepareContextPanelProps) {
  const [input, setInput] = useState("");

  function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (input.trim()) {
      onSendContextNote(input.trim());
      setInput("");
    }
  }

  const expanded = contextNotes.length <= 3;

  return (
    <div className="flex flex-col gap-4">
      <div className="space-y-2">
        <h3 className="text-sm font-medium">已补充上下文 {contextNotes.length} 条</h3>
        {contextNotes.length > 0 && (
          <ul className={`space-y-1 ${expanded ? "" : "max-h-32 overflow-y-auto"}`}>
            {contextNotes.map((note, i) => (
              <li
                key={i}
                className="rounded bg-slate-50 px-2 py-1 text-sm text-slate-700"
              >
                {note}
              </li>
            ))}
          </ul>
        )}
      </div>

      <form onSubmit={handleSubmit} className="flex gap-2">
        <textarea
          value={input}
          onChange={(e) => setInput(e.target.value)}
          placeholder="补充上下文（回车换行，点击发送按钮提交）"
          disabled={disabled}
          rows={3}
          className="flex-1 resize-y rounded border px-2 py-1 text-sm disabled:bg-slate-100"
        />
        <button
          type="submit"
          disabled={disabled || !input.trim()}
          className="self-end rounded bg-slate-200 px-3 py-1.5 text-sm hover:bg-slate-300 disabled:opacity-50"
        >
          发送上下文
        </button>
      </form>

      <button
        onClick={onStartGeneration}
        disabled={disabled}
        className="w-full rounded bg-blue-600 py-2 text-sm font-medium text-white hover:bg-blue-700 disabled:opacity-50"
      >
        🚀 开始生成
      </button>
    </div>
  );
}
```

- [ ] **Step 3: 跑测试确认通过**

Run: `pnpm --filter web test -- PrepareContextPanel`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add web/src/components/workspace/PrepareContextPanel.tsx web/src/components/workspace/PrepareContextPanel.test.tsx
git commit -m "feat(ui): add PrepareContextPanel with context_note input + start generation CTA"
```

---

### Task 4: 前端 store 追加 context_notes 状态

**Files:**
- 修改: `web/src/state/workspace-ws-store.ts`
- 测试: `web/src/state/workspace-ws-store.test.ts`

- [ ] **Step 1: 写 failing 测试**

```typescript
  it("adds context_note to list", () => {
    const store = useWorkspaceStore.getState();
    store.addContextNote("第一条");
    store.addContextNote("第二条");
    expect(useWorkspaceStore.getState().contextNotes).toEqual(["第一条", "第二条"]);
  });

  it("clears context notes on start_generation", () => {
    const store = useWorkspaceStore.getState();
    store.addContextNote("第一条");
    store.clearContextNotes();
    expect(useWorkspaceStore.getState().contextNotes).toEqual([]);
  });
```

Run: `pnpm --filter web test -- workspace-ws-store`
Expected: 失败 — addContextNote / clearContextNotes 未定义

- [ ] **Step 2: 在 store 中追加**

```typescript
export interface WorkspaceWsState {
  // ... 现有字段 ...
  contextNotes: string[];
}

export interface WorkspaceWsActions {
  // ... 现有 actions ...
  addContextNote: (content: string) => void;
  clearContextNotes: () => void;
}
```

实现：

```typescript
  addContextNote: (content) =>
    set((prev) => ({ contextNotes: [...prev.contextNotes, content] })),
  clearContextNotes: () => set({ contextNotes: [] }),
```

初始值：`contextNotes: []`

- [ ] **Step 3: 跑测试确认通过**

Run: `pnpm --filter web test -- workspace-ws-store`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add web/src/state/workspace-ws-store.ts web/src/state/workspace-ws-store.test.ts
git commit -m "feat(store): add contextNotes state for PrepareContext stage"
```

---

### Task 5: WorkspacePage 重构接入

**Files:**
- 修改: `web/src/pages/WorkspacePage.tsx`

- [ ] **Step 1: 读当前 WorkspacePage.tsx，理解输入区**

```bash
grep -n "handleSubmit\|input\|textarea\|startGeneration\|showProviderPanel" web/src/pages/WorkspacePage.tsx
```

现有逻辑：
- `handleSubmit` (line 120): 通过 `sendMessage` 发送统一输入 → 后端 guess intent
- `startGeneration` (line 155): 调用 `sendMessage("开始生成")`
- `showProviderPanel` toggle (line 110): Provider 配置折叠面板

- [ ] **Step 2: 重构输入区**

从 WorkspacePage 中删除旧输入区，改为根据 stage 渲染不同面板：

```tsx
import { useStageUI } from "../../hooks/useStageUI";
import { PrepareContextPanel } from "../../components/workspace/PrepareContextPanel";
import { ProviderConfigPanel } from "../../components/workspace/ProviderConfigPanel";

// ...
function WorkspacePage({ sessionId, ... }: WorkspacePageProps) {
  const { sendContextNote, sendStartGeneration, selectProvider } = useWorkspaceWs(sessionId);
  const store = useWorkspaceStore();
  const stageConfig = useStageUI(store.stage);
  // ...

  // 主区右侧面板
  const rightPanel = (() => {
    switch (stageConfig.panel) {
      case "PrepareContextPanel":
        return (
          <div className="space-y-4">
            <ProviderConfigPanel
              providers={store.providers}
              editable={stageConfig.providerEditable}
              onSelectProvider={selectProvider}
              reviewerEnabled={store.reviewerEnabled ?? true}
              onToggleReviewer={(enabled) => useWorkspaceStore.setState({ reviewerEnabled: enabled })}
            />
            <PrepareContextPanel
              onSendContextNote={(content) => {
                sendContextNote(content);
                store.addContextNote(content);
              }}
              onStartGeneration={() => {
                // 构造 Provider snapshot
                const snapshot: ProviderConfigSnapshot = {
                  author: store.providers?.author ?? "claude_code",
                  reviewer: store.reviewerEnabled ? (store.providers?.reviewer ?? "codex") : null,
                  review_rounds: store.reviewRounds ?? 1,
                };
                sendStartGeneration(snapshot, store.reviewerEnabled ?? true);
                store.clearContextNotes();
              }}
              contextNotes={store.contextNotes}
            />
          </div>
        );
      // P4 会追加其他 case
      default:
        return <div>待实现面板: {stageConfig.panel}</div>;
    }
  })();

  // ...
}
```

- [ ] **Step 3: 在 store 中追加 reviewerEnabled / reviewRounds**

```typescript
export interface WorkspaceWsState {
  // ...
  reviewerEnabled: boolean;
  reviewRounds: number;
}
```

初始值：`reviewerEnabled: true`, `reviewRounds: 1`

- [ ] **Step 4: 跑 WorkspacePage 测试确认不破坏**

Run: `pnpm --filter web test -- WorkspacePage`
Expected: 可能部分用例需要调整（sendMessage 改为 sendContextNote），但核心流程通过

- [ ] **Step 5: Commit**

```bash
git add web/src/pages/WorkspacePage.tsx web/src/state/workspace-ws-store.ts
git commit -m "feat(ui): integrate PrepareContextPanel + ProviderConfigPanel into WorkspacePage"
```

---

### Task 6: 全量回归测试

- [ ] **Step 1: 跑前端单元测试**

Run: `pnpm --filter web test`
Expected: PASS（如有 user_message 相关旧用例失败，更新为 context_note）

- [ ] **Step 2: Commit（如有修复）**

```bash
git commit -am "fix: update tests for PrepareContext UI refactor"
```

---

## 自审检查

**1. Spec coverage:**

| 设计 § | 实现位置 |
|---|---|
| §3.1 整体布局（PrepareContext 模式） | Task 5 (WorkspacePage 接入) |
| §4.2 输入区（回车换行、发送按钮） | Task 3 (PrepareContextPanel) |
| §4.2 Provider 配置区（常驻展开） | Task 2 (ProviderConfigPanel) |
| §4.2 Reviewer 默认勾选 + 取消提示 | Task 2 (reviewerEnabled + onToggleReviewer) |
| §4.2 开始生成按钮（唯一入口） | Task 3 (onStartGeneration) |
| §4.3 Timeline 增强（context_note 节点） | Task 7 (P1) + store contextNotes |
| §4.4 状态机视角 | Task 1 (useStageUI) |

**2. Placeholder scan:**
- 没有 TBD/TODO
- `store.reviewerEnabled` / `store.reviewRounds` 需要在 store 中定义
- `ProviderConfigSnapshot` 类型需要从 api/types.ts 导入

**3. Type consistency:**
- `sendStartGeneration` 签名与 P1 一致：(snapshot, reviewerEnabled)
- `ProviderConfigPanel` 的 `onSelectProvider` 签名与现有 `selectProvider` 兼容

---

## 本 plan 验收清单

- [ ] PrepareContext 阶段显示 ProviderConfigPanel（常驻展开）+ PrepareContextPanel
- [ ] 发送 context_note 后 Timeline 追加节点，Provider 未启动
- [ ] 点击"开始生成"后发送 start_generation，Provider 锁定，阶段切 Running
- [ ] Reviewer 默认勾选，取消时显示警告
- [ ] Running 阶段 ProviderConfigPanel 锁定，输入区隐藏
- [ ] `pnpm --filter web test` PASS
