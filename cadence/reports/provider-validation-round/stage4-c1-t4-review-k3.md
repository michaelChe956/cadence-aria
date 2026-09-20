# 阶段4-C1 T4 flaky 根治 k3 审查报告

- 对象：commit d7525220（terminal.rs 测试区 + wp3-flaky-eradication.md）
- 审查人：C1T4K3（k3 惯例：diff 即视图，不重跑采信实施报告证据）
- 日期：2026-09-18
- 结论：**PASS（correct），findings = 0**

## 审查项核对

| # | 审查动作 | 结果 |
|---|---|---|
| 1 | 计划 Task 4 段 + 实施报告 wp3-flaky-eradication.md 已读 | ✓ 报告含行为语义对照表 8 行、复跑证据表、RR-3① 销账登记，结构与计划 Step 3/4/6 对齐 |
| 2 | 零生产改动：diff 全部 hunk 落在 `#[cfg(test)] mod tests` 区（:699–:923），`read_terminal_stream`（:518）与生产调用点（:407–:421）零触碰 | ✓ |
| 2 | 注入点泛型直驱：`read_terminal_stream<R: AsyncRead + Unpin>`，duplex reader 满足约束；测试 helper 传参类型（Arc<AtomicUsize>/Arc<AtomicBool>/Arc<Mutex<String>>，budget 初值 0）与生产 spawn 点完全同构 | ✓ |
| 2 | 三 flaky 形态删除 → 确定性族覆盖同一边界组合 {MAX, MAX+1, 双流 2×MAX}，组合不减少 | ✓ |
| 2 | `byte == b'a'` 修正：`String::bytes()` 迭代产出 `u8`，`Ok(b'a')` 确为 E0277；修正后断言意图不变（首 MAX 字节全 'a'，无错位/污染），且 'a' 为合法 UTF-8，`from_utf8_lossy` 分块不引入长度漂移 | ✓ |
| 3 | 替代验收证据：10 次隔离复跑零触发（76/轮）、8 行语义等价表、唯一 smoke 60s 负载不敏感口径 | ✓ 采信实施报告 |
| 4 | `#[ignore]` 零新增（全文件 grep 无匹配）；RR-3① 销账行落在 gitignored `.superpowers/` 本地台账、不回写 git 提交文件 | ✓ 合规 |
| 5 | 越界核查：commit 仅 2 文件（terminal.rs 测试区 + 新报告），无越界改动 | ✓ |

## 交叉边界核查（cross-boundary）

- 消费侧：`read_terminal_stream` 为 cap 逻辑唯一实现点（预算 CAS 预留/截断标记/保留输出），确定性族直驱该函数本身，无消息分发/路由遗漏面。
- smoke 保留 manager 全链路（create→start→wait_for_exit→output→release×2 幂等），并**加强**断言（新增 exit_code==Some(0)、release 幂等），无断言放宽。
- 既有非 cap 用例（幂等清理/未知 id/并发上限/超时/ETXTBSY）零改动保留。

## 计数

- findings：0（P0=0 / P1=0 / P2=0 / P3=0）
- overall_correctness：**correct**
- confidence：0.93
