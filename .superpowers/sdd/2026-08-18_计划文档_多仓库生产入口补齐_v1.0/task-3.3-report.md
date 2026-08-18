# Task 3.3 Report：POST 后台执行、token cancel 与重启恢复

## 状态

已完成。POST 在同一请求内 begin、register lease/token，并将 lease 持有至后台 execute 完成；cancel 先持久化 cancellation，再通过 registry token 通知 worker；GET 仅对无 active lease 的 Running 记录执行 recover_interrupted。完成后以 detached spawn_blocking 安排 aggregate index 首建，不回滚初始化状态。

## TDD 证据

- 红：新增 HTTP 测试 `aggregate_initialization_post_spawns_worker_until_completed` 在修复前轮询超时，证明 POST 只创建 Created record、没有后台 execute。
- 绿：实现后台 worker、token cancel、Running 无 lease 恢复后，新增初始化入口测试全部通过。

## 验证

- `cargo test --locked --test web_logical_codebase_entrypoints initialization::aggregate_initialization_`：4 passed。
- `cargo test --locked --test web_logical_codebase_entrypoints`：17 passed。
- `cargo test --locked --lib aggregate_initialization`：27 passed。
- `cargo clippy --all-targets --all-features --locked -- -D warnings`：通过。
- `cargo fmt --check`：通过。
- `git diff --check`：通过。

## 变更文件

- `src/web/handlers/aggregate_initialization/handlers.inc.rs`：后台 spawn、lease 生命周期、token cancel、Running 无 lease 恢复、完成后 detached index build。
- `tests/web_logical_codebase_entrypoints.rs`：挂载初始化入口测试模块。
- `tests/web_logical_codebase_entrypoints/initialization.rs`：HTTP 完成、step-boundary cancel、重启恢复、new idempotency key 测试。

## 风险与后续

- index 首建为 best-effort detached task；失败只记录 tracing，不改变已完成的初始化记录，符合任务约定。
- worktree 原有未跟踪 `.pi/subagents/`、`cadence/notes/...` 保持不纳入本任务提交。
