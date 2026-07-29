# Coding Attempt 删除健壮化 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development 或 superpowers:executing-plans 实施。Steps use checkbox (`- [ ]`)。

**Goal:** `DELETE /api/coding-attempts/{id}` 清理完整——删 attempt 后条件清 shared-worktree、清残留 lock、worktree 缺失容错，无残留、不误伤同 issue 其他 attempt。

**Architecture:** 同 `harden-work-item-group-deletion` 模式（尽力清理 + 容错）。复用 `delete_issue_shared_worktree`、`remove_file_if_exists`/`remove_dir_all_if_exists`、`purge_attempt_lock_residue` 的精确删 lock 模式。

**Change:** `harden-coding-attempt-deletion`（OpenSpec 契约 strict-valid 并获用户确认）。展开 tasks 1.1–3.5，对应 spec 四组 requirement。

## Global Constraints

- 严格 TDD。`cargo test --locked`（🔴 禁止 `-j 1`）、`cargo clippy --all-targets --all-features --locked -- -D warnings`、`cargo fmt --check`。
- 复用 `harden-work-item-group-deletion` 已实现的工具：`LifecycleStore::delete_issue_shared_worktree`（`lifecycle_store/worktree.rs`）、`remove_file_if_exists`/`remove_dir_all_if_exists`（`lifecycle_store/utils.rs:159,170`）。
- 不改 `handle_delete_attempt`（lock 释放已正确）、不改 group 删除路径、不改 attempt 创建/推进。

## File Structure

| 文件 | 职责 |
|---|---|
| `src/web/handlers/support.rs` | `cleanup_coding_attempt_workspace` 加 worktree 缺失容错 |
| `src/web/handlers/coding.rs` | `delete_coding_attempt` 加残留 lock 清理 + shared-worktree 条件清理 |
| `tests/it_web/web_coding_attempt_api/` | 端到端测试（沿用现有 attempt 删除测试模式） |

---

### Task 1: cleanup_coding_attempt_workspace worktree 容错

**Files:** Modify `src/web/handlers/support.rs:359`

当前实现：
```rust
pub(crate) async fn cleanup_coding_attempt_workspace(
    repository: &RepositoryRecord,
    attempt: &CodingExecutionAttempt,
) -> ApiResult<()> {
    let git = GitWorkspaceService::new();
    if let Some(worktree_path) = attempt.worktree_path.as_ref() {
        git.remove_worktree(&repository.path, worktree_path).await.map_err(git_workspace_api_error)?;
    }
    git.prune_worktrees(&repository.path).await.map_err(git_workspace_api_error)?;
    git.delete_local_branch(&repository.path, &attempt.branch_name).await.map_err(git_workspace_api_error)?;
    Ok(())
}
```

- [ ] **Step 1.1: 写失败测试** — worktree 目录已被删除时，`cleanup_coding_attempt_workspace` 不报错（返回 Ok）。在 support 测试或 it_web 构造：attempt.worktree_path 指向不存在的目录，调用应成功。
- [ ] **Step 1.2: 跑确认失败**（当前 remove_worktree/prune/delete_branch 在 worktree 不存在时报 git 错）
- [ ] **Step 1.3: 实现** — worktree_path 目录不存在时跳过 `remove_worktree`（`path.exists()` 判定）；`prune_worktrees` 仍执行（prune 本就容错无 worktree 情况）；`delete_local_branch` 容错（分支不存在视为成功，或检查 GitWorkspaceService 是否已有 NotFound 处理）。实现时看 `GitWorkspaceService` 各方法的错误类型，把 `NotFound`/worktree-missing 视为成功。
- [ ] **Step 1.4: 跑确认通过**

### Task 2: delete_coding_attempt 残留 lock 清理

**Files:** Modify `src/web/handlers/coding.rs:634`（`delete_coding_attempt`，在 `delete_attempt` 调用后）

- [ ] **Step 2.1: 写失败测试** — 删 group attempt 后，`.coding_attempt_<id>.json.lock`、`.group-initialization-arbitration.lock`、各 work_item 的 `work-item-attempt-locks/<wi>.lock` 被删；另一 attempt 的 work_item lock 保留。
- [ ] **Step 2.2: 跑确认失败**
- [ ] **Step 2.3: 实现** — 在 `delete_attempt`（coding.rs:718）成功后加残留 lock 清理：
  - `.coding_attempt_<attempt_id>.json.lock`（coding-attempts 顶层，attempt_id 已知）
  - `.group-initialization-arbitration.lock`（仅 `attempt.scope == WorkItemGroup`）
  - `work-item-attempt-locks/<wi>.lock` + `<wi>`：按该 attempt 的 work_item（group 用各 unit 的 logical_work_item_id，single 用 attempt.work_item_id）逐个 `remove_file_if_exists`，**不整目录**。
  - 复用 `crate::product::lifecycle_store::{remove_file_if_exists, remove_dir_all_if_exists}`。路径基础用 `app_paths.issue_root(project_id, issue_id).join("coding-attempts")`。
- [ ] **Step 2.4: 跑确认通过**

### Task 3: delete_coding_attempt shared-worktree 条件清理

**Files:** Modify `src/web/handlers/coding.rs`（Task 2 的清理之后）

- [ ] **Step 3.1: 写失败测试** — 删 attempt 后该 issue 无其他 attempt → shared-worktree.json + .lock 不存在；有其他 attempt → 保留。
- [ ] **Step 3.2: 跑确认失败**
- [ ] **Step 3.3: 实现** — 残留 lock 清理后：list 该 issue 的 attempt（`coding_store.list_attempts_for_issue` 或对应方法，用 `ast-grep outline` 确认 CodingAttemptStore 的 issue 级 list 方法名），若为空（当前 attempt 已删，list 不含它）→ `LifecycleStore::delete_issue_shared_worktree(project_id, issue_id)`（NotFound=OK）。
- [ ] **Step 3.4: 跑确认通过**

### Task 4: 验证

- [ ] **Step 4.1:** `cargo fmt --check`
- [ ] **Step 4.2:** `cargo clippy --all-targets --all-features --locked -- -D warnings`
- [ ] **Step 4.3:** `cargo test --locked --lib`、`cargo test --locked --test it_web`
- [ ] **Step 4.4:** 确认 large_file_guard 无新增超限；现有 attempt 删除测试语义未破坏（如有编码旧行为的测试，按新语义调整）。

## Self-Review

- **Spec 覆盖**：Task 1 → worktree 容错 requirement；Task 2 → lock 清理 + 不误删其他 attempt lock；Task 3 → shared-worktree 条件清理（无其他删/有保留）；不误伤其他 attempt 由 Task 2/3 的精确性 + 条件保证。
- **复用**：`delete_issue_shared_worktree`、`remove_*_if_exists` 来自 `harden-work-item-group-deletion`，不新造。
- **顺序**：delete_attempt → lock 清理 → shared-worktree 条件清理（list 此时反映剩余 attempt）。
