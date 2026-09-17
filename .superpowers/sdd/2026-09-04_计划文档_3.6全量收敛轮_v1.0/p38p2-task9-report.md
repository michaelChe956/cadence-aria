# 3.8 P2 Phase D Task 9 完成报告

## 状态

已完成。workspace session manager 现在独占事件序号、事件 journal 与广播 fan-out：runtime 事件在序列化边界加入可选顶层 `event_seq`，将同一份 stamped JSON 写入 session-owned journal，再以非阻塞 `try_send` 发往每个 attachment。没有修改任何 `WsOutMessage` 变体字段或 durable schema。

## 交付内容

- 新增 `EventJournal`：活动 run 期间不按尾窗裁剪；run 正常完成或显式中止后裁至 `JOURNAL_TAIL = 1_024`；始终受 `JOURNAL_HARD_CAP = 65_536` 限制。窗口内 cursor（含 `oldest_seq - 1` 的 attach 基线）严格回放其后的事件；hard cap 丢弃最早事件并永久标记 `truncated`，任意 cursor 重放返回 `None`，为后续 snapshot 基线路径提供诚实降级判定。
- manager 维护 session 生命周期内的 `AtomicU64` 序号。router 的路径固定为：map → serialize → stamp → journal.push → 对全部 attachment `try_send`。每个 attachment 收到同一原始 JSON 文本和同一个 `event_seq`。
- `finish_run` 与 manager 的显式 abort 路径标记 journal run terminal，确保 run 完成后执行尾窗裁剪。
- attach 的 `session_state` 在序列化边界写入当前 `event_seq` 基线（首 attach 为 `0`），仍保留原有 `connection_id` 装饰。已写入 T10 接口注释：携带 `after_event_seq` 的 Hello 必须延迟/抑制该初始基线帧，避免回放被前端去重吞掉；本 Task 不提前改变该既有 attach 时序。
- 增加单元测试：journal 窗口、run 终态尾窗与 hard-cap 降级；两 attachment 的单调序号和同一 stamped 事件。
- 增加真实 workspace WS 集成测试：两个连接获得相同 attach 基线，显式 Observer 不接管 lease，随后的 live stream chunk 在两端有相同且大于基线的 `event_seq`。

## TDD 证据

红灯：先创建空 `journal.rs` 并写入 journal 与 manager fan-out 用例，`cargo test --locked --lib workspace_session` 按预期因 `EventJournal`、`attach`、`broadcast_test_event` 缺失而编译失败。

绿灯：最小实现后，`cargo test --locked --lib workspace_session` 通过（24 passed）。新增 WS 集成用例的首次失败显示 observer 在未发送 Hello 时按 legacy Driver 接管 lease，导致 driver 写被拒；测试改为显式 `HelloRole::Observer` 后，定向用例通过，验证的是实际角色/广播契约而非 mock。

## 验证

```text
cargo fmt --check                                                # 通过
cargo check --locked                                             # 通过
cargo clippy --all-targets --all-features --locked -- -D warnings # 通过
cargo test --locked --lib                                        # 3346 passed, 2 ignored
cargo test --locked --test it_core workspace_ws_integration      # 54 passed
cargo test --locked --lib workspace_session                      # 24 passed
cargo test --locked --lib workspace_ws_handler                   # 116 passed
cargo test --locked --test it_core workspace_ws_fanout_events_have_shared_monotonic_event_seq # 1 passed
```

完整 integration 首跑中，既有 `workspace_ws_idle_timeout_records_server_idle_connection_diagnostic` 因 `ResetWithoutClosingHandshake` 失败一次；立即单独复跑通过，随后完整 `workspace_ws_integration` 复跑为 54 passed。该用例及 idle-close 路径不在本 Task 的变更范围内。

## 提交

- `84bf81ee feat(workspace): session 级单调 event_seq 与有界事件 journal，广播总线落地`

## Concerns

- `after_event_seq` 的回放、snapshot 补偿及延迟 attach 初帧消费仍由 Task 10 实施；本 Task 仅建立 journal/attach 基线和必要注释，避免在 cursor 协议尚未落地时改变既有 attach 时序。
- `try_send` 满/失败当前只记录诊断；attachment 的慢订阅者 resync/隔离由 Task 11 接续实施。
- 工作树中原有两项无关文档改动仍未触碰：一个报告删除和一个 cadence note 修改；本 Task 的提交未包含它们。
