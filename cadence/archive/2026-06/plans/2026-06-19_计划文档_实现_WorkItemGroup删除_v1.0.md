# WorkItemGroup 删除 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 允许用户从 Workbench 删除 Work Item Group，并级联删除对应 Work Item Plan、子 Work Item、子 Workspace 与 Coding Attempt 相关资源。

**Architecture:** 后端新增 `DELETE /work-item-plans/{plan_id}`，handler 读取 plan 后复用现有 Work Item 删除清理逻辑逐个删除子项，再删除 plan 自身关联资源。前端新增 API client 方法，Workbench 对 `work_item_group` 暴露删除按钮和 drawer 删除入口。

**Tech Stack:** Rust/Axum、LifecycleStore、CodingAttemptStore、React、TypeScript、Vitest、Testing Library。

---

### Task 1: 后端红灯测试

**Files:**
- Modify: `tests/it_web/web_coding_attempt_api.rs`

- [ ] **Step 1: 新增 API 测试**

新增测试 `delete_work_item_plan_cascades_children_sessions_and_attempts`：

- 准备一个 confirmed Work Item Plan，包含 `work_item_0001`。
- 为该 Work Item 创建 coding attempt，并准备 worktree/artifact。
- 调用 `DELETE /api/projects/project_0001/issues/issue_0001/work-item-plans/issue_work_item_plan_0001`。
- 断言 lifecycle 中 `work_item_plans`、`work_items`、`coding_attempts` 为空。
- 断言 `CodingAttemptStore` 中该 work item 的 attempts 已空。

- [ ] **Step 2: 运行后端红灯**

Run:

```bash
cargo test --locked --test web_coding_attempt_api delete_work_item_plan_cascades_children_sessions_and_attempts
```

Expected: FAIL，当前路由不存在或返回非 OK。

### Task 2: 后端实现

**Files:**
- Modify: `src/product/lifecycle_store.rs`
- Modify: `src/web/handlers.rs`
- Modify: `src/web/app.rs`

- [ ] **Step 1: Store 增加 plan 删除方法**

新增 `LifecycleStore::delete_issue_work_item_plan(project_id, issue_id, plan_id)`：

- 读取 plan，保留 `work_item_ids`、`verification_plan_ids`、`repository_profile_ref`。
- 删除 plan json 文件。
- 删除 `WorkspaceType::WorkItemPlan` 的 workspace session/timeline。
- 删除 plan 关联 verification plan 文件。
- 删除 plan 关联 repository profile 文件。
- 返回被删除的 plan，供 handler 继续删除子 Work Item。

- [ ] **Step 2: Handler 复用 Work Item 清理**

抽取私有 helper：

```rust
async fn delete_work_item_with_cleanup(
    app_paths: &ProductAppPaths,
    store: &LifecycleStore,
    project_id: &str,
    issue_id: &str,
    work_item_id: &str,
) -> ApiResult<()>
```

`delete_work_item` 和新 `delete_work_item_plan` 都调用它，保持 coding attempt/worktree/branch 清理一致。

- [ ] **Step 3: 增加路由**

在 `src/web/app.rs` 增加：

```rust
.route(
  "/api/projects/{project_id}/issues/{issue_id}/work-item-plans/{plan_id}",
  delete(handlers::delete_work_item_plan),
)
```

- [ ] **Step 4: 运行后端绿灯**

Run:

```bash
cargo test --locked --test web_coding_attempt_api delete_work_item_plan_cascades_children_sessions_and_attempts
```

Expected: PASS。

### Task 3: 前端红灯测试

**Files:**
- Modify: `web/src/components/lifecycle/IssueLifecycleWorkbench.test.tsx`
- Modify: `web/src/api/client.test.ts`

- [ ] **Step 1: API client 测试**

新增 `deleteWorkItemPlan` 测试，断言 DELETE 路径为：

```text
/api/projects/project%2Fwith%20space/issues/issue%2Fwith%20space/work-item-plans/plan%2Fwith%20space
```

- [ ] **Step 2: Workbench 测试**

把现有 `deletes specs from selected issue content and does not expose group deletion` 改成 `deletes specs and work item groups from selected issue content`：

- 仍保留 Story/Design 删除断言。
- 点击 Work Item Group 的删除按钮。
- 断言调用 `DELETE /work-item-plans/issue_work_item_plan_0001`。
- 断言 Work Item 内容不再显示 `Work Item Group`。

新增 drawer 测试：

- 打开 Work Item Group drawer。
- 点击 `删除 Work Item Group`。
- 断言调用 plan 删除 API。

- [ ] **Step 3: 运行前端红灯**

Run:

```bash
pnpm exec vitest --run src/api/client.test.ts src/components/lifecycle/IssueLifecycleWorkbench.test.tsx
```

Expected: FAIL，当前没有 `deleteWorkItemPlan` 和 Group 删除入口。

### Task 4: 前端实现

**Files:**
- Modify: `web/src/api/client.ts`
- Modify: `web/src/components/lifecycle/IssueLifecycleWorkbench.tsx`
- Modify: `web/src/components/lifecycle/LifecycleCardDrawer.tsx`

- [ ] **Step 1: API client 增加 deleteWorkItemPlan**

新增：

```ts
export function deleteWorkItemPlan(projectId: string, issueId: string, planId: string): Promise<{ status: string }>
```

- [ ] **Step 2: Workbench 删除分支**

`handleDeleteLifecycleCard` 支持 `work_item_group`，调用 `deleteWorkItemPlan`。

列表卡片不再对 `work_item_group` 传 `undefined` 删除 handler。

Drawer 对 `work_item_group` 显示 `删除 Work Item Group`。

- [ ] **Step 3: 运行前端绿灯**

Run:

```bash
pnpm exec vitest --run src/api/client.test.ts src/components/lifecycle/IssueLifecycleWorkbench.test.tsx
```

Expected: PASS。

### Task 5: 验证与提交

**Files:**
- All modified files from Tasks 1-4

- [ ] **Step 1: 格式化与检查**

Run:

```bash
cargo fmt --check
cargo test --locked --test web_coding_attempt_api delete_work_item_plan_cascades_children_sessions_and_attempts
pnpm exec vitest --run src/api/client.test.ts src/components/lifecycle/IssueLifecycleWorkbench.test.tsx
pnpm build
```

Expected: 全部 PASS，`pnpm build` 允许既有 chunk size warning。

- [ ] **Step 2: 提交**

Run:

```bash
git add .
git commit -m "fix: allow deleting work item groups"
```

Expected: commit 成功。
