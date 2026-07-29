# Spec: work-item-draft-handoff-contract-refs

## ADDED Requirements

### Requirement: 末端 WorkItem 的 provided_contract_refs 允许为空

Work Item Draft 生成 Prompt SHALL 允许 `handoff_contract.provided_contract_refs` 为空数组，且 MUST 明确要求：仅当本 WorkItem 的产出被某个后续 WorkItem 的 `input_contracts` 消费时，才在 `provided_contract_refs` 中列出对应契约 ref；无下游消费者（链路末端）时必须为空数组。`required_fields` 与 `reviewer_check_refs` 的非空且不重复约束 MUST 保持不变。

#### Scenario: 末端 WorkItem 生成空交接契约引用

- **WHEN** Provider 为无下游消费者的链路末端 WorkItem 生成 Draft
- **THEN** 该 Draft 的 `handoff_contract.provided_contract_refs` 为空数组，且能通过 Draft 解析与 Canonical 校验

#### Scenario: 中间 WorkItem 生成交接契约引用

- **WHEN** Provider 为存在下游消费者的 WorkItem 生成 Draft
- **THEN** `provided_contract_refs` 仅列出被下游 `input_contracts` 实际消费的契约 ref，且元素唯一、非空白

### Requirement: Prompt 规则与 Final Compile 校验器不变量一致

Draft 生成 Prompt 对 `provided_contract_refs` 的约束 MUST 与 Final Compile 的 Canonical 依赖图校验（`unconsumed_required_handoff`）保持一致：Prompt 指示 Provider 产出的任何 `provided_contract_refs` 组合都不得触发该校验的 error finding。后端校验器语义 MUST NOT 因本能力而放宽。

#### Scenario: 两项链路通过 Final Compile

- **WHEN** 一个 Plan 包含"实现 + 测试"两个 WorkItem，测试项消费实现项的产出契约，实现项按 Prompt 列出该 ref，测试项作为末端项留空
- **THEN** Final Compile 的 Canonical 依赖图校验不产生 `unconsumed_required_handoff` finding，编译事务可以推进到提交完成

#### Scenario: 校验器语义不放宽

- **WHEN** 某 WorkItem 的 `provided_contract_refs` 列出未被任何下游消费的契约 ref
- **THEN** Final Compile 的 Canonical 依赖图校验仍产生 `unconsumed_required_handoff` error finding
