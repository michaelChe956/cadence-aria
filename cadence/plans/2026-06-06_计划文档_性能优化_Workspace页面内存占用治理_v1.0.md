# Workspace 页面内存占用治理 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在不削减 Workspace 流式输出、历史查看、Provider Prompt、Execution Output、Artifact 版本与 Diff 功能的前提下，将大型 Workspace 页面内存占用从十 GB 级降到可控范围。

**Architecture:** 将 Workspace 页面从“全量详情传输 + 全量文本复制 + 全量 DOM/Markdown 渲染”改为“轻量 session state + 大文本引用化 + 按需详情加载 + 虚拟化列表 + 流式节流渲染”。后端继续保留现有 `.aria` 持久化格式，优先新增只读摘要/API 与前端缓存策略，避免一次性迁移历史数据。

**Tech Stack:** Rust 1.95/Axum/Cargo、React/Vite/TypeScript/Zustand、Vitest/Testing Library、Playwright E2E、Monaco、marked。

---

## 背景与问题证据

当前 `workspace_session_0003` 页面在浏览器中可达到约 `15G` 内存占用。只读排查结果显示，磁盘持久化数据本身并不大：当前 `.aria` 总量约 `7.3M`，`workspace_session_0003` timeline detail 总量约 `4.6M`，最大单个 node detail 约 `704K`。因此主要问题不是后端数据绝对体积，而是前端运行时重复复制、重复解析、重复渲染造成内存放大。

主要代码路径：

- `src/product/workspace_engine.rs:2789` 的 `build_session_state()` 会加载并返回完整 `timeline_node_details`。
- `src/web/workspace_ws_types.rs:100` 的 `SessionState` 包含 `artifact`、`artifact_versions`、`timeline_node_details`。
- `web/src/state/workspace-ws-store.ts` 同时保存 `nodeDetails`、`artifactVersions`、`chatEntries`。
- `web/src/state/workspace-ws-store.ts` 的 `buildChatEntries()` 会把 `prompt`、`streaming_content/messages`、`execution_events.output`、`artifact.markdown` 再复制进 `chatEntries`。
- `web/src/components/chat-workspace/entries/ProviderStreamEntry.tsx` 每次 render 都对完整内容执行 `normalizeProviderContent()` 和 `marked.lexer()`。
- `web/src/components/chat-workspace/ChatEntryList.tsx` 对所有历史 entry 全量 map 渲染，没有虚拟列表。

本计划遵循 `cadence/project-rules/workspace-artifact-bug-triage.md`：Workspace 产物链路涉及 Story Spec、Design Spec、Work Item 共用路径，所有共享层修复必须覆盖三种 workspace type，除非明确说明不适用原因。

## 目标与非目标

目标：

- 当前 active provider 输出仍保持肉眼实时流式效果。
- 历史 author/reviewer/revision 输出仍可完整查看。
- Provider Prompt、Execution Output、Artifact 全版本和 Diff 仍可完整查看。
- Timeline 节点定位仍可用。
- 刷新和重连后仍能恢复 Workspace 历史。
- 打开大型 Workspace 的初始内存和 DOM 数量受控。

非目标：

- 不改变 `.aria` 历史持久化 schema 作为本计划前置要求。
- 不删除历史内容，不裁剪最终内容。
- 不改变 provider 运行逻辑和审核/确认业务语义。
- 不把真实 provider 输出转成仅服务端渲染的静态页面。

## 文件结构与职责

后端：

- Modify: `src/web/workspace_ws_types.rs`。新增轻量 `SessionState` 字段类型、Node/Artifact summary DTO、按需内容响应 DTO。
- Modify: `src/product/workspace_engine.rs`。调整 `build_session_state()` 生成摘要而不是完整详情；保留可选兼容路径；增加 summary 构造辅助函数。
- Modify: `src/product/lifecycle_store.rs`。新增读取单个 node detail、prompt、event output、artifact version markdown 的只读方法或复用现有方法封装。
- Modify: `src/web/app.rs`。在 `build_web_router()` 注册 Workspace 按需读取 API。
- Modify: `src/web/handlers.rs`。新增 `GET /api/workspace-sessions/{session_id}/...` 只读 handler。
- Test: `tests/it_core/workspace_ws_integration.rs` 或新增 workspace API integration test。覆盖轻量 session state 与按需读取。

前端状态与 API：

- Modify: `web/src/api/types.ts`。新增 `WorkspaceContentRef`、summary DTO、按需内容响应类型。
- Modify: `web/src/state/chat-entries.ts`。扩展 `ChatEntry` 为 preview/ref 模型，禁止大文本放入 metadata。
- Modify: `web/src/state/workspace-ws-store.ts`。重构 `buildChatEntries()`、stream buffer、detail/artifact cache、selector。
- Modify/Create: `web/src/api/workspace-content.ts`。封装按需加载 node detail、prompt、event output、artifact version markdown。
- Test: `web/src/state/workspace-ws-store.test.ts`。覆盖去重副本、ref entry、stream buffer。

前端渲染：

- Modify: `web/src/components/chat-workspace/ChatEntryList.tsx`。引入虚拟列表，保留 `scrollToEntry()`。
- Modify: `web/src/components/chat-workspace/MessageGroupView.tsx`。支持 content ref、展开全文、active stream 轻量渲染。
- Modify: `web/src/components/chat-workspace/entries/ProviderStreamEntry.tsx`。将 Markdown 渲染改为 memo、完成后渲染、超长折叠/按需。
- Modify: `web/src/components/chat-workspace/InlineEventRow.tsx`。输出折叠时不持有完整 output，展开时按需加载。
- Modify: `web/src/components/chat-workspace/ArtifactPane.tsx`。Artifact 正文按需加载，Monaco 只保留当前 selected/previous。
- Test: `web/src/components/chat-workspace/*.test.tsx`、`web/src/pages/ChatWorkspacePage.test.tsx`。

E2E：

- Modify/Create: `web/e2e/issue-lifecycle-workspace.spec.ts` 或新增 `web/e2e/workspace-memory.spec.ts`。使用 fixture 验证大型 Workspace 页面功能不退化。

---

### Task 1: 建立大文本不进 ChatEntry metadata 的回归测试

**Files:**
- Modify: `web/src/state/workspace-ws-store.test.ts`
- Read: `web/src/state/workspace-ws-store.ts`
- Read: `web/src/state/chat-entries.ts`

- [ ] **Step 1: 写失败测试，证明 `artifact.markdown` 不应复制进 `chatEntries.metadata`**

在 `web/src/state/workspace-ws-store.test.ts` 增加测试。测试构造一个 artifact version，调用 `setSessionState()` 与 `rebuildChatEntries()`，断言 `artifact_update` entry 只保留版本引用，不包含完整 markdown。

```ts
import { beforeEach, describe, expect, it } from "vitest";
import { useWorkspaceStore } from "./workspace-ws-store";

describe("workspace chat entry memory discipline", () => {
  beforeEach(() => {
    useWorkspaceStore.getState().reset();
  });

  it("does not copy artifact markdown into chat entry metadata", () => {
    const hugeMarkdown = "# Artifact\n" + "content\n".repeat(10_000);

    useWorkspaceStore.getState().setSessionState({
      session_id: "workspace_session_memory",
      workspace_type: "story",
      stage: "human_confirm",
      superpowers_enabled: true,
      openspec_enabled: true,
      messages: [],
      checkpoints: [],
      artifact: hugeMarkdown,
      providers: { author: "codex", reviewer: "claude_code" },
      timeline_nodes: [
        {
          node_id: "timeline_node_001",
          node_type: "author_run",
          agent: "codex",
          stage: "running",
          status: "completed",
          title: "Story Spec 生成",
          started_at: "2026-06-06T00:00:00Z",
          completed_at: "2026-06-06T00:00:01Z",
          provider_config_snapshot: {
            author: "codex",
            reviewer: "claude_code",
            review_rounds: 1,
          },
        },
      ],
      active_node_id: null,
      artifact_versions: [
        {
          version: 1,
          markdown: hugeMarkdown,
          generated_by: "codex",
          reviewed_by: null,
          review_verdict: null,
          confirmed_by: null,
          is_current: true,
          created_at: "2026-06-06T00:00:01Z",
          source_node_id: "timeline_node_001",
        },
      ],
      timeline_node_details: {},
      active_run_id: null,
    });
    useWorkspaceStore.getState().rebuildChatEntries();

    const artifactEntry = useWorkspaceStore
      .getState()
      .chatEntries.find((entry) => entry.type === "artifact_update");

    expect(artifactEntry).toBeDefined();
    expect(artifactEntry?.metadata?.markdown).toBeUndefined();
    expect(JSON.stringify(artifactEntry)).not.toContain(hugeMarkdown.slice(0, 100));
  });
});
```

- [ ] **Step 2: 运行测试确认失败**

Run: `pnpm test -- workspace-ws-store.test.ts`

Workdir: `web/`

Expected: FAIL，原因是当前 `artifact_update` entry 的 `metadata.markdown` 保存了完整 markdown。

- [ ] **Step 3: 写失败测试，证明 Provider Prompt 不应复制进 ChatEntry metadata**

在同一 describe 中增加测试。

```ts
it("does not copy provider prompt into chat entry metadata output", () => {
  const hugePrompt = "[system]\n" + "prompt line\n".repeat(10_000);

  useWorkspaceStore.getState().setSessionState({
    session_id: "workspace_session_memory",
    workspace_type: "design",
    stage: "author_confirm",
    superpowers_enabled: true,
    openspec_enabled: true,
    messages: [],
    checkpoints: [],
    artifact: null,
    providers: { author: "codex", reviewer: "claude_code" },
    timeline_nodes: [
      {
        node_id: "timeline_node_001",
        node_type: "author_run",
        agent: "codex",
        stage: "running",
        status: "completed",
        title: "Design Spec 生成",
        started_at: "2026-06-06T00:00:00Z",
        completed_at: "2026-06-06T00:00:01Z",
        provider_config_snapshot: {
          author: "codex",
          reviewer: "claude_code",
          review_rounds: 1,
        },
      },
    ],
    active_node_id: null,
    artifact_versions: [],
    timeline_node_details: {
      timeline_node_001: {
        node_id: "timeline_node_001",
        session_id: "workspace_session_memory",
        node_type: "author_run",
        status: "completed",
        agent_role: "author",
        provider: { name: "codex", model: "codex" },
        prompt: hugePrompt,
        messages: [],
        streaming_content: "生成完成",
        execution_events: [],
        permission_events: [],
        verdict: null,
        artifact_ref: null,
        is_revision: false,
        base_artifact_ref: null,
        started_at: "2026-06-06T00:00:00Z",
        ended_at: "2026-06-06T00:00:01Z",
      },
    },
    active_run_id: null,
  });
  useWorkspaceStore.getState().rebuildChatEntries();

  const promptEntry = useWorkspaceStore
    .getState()
    .chatEntries.find((entry) => entry.id === "timeline_node_001:provider-prompt");

  expect(promptEntry).toBeDefined();
  expect(promptEntry?.metadata?.output).toBeUndefined();
  expect(JSON.stringify(promptEntry)).not.toContain(hugePrompt.slice(0, 100));
});
```

- [ ] **Step 4: 运行测试确认失败**

Run: `pnpm test -- workspace-ws-store.test.ts`

Workdir: `web/`

Expected: FAIL，原因是 Provider Prompt 仍进入 `metadata.output`。

---

### Task 2: 引入 ChatEntry 内容引用模型并移除大文本副本

**Files:**
- Modify: `web/src/state/chat-entries.ts`
- Modify: `web/src/state/workspace-ws-store.ts`
- Test: `web/src/state/workspace-ws-store.test.ts`

- [ ] **Step 1: 在 `ChatEntry` 中增加内容引用字段**

修改 `web/src/state/chat-entries.ts`：

```ts
export type WorkspaceContentRef =
  | { kind: "node_stream"; nodeId: string }
  | { kind: "provider_prompt"; nodeId: string }
  | { kind: "execution_output"; nodeId: string; eventId: string }
  | { kind: "artifact_version"; version: number; sourceNodeId?: string };

export interface ChatEntry {
  id: string;
  type: ChatEntryType;
  role: ChatEntryRole;
  content: string;
  timestamp: string;
  node_id?: string;
  content_ref?: WorkspaceContentRef;
  content_size?: number;
  has_full_content?: boolean;
  metadata?: Record<string, unknown>;
  resolved?: boolean;
  resolution?: ChatEntryResolution;
}
```

- [ ] **Step 2: 修改 Provider Prompt entry 为 preview + ref**

在 `web/src/state/workspace-ws-store.ts` 的 `buildChatEntries()` 中，将 provider prompt entry 的 metadata output 替换为引用。

目标结构：

```ts
entries.push({
  id: chatEntryId(node.node_id, "provider-prompt"),
  type: "execution_event",
  role,
  content: `${providerPromptContent(node.title)} · ${formatContentSize(prompt.length)}`,
  timestamp: detail.started_at || node.started_at,
  node_id: node.node_id,
  content_ref: { kind: "provider_prompt", nodeId: node.node_id },
  content_size: prompt.length,
  has_full_content: true,
  metadata: {
    event_id: `${node.node_id}_prompt`,
    node_id: node.node_id,
    agent: provider,
    kind: "output",
    status: "started",
    title: "Provider Prompt",
    detail: "发送给 Workspace provider 的完整提示词",
    command: null,
    cwd: null,
    exit_code: null,
    ...(provider ? { provider } : {}),
  },
});
```

新增辅助函数：

```ts
function formatContentSize(chars: number) {
  if (chars < 1024) {
    return `${chars} 字符`;
  }
  return `约 ${Math.ceil(chars / 1024)}KB`;
}
```

- [ ] **Step 3: 修改 artifact_update entry 为 version ref**

在 artifact entry 构造处移除 `markdown: artifact.markdown`，改为：

```ts
entries.push({
  id: chatEntryId(node.node_id, `artifact-${artifact.version}`),
  type: "artifact_update",
  role: "system",
  content: `产物已更新 -> v${artifact.version}`,
  timestamp: artifact.created_at,
  node_id: node.node_id,
  content_ref: {
    kind: "artifact_version",
    version: artifact.version,
    sourceNodeId: artifact.source_node_id,
  },
  content_size: artifact.markdown.length,
  has_full_content: true,
  metadata: {
    version: artifact.version,
    generated_by: artifact.generated_by,
    reviewed_by: artifact.reviewed_by ?? null,
    review_verdict: artifact.review_verdict ?? null,
    confirmed_by: artifact.confirmed_by ?? null,
    source_node_id: artifact.source_node_id,
  },
});
```

- [ ] **Step 4: 修改 `artifact_update` WebSocket 增量处理避免复制 markdown 到 chat entry**

在 `web/src/hooks/useWorkspaceWs.ts` 的 `artifact_update` case 中，将 `metadata.markdown` 移除，保留 `content_ref`。

```ts
store.appendChatEntry({
  id: chatEntryId("artifact_update", String(msg.version as number | undefined)),
  type: "artifact_update",
  role: "system",
  content: `产物已更新 -> v${msg.version as number | undefined}`,
  timestamp: new Date().toISOString(),
  content_ref:
    typeof msg.version === "number"
      ? { kind: "artifact_version", version: msg.version }
      : undefined,
  content_size: typeof msg.markdown === "string" ? msg.markdown.length : undefined,
  has_full_content: true,
  metadata: {
    version: msg.version as number | undefined,
    diff: (msg as { diff?: string | null }).diff ?? null,
  },
});
```

- [ ] **Step 5: 运行状态测试**

Run: `pnpm test -- workspace-ws-store.test.ts`

Workdir: `web/`

Expected: PASS，包括 Task 1 新增测试。

---

### Task 3: 流式输出节流缓冲，保留实时效果

**Files:**
- Modify: `web/src/state/workspace-ws-store.ts`
- Modify: `web/src/hooks/useWorkspaceWs.ts`
- Test: `web/src/state/workspace-ws-store.test.ts`

- [ ] **Step 1: 写失败测试，要求多 chunk 批处理后内容顺序不变**

在 `workspace-ws-store.test.ts` 增加测试。先以 store action 级别验证顺序和最终内容，不要求真实 timer。

```ts
it("preserves active stream chunk order while avoiding duplicate historical entries", () => {
  useWorkspaceStore.getState().setSessionState({
    session_id: "workspace_session_stream",
    workspace_type: "story",
    stage: "running",
    superpowers_enabled: true,
    openspec_enabled: true,
    messages: [],
    checkpoints: [],
    artifact: null,
    providers: { author: "codex", reviewer: "claude_code" },
    timeline_nodes: [
      {
        node_id: "timeline_node_001",
        node_type: "author_run",
        agent: "codex",
        stage: "running",
        status: "active",
        title: "Story Spec 生成",
        started_at: "2026-06-06T00:00:00Z",
        provider_config_snapshot: {
          author: "codex",
          reviewer: "claude_code",
          review_rounds: 1,
        },
      },
    ],
    active_node_id: "timeline_node_001",
    artifact_versions: [],
    timeline_node_details: {},
    active_run_id: "run-1",
  });

  useWorkspaceStore.getState().appendBufferedStreamChunk("A", "timeline_node_001", "author");
  useWorkspaceStore.getState().appendBufferedStreamChunk("B", "timeline_node_001", "author");
  useWorkspaceStore.getState().flushBufferedStream("timeline_node_001");

  const streamEntry = useWorkspaceStore
    .getState()
    .chatEntries.find((entry) => entry.id === "timeline_node_001:stream-active");

  expect(streamEntry?.content).toBe("AB");
  expect(
    useWorkspaceStore
      .getState()
      .chatEntries.filter((entry) => entry.id === "timeline_node_001:stream-active"),
  ).toHaveLength(1);
});
```

- [ ] **Step 2: 增加 store actions 类型**

在 `WorkspaceWsActions` 中增加：

```ts
appendBufferedStreamChunk: (
  content: string,
  nodeId: string,
  role: ChatEntryRole,
) => void;
flushBufferedStream: (nodeId: string) => void;
completeBufferedStream: (nodeId: string, messageId: string, checkpointId: string) => void;
```

在 state 中增加：

```ts
streamBuffers: Record<string, { chunks: string[]; visibleText: string; role: ChatEntryRole }>;
```

- [ ] **Step 3: 实现缓冲 action**

在 `workspace-ws-store.ts` 实现：

```ts
appendBufferedStreamChunk: (content, nodeId, role) =>
  set((prev) => {
    const existing = prev.streamBuffers[nodeId] ?? { chunks: [], visibleText: "", role };
    return {
      streamBuffers: {
        ...prev.streamBuffers,
        [nodeId]: {
          ...existing,
          role,
          chunks: [...existing.chunks, content],
        },
      },
    };
  }),

flushBufferedStream: (nodeId) =>
  set((prev) => {
    const buffer = prev.streamBuffers[nodeId];
    if (!buffer || buffer.chunks.length === 0) {
      return {};
    }
    const appended = buffer.chunks.join("");
    const visibleText = buffer.visibleText + appended;
    const entryId = chatEntryId(nodeId, "stream-active");
    const index = prev.chatEntries.findIndex((entry) => entry.id === entryId);
    const entry: ChatEntry = {
      id: entryId,
      type: "provider_stream",
      role: buffer.role,
      content: visibleText,
      timestamp: new Date().toISOString(),
      node_id: nodeId,
      content_ref: { kind: "node_stream", nodeId },
    };
    const chatEntries = index === -1 ? [...prev.chatEntries, entry] : [...prev.chatEntries];
    if (index !== -1) {
      chatEntries[index] = entry;
    }
    return {
      chatEntries,
      streamBuffers: {
        ...prev.streamBuffers,
        [nodeId]: { ...buffer, chunks: [], visibleText },
      },
      activeStreamEntryId: entryId,
    };
  }),
```

- [ ] **Step 4: 修改 WS stream handler 使用缓冲并节流**

在 `useWorkspaceWs.ts` 中用 `useRef<Record<string, number | null>>` 或单个 timeout/ref 做 flush 调度。收到 `stream_chunk` 时：

```ts
const nodeId = msg.node_id as string | null | undefined;
if (nodeId) {
  const role = entryRoleForNode(store, nodeId, (msg.role as ChatEntryRole) ?? "author");
  store.appendBufferedStreamChunk(msg.content as string, nodeId, role);
  scheduleFlush(nodeId);
} else {
  store.appendStreamChunk(msg.content as string, nodeId);
}
```

flush 间隔建议 `80ms`。实现 `scheduleFlush(nodeId)` 时确保同一个 node 同时只有一个 timer。

- [ ] **Step 5: message_complete 时 flush 再完成**

在 `message_complete` case 中先执行：

```ts
if (msg.node_id) {
  store.flushBufferedStream(msg.node_id as string);
}
```

再执行现有 `completeMessage()` 和 `finalizeStreamingEntry()`。

- [ ] **Step 6: 运行测试**

Run: `pnpm test -- workspace-ws-store.test.ts`

Workdir: `web/`

Expected: PASS。

---

### Task 4: Markdown 渲染延迟与超长内容折叠

**Files:**
- Modify: `web/src/components/chat-workspace/entries/ProviderStreamEntry.tsx`
- Test: `web/src/components/chat-workspace/entries/entries.test.tsx`

- [ ] **Step 1: 写测试，超长内容默认显示 preview 和展开按钮**

在 entries 测试文件中增加：

```tsx
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MarkdownContent } from "./ProviderStreamEntry";

it("collapses very large markdown content until expanded", async () => {
  const huge = "# Title\n" + "line\n".repeat(30_000);
  render(<MarkdownContent content={huge} />);

  expect(screen.getByText(/内容较长/)).toBeInTheDocument();
  expect(screen.queryByText("line".repeat(100))).not.toBeInTheDocument();

  await userEvent.click(screen.getByRole("button", { name: /展开全文/ }));

  expect(screen.getByRole("heading", { name: "Title" })).toBeInTheDocument();
});
```

- [ ] **Step 2: 实现折叠阈值**

在 `ProviderStreamEntry.tsx` 中：

```tsx
const LARGE_MARKDOWN_COLLAPSE_CHARS = 80_000;
const LARGE_MARKDOWN_PREVIEW_CHARS = 8_000;
```

`MarkdownContent` 改为：

```tsx
function MarkdownContent({ content }: { content: string }) {
  const [expanded, setExpanded] = useState(false);
  const isLarge = content.length > LARGE_MARKDOWN_COLLAPSE_CHARS;
  const visibleContent = isLarge && !expanded ? content.slice(0, LARGE_MARKDOWN_PREVIEW_CHARS) : content;
  const tokens = useMemo(
    () =>
      lexer(normalizeProviderContent(visibleContent)).filter(
        (token) => token.type !== "space" && token.type !== "def",
      ),
    [visibleContent],
  );

  return (
    <div className="space-y-2 break-words text-sm text-[var(--aria-ink)]">
      {isLarge && !expanded ? (
        <div className="rounded-md border border-amber-200 bg-amber-50 px-3 py-2 text-xs text-amber-800">
          内容较长，当前显示前 {LARGE_MARKDOWN_PREVIEW_CHARS} 字符。完整内容仍可展开查看。
        </div>
      ) : null}
      {tokens.map((token, index) => renderBlockToken(token, `block-${index}`))}
      {isLarge ? (
        <button
          type="button"
          onClick={() => setExpanded((value) => !value)}
          className="rounded-md border border-[var(--aria-line)] bg-white px-3 py-1 text-xs font-semibold text-[var(--aria-ink)] hover:bg-[var(--aria-panel-muted)]"
        >
          {expanded ? "收起全文" : "展开全文"}
        </button>
      ) : null}
    </div>
  );
}
```

同时补充 imports：

```ts
import { useMemo, useState, type ReactNode } from "react";
```

- [ ] **Step 3: 运行测试**

Run: `pnpm test -- entries.test.tsx`

Workdir: `web/`

Expected: PASS。

---

### Task 5: 虚拟化 ChatEntryList 并保留 Timeline 定位

**Files:**
- Modify: `web/package.json`
- Modify: `web/pnpm-lock.yaml`
- Modify: `web/src/components/chat-workspace/ChatEntryList.tsx`
- Test: `web/src/components/chat-workspace/ChatEntryList.test.tsx`

- [ ] **Step 1: 增加依赖测试前准备**

若项目未安装虚拟列表库，添加 `@tanstack/react-virtual`。

Run: `pnpm add @tanstack/react-virtual`

Workdir: `web/`

Expected: `package.json` 和 `pnpm-lock.yaml` 更新。

- [ ] **Step 2: 写定位测试**

在 `ChatEntryList.test.tsx` 增加测试，验证 `scrollToEntry()` 可按 entry id 调用虚拟列表滚动。若 jsdom 无法验证真实滚动，暴露并 mock virtualizer 的 `scrollToIndex`。

```tsx
it("keeps scrollToEntry available for timeline selection", () => {
  const ref = createRef<ChatEntryListHandle>();
  const entries = Array.from({ length: 100 }, (_, index) => ({
    id: `entry-${index}`,
    type: "stage_change" as const,
    role: "system" as const,
    content: `Entry ${index}`,
    timestamp: "2026-06-06T00:00:00Z",
  }));

  render(<ChatEntryList ref={ref} entries={entries} />);

  expect(() => ref.current?.scrollToEntry("entry-80")).not.toThrow();
});
```

- [ ] **Step 3: 改造 `ChatEntryList` 使用虚拟列表**

目标结构：

```tsx
const parentRef = useRef<HTMLDivElement | null>(null);
const groupedItems = useMemo(() => groupEntries(entries), [entries]);
const entryIndexById = useMemo(() => {
  const map = new Map<string, number>();
  groupedItems.forEach((item, index) => {
    if (item.kind === "group") {
      map.set(entryIdForGroup(item.group), index);
    } else {
      map.set(item.entry.id, index);
    }
  });
  return map;
}, [groupedItems]);
const rowVirtualizer = useVirtualizer({
  count: groupedItems.length,
  getScrollElement: () => parentRef.current,
  estimateSize: () => 140,
  overscan: 6,
});
```

`scrollToEntry`：

```tsx
scrollToEntry(entryId: string) {
  const index = entryIndexById.get(entryId);
  if (index !== undefined) {
    rowVirtualizer.scrollToIndex(index, { align: "start" });
  }
}
```

渲染只渲染 `rowVirtualizer.getVirtualItems()`。

- [ ] **Step 4: 保留空状态和底部滚动**

如果 `entries.length === 0`，保留当前空状态。流式自动滚动可以先保持现有 `endRef` 行为，后续 Task 6 再优化“用户在历史区域不自动拉回”。

- [ ] **Step 5: 运行组件测试**

Run: `pnpm test -- ChatEntryList.test.tsx`

Workdir: `web/`

Expected: PASS。

---

### Task 6: 拆分 Workspace 页面 store selector，降低无关重渲染

**Files:**
- Modify: `web/src/pages/ChatWorkspacePage.tsx`
- Modify: `web/src/state/workspace-ws-store.ts`
- Test: `web/src/pages/ChatWorkspacePage.test.tsx`

- [ ] **Step 1: 增加 selectors**

在 `workspace-ws-store.ts` 导出 selector helper：

```ts
export const selectWorkspaceHeaderState = (state: WorkspaceWsState) => ({
  sessionId: state.sessionId,
  workspaceType: state.workspaceType,
  providers: state.providers,
  reviewRounds: state.reviewRounds,
  stage: state.stage,
  providerLocked: state.providerLocked,
  providerLockedAt: state.providerLockedAt,
  superpowersEnabled: state.superpowersEnabled,
  openSpecEnabled: state.openSpecEnabled,
});

export const selectChatPanelState = (state: WorkspaceWsState) => ({
  chatEntries: state.chatEntries,
  stage: state.stage,
  selectedNodeId: state.selectedNodeId,
});
```

- [ ] **Step 2: 替换 `const store = useWorkspaceStore()`**

在 `ChatWorkspacePage.tsx` 中避免订阅整个 store。拆为多个 selector，例如：

```ts
const stage = useWorkspaceStore((state) => state.stage);
const providers = useWorkspaceStore((state) => state.providers);
const timelineNodes = useWorkspaceStore((state) => state.timelineNodes);
const chatEntries = useWorkspaceStore((state) => state.chatEntries);
const selectedNodeId = useWorkspaceStore((state) => state.selectedNodeId);
```

将原 `store.xxx` 替换为对应局部变量。事件 handler 中需要最新状态时使用 `useWorkspaceStore.getState()`。

- [ ] **Step 3: 运行页面测试**

Run: `pnpm test -- ChatWorkspacePage.test.tsx`

Workdir: `web/`

Expected: PASS。

---

### Task 7: 后端轻量 SessionState 与按需详情 API

**Files:**
- Modify: `src/web/workspace_ws_types.rs`
- Modify: `src/product/workspace_engine.rs`
- Modify: `src/product/lifecycle_store.rs`
- Modify: `src/web/handlers.rs`
- Modify: `src/web/app.rs`
- Test: `tests/it_core/workspace_ws_integration.rs`

- [ ] **Step 1: 写后端失败测试，SessionState 不应包含完整 detail**

在 `tests/it_core/workspace_ws_integration.rs` 或合适 integration test 中新增测试。使用 fake workspace session 创建一个含大 prompt/detail 的 node，打开 WS，读取 `session_state`，断言响应不含完整大 prompt，只含 summary/size/preview。

预期断言：

```rust
assert_eq!(message["type"], "session_state");
assert!(message.get("timeline_node_details").is_none());
assert!(message["timeline_node_summaries"].is_object());
assert_eq!(message["timeline_node_summaries"]["timeline_node_001"]["prompt_size"], huge_prompt.len());
assert_ne!(message.to_string().contains(&huge_prompt), true);
```

- [ ] **Step 2: 新增 DTO**

在 `workspace_ws_types.rs` 增加：

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeDetailSummary {
    pub node_id: String,
    pub node_type: String,
    pub status: String,
    pub agent_role: Option<String>,
    pub provider_name: Option<String>,
    pub prompt_size: usize,
    pub prompt_preview: Option<String>,
    pub stream_size: usize,
    pub stream_preview: Option<String>,
    pub execution_event_count: usize,
    pub has_large_outputs: bool,
    pub artifact_ref: Option<String>,
    pub started_at: String,
    pub ended_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactVersionSummary {
    pub version: u32,
    pub generated_by: ProviderName,
    pub reviewed_by: Option<ProviderName>,
    pub review_verdict: Option<ReviewVerdictType>,
    pub confirmed_by: Option<String>,
    pub is_current: bool,
    pub created_at: String,
    pub source_node_id: String,
    pub markdown_size: usize,
    pub markdown_preview: String,
}
```

将 `SessionState` 调整为新增 summary 字段。兼容期可保留 `timeline_node_details: HashMap<String, NodeDetail>` 但默认传空 map；最终任务移除前端依赖后再删除。

- [ ] **Step 3: 实现 summary 构造**

在 `workspace_engine.rs` 增加 helper：

```rust
const SUMMARY_PREVIEW_CHARS: usize = 2048;

fn preview(value: &str) -> String {
    value.chars().take(SUMMARY_PREVIEW_CHARS).collect()
}
```

构造 node summary 时：

```rust
let prompt = detail.prompt.as_deref().unwrap_or("");
let stream = if !detail.streaming_content.is_empty() {
    detail.streaming_content.as_str()
} else {
    detail
        .messages
        .last()
        .map(|message| message.content.as_str())
        .unwrap_or("")
};
```

- [ ] **Step 4: 新增只读 API handler**

新增路由：

```text
GET /api/workspace-sessions/:session_id/timeline-node-details/:node_id
GET /api/workspace-sessions/:session_id/timeline-node-details/:node_id/prompt
GET /api/workspace-sessions/:session_id/timeline-node-details/:node_id/events/:event_id/output
GET /api/workspace-sessions/:session_id/artifact-versions/:version
```

响应示例：

```json
{
  "node_id": "timeline_node_001",
  "prompt": "完整 Provider Prompt 文本"
}
```

```json
{
  "version": 3,
  "markdown": "# Artifact v3\n\n完整 Markdown 文本"
}
```

- [ ] **Step 5: 运行后端定向测试**

Run: `cargo test --locked --test it_core workspace_ws_integration`

Workdir: repository root。

Expected: PASS。

---

### Task 8: 前端按需加载 Node Detail、Prompt、Output、Artifact

**Files:**
- Create: `web/src/api/workspace-content.ts`
- Modify: `web/src/api/types.ts`
- Modify: `web/src/state/workspace-ws-store.ts`
- Modify: `web/src/components/chat-workspace/InlineEventRow.tsx`
- Modify: `web/src/components/chat-workspace/ArtifactPane.tsx`
- Test: relevant frontend tests

- [ ] **Step 1: 创建 API 封装**

新增 `web/src/api/workspace-content.ts`：

```ts
export async function fetchWorkspaceNodeDetail(sessionId: string, nodeId: string) {
  const response = await fetch(`/api/workspace-sessions/${sessionId}/timeline-node-details/${nodeId}`);
  if (!response.ok) {
    throw new Error(`加载节点详情失败：${response.status}`);
  }
  return response.json();
}

export async function fetchWorkspacePrompt(sessionId: string, nodeId: string) {
  const response = await fetch(`/api/workspace-sessions/${sessionId}/timeline-node-details/${nodeId}/prompt`);
  if (!response.ok) {
    throw new Error(`加载 Prompt 失败：${response.status}`);
  }
  return response.json() as Promise<{ node_id: string; prompt: string }>;
}

export async function fetchWorkspaceEventOutput(sessionId: string, nodeId: string, eventId: string) {
  const response = await fetch(`/api/workspace-sessions/${sessionId}/timeline-node-details/${nodeId}/events/${eventId}/output`);
  if (!response.ok) {
    throw new Error(`加载输出失败：${response.status}`);
  }
  return response.json() as Promise<{ node_id: string; event_id: string; output: string }>;
}

export async function fetchWorkspaceArtifactVersion(sessionId: string, version: number) {
  const response = await fetch(`/api/workspace-sessions/${sessionId}/artifact-versions/${version}`);
  if (!response.ok) {
    throw new Error(`加载 Artifact 失败：${response.status}`);
  }
  return response.json() as Promise<{ version: number; markdown: string }>;
}
```

- [ ] **Step 2: Store 增加内容缓存**

增加：

```ts
contentCache: Record<string, string>;
artifactContentCache: Record<number, string>;
```

key 规则：

```ts
provider_prompt:${nodeId}
execution_output:${nodeId}:${eventId}
node_stream:${nodeId}
```

- [ ] **Step 3: InlineEventRow 展开时加载 output**

如果 entry 有 `content_ref.kind === "execution_output"`，展开时调用 API，将结果存入 cache，再显示。没有全文时显示 loading。

- [ ] **Step 4: ArtifactPane 切换版本时加载 markdown**

`ArtifactPane` 接收 version summaries 和 `loadArtifactVersion(version)`。只渲染当前 selected markdown；diff 模式只加载 selected 和 previous。

- [ ] **Step 5: 运行前端测试**

Run: `pnpm test`

Workdir: `web/`

Expected: PASS。

---

### Task 9: 大型 Workspace E2E 与性能守护

**Files:**
- Create/Modify: `web/e2e/workspace-memory.spec.ts`
- Modify: `web/e2e/helpers/workspace.ts`

- [ ] **Step 1: 创建大型 workspace fixture 或测试构造器**

构造包含以下内容的 workspace：

- 40+ timeline nodes。
- 至少 10 个 provider stream。
- 多个 provider prompt，每个 100KB 以上。
- 多个 execution output，每个 100KB 以上。
- 至少 5 个 artifact versions。

- [ ] **Step 2: E2E 验证页面功能**

测试流程：

```ts
test("large workspace opens without eager rendering all full content", async ({ page }) => {
  await page.goto("/workbench/workspace/workspace_session_large_fixture");
  await expect(page.getByTestId("chat-entry-list")).toBeVisible();
  await expect(page.getByText(/Provider Prompt/)).toBeVisible();
  await page.getByText(/Provider Prompt/).first().click();
  await expect(page.getByText(/完整提示词/)).toBeVisible();
});
```

- [ ] **Step 3: 增加结构性性能断言**

不要依赖不同机器的精确内存数，优先断言结构：

```ts
const domNodeCount = await page.evaluate(() => document.querySelectorAll("*").length);
expect(domNodeCount).toBeLessThan(3000);
```

检查 chat entries 不携带大 metadata：

```ts
const hasHugeMetadata = await page.evaluate(() => {
  const state = (window as any).__ARIA_WORKSPACE_STORE__?.getState?.();
  if (!state) return false;
  return state.chatEntries.some((entry: any) => JSON.stringify(entry.metadata ?? {}).length > 10_000);
});
expect(hasHugeMetadata).toBe(false);
```

如果没有 debug hook，不为了测试暴露全局状态；改用组件可见行为和网络 payload 断言。

- [ ] **Step 4: 运行 E2E**

Run: `pnpm test:e2e -- workspace-memory.spec.ts`

Workdir: `web/`

Expected: PASS。

---

### Task 10: 全量验证与回归说明

**Files:**
- Modify if needed: `cadence/reports/2026-06-06_进度报告_Workspace页面内存占用治理验证_v1.0.md`

- [ ] **Step 1: 运行前端单元测试**

Run: `pnpm test`

Workdir: `web/`

Expected: PASS。

- [ ] **Step 2: 运行前端构建或类型检查**

根据 `web/package.json` 实际脚本执行。优先：

Run: `pnpm build`

Workdir: `web/`

Expected: PASS。

- [ ] **Step 3: 运行 Rust 格式检查**

Run: `cargo fmt --check`

Workdir: repository root。

Expected: PASS。

- [ ] **Step 4: 运行 Rust clippy**

Run: `cargo clippy --all-targets --all-features --locked -- -D warnings`

Workdir: repository root。

Expected: PASS。

- [ ] **Step 5: 运行 Rust check**

Run: `cargo check --locked`

Workdir: repository root。

Expected: PASS。

- [ ] **Step 6: 运行 Rust test**

Run: `cargo test --locked`

Workdir: repository root。

Expected: PASS。

- [ ] **Step 7: 真实页面验收**

启动服务后打开：

```text
http://127.0.0.1:5173/workbench/workspace/workspace_session_0003
```

验收项：

- 页面能打开，不白屏。
- 当前 active provider 输出仍流式展示。
- 历史节点可通过 Timeline 定位。
- Provider Prompt 可展开全文。
- Execution Output 可展开全文。
- Artifact 最新版本可查看。
- Artifact 历史版本可切换。
- Artifact Diff 可显示。
- DOM 节点数和内存占用不再随历史条目全量线性膨胀。

---

## 验收指标

- 初始 `session_state` payload 目标 `< 500KB`。若 fixture 本身 summary 很多，可记录合理上限并解释。
- `chatEntries` 中任意 `metadata` 序列化长度 `< 10KB`。
- 默认 DOM 节点数 `< 3000`。
- 流式输出 100K 字符期间 UI 保持响应。
- Markdown parser 不在每个 chunk 上对完整历史内容重复运行。
- `workspace_session_0003` 浏览器 tab 内存不再到十 GB 级；开发模式目标 `< 1.5GB`，生产构建目标 `< 1GB`。
- Story Spec、Design Spec、Work Item 三种 workspace type 均覆盖 session restore、timeline node、artifact/ref 行为；若某类无 artifact 或节点类型不同，测试说明中明确排除原因。

## 风险与回滚

- 风险：虚拟列表影响 Timeline 定位。缓解：使用 entry id 到 virtual index 的稳定映射，不再依赖 DOM query。
- 风险：按需 API 失败导致展开全文失败。缓解：展开区域显示错误和重试按钮，不影响页面主体。
- 风险：流式节流被感知为不够实时。缓解：刷新间隔控制在 `50ms` 到 `100ms`，保留 active stream 底部跟随。
- 风险：轻量 session state 与旧测试不兼容。缓解：先兼容保留旧字段为空或 feature flag，前端切换后再清理。
- 回滚：每阶段独立提交。若后端轻量化风险过高，可先保留前端去副本、虚拟化和节流三项，它们已能显著降低内存。

## 自查结果

- Spec coverage：计划覆盖传输层、状态层、渲染层、流式层、Artifact/Monaco、测试和验收。
- Placeholder scan：无 `TBD`、`TODO`、`implement later`；每个任务包含文件、步骤、命令和期望。
- Type consistency：`WorkspaceContentRef`、`ChatEntry.content_ref`、`content_size`、`has_full_content` 在 Task 2 后续任务中保持一致。
- Project rule coverage：计划文档存放在 `cadence/plans/`；Rust 命令未使用 `-j 1`；Workspace 产物链路要求覆盖 Story/Design/Work Item。
