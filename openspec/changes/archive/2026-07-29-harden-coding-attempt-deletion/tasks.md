## 1. 失败测试

- [x] 1.1 shared-worktree 条件清理：删 attempt 后该 issue 无其他 attempt → shared-worktree.json + .lock 不存在。（映射：Requirement 条件清理 shared-worktree）
- [x] 1.2 shared-worktree 保留：删 attempt 后该 issue 仍有其他 attempt → shared-worktree.json 保留。（映射：同上）
- [x] 1.3 残留 lock 清理：删 group attempt 后 `.coding_attempt_<id>.json.lock`、`.group-initialization-arbitration.lock`、各 work_item 的 `work-item-attempt-locks/<wi>.lock` 被删。（映射：Requirement 清理残留 lock）
- [x] 1.4 不误删其他 attempt 的 lock：同 issue 另一 attempt 的 work_item lock 在 `work-item-attempt-locks/`，删本 attempt 后另一 attempt 的 lock 仍在。（映射：同上）
- [x] 1.5 worktree 缺失不阻断：attempt 的 worktree 目录已被删除时，DELETE coding-attempt 成功（不报 git 错误）。（映射：Requirement worktree 缺失不阻断）
- [x] 1.6 不误伤其他 attempt：同 issue 多 attempt 删一个，其他 attempt 记录/shared-worktree/worktree 不受影响。（映射：Requirement 不得误伤）
- [x] 1.7 确认以上测试全部失败且失败原因是缺少实现。

## 2. 实现

- [x] 2.1 `support.rs` 的 `cleanup_coding_attempt_workspace` 加 worktree 缺失容错：worktree 目录不存在时跳过 `remove_worktree`/`prune_worktrees`/`delete_local_branch`（NotFound=OK）。（映射：Requirement worktree 缺失不阻断）
- [x] 2.2 `coding.rs` 的 `delete_coding_attempt` 在 `delete_attempt` 后加残留 lock 清理：删 `.coding_attempt_<id>.json.lock`、（group scope）`.group-initialization-arbitration.lock`、该 attempt 各 work_item 的 `work-item-attempt-locks/<wi>.lock`。复用 `remove_file_if_exists`，按 work_item 精确删（不整目录）。（映射：Requirement 清理残留 lock）
- [x] 2.3 `delete_coding_attempt` 在残留 lock 清理后加 shared-worktree 条件清理：list 该 issue 的 attempt，若为空（无其他 attempt）调 `LifecycleStore::delete_issue_shared_worktree`（复用 `harden-work-item-group-deletion` 已实现，NotFound=OK）。（映射：Requirement 条件清理 shared-worktree）
- [x] 2.4 确认 `handle_delete_attempt` 的 `release_issue_shared_worktree_lock_if_holder` 与新增的 shared-worktree json 删除顺序不互相干扰（release 在前，删 json 在 delete_attempt 之后）。

## 3. 验证

- [x] 3.1 `cargo fmt --check`
- [x] 3.2 `cargo clippy --all-targets --all-features --locked -- -D warnings`
- [x] 3.3 `cargo test --locked --lib`
- [x] 3.4 `cargo test --locked --test it_web`
- [x] 3.5 线上数据验证：删一个 attempt 后该 issue 无其他 attempt 时 shared-worktree 清理、可立即重新发起 attempt 而不冲突。（441047bd 删除验证通过：attempt 数据 + `.coding_attempt_<id>.json.lock` + `.group-initialization-arbitration.lock` + `work-item-attempt-locks/<wi>.lock` + `issue-shared-worktree.json` 全清，无残留）
