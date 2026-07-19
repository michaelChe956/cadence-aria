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
   - 新增 per-attempt arbitration guard，统一锁序为 arbitration → Attempt/Journal/Role/Gate；
   - prepare、journal advance、retry role-run、Attempt Running、gate resolve、complete/archive 每个写边界都重读 authoritative Attempt 与 active amendment；
   - Plan Repair pause 与 P4 durable 写入持有同一 arbitration，可回滚未完成 recovery 前缀并恢复 stale failed run/gate；
   - Completed recovery journal 保持终态，不允许 Plan Repair 抢占；
   - 全局同名 gate 搜索将 recoverable 与 amendment-blocked candidates 分开，唯一正常 recoverable candidate 不被无关同名 gate 阻断。
8. P5 复用 P4 canonical parent/child/link arbitration：
   - deterministic orphan child 在 link 缺失前缀下不再被误判为第二 parent；
   - P4 自身验证传入 parent 是 canonical parent；
   - reconnect 复用 P4 canonical link/child validator，拒绝伪造 link ID、child ID、parent、return route 与 child record identity。
9. Coder、reviewer-triggered CoderRework 与 Tester 的 canonical typed Plan Defect report 由 runner 统一消费：只有 exact `StartPlanRepair` 创建 Repair；Story/Design/Operational/Verification/HumanTriage 保持原 route/safe-stop。
10. `tests/it_web/web_coding_ws_handler/part_09.rs` 补齐 authoritative Plan lineage；`part_08.rs` 补齐 canonical WorkItemPlan parent workspace。生产代码保持严格 schema，未通过放宽 lineage 或 parent 约束修复旧 fixture。

## TDD 证据

### RED

- 历史 Plan Repair link 与当前 link 并存时，重连错误返回 `coding_linked_plan_repair` Ambiguous。
- active repair 期间提交不同 fingerprint finding 时，旧实现错误返回成功并复用 active child/request。
- failed-review recovery prepare 与 amendment transition 竞态可留下 `Prepared` journal。
- 全局 recovery 遇到无关 Attempt 的同名 amendment-blocked gate 时，错误提前返回 amendment block。
- durable request/session link 已落盘但 Attempt、UnitRun 或 timeline 尚未推进时，provider/session-state 缺少统一恢复。
- `it_web` 的旧 Group fixture 缺 authoritative Plan lineage，WebSocket handler fail closed 且测试等待首帧，导致后续用例被 `WS_TEST_LOCK` 阻塞。
- P4 orphan child 已创建但 link 尚未写入时，P5 把 child 当作第二 parent，返回 `parent WorkItemPlan workspace is ambiguous`。
- same fingerprint、不同 Attempt/UnitRun 的 request 在 identity mismatch 前已合并 evidence，违反零写入 fail-closed。
- reconnect 在伪造 deterministic link/child identity 后仍能成功返回 snapshot。
- Coder/CoderRework typed outcome 丢失 canonical execution report，runner 无法统一启动 Repair。
- recovery 已推进到 advance/retry role/Attempt Running/gate resolve/complete 边界时，amendment pause 仍可能被 stale 写回覆盖。
- 旧 reviewer-triggered rework 集成 fixture 只有两个 `WorkItem` workspace、没有 canonical `WorkItemPlan` parent；chat route 已为 `start_plan_repair`，但 request、active amendment 与 counter 均保持 0。

### GREEN / Focused

- `cargo test --locked --lib coding_plan_repair_`：61 passed，0 failed。
- `cargo test --locked --lib coding_ws_plan_repair_`：4 passed，0 failed。
- `cargo test --locked --lib failed_review_recovery`：39 passed，0 failed。
- recovery/amendment 并发回归循环 20 次：全部通过，最终 journal 均为 `None`。
- 修复 Group fixture 后，原挂起精确 `it_web` 用例：1 passed，0 failed，约 0.04s。
- 补齐 canonical WorkItemPlan parent 后，reviewer-triggered rework 集成用例：1 passed，0 failed；同时断言 WS event request 等于 durable request、trigger identity 正确、`plan_repair_count == 1`。
- `cargo test --locked --test it_web`：258 passed，0 failed，12 ignored。

## Review Follow-up 闭环

1. **Historical PlanRepair links**：当前 link 按 authoritative `active_amendment_id` 唯一选择；历史 link 不参与当前候选。
2. **Distinct fingerprint active request**：expected fingerprint、request、UnitRun 与 linked snapshot 全量 identity 校验；不同 fingerprint fail closed 且不产生第二 request/link/session。
3. **跨 Store 一致性**：P4 durable anchor 驱动 Attempt、UnitRun、timeline 的幂等 reconcile，支持各持久化前缀重放。
4. **Recovery 全阶段 arbitration**：共享 per-attempt arbitration 覆盖 prepare、advance、role-run、Attempt、gate、complete/archive 与 Plan Repair pause；每个写边界重验 authoritative amendment 状态。
5. **Global gate ID collision**：唯一 recoverable candidate 优先；blocked candidate 不创建第二 gate/journal。
6. **Orphan child recovery**：P5 使用 P4 deterministic child/canonical parent 逻辑，不再阻断 child-before-link 崩溃前缀。
7. **Existing request zero-write identity**：在调用 P4/evidence/session 写入前校验 Attempt 与 UnitRun，mismatch 不产生任何持久化变化。
8. **Canonical reconnect validation**：active amendment 唯一选择 request，并复用 P4 link/child validator 校验全部 deterministic identity 与真实 child record。
9. **Typed execution sources**：Coder、CoderRework、Tester 保留 canonical report；runner 只对 exact `StartPlanRepair` 调用统一 start API。
10. **旧集成 fixture**：补 canonical WorkItemPlan parent，测试等待真实 `PlanRepairRequired`，并比较事件 request 与同一持久化根下的 durable request。

## 最终完整门禁

- `cargo fmt --check`：PASS，exit 0。
- `cargo check --locked`：PASS，exit 0。
- `cargo clippy --all-targets --all-features --locked -- -D warnings`：PASS，exit 0，0 warnings。
- `cargo test --locked`：PASS，exit 0：
  - lib：1080 tests，0 failed；
  - it_core：143 passed，0 failed；
  - it_web：258 passed，0 failed，12 ignored；
  - doc-test：1 passed，0 failed；
  - 其余 integration targets 全部通过。
- `cd web && pnpm tsc -b`：PASS，exit 0。
- `cargo test --locked --test it_core large_file_guard::product_source_and_test_files_stay_under_line_limit`：1 passed，0 failed。
- Task 3 测试命名审计（含未跟踪 `identity.rs`、`typed_sources.rs`）：PASS，新增测试均以 `coding_plan_repair_` 或 `coding_amendment_` 开头。
- `git diff --check`：PASS，exit 0。

## 文件规模

- `src/product/coding_attempt_store/recovery.rs`：720 行。
- `src/product/workspace_engine/plan_repair.rs`：732 行。
- `src/web/coding_ws_handler/runner.rs`：776 行。
- `src/product/coding_attempt_store/tests/failed_review_recovery.rs`：677 行。
- `src/web/coding_ws_handler/tests/plan_repair/identity.rs`：444 行。
- `src/web/coding_ws_handler/tests/plan_repair/typed_sources.rs`：235 行。
- `tests/it_web/web_coding_ws_handler/part_15.rs`：452 行。

所有相关 Rust/TS/TSX 文件均满足不超过 800 行门禁。

## 自审结论与交付边界

- Task 3 的 request、pause、counter、timeline、WS、重连与 provider fail-closed 已闭环；未发现剩余 blocker 或 warning。
- 保持严格 authoritative schema，不兼容缺失 WorkItemGroup Plan lineage 的历史 fixture/data。
- Amendment 应用、Binding 切换、Resume Target 与 Active Amendment Lock 释放留给 P5 Task 4。
- 按明确要求未运行 E2E、Playwright 或浏览器测试。
- 本次只创建原子 commit，不执行 push。
