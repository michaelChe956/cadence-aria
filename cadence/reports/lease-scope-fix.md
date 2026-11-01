# 热修报告:租约分类收窄 attempt 作用域(多仓启动误杀)

- 日期:2026-10-14
- 分支/工作树:`feat-b-0808-add-monorepo`(`.worktrees/feat-b-0808-add-monorepo`)
- 关联诊断:`local://diag-coding-start-race.md`(scout)、design-task-sol 定案(四字段)
- 提交:`fix: 租约分类收窄attempt作用域(多仓启动误杀)`

## 0. 事故与修法一句话

多仓 plan 一键启动时,issue 级单飞闸 `classify_worktree_lease(project, issue)` 跨仓扫描、按 UUID 升序返回**首个有 owner 的仓记录**,把兄弟仓尚未启动 attempt 的合法预绑定锁误判为「另一场 run 正在运行」→ 首项 runner 在 Coding 阶段 admission 被判 `coding_run_already_running` → runner 判死 AMR。修复 = 租约判定按 attempt 自身路由(target_snapshot→仓维;Legacy→旧 issue 键)收窄,兄弟仓合法预绑定不构成冲突,同仓异主真冲突语义保留。

## 1. 字段一:run_exclusion.rs(scoped 分类入口 + admission 两处换接)

- 新增 `CodingAttemptStore::classify_worktree_lease_for_attempt(&attempt)`(run_exclusion.rs):
  - `target_snapshot = Some` → 只分类 `get_repo_shared_worktree(project, issue, snapshot.logical_repository_id)` 单条记录;记录缺失 → 等价「worktree record not found」无租约事实(不拦,与 legacy 未启锁同语义);**旧 issue 维记录混在场 → fail-closed(`legacy_shared_worktree_present`,对齐 §4.2.6 preflight)**。
  - 无 snapshot + 路由 Legacy → **只读旧 issue 维记录**(`get_issue_shared_worktree`),不落入原 283-317 的仓扫描回退;仓维记录混在场 → fail-closed(`repo_shared_worktree_present_for_legacy_route`)。
  - 无 snapshot + 路由 Logical → fail-closed(`target_snapshot_missing_for_logical`,对齐 gates.rs:132-134);FailClosed 路由 → `target_snapshot_identity_drifted`;routing 加载失败 → Unknown 停等。
  - 单条记录分类复用既有 `classify_worktree_record`(owner 活跃/死亡/瞬态三态语义原样)。
- `admit_coding_run_exclusive` 两处换接:首次分类(原 ②)与 NeedsHuman 停等重放(原 ① 内)均改调 scoped 入口。**账本语义原样保留**:Accepted 重放跳过租约再分类直接放行(未触碰);NeedsHuman 重放沿用「以当前租约证据重放同类停等」,仅判定来源换 scoped。
- issue 级 `classify_worktree_lease` 保留为显式聚合观察 API(confirm_takeover / plan_confirmed_info 投影 / engine 委托原样)。

## 2. 字段二:gates_parts/worktree_lock.inc.rs(恢复锁复位换接)

`ensure_issue_worktree_lock_for_resumed_attempt` 的租约三态判定改调 scoped 入口(原 issue 级 classify;该函数 63 行本已算出 route 却仍调 issue 级,半截路由化)。三态处置逻辑不变:自持活跃原样继续;自持死亡残留/已释放 → 按路由释放+重绑;活跃他人停等(`coding_run_already_running`);死亡他人指向确认接管面;未知 fail-closed。**修复前该面是 AMR 恢复死循环点**(RecoverCoding ①锁复位即撞兄弟仓锁报错);修复后 WI-001 型 AMR 自持残留可走 释放+重绑 → CAS 回 Running → respawn。

## 3. 字段三:web/coding_start.rs(restart 预检换接)

`restart_coding_attempt` 租约非活跃预检改调 scoped 入口:活跃**本仓**异主才停等;兄弟仓预绑定不再误拦。后续 registry/`restart_terminal_attempt_for_execution`/G5 锁复位链路未动。

## 4. 字段四:TDD 验收(红→绿→回归)

### 红(桩复现事故:scoped 入口先委托 issue 级 classify,四调用点已换接)

- `multi_repo_admission_ignores_sibling_repo_prebound_lock`:兄弟仓锁在场首项 → 误判 `AlreadyRunning`(事故复现)❌
- `resume_multi_repo_amr_self_held_residue_rebinds_own_repo_and_keeps_sibling_lock`:`coding_run_already_running: lease coding_attempt_…`(事故错误原文)❌
- `multi_repo_scoped_classification_fails_closed_on_missing_snapshot_or_mixed_records`:旧分类回 `DeadNeedsTakeover` 而非 fail-closed ❌
- 语义保持测试(同仓冲突/死亡/未知)全绿——确认桩与旧行为等价。

### 绿(scoped 实现落地)

| 测试面 | 用例 | 结果 |
|---|---|---|
| admission_lease_tests.rs | `multi_repo_admission_ignores_sibling_repo_prebound_lock`(异仓预绑定不拦、兄弟锁事实保留、重放幂等) | ✅ |
| 同上 | `multi_repo_admission_same_repo_foreign_active_holder_is_already_running`(同仓异主活跃 AlreadyRunning;账本 NeedsHuman 恰一条;重放同结论;异 payload Conflict;不抢锁) | ✅ |
| 同上 | `multi_repo_admission_same_repo_dead_or_unknown_holder_never_preempts`(死亡→TakeoverRequired;瞬态 owner→LeaseUnknown;均不抢) | ✅ |
| 同上 | `multi_repo_scoped_classification_fails_closed_on_missing_snapshot_or_mixed_records`(Logical 缺 snapshot→`target_snapshot_missing_for_logical`;legacy 混在→`legacy_shared_worktree_present`) | ✅ |
| 同上 | 既有 36 项(legacy 单仓零回归) | ✅ |
| resume_worktree_lock.rs | `resume_multi_repo_amr_self_held_residue_rebinds_own_repo_and_keeps_sibling_lock`(多仓 AMR 自持:本仓释放+重绑、兄弟锁不变) | ✅ |
| 同上 | 既有 6 项 Legacy 回归 | ✅ |
| coding_start_parts/tests.inc.rs | `restart_multi_repo_attempt_passes_precheck_despite_sibling_prebound_lock`(真实工件链:project/manifest/member/checkout/repos.json/policy;Accepted;兄弟锁不变;本仓重绑) | ✅ |
| plan_start_tests.rs | `plan_coding_n3_one_click_binds_all_and_starts_selected_once` **补受控 provider 进入断言**:轮询至 Coding 阶段 coder role run 存在(durable provider 进入事实,穿过真实 admission;不以 Accepted/claim barrier/runner_count 代替),期间不得落 AwaitingManualRecovery | ✅ |
| 同上 | N3 族 7 项(含 replay/restart/get_aggregation/old_post_shape/claimed_barrier/deleted_member) | ✅ |

### 回归

- `cargo test` 全量(基线 4624/0):见下方「验证」最新一次结果。

## 5. 不动面(设计冻结遵守)

durable 格式/spawn/selection/初始化锁时机/`Created.is_active`/advance 面/worktree.rs/级联面(confirm_takeover、plan_confirmed_info 投影)均未触碰;Busy 状态机(真 busy 转 WaitingForHuman)不属本包;旧 issue classifier 保留聚合观察用途。

## 6. 验证(实测命令与结果)
- `cargo test --lib`(基线口径 4624/0)→ **4630 passed / 0 failed / 7 ignored**(+6 恰为本包新增测试;首轮曾现 2 例 automation/coding_info 负载 flaky,隔离复跑均绿,后续两轮全量 lib 均 0 failed)。
- `cargo test --no-fail-fast` 其余套件现存失败与本包无关(均已核实归属):
  - `it_core::large_file_guard`(24 文件超 1200 行,全部为已提交内容,含本包前即超限的 coding_plan_run.rs/advance_store.rs 等;本包最大改动文件 run_exclusion.rs 520 行);
  - `it_core::workspace_ws_integration` WS 3s 超时(工作树零 diff 子系统,负载相关);
  - `it_web::issue_delivery_summary::multi_repo_*`(provider_gateway capability `provider_capability_launch_not_confirmed`——LC gateway capability 域,兄弟工作流在途;本修复使其失败点从 admission 误杀后移至 capability 门禁);
  - `it_product::ambiguous_push_*`(纯 legacy 单仓推送外部远端轮询,与本包 Legacy 分支行为恒等)。
- `cargo test --lib coding_attempt_store::admission` → 40 passed / 0 failed
- `cargo test --lib resume_worktree_lock` → 7 passed / 0 failed
- `cargo test --lib restart_multi_repo` / `resume_multi_repo` → 1 passed / 0 failed 各
- `cargo test --lib plan_coding_n3` → 7 passed / 0 failed
- 全量其余套件归属明细见上方第 3-7 条目。

## 7. 撞面记录

`src/web/handlers/coding/plan_start_tests.rs` 与 FixDashDeps(Slice B 仪表盘跨仓依赖投影:types.rs DTO 增三字段 + get_aggregation 断言)additive 交集、不同函数;双方不触对方行,已通报 Main 协调。本包提交对 plan_start_tests.rs 仅选择性暂存本包 hunks(导入块 + one_click 断言块),不携带 Slice B 改动。
