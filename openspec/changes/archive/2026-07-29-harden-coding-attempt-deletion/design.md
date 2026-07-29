## 背景

coding attempt 的删除路径与 group 删除路径共享同一组 issue 级产物（shared-worktree、coding-attempts 下的 lock），但 attempt 删除路径的清理不如 group 删除完整。`harden-work-item-group-deletion` 已把 group 删除改为「尽力清理 + 容错」，本 change 把同一原则应用到 attempt 删除。

当前 `delete_coding_attempt`（`coding.rs:634`）的清理：

| 产物 | 当前清理 | 处置 |
|---|---|---|
| attempt json + 目录 | ✓ `delete_attempt` | 保留 |
| handoff revisions | ✓ `cleanup_attempt_handoff_revisions` | 保留 |
| shared-worktree **lock 释放** | ✓ `handle_delete_attempt` → `release_issue_shared_worktree_lock_if_holder` | 保留 |
| shared-worktree **json** | ✗ 不删 | 新增条件清理 |
| `.coding_attempt_<id>.json.lock` | ✗ 不清 | 新增 |
| `.group-initialization-arbitration.lock` | ✗ 不清 | 新增 |
| `work-item-attempt-locks/<wi>.lock` | ✗ 不清 | 新增 |
| worktree + 分支 | ✓ `cleanup_coding_attempt_workspace` | 加容错 |

## 根因

三处缺口同源于「清理路径不连带清 issue 级共享产物 + 要求被清理对象健康」：

1. `handle_delete_attempt`（`handoffs.rs:203`）只 `release_issue_shared_worktree_lock_if_holder`（清 lock_owner），不删 `issue-shared-worktree.json`。删 attempt 后 json 残留脏状态。
2. `delete_coding_attempt` 不清 coding-attempts 下的残留 lock（attempt 自身的 `.lock`、group 仲裁 lock、work-item-attempt-locks）。
3. `cleanup_coding_attempt_workspace`（`support.rs:359`）在 worktree 目录缺失时 `remove_worktree` / `prune_worktrees` / `delete_local_branch` 直接失败，中断整个删除。

## 决策

### 决策一：shared-worktree 条件清理（方案 A）

删 attempt 后，若该 issue **无其他 attempt 记录**，删 `issue-shared-worktree.json` + `.lock`。

理由：
- shared-worktree 是 issue 级，single + group attempt 共用。无条件删会在「多 attempt 共 issue」时误删其他 active attempt 正在用的 shared-worktree。
- 「该 issue 无其他 attempt」用 `CodingAttemptStore` 的 issue 级 list 判定（删当前 attempt 后 list 为空即无其他）。
- 当前一个 issue 通常一个 active attempt，条件 A 与无条件删在常见场景等价，但 A 在多 attempt 时安全。
- 复用 `LifecycleStore::delete_issue_shared_worktree`（`harden-work-item-group-deletion` 已实现，NotFound=OK）。

### 决策二：残留 lock 按标识精确清理

- `.coding_attempt_<id>.json.lock`：删当前 attempt 的（attempt_id 已知）。
- `.group-initialization-arbitration.lock`：仅 group scope attempt 清（single attempt 不持有）。
- `work-item-attempt-locks/<wi>.lock`：按该 attempt 的 work_item（group scope 用各 unit 的 work_item；single 用 attempt.work_item_id）精确删，**不整目录**（其他 attempt 的 work_item lock 可能共存）。

复用 `harden-work-item-group-deletion` 的 `purge_attempt_lock_residue` 模式。但 attempt 删除的 work_item 范围与 group 删除不同（group 用 plan.work_item_ids，attempt 用 attempt 的 active work_item + group 的 unit work_items），实现时按 attempt 实际 work_item 集精确删。

### 决策三：worktree 容错

`cleanup_coding_attempt_workspace` 的三个 git 操作（remove_worktree / prune_worktrees / delete_local_branch）改为：worktree 目录不存在时跳过（`NotFound` 视为成功），其他 git 错误仍返回。这覆盖「用户已手动删 worktree」的常见场景，与 `harden-work-item-group-deletion` 的 worktree 容错原则一致。

### 决策四：尽力清理，每步 NotFound=OK

shared-worktree 与 lock 清理都用 `remove_file_if_exists` / `remove_dir_all_if_exists`（`lifecycle_store/utils.rs:159,170`），缺失视为成功。绝不因某项产物已不存在而中断整个 attempt 删除。

### 决策五：清理顺序

`delete_attempt`（删 attempt 数据）→ 清残留 lock → 条件清 shared-worktree。先删 attempt 数据，再清依赖（lock 持有者已不存在，清 lock 安全）；shared-worktree 最后（条件依赖「无其他 attempt」，此时 attempt 已删，list 反映剩余）。

## 边界

- 不改 group 删除（`harden-work-item-group-deletion` 已修复）。
- 不改 `handle_delete_attempt` 的 lock 释放（已正确，只是不删 json）。
- 不改 attempt 创建 / 推进 / abort。
- 不改 `coding_runs.abort_attempt`（运行时 abort，已有）。
- 不迁移历史脏数据。

## 已知缺口

1. **`release_issue_shared_worktree_lock_if_holder` 与 shared-worktree json 删除的语义边界**：`handle_delete_attempt` 释放 lock（清 current_lock_owner_id），本 change 删 json（条件：无其他 attempt）。两者顺序：release lock 在前（handle_delete_attempt），删 json 在后（delete_attempt 后）。若 release lock 失败（attempt 不持有），不影响后续删 json（条件独立）。实现时确认 release 与删 json 不互相干扰。
2. **multi-attempt 共 issue 的 shared-worktree 保留**：若同 issue 有多个 attempt（如一个 single + 一个 group），删一个时另一个的 shared-worktree 保留。当前一个 issue 通常一个 active attempt，此场景罕见，但条件 A 已覆盖。
