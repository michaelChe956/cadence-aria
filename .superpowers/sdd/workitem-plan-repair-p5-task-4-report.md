# P5 Task 4 幂等应用 Plan Amendment 与崩溃恢复实施报告

## 交付摘要

- 基线：`1bf8955b9fabf5927c186d703b84952dfbf5e85e`
- 分支：`feat-b-0715`
- 目标提交：`feat(coding-repair): apply and recover plan amendments`
- 范围：P5 Task 4，仅实现 Amendment 应用、Journal 恢复、Binding/UnitRun/Resume Target 切换与 Coding WS runner 恢复。
- 明确未实现：P5 Task 5 Handoff Revision 与运行时影响传播。

## 实现结果

1. 新增 Amendment application engine：
   - 严格校验 WorkItemGroup Attempt、Plan Binding、Plan lineage、active revision、active amendment lock、repair request、stored manifest、revision supersedes、session link 与 snapshot identity。
   - 缺失或不一致的 v1/历史 lineage 不做兼容 fallback，统一 fail-closed。
   - dirty worktree 在创建 Journal 前阻断，并幂等创建 `worktree_dirty_before_plan_amendment` manual gate。

2. 新增 durable application Journal：
   - Phase：`Started -> PlanBindingWritten -> UnitRunsWritten -> ResumeTargetWritten -> Completed`。
   - Phase 仅允许单调推进；相同 amendment ID 重放保持幂等。
   - 任一步失败时保留最后成功 Phase，写入 error，并将 Attempt 更新为 `AmendmentApplyFailed`。
   - 恢复时清除 error，从最后 durable Phase 继续。

3. Binding 与 UnitRun materialization：
   - Plan Binding 只允许从 manifest previous revision 前进到 new revision，并将 amendment ID 追加到 applied 列表末尾。
   - 只 materialize revised、stale、revalidation-required 与 replacement target Units。
   - unaffected Units 不创建新 UnitRun。
   - 历史 Completed UnitRun 保持 Completed；仅 supersede active run。
   - 新 UnitRun 的 unit/verification/operational retry counter 归零，Attempt 全局 `rework_count` 不变。
   - 相同 amendment 重放不会创建重复 UnitRun。

4. Resume Target 与最终化：
   - `Reexecute` 恢复到 Coding/Running。
   - `Revalidate` 恢复到 Testing/NeedsRevalidation。
   - `AwaitHandoff` 保持 AwaitingAmendment 并恢复到 Coding stage。
   - repair request 更新为 Applied、child session 更新为 Completed、workspace session 终止。
   - active amendment lock 仅在 Journal durable Completed 后释放。
   - Completed Journal 不 early-return，仍会重复 reconcile request/session/lock/Attempt resume。

5. 失败与崩溃恢复：
   - 覆盖 Started、PlanBindingWritten、UnitRunsWritten、ResumeTargetWritten 每个 durable boundary。
   - 覆盖 Completed 后 request/session/lock/Attempt 未最终化的恢复。
   - 覆盖 Completed 后最终化失败并再次恢复。
   - phase failure 后 session snapshot 进入 `AmendmentApplyFailed`；恢复 identity 校验仅在 Attempt 同为 `AmendmentApplyFailed` 且 snapshot 带 error 时接受该 stage，其他 identity 条件不放宽。

6. Coding WS runner：
   - runner 入口读取 authoritative Attempt 后，遇到 `AwaitingPlanAmendment`、`ApplyingPlanAmendment` 或 `AmendmentApplyFailed`，先调用 `recover_plan_amendment`。
   - provider gate 使用恢复后的 Attempt；application durable completion 前 provider 不会进入。
   - Awaiting、Applying crash 与 Failed recovery 三态均覆盖。
   - 每次 runner recovery 在 Coding stage gate 前恰好发送一次 `PlanAmendmentUpdated`，并确认 provider start count 为 0。

## TDD 与问题定位记录

### Amendment failure recovery

- RED：`coding_amendment_recovers_failed_phase_and_clears_error` 返回：
  - `IdentityMismatch { kind: "coding_plan_amendment_application", id: "plan_amendment_plan_repair_fingerprint_0001" }`
- exact identity 审计确认 journal、binding、plan、request、stored manifest、session link 与 snapshot ID 全部一致。
- 根因：首轮失败有意将 snapshot stage 写为 `AmendmentApplyFailed`，恢复校验却无条件要求 `Published`。
- GREEN：仅增加 Attempt/snapshot/error 三字段一致的失败恢复状态组合，focused 14/14 通过。

### WS runner recovery

- RED：Awaiting case 在任何 amendment event 前结束，首个失败为 `runner ended before amendment recovery for Awaiting`。
- 根因：`execute_start_coding_flow` 在恢复前立即调用 `ensure_provider_run_allowed`。
- GREEN：将三种 amendment status 的恢复放在 provider gate 前；Awaiting/Applying/Failed 三态均恢复完成，provider 未提前启动。

### 质量门禁修复

- strict clippy 首次失败：测试 helper `seed_unit_run` 8 参数触发 `too_many_arguments`。
- 修复：helper 内从 `store.paths()` 构造 revision store，删除冗余参数，不添加 lint allow。
- full test 首次唯一失败：rustfmt 后 `tests/plan_amendment.rs` 为 818 行，触发 800 行 guard。
- 修复：将 `seed_unit_run` 原样迁入 `tests/plan_amendment/support.rs`；最终主测试文件 764 行。

## 验证结果

### Task 4 focused

- `cargo fmt --check`：PASS。
- `cargo check --locked`：PASS。
- `cargo test --locked --lib coding_unit_run_`：10 passed，0 failed。
- `cargo test --locked --lib coding_plan_repair_`：86 passed，0 failed。
- `cargo test --locked --lib coding_amendment_`：14 passed，0 failed。
- `cargo test --locked --lib coding_ws_plan_repair_`：1 passed，0 failed。

### 最终非 E2E 门禁

- `cargo fmt --check`：PASS，exit 0。
- `cargo check --locked`：PASS，exit 0。
- `cargo clippy --all-targets --all-features --locked -- -D warnings`：PASS，exit 0，0 warnings。
- `cargo test --locked`：PASS，exit 0：
  - lib：1100 passed，0 failed；
  - it_core：143 passed，0 failed；
  - it_web：258 passed，0 failed，12 ignored；
  - doc-test：1 passed，0 failed；
  - 其余 integration targets 全部通过。
- `cd web && pnpm tsc -b`：PASS，exit 0。
- `cargo test --locked --test it_core large_file_guard::product_source_and_test_files_stay_under_line_limit`：1 passed，0 failed。
- 测试命名审计：PASS；新增行为测试分别以 `coding_amendment_`、`coding_ws_plan_repair_` 开头。
- `git diff --check`：PASS，exit 0。

## 文件规模

- `src/product/coding_attempt_store/unit_run.rs`：766 行。
- `src/product/coding_attempt_store/recovery.rs`：789 行，未继续扩张。
- `src/product/coding_attempt_store/amendment_recovery.rs`：77 行。
- `src/product/coding_workspace_engine/amendment.rs`：534 行。
- `src/product/coding_workspace_engine/tests/plan_amendment.rs`：764 行。
- `src/product/coding_workspace_engine/tests/plan_amendment/support.rs`：126 行。
- `src/web/coding_ws_handler/runner.rs`：784 行。
- `src/web/coding_ws_handler/tests/plan_repair.rs`：761 行。
- `src/web/coding_ws_handler/tests/plan_repair/runner_amendment_recovery.rs`：381 行。

所有产品源码与测试文件均满足不超过 800 行门禁。

## 自审结论与边界

- active amendment lock、Journal Phase 与最终化顺序符合 Task 4 durable recovery 要求。
- Completed Journal 重放会收敛全部最终状态，不依赖调用方陈旧 Attempt。
- provider entry 在 application durable completion 前保持阻断。
- 未添加历史 v1/缺失 Plan lineage 兼容。
- 未实现 Handoff Revision 解析、runtime handoff propagation 或 Task 5 impact propagation。
- worktree 保留，不 push。
