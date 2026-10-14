# S4 交付报告:role run 收尾

- 切片:S4(设计权威 DesignPushLocal2「Q3 数据层」裁定)
- 范围:runner 死亡转 AMR 时同步收尾 Running role run→Failed(reason=`coding_runner_failed_while_running`);RecoverCoding 恢复入口把残留 Running role run 标 Superseded(先例 `recovery.rs` `ensure_failed_code_review_retry_role_run` 的 stale 收尾)。不动 unit 状态与 `start_claim`(设计红线)。

## 变更清单(4 文件,与 S1/S2a/S3/S6/FE 零重叠)

| 文件 | 变更 |
|---|---|
| `src/product/coding_attempt_store/role_run.rs` | 新增两个存储方法:`fail_running_role_runs_for_attempt(attempt, reason)`(Running→Failed+reason+completed_at)与 `supersede_residual_running_role_runs_for_attempt(attempt)`(Running→Superseded+completed_at,不预写 superseded_by)。均只改写 Running,非 Running 历史零触碰 |
| `src/web/coding_ws_handler/runner/task.rs` | 死亡转 AMR 的 `Ok(())` 分支内、快照重放之前,同步调用 `fail_running_role_runs_for_attempt(&latest, MANUAL_RECOVERY_REASON)`;失败仅 warn(fail-visible,不吞没 protocol error) |
| `src/web/coding_ws_handler/socket.rs` | RecoverCoding 分支 `Ok(updated)` 内、`spawn_coding_runner` 之前,调用 `supersede_residual_running_role_runs_for_attempt(&updated)`;必须在 spawn 前执行,避免误杀重启后新建的 Running run;失败仅 warn 不阻断恢复 |
| `src/web/coding_ws_handler/tests/runner_recovery.rs` | 新增两条红绿测试(仿本文件 F-14/F-16 先例) |

## 红绿证据(TDD)

1. **红(已观察)**:先写 `runner_death_finalizes_running_role_runs_to_failed` 并运行——
   `assertion left: Running, right: Failed`(在途 coder run 死亡后仍 Running,即 S4 缺口)。
2. **绿**:实现存储方法+task.rs 接线后,`cargo test --lib coding_ws_handler::tests::runner_recovery::` → **10 passed; 0 failed**(8 条既有 + 2 条新)。
3. `recover_coding_supersedes_residual_running_role_runs` 的红为「方法缺失编译红」,因两个存储方法一次性落地未单独观察;其行为缺口与测试 A 同类(无任何收尾写方)。绿面断言:残留 run→Superseded+completed_at;死亡收尾的 Failed run 不被改写;清场后无 Running 残留。

## 语义安全性核查(改前已完成)

- 各阶段重入(coding.rs:75 / code_review.rs:64 / internal_pr_review.rs:278)仅复用 `Running && node_id.is_none()` 的 run;收尾为 Failed/Superseded 后重入走新建分支,review_request 重入由 journal 相位驱动,不依赖旧 run 存活。
- `should_resume_coder_session_for_role_run` 不读 run status,会话续接不受影响。
- `create_manual_retry_role_run` 要求 latest 为 Failed——死亡收尾反而使其可达。
- RecoverCoding 与 failed-review-retry 流程以 attempt 状态互斥(AMR vs Running/Blocked),无清场竞态。

## 回归

- `cargo test --lib "coding_ws_handler::tests::"` → **121 passed; 0 failed**(含 F-14/F-16/startup reconcile 全套)。
- `cargo test --lib "coding_attempt_store::admission"` → **40 passed; 0 failed**。
- `cargo test --lib "failed_review_recovery"` → **51 passed; 0 failed**(role run stale 收尾先例消费方)。

## 范围外(未动,设计裁定)

- `resumption.rs:171`、`group.rs:114`、`group_completion.rs:76`、`autopilot_orchestrator.rs:348` 等其他 AMR 入口的 role run 收尾——设计切片仅锚定 task.rs 死亡路径与 RecoverCoding。
- socket RecoverCoding 分支接线为薄组合(CAS→清场→spawn),无 lib 级分支测试,与 F-16 先例同款;测试在 lib 层复刻同款组合。

## 修复轮(P1 审查裁定:豁免恢复 journal 的 retry run)

**缺口**:`fail_/supersede_` 无差别收尾会打穿 failed-code-review 恢复 journal 持有的 retry role run——它必须保持 `Running + reason=recovery_key` 才能被 `ensure_failed_code_review_retry_role_run` 的 reason 匹配与 `validate_retry_role_run` 识别复用;打穿后 retry_review 通道永久关死(gate 悬挂无动作)。

**修复**(role_run.rs):新增 journal 豁免——journal 未 completed 时返回 `(retry_role_run_id, recovery_key)`,两个收尾/清场方法对「id 登记命中 **或** `reason_code==recovery_key`」的 Running run 跳过不收尾。reason 匹配覆盖「retry run 已建而 journal 尚未登记 id」的窗口,与 store 自身恢复/回滚逻辑(rollback 的双路识别)同源,宽于审查建议的纯 id 匹配,属同根因的完备化。

**裁定取舍(登记)**:接受豁免 run 在 AMR 态投影为 running 的已知单例投影瑕疵(<恢复通道永久死);若要消此瑕疵,走「恢复流程重开 journal run」另立切片。

**红绿**:红已观察——`runner_death_and_recovery_sweeps_exempt_incomplete_journal_retry_run`(failed_review_recovery/runner.rs,fixture 构建 journal 至 AttemptRunning 前缀 + 空 registry runner 前段死亡)`left: Failed, right: Running`;修复后全绿。断言面:死亡后 journal retry run 仍 Running+reason 原样、对照 decoy run 照常 Failed;RecoverCoding 清场后 retry run 仍 Running、残留 run 照常 Superseded;`recover_failed_code_review` 仍收敛(引擎侧至 GateResolved——Completed 由 reserved spawn 的 runner 在 provider 进入前完成,先例 reserved_spawn_* 事件序)。

**回归(修复轮,按过滤器口径)**:filter `coding_ws_handler::tests::failed_review_recovery::`(**ws_handler 侧 31** passed 0 failed);filter `coding_attempt_store::tests::failed_review_recovery`(**store 侧 15** passed 0 failed,含 rollback 2);filter `provider_failure_recovery::reviewer_recovery`(engine 侧 6 passed 0 failed);另有 `runner_recovery` 10/0、`coding_ws_handler::tests::` 全量 122/0(121+1 新)、`admission` 40/0。裸 filter `failed_review_recovery` 共 53 项=ws_handler 32+store 15+engine 4+web 其他 2(修复轮 2 后统计)。

## 修复轮 2(复审残余观察①裁定必修:豁免补 completed-未 spawn 窗口 + 格式清偿)

**缺口**:reserved spawn 把 journal 推到 Completed 之后、retry run attach node 之前的死亡窗口——retry run 仍 `Running+node_id.is_none()` 是 `completed_journal_waits_for_retry_node` 的等待凭据(completed journal 不再可走未完成恢复路径);收尾打穿它则该判定永久 false,retry_review 通道死。

**修复**(role_run.rs):豁免 helper 更名 `recovery_journal_retry_exemption` 并放宽——「journal 未 completed **或**(completed 且 `retry_role_run_id` 的 run 仍 `node_id.is_none()`)」均返回豁免凭据;已 spawn(node_id 有值)的 retry run 按普通 run 收尾。同轮 rustfmt 清偿 role_run.rs 三处行宽偏差,`rustfmt --edition 2024 --check` 该文件与 runner.rs 测试文件零 diff。

**红绿**:红已观察——`runner_death_sweep_exempts_completed_journal_unspawned_retry_run`(journal 前缀 AttemptRunning 后直接 advance 至 Completed、retry 节点不 spawn,空 registry runner 死亡)`left: Failed, right: Running`;修复后全绿。断言面:前置 `recoverable_failed_code_review` 为 Some;死亡后 retry run 仍 Running+node_id None+reason 原样、decoy 照常 Failed;RecoverCoding 清场后仍 Running;`recoverable_failed_code_review` 复检仍 Some。

**回归(修复轮 2)**:ws_handler 侧 failed_review_recovery **32/0**(31+1 新);store 侧 **15/0**;engine `provider_failure_recovery::reviewer_recovery` **6/0**;`runner_recovery` **10/0**;`coding_ws_handler::tests::` 全量 **123/0**(122+1 新);`admission` **40/0**;`rustfmt --edition 2024 --check` role_run.rs 与 runner.rs 零 diff。
