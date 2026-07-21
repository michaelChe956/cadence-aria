# Work Item Group 单一 Coding Attempt 约束实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 保证一个 Work Item Group 在关联 Attempt 存在期间只使用同一个 Coding Attempt，并让重复创建请求幂等返回原 Attempt。

**Architecture:** 前端按 Group ID 解析所有状态的关联 Attempt；Web Handler 使用 Group 级进程内互斥并优先返回既有 Attempt；文件存储层提供按 Group 查询和重复创建兜底。三层共同避免已完成 Attempt 被误判为不存在以及重复请求生成新数据。

**Tech Stack:** Rust 2024、Axum、JSON 文件存储、React、TypeScript、Vitest。

## Global Constraints

- 不修改 Coding worktree 中的业务代码或现有 Attempt 数据。
- 不引入 E2E、Playwright 或浏览器自动化测试。
- 普通单 Work Item Coding Attempt 的现有规则保持不变。
- 明确删除已有 Attempt 后允许该 Group 重新创建 Attempt。
- Rust 验证命令遵循 `cadence/project-rules/build-test-commands.md`，不得使用 `-j 1`。
- 不暂存或提交 `.superpowers/sdd/final-review-fix-report.md`。

---

### Task 1: 前端始终复用 Group 已有关联 Attempt

**Files:**
- Modify: `web/src/components/lifecycle/IssueLifecycleWorkbenchParts.tsx:18-45`
- Test: `web/src/components/lifecycle/IssueLifecycleWorkbench.drawer.test.tsx:211-254`

**Interfaces:**
- Consumes: `CodingAttempt.work_item_group_id` 与 `CodingAttempt.status`。
- Produces: `resolveGroupCodingAttempt(raw, codingAttempts, planId): CodingAttempt | null`，不再按状态过滤。

- [ ] **Step 1: 把 completed 场景改成失败回归测试**

将现有“completed 后创建新 Attempt”测试改为：

```tsx
it("reuses a completed group coding attempt instead of creating another one", async () => {
  const fetchMock = lifecycleFetch({
    confirmedWorkItem: true,
    splitWorkItems: true,
    workItemPlans: [
      issueWorkItemPlanRecord({
        id: "issue_plan_0001",
        status: "confirmed",
        work_item_ids: ["work_item_backend", "work_item_frontend"],
      }),
    ],
    codingAttempts: [
      {
        ...codingGroupAttemptRecord("issue_plan_0001"),
        attempt_id: "coding_attempt_completed_group_0001",
        status: "completed",
      },
    ],
  });
  vi.stubGlobal("fetch", fetchMock);
  const user = userEvent.setup();
  const onOpenCodingWorkspace = vi.fn();

  render(<IssueLifecycleWorkbench onOpenCodingWorkspace={onOpenCodingWorkspace} />);
  await user.click(await screen.findByRole("button", { name: "Work Item Group" }));
  expect(screen.getByRole("button", { name: "进入 Coding Workspace" })).toBeInTheDocument();

  await user.click(screen.getByTestId("drawer-open-coding-workspace"));
  await waitFor(() =>
    expect(onOpenCodingWorkspace).toHaveBeenCalledWith(
      "coding_attempt_completed_group_0001",
    ),
  );
  expect(fetchMock).not.toHaveBeenCalledWith(
    "/api/projects/project_0001/issues/issue_0001/work-item-plans/issue_plan_0001/coding-attempts",
    expect.objectContaining({ method: "POST" }),
  );
});
```

同时从 `IssueLifecycleWorkbenchParts` 导入 `resolveGroupCodingAttempt`，增加非活跃终态的表驱动测试：

```tsx
it.each(["failed", "aborted"] as const)(
  "resolves an existing %s group coding attempt",
  (status) => {
    const attempt = {
      ...codingGroupAttemptRecord("issue_plan_0001"),
      attempt_id: `coding_attempt_${status}_group_0001`,
      status,
    };

    expect(
      resolveGroupCodingAttempt({}, [attempt], "issue_plan_0001"),
    ).toEqual(attempt);
  },
);
```

- [ ] **Step 2: 运行前端定向测试并确认按预期失败**

Run: `cd web && pnpm test -- IssueLifecycleWorkbench.drawer.test.tsx`

Expected: FAIL，completed Attempt 的按钮仍为“开始 Coding”。

- [ ] **Step 3: 实现不按状态过滤的解析逻辑**

删除 `ACTIVE_GROUP_ATTEMPT_STATUSES`，fallback 改为：

```ts
return (
  codingAttempts.find(
    (attempt) => attempt.work_item_group_id === planId,
  ) ?? null
);
```

保留后端直接提供 `latest_group_attempt` 时的优先级。

- [ ] **Step 4: 运行前端定向测试并确认通过**

Run: `cd web && pnpm test -- IssueLifecycleWorkbench.drawer.test.tsx`

Expected: PASS，所有 drawer 测试通过。

- [ ] **Step 5: 提交前端修复**

```bash
git add web/src/components/lifecycle/IssueLifecycleWorkbenchParts.tsx web/src/components/lifecycle/IssueLifecycleWorkbench.drawer.test.tsx
git commit -m "fix: reuse completed group coding attempt"
```

### Task 2: 存储层阻止同一 Group 创建第二个 Attempt

**Files:**
- Modify: `src/product/coding_attempt_store/group.rs:14-80`
- Test: `src/product/coding_attempt_store/tests.rs`

**Interfaces:**
- Produces: `get_attempt_for_work_item_group(&self, project_id: &str, issue_id: &str, plan_id: &str) -> Result<Option<CodingExecutionAttempt>, ProductStoreError>`。
- Produces error: `product_store_io: coding_attempt_group_already_exists: <attempt_id>`。

- [ ] **Step 1: 编写 completed Attempt 的重复创建失败测试**

新增测试：先创建 Group Attempt，依次更新为 `Running`、`Completed`，再对同一 `plan_id` 创建，断言错误包含原 ID。

```rust
#[test]
fn rejects_second_group_attempt_after_original_completed() {
    let (_tmp, store) = setup_store();
    let first = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_work_item_id: WORK_ITEM_ID.to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: None,
            provider_config_snapshot: provider_snapshot(),
            max_auto_rework: 2,
        })
        .expect("first group attempt");
    store
        .update_attempt_status(PROJECT_ID, ISSUE_ID, &first.id, CodingAttemptStatus::Running)
        .expect("start group attempt");
    store
        .update_attempt_status(PROJECT_ID, ISSUE_ID, &first.id, CodingAttemptStatus::Completed)
        .expect("complete group attempt");

    let error = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_work_item_id: WORK_ITEM_ID.to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: None,
            provider_config_snapshot: provider_snapshot(),
            max_auto_rework: 2,
        })
        .expect_err("same group must not create a second attempt");

    assert_eq!(
        error.to_string(),
        format!("product_store_io: coding_attempt_group_already_exists: {}", first.id),
    );
}
```

- [ ] **Step 2: 运行 Store 定向测试并确认按预期失败**

Run: `cargo test --locked --lib rejects_second_group_attempt_after_original_completed`

Expected: FAIL，当前实现会成功创建第二个 Attempt。

- [ ] **Step 3: 实现按 Group 查询与唯一性检查**

在 `group.rs` 增加查询方法，校验三个 ID，过滤 `work_item_group_id == plan_id`，并按 `(attempt_no, id)` 排序后返回第一条：

```rust
pub fn get_attempt_for_work_item_group(
    &self,
    project_id: &str,
    issue_id: &str,
    plan_id: &str,
) -> Result<Option<CodingExecutionAttempt>, ProductStoreError> {
    validate_relative_id(project_id)?;
    validate_relative_id(issue_id)?;
    validate_relative_id(plan_id)?;
    let mut attempts: Vec<CodingExecutionAttempt> = super::list_json_records(
        &self.coding_attempts_root(project_id, issue_id),
    )?
    .into_iter()
    .filter(|attempt| attempt.work_item_group_id.as_deref() == Some(plan_id))
    .collect();
    attempts.sort_by(|left, right| {
        left.attempt_no
            .cmp(&right.attempt_no)
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(attempts.into_iter().next())
}
```

在 `create_group_attempt` 的活跃 Attempt 检查之前调用该方法，存在时返回：

```rust
return Err(ProductStoreError::Io(format!(
    "coding_attempt_group_already_exists: {}",
    existing.id,
)));
```

- [ ] **Step 4: 运行 Store 定向测试并确认通过**

Run: `cargo test --locked --lib rejects_second_group_attempt_after_original_completed`

Expected: PASS。

- [ ] **Step 5: 运行全部 CodingAttemptStore 单元测试**

Run: `cargo test --locked --lib coding_attempt_store::tests`

Expected: PASS，无回归失败。

在同一测试文件增加独立用例，证明唯一性只作用于同一 Group：

```rust
#[test]
fn allows_group_attempt_for_different_plan_after_original_completed() {
    let (_tmp, store) = setup_store();
    let first = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_work_item_id: WORK_ITEM_ID.to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: None,
            provider_config_snapshot: provider_snapshot(),
            max_auto_rework: 2,
        })
        .expect("first group attempt");
    store
        .update_attempt_status(PROJECT_ID, ISSUE_ID, &first.id, CodingAttemptStatus::Running)
        .expect("start first attempt");
    store
        .update_attempt_status(PROJECT_ID, ISSUE_ID, &first.id, CodingAttemptStatus::Completed)
        .expect("complete first attempt");

    let second = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            plan_id: "work_item_plan_0002".to_string(),
            current_work_item_id: "work_item_0002".to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: None,
            provider_config_snapshot: provider_snapshot(),
            max_auto_rework: 2,
        })
        .expect("different group attempt");

    assert_eq!(second.work_item_group_id.as_deref(), Some("work_item_plan_0002"));
}
```

- [ ] **Step 6: 提交存储层修复**

```bash
git add src/product/coding_attempt_store/group.rs src/product/coding_attempt_store/tests.rs
git commit -m "fix: enforce one attempt per work item group"
```

### Task 3: Group 创建 API 幂等返回原 Attempt

**Files:**
- Modify: `src/web/handlers/coding.rs:6-145`
- Test: `tests/it_web/web_coding_attempt_api/part_01.rs:244-269`

**Interfaces:**
- Consumes: `CodingAttemptStore::get_attempt_for_work_item_group`。
- Produces: 同一 Group 重复 POST 返回 HTTP 200 和原 `CodingAttemptDto`。

- [ ] **Step 1: 编写重复 POST 的 API 回归测试**

在 Group 创建 API 测试后新增：

```rust
#[tokio::test]
async fn repeated_group_coding_attempt_create_returns_original_attempt() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item_plan_group(app.clone(), repo.path()).await;
    let path = "/api/projects/project_0001/issues/issue_0001/work-item-plans/work_item_plan_0001/coding-attempts";

    let (first_status, first) = request_json(app.clone(), Method::POST, path, json!({})).await;
    let (second_status, second) = request_json(app.clone(), Method::POST, path, json!({})).await;

    assert_eq!(first_status, StatusCode::OK);
    assert_eq!(second_status, StatusCode::OK);
    assert_eq!(second["attempt_id"], first["attempt_id"]);
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    assert_eq!(
        store
            .list_coding_units("project_0001", "issue_0001", first["attempt_id"].as_str().unwrap())
            .expect("units")
            .len(),
        2,
    );
}
```

- [ ] **Step 2: 运行 API 定向测试并确认按预期失败**

Run: `cargo test --locked --test it_web web_coding_attempt_api::repeated_group_coding_attempt_create_returns_original_attempt`

Expected: FAIL，第二次请求当前返回冲突。

- [ ] **Step 3: 实现 Group 级互斥和早返回**

在确认 Plan 后、准备 worktree 前：

```rust
let group_lock_key = format!("work_item_group:{project_id}:{issue_id}:{plan_id}");
let _group_guard = state.coding_runs.lock_attempt(&group_lock_key).await;
let coding_store = CodingAttemptStore::new(app_paths.clone());
if let Some(existing) = coding_store
    .get_attempt_for_work_item_group(&project_id, &issue_id, &plan_id)
    .map_err(product_store_api_error)?
{
    return Ok(Json(coding_attempt_dto(&existing)));
}
```

复用该 `coding_store`，删除后面的重复初始化。若存储层仍返回 `coding_attempt_group_already_exists:<id>`，解析 ID、读取 Attempt 并返回，覆盖绕过早查询的竞争窗口。

在现有 `create_group_attempt` 错误分支中加入精确处理：

```rust
Err(ProductStoreError::Io(message))
    if message.starts_with("coding_attempt_group_already_exists:") =>
{
    if !already_locked_by_current {
        let _ = lifecycle.release_issue_worktree_lock(
            &project_id,
            &issue_id,
            &current_work_item.id,
        );
    }
    let existing_id = message
        .strip_prefix("coding_attempt_group_already_exists:")
        .expect("matched prefix")
        .trim();
    let existing = coding_store
        .get_attempt(&project_id, &issue_id, existing_id)
        .map_err(product_store_api_error)?;
    return Ok(Json(coding_attempt_dto(&existing)));
}
```

- [ ] **Step 4: 运行 API 定向测试并确认通过**

Run: `cargo test --locked --test it_web web_coding_attempt_api::repeated_group_coding_attempt_create_returns_original_attempt`

Expected: PASS。

- [ ] **Step 5: 运行 Coding Attempt API 测试模块**

Run: `cargo test --locked --test it_web web_coding_attempt_api`

Expected: PASS，无现有单 Work Item 或 Group API 回归。

- [ ] **Step 6: 提交 API 幂等修复**

```bash
git add src/web/handlers/coding.rs tests/it_web/web_coding_attempt_api/part_01.rs
git commit -m "fix: make group coding attempt creation idempotent"
```

### Task 4: 综合验证与推送

**Files:**
- Verify only: all files changed by Tasks 1-3

**Interfaces:**
- Consumes: 前端解析、Store 唯一性和 API 幂等行为。
- Produces: 可推送的 `feat-b-0709` 分支。

- [ ] **Step 1: 运行前端全量单元测试**

Run: `cd web && pnpm test`

Expected: PASS，无失败用例。

- [ ] **Step 2: 运行前端类型检查**

Run: `cd web && pnpm tsc -b`

Expected: exit 0。

- [ ] **Step 3: 运行 Rust 格式、检查和相关测试**

Run:

```bash
cargo fmt --check
cargo check --locked
cargo test --locked --lib coding_attempt_store::tests
cargo test --locked --test it_web web_coding_attempt_api
```

Expected: 全部 exit 0。

- [ ] **Step 4: 检查改动边界**

Run: `git status --short && git diff origin/feat-b-0709...HEAD --check`

Expected: 仅 `.superpowers/sdd/final-review-fix-report.md` 保持为用户未提交修改；本任务文件均已提交，diff 无空白错误。

- [ ] **Step 5: 推送分支**

Run: `git push origin feat-b-0709`

Expected: `origin/feat-b-0709` 更新到本任务最终提交。
