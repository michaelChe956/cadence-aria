# Round 3 Group 初始化与 Abort 竞态实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. 本任务明确禁止委派子 Agent，必须在当前 session 内内联执行。

**Goal:** 使 Group Attempt 初始化可跨进程幂等恢复，并保证 Abort/Delete 返回后旧 runner 不再有 durable mutation。

**Architecture:** Group 使用 journal-first 身份快照和 phase replay，在 issue 级文件仲裁锁内幂等物化。Abort 通过 registry retired tombstone、reservation 撤销和 per-run completion 等待关闭竞态，再使用 Attempt mutation lease 执行终态操作。

**Tech Stack:** Rust 2024、Axum、Tokio、Serde JSON file store、GitWorkspaceService。

## Global Constraints

- 不兼容历史无 journal Group Attempt；缺 journal 必须 fail-closed。
- 不新增 HTTP test route；所有 failpoint/pause seam 默认关闭。
- 禁止跨 Store delete Attempt + release lease 回滚半初始化状态。
- 禁止 `-j 1`、E2E、Playwright、真实 Provider 和网络 CLI。
- 不 amend、不 push，最终只创建一个新原子提交。

---

### Task 1: Group 初始化重启回归测试

**Files:**
- Modify: `src/web/test_controls/mod.rs`
- Modify: `tests/it_web/web_coding_attempt_api/part_10.rs`

**Interfaces:**
- Produces: `GroupAttemptInitializationCheckpoint` 与一次性 TestControls 配置/消费方法。

- [ ] 增加 persist-before-bind、bind-before-binding、partial-units 三条新 router/new Store 重启测试。
- [ ] 用 `cargo test --locked --test it_web web_coding_attempt_api::group_initialization -- --nocapture` 运行 RED，确认分别因缺少 checkpoint/recovery 失败。
- [ ] 增加权威 plan/provider 身份漂移的零写 fail-closed RED。

### Task 2: Group journal-first replay

**Files:**
- Create: `src/product/coding_attempt_store/group_initialization.rs`
- Modify: `src/product/coding_attempt_store/mod.rs`
- Modify: `src/product/coding_attempt_store/paths.rs`
- Modify: `src/product/coding_attempt_store/attempt.rs`
- Modify: `src/web/handlers/coding/group.rs`

**Interfaces:**
- Produces: `CodingGroupInitializationJournal`、`CodingGroupInitializationPhase`、issue 级 arbitration guard、prepare/ensure/advance/get/delete API。

- [ ] 定义 journal，固定 Attempt、provider config、plan binding 和有序 Unit 记录，完成严格 identity validation。
- [ ] 先写 journal，再幂等 ensure Attempt/provider config；已存在记录必须精确相等。
- [ ] 将 handler 改为 prepare→lease→Attempt→bind→binding→units→integrity→completed，在三个确定性 checkpoint 消费 failpoint。
- [ ] 移除初始化失败的跨 Store rollback；显式 delete Group Attempt 时同步删除对应 journal。
- [ ] 运行 Task 1 focused tests，确认 GREEN；再运行现有 Group API/Store focused tests。

### Task 3: Ownerless 与 registry wait RED

**Files:**
- Modify: `tests/it_product/product_lifecycle_store/part_04.rs`
- Modify: `src/web/state/coding_run_registry.rs`

**Interfaces:**
- Produces: 严格 ownerless 回归测试和 `abort_attempt` 等待 runner remove 的行为测试。

- [ ] 先写 ownerless 记录不能通过 `validate_issue_worktree_lock_owner` 的 RED。
- [ ] 先写 abort 撤销 reservation、拒绝 late activation/insert、且在 runner `remove` 前不返回的 RED。
- [ ] 用 `cargo test --locked --lib coding_run_registry` 和定向 lifecycle integration test 运行 RED。

### Task 4: Retired runner wait 与终态串行

**Files:**
- Modify: `src/web/state/coding_run_registry.rs`
- Modify: `src/web/handlers/coding.rs`
- Modify: `src/web/coding_ws_handler/socket.rs`
- Modify: `src/product/coding_workspace_engine/handoffs.rs`
- Modify: `src/product/lifecycle_store/worktree.rs`

**Interfaces:**
- Produces: per-run completion handle、Attempt retired tombstone、wait-until-removed abort、HTTP Abort/Delete mutation lease 串行。

- [ ] 调整 run registry entry，`remove` 标记 completion 并唤醒 waiter；`abort_attempt` 不持锁跨 `.await`。
- [ ] 撤销 reservation，使 reservation activation、insert 和新 reserve 在 retired Attempt 上失败。
- [ ] HTTP Abort/Delete 在 wait 后获取 mutation lease并重载 Attempt；WS Abort 使用已获取 lease，并重载幂等终态。
- [ ] `handle_abort` 对已 Aborted Attempt 零写幂等返回；owner validator 仅允许精确 work item + owner。
- [ ] 更新所有使用 dummy sender 的 abort 测试，让模拟 runner 收到命令后调用 `remove`。
- [ ] 运行 Task 3 focused tests 和 Web abort focused tests，确认 GREEN。

### Task 5: Abort 与 durable transition 竞态回归

**Files:**
- Create: `src/product/coding_workspace_engine/mutation_test_pause.rs`
- Modify: `src/product/coding_workspace_engine/mod.rs`
- Modify: `src/product/coding_workspace_engine/group_completion.rs`
- Modify: `src/product/coding_workspace_engine/provider_failure.rs`
- Modify: `src/product/coding_workspace_engine/tests/group_completion_recovery.rs`
- Modify: `src/product/coding_workspace_engine/tests/provider_failure_recovery.rs`

**Interfaces:**
- Produces: 按 workspace root + transition 类型隔离的 test-only pause guard，以及入口 status/stage 再校验。

- [ ] 先写 abort-vs-Running completion RED：owner preflight 后 pause，abort 在 runner remove 前必须 pending，返回后快照稳定。
- [ ] 先写 abort-vs-CompletedRetry 和 abort-vs-provider-failure RED，快照覆盖 Git/Attempt/Unit/Run/handoff/timeline/event/lease。
- [ ] Group completion 入口要求 persisted `Running + ReviewRequest`；provider failure 重载 Attempt，要求 active status 且 stage 未漂移。
- [ ] 实现 test-only pause seam，在 owner/status preflight 后、第一个 durable/Git 写前触发。
- [ ] 运行三条 focused race tests 确认 GREEN。

### Task 6: 报告、全量验证与原子提交

**Files:**
- Modify: `.superpowers/sdd/feat-b-0715-final-fix-report.md`

- [ ] 运行所有 focused tests。
- [ ] 运行 `cargo fmt --check`、`cargo check --locked`、`cargo clippy --all-targets --all-features --locked -- -D warnings`、`cargo test --locked`。
- [ ] 运行 `cd web && pnpm tsc -b` 和 `git diff --check`；未改前端时在报告说明未重跑 Vitest/build。
- [ ] 审计 staged diff、确认所有 Rust/TS 文件不超过 800 行，确认无无关改动。
- [ ] 追加 Round 3 最终修复报告，创建一个新原子提交，确认 worktree clean，不 amend、不 push。
