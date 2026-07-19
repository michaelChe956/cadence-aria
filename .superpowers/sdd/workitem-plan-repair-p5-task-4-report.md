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
  3. 发送 `PlanAmendmentUpdated`；
  4. marker compare-and-mark `Delivered`；
  5. 恢复 Attempt 为 runnable，或保持 `AwaitHandoff` blocked。
- send 失败或 Delivered 标记失败时 Attempt 不会变为 runnable。
- send 成功但 mark 前崩溃时，恢复会用相同 `event_id` 重发；因此语义是 **no-loss + consumer-idempotent，允许重复投递**，不是物理 exactly-once。
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
- `coding_amendment_delivery_send_failure_keeps_attempt_non_runnable_with_pending_marker`。
- `coding_amendment_delivery_retries_same_event_after_send_before_mark_failure`。
- `coding_amendment_concurrent_recovery_reconciles_one_durable_delivery`。

## Full gate 首个失败与最小修复

- RED：首次 strict clippy 在 `amendment_recovery.rs` 报 `clippy::nonminimal_bool`。
- 根因：新增 `AwaitHandoff` 合法状态后使用 `!A && !B`，语义正确但不满足 strict lint。
- GREEN：改为等价 De Morgan 表达式 `!(A || B)`，不添加 lint allow；AwaitHandoff 定向测试 1/1 与 strict clippy fresh 通过。

## 验证结果

### Fresh focused

- `cargo test --locked --lib coding_amendment_`：31 passed，0 failed。
- `cargo test --locked --lib coding_ws_plan_repair_`：2 passed，0 failed。
- `cargo test --locked --lib coding_amendment_updated_roundtrips`：1 passed，0 failed。
- 上述 Rust 命令共 34 次 test execution；protocol roundtrip 已包含于 Amendment 31，因此为 33 个 unique tests。
- `cd web && pnpm tsc -b`：PASS，exit 0。

### 最终非 E2E full gate

- `cargo fmt --check`：PASS，exit 0。
- `cargo check --locked`：PASS，exit 0。
- `cargo clippy --all-targets --all-features --locked -- -D warnings`：PASS，exit 0，0 warnings。
- `cargo test --locked`：PASS，exit 0：
  - lib：1119 passed；
  - it_core：143 passed；
  - it_interactive：43 passed；
  - it_product：211 passed；
  - it_provider：55 passed；
  - it_task_run：31 passed；
  - it_web：258 passed，12 ignored；
  - doc-test：1 passed；
  - 合计：1861 passed，12 ignored，0 failed。
- `cd web && pnpm tsc -b`：PASS，exit 0。
- `cargo test --locked --test it_core large_file_guard::product_source_and_test_files_stay_under_line_limit`：1 passed，0 failed。
- 测试命名审计：PASS；新增行为测试使用 `coding_amendment_` / `coding_ws_plan_repair_` 前缀，无 `test`、`tmp`、`todo` 等占位命名。
- `git diff --check`：PASS，exit 0。
- 未运行 E2E、Playwright 或 browser。

## 文件行数

| 文件 | 行数 |
| --- | ---: |
| `src/product/coding_attempt_store/tests/plan_repair.rs` | 782 |
| `src/product/coding_workspace_engine/tests/plan_amendment.rs` | 769 |
| `src/web/coding_ws_handler/tests/plan_repair.rs` | 766 |
| `src/product/lifecycle_store/workspace.rs` | 733 |
| `src/product/coding_workspace_engine/amendment.rs` | 717 |
| `src/product/coding_workspace_engine/tests/plan_amendment/review_fix_identity.rs` | 656 |
| `src/product/work_item_revision_store/tests/concurrency.rs` | 641 |
| `web/src/api/types/coding.ts` | 622 |
| `src/product/coding_attempt_store/unit_run_amendment.rs` | 621 |
| `src/product/work_item_revision_store/repair.rs` | 536 |
| `src/product/coding_attempt_store/unit_run.rs` | 486 |
| `src/web/coding_ws_handler/tests/plan_repair/runner_amendment_recovery.rs` | 457 |
| `src/product/work_item_revision_store/plan.rs` | 385 |
| `src/product/coding_attempt_store/plan_binding.rs` | 384 |
| `src/product/coding_attempt_store/paths.rs` | 353 |
| `src/product/coding_workspace_engine/tests/plan_amendment/review_fix_unit_runs.rs` | 255 |
| `src/web/coding_ws_handler/protocol.rs` | 217 |
| `src/product/coding_attempt_store/amendment_delivery.rs` | 217 |
| `src/product/coding_workspace_engine/tests/plan_amendment/review_fix_delivery.rs` | 192 |
| `src/product/coding_attempt_store/mod.rs` | 128 |
| `src/product/coding_models/plan_repair.rs` | 121 |
| `src/product/coding_attempt_store/locking.rs` | 91 |
| `src/product/coding_attempt_store/amendment_recovery.rs` | 89 |
| `src/product/coding_attempt_store/amendment_arbitration.rs` | 28 |

本次 changed Rust/TS/TSX 文件最大为 782 行；large-file guard fresh 通过，全部不超过 800 行。

## 自审结论与边界

- authoritative identity、durable Journal、arbitration/CAS 与锁序满足 fail-closed、zero-write 和崩溃恢复要求。
- forged same-ID Journal/UnitRun、lineage replacement、并发 recovery、dirty identity mismatch 均有回归覆盖。
- provider entry 在 durable delivery 成功且 finalization 收敛前保持阻断。
- delivery 保证 no-loss 与稳定 event ID 下的 consumer idempotency；可能重复，不宣称 exactly-once。
- 未添加历史 v1/缺失 Plan lineage 兼容 fallback。
- worktree 保留；单个原子 fix commit；不 push。
