# P5 Task 4 Plan Amendment 应用与恢复评审修复报告

## 交付摘要

- 评审修复基线：`3d88790c65422bc233a9041296a3e837c44f9b73`（`feat(coding-repair): apply and recover plan amendments`）。
- 分支：`feat-b-0715`。
- 提交策略：单个原子 fix commit，不 push。
- 范围：P5 Task 4 的 Amendment 权威身份、durable Journal、并发仲裁/CAS、UnitRun materialization、Resume Target、WS runner 恢复与 durable event delivery。
- 明确不含：P5 Task 5 Handoff Revision 解析与运行时 impact propagation；未运行 E2E、Playwright 或 browser。

## 最终实现

### 1. 权威 Amendment 身份与 durable Journal

- Recovery identity 只允许来自 active amendment lock，或唯一满足当前 revision 与 durable delivery/pending recovery 条件的 authoritative Journal。
- 删除 binding history fallback；历史 Completed Journal 不再能冒充当前 Amendment。
- Completed/Running Journal 只有在存在对应 durable `Delivered` marker 时才可作为当前 Journal；`AwaitHandoff` 的 Completed Journal 仍可继续幂等收敛。
- existing Journal 在 dirty-worktree gate 之前完成 identity 与 durable-prefix 校验；IdentityMismatch/Ambiguous 不进入 operational failure recorder，保持 zero-write。
- Journal ID 由 Attempt + Amendment 确定生成；Phase 只允许同 phase 幂等写或相邻 phase 推进。
- 每个 durable phase 都校验 Binding、UnitRuns、Resume Target，以及 Completed session/finalization prefix。

### 2. 并发仲裁、锁序与 CAS

- Public apply/recover 入口均先持有 per-attempt amendment application arbitration flock，串行化同一 Attempt 的 apply/recover。
- 统一锁序：
  1. Attempt amendment arbitration；
  2. Plan lineage lock；
  3. Binding / Unit / UnitRun / Journal 等下游实体锁。
- Binding write 在 Plan lineage lock 内重新校验 exact amendment ID 与 active revision，等待期间 lineage 被替换时 fail-closed。
- UnitRuns、ResumeTarget、Completed phase 均在 exact amendment + revision 的 Plan lineage lock 内执行。
- finalization 使用 expected-state CAS：
  - repair request compare-and-mark `Applied`；
  - plan compare-and-release exact applied amendment + active revision；
  - Plan Repair session snapshot compare-and-save；
  - Workspace session compare-and-update；
  - Attempt 在写锁内比较完整 expected Attempt 后恢复。

### 3. UnitRun 与 AwaitHandoff

- Amendment UnitRun 职责从 `unit_run.rs` 拆至 `unit_run_amendment.rs`，删除原文件中禁用/复制实现。
- deterministic UnitRun identity 完整覆盖 execution/revision/handoffs/contracts/projection/compiler/renderers、execution-context hashes、status/counters/start/completion commits。
- forged same-ID UnitRun 在 recovery 与 status update 两条路径均 fail-closed。
- replacement source 只允许 active run 转为 `Superseded`；历史 `Completed` run 保持不变；重复恢复不产生重复 UnitRun。
- `AwaitHandoff` 完成后 Attempt 保持 `AwaitingPlanAmendment`，stage 为 Coding，provider 继续 blocked。
- WS runner 覆盖已排队 Coding stage-gate continue；恢复完成前 provider start count 为 0。

### 4. Durable WS delivery

- 新增 amendment-ID keyed durable delivery marker，状态为 `Pending` / `Delivered`。
- event ID 确定生成：`coding_plan_amendment_updated_{attempt_id}_{amendment_id}`，Rust 与 TypeScript contract 均携带 `event_id`。
- durable 顺序固定为：
  1. request/session/active-lock finalization；
  2. 写入或读取 durable `Pending` marker；
  3. producer 注册 process-local socket-write waiter，并将 `PlanAmendmentUpdated` 入队；
  4. WebSocket writer 只有在 `Sink::send(...).await` 成功后确认 waiter；socket write 失败或断连 drain 会拒绝 waiter；
  5. producer 收到 writer 确认后 marker compare-and-mark `Delivered`；
  6. 恢复 Attempt 为 runnable，或保持 `AwaitHandoff` blocked。
- mpsc enqueue、socket write 或 Delivered 标记任一步失败时 Attempt 都不会变为 runnable，marker 保持 `Pending`。
- socket send 成功但 durable mark 前崩溃时，恢复会在重连后用相同 `event_id` 重发；因此语义是 **socket-write-confirmed at-least-once + stable event ID**，允许重复，不是客户端 exactly-once。
- socket loop 退出前先关闭 event receiver，再拒绝所有已排队 Amendment waiter，避免 drain 与新 enqueue 的竞态悬挂。
- 同一 Attempt 的并发 recovery 由 arbitration 串行化，durable marker 最终收敛到一次 `Delivered` 状态。

## Reviewer 3 Critical / 5 Important：RED → GREEN

| 级别 | 问题 | RED 证据 | GREEN 结果 |
| --- | --- | --- | --- |
| Critical 1 | Recovery 使用非权威历史/Binding 身份，且 dirty gate 可能先于 identity mismatch 写状态 | 历史 Completed Journal 被选中；existing Journal identity mismatch 在 dirty worktree 下先命中 gate | 删除 binding history fallback；只接受 active lock 或唯一 authoritative Journal；所有 durable phase 的 clean/dirty identity mismatch 均 zero-write |
| Critical 2 | apply/recover 与 lineage replacement 并发时缺少统一仲裁和锁内重读 | 等待 lineage lock 后，已被替换的 binding 仍可写入；第二个 recovery 可能收到 identity mismatch | per-attempt arbitration + `attempt arbitration -> plan lineage -> entity` 锁序；Binding/UnitRuns/ResumeTarget/Completed 均锁内复核 exact amendment/revision；并发恢复收敛 |
| Critical 3 | WS event send/mark 失败可能丢事件或让 Attempt 提前 runnable | send failure 被忽略并恢复 Running | durable Pending marker 先于 send；send/mark 任一失败均保持非 runnable；mark 前崩溃用同一 event ID 重发；并发恢复只收敛一个 durable delivery |
| Important 1 | Journal identity/phase/prefix 约束不足 | 非相邻 phase 可直接推进；forged deterministic ID 或 corrupt prefix 可被接受 | deterministic Journal ID；仅同 phase/相邻 phase；Binding、UnitRuns、ResumeTarget、Completed session prefix 均严格验证 |
| Important 2 | deterministic UnitRun 只看 ID，forged same-ID 内容可污染恢复 | forged UnitRun 被恢复到 Running，或 status update 接受伪造内容 | 对完整 immutable execution context、contracts、hashes、counters 与 commits 做 identity 比较，两条写路径均 fail-closed |
| Important 3 | replacement source 的终态规则不正确 | active replacement source 保持 Running，或可能改写历史 Completed run | 只把 active source 转为 Superseded；Completed 不变；重复恢复幂等 |
| Important 4 | `AwaitHandoff` 完成后错误恢复为 Running | Attempt 进入 Running，provider 可继续 | Attempt 保持 `AwaitingPlanAmendment`；runner stage-gate continue 集成测试确认 provider starts = 0 |
| Important 5 | Completed finalization 缺少完整 expected-state/prefix CAS | corrupt Completed session 时 repair request 可先变 Applied；corrupt ResumeTarget 时 Journal 仍推进 Completed | finalization 前验证 durable prefix；request/session/workspace/Attempt/active lock 全部使用 exact expected-state compare-and-update，冲突 fail-closed |

## Re-review 2 Critical / 1 Important：RED → GREEN

| 级别 | 问题 | RED 证据 | GREEN 结果 |
| --- | --- | --- | --- |
| Critical 1 | Completed replay 把合法 runtime progress 误判为 forged，并可能把 Testing/CodeReview Attempt 回退 | execution-context binding、UnitRun Completed、Attempt Testing 三个 replay 测试均失败 | immutable materialization identity 与 mutable runtime evolution 分离；Completed replay 接受状态/counters/context hashes/completion commit 的合法单调演进；只恢复仍处于 amendment-blocked 状态的 Attempt |
| Critical 2 | 同步 `flock(LOCK_EX)` 阻塞 Tokio worker | current-thread ticker 延迟 329ms；6 waiters > 2 workers 发生 worker starvation，手工终止 exit 130 | `spawn_blocking` 获取 cross-process flock，guard 继续跨 await 持有；测试改为 runtime 释放信号 + std watchdog 的确定性调度证明，不依赖机器时间阈值；锁序仍为 attempt arbitration → plan lineage → entity |
| Important 1 | mpsc enqueue 后即标记 Delivered，socket 尚未实际写出 | enqueue 回归得到 marker `Delivered`，期望 `Pending`；临时移除 writer settle 后 success/failure 测试 0/2，均因 waiter `Elapsed` 失败 | producer 等待 writer success/failure ACK；writer success/failure 2/2，enqueue Pending、socket failure、send-before-mark crash + reconnect、并发 recovery 全部收敛 |

## Final Re-review 1 Critical / 1 Important：RED → GREEN

| 级别 | 问题与根因 | RED 证据 | GREEN 结果 |
| --- | --- | --- | --- |
| Critical 1 | Completed replay 继续使用当前 Attempt HEAD 重建 UnitRun `start_commit`；真实 group completion 会推进 HEAD，provider 绑定也会合法更新 renderer 与 execution-context hash，因此稳定物化身份与运行时演进仍被混为一体 | `cargo test --locked --lib coding_amendment_completed_replay_accepts`：新增 2 个真实路径回归均在 `coding_amendment_unit_run` 报 `IdentityMismatch` | Journal 创建时重读权威 Attempt，并持久化 `materialization_head_commit`；初始物化和 completed-prefix validation 显式使用 Journal HEAD；稳定 lineage/revision/handoff/contract/projection/compiler/hash/start commit 保持严格，renderer/context/status/counters/completion commit 仅允许受控演进；2/2 回归转绿 |
| Important 1 | `send_coding_event` 只在 `socket.send(...).await` 返回后 settle；writer future 在 Pending send 中被 abort 时跳过后置 fail。自然退出才执行的 receiver drain 也覆盖不了 handler cancellation，导致当前或队列 ACK sender 悬挂，producer 持有 arbitration 永久等待 | writer abort 回归超时为 `Elapsed(())`；queued receiver-drop 回归在 `OutboundEventReceiver` 尚不存在时编译 RED | 新增 `outbound.rs`：`OutboundWriteSettlement` 在 Drop 自动 fail 当前 dequeued event；`OutboundEventReceiver` 在 Drop close + drain queued events；success/failure/abort 与 receiver-drop 均释放 waiter，允许相同 event ID 重新注册并恢复，低层 4/4、业务集成 2/2 转绿 |

### Final Re-review RED/GREEN 命令与输出

- RED：`cargo test --locked --lib coding_amendment_completed_replay_accepts`，2 个新增测试均失败，错误为 `IdentityMismatch { kind: "coding_amendment_unit_run", ... }`。
- RED：`cargo test --locked --lib coding_ws_plan_repair_socket_writer_abort_rejects_dequeued_delivery_acknowledgement`，waiter 未被 settle，测试以 `Elapsed(())` 失败。
- RED：queued receiver-drop 回归首次编译失败，原因是 cancellation-safe `OutboundEventReceiver` 尚未实现。
- GREEN：`cargo test --locked --lib coding_amendment_completed_replay`，5 passed，0 failed。
- GREEN：`cargo test --locked --lib coding_amendment_journal`，3 passed，0 failed。
- GREEN：`cargo test --locked --lib coding_amendment_`，42 passed，0 failed。
- GREEN：`cargo test --locked --lib coding_ws_plan_repair_`，6 passed，0 failed。
- GREEN：writer success/failure/abort 3/3，receiver drop 1/1；writer-abort 与 receiver-drop 业务恢复集成 2/2。

## Final Re-review Follow-up 1 Important：RED → GREEN

| 级别 | 问题与根因 | RED 证据 | GREEN 结果 |
| --- | --- | --- | --- |
| Important 1 | `OutboundEventReceiver::drop` 的 `close() + try_recv()` 只能 settle 已进入可见队列的 event；close 前取得的 Tokio mpsc `Permit/OwnedPermit` 可在 receiver drop 后继续 `send`，该 event 没有 writer settlement。producer 在 enqueue 后只等待 socket ACK，会永久持有 amendment arbitration | 新回归用真实 `reserve_owned` 证明 receiver drop 后 permit 仍可 send，纯 `socket_write.wait()` 在 50ms 内保持 Pending；随后期望 channel-aware wait 时，精确测试编译 RED：`E0599 no method named wait_or_channel_closed` | `PlanAmendmentSocketWriteWaiter` 新增 biased `wait_or_channel_closed`，producer enqueue 后同时等待 socket ACK 与 `event_tx.closed()`；closed 优先，ACK/closed 同时 ready 时保守失败并重发，不会错误标记 Delivered。失败会 Drop waiter、释放 registration/arbitration；相同 event ID 可立即重新注册并恢复 |

### Final Re-review Follow-up RED/GREEN 命令与输出

- RED：`cargo test --locked --lib coding_ws_plan_repair_outstanding_permit_receiver_drop_rejects_channel_aware_delivery_wait`，exit 101，`E0599`：`PlanAmendmentSocketWriteWaiter` 不存在 `wait_or_channel_closed`。
- GREEN：同一精确测试 1 passed，0 failed；测试同时确认 ACK-only wait 会超时、channel-aware wait 返回 `plan_amendment_delivery_channel_closed:<event_id>`、registration 清理后相同 event ID 可重新注册。
- GREEN：`cargo test --locked --lib coding_amendment_delivery_outstanding_permit_receiver_drop_releases_waiter_and_recovers_same_event`，1 passed，0 failed；Attempt 保持 non-runnable、marker 保持 Pending、arbitration 释放并用相同 event ID 恢复。
- GREEN：`cargo test --locked --lib coding_amendment_`，43 passed，0 failed。
- GREEN：`cargo test --locked --lib coding_amendment_delivery_`，7 passed，0 failed。
- GREEN：`cargo test --locked --lib coding_ws_plan_repair_`，7 passed，0 failed。
- GREEN：`cargo fmt --check`、`cargo check --locked`、strict clippy、large-file guard、测试命名审计、`git diff --check` 全部通过。
- 本轮按指令未运行 full `cargo test --locked`；由主线程在最终交付前 fresh 执行。

## 主要 RED/GREEN 测试

- `coding_amendment_existing_journal_identity_mismatch_is_zero_write_at_every_phase`：Started、PlanBindingWritten、UnitRunsWritten、ResumeTargetWritten × clean/dirty，共 8 个矩阵场景。
- `coding_amendment_recovery_does_not_select_historical_completed_journal`。
- `coding_amendment_journal_rejects_non_adjacent_phase_advance`。
- `coding_amendment_journal_rejects_forged_deterministic_id`。
- `coding_amendment_recovery_rejects_corrupt_skipped_binding_prefix_without_writes`。
- `coding_amendment_recovery_rejects_corrupt_resume_target_prefix_without_writes`。
- `coding_amendment_completed_prefix_rejects_corrupt_session_identity_without_writes`。
- `coding_amendment_binding_write_rejects_replaced_lineage_lock`。
- `coding_amendment_arbitration_rechecks_lineage_before_first_write`。
- `coding_amendment_recovery_rejects_forged_deterministic_unit_run_without_writes`。
- `coding_amendment_status_update_rejects_forged_deterministic_unit_run`。
- `coding_amendment_supersedes_only_active_replacement_source_runs`。
- `coding_amendment_await_handoff_keeps_attempt_provider_blocked`。
- `coding_ws_plan_repair_await_handoff_stays_blocked_after_stage_gate_continue`。
- `coding_amendment_completed_replay_allows_bound_execution_context`。
- `coding_amendment_completed_replay_allows_completed_unit_run`。
- `coding_amendment_completed_replay_preserves_later_attempt_stage`。
- `coding_amendment_arbitration_contention_does_not_block_current_thread_runtime`。
- `coding_amendment_arbitration_waiters_exceeding_workers_still_make_progress`。
- `coding_amendment_delivery_enqueue_keeps_marker_pending_until_socket_write`。
- `coding_amendment_delivery_channel_closed_keeps_attempt_non_runnable_with_pending_marker`。
- `coding_amendment_delivery_socket_write_failure_keeps_attempt_non_runnable_with_pending_marker`。
- `coding_amendment_delivery_retries_same_event_after_send_before_mark_failure`。
- `coding_amendment_concurrent_recovery_reconciles_one_durable_delivery`。
- `coding_ws_plan_repair_socket_writer_success_acknowledges_delivery`。
- `coding_ws_plan_repair_socket_writer_failure_rejects_delivery_acknowledgement`。
- `coding_amendment_completed_replay_accepts_group_completion_head_evolution`。
- `coding_amendment_completed_replay_accepts_provider_renderer_context_evolution`。
- `coding_ws_plan_repair_socket_writer_abort_rejects_dequeued_delivery_acknowledgement`。
- `coding_ws_plan_repair_outbound_receiver_drop_rejects_queued_delivery_acknowledgement`。
- `coding_amendment_delivery_writer_abort_releases_arbitration_and_recovers_same_event`。
- `coding_amendment_delivery_receiver_drop_releases_arbitration_and_recovers_same_event`。

## Full gate 首个失败与最小修复

- RED：首次 strict clippy 在 `amendment_recovery.rs` 报 `clippy::nonminimal_bool`。
- 根因：新增 `AwaitHandoff` 合法状态后使用 `!A && !B`，语义正确但不满足 strict lint。
- GREEN：改为等价 De Morgan 表达式 `!(A || B)`，不添加 lint allow；AwaitHandoff 定向测试 1/1 与 strict clippy fresh 通过。

## 验证结果

### Fresh focused

- `cargo test --locked --lib coding_amendment_completed_replay`：5 passed，0 failed。
- `cargo test --locked --lib coding_amendment_journal`：3 passed，0 failed。
- `cargo test --locked --lib coding_amendment_arbitration_`：3 passed，0 failed。
- `cargo test --locked --lib coding_amendment_delivery_`：7 passed，0 failed；另 `coding_amendment_concurrent_recovery_reconciles_one_durable_delivery`：1 passed，0 failed。
- `cargo test --locked --lib coding_ws_plan_repair_socket_writer_`：3 passed，0 failed；receiver drop 与 outstanding permit 各 1 passed，0 failed。
- `cargo test --locked --lib coding_amendment_`：43 passed，0 failed。
- `cargo test --locked --lib coding_ws_plan_repair_`：7 passed，0 failed。
- `cargo test --locked --lib coding_amendment_updated_roundtrips`：1 passed，0 failed。
- `cd web && pnpm tsc -b`：PASS，exit 0。

### 最终非 E2E full gate

- `cargo fmt --check`：PASS，exit 0。
- `cargo check --locked`：PASS，exit 0。
- `cargo clippy --all-targets --all-features --locked -- -D warnings`：PASS，exit 0，0 warnings。
- `cargo test --locked`：PASS，exit 0：
  - lib：1136 passed；
  - main：0 tests；
  - it_core：143 passed；
  - it_interactive：43 passed；
  - it_product：211 passed；
  - it_provider：55 passed；
  - it_task_run：31 passed；
  - it_web：258 passed，12 ignored；
  - doc-test：1 passed；
  - 合计：1878 passed，12 ignored，0 failed。
- `cd web && pnpm tsc -b`：PASS，exit 0。
- `cargo test --locked --test it_core large_file_guard::product_source_and_test_files_stay_under_line_limit`：1 passed，0 failed。
- 测试命名审计：PASS；新增行为测试使用 `coding_amendment_` / `coding_ws_plan_repair_` 前缀，无 `test`、`tmp`、`todo` 等占位命名。
- `git diff --check`：PASS，exit 0。
- 未运行 E2E、Playwright 或 browser。

## 文件行数

| 文件 | 行数 |
| --- | ---: |
| `src/product/coding_workspace_engine/tests/plan_amendment.rs` | 795 |
| `src/product/coding_attempt_store/tests/plan_repair.rs` | 783 |
| `src/web/coding_ws_handler/socket.rs` | 758 |
| `src/product/coding_workspace_engine/amendment.rs` | 732 |
| `src/product/coding_attempt_store/unit_run_amendment.rs` | 667 |
| `src/product/coding_attempt_store/unit_run.rs` | 601 |
| `src/product/coding_workspace_engine/tests/plan_amendment/review_fix_delivery.rs` | 511 |
| `src/product/coding_attempt_store/plan_binding.rs` | 386 |
| `src/product/coding_workspace_engine/tests/plan_amendment/review_fix_replay.rs` | 308 |
| `src/product/coding_workspace_engine/tests/plan_amendment/review_fix_unit_runs.rs` | 271 |
| `src/web/coding_ws_handler/tests/plan_repair/delivery_ack.rs` | 233 |
| `src/product/coding_workspace_engine/tests/plan_amendment/support.rs` | 130 |
| `src/product/coding_models/plan_repair.rs` | 122 |
| `src/web/coding_ws_handler/outbound.rs` | 99 |
| `src/web/coding_ws_handler/mod.rs` | 29 |

本轮 changed Rust/TS/TSX 文件最大为 795 行；`socket.rs` 为 758 行，新增 `outbound.rs` 为 99 行。large-file guard fresh 通过，全部不超过 800 行。

## 自审结论与边界

- authoritative identity、durable Journal、arbitration/CAS 与锁序满足 fail-closed、zero-write 和崩溃恢复要求。
- forged same-ID Journal/UnitRun、lineage replacement、并发 recovery、dirty identity mismatch 均有回归覆盖。
- provider entry 在 durable delivery 成功且 finalization 收敛前保持阻断。
- delivery 保证 socket-write-confirmed at-least-once 与稳定 event ID；socket write 后 durable mark 前崩溃可能重复，不宣称客户端 ACK、消费成功或 exactly-once。
- Completed replay 的稳定身份继续严格校验 materialization-time HEAD；renderer 改变必须伴随非空 execution-context hash，无 context hash 时 renderer 必须保持初始值，internal reviewer renderer/hash 必须成对出现。
- socket writer future abort 与 outbound receiver drop 都会 settle 当前或排队 Amendment ACK；marker 保持 Pending、Attempt 保持非 runnable，并可使用相同 event ID 立即恢复。
- P6 客户端 ACK/dedup 与 Repair Session UI 未在 Task 4 实现。
- 未添加历史 v1/缺失 Plan lineage 兼容 fallback。
- worktree 保留；单个原子 fix commit；不 push。
