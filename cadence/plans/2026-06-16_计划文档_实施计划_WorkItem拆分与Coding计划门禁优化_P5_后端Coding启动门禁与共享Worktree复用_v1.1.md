# WorkItem 拆分 P5 后端 Coding 启动门禁与共享 Worktree 复用 Implementation Plan

> **文档版本：** v1.1
>
> **v1.1 修订摘要：** 修复 active lock 在异常终态/attempt 被取代时不释放导致的死锁（新增任务 3 的异常终态释放步骤，并将原先引用的不存在方法 `handle_attempt_failure` 修正为需新增的 `handle_attempt_failed`）；修正最终验证过滤名为本计划实际新增测试函数名；补充新增 test helper 的显式步骤；点名 branch 改为 `aria/issues/{issue_id}` 后需同步更新基于 `aria-work-items/{work_item}/attempt-{n}` 的既有断言测试；明确严格依赖 P1/P3/P4 已合并并要求字段名与上游逐字对齐（注意 `exclusive_write_scopes` 与现有 `WorkItemRecord.allowed_write_scope` 命名漂移）。

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Coding 启动前检查依赖完成、Issue 共享 worktree 准备、active Work Item 串行锁、写入范围和 handoff 可读性，并让同一 Issue 下的 Coding attempt 复用 Issue 共享 branch/worktree。

**Architecture:** `create_coding_attempt` 负责启动前门禁和 branch 选择；`CodingWorkspaceEngine::execute_worktree_prepare` 负责按 attempt 中的 Issue branch/worktree 创建或复用 worktree；`LifecycleStore` 提供 active lock。第一版仍保持同一 Issue 同一时刻只有一个 active Work Item，避免共享 worktree 下 `git add -A` 污染其他 Work Item。

**Tech Stack:** Rust 1.95.0、Axum、LifecycleStore、CodingAttemptStore、GitWorkspaceService、Cargo integration tests、TDD。

---

## 前置交付摘要

执行本计划前确认：

- P3 已让 `generate_work_items` 创建多个 Work Item，并写入 `depends_on`、`exclusive_write_scopes`、`forbidden_write_scopes`、`required_handoff_from`。
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

- [ ] **步骤 5：运行 gate tests 并确认通过**

Run the two commands from Step 3 again.

预期：两条测试都通过。

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

## 任务 3：Release Active Lock On Completion And Abort

> **🔴 v1.1 阻塞修复：** 原计划只在 `handle_final_confirm`（完成）和 `handle_abort`（中止）两个路径释放 active lock。若 attempt 走到**其他终态**（Coding/Review 失败、卡 gate 后被同 Issue 新 attempt 取代、provider run 异常退出等），锁不会释放，会导致同 Issue 后续 Work Item 永久卡在 `issue_worktree_active` 死锁。本任务新增步骤 5 覆盖异常终态/attempt 被取代时的释放逻辑。

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

在 `handle_final_confirm()`, after updating the Work Item to completed, call:

```rust
LifecycleStore::new(self.store.paths())
    .mark_issue_worktree_completed_item(project_id, issue_id, &updated.work_item_id)?;
```

在 `handle_abort()`, call:

```rust
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

随后在实现中，确保**所有 attempt 终态收敛点**都释放锁（而非仅完成/中止）：

1. 新增 `pub async fn handle_attempt_failed(&self, project_id: &str, issue_id: &str, attempt_id: &str) -> Result<(), CodingWorkspaceError>`：
   - 加载 attempt 及对应 Work Item。
   - 将 attempt 状态置为 `CodingAttemptStatus::Failed`（若尚未置为 Failed）。
   - 调用 `release_issue_worktree_lock(project_id, issue_id, &work_item_id)`，仅当当前持锁 Work Item 与本 attempt 的 Work Item 一致时才释放，避免误释放后续 attempt 的锁。
   - 缺少 shared worktree 记录时按 backward-compatible no-op 处理。
2. 在已有 `handle_final_confirm`（Completed）和 `handle_abort`（Aborted）路径中保留并复核锁释放逻辑。
3. 其他收敛到 Failed/Superseded/Blocked 等终态的代码路径（如 provider run 异常退出、attempt 被新 attempt 取代）应统一调用 `handle_attempt_failed` 或在本地执行同样的「仅当持锁者匹配才释放」逻辑。
4. 新 attempt 抢占（取代旧 attempt）时，应在创建新 attempt 的锁获取逻辑中处理旧持锁项的释放/接管，避免旧项残留锁。

> 若团队决定将「异常终态释放锁」收口到统一状态机改造而不在本计划内完成，则必须显式将其列为已知后续项并在 P6 或后续计划承接；本 v1.1 推荐直接在本步骤补齐实现。

预期：`failed_attempt_releases_issue_shared_worktree_lock` 通过，同 Issue 后续 Work Item 不再死锁。

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

```bash
git add src/web/handlers.rs src/product/coding_workspace_engine.rs src/product/lifecycle_store.rs tests/it_web/web_coding_attempt_api.rs tests/it_product/product_coding_workspace_engine.rs
git commit -m "feat: gate coding attempts on split work items"
```
