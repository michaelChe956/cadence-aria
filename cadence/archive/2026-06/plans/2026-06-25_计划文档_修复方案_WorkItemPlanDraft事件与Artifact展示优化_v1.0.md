# WorkItemPlan Draft Event and Artifact UX Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修复 Work Item Plan 逐项生成阶段的 4 个可见性问题：Draft 缺 Prompt 事件、staged artifact 版本文案误导、左侧 timeline 无法辨别 Draft 对象、HTML entity/JSON-like 内容不可读。

**Architecture:** 后端负责补齐 Work Item Draft provider prompt 事件和 timeline 节点业务标识；前端负责把内部版本号降级为辅助信息、将 Work Item Plan staged artifact 按业务对象展示，并建立通用文本解码/格式化层。方案不修改 `.aria` 运行数据，`workspace_session_0003` 只作为问题证据和回归观察样本。

**Tech Stack:** Rust 1.95 / Tokio / Axum WebSocket / serde_json；React / TypeScript / Zustand / Vitest / Testing Library。

---

## 背景与证据

### 现象 1：Work Item Draft 消息气泡缺 Prompt 事件

`workspace_session_0003` 的 Draft 节点详情显示：

- `timeline_node_007`：`prompt_len = 0`，execution events 只有 `Claude Code provider started`、`Turn completed` 和 Bash 工具调用。
- `timeline_node_010`：`prompt_len = 0`，同样没有 `Provider Prompt` event。

代码路径对比：

- Outline run：`src/web/workspace_ws_handler/run.rs` 已在 provider start 前调用 `engine.emit_provider_prompt_event(...)`。
- Draft run：`ProviderRunKind::WorkItemPlanDraft` 分支只构造 `provider_input` 后直接 `provider.start(...)`。
- 自动 followup Draft run：`src/web/workspace_ws_handler/run/followups.rs` 也只构造 `provider_input` 后直接 start provider。

结论：这是后端事件缺失，不是纯前端漏渲染。

### 现象 2：Work Item Plan Outline / Draft artifact 被展示成 v4/v5

当前前端 `artifact_update` 文案直接使用内部 `version`：

- `Work Item Plan staged artifact 已更新 -> v4`
- `Work Item Plan staged artifact 已更新 -> v5`

这会把内部历史版本号误导为业务 artifact 名称。Work Item Plan Outline 和 Work Item Draft 都是 staged artifact 集合中的业务对象，用户更需要看到：

- artifact 类型：Outline / Draft / Batch / Compile Report
- 当前对象：`round_015`、`outline_backend_api`、`draft_002`
- 状态：`draft`、`accepted`、`validation_failed`

### 现象 3：左侧 timeline 的 Draft 节点无法辨认

`timeline_nodes.json` 中多个节点 title 都是：

- `Work Item Draft 生成`
- `Work Item Draft 确认`
- `Work Item Draft Review Round 2`

outline id 只藏在 summary 或 artifact payload 中。前端 `TimelineNodeList.tsx` 又只展示截断 title 和单行 summary，所以多个 Draft 节点看起来几乎一样。

### 现象 4：前端有 `&quot;` 等未转译内容

当前 `round_015/draft_001` 与 `draft_002` 落盘 JSON 中未发现 `&quot;`，说明截图 3 更可能来自 provider stream、inline event、或即时详情展示。现有前端只在 tester plan JSON formatter 里局部 decode HTML entities，缺少通用展示层处理。

---

## 设计原则

1. Prompt 事件由后端补齐，因为 prompt 是 provider input 的事实记录，必须持久化到 timeline node detail。
2. Artifact update 消息只表达“业务对象更新”，内部 version 作为辅助 metadata 展示。
3. Timeline 先解决“人能分辨谁是谁”，再考虑精细视觉；Draft 节点必须显式展示 outline id、draft id、业务 title。
4. HTML entity 解码只用于展示层和入库防御，不改变安全边界；React 仍按文本渲染，不使用 `dangerouslySetInnerHTML`。
5. TDD 优先：每个问题先补失败测试，再实现。

---

## 文件结构

### 后端

- Modify: `src/web/workspace_ws_handler/run.rs`
  - 在 `ProviderRunKind::WorkItemPlanDraft` 启动 provider 前 emit Draft prompt event。
- Modify: `src/web/workspace_ws_handler/run/followups.rs`
  - 在自动继续的 Work Item Draft run 启动 provider 前 emit Draft prompt event。
- Modify: `src/product/workspace_engine/draft_batch/runs.rs`
  - 创建 Draft run / review run timeline node 时，把 title/summary 改成可辨识的业务对象。
- Test: `tests/it_web/web_work_item_plan_serial.rs`
  - 覆盖 serial Draft run 通过 WebSocket 发出 `Provider Prompt` event。
  - 覆盖 session state 恢复后 Draft node detail 有 prompt snapshot。

### 前端状态映射

- Modify: `web/src/hooks/useWorkspaceWs.ts`
  - 将 staged artifact update 文案改成业务摘要。
  - `artifact_update` metadata 增加 `artifact_label`、`object_id`、`object_title`、`status_label`、`version_label`。
- Modify: `web/src/state/workspace-ws-store.ts`
  - `buildChatEntries` 从 `artifactVersions` rebuild 时也使用同一套业务文案。
  - 增加 timeline 节点副标题/对象摘要辅助函数，供左侧 timeline 组件使用。
- Test: `web/src/hooks/useWorkspaceWs.test.tsx`
  - 覆盖实时 artifact update 文案不再以 `vN` 为主。
- Test: `web/src/state/workspace-ws-store.test.ts`
  - 覆盖 session state rebuild 后 artifact update 文案仍一致。

### 前端展示组件

- Modify: `web/src/components/chat-workspace/entries/ArtifactUpdateEntry.tsx`
  - 改为业务摘要行，内部 version 放小号辅助文本。
- Modify: `web/src/components/chat-workspace/entries/StageChangeEntry.tsx`
  - 阶段名转中文，避免直接展示 `author_confirm`、`running` 等底层状态。
- Modify: `web/src/components/chat-workspace/TimelineNodeList.tsx`
  - 对 Work Item Plan 节点展示两行结构：业务 title + object id/status。
- Modify: `web/src/components/chat-workspace/InlineEventRow.tsx`
  - 展开输出前做 HTML entity decode；JSON-like 内容格式化。
- Modify: `web/src/components/chat-workspace/entries/ProviderStreamEntry.tsx`
  - 抽出或复用通用 decode/format 工具，覆盖 provider stream。
- Modify: `web/src/components/workspace/WorkItemPlanArtifactPanel.tsx`
  - 对 `ReadableBlock`、`KeyValue`、`BulletList` 输出做展示层 decode。
- Create: `web/src/components/chat-workspace/text-display.ts`
  - 提供 `decodeHtmlEntitiesForDisplay`、`formatJsonLikeTextForDisplay`、`normalizeDisplayText`。
- Test:
  - `web/src/components/chat-workspace/entries/p1-entries.test.tsx`
  - `web/src/components/chat-workspace/InlineEventRow.test.tsx`
  - `web/src/components/chat-workspace/TimelineNodeList.test.tsx`
  - `web/src/components/workspace/WorkItemPlanArtifactPanel.test.tsx`

---

## Task 1: 后端补齐 Work Item Draft Prompt Event

**Files:**
- Modify: `src/web/workspace_ws_handler/run.rs`
- Modify: `src/web/workspace_ws_handler/run/followups.rs`
- Test: `tests/it_web/web_work_item_plan_serial.rs`

- [ ] **Step 1: 写失败测试，确认 serial Draft run 会发 Provider Prompt event**

在 `tests/it_web/web_work_item_plan_serial.rs` 新增测试：

```rust
#[tokio::test]
async fn serial_draft_run_emits_provider_prompt_event() {
    let _guard = enable_test_controls().await;
    let _ws_guard = WS_TEST_LOCK.lock().await;
    let (app, _workspace) = app_with_confirmed_story_and_design_and_streaming_outputs(vec![
        valid_outline_output(),
        valid_draft_output("outline_backend_core"),
    ])
    .await;

    let (_session_id, _plan_id, mut ws) =
        prepare_plan_accept_outline_and_select_serial(&app).await;

    let messages = recv_ws_until(&mut ws, Duration::from_secs(15), |messages| {
        messages.iter().any(|message| {
            message["type"] == "execution_event"
                && message["event"]["title"] == "Provider Prompt"
                && message["event"]["node_id"].as_str().is_some_and(|node_id| {
                    messages.iter().any(|created| {
                        created["type"] == "timeline_node_created"
                            && created["node"]["node_id"] == node_id
                            && created["node"]["node_type"] == "work_item_draft_run"
                    })
                })
        })
    })
    .await;

    let prompt_event = messages
        .iter()
        .find(|message| {
            message["type"] == "execution_event"
                && message["event"]["title"] == "Provider Prompt"
        })
        .expect("draft Provider Prompt event");

    assert_eq!(prompt_event["event"]["kind"], "output");
    assert_eq!(prompt_event["event"]["status"], "started");
    assert!(
        prompt_event["event"]["output"]
            .as_str()
            .expect("prompt output")
            .contains("Work Item Draft author")
    );
}
```

若 helper 名称与现有 fixture 不一致，使用同文件中已有的 fake output helper，测试目标保持不变：必须观察到 `work_item_draft_run` 节点上的 `Provider Prompt` event。

- [ ] **Step 2: 运行失败测试**

Run:

```bash
cargo test --locked --test it_web serial_draft_run_emits_provider_prompt_event
```

Expected: FAIL，原因是没有收到 `Provider Prompt` execution_event。

- [ ] **Step 3: 在手动/直接 Draft run 分支 emit prompt**

在 `src/web/workspace_ws_handler/run.rs` 的 `ProviderRunKind::WorkItemPlanDraft { feedback }` 分支中，`provider_input` 构造成功后、`provider_for_run.start(...)` 前增加：

```rust
engine
    .emit_provider_prompt_event(
        &node_id,
        provider_input.prompt.clone(),
        if feedback.is_some() {
            "发送给 WorkItemDraft provider 的增量返修提示词"
        } else {
            "发送给 WorkItemDraft provider 的完整提示词"
        },
        Some(author_provider.clone()),
    )
    .await;
```

注意 `author_provider` 需要在 emit 前 clone 出来，后续 `drive_work_item_plan_provider_session_to_output` 仍使用同一个 provider name。

- [ ] **Step 4: 在自动 followup Draft run 分支 emit prompt**

在 `src/web/workspace_ws_handler/run/followups.rs` 中，`provider_input` 构造成功后、`provider_for_draft.start(...)` 前增加：

```rust
$engine
    .emit_provider_prompt_event(
        &node_id,
        provider_input.prompt.clone(),
        "发送给 WorkItemDraft provider 的完整提示词",
        Some(author_name.clone()),
    )
    .await;
```

- [ ] **Step 5: 运行后端定向验证**

Run:

```bash
cargo test --locked --test it_web serial_draft_run_emits_provider_prompt_event
cargo test --locked --test it_web work_item_plan_serial
```

Expected: 两个命令均 PASS。

---

## Task 2: Artifact Update 文案改成业务对象摘要

**Files:**
- Modify: `web/src/hooks/useWorkspaceWs.ts`
- Modify: `web/src/state/workspace-ws-store.ts`
- Modify: `web/src/components/chat-workspace/entries/ArtifactUpdateEntry.tsx`
- Test: `web/src/hooks/useWorkspaceWs.test.tsx`
- Test: `web/src/state/workspace-ws-store.test.ts`
- Test: `web/src/components/chat-workspace/entries/p1-entries.test.tsx`

- [ ] **Step 1: 写实时 WebSocket 文案测试**

在 `web/src/hooks/useWorkspaceWs.test.tsx` 的 staged artifact 测试附近新增断言：

```ts
expect(useWorkspaceStore.getState().chatEntries.at(-1)).toMatchObject({
  type: "artifact_update",
  content: "Compile Report 已更新 · committed",
  metadata: {
    version: 4,
    version_label: "内部版本 v4",
    artifact_type: "compile_report",
    artifact_label: "Compile Report",
    status_label: "committed",
  },
});
expect(useWorkspaceStore.getState().chatEntries.at(-1)?.content).not.toContain("-> v4");
```

对 `draft_candidate` 增加一条独立测试：

```ts
expect(draftEntry).toMatchObject({
  type: "artifact_update",
  content: "Draft 已更新 · outline_backend_core · draft_001",
  metadata: expect.objectContaining({
    artifact_type: "draft_candidate",
    artifact_label: "Draft",
    object_id: "outline_backend_core",
    draft_id: "draft_001",
    status_label: "draft",
    version_label: "内部版本 v2",
  }),
});
```

- [ ] **Step 2: 写 rebuild 文案测试**

在 `web/src/state/workspace-ws-store.test.ts` 中构造 `artifact_versions` 与 `workItemPlanArtifact` 的 session state，断言 rebuild 后：

```ts
const artifactEntry = useWorkspaceStore
  .getState()
  .chatEntries.find((entry) => entry.type === "artifact_update");

expect(artifactEntry).toMatchObject({
  type: "artifact_update",
  content: "Draft 已更新 · outline_backend_core · draft_001",
  metadata: expect.objectContaining({
    version_label: "内部版本 v4",
    artifact_label: "Draft",
  }),
});
```

- [ ] **Step 3: 实现 artifact 摘要辅助函数**

在 `web/src/hooks/useWorkspaceWs.ts` 增加：

```ts
function workItemPlanArtifactUpdateSummary(
  artifact: WorkItemPlanArtifactPayload,
  version: number,
) {
  const versionLabel = `内部版本 v${version}`;
  if (artifact.type === "outline_candidate") {
    const outline = artifact.payload.outline;
    const items = outline.work_item_outlines ?? outline.work_items ?? [];
    const round = artifact.payload.current_generation_round_id ?? outline.id ?? "未命名 round";
    return {
      content: `Outline 已更新 · ${round} · ${items.length} items`,
      metadata: {
        version,
        version_label: versionLabel,
        artifact_type: artifact.type,
        artifact_label: "Outline",
        object_id: round,
        status_label: outline.status ?? null,
      },
    };
  }
  if (artifact.type === "draft_candidate") {
    const record = artifact.payload.draft_record;
    return {
      content: `Draft 已更新 · ${record.outline_id} · ${record.draft_id}`,
      metadata: {
        version,
        version_label: versionLabel,
        artifact_type: artifact.type,
        artifact_label: "Draft",
        object_id: record.outline_id,
        object_title: record.candidate.title,
        draft_id: record.draft_id,
        status_label: record.status,
      },
    };
  }
  if (artifact.type === "batch_state") {
    return {
      content: `Batch Draft 已更新 · ${artifact.payload.batch_status}`,
      metadata: {
        version,
        version_label: versionLabel,
        artifact_type: artifact.type,
        artifact_label: "Batch Draft",
        object_id: artifact.payload.batch_id,
        status_label: artifact.payload.batch_status,
      },
    };
  }
  return {
    content: `Compile Report 已更新 · ${artifact.payload.status}`,
    metadata: {
      version,
      version_label: versionLabel,
      artifact_type: artifact.type,
      artifact_label: "Compile Report",
      object_id: artifact.payload.compile_id,
      status_label: artifact.payload.status,
    },
  };
}
```

`workspace-ws-store.ts` 需要同等逻辑。为避免重复，可把纯函数放到新文件 `web/src/state/work-item-plan-artifact-summary.ts`，由 hook 和 store 共用。

- [ ] **Step 4: 更新 ArtifactUpdateEntry 展示**

`ArtifactUpdateEntry.tsx` 改为：

```tsx
export function ArtifactUpdateEntry({ entry }: { entry: ChatEntry }) {
  const versionLabel =
    typeof entry.metadata?.version_label === "string" ? entry.metadata.version_label : null;
  const objectTitle =
    typeof entry.metadata?.object_title === "string" ? entry.metadata.object_title : null;

  return (
    <ChatEntryContainer role="system" title="产物更新">
      <div className="flex items-start gap-2 text-sm text-[var(--aria-ink)]">
        <Package className="mt-0.5 h-4 w-4 shrink-0 text-[var(--aria-primary)]" />
        <div className="min-w-0">
          <div className="font-medium">{entry.content}</div>
          {objectTitle ? (
            <div className="mt-1 truncate text-xs text-[var(--aria-ink-muted)]">
              {objectTitle}
            </div>
          ) : null}
          {versionLabel ? (
            <div className="mt-1 text-[11px] text-[var(--aria-ink-muted)]">
              {versionLabel}
            </div>
          ) : null}
        </div>
      </div>
    </ChatEntryContainer>
  );
}
```

- [ ] **Step 5: 运行前端定向测试**

Run:

```bash
pnpm -C web test -- useWorkspaceWs workspace-ws-store p1-entries
```

Expected: PASS。

---

## Task 3: Timeline Draft 节点展示 outline id / draft id / name

**Files:**
- Modify: `src/product/workspace_engine/draft_batch/runs.rs`
- Modify: `src/product/workspace_engine/draft_batch/authoring.rs`
- Modify: `web/src/components/chat-workspace/TimelineNodeList.tsx`
- Test: `web/src/components/chat-workspace/TimelineNodeList.test.tsx`
- Test: `tests/it_web/web_work_item_plan_serial.rs`

- [ ] **Step 1: 写前端 timeline 展示测试**

在 `TimelineNodeList.test.tsx` 新增：

```tsx
it("renders work item draft nodes with outline id and readable summary", () => {
  render(
    <TimelineNodeList
      nodes={[
        timelineNode({
          node_id: "draft-run-1",
          node_type: "work_item_draft_run",
          title: "Draft · Provider 依赖 HTTP API 端点",
          summary: "outline_backend_api · draft_002 · draft",
          status: "active",
        }),
      ]}
      activeNodeId="draft-run-1"
      selectedNodeId={null}
      onSelectNode={vi.fn()}
    />,
  );

  const node = screen.getByTestId("timeline-node-work_item_draft_run");
  expect(node).toHaveTextContent("Draft · Provider 依赖 HTTP API 端点");
  expect(node).toHaveTextContent("outline_backend_api");
  expect(node).toHaveTextContent("draft_002");
});
```

- [ ] **Step 2: 后端 Draft run title 携带 outline id**

在 `create_serial_work_item_draft_run_node` 中，读取当前 outline item title，生成：

```rust
title: format!("Draft · {}", current_outline_title),
summary: Some(format!("{} · pending", outline_id)),
```

`current_outline_title` 来自 latest outline candidate 中匹配 `outline_id` 的 item；找不到时使用 `outline_id`。

- [ ] **Step 3: Draft author 完成后更新节点 summary 携带 draft id/status**

在 `complete_work_item_draft_author` 生成 `record` 后，完成 active node 的 summary 改为：

```rust
let summary = format!(
    "{} · {} · {}",
    record.outline_id,
    record.draft_id,
    work_item_draft_status_label(&record.status)
);
self.complete_active_node(Some(summary)).await;
```

保持现有进入 confirm 的行为不变。

- [ ] **Step 4: 前端 TimelineNodeList 支持两行显示**

`TimelineNodeButton` 中取消 summary 的强制 `truncate`，改为最多两行：

```tsx
{node.summary ? (
  <p className="mt-1 line-clamp-2 break-words text-xs leading-4 text-[var(--aria-ink-muted)]">
    {node.summary}
  </p>
) : null}
```

按钮保留固定 border/ring；active 和 selected 都必须显示完整边框。

- [ ] **Step 5: 运行测试**

Run:

```bash
pnpm -C web test -- TimelineNodeList
cargo test --locked --test it_web work_item_plan_serial
```

Expected: PASS。

---

## Task 4: 通用 HTML Entity 解码与 JSON-like 可读化

**Files:**
- Create: `web/src/components/chat-workspace/text-display.ts`
- Modify: `web/src/components/chat-workspace/entries/ProviderStreamEntry.tsx`
- Modify: `web/src/components/chat-workspace/InlineEventRow.tsx`
- Modify: `web/src/components/workspace/WorkItemPlanArtifactPanel.tsx`
- Test: `web/src/components/chat-workspace/entries/entries.test.tsx`
- Test: `web/src/components/chat-workspace/InlineEventRow.test.tsx`
- Test: `web/src/components/workspace/WorkItemPlanArtifactPanel.test.tsx`

- [ ] **Step 1: 写工具测试**

在 `web/src/components/chat-workspace/text-display.test.ts` 新增：

```ts
import { describe, expect, it } from "vitest";
import { decodeHtmlEntitiesForDisplay, normalizeDisplayText } from "./text-display";

describe("text-display", () => {
  it("decodes common html entities without using html injection", () => {
    expect(decodeHtmlEntitiesForDisplay("&quot;cmd&quot; &amp; &lt;safe&gt;")).toBe(
      '"cmd" & <safe>',
    );
  });

  it("formats html-entity escaped json objects", () => {
    const raw = "{&quot;required_gates&quot;:[&quot;cmd_check&quot;]}";
    expect(normalizeDisplayText(raw)).toContain('"required_gates": [');
    expect(normalizeDisplayText(raw)).not.toContain("&quot;");
  });
});
```

- [ ] **Step 2: 实现工具**

创建 `text-display.ts`：

```ts
export function decodeHtmlEntitiesForDisplay(content: string) {
  if (!content.includes("&")) {
    return content;
  }
  return content
    .replace(/&quot;/g, '"')
    .replace(/&#34;/g, '"')
    .replace(/&#x22;/gi, '"')
    .replace(/&apos;/g, "'")
    .replace(/&#39;/g, "'")
    .replace(/&#x27;/gi, "'")
    .replace(/&amp;/g, "&")
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">");
}

export function normalizeDisplayText(content: string) {
  const decoded = decodeHtmlEntitiesForDisplay(content);
  const trimmed = decoded.trim();
  if (
    (trimmed.startsWith("{") && trimmed.endsWith("}")) ||
    (trimmed.startsWith("[") && trimmed.endsWith("]"))
  ) {
    try {
      return JSON.stringify(JSON.parse(trimmed), null, 2);
    } catch {
      return decoded;
    }
  }
  return decoded;
}
```

- [ ] **Step 3: 接入 ProviderStreamEntry**

将 `decodeJsonHtmlEntities` 的局部函数替换为共享工具。`normalizeProviderContent` 开头先执行：

```ts
const decoded = normalizeDisplayText(content);
const normalized = decoded.replace(/\r\n?/g, "\n").replace(/\\n/g, "\n");
```

保留 tester plan formatter，但内部使用 `normalizeDisplayText(content.trim())` 再 JSON.parse。

- [ ] **Step 4: 接入 InlineEventRow**

对 detail、command、output 展示前处理：

```ts
const displayDetail = detail ? normalizeDisplayText(detail) : null;
const displayCommand = command ? normalizeDisplayText(command) : null;
const displayOutput = output ? normalizeDisplayText(output) : null;
```

渲染使用 `display*` 变量。

- [ ] **Step 5: 接入 WorkItemPlanArtifactPanel**

`Paragraph`、`ReadableBlock`、`BulletList`、`KeyValue` 对内容调用 `normalizeDisplayText`。数组 join 后再 normalize。

- [ ] **Step 6: 运行前端测试**

Run:

```bash
pnpm -C web test -- text-display entries InlineEventRow WorkItemPlanArtifactPanel
```

Expected: PASS，且可见文本不包含 `&quot;`。

---

## Task 5: Stage Change 文案去底层状态名

**Files:**
- Modify: `web/src/components/chat-workspace/entries/StageChangeEntry.tsx`
- Modify: `web/src/hooks/useWorkspaceWs.ts`
- Test: `web/src/components/chat-workspace/entries/p1-entries.test.tsx`
- Test: `web/src/hooks/useWorkspaceWs.test.tsx`

- [ ] **Step 1: 写测试**

在 `p1-entries.test.tsx` 更新 stage change 测试：

```tsx
const entry = makeEntry({
  type: "stage_change",
  role: "system",
  content: "阶段变更 -> author_confirm",
  metadata: { stage: "author_confirm" },
});

render(<StageChangeEntry entry={entry} />);

expect(screen.getByText("等待作者确认")).toBeInTheDocument();
expect(screen.queryByText("author_confirm")).not.toBeInTheDocument();
```

- [ ] **Step 2: 增加阶段 label 映射**

在 `StageChangeEntry.tsx` 增加：

```ts
const STAGE_LABELS: Record<string, string> = {
  prepare_context: "准备上下文",
  running: "运行中",
  author_confirm: "等待作者确认",
  review: "审核中",
  revision: "返修中",
  human_confirm: "等待人工确认",
  work_item_plan_outline_confirm: "等待 Outline 确认",
  work_item_generation_mode: "选择 Work Item 生成模式",
  work_item_draft_confirm: "等待 Draft 确认",
  work_item_batch_confirm: "等待 Batch 确认",
};
```

渲染优先使用 `entry.metadata.stage`；没有 metadata 时从 `entry.content` 解析 `->` 后的 stage。

- [ ] **Step 3: WebSocket stage_change 写入 metadata**

在 `useWorkspaceWs.ts` 的 `stage_change` 分支中，append entry 时加：

```ts
metadata: { stage: nextStage },
content: stageChangeContent(nextStage),
```

`stageChangeContent("author_confirm")` 返回 `等待作者确认`。

- [ ] **Step 4: 运行测试**

Run:

```bash
pnpm -C web test -- p1-entries useWorkspaceWs
```

Expected: PASS。

---

## Task 6: 回归验证与人工验收

**Files:**
- No source change in this task.

- [ ] **Step 1: 后端格式与编译**

Run:

```bash
cargo fmt --check
cargo check --locked
```

Expected: PASS。

- [ ] **Step 2: 后端定向测试**

Run:

```bash
cargo test --locked --test it_web work_item_plan_serial
cargo test --locked --test it_web work_item_plan_batch
```

Expected: PASS。

- [ ] **Step 3: 前端定向测试**

Run:

```bash
pnpm -C web test -- useWorkspaceWs workspace-ws-store TimelineNodeList p1-entries entries InlineEventRow WorkItemPlanArtifactPanel text-display
```

Expected: PASS。

- [ ] **Step 4: 前端类型检查**

Run:

```bash
pnpm -C web test
```

Expected: PASS。

- [ ] **Step 5: 全量 Rust 验证**

Run:

```bash
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --locked
```

Expected: PASS。若出现已知 WebSocket 超时型偶发失败，单独复跑失败用例并在汇报中说明首次失败与复跑结果。

- [ ] **Step 6: 人工验收 workspace_session_0003 类场景**

在 Work Item Plan 逐项生成流程中检查：

1. Draft 运行消息气泡里出现 `Provider Prompt` 可展开行。
2. artifact update 主文案显示 `Outline 已更新` / `Draft 已更新 · outline_id · draft_id`，内部版本只在辅助信息出现。
3. 左侧 timeline 至少能看到 `outline_backend_api` 或对应 Draft title，不再出现多个不可辨认的 `Work Item Draft ...`。
4. 详情区域不再直接显示 `&quot;`，JSON-like 内容有换行缩进。

---

## 风险与边界

- 不回写既有 `.aria` 数据；老 session 中已经缺失的 prompt snapshot 不做迁移。修复只保证新 run 和后续 session 正确记录。
- Artifact version 仍保留在 metadata 和辅助 UI 中，避免影响 artifact history 按需加载。
- HTML entity decode 只在文本展示和入库防御中使用，不使用 HTML 注入，不降低 XSS 安全性。
- Timeline title 后端增强可能影响少量快照测试，测试应按用户可见文案更新。

---

## 审核点

请重点确认这 4 个决策：

1. Draft prompt event 是否只对新 run 生效，不迁移旧 session。
2. Artifact update 主文案是否接受“业务对象优先，内部版本辅助”的方向。
3. Timeline 是否需要后端 title 改名；如果你希望完全前端推导，也可以只改 `TimelineNodeList.tsx`，但历史 session 的 draft id 需要从当前 artifact 推导，可靠性较差。
4. HTML entity decode 是否只做展示层，还是也要在 WorkItemDraft parse 入库前做防御性 normalize。
