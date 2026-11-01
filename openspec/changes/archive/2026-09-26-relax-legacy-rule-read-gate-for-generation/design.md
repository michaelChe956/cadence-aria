# Design: relax-legacy-rule-read-gate-for-generation

## Context

`direct_cadence_routing_rules_reference(&RoutingReferenceContext)` 是规则引用的唯一注入源，被 5 类 prompt 构建点调用（split_engine outline/draft、workspace_engine plan/revision、draft_batch、web workspace_context）以及 `provider_context_builder.rs`（coding 路径的 `routing_reference` 槽位）。当前 Legacy 分支返回的 `LEGACY_REFERENCE` 含两段：规则依据声明 + 强制完整读取/失败关闭门禁。

## Goals / Non-Goals

- Goals：仅降级 Legacy 生成类 prompt 的规则引用；保持注入 API 形状稳定；coding 与 Logical 路径零变化。
- Non-Goals：不动 `RoutingReferenceContext` 枚举与 `routing_reference_context_from_policy`；不动 Logical 文案；不引入新配置项。

## Decision：按调用点分流而非按上下文改全局函数

`direct_cadence_routing_rules_reference` 全局替换会同时命中 coding 路径（provider_context_builder 与 coding_workspace_engine），违反"coding 保持现状"。因此：

1. 在 `routing_reference.rs` 新增 `generation_cadence_routing_rules_reference(context) -> String`：
   - `Legacy` 分支返回降级文案：声明目标仓根目录 AGENTS.md/CLAUDE.md 是流程规则依据，**按需查阅适用章节即可**；读取失败或文件缺失时在输出中注明"规则未加载"，继续生成，不阻塞、不作为输出前置。
   - `Logical` 分支直接委托现有 `logical_cadence_routing_rules_reference`（逐字一致，保持政策门禁）。
2. 生成类调用点（`work_item_split_engine/prompts.rs` 的 outline/draft runtime contract、`workspace_engine/prompts.rs` 的 `initial_author_runtime_contract` 与 `reviewer_output_contract`、`workspace_engine/prompts/revision.rs` 的 revision delta/full prompt）从 `direct_...` 切换到 `generation_...`。降级文案保留 `[cadence_project_rules]` 段标记，使 `has_direct_cadence_routing_rules_system_context` 的去重谓词对新旧两变体同判，resume/重复注入行为不变。
3. `provider_context_builder.rs`、`coding_workspace_engine/prompts.rs`（coding 路径）与 `web/workspace_context/prompts.rs`（交互会话入口，规则路由的生效位置）继续调用 `direct_...`，不改一字；`workspace_engine/draft_batch/runs.rs` 仅透传 context 给 split engine，无需改动。

备选方案（否决）：给 `direct_...` 加 `stage` 参数——改动面更小但把"哪些阶段算生成"的判断散进函数签名，且 provider_context_builder 的节点粒度（node_id）与"生成/编码"分类不正交。

## Failure Handling

- 降级文案仍保留规则文件位置声明，模型可在需要编码规范细节时自行查阅（按需而非强制）。
- 若目标仓库确实依赖规则才能产出合法拆分（罕见），由既有 outline/draft 校验器与 reviewer 兜底，而非靠前置读取门禁。

## Migration

无数据迁移。文案为纯 prompt 变更；同步更新以下测试对固定文案的断言：`routing_reference.rs` 内嵌单测、`coding_workspace_engine/tests/parser_prompt/*`、`work_item_split_engine/tests/routing_reference_contract.rs`、`workspace_engine/prompts.rs` 与 `prompts/revision.rs` 内嵌 prompt 契约测试。

## Testing

- 单测：`generation_...` Legacy 文案包含按需语义、不含"完整读取"与"只报告阻塞"字样；Logical 输出与 `direct_...` 逐字一致；`direct_...` Legacy 文案不变（守卫 coding 路径）。
- prompt 契约测试：各生成类 prompt 渲染结果引用新函数输出；coding 路径渲染仍引用旧文案。
- 回归：`cargo test -p <crate>` 定向跑 routing_reference、parser_prompt、routing_reference_contract 相关模块。
