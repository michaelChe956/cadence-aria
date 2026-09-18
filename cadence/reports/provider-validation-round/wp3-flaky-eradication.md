# WP3 kimi terminal flaky 根治报告（close-provider-validation，RR-3① 销账）

- 计划：`cadence/plans/2026-09-18_计划文档_阶段4-C1_provider验证与收尾_v1.0.md`（v1.1，commit c8101323）Task 4
- 基线：工作树 feat-b-0808-add-monorepo，HEAD=c8101323；改造前断言行号锚=4cc834e2（计划实读锚）
- 日期：2026-09-18/19（实施窗口 23:0x–00:3x）

## §1 行为语义零变化对照表（tasks 3.3）

| 行为约束（主 spec 终端生命周期） | 改造前断言（行号=4cc834e2） | 改造后断言 | 等价判定 |
|---|---|---|---|
| 上限常量 1_048_576 | 引用 `MAX_TERMINAL_OUTPUT_BYTES`（:711/:772/:804） | 同常量引用（未改 :25） | ✓ |
| 恰为上限完整返回且不截断 | `output_exactly_at_cap…`:788-790（exit 0+!truncated+==MAX） | `…exactly_at_cap…deterministic`（len==MAX+!truncated+budget==MAX） | ✓ byte-exact 同构 |
| 超限截断且只标记一次 | `output_is_capped…`:727-728（truncated+==MAX） | `…over_cap…deterministic`（len==MAX+truncated+budget==MAX+字节全保留校验） | ✓ |
| 并发共享预算不超额 | `concurrent_streams…`:820-821（truncated+==MAX） | `…concurrent…deterministic`（合计==MAX+truncated+budget==MAX，任意交错） | ✓ |
| 真实管道端到端（进程组/pipe/输出回收） | 三测试真实进程形态 | 唯一 `real_process_pipe_smoke_cap_end_to_end`（全链路+cap 生效+exit 0+release 幂等） | 覆盖保留（唯一 smoke，tasks 3.2） |
| 边界组合覆盖 | {MAX, MAX+1, 双流 2×MAX} | 同三组合确定性锁定（duplex 受控内存流直驱 `read_terminal_stream` :518 注入点） | 组合不减少 ✓ |
| 幂等清理/未知 id/并发上限/超时终止/ETXTBSY 重试 | 既有非 cap 用例 10 条（:599-:897 区域） | 零改动保留（本 Task 全量 lib 绿佐证） | ✓ |
| 无症状压制 | — | 无断言放宽/无预算加大（确定性族零 timeout；smoke 60s 为负载不敏感口径非原紧预算放大） | ✓（grep `#[ignore]` 全文件零匹配，新增=零） |

### 实施备注（对计划片段的唯一修正）

计划 Step 1 代码片段 `assert!(output.bytes().all(|byte| byte == Ok(b'a')))` 存在类型错误：`String::bytes()` 迭代产出 `u8`（非 `Result<u8,_>`），`Ok(b'a')` 包装导致 E0277 编译失败。最小修正为 `byte == b'a'`（断言意图不变：超限场景下保留的字节全为 'a'，即首 MAX 字节完整保留、无错位/污染）。零生产代码改动（`read_terminal_stream` 泛型注入点原样复用）。

## §2 复跑稳定性证据（Step 4）

验证方式迁移非行为修复：新确定性测试对现行实现即绿（特征化锚定），「绿」证据=多次复跑零触发+边界组合不减少+§1 对照表等价。

| 验证项 | 命令 | 结果 |
|---|---|---|
| 确定性边界族 | `cargo test --lib cap_boundary -- --nocapture` | **3 passed / 0 failed，finished in 0.02s**（无墙钟预算，负载不敏感） |
| 唯一真实进程 smoke | `cargo test --lib real_process_pipe_smoke_cap_end_to_end -- --nocapture` | **1 passed / 0 failed**（60s 负载不敏感口径，实际亚秒完成） |
| 10 次隔离复跑 terminal 族 | `for i in $(seq 1 10); do cargo test --lib terminal -- --nocapture; done`（`/tmp/wp3-rerun.log`） | **10/10 全绿：每轮 76 passed / 0 failed**，单轮 1.14–2.85s（原三测试单轮 10–15s 紧墙钟预算，flaky 在案 RR-3①） |
| 全量 lib | `cargo test --lib` | **3368 passed / 0 failed / 3 ignored（既有），36.48s** |
| 测试清单净变化 | — | −3 flaky（`output_is_capped_and_truncation_flagged_once`/`output_exactly_at_cap_is_not_truncated`/`concurrent_streams_share_budget_without_overdraw`）+4（确定性族 3+smoke 1），净 +1 |

## §3 RR-3① 销账登记（Step 6）

`.superpowers/sdd/2026-09-04_计划文档_3.6全量收敛轮_v1.0/progress.md` 末尾追加销账行（该文件位于 gitignored `.superpowers/` 树，为本地台账，不入 git 提交——计划 Step 7 的 `git add` 对该路径不生效，提交号实填 WP3 代码提交）：

> - 阶段4-C1 WP3 flaky 根治销账（2026-09-19，\<WP3 commit\>）：kimi terminal 三测试（output_is_capped/output_exactly_at_cap/concurrent_streams，RR-3①/DEF-7 桶）确定性化完成——cap 边界组合受控内存流锁定（无墙钟预算）+唯一真实进程 smoke；10 次隔离复跑+全量 lib 零触发；行为语义零变化对照表在案（cadence/reports/provider-validation-round/wp3-flaky-eradication.md）。RR-3① kimi terminal 部分销账。
