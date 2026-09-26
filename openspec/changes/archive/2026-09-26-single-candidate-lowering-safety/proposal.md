# Proposal

## Why

F-52 实证 P0：SC 编译器 lowering 对重复字段 last-write-wins——`lower.rs` 的 `lower_outputs` 对同一 contract 的重复 `- capabilities:` 行做覆盖赋值（`lower_inputs` 的 `required_capabilities` 同源）。真实案例 CT-001 有 6 行能力，canonical IR 只剩最后 1 行逗号切分值，造成「缺能力」假缺口 → reviewer 按硬规则转 must_fix → 假阳性工厂驱动用户陷入修订循环（0009 的 R3/R5/R7 反复同一假问题；前 6 版仅两个字节级状态，自动返修确定性还原，形成双吸引子振荡）。

## What Changes

1. **重复字段累积**：同一 contract 内重复 `capabilities`/`required_capabilities` 行按出现顺序 append，完全相同的 capability 值稳定去重（first-seen）；contract 边界 flush 严格隔离，禁止跨 contract 串项。
2. **零语义改写**：不排序、不改大小写/正文/标点/内部空白；只消除完全相同的重复元素。
3. **兼容取向**：选累积而非「重复键报错」——既有交付已含重复键（真实 markdown 形态），报错会把兼容交付变成新阻断；grammar 未来收紧另立 change。

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

- `work-item-plan-single-candidate`: REQ-WSC-02 增补 lowering 重复字段累积语义（4 个新场景）

## Non-Goals

- 不改 validator coverage 口径、reviewer prompt、options、fingerprint（C1 域）
- 不改 `verify_publish_freshness` 语义（新编译产生新 IR hash 走既有 freshness 链）
- 不后台重算既有 publication / 不迁移历史 IR（0009 存量只读）
