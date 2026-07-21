# Round 4 创建仲裁与 Runner 清理实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. 本任务禁止委派子 Agent，步骤使用 checkbox 跟踪。

**Goal:** 关闭 Single/Group 同 Work Item 创建、runner cleanup/backpressure、Group replay/Delete 三类竞态，并保持严格新 Schema。

**Architecture:** 创建路径共享带 identity 的 Work Item 文件 guard，Group journal 固定 required lease ID。runner 通过 RAII remove，WS Abort 同时 drain event queue。Group Delete 与 replay 共享 initialization arbitration。

**Tech Stack:** Rust 2024、Axum、Tokio mpsc/watch/oneshot、Serde JSON file store、GitWorkspaceService。

## Global Constraints

- `worktree_lease_id` 为 required 字段，无 serde default，不兼容旧 Group initialization journal。
- 不新增 HTTP test route；TestControls seam 默认关闭、一次性消费。
- 禁止 `-j 1`、E2E、Playwright、browser、真实 Provider 与网络 CLI。
- 所有修改/新增 Rust、TS 文件不得超过 800 行。
- 最终使用一个新原子提交，不 amend、不 push，worktree clean。

---

### Task 1: Single/Group 双向交错 RED

**Files:**
- Modify: `src/web/test_controls/mod.rs`
- Create: `tests/it_web/web_coding_attempt_api/part_11.rs`
- Modify: `tests/it_web/web_coding_attempt_api.rs`

**Interfaces:**
- Produces: `pause_next_group_attempt_after_worktree_acquire` 与双 router 跨 Store 确定性测试。

- [x] 新增 Group lease 后 pause seam，使用 `Notify` 协调 entered/released，不使用 sleep 竞争。
- [x] 写 `group_then_single_creation_is_serialized_by_work_item_guard`，先暂停 Group，再启动 Single，断言修复前两个路径可交错或产生第二 active/identity conflict。
- [x] 写 `single_then_group_creation_is_serialized_by_work_item_guard`，先暂停 Single，再启动 Group，断言修复前 Group 可写 journal 或进入错误 owner 路径。
- [x] 运行 `cargo test --locked --test it_web creation_is_serialized_by_work_item_guard -- --nocapture`，记录 RED。

### Task 2: 共享 creation guard 与 required lease identity GREEN

**Files:**
- Create: `src/product/coding_attempt_store/attempt_creation.rs`
- Modify: `src/product/coding_attempt_store/mod.rs`
- Modify: `src/product/coding_attempt_store/paths.rs`
- Modify: `src/product/coding_attempt_store/attempt.rs`
- Modify: `src/product/coding_attempt_store/group_initialization.rs`
- Modify: `src/web/handlers/coding.rs`
- Modify: `src/web/handlers/coding/group.rs`

**Interfaces:**
- Produces: `WorkItemAttemptCreationGuard`、`acquire_work_item_attempt_creation`、guard-aware Single create/Group ensure。

- [x] 将现有 `work-item-attempt-locks/{work_item}` 路径封装为带 project/issue/work-item identity 的 RAII guard。
- [x] `create_attempt` 作为自锁包装；handler 取得 guard 后调用 guard-aware create，锁内再次检查 active 并写 Attempt/provider。
- [x] Group journal 新增 required `worktree_lease_id`，prepare 固定 UUID lease identity。
- [x] Group handler 在 initialization arbitration 后获取 creation guard，并持有到 bind/binding/units 完成；使用 journal lease ID。
- [x] Group ensure 在 guard 内只接受 journal Attempt 精确存在且无其他 active，或确认无 active 后首次写。
- [x] `acquired=false` 仅接受已绑定 journal Attempt；其他 owner 返回 typed fail-closed error。
- [x] 运行 Task 1 两条测试与现有 single/group restart focused tests，确认 GREEN。

### Task 3: Runner cleanup/backpressure RED

**Files:**
- Modify: `src/web/state/coding_run_registry.rs`
- Create: `src/web/coding_ws_handler/tests/runner_cleanup.rs`
- Modify: `src/web/coding_ws_handler/tests.rs`
- Create: `src/web/coding_ws_handler/socket/abort.rs`

**Interfaces:**
- Produces: panic、closed receiver、满 outbound queue Abort 三条有界回归。

- [x] 写真实 `spawn_coding_runner_task` 注册后 panic 测试，timeout 内要求 runner count=0；修复前 completion 缺失。
- [x] 写 command receiver 已关闭的 registry Abort 测试，timeout 内要求返回且 registry count=0；修复前永久等待。
- [x] 写容量 1 event queue 测试：第二个 event send 阻塞，Abort helper 必须 drain、传递 Abort、等待 remove并返回有序事件；修复前死锁。
- [x] 分别运行 `cargo test --locked --lib runner_cleanup -- --nocapture` 与 `cargo test --locked --lib abort_completes_when_command_receiver_is_closed -- --nocapture`，记录 RED。

### Task 4: RAII registration 与 WS drain GREEN

**Files:**
- Create: `src/web/coding_ws_handler/runner/registration.rs`
- Modify: `src/web/coding_ws_handler/runner.rs`
- Modify: `src/web/state/coding_run_registry.rs`
- Create: `src/web/coding_ws_handler/socket/abort.rs`
- Modify: `src/web/coding_ws_handler/socket.rs`

**Interfaces:**
- Produces: `CodingRunnerRegistrationGuard` 与 `abort_attempt_while_draining_events`。

- [x] runner spawned future 首行创建 registration guard，删除 task 内正常/早退显式 remove，保留 spawn 激活失败的外部 remove。
- [x] registry Abort snapshot 携带 run ID；command send 失败时立即 remove，再等待 completion receivers。
- [x] Abort drain helper使用 `tokio::select!` 在等待 future 与 event recv 间推进，返回按序缓存事件。
- [x] Socket Abort 分支使用 helper，随后通过 `send_coding_event` 结算所有缓存 ACK；写失败时标记剩余 delivery failure并退出。
- [x] 运行 Task 3 三条测试、registry 模块与 Coding WS focused tests，确认 GREEN。

### Task 5: Retry/Delete arbitration RED→GREEN

**Files:**
- Modify: `src/web/test_controls/mod.rs`
- Modify: `src/web/handlers/coding.rs`
- Modify: `src/web/handlers/coding/group.rs`
- Modify: `tests/it_web/web_coding_attempt_api/part_11.rs`

**Interfaces:**
- Produces: Group replay pause 与 Group Delete arbitration。

- [x] 写 `group_retry_and_delete_are_serialized_by_initialization_arbitration`：制造 `AttemptPersisted`，重启 replay 持 arbitration 暂停，并发 Delete 必须保持 pending。
- [x] 运行定向测试记录 RED，确认 Delete 可越过 replay。
- [x] Group Delete 在 Attempt mutation lease 后、任何 cleanup 前获取 initialization arbitration，并持有到 `delete_attempt` 完成。
- [x] replay pause 恢复后先完成初始化，Delete 再完成；断言 Attempt/provider/binding/units/journal/lease 全部清除。
- [x] 运行 retry/delete 与所有 Group initialization focused tests，确认 GREEN。

### Task 6: 报告、门禁与提交

**Files:**
- Modify: `.superpowers/sdd/feat-b-0715-final-fix-report.md`

- [x] 确认 `src/web/app.rs` 无新增默认 route，记录 test-controls Minor 保留理由。
- [x] 报告逐项记录 finding 核验、锁序无环证明、RED/GREEN 与 focused 结果。
- [x] 运行 `cargo fmt --check`、`cargo check --locked`、`cargo clippy --all-targets --all-features --locked -- -D warnings`、`cargo test --locked`。
- [x] 运行 `cd web && pnpm tsc -b`；本轮无前端改动时说明未运行 Vitest/build。
- [x] 运行 `git diff --check`，审计 unstaged/staged diff与所有 Rust/TS 文件行数。
- [x] 创建一个新原子提交，不 amend、不 push；提交后 `git status --short` 必须为空。
