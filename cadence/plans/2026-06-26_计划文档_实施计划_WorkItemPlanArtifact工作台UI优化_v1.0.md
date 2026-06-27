# WorkItemPlan Artifact Workspace Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将 Work Item Plan Artifact 从单一文本/卡片预览升级为能表达阶段完成度、Outline/Draft 结构、版本分组和结构化 diff 的工作台。

**Architecture:** 第一阶段不修改后端 contract，只在前端 typed artifact 展示层新增 selector/helper 和组件拆分。`ChatWorkspacePage` 继续负责 workspace 数据选择与 handler 注入，新的 `WorkItemPlanArtifactPanel` 负责 Work Item Plan 专用工作台布局、tab 状态和只读历史展示。

**Tech Stack:** React 19、TypeScript、Vitest、Testing Library、Tailwind CSS、lucide-react、MonacoViewer。

---

## File Structure

- Modify: `web/src/components/workspace/WorkItemPlanArtifactPanel.tsx`
  - 承载新的 Artifact 工作台、状态条、版本 rail、tabs、Outline/Drafts/Diff/Review/JSON 内容。
- Modify: `web/src/components/workspace/WorkItemPlanArtifactPanel.test.tsx`
  - 组件级 TDD 覆盖状态语义、Outline/Draft 展示、版本分组和 diff。
- Modify: `web/src/pages/ChatWorkspacePage.tsx`
  - 移除页面内旧的横向 Work Item Plan version rail，把版本列表和选择回调传入 `WorkItemPlanArtifactPanel`。
- Modify: `web/src/pages/ChatWorkspacePage.test.tsx`
  - 页面级覆盖历史版本只读、版本切换和 Story/Design markdown artifact 不回归。
- No backend changes.

## Task 1: Component Contract And Failing Tests

**Files:**
- Modify: `web/src/components/workspace/WorkItemPlanArtifactPanel.test.tsx`
- Modify later: `web/src/components/workspace/WorkItemPlanArtifactPanel.tsx`

- [ ] **Step 1: Write failing component tests**

Add tests that render `WorkItemPlanArtifactPanel` with:

```tsx
<WorkItemPlanArtifactPanel
  artifact={{ type: "outline_candidate", payload: workItemPlanOutlinePayload() }}
  versions={[
    { version: 1, is_current: false, source_node_id: "node_outline_v1", artifact: outlineV1 },
    { version: 2, is_current: true, source_node_id: "node_outline_v2", artifact: outlineV2 },
  ]}
  selectedVersion={2}
  onSelectVersion={vi.fn()}
/>
```

Assertions:

- `screen.getByText("Work Item Plan 工作台")`
- `screen.getByText("Outline 已生成，等待确认。Work Item 尚未生成。")`
- `screen.getByTestId("work-item-plan-version-rail")`
- `screen.getByText("Outline")`
- `screen.getByText("Drafts")`
- `screen.getByText("Diff")`
- `screen.getByText("Review")`
- `screen.getByText("JSON")`

- [ ] **Step 2: Verify RED**

Run:

```bash
pnpm -C web test -- WorkItemPlanArtifactPanel.test.tsx
```

Expected: FAIL because the new props, title, status text, rail and tabs do not exist.

- [ ] **Step 3: Minimal contract implementation**

Extend `WorkItemPlanArtifactPanelProps`:

```ts
versions?: WorkItemPlanArtifactVersion[];
selectedVersion?: number | null;
onSelectVersion?: (version: number | null) => void;
activeNodeType?: string | null;
```

Render the shell title, status text, version rail container and tab buttons without changing page integration yet.

- [ ] **Step 4: Verify GREEN**

Run:

```bash
pnpm -C web test -- WorkItemPlanArtifactPanel.test.tsx
```

Expected: PASS for the new shell test and existing tests.

## Task 2: Status Semantics

**Files:**
- Modify: `web/src/components/workspace/WorkItemPlanArtifactPanel.test.tsx`
- Modify: `web/src/components/workspace/WorkItemPlanArtifactPanel.tsx`

- [ ] **Step 1: Write failing status tests**

Add tests for:

- `draft_candidate`: expects `当前仅展示单个 Draft，不代表整组 Work Item 完成。`
- `batch_state` with two draft records: expects `已生成 2 个 Draft，等待接受全部或返修。`
- `compile_report` with status `committed`: expects `Compile 已提交，生成 2 个 Work Item、2 个 Verification Plan、1 个 child session。`
- readonly historical draft: expects `正在查看历史版本 v1，不影响当前流程。`

- [ ] **Step 2: Verify RED**

Run:

```bash
pnpm -C web test -- WorkItemPlanArtifactPanel.test.tsx
```

Expected: FAIL on the new status messages.

- [ ] **Step 3: Implement `artifactStatusMessage`**

Add a pure helper in `WorkItemPlanArtifactPanel.tsx`:

```ts
function artifactStatusMessage(
  artifact: WorkItemPlanArtifactPayload,
  readonly: boolean,
  selectedVersion?: number | null,
): string
```

Use conservative wording for partial states and committed wording only for `compile_report.status === "committed"`.

- [ ] **Step 4: Verify GREEN**

Run:

```bash
pnpm -C web test -- WorkItemPlanArtifactPanel.test.tsx
```

Expected: PASS.

## Task 3: Outline And Draft Workspace Tabs

**Files:**
- Modify: `web/src/components/workspace/WorkItemPlanArtifactPanel.test.tsx`
- Modify: `web/src/components/workspace/WorkItemPlanArtifactPanel.tsx`

- [ ] **Step 1: Write failing interaction tests**

Add tests that:

- Click `Outline` tab and assert a table/list exposes `outline_id`, `exclusive_write_scopes`, `forbidden_write_scopes`, dependencies and risk notes.
- Click `Drafts` tab on `batch_state` and assert both draft rows render plus selected draft details.
- Click `Review` tab and assert validator findings are grouped with severity and code.
- Click `JSON` tab and assert Monaco renders JSON.

- [ ] **Step 2: Verify RED**

Run:

```bash
pnpm -C web test -- WorkItemPlanArtifactPanel.test.tsx
```

Expected: FAIL because these tabs do not have the required structured content.

- [ ] **Step 3: Implement tabs minimally**

Implement:

- `WorkItemPlanTabs`
- `OutlineTab`
- `DraftsTab`
- `ReviewTab`
- `JsonTab`

Keep the existing readable blocks and validator finding helpers where possible, but render Outline/Draft data in denser grids/tables.

- [ ] **Step 4: Verify GREEN**

Run:

```bash
pnpm -C web test -- WorkItemPlanArtifactPanel.test.tsx
```

Expected: PASS.

## Task 4: Version Rail And Structured Diff

**Files:**
- Modify: `web/src/components/workspace/WorkItemPlanArtifactPanel.test.tsx`
- Modify: `web/src/components/workspace/WorkItemPlanArtifactPanel.tsx`

- [ ] **Step 1: Write failing version and diff tests**

Add tests that:

- Rail groups versions under `Outline`, `Drafts`, `Batch`, `Compile`.
- Clicking a rail item calls `onSelectVersion(version)`.
- A version without typed artifact renders disabled text `无内容`.
- Diff tab compares two Outline versions and shows changed write scopes.
- Diff tab compares two Draft versions for the same `outline_id` and shows verification command changes.
- Cross-type or missing same-type diff shows `暂无可比较的 Outline/Draft 版本` or a type mismatch note.

- [ ] **Step 2: Verify RED**

Run:

```bash
pnpm -C web test -- WorkItemPlanArtifactPanel.test.tsx
```

Expected: FAIL on grouped rail and diff assertions.

- [ ] **Step 3: Implement rail and diff helpers**

Add helpers in the component file unless size becomes hard to maintain:

```ts
function versionGroupLabel(artifact: WorkItemPlanArtifactPayload | null): string
function workItemPlanArtifactLabel(artifact: WorkItemPlanArtifactPayload): string
function diffWorkItemPlanArtifacts(
  base: WorkItemPlanArtifactPayload,
  compare: WorkItemPlanArtifactPayload,
): Array<{ field: string; before: string; after: string }>
```

Support Outline and Draft field-level diff first; Batch/Compile can show a concise unsupported or no-change message for this phase.

- [ ] **Step 4: Verify GREEN**

Run:

```bash
pnpm -C web test -- WorkItemPlanArtifactPanel.test.tsx
```

Expected: PASS.

## Task 5: Page Integration

**Files:**
- Modify: `web/src/pages/ChatWorkspacePage.test.tsx`
- Modify: `web/src/pages/ChatWorkspacePage.tsx`

- [ ] **Step 1: Write failing page tests**

Update Work Item Plan page tests to expect:

- `work-item-plan-version-rail` is rendered inside the new panel.
- Selecting a historical typed artifact still shows readonly status.
- Story workspace still renders `ArtifactPane` / Monaco markdown after clicking `Artifact`.
- Page no longer depends on the old `work-item-plan-artifact-version-list` test id.

- [ ] **Step 2: Verify RED**

Run:

```bash
pnpm -C web test -- ChatWorkspacePage.test.tsx
```

Expected: FAIL until page passes versions and callbacks into `WorkItemPlanArtifactPanel`.

- [ ] **Step 3: Integrate panel props**

Remove page-local `WorkItemPlanArtifactVersionRail` rendering and pass:

```tsx
versions={workItemPlanArtifactVersions}
selectedVersion={displayedWorkItemPlanArtifactVersion?.version ?? null}
currentArtifact={workItemPlanArtifact}
onSelectVersion={setSelectedWorkItemPlanArtifactVersionNumber}
activeNodeType={activeNode?.node_type ?? null}
```

Keep `WorkItemPlanStagedPanel` behavior unchanged for this implementation unless tests require action bar relocation.

- [ ] **Step 4: Verify GREEN**

Run:

```bash
pnpm -C web test -- ChatWorkspacePage.test.tsx
```

Expected: PASS.

## Task 6: Focused Regression And Build Verification

**Files:**
- No new production files unless needed for cleanup.

- [ ] **Step 1: Run component and page tests**

Run:

```bash
pnpm -C web test -- WorkItemPlanArtifactPanel.test.tsx ChatWorkspacePage.test.tsx
```

Expected: PASS.

- [ ] **Step 2: Run full web tests**

Run:

```bash
pnpm -C web test
```

Expected: PASS.

- [ ] **Step 3: Run web build**

Run:

```bash
pnpm -C web build
```

Expected: PASS with TypeScript and Vite build success.

- [ ] **Step 4: Inspect diff**

Run:

```bash
git diff --check
git diff --stat
```

Expected: no whitespace errors; changed files are limited to the Work Item Plan UI/tests and the plan document.

## Self-Review

- Spec coverage: status semantics, Outline, Drafts, version rail, Diff, Review, JSON, Story/Design non-regression are covered by tasks.
- No backend contract changes are planned.
- No unbounded refactor is planned.
- TDD order is explicit for every behavior group.
