# 批2 Task 21 报告:取消/崩溃启动补偿与人工恢复续接

- **状态**: DONE(待独立审查)
- **分支/worktree**: `feat-b-0808-add-monorepo` @ `.worktrees/feat-b-0808-add-monorepo`
- **Commit**: 见回执 — `fix(review): 启动先补偿窗口再放行编排`
- **测试**: 红测确认(`startup_scan_compensates_open_window_before_resuming_runner` 于实现前 HEAD 上 FAIL:「启动分诊前必须先补偿窗口到终态,当前状态 running(旧序:runner 先于窗恢复)」);绿:`cargo test --locked --lib -- startup_window_compensation reviewer_window_store git_operation_reconcile` → **34 passed / 0 failed**;受影响集成面 `-- runner_recovery reviewer_window reviewer_reconciliation reviewer_snapshot git_operation_reconcile failed_review_recovery provider_failure_recovery code_review_triage internal_pr_review retry_push plan_repair restart runner_cleanup sc_start_guard` → **549 passed / 1 failed**,唯一红 = `plan_repair_cancel_replay_requires_authoritative_terminal_state`(task-1/T19/T20 登记的 part_26 存量基线红「T2 粘滞守卫」,非本 Task 引入);clippy 本面文件 0 命中(残留 `set_readonly` 命中为存量,不在本 Task diff);rustfmt 本面已跑。

## 交付面

### 1. 启动补偿驱动(engine `code_review.rs`,`compensate_open_reviewer_windows_at_startup`)

- 枚举全部未终态窗口(`ReviewerWindowStore::list_windows` 新增,损坏记录跳过告警不停扫)逐一补偿:**停旧登记资源且确认静止**(窗口自身 `quiesced_scopes` + 资源 store 中 `aria.worktree==tree_key` 的兄弟 scope;`with_worktree_key` 注入的归属标签;先静止证明再 cleanup,killpg/Drop 不当成功证据)→ 按崩溃前缀分派:Preparing/无 baseline → 弃置释放(不重采);Running → begin_quiesce → 恢复;Quiescing/ViolationRecorded/Restoring/RecoveryRequired(带 baseline)→ `restore_window` 按**原 manifest** 续恢复(T16 游标幂等,不重冻/不重采污染基线);恢复完整且无违规 → 弃置释放(run 已死,无报告可采);有违规 → ManualGate 保持锁;恢复失败 → `RecoveryRequired` 已由 restore_window 持久化(锁保留)。
- **先持久新发现违规**:消费 `restore_window` 内建 T18 journal-first 链(恢复前观察+派生事实先落 journal 再迁移)。
- **未知 writer/静止不可证** → `park_window_recovery`(新 pub(crate),RecoveryRequired,锁/冻结保留)。
- **局部坏窗其它无冲突树继续**:单窗失败记入结局不停扫;该树经租约闸门持续 fail-closed。
- **人工门投影**:窗口保持锁时 attempt 活跃态 → `AwaitingManualRecovery`(幂等,稳定原因码 `reviewer_window_manual_gate`/`reviewer_window_recovery_required`),启动分诊/attach 恢复据此零 runner,人工恢复续接为唯一出口。
- **释放后 Git 补偿续接**(`resume_pending_review_compensation`):复用 T19 lock order——仅窗口终态释放后,对取消已决定的 attempt(非 Running/Created)续接被冻结拒绝的 ReviewRequest 补偿;失败仅告警(journal durable 保守)。

### 2. 启动次序修正(`serve_web` + `reconcile_claimed_coding_runs_once` 前置)

- `reconcile_claimed_coding_runs_once` 内嵌补偿前置(先窗口补偿再分诊);`serve_web` 将该调用移到 Provider 探活/编排器启动之前——不再沿「仅日志后继续启动同树 runner」旧序。
- `recover_claimed_attempt_at_startup` 增加树上未终态窗口闸门(`ensure_no_open_reviewer_window`):开窗树零 runner + attempt 投影人工恢复面;其余树继续。

### 3. 人工恢复续接与人工门生产面

- **socket `RecoverCoding` 前置**(socket.rs 手工恢复面):`redrive_reviewer_window_recovery_for_attempt` 先驱动补偿;ManualGate 需显式裁决(redrive 不代作废),未收口即 `coding_recover_reviewer_window_open` 拒绝——不得带窗重启 runner。
- **REST 生产入口**:`POST /api/coding-attempts/{attempt_id}/reviewer-window:resolve`(decision ∈ invalidate|require_new_review)→ `resolve_reviewer_manual_gate_for_attempt`(engine);`ApiError` 映射补 `reviewer_window_decision_invalid`→400。
- **人工拒绝补偿(Invalidate)不解锁 attempt**:窗口按 T17 语义关闭释放树,但 attempt 保持 AwaitingManualRecovery(无自动 resume),旧报告永久 Invalidated(`recheck_review_acceptance_for_delivery` 同源),启动分诊零恢复。
- **人工许可新轮(RequireNewReview)另取租约/新基线**:NewReviewRequired 终态;新 run 新窗(新 lease_id/新 manifest),不 resume 旧违规报告。

### 4. attempt 删除/终止 → reviewer-windows 收口

- `close_reviewer_windows_for_attempt(mode)`:先静止确认 → 恢复 baseline(保 Coder dirty)→ 弃置关闭(违规 journal 随记录保留审计)+ 释放后续接取消补偿。**Terminate**:恢复失败 fail-closed 保持锁(Err 上抛,终止结果可见);**Delete**:无条件收口(工作树随之删除,通道解冻+租约释放,不留冻结残锁卡共享 common-dir)。
- 接线:`delete_coding_attempt`(先窗口收口再 git 终止对账)与 `finalize_attempt_for_plan_termination`(T14 终止入口 1 行接线 `recover_coder_delivery_scopes_for_attempt` + 窗口收口)。

### 5. RevB2T17 ⚠3 收口:同 window_id 并发 begin 跨进程互斥

- `WindowLeaseRecord` 增 owner 进程标识(pid+start_ticks,serde default 兼容);`acquire_lease` 同窗幂等仅限 (a)同 owner 进程(自持)或 (b)owner 已死(崩溃接管,续写 owner 字段)——另一存活进程持有同窗 → `Conflict[reviewer_window_lease_owner]`。同进程并发仍由 engine 单 runner 不变量 + generation CAS 串行(取舍已在代码注释记录)。

## 测试矩阵(14+2 新钉测,均注入式崩溃前缀;真进程重启演练归 T24)

| brief 要求 | 测试 | 断言要点 |
|---|---|---|
| 旧序红测(Step 2) | `startup_scan_compensates_open_window_before_resuming_runner` | 分诊前窗口必须先补偿到终态(实现前 HEAD 红);reviewer 残留清/Coder dirty 留/HEAD 回基线;零 provider 会话;释放后 runner 才放行;通道解冻 |
| 租约持久/manifest未全 | `crash_prefix_preparing_with_lease_abandons_without_recapture` | ClosedInvalidated+租约释放+无 manifest(不重采);重复启动零补偿零推进 |
| 恢复游标中途 | `crash_prefix_mid_cursor_resumes_from_original_manifest` | T16 failpoint 中断→Restoring;驱动按原 manifest 续接(manifest 内容指纹不变)→释放 |
| ViolationRecorded | `violation_window_reaches_manual_gate_keeps_lock_and_projects_attempt` | journal-first→恢复完整→ManualGate 保持锁;attempt 投影人工恢复面;Coder dirty 保留;重复启动 generation/journal/会话零变化 |
| journal 不一致 | `journal_inconsistency_keeps_lock_via_manual_gate` | 序号断裂→ManualGate 保持锁(不洗白) |
| 未知 writer | `unknown_writer_keeps_lock_zero_provider_activity` | RecoveryRequired+租约/通道保持+provider/材料 0+该树分诊零 runner |
| PID 复用口径 | store `lease_owner_liveness_guards_same_window_reacquire` + T17 存量 `channel_reaps_zombie_and_pid_reuse` | 存活外部 owner 同窗重入 Conflict;owner 死亡接管放行;通道 writer PID 复用收割为 T17 存量语义 |
| 登记容器停清 | `registered_writer_process_stopped_and_scope_cleared`(进程类:SIGKILL 组信号+身份观察+scope 清理);`container_without_observer_parks_lock_kept`(容器无观察通道→停等,Drop/stop 声称不当成功证据) | 生产无 OwnedEngineObserver 实现(存量边界,报告披露) |
| 局部坏窗 | `bad_tree_fail_closed_while_conflict_free_tree_resumes` | 坏树停等 fail-closed,无冲突树照常补偿+分诊恢复 runner;窗口终态迁移时间戳早于分诊后观察(trace) |
| 人工拒绝补偿 | `manual_gate_invalidate_keeps_attempt_locked_and_new_round_takes_new_lease` | Invalidate:窗口释放但 attempt 不解锁(零自动 resume/分诊零恢复/旧报告 Invalidated/journal 留审计);RequireNewReview:新窗另取 lease/新 manifest |
| 人工门生产入口 | `reviewer_window_resolve_route_decides_manual_gate`(router oneshot) | 200+state/acceptance 投影+租约释放;非法 decision→400 |
| 人工恢复续接 | `manual_redrive_completes_recovery_or_requires_decision` | Restoring redrive 续接完成;ManualGate 拒(redrive 不代作废);无窗放行 |
| attempt 删除/终止→清理 | `attempt_termination_closes_windows_scopes_and_keeps_coder_dirty`、`attempt_delete_close_is_unconditional_after_park` | 终止:窗口 ClosedInvalidated+租约释放+T14 scope 回收+Coder dirty 保留+Aborted;删除:停等窗也无条件收口(通道解冻+租约释放) |
| Git 补偿串行续接 | git_operation_reconcile `startup_compensation_resumes_cancelled_review_commit_after_window_release` | 冻结期补偿拒(T19 存量);驱动恢复→释放→同驱动内续接 Compensated+HEAD 回 before_head+同树可再开窗(死锁反证) |

## 硬约束处置表(审查积累)

| # | 约束 | 处置 |
|---|---|---|
| 1 | RevB2T19 ⚠2:resolve_manual_gate 无生产入口;attempt 删除/终止窗口收口;socket.rs:580 手工恢复面 | ✅ REST `reviewer-window:resolve`+engine 入口;delete/finalize 双接线(钉测覆盖);RecoverCoding 前置 redrive |
| 2 | T14 终止入口 1 行接线 | ✅ `finalize_attempt_for_plan_termination` 内 `recover_coder_delivery_scopes_for_attempt`+钉测(scope 回收断言) |
| 3 | RevB2T17 ⚠3:同 window_id 并发 begin 无互斥 | ✅ lease owner 进程标识收口(存活 owner 互斥/死亡接管);同进程面记录取舍(单 runner 不变量+CAS) |
| 4 | RevB2T18:启动对账沿 reconcile 权威;T16 Err→RecoveryRequired 语义 | ✅ 消费 `restore_window` 原语义(Err 不迁移/RecoveryRequired 持久+游标);`recheck_review_acceptance_for_delivery` 同源复检 |
| 5 | T17 exit finally 语义:同进程 panic 泄漏 writer 致排空停等 | ✅ 启动补偿 quiesce 只凭 durable 身份(killpg+身份观察);死进程通道 writer 由 T17 排空收割;补偿续接的通道 enter/exit 沿 T19 finally 语义(失败回退释放) |
| 6 | 启动不重冻污染基线 | ✅ 只按已发布 manifest 恢复;Preparing 无 baseline 直接弃置(不重采);指纹不变钉测 |

## Concerns(交审查)

1. **「人工拒绝补偿不解锁」的落地解释**:窗口租约释放沿 T17 已测终态语义(整轮作废后树可复用);「不解锁」落在 attempt 面——不自动 resume、人工恢复面为唯一出口、旧报告永久 Invalidated(钉测三断言)。如审查裁定需树级持续锁死,需改 T17 终态语义(越界)。
2. 在途 attempt(Running/Created)的 mid-commit git journal 不由启动驱动抢跑补偿(归 runner 恢复/终止收敛面),仅取消已决定形态续接;启动驱动对 Running attempt 的 journal 语义未单测。
3. 容器类登记资源生产无 OwnedEngineObserver 实现(T19 登记的存量边界):启动补偿对容器恒停等 fail-closed,测试以进程类资源证明「停清」、容器以「无观察通道→停等」钉住。
4. 平台合法写入域漂移(platform_domain_drift→RecoveryRequired)路径继承 T16/T17 已测语义,本 Task 未在新驱动下重复单测(驱动经 `restore_window` 同链路)。
5. 全量 lib(5177)未跑完(预算),受影响面按 brief+接线邻接 14 套件跑(549/1,唯一红为登记存量);`git archive` detached 快照复验未执行(预算截止),测试在待提交工作树原位运行(内容=提交内容,git status 核对无未预期文件)。
6. serve_web 次序修正使启动扫描先于 Provider 探活;若探活失败语义依赖先行,属可再议的次序取舍(补偿先行是本 Task 合同)。
