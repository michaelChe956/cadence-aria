# S2b 报告：N5 步暂停移除与完成链自动推进

- 日期：2026-10-14
- 分支：feat-b-0808-add-monorepo（worktree）
- 提交：`fix: S2b N5步暂停移除与完成链自动推进`
- 面：engine 完成链（B 半）；与 S2a（web 入口面：plan_start.rs/coding_plan_run.rs/plan_start_tests.rs/mod.rs）零文件交集

## 冻结语义（owner 直批）

unit 完成→引擎自动推进全部就绪项（依赖图并行，per-unit/per-plan 粒度，无跨 plan 串行假设——终态为全局工作池调度）；每步暂停不再默认；失败→依赖链停/独立链续；plan_complete=全 Completed。

## 改动

### 1. `src/product/coding_workspace_engine/group_completion.rs`（N5 短路点）
- 尾部：`complete_group_unit_after_code_review` 完成事实落定后不再调用 `pause_completed_plan_first_unit` 短路返回，直接 `advance_to_next_group_unit` 自动推进。推进保持独立 pub 入口（per-unit/per-plan 粒度），供自动化编排器/入口面 drive 桥复用同一推进语义。
- 头部：移除 `reconcile_paused_plan_first_unit`（CompletedRetry 暂停身份识别/补写）。legacy 暂停形状（Blocked+CodeReview+清指针）重入由 preflight 失败关闭（`group_completion_stage_not_ready`），恢复走入口面 resume 语义。
- 删除 `plan_first_unit_safely_paused`、`plan_coding_pause_shape`（全库零残留引用）。

### 2. `src/web/coding_ws_handler/runner.rs`（二保险同步+完成事件挂点）
- 移除 N5 停止保险块（完成返回后 `plan_first_unit_safely_paused`→emit+return）。完成链自动推进后直接 emit session_state；PrepareContext 即 `continue 'pipeline` 内循环驱动下一 unit。
- 完成事件→跨桶就绪集推进挂点：完成 emit 后调用 `crate::web::handlers::drive_ready_units_for_plan`（S2a 面 plan_start.rs）。runner 任务不在入口 POST 锁内，按缝合锁契约自取 `work_item_group:{p}:{i}:{plan}` 串行锁（`state.coding_runs.lock_named`）；桥幂等、跳过忙 attempt，失败仅 `tracing::warn` 不阻断本桶执行链。

### 3. `src/product/coding_attempt_store/gate.rs`（P5 撤除）
- `ensure_provider_run_allowed` 移除「冻结 selected unit 已 Completed 而 pause 尚未写回」中断窗口拒绝（`plan_coding_first_unit_completed`）。完成链自动推进在同一 run 授权下启动后续 unit 的 provider run。
- 保留 Paused（legacy 步暂停，恢复走入口面 resume）与 Invalidated（删除授权失效）拒入。

### 4. 收口（S2a 面落地后）
- 删除 `pause_completed_plan_first_unit`（`src/product/coding_attempt_store/group.rs`）：生产调用点全部移除、S2a 测试迁移零调用后收口；唯一暂停写入方消失，新数据不再产生步暂停。
- `src/web/handlers/coding.rs` 透出 `pub(crate) use plan_start::drive_ready_units_for_plan;`（接口暴露行；handlers/mod.rs 的再透出由 S2a 提交承载）。

### 5. 保持不变（语义保持面）
- `advance_to_next_group_unit` 语义/锁转移不变（Waiting 清执行权威指针保留进度指针、FailedClosed、Complete→ReviewRequest）。
- AwaitingStructuralClosure 恢复分域语义未触碰。
- `group_validation.rs` 的 legacy 暂停形状豁免（`plan_coding_pause_matches_completed_selection`）保留：advance/group 端点完整性门依赖其放行 legacy 暂停数据，删除会阻断 S2a resume 语义；豁免只读不写，不产生新暂停。
- `mark_paused`/`PLAN_CODING_CONTINUATION_NOT_ENABLED`/prepare/selection（S2a 面）未动。

## 测试（TDD）

红→绿：S2 族 6 测（`plan_coding_s2_*`）：
1. `completion_auto_advances_next_ready_unit`（红测①）：WI-001 完成→引擎自动 start WI-002（unit Running+active 指针+PrepareContext+Ready 快照；run 保持 Started、零暂停 gate；P5 撤除后完成窗口 provider 准入放行）。
2. `unmet_dependency_waits_without_start`（红测②）：依赖未就绪→Waiting（consumer Pending 零 run、清指针、Waiting 快照；run 保持 Started）。
3. `failure_stops_dependent_chain_without_pause`（红测③）：非重试失败→attempt/unit Blocked、依赖 unit Pending 零 run、失败门（非暂停门）、run 保持 Started。
4. `last_unit_completion_reaches_review_request`：plan_complete→ReviewRequest（不再 Blocked+CodeReview 暂停）。
5. `completed_retry_recovers_and_advances`：中断窗口重入恢复完成并自动推进；legacy 暂停形状重入失败关闭。
6. `runner_flows_to_final_confirm_with_plan_run`（runner）：单 unit plan 全流程推进至 FinalConfirm+WaitingForHuman（review request 发出、run 保持 Started、零暂停 gate、零 provider start）。

N5 既有测试迁移：`plan_coding_n5_completion_pauses_before_selector`→①、`_last_target_unit_stays_code_review`→④、`_completed_retry_reconciles_pause_window`→⑤、runner `_retry_and_runner_do_not_push_or_start_next`→⑥；`_no_run_uses_existing_advance`、`_second_unit_advance_keeps_worktree_lock_transfer` 保留（无 run 行为不变）。授权类（coding_plan_run.rs n2 族）由 S2a 单写者迁移（窗口断言反转为准入放行）。

## 验证

- 红阶段：S2 族按新契约在旧代码上先红（完成即暂停/停点短路）。
- 绿阶段（主树，S2a 面合并态）：
  - `cargo test --lib plan_coding_s2`：10/10 绿（我面 6+S2a 入口面 4，缝合端到端）。
  - `cargo test --lib`（全量）：**4691 passed; 0 failed; 7 ignored**（含 group_completion 42、campaign_stage3 族、sc_group_dependency_gate、group_final_readiness、runtime_handoff、runner_fallback、coding_ws_handler::runner 族）。
- 隔离树基线（HEAD+仅我 4 文件）：`plan_coding_s2` 6/6 绿——我的面在干净基线上独立成立。
- rustfmt 已限我文件执行。

## 缝合（S2a↔S2b，冻结契约）

- 入口面 `drive_ready_units_for_plan(state, project, issue, plan) -> PlanContinuationDriveOutcome{started,resumed,busy}`（S2a，plan_start.rs）：就绪集并行驱动（无效授权 409、首启窗口零动作、Paused 先 resume）；锁契约=调用方持 `work_item_group:{p}:{i}:{plan}` 串行锁（runner 挂点侧自取，已接线）。
- web/error.rs 等入口面 `plan_coding_first_unit_completed` 错误码映射已成死码（S2a 面处置，未动）。

## 四字段

- 状态：DONE
- 提交：`fix: S2b N5步暂停移除与完成链自动推进`（gate.rs/group.rs/group_completion.rs/group_completion_recovery.rs/runner.rs/coding.rs+本报告）
- 验证：全量 `cargo test --lib` 4691 过 0 挂；S2 缝合 10/10 绿；隔离树基线 6/6 绿
- 顾虑：无阻塞。备注两点：①legacy 暂停数据的 resume 语义归 S2a 桥（我面失败关闭+豁免保留，已对齐）；②`plan_coding_first_unit_completed` 的 web 错误码映射为死码，归 S2a 面后续清理。
