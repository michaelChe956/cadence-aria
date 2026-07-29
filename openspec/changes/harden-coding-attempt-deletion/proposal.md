## Why

`DELETE /api/coding-attempts/{attempt_id}` 删除一个 coding attempt 时清理不完整，留下指向已删 worktree / 已删 attempt 的脏记录，阻断同一 issue 重新发起 attempt。实测：用户删掉卡住的 group coding workspace 后，`issue-shared-worktree.json` 残留 `status=running`、`worktree_path` 指向已被删除的 worktree、`current_lock_owner_id` 是已删的 attempt，重新发起 group attempt 时 aria 读这个脏记录而冲突。

具体三处缺口（`src/web/handlers/coding.rs:634` 的 `delete_coding_attempt`）：

1. **不删 `issue-shared-worktree.json`**。`handle_delete_attempt`（`handoffs.rs:203`）只调 `release_issue_shared_worktree_lock_if_holder` 释放 lock，不删 json。删 attempt 后 json 残留脏状态（status=running + lock_owner=已删 attempt + worktree_path=已删 worktree）。
2. **不清 attempt 残留 lock**：`.coding_attempt_<id>.json.lock`（孤儿）、`.group-initialization-arbitration.lock`（group scope）、`work-item-attempt-locks/<wi>.lock`。
3. **`cleanup_coding_attempt_workspace` 不容错**（`support.rs:359`）：worktree 目录缺失时 `remove_worktree` / `prune_worktrees` / `delete_local_branch` 失败，整个删除中断。

**根因与 `harden-work-item-group-deletion` 同模式**：清理路径只清自身数据，不连带清 issue 级共享产物（shared-worktree + lock），且要求被清理对象（worktree）处于健康状态。

## What Changes

- **尽力清理 + 容错**：删 attempt 自身数据后，尽力清 shared-worktree + 残留 lock。每步「NotFound=OK」，绝不因缺失中断。
- **shared-worktree 条件清理**（用户确认方案 A）：删 attempt 后，若该 issue **无其他 attempt 记录**，删 `issue-shared-worktree.json` + `.lock`（复用 `LifecycleStore::delete_issue_shared_worktree`，`harden-work-item-group-deletion` 已实现）。有其他 active attempt 则保留（它们仍在用 shared-worktree）。
- **attempt 残留 lock 清理**：`.coding_attempt_<id>.json.lock`（孤儿）、`.group-initialization-arbitration.lock`（group scope）、`work-item-attempt-locks/<wi>.lock`（该 attempt 各 work_item）。复用 `harden-work-item-group-deletion` 的 `purge_attempt_lock_residue` 模式（按 work_item 精确删 lock，不整目录）。
- **worktree 容错**：`cleanup_coding_attempt_workspace` 在 worktree 目录缺失时跳过 git 回滚（`remove_worktree` / `prune_worktrees` / `delete_local_branch` 容错），不因 worktree 已删而失败。这是 `harden-work-item-group-deletion` design 已知缺口一的正式修复。
- **不误伤**：有其他 active attempt 时保留 shared-worktree；lock 清理只针对该 attempt 的 work_item，不触碰其他 attempt 的 lock。

## 非目标

- 不改 group 删除路径（`harden-work-item-group-deletion` 已修复）。
- 不改 attempt 创建 / 推进 / abort 逻辑。
- 不改 `coding_runs` 的运行时 abort（已有）。
- 不迁移历史脏数据（按全新系统处置；用户已手动清理当前脏数据）。
- 不改 `handle_delete_attempt` 的 lock 释放逻辑（已正确，只是不删 json）。

## Capabilities

### New Capabilities

- `coding-attempt-deletion-resilience`：coding attempt 删除的健壮性契约，包括 shared-worktree 条件清理、残留 lock 清理、worktree 缺失容错、以及不误伤同 issue 其他 attempt。

### Modified Capabilities

（无。现有 specs 未覆盖 attempt 删除的清理健壮性。）

## Impact

- `src/web/handlers/coding.rs`：`delete_coding_attempt` 在 `delete_attempt` 后加 shared-worktree 条件清理 + 残留 lock 清理。
- `src/web/handlers/support.rs`：`cleanup_coding_attempt_workspace` 加 worktree 缺失容错（worktree 不存在时跳过 git 回滚）。
- 复用 `LifecycleStore::delete_issue_shared_worktree`（`lifecycle_store/worktree.rs`）、`remove_file_if_exists` / `remove_dir_all_if_exists`（`lifecycle_store/utils.rs`）、`harden-work-item-group-deletion` 的 `purge_attempt_lock_residue` 模式。
- 受影响的用户可见行为：删 attempt 后该 issue 无其他 attempt 时 shared-worktree 一并清理，重新发起 attempt 不再因脏 shared-worktree 冲突；worktree 已手动删除时删 attempt 不报错。

## 依赖与顺序

本 change 与 `harden-work-item-group-deletion`（已完成）同模式但独立。两者复用同一批清理工具（`delete_issue_shared_worktree`、`remove_*_if_exists`），无实现冲突。
