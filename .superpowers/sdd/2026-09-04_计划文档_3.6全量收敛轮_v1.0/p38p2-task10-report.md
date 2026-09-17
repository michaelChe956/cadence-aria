# 3.8 P2 Phase D Task 10 实施报告

## 状态

已完成：实现 `Hello.after_event_seq` 重订阅、snapshot 基线和缺口恢复；不改动 3.7 交互/引擎/审计语义。

## 变更

- attachment 初始帧改为首条入站裁决：非 cursor 首条消息先冲刷初帧对后激活直播；cursor Hello 丢弃初帧，直接进入重订阅；无入站在约 500ms 宽限后发送初帧并激活直播。短于宽限的测试 idle timeout 保持初帧先行。
- pending attachment 独立于直播 attachment，pending 窗口不接收 fan-out；cursor 重订阅在 manager 状态锁内转入直播表并冻结 journal，保证回放/补发与后续直播无遗漏，重复交给 `event_seq` 客户端去重。
- `replay_after` 命中时只依序回放，不发送 session_state 基线。
- 缺口/过旧/未来 cursor：无活跃 run 时发送当前 seq snapshot；活跃 run 时以当前 run 的第一个补发 event 减一作为 snapshot 基线，再补发当前 run journal 窗口（oracle F2）。hard-cap 截断时从实际保留的窗口左端诚实补发。
- 记录 `run_start_seq`，防止活跃 run 补发误带入上一 run 的 journal 尾窗。
- cursor Hello 晚于宽限初帧的已披露窄窗口保留诊断日志，客户端 cursor 重试吸收。
- 增加 gated streaming provider 与三条集成用例：正常 cursor 回放、活跃 run 窗外 cursor 的 F2 补发、非活跃 stale cursor snapshot 基线。

## 验证

`cargo fmt -- src/web/workspace_session/journal.rs src/web/workspace_session/manager.rs src/web/workspace_ws_handler/decisions/inbound.rs src/web/workspace_ws_handler/socket.rs tests/it_core/workspace_ws_integration/part_06.rs && cargo test --locked --test it_core workspace_ws_integration`

结果：57 passed，0 failed。

`cargo check --locked` 完成；`cargo clippy --locked --tests -- -D warnings` 完成。

## Commit

- `84bd5f1e feat(workspace): Hello after_event_seq 重订阅——journal 回放与 snapshot 基线双路径`
- `b4866fc1 fix(workspace): keep cursor recovery clippy clean`

## Concerns

- `after_event_seq` 迟于 `ATTACH_INITIAL_PUSH_GRACE` 到达时，初始 snapshot 已可能排队；服务端仅记录诊断，客户端必须按 cursor 重试吸收该 r2 已披露的窄窗口。
- 本 Task 未触碰 coding WS、durable schema 或 3.7 交互面。
