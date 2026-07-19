# P5 Task 3 实施报告：创建 PlanRepairRequest 并暂停 Coding

## 状态与范围

- **COMPLETE**
- 基线：`f1190f16c7ee1320217a6b8196d05447c2f60f93`
- 提交消息：`feat(coding-repair): pause coding for plan amendments`
- 范围：把 P5 Task 2 已判定的 Plan Defect 转换为 canonical `PlanRepairRequest`，持久化/复用 Work Item Plan Repair child session，暂停 Coding，并通过 WebSocket 暴露 request、session link 与重连 snapshot。
- 本 Task 不应用 Amendment、不释放 Active Amendment Lock，也不启动 P6 前端 Repair Center。
- 按用户裁决严格使用当前 schema：WorkItemGroup 必须存在 authoritative Plan lineage；不为历史缺失 lineage 数据增加兼容 fallback。

## 实现摘要

1. 新增 `CodingWorkspaceEngine::start_plan_repair_from_review` 与 Internal/Group Reviewer 入口：
   - 先用 authoritative Reviewer Projection 校验 Finding；
   - 生成 canonical fingerprint；
   - 同 fingerprint 复用 P4 open request/child session，并合并 evidence；
   - 不同 fingerprint 不得误复用 active request、UnitRun 或 child session，冲突时 fail closed；
   - Group Reviewer 必须唯一绑定到 authoritative latest `CodingUnitRun` 与 projection。
2. 复用 P4 已持久化的 Repair Request、Active Amendment 与 Workspace Session Link 作为 durable anchor，不新增第二套跨 Store journal。幂等协调顺序为：
   - Attempt → `AwaitingPlanAmendment`；
   - trigger UnitRun → `BlockedByPlanDefect`；
   - 唯一 `Plan Repair` blocked timeline node。
3. Plan Repair counter 只在首次阻塞 UnitRun 时增加一次；`unit_rework_count`、verification retry 与 operational retry 均不增加。重连与重复 finding 不重复 request、session、link、timeline 或 counter。
4. `linked_active_plan_repair_snapshot` 以 `plan.active_amendment_id` 选择唯一当前 request/link/snapshot：
   - 历史 Applied/Completed link 不再造成当前重连 Ambiguous；
   - 当前候选缺失、重复或 identity 不匹配继续 fail closed；
   - session-state 与 provider 入口可从仅完成 durable anchor 的崩溃前缀恢复暂停状态。
5. `StartCoding`、Coder、Tester、CodeReviewer、Internal/Group Reviewer、rework、provider stream 与 failed-review recovery 均调用统一 provider gate。三种 amendment 状态统一返回 `plan_amendment_blocks_provider_run`，真实 provider start 保持 0。
6. WebSocket 新增：
   - `PlanRepairRequired { request, session_link }`；
   - `PlanAmendmentUpdated { amendment }`；
   - `CodingSessionState.linked_plan_repair`。
   Rust DTO 使用 P4 canonical types；TypeScript 同步完整 Plan Repair request/link/snapshot/manifest 类型。
7. failed-review recovery 与 amendment 并发收敛：
   - prepare 在 Attempt lock 内重读 authoritative Attempt，再在 journal lock 内 load/create；
   - 三种 amendment 状态写 journal 前统一拒绝；
   - amendment transition 只移除尚未推进的 `Prepared` journal；
   - 全局同名 gate 搜索将 recoverable 与 amendment-blocked candidates 分开，唯一正常 recoverable candidate 不被无关同名 gate 阻断。
8. `tests/it_web/web_coding_ws_handler/part_09.rs` 的 WorkItemGroup helper 补齐现有 `seed_authoritative_group_plan_fixture`。生产代码保持严格 lineage，未通过放宽 schema 修复旧 fixture。

## TDD 证据

### RED

- 历史 Plan Repair link 与当前 link 并存时，重连错误返回 `coding_linked_plan_repair` Ambiguous。
- active repair 期间提交不同 fingerprint finding 时，旧实现错误返回成功并复用 active child/request。
- failed-review recovery prepare 与 amendment transition 竞态可留下 `Prepared` journal。
- 全局 recovery 遇到无关 Attempt 的同名 amendment-blocked gate 时，错误提前返回 amendment block。
- durable request/session link 已落盘但 Attempt、UnitRun 或 timeline 尚未推进时，provider/session-state 缺少统一恢复。
- `it_web` 的旧 Group fixture 缺 authoritative Plan lineage，WebSocket handler fail closed 且测试等待首帧，导致后续用例被 `WS_TEST_LOCK` 阻塞。

### GREEN / Focused

- `cargo test --locked --lib coding_plan_repair_`：61 passed，0 failed。
- `cargo test --locked --lib coding_ws_plan_repair_`：4 passed，0 failed。
- `cargo test --locked --lib failed_review_recovery`：37 passed，0 failed。
- recovery/amendment 并发回归循环 20 次：全部通过，最终 journal 均为 `None`。
- 修复 Group fixture 后，原挂起精确 `it_web` 用例：1 passed，0 failed，约 0.04s。
- `cargo test --locked --test it_web`：258 passed，0 failed，12 ignored。

## Review Follow-up 闭环

1. **Historical PlanRepair links**：当前 link 按 authoritative `active_amendment_id` 唯一选择；历史 link 不参与当前候选。
2. **Distinct fingerprint active request**：expected fingerprint、request、UnitRun 与 linked snapshot 全量 identity 校验；不同 fingerprint fail closed 且不产生第二 request/link/session。
3. **跨 Store 一致性**：P4 durable anchor 驱动 Attempt、UnitRun、timeline 的幂等 reconcile，支持各持久化前缀重放。
4. **Recovery journal race**：Attempt → journal 锁顺序与 authoritative re-read 阻止 amendment 状态下的新 Prepared journal。
5. **Global gate ID collision**：唯一 recoverable candidate 优先；blocked candidate 不创建第二 gate/journal。

## 最终完整门禁

- `cargo fmt --check`：PASS，exit 0。
- `cargo check --locked`：PASS，exit 0。
- `cargo clippy --all-targets --all-features --locked -- -D warnings`：PASS，exit 0，0 warnings。
- `cargo test --locked`：PASS，exit 0：
  - lib：1069 passed，0 failed；
  - it_core：143 passed，0 failed；
  - it_web：258 passed，0 failed，12 ignored；
  - doc-test：1 passed，0 failed；
  - 其余 integration targets 全部通过。
- `cd web && pnpm tsc -b`：PASS，exit 0。
- `cargo test --locked --test it_core large_file_guard::product_source_and_test_files_stay_under_line_limit`：1 passed，0 failed。
- `git diff --check`：PASS，exit 0。

## 文件规模

- `src/web/coding_ws_handler/tests/plan_repair.rs`：756 行。
- `src/web/coding_ws_handler/tests/failed_review_recovery.rs`：715 行。
- `src/web/coding_ws_handler/tests/plan_repair/reconciliation.rs`：314 行。
- `src/product/coding_workspace_engine/plan_repair_start.rs`：322 行。
- `src/product/coding_attempt_store/plan_repair_reconcile.rs`：250 行。
- `src/web/coding_ws_handler/tests/failed_review_recovery/plan_amendment.rs`：112 行。

所有相关 Rust/TS/TSX 文件均满足不超过 800 行门禁。

## 自审结论与交付边界

- Task 3 的 request、pause、counter、timeline、WS、重连与 provider fail-closed 已闭环；未发现剩余 blocker 或 warning。
- 保持严格 authoritative schema，不兼容缺失 WorkItemGroup Plan lineage 的历史 fixture/data。
- Amendment 应用、Binding 切换、Resume Target 与 Active Amendment Lock 释放留给 P5 Task 4。
- 按明确要求未运行 E2E、Playwright 或浏览器测试。
- 本次只创建原子 commit，不执行 push。
