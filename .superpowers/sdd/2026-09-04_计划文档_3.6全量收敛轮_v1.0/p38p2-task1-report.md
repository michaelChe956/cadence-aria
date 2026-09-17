# 3.8 P2 Phase A Task 1 实施报告

## 状态

**已完成并验证。** Task 1 已将 workspace session 的 engine 所有权迁入 `WorkspaceSessionManager`，提供同 session 单实例 registry、非阻塞 attach durable 投影回退，以及 engine 事件向 workspace WS attachment 的广播。未触及 `coding_ws_handler` 与前端。

## 提交

- `1a62d435 feat(workspace): session-owned manager 持有 engine 与事件源`
- 本报告与其后 Task 1 收尾修复将在本次收尾提交中纳入。

工作树原有的 `final-fix-report.md` 删除未纳入暂存或提交。

## 实现要点

- `WorkspaceSessionRegistry` 以按 session id 的创建锁串行同键 factory：构造不占用 sessions map 锁，且同 session 不会并发构造多个 engine。
- `WorkspaceSessionManager::create` 接管原 socket-local 的 durable 加载、checkpoint 恢复、`WorkspaceEngine::new_persistent`、logical gateway 与 plan repair artifact 初始化链路。
- `attach` 先登记 attachment，随后用两次 `try_lock` 获取 live snapshot；首次失败后等待 250ms，两次均失败时通过只读 durable 投影器返回状态。投影器不执行 plan repair、不启动 provider、不注册 active run、不订阅 engine event。
- manager router 订阅唯一 engine event receiver，将 artifact、stream、stage 和 choice 等事件向所有 workspace attachment 广播；choice request 保留 `workspace_runs.register_choice` 副作用，`TextFallback` 不登记。
- 正在运行 provider 时，附加连接跳过 socket startup recovery，避免 recovery 等待 engine 锁而阻塞其 Abort/ChoiceResponse receiver loop。
- 保留仅测试的 legacy event forwarder，避免破坏既有 handler 单测；生产 socket 不再创建 per-socket forwarder。
- 增加 registry 并发单测：16 个并发 `get_or_create` 调用只执行一次 factory，且全部返回同一 `Arc<WorkspaceSessionManager>`。

## 已验证

- 红灯（实现前）：`cargo test --locked --test it_core workspace_ws_multiple_connections_share_one_session_manager` 缺少 `workspace_session` 模块和 `WebAppState.workspace_sessions`，按预期失败（`artifact://1648`）。
- `cargo check --locked`：通过。
- `cargo fmt --check`：通过。
- `cargo clippy --all-targets --all-features --locked -- -D warnings`：通过。
- `cargo test --locked --lib workspace_session`：17 passed。
- `cargo test --locked --lib workspace_ws_handler`：116 passed。
- `cargo test --locked --test it_core workspace_ws_integration`：45 passed。
- 重点覆盖用例均通过：
  - `workspace_ws_multiple_connections_share_one_session_manager`
  - `workspace_ws_second_connection_attaches_while_run_holds_engine_lock`
  - `workspace_ws_passive_connection_receives_live_stream_events`
  - `workspace_ws_secondary_connection_can_abort_active_run_started_by_primary`
  - `workspace_ws_claude_author_ask_user_question_choice_continues_same_provider`
  - `workspace_ws_stale_choice_response_after_new_run_is_rejected_before_provider`
  - `workspace_ws_hello_during_pending_choice_does_not_block_choice_response`

## 约束复核与 concern

- 未触及 `coding_ws_handler`、前端、wire role/cursor、lease 或 durable schema。
- 3.7 的 human gate、stage 校验、诊断及 transitional close 逻辑仍在 socket；Task 2/3 的 run 所有权与 recovery 迁移不在本 Task 范围内。
- 附加连接只在已有 active workspace run 时跳过 startup recovery；不存在 active run 时保持既有 recovery 行为。
- 本 Task 未宣称根治断连主嫌；断连责任归并仍属后续 Task 范围。
