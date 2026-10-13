# N5 报告：首项完成安全暂停与 runner 返回（REQ-CS-05/03）

- Change：`openspec/changes/fix-multi-repo-group-coding-start/`（Task 2.3，REQ-CS-05/03）
- Plan：`cadence/plans/2026-10-13_实施计划_多仓分组coding启动_v1.0.md` §6
- Worktree：`.worktrees/feat-b-0808-add-monorepo`（分支 feat-b-0808-add-monorepo）
- 状态：DONE（红→绿→回归全绿；独立 commit `01330f24`，父提交 `2ba4bd12`=N3）

## 交付（三处窄改；计划 Files 中 engine group.rs 最终零改动）

- **Modify `src/product/coding_workspace_engine/group_completion.rs`**：
  - **入口 reconcile**：`complete_group_unit_after_code_review` 在 preflight 之前新增
    `reconcile_paused_plan_first_unit`——attempt 已处于 N2 安全暂停形状
    （Blocked+CodeReview+清 active/current 指针）且 `matching_first_ready_run` 命中、
    run 未 Invalidated 时，以幂等 `pause_completed_plan_first_unit` 对齐并立即返回暂停
    attempt。覆盖两窗口：①暂停后 CompletedRetry 重入不再被 preflight 按「非 Running」
    误拒（旧形状会 `group_completion_status_not_ready` 失败）；②attempt 暂停已写而
    run Paused 补记中断——重入自动补记 run Paused。
  - **尾部短路**：完成事实（completion commit/权威 unit run Completed/handoff/apply
    transition）落定后、`advance_to_next_group_unit` 及其 SC selector 之前，Running 分支
    显式把 selected unit 置 Completed（原 `complete_current_group_unit` 的前半段，同一
    summary），随后调用 N2 `pause_completed_plan_first_unit`；命中即返回暂停 attempt。
    旧无 run 路径行为等价保留：unit Completed 后直接 `advance_to_next_group_unit`（与原
    `complete_current_group_unit→advance` 组合逐步等价，`complete_current_group_unit`
    函数本体与其余调用方零改动）。
  - **runner 识别入口**：`pub fn plan_first_unit_safely_paused`——暂停形状 +
    matching first_ready run `phase=Paused` 才为 true（真实暂停身份，非任意 Blocked
    文本）；无 run/未暂停/Invalidated 均 false。
  - 自由函数 `plan_coding_pause_shape` 仅做形状判定，暂停证据始终由 run 相位核验。
- **Modify `src/web/coding_ws_handler/runner.rs`（生产面 13 行）**：`execute_start_coding_flow`
  group completion 分支在 `refresh_runtime_history` 后、任何 emit/ReviewRequest/push 之前
  检查 `plan_first_unit_safely_paused`，命中即 emit session_state 并立即 `return Ok`——
  不依赖 stage 自然停止，内部循环不再获得推进机会。N2 `ensure_provider_run_allowed`
  （flow 入口调用）继续作为普通 retry/StartCoding 的第二防线，未新增 WS 自动
  start/重置 claim。
- **Test**：`tests/group_completion_recovery.rs` 追加 4 测（含
  `seed_plan_coding_first_ready_run` 夹具：按 N2 冻结地址直写
  `{issue_root}/coding-plan-runs/{plan_id}.json`，selection 指向 fixture attempt/首 unit，
  phase=Started）；`runner.rs` 新增内联 `#[cfg(test)] mod tests`（原无测试模块），复刻
  web 版最小 group 夹具（真实 Git worktree+plan lineage/binding/plan session 历史工件+
  Running unit run）+ CountingProvider 受控 probe（start 即计数并失败）。

## 证据（cargo test --locked --lib，红→绿→回归）

| 命令 | 红（实现前） | 绿（rustfmt 后复验同） |
|---|---|---|
| `plan_coding_n5_` | 引擎 3 测业务红：完成后 attempt 仍 Running（旧路径推进下一 unit），`left: Running, right: Blocked`；runner 测业务红：单 unit 完成后直入 ReviewRequest/push——`git ls-remote --heads origin` 失败暴露无暂停短路；`plan_coding_n5_no_run_uses_existing_advance` 基线即绿（回归位） | **5 passed** |
| `group_completion_authority`（计划指定绿窗） | — | 38 passed（含既有 owner-conflict/abort/mutation-pause 全族） |
| `coding_plan_repair_group_completion` | — | 15 passed |
| `plan_coding_n2_`（N2 pause 原语族，含 77ab13d7 相位门修复后状态） | — | 8 passed |
| `plan_coding_n1_` | — | 3 passed |
| `group_final_readiness`（execute_start_coding_flow 面） | — | 30 passed |
| `runner`（web 全族） | — | 61 passed / **2 failed（既有红，非 N5，见下）** |

新测试断言覆盖 plan 验收句：completion commit=git HEAD/权威 run Completed+commit/handoff/
unit Completed 均不回滚；有/无剩余 units 两族均 Blocked+CodeReview+清指针+恰一空 actions
gate（reason=`plan_coding_continuation_not_enabled`）；其余 units 保持 Pending 且零 run、
SC selector 零落盘（`get_group_dependency_gate_snapshot` None）、review request/push 零
（`list_review_requests` 空）、provider probe starts=0；runner emit session_state 后返回；
暂停后重入 runner 被 `plan_coding_paused` 拒且状态零变化；integrity 认可合法暂停分支；
无 run 旧 group 仍推进（unit2 Running+PrepareContext+无 gate）。

## 既有红（非 N5，请 Main 知悉）

`runner` 族 2 失败：`web::coding_start::tests::runner_registered_window_resumes_same_command_only`
与 `web::advance_plan::tests::enrolled_advance_replays_one_ready_attempt_without_starting_runner`，
均 panic 于 `src/web/wiga_gate_fixture.rs:674`（"provenance boundary must leave durable
compile reservation"）。**基线归因**：在临时 worktree 于 HEAD `9c6bf236`（N7 提交，不含
N5 任何改动）复跑，两测同样失败——与 N5 改动无因果（N5 短路仅在存在 matching
first_ready plan-run 时激活，该两测无 coding-plan-runs）。属 wiga/advance 既有回归，
未越权修复。

## 边界与移交

- `coding_workspace_engine/group.rs`（计划 Files 列为 Modify）最终零改动：短路落点在
  `complete_current_group_unit/advance_to_next_group_unit` 调用之前（group_completion.rs
  尾部），两函数本体与其余调用方不受影响——比计划更窄且满足「在 selector 之前停止」。
- 消费 N2 接口：`pause_completed_plan_first_unit`（幂等可重入，含 run Paused 补记）、
  `matching_first_ready_run`；已按 N2 修复轮（77ab13d7 相位门）后的语义联测通过。
  未改 `coding_attempt_store/group.rs`/`coding_plan_run.rs`/`gate.rs`。
- 编译窗口协调记录：期间两次共享 lib test 目标被兄弟在途编辑瞬时弄破（N2
  coding_plan_run.rs 半写、N3 plan_start_tests.rs 缺失/导入错），均经消息协调+有界重试
  等待恢复，未触碰任何他人文件。
- S2（暂停后的显式 plan 授权恢复/serial 模式）不在本片；本片保证未授权恢复一律
  Blocked+CodeReview 停住。
