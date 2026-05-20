# P3: 看板侧滑详情面板 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把看板卡片点击从"直接进全屏 Workspace"改为"侧滑详情面板（Drawer）"，解决 Story confirmed 后无法触发 Design 的问题，同时让看板上下文不丢失。

**Architecture:** 新增 LifecycleCardDrawer 组件（480px 固定宽，不灰化看板），看板状态新增 `focusedEntityId` 与 `isDrawerOpen`，URL query param `?focus=` 双向同步，卡片 onClick 改为打开 Drawer。Workspace 路由沿用当前 `/workbench/workspace/$sessionId`；Drawer 内"生成下一阶段"只创建实体和 PrepareContext session，然后把 Drawer 切到新实体，不自动打开 Workspace、不自动启动 Provider。

**Tech Stack:** React + TypeScript + Tailwind CSS + Zustand + @tanstack/react-router + vitest

**前置依赖:** 无（P3 完全独立，只动 lifecycle 看板端）

**后续 plan 消费点:**
- P7 E2E 测试消费看板侧滑导航路径（C1-C6 用例）

**文件结构总览:**

| 文件 | 操作 | 职责 |
|---|---|---|
| `web/src/components/lifecycle/LifecycleCardDrawer.tsx` | 新建 | 侧滑详情面板 |
| `web/src/components/lifecycle/LifecycleCardDrawer.test.tsx` | 新建 | 面板交互测试 |
| `web/src/state/lifecycle-workbench-store.ts` | 修改 | 新增 focusedEntityId / isDrawerOpen / queryParam 同步 |
| `web/src/components/lifecycle/IssueLifecycleWorkbench.tsx` | 修改 | 卡片 onClick → openDrawer |
| `web/src/components/lifecycle/LifecycleCard.tsx` | 修改 | 移除"打开 Workspace"按钮 |
| `web/src/api/client.ts` | 确认/复用 | 复用现有 generateStorySpecs / generateDesignSpecs / generateWorkItems API，只创建下一阶段实体和 Workspace session |

---

### Task 1: lifecycle-workbench-store 追加 Drawer 状态 + URL 同步

**Files:**
- 修改: `web/src/state/lifecycle-workbench-store.ts`
- 测试: `web/src/state/lifecycle-workbench-store.test.ts`

- [ ] **Step 1: 写 failing 测试**

```typescript
import { describe, it, expect } from "vitest";
import { useLifecycleWorkbenchStore } from "./lifecycle-workbench-store";

describe("drawer state", () => {
  it("opens drawer with entity id", () => {
    const store = useLifecycleWorkbenchStore.getState();
    store.openDrawer("story-12");
    expect(store.focusedEntityId).toBe("story-12");
    expect(store.isDrawerOpen).toBe(true);
  });

  it("closes drawer and clears focus", () => {
    const store = useLifecycleWorkbenchStore.getState();
    store.openDrawer("story-12");
    store.closeDrawer();
    expect(store.focusedEntityId).toBeNull();
    expect(store.isDrawerOpen).toBe(false);
  });
});
```

Run: `pnpm --filter web test -- lifecycle-workbench-store`
Expected: 失败 — openDrawer / closeDrawer 未定义

- [ ] **Step 2: 在 store 中追加状态**

```typescript
export interface LifecycleWorkbenchState {
  // ... 现有字段 ...
  focusedEntityId: string | null;
  isDrawerOpen: boolean;
}

export interface LifecycleWorkbenchActions {
  // ... 现有 actions ...
  openDrawer: (entityId: string) => void;
  closeDrawer: () => void;
}
```

实现：

```typescript
  focusedEntityId: null,
  isDrawerOpen: false,
  // ...
  openDrawer: (entityId) => set({ focusedEntityId: entityId, isDrawerOpen: true }),
  closeDrawer: () => set({ focusedEntityId: null, isDrawerOpen: false }),
```

- [ ] **Step 3: URL query param 双向同步**

在 `IssueLifecycleWorkbench.tsx` 中使用 `@tanstack/react-router` 的 `useSearch`：

```typescript
import { useSearch, useNavigate } from "@tanstack/react-router";

// 读取 URL
const search = useSearch({ from: "/workbench" });
const navigate = useNavigate({ from: "/workbench" });

// URL → store（初始化时）
useEffect(() => {
  if (search.focus && typeof search.focus === "string") {
    store.openDrawer(search.focus);
  }
}, [search.focus]);

// store → URL
useEffect(() => {
  if (store.isDrawerOpen && store.focusedEntityId) {
    navigate({ search: { focus: store.focusedEntityId } });
  } else {
    navigate({ search: {} });
  }
}, [store.isDrawerOpen, store.focusedEntityId]);
```

- [ ] **Step 4: 跑测试确认通过**

Run: `pnpm --filter web test -- lifecycle-workbench-store`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add web/src/state/lifecycle-workbench-store.ts web/src/state/lifecycle-workbench-store.test.ts
git commit -m "feat(store): add drawer state with URL sync"
```

---

### Task 2: LifecycleCardDrawer 组件

**Files:**
- 新建: `web/src/components/lifecycle/LifecycleCardDrawer.tsx`
- 测试: `web/src/components/lifecycle/LifecycleCardDrawer.test.tsx`

- [ ] **Step 1: 写 failing 测试**

```tsx
import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { LifecycleCardDrawer } from "./LifecycleCardDrawer";

describe("LifecycleCardDrawer", () => {
  it("renders entity info and action buttons", () => {
    render(
      <LifecycleCardDrawer
        entity={{
          id: "story-12",
          kind: "story_spec",
          title: "用户认证模块",
          status: "confirmed",
          version: 2,
          artifact_versions: [
            { version: 2, markdown: "# v2", generated_by: "claude-code", created_at: "2026-05-20T14:30:00Z", source_node_id: "node-1" },
          ],
        }}
        onClose={vi.fn()}
        onOpenWorkspace={vi.fn()}
        onGenerateNext={vi.fn()}
      />
    );
    expect(screen.getByText("用户认证模块")).toBeInTheDocument();
    expect(screen.getByText(/生成 Design Spec/i)).toBeInTheDocument();
  });

  it("calls onClose when close button clicked", () => {
    const onClose = vi.fn();
    render(
      <LifecycleCardDrawer
        entity={{ id: "story-12", kind: "story_spec", title: "测试", status: "confirmed" }}
        onClose={onClose}
        onOpenWorkspace={vi.fn()}
        onGenerateNext={vi.fn()}
      />
    );
    fireEvent.click(screen.getByLabelText(/关闭/i));
    expect(onClose).toHaveBeenCalled();
  });
});
```

Run: `pnpm --filter web test -- LifecycleCardDrawer`
Expected: 编译失败 — LifecycleCardDrawer 未定义

- [ ] **Step 2: 实现 LifecycleCardDrawer**

```tsx
import { useState } from "react";

interface ArtifactVersion {
  version: number;
  markdown: string;
  generated_by: string;
  reviewed_by?: string | null;
  review_verdict?: string | null;
  confirmed_by?: string | null;
  created_at: string;
  source_node_id: string;
}

interface DrawerEntity {
  id: string;
  kind: "issue" | "story_spec" | "design_spec" | "work_item";
  title: string;
  status: string;
  version?: number;
  artifact_versions?: ArtifactVersion[];
}

interface LifecycleCardDrawerProps {
  entity: DrawerEntity;
  onClose: () => void;
  onOpenWorkspace: () => void;
  onGenerateNext?: () => void;
}

export function LifecycleCardDrawer({
  entity,
  onClose,
  onOpenWorkspace,
  onGenerateNext,
}: LifecycleCardDrawerProps) {
  const [showAllVersions, setShowAllVersions] = useState(false);

  const statusLabel: Record<string, string> = {
    confirmed: "已确认",
    rejected: "已驳回",
    running: "生成中",
    open: "待处理",
  };

  const nextActionLabel: Record<string, string> = {
    story_spec: "生成 Design Spec",
    design_spec: "生成 Work Item",
  };

  return (
    <div className="flex h-full flex-col border-l bg-white">
      {/* 头部 */}
      <div className="flex items-center justify-between border-b px-4 py-3">
        <div>
          <span className="text-xs text-[var(--aria-ink-muted)] uppercase">{entity.kind}</span>
          <h2 className="text-base font-semibold">{entity.title}</h2>
          <div className="flex gap-2 text-xs text-[var(--aria-ink-muted)]">
            <span>#{entity.id}</span>
            <span className="rounded bg-slate-100 px-1.5 py-0.5">{statusLabel[entity.status] ?? entity.status}</span>
            {entity.version && <span>v{entity.version}</span>}
          </div>
        </div>
        <button onClick={onClose} aria-label="关闭" className="rounded p-1 hover:bg-slate-100">
          ✕
        </button>
      </div>

      {/* 版本历史 */}
      {entity.artifact_versions && entity.artifact_versions.length > 0 && (
        <div className="border-b px-4 py-3">
          <h3 className="mb-2 text-sm font-medium">版本历史</h3>
          <div className="space-y-2">
            {(showAllVersions ? entity.artifact_versions : entity.artifact_versions.slice(0, 3)).map((v) => (
              <div key={v.version} className="rounded bg-slate-50 px-2 py-1.5 text-xs">
                <div className="flex justify-between">
                  <span className="font-medium">v{v.version}</span>
                  <span className="text-[var(--aria-ink-muted)]">{v.created_at.slice(0, 10)}</span>
                </div>
                <div className="text-[var(--aria-ink-muted)]">
                  作者: {v.generated_by}
                  {v.reviewed_by && ` · 审核: ${v.reviewed_by}`}
                  {v.confirmed_by && ` · 确认: ${v.confirmed_by}`}
                </div>
              </div>
            ))}
          </div>
          {entity.artifact_versions.length > 3 && (
            <button
              onClick={() => setShowAllVersions((v) => !v)}
              className="mt-1 text-xs text-blue-600 hover:underline"
            >
              {showAllVersions ? "收起" : `查看全部 ${entity.artifact_versions.length} 个版本`}
            </button>
          )}
        </div>
      )}

      {/* Artifact 预览 */}
      {entity.artifact_versions && entity.artifact_versions[0] && (
        <div className="flex-1 overflow-y-auto px-4 py-3">
          <h3 className="mb-2 text-sm font-medium">最新版本预览</h3>
          <div className="prose prose-sm max-w-none">
            <pre className="whitespace-pre-wrap text-xs">
              {entity.artifact_versions[0].markdown.slice(0, 400)}
              {entity.artifact_versions[0].markdown.length > 400 && "..."}
            </pre>
          </div>
        </div>
      )}

      {/* 操作区 */}
      <div className="border-t px-4 py-3 space-y-2">
        <button
          onClick={onOpenWorkspace}
          className="w-full rounded bg-slate-800 py-2 text-sm font-medium text-white hover:bg-slate-900"
        >
          打开 Workspace
        </button>

        {entity.status === "confirmed" && nextActionLabel[entity.kind] && onGenerateNext && (
          <button
            onClick={onGenerateNext}
            className="w-full rounded border border-blue-600 py-2 text-sm font-medium text-blue-600 hover:bg-blue-50"
          >
            🚀 {nextActionLabel[entity.kind]}
          </button>
        )}
      </div>
    </div>
  );
}
```

- [ ] **Step 3: 跑测试确认通过**

Run: `pnpm --filter web test -- LifecycleCardDrawer`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add web/src/components/lifecycle/LifecycleCardDrawer.tsx web/src/components/lifecycle/LifecycleCardDrawer.test.tsx
git commit -m "feat(ui): add LifecycleCardDrawer with version history + artifact preview"
```

---

### Task 3: 改造 IssueLifecycleWorkbench 接入 Drawer

**Files:**
- 修改: `web/src/components/lifecycle/IssueLifecycleWorkbench.tsx`
- 修改: `web/src/components/lifecycle/LifecycleCard.tsx`

- [ ] **Step 1: 修改 IssueLifecycleWorkbench**

在 `IssueLifecycleWorkbench` 中：

```tsx
const store = useLifecycleWorkbenchStore();
const { focusedEntityId, isDrawerOpen, openDrawer, closeDrawer } = store;

// 卡片点击处理
function handleCardClick(card: LifecycleCardData) {
  openDrawer(card.id);
}

// 找到当前 focused entity 的完整数据
const focusedEntity = useMemo(() => {
  for (const lifecycle of lifecycles) {
    const allCards = [
      ...(lifecycle.issue ? [{ ...lifecycle.issue, kind: "issue" as const }] : []),
      ...(lifecycle.story_specs ?? []).map((s) => ({ ...s, kind: "story_spec" as const })),
      ...(lifecycle.design_specs ?? []).map((d) => ({ ...d, kind: "design_spec" as const })),
      ...(lifecycle.work_items ?? []).map((w) => ({ ...w, kind: "work_item" as const })),
    ];
    const found = allCards.find((c) => c.id === focusedEntityId);
    if (found) return found;
  }
  return null;
}, [lifecycles, focusedEntityId]);

// 渲染 Drawer
{isDrawerOpen && focusedEntity && (
  <div className="fixed right-0 top-0 z-50 h-full w-[480px] shadow-xl">
    <LifecycleCardDrawer
      entity={focusedEntity}
      onClose={closeDrawer}
      onOpenWorkspace={() => {
        const sessionId = workspaceSessionIdForEntity(focusedEntity);
        if (sessionId) {
          onOpenWorkspace(sessionId);
        }
        closeDrawer();
      }}
      onGenerateNext={
        focusedEntity.status === "confirmed" && ["story_spec", "design_spec"].includes(focusedEntity.kind)
          ? () => handleGenerateNext(focusedEntity.id, focusedEntity.kind)
          : undefined
      }
    />
  </div>
)}
```

- [ ] **Step 1.5: 实现 handleGenerateNext，严格按"只创建不执行"流程**

`handleGenerateNext` 只调用已有创建 API、刷新数据、切换 Drawer focus；不得调用 `onOpenWorkspace`，不得发送 `start_generation` 或旧 `message/run-next`。

```tsx
async function handleGenerateNext(entityId: string, kind: DrawerEntity["kind"]) {
  if (!selectedProjectId || !focusedEntity) {
    setError("缺少 Project 或生命周期实体");
    return;
  }

  if (kind === "story_spec") {
    const response = await generateDesignSpecs(selectedProjectId, focusedEntity.issueId, {
      title: defaultLaunchTitle({ target: "design", card: focusedEntity }),
      story_spec_ids: [entityId],
      design_kind: "frontend",
    });
    const nextId = response.design_specs[0]?.design_spec_id;
    await refresh(selectedProjectId);
    if (nextId) {
      openDrawer(nextId);
    }
    return;
  }

  if (kind === "design_spec") {
    const response = await generateWorkItems(selectedProjectId, focusedEntity.issueId, {
      title: defaultLaunchTitle({ target: "work_item", card: focusedEntity }),
      story_spec_ids: focusedEntity.raw.story_spec_ids,
      design_spec_ids: [entityId],
    });
    const nextId = response.work_items[0]?.work_item_id;
    await refresh(selectedProjectId);
    if (nextId) {
      openDrawer(nextId);
    }
    return;
  }

  setError("当前实体不支持生成下一阶段");
}
```

Drawer 切到新实体后显示二级 CTA：

```tsx
{focusedEntity.workspace_session_id && focusedEntity.status === "draft" ? (
  <button
    type="button"
    onClick={() => onOpenWorkspace(focusedEntity.workspace_session_id)}
    className="w-full rounded bg-slate-800 py-2 text-sm font-medium text-white hover:bg-slate-900"
  >
    打开 Workspace 配置 Provider 并开始生成
  </button>
) : null}
```

- [ ] **Step 2: 修改 LifecycleCard 移除"打开 Workspace"按钮**

在 `LifecycleCard.tsx` 中：

```tsx
// 删除或隐藏"打开 Workspace"按钮
// 原来：
// <button onClick={onOpenWorkspace} ...>打开</button>
// 改为：只保留 onSelect（打开 Drawer）
```

- [ ] **Step 3: 修复 handleLaunchWorkspace race**

```tsx
async function handleLaunchWorkspace(entityId: string) {
  await refresh(); // 确保数据最新
  navigate({ to: "/workbench/workspace/$sessionId", params: { sessionId: entityId } });
}
```

- [ ] **Step 4: 跑 IssueLifecycleWorkbench 测试**

Run: `pnpm --filter web test -- IssueLifecycleWorkbench`
Expected: 可能部分用例需要调整（onClick 行为变化），但核心通过

- [ ] **Step 5: Commit**

```bash
git add web/src/components/lifecycle/IssueLifecycleWorkbench.tsx web/src/components/lifecycle/LifecycleCard.tsx
git commit -m "feat(ui): integrate LifecycleCardDrawer into workbench"
```

---

### Task 4: 全量回归测试

- [ ] **Step 1: 跑前端单元测试**

Run: `pnpm --filter web test`
Expected: PASS

- [ ] **Step 2: Commit（如有修复）**

```bash
git commit -am "fix: adjust tests for drawer integration"
```

---

## 自审检查

**1. Spec coverage:**

| 设计 § | 实现位置 |
|---|---|
| §5.1 卡片点击 → Drawer | Task 3 (handleCardClick) |
| §5.2 Drawer 内容分区 | Task 2 (LifecycleCardDrawer) |
| §5.3 操作按钮矩阵 | Task 2 (onOpenWorkspace + onGenerateNext) |
| §5.4 生成下一阶段按钮 | Task 3 Step 1.5 (只创建下一阶段实体和 PrepareContext session，Drawer 切换到新实体，不自动打开 Workspace，不启动 Provider) |
| §5.6 Drawer URL / 路由 | Task 1 (URL 双向同步) |
| §5.7 焦点过滤 | Task 3 (卡片高亮可后续追加) |

**2. Placeholder scan:**
- 无 TBD/TODO
- `handleGenerateNext` 必须复用现有创建 API，但只创建下一阶段实体和 Workspace session；禁止调用 `onOpenWorkspace` 或触发 Provider

**3. Type consistency:**
- `DrawerEntity` 与 `LifecycleCardData` 兼容
- `ArtifactVersion` 与 api/types.ts 中的定义一致

---

## 本 plan 验收清单

- [ ] 卡片点击打开 Drawer（480px，右侧滑出）
- [ ] Drawer 关闭后 URL 清除 `?focus=`
- [ ] 直接访问 `?focus=story-12` 自动打开 Drawer
- [ ] Story confirmed 后 Drawer 显示"生成 Design Spec"按钮
- [ ] 点击"生成 Design Spec"只创建 Design 实体 + PrepareContext session，Drawer 切到 Design，不自动打开 Workspace，不启动 Provider
- [ ] Drawer 内"打开 Workspace"进入全屏 Workspace
- [ ] 看板不灰化，允许并行操作
- [ ] `pnpm --filter web test` PASS
