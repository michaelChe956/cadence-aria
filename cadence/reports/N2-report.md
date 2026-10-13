# N2 报告：plan-run 冻结与暂停准入（REQ-CS-02/05/07）

- Change：`openspec/changes/fix-multi-repo-group-coding-start/`（Task 1.2，REQ-CS-02/05/07）
- Plan：`cadence/plans/2026-10-13_实施计划_多仓分组coding启动_v1.0.md` §3
- Worktree：`.worktrees/feat-b-0808-add-monorepo`（分支 feat-b-0808-add-monorepo）
- 状态：DONE（红→绿→回归全绿；独立 commit `5b9adff3`）

## 交付（新建 + 三处窄改）

- **Create `src/product/coding_plan_run.rs`**（模型/store/内联 tests，自成模块）：
  - `CodingPlanRun`（project/issue/plan/command_id/bound_plan_revision_id/
    dependency_graph_revision_id/target_attempts(复用 `AdvanceTargetAttemptBinding`)/
    mode=`first_ready_unit`/selection/phase/start_outcome/pause_reason/created_at/updated_at）、
    `CodingPlanSelection`、`CodingPlanStartOutcome`（tagged `kind=started|already_started|needs_human`，
    NeedsHuman 含 reason）。
  - 磁盘地址冻结：`{issue_root}/coding-plan-runs/{plan_id}.json`，锁经
    `with_exclusive_lock` target-derived 派生为同目录前导点 `.{plan_id}.json.lock`
    （与计划修订后的锁名一致）；路径方法封装在 store，未改 app_paths.rs。
  - `prepare`：confirmed+当前权威 revision（拒绝在途 amendment）+同源权威绑定冻结 graph；
    同 plan/revision 第二 command 返回**原 run**（原 command/selection）；异
    revision（`coding_plan_run_revision`）/graph 漂移（`coding_plan_run_graph`）拒绝且不写；
    Paused/Invalidated run 不再返回 active（`coding_plan_run_not_active`，无旧数据补授权）。
  - `bind_ready`：首次绑定逐 attempt 核 plan 归属+target 桶+**未首启（Created）**，
    已有 Running/等待/阻断/失败/终态集合拒绝（`coding_plan_run_set_not_first_start`，不收养）；
    无 ready 仍 durable 绑定全集、selection=None 保持 `ready_bound` 等待，ready 出现后
    可一次补冻结；selection 校验：attempt∈绑定集、target 一致、start_command_id 必须
    等于确定性派生值、unit 在该 attempt 且未进终态（桶内首 unit 初始化即 Running 是物化
    事实，与 N3 DTO execution_state 同语义，可选；Completed/Failed/Superseded/Skipped 拒选）。
    重放必须命中同一冻结集合/selection，漂移拒绝；Invalidated/Paused 拒绝。
  - `mark_start_requested`：必须已有冻结 selection（`coding_plan_run_selection_missing`
    不能发空首启）；之后幂等重放。`record_start_outcome`：回执冻结后不漂移。
  - `get`/`matching_first_ready_run`：身份错配与 IO/JSON 错误上抛（IdentityMismatch/
    Ambiguous），不当 None；`mark_paused` 幂等补记（reason=
    `plan_coding_continuation_not_enabled`）。
  - 删除接口：`invalidate_for_deleted_attempt`（成员级：target_attempts 或 selection
    命中该 attempt 即 Invalidated；幂等；无匹配 run 直接 OK，不自动补建）、
    `purge_plan`（核归属；缺失幂等 OK；删除窗口清 run 文件+前导点锁）、
    `purge_plan_with_lease`（scope 必须 Plan）。
- **Modify `group.rs`**：`pause_completed_plan_first_unit(attempt, completed_unit_id)`
  ——匹配「本 plan first_ready run + selection 指向本 attempt + completed_unit_id 恰为
  选中首项 + unit Completed 且最新权威 unit run Completed + handoff 在场」时，attempt
  文件锁内原子写 Blocked+CodeReview/清 active/current，幂等建空 actions blocked gate
  （reason=`plan_coding_continuation_not_enabled`）；锁释放后幂等补 run Paused
  （attempt 单文件是停止事实，Paused 回写中断可按 matching completed selection 恢复）。
  不匹配返回 None（旧 advance 路径零影响）；匹配但完成证据缺失按坏数据拒绝；
  非 Running 且非已暂停形状拒绝（`coding_plan_pause_attempt_status`）。锁序：绝不持
  plan run 锁取 attempt 锁。
- **Modify `group_validation.rs`（N1 交接面，仅 pause 合法性）**：
  `validate_group_attempt_pointers` 新增 `plan_coding_pause_allowed` 例外——仅当
  Blocked+CodeReview+空指针+恰一个 Completed unit，且 `matching_first_ready_run` 命中
  （attempt_id/unit_id/logical_work_item_id 三重严格匹配冻结 selection；Invalidated
  run 不构成例外）才放行"无 active 指针”；其余（含无 run 的同形状）维持
  `attempt has no legal active or resume target` 失败关闭。完整性/单 target/
  桶收窄语义零变化。
- **Modify `gate.rs`**：`ensure_provider_run_allowed` 末段新增授权边界——匹配 run 的
  phase=Paused/Invalidated 直接拒（`plan_coding_paused`/`plan_coding_invalidated`）；
  冻结 selected unit 已 Completed 而 pause 未写回的中断窗口拒
  （`plan_coding_first_unit_completed`）。普通 retry/StartCoding/gate response 不能
  绕行安全暂停。
- **Modify `product/mod.rs`**：仅挂载 `pub mod coding_plan_run;`。

## 证据（cargo test --locked --lib，红→绿→回归）

| 命令 | 红（行为改动前） | 绿（rustfmt 后复验同） |
|---|---|---|
| `plan_coding_n2_` | 首轮全红暴露 selection 准入误设 Pending（桶内首 unit 物化即 Running，修正为仅拒终态 unit）；修正后 5 绿 + `plan_coding_n2_pause_pointer_and_provider_admission` 业务红：中断窗口 `ensure_provider_run_allowed` 返回 Ok（provider 放行）——归因准入行为缺失 | 6 passed |
| `coding_plan_repair_group_attempt_missing_active_pointer`（计划指定坏指针回归） | — | 1 passed |
| `group_split_target` / `sc_group_dependency_gate` / `group_uniqueness` / `plan_coding_n1_` | — | 11 / 9 / 2 / 3 passed |
| `group_terminal` / `plan_amendment` | — | 11 / 54 passed |
| `advance_split_targets` / `advance_split_recovery_matrix`（N4 面，只读回归） | — | 8 / 10 passed |
| `coding_attempt_store`（store 全族） | — | 198 passed |
| `provider_execution_context` / `plan_defect_entrypoints` / `coding_start_claim` | — | 3 / 8 / 3 passed |

新增内联测试（`coding_plan_run.rs`，夹具同 N1/split 路线：PlanRepair 种子 + 两 logical
仓 + per-unit target 草稿 + Manual advance 到 durable Ready）：

- `plan_coding_n2_command_replay_freezes_identity`：同 command/revision 重放原 run；
  异 revision 拒绝不改写；graph 篡改后 prepare 拒 `coding_plan_run_graph` 且不复活。
- `plan_coding_n2_second_command_returns_active_run`：第二 command 返回原 run（原
  command/selection/start_command_id 派生不变；仍只有一个 run 文件）。
- `plan_coding_n2_identity_drift_and_existing_started_set_rejected`：异 revision 不落
  文件；web attempt 播 Running 后首次绑定拒 `set_not_first_start` 且不 durable 收养；
  target 缩水/selection 改选均拒且冻结集不变。
- `plan_coding_n2_readybound_restart_keeps_selection`：无 ready 等待（selection=None
  durable 全集）→ 空 selection 拒 mark_start_requested → 重启保留 → ready 补冻结 →
  再重启 selection/start_command_id 保留 → start_requested 幂等重放。
- `plan_coding_n2_pause_pointer_and_provider_admission`：Started 回执后补全完成事实
  （unit run Completed+handoff+completion_commit+unit Completed）；清指针后 pause 写回前
  `ensure_provider_run_allowed` 拒 `plan_coding_first_unit_completed`；pause 写
  Blocked+CodeReview/空指针/恰一空 actions gate/run Paused；integrity 认可合法分支；
  provider 拒 `plan_coding_paused`；重复 pause 幂等（gate 仍恰 1）；purge run 后同形状
  回到 `attempt has no legal active or resume target` 失败关闭（普通坏指针仍拒）。
- `plan_coding_n2_late_write_and_deleted_member_invalidates`：删 web 成员→run
  Invalidated（冻结集/selection 保留不抹除）；迟到 mark_start_requested/
  record_start_outcome/bind_ready/prepare（同/新 command）全部 Conflict 不复活；
  幂等 invalidate；无匹配 run 不自动补建；purge 清本 plan run+前导点锁、保留他 plan
  run；缺失幂等 OK；对缺失 run 的迟到写上抛 NotFound（不当 None 吞）；
  `purge_plan_with_lease` Plan scope 通过、Story scope 拒
  `coding_plan_run_purge_scope`。

## 边界与移交

- 新增授权记录不是第二 claim 账本：AdvanceRecord/start_claim 各自保持权威，run 只冻结
  显式授权/集合/selection/暂停事实；现 worktree/snapshot/claim 零修改。
- 锁序声明：plan run 锁（`.{plan_id}.json.lock`）与 attempt 锁不嵌套取得；
  `pause_completed_plan_first_unit` 先无锁只读匹配 → attempt 锁内写 attempt/gate →
  释放后补 run Paused。
- **N3 消费**：`prepare`→`bind_ready`（Ready 集互证后带确定性 selection 或 None 等待）→
  `mark_start_requested`→现 Manual `start_coding_once`（用冻结
  `selection.start_command_id`）→`record_start_outcome`；`AdvanceInput.command_id`
  用 prepare 返回的 run.command_id。`CodingPlanSelection::deterministic_start_command_id`
  是首启 command 唯一派生入口。
- **N5 消费**：完成事实落定后调 `pause_completed_plan_first_unit`（幂等可重入，覆盖
  Paused 回写中断窗口）；runner 侧以 `matching_first_ready_run`+phase=Paused 识别真实
  暂停（而非任意 Blocked 文本）。
- **N7/D2.2 消费**：member 删除接 `invalidate_for_deleted_attempt`；现 plan 删除闭包接
  `purge_plan`；D2.2 持 Plan lease 接 `purge_plan_with_lease`。run 磁盘地址/锁名见上，
  已按计划登记 D2.2 闭包。
- S/P 包（跨 attempt 串行 resume、plan 终结、并行）不在本片；暂停后的恢复授权属 S2。
