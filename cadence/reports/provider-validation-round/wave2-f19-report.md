# wave2 F-19 修复报告：coding runner 事件流单连接绑定无广播 → per-attempt hub fan-out

- 工作树：`.worktrees/feat-b-0808-add-monorepo`
- 输入证据：W2b 复核发现（wave2-2b，报告文件未落盘，发现内容经任务指派传递）——
  coding runner 事件流绑定驱动 socket 私有 channel：唯一消费者（驱动 WS）断开即死、
  外部恢复后 campaign 驱动失明。对照面：workspace 面 P2 已有 broadcast+journal 先例
  （`workspace_session/manager.rs` + `router.rs`），coding 面缺同款。
- 影响面：kimi 升级驱动失明 + F-16 恢复后驱动无法继续观察。

## 1. 现状定位（缺口的三条链）

1. **绑定点**：`coding_ws_handler/socket.rs` 的 `handle_coding_socket` 为每个连接建
   `mpsc::channel(1024)`，`spawn_coding_runner` / `CodingWorkspaceEngine::new` /
   `prepare_coding_message` 全部拿**本连接私有 event_tx** —— runner 一旦启动，事件只进
   驱动连接的 channel。
2. **断开即死**：驱动 WS 断开 → 该 channel receiver drop → runner/engine 的
   `event_tx.send` 全部失败（`let _ =` 吞掉）→ 事件蒸发；runner 本体继续跑但完全失明。
3. **恢复后失明**：F-16 `RecoverCoding`/重连后 `ensure_runner_for_resumed_attempt` 对
   活 runner 判定 `NotNeeded`，新连接只拿到快照，runner 事件继续发往已死 channel——
   重连驱动永远收不到实时帧。
   （registry 已有多 socket 槽位 `BTreeMap<token, sender>`，但 `sender()` 只取最后一个
   且仅 plan_repair 激活使用——广播面从未接上。）

## 2. 修法选择：per-attempt 事件 hub（多订阅者 fan-out）+ 因果序 barrier

对照 workspace 面 P2（manager 持 engine + session-owned router 任务 fan-out 全部
attachment，配 event_seq journal + cursor 重放），coding 面按规模最小化：

- **不引入 tokio::broadcast**：发射 API（`mpsc::Sender<CodingWsOutMessage>`）贯穿
  engine/runner 全部签名，换通道类型 ripple 过大；且 Lagged 语义引入新丢失面。
- **不引入 journal+cursor 重放**：coding 面已有 F-13 journal 回放
  （`coding_execution_event_replay` 随 session-state 快照下发），重连补窗已由快照覆盖；
  缺的只是**实时多消费者 + 断开后事件流存活**。
- **选定**：`CodingSocketRegistry` 内建 per-attempt hub——
  `hub_sender(key)` 返回 hub 的 mpsc sender（发射 API 零变化）；hub 路由任务顺序
  fan-out 到 registry 现存全部存活 socket；目标按现存 socket **动态解析**（与 hub
  实例无关，旧 hub 的路由也能送到新 socket）。

### 因果序 barrier（零回归的关键）

hub 多了一跳异步转发，socket 循环「engine 操作 → flush → 直写快照」的旧顺序被打破
（直写快照会抢在 engine 事件前上线，e2e `coding_ws_final_confirm_completes_attempt_`
`and_sends_snapshot` / `coding_ws_abort_attempt_closes_active_node_and_sends_snapshot`
实测红）。加 `wait_until_hub_drained`：控制 channel + biased select——barrier ack 只在
hub 瞬时空转时放行，即 barrier 之前入队的事件必已 fan-out 完毕。socket 循环在 4 处
flush 前（FinalConfirm / AbortAttempt×2 / RetryPush / GateResponse）先 barrier 再
flush，wire 顺序与旧直连路径逐帧一致。

## 3. 改动点

1. `src/web/state/coding_socket_registry.rs` —— hub 存储 + `hub_sender` /
   `hub_sender_if_live`（保留 plan_amendment 激活「无存活 socket 即 fail-closed」门）/
   `wait_until_hub_drained`（barrier）/ `broadcast`（fan-out；零投递时 fail
   plan_amendment socket-write ack，等价旧 channel 关闭失败语义）。删除死代码
   `sender()`（唯一调用方已迁移）。生命周期：registry 持有的 hub 引用随最后一个
   socket 断开而摘除；runner/engine 持有的 clone 使 hub 跨驱动断开存活；全部
   producer 退出后 channel 关闭、路由任务自然结束，无泄漏。
2. `src/web/coding_ws_handler/socket.rs` —— 私有通道改名 `socket_event_tx`（仅
   register 用），发射面 `event_tx = coding_sockets.hub_sender(&attempt_key)`；
   runner spawn / engine 构造 / preparation 全部沿用 `event_tx` 名（其余 10 处
   发射点零改动）；4 处 flush 前插 barrier。
3. `src/web/workspace_ws_handler/plan_repair_activation.rs` —— `sender()` →
   `hub_sender_if_live()`（无 socket 仍报 `plan_amendment_coding_socket_unavailable`）。

## 4. TDD 证据

红（修复前实测，`tests/it_web/web_coding_ws_handler/part_17.rs` 新增两条）：
- `coding_ws_runner_events_broadcast_to_second_connection`：第二连接在初始快照后
  死寂 → `ws message timeout` 失败。
- `coding_ws_runner_event_stream_survives_driver_disconnect`：驱动断开后存活连接
  收不到任何 runner 帧 → `ws message timeout` 失败。
（中途 barrier 缺失版本还暴露 2 条顺序敏感存量测试红——见 §2，barrier 落地后复绿。）

绿（修复后）：
- 上述 2 条 F-19 e2e ✔（含断开后存活连接收到 fail-closed `coding_start_failed` 终态）
- lib 契约 4 条（`src/web/coding_ws_handler/tests/event_hub.rs`）：多 socket 广播、
  驱动断开存活+重连续收、`hub_sender_if_live` fail-closed 门、producer/socket 全退后
  新 hub 可用（无陈旧路由）✔
- 定向全绿：`--test it_web web_coding_ws_handler` 66/66（含 2 条顺序敏感复绿）；
  `--lib coding_ws_handler` 116/116
- fmt：改动文件 rustfmt clean；clippy（--lib）：改动面零警告

## 5. 已知边界（记录，不扩scope）

- **多 socket 慢消费者队头阻塞**：broadcast 顺序 `send().await`，一个填满 1024 缓冲
  且不读的连接会延迟其它连接的投递（直至其断开）。单连接行为与旧直连完全一致
  （背压上传 runner）。workspace 面对应解法是 degraded 标记 + 快照 resync，coding 面
  如需同款另行立项。
- **重连补窗**：断开期间的无消费者事件会被丢弃（与旧「断开即死」相比已是纯改善——
  runner 不再死、重连后实时流恢复）；历史窗由 F-13 快照回放覆盖，事件级 cursor
  重放未引入（见 §2 取舍）。
- 并发验证环境注记：本修复验证期间共享工作树内 P0Watchdog 的 lifecycle_store/
  workspace_engine 在途改动多次阻断 lib 构建，所有定向测试均在编译窗口内完成，
  结果如上；全仓全量验证归 controller 终态复核。

## 6. k3 review round 1 修复（P1 无 entry 漏 fail-ack + P2 fan-out 单份 ack 误判）

- **P1（amendment waiter 无限阻塞）**：`broadcast` 在 registry 无该 attempt 的
  sockets entry 时 `let-else` 早返回，跳过 `fail_plan_amendment_socket_write`——
  驱动断开后 runner 持 hub clone 保活、hub channel 不关闭，`wait_or_channel_closed`
  两臂均不触发=无限阻塞（旧直连实现会因 channel 关闭快速失败）。修：早返回改
  `match`（None 分支返回空目标 Vec），零份额统一由登记侧结算失败。
- **P2（一败一胜误判失败）**：`PlanAmendmentUpdated` 广播到多个 socket 但 ack 单例、
  首 settle 胜出——任一 socket 写失败立即 fail 结算，其它 socket 的成功写救不回，
  amendment 被误判失败中断。修：`delivery_ack` entry 增 `pending_writes` 份额计数，
  broadcast 在**发送循环之前**登记目标数（socket 循环的写结算只会在事件进入
  channel 后发生，登记先行即无竞态）；结算规则=任一 confirm 立即成功、份额递减
  到零（全部失败）才失败、send 失败份额按 fail 计；未登记（非广播路径）保持
  「首 settle 即结算」旧单写语义。
- TDD：新增 2 例红→绿——`hub_without_live_sockets_fails_plan_amendment_delivery_
  quickly`（修复前 waiter 2s 超时挂起=红）、`plan_amendment_fan_out_one_failure_
  one_success_confirms`（修复前 fail 先结算 → wait Err=红）。
- 复跑全绿：`event_hub` 6/6；`--lib coding_ws_handler` 118/118；
  `--lib plan_amendment` 49/49；`--test it_web web_coding_ws_handler` 66/66；
  fmt/clippy（改动面）clean。
