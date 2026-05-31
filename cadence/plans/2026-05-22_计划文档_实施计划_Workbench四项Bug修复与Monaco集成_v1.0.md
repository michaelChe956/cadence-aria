# Workbench 四项 Bug 修复与 Monaco 集成 - 实施计划

## 概述

基于设计文档 `cadence/designs/2026-05-22_技术方案_Workbench四项Bug修复与Monaco集成_v1.0.md`，在 worktree 分支 `product-workbench-issue-lifecycle` 中实施。

## 前置条件

- 工作目录：`/Users/michaelche/Documents/git-folder/github-folder/cadence-aria/.worktrees/product-workbench-issue-lifecycle`
- 分支：`product-workbench-issue-lifecycle`
- 测试代码库：`/Users/michaelche/Documents/git-folder/github-folder/naruto`

## 实施阶段

### Phase 1：Monaco Editor 基础设施（共享组件）

**目标**：引入 Monaco 依赖，创建两个可复用的封装组件。

#### Step 1.1：安装依赖

```bash
cd web && pnpm add @monaco-editor/react monaco-editor
```

#### Step 1.2：创建 `web/src/components/shared/MonacoViewer.tsx`

```typescript
import { lazy, Suspense } from "react";

const Editor = lazy(() =>
  import("@monaco-editor/react").then((mod) => ({ default: mod.Editor }))
);

interface MonacoViewerProps {
  value: string;
  language?: string;
  height?: string;
}

export function MonacoViewer({ value, language = "markdown", height = "300px" }: MonacoViewerProps) {
  return (
    <Suspense fallback={<ViewerSkeleton height={height} />}>
      <Editor
        height={height}
        language={language}
        value={value}
        options={{
          readOnly: true,
          minimap: { enabled: false },
          wordWrap: "on",
          lineNumbers: "on",
          scrollBeyondLastLine: false,
          folding: true,
          renderLineHighlight: "none",
          overviewRulerLanes: 0,
          hideCursorInOverviewRuler: true,
          contextmenu: false,
          domReadOnly: true,
        }}
        theme="vs"
      />
    </Suspense>
  );
}

function ViewerSkeleton({ height }: { height: string }) {
  return (
    <div
      style={{ height }}
      className="animate-pulse rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)]"
    />
  );
}
```

#### Step 1.3：创建 `web/src/components/shared/MonacoDiffViewer.tsx`

```typescript
import { lazy, Suspense } from "react";

const DiffEditor = lazy(() =>
  import("@monaco-editor/react").then((mod) => ({ default: mod.DiffEditor }))
);

interface MonacoDiffViewerProps {
  original: string;
  modified: string;
  language?: string;
  height?: string;
  sideBySide?: boolean;
}

export function MonacoDiffViewer({
  original,
  modified,
  language = "markdown",
  height = "400px",
  sideBySide = true,
}: MonacoDiffViewerProps) {
  return (
    <Suspense fallback={<DiffSkeleton height={height} />}>
      <DiffEditor
        height={height}
        language={language}
        original={original}
        modified={modified}
        options={{
          readOnly: true,
          renderSideBySide: sideBySide,
          minimap: { enabled: false },
          scrollBeyondLastLine: false,
          renderOverviewRuler: false,
          contextmenu: false,
          domReadOnly: true,
        }}
        theme="vs"
      />
    </Suspense>
  );
}

function DiffSkeleton({ height }: { height: string }) {
  return (
    <div
      style={{ height }}
      className="animate-pulse rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)]"
    />
  );
}
```

#### Step 1.4：单元测试

创建 `web/src/components/shared/__tests__/MonacoViewer.test.tsx` 和 `MonacoDiffViewer.test.tsx`：
- 验证组件渲染不报错
- 验证 props 正确传递
- 验证骨架屏在加载时显示

---

### Phase 2：问题 1 - Artifact Diff 修复

**目标**：替换 `ArtifactPane.tsx` 中的手写 diff 和 markdown 预览为 Monaco 组件。

#### Step 2.1：重构 `ArtifactPane.tsx`

改动点：
1. 删除 `lineDiff()` 函数（第 150-159 行）
2. 删除 `MarkdownPreview` 组件和 `renderBlock` 函数（第 110-140 行）
3. 导入 `MonacoDiffViewer` 和 `MonacoViewer`
4. Diff 模式（第 96-103 行）替换为：
   ```tsx
   {showDiff && selected && previous ? (
     <MonacoDiffViewer
       original={previous.markdown}
       modified={selected.markdown}
       language="markdown"
       height="100%"
     />
   ) : (
     <MonacoViewer value={markdown} language="markdown" height="100%" />
   )}
   ```
5. 调整外层容器使 Monaco 能填满可用高度（`flex-1 min-h-0`）

#### Step 2.2：验证

- 启动 dev server，打开 workspace 页面
- 切换 artifact 版本，确认 Monaco 正确渲染 markdown
- 点击"显示 Diff"，确认 DiffEditor 正确展示两个版本的差异
- 切换 inline/side-by-side 模式（可选，后续增加按钮）

---

### Phase 3：问题 2 - 人工确认逻辑修复

**目标**：GatePromptEntry 支持 resolved 状态，确保流程闭环。

#### Step 3.1：扩展 `ChatEntry` 类型

文件：`web/src/state/chat-entries.ts`

```typescript
export interface ChatEntry {
  id: string;
  type: ChatEntryType;
  role: ChatEntryRole;
  content: string;
  timestamp: string;
  node_id?: string;
  metadata?: Record<string, unknown>;
  resolved?: boolean;
  resolution?: "confirm" | "request-change" | "terminate";
}
```

#### Step 3.2：修改 `GatePromptEntry.tsx`

```typescript
export function GatePromptEntry({
  entry,
  onDecision,
}: {
  entry: ChatEntry;
  onDecision?: (decision: "confirm" | "terminate") => void;
}) {
  const summary = summaryFromEntry(entry);
  const isResolved = entry.resolved === true;

  return (
    <ChatEntryContainer
      role="system"
      title="人工确认"
      className="border-slate-200 bg-slate-50"
      testId="gate-prompt-entry"
    >
      <div className="space-y-3">
        <div className="text-sm text-[var(--aria-ink)]">{entry.content}</div>
        {summary ? <div className="text-xs text-[var(--aria-ink-muted)]">{summary}</div> : null}
        {isResolved ? (
          <ResolutionBadge resolution={entry.resolution} />
        ) : onDecision ? (
          <div className="flex flex-wrap justify-end gap-2">
            {/* 确认 + 终止按钮（保持现有实现） */}
          </div>
        ) : null}
      </div>
    </ChatEntryContainer>
  );
}

function ResolutionBadge({ resolution }: { resolution?: string }) {
  if (resolution === "confirm") {
    return <span className="inline-flex items-center rounded-md bg-emerald-50 px-2 py-1 text-xs font-semibold text-emerald-700 ring-1 ring-emerald-200">已确认</span>;
  }
  if (resolution === "request-change") {
    return <span className="inline-flex items-center rounded-md bg-amber-50 px-2 py-1 text-xs font-semibold text-amber-700 ring-1 ring-amber-200">已要求修改</span>;
  }
  if (resolution === "terminate") {
    return <span className="inline-flex items-center rounded-md bg-red-50 px-2 py-1 text-xs font-semibold text-red-700 ring-1 ring-red-200">已终止</span>;
  }
  return null;
}
```

#### Step 3.3：修改 WebSocket store 处理逻辑

文件：`web/src/state/workspace-ws-store.ts`

在 `appendChatEntry` 或处理 `human_confirm` 响应的地方，增加逻辑：
- 当发送 `human_confirm` 消息时，找到最近一个 `type === "gate_prompt"` 且 `resolved !== true` 的 entry，标记为 `resolved = true`，设置 `resolution`

需要新增一个 action：
```typescript
resolveGateEntry: (resolution: "confirm" | "request-change" | "terminate") => void;
```

实现：
```typescript
resolveGateEntry: (resolution) =>
  set((prev) => {
    const entries = [...prev.chatEntries];
    for (let i = entries.length - 1; i >= 0; i--) {
      if (entries[i].type === "gate_prompt" && !entries[i].resolved) {
        entries[i] = { ...entries[i], resolved: true, resolution };
        break;
      }
    }
    return { chatEntries: entries };
  }),
```

#### Step 3.4：在 `ChatInputBar` 或消息发送处调用 `resolveGateEntry`

当用户通过输入框发送修改意见（触发 `human_confirm { decision: "request-change" }`）时，同步调用 `resolveGateEntry("request-change")`。

#### Step 3.5：单元测试

- GatePromptEntry resolved=false 时显示按钮
- GatePromptEntry resolved=true + resolution="confirm" 时显示绿色标签
- GatePromptEntry resolved=true + resolution="request-change" 时显示橙色标签
- GatePromptEntry resolved=true + resolution="terminate" 时显示红色标签
- resolveGateEntry action 正确标记最近的未 resolved gate entry

---

### Phase 4：问题 3 - Issue 抽屉展示具体信息

**目标**：当选中 issue 时，抽屉展示 issue 描述、关联产物、元信息。

#### Step 4.1：扩展 `DrawerEntity` 类型

文件：`web/src/components/lifecycle/LifecycleCardDrawer.tsx`

```typescript
export interface DrawerEntity {
  id: string;
  kind: DrawerEntityKind;
  title: string;
  status: string;
  version: number | null;
  artifactVersions?: ArtifactVersion[];
  // 新增字段
  description?: string;
  artifacts?: ProductIssueArtifact[];
  phase?: string;
  createdAt?: string;
}
```

#### Step 4.2：修改 `LifecycleCardDrawer` 内容区

当 `entity.kind === "issue"` 时，渲染：

```tsx
{entity.kind === "issue" ? (
  <>
    {entity.description ? (
      <section className="border-b border-[var(--aria-line)] px-4 py-3">
        <h3 className="mb-2 text-sm font-semibold text-[var(--aria-ink)]">Issue 描述</h3>
        <MonacoViewer value={entity.description} language="markdown" height="200px" />
      </section>
    ) : null}
    {entity.artifacts && entity.artifacts.length > 0 ? (
      <section className="border-b border-[var(--aria-line)] px-4 py-3">
        <h3 className="mb-2 text-sm font-semibold text-[var(--aria-ink)]">关联产物</h3>
        <div className="space-y-2">
          {entity.artifacts.map((artifact) => (
            <div key={artifact.artifact_ref} className="rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-2 py-2 text-xs">
              <div className="flex items-center justify-between">
                <span className="font-semibold text-[var(--aria-ink)]">{artifact.artifact_kind}</span>
                <span className="text-[var(--aria-ink-muted)]">{artifact.stage}</span>
              </div>
              <div className="mt-1 text-[var(--aria-ink-muted)]">{artifact.summary}</div>
            </div>
          ))}
        </div>
      </section>
    ) : null}
    {entity.phase || entity.createdAt ? (
      <section className="px-4 py-3">
        <h3 className="mb-2 text-sm font-semibold text-[var(--aria-ink)]">元信息</h3>
        <div className="space-y-1 text-xs text-[var(--aria-ink-muted)]">
          {entity.phase ? <div>阶段: {entity.phase}</div> : null}
          {entity.createdAt ? <div>创建时间: {entity.createdAt.slice(0, 10)}</div> : null}
        </div>
      </section>
    ) : null}
  </>
) : /* 现有的版本历史 + 预览逻辑 */ null}
```

#### Step 4.3：修改数据传递

文件：`web/src/components/lifecycle/IssueLifecycleWorkbench.tsx`

`LifecycleCardDrawer` 的 `entity` prop 实际接收 `LifecycleCard` 类型。`LifecycleCard` 的 issue variant 已有 `raw: ProductIssue`。

方案：修改 `LifecycleCardDrawer` 的 props 接收 `LifecycleCard` 而非 `DrawerEntity`，或者在传入前构建 `DrawerEntity`。

推荐：在 `IssueLifecycleWorkbench.tsx` 中增加一个 `toDrawerEntity(card: LifecycleCard): DrawerEntity` 转换函数：

```typescript
function toDrawerEntity(card: LifecycleCard): DrawerEntity {
  const base = {
    id: card.id,
    kind: card.kind,
    title: card.title,
    status: card.status,
    version: card.version,
  };

  if (card.kind === "issue") {
    return {
      ...base,
      description: card.raw.description ?? undefined,
      artifacts: card.raw.artifacts,
      phase: card.raw.phase,
      createdAt: card.raw.created_at,
    };
  }

  if (card.kind === "story_spec" || card.kind === "design_spec") {
    return {
      ...base,
      artifactVersions: card.artifactVersions,
    };
  }

  return base;
}
```

在第 521 行传入：`entity={toDrawerEntity(focusedEntity)}`

---

### Phase 5：问题 4 - Story Spec 抽屉版本切换 + Monaco 展示

**目标**：版本历史可点击切换，内容用 Monaco 展示，支持版本对比。

#### Step 5.1：增加版本选择状态

文件：`web/src/components/lifecycle/LifecycleCardDrawer.tsx`

```typescript
const [selectedVersionIndex, setSelectedVersionIndex] = useState(0);
const selectedArtifact = versions[selectedVersionIndex] ?? null;
```

#### Step 5.2：版本列表项增加点击交互

```tsx
{visibleVersions.map((version, index) => (
  <button
    type="button"
    key={`${version.version}-${version.source_node_id}`}
    onClick={() => setSelectedVersionIndex(index)}
    className={`w-full rounded-md border px-2 py-2 text-left text-xs transition-colors ${
      index === selectedVersionIndex
        ? "border-[var(--aria-primary)] bg-[var(--aria-primary)]/5"
        : "border-[var(--aria-line)] bg-[var(--aria-panel-muted)] hover:border-[var(--aria-primary)]/50"
    }`}
  >
    {/* 版本信息内容保持不变 */}
  </button>
))}
```

#### Step 5.3：替换预览区为 Monaco

```tsx
{selectedArtifact ? (
  <section className="px-4 py-3">
    <div className="mb-2 flex items-center gap-2 text-sm font-semibold text-[var(--aria-ink)]">
      <FileText className="h-4 w-4 text-[var(--aria-primary)]" />
      版本 v{selectedArtifact.version} 预览
    </div>
    <MonacoViewer
      value={selectedArtifact.markdown}
      language="markdown"
      height="320px"
    />
  </section>
) : null}
```

#### Step 5.4：版本对比 Diff（可选增强）

当选中非最新版本时，显示"与最新版本对比"按钮：

```tsx
const [showVersionDiff, setShowVersionDiff] = useState(false);
const canShowDiff = selectedVersionIndex > 0 && versions.length > 1;

{canShowDiff ? (
  <button
    type="button"
    onClick={() => setShowVersionDiff((v) => !v)}
    className="mt-2 text-xs font-semibold text-[var(--aria-primary)] hover:underline"
  >
    {showVersionDiff ? "隐藏对比" : "与最新版本对比"}
  </button>
) : null}

{showVersionDiff && canShowDiff ? (
  <MonacoDiffViewer
    original={selectedArtifact.markdown}
    modified={versions[0].markdown}
    language="markdown"
    height="320px"
  />
) : null}
```

#### Step 5.5：删除 `previewMarkdown()` 函数

不再需要 400 字符截断。

---

### Phase 6：集成验证与 E2E 测试

#### Step 6.1：启动 dev server 验证

```bash
cd web && pnpm dev
```

验证清单：
- [ ] Artifact 面板 Monaco 渲染正常
- [ ] Artifact Diff 展示正确（side-by-side）
- [ ] 人工确认按钮在 resolved 后消失，显示状态标签
- [ ] 新的人工确认卡片在修改轮次后出现
- [ ] Issue 抽屉展示描述和关联产物
- [ ] Story Spec 抽屉版本可切换
- [ ] Story Spec 内容用 Monaco 展示

#### Step 6.2：E2E 测试

使用测试场景（爬楼梯问题，代码库 `/Users/michaelche/Documents/git-folder/github-folder/naruto`）：
1. 创建 issue → 验证 issue 抽屉展示描述
2. 生成 story spec → 验证版本切换和 Monaco 展示
3. 进入 workspace → 触发人工确认 → request-change → 验证旧卡片 resolved + 新卡片出现
4. 确认后验证 artifact diff 展示

---

## 执行顺序与依赖关系

```
Phase 1 (基础设施)
  ├── Phase 2 (Artifact Diff) ← 依赖 Phase 1
  ├── Phase 4 (Issue 抽屉) ← 依赖 Phase 1
  └── Phase 5 (Story Spec 抽屉) ← 依赖 Phase 1
Phase 3 (人工确认) ← 独立，无依赖

Phase 6 (集成验证) ← 依赖 Phase 1-5 全部完成
```

Phase 2/4/5 可以并行执行（都依赖 Phase 1 但互不依赖）。Phase 3 完全独立。

## 预估工作量

| Phase | 预估时间 | 复杂度 |
|-------|---------|--------|
| Phase 1 | 15 min | 低 |
| Phase 2 | 10 min | 低 |
| Phase 3 | 25 min | 中 |
| Phase 4 | 15 min | 低 |
| Phase 5 | 20 min | 中 |
| Phase 6 | 30 min | 中 |
| **总计** | **~2 小时** | |
