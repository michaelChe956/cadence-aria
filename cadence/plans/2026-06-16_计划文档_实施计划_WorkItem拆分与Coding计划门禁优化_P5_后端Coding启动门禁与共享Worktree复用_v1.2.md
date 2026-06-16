# WorkItem 拆分 P5 后端 Coding 启动门禁与共享 Worktree 复用 Implementation Plan

> **文档版本：** v1.2
>
> **v1.1 修订摘要：** 修复 active lock 在异常终态/attempt 被取代时不释放导致的死锁（新增任务 3 的异常终态释放步骤，并将原先引用的不存在方法 `handle_attempt_failure` 修正为需新增的 `handle_attempt_failed`）；修正最终验证过滤名为本计划实际新增测试函数名；补充新增 test helper 的显式步骤；点名 branch 改为 `aria/issues/{issue_id}` 后需同步更新基于 `aria-work-items/{work_item}/attempt-{n}` 的既有断言测试；明确严格依赖 P1/P3/P4 已合并并要求字段名与上游逐字对齐（注意 `exclusive_write_scopes` 与现有 `WorkItemRecord.allowed_write_scope` 命名漂移）。
>
> **v1.2 修订摘要（架构评审修复）：** 1) `abort_coding_attempt` / `delete_coding_attempt` 必须经 `CodingWorkspaceEngine` 释放 active lock；2) `create_coding_attempt` 禁止 supersede 已有 active attempt，改为直接拒绝；3) `execute_worktree_prepare` 依赖 P4 的幂等 `create_branch`/`create_worktree`；4) `max_auto_rework_exceeded` 保持 `Blocked` 可恢复状态，但在进入 Blocked 后统一执行 clean-gate helper，clean 时释放锁、dirty 时保持锁并创建人工 gate。

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Coding 启动前检查依赖完成、Issue 共享 worktree 准备、active Work Item 串行锁、写入范围和 handoff 可读性，并让同一 Issue 下的 Coding attempt 复用 Issue 共享 branch/worktree。

**Architecture:** `create_coding_attempt` 负责启动前门禁和 branch 选择；`CodingWorkspaceEngine::execute_worktree_prepare` 负责按 attempt 中的 Issue branch/worktree 创建或复用 worktree；`LifecycleStore` 提供 active lock。第一版仍保持同一 Issue 同一时刻只有一个 active Work Item，避免共享 worktree 下 `git add -A` 污染其他 Work Item。
所有释放 active lock 的路径必须先执行 shared worktree clean gate。每个 Work Item 完成、失败、阻塞或中止后不得遗留未提交/半提交改动；若 shared worktree dirty，进入强制人工 gate，保持当前 Work Item 的 active lock，不允许后续 Work Item 接管。

**Tech Stack:** Rust 1.95.0、Axum、LifecycleStore、CodingAttemptStore、GitWorkspaceService、Cargo integration tests、TDD。

---

## 前置交付摘要

执行本计划前确认：

- P3 已让 `generate_work_items` 创建多个 Work Item，并写入 `depends_on`、`exclusive_write_scopes`、`forbidden_write_scopes`、`required_handoff_from`。
- P3 已保存 provider 输出的 `VerificationPlan`，并在 IssueWorkItemPlan 级 confirm 后才把 Work Item `plan_status` 置为 confirmed。
- P4 已提供 `IssueSharedWorktree` store API，并允许 `aria/issues/*` branch 与 `.worktrees/aria-issues/*` worktree。
- P4 未改变 existing attempt worktree 行为；本计划负责切换 Coding attempt 到 Issue 级共享 worktree。

> **🔴 严格依赖说明（v1.1 新增）：** 本计划严格依赖 P1/P3/P4 已合并。以下符号在当前源码中**尚不存在**，必须由上游先落地后本计划才能开工：
>
> - `exclusive_write_scopes`（来自 P3 的 Work Item 字段）。
> - `IssueSharedWorktree` store API（来自 P4），包括 `upsert_issue_shared_worktree`、`try_acquire_issue_worktree_lock`、`get_issue_shared_worktree`、`mark_issue_worktree_completed_item`、`release_issue_worktree_lock` 等。
> - `UpsertIssueSharedWorktreeInput` 输入结构及 `IssueSharedWorktree` 的 `current_active_work_item_id`、`last_completed_work_item_id` 字段。
>
> **字段名逐字对齐（命名漂移警告）：** 现有 `WorkItemRecord` 使用的是 `allowed_write_scope`（单数），而 P3 引入的是 `exclusive_write_scopes`（复数 + 语义不同）。实现前必须确认上游最终字段名，并在本计划所有引用处逐字对齐，避免编译期字段名不匹配。

## 计划大小边界

本计划只做启动门禁与共享 worktree 复用：

- 不实现 `WorkItemExecutionPlan` provider run。
- 不实现 handoff provider run。
- 不修改前端。
- 不写真实浏览器 E2E。

如果需要新增 execution plan/handoff 模型，停止并留给 P6。

## 文件结构

- Modify: `src/web/handlers.rs`
  - `create_coding_attempt` 增加依赖、active lock、handoff、execution plan 可配置门禁。
  - branch 改为 `aria/issues/{issue_id}`。
- Modify: `src/product/coding_workspace_engine.rs`
  - `execute_worktree_prepare` 使用 attempt 的 Issue branch 和共享 worktree path。
  - `handle_abort`、`handle_final_confirm` 释放 active lock。
- Modify: `src/product/lifecycle_store.rs`
  - 补充按 Work Item 查询依赖状态和 shared worktree helpers。
- Modify: `tests/it_web/web_coding_attempt_api.rs`
  - 覆盖启动门禁和 branch/path 变化。
- Modify: `tests/it_product/product_coding_workspace_engine.rs`
  - 覆盖 shared worktree prepare 与 lock release。

## 任务 0：新增测试 Helper（v1.1 新增）

> 本计划的多条测试依赖一批当前尚不存在的 test helper。下面这些 helper 必须先显式新增，否则后续任务的测试无法编译。现有可参考的 helper：`bootstrap_confirmed_work_item`、`git_repo`、`request_json`。

**文件：**

- Modify: `tests/it_web/web_coding_attempt_api.rs`
- Modify: `tests/it_product/product_coding_workspace_engine.rs`

- [ ] **步骤 1：新增 it_web helper**

在 `tests/it_web/web_coding_attempt_api.rs` 新增（参考既有 `bootstrap_confirmed_work_item`）：

- `bootstrap_confirmed_split_work_items(app, repo_path)`：创建 `work_item_0001` 已完成、`work_item_0002` 依赖 `work_item_0001`（`depends_on=["work_item_0001"]`）且未完成。
- `bootstrap_two_ready_confirmed_work_items(app, repo_path)`：同一 Issue 下创建两个无依赖、均 ready/confirmed 的 Work Item（`work_item_0001`、`work_item_0002`），用于 active lock 串行测试。
- `bootstrap_completed_dependency_without_handoff(app, repo_path)`：`work_item_0001` 已完成但 `handoff_summary_ref=None`，`work_item_0002` 通过 `required_handoff_from=["work_item_0001"]` 依赖其 handoff。

- [ ] **步骤 2：新增 it_product helper**

在 `tests/it_product/product_coding_workspace_engine.rs` 新增：

- `git_repo_in(path)`：在指定路径初始化 git repo（与现有 `git_repo()` 行为一致，但可指定根目录）。
- `coding_store_with_attempt(root, work_item_id, branch_name)`：构造 `CodingAttemptStore` 并写入一个指定 branch 的 attempt，返回 `(store, attempt)`。
- `final_confirm_attempt(paths, work_item_id)`：构造一个处于 final confirm 前置状态的 attempt，返回 `(store, attempt)`。
- `failed_attempt(paths, work_item_id)`：构造一个 `status` 为 `CodingAttemptStatus::Failed` 的 attempt，返回 `(store, attempt)`，用于验证异常终态释放锁。
- `dirty_failed_attempt(paths, work_item_id)`：构造一个 `status=Failed` 且 shared worktree 存在未提交改动的 attempt，用于验证 dirty clean gate 保持锁。

- [ ] **步骤 3：编译确认**

运行 `cargo check --locked --tests`，确认 helper 签名与各测试调用一致（此时业务实现尚未完成，相关断言测试仍会失败属预期）。

## 任务 1：Gate Coding Attempt On Dependencies And Active Lock

**文件：**

- Modify: `src/web/handlers.rs`
- Modify: `tests/it_web/web_coding_attempt_api.rs`

- [ ] **步骤 1：编写失败态 dependency gate test**

Append to `tests/it_web/web_coding_attempt_api.rs`:

```rust
#[tokio::test]
async fn rejects_coding_attempt_when_dependency_work_item_is_not_completed() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_split_work_items(app.clone(), repo.path()).await;

    let (status, body) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0002/coding-attempts",
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "work_item_dependency_not_completed");
    assert_eq!(body["details"]["missing_dependencies"], json!(["work_item_0001"]));
}
```

- [ ] **步骤 2：编写失败态 active lock test**

追加:

```rust
#[tokio::test]
async fn rejects_second_active_work_item_on_same_issue_shared_worktree() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_two_ready_confirmed_work_items(app.clone(), repo.path()).await;

    let (status, first) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(first["branch_name"], "aria/issues/issue_0001");

    let (status, second) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0002/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(second["code"], "issue_worktree_active");
}
```

- [ ] **步骤 3：运行 gate tests 并确认失败**

运行:

```bash
cargo test --locked --test it_web rejects_coding_attempt_when_dependency_work_item_is_not_completed
cargo test --locked --test it_web rejects_second_active_work_item_on_same_issue_shared_worktree
```

预期: tests fail because current handler only checks plan status and per-work-item active attempt.

- [ ] **步骤 4：实现 handler gates**

在 `create_coding_attempt`:

1. Load all Work Items for the Issue.
2. Reject if any `depends_on` item is missing or not `WorkItemStatus::Completed`.
3. Reject if `required_handoff_from` contains an item whose `handoff_summary_ref` is `None`.
4. Ensure or create `IssueSharedWorktree` record with:

```rust
branch_name: format!("aria/issues/{issue_id}")
worktree_path: repository.path.join(".worktrees").join("aria-issues").join(&issue_id)
base_branch: current_git_branch(&repository.path).unwrap_or_else(|| "HEAD".to_string())
```

5. Acquire `current_active_work_item_id` lock before creating the attempt.
6. On attempt creation failure after lock acquisition, release the lock before returning error.

Attempt branch must be `aria/issues/{issue_id}`.

> **v1.2 明确规则：** 不允许 supersede 已有 active attempt。如果同一 work item 已存在 active attempt，直接拒绝创建新 attempt。同一 Issue 的 active lock 机制已经阻止另一个 work item 在已有 active 时创建新 attempt；本规则针对的是“同一 work item 内部”的重复创建。返回错误码 `coding_attempt_already_active`（或复用现有 `ApiError::Conflict` 格式）。

- [ ] **步骤 5：运行 gate tests 并确认通过**

Run the two commands from Step 3 again.

预期：两条测试都通过。

## 任务 1B：修复 abort/delete coding attempt 以释放 active lock

> **v1.2 新增任务：** 当前 `handlers::abort_coding_attempt` 与 `handlers::delete_coding_attempt` 直接修改 `CodingAttemptStore`，不经过 `CodingWorkspaceEngine`，导致 Issue 共享 worktree 的 active lock 泄漏。本任务要求两个 handler 都必须经 engine 走统一的 clean-gate + 锁释放路径。

**文件：**

- Modify: `src/web/handlers.rs`
- Modify: `src/product/coding_workspace_engine.rs`
- Modify: `tests/it_web/web_coding_attempt_api.rs`

- [ ] **步骤 1：修改 `handlers::abort_coding_attempt`**

在 `src/web/handlers.rs` 的 `abort_coding_attempt` 中：

1. 先通过 `coding_store.get_attempt(attempt_id)` 加载 attempt 及对应 Work Item。
2. 构造 `CodingWorkspaceEngine`（复用与 `create_coding_attempt` / 最终确认相同的构造方式）。
3. 调用 `engine.handle_abort(project_id, issue_id, attempt_id).await`，由 engine 统一设置 attempt 状态为 `Aborted`、执行 shared worktree clean gate、释放 active lock。
4. 最后返回 abort 后的 attempt DTO。

> 不得直接调用 `coding_store.update_attempt_status(..., Aborted)` 后返回。

- [ ] **步骤 2：新增 engine helper `handle_delete_attempt`**

在 `src/product/coding_workspace_engine.rs` 新增：

```rust
pub async fn handle_delete_attempt(
    &self,
    project_id: &str,
    issue_id: &str,
    attempt_id: &str,
) -> Result<(), CodingWorkspaceError>
```

逻辑：

1. 加载 attempt 及 Work Item。
2. 若该 Work Item 当前持有 Issue shared worktree active lock：
   - 将 attempt 状态置为 `Aborted`（或保持原状态，但后续清理路径按 Aborted 处理）。
   - 调用 `ensure_issue_shared_worktree_clean(...)`。
   - clean 时调用 `release_issue_worktree_lock(...)` 释放 active lock。
   - dirty 时返回 `shared_worktree_dirty_manual_gate`，不删除 attempt 记录。
3. 若未持锁，直接允许删除。

- [ ] **步骤 3：修改 `handlers::delete_coding_attempt`**

在 `src/web/handlers.rs` 的 `delete_coding_attempt` 中：

1. 加载 attempt 及对应 Work Item。
2. 若该 Work Item 是当前 Issue shared worktree 的 active holder，必须先调用 `engine.handle_delete_attempt(...).await`。
3. `handle_delete_attempt` 成功（clean 并释放锁）后，再执行文件系统/记录删除。
4. 删除顺序：**engine 释放锁必须先于文件系统删除**，避免删除后仍残留 active lock 记录。

- [ ] **步骤 4：新增 lock 释放测试**

在 `tests/it_web/web_coding_attempt_api.rs` 追加：

```rust
#[tokio::test]
async fn abort_coding_attempt_releases_issue_shared_worktree_lock() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_two_ready_confirmed_work_items(app.clone(), repo.path()).await;

    let (status, first) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _body) = request_json(
        app.clone(),
        Method::POST,
        &format!("/api/coding-attempts/{}/abort", first["attempt_id"].as_str().unwrap()),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // 锁释放后，第二个 work item 应能创建 attempt。
    let (status, second) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0002/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(second["work_item_id"], "work_item_0002");
}

#[tokio::test]
async fn delete_coding_attempt_releases_active_lock_when_clean() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_two_ready_confirmed_work_items(app.clone(), repo.path()).await;

    let (status, first) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let attempt_id = first["attempt_id"].as_str().unwrap();

    let (status, _body) = request_json(
        app.clone(),
        Method::DELETE,
        &format!("/api/coding-attempts/{}", attempt_id),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // 锁释放后，第二个 work item 应能创建 attempt。
    let (status, second) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0002/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}
```

- [ ] **步骤 5：运行新增测试并确认通过**

运行:

```bash
cargo test --locked --test it_web abort_coding_attempt_releases_issue_shared_worktree_lock
cargo test --locked --test it_web delete_coding_attempt_releases_active_lock_when_clean
```

预期：通过；若失败，检查 handler 是否确实调用了 engine 的 clean-gate 与锁释放路径。

## 任务 2：Reuse Issue Shared Worktree During Worktree Prepare

**文件：**

- Modify: `src/product/coding_workspace_engine.rs`
- Modify: `tests/it_product/product_coding_workspace_engine.rs`

- [ ] **步骤 1：编写失败态 engine test**

Append to `tests/it_product/product_coding_workspace_engine.rs`:

```rust
#[tokio::test]
async fn worktree_prepare_uses_issue_shared_worktree_path_for_issue_branch() {
    let root = tempdir().expect("root");
    let repo = git_repo_in(root.path());
    let (store, attempt) = coding_store_with_attempt(
        root.path(),
        "work_item_0001",
        "aria/issues/issue_0001",
    );
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    let updated = engine
        .execute_worktree_prepare(&attempt, &repo)
        .await
        .expect("prepare shared worktree");

    assert_eq!(
        updated.worktree_path.as_deref(),
        Some(
            repo.join(".worktrees")
                .join("aria-issues")
                .join("issue_0001")
                .as_path()
        )
    );
}
```

- [ ] **步骤 2：运行 engine test 并确认失败**

运行:

```bash
cargo test --locked --test it_product worktree_prepare_uses_issue_shared_worktree_path_for_issue_branch
```

预期：当前 `worktree_path_for_attempt()` returns `.worktrees/aria-work-items/{work_item}/attempt-{n}`.

- [ ] **步骤 3：实现 shared worktree path resolver**

替换 `worktree_path_for_attempt()` with branch-aware logic:

```rust
fn worktree_path_for_attempt(repo_path: &Path, attempt: &CodingExecutionAttempt) -> PathBuf {
    if let Some(issue_id) = attempt.branch_name.strip_prefix("aria/issues/") {
        return repo_path
            .join(".worktrees")
            .join("aria-issues")
            .join(issue_id);
    }
    repo_path
        .join(".worktrees")
        .join("aria-work-items")
        .join(&attempt.work_item_id)
        .join(format!("attempt-{}", attempt.attempt_no))
}
```

不要 remove old behavior; it preserves compatibility for stored attempts。

> **v1.2 幂等依赖：** `execute_worktree_prepare` 只需按上述规则计算 Issue shared worktree path，然后直接调用 `git_workspace_service::create_branch` 与 `create_worktree`。P4 已为这两个方法实现幂等语义：branch 已存在或 worktree 已注册同一 branch 时返回 `Ok(())`。因此本步骤不需要额外实现「检查并复用」逻辑。

> **⚠️ 既有断言测试影响（v1.1 新增）：** 当前 `worktree_path_for_attempt`（`src/product/coding_workspace_engine.rs:5045`）及既有测试断言基于 `aria-work-items/{work_item}/attempt-{n}` 路径。本任务保留旧分支逻辑，但当 Coding attempt 改用 `aria/issues/{issue_id}` branch 后，凡断言 attempt worktree 落在 `aria-work-items/.../attempt-{n}` 的既有测试将不再适用于 Issue branch 场景。实施时需：
>
> - 在 `tests/it_product/product_coding_workspace_engine.rs` 中定位所有断言 `aria-work-items/{work_item}/attempt-{n}` worktree 路径的既有测试。
> - 对走 Issue branch 的用例更新断言为 `.worktrees/aria-issues/{issue_id}`。
> - 对仍走 work-item branch 的回归用例保留旧断言，确认 strip_prefix 分支兼容性不被破坏。

- [ ] **步骤 4：Run engine test and existing worktree tests**

运行:

```bash
cargo test --locked --test it_product worktree_prepare_uses_issue_shared_worktree_path_for_issue_branch
cargo test --locked --test it_product product_coding_workspace_engine
```

预期：新增和既有 engine tests pass.

## 任务 3：Release Active Lock Only After Shared Worktree Clean Gate

> **🔴 v1.2 阻塞修复：** active lock 释放不能只看 attempt 终态。共享 worktree 是同一 Issue 的连续交付状态，任何终态只要 worktree dirty，都可能包含未提交/半提交改动。此时必须进入强制人工 gate 并保持 active lock，不允许下一个 Work Item 复用污染状态。只有 worktree clean 时，Completed/Aborted/Failed/Blocked/Superseded 才能释放或转移锁。

> **提交分段：** 本任务拆为两段提交，降低单 commit 风险。
> - 段一（步骤 1-4）：`handle_final_confirm`、`handle_abort` 在 clean gate 通过后释放锁，新增 `handle_attempt_failed` 并在 clean 时释放。
> - 段二（步骤 5-6）：dirty 强制人工 gate、Blocked/Superseded/新 attempt 取代旧 attempt 的 clean-gate 处理、`complete_attempt_after_final_rework` clean-gate 释放。

**文件：**

- Modify: `src/product/coding_workspace_engine.rs`
- Modify: `tests/it_product/product_coding_workspace_engine.rs`

- [ ] **步骤 1：编写失败态 lock release tests**

追加:

```rust
#[tokio::test]
async fn final_confirm_releases_issue_shared_worktree_lock() {
    let root = tempdir().expect("root");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(paths.clone());
    lifecycle
        .upsert_issue_shared_worktree(UpsertIssueSharedWorktreeInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: root.path().join("repo/.worktrees/aria-issues/issue_0001"),
            base_branch: "main".to_string(),
        })
        .expect("shared worktree");
    lifecycle
        .try_acquire_issue_worktree_lock("project_0001", "issue_0001", "work_item_0001")
        .expect("lock");
    let (store, attempt) = final_confirm_attempt(paths.clone(), "work_item_0001");
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);

    engine
        .handle_final_confirm("project_0001", "issue_0001", &attempt.id)
        .await
        .expect("final confirm");

    let shared = lifecycle
        .get_issue_shared_worktree("project_0001", "issue_0001")
        .expect("load shared")
        .expect("shared exists");
    assert_eq!(shared.current_active_work_item_id, None);
    assert_eq!(shared.last_completed_work_item_id.as_deref(), Some("work_item_0001"));
}
```

- [ ] **步骤 2：运行 lock release test 并确认失败**

运行:

```bash
cargo test --locked --test it_product final_confirm_releases_issue_shared_worktree_lock
```

预期：锁仍被持有，测试失败。

- [ ] **步骤 3：释放 lock in completion and abort paths**

在 `handle_final_confirm()`, after updating the Work Item to completed, first assert shared worktree clean, then call:

```rust
LifecycleStore::new(self.store.paths())
    .mark_issue_worktree_completed_item(project_id, issue_id, &updated.work_item_id)?;
```

在 `handle_abort()`, call:

```rust
ensure_issue_shared_worktree_clean(project_id, issue_id, &updated.work_item_id)?;
let _ = LifecycleStore::new(self.store.paths())
    .release_issue_worktree_lock(project_id, issue_id, &updated.work_item_id);
```

Abort should not fail solely because there is no shared worktree record; treat missing shared worktree as backward-compatible no-op.

- [ ] **步骤 4：运行 lock release test 并确认通过**

重新运行步骤 2 的命令。

预期：通过。

- [ ] **步骤 5：异常终态/attempt 被取代时释放锁（v1.1 新增阻塞修复）**

> **注意：当前源码不存在 `handle_attempt_failure` 方法。** 本步骤改为新增 `CodingWorkspaceEngine::handle_attempt_failed(project_id, issue_id, attempt_id)` 作为 `CodingAttemptStatus::Failed` 终态的统一处理入口，并在其中释放 Issue shared worktree 锁。

先编写失败态测试，覆盖「attempt 进入 Failed 终态后锁仍被持有」的死锁场景。追加到 `tests/it_product/product_coding_workspace_engine.rs`：

```rust
#[tokio::test]
async fn failed_attempt_releases_issue_shared_worktree_lock() {
    let root = tempdir().expect("root");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(paths.clone());
    lifecycle
        .upsert_issue_shared_worktree(UpsertIssueSharedWorktreeInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: root.path().join("repo/.worktrees/aria-issues/issue_0001"),
            base_branch: "main".to_string(),
        })
        .expect("shared worktree");
    lifecycle
        .try_acquire_issue_worktree_lock("project_0001", "issue_0001", "work_item_0001")
        .expect("lock");
    let (store, attempt) = failed_attempt(paths.clone(), "work_item_0001");
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);

    // 处理 Failed 终态，应释放 active lock。
    engine
        .handle_attempt_failed("project_0001", "issue_0001", &attempt.id)
        .await
        .expect("handle failed");

    let shared = lifecycle
        .get_issue_shared_worktree("project_0001", "issue_0001")
        .expect("load shared")
        .expect("shared exists");
    assert_eq!(shared.current_active_work_item_id, None);
}
```

随后在实现中，确保**所有 attempt 终态收敛点**都先走 clean gate，再决定是否释放锁：

1. 新增 `pub async fn handle_attempt_failed(&self, project_id: &str, issue_id: &str, attempt_id: &str) -> Result<(), CodingWorkspaceError>`：
   - 加载 attempt 及对应 Work Item。
   - 将 attempt 状态置为 `CodingAttemptStatus::Failed`（若尚未置为 Failed）。
   - 调用 `ensure_issue_shared_worktree_clean(project_id, issue_id, &work_item_id)`。
   - clean 时调用 `release_issue_worktree_lock(project_id, issue_id, &work_item_id)`，仅当当前持锁 Work Item 与本 attempt 的 Work Item 一致时才释放，避免误释放后续 attempt 的锁。
   - dirty 时创建 `shared_worktree_dirty_manual_gate`，保持 active lock，并返回可恢复错误。
   - 缺少 shared worktree 记录时按 backward-compatible no-op 处理。
2. 在已有 `handle_final_confirm`（Completed）和 `handle_abort`（Aborted）路径中保留并复核锁释放逻辑。
3. 其他收敛到 Failed/Superseded/Blocked 等终态的代码路径（如 provider run 异常退出、attempt 被新 attempt 取代）应统一调用 clean-gate 释放 helper。
4. 新 attempt 抢占（取代旧 attempt）时，应先检查旧持锁项 worktree clean；dirty 时拒绝新 attempt 并返回 `shared_worktree_dirty_manual_gate`，clean 时才释放/接管。

> **选 B 的落地要求：** 引入真正的 `CodingAttemptStatus::Failed` 终态，并将不可恢复的失败路径从 `Blocked` 改为 `Failed`。可恢复的人工 gate（testing 失败但可 retry、review 需要人工决策）保持 `Blocked`，但只有 shared worktree clean 时才释放 active lock；dirty 时必须继续由当前 Work Item 独占直到人工处理干净。

预期：`failed_attempt_releases_issue_shared_worktree_lock` 通过，同 Issue 后续 Work Item 不再死锁；dirty 场景由步骤 6 覆盖，不能释放锁。

### 具体路径改造清单

1. **`fail_provider_stream()`（`coding_workspace_engine.rs:1136`）**
   - 当前将 attempt 置为 `Blocked`。
   - 改为置为 `CodingAttemptStatus::Failed`。
   - 调用 `self.handle_attempt_failed(project_id, issue_id, attempt_id).await?` 后返回错误。

2. **`max_auto_rework_exceeded` 路径（约 line 4880）**
   - 保持 `Blocked`（可恢复人工 gate），保留用户继续/提供上下文的选项。
   - 但在进入 `Blocked` 后调用统一 clean-gate helper：clean 时释放 active lock；dirty 时保持锁并创建 `shared_worktree_dirty_manual_gate`。

3. **其他 provider stream 失败且无法 retry 的路径**
   - 搜索所有 `update_attempt_status(..., CodingAttemptStatus::Blocked)` 的调用点。
   - 若该路径不是人工 gate（无 retry/continue 选项），改为 `Failed` 并调用 `handle_attempt_failed`。

4. **新 attempt 取代旧 attempt**
   - 在 `create_coding_attempt`（`src/web/handlers.rs`）中，若同一 work item 已存在 active attempt，直接拒绝（见任务 1 步骤 4）。
   - 不允许“先失败旧 attempt 再创建新 attempt”的 supersede 逻辑；旧 attempt 必须由用户显式 abort/delete 或由其自身终态路径释放锁后，新 attempt 才能创建。
   - 同 Issue 不同 work item 之间的 active lock 竞争仍由 `try_acquire_issue_worktree_lock` 处理。

5. **Blocked 路径也需要 clean-gate 释放**
   - 对于保持 `Blocked` 的可恢复路径（如 testing 失败但可 retry），在进入 `Blocked` 后调用统一 clean-gate helper。
   - clean 时释放 active lock；dirty 时保持锁并创建 `shared_worktree_dirty_manual_gate`。
   - 这样既避免干净失败路径死锁，也避免脏 worktree 污染后续 Work Item。

6. **`complete_attempt_after_final_rework()`（`coding_workspace_engine.rs:4762`）**
   - 该函数内部直接置 `Completed` 并调用 `mark_work_item_completed_if_present`，**不经过 `handle_final_confirm`**。
   - 必须在该函数内先执行 shared worktree clean gate；clean 后才补充 `mark_issue_worktree_completed_item` 调用，确保锁释放。
   - dirty 时返回 `shared_worktree_dirty_manual_gate`，不得置 Completed，不得释放锁。
   - diff/verification/handoff 完整 completion gate 由 P6 抽取为 `run_completion_gates` helper 后接入；本步骤只负责 shared worktree clean gate 与锁释放。

- [ ] **步骤 6：dirty shared worktree blocks lock release**

追加失败态测试：

```rust
#[tokio::test]
async fn dirty_shared_worktree_blocks_lock_release_and_next_work_item() {
    let root = tempdir().expect("root");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(paths.clone());
    let shared_path = root.path().join("repo/.worktrees/aria-issues/issue_0001");
    std::fs::create_dir_all(&shared_path).expect("shared path");
    std::fs::write(shared_path.join("dirty.txt"), "uncommitted").expect("dirty file");
    lifecycle
        .upsert_issue_shared_worktree(UpsertIssueSharedWorktreeInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: shared_path,
            base_branch: "main".to_string(),
        })
        .expect("shared worktree");
    lifecycle
        .try_acquire_issue_worktree_lock("project_0001", "issue_0001", "work_item_0001")
        .expect("lock");
    let (store, attempt) = dirty_failed_attempt(paths.clone(), "work_item_0001");
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);

    let error = engine
        .handle_attempt_failed("project_0001", "issue_0001", &attempt.id)
        .await
        .expect_err("dirty worktree keeps lock");

    assert!(format!("{error}").contains("shared_worktree_dirty_manual_gate"));
    let shared = lifecycle
        .get_issue_shared_worktree("project_0001", "issue_0001")
        .expect("load shared")
        .expect("shared exists");
    assert_eq!(
        shared.current_active_work_item_id.as_deref(),
        Some("work_item_0001")
    );
}
```

实现要求：

- 新增 `ensure_issue_shared_worktree_clean(project_id, issue_id, work_item_id)` helper。
- helper 使用 git status porcelain 或既有 Git service 状态能力判断 shared worktree 是否 clean。
- dirty 时创建/记录人工 gate，错误 code 为 `shared_worktree_dirty_manual_gate`。
- dirty 时不 stash、不 rollback、不 release lock；人工处理 clean 后再显式继续或释放。

## 任务 4：Block Missing Handoff For Required Dependencies

**文件：**

- Modify: `src/web/handlers.rs`
- Modify: `tests/it_web/web_coding_attempt_api.rs`

- [ ] **步骤 1：编写失败态 handoff gate test**

追加:

```rust
#[tokio::test]
async fn rejects_coding_attempt_when_required_dependency_handoff_is_missing() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_completed_dependency_without_handoff(app.clone(), repo.path()).await;

    let (status, body) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0002/coding-attempts",
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "work_item_handoff_missing");
    assert_eq!(body["details"]["missing_handoffs"], json!(["work_item_0001"]));
}
```

- [ ] **步骤 2：运行 handoff gate test 并确认失败**

运行:

```bash
cargo test --locked --test it_web rejects_coding_attempt_when_required_dependency_handoff_is_missing
```

预期：当前 handler 仍允许 Coding attempt.

- [ ] **步骤 3：实现 handoff gate**

For each `required_handoff_from` ID, find the dependency Work Item in the same Issue and require `handoff_summary_ref.is_some()`.

Return:

```rust
ApiError::validation_with_details(
    "work_item_handoff_missing",
    "required dependency handoff summary is missing",
    json!({ "missing_handoffs": missing })
)
```

使用 the project’s existing `ApiError` constructor shape; do not invent a separate error response format.

- [ ] **步骤 4：运行 handoff gate test 并确认通过**

重新运行步骤 2 的命令。

预期：通过。

## 最终验证

> **v1.1 修正：** 此前使用的两个宽泛过滤名匹配不到任何现有/新增测试函数（grep 0 命中），会静默跑 0 用例假通过。下面已改为与本计划实际新增测试函数名一致的子串过滤器。

运行:

```bash
cargo test --locked --test it_web rejects_coding_attempt_when_dependency_work_item_is_not_completed
cargo test --locked --test it_web rejects_second_active_work_item_on_same_issue_shared_worktree
cargo test --locked --test it_web rejects_coding_attempt_when_required_dependency_handoff_is_missing
cargo test --locked --test it_product worktree_prepare_uses_issue_shared_worktree_path_for_issue_branch
cargo test --locked --test it_product final_confirm_releases_issue_shared_worktree_lock
cargo test --locked --test it_product failed_attempt_releases_issue_shared_worktree_lock
cargo test --locked --test it_product dirty_shared_worktree_blocks_lock_release_and_next_work_item
cargo test --locked --test it_product product_coding_workspace_engine
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo check --locked
```

预期:

- Coding attempt API gates pass.
- Shared worktree engine tests pass.
- Formatting, clippy and check pass.

## 提交

段一（任务 1-3 步骤 1-4，clean-gate 锁释放 + Failed 终态）：

```bash
git add src/web/handlers.rs src/product/coding_workspace_engine.rs src/product/lifecycle_store.rs tests/it_web/web_coding_attempt_api.rs tests/it_product/product_coding_workspace_engine.rs
git commit -m "feat: gate coding attempts and release clean shared worktree locks"
```

段二（任务 3 步骤 5-6，Blocked/Superseded/dirty 路径 clean gate）：

```bash
git add src/product/coding_workspace_engine.rs src/product/lifecycle_store.rs tests/it_product/product_coding_workspace_engine.rs
git commit -m "feat: enforce clean gate before shared worktree lock release"
```
