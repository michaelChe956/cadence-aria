# Design: single-candidate-lowering-safety

> 依据：统一方案 `.superpowers/sdd/2026-09-24_计划文档_change实施_ears-delivery-normalization_v1.0/f51-53-unified-design.md` §5-C0；实证 `f52-diagnosis.md`（P0 假阳性工厂）。

## Context

`src/product/work_item_plan_compiler/lower.rs:587` 附近：`lower_outputs` 对重复 `capabilities` 行 `*capabilities = split_value(...)` 覆盖赋值；`lower_inputs` 的 `required_capabilities` 同源。CT-001 案例 6 行能力只剩最后 1 行。

## Decisions

### D1 append + first-seen-dedup helper

抽共享 helper（outputs/inputs 两处复用）：值按行出现顺序依次 append；完全相同（字节级）的值仅保留首次。不排序、不改写值形态。

### D2 contract 边界 flush

`contract_id` 新出现时先 flush 前一个 entry；无当前 entry 的字段行按既有诊断/fail-closed 规则处理，不归给下一个 contract。

### D3 累积而非报错（兼容取向）

既有交付已含重复键形态（真实 markdown）；grammar 收紧属未来 change。理由与 EARS 空白归一化同族：确定性地救回已知良性形态，fail-closed 留给语义错误。

### D4 freshness 链零改动

相同 source 修复前后编译产生不同 IR hash=预期；`verify_publish_freshness` 语义不动；历史 publication 不重算（0009 存量只读）。

## Risks

- 极低：纯 lowering 内部语义，无 wire/schema/UI 耦合；唯一外显=含重复字段 source 的 IR 变化（正是修复目标）。

## Open Questions

（无——统一方案已定）
