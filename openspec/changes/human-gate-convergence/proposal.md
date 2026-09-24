# Proposal

## Why

统一方案 C2：F-52 P1/P2 + F-49 A8 + F-54 的门呈现域。实证：3 次门内修订后 `manual_repairs_used=0`、门卡「剩余修复轮次」恒显 3（静态假数）；cycle key 每轮变化使预算永不耗尽；`severity`（用户语义）与 `class_hint`（策略语义）双轨致「建议不阻断」文案与实际需人工门矛盾（F-49 实证）；reviewer 结构化输出 `invalid_json` 无重试白耗一轮；SC 修订教学「只改反馈点名的内容」锁死反馈要求结构变更时的联动更新；两道连续确认门（候选确认→发布确认）卡片完全同构致用户无法区分（F-54 实证）；门卡无「距通过差什么」收敛视图。

## What Changes

1. **预算真值**：`human_gate_snapshot.manual_repairs_remaining` 为唯一授权事实（CAS 恰扣一次）；同一 logical gate 修订不重置预算（carry-forward，修订 REQ-CG-02 的每次重置边界）；gate-local accepted_feedback_turns 计数（不与 policy repairs_used 双计）；预算耗尽仅 approve/abandon。
2. **收敛投影**：门卡显示当前真值预算+findings 跨轮 delta（新增/已解决/复现/unknown）+「距通过」清单（当前 must_fix/options 预检/Verification scope/可确认性）；批次确认门专属文案（「确认发布整组 Work Items」）与候选确认门区分。
3. **有效分类透明**：UI 同时显示 severity 与 effective class（class_hint 优先的策略语义），消灭「建议不阻断」误导。
4. **invalid_json 有界重试**：同 invocation 一次结构化解析重试（不增任何预算计数）；仍失败保留 diagnostic 进人工，不合成空返修目标。
5. **修订教学逃生条款**：反馈要求结构变更（增删 WI/contract）时允许受影响闭包联动更新（文首声明影响面）；仍禁删必需字段/绕过 validator。

## Capabilities

### New Capabilities

- `human-gate-convergence`: 人工门的预算权威、收敛投影与呈现区分契约

### Modified Capabilities

- `work-item-plan-conversational-gate`: REQ-CG-02（预算 carry-forward：按 logical gate/candidate 而非每次修订重置）、REQ-CG-03（修订教学逃生条款）、REQ-CG-05（accepted turns 计数口径）

## Non-Goals

- 不把 advisory 自动提升 must_fix；不做无限 retry；UI 不自行扣预算
- 不重复 C1 preflight/identity（已归 C1）
- 不动 abort/terminate 语义（C3 域）
