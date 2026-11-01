# Task 4 报告:Bug2-B 启动补偿与推进检测

- 状态: DONE(停等 k3 审查,审过再做 Task 5)
- Commit: 见 git log 消息 `fix(orchestrator): 启动 gate-only 对账+残留门告警——崩溃窗口收敛、旧门不自动解(Bug2-B)`(以该消息最新一条为准)
- 测试摘要: TDD 红测确认(四前缀不收敛/origin 不补偿/证据不回填 3 类语义红)→ 绿:`cargo test --locked --lib startup_gate_reconcile` 4 通过、`residual_open_gate` 1 通过;回归 autopilot 17 通过、coding_ws_handler 147 通过、coding_attempt_store 218 通过,0 失败;本人文件 rustfmt/clippy 干净。

## 交付内容

1. **启动 gate-only 对账**(`reconcile_blocked_gates_for_attempt_at_startup`,autopilot_orchestrator.rs):内嵌于 serve_web→`reconcile_claimed_coding_runs_once` 每 attempt 分诊之前(先于 Running/claim 分诊),只动门、零 provider 调用,失败显式 eprintln 不阻断扫描:
   - a. 根已写完整关闭事实、归档未落(崩溃前缀)→ 统一关闭原语同动作重放补齐归档(幂等、首关事实不重写);open 根×上一代残留归档(重开半完成)→ reopen 原语清残留(代次不动,与 Task 3 修复轮 F1 同向);
   - b. open 门×run gate_origin 接受事实一致 → `resolve_blocked_gate_for_origin` 精确匹配(gate_id+代次)幂等补偿;仅终态 run(Completed/Failed/Blocked/Aborted——仅补记「动作已接受」,不代表业务通过;run 终态已持久化为前提,Running run 不关);
   - c. 归档缺 action 且恢复 journal(活跃或归档,`retry_role_run_id` 有值)可证 retry_review → 按证据回填动作(`backfill_archived_blocked_gate_action`,resolved_at 历史不伪造);
   - d. 无关联/代次失配/异动作/无证据 → 只告警,durable JSON 不动;
   - e. 恢复 journal 仲裁中(未 completed 且关联该门)→ 不抢先关(优先于 b,防与在飞恢复互抢)。
2. **推进检测**(runner.rs):`residual_open_blocked_gate_warnings` 纯读函数——open 门中 stage 严格早于当前 stage 的为残留,返回告警行由调用侧 eprintln;接线在 CodeReview ContinueAfterApprove→ReviewRequest 推进后;**绝不自动解门**;读取失败静默尽力而为(告警面不阻塞推进主链)。
3. **Task 3 产出消费**:origin 精确匹配+代次相同补偿约束全面落实;新增 store 读/写口(gate.rs,均在文件面内):`blocked_gate_states_for_startup_reconcile`(根记录视图:open+代次+崩溃前缀事实)、`archived_blocked_gate_facts_for_attempt`(归档清单)、`backfill_archived_blocked_gate_action`(证据回填,mutations 锁内,已有动作不覆盖);经 mod.rs 导出 `StartupBlockedGateState`。

## O2 显式回答(审查要求):plan_terminated 与在飞 failed-review 恢复是否可交错?

**可以交错(二者无共享互斥),但方向唯一收敛:终止胜出,恢复通道显式失败或自然收口,零门复活、零半写。**按代码事实分三个时序:

1. **终止先于恢复入口**:admission 链(`admit_provider_run`→`ensure_provider_run_allowed`→`automation_plan_terminated_for_attempt`)在恢复任何写盘前拒绝;若门已被 `close_all_open_gates_for_termination`(plan_terminated 审计动作)归档,新鲜恢复的可恢复判定(`recoverable_failed_code_review` 要求 attempt Blocked+门 open)返回 None → `recovery_state_changed` 显式失败。
2. **终止砸中在飞恢复**(WS 恢复调用与终止编排并发):终止把 open 门以 plan_terminated 归档后,恢复链随后的 `resolve_failed_code_review_gate_idempotent`(retry_review)撞 Task 3 原语的异动作冲突 → `Conflict` 显式失败、不覆盖首关审计;反之恢复先完成归档,`close_all` 无 open 门可关,幂等 no-op。
3. **终止晚于恢复完成**:retry run 已在跑,下一次 admission 被 terminated 拒绝,run 收尾 Failed/AMR;门保持 retry_review 归档不复活。

已知无害残余:时序 2 中断的活跃 journal 可能滞留未归档——admission 已拒再入,滞留仅影响「再次显式恢复请求」的报错形态(Conflict/recovery_state_changed),不影响状态一致性;本任务启动对账对仲裁中门不抢先关,亦不误处理该 journal。**结论:批 1 无需追加互斥**;交错安全性由「admission 前置拒绝 + 原语异动作冲突 + 可恢复判定要求 open 门」三重事实保证。若未来要为时序 2 增加「终止时顺带归档 journal」属增强,非本批必需。

## O4 记录(审查要求)

`open_blocked_gate_generation` 不在 blocked-gate-mutations 锁内读根——并发关门的中间态可能让 origin 构造退化 None(origin None 仅意味着「无关联」,不会误关任何门)。Task 4 启动对账在 serve_web 早期单线程执行,读-判-动之间无并发写者,**实际风险为零**;后续在其他并发场景复用该读口(如 handler 内再取 origin)时需注意此窗口。

## 测试(TDD 红→绿)

- 红测实录:`startup_gate_reconcile_converges_crash_prefixes_without_providers`(归档缺失/残留根未清)、`..._compensates_only_terminal_origin_matched_runs`(终态 origin run 未补偿关)、`..._backfills_archived_action_from_journal_evidence`(缺 action 未回填)3 败;`..._skips_gates_under_recovery_arbitration` 守护绿(实现后仍绿)。
- 迭代中修两处测试自伤:①三门同 reason 被建门去重合并 → 每门独立 reason_code;②旧档模拟需手工剥除 resolved_action/resolved_at(Task 3 原语首关总会写时间,真旧档才缺)。
- 绿测:四用例+runner 告警单测全过;断言含「零 provider 交互」(resumed=0+runner_count=0)与「JSON 未变」(逐字节比对)。

## 回归

autopilot 17 / coding_ws_handler 147 / coding_attempt_store 218 全绿;it_web/it_product 已知存量 5 败不在本面(Main 已裁并入基线);cargo fmt --check 本人文件干净;clippy 本人文件 0 告警(全仓存量红非本批)。

## Concerns

1. 归档缺 action 的证据回填仅覆盖「journal 可证 retry_review」这一种证据(方案指定);其余缺 action 旧档一律只告警留人工,与「历史不伪造」一致。
2. 启动对账的 b 规则取「第一条 origin 精确匹配的终态 run」;同门同代次多条终态 run(理论上经继承可能)不构成歧义——origin 相同则关门事实相同。
3. runner 告警接线仅 ContinueAfterApprove→ReviewRequest 一个推进点(brief 锚 839-868 标注「可选」);其余 stage 推进点的同类残留由启动对账与门自身生命周期兜底。
