# change 实施 C2 human-gate-convergence 计划 v1.0

> 按 superpowers:executing-plans / subagent-driven-development 执行。**前置：C1 已合入**（跨轮 delta 消费 C1 结构化 identity）。

**Goal:** 门预算真实递减+收敛投影（距通过/跨轮 delta/批次门区分）+分类透明+invalid_json 重试+教学逃生。

**Architecture:** 后端（gate-local 计数+CG-02 carry-forward+解析重试）+前端投影（批次门文案/距通过/delta/双轨分类）。

**Spec:** `openspec/changes/human-gate-convergence/`（strict valid）；统一方案 §5-C2。

## Global Constraints

- manual_repairs_remaining 快照为唯一授权；accepted_feedback_turns 与预留同 CAS 原子；不与 policy 计数双计。
- UI 不自行扣预算；advisory 不自动升 must_fix。
- invalid_json 重试恰一次、零计数增量、失败保 diagnostic 进人工。
- 教学逃生条款不放开删必需字段/绕过 validator。
- 旧会话缺 gate-local 事实保守 unknown。
- 测试命令照仓规。

## Review Focus

1. carry-forward 误伤 amendment 接续语义 → T1 amendment 回归
2. delta 误推「已解决」（findings 空≠历史解决）→ T3 空集 unknown 断言
3. 批次门文案改坏既有门卡测试 → T2 改写登记
4. 重试计数泄漏（review/repair/turn 任一增加）→ T4 零增量断言

---

### Task 1: 预算真值（后端）

- [ ] 1.1 失败测试：①三轮修订 3→2→1→0 递减 ②刷新/重连不回填默认 ③accepted 计数原子+不污染 policy ④amendment 接续回归（不动）⑤旧会话保守 unknown。
- [ ] 1.2 跑红→实现（gate-local 字段+CG-02 普通门臂 carry-forward）→绿；既有 CG-02 重置断言按新语义改写并登记。

### Task 2: 批次门区分+距通过（前端投影）

- [ ] 2.1 失败测试：①批次门专属 title/why（「确认发布整组 Work Items」）②候选门不受影响 ③距通过清单（must_fix/预检/Verification/approve 可用性）④删除误导相位提示行。
- [ ] 2.2 跑红→实现→绿；门卡既有测试改写登记。

### Task 3: 跨轮 delta（前端，消费 C1 identity）

- [ ] 3.1 失败测试：①新增/已解决/复现计数（C1 golden 数据驱动）②findings 空→unknown 不推断已解决 ③历史不全 unknown。
- [ ] 3.2 跑红→实现→绿。

### Task 4: 分类透明+解析重试（前后端）

- [ ] 4.1 finding-list 双轨显示（effective class 主显+severity 标注）红绿。
- [ ] 4.2 invalid_json 同 invocation 一次重试（零计数增量断言×3）+失败保 diagnostic 进人工；红绿。

### Task 5: 教学逃生+门禁

- [ ] 5.1 REVISION_TEACHING 逃生条款+prompt contract 断言。
- [ ] 5.2 lib/it_core/it_web/vitest 全量+strict+fmt/clippy；F-52 预算假数现场回放（三轮真递减证据）。

---

## Self-Review

REQ-HGC-01→T1、02→T2/T3、03→T4；CG-02 场景→T1；Review Focus 四条全挂测试。前置 C1；内部 T1→T3 依赖（delta 消费 identity/计数），T2/T4 可与 T1 并行但单 worker 串行最稳。
