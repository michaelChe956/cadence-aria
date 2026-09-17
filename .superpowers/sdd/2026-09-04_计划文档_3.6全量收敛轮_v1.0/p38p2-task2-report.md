# 3.8 P2 Phase A Task 2 实施报告

## 状态

**已完成并验证。** Task 2 将 workspace provider run 的唯一状态所有权收编到 `WorkspaceSessionManager` 的单一 `ManagerState` 临界区；`WorkspaceRunRegistry` 仅保留 provider drive-depth。未触及 `src/product/issue_store.rs`、`src/product/id.rs`、`coding_ws_handler` 或前端。

## 提交

- `a41baa3a refactor(workspace): 收编运行所有权至 session manager`
- 本报告及 Task 2 测试夹具、集成测试和语义回归收尾将在后续提交纳入。

## 实现要点

- `workspace_session::ActiveRun` 承载 `id`、token、node、取消 token、命令 sender、待处理 choice 集和 `lease_epoch`；`ManagerState` 持有 `next_run_id`、`active_run` 与 `LeaseState`。
- `start_run` 在 manager state 锁内完成旧 run 取消、run id/token 分配和新 `active_run` 写入；`finish_run` 以 token 判等清除，迟到收尾不会删除新 run；显式 Abort 统一走 `abort_active_run`。
- `ProviderRunContext` 持有共享 manager；provider 事件启动、followups 命令 sender 替换、Choice/Permission/Abort、idle guard、诊断和过渡期 close 都从 manager 读取 run 状态。
- 保留 Task 2 过渡期 close 行为：关闭仍写入 `append_aborted_by_disconnect` 并回到 PrepareContext；Task 4 才会改为纯 detach。
- 为保持 3.7 行为，handler-originated run 在等待 engine 锁和 provider 可用性检查前先通过 manager 取消旧 run；这保证用户新消息可立即中断流式 run，也保证旧 choice 不会进入替代后的 run。
- `WorkspaceRunRegistry` 的 run map 及 `WorkspaceActiveRun` 已删除；保留且仅保留 provider drive-depth，供跨 socket idle guard 使用。
- 迁移所有 workspace handler 单测夹具；集成测试改由 `WorkspaceSessionRegistry::get` 读取 manager 的 active run。

## 已验证

- 红灯（实现前）：`cargo test --locked --lib workspace_session` 因 `start_run` / `active_run` / `finish_run` / `abort_active_run` 未定义，按预期失败。
- `cargo fmt --check`：通过。
- `cargo check --locked`：通过。
- `cargo clippy --all-targets --all-features --locked -- -D warnings`：通过。
- `cargo test --locked --lib workspace_session`：19 passed。
- `cargo test --locked --lib workspace_ws_handler`：116 passed。
- `cargo test --locked --lib workspace_engine`：1227 passed，1 ignored。
- `cargo test --locked --test it_core workspace_ws_integration`：45 passed。
- `cargo test --locked --lib`：3336 passed，2 ignored。
- 重点回归：`workspace_ws_user_message_interrupts_active_stream_before_completion`、`workspace_ws_stale_choice_response_after_new_run_is_rejected_before_provider`、`workspace_ws_secondary_connection_can_abort_active_run_started_by_primary` 与 `workspace_ws_disconnect_during_active_run_writes_aborted_by_disconnect` 均通过。

## 约束复核与 concern

- `workspace_runs` 在 handler 中仅作为 provider drive-depth 依赖和测试夹具参数保留，不再提供 run 读取、写入、choice 登记或取消接口。
- `start_run` 内部取消路径使用非阻塞 `try_send(ProviderCommand::Abort)` 并取消 token；token 是 provider 等待的可靠取消信号，仍保持 Abort/supersede 的既有对外行为。
- 未宣称断连主嫌已根治；本 Task 按阶段要求保留 transitional close 终态写入。