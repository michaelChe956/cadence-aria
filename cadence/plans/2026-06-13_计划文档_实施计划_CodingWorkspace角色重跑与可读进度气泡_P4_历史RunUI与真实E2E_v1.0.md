# Coding Workspace 历史 Run UI 与真实 E2E 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 P1/P2/P3 的 role run 闭环基础上，为 Coding Workspace 增加可读的角色运行历史 UI，并用浏览器 E2E 验证历史 run、聊天 run badge、blocked gate retry 与 WebSocket 恢复。

**Architecture:** 前端复用 `roleRuns` snapshot，不新增业务状态源；新增 `RoleRunHistoryPanel` 只负责展示和定位。浏览器 E2E 使用 test controls seed route 创建确定性的初始数据，避免 E2E 被漫长的前置流水线干扰；核心验收仍走真实前端、真实后端、真实 WebSocket、真实 store 和真实 gate response。

**Tech Stack:** Rust 1.95、Axum test controls、serde JSON store、Playwright、React、Zustand、Vitest、lucide-react。

---

## 当前基线

本计划基于 P1/P2/P3 全部完成后的代码。

已存在能力：

- WebSocket `CodingSessionState` 已包含 `role_runs`。
- 前端 `useCodingWorkspaceStore` 已保存 `roleRuns`。
- Tester/Analyst/CodeReviewer/InternalReviewer 的 chat entry metadata 已包含 `role_run_id/run_no`。
- blocked gate retry 已能为 Tester、Analyst、Code Reviewer、Internal Reviewer 创建新的 role run。
- `web/playwright.config.ts` 已启用 API server 和 Vite server，`ARIA_E2E_TEST_CONTROLS=1` 下可使用 test controls。

当前缺口：

- 用户只能从消息气泡里零散看 role 输出，不能整体看到“当前第几次 run、哪个 run 被 superseded、哪个 run blocked、retry 原因是什么”。
- 消息分组标题没有对所有 Coding roles 稳定显示 `Run #n`。
- E2E 没有直接覆盖 Coding role run 历史 UI 和浏览器里的 retry 行为。
- 公开 API 前置创建 confirmed work item 的流程较长，直接串全链路会让 UI E2E 不稳定；需要一个只准备初始数据的 test controls fixture。

## Design Readiness Review

当前 design 符合实施落地条件。

- P4 不新增后端执行语义，主要消费 P1/P2/P3 已产出的 role run contract。
- UI 改动集中在 Coding Workspace 页面，不影响 Story/Design/Work Item workspace 共享链路；三模块联动规则不适用。
- E2E fixture 只创建确定性初始数据，之后点击 retry、WebSocket 恢复、role run 更新、chat entry 展示都走真实产品路径。
- 单 session 可完成：一个展示组件、页面集成、一个 test controls seed route、两条 Playwright 用例和集中验证。

不做范围：

- 不再改 Tester/Analyst/Reviewer 的 provider prompt。
- 不新增 role run 查询 API；继续走 WebSocket snapshot。
- 不做复杂过滤器、搜索、分页；当前历史 run 数量很小，按角色和时间排序展示即可。
- 不实现任意历史 artifact 内容弹窗；只展示 ref 摘要，点击 artifact 内容留给后续专门计划。

## File Structure

- Create: `web/src/components/coding-workspace/RoleRunHistoryPanel.tsx`
  - 展示 role run 列表、状态、trigger、node title、reason、raw/artifact refs。
  - 提供 `onSelectNode(nodeId)` 回调，点击 run 可定位到 timeline/chat。

- Create: `web/src/components/coding-workspace/RoleRunHistoryPanel.test.tsx`
  - 覆盖 running/completed/blocked/superseded 展示。
  - 覆盖 refs 和 node title。
  - 覆盖点击定位。

- Modify: `web/src/pages/CodingWorkspacePage.tsx`
  - 在“运行对话”面板中插入 role run history。
  - 把 run history 与 timeline/chat selection 串起来。

- Modify: `web/src/components/chat-workspace/MessageGroupView.tsx`
  - 所有 Coding role 消息分组标题统一显示 `Run #n`。

- Modify: `web/src/components/chat-workspace/MessageGroupView.test.tsx`
  - 覆盖 Tester、Analyst、Code Reviewer、Internal Reviewer 的 run badge。

- Modify: `src/web/test_controls.rs`
  - 新增 coding role run fixture seed handler。
  - 返回 `attempt_id`、`project_id`、`issue_id`。

- Modify: `src/web/app.rs`
  - 在 test controls enabled 时注册 seed route。

- Modify: `tests/it_web/web_test_controls.rs`
  - 验证 test controls route 只在 E2E 环境可用，并能创建 role run fixture。

- Create: `web/e2e/helpers/coding.ts`
  - 提供 `seedCodingRoleRunFixture`、`enableCodingReviewFixture`、`openCodingAttempt`。

- Create: `web/e2e/coding-role-runs.spec.ts`
  - 验证历史 run UI。
  - 验证浏览器点击 retry 后 WebSocket 更新 role run。

## Task 1: RoleRunHistoryPanel

**Files:**
- Create: `web/src/components/coding-workspace/RoleRunHistoryPanel.tsx`
- Create: `web/src/components/coding-workspace/RoleRunHistoryPanel.test.tsx`

- [ ] **Step 1: RED - component renders role run history**

Create `web/src/components/coding-workspace/RoleRunHistoryPanel.test.tsx`:

```tsx
import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { CodingRoleRun, CodingTimelineNode } from "../../api/types";
import { RoleRunHistoryPanel } from "./RoleRunHistoryPanel";

describe("RoleRunHistoryPanel", () => {
  it("renders run status, trigger, refs and node title", () => {
    render(
      <RoleRunHistoryPanel
        roleRuns={[
          roleRun({
            id: "coding_role_run_0001",
            role: "tester",
            stage: "testing",
            run_no: 1,
            status: "superseded",
            trigger: "initial",
            node_id: "coding_node_0003",
            superseded_by_run_id: "coding_role_run_0002",
            reason_code: "test_plan_missing_json",
            raw_provider_output_refs: ["provider-raw/testing/plan_tests_0001.txt"],
          }),
          roleRun({
            id: "coding_role_run_0002",
            role: "tester",
            stage: "testing",
            run_no: 2,
            status: "completed",
            trigger: "retry_test_plan",
            node_id: "coding_node_0004",
            artifact_refs: ["provider-raw/testing/testing_report_0002.json"],
          }),
        ]}
        timelineNodes={[
          node("coding_node_0003", "执行测试"),
          node("coding_node_0004", "执行测试重跑"),
        ]}
        selectedNodeId={null}
        onSelectNode={vi.fn()}
      />,
    );

    const panel = screen.getByTestId("coding-role-run-history");
    expect(panel).toHaveTextContent("角色运行历史");
    expect(panel).toHaveTextContent("Tester #1");
    expect(panel).toHaveTextContent("已被替代");
    expect(panel).toHaveTextContent("initial");
    expect(panel).toHaveTextContent("test_plan_missing_json");
    expect(panel).toHaveTextContent("provider-raw/testing/plan_tests_0001.txt");
    expect(panel).toHaveTextContent("Tester #2");
    expect(panel).toHaveTextContent("已完成");
    expect(panel).toHaveTextContent("retry_test_plan");
    expect(panel).toHaveTextContent("执行测试重跑");
  });

  it("selects the linked timeline node", () => {
    const onSelectNode = vi.fn();
    render(
      <RoleRunHistoryPanel
        roleRuns={[roleRun({ node_id: "coding_node_0005" })]}
        timelineNodes={[node("coding_node_0005", "Analyst 路由决策")]}
        selectedNodeId={null}
        onSelectNode={onSelectNode}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: /Analyst #1/ }));

    expect(onSelectNode).toHaveBeenCalledWith("coding_node_0005");
  });
});

function roleRun(overrides: Partial<CodingRoleRun> = {}): CodingRoleRun {
  return {
    id: "coding_role_run_0001",
    attempt_id: "coding_attempt_0001",
    stage: "rework",
    role: "analyst",
    run_no: 1,
    status: "blocked",
    trigger: "retry_analyst",
    node_id: "coding_node_0005",
    started_at: "2026-06-13T00:00:00Z",
    completed_at: null,
    supersedes_run_id: null,
    superseded_by_run_id: null,
    reason_code: "analyst_human_gate",
    raw_provider_output_refs: [],
    artifact_refs: [],
    ...overrides,
  };
}

function node(id: string, title: string): CodingTimelineNode {
  return {
    id,
    attempt_id: "coding_attempt_0001",
    stage: "rework",
    title,
    status: "blocked",
    agent_role: "system",
    summary: null,
    started_at: "2026-06-13T00:00:00Z",
    completed_at: null,
    artifact_refs: [],
  };
}
```

Run:

```bash
pnpm -C web exec vitest --run src/components/coding-workspace/RoleRunHistoryPanel.test.tsx
```

Expected: FAIL because the component does not exist.

- [ ] **Step 2: GREEN - create component**

Create `web/src/components/coding-workspace/RoleRunHistoryPanel.tsx` with these exported labels and component:

```tsx
import { Circle, CircleCheck, CircleDot, History, RotateCcw, XCircle } from "lucide-react";
import type { CodingRoleRun, CodingTimelineNode } from "../../api/types";

interface RoleRunHistoryPanelProps {
  roleRuns: CodingRoleRun[];
  timelineNodes: CodingTimelineNode[];
  selectedNodeId: string | null;
  onSelectNode: (nodeId: string) => void;
}

export function RoleRunHistoryPanel({
  roleRuns,
  timelineNodes,
  selectedNodeId,
  onSelectNode,
}: RoleRunHistoryPanelProps) {
  const ordered = [...roleRuns].sort((a, b) =>
    a.started_at === b.started_at ? a.run_no - b.run_no : a.started_at.localeCompare(b.started_at),
  );
  const nodeTitleById = new Map(timelineNodes.map((node) => [node.id, node.title]));

  return (
    <section
      data-testid="coding-role-run-history"
      aria-label="角色运行历史"
      className="border-b border-[var(--aria-line)] bg-white px-3 py-2"
    >
      <div className="mb-2 flex min-w-0 items-center gap-2 text-xs font-semibold text-[var(--aria-ink)]">
        <History className="h-3.5 w-3.5" />
        <span>角色运行历史</span>
      </div>
      {ordered.length === 0 ? (
        <div className="text-xs text-[var(--aria-ink-muted)]">暂无角色运行记录</div>
      ) : (
        <div className="flex min-w-0 gap-2 overflow-x-auto pb-1">
          {ordered.map((run) => {
            const selected = run.node_id !== null && run.node_id === selectedNodeId;
            const title = run.node_id ? nodeTitleById.get(run.node_id) ?? run.node_id : "未绑定节点";
            return (
              <button
                key={run.id}
                type="button"
                disabled={!run.node_id}
                onClick={() => run.node_id && onSelectNode(run.node_id)}
                className={[
                  "grid min-w-[13rem] max-w-[18rem] gap-1 rounded-md border px-2 py-1.5 text-left text-xs",
                  selected
                    ? "border-[var(--aria-primary)] bg-[var(--aria-primary-soft)]"
                    : "border-[var(--aria-line)] bg-[var(--aria-panel-muted)] hover:bg-white",
                ].join(" ")}
              >
                <div className="flex min-w-0 items-center justify-between gap-2">
                  <span className="truncate font-semibold text-[var(--aria-ink)]">
                    {roleRunTitle(run)}
                  </span>
                  <span className="inline-flex shrink-0 items-center gap-1 text-[var(--aria-ink-muted)]">
                    {statusIcon(run.status)}
                    {roleRunStatusLabel(run.status)}
                  </span>
                </div>
                <div className="truncate text-[var(--aria-ink-muted)]">{title}</div>
                <div className="truncate font-mono text-[var(--aria-ink-muted)]">
                  {run.trigger}
                </div>
                {run.reason_code ? (
                  <div className="truncate text-[var(--aria-ink-muted)]">{run.reason_code}</div>
                ) : null}
                <RefsSummary run={run} />
              </button>
            );
          })}
        </div>
      )}
    </section>
  );
}

function RefsSummary({ run }: { run: CodingRoleRun }) {
  const refs = [...run.raw_provider_output_refs, ...run.artifact_refs];
  if (refs.length === 0) return null;
  return (
    <div className="grid gap-0.5">
      {refs.slice(0, 2).map((ref) => (
        <div key={ref} className="truncate font-mono text-[10px] text-[var(--aria-ink-muted)]">
          {ref}
        </div>
      ))}
      {refs.length > 2 ? (
        <div className="text-[10px] text-[var(--aria-ink-muted)]">+{refs.length - 2} refs</div>
      ) : null}
    </div>
  );
}

export function roleRunTitle(run: CodingRoleRun) {
  return `${roleLabel(run.role)} #${run.run_no}`;
}

export function roleRunStatusLabel(status: CodingRoleRun["status"]) {
  const labels: Record<CodingRoleRun["status"], string> = {
    running: "运行中",
    completed: "已完成",
    failed: "失败",
    blocked: "阻塞",
    superseded: "已被替代",
    aborted: "已终止",
  };
  return labels[status];
}

function roleLabel(role: CodingRoleRun["role"]) {
  const labels: Record<CodingRoleRun["role"], string> = {
    coder: "Coder",
    tester: "Tester",
    analyst: "Analyst",
    code_reviewer: "Code Reviewer",
    internal_reviewer: "Internal Reviewer",
  };
  return labels[role];
}

function statusIcon(status: CodingRoleRun["status"]) {
  if (status === "running") return <CircleDot className="h-3 w-3" />;
  if (status === "completed") return <CircleCheck className="h-3 w-3" />;
  if (status === "superseded") return <RotateCcw className="h-3 w-3" />;
  if (status === "failed" || status === "aborted") return <XCircle className="h-3 w-3" />;
  return <Circle className="h-3 w-3" />;
}
```

- [ ] **Step 3: Run component test**

```bash
pnpm -C web exec vitest --run src/components/coding-workspace/RoleRunHistoryPanel.test.tsx
```

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add web/src/components/coding-workspace/RoleRunHistoryPanel.tsx web/src/components/coding-workspace/RoleRunHistoryPanel.test.tsx
git commit -m "feat: add coding role run history panel"
```

## Task 2: Integrate RoleRunHistoryPanel In Coding Workspace

**Files:**
- Modify: `web/src/pages/CodingWorkspacePage.tsx`
- Test: `web/src/pages/CodingWorkspacePage.test.tsx`

- [ ] **Step 1: RED - page renders role run history**

Add to `web/src/pages/CodingWorkspacePage.test.tsx`:

```tsx
it("renders role run history and selects linked timeline nodes", async () => {
  mockCodingWs();
  useCodingWorkspaceStore.setState({
    attemptId: "coding_attempt_0001",
    status: "blocked",
    stage: "rework",
    timelineNodes: [
      {
        id: "coding_node_0003",
        attempt_id: "coding_attempt_0001",
        stage: "testing",
        title: "执行测试",
        status: "completed",
        agent_role: "tester",
        summary: "测试阻塞",
        started_at: "2026-06-13T00:00:00Z",
        completed_at: "2026-06-13T00:00:01Z",
        artifact_refs: [],
      },
      {
        id: "coding_node_0004",
        attempt_id: "coding_attempt_0001",
        stage: "rework",
        title: "Analyst 路由决策",
        status: "blocked",
        agent_role: "system",
        summary: "需要人工处理",
        started_at: "2026-06-13T00:00:02Z",
        completed_at: null,
        artifact_refs: [],
      },
    ],
    roleRuns: [
      {
        id: "coding_role_run_0001",
        attempt_id: "coding_attempt_0001",
        stage: "testing",
        role: "tester",
        run_no: 1,
        status: "completed",
        trigger: "initial",
        node_id: "coding_node_0003",
        started_at: "2026-06-13T00:00:00Z",
        completed_at: "2026-06-13T00:00:01Z",
        reason_code: null,
        raw_provider_output_refs: ["provider-raw/testing/plan_tests_0001.txt"],
        artifact_refs: [],
      },
      {
        id: "coding_role_run_0002",
        attempt_id: "coding_attempt_0001",
        stage: "rework",
        role: "analyst",
        run_no: 1,
        status: "blocked",
        trigger: "retry_analyst",
        node_id: "coding_node_0004",
        started_at: "2026-06-13T00:00:02Z",
        completed_at: null,
        reason_code: "analyst_human_gate",
        raw_provider_output_refs: [],
        artifact_refs: ["provider-raw/rework/analyst_evidence_0001.txt"],
      },
    ],
  });

  render(<CodingWorkspacePage attemptId="coding_attempt_0001" onBack={vi.fn()} />);

  const panel = screen.getByTestId("coding-role-run-history");
  expect(panel).toHaveTextContent("Tester #1");
  expect(panel).toHaveTextContent("provider-raw/testing/plan_tests_0001.txt");
  expect(panel).toHaveTextContent("Analyst #1");
  expect(panel).toHaveTextContent("analyst_human_gate");

  await userEvent.click(screen.getByRole("button", { name: /Analyst #1/ }));

  expect(useCodingWorkspaceStore.getState().selectedNodeId).toBe("coding_node_0004");
});
```

Run:

```bash
pnpm -C web exec vitest --run src/pages/CodingWorkspacePage.test.tsx
```

Expected: FAIL because page does not render the panel.

- [ ] **Step 2: GREEN - integrate panel**

In `web/src/pages/CodingWorkspacePage.tsx`, import:

```tsx
import { RoleRunHistoryPanel } from "../components/coding-workspace/RoleRunHistoryPanel";
```

Change the chat panel grid rows from:

```tsx
<div className="grid min-h-0 grid-rows-[auto_minmax(0,1fr)_auto_auto]">
```

to:

```tsx
<div className="grid min-h-0 grid-rows-[auto_auto_minmax(0,1fr)_auto_auto]">
```

Insert after `CodingProviderConfigPanel`:

```tsx
<RoleRunHistoryPanel
  roleRuns={store.roleRuns}
  timelineNodes={store.timelineNodes}
  selectedNodeId={store.selectedNodeId}
  onSelectNode={(nodeId) => {
    useCodingWorkspaceStore.getState().setSelectedNode(nodeId);
    const targetEntry = useCodingWorkspaceStore
      .getState()
      .chatEntries.find((entry) => entry.node_id === nodeId);
    if (targetEntry) {
      chatListRef.current?.scrollToEntry(targetEntry.id);
    }
  }}
/>
```

- [ ] **Step 3: Run page test**

```bash
pnpm -C web exec vitest --run src/pages/CodingWorkspacePage.test.tsx
```

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add web/src/pages/CodingWorkspacePage.tsx web/src/pages/CodingWorkspacePage.test.tsx
git commit -m "feat: show role run history in coding workspace"
```

## Task 3: Unified Run Badge In Message Groups

**Files:**
- Modify: `web/src/components/chat-workspace/MessageGroupView.tsx`
- Test: `web/src/components/chat-workspace/MessageGroupView.test.tsx`

- [ ] **Step 1: RED - all Coding roles show run badge**

Add to `web/src/components/chat-workspace/MessageGroupView.test.tsx`:

```tsx
it.each([
  ["tester", "Tester · Fake · Run #2"],
  ["analyst", "Analyst · Fake · Run #3"],
  ["code_reviewer", "Code Reviewer · Fake · Run #4"],
  ["internal_reviewer", "Internal Reviewer · Fake · Run #5"],
] as const)("shows run number in %s group title", (role, expectedTitle) => {
  const runNo = Number(expectedTitle.match(/#(\d+)/)?.[1]);
  render(
    <MessageGroupView
      group={{
        id: `group-${role}`,
        nodeId: "coding_node_0001",
        role,
        primaryEntry: makeEntry(
          `entry-${role}`,
          "provider_stream",
          role,
          "Readable provider output",
          {
            provider: "fake",
            role_run_id: `coding_role_run_${runNo}`,
            run_no: runNo,
          },
        ),
        inlineEvents: [],
        interruptEntries: [],
      }}
    />,
  );

  expect(screen.getByText(expectedTitle)).toBeInTheDocument();
});
```

Run:

```bash
pnpm -C web exec vitest --run src/components/chat-workspace/MessageGroupView.test.tsx
```

Expected: FAIL until `Run #n` is added for all role groups.

- [ ] **Step 2: GREEN - append run number to group title**

In `MessageGroupView.tsx`, update `groupTitle`:

```tsx
function groupTitle(group: MessageGroup) {
  const base = ROLE_LABELS[group.role] ?? group.role;
  const provider = providerForGroup(group);
  const runNo = runNoForGroup(group);
  return [base, provider ? providerLabel(provider) : null, runNo ? `Run #${runNo}` : null]
    .filter(Boolean)
    .join(" · ");
}
```

Add:

```tsx
function runNoForGroup(group: MessageGroup) {
  const entries = [
    group.primaryEntry,
    ...group.inlineEvents,
    ...group.interruptEntries,
  ].filter((entry): entry is ChatEntry => Boolean(entry));
  for (const entry of entries) {
    const runNo = entry.metadata?.run_no;
    if (typeof runNo === "number" && Number.isFinite(runNo)) {
      return runNo;
    }
  }
  return null;
}
```

- [ ] **Step 3: Run test**

```bash
pnpm -C web exec vitest --run src/components/chat-workspace/MessageGroupView.test.tsx
```

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add web/src/components/chat-workspace/MessageGroupView.tsx web/src/components/chat-workspace/MessageGroupView.test.tsx
git commit -m "feat: show coding run badges in chat groups"
```

## Task 4: Coding RoleRun E2E Fixture

**Files:**
- Modify: `src/web/test_controls.rs`
- Modify: `src/web/app.rs`
- Modify: `tests/it_web/web_test_controls.rs`

- [ ] **Step 1: RED - test control seed route**

Add to `tests/it_web/web_test_controls.rs`:

```rust
#[tokio::test]
async fn coding_role_run_fixture_seed_route_creates_attempt_with_runs() {
    let _guard = ENV_LOCK.lock().await;
    unsafe {
        std::env::set_var("ARIA_E2E_TEST_CONTROLS", "1");
    }
    let root = tempdir().expect("root");
    let state = WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    );
    let app = build_web_router(state);

    let body = request_json(
        app.clone(),
        Method::POST,
        "/api/test/coding-attempts/role-run-fixture",
        json!({"blocked_stage":"rework"}),
    )
    .await;

    assert_eq!(body["attempt_id"], "coding_attempt_0001");
    assert_eq!(body["project_id"], "project_0001");
    assert_eq!(body["issue_id"], "issue_0001");

    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let runs = store
        .list_role_runs("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("role runs");
    assert_eq!(runs.len(), 2);
    assert!(runs.iter().any(|run| run.role == CodingProviderRole::Tester));
    assert!(runs.iter().any(|run| run.role == CodingProviderRole::Analyst));

    unsafe {
        std::env::remove_var("ARIA_E2E_TEST_CONTROLS");
    }
}
```

Add imports for `CodingAttemptStore`, `CodingProviderRole`, and `ProductAppPaths` at the top of `tests/it_web/web_test_controls.rs`.

Run:

```bash
cargo test --locked --test it_web coding_role_run_fixture_seed_route_creates_attempt_with_runs
```

Expected: FAIL because the route does not exist.

- [ ] **Step 2: GREEN - add route**

In `src/web/app.rs`, inside the `test_controls_enabled()` router block, add:

```rust
.route(
    "/api/test/coding-attempts/role-run-fixture",
    post(test_controls::seed_coding_role_run_fixture),
)
```

In `src/web/test_controls.rs`, add request/response structs:

```rust
#[derive(Debug, Deserialize)]
pub struct CodingRoleRunFixtureRequest {
    #[serde(default = "default_blocked_stage")]
    pub blocked_stage: String,
}

fn default_blocked_stage() -> String {
    "rework".to_string()
}
```

Add handler:

```rust
pub async fn seed_coding_role_run_fixture(
    State(state): State<WebAppState>,
    Json(request): Json<CodingRoleRunFixtureRequest>,
) -> Json<serde_json::Value> {
    match create_coding_role_run_fixture(
        ProductAppPaths::new(state.workspace_root.join(".aria")),
        &state.workspace_root,
        &request.blocked_stage,
    ) {
        Ok(value) => Json(value),
        Err(error) => Json(json!({"error": error.to_string()})),
    }
}
```

Implement `create_coding_role_run_fixture` to:

- initialize a git repo under `state.workspace_root.join("coding-role-run-fixture-repo")`;
- create project `project_0001`, repository `repository_0001`, issue `issue_0001`, confirmed work item `work_item_0001`（可参考 `seed_large_workspace_fixture` 中的 `ProjectStore::new(...).create(...)`、`RepositoryStore::new(...).create(...)`、`IssueStore::new(...).create(...)` 调用模式，并确认 work item 状态为 `confirmed`）；
- create `coding_attempt_0001` with `status = Blocked`, `stage = Rework` when request is `rework`;
- create timeline node `coding_node_0001` for Testing and `coding_node_0002` for Rework;
- create role run #1 Tester completed with raw ref `provider-raw/testing/plan_tests_0001.txt`;
- create role run #1 Analyst blocked with artifact ref `provider-raw/rework/analyst_evidence_0001.txt`;
- save chat entries for Tester and Analyst with `role_run_id/run_no`;
- create blocked gate `coding_blocked_gate_0001` with `retry_analyst`, `manual_continue`, `abort`;
- write raw output files through `save_provider_raw_output`;
- return:

```rust
json!({
    "status": "ok",
    "project_id": project.id,
    "issue_id": issue.id,
    "attempt_id": attempt.id
})
```

When `blocked_stage == "internal_pr_review"`, create the same project/repository/issue/work item/attempt baseline, plus:

- save a `ReviewRequest` for `coding_attempt_0001` before creating the blocked reviewer state:

```rust
ReviewRequest {
    id: "review_request_0001".to_string(),
    attempt_id: "coding_attempt_0001".to_string(),
    kind: ReviewRequestKind::GitBranchOnly,
    remote_kind: RemoteKind::GenericGit,
    remote: "origin".to_string(),
    base_branch: "main".to_string(),
    branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
    commit_sha: "e2e-fixture-commit".to_string(),
    push_status: PushStatus::Pushed,
    external_url: None,
    manual_instructions: vec!["E2E fixture review request".to_string()],
    created_at: now.clone(),
    updated_at: now.clone(),
}
```

- create `coding_attempt_0001` with `status = Blocked`, `stage = InternalPrReview`;
- create an InternalReviewer blocked role run with `trigger = Initial`, `status = Blocked`, and raw ref `provider-raw/internal_pr_review/internal_pr_review_0001.txt`;
- create a blocked gate with `stage = InternalPrReview`, `role = InternalReviewer`, and recovery action `retry_review`;
- save an InternalReviewer chat entry with `role_run_id/run_no`.

The saved `ReviewRequest` is required because `execute_internal_pr_review_with_commands` reads the latest review request before invoking the InternalReviewer provider.

The fixture only prepares deterministic initial data. It must not bypass or fake the retry handler, WebSocket transport, store update, or frontend rendering that the browser E2E verifies.

- [ ] **Step 3: Run route test**

```bash
cargo test --locked --test it_web coding_role_run_fixture_seed_route_creates_attempt_with_runs
```

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add src/web/test_controls.rs src/web/app.rs tests/it_web/web_test_controls.rs
git commit -m "test: seed coding role run e2e fixture"
```

## Task 5: Browser E2E For History And Retry

**Files:**
- Create: `web/e2e/helpers/coding.ts`
- Create: `web/e2e/coding-role-runs.spec.ts`

- [ ] **Step 1: RED - Playwright helpers and tests**

Create `web/e2e/helpers/coding.ts`:

```ts
import { expect, type Page } from "@playwright/test";

export async function seedCodingRoleRunFixture(
  page: Page,
  blockedStage: "rework" | "internal_pr_review" = "rework",
): Promise<{ attemptId: string; projectId: string; issueId: string }> {
  const response = await page.request.post("/api/test/coding-attempts/role-run-fixture", {
    data: { blocked_stage: blockedStage },
  });
  expect(response).toBeOK();
  const body = await response.json();
  expect(body.status).toBe("ok");
  return {
    attemptId: body.attempt_id as string,
    projectId: body.project_id as string,
    issueId: body.issue_id as string,
  };
}

export async function enableCodingReviewFixture(page: Page, attemptId: string, rawJson: unknown) {
  const response = await page.request.post(
    `/api/test/coding-attempts/${encodeURIComponent(attemptId)}/review-fixture`,
    {
      data: {
        verdict: "approve",
        summary: "fixture",
        comments: "fixture",
        raw_json: rawJson,
      },
    },
  );
  expect(response).toBeOK();
}

export async function openCodingAttempt(page: Page, attemptId: string) {
  await page.goto(`/workbench/coding/${encodeURIComponent(attemptId)}`);
  await expect(page.getByText(`Coding Attempt #${attemptId}`)).toBeVisible();
  await expect(page.getByTestId("coding-role-run-history")).toBeVisible();
}
```

Create `web/e2e/coding-role-runs.spec.ts`:

```ts
import { expect, test } from "@playwright/test";
import {
  enableCodingReviewFixture,
  openCodingAttempt,
  seedCodingRoleRunFixture,
} from "./helpers/coding";

test("coding role run history renders seeded runs and chat badges", async ({ page }) => {
  const seeded = await seedCodingRoleRunFixture(page, "rework");

  await openCodingAttempt(page, seeded.attemptId);

  const history = page.getByTestId("coding-role-run-history");
  await expect(history).toContainText("Tester #1");
  await expect(history).toContainText("Analyst #1");
  await expect(history).toContainText("阻塞");
  await expect(history).toContainText("provider-raw/rework/analyst_evidence");
  await expect(page.getByTestId("chat-entry-list")).toContainText("Run #1");
  await expect(page.getByTestId("coding-pending-gate")).toContainText("重试 Analyst");
});

test("retry analyst from browser gate creates a new visible run", async ({ page }) => {
  const seeded = await seedCodingRoleRunFixture(page, "rework");
  await enableCodingReviewFixture(page, seeded.attemptId, {
    verdict: "proceed",
    next_stage: "code_review",
    reason: "retry analyst accepted from browser",
    evidence_refs: ["provider-raw/rework/analyst_evidence_0001.txt"],
    raw_provider_output_refs: [],
  });

  await openCodingAttempt(page, seeded.attemptId);
  await page.getByRole("button", { name: "重试 Analyst" }).click();

  const history = page.getByTestId("coding-role-run-history");
  await expect(history).toContainText("Analyst #2", { timeout: 30_000 });
  await expect(history).toContainText("retry_analyst");
  await expect(page.getByTestId("chat-entry-list")).toContainText("retry analyst accepted from browser", {
    timeout: 30_000,
  });
});

test("retry internal reviewer from browser gate stays on internal review run", async ({ page }) => {
  const seeded = await seedCodingRoleRunFixture(page, "internal_pr_review");
  await enableCodingReviewFixture(page, seeded.attemptId, {
    verdict: "approve",
    summary: "internal reviewer retry accepted",
    findings: [],
    impact_scope: ["src/lib.rs"],
    pr_description: "PR ready",
    commit_message_suggestion: "feat: work",
  });

  await openCodingAttempt(page, seeded.attemptId);
  await page.getByRole("button", { name: "重试审查" }).click();

  const history = page.getByTestId("coding-role-run-history");
  await expect(history).toContainText("Internal Reviewer #2", { timeout: 30_000 });
  await expect(history).toContainText("retry_internal_review");
  await expect(history).not.toContainText("Code Reviewer #2");
});
```

Run:

```bash
pnpm -C web exec playwright test e2e/coding-role-runs.spec.ts
```

Expected: FAIL before Task 1-4 are implemented.

- [ ] **Step 2: GREEN - make tests pass with real UI + WebSocket**

After Tasks 1-4 are implemented, run:

```bash
pnpm -C web exec playwright test e2e/coding-role-runs.spec.ts
```

Expected: PASS. The test may take up to 120 seconds because Playwright starts the API server through `web/e2e/start-api.mjs`.

- [ ] **Step 3: Commit**

```bash
git add web/e2e/helpers/coding.ts web/e2e/coding-role-runs.spec.ts
git commit -m "test: cover coding role run history e2e"
```

## Verification

Run focused checks:

```bash
pnpm -C web exec vitest --run src/components/coding-workspace/RoleRunHistoryPanel.test.tsx src/components/chat-workspace/MessageGroupView.test.tsx src/pages/CodingWorkspacePage.test.tsx
cargo test --locked --test it_web coding_role_run_fixture_seed_route_creates_attempt_with_runs
pnpm -C web exec playwright test e2e/coding-role-runs.spec.ts
```

Run full local gate:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo check --locked
cargo test --locked
pnpm -C web exec vitest --run
pnpm -C web exec playwright test e2e/coding-role-runs.spec.ts
```

Do not use Docker. Do not add `-j 1` to any Cargo command.

## Implementation Handoff

This plan should be executed after P2 and P3.

Recommended execution mode:

1. Use `superpowers:subagent-driven-development` for Task 1-5.
2. Commit after each task.
3. Stop after verification and report exact commands/results.
