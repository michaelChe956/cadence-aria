# WP5 恢复矩阵留档（T5 / REQ-MTG-05，ter A-WP5 验收口径）

- **change**: `openspec/changes/multi-repo-group-coding/`
- **计划**: `cadence/plans/2026-09-19_计划文档_阶段4-C2_多仓group编码_v1.0.md` Task 5 Step 2
- **矩阵定义**（k3 双审定案）：`AdvanceInitializationFailpoint` 七 checkpoint
  （RecordPersisted / JournalPrepared / GroupAttemptPersisted / AttemptPersisted /
  WorktreeBound / PlanBindingSaved / UnitsMaterialized）× target 形态
  （单 target 回归零变化 / 多 target 每 attempt 独立等价单 target 语义），
  R4 组合爆炸缓解=2-target 全阶段全量+3-target 抽样两中断点
  （WorktreeBound/UnitsMaterialized）。

## 一、矩阵总览（红→绿）

| 组合 | 用例 | 状态 |
|---|---|---|
| 单 target × 7 checkpoint（Crash） | `workspace_engine::tests::advance_handler::advance_initialization_replay_resumes_same_record_attempt_and_units`（T5 扩展第 7 格 GroupAttemptPersisted+审计零断言） | 绿 |
| 2-target × 7 checkpoint（Crash） | `workspace_engine::tests::advance_split_recovery_matrix::split_recovery_matrix_all_checkpoints_resume_same_attempt_set` | 绿（红=split_audit API 缺失编译红 11 errors） |
| 3-target × {WorktreeBound, UnitsMaterialized}（Crash，抽样） | `…::split_recovery_matrix_three_targets_samples_two_checkpoints` | 绿（红=首跑暴露种子依赖边跨桶，夹具依赖图手术修正后绿） |
| 半启动（Coding+已物化） | `…::split_semi_started_resume_backfills_head_per_attempt_only` | 绿 |
| 半启动（Coding+未物化） | `…::split_semi_started_unmaterialized_demotes_per_attempt_only` | 绿 |
| 断连重连（attach resume 门） | `web::coding_ws_handler::socket::resumption::tests::sc_advance_restart_gate_accepts_split_target_attempt_set_membership` | 绿 |
| 审计族（创建留痕/检索/幂等/无编排） | `split_advance_records_proliferation_audit_for_every_target_attempt` / `split_audit_retrieval_resolves_per_plan_target_and_filters_foreign_plans` / `split_audit_store_write_is_idempotent_first_writer_wins` | 绿 |

## 二、逐格明细

### 单 target 列（回归零变化——七 checkpoint 全量）

用例循环体逐格断言 durable 中断态 → 重启恢复 → 同 attempt 同 units → replay Replayed →
**不落任何 split-audit 记录**（增殖审计为分流专属，单 target 零变化锁）。

| checkpoint | durable 中断态（outer journal / group journal / attempt 文件） | 恢复结果 |
|---|---|---|
| RecordPersisted | none / none / 无 | 同 record 同 attempt 恢复 Completed；attempts=1 |
| JournalPrepared | JournalPrepared / Prepared / **无**（attempt 文件未落盘） | 同上 |
| GroupAttemptPersisted（T5 新增格） | JournalPrepared / Prepared / **在场**（ensure 已写、相位未推进） | 同上（ensure 幂等重放收养既有 attempt 文件） |
| AttemptPersisted | JournalPrepared / AttemptPersisted / 在场 | 同上 |
| WorktreeBound | AttemptPersisted / WorktreeBound / 在场 | 同上 |
| PlanBindingSaved | PlanBindingSaved / PlanBindingSaved / 在场 | 同上 |
| UnitsMaterialized | UnitsMaterialized / UnitsMaterialized / 在场 | 同上 |

### 多 target 列（2-target 七 checkpoint 全量——每 attempt 独立等价）

| checkpoint | durable 中断态（per-target journals / attempts / 审计） | 恢复结果（同套 attempt 集断言） |
|---|---|---|
| RecordPersisted | 0 / 0 / 0（分支判定前中断） | 重放新建同套 2 attempts → Completed；审计 2 条补齐 |
| JournalPrepared | 2×Prepared / 0 / 2（审计随 journal 创建即落） | 同套 2 attempts 恢复；审计 created_at 不漂移 |
| GroupAttemptPersisted | 2×Prepared / 1 / 2（首个 target attempt 已落盘、相位未推进） | 同套恢复（ensure 幂等收养+第二 target 续建） |
| AttemptPersisted | 1×AttemptPersisted+1×Prepared / 1 / 2 | 同上 |
| WorktreeBound | 2×WorktreeBound / 2 / 2（repo 维三件套全登记） | 同上 |
| PlanBindingSaved | 2×PlanBindingSaved / 2 / 2 | 同上 |
| UnitsMaterialized | 2×UnitsMaterialized / 2 / 2 | 同上 |

每格恢复后统一断言：per-target journal 全 Completed；attempt 集=pre-crash 集
（RecordPersisted 格为新建集，无增殖）；record Ready 且 `target_attempts`=全集；
审计 2 条（身份=最终集、trigger=advance+command_id、created_at 与 pre-crash 一致
——**不重写不漂移**）；新 command_id replay → Replayed 且审计不新增。

### 3-target 抽样（R4）

`WorktreeBound`/`UnitsMaterialized` 两格同套 3 attempts 恢复+审计 3 条（第三仓
`split-extra-3` 独立审计在案）。夹具注记：种子依赖图 `wi_core→wi_registration` 边
在「每桶恰一 unit」形态下构成跨桶依赖——一期语义显式 fail-closed（无跨 attempt
编排），夹具将两侧 edge 清单一致改写为空（合法 plan 形态），见
`clear_split_dependency_edges`。

### 半启动（Coding 态，T2 #10 深度恢复移交项）

- **已物化**：attempt A（Running/Coding/head 缺失，真实 git worktree 在场）→
  `prepare_resumed_attempt_for_runner` 从**自身** worktree 补 head；兄弟 attempt B
  保持 (Created, PrepareContext)，resume 恒等返回——per-attempt 独立等价。
- **未物化**：worktree 路径目录不存在 → 回落 WorktreePrepare（物化归 runner 管道
  既有分支）；兄弟 attempt 零牵连。

### 断连重连（attach resume 路径）

`sc_advance_restart_blocked` 门对多 target Ready record 的集绑定：
`target_attempts` 集内两 attempt 均放行（per-attempt 判定=精确匹配或集合包含，
T2 Batch 1 判定函数自动生效）；集外 attempt fail-closed
（`sc_advance_restart_not_durable_ready`）。引擎面同款
（`advance_is_ready_for_attempt` 集包含）由 T2 `advance_ready_judgment_*` 三测试锁定。

## 三、审计面矩阵联动（5.1）

| 计划项 | 测试 |
|---|---|
| #1 分流创建留审计（全字段） | `split_advance_records_proliferation_audit_for_every_target_attempt`（id=attempt_id/project/issue/plan/target 唯一/bound_plan_revision_id=authoritative/dependency_graph_revision_id=authoritative/trigger.kind=advance/trigger.command_id=AdvanceInput 透传/created_at） |
| #2 恢复后可追溯（不重写不漂移） | 矩阵主格逐 checkpoint created_at 断言+`split_audit_store_write_is_idempotent_first_writer_wins`（首写定档：同身份异 command/created_at 不重写；同 attempt 异 target 冲突 fail-closed） |
| #3 检索 per-(plan,target) | `split_audit_retrieval_resolves_per_plan_target_and_filters_foreign_plans`（N targets N 条每 target 唯一；异 plan 过滤；空 plan 空集） |
| #4 审计不承载编排（行为面） | 创建留痕测试内联断言（审计落盘后 attempts 全停 (Created, PrepareContext)）+编译面 grep（split_audit.rs 零调度调用——start/spawn/queue/schedule/transition 均无；advance.rs 命中项均为既有失败标记与初始化推进，非审计面） |

## 四、执行记录

- 主树定向：`advance_split*` 15/15（新 7+既有 8）；`advance_initialization_replay_resumes_same_record_attempt_and_units` 绿（7 格）；
  `workspace_engine::` 全量 1189/1189；`coding_attempt_store` 179/179；`coding_ws_handler` 112/112；it_web `mixed_target_group` 3/3（漂移稳定码契约锚）。
- 隔离树终验（干净 HEAD=`023f5892`+仅本任务 patch+独立 target dir+拷 web/dist）：见 `stage4-c2-t5-report.md` §终验。
