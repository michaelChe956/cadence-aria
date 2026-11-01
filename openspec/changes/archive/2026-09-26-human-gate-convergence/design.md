# Design: human-gate-convergence

> 依据：统一方案 §5-C2；实证 f52-diagnosis.md（预算假数/cycle key/severity 双轨/invalid_json）、F-49 A8、f54-diagnosis.md（两道门不可区分）。

## Context

manual_repairs_used=0 而用户已 3 轮修订；chip 静态 3；cycle key 每轮新；severity advisory 文案与 must_fix 实效矛盾；invalid_json 白耗轮；教学锁死结构联动；批次门与候选门同构。

## Decisions

### D1 预算权威=快照+gate-local 计数

manual_repairs_remaining（快照）唯一授权；新增 gate-local accepted_feedback_turns（与预留同 CAS 原子提交）；门卡优先读快照（F-49 A8 的 opened_snapshot_identity 机制沿用），旧 turn 值仅历史。REQ-CG-02 的重置边界修订为 logical gate 粒度（评估器侧改：重建快照时接续而非重置——普通门臂）。amendment 接续臂不动。

### D2 收敛投影从 durable 派生

跨轮 delta：本轮 verdict findings × 前轮（snapshot.findings/C1 结构化 identity）对比——新增/已解决/复现；不可比显 unknown。「距通过」：must_fix 清单+C1 预检结果+Verification 状态+approve 可用性，全部从既有 durable 事实派生（零新状态机）。批次门专属文案：投影层按门类型（gateKindOf 已有 batch/human_gate 区分）出专属 title/why 文案，删 F-54 暴露的误导相位提示行。

### D3 双轨分类+invalid_json 重试+教学逃生

UI：effective class 主显+severity 标注（finding-list 组件扩展）。解析重试：structured_output 解析失败时同 invocation 一次重试（bounded，不增 review/repair/turn 计数），失败保 diagnostic 进人工。REVISION_TEACHING 加逃生条款（反馈点名结构变更→允许受影响闭包联动+文首声明影响面；禁删必需字段/绕过 validator 不变）。

## Risks

- CG-02 重置边界修订触既有测试语义（F-40 era 的重置断言）——按新语义改写并登记
- delta 对比依赖 C1 identity（结构化）——C2 实施在 C1 后（顺序保证）
- 批次门文案变化是纯投影层（零协议变化）

## Open Questions

（无）
