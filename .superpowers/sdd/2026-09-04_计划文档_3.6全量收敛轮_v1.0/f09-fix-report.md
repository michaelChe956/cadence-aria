# F-09 修复报告

## 问题与根因

已连接页面只收到 `stage_change` 与时间线增量；`single_candidate_phase=approval` 和 `human_gate_snapshot` 仅在 attach 的 `session_state` 中投影。因此同一连接上的旧页面在开门后仍保留旧相位，而新开页面通过初始快照立即正确。

另有传输风险：attachment 出站队列满后会被永久标记 `degraded`，且单发的 `resync_required` 也可能因队列已满而丢失。

## 修复

1. 增加 `EngineEvent::HumanGateOpened { stage }`，并由 `enter_human_confirm` 在状态、时间线节点和 durable 会话状态更新完成后发送。
2. `WorkspaceSessionManager` 的事件 router 专门处理该事件：持有 engine 锁构建当前 `session_state`，再走现有 `broadcast` 管线；`map_engine_event` 对该 runtime-only 事件返回 `None`，避免重复映射。
3. degraded attachment 改为可恢复：后续广播为其生成带当前 `event_seq` 的完整 `session_state` 基线，单次 `try_send` 成功后清除 degraded 标记；满队列时保持降级，关闭通道时移除 attachment。取消了会丢失的 `ResyncRequired` 单帧依赖。

## 红绿证据

### 红

实现前新增锚按预期失败：

- `manager_recovers_degraded_attachment_with_session_state_baseline`：断言「队列恢复容量后必须摘除 degraded 标记」失败。
- `human_gate_open_rebuilds_session_state_for_existing_attachments`：等待 `session_state` 超时。

### 绿

- `cargo test --lib human_gate_open_rebuilds_session_state_for_existing_attachments`：1 passed。
- `cargo test --lib manager_recovers_degraded_attachment_with_session_state_baseline`：1 passed。
- `cargo test --lib enter_human_confirm_emits_human_gate_opened_after_gate_state_is_current`：1 passed。
- `cargo test --lib conversational_gate_abandon_is_terminal_without_compile`：1 passed（既有关门链）。
- `cargo test --lib workspace_session`：28 passed。
- `cargo test --lib workspace_ws_handler`：120 passed。
- `cargo test --lib decisions`：4 passed。
- `cargo fmt --check -- ...`：通过。
- `cargo clippy --lib -- -D warnings`：通过。

## 风险面与结论

- `event_seq` 仍由 `broadcast` 单调分配；恢复基线带当前序号，使后续增量持续可去重。
- 门开启 router 分支在锁内只构建全量会话状态；RCA 约束的门开启无在途流式，且 batch pause 频率低。
- session_state 的 chat entries 重建沿用现有 attach/resubscribe 投影路径，未引入前端协议变更。
