# Aria Web 工作台 P4 工作台 UI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 实现高密度 Web 工作台浏览界面：TopStatusBar、FlowRail、NodeWorkspace、EvidencePanel、DiagnosticsPanel、ArtifactViewer 和 rich content renderer。

**Architecture:** P4 只做浏览和诊断 UI，不实现 provider 确认和 rollback 控制。组件读取 P1/P2/P3 提供的 projection、artifact、diagnostics 和 event 数据。

**Tech Stack:** React、TypeScript、Tailwind CSS、Radix tabs/dialog primitives、lucide-react、react-markdown、Vitest、Testing Library。

---

## Design Coverage

P4 覆盖：

- 顶部状态栏：workspace、task、change、phase/status、当前节点/worktask、policy、provider、git、SSE、running state
- `blocked_by_gate` 拆解：business code、unit tests、coverage gate、archive/integration gate、root cause
- Flow Rail：N00-N28、状态、provider、attempt/rework、artifact count、gate/diagnostic
- Node Workspace：Overview、Inputs、Run、Outputs、Diff
- Timeline/Changes：turn、checkpoint、changed files、diff、dropped history browse state
- Evidence Panel：OpenSpec、Aria artifacts、reports、provider records、testing/final/blocked report、node-events、源码和测试
- Markdown 目录/锚点、JSON 长字段折叠、source/test/log 渲染
- Diagnostics 分类展示

## Source Tasks From Master Plan

| Master Task | Scope |
|------|------|
| Task 11 | workbench layout components |
| Task 13.5 | rich artifact content rendering |
| Task 15 Step 3 | Fibonacci browsing UI fixture assertion；backend acceptance remains P6 |

## Files

| Path | Responsibility |
|------|------|
| `web/src/components/shell/TopStatusBar.tsx` | status bar |
| `web/src/components/shell/TaskSwitcher.tsx` | continue task selector |
| `web/src/components/flow/FlowRail.tsx` | node flow |
| `web/src/components/node/NodeWorkspace.tsx` | node tabs |
| `web/src/components/evidence/EvidencePanel.tsx` | evidence list |
| `web/src/components/evidence/ArtifactViewer.tsx` | artifact content shell |
| `web/src/components/evidence/ArtifactContentRenderer.tsx` | markdown/json/source renderer |
| `web/src/components/diagnostics/DiagnosticsPanel.tsx` | grouped diagnostics |
| `web/src/main.tsx` | layout composition |
| `web/package.json` | react-markdown dependency |

## Tasks

### Task P4.1: Workbench Layout Components

- [ ] **Step 1: Execute master Task 11**

Run:

```bash
pnpm --dir web test -- --run web/src/components/shell/TopStatusBar.test.tsx web/src/components/shell/TaskSwitcher.test.tsx web/src/components/flow/FlowRail.test.tsx web/src/components/evidence/EvidencePanel.test.tsx
pnpm --dir web build
```

Expected: PASS, including Timeline/Changes browse affordance for changed files, checkpoint and dropped turn state.

- [ ] **Step 2: Commit**

```bash
git add web/src/components/shell/TopStatusBar.tsx web/src/components/shell/TaskSwitcher.tsx web/src/components/flow/FlowRail.tsx web/src/components/node/NodeWorkspace.tsx web/src/components/evidence/EvidencePanel.tsx web/src/components/diagnostics/DiagnosticsPanel.tsx web/src/main.tsx web/src/components/shell/TopStatusBar.test.tsx web/src/components/shell/TaskSwitcher.test.tsx web/src/components/flow/FlowRail.test.tsx web/src/components/evidence/EvidencePanel.test.tsx
git commit -m "feat: add aria web workbench layout"
```

### Task P4.2: Rich Artifact Rendering

- [ ] **Step 1: Execute master Task 13.5**

Run:

```bash
pnpm --dir web test -- --run web/src/components/evidence/ArtifactContentRenderer.test.tsx web/src/components/evidence/ArtifactViewer.test.tsx
pnpm --dir web build
```

Expected: PASS.

- [ ] **Step 2: Commit**

```bash
git add web/package.json web/pnpm-lock.yaml web/src/components/evidence/ArtifactContentRenderer.tsx web/src/components/evidence/ArtifactViewer.tsx web/src/components/evidence/ArtifactContentRenderer.test.tsx
git commit -m "feat: add aria web artifact renderer"
```

### Task P4.3: Fibonacci Browse UI Diagnostics Hook

- [ ] **Step 1: Execute master Task 15 Step 3**

Run:

```bash
pnpm --dir web test -- --run web/src/components/shell/TopStatusBar.test.tsx web/src/components/evidence/EvidencePanel.test.tsx web/src/components/diagnostics/DiagnosticsPanel.test.tsx
pnpm --dir web build
```

Expected: PASS and UI fixture diagnostics render `archive_worktask` and `write scope contract` without backend changes.

- [ ] **Step 2: Commit**

```bash
git add web/src/components/shell/TopStatusBar.tsx web/src/components/evidence/EvidencePanel.tsx web/src/components/diagnostics/DiagnosticsPanel.tsx web/src/components/shell/TopStatusBar.test.tsx web/src/components/evidence/EvidencePanel.test.tsx web/src/components/diagnostics/DiagnosticsPanel.test.tsx
git commit -m "feat: surface aria web gate diagnostics"
```

## P4 Exit Criteria

Run:

```bash
pnpm --dir web test -- --run web/src/components/shell/TopStatusBar.test.tsx web/src/components/shell/TaskSwitcher.test.tsx web/src/components/flow/FlowRail.test.tsx web/src/components/evidence/EvidencePanel.test.tsx web/src/components/diagnostics/DiagnosticsPanel.test.tsx web/src/components/evidence/ArtifactContentRenderer.test.tsx web/src/components/evidence/ArtifactViewer.test.tsx
pnpm --dir web build
```

Expected: tests and build PASS.

## Self-Review

- [x] P4 first screen remains workbench UI.
- [x] P4 does not copy vibe-kanban visuals.
- [x] P4 covers all read-only TUI information fields from design.
