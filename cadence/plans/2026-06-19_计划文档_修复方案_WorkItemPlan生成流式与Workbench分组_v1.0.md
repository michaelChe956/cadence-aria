# WorkItemPlan 生成流式与 Workbench 分组修复 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修复 WorkItemPlan 生成失败诊断、生成过程无流式气泡、Workbench 平铺子 Work Item 三个问题，并保持 Story Spec、Design Spec、Work Item 共享 Workspace 链路一致。

**Architecture:** 后端 lifecycle API 增加 IssueWorkItemPlan 列表作为 Workbench 的 Work Item Group 数据源；前端 Workbench 的 Work Item 列只展示 group 卡片，group drawer 内展示子 Work Item。WorkItemPlanAuthor 继续复用现有 Workspace WebSocket 协议，通过 `stream_chunk`、`message_complete`、timeline detail 持久化来提供实时气泡和刷新恢复；structured output 修复以 prompt 约束和错误诊断为主，不对缺失 start sentinel 的半截 JSON 做危险容错。

**Tech Stack:** Rust 1.95.0、Axum WebSocket、serde、Vitest、React 19、Zustand、pnpm、Cargo 宿主机命令。

---

## 现状与约束

- 工作目录：`/home/michael/workspace/github/cadence-aria/.worktrees/feat-b-0616`
- 当前分支：`feat-b-0616`
- 当前已有未提交改动：
  - `src/product/work_item_split_engine.rs`
  - `tests/it_product/product_work_item_split_engine.rs`
- 不回退上述改动，执行时必须先读 diff 并在其基础上继续。
- Rust 验证必须在宿主机直接运行 Cargo，禁止 Docker；`cargo test` 禁止 `-j 1`。
- 前端包管理和脚本必须使用 `pnpm`。
- 本次涉及 Work Item Workspace，必须按 `cadence/project-rules/workspace-artifact-bug-triage.md` 同时评估 Story、Design、Work Item 共享链路。

## 文件结构

- Modify: `src/web/types.rs`
  - `IssueLifecycleResponse` 增加 `work_item_plans: Vec<IssueWorkItemPlanDetailDto>`。
- Modify: `src/web/handlers.rs`
  - `issue_lifecycle` 返回 issue 级 WorkItemPlan 列表。
  - 复用现有 `issue_work_item_plan_detail_dto`。
- Modify: `tests/it_web/web_work_item_plan_author.rs` 或新增 `tests/it_web/web_issue_lifecycle_work_item_plan.rs`
  - 覆盖 lifecycle API 返回 plan group。
  - 覆盖 WorkItemPlanAuthor 生成过程至少推送一个流式 chunk。
- Modify: `web/src/api/types.ts`
  - `IssueLifecycleResponse` 增加 `work_item_plans: IssueWorkItemPlanDetailDto[]`。
- Modify: `web/src/api/types.test.ts`
  - 覆盖 lifecycle 响应类型含 `work_item_plans`。
- Modify: `web/src/state/lifecycle-workbench-store.ts`
  - 增加 `work_item_group` card kind。
  - Work Item 列从 `lifecycle.work_item_plans` 构造 group 卡片，不再平铺 `lifecycle.work_items`。
- Modify: `web/src/state/lifecycle-workbench-store.test.ts`
  - 覆盖 group 卡片、子 Work Item 查找、coding 阻塞条件。
- Modify: `web/src/components/lifecycle/IssueLifecycleWorkbench.tsx`
  - Work Item 列展示 group。
  - 点击 group 打开 drawer。
  - group drawer 内展示该 plan 的所有子 Work Item。
  - 子 Work Item 的 Coding 入口仍按子项触发。
- Modify: `web/src/components/lifecycle/LifecycleCardDrawer.tsx`
  - 增加 `work_item_group` drawer 展示。
  - 展示 plan status、source spec ids、validator findings、dependency graph、子 Work Item 列表。
- Modify: `web/src/components/lifecycle/IssueLifecycleWorkbench.test.tsx`
  - 覆盖 Workbench 只展示一个 Work Item Group，点击后展示子 Work Item。
- Modify: `src/web/workspace_ws_handler.rs`
  - WorkItemPlanAuthor / WorkItemPlanRevision 在 dedicated provider run 中发送进度流式消息。
  - 进度消息写入 timeline detail，刷新后可重建。
- Modify: `src/product/workspace_engine.rs`
  - 增加小型 helper 用于 WorkItemPlan dedicated run 追加 provider stream/progress，并发送 `MessageComplete`。
  - 不改变 Story/Design 普通 streaming provider session 行为。
- Modify: `web/src/hooks/useWorkspaceWs.ts`
  - 若后端沿用 `stream_chunk`，前端只补测试；若需要稳定 active stage，则允许 `work_item_plan` workspace 在 `running` 阶段接收 chunk。
- Modify: `web/src/state/workspace-ws-store.test.ts`
  - 覆盖刷新恢复时 WorkItemPlanAuthor 的 `streaming_content` 能重建为 provider stream 气泡。
- Modify: `web/src/pages/ChatWorkspacePage.test.tsx`
  - 覆盖 WorkItemPlan 生成开始后出现流式气泡，最终 candidate panel 仍正常展示。
- Modify: `src/product/work_item_split_engine.rs`
  - 强化 split/revision prompt 的 sentinel 约束。
  - 错误 detail 保留 stdout/stderr 供 UI 或日志定位。
- Modify: `tests/it_product/product_work_item_split_engine.rs`
  - 增加 prompt 回归测试，要求 start/end sentinel 都出现且明确“只输出 sentinel JSON”。
- Modify: `tests/it_provider/provider_adapter_baseline.rs`
  - 保留 parser 对缺失 start sentinel 返回 `None` 的行为，明确不解析半截 JSON。

## Task 1: 后端 lifecycle API 返回 Work Item Group

**Files:**
- Modify: `src/web/types.rs`
- Modify: `src/web/handlers.rs`
- Test: `tests/it_web/web_work_item_plan_author.rs` 或 `tests/it_web/web_issue_lifecycle_work_item_plan.rs`

- [ ] **Step 1: 写失败测试**

在 web integration test 中创建 issue、story、design、issue work item plan、两个 lifecycle work item，然后调用 lifecycle API，断言 `work_item_plans` 返回 1 个 group 且 `work_items` 仍包含子项。

```rust
#[tokio::test]
async fn issue_lifecycle_returns_work_item_plan_groups_with_child_work_items() {
    let app = test_app().await;
    let project_id = "project_0001";
    let issue_id = "issue_0001";

    seed_issue_with_confirmed_story_and_design(&app, project_id, issue_id).await;
    let plan = seed_issue_work_item_plan_with_children(
        &app,
        project_id,
        issue_id,
        ["work_item_frontend", "work_item_backend"],
    )
    .await;

    let response = app
        .get_json::<serde_json::Value>(&format!(
            "/api/product/issues/{issue_id}/lifecycle?project_id={project_id}"
        ))
        .await;

    assert_eq!(response["work_item_plans"].as_array().unwrap().len(), 1);
    assert_eq!(response["work_item_plans"][0]["id"], plan.id);
    assert_eq!(
        response["work_item_plans"][0]["work_item_ids"],
        serde_json::json!(["work_item_frontend", "work_item_backend"])
    );
    assert_eq!(response["work_items"].as_array().unwrap().len(), 2);
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --locked --test web_work_item_plan_author issue_lifecycle_returns_work_item_plan_groups_with_child_work_items`

Expected: FAIL，错误为响应中不存在 `work_item_plans` 或数组长度为 0。

- [ ] **Step 3: 修改 DTO**

在 `src/web/types.rs` 中将 `IssueLifecycleResponse` 改成：

```rust
pub struct IssueLifecycleResponse {
    pub issue: ProductIssueDto,
    pub story_specs: Vec<StorySpecDto>,
    pub design_specs: Vec<DesignSpecDto>,
    pub work_item_plans: Vec<IssueWorkItemPlanDetailDto>,
    pub work_items: Vec<LifecycleWorkItemDto>,
    pub workspace_sessions: Vec<WorkspaceSessionDto>,
    pub coding_attempts: Vec<CodingAttemptDto>,
}
```

- [ ] **Step 4: 修改 handler**

在 `src/web/handlers.rs::issue_lifecycle` 中，`work_items` 构造前后均可插入：

```rust
let work_item_plans = lifecycle
    .list_issue_work_item_plans(&project_id, &issue_id)
    .map_err(product_store_api_error)?
    .into_iter()
    .map(|plan| issue_work_item_plan_detail_dto(&plan))
    .collect::<Vec<_>>();
```

返回体改成：

```rust
Ok(Json(IssueLifecycleResponse {
    issue: product_issue_dto_with_binding(&app_paths, issue)?,
    story_specs,
    design_specs,
    work_item_plans,
    work_items,
    workspace_sessions,
    coding_attempts,
}))
```

- [ ] **Step 5: 运行测试确认通过**

Run: `cargo test --locked --test web_work_item_plan_author issue_lifecycle_returns_work_item_plan_groups_with_child_work_items`

Expected: PASS。

- [ ] **Step 6: 提交**

```bash
git add src/web/types.rs src/web/handlers.rs tests/it_web/web_work_item_plan_author.rs
git commit -m "fix: expose work item plan groups in lifecycle api"
```

## Task 2: 前端 store 将 Work Item 列改为 Group 卡片

**Files:**
- Modify: `web/src/api/types.ts`
- Modify: `web/src/api/types.test.ts`
- Modify: `web/src/state/lifecycle-workbench-store.ts`
- Modify: `web/src/state/lifecycle-workbench-store.test.ts`

- [ ] **Step 1: 写 API 类型失败测试**

在 `web/src/api/types.test.ts` 的 lifecycle response fixture 中加入：

```ts
const response = {
  issue: {} as IssueLifecycleResponse["issue"],
  story_specs: [],
  design_specs: [],
  work_item_plans: [
    {
      plan_id: "issue_work_item_plan_0001",
      project_id: "project_0001",
      issue_id: "issue_0001",
      title: "登录会话过期 Work Item Plan",
      source_story_spec_ids: ["story_spec_0001"],
      source_design_spec_ids: ["design_spec_0001"],
      options: {
        include_integration_tests: true,
        include_e2e_tests: false,
        force_frontend_backend_split: true,
        require_execution_plan_confirm: false,
      },
      status: "draft",
      work_item_ids: ["work_item_frontend", "work_item_backend"],
      repository_profile_ref: null,
      verification_plan_ids: [],
      dependency_graph: [],
      created_from_provider_run: null,
      validator_findings: [],
      review_summary: null,
      created_at: "2026-06-19T00:00:00Z",
      updated_at: "2026-06-19T00:00:00Z",
    },
  ],
  work_items: [],
  workspace_sessions: [],
  coding_attempts: [],
} satisfies IssueLifecycleResponse;

expect(response.work_item_plans[0].work_item_ids).toEqual([
  "work_item_frontend",
  "work_item_backend",
]);
```

- [ ] **Step 2: 写 store 失败测试**

在 `web/src/state/lifecycle-workbench-store.test.ts` 增加：

```ts
it("groups work items under issue work item plan cards", () => {
  const columns = groupLifecycleCards([
    {
      ...lifecycle,
      work_item_plans: [
        {
          plan_id: "issue_work_item_plan_0001",
          project_id: "project_0001",
          issue_id: "issue_0001",
          title: "登录会话过期 Work Item Plan",
          source_story_spec_ids: ["story_spec_0001"],
          source_design_spec_ids: ["design_spec_0001"],
          options: {
            include_integration_tests: true,
            include_e2e_tests: false,
            force_frontend_backend_split: true,
            require_execution_plan_confirm: false,
          },
          status: "draft",
          work_item_ids: ["work_item_frontend", "work_item_backend"],
          repository_profile_ref: null,
          verification_plan_ids: [],
          dependency_graph: [],
          created_from_provider_run: null,
          validator_findings: [],
          review_summary: null,
          created_at: "2026-06-19T00:00:00Z",
          updated_at: "2026-06-19T00:00:00Z",
        },
      ],
      work_items: [
        lifecycleWorkItem({
          work_item_id: "work_item_frontend",
          title: "前端登录提示",
        }),
        lifecycleWorkItem({
          work_item_id: "work_item_backend",
          title: "后端会话状态",
        }),
      ],
    },
  ]);

  expect(columns.work_item).toHaveLength(1);
  expect(columns.work_item[0]).toMatchObject({
    kind: "work_item_group",
    id: "issue_work_item_plan_0001",
    title: "登录会话过期 Work Item Plan",
    status: "draft",
  });
  expect(columns.work_item[0].childWorkItemIds).toEqual([
    "work_item_frontend",
    "work_item_backend",
  ]);
});
```

- [ ] **Step 3: 运行前端测试确认失败**

Run: `cd web && pnpm test -- api/types.test.ts lifecycle-workbench-store.test.ts`

Expected: FAIL，TypeScript 报 `work_item_plans` 或 `work_item_group` 类型不存在。

- [ ] **Step 4: 修改 API 类型**

在 `web/src/api/types.ts` 中修改：

```ts
export type IssueLifecycleResponse = {
  issue: ProductIssue;
  story_specs: StorySpec[];
  design_specs: DesignSpec[];
  work_item_plans: IssueWorkItemPlanDetailDto[];
  work_items: LifecycleWorkItem[];
  workspace_sessions: WorkspaceSession[];
  coding_attempts: CodingAttempt[];
};
```

- [ ] **Step 5: 修改 card union**

在 `web/src/state/lifecycle-workbench-store.ts` 引入 `IssueWorkItemPlanDetailDto`，并新增 card variant：

```ts
| {
    kind: "work_item_group";
    id: string;
    issueId: string;
    title: string;
    status: string;
    version: number | null;
    preview: string | null;
    sourceIds: string[];
    childWorkItemIds: string[];
    artifactVersions: ArtifactVersion[];
    raw: IssueWorkItemPlanDetailDto;
  }
```

在 `groupLifecycleCards` 中将原 `lifecycle.work_items.forEach` 推入 `columns.work_item` 的逻辑替换为：

```ts
lifecycle.work_item_plans.forEach((plan) => {
  columns.work_item.push({
    kind: "work_item_group",
    id: plan.plan_id,
    issueId: plan.issue_id,
    title: plan.title,
    status: plan.status,
    version: null,
    preview: `${plan.work_item_ids.length} 个 Work Item`,
    sourceIds: [...plan.source_story_spec_ids, ...plan.source_design_spec_ids],
    childWorkItemIds: [...plan.work_item_ids],
    artifactVersions: [],
    raw: plan,
  });
});
```

- [ ] **Step 6: 保留 coding 阻塞判断**

`lifecycleBlockedReason("coding")` 继续基于 `lifecycle.work_items.some((item) => item.plan_status === "confirmed")`，因为 coding gate 需要子 Work Item 已 confirmed，而不是 group 存在。

- [ ] **Step 7: 运行测试确认通过**

Run: `cd web && pnpm test -- api/types.test.ts lifecycle-workbench-store.test.ts`

Expected: PASS。

- [ ] **Step 8: 提交**

```bash
git add web/src/api/types.ts web/src/api/types.test.ts web/src/state/lifecycle-workbench-store.ts web/src/state/lifecycle-workbench-store.test.ts
git commit -m "fix: group work items by issue plan in workbench state"
```

## Task 3: Workbench drawer 展示 Group 下属 Work Item

**Files:**
- Modify: `web/src/components/lifecycle/IssueLifecycleWorkbench.tsx`
- Modify: `web/src/components/lifecycle/LifecycleCardDrawer.tsx`
- Modify: `web/src/components/lifecycle/IssueLifecycleWorkbench.test.tsx`

- [ ] **Step 1: 写失败测试**

在 `web/src/components/lifecycle/IssueLifecycleWorkbench.test.tsx` 增加：

```tsx
it("shows one work item group and reveals child work items in the drawer", async () => {
  mockLifecycleApi({
    work_item_plans: [
      makeIssueWorkItemPlan({
        plan_id: "issue_work_item_plan_0001",
        title: "登录会话过期 Work Item Plan",
        work_item_ids: ["work_item_frontend", "work_item_backend"],
      }),
    ],
    work_items: [
      makeLifecycleWorkItem({
        work_item_id: "work_item_frontend",
        title: "前端登录提示",
      }),
      makeLifecycleWorkItem({
        work_item_id: "work_item_backend",
        title: "后端会话状态",
      }),
    ],
  });

  render(<IssueLifecycleWorkbench onOpenWorkspace={vi.fn()} onOpenCodingWorkspace={vi.fn()} />);

  expect(await screen.findByText("登录会话过期 Work Item Plan")).toBeInTheDocument();
  expect(screen.queryByText("前端登录提示")).not.toBeInTheDocument();
  expect(screen.queryByText("后端会话状态")).not.toBeInTheDocument();

  await userEvent.click(screen.getByText("登录会话过期 Work Item Plan"));

  expect(await screen.findByTestId("work-item-group-children")).toHaveTextContent("前端登录提示");
  expect(screen.getByTestId("work-item-group-children")).toHaveTextContent("后端会话状态");
});
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cd web && pnpm test -- IssueLifecycleWorkbench.test.tsx`

Expected: FAIL，当前 Work Item 列直接显示子 Work Item，drawer 无 `work-item-group-children`。

- [ ] **Step 3: 扩展 DrawerEntity**

在 `LifecycleCardDrawer.tsx` 中：

```ts
export type DrawerEntityKind =
  | "issue"
  | "story_spec"
  | "design_spec"
  | "work_item_group"
  | "work_item";

export interface DrawerEntity {
  id: string;
  kind: DrawerEntityKind;
  title: string;
  status: string;
  version: number | null;
  childWorkItems?: LifecycleWorkItem[];
  workItemPlanSourceStorySpecIds?: string[];
  workItemPlanSourceDesignSpecIds?: string[];
  workItemPlanValidatorFindings?: WorkItemSplitFinding[];
  workItemPlanDependencyGraph?: WorkItemDependencyEdgeDto[];
}
```

同时更新 label：

```ts
const KIND_LABELS: Record<DrawerEntityKind, string> = {
  issue: "Issue",
  story_spec: "Story Spec",
  design_spec: "Design Spec",
  work_item_group: "Work Item Group",
  work_item: "Work Item",
};
```

- [ ] **Step 4: 增加 group 详情组件**

在 `LifecycleCardDrawer.tsx` 中新增：

```tsx
function WorkItemGroupDetail({ entity }: { entity: DrawerEntity }) {
  const children = entity.childWorkItems ?? [];

  return (
    <section className="border-t border-[var(--aria-line)] px-4 py-3">
      <h3 className="text-sm font-semibold text-[var(--aria-ink)]">子 Work Item</h3>
      <div data-testid="work-item-group-children" className="mt-2 space-y-2">
        {children.length === 0 ? (
          <p className="text-sm text-[var(--aria-ink-muted)]">暂无子 Work Item</p>
        ) : (
          children.map((item) => (
            <div
              key={item.work_item_id}
              className="rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] p-2"
            >
              <div className="text-sm font-semibold text-[var(--aria-ink)]">{item.title}</div>
              <div className="mt-1 flex flex-wrap gap-1.5 font-mono text-[11px] text-[var(--aria-ink-muted)]">
                <span className="rounded border border-[var(--aria-line)] px-1.5 py-0.5">
                  {item.work_item_id}
                </span>
                <span className="rounded border border-[var(--aria-line)] px-1.5 py-0.5">
                  {item.execution_status}
                </span>
              </div>
            </div>
          ))
        )}
      </div>
    </section>
  );
}
```

在 drawer body 中挂载：

```tsx
{entity.kind === "work_item_group" ? <WorkItemGroupDetail entity={entity} /> : null}
```

- [ ] **Step 5: 修改 toDrawerEntity**

在 `IssueLifecycleWorkbench.tsx::toDrawerEntity` 中，group 分支返回：

```ts
if (card.kind === "work_item_group") {
  const plan = card.raw;
  return {
    id: card.id,
    kind: "work_item_group",
    title: card.title,
    status: card.status,
    version: null,
    childWorkItems: allWorkItems.filter((item) =>
      card.childWorkItemIds.includes(item.work_item_id),
    ),
    workItemPlanSourceStorySpecIds: plan.source_story_spec_ids,
    workItemPlanSourceDesignSpecIds: plan.source_design_spec_ids,
    workItemPlanValidatorFindings: plan.validator_findings,
    workItemPlanDependencyGraph: plan.dependency_graph,
  };
}
```

- [ ] **Step 6: 调整 workspace/coding actions**

`handleOpenWorkspaceFromDrawer` 允许 `work_item_group` 使用 `WorkspaceType::WorkItemPlan` 的 session：

```ts
function findWorkspaceSession(lifecycles: IssueLifecycleResponse[], card: LifecycleCardData) {
  const workspaceType =
    card.kind === "work_item_group" ? "work_item_plan" :
    card.kind === "story_spec" ? "story" :
    card.kind === "design_spec" ? "design" :
    card.kind === "work_item" ? "work_item" :
    null;
  if (!workspaceType) return null;
  return lifecycles
    .find((lifecycle) => lifecycle.issue.issue_id === card.issueId)
    ?.workspace_sessions.find(
      (session) => session.entity_id === card.id && session.workspace_type === workspaceType,
    ) ?? null;
}
```

子 Work Item 的 Coding 入口不放在 group 主按钮上；group drawer 只展示子项，后续若需要从子项直接 Coding，再加每个子项按钮。

- [ ] **Step 7: 运行测试确认通过**

Run: `cd web && pnpm test -- IssueLifecycleWorkbench.test.tsx`

Expected: PASS。

- [ ] **Step 8: 提交**

```bash
git add web/src/components/lifecycle/IssueLifecycleWorkbench.tsx web/src/components/lifecycle/LifecycleCardDrawer.tsx web/src/components/lifecycle/IssueLifecycleWorkbench.test.tsx
git commit -m "fix: show work item groups in workbench drawer"
```

## Task 4: WorkItemPlanAuthor 发送可见流式进度

**Files:**
- Modify: `src/product/workspace_engine.rs`
- Modify: `src/web/workspace_ws_handler.rs`
- Test: `tests/it_web/web_work_item_plan_author.rs`
- Test: `tests/it_web/web_workspace_recovery_consistency.rs`

- [ ] **Step 1: 写失败测试**

在 `tests/it_web/web_work_item_plan_author.rs` 增加测试，启动 work_item_plan workspace 后发送 `run_next`，断言 candidate artifact 之前收到 `stream_chunk`：

```rust
#[tokio::test]
async fn work_item_plan_author_streams_progress_before_candidate_artifact() {
    let app = test_app_with_scripted_work_item_splitter().await;
    let session_id = seed_work_item_plan_workspace(&app).await;
    let mut ws = app.open_workspace_ws(&session_id).await;

    ws.send_json(serde_json::json!({"type": "run_next", "user_prompt": null}))
        .await;

    let mut saw_progress = false;
    let mut saw_candidate = false;
    while let Some(message) = ws.next_json().await {
        match message["type"].as_str() {
            Some("stream_chunk") => {
                if message["content"]
                    .as_str()
                    .unwrap_or_default()
                    .contains("正在生成 Work Item Plan")
                {
                    saw_progress = true;
                }
            }
            Some("artifact_update") if message.get("candidate").is_some() => {
                saw_candidate = true;
                break;
            }
            Some("error") => panic!("unexpected ws error: {message}"),
            _ => {}
        }
    }

    assert!(saw_progress, "expected progress stream_chunk before candidate");
    assert!(saw_candidate, "expected candidate artifact_update");
}
```

- [ ] **Step 2: 写刷新恢复测试**

在 `tests/it_web/web_workspace_recovery_consistency.rs` 增加断言：WorkItemPlanAuthor 完成后重新打开 session state，author node detail 的 `streaming_content` 包含进度文本。

```rust
assert!(
    session_state["timeline_node_details"]
        .as_object()
        .unwrap()
        .values()
        .any(|detail| {
            detail["node_type"] == "author_run"
                && detail["streaming_content"]
                    .as_str()
                    .unwrap_or_default()
                    .contains("正在生成 Work Item Plan")
        })
);
```

- [ ] **Step 3: 运行测试确认失败**

Run: `cargo test --locked --test web_work_item_plan_author work_item_plan_author_streams_progress_before_candidate_artifact`

Expected: FAIL，当前只有最终 `artifact_update` 或 error，没有 progress `stream_chunk`。

- [ ] **Step 4: 增加 WorkspaceEngine helper**

在 `src/product/workspace_engine.rs` 中增加：

```rust
pub async fn append_active_run_stream(
    &mut self,
    role: &str,
    content: impl Into<String>,
) -> Result<(), String> {
    let content = content.into();
    let node_id = self.active_node_id.clone();
    if let Some(node_id) = node_id.as_deref() {
        self.buffer_stream_chunk(node_id, content.clone()).await?;
    }
    let _ = self
        .event_tx
        .send(EngineEvent::StreamChunk {
            role: role.to_string(),
            content,
            node_id,
        })
        .await;
    Ok(())
}
```

如果 `flush_stream_buffer` 只写 detail 不发事件，保留该行为；实时事件由 helper 发送，持久化由 `buffer_stream_chunk` 写入 detail。

- [ ] **Step 5: 在 WorkItemPlanAuthor dedicated run 中发送阶段消息**

在 `src/web/workspace_ws_handler.rs` 的 `ProviderRunKind::WorkItemPlanAuthor` 分支中，在每个长耗时点前后加：

```rust
let _ = engine
    .append_active_run_stream("author", "正在生成 Work Item Plan：准备上下文\n")
    .await;
```

释放 engine 前追加：

```rust
let _ = engine
    .append_active_run_stream("author", "正在生成 Work Item Plan：调用 provider\n")
    .await;
```

provider 成功返回后、`complete_work_item_plan_author` 前追加：

```rust
let _ = engine
    .append_active_run_stream("author", "正在生成 Work Item Plan：解析并校验候选拆分\n")
    .await;
```

AutoRevision 循环每轮追加：

```rust
let _ = engine
    .append_active_run_stream(
        "author",
        format!("正在生成 Work Item Plan：根据校验结果自动返修第 {revision_iterations} 轮\n"),
    )
    .await;
```

- [ ] **Step 6: 结束时补 MessageComplete**

在 `WorkItemPlanAuthor` 和 `WorkItemPlanRevision` 正常完成路径调用：

```rust
let _ = engine.complete_active_stream_message("生成过程已记录").await;
```

如果没有现成 helper，则新增：

```rust
pub async fn complete_active_stream_message(&mut self, summary: &str) -> Result<(), String> {
    let Some(node_id) = self.active_node_id.clone() else {
        return Ok(());
    };
    self.flush_stream_buffer(&node_id).await?;
    let message_id = format!("msg_{:03}", self.session.messages.len() + 1);
    let checkpoint_id = self.persist_checkpoint_for_active_node(summary).await?;
    let _ = self
        .event_tx
        .send(EngineEvent::MessageComplete {
            message_id,
            checkpoint_id,
            node_id: Some(node_id),
        })
        .await;
    Ok(())
}
```

实现时若 `persist_checkpoint_for_active_node` 不存在，不新增大抽象；复用现有普通 author run 生成 checkpoint 的私有逻辑，提取一个小 helper，确保 Story/Design 流程测试不变。

- [ ] **Step 7: WorkItemPlanRevision 同步处理**

在 `ProviderRunKind::WorkItemPlanRevision` 分支使用相同 helper，文本改为：

```rust
"正在返修 Work Item Plan：调用 provider\n"
"正在返修 Work Item Plan：解析并校验候选拆分\n"
```

- [ ] **Step 8: 运行后端测试**

Run: `cargo test --locked --test web_work_item_plan_author work_item_plan_author_streams_progress_before_candidate_artifact`

Expected: PASS。

Run: `cargo test --locked --test web_workspace_recovery_consistency`

Expected: PASS。

- [ ] **Step 9: 提交**

```bash
git add src/product/workspace_engine.rs src/web/workspace_ws_handler.rs tests/it_web/web_work_item_plan_author.rs tests/it_web/web_workspace_recovery_consistency.rs
git commit -m "fix: stream work item plan generation progress"
```

## Task 5: 前端 Chat Workspace 展示 WorkItemPlan 进度气泡

**Files:**
- Modify: `web/src/hooks/useWorkspaceWs.ts`
- Modify: `web/src/state/workspace-ws-store.test.ts`
- Modify: `web/src/pages/ChatWorkspacePage.test.tsx`

- [ ] **Step 1: 写 hook/store 测试**

在 `web/src/state/workspace-ws-store.test.ts` 增加：

```ts
it("rebuilds work item plan provider stream entries from timeline node details", () => {
  useWorkspaceStore.setState({
    sessionId: "workspace_session_0001",
    workspaceType: "work_item_plan",
    stage: "author_confirm",
    timelineNodes: [
      {
        node_id: "timeline_node_author_001",
        node_type: "author_run",
        title: "Work Item Plan Author",
        status: "completed",
        agent: "claude_code",
        started_at: "2026-06-19T00:00:00Z",
        completed_at: "2026-06-19T00:00:10Z",
        summary: "生成完成",
      },
    ],
    nodeDetails: {
      timeline_node_author_001: makeNodeDetail({
        node_id: "timeline_node_author_001",
        node_type: "author_run",
        streaming_content: "正在生成 Work Item Plan：调用 provider\n",
      }),
    },
  });

  useWorkspaceStore.getState().rebuildChatEntries();

  expect(useWorkspaceStore.getState().chatEntries).toEqual(
    expect.arrayContaining([
      expect.objectContaining({
        type: "provider_stream",
        content: expect.stringContaining("正在生成 Work Item Plan"),
      }),
    ]),
  );
});
```

- [ ] **Step 2: 写页面测试**

在 `web/src/pages/ChatWorkspacePage.test.tsx` 增加：

```tsx
it("renders streaming progress bubbles for work item plan generation", async () => {
  mockWorkspaceWs();
  useWorkspaceStore.setState({
    sessionId: "workspace_session_0001",
    workspaceType: "work_item_plan",
    stage: "running",
    activeRunId: "run-1",
    activeNodeId: "timeline_node_author_001",
    providers: { author: "claude_code", reviewer: "codex" },
  });

  render(<ChatWorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />);

  useWorkspaceStore.getState().appendBufferedStreamChunk(
    "正在生成 Work Item Plan：调用 provider\n",
    "timeline_node_author_001",
    "author",
  );
  useWorkspaceStore.getState().flushBufferedStream("timeline_node_author_001");

  expect(await screen.findByText(/正在生成 Work Item Plan/)).toBeInTheDocument();
});
```

- [ ] **Step 3: 运行测试确认失败**

Run: `cd web && pnpm test -- workspace-ws-store.test.ts ChatWorkspacePage.test.tsx`

Expected: FAIL，当前 work_item_plan dedicated run 不产生可显示 stream 或测试 helper 不支持该路径。

- [ ] **Step 4: 修改 hook 接收条件**

确认 `ACTIVE_PROVIDER_STAGES` 已含 `"running"`；如果测试证明 `activeRunId` 未及时设置导致丢 chunk，把 `stream_chunk` 分支改为只在明确 idle 阶段丢弃：

```ts
case "stream_chunk":
  if (!ACTIVE_PROVIDER_STAGES.has(store.stage) && !store.activeRunId) {
    break;
  }
```

不要针对 Story/Design 引入新分支。

- [ ] **Step 5: 运行前端测试**

Run: `cd web && pnpm test -- workspace-ws-store.test.ts ChatWorkspacePage.test.tsx`

Expected: PASS。

- [ ] **Step 6: 提交**

```bash
git add web/src/hooks/useWorkspaceWs.ts web/src/state/workspace-ws-store.test.ts web/src/pages/ChatWorkspacePage.test.tsx
git commit -m "fix: render work item plan progress bubbles"
```

## Task 6: structured output sentinel 提示与诊断

**Files:**
- Modify: `src/product/work_item_split_engine.rs`
- Modify: `tests/it_product/product_work_item_split_engine.rs`
- Modify: `tests/it_provider/provider_adapter_baseline.rs`

- [ ] **Step 1: 写 prompt 回归测试**

在 `tests/it_product/product_work_item_split_engine.rs` 或现有 `src/product/work_item_split_engine.rs` 单测中增加：

```rust
#[test]
fn build_split_prompt_requires_only_sentinel_wrapped_json() {
    let prompt = build_default_split_prompt_for_test();

    assert!(prompt.contains("<ARIA_STRUCTURED_OUTPUT>"));
    assert!(prompt.contains("</ARIA_STRUCTURED_OUTPUT>"));
    assert!(prompt.contains("最终输出必须只包含一个 <ARIA_STRUCTURED_OUTPUT> JSON block"));
    assert!(prompt.contains("不要输出 Markdown、解释、代码块或标签外文本"));
    assert!(prompt.contains("第一个非空字符必须是 <ARIA_STRUCTURED_OUTPUT>"));
    assert!(prompt.contains("最后一个非空字符必须是 </ARIA_STRUCTURED_OUTPUT>"));
}
```

- [ ] **Step 2: 写 parser 行为锁定测试**

在 `tests/it_provider/provider_adapter_baseline.rs` 增加：

```rust
#[test]
fn parser_does_not_parse_truncated_json_with_only_end_sentinel() {
    let stdout = "\"work_items\": []\n}</ARIA_STRUCTURED_OUTPUT>\n";
    let parsed = parse_last_structured_output(stdout).expect("parse should not fail");

    assert!(parsed.is_none(), "missing start sentinel must stay invalid");
}
```

- [ ] **Step 3: 运行测试确认失败**

Run: `cargo test --locked --lib build_split_prompt_requires_only_sentinel_wrapped_json`

Expected: FAIL，prompt 尚未包含更严格的输出要求。

Run: `cargo test --locked --test provider_adapter_baseline parser_does_not_parse_truncated_json_with_only_end_sentinel`

Expected: PASS 或 FAIL。若 PASS，说明 parser 行为已锁定，只需保留测试。

- [ ] **Step 4: 强化 split prompt**

在 `src/product/work_item_split_engine.rs::build_split_prompt` 的 `[output_schema]` 文案中改为：

```rust
"最终输出必须只包含一个 <ARIA_STRUCTURED_OUTPUT> JSON block。\n\
 第一个非空字符必须是 <ARIA_STRUCTURED_OUTPUT>，最后一个非空字符必须是 </ARIA_STRUCTURED_OUTPUT>。\n\
 标签内部必须是一个完整 JSON object，不要输出 Markdown、解释、代码块或标签外文本。\n\
 严格按以下 JSON schema 输出。\n\
 work_items 数组顺序即执行顺序；depends_on 使用同数组中的 0-based 索引。verification_plans 数组与 work_items 一一对应。\n\
 每个 work_item 必须包含 `kind` 字段（不要写成 `type`），合法取值为以下之一：backend、frontend、integration、e2e、docs、infra、other。\n\n\
 {schema}"
```

`build_revision_prompt` 使用同等约束，保留 redo-only 说明。

- [ ] **Step 5: 错误诊断保持 raw stdout/stderr**

确认 `map_provider_adapter_error` 的 `json!` detail 包含：

```rust
json!({
    "provider_error_code": error.code,
    "stdout": error.stdout,
    "stderr": error.stderr,
    "exit_code": error.exit_code,
})
```

如果当前 WebSocket error 只发 `message`，本任务不扩展 WS error contract；先在后端日志和 API error detail 保留 raw 输出。后续若用户希望 UI 显示日志路径，再单独加协议。

- [ ] **Step 6: 运行测试**

Run: `cargo test --locked --lib build_split_prompt_requires_only_sentinel_wrapped_json`

Expected: PASS。

Run: `cargo test --locked --test provider_adapter_baseline parser_does_not_parse_truncated_json_with_only_end_sentinel`

Expected: PASS。

- [ ] **Step 7: 提交**

```bash
git add src/product/work_item_split_engine.rs tests/it_product/product_work_item_split_engine.rs tests/it_provider/provider_adapter_baseline.rs
git commit -m "fix: tighten work item split structured output contract"
```

## Task 7: 集成验证与人工 E2E 指导

**Files:**
- No code files if previous tasks pass.

- [ ] **Step 1: 后端定向验证**

Run:

```bash
cargo fmt --check
cargo test --locked --test web_work_item_plan_author
cargo test --locked --test web_workspace_recovery_consistency
cargo test --locked --test provider_adapter_baseline parser_does_not_parse_truncated_json_with_only_end_sentinel
cargo test --locked --lib build_split_prompt_requires_only_sentinel_wrapped_json
```

Expected: 全部 PASS。

- [ ] **Step 2: 前端定向验证**

Run:

```bash
cd web && pnpm test -- api/types.test.ts lifecycle-workbench-store.test.ts IssueLifecycleWorkbench.test.tsx workspace-ws-store.test.ts ChatWorkspacePage.test.tsx
cd web && pnpm build
```

Expected: 全部 PASS。

- [ ] **Step 3: 后端全量快检**

Run:

```bash
cargo check --locked
cargo clippy --all-targets --all-features --locked -- -D warnings
```

Expected: 全部 PASS。

- [ ] **Step 4: 启动服务让用户复测**

Run:

```bash
cargo watch -x run
cd web && pnpm dev -- --port 5173
curl --noproxy '*' http://127.0.0.1:4317/api/health
curl --noproxy '*' http://127.0.0.1:5173/api/health
```

Expected:

```json
{"status":"ok"}
```

- [ ] **Step 5: 人工 E2E 验收清单**

用户在浏览器里验证：

1. 进入 Workbench，选中含 confirmed Design Spec 的 Issue。
2. 点击生成 Work Item。
3. 进入 WorkItemPlan workspace 后点击开始生成。
4. Chat 区应立即出现 “正在生成 Work Item Plan...” 的 provider stream 气泡。
5. provider 完成后出现 Work Item Plan candidate panel。
6. 回到 Workbench，Work Item 列只显示一个 Work Item Group。
7. 点击 Work Item Group，drawer 内显示所有子 Work Item。
8. 刷新页面后，WorkItemPlan workspace 的生成进度气泡仍可从 timeline detail 恢复。

- [ ] **Step 6: 回归范围说明**

汇报时明确：

- Story Spec 普通 author/reviewer streaming 路径未改变；通过 workspace store/chat tests 保持兼容。
- Design Spec 普通 author/reviewer streaming 路径未改变；通过同一共享 chat rebuild 逻辑覆盖。
- Work Item Plan dedicated provider run 新增流式进度和恢复测试。
- Workbench Work Item 列行为已从子项平铺改为 issue-level group。

## Self-Review

- Spec coverage:
  - 问题 1 `missing structured output sentinel`：Task 6 覆盖 prompt 约束和 parser 行为锁定。
  - 问题 2 无流式气泡：Task 4 和 Task 5 覆盖后端 WS 进度与前端展示/恢复。
  - 问题 3 Workbench 应展示 Work Item Group：Task 1、Task 2、Task 3 覆盖后端数据源、前端状态和 drawer 展示。
- Placeholder scan:
  - 本计划没有未落地的占位说明。
  - 每个代码修改任务都有明确文件、测试、命令和期望结果。
- Type consistency:
  - 后端 HTTP DTO 使用 `IssueWorkItemPlanDetailDto`。
  - 前端沿用现有 `IssueWorkItemPlanDetailDto = WorkItemPlanDto`，group card 使用 `kind: "work_item_group"`。
  - Workbench 列名仍为 `work_item`，但其中卡片表示 group，避免大范围 UI 布局重构。

## Execution Handoff

Plan complete and saved to `cadence/plans/2026-06-19_计划文档_修复方案_WorkItemPlan生成流式与Workbench分组_v1.0.md`.

Two execution options:

1. Subagent-Driven (recommended) - dispatch a fresh subagent per task, review between tasks, fast iteration.
2. Inline Execution - execute tasks in this session using executing-plans, batch execution with checkpoints.
