# 3.8 P2 Phase A Task 3 报告

## 状态

已完成并提交：manager 内存回收点与进程重启诚实恢复接线。

## 实施

- `WorkspaceSessionManager` 在 `finish_run` 与 `detach` 尾部检查：无 active run、无 attachment、且无 provider drive 时，从 session registry 回收 manager。
- 回收后的下一次 websocket attach 通过既有 `get_or_create` 由 durable 状态重建 manager；新增集成回归覆盖不新增 `aborted_by_disconnect` 审计节点。
- human-gate 恢复及 Work Item Plan outline resume 从 socket 的每连接路径迁至 `WorkspaceSessionManager::create`，每个 manager 生命周期仅执行一次。
- 保持 `recover_stale_active_run_after_disconnect` 无生产调用方；未新增 `aborted_by_disconnect` 写入路径。
- socket 入口移除重复恢复块，仅负责 attach、入站和连接生命周期。

## 提交

- `b9709837 feat(workspace): manager 内存回收点与进程重启诚实恢复接线`

## 验证

- 红灯：`cargo test --locked --test it_core workspace_session_manager_recycled_after_terminal` 在实现前因 manager 未回收失败。
- 绿灯：`cargo test --locked --test it_core workspace_session_manager_recycled_after_terminal`：1 passed。
- `cargo fmt --check`：通过。
- `cargo clippy --all-targets --all-features --locked -- -D warnings`：通过。
- `cargo test --locked --lib workspace_session`：19 passed。
- `cargo test --locked --test it_core workspace_ws_integration`：46 passed。
- `cargo test --locked --lib`：3336 passed，2 ignored。
- 四项 Work Item Plan outline 恢复集成用例：各 1 passed。

## Concerns

- 无阻塞项。

## 审查修复（round 1/5）

- F1：provider drive guard 在所有会调用 `finish_run` 的终态路径先释放；回收用例改为 test-control 强制服务端 `detach` 完成后再通知 provider 终态，覆盖原死分支。
- F2：attachment 注册与 idle 摘除统一由 registry 的 sessions 锁串行；摘除在锁内核对 `Arc` 身份和 active run/attachment/drive 三项 idle 不变式。
- F3：event router 改持 `Weak<WorkspaceSessionManager>`，最后强引用回收后 router 退出，消除 router→manager→engine sender 自引用环。
- F4：创建期恢复失败改为记录并向首次 attachment 发送 `WsOutMessage::Error`，不再使 `create` 失败或 spawn provider。

### 定向验证

- `cargo check --locked`：通过。
- `cargo test --locked --test it_core workspace_session_manager_recycled_after_terminal_without_subscribers`：1 passed。
- `cargo test --locked --test it_web corrupt_outline_revision_journal_fails_closed_without_starting_provider`：1 passed。
- `cargo test --locked --lib workspace_session`：19 passed。
- `cargo test --locked --test it_core workspace_ws_integration`：46 passed。

### 修复提交

- `edf6f4c2 fix: manager 回收可达性/摘除原子性/router 生命周期/恢复错误降级`。
- 工作树中用户既有的 `cadence/` 文档改动与 `web/` 改动未暂存、未修改。
