# Proposal: relax-terminal-handoff-contract-refs

## Why

Work Item Draft 生成 Prompt 强制要求每个 Draft 的 `handoff_contract.provided_contract_refs` 非空，而 Final Compile 的 Canonical 依赖图校验（`unconsumed_required_handoff`）要求每个 listed ref 必须被下游 WorkItem 的 `input_contracts` 消费。对链路末端 WorkItem（无下游消费者），两条规则不可同时满足，导致任何按 Prompt 生成的 Plan 都无法通过 Final Compile：编译事务停在 `committing` 步进入 `recovery_required`，Plan 永远得不到 `work_item_ids`，人工确认被 `INVALID_HUMAN_CONFIRM_ACTION` 拒绝。已在真实环境复现（issue_0001 / compile_20260726062350082）。

## What Changes

- 放宽 Work Item Draft 生成 Prompt 中 `handoff_contract.provided_contract_refs` 的非空约束：允许空数组，并写入与编译校验器一致的不变量——仅当本项产出被后续 WorkItem 的 `input_contracts` 消费时列出对应契约 ref；无下游消费者（链路末端）时必须为空数组。
- 同步更新 Prompt schema 记号行（`唯一 str+ 数组` 表述）与相关 Prompt 契约测试断言。
- 不改后端校验器语义：`unconsumed_required_handoff` 仍为 error；JSON Schema 与 Canonical 校验本就不要求该数组非空，无需改动。

## Capabilities

### New Capabilities

- `work-item-draft-handoff-contract-refs`: Draft 生成 Prompt 对 `handoff_contract.provided_contract_refs` 的可为空规则、与 Final Compile 校验器不变量的一致性要求，以及 Prompt 文本与测试断言的约束。

### Modified Capabilities

无。

## Impact

- 后端：`src/product/work_item_split_engine/prompts.rs`（两处 Prompt 文本）、`src/product/work_item_split_engine/tests/`（Prompt 契约断言与夹具）。
- 质量预算：Prompt 文本略增，需重新通过 12,000 字节质量预算断言。
- 真实验证：按项目规则，交付前需操作者授权执行 Case A / Case B 各 10 次真实 Claude Code 验证。
- 数据修复：本 change 不修复既有失败 Plan 数据；用户需在界面上对受影响 Plan 重新生成末端 Draft 并重跑 Final Compile。
