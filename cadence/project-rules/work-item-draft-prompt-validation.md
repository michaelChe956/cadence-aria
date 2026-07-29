# Work Item Draft Prompt 测试提醒规则

## 触发条件

当变更会修改 Work Item Draft 的 Provider 输入或候选 JSON 约束时，本规则生效。包括但不限于：

- `src/product/work_item_split_engine/prompts.rs` 中 `build_work_item_draft_prompt`、运行时契约、Canonical Contract 字段约束或 Prompt 版本的改动；
- `src/product/work_item_split_engine/tests/prompt_contract.rs` 中反映 Draft Prompt 语义的测试改动；
- 其他直接改变 Draft Prompt、其 Canonical Contract 投影或 Provider 输出结构约束的改动。

纯 UI 展示、失败提示文案或不改变上述 Provider 输入/输出契约的 Validator 实现改动不触发本规则。

## 强制提醒

在交付触发变更前，Agent 必须向操作者明确提示：

> 本次改动涉及 Work Item Draft Prompt 或其结构化契约。建议按 `cadence/designs/2026-07-24_技术方案_WorkItemDraftPrompt测试基线_v1.0.md` 执行 Case A 与 Case B 各 10 个有效首次输出的 Claude Code 验证；是否授权执行？

操作者未明确授权时，Agent 不得调用 Provider。提醒本身不得自动创建 CI、Hook、产品 CLI、持久化评估报告或版本控制语料。

## 验收口径

- 仅以首次 Provider 输出经既有 Parser 与 `WorkItemDraftLocalValidator` 的结果判定成功；自动修复不计入。
- 每个 Case 需要 10 个有效样本且必须 10/10 `pass`。
- `provider_inconclusive` 不消耗有效样本；连续两次或累计第三次后停止并报告。
- 完整 Prompt、Provider Draft、认证信息与目标仓库内容不得写入或提交。
