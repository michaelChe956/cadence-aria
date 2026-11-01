# S2a 续跑全自动化——入口与相位(实施报告)

- 日期:2026-10-14
- 工位:S2a(A 半:plan_start.rs 续跑入口 + N2 store 相位 + plan_coding_continuation 语义反转 + error 映射)
- 基线:HEAD 031c7435(worktree `.worktrees/feat-b-0808-add-monorepo`)
- 姊妹工位:S2b(B 半:N5 移除/engine 完成链/gates/runner,并行落码,共享工作树)

## 交付语义(owner 冻结口径)

1. **续跑入口**:POST `/plan-coding-start` 在回执相位(Started/AlreadyStarted/NeedsHuman)上的再点击,按全局就绪集(就绪=直接依赖全部 Completed 且未启动)重估推进;就绪集>1 并行 start。门条件:冻结绑定集内已存在终态 unit(完成/失败)才推进——纯重放/首启窗口零新启(n3 族单选首启冻结不破)。
2. **Paused 语义反转**:`prepare` 不再对 Paused run 报 409 `coding_plan_run_not_active`;入口 `resume_from_pause` 回首启回执相位+清 pause_reason 后正常续跑。`plan_coding_continuation_not_enabled` 码保留,正常推进链零产出(仅 legacy Paused 数据可见,且入口 resume 即清)。
3. **失败语义**:unit 依赖未 Completed(含 Failed)→依赖链不启动(保持 waiting_dependency 事实,非天书文案);独立链继续启动。
4. **幂等重试**:已启动 attempt 跳过(claim 确定性 command=deterministic_start_command_id(run.command_id,target),claim/registry 单发幂等);再点零新 claim/零二次 barrier,回 Replayed。
5. **plan_complete**:聚合判定不变(全部 unit Completed)。

## 跨 attempt 自动推进桥(编排器引擎种子)

`pub(crate) async fn drive_ready_units_for_plan(state, project, issue, plan) -> Result<PlanContinuationDriveOutcome, ApiError>`(plan_start.rs):
- **S2b 挂点契约**:runner 完成链在 completion commit 落定后调用;调用方自持 per-plan 串行键 `work_item_group:{project}:{issue}:{plan}`(函数不自取,防入口重入死锁)。
- 就绪判定粒度=单 unit(单 plan 作用域,无跨 plan 全局串行假设,可多 plan 并行)。
- Created attempt:桶根(order_index 0,建组物化)全局就绪→`start_coding_once`(Manual origin)。
- Running 无 active unit(跨桶 parked,桶内 SC 门只见本桶 units、桶外上游恒 Waiting):物化确定性首个就绪 Pending unit(update_coding_unit_status→Running+指针对齐、stage 回 PrepareContext、worktree 锁转移同 advance_to_next_group_unit 源)+`spawn_coding_runner` 重启。
- 忙(在册 runner/active unit)/人控/终态:跳过;单 attempt 启动 NeedsHuman 不阻断其余链。
- selection 轮换:每轮轮换到本轮新启 unit 中(拓扑位→logical→target)最小者(store 新原语 `rotate_selection`,仅回执相位可轮换,合法性校验与首启同源)。
- 导出链:`plan_start.rs → coding.rs → handlers/mod.rs`(crate::web::handlers::drive_ready_units_for_plan)。

## Store 改动(coding_plan_run.rs)

- `prepare`:仅 Invalidated 拒绝;Paused 原样返回(交入口 resume)。
- `resume_from_pause`(新):Paused→回执相位+清 pause_reason;幂等;无回执 Paused fail-closed;Invalidated 拒。
- `rotate_selection`(新):回执相位限定;同值幂等;漂移(异 unit/attempt/command)拒。
- `mark_paused`/`PLAN_CODING_CONTINUATION_NOT_ENABLED` 原样保留(legacy 面)。
- 测试迁移(P5 撤除对齐,原语调用面清零):`plan_coding_n2_pause_pointer_and_provider_admission` 重写为「完成清指针窗口 provider 准入放行+run 保持 Started 零暂停事实」;`plan_coding_n2_pause_rejects_invalidated_run` 整测替换为 `plan_coding_n2_resume_and_selection_rotation_phases`(resume/rotate 相位语义+Invalidated 双拒绝)。

## 契约边界登记(REQ-MTG-03)

`openspec/specs/multi-target-group-coding/spec.md` 将「跨 target-attempt 依赖就绪自动编排」列为二期(owner 已口头裁定提前实施)。本桥(drive_ready_units_for_plan+S2b 完成链挂点)即该能力的一期落地;归档时按此更新契约条目。

## TDD 记录

红(plan_start_tests.rs 新增 s2 族,基线全失败形态与预期一致):
- ①`plan_coding_s2_continuation_starts_all_ready_units_in_parallel`:wi_core 完成后新点击→{wi_registration,wi_unrelated}并行 start(原:Replayed 零新启)。
- ②`plan_coding_s2_continuation_retry_is_idempotent`:再点零新 claim/零二次 barrier/selection 不漂移。
- ③`plan_coding_s2_failed_unit_stops_dependents_and_starts_independent_chain`:wi_core 失败→wi_registration 不启动(waiting_dependency 事实)、wi_unrelated 启动(原:Replayed)。
- ④`plan_coding_s2_paused_run_resumes_and_drives_instead_of_409`:legacy Paused resume 续跑(原:409 coding_plan_run_not_active)。

绿:S2 族 10 测试(A 面 4+B 面 6)全过;`plan_coding_` 全族(n2/n3/h4/n5/s2)复跑通过。

## 验证命令

- `cargo test --lib plan_coding_s2`(10 passed)
- `cargo test --lib plan_coding_`(全族)
- `cargo check --lib`(0 error)

## 遗留与关注

- **h4 套件并行竞态(非本面回归)**:`plan_coding_h4_group_delete_end_to_end_then_new_round` 在全量并行下偶发 `git worktree remove` 撞 `worktree add` 的 "initializing" 锁(runner 物化与 teardown 竞速,既有窗口被新增测试负载放大);单跑/族跑均绿。建议后续在 teardown 前 wait-for-runner-settle(runner 面,归 B 域)。
- 入口对 Created attempt 开桶以「桶根就绪」为条件;若桶根被跨桶依赖挡住而桶内更深层 unit 就绪(建组拓扑序下不可达的理论形态),该桶等待桶根——与 N1 桶内推进语义一致。
- selection 轮换是可观察性投影,非第二调度账本;启动事实权威仍为 attempt claim/runner registry。

## P1 修复轮(2026-10-14 审查派回)

- **缺陷**(plan_start.rs parked 续跑):锁转移 from 取自推进后重读 attempt——`update_coding_unit_status` 已把指针权同步为新 unit,from==to 自转移;真实锁记录 active 仍指上一完成 unit→transfer 必败 `repo/issue_worktree_active`→入口报 `continuation_resume_failed`/挂点 warn 吞错静默停半程(引擎 group.rs advance 同款缺陷的入口面镜像)。
- **修法**:from 取**推进前**内存 attempt 的 `current_work_item_id`(park 形状=上一完成 unit=真实锁记录 active 端),缺省回落桶根 `work_item_id`——与引擎 `advance_to_next_group_unit` 先例同源。
- **TDD**:红=`plan_coding_s2_parked_resume_transfers_lock_from_pre_advance_identity`(跨桶 parked 分支此前全库零覆盖:frontend 桶 Running 无 active unit+wi_service 完成+wi_panel 跨桶就绪+repo 锁记录 owner=attempt/active=wi_service→旧码 transfer 必败);绿=锁 active 落 wi_panel、owner 保持、新 unit 物化+runner 重启+selection 轮换。
- **顺带清理**:死码 `plan_coding_first_unit_completed` 三处(error.rs 状态码表/测试表+product_store_error.inc.rs Io 映射)删除。

## 登记项(归档轮处理,owner 知悉)

- **REQ-MTG-03 契约改写归归档轮**:`openspec/specs/multi-target-group-coding/spec.md` 的「跨 target-attempt 依赖就绪自动编排列为二期」红线由本次实施突破(S2a 桥+S2b 完成链挂点即该能力落地),spec 条目改写在归档轮统一执行,不在本修复轮。
- **legacy 暂停 attempt 形状残留**:历史 N5 暂停数据留下的 Blocked+CodeReview+空 actions gate attempt(生产新数据不再产生;入口 resume 只恢复 run 相位,attempt 形状不自动解除)——该残留当前仅可经 H4 整轮删除重建恢复;是否补显式解除动作由 owner 另行裁定。
