# Design: relax-terminal-handoff-contract-refs

## Context

真实环境复现链路（issue_0001）：

1. Draft 生成 Prompt（`src/product/work_item_split_engine/prompts.rs:653`、`:660`）强制 `handoff_contract.provided_contract_refs` 为非空数组；
2. Final Compile 提交阶段构建 Canonical 依赖图并执行 `report_unconsumed_handoffs`（`src/product/work_item_contract/dependency.rs`）：任何 `provided_contract_refs` 中的 ref 若未被其他 WorkItem 的 `input_contracts` 消费，即产生 error finding，编译事务在 `committing` 步进入 `recovery_required`；
3. 链路末端 WorkItem 按定义无下游消费者 → 两条规则矛盾 → 任何按 Prompt 生成的 Plan 永远无法完成 Final Compile，后续人工确认被 `INVALID_HUMAN_CONFIRM_ACTION` 拒绝。

已核实的无需改动面：

- Draft JSON Schema（`work_item_split_engine/schema.rs:290-294`）对 `provided_contract_refs` 无 `minItems`，空数组合法；
- Canonical 校验（`work_item_contract/validation.rs`）只检查元素空白与重复，不检查数组非空；
- 代码库既有测试夹具（`workspace_engine/tests/part_03/part_02.rs:647`、`part_08.rs:108`）对末端项手动 `provided_contract_refs.clear()`，与本次 Prompt 放宽方向一致。

约束：Prompt 有 12,000 字节质量预算（`QUALITY_BUDGET`，测试锁定）；改 Draft Prompt 交付前须按项目规则提醒操作者授权 Case A / Case B 各 10 次真实 Claude Code 验证。

## Goals / Non-Goals

**Goals:**

- 消除 Prompt 非空约束与编译校验器之间的矛盾，使含末端项的 Plan 能完成 Final Compile；
- 在 Prompt 中写入与校验器一致的可执行判断标准，避免 Provider 继续给末端项虚构交接契约；
- 保持 Prompt 质量预算断言通过。

**Non-Goals:**

- 不放宽后端校验器：`unconsumed_required_handoff` 仍为 error；
- 不修复既有失败 Plan 的存量数据（用户在界面重新生成末端 Draft 后重跑 Final Compile）；
- 不改动 `required_fields` / `reviewer_check_refs` 的非空约束；
- 不新增运行时校验、CLI 或评估报告。

## Decisions

### D1: 放宽方式 = 修 Prompt，而非修校验器

选择修改 Prompt（方案 A2），而非把末端项 unconsumed handoff 降级为 warning。理由：校验器的 error 语义有测试锁定且合理（中间项的交接契约必须被消费，否则交接断裂）；矛盾的唯一来源是 Prompt 的"均非空"表述。修 Prompt 是改动最小且语义正确的方向。

### D2: Prompt 新表述 = 条件非空 + 末端必空

将 `prompts.rs` 两处调整为：

- schema 记号行（:653）：`provided_contract_refs: 唯一 str+ 数组` → 改为可空表述，例如 `provided_contract_refs: 唯一 str+ 数组（可为空）`；
- 约束行（:660）：`required_fields、provided_contract_refs、reviewer_check_refs 均非空且不重复` → 改为：`required_fields、reviewer_check_refs 非空且不重复；provided_contract_refs 元素唯一非空白，仅列出被下游 WorkItem input_contracts 消费的契约 ref，无下游消费者（链路末端）时必须为空数组`。

理由：Provider 生成 Draft 时能拿到 outline 依赖顺序，具备判断"是否末端"的上下文；把校验器不变量直接写成 Prompt 规则，比单纯"可为空"更能防止复发。

备选（已否决）：

- 仅改"可为空"不加指引（变体 A1）：Provider 缺乏判断标准，失败会偶发；
- 校验器豁免末端项：削弱交接完整性保证，且需改动有测试锁定的既有语义。

### D3: 测试策略

- 更新 `work_item_split_engine/tests/prompt_contract.rs` 中断言旧"均非空"文本的用例，改为断言新的条件非空表述与"末端必空"规则存在；
- 更新引用旧文本的其他夹具/断言（如 `tests/part_01.rs` 中相关 Prompt 断言）；
- 不新增后端行为测试（后端语义未变，既有 `unconsumed_required_handoff` 测试保持绿色即为回归证据）。

## Risks / Trade-offs

- [Prompt 文本变长突破 12,000 质量预算] → 新表述控制在最小增量；若超预算，按既有先例在预算内等价压缩措辞，不提高预算常量。
- [Provider 不遵守"末端必空"，仍虚构下游] → Prompt 已给出明确判断标准；残余风险由编译校验器兜底（fail-closed，不产生脏数据）。
- [存量失败 Plan 无法自动恢复] → 明确告知用户在界面对受影响 Plan 重新生成末端 Draft 并重跑 Final Compile；属预期操作路径（recovery Continue / 重新生成）。
