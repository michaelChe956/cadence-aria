# 3.8 P2 Phase C Task 8 完成报告

## 状态

已完成。Workspace WebSocket 的 driver lease 现在是连接持有型、无定时器的会话级单活性授权：绑定为 Driver（含缺席 `role` 的 legacy 客户端）即获得或接管 lease；显式 Observer 不获取也不接管。当前 holder 的连接关闭只撤销 lease，不取消运行，也不写入 durable 终态。

## 交付内容

- `LeaseState` 持有 `holder`、`epoch` 与诊断用 `last_holder`；仅当前 holder 的关闭会撤销 lease。
- attachment 记录其绑定 epoch。新 Driver attachment 接管 lease 后，旧 driver 的写命令在 socket 仲裁层得到 `ProtocolError { code: "STALE_DRIVER_LEASE" }`，不会竞争 engine 锁或改变运行。
- Observer 的 Hello 会回滚连接建立阶段为 legacy 兼容而暂持的 lease，不影响已在场 Driver 的写权限；读面初始 snapshot 保持可用。
- handler 发起的 supersede 在取得 engine 锁前，以连接与 epoch 在 manager 临界区完成授权和旧 run 取消；provider 启动前再次校验 epoch，避免接管竞争下旧连接登记新 run。内部 engine relay 保留既有启动语义。
- 迁移 `part_03` 的次连接回归：无 role 次连接接管后仍能 Abort，同时主连接的迟到 Abort 被拒绝；原本仅为 attach/read 面的第二连接显式声明 Observer。
- 新增集成用例覆盖：legacy driver 接管与迟到写拒绝、driver close 仅撤 lease 且 run 自然完成、Observer 连接不接管 lease。
- 报告脚本 Hello 发送点保持零改动；核对数为 5。

## TDD 证据

红灯：新增 `workspace_ws_lease_takeover_and_stale_write_rejection` 在实现前收到 `ProviderStatus::Aborted`，证明旧 holder 的 Abort 仍会实际取消 run；实现后返回 `STALE_DRIVER_LEASE`。driver close 与 observer 钉子用例在实现前已符合其分别的既有 close/observer 行为，作为回归边界保留。

## 验证

```text
cargo fmt --check                                      # 通过
cargo check --locked                                   # 通过
cargo clippy --all-targets --all-features --locked -- -D warnings  # 通过
cargo test --locked --lib                              # 3343 passed, 2 ignored
cargo test --locked --test it_core workspace_ws_integration       # 53 passed
cargo test --locked --lib workspace_session            # 21 passed
cargo test --locked --lib workspace_ws_handler         # 116 passed
```

额外定向验证通过：

```text
workspace_ws_lease_takeover_and_stale_write_rejection
workspace_ws_secondary_connection_takes_lease_and_can_abort_active_run_started_by_primary
workspace_ws_user_message_interrupts_active_stream_before_completion
workspace_ws_stale_choice_response_after_new_run_is_rejected_before_provider
```

## 提交

- `727fe95a feat(workspace): 会话级单活性 driver lease——接管/迟到写拒/关闭只撤销，supersede 收窄到 holder`
- `d2233722 fix(workspace): 收敛 lease 启动临界区`
- `34e3ad44 fix(workspace): 保持内部运行接替取消语义`

## Concerns

- 部署前已建立、未刷新或重连的旧 observer tab 在其当前连接生命周期内仍按 legacy Driver 归一；刷新/重连后发送显式 `role: "observer"` 并进入本 Task 的不接管读面。这是 Task 6 已记录的自限部署窗口。
- 本 Task 没有宣称断连主嫌已根治；关闭路径仅按 REQ-WCR-02/03 改为撤销 lease、保留 run 的真实业务终态。

## P1 修复（round 1/5）

- 审查发现的 abort/start 两阶段 orphaned run 窗口已关闭：`start_run_from_attachment` 现在在单一 manager 临界区内依次完成 attachment epoch 校验、`take` 被覆盖 run 与新 run 登记；释放锁后仅取消所取出的旧 run。
- socket 路径保留取得 engine 锁前的 `abort_active_run_from_attachment` 校验与中止，以及启动前的第二次 epoch 校验；内部 relay/恢复路径继续以 `None` 跳过 lease 校验，并保留既有 supersede hand-off 语义。
- 新增 `socket_supersede_resuming_after_relay_start_cancels_relay_run`：复现 R1 被 socket 取消、relay 登记 R2、socket 恢复登记 R3 的交错；断言 R2 token 必被取消且 R2 的迟到 `finish_run` 不会清除 R3。修复前该用例在等待 R2 取消时超时失败。

### P1 修复验证

```text
cargo fmt --check                                                     # 通过
cargo check --locked                                                  # 通过
cargo clippy --all-targets --all-features --locked -- -D warnings    # 通过
cargo test --locked --lib                                             # 3344 passed, 2 ignored
cargo test --locked --test it_core workspace_ws_integration           # 53 passed
cargo test --locked --lib workspace_session::tests                    # 9 passed
```
